//! FIFO jobs keep their callback and resolution targets rooted across driver turns.
use super::{
    Completion, ContextId, ObjectRef, Phase, PromiseResume, PromiseStep, Runtime, RuntimeError,
    Value,
};
use crate::engine::heap::{ObjectId, PromiseReaction, PromiseReactionKind, RawValue};

pub(super) struct ReactionTargets {
    pub resolve: ObjectRef,
    pub reject: ObjectRef,
}
impl PromiseStep {
    pub(crate) fn thenable_job(
        runtime: &Runtime,
        realm: ContextId,
        promise: ObjectId,
        thenable: ObjectId,
        then: ObjectId,
    ) -> Result<Self, RuntimeError> {
        let promise = ObjectRef::from_borrowed_handle(runtime.clone(), promise)?;
        let thenable = ObjectRef::from_borrowed_handle(runtime.clone(), thenable)?;
        let then = ObjectRef::from_borrowed_handle(runtime.clone(), then)?;
        let then = runtime.as_callable(&then)?.ok_or(RuntimeError::Invariant(
            "queued Promise then action was no longer callable",
        ))?;
        let (resolve, reject) = runtime.create_promise_resolving_functions(realm, &promise)?;
        let arguments = vec![
            Value::Object(resolve.as_object().clone()),
            Value::Object(reject.as_object().clone()),
        ];
        Ok({
            let __pending_field_callable = then;
            let __pending_field_receiver = Value::Object(thenable);
            let __pending_field_arguments = arguments;
            let __pending_field_resume = Box::new(PromiseResume {
                pending_effect: super::PromiseStepPending::default(),
                realm,
                phase: Phase::Thenable(reject),
            });
            Self::request_call(
                __pending_field_callable,
                __pending_field_receiver,
                __pending_field_arguments,
                __pending_field_resume,
            )
        })
    }

    pub(crate) fn reaction_job(
        runtime: &Runtime,
        realm: ContextId,
        reaction: &PromiseReaction,
        argument: &RawValue,
    ) -> Result<Self, RuntimeError> {
        let argument = runtime.root_raw_value(argument)?;
        let targets = reaction
            .capability
            .map(|capability| {
                Ok::<_, RuntimeError>(ReactionTargets {
                    resolve: ObjectRef::from_borrowed_handle(runtime.clone(), capability.resolve)?,
                    reject: ObjectRef::from_borrowed_handle(runtime.clone(), capability.reject)?,
                })
            })
            .transpose()?;
        let resume = Box::new(PromiseResume {
            pending_effect: super::PromiseStepPending::default(),
            realm,
            phase: Phase::Reaction(targets),
        });
        if let Some(handler) = reaction.handler {
            let handler = ObjectRef::from_borrowed_handle(runtime.clone(), handler)?;
            let handler = runtime
                .as_callable(&handler)?
                .ok_or(RuntimeError::Invariant(
                    "queued Promise reaction handler was no longer callable",
                ))?;
            Ok({
                let __pending_field_callable = handler;
                let __pending_field_receiver = Value::Undefined;
                let __pending_field_arguments = vec![argument];
                let __pending_field_resume = resume;
                Self::request_call(
                    __pending_field_callable,
                    __pending_field_receiver,
                    __pending_field_arguments,
                    __pending_field_resume,
                )
            })
        } else {
            let completion = if reaction.kind == PromiseReactionKind::Reject {
                Completion::Throw(argument)
            } else {
                Completion::Return(argument)
            };
            resume.resume(runtime, completion)
        }
    }
}
