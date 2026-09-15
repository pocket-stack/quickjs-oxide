//! Array stringification retains partial output and failure ordering across callbacks.
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::ArrayJoinKind,
    heap::ContextId,
    object::{CallableRef, ObjectRef, PropertyKey},
    value::{JsString, JsStringBuilder, JsStringError, Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum ArrayStringKind {
    Join(ArrayJoinKind),
    ToString,
}
impl ArrayStringKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        match target {
            NativeFunctionId::ArrayPrototypeJoin(kind) => Some(Self::Join(kind)),
            NativeFunctionId::ArrayPrototypeToString => Some(Self::ToString),
            _ => None,
        }
    }
}
pub(crate) enum ArrayStringStep {
    Complete(Completion),
    Read { resume: ArrayStringResume },
    Number { resume: ArrayStringResume },
    String { resume: ArrayStringResume },
    Call { resume: ArrayStringResume },
    ObjectTag { receiver: Value },
}
enum Phase {
    Length,
    Number,
    Separator,
    Element,
    LocaleMethod,
    LocaleResult,
    ElementString,
    JoinMethod,
    JoinResult,
}
pub(crate) struct ArrayStringResume(Box<ArrayStringResumeState>);
impl std::ops::Deref for ArrayStringResume {
    type Target = ArrayStringResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ArrayStringResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ArrayStringResume>() <= 8);
