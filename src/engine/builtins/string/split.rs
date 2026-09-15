//! String split retains its fresh result through limit and separator coercion.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, WellKnownSymbol},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{DirectCallTarget, NativeArguments, NativeInvocation},
    },
};
pub(crate) enum StringSplitStep {
    Complete(Completion),
    Read { resume: StringSplitResume },
    Primitive { resume: StringSplitResume },
    Call { resume: StringSplitResume },
}
pub(crate) struct StringSplitResume(Box<StringSplitResumeState>);
impl std::ops::Deref for StringSplitResume {
    type Target = StringSplitResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for StringSplitResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<StringSplitResume>() <= 8);
pub(crate) struct StringSplitResumeState {
    step_pending: StringSplitStepPending,
    realm: ContextId,
    receiver: Value,
    separator: Value,
    limit: Value,
    phase: SplitPhase,
}
enum SplitPhase {
    Method,
    Called,
    Source,
    Limit {
        source: JsString,
        result: ObjectRef,
    },
    Separator {
        source: JsString,
        result: ObjectRef,
        limit: u32,
    },
}
impl StringSplitStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String split did not receive a generic invocation",
            ));
        };
        if matches!(this_value, Value::Undefined | Value::Null) {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "cannot convert to object",
                )?,
            )));
        }
        let separator = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(
                "String split separator argv was not padded",
            ))?
            .clone();
        let limit = arguments
            .readable
            .get(1)
            .ok_or(RuntimeError::Invariant(
                "String split limit argv was not padded",
            ))?
            .clone();
        let resume = StringSplitResume(Box::new(StringSplitResumeState {
            step_pending: StringSplitStepPending::default(),
            realm,
            receiver: this_value.clone(),
            separator,
            limit,
            phase: SplitPhase::Method,
        }));
        if let Value::Object(object) = &resume.separator {
            Ok(Self::make_read(
                object.clone(),
                PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Split)),
                resume,
            ))
        } else {
            Ok(resume.source())
        }
    }
}
impl StringSplitResume {
    fn source(mut self) -> StringSplitStep {
        StringSplitStep::make_primitive(self.0.receiver.clone(), ToPrimitiveHint::String, {
            let updated_0 = SplitPhase::Source;
            self.0.phase = updated_0;
            self
        })
    }
    fn separator(mut self, source: JsString, result: ObjectRef, limit: u32) -> StringSplitStep {
        StringSplitStep::make_primitive(self.0.separator.clone(), ToPrimitiveHint::String, {
            let updated_0 = SplitPhase::Separator {
                source,
                result,
                limit,
            };
            self.0.phase = updated_0;
            self
        })
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<StringSplitStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(StringSplitStep::Complete(Completion::Throw(value)));
            }
        };
        let realm = self.0.realm;
        match self.0.phase {
            SplitPhase::Method => {
                if matches!(value, Value::Undefined | Value::Null) {
                    return Ok(self.source());
                }
                let callable = match value {
                    Value::Object(object) => runtime.as_callable(&object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return Ok(StringSplitStep::Complete(Completion::Throw(
                        runtime.new_native_error(realm, NativeErrorKind::Type, "not a function")?,
                    )));
                };
                let mut arguments = Vec::new();
                if arguments.try_reserve_exact(2).is_err() {
                    return Ok(StringSplitStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            realm,
                            NativeErrorKind::Internal,
                            "out of memory",
                        )?,
                    )));
                }
                arguments.push(self.0.receiver.clone());
                arguments.push(self.0.limit.clone());
                Ok(StringSplitStep::make_call(
                    DirectCallTarget::Callable(callable),
                    self.0.separator.clone(),
                    arguments,
                    {
                        let updated_0 = SplitPhase::Called;
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            SplitPhase::Called => Ok(StringSplitStep::Complete(Completion::Return(value))),
            SplitPhase::Source => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "String split source conversion returned an object",
                    ));
                }
                let source = match runtime.native_to_js_string(realm, &value)? {
                    NativeConversion::Value(value) => value.linearize(),
                    NativeConversion::Throw(value) => {
                        return Ok(StringSplitStep::Complete(Completion::Throw(value)));
                    }
                };
                let result = runtime.new_array(realm)?;
                if matches!(self.0.limit, Value::Undefined) {
                    Ok(self.separator(source, result, u32::MAX))
                } else {
                    Ok(StringSplitStep::make_primitive(
                        self.0.limit.clone(),
                        ToPrimitiveHint::Number,
                        {
                            let updated_0 = SplitPhase::Limit { source, result };
                            self.0.phase = updated_0;
                            self
                        },
                    ))
                }
            }
            SplitPhase::Limit { source, result } => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "String split limit conversion returned an object",
                    ));
                }
                let limit = match runtime.native_to_number(realm, &value)? {
                    NativeConversion::Value(value) => Runtime::to_uint32_number(value),
                    NativeConversion::Throw(value) => {
                        return Ok(StringSplitStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok({
                    let updated_0 = SplitPhase::Called;
                    self.0.phase = updated_0;
                    self
                }
                .separator(source, result, limit))
            }
            SplitPhase::Separator {
                source,
                result,
                limit,
            } => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "String split separator conversion returned an object",
                    ));
                }
                let separator = match runtime.native_to_js_string(realm, &value)? {
                    NativeConversion::Value(value) => value.linearize(),
                    NativeConversion::Throw(value) => {
                        return Ok(StringSplitStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok(StringSplitStep::Complete(runtime.finish_string_split(
                    realm,
                    source,
                    result,
                    &self.0.separator,
                    separator,
                    limit,
                )?))
            }
        }
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: StringSplitStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            StringSplitStep::Complete(result) => return Ok(result),
            StringSplitStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            StringSplitStep::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                let hint = resume.take_primitive_hint();
                {
                    let result = if matches!(value, Value::Object(_)) {
                        runtime.to_primitive(realm, value, hint)?
                    } else {
                        Completion::Return(value)
                    };
                    resume.resume(runtime, result)?
                }
            }
            StringSplitStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                {
                    let DirectCallTarget::Callable(callable) = target else {
                        return Err(RuntimeError::Invariant(
                            "String split requested an invalid call target",
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
pub(crate) struct StringSplitStepPending {
    object: Option<ObjectRef>,
    key: Option<PropertyKey>,
    value: Option<Value>,
    hint: Option<ToPrimitiveHint>,
    target: Option<DirectCallTarget>,
    receiver: Option<Value>,
    arguments: Option<Vec<Value>>,
}
impl StringSplitStep {
    pub(crate) fn make_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: StringSplitResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn make_primitive(
        value: Value,
        hint: ToPrimitiveHint,
        mut resume: StringSplitResume,
    ) -> Self {
        resume.0.step_pending.value = Some(value);
        resume.0.step_pending.hint = Some(hint);
        Self::Primitive { resume }
    }
    pub(crate) fn make_call(
        target: DirectCallTarget,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: StringSplitResume,
    ) -> Self {
        resume.0.step_pending.target = Some(target);
        resume.0.step_pending.receiver = Some(receiver);
        resume.0.step_pending.arguments = Some(arguments);
        Self::Call { resume }
    }
}
impl StringSplitResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("StringSplitStep::Read lost object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("StringSplitStep::Read lost key")
    }

    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("StringSplitStep::Primitive lost value")
    }
    pub(crate) fn take_primitive_hint(&mut self) -> ToPrimitiveHint {
        self.0
            .step_pending
            .hint
            .take()
            .expect("StringSplitStep::Primitive lost hint")
    }

    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .step_pending
            .target
            .take()
            .expect("StringSplitStep::Call lost target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .step_pending
            .receiver
            .take()
            .expect("StringSplitStep::Call lost receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .step_pending
            .arguments
            .take()
            .expect("StringSplitStep::Call lost arguments")
    }
}

const _: () = assert!(std::mem::size_of::<StringSplitStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<StringSplitStep>() <= 64);
