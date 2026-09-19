//! Set.forEach keeps its active record until the callback reply is delivered.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{CallableRef, ObjectRef},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
        frames::{ActiveCollectionRecord, ActiveCollectionRecordGuard},
    },
};
pub(crate) enum EachStep {
    Complete(Completion),
    Call { resume: EachResume },
}
pub(crate) struct EachResume(Box<EachResumeState>);
impl std::ops::Deref for EachResume {
    type Target = EachResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for EachResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<EachResume>() <= 8);
pub(crate) struct EachResumeState {
    pending_effect: EachStepPending,
    record: Option<ActiveCollectionRecordGuard>,
    set: ObjectRef,
    callback: CallableRef,
    receiver: Value,
    index: usize,
}
impl EachStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let set = match runtime.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Set.prototype.forEach callback argv was not padded",
        ))?;
        let callback = match value {
            Value::Object(object) => runtime.as_callable(object)?,
            _ => None,
        };
        let Some(callback) = callback else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(realm, NativeErrorKind::Type, "not a function")?,
            )));
        };
        EachResume(Box::new(EachResumeState {
            pending_effect: EachStepPending::default(),
            set: set.clone(),
            callback,
            receiver: arguments
                .readable
                .get(1)
                .cloned()
                .unwrap_or(Value::Undefined),
            index: 0,
            record: None,
        }))
        .next(runtime)
    }
}
impl EachResume {
    fn next(mut self, runtime: &Runtime) -> Result<EachStep, RuntimeError> {
        let Some((record_index, value)) =
            runtime.next_live_set_record(&self.0.set, &mut self.0.index)?
        else {
            return Ok(EachStep::Complete(Completion::Return(Value::Undefined)));
        };
        self.0.record = Some(
            runtime.push_active_collection_record(ActiveCollectionRecord::Set {
                object: self.0.set.object_id(),
                index: record_index,
            }),
        );
        Ok(EachStep::request_call(
            self.0.callback.clone(),
            self.0.receiver.clone(),
            vec![value.clone(), value, Value::Object(self.0.set.clone())],
            self,
        ))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<EachStep, RuntimeError> {
        if let Some(record) = self.0.record.take() {
            record.finish()?;
        }
        match reply {
            Completion::Throw(value) => Ok(EachStep::Complete(Completion::Throw(value))),
            Completion::Return(_) => self.next(runtime),
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: EachStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            EachStep::Complete(result) => return Ok(result),
            EachStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &arguments)?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct EachStepPending {
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
}
impl EachStep {
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: EachResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
}
impl EachResume {
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("EachStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("EachStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("EachStep Call arguments")
    }
}
const _: () = assert!(std::mem::size_of::<EachStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<EachStep>() <= 64);