pub(crate) struct ArrayStringResumeState {
    pending_effect: ArrayStringStepPending,
    realm: ContextId,
    kind: ArrayStringKind,
    object: ObjectRef,
    phase: Phase,
    separator_value: Value,
    separator: JsString,
    output: JsStringBuilder,
    separator_error: Option<JsStringError>,
    length: u64,
    index: u64,
    element: Value,
}
impl ArrayStringStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: ArrayStringKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array string method requires generic invocation",
            ));
        };
        let object = match runtime.native_to_object(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let to_string = matches!(kind, ArrayStringKind::ToString);
        Ok({
            let __pending_field_receiver = Value::Object(object.clone());
            let __pending_field_key =
                runtime.intern_property_key(if to_string { "join" } else { "length" })?;
            let __pending_field_resume = ArrayStringResume(Box::new(ArrayStringResumeState {
                pending_effect: ArrayStringStepPending::default(),
                realm,
                kind,
                object,
                phase: if to_string {
                    Phase::JoinMethod
                } else {
                    Phase::Length
                },
                separator_value: arguments
                    .readable
                    .first()
                    .cloned()
                    .unwrap_or(Value::Undefined),
                separator: JsString::from_static(","),
                output: JsStringBuilder::with_limit(0, string_limit),
                separator_error: None,
                length: 0,
                index: 0,
                element: Value::Undefined,
            }));
            Self::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
}
impl ArrayStringResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<ArrayStringStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(ArrayStringStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Length => {
                self.0.phase = Phase::Number;
                Ok({
                    let __pending_field_value = value;
                    let __pending_field_resume = self;
                    ArrayStringStep::request_number(__pending_field_value, __pending_field_resume)
                })
            }
            Phase::Element => {
                if matches!(value, Value::Undefined | Value::Null) {
                    self.0.index += 1;
                    return self.next(runtime);
                }
                if matches!(
                    self.0.kind,
                    ArrayStringKind::Join(ArrayJoinKind::ToLocaleString)
                ) {
                    self.0.element = value.clone();
                    self.0.phase = Phase::LocaleMethod;
                    return Ok({
                        let __pending_field_receiver = value;
                        let __pending_field_key = runtime.intern_property_key("toLocaleString")?;
                        let __pending_field_resume = self;
                        ArrayStringStep::request_read(
                            __pending_field_receiver,
                            __pending_field_key,
                            __pending_field_resume,
                        )
                    });
                }
                self.element_string(value)
            }
            Phase::LocaleMethod => {
                let callable = match value {
                    Value::Object(object) => runtime.as_callable(&object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return Ok(ArrayStringStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "not a function",
                        )?,
                    )));
                };
                self.0.phase = Phase::LocaleResult;
                Ok({
                    let __pending_field_callable = callable;
                    let __pending_field_receiver = self.0.element.clone();
                    let __pending_field_resume = self;
                    ArrayStringStep::request_call(
                        __pending_field_callable,
                        __pending_field_receiver,
                        __pending_field_resume,
                    )
                })
            }
            Phase::LocaleResult => {
                self.0.element = Value::Undefined;
                self.element_string(value)
            }
            Phase::JoinMethod => {
                let callable = match value {
                    Value::Object(object) => runtime.as_callable(&object)?,
                    _ => None,
                };
                if let Some(callable) = callable {
                    self.0.phase = Phase::JoinResult;
                    Ok({
                        let __pending_field_callable = callable;
                        let __pending_field_receiver = Value::Object(self.0.object.clone());
                        let __pending_field_resume = self;
                        ArrayStringStep::request_call(
                            __pending_field_callable,
                            __pending_field_receiver,
                            __pending_field_resume,
                        )
                    })
                } else {
                    Ok(ArrayStringStep::ObjectTag {
                        receiver: Value::Object(self.0.object),
                    })
                }
            }
            Phase::JoinResult => Ok(ArrayStringStep::Complete(Completion::Return(value))),
            _ => Err(RuntimeError::Invariant("Array string value phase mismatch")),
        }
    }
    fn element_string(mut self, value: Value) -> Result<ArrayStringStep, RuntimeError> {
        if let Some(error) = self.0.separator_error {
            return Err(error.into());
        }
        self.0.phase = Phase::ElementString;
        Ok({
            let __pending_field_value = value;
            let __pending_field_resume = self;
            ArrayStringStep::request_string(__pending_field_value, __pending_field_resume)
        })
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<ArrayStringStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Number) {
            return Err(RuntimeError::Invariant(
                "Array string number phase mismatch",
            ));
        }
        self.0.length = match result {
            NativeConversion::Value(value) => Runtime::length_from_number(value),
            NativeConversion::Throw(value) => {
                return Ok(ArrayStringStep::Complete(Completion::Throw(value)));
            }
        };
        if matches!(self.0.kind, ArrayStringKind::Join(ArrayJoinKind::Join))
            && !matches!(self.0.separator_value, Value::Undefined)
        {
            self.0.phase = Phase::Separator;
            let value = std::mem::replace(&mut self.0.separator_value, Value::Undefined);
            return Ok({
                let __pending_field_value = value;
                let __pending_field_resume = self;
                ArrayStringStep::request_string(__pending_field_value, __pending_field_resume)
            });
        }
        self.next(runtime)
    }
    pub(crate) fn string(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<JsString>,
    ) -> Result<ArrayStringStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ArrayStringStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Separator => self.0.separator = value,
            Phase::ElementString => {
                self.0.output.push_js_string(&value)?;
                self.0.index += 1;
            }
            _ => {
                return Err(RuntimeError::Invariant(
                    "Array string conversion phase mismatch",
                ));
            }
        }
        self.next(runtime)
    }
    fn next(mut self, runtime: &Runtime) -> Result<ArrayStringStep, RuntimeError> {
        if self.0.index == self.0.length {
            if let Some(error) = self.0.separator_error {
                return Err(error.into());
            }
            return Ok(ArrayStringStep::Complete(Completion::Return(
                Value::String(self.0.output.finish()?),
            )));
        }
        if self.0.index != 0
            && let Err(error) = self.0.output.push_js_string(&self.0.separator)
        {
            self.0.separator_error.get_or_insert(error);
        }
        self.0.phase = Phase::Element;
        Ok({
            let __pending_field_receiver = Value::Object(self.0.object.clone());
            let __pending_field_key =
                runtime.property_key_for_index(u64::from(self.0.index as u32))?;
            let __pending_field_resume = self;
            ArrayStringStep::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ArrayStringStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            ArrayStringStep::Complete(result) => return Ok(result),
            ArrayStringStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            ArrayStringStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            ArrayStringStep::String { mut resume } => {
                let value = resume.take_string_value();
                resume.string(runtime, runtime.native_to_js_string(realm, &value)?)?
            }
            ArrayStringStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &[])?,
                )?
            }
            ArrayStringStep::ObjectTag { receiver } => {
                return runtime.call_object_prototype_to_string(
                    realm,
                    NativeInvocation::Call {
                        this_value: receiver,
                    },
                );
            }
        };
    }
}

#[derive(Default)]
struct ArrayStringStepPending {
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    string_value: Option<Value>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
}
impl ArrayStringStep {
    pub(crate) fn request_read(
        receiver: Value,
        key: PropertyKey,
        mut resume: ArrayStringResume,
    ) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: ArrayStringResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_string(value: Value, mut resume: ArrayStringResume) -> Self {
        resume.0.pending_effect.string_value = Some(value);
        Self::String { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        mut resume: ArrayStringResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        Self::Call { resume }
    }
}
impl ArrayStringResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("ArrayStringStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ArrayStringStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("ArrayStringStep Number value")
    }
    pub(crate) fn take_string_value(&mut self) -> Value {
        self.0
            .pending_effect
            .string_value
            .take()
            .expect("ArrayStringStep String value")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("ArrayStringStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ArrayStringStep Call receiver")
    }
}
const _: () = assert!(std::mem::size_of::<ArrayStringStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ArrayStringStep>() <= 64);
