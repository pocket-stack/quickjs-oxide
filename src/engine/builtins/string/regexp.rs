//! RegExp-backed String prototype methods.

use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, WellKnownSymbol},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{ConstructorRef, DirectCallTarget, NativeArguments, NativeInvocation},
    },
};

#[derive(Clone, Copy)]
pub(crate) enum StringProtocolKind {
    Match,
    MatchAll,
    Search,
}

impl StringProtocolKind {
    const fn symbol(self) -> WellKnownSymbol {
        match self {
            Self::Match => WellKnownSymbol::Match,
            Self::MatchAll => WellKnownSymbol::MatchAll,
            Self::Search => WellKnownSymbol::Search,
        }
    }

    const fn invocation_invariant(self) -> &'static str {
        match self {
            Self::Match => "String match did not receive a generic-magic invocation",
            Self::MatchAll => "String matchAll did not receive a generic-magic invocation",
            Self::Search => "String search did not receive a generic-magic invocation",
        }
    }

    const fn argument_invariant(self) -> &'static str {
        match self {
            Self::Match => "String match pattern argv was not padded",
            Self::MatchAll => "String matchAll pattern argv was not padded",
            Self::Search => "String search pattern argv was not padded",
        }
    }
}

impl Runtime {
    pub(crate) fn call_string_prototype_match(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_regexp_protocol(realm, invocation, arguments, StringProtocolKind::Match)
    }

    pub(crate) fn call_string_prototype_search(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_regexp_protocol(realm, invocation, arguments, StringProtocolKind::Search)
    }

    pub(crate) fn call_string_prototype_match_all(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_regexp_protocol(realm, invocation, arguments, StringProtocolKind::MatchAll)
    }

    /// Rust port of the `Symbol.match`, `Symbol.matchAll`, and `Symbol.search`
    /// branches in pinned
    /// QuickJS `js_string_match`.
    ///
    /// Object patterns may intercept the operation before receiver coercion.
    /// The fallback converts the receiver, constructs with the defining
    /// realm's retained intrinsic RegExp constructor, and dynamically invokes
    /// the selected protocol on that fresh value so prototype mutations remain
    /// observable.
    fn call_string_regexp_protocol(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        protocol: StringProtocolKind,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            StringProtocolStep::start(self, realm, protocol, &invocation, arguments)?,
        )
    }
}

impl StringProtocolKind {
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::StringPrototypeMatch => Self::Match,
            NativeFunctionId::StringPrototypeMatchAll => Self::MatchAll,
            NativeFunctionId::StringPrototypeSearch => Self::Search,
            _ => return None,
        })
    }
}
pub(crate) enum StringProtocolStep {
    Complete(Completion),
    Read { resume: StringProtocolResume },
    Primitive { resume: StringProtocolResume },
    Call { resume: StringProtocolResume },
    Construct { resume: StringProtocolResume },
}
pub(crate) struct StringProtocolResume(Box<StringProtocolResumeState>);
impl std::ops::Deref for StringProtocolResume {
    type Target = StringProtocolResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for StringProtocolResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<StringProtocolResume>() <= 8);
