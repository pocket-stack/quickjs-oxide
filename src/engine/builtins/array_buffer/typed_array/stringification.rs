//! `%TypedArray%.prototype` stringification algorithms.
//!
//! Pinned QuickJS uses a dedicated TypedArray kernel rather than the generic
//! Array join path. It validates the branded view up front, snapshots the old
//! element count, and then keeps resizable-buffer changes observable without
//! consulting ordinary `length` or indexed properties.

use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::ArrayJoinKind,
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{JsString, JsStringBuilder, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{DirectCallTarget, NativeArguments, NativeInvocation},
    },
};

#[cfg(test)]
mod tests;

impl Runtime {
    pub(crate) fn call_typed_array_join(
        &self,
        realm: ContextId,
        kind: ArrayJoinKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_typed_array_join_with_string_limit(
            realm,
            kind,
            invocation,
            arguments,
            JsString::MAX_LEN,
        )
    }

    fn call_typed_array_join_with_string_limit(
        &self,
        realm: ContextId,
        kind: ArrayJoinKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            TypedStringStep::start_with_limit(
                self,
                realm,
                kind,
                &invocation,
                arguments,
                string_limit,
            )?,
        )
    }
}
pub(crate) enum TypedStringStep {
    Complete(Completion),
    Primitive { resume: TypedStringResume },
    Read { resume: TypedStringResume },
    Call { resume: TypedStringResume },
}
pub(crate) struct TypedStringResume(Box<TypedStringResumeState>);
impl std::ops::Deref for TypedStringResume {
    type Target = TypedStringResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedStringResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedStringResume>() <= 8);
pub(crate) struct TypedStringResumeState {
    pending_effect: TypedStringStepPending,
    realm: ContextId,
    target: ObjectRef,
    kind: ArrayJoinKind,
    initial_length: u64,
    current_length: u64,
    index: u64,
    separator: JsString,
    output: JsStringBuilder,
    phase: Phase,
}
enum Phase {
    Separator,
    LocaleMethod(Value),
    LocaleResult,
    Element,
}
impl TypedStringStep {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: ArrayJoinKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        Self::start_with_limit(
            runtime,
            realm,
            kind,
            invocation,
            arguments,
            JsString::MAX_LEN,
        )
    }
    fn start_with_limit(
        runtime: &Runtime,
        realm: ContextId,
        kind: ArrayJoinKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
        limit: usize,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray stringification received a constructor invocation",
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
        let state = TypedStringResume(Box::new(TypedStringResumeState {
            pending_effect: TypedStringStepPending::default(),
            realm,
            target,
            kind,
            initial_length: length,
            current_length: length,
            index: 0,
            separator: JsString::from_static(","),
            output: JsStringBuilder::with_limit(0, limit),
            phase: Phase::Separator,
        }));
        if matches!(kind, ArrayJoinKind::Join)
            && arguments.actual_arg_count != 0
            && !matches!(arguments.readable.first(), Some(Value::Undefined))
        {
            return Ok(Self::request_primitive(
                arguments
                    .readable
                    .first()
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray.join separator argv was not padded",
                    ))?
                    .clone(),
                state,
            ));
        }
        state.next(runtime)
    }
}
impl TypedStringResume {
    fn next(mut self, runtime: &Runtime) -> Result<TypedStringStep, RuntimeError> {
        while self.0.index < self.0.initial_length.min(self.0.current_length) {
            if self.0.index != 0 {
                self.0.output.push_js_string(&self.0.separator)?;
            }
            let Some(element) = runtime.typed_array_read_index(&self.0.target, self.0.index)?
            else {
                self.0.index += 1;
                continue;
            };
            match self.0.kind {
                ArrayJoinKind::Join => {
                    // Integer-indexed storage returns only primitive numeric values.
                    let string = match runtime.native_to_js_string(self.0.realm, &element)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(TypedStringStep::Complete(Completion::Throw(value)));
                        }
                    };
                    self.0.output.push_js_string(&string)?;
                    self.0.index += 1;
                }
                ArrayJoinKind::ToLocaleString => {
                    let key = runtime.intern_property_key("toLocaleString")?;
                    self.0.phase = Phase::LocaleMethod(element.clone());
                    return Ok(TypedStringStep::request_read(element, key, self));
                }
            }
        }
        for _ in self.0.current_length.max(1)..self.0.initial_length {
            self.0.output.push_js_string(&self.0.separator)?;
        }
        Ok(TypedStringStep::Complete(Completion::Return(
            Value::String(self.0.output.finish()?),
        )))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<TypedStringStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(TypedStringStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Separator => {
                self.0.separator = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(TypedStringStep::Complete(Completion::Throw(value)));
                    }
                };
                self.0.current_length =
                    u64::from(runtime.typed_array_state(&self.0.target)?.length);
                self.next(runtime)
            }
            Phase::LocaleMethod(receiver) => {
                let callable = match value {
                    Value::Object(object) => runtime.as_callable(&object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return Ok(TypedStringStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "not a function",
                        )?,
                    )));
                };
                self.0.phase = Phase::LocaleResult;
                Ok(TypedStringStep::request_call(
                    DirectCallTarget::Callable(callable),
                    receiver,
                    Vec::new(),
                    self,
                ))
            }
            Phase::LocaleResult => {
                self.0.phase = Phase::Element;
                Ok(TypedStringStep::request_primitive(value, self))
            }
            Phase::Element => {
                let string = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(TypedStringStep::Complete(Completion::Throw(value)));
                    }
                };
                self.0.output.push_js_string(&string)?;
                self.0.index += 1;
                self.next(runtime)
            }
        }
    }
}
fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TypedStringStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            TypedStringStep::Complete(result) => return Ok(result),
            TypedStringStep::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                {
                    let result = if matches!(value, Value::Object(_)) {
                        runtime.to_primitive(realm, value, ToPrimitiveHint::String)?
                    } else {
                        Completion::Return(value)
                    };
                    resume.resume(runtime, result)?
                }
            }
            TypedStringStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            TypedStringStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                {
                    let DirectCallTarget::Callable(callable) = target else {
                        return Err(RuntimeError::Invariant(
                            "TypedArray stringification requested invalid call target",
                        ));
                    };
                    resume.resume(
                        runtime,
                        runtime.call_internal(realm, &callable, receiver, &arguments)?,
                    )?
                }
            }
        };
    }
}

#[derive(Default)]
struct TypedStringStepPending {
    primitive_value: Option<Value>,
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
}
impl TypedStringStep {
    pub(crate) fn request_primitive(value: Value, mut resume: TypedStringResume) -> Self {
        resume.0.pending_effect.primitive_value = Some(value);
        Self::Primitive { resume }
    }
    pub(crate) fn request_read(
        receiver: Value,
        key: PropertyKey,
        mut resume: TypedStringResume,
    ) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        target: DirectCallTarget,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: TypedStringResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
}
impl TypedStringResume {
    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .pending_effect
            .primitive_value
            .take()
            .expect("TypedStringStep Primitive value")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("TypedStringStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("TypedStringStep Read key")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("TypedStringStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("TypedStringStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("TypedStringStep Call arguments")
    }
}
const _: () = assert!(std::mem::size_of::<TypedStringStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedStringStep>() <= 64);
