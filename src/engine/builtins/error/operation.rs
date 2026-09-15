//! Error construction and stringification retain intermediate values in observable order.
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::ErrorConstructorKind,
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum ErrorKind {
    Constructor(ErrorConstructorKind),
    ToString,
}
impl ErrorKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        match target {
            NativeFunctionId::ErrorConstructor(kind) => Some(Self::Constructor(kind)),
            NativeFunctionId::ErrorPrototypeToString => Some(Self::ToString),
            _ => None,
        }
    }
}
pub(crate) enum ErrorStep {
    Complete(Completion),
    Read { resume: ErrorResume },
    String { resume: ErrorResume },
    Has { resume: ErrorResume },
    Aggregate { resume: ErrorResume },
}
enum Phase {
    Prototype,
    Message,
    CauseHas,
    Cause,
    Aggregate,
    NameRead,
    Name,
    TextRead,
    Text,
}
pub(crate) struct ErrorResume(Box<ErrorResumeState>);
impl std::ops::Deref for ErrorResume {
    type Target = ErrorResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ErrorResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ErrorResume>() <= 8);
pub(crate) struct ErrorResumeState {
    pending_effect: ErrorStepPending,
    realm: ContextId,
    kind: ErrorKind,
    phase: Phase,
    object: Option<ObjectRef>,
    new_target: Value,
    arguments: Vec<Value>,
    actual: usize,
    name: JsString,
}
impl ErrorStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: ErrorKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let mut resume = ErrorResume(Box::new(ErrorResumeState {
            pending_effect: ErrorStepPending::default(),
            realm,
            kind,
            phase: Phase::Prototype,
            object: None,
            new_target: Value::Undefined,
            arguments: arguments.readable.clone(),
            actual: arguments.actual_arg_count,
            name: JsString::from_static("Error"),
        }));
        match kind {
            ErrorKind::Constructor(_) => {
                let NativeInvocation::Construct { new_target } = invocation else {
                    return Err(RuntimeError::Invariant(
                        "Error constructor requires constructor-or-function invocation",
                    ));
                };
                resume.new_target = if matches!(new_target, Value::Undefined) {
                    Value::Object(runtime.active_function()?)
                } else {
                    new_target.clone()
                };
                Ok({
                    let __pending_field_receiver = resume.new_target.clone();
                    let __pending_field_key = runtime.intern_property_key("prototype")?;
                    let __pending_field_resume = resume;
                    Self::request_read(
                        __pending_field_receiver,
                        __pending_field_key,
                        __pending_field_resume,
                    )
                })
            }
            ErrorKind::ToString => {
                let NativeInvocation::Call { this_value } = invocation else {
                    return Err(RuntimeError::Invariant(
                        "Error string requires generic invocation",
                    ));
                };
                let Value::Object(object) = this_value else {
                    return Ok(Self::Complete(Completion::Throw(
                        runtime.new_native_error(realm, NativeErrorKind::Type, "not an object")?,
                    )));
                };
                resume.object = Some(object.clone());
                resume.phase = Phase::NameRead;
                Ok({
                    let __pending_field_receiver = this_value.clone();
                    let __pending_field_key = runtime.intern_property_key("name")?;
                    let __pending_field_resume = resume;
                    Self::request_read(
                        __pending_field_receiver,
                        __pending_field_key,
                        __pending_field_resume,
                    )
                })
            }
        }
    }
}
impl ErrorResume {
    fn object(&self) -> Result<ObjectRef, RuntimeError> {
        self.0
            .object
            .clone()
            .ok_or(RuntimeError::Invariant("Error operation object missing"))
    }
    fn aggregate_kind(&self) -> bool {
        matches!(
            self.0.kind,
            ErrorKind::Constructor(ErrorConstructorKind::Native(NativeErrorKind::Aggregate))
        )
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<ErrorStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(ErrorStep::Complete(Completion::Throw(value))),
        };
        match self.0.phase {
            Phase::Prototype => {
                let ErrorKind::Constructor(kind) = self.0.kind else {
                    return Err(RuntimeError::Invariant("Error prototype kind mismatch"));
                };
                let prototype = if let Value::Object(object) = value {
                    object
                } else {
                    let realm = match runtime
                        .function_realm_from_value(self.0.realm, &self.0.new_target)?
                    {
                        NativeConversion::Value(realm) => realm,
                        NativeConversion::Throw(value) => {
                            return Ok(ErrorStep::Complete(Completion::Throw(value)));
                        }
                    };
                    let prototype = {
                        let state = runtime.0.state.borrow();
                        let context = state.heap.context(realm)?;
                        match kind {
                            ErrorConstructorKind::Error => context
                                .error_prototype
                                .ok_or(RuntimeError::Invariant("realm has no Error prototype"))?,
                            ErrorConstructorKind::Native(kind) => {
                                context.native_error_prototypes[kind.index()].ok_or(
                                    RuntimeError::Invariant("realm has no native Error prototype"),
                                )?
                            }
                        }
                    };
                    ObjectRef::from_borrowed_handle(runtime.clone(), prototype)?
                };
                self.0.object = Some(runtime.new_error_object(&prototype)?);
                let message = self
                    .0
                    .arguments
                    .get(usize::from(self.aggregate_kind()))
                    .cloned()
                    .ok_or(RuntimeError::Invariant("Error message argv missing"))?;
                if matches!(message, Value::Undefined) {
                    self.cause(runtime)
                } else {
                    self.0.phase = Phase::Message;
                    Ok({
                        let __pending_field_value = message;
                        let __pending_field_resume = self;
                        ErrorStep::request_string(__pending_field_value, __pending_field_resume)
                    })
                }
            }
            Phase::Cause => {
                runtime.define_function_data_property(
                    &self.object()?,
                    "cause",
                    value,
                    true,
                    true,
                )?;
                self.aggregate(runtime)
            }
            Phase::Aggregate => {
                if !matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "AggregateError iterable returned non-object",
                    ));
                }
                runtime.define_function_data_property(
                    &self.object()?,
                    "errors",
                    value,
                    true,
                    true,
                )?;
                self.finish(runtime)
            }
            Phase::NameRead => {
                if matches!(value, Value::Undefined) {
                    self.text(runtime)
                } else {
                    self.0.phase = Phase::Name;
                    Ok({
                        let __pending_field_value = value;
                        let __pending_field_resume = self;
                        ErrorStep::request_string(__pending_field_value, __pending_field_resume)
                    })
                }
            }
            Phase::TextRead => {
                self.0.phase = Phase::Text;
                if matches!(value, Value::Undefined) {
                    self.string(runtime, NativeConversion::Value(JsString::from_static("")))
                } else {
                    Ok({
                        let __pending_field_value = value;
                        let __pending_field_resume = self;
                        ErrorStep::request_string(__pending_field_value, __pending_field_resume)
                    })
                }
            }
            _ => Err(RuntimeError::Invariant("Error value reply phase mismatch")),
        }
    }
    pub(crate) fn string(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<JsString>,
    ) -> Result<ErrorStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ErrorStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Message => {
                runtime.define_function_data_property(
                    &self.object()?,
                    "message",
                    Value::String(value),
                    true,
                    true,
                )?;
                self.cause(runtime)
            }
            Phase::Name => {
                self.0.name = value;
                self.text(runtime)
            }
            Phase::Text => {
                let value = if self.0.name.is_empty() {
                    value
                } else if value.is_empty() {
                    self.0.name
                } else {
                    self.0
                        .name
                        .try_concat(&JsString::from_static(": "))?
                        .try_concat(&value)?
                };
                Ok(ErrorStep::Complete(Completion::Return(Value::String(
                    value,
                ))))
            }
            _ => Err(RuntimeError::Invariant("Error string reply phase mismatch")),
        }
    }
    fn text(mut self, runtime: &Runtime) -> Result<ErrorStep, RuntimeError> {
        self.0.phase = Phase::TextRead;
        Ok({
            let __pending_field_receiver = Value::Object(self.object()?);
            let __pending_field_key = runtime.intern_property_key("message")?;
            let __pending_field_resume = self;
            ErrorStep::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
    fn cause(mut self, runtime: &Runtime) -> Result<ErrorStep, RuntimeError> {
        let index = usize::from(self.aggregate_kind()) + 1;
        if self.0.actual > index
            && let Some(Value::Object(options)) = self.0.arguments.get(index)
        {
            let object = options.clone();
            self.0.phase = Phase::CauseHas;
            return Ok({
                let __pending_field_object = object;
                let __pending_field_key = runtime.intern_property_key("cause")?;
                let __pending_field_resume = self;
                ErrorStep::request_has(
                    __pending_field_object,
                    __pending_field_key,
                    __pending_field_resume,
                )
            });
        }
        self.aggregate(runtime)
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<ErrorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::CauseHas) {
            return Err(RuntimeError::Invariant(
                "Error cause boolean phase mismatch",
            ));
        }
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ErrorStep::Complete(Completion::Throw(value)));
            }
        };
        if !value {
            return self.aggregate(runtime);
        }
        let receiver = self.0.arguments[usize::from(self.aggregate_kind()) + 1].clone();
        self.0.phase = Phase::Cause;
        Ok({
            let __pending_field_receiver = receiver;
            let __pending_field_key = runtime.intern_property_key("cause")?;
            let __pending_field_resume = self;
            ErrorStep::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
    fn aggregate(mut self, runtime: &Runtime) -> Result<ErrorStep, RuntimeError> {
        if self.aggregate_kind() {
            self.0.phase = Phase::Aggregate;
            Ok({
                let __pending_field_iterable =
                    self.0
                        .arguments
                        .first()
                        .cloned()
                        .ok_or(RuntimeError::Invariant(
                            "AggregateError errors argv missing",
                        ))?;
                let __pending_field_resume = self;
                ErrorStep::request_aggregate(__pending_field_iterable, __pending_field_resume)
            })
        } else {
            self.finish(runtime)
        }
    }
    fn finish(self, runtime: &Runtime) -> Result<ErrorStep, RuntimeError> {
        let value = Value::Object(self.object()?);
        runtime.ensure_error_backtrace(&value, true, None)?;
        Ok(ErrorStep::Complete(Completion::Return(value)))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ErrorStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            ErrorStep::Complete(result) => return Ok(result),
            ErrorStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            ErrorStep::String { mut resume } => {
                let value = resume.take_string_value();
                resume.string(runtime, runtime.native_to_js_string(realm, &value)?)?
            }
            ErrorStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                resume.boolean(
                    runtime,
                    runtime.internal_has_property(realm, &object, &key)?,
                )?
            }
            ErrorStep::Aggregate { mut resume } => {
                let iterable = resume.take_aggregate_iterable();
                resume.resume(
                    runtime,
                    super::aggregate::finish(
                        runtime,
                        realm,
                        super::aggregate::AggregateStep::start(runtime, realm, iterable)?,
                    )?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct ErrorStepPending {
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    string_value: Option<Value>,
    has_object: Option<ObjectRef>,
    has_key: Option<PropertyKey>,
    aggregate_iterable: Option<Value>,
}
impl ErrorStep {
    pub(crate) fn request_read(receiver: Value, key: PropertyKey, mut resume: ErrorResume) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_string(value: Value, mut resume: ErrorResume) -> Self {
        resume.0.pending_effect.string_value = Some(value);
        Self::String { resume }
    }
    pub(crate) fn request_has(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ErrorResume,
    ) -> Self {
        resume.0.pending_effect.has_object = Some(object);
        resume.0.pending_effect.has_key = Some(key);
        Self::Has { resume }
    }
    pub(crate) fn request_aggregate(iterable: Value, mut resume: ErrorResume) -> Self {
        resume.0.pending_effect.aggregate_iterable = Some(iterable);
        Self::Aggregate { resume }
    }
}
impl ErrorResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("ErrorStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ErrorStep Read key")
    }
    pub(crate) fn take_string_value(&mut self) -> Value {
        self.0
            .pending_effect
            .string_value
            .take()
            .expect("ErrorStep String value")
    }
    pub(crate) fn take_has_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .has_object
            .take()
            .expect("ErrorStep Has object")
    }
    pub(crate) fn take_has_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .has_key
            .take()
            .expect("ErrorStep Has key")
    }
    pub(crate) fn take_aggregate_iterable(&mut self) -> Value {
        self.0
            .pending_effect
            .aggregate_iterable
            .take()
            .expect("ErrorStep Aggregate iterable")
    }
}
const _: () = assert!(std::mem::size_of::<ErrorStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ErrorStep>() <= 64);
