//! RegExp String Iterator next retains its matcher snapshot across observable work.
use super::match_protocol::advance_string_index;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::{ContextId, ObjectPayload},
    object::{ObjectRef, PropertyKey, operations::InternalSetResult},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{NativeInvocation, NativeInvokeOutcome},
    },
};

pub(crate) enum RegExpIteratorStep {
    Complete(NativeInvokeOutcome),
    Exec { resume: RegExpIteratorResume },
    Read { resume: RegExpIteratorResume },
    String { resume: RegExpIteratorResume },
    Primitive { resume: RegExpIteratorResume },
    Set { resume: RegExpIteratorResume },
}
enum Phase {
    Exec,
    MatchString,
    LastIndex,
    Advance,
    Set,
}
pub(crate) struct RegExpIteratorResume(Box<RegExpIteratorResumeState>);
impl std::ops::Deref for RegExpIteratorResume {
    type Target = RegExpIteratorResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RegExpIteratorResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<RegExpIteratorResume>() <= 8);
pub(crate) struct RegExpIteratorResumeState {
    step_pending: RegExpIteratorStepPending,
    scheduler_set_key: Option<PropertyKey>,
    realm: ContextId,
    iterator: ObjectRef,
    regexp: ObjectRef,
    string: JsString,
    global: bool,
    full_unicode: bool,
    matched: Option<ObjectRef>,
    // These getter results remain live through the following conversions and
    // lastIndex setter, as in the synchronous algorithm's local bindings.
    match_value: Value,
    index_value: Value,
    phase: Phase,
}
impl RegExpIteratorStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp String Iterator next did not receive an iterator-next invocation",
            ));
        };
        let iterator = match this_value {
            Value::Object(iterator)
                if matches!(
                    runtime
                        .0
                        .state
                        .borrow()
                        .heap
                        .object(iterator.object_id())?
                        .payload,
                    ObjectPayload::RegExpStringIterator { .. }
                ) =>
            {
                iterator.clone()
            }
            _ => {
                return Ok(Self::Complete(NativeInvokeOutcome::Completion(
                    Completion::Throw(runtime.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "RegExp String Iterator object expected",
                    )?),
                )));
            }
        };
        let (regexp_id, string, global, full_unicode, done) = runtime
            .0
            .state
            .borrow()
            .heap
            .regexp_string_iterator_state(iterator.object_id())?;
        if done {
            return Ok(Self::Complete(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            }));
        }
        let regexp = ObjectRef::from_borrowed_handle(runtime.clone(), regexp_id)?;
        Ok(Self::make_exec(
            Value::Object(regexp.clone()),
            Value::String(string.clone()),
            RegExpIteratorResume(Box::new(RegExpIteratorResumeState {
                step_pending: RegExpIteratorStepPending::default(),
                scheduler_set_key: None,
                realm,
                iterator,
                regexp,
                string,
                global,
                full_unicode,
                matched: None,
                match_value: Value::Undefined,
                index_value: Value::Undefined,
                phase: Phase::Exec,
            })),
        ))
    }
}
impl RegExpIteratorResume {
    pub(crate) fn with_scheduler_set_key(mut self, key: PropertyKey) -> Self {
        self.0.scheduler_set_key = Some(key);
        self
    }
    pub(crate) fn take_scheduler_set_key(&mut self) -> PropertyKey {
        self.0.scheduler_set_key.take().expect("waiting Set key")
    }

