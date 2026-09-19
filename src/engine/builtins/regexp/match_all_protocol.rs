//! MatchAll setup owns its matcher and flags across observable operations.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, operations::InternalSetResult},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{ConstructorRef, NativeArguments, NativeInvocation},
    },
};
pub(crate) enum RegExpMatchAllStep {
    Complete(Completion),
    Primitive { resume: RegExpMatchAllResume },
    Read { resume: RegExpMatchAllResume },
    Species { resume: RegExpMatchAllResume },
    Construct { resume: RegExpMatchAllResume },
    Set { resume: RegExpMatchAllResume },
}
pub(crate) struct RegExpMatchAllResume(Box<RegExpMatchAllResumeState>);
impl std::ops::Deref for RegExpMatchAllResume {
    type Target = RegExpMatchAllResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RegExpMatchAllResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<RegExpMatchAllResume>() <= 8);
pub(crate) struct RegExpMatchAllResumeState {
    step_pending: RegExpMatchAllStepPending,
    realm: ContextId,
    regexp: ObjectRef,
    phase: Phase,
}
enum Phase {
    Input,
    Species(JsString),
    Flags {
        input: JsString,
        constructor: ConstructorRef,
    },
    FlagsPrimitive {
        input: JsString,
        constructor: ConstructorRef,
    },
    Construct {
        input: JsString,
        flags: JsString,
    },
    LastIndex {
        input: JsString,
        flags: JsString,
        matcher: ObjectRef,
    },
    LastIndexPrimitive {
        input: JsString,
        flags: JsString,
        matcher: ObjectRef,
    },
    Set {
        input: JsString,
        flags: JsString,
        matcher: ObjectRef,
    },
}
impl RegExpMatchAllStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@matchAll did not receive a generic invocation",
            ));
        };
        let Value::Object(regexp) = this_value else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(realm, NativeErrorKind::Type, "not an object")?,
            )));
        };
        Ok(Self::make_primitive(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "RegExp @@matchAll input argv was not padded",
                ))?
                .clone(),
            ToPrimitiveHint::String,
            RegExpMatchAllResume(Box::new(RegExpMatchAllResumeState {
                step_pending: RegExpMatchAllStepPending::default(),
                realm,
                regexp: regexp.clone(),
                phase: Phase::Input,
            })),
        ))
    }
}
impl RegExpMatchAllResume {
    pub(crate) fn species(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<ConstructorRef>,
    ) -> Result<RegExpMatchAllStep, RuntimeError> {
        let constructor = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(RegExpMatchAllStep::Complete(Completion::Throw(value)));
            }
        };
        let Phase::Species(input) = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "RegExp matchAll species reply in wrong phase",
            ));
        };
        Ok(RegExpMatchAllStep::make_read(
            self.0.regexp.clone(),
            runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Flags)?,
            {
                let updated_0 = Phase::Flags { input, constructor };
                self.0.phase = updated_0;
                self
            },
        ))
    }
    pub(crate) fn set(
        self,
        runtime: &Runtime,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<RegExpMatchAllStep, RuntimeError> {
        let key =
            runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::LastIndex)?;
        let completion = match runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            Some(value) => Completion::Throw(value),
            None => Completion::Return(Value::Undefined),
        };
        self.resume(runtime, completion)
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<RegExpMatchAllStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(RegExpMatchAllStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Input => {
                let input = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpMatchAllStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok(RegExpMatchAllStep::make_species(self.0.regexp.clone(), {
                    let updated_0 = Phase::Species(input);
                    self.0.phase = updated_0;
                    self
                }))
            }
            Phase::Flags { input, constructor } => Ok(RegExpMatchAllStep::make_primitive(
                value,
                ToPrimitiveHint::String,
                {
                    let updated_0 = Phase::FlagsPrimitive { input, constructor };
                    self.0.phase = updated_0;
                    self
                },
            )),
            Phase::FlagsPrimitive { input, constructor } => {
                let flags = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpMatchAllStep::Complete(Completion::Throw(value)));
                    }
                };
                let mut arguments = Vec::new();
                if arguments.try_reserve_exact(2).is_err() {
                    return Ok(RegExpMatchAllStep::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            self.0.realm,
                            NativeErrorKind::Internal,
                            "out of memory",
                        )?,
                    )));
                }
                arguments.push(Value::Object(self.0.regexp.clone()));
                arguments.push(Value::String(flags.clone()));
                Ok(RegExpMatchAllStep::make_construct(
                    constructor,
                    arguments,
                    {
                        let updated_0 = Phase::Construct { input, flags };
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            Phase::Construct { input, flags } => {
                let Value::Object(matcher) = value else {
                    return Err(RuntimeError::Invariant(
                        "RegExp matchAll species constructor returned a primitive",
                    ));
                };
                Ok(RegExpMatchAllStep::make_read(
                    self.0.regexp.clone(),
                    runtime
                        .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::LastIndex)?,
                    {
                        let updated_0 = Phase::LastIndex {
                            input,
                            flags,
                            matcher,
                        };
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            Phase::LastIndex {
                input,
                flags,
                matcher,
            } => Ok(RegExpMatchAllStep::make_primitive(
                value,
                ToPrimitiveHint::Number,
                {
                    let updated_0 = Phase::LastIndexPrimitive {
                        input,
                        flags,
                        matcher,
                    };
                    self.0.phase = updated_0;
                    self
                },
            )),
            Phase::LastIndexPrimitive {
                input,
                flags,
                matcher,
            } => {
                let length = match runtime.native_to_length(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpMatchAllStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok(RegExpMatchAllStep::make_set(
                    matcher.clone(),
                    runtime
                        .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::LastIndex)?,
                    Value::number(length as f64),
                    {
                        let updated_0 = Phase::Set {
                            input,
                            flags,
                            matcher,
                        };
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            Phase::Set {
                input,
                flags,
                matcher,
            } => {
                let global = flags.utf16_units().any(|unit| unit == u16::from(b'g'));
                let full_unicode = flags
                    .utf16_units()
                    .any(|unit| unit == u16::from(b'u') || unit == u16::from(b'v'));
                Ok(RegExpMatchAllStep::Complete(Completion::Return(
                    Value::Object(runtime.new_regexp_string_iterator(
                        self.0.realm,
                        &matcher,
                        input,
                        global,
                        full_unicode,
                    )?),
                )))
            }
            Phase::Species(_) => Err(RuntimeError::Invariant(
                "RegExp matchAll completion in species phase",
            )),
        }
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: RegExpMatchAllStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            RegExpMatchAllStep::Complete(result) => return Ok(result),
            RegExpMatchAllStep::Primitive { mut resume } => {
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
            RegExpMatchAllStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            RegExpMatchAllStep::Species { mut resume } => {
                let regexp = resume.take_species_regexp();
                resume.species(runtime, runtime.regexp_species_constructor(realm, &regexp)?)?
            }
            RegExpMatchAllStep::Construct { mut resume } => {
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
            RegExpMatchAllStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                resume.set(
                    runtime,
                    runtime.internal_set(
                        realm,
                        &object,
                        &key,
                        value,
                        Value::Object(object.clone()),
                    )?,
                )?
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct RegExpMatchAllStepPending {
    value: Option<Value>,
    hint: Option<ToPrimitiveHint>,
    object: Option<ObjectRef>,
    key: Option<PropertyKey>,
    regexp: Option<ObjectRef>,
    constructor: Option<ConstructorRef>,
    arguments: Option<Vec<Value>>,
}
impl RegExpMatchAllStep {
    pub(crate) fn make_primitive(
        value: Value,
        hint: ToPrimitiveHint,
        mut resume: RegExpMatchAllResume,
    ) -> Self {
        resume.0.step_pending.value = Some(value);
        resume.0.step_pending.hint = Some(hint);
        Self::Primitive { resume }
    }
    pub(crate) fn make_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: RegExpMatchAllResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn make_species(regexp: ObjectRef, mut resume: RegExpMatchAllResume) -> Self {
        resume.0.step_pending.regexp = Some(regexp);
        Self::Species { resume }
    }
    pub(crate) fn make_construct(
        constructor: ConstructorRef,
        arguments: Vec<Value>,
        mut resume: RegExpMatchAllResume,
    ) -> Self {
        resume.0.step_pending.constructor = Some(constructor);
        resume.0.step_pending.arguments = Some(arguments);
        Self::Construct { resume }
    }
    pub(crate) fn make_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: RegExpMatchAllResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        resume.0.step_pending.value = Some(value);
        Self::Set { resume }
    }
}
impl RegExpMatchAllResume {
    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpMatchAllStep::Primitive lost value")
    }
    pub(crate) fn take_primitive_hint(&mut self) -> ToPrimitiveHint {
        self.0
            .step_pending
            .hint
            .take()
            .expect("RegExpMatchAllStep::Primitive lost hint")
    }

    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpMatchAllStep::Read lost object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpMatchAllStep::Read lost key")
    }

    pub(crate) fn take_species_regexp(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .regexp
            .take()
            .expect("RegExpMatchAllStep::Species lost regexp")
    }

    pub(crate) fn take_construct_constructor(&mut self) -> ConstructorRef {
        self.0
            .step_pending
            .constructor
            .take()
            .expect("RegExpMatchAllStep::Construct lost constructor")
    }
    pub(crate) fn take_construct_arguments(&mut self) -> Vec<Value> {
        self.0
            .step_pending
            .arguments
            .take()
            .expect("RegExpMatchAllStep::Construct lost arguments")
    }

    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpMatchAllStep::Set lost object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpMatchAllStep::Set lost key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpMatchAllStep::Set lost value")
    }
}

const _: () = assert!(std::mem::size_of::<RegExpMatchAllStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<RegExpMatchAllStep>() <= 64);
