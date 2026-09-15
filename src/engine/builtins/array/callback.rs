//! Array callback loops retain the captured length and reread each observable property.
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::{ArrayFindKind, ArrayIterationKind, ArrayReduceKind},
    heap::ContextId,
    object::{
        CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
        operations::InternalDefineResult,
    },
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum CallbackKind {
    Iteration(ArrayIterationKind),
    Reduce(ArrayReduceKind),
    Find(ArrayFindKind),
}
impl CallbackKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        match target {
            NativeFunctionId::ArrayPrototypeIteration(kind) => Some(Self::Iteration(kind)),
            NativeFunctionId::ArrayPrototypeReduce(kind) => Some(Self::Reduce(kind)),
            NativeFunctionId::ArrayPrototypeFind(kind) => Some(Self::Find(kind)),
            _ => None,
        }
    }
}
pub(crate) enum CallbackStep {
    Complete(Completion),
    Read { resume: CallbackResume },
    Number { resume: CallbackResume },
    Has { resume: CallbackResume },
    Call { resume: CallbackResume },
    Species { resume: CallbackResume },
    Define { resume: CallbackResume },
}
enum Phase {
    Length,
    Number,
    Species,
    Has,
    Read,
    Callback,
    Define,
}
pub(crate) struct CallbackResume(Box<CallbackResumeState>);
impl std::ops::Deref for CallbackResume {
    type Target = CallbackResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for CallbackResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<CallbackResume>() <= 8);