pub(crate) struct StringProtocolResumeState {
    step_pending: StringProtocolStepPending,
    realm: ContextId,
    kind: StringProtocolKind,
    receiver: Value,
    pattern: Value,
    phase: ProtocolPhase,
}
enum ProtocolPhase {
    Method,
    Match(Value),
    Flags(Value),
    FlagsString(Value),
    Source,
    Constructed(JsString),
    ConstructMethod { regexp: ObjectRef, source: JsString },
    Called,
}
impl StringProtocolStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: StringProtocolKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(kind.invocation_invariant()));
        };
        if matches!(this_value, Value::Null | Value::Undefined) {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(
                    realm,
                    NativeErrorKind::Type,
                    "cannot convert to object",
                )?,
            )));
        }
        let pattern = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(kind.argument_invariant()))?
            .clone();
        let key = PropertyKey::from(runtime.well_known_symbol(kind.symbol()));
        let resume = StringProtocolResume(Box::new(StringProtocolResumeState {
            step_pending: StringProtocolStepPending::default(),
            realm,
            kind,
            receiver: this_value.clone(),
            pattern,
            phase: ProtocolPhase::Method,
        }));
        if let Value::Object(object) = &resume.pattern {
            Ok(Self::make_read(object.clone(), key, resume))
        } else {
            Ok(resume.source())
        }
    }
}
impl StringProtocolResume {
    fn source(mut self) -> StringProtocolStep {
        StringProtocolStep::make_primitive(self.0.receiver.clone(), {
            let updated_0 = ProtocolPhase::Source;
            self.0.phase = updated_0;
            self
        })
    }
    fn selected(
        self,
        runtime: &Runtime,
        method: Value,
    ) -> Result<StringProtocolStep, RuntimeError> {
        if matches!(method, Value::Undefined | Value::Null) {
            return Ok(self.source());
        }
        let receiver = self.0.pattern.clone();
        let argument = self.0.receiver.clone();
        self.call(runtime, receiver, method, argument)
    }
    fn call(
        mut self,
        runtime: &Runtime,
        receiver: Value,
        method: Value,
        argument: Value,
    ) -> Result<StringProtocolStep, RuntimeError> {
        let callable = match method {
            Value::Object(object) => runtime.as_callable(&object)?,
            _ => None,
        };
        let Some(callable) = callable else {
            return Ok(StringProtocolStep::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(self.0.realm, NativeErrorKind::Type, "not a function")?,
            )));
        };
        let mut arguments = Vec::new();
        if arguments.try_reserve_exact(1).is_err() {
            return protocol_oom(runtime, self.0.realm);
        }
        arguments.push(argument);
        Ok(StringProtocolStep::make_call(
            DirectCallTarget::Callable(callable),
            receiver,
            arguments,
            {
                let updated_0 = ProtocolPhase::Called;
                self.0.phase = updated_0;
                self
            },
        ))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<StringProtocolStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(StringProtocolStep::Complete(Completion::Throw(value)));
            }
        };
        let realm = self.0.realm;
        match self.0.phase {
            ProtocolPhase::Method => {
                if matches!(self.0.kind, StringProtocolKind::MatchAll) {
                    let Value::Object(object) = &self.0.pattern else {
                        return Err(RuntimeError::Invariant(
                            "String matchAll check lost its pattern object",
                        ));
                    };
                    Ok(StringProtocolStep::make_read(
                        object.clone(),
                        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Match)),
                        {
                            let updated_0 = ProtocolPhase::Match(value);
                            self.0.phase = updated_0;
                            self
                        },
                    ))
                } else {
                    self.selected(runtime, value)
                }
            }
            ProtocolPhase::Match(method) => {
                let Value::Object(object) = &self.0.pattern else {
                    return Err(RuntimeError::Invariant(
                        "String matchAll check lost its pattern object",
                    ));
                };
                let regexp = runtime.is_regexp_from_match(object, &value)?;
                if regexp {
                    Ok(StringProtocolStep::make_read(
                        object.clone(),
                        runtime
                            .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Flags)?,
                        {
                            let updated_0 = ProtocolPhase::Flags(method);
                            self.0.phase = updated_0;
                            self
                        },
                    ))
                } else {
                    {
                        let updated_0 = ProtocolPhase::Called;
                        self.0.phase = updated_0;
                        self
                    }
                    .selected(runtime, method)
                }
            }
            ProtocolPhase::Flags(method) => {
                if matches!(value, Value::Undefined | Value::Null) {
                    return Ok(StringProtocolStep::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            realm,
                            NativeErrorKind::Type,
                            "cannot convert to object",
                        )?,
                    )));
                }
                Ok(StringProtocolStep::make_primitive(value, {
                    let updated_0 = ProtocolPhase::FlagsString(method);
                    self.0.phase = updated_0;
                    self
                }))
            }
            ProtocolPhase::FlagsString(method) => {
                let flags = match protocol_string(runtime, realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringProtocolStep::Complete(Completion::Throw(value)));
                    }
                };
                if !flags.utf16_units().any(|unit| unit == u16::from(b'g')) {
                    return Ok(StringProtocolStep::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            realm,
                            NativeErrorKind::Type,
                            "regexp must have the 'g' flag",
                        )?,
                    )));
                }
                {
                    let updated_0 = ProtocolPhase::Called;
                    self.0.phase = updated_0;
                    self
                }
                .selected(runtime, method)
            }
            ProtocolPhase::Source => {
                let source = match protocol_string(runtime, realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringProtocolStep::Complete(Completion::Throw(value)));
                    }
                };
                let constructor = ObjectRef::from_borrowed_handle(
                    runtime.clone(),
                    runtime.regexp_realm_data(realm)?.constructor,
                )?;
                runtime
                    .as_callable(&constructor)?
                    .ok_or(RuntimeError::Invariant(
                        "realm RegExp constructor root was not callable",
                    ))?;
                let mut arguments = Vec::new();
                let all = matches!(self.0.kind, StringProtocolKind::MatchAll);
                if arguments
                    .try_reserve_exact(if all { 2 } else { 1 })
                    .is_err()
                {
                    return protocol_oom(runtime, realm);
                }
                arguments.push(self.0.pattern.clone());
                if all {
                    arguments.push(Value::String(JsString::from_static("g")));
                }
                Ok(StringProtocolStep::make_construct(
                    ConstructorRef::from_validated_object(constructor),
                    arguments,
                    {
                        let updated_0 = ProtocolPhase::Constructed(source);
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            ProtocolPhase::Constructed(source) => {
                let Value::Object(regexp) = value else {
                    return Err(RuntimeError::Invariant(
                        "intrinsic RegExp constructor returned a primitive",
                    ));
                };
                Ok(StringProtocolStep::make_read(
                    regexp.clone(),
                    PropertyKey::from(runtime.well_known_symbol(self.0.kind.symbol())),
                    {
                        let updated_0 = ProtocolPhase::ConstructMethod { regexp, source };
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            ProtocolPhase::ConstructMethod { regexp, source } => {
                let updated_0 = ProtocolPhase::Called;
                self.0.phase = updated_0;
                self
            }
            .call(runtime, Value::Object(regexp), value, Value::String(source)),
            ProtocolPhase::Called => Ok(StringProtocolStep::Complete(Completion::Return(value))),
        }
    }
}
fn protocol_string(
    runtime: &Runtime,
    realm: ContextId,
    value: Value,
) -> Result<NativeConversion<JsString>, RuntimeError> {
    if matches!(value, Value::Object(_)) {
        return Err(RuntimeError::Invariant(
            "String protocol conversion returned an object",
        ));
    }
    runtime.native_to_js_string(realm, &value)
}
fn protocol_oom(runtime: &Runtime, realm: ContextId) -> Result<StringProtocolStep, RuntimeError> {
    Ok(StringProtocolStep::Complete(Completion::Throw(
        runtime.new_native_error_jsvalue(realm, NativeErrorKind::Internal, "out of memory")?,
    )))
}
fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: StringProtocolStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            StringProtocolStep::Complete(result) => return Ok(result),
            StringProtocolStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            StringProtocolStep::Primitive { mut resume } => {
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
            StringProtocolStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                {
                    let DirectCallTarget::Callable(callable) = target else {
                        return Err(RuntimeError::Invariant(
                            "String protocol requested an invalid call target",
                        ));
                    };
                    resume.resume(
                        runtime,
                        runtime.call_internal(realm, &callable, receiver, &arguments)?,
                    )?
                }
            }
            StringProtocolStep::Construct { mut resume } => {
                let constructor = resume.take_construct_constructor();
                let arguments = resume.take_construct_arguments();
                resume.resume(
                    runtime,
                    runtime.construct_constructor_internal(
                        realm,
                        &constructor,
                        &constructor,
                        &arguments,
                    )?,
                )?
            }
        };
    }
}

#[derive(Default)]
pub(crate) struct StringProtocolStepPending {
    object: Option<ObjectRef>,
    key: Option<PropertyKey>,
    value: Option<Value>,
    target: Option<DirectCallTarget>,
    receiver: Option<Value>,
    arguments: Option<Vec<Value>>,
    constructor: Option<ConstructorRef>,
}
impl StringProtocolStep {
    pub(crate) fn make_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: StringProtocolResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn make_primitive(value: Value, mut resume: StringProtocolResume) -> Self {
        resume.0.step_pending.value = Some(value);
        Self::Primitive { resume }
    }
    pub(crate) fn make_call(
        target: DirectCallTarget,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: StringProtocolResume,
    ) -> Self {
        resume.0.step_pending.target = Some(target);
        resume.0.step_pending.receiver = Some(receiver);
        resume.0.step_pending.arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn make_construct(
        constructor: ConstructorRef,
        arguments: Vec<Value>,
        mut resume: StringProtocolResume,
    ) -> Self {
        resume.0.step_pending.constructor = Some(constructor);
        resume.0.step_pending.arguments = Some(arguments);
        Self::Construct { resume }
    }
}
impl StringProtocolResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("StringProtocolStep::Read lost object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("StringProtocolStep::Read lost key")
    }

    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("StringProtocolStep::Primitive lost value")
    }

    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .step_pending
            .target
            .take()
            .expect("StringProtocolStep::Call lost target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .step_pending
            .receiver
            .take()
            .expect("StringProtocolStep::Call lost receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .step_pending
            .arguments
            .take()
            .expect("StringProtocolStep::Call lost arguments")
    }

    pub(crate) fn take_construct_constructor(&mut self) -> ConstructorRef {
        self.0
            .step_pending
            .constructor
            .take()
            .expect("StringProtocolStep::Construct lost constructor")
    }
    pub(crate) fn take_construct_arguments(&mut self) -> Vec<Value> {
        self.0
            .step_pending
            .arguments
            .take()
            .expect("StringProtocolStep::Construct lost arguments")
    }
}

const _: () = assert!(std::mem::size_of::<StringProtocolStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<StringProtocolStep>() <= 64);
