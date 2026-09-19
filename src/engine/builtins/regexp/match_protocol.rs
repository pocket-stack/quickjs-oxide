//! RegExp @@match preserves global collection and empty-match progress.
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

pub(crate) fn advance_string_index(input: &JsString, index: u64, unicode: bool) -> u64 {
    let width = if unicode
        && usize::try_from(index)
            .ok()
            .filter(|index| *index < input.len())
            .and_then(|index| input.code_point_at(index))
            .is_some_and(|code_point| code_point > u32::from(u16::MAX))
    {
        2
    } else {
        1
    };
    index + width
}

pub(crate) enum RegExpMatchStep {
    Complete(Completion),
    Primitive { resume: RegExpMatchResume },
    Read { resume: RegExpMatchResume },
    Set { resume: RegExpMatchResume },
    Exec { resume: RegExpMatchResume },
}
pub(crate) struct RegExpMatchResume(Box<RegExpMatchResumeState>);
impl std::ops::Deref for RegExpMatchResume {
    type Target = RegExpMatchResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RegExpMatchResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<RegExpMatchResume>() <= 8);
pub(crate) struct RegExpMatchResumeState {
    step_pending: RegExpMatchStepPending,
    realm: ContextId,
    regexp: ObjectRef,
    phase: MatchPhase,
}
struct MatchCollection {
    input: JsString,
    unicode: bool,
    matches: ObjectRef,
    zero: PropertyKey,
    count: u32,
}
enum MatchPhase {
    Input,
    Flags(JsString),
    FlagsString(JsString),
    InitialSet { input: JsString, unicode: bool },
    Single,
    Exec(MatchCollection),
    Match(MatchCollection),
    MatchString(MatchCollection),
    LastIndex(MatchCollection),
    LastIndexNumber(MatchCollection),
    AdvancedSet(MatchCollection),
}
impl RegExpMatchStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@match did not receive a generic invocation",
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
                    "RegExp @@match input argv was not padded",
                ))?
                .clone(),
            ToPrimitiveHint::String,
            RegExpMatchResume(Box::new(RegExpMatchResumeState {
                step_pending: RegExpMatchStepPending::default(),
                realm,
                regexp: regexp.clone(),
                phase: MatchPhase::Input,
            })),
        ))
    }
}
impl RegExpMatchResume {
    fn execute(mut self, state: MatchCollection) -> RegExpMatchStep {
        RegExpMatchStep::make_exec(
            Value::Object(self.0.regexp.clone()),
            Value::String(state.input.clone()),
            {
                let updated_0 = MatchPhase::Exec(state);
                self.0.phase = updated_0;
                self
            },
        )
    }
    pub(crate) fn set(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<RegExpMatchStep, RuntimeError> {
        let key =
            runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::LastIndex)?;
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            return Ok(RegExpMatchStep::Complete(Completion::Throw(value)));
        }
        match self.0.phase {
            MatchPhase::InitialSet { input, unicode } => {
                let matches = runtime.new_array(self.0.realm)?;
                let zero = runtime
                    .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Literal1)?;
                Ok({
                    let updated_0 = MatchPhase::Single;
                    self.0.phase = updated_0;
                    self
                }
                .execute(MatchCollection {
                    input,
                    unicode,
                    matches,
                    zero,
                    count: 0,
                }))
            }
            MatchPhase::AdvancedSet(state) => Ok({
                let updated_0 = MatchPhase::Single;
                self.0.phase = updated_0;
                self
            }
            .execute(state)),
            _ => Err(RuntimeError::Invariant(
                "RegExp match received an unexpected Set reply",
            )),
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<RegExpMatchStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(RegExpMatchStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            MatchPhase::Input => {
                let input = match match_string(runtime, self.0.realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpMatchStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok(RegExpMatchStep::make_read(
                    self.0.regexp.clone(),
                    runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Flags)?,
                    {
                        let updated_0 = MatchPhase::Flags(input);
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            MatchPhase::Flags(input) => Ok(RegExpMatchStep::make_primitive(
                value,
                ToPrimitiveHint::String,
                {
                    let updated_0 = MatchPhase::FlagsString(input);
                    self.0.phase = updated_0;
                    self
                },
            )),
            MatchPhase::FlagsString(input) => {
                let flags = match match_string(runtime, self.0.realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpMatchStep::Complete(Completion::Throw(value)));
                    }
                };
                if !flags.utf16_units().any(|unit| unit == u16::from(b'g')) {
                    return Ok(RegExpMatchStep::make_exec(
                        Value::Object(self.0.regexp.clone()),
                        Value::String(input),
                        {
                            let updated_0 = MatchPhase::Single;
                            self.0.phase = updated_0;
                            self
                        },
                    ));
                }
                let unicode = flags
                    .utf16_units()
                    .any(|unit| unit == u16::from(b'u') || unit == u16::from(b'v'));
                Ok(RegExpMatchStep::make_set(
                    self.0.regexp.clone(),
                    runtime
                        .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::LastIndex)?,
                    Value::Int(0),
                    {
                        let updated_0 = MatchPhase::InitialSet { input, unicode };
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            MatchPhase::Single => Ok(RegExpMatchStep::Complete(Completion::Return(value))),
            MatchPhase::Exec(state) => {
                let result = match value {
                    Value::Null => {
                        return Ok(RegExpMatchStep::Complete(Completion::Return(
                            if state.count == 0 {
                                Value::Null
                            } else {
                                Value::Object(state.matches)
                            },
                        )));
                    }
                    Value::Object(result) => result,
                    _ => {
                        return Err(RuntimeError::Invariant(
                            "RegExpExec returned neither an object nor null",
                        ));
                    }
                };
                Ok(RegExpMatchStep::make_read(result, state.zero.clone(), {
                    let updated_0 = MatchPhase::Match(state);
                    self.0.phase = updated_0;
                    self
                }))
            }
            MatchPhase::Match(state) => Ok(RegExpMatchStep::make_primitive(
                value,
                ToPrimitiveHint::String,
                {
                    let updated_0 = MatchPhase::MatchString(state);
                    self.0.phase = updated_0;
                    self
                },
            )),
            MatchPhase::MatchString(mut state) => {
                let matched = match match_string(runtime, self.0.realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpMatchStep::Complete(Completion::Throw(value)));
                    }
                };
                let empty = matched.is_empty();
                let Some(next) = state.count.checked_add(1) else {
                    return Ok(RegExpMatchStep::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            self.0.realm,
                            NativeErrorKind::Range,
                            "invalid array length",
                        )?,
                    )));
                };
                // This Array is private until terminal completion, including
                // across custom exec/result callbacks. Its consecutive C/W/E
                // elements use the same constructor effect as builtin exec
                // results; inherited indexed setters must never be observed.
                runtime.append_fresh_array_value(&state.matches, Value::String(matched))?;
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    "regexp_result.match_append",
                );
                state.count = next;
                if empty {
                    Ok(RegExpMatchStep::make_read(
                        self.0.regexp.clone(),
                        runtime.pinned_property_key(
                            crate::engine::atom::pinned::PinnedAtom::LastIndex,
                        )?,
                        {
                            let updated_0 = MatchPhase::LastIndex(state);
                            self.0.phase = updated_0;
                            self
                        },
                    ))
                } else {
                    Ok({
                        let updated_0 = MatchPhase::Single;
                        self.0.phase = updated_0;
                        self
                    }
                    .execute(state))
                }
            }
            MatchPhase::LastIndex(state) => Ok(RegExpMatchStep::make_primitive(
                value,
                ToPrimitiveHint::Number,
                {
                    let updated_0 = MatchPhase::LastIndexNumber(state);
                    self.0.phase = updated_0;
                    self
                },
            )),
            MatchPhase::LastIndexNumber(state) => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "RegExp match lastIndex conversion returned an object",
                    ));
                }
                let current = match runtime.native_to_length(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpMatchStep::Complete(Completion::Throw(value)));
                    }
                };
                let next = advance_string_index(&state.input, current, state.unicode);
                Ok(RegExpMatchStep::make_set(
                    self.0.regexp.clone(),
                    runtime
                        .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::LastIndex)?,
                    Value::number(next as f64),
                    {
                        let updated_0 = MatchPhase::AdvancedSet(state);
                        self.0.phase = updated_0;
                        self
                    },
                ))
            }
            MatchPhase::InitialSet { .. } | MatchPhase::AdvancedSet(_) => Err(
                RuntimeError::Invariant("RegExp match Set received an untyped reply"),
            ),
        }
    }
}
fn match_string(
    runtime: &Runtime,
    realm: ContextId,
    value: Value,
) -> Result<NativeConversion<JsString>, RuntimeError> {
    if matches!(value, Value::Object(_)) {
        return Err(RuntimeError::Invariant(
            "RegExp match String conversion returned an object",
        ));
    }
    runtime.native_to_js_string(realm, &value)
}
impl Runtime {
    pub(crate) fn call_regexp_symbol_match(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let mut step = RegExpMatchStep::start(self, realm, &invocation, arguments)?;
        loop {
            step = match step {
                RegExpMatchStep::Complete(result) => return Ok(result),
                RegExpMatchStep::Primitive { mut resume } => {
                    let value = resume.take_primitive_value();
                    let hint = resume.take_primitive_hint();
                    {
                        let result = if matches!(value, Value::Object(_)) {
                            self.to_primitive(realm, value, hint)?
                        } else {
                            Completion::Return(value)
                        };
                        resume.resume(self, result)?
                    }
                }
                RegExpMatchStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    resume.resume(self, self.get_property_in_realm(realm, &object, &key)?)?
                }
                RegExpMatchStep::Exec { mut resume } => {
                    let regexp = resume.take_exec_regexp();
                    let input = resume.take_exec_input();
                    resume.resume(self, self.regexp_exec_abstract(realm, regexp, input)?)?
                }
                RegExpMatchStep::Set { mut resume } => {
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
pub(crate) struct RegExpMatchStepPending {
    value: Option<Value>,
    hint: Option<ToPrimitiveHint>,
    object: Option<ObjectRef>,
    key: Option<PropertyKey>,
    regexp: Option<Value>,
    input: Option<Value>,
}
impl RegExpMatchStep {
    pub(crate) fn make_primitive(
        value: Value,
        hint: ToPrimitiveHint,
        mut resume: RegExpMatchResume,
    ) -> Self {
        resume.0.step_pending.value = Some(value);
        resume.0.step_pending.hint = Some(hint);
        Self::Primitive { resume }
    }
    pub(crate) fn make_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: RegExpMatchResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn make_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: RegExpMatchResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        resume.0.step_pending.value = Some(value);
        Self::Set { resume }
    }
    pub(crate) fn make_exec(regexp: Value, input: Value, mut resume: RegExpMatchResume) -> Self {
        resume.0.step_pending.regexp = Some(regexp);
        resume.0.step_pending.input = Some(input);
        Self::Exec { resume }
    }
}
impl RegExpMatchResume {
    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpMatchStep::Primitive lost value")
    }
    pub(crate) fn take_primitive_hint(&mut self) -> ToPrimitiveHint {
        self.0
            .step_pending
            .hint
            .take()
            .expect("RegExpMatchStep::Primitive lost hint")
    }

    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpMatchStep::Read lost object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpMatchStep::Read lost key")
    }

    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpMatchStep::Set lost object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpMatchStep::Set lost key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpMatchStep::Set lost value")
    }

    pub(crate) fn take_exec_regexp(&mut self) -> Value {
        self.0
            .step_pending
            .regexp
            .take()
            .expect("RegExpMatchStep::Exec lost regexp")
    }
    pub(crate) fn take_exec_input(&mut self) -> Value {
        self.0
            .step_pending
            .input
            .take()
            .expect("RegExpMatchStep::Exec lost input")
    }
}

const _: () = assert!(std::mem::size_of::<RegExpMatchStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<RegExpMatchStep>() <= 64);
