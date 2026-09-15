//! sumPrecise retains the exact accumulator across iterator replies.
use super::SumPrecise;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::{
        iterator::step::{CloseStep, NextStep, finish_close, finish_next},
        object::ObjectIteratorStep,
    },
    heap::ContextId,
    object::{CallableRef, ObjectRef, PropertyKey, WellKnownSymbol},
    value::Value,
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum SumStep {
    Complete(Completion),
    Read { resume: SumResume },
    Call { resume: SumResume },
    Next { resume: SumResume },
    Close { resume: SumResume },
}
enum Phase {
    Method,
    Iterator,
    NextMethod,
    Next,
}
pub(crate) struct SumResume(Box<SumResumeState>);
impl std::ops::Deref for SumResume {
    type Target = SumResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for SumResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<SumResume>() <= 8);
pub(crate) struct SumResumeState {
    pending_effect: SumStepPending,
    realm: ContextId,
    phase: Phase,
    iterable: Value,
    iterator: Option<ObjectRef>,
    next: Value,
    sum: SumPrecise,
}
impl SumStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        if !matches!(invocation, NativeInvocation::Call { .. }) {
            return Err(RuntimeError::Invariant(
                "Math.sumPrecise requires generic invocation",
            ));
        }
        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Math.sumPrecise argv was not padded",
            ))?;
        if matches!(iterable, Value::Null | Value::Undefined) {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    &format!(
                        "cannot read property 'Symbol.iterator' of {}",
                        if matches!(iterable, Value::Null) {
                            "null"
                        } else {
                            "undefined"
                        }
                    ),
                )?,
            )));
        }
        Ok({
            let __pending_field_receiver = iterable.clone();
            let __pending_field_key =
                PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
            let __pending_field_resume = SumResume(Box::new(SumResumeState {
                pending_effect: SumStepPending::default(),
                realm,
                phase: Phase::Method,
                iterable,
                iterator: None,
                next: Value::Undefined,
                sum: SumPrecise::new(),
            }));
            Self::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
}
impl SumResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<SumStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(SumStep::Complete(Completion::Throw(value))),
        };
        match self.0.phase {
            Phase::Method => {
                let callable = match value {
                    Value::Object(object) => runtime.as_callable(&object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return Ok(SumStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "value is not iterable",
                        )?,
                    )));
                };
                self.0.phase = Phase::Iterator;
                Ok({
                    let __pending_field_callable = callable;
                    let __pending_field_receiver = self.0.iterable.clone();
                    let __pending_field_resume = self;
                    SumStep::request_call(
                        __pending_field_callable,
                        __pending_field_receiver,
                        __pending_field_resume,
                    )
                })
            }
            Phase::Iterator => {
                let Value::Object(iterator) = value else {
                    return Ok(SumStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "not an object",
                        )?,
                    )));
                };
                self.0.iterable = Value::Undefined;
                self.0.iterator = Some(iterator.clone());
                self.0.phase = Phase::NextMethod;
                Ok({
                    let __pending_field_receiver = Value::Object(iterator);
                    let __pending_field_key = runtime.intern_property_key("next")?;
                    let __pending_field_resume = self;
                    SumStep::request_read(
                        __pending_field_receiver,
                        __pending_field_key,
                        __pending_field_resume,
                    )
                })
            }
            Phase::NextMethod => {
                self.0.next = value;
                self.next()
            }
            _ => Err(RuntimeError::Invariant("Math sum value phase mismatch")),
        }
    }
    fn next(mut self) -> Result<SumStep, RuntimeError> {
        self.0.phase = Phase::Next;
        Ok({
            let __pending_field_iterator = self
                .0
                .iterator
                .clone()
                .ok_or(RuntimeError::Invariant("Math sum iterator missing"))?;
            let __pending_field_next = self.0.next.clone();
            let __pending_field_resume = self;
            SumStep::request_next(
                __pending_field_iterator,
                __pending_field_next,
                __pending_field_resume,
            )
        })
    }
    pub(crate) fn item(
        mut self,
        runtime: &Runtime,
        result: ObjectIteratorStep,
    ) -> Result<SumStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Next) {
            return Err(RuntimeError::Invariant("Math sum iterator phase mismatch"));
        }
        let item = match result {
            ObjectIteratorStep::Yield(value) => value,
            ObjectIteratorStep::Done => {
                return Ok(SumStep::Complete(Completion::Return(Value::Float(
                    self.0.sum.result(),
                ))));
            }
            ObjectIteratorStep::Throw(value) => {
                return Ok(SumStep::Complete(Completion::Throw(value)));
            }
        };
        let number = match item {
            Value::Int(value) => f64::from(value),
            Value::Float(value) => value,
            _ => {
                return Ok({
                    let __pending_field_iterator = self
                        .0
                        .iterator
                        .take()
                        .ok_or(RuntimeError::Invariant("Math sum iterator missing"))?;
                    let __pending_field_completion = Completion::Throw(runtime.new_native_error(
                        self.0.realm,
                        NativeErrorKind::Type,
                        "not a number",
                    )?);
                    let __pending_field_resume = self;
                    SumStep::request_close(
                        __pending_field_iterator,
                        __pending_field_completion,
                        __pending_field_resume,
                    )
                });
            }
        };
        self.0.sum.add(number);
        self.next()
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: SumStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            SumStep::Complete(result) => return Ok(result),
            SumStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            SumStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &[])?,
                )?
            }
            SumStep::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let next = resume.take_next_next();
                resume.item(
                    runtime,
                    finish_next(
                        runtime,
                        realm,
                        NextStep::start(runtime, realm, iterator, next)?,
                    )?,
                )?
            }
            SumStep::Close { mut resume } => {
                let iterator = resume.take_close_iterator();
                let completion = resume.take_close_completion();
                {
                    return finish_close(
                        runtime,
                        realm,
                        CloseStep::start(runtime, realm, iterator, completion)?,
                    );
                }
            }
        };
    }
}