    fn abrupt(self, value: Value) -> RegExpIteratorStep {
        RegExpIteratorStep::Complete(NativeInvokeOutcome::Completion(Completion::Throw(value)))
    }
    fn yielded(self) -> Result<RegExpIteratorStep, RuntimeError> {
        Ok(RegExpIteratorStep::Complete(
            NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Object(
                    self.0
                        .matched
                        .ok_or(RuntimeError::Invariant("RegExp iterator lost match result"))?,
                ),
                done: false,
            },
        ))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<RegExpIteratorStep, RuntimeError> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(self.abrupt(value)),
        };
        match self.0.phase {
            Phase::Exec => {
                match value {
                    Value::Null => {
                        runtime
                            .0
                            .state
                            .borrow_mut()
                            .heap
                            .finish_regexp_string_iterator(self.0.iterator.object_id())?;
                        return Ok(RegExpIteratorStep::Complete(
                            NativeInvokeOutcome::IteratorNextRaw {
                                value: Value::Undefined,
                                done: true,
                            },
                        ));
                    }
                    Value::Object(matched) => self.0.matched = Some(matched),
                    _ => {
                        return Err(RuntimeError::Invariant(
                            "RegExpExec returned neither an object nor null",
                        ));
                    }
                }
                if !self.0.global {
                    runtime
                        .0
                        .state
                        .borrow_mut()
                        .heap
                        .finish_regexp_string_iterator(self.0.iterator.object_id())?;
                    return self.yielded();
                }
                self.0.phase = Phase::MatchString;
                Ok(RegExpIteratorStep::make_read(
                    self.0.matched.as_ref().unwrap().clone(),
                    runtime.intern_property_key("0")?,
                    self,
                ))
            }
            Phase::MatchString => {
                self.0.match_value = value.clone();
                Ok(RegExpIteratorStep::make_string(value, self))
            }
            Phase::LastIndex => {
                self.0.index_value = value.clone();
                self.0.phase = Phase::Advance;
                Ok(RegExpIteratorStep::make_primitive(value, self))
            }
            Phase::Advance => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "RegExp iterator length conversion returned an object",
                    ));
                }
                let current = match runtime.native_to_length(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                };
                let next = advance_string_index(&self.0.string, current, self.0.full_unicode);
                self.0.phase = Phase::Set;
                Ok(RegExpIteratorStep::make_set(
                    self.0.regexp.clone(),
                    runtime.intern_property_key("lastIndex")?,
                    Value::number(next as f64),
                    self,
                ))
            }
            Phase::Set => Err(RuntimeError::Invariant(
                "RegExp iterator set needs typed reply",
            )),
        }
    }
    pub(crate) fn string(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<JsString>,
    ) -> Result<RegExpIteratorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::MatchString) {
            return Err(RuntimeError::Invariant(
                "RegExp iterator string phase mismatch",
            ));
        }
        let string = match reply {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
        };
        if !string.is_empty() {
            return self.yielded();
        }
        self.0.phase = Phase::LastIndex;
        Ok(RegExpIteratorStep::make_read(
            self.0.regexp.clone(),
            runtime.intern_property_key("lastIndex")?,
            self,
        ))
    }
    pub(crate) fn set(
        self,
        runtime: &Runtime,
        key: PropertyKey,
        reply: NativeConversion<InternalSetResult>,
    ) -> Result<RegExpIteratorStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Set) {
            return Err(RuntimeError::Invariant(
                "RegExp iterator set phase mismatch",
            ));
        }
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, reply)? {
            return Ok(self.abrupt(value));
        }
        self.yielded()
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: RegExpIteratorStep,
) -> Result<NativeInvokeOutcome, RuntimeError> {
    loop {
        step = match step {
            RegExpIteratorStep::Complete(result) => return Ok(result),
            RegExpIteratorStep::Exec { mut resume } => {
                let regexp = resume.take_exec_regexp();
                let input = resume.take_exec_input();
                resume.resume(runtime, runtime.regexp_exec_abstract(realm, regexp, input)?)?
            }
            RegExpIteratorStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            RegExpIteratorStep::String { mut resume } => {
                let value = resume.take_string_value();
                resume.string(runtime, runtime.native_to_js_string(realm, &value)?)?
            }
            RegExpIteratorStep::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                resume.resume(
                    runtime,
                    runtime.to_primitive(realm, value, ToPrimitiveHint::Number)?,
                )?
            }
            RegExpIteratorStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                resume.set(
                    runtime,
                    key.clone(),
                    runtime.internal_set(
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

#[derive(Default)]
pub(crate) struct RegExpIteratorStepPending {
    regexp: Option<Value>,
    input: Option<Value>,
    object: Option<ObjectRef>,
    key: Option<PropertyKey>,
    value: Option<Value>,
}
impl RegExpIteratorStep {
    pub(crate) fn make_exec(regexp: Value, input: Value, mut resume: RegExpIteratorResume) -> Self {
        resume.0.step_pending.regexp = Some(regexp);
        resume.0.step_pending.input = Some(input);
        Self::Exec { resume }
    }
    pub(crate) fn make_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: RegExpIteratorResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn make_string(value: Value, mut resume: RegExpIteratorResume) -> Self {
        resume.0.step_pending.value = Some(value);
        Self::String { resume }
    }
    pub(crate) fn make_primitive(value: Value, mut resume: RegExpIteratorResume) -> Self {
        resume.0.step_pending.value = Some(value);
        Self::Primitive { resume }
    }
    pub(crate) fn make_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: RegExpIteratorResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        resume.0.step_pending.value = Some(value);
        Self::Set { resume }
    }
}
impl RegExpIteratorResume {
    pub(crate) fn take_exec_regexp(&mut self) -> Value {
        self.0
            .step_pending
            .regexp
            .take()
            .expect("RegExpIteratorStep::Exec lost regexp")
    }
    pub(crate) fn take_exec_input(&mut self) -> Value {
        self.0
            .step_pending
            .input
            .take()
            .expect("RegExpIteratorStep::Exec lost input")
    }

    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpIteratorStep::Read lost object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpIteratorStep::Read lost key")
    }

    pub(crate) fn take_string_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpIteratorStep::String lost value")
    }

    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpIteratorStep::Primitive lost value")
    }

    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpIteratorStep::Set lost object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpIteratorStep::Set lost key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpIteratorStep::Set lost value")
    }
}

const _: () = assert!(std::mem::size_of::<RegExpIteratorStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<RegExpIteratorStep>() <= 64);