pub(crate) struct CallbackResumeState {
    pending_effect: CallbackStepPending,
    realm: ContextId,
    kind: CallbackKind,
    object: ObjectRef,
    original: Value,
    callback_value: Value,
    callback: Option<CallableRef>,
    this_arg: Value,
    accumulator: Option<Value>,
    result: Value,
    value: Value,
    phase: Phase,
    length: u64,
    cursor: u64,
    selected: u64,
}
impl CallbackStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: CallbackKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array callback requires generic invocation",
            ));
        };
        let object = match runtime.native_to_object(realm, this_value.clone())? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let callback_value = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(
                "Array callback argv was not padded",
            ))?
            .clone();
        let second = if arguments.actual_arg_count > 1 {
            Some(
                arguments
                    .readable
                    .get(1)
                    .ok_or(RuntimeError::Invariant(
                        "Array callback second argument missing",
                    ))?
                    .clone(),
            )
        } else {
            None
        };
        Ok(Self::request_read(
            object.clone(),
            runtime.intern_property_key("length")?,
            CallbackResume(Box::new(CallbackResumeState {
                pending_effect: CallbackStepPending::default(),
                realm,
                kind,
                object,
                original: this_value.clone(),
                callback_value,
                callback: None,
                this_arg: second.clone().unwrap_or(Value::Undefined),
                accumulator: if matches!(kind, CallbackKind::Reduce(_)) {
                    second
                } else {
                    None
                },
                result: Value::Undefined,
                value: Value::Undefined,
                phase: Phase::Length,
                length: 0,
                cursor: 0,
                selected: 0,
            })),
        ))
    }
}
impl CallbackResume {
    fn index(&self) -> u64 {
        match self.0.kind {
            CallbackKind::Reduce(ArrayReduceKind::ReduceRight)
            | CallbackKind::Find(ArrayFindKind::FindLast | ArrayFindKind::FindLastIndex) => {
                self.0.length - self.0.cursor - 1
            }
            _ => self.0.cursor,
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<CallbackStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(CallbackStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Length => {
                self.0.phase = Phase::Number;
                Ok(CallbackStep::request_number(value, self))
            }
            Phase::Species => {
                if !matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "ArraySpeciesCreate returned a primitive",
                    ));
                }
                self.0.result = value;
                self.next(runtime)
            }
            Phase::Read => {
                if matches!(self.0.kind, CallbackKind::Reduce(_)) && self.0.accumulator.is_none() {
                    self.0.accumulator = Some(value);
                    self.0.cursor += 1;
                    return self.next(runtime);
                }
                let index = Value::number(self.index() as f64);
                let arguments = if let CallbackKind::Reduce(_) = self.0.kind {
                    vec![
                        self.0
                            .accumulator
                            .take()
                            .ok_or(RuntimeError::Invariant("Array reduce accumulator missing"))?,
                        value.clone(),
                        index,
                        Value::Object(self.0.object.clone()),
                    ]
                } else {
                    vec![
                        value.clone(),
                        index,
                        if matches!(self.0.kind, CallbackKind::Find(_)) {
                            self.0.original.clone()
                        } else {
                            Value::Object(self.0.object.clone())
                        },
                    ]
                };
                self.0.value = value;
                self.0.phase = Phase::Callback;
                Ok(CallbackStep::request_call(
                    self.0
                        .callback
                        .as_ref()
                        .ok_or(RuntimeError::Invariant("Array callback missing"))?
                        .clone(),
                    if matches!(self.0.kind, CallbackKind::Reduce(_)) {
                        Value::Undefined
                    } else {
                        self.0.this_arg.clone()
                    },
                    arguments,
                    self,
                ))
            }
            Phase::Callback => {
                match self.0.kind {
                    CallbackKind::Reduce(_) => self.0.accumulator = Some(value),
                    CallbackKind::Find(kind) => {
                        if runtime.value_to_boolean(&value)? {
                            return Ok(CallbackStep::Complete(Completion::Return(match kind {
                                ArrayFindKind::Find | ArrayFindKind::FindLast => self.0.value,
                                _ => Value::number(self.index() as f64),
                            })));
                        }
                    }
                    CallbackKind::Iteration(kind) => match kind {
                        ArrayIterationKind::Every if !runtime.value_to_boolean(&value)? => {
                            return Ok(CallbackStep::Complete(Completion::Return(Value::Bool(
                                false,
                            ))));
                        }
                        ArrayIterationKind::Some if runtime.value_to_boolean(&value)? => {
                            return Ok(CallbackStep::Complete(Completion::Return(Value::Bool(
                                true,
                            ))));
                        }
                        ArrayIterationKind::Map => {
                            let index = self.index();
                            return self.define(runtime, index, value);
                        }
                        ArrayIterationKind::Filter if runtime.value_to_boolean(&value)? => {
                            let original = self.0.value.clone();
                            let index = self.0.selected;
                            return self.define(runtime, index, original);
                        }
                        _ => {}
                    },
                }
                self.0.value = Value::Undefined;
                self.0.cursor += 1;
                self.next(runtime)
            }
            _ => Err(RuntimeError::Invariant(
                "Array callback value reply phase mismatch",
            )),
        }
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<CallbackStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Number) {
            return Err(RuntimeError::Invariant(
                "Array callback number phase mismatch",
            ));
        }
        self.0.length = match result {
            NativeConversion::Value(value) => Runtime::length_from_number(value),
            NativeConversion::Throw(value) => {
                return Ok(CallbackStep::Complete(Completion::Throw(value)));
            }
        };
        self.0.callback = Some(runtime.callable_from_value(self.0.callback_value.clone())?);
        if let CallbackKind::Iteration(kind) = self.0.kind {
            self.0.result = match kind {
                ArrayIterationKind::Every => Value::Bool(true),
                ArrayIterationKind::Some => Value::Bool(false),
                _ => Value::Undefined,
            };
            if matches!(kind, ArrayIterationKind::Map | ArrayIterationKind::Filter) {
                self.0.phase = Phase::Species;
                return Ok(CallbackStep::request_species(
                    self.0.object.clone(),
                    if kind == ArrayIterationKind::Map {
                        self.0.length
                    } else {
                        0
                    },
                    self,
                ));
            }
        }
        self.next(runtime)
    }
    fn next(mut self, runtime: &Runtime) -> Result<CallbackStep, RuntimeError> {
        if self.0.cursor == self.0.length {
            let result = match self.0.kind {
                CallbackKind::Iteration(_) => self.0.result,
                CallbackKind::Reduce(_) => match self.0.accumulator {
                    Some(value) => value,
                    None => {
                        return Ok(CallbackStep::Complete(Completion::Throw(
                            runtime.new_native_error(
                                self.0.realm,
                                NativeErrorKind::Type,
                                "empty array",
                            )?,
                        )));
                    }
                },
                CallbackKind::Find(ArrayFindKind::Find | ArrayFindKind::FindLast) => {
                    Value::Undefined
                }
                CallbackKind::Find(_) => Value::Int(-1),
            };
            return Ok(CallbackStep::Complete(Completion::Return(result)));
        }
        let key = runtime.property_key_for_index(self.index())?;
        if matches!(self.0.kind, CallbackKind::Find(_)) {
            self.0.phase = Phase::Read;
            Ok(CallbackStep::request_read(self.0.object.clone(), key, self))
        } else {
            self.0.phase = Phase::Has;
            Ok(CallbackStep::request_has(self.0.object.clone(), key, self))
        }
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<CallbackStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Has) {
            return Err(RuntimeError::Invariant(
                "Array callback boolean phase mismatch",
            ));
        }
        let present = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(CallbackStep::Complete(Completion::Throw(value)));
            }
        };
        if !present {
            self.0.cursor += 1;
            return self.next(runtime);
        }
        let key = runtime.property_key_for_index(self.index())?;
        self.0.phase = Phase::Read;
        Ok(CallbackStep::request_read(self.0.object.clone(), key, self))
    }
    fn define(
        mut self,
        runtime: &Runtime,
        index: u64,
        value: Value,
    ) -> Result<CallbackStep, RuntimeError> {
        let Value::Object(object) = &self.0.result else {
            return Err(RuntimeError::Invariant(
                "Array callback result was not an object",
            ));
        };
        let object = object.clone();
        self.0.phase = Phase::Define;
        Ok(CallbackStep::request_define(
            object,
            runtime.property_key_for_index(index)?,
            OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
            self,
        ))
    }
    pub(crate) fn defined(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<InternalDefineResult>,
    ) -> Result<CallbackStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Define) {
            return Err(RuntimeError::Invariant(
                "Array callback define phase mismatch",
            ));
        }
        let filter = matches!(
            self.0.kind,
            CallbackKind::Iteration(ArrayIterationKind::Filter)
        );
        let index = if filter {
            self.0.selected
        } else {
            self.index()
        };
        if let Some(value) =
            runtime.finish_create_indexed_data_property(self.0.realm, index, result)?
        {
            return Ok(CallbackStep::Complete(Completion::Throw(value)));
        }
        if filter {
            self.0.selected += 1;
        }
        self.0.value = Value::Undefined;
        self.0.cursor += 1;
        self.next(runtime)
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: CallbackStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            CallbackStep::Complete(result) => return Ok(result),
            CallbackStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            CallbackStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            CallbackStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                resume.boolean(
                    runtime,
                    runtime.internal_has_property(realm, &object, &key)?,
                )?
            }
            CallbackStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &arguments)?,
                )?
            }
            CallbackStep::Species { mut resume } => {
                let source = resume.take_species_source();
                let length = resume.take_species_length();
                resume.resume(
                    runtime,
                    super::species::finish(
                        runtime,
                        realm,
                        super::species::SpeciesStep::start(runtime, realm, &source, length)?,
                    )?,
                )?
            }
            CallbackStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                resume.defined(
                    runtime,
                    runtime.internal_define_own_property(realm, &object, &key, &descriptor)?,
                )?
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unpublished_species_result_and_mapper_survive_wait_then_release() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let mapper = context.eval("(function(value) { return value; })").unwrap();
        let Value::Object(mapper_object) = &mapper else {
            panic!("expected mapper");
        };
        let source = runtime.new_array(context.realm).unwrap();
        let target = runtime.new_object(None).unwrap();
        let ids = [
            source.object_id(),
            target.object_id(),
            mapper_object.object_id(),
        ];
        let invocation = NativeInvocation::Call {
            this_value: Value::Object(source),
        };
        let arguments = NativeArguments {
            actual_arg_count: 1,
            readable: vec![mapper],
        };
        let CallbackStep::Read { mut resume } = CallbackStep::start(
            &runtime,
            context.realm,
            CallbackKind::Iteration(ArrayIterationKind::Map),
            &invocation,
            &arguments,
        )
        .unwrap() else {
            panic!("expected length read");
        };
        let _ = resume.take_read_object();
        let _ = resume.take_read_key();

        drop(invocation);
        drop(arguments);
        let CallbackStep::Number { mut resume } = resume
            .resume(&runtime, Completion::Return(Value::Int(1)))
            .unwrap()
        else {
            panic!("expected length conversion");
        };
        let _ = resume.take_number_value();

        let CallbackStep::Species { mut resume } = resume
            .number(&runtime, NativeConversion::Value(1.0))
            .unwrap()
        else {
            panic!("expected species");
        };
        let _ = resume.take_species_source();
        let _ = resume.take_species_length();

        let CallbackStep::Has { mut resume } = resume
            .resume(&runtime, Completion::Return(Value::Object(target)))
            .unwrap()
        else {
            panic!("expected indexed lookup");
        };
        let _ = resume.take_has_object();
        let _ = resume.take_has_key();

        runtime.run_gc().unwrap();
        for id in ids {
            assert!(runtime.0.state.borrow().heap.object(id).is_ok());
        }
        drop(resume);
        runtime.run_gc().unwrap();
        for id in ids {
            assert!(runtime.0.state.borrow().heap.object(id).is_err());
        }
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }
}