#[cfg(test)]
#[test]
fn sum_resume_keeps_one_resident_owner_across_iterator_transitions() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let callable = context.eval("(function(){})").unwrap();
    let object = runtime.new_object(None).unwrap();
    let invocation = NativeInvocation::Call {
        this_value: Value::Undefined,
    };
    let arguments = NativeArguments {
        actual_arg_count: 1,
        readable: vec![Value::Object(object.clone())],
    };
    let SumStep::Read { mut resume } =
        SumStep::start(&runtime, context.realm, &invocation, &arguments).unwrap()
    else {
        panic!("method read")
    };
    drop(resume.take_read_receiver());
    drop(resume.take_read_key());
    let address = &*resume.0 as *const SumResumeState;
    let SumStep::Call { mut resume } = resume
        .resume(&runtime, Completion::Return(callable))
        .unwrap()
    else {
        panic!("iterator call")
    };
    drop(resume.take_call_callable());
    drop(resume.take_call_receiver());
    assert_eq!(&*resume.0 as *const SumResumeState, address);
    let SumStep::Read { mut resume } = resume
        .resume(&runtime, Completion::Return(Value::Object(object)))
        .unwrap()
    else {
        panic!("next read")
    };
    drop(resume.take_read_receiver());
    drop(resume.take_read_key());
    assert_eq!(&*resume.0 as *const SumResumeState, address);
}

#[derive(Default)]
struct SumStepPending {
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    next_iterator: Option<ObjectRef>,
    next_next: Option<Value>,
    close_iterator: Option<ObjectRef>,
    close_completion: Option<Completion>,
}
impl SumStep {
    pub(crate) fn request_read(receiver: Value, key: PropertyKey, mut resume: SumResume) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        mut resume: SumResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        Self::Call { resume }
    }
    pub(crate) fn request_next(iterator: ObjectRef, next: Value, mut resume: SumResume) -> Self {
        resume.0.pending_effect.next_iterator = Some(iterator);
        resume.0.pending_effect.next_next = Some(next);
        Self::Next { resume }
    }
    pub(crate) fn request_close(
        iterator: ObjectRef,
        completion: Completion,
        mut resume: SumResume,
    ) -> Self {
        resume.0.pending_effect.close_iterator = Some(iterator);
        resume.0.pending_effect.close_completion = Some(completion);
        Self::Close { resume }
    }
}
impl SumResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("SumStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("SumStep Read key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("SumStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("SumStep Call receiver")
    }
    pub(crate) fn take_next_iterator(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .next_iterator
            .take()
            .expect("SumStep Next iterator")
    }
    pub(crate) fn take_next_next(&mut self) -> Value {
        self.0
            .pending_effect
            .next_next
            .take()
            .expect("SumStep Next next")
    }
    pub(crate) fn take_close_iterator(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .close_iterator
            .take()
            .expect("SumStep Close iterator")
    }
    pub(crate) fn take_close_completion(&mut self) -> Completion {
        self.0
            .pending_effect
            .close_completion
            .take()
            .expect("SumStep Close completion")
    }
}
const _: () = assert!(std::mem::size_of::<SumStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<SumStep>() <= 64);
