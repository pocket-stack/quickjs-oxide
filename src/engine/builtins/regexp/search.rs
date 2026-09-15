//! RegExp search restores lastIndex only after successful execution and reread.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, operations::InternalSetResult},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum RegExpSearchStep {
    Complete(Completion),
    Primitive { resume: RegExpSearchResume },
    Read { resume: RegExpSearchResume },
    Set { resume: RegExpSearchResume },
    Exec { resume: RegExpSearchResume },
}
pub(crate) struct RegExpSearchResume(Box<RegExpSearchResumeState>);
impl std::ops::Deref for RegExpSearchResume {
    type Target = RegExpSearchResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RegExpSearchResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<RegExpSearchResume>() <= 8);
pub(crate) struct RegExpSearchResumeState {
    step_pending: RegExpSearchStepPending,
    realm: ContextId,
    regexp: ObjectRef,
    phase: SearchPhase,
}
enum SearchPhase {
    Input,
    Previous(JsString),
    InitialSet { input: JsString, previous: Value },
    Exec(Value),
    Current { previous: Value, result: Value },
    Restored(Value),
    Index,
}
impl RegExpSearchStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@search did not receive a generic invocation",
            ));
        };
        let Value::Object(regexp) = this_value else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(realm, NativeErrorKind::Type, "not an object")?,
            )));
        };
        Ok(Self::make_primitive(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "RegExp @@search input argv was not padded",
                ))?
                .clone(),
            RegExpSearchResume(Box::new(RegExpSearchResumeState {
                step_pending: RegExpSearchStepPending::default(),
                realm,
                regexp: regexp.clone(),
                phase: SearchPhase::Input,
            })),
        ))
    }
}
impl RegExpSearchResume {
    fn execute(mut self, input: JsString, previous: Value) -> RegExpSearchStep {
        RegExpSearchStep::make_exec(
            Value::Object(self.0.regexp.clone()),
            Value::String(input),
            {
                let updated_0 = SearchPhase::Exec(previous);
                self.0.phase = updated_0;
                self
            },
        )
    }
    fn result(
        mut self,
        runtime: &Runtime,
        result: Value,
    ) -> Result<RegExpSearchStep, RuntimeError> {
        match result {
            Value::Null => Ok(RegExpSearchStep::Complete(Completion::Return(Value::Int(
                -1,
            )))),
            Value::Object(result) => Ok(RegExpSearchStep::make_read(
                result,
                runtime.intern_property_key("index")?,
                {
                    let updated_0 = SearchPhase::Index;
                    self.0.phase = updated_0;
                    self
                },
            )),
            _ => Err(RuntimeError::Invariant(
                "RegExpExec returned neither an object nor null",
            )),
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<RegExpSearchStep, RuntimeError> {
        // Both RegExpExec and the post-exec Get throw without restoration.
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(RegExpSearchStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            SearchPhase::Input => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "RegExp search input conversion returned an object",
                    ));
                }
                let input = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpSearchStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok(RegExpSearchStep::make_read(
                    self.0.regexp.clone(),
                    runtime.intern_property_key("lastIndex")?,
                    {
                        let updated_0 = SearchPhase::Previous(input);
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            SearchPhase::Previous(input) => {
                if value.same_value(&Value::Int(0)) {
                    Ok({
                        let updated_0 = SearchPhase::Index;
                        self.0.phase = updated_0;
                        self
                    }
                    .execute(input, value))
                } else {
                    Ok(RegExpSearchStep::make_set(
                        self.0.regexp.clone(),
                        runtime.intern_property_key("lastIndex")?,
                        Value::Int(0),
                        {
                            let updated_0 = SearchPhase::InitialSet {
                                input,
                                previous: value,
                            };
                            self.0.phase = updated_0;
                            self
                        },
                    ))
                }
            }
            SearchPhase::Exec(previous) => Ok(RegExpSearchStep::make_read(
                self.0.regexp.clone(),
                runtime.intern_property_key("lastIndex")?,
                {
                    let updated_0 = SearchPhase::Current {
                        previous,
                        result: value,
                    };
                    self.0.phase = updated_0;
                    self
                },
            )),
            SearchPhase::Current { previous, result } => {
                if value.same_value(&previous) {
                    {
                        let updated_0 = SearchPhase::Index;
                        self.0.phase = updated_0;
                        self
                    }
                    .result(runtime, result)
                } else {
                    Ok(RegExpSearchStep::make_set(
                        self.0.regexp.clone(),
                        runtime.intern_property_key("lastIndex")?,
                        previous,
                        {
                            let updated_0 = SearchPhase::Restored(result);
                            self.0.phase = updated_0;
                            self
                        },
                    ))
                }
            }
            SearchPhase::Index => Ok(RegExpSearchStep::Complete(Completion::Return(value))),
            _ => Err(RuntimeError::Invariant(
                "RegExp search Set received an untyped reply",
            )),
        }
    }
    pub(crate) fn set(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<RegExpSearchStep, RuntimeError> {
        let key = runtime.intern_property_key("lastIndex")?;
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            return Ok(RegExpSearchStep::Complete(Completion::Throw(value)));
        }
        match self.0.phase {
            SearchPhase::InitialSet { input, previous } => Ok({
                let updated_0 = SearchPhase::Index;
                self.0.phase = updated_0;
                self
            }
            .execute(input, previous)),
            SearchPhase::Restored(result) => {
                let updated_0 = SearchPhase::Index;
                self.0.phase = updated_0;
                self
            }
            .result(runtime, result),
            _ => Err(RuntimeError::Invariant(
                "RegExp search received an unexpected Set reply",
            )),
        }
    }
}
impl Runtime {
    pub(crate) fn call_regexp_symbol_search(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let mut step = RegExpSearchStep::start(self, realm, &invocation, arguments)?;
        loop {
            step = match step {
                RegExpSearchStep::Complete(result) => return Ok(result),
                RegExpSearchStep::Primitive { mut resume } => {
                    let value = resume.take_primitive_value();
                    {
                        let result = if matches!(value, Value::Object(_)) {
                            self.to_primitive(realm, value, ToPrimitiveHint::String)?
                        } else {
                            Completion::Return(value)
                        };
                        resume.resume(self, result)?
                    }
                }
                RegExpSearchStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    resume.resume(self, self.get_property_in_realm(realm, &object, &key)?)?
                }
                RegExpSearchStep::Exec { mut resume } => {
                    let regexp = resume.take_exec_regexp();
                    let input = resume.take_exec_input();
                    resume.resume(self, self.regexp_exec_abstract(realm, regexp, input)?)?
                }
                RegExpSearchStep::Set { mut resume } => {
                    let object = resume.take_set_object();
                    let key = resume.take_set_key();
                    let value = resume.take_set_value();
                    resume.set(
                        self,
                        self.internal_set(
                            realm,
                            &object,
                            &key,
                            value,
                            Value::Object(object.clone()),
                        )?,
                    )?
                }
            };
        }
    }
}

#[derive(Default)]
pub(crate) struct RegExpSearchStepPending {
    value: Option<Value>,
    object: Option<ObjectRef>,
    key: Option<PropertyKey>,
    regexp: Option<Value>,
    input: Option<Value>,
}
impl RegExpSearchStep {
    pub(crate) fn make_primitive(value: Value, mut resume: RegExpSearchResume) -> Self {
        resume.0.step_pending.value = Some(value);
        Self::Primitive { resume }
    }
    pub(crate) fn make_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: RegExpSearchResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn make_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: RegExpSearchResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        resume.0.step_pending.value = Some(value);
        Self::Set { resume }
    }
    pub(crate) fn make_exec(regexp: Value, input: Value, mut resume: RegExpSearchResume) -> Self {
        resume.0.step_pending.regexp = Some(regexp);
        resume.0.step_pending.input = Some(input);
        Self::Exec { resume }
    }
}
impl RegExpSearchResume {
    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpSearchStep::Primitive lost value")
    }

    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpSearchStep::Read lost object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpSearchStep::Read lost key")
    }

    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpSearchStep::Set lost object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpSearchStep::Set lost key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpSearchStep::Set lost value")
    }

    pub(crate) fn take_exec_regexp(&mut self) -> Value {
        self.0
            .step_pending
            .regexp
            .take()
            .expect("RegExpSearchStep::Exec lost regexp")
    }
    pub(crate) fn take_exec_input(&mut self) -> Value {
        self.0
            .step_pending
            .input
            .take()
            .expect("RegExpSearchStep::Exec lost input")
    }
}

const _: () = assert!(std::mem::size_of::<RegExpSearchStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<RegExpSearchStep>() <= 64);