#[derive(Default)]
struct CallbackStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    has_object: Option<ObjectRef>,
    has_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    species_source: Option<ObjectRef>,
    species_length: Option<u64>,
    define_object: Option<ObjectRef>,
    define_key: Option<PropertyKey>,
    define_descriptor: Option<OrdinaryPropertyDescriptor>,
}
impl CallbackStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: CallbackResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: CallbackResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_has(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: CallbackResume,
    ) -> Self {
        resume.0.pending_effect.has_object = Some(object);
        resume.0.pending_effect.has_key = Some(key);
        Self::Has { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: CallbackResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_species(
        source: ObjectRef,
        length: u64,
        mut resume: CallbackResume,
    ) -> Self {
        resume.0.pending_effect.species_source = Some(source);
        resume.0.pending_effect.species_length = Some(length);
        Self::Species { resume }
    }
    pub(crate) fn request_define(
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
        mut resume: CallbackResume,
    ) -> Self {
        resume.0.pending_effect.define_object = Some(object);
        resume.0.pending_effect.define_key = Some(key);
        resume.0.pending_effect.define_descriptor = Some(descriptor);
        Self::Define { resume }
    }
}
impl CallbackResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("CallbackStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("CallbackStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("CallbackStep Number value")
    }
    pub(crate) fn take_has_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .has_object
            .take()
            .expect("CallbackStep Has object")
    }
    pub(crate) fn take_has_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .has_key
            .take()
            .expect("CallbackStep Has key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("CallbackStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("CallbackStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("CallbackStep Call arguments")
    }
    pub(crate) fn take_species_source(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .species_source
            .take()
            .expect("CallbackStep Species source")
    }
    pub(crate) fn take_species_length(&mut self) -> u64 {
        self.0
            .pending_effect
            .species_length
            .take()
            .expect("CallbackStep Species length")
    }
    pub(crate) fn take_define_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .define_object
            .take()
            .expect("CallbackStep Define object")
    }
    pub(crate) fn take_define_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .define_key
            .take()
            .expect("CallbackStep Define key")
    }
    pub(crate) fn take_define_descriptor(&mut self) -> OrdinaryPropertyDescriptor {
        self.0
            .pending_effect
            .define_descriptor
            .take()
            .expect("CallbackStep Define descriptor")
    }
}
const _: () = assert!(std::mem::size_of::<CallbackStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<CallbackStep>() <= 64);
