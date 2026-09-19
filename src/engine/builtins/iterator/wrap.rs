//! Wrapped iterator next parses results; return forwards the original result object.
use super::{
    ObjectIteratorStep,
    step::{NextStep, finish_next},
};
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::{ContextId, HeapError, IteratorResumeKind},
    object::{CallableRef, ObjectRef, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::{Completion, call::NativeInvocation},
};
pub(crate) enum WrapStep {
    Complete(Completion),
    Read { resume: WrapResume },
    Call { resume: WrapResume },
    Next { resume: WrapResume },
    Parse { resume: WrapResume },
}
pub(crate) struct WrapResume(Box<WrapResumeState>);
impl std::ops::Deref for WrapResume {
    type Target = WrapResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for WrapResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<WrapResume>() <= 8);
pub(crate) struct WrapResumeState {
    pending_effect: WrapStepPending,
    realm: ContextId,
    source: Value,
    phase: Phase,
}
enum Phase {
    ReturnMethod,
    ReturnResult,
    NextResult,
    Next,
}
impl WrapStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        mode: IteratorResumeKind,
        invocation: &NativeInvocation,
    ) -> Result<Self, RuntimeError> {
        let receiver = match runtime.iterator_receiver(realm, invocation.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let state = {
            runtime
                .0
                .state
                .borrow()
                .heap
                .iterator_wrap_state(receiver.object_id())
        };
        let (source, next) = match state {
            Ok(state) => state,
            Err(HeapError::Invariant(_)) => {
                return Ok(Self::Complete(Completion::Throw(
                    runtime.new_native_error_jsvalue(
                        realm,
                        NativeErrorKind::Type,
                        "not an Iterator Wrap",
                    )?,
                )));
            }
            Err(error) => return Err(error.into()),
        };
        let source = runtime.root_raw_value(&source)?;
        let resume = WrapResume(Box::new(WrapResumeState {
            pending_effect: WrapStepPending::default(),
            realm,
            source: source.clone(),
            phase: Phase::ReturnMethod,
        }));
        match mode {
            IteratorResumeKind::Return => Ok({
                let __pending_field_receiver = source;
                let __pending_field_key =
                    runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Return)?;
                let __pending_field_resume = resume;
                Self::request_read(
                    __pending_field_receiver,
                    __pending_field_key,
                    __pending_field_resume,
                )
            }),
            IteratorResumeKind::Next => {
                let method = runtime.root_raw_value(&next)?;
                if let Value::Object(iterator) = source {
                    return Ok({
                        let __pending_field_iterator = iterator;
                        let __pending_field_method = method;
                        let __pending_field_resume = {
                            let updated_0 = Phase::Next;
                            let mut resident = resume;
                            resident.0.phase = updated_0;
                            resident
                        };
                        Self::request_next(
                            __pending_field_iterator,
                            __pending_field_method,
                            __pending_field_resume,
                        )
                    });
                }
                let callable = match runtime.iterator_callable_value(realm, method)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(Self::Complete(Completion::Throw(value)));
                    }
                };
                Ok({
                    let __pending_field_callable = callable;
                    let __pending_field_receiver = source;
                    let __pending_field_resume = {
                        let updated_0 = Phase::NextResult;
                        let mut resident = resume;
                        resident.0.phase = updated_0;
                        resident
                    };
                    Self::request_call(
                        __pending_field_callable,
                        __pending_field_receiver,
                        __pending_field_resume,
                    )
                })
            }
        }
    }
}
impl WrapResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<WrapStep, RuntimeError> {
        if matches!(self.0.phase, Phase::NextResult) {
            self.0.phase = Phase::Next;
            return Ok({
                let __pending_field_result = reply;
                let __pending_field_resume = self;
                WrapStep::request_parse(__pending_field_result, __pending_field_resume)
            });
        }
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(WrapStep::Complete(Completion::Throw(value))),
        };
        match self.0.phase {
            Phase::ReturnMethod => {
                if matches!(value, Value::Undefined | Value::Null) {
                    return Ok(WrapStep::Complete(Completion::Return(Value::Object(
                        runtime.new_iterator_result(self.0.realm, Value::Undefined, true)?,
                    ))));
                }
                let callable = match runtime.iterator_callable_value(self.0.realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(WrapStep::Complete(Completion::Throw(value)));
                    }
                };
                self.0.phase = Phase::ReturnResult;
                Ok({
                    let __pending_field_callable = callable;
                    let __pending_field_receiver = self.0.source.clone();
                    let __pending_field_resume = self;
                    WrapStep::request_call(
                        __pending_field_callable,
                        __pending_field_receiver,
                        __pending_field_resume,
                    )
                })
            }
            Phase::ReturnResult => Ok(WrapStep::Complete(if matches!(value, Value::Object(_)) {
                Completion::Return(value)
            } else {
                Completion::Throw(runtime.new_native_error_jsvalue(
                    self.0.realm,
                    NativeErrorKind::Type,
                    "iterator must return an object",
                )?)
            })),
            _ => Err(RuntimeError::Invariant(
                "Iterator Wrap completion phase mismatch",
            )),
        }
    }
    pub(crate) fn next(
        self,
        runtime: &Runtime,
        reply: ObjectIteratorStep,
    ) -> Result<WrapStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Next) {
            return Err(RuntimeError::Invariant("Iterator Wrap next phase mismatch"));
        }
        let (value, done) = match reply {
            ObjectIteratorStep::Throw(value) => {
                return Ok(WrapStep::Complete(Completion::Throw(value)));
            }
            ObjectIteratorStep::Yield(value) => (value, false),
            ObjectIteratorStep::Done => (Value::Undefined, true),
        };
        Ok(WrapStep::Complete(Completion::Return(Value::Object(
            runtime.new_iterator_result(self.0.realm, value, done)?,
        ))))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: WrapStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            WrapStep::Complete(result) => return Ok(result),
            WrapStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            WrapStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &[])?,
                )?
            }
            WrapStep::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                resume.next(
                    runtime,
                    finish_next(
                        runtime,
                        realm,
                        NextStep::start(runtime, realm, iterator, method)?,
                    )?,
                )?
            }
            WrapStep::Parse { mut resume } => {
                let result = resume.take_parse_result();
                resume.next(
                    runtime,
                    finish_next(
                        runtime,
                        realm,
                        NextStep::parse_result(runtime, realm, result)?,
                    )?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct WrapStepPending {
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    next_iterator: Option<ObjectRef>,
    next_method: Option<Value>,
    parse_result: Option<Completion>,
}
impl WrapStep {
    pub(crate) fn request_read(receiver: Value, key: PropertyKey, mut resume: WrapResume) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        mut resume: WrapResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        Self::Call { resume }
    }
    pub(crate) fn request_next(iterator: ObjectRef, method: Value, mut resume: WrapResume) -> Self {
        resume.0.pending_effect.next_iterator = Some(iterator);
        resume.0.pending_effect.next_method = Some(method);
        Self::Next { resume }
    }
    pub(crate) fn request_parse(result: Completion, mut resume: WrapResume) -> Self {
        resume.0.pending_effect.parse_result = Some(result);
        Self::Parse { resume }
    }
}
impl WrapResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("WrapStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("WrapStep Read key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("WrapStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("WrapStep Call receiver")
    }
    pub(crate) fn take_next_iterator(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .next_iterator
            .take()
            .expect("WrapStep Next iterator")
    }
    pub(crate) fn take_next_method(&mut self) -> Value {
        self.0
            .pending_effect
            .next_method
            .take()
            .expect("WrapStep Next method")
    }
    pub(crate) fn take_parse_result(&mut self) -> Completion {
        self.0
            .pending_effect
            .parse_result
            .take()
            .expect("WrapStep Parse result")
    }
}
const _: () = assert!(std::mem::size_of::<WrapStep>() <= 56);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<WrapStep>() <= 64);
