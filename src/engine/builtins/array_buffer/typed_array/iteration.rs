//! TypedArray iteration owns callbacks, species results, and live element reads.
#[cfg(test)]
use super::*;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::{ArrayIterationKind, TypedArrayElementKind},
    heap::ContextId,
    object::{CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{DirectCallTarget, NativeArguments, NativeInvocation},
    },
};
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transform_tests;

pub(crate) enum TypedIterationStep {
    Complete(Completion),
    Species { resume: TypedIterationResume },
    Call { resume: TypedIterationResume },
    Element { resume: TypedIterationResume },
    Read { resume: TypedIterationResume },
}
pub(crate) struct TypedIterationResume(Box<TypedIterationResumeState>);
impl std::ops::Deref for TypedIterationResume {
    type Target = TypedIterationResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedIterationResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedIterationResume>() <= 8);
pub(crate) struct TypedIterationResumeState {
    pending_effect: TypedIterationStepPending,
    realm: ContextId,
    phase: IterationPhase,
}
struct IterationInput {
    target: ObjectRef,
    callback: CallableRef,
    this_arg: Value,
    element: TypedArrayElementKind,
    length: u64,
}
struct IterationState {
    input: IterationInput,
    mode: IterationMode,
    index: u64,
}
enum IterationMode {
    Every,
    Some,
    ForEach,
    Map(ObjectRef),
    Filter { selected: ObjectRef, length: u64 },
}
enum IterationPhase {
    MapSpecies(IterationInput),
    Called {
        state: IterationState,
        value: Value,
        index: u64,
    },
    Mapped {
        state: IterationState,
        index: u64,
    },
    FilterSpecies(ObjectRef),
    FilterMethod {
        target: ObjectRef,
        selected: ObjectRef,
    },
    FilterCalled(ObjectRef),
}
impl TypedIterationStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: ArrayIterationKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype iteration received a constructor invocation",
            ));
        };
        let target = match runtime.require_typed_array(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let length = match runtime.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => u64::from(value),
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let callback = runtime.callable_from_value(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "TypedArray iteration callback argv was not padded",
                ))?
                .clone(),
        )?;
        let this_arg = if arguments.actual_arg_count > 1 {
            arguments
                .readable
                .get(1)
                .ok_or(RuntimeError::Invariant(
                    "TypedArray iteration thisArg was missing",
                ))?
                .clone()
        } else {
            Value::Undefined
        };
        let element = runtime.typed_array_snapshot(&target)?.element;
        let input = IterationInput {
            target,
            callback,
            this_arg,
            element,
            length,
        };
        let mode = match kind {
            ArrayIterationKind::Every => IterationMode::Every,
            ArrayIterationKind::Some => IterationMode::Some,
            ArrayIterationKind::ForEach => IterationMode::ForEach,
            ArrayIterationKind::Map => {
                return Ok(Self::request_species(
                    input.target.clone(),
                    element,
                    length,
                    TypedIterationResume(Box::new(TypedIterationResumeState {
                        pending_effect: TypedIterationStepPending::default(),
                        realm,
                        phase: IterationPhase::MapSpecies(input),
                    })),
                ));
            }
            ArrayIterationKind::Filter => IterationMode::Filter {
                selected: runtime.new_array(realm)?,
                length: 0,
            },
        };
        TypedIterationResume::next(
            runtime,
            realm,
            IterationState {
                input,
                mode,
                index: 0,
            },
        )
    }
}
impl TypedIterationResume {
    fn next(
        runtime: &Runtime,
        realm: ContextId,
        mut state: IterationState,
    ) -> Result<TypedIterationStep, RuntimeError> {
        if state.index == state.input.length {
            return Ok(TypedIterationStep::Complete(Completion::Return(
                match state.mode {
                    IterationMode::Every => Value::Bool(true),
                    IterationMode::Some => Value::Bool(false),
                    IterationMode::ForEach => Value::Undefined,
                    IterationMode::Map(target) => Value::Object(target),
                    IterationMode::Filter { selected, length } => {
                        return Ok(TypedIterationStep::request_species(
                            state.input.target,
                            state.input.element,
                            length,
                            Self(Box::new(TypedIterationResumeState {
                                pending_effect: TypedIterationStepPending::default(),
                                realm,
                                phase: IterationPhase::FilterSpecies(selected),
                            })),
                        ));
                    }
                },
            )));
        }
        let index = state.index;
        state.index += 1;
        let value = runtime
            .typed_array_read_index(&state.input.target, index)?
            .unwrap_or(Value::Undefined);
        let mut arguments = Vec::new();
        if arguments.try_reserve_exact(3).is_err() {
            return iteration_oom(runtime, realm);
        }
        arguments.push(value.clone());
        arguments.push(Value::number(index as f64));
        arguments.push(Value::Object(state.input.target.clone()));
        Ok(TypedIterationStep::request_call(
            DirectCallTarget::Callable(state.input.callback.clone()),
            state.input.this_arg.clone(),
            arguments,
            Self(Box::new(TypedIterationResumeState {
                pending_effect: TypedIterationStepPending::default(),
                realm,
                phase: IterationPhase::Called {
                    state,
                    value,
                    index,
                },
            })),
        ))
    }
    pub(crate) fn species(
        self,
        runtime: &Runtime,
        result: NativeConversion<ObjectRef>,
    ) -> Result<TypedIterationStep, RuntimeError> {
        let target = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(TypedIterationStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            IterationPhase::MapSpecies(input) => Self::next(
                runtime,
                self.0.realm,
                IterationState {
                    input,
                    mode: IterationMode::Map(target),
                    index: 0,
                },
            ),
            IterationPhase::FilterSpecies(selected) => Ok(TypedIterationStep::request_read(
                target.clone(),
                runtime.intern_property_key("set")?,
                Self(Box::new(TypedIterationResumeState {
                    pending_effect: TypedIterationStepPending::default(),
                    realm: self.0.realm,
                    phase: IterationPhase::FilterMethod { target, selected },
                })),
            )),
            _ => Err(RuntimeError::Invariant(
                "TypedArray iteration received an unexpected species reply",
            )),
        }
    }
    pub(crate) fn element(
        self,
        runtime: &Runtime,
        result: NativeConversion<[u8; 8]>,
    ) -> Result<TypedIterationStep, RuntimeError> {
        let bytes = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(TypedIterationStep::Complete(Completion::Throw(value)));
            }
        };
        let IterationPhase::Mapped { state, index } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "TypedArray iteration received an unexpected element reply",
            ));
        };
        let IterationMode::Map(target) = &state.mode else {
            return Err(RuntimeError::Invariant("TypedArray map lost its result"));
        };
        // Ignore an invalidated target index, exactly like the shared indexed Set.
        let _ = runtime.typed_array_write_converted_index(target, index, &bytes)?;
        Self::next(runtime, self.0.realm, state)
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<TypedIterationStep, RuntimeError> {
        let result = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(TypedIterationStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            IterationPhase::Called {
                mut state,
                value,
                index,
            } => {
                match &mut state.mode {
                    IterationMode::Every if !runtime.value_to_boolean(&result)? => {
                        return Ok(TypedIterationStep::Complete(Completion::Return(
                            Value::Bool(false),
                        )));
                    }
                    IterationMode::Some if runtime.value_to_boolean(&result)? => {
                        return Ok(TypedIterationStep::Complete(Completion::Return(
                            Value::Bool(true),
                        )));
                    }
                    IterationMode::Every | IterationMode::Some | IterationMode::ForEach => {}
                    IterationMode::Map(target) => {
                        return Ok(TypedIterationStep::request_element(
                            runtime.typed_array_snapshot(target)?.element,
                            result,
                            Self(Box::new(TypedIterationResumeState {
                                pending_effect: TypedIterationStepPending::default(),
                                realm: self.0.realm,
                                phase: IterationPhase::Mapped { state, index },
                            })),
                        ));
                    }
                    IterationMode::Filter { selected, length } => {
                        if runtime.value_to_boolean(&result)? {
                            let key = runtime.property_key_for_index(*length)?;
                            if !runtime.define_own_property(
                                selected,
                                &key,
                                &OrdinaryPropertyDescriptor {
                                    value: DescriptorField::Present(value),
                                    writable: DescriptorField::Present(true),
                                    enumerable: DescriptorField::Present(true),
                                    configurable: DescriptorField::Present(true),
                                    ..OrdinaryPropertyDescriptor::new()
                                },
                            )? {
                                return Err(RuntimeError::Invariant(
                                    "TypedArray filter temporary Array rejected a dense element",
                                ));
                            }
                            *length = length.checked_add(1).ok_or(RuntimeError::Invariant(
                                "TypedArray filter selected length overflowed u64",
                            ))?;
                        }
                    }
                }
                Self::next(runtime, self.0.realm, state)
            }
            IterationPhase::FilterMethod { target, selected } => {
                let callable = runtime.callable_from_value(result)?;
                let mut arguments = Vec::new();
                if arguments.try_reserve_exact(1).is_err() {
                    return iteration_oom(runtime, self.0.realm);
                }
                arguments.push(Value::Object(selected));
                Ok(TypedIterationStep::request_call(
                    DirectCallTarget::Callable(callable),
                    Value::Object(target.clone()),
                    arguments,
                    Self(Box::new(TypedIterationResumeState {
                        pending_effect: TypedIterationStepPending::default(),
                        realm: self.0.realm,
                        phase: IterationPhase::FilterCalled(target),
                    })),
                ))
            }
            IterationPhase::FilterCalled(target) => Ok(TypedIterationStep::Complete(
                Completion::Return(Value::Object(target)),
            )),
            _ => Err(RuntimeError::Invariant(
                "TypedArray iteration received an untyped reply",
            )),
        }
    }
}
fn iteration_oom(runtime: &Runtime, realm: ContextId) -> Result<TypedIterationStep, RuntimeError> {
    Ok(TypedIterationStep::Complete(Completion::Throw(
        runtime.new_native_error(realm, NativeErrorKind::Internal, "out of memory")?,
    )))
}
impl Runtime {
    pub(crate) fn call_typed_array_iteration(
        &self,
        realm: ContextId,
        kind: ArrayIterationKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let mut step = TypedIterationStep::start(self, realm, kind, &invocation, arguments)?;
        loop {
            step = match step {
                TypedIterationStep::Complete(result) => return Ok(result),
                TypedIterationStep::Species { mut resume } => {
                    let source = resume.take_species_source();
                    let element = resume.take_species_element();
                    let length = resume.take_species_length();
                    resume.species(
                        self,
                        self.typed_array_species_create(realm, &source, element, length)?,
                    )?
                }
                TypedIterationStep::Call { mut resume } => {
                    let target = resume.take_call_target();
                    let receiver = resume.take_call_receiver();
                    let arguments = resume.take_call_arguments();
                    {
                        let DirectCallTarget::Callable(callable) = target else {
                            return Err(RuntimeError::Invariant(
                                "TypedArray iteration requested an invalid call target",
                            ));
                        };
                        resume.resume(
                            self,
                            self.call_internal(realm, &callable, receiver, &arguments)?,
                        )?
                    }
                }
                TypedIterationStep::Element { mut resume } => {
                    let element = resume.take_element_element();
                    let value = resume.take_element_value();
                    resume.element(
                        self,
                        super::element::ElementStep::start(self, realm, element, value)?
                            .finish_sync(self, realm)?,
                    )?
                }
                TypedIterationStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    resume.resume(self, self.get_property_in_realm(realm, &object, &key)?)?
                }
            };
        }
    }
}

#[derive(Default)]
struct TypedIterationStepPending {
    species_source: Option<ObjectRef>,
    species_element: Option<TypedArrayElementKind>,
    species_length: Option<u64>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    element_element: Option<TypedArrayElementKind>,
    element_value: Option<Value>,
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
}
impl TypedIterationStep {
    pub(crate) fn request_species(
        source: ObjectRef,
        element: TypedArrayElementKind,
        length: u64,
        mut resume: TypedIterationResume,
    ) -> Self {
        resume.0.pending_effect.species_source = Some(source);
        resume.0.pending_effect.species_element = Some(element);
        resume.0.pending_effect.species_length = Some(length);
        Self::Species { resume }
    }
    pub(crate) fn request_call(
        target: DirectCallTarget,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: TypedIterationResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_element(
        element: TypedArrayElementKind,
        value: Value,
        mut resume: TypedIterationResume,
    ) -> Self {
        resume.0.pending_effect.element_element = Some(element);
        resume.0.pending_effect.element_value = Some(value);
        Self::Element { resume }
    }
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: TypedIterationResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
}
impl TypedIterationResume {
    pub(crate) fn take_species_source(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .species_source
            .take()
            .expect("TypedIterationStep Species source")
    }
    pub(crate) fn take_species_element(&mut self) -> TypedArrayElementKind {
        self.0
            .pending_effect
            .species_element
            .take()
            .expect("TypedIterationStep Species element")
    }
    pub(crate) fn take_species_length(&mut self) -> u64 {
        self.0
            .pending_effect
            .species_length
            .take()
            .expect("TypedIterationStep Species length")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("TypedIterationStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("TypedIterationStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("TypedIterationStep Call arguments")
    }
    pub(crate) fn take_element_element(&mut self) -> TypedArrayElementKind {
        self.0
            .pending_effect
            .element_element
            .take()
            .expect("TypedIterationStep Element element")
    }
    pub(crate) fn take_element_value(&mut self) -> Value {
        self.0
            .pending_effect
            .element_value
            .take()
            .expect("TypedIterationStep Element value")
    }
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("TypedIterationStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("TypedIterationStep Read key")
    }
}
const _: () = assert!(std::mem::size_of::<TypedIterationStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedIterationStep>() <= 64);
