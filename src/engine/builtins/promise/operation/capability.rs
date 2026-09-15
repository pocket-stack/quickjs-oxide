//! Capability construction roots its executor until the constructor reply is validated.
use super::{
    Completion, ContextId, NativeConversion, Phase, PromiseResume, PromiseStep,
    RootedPromiseCapability, Runtime, RuntimeError, Value,
};
use crate::engine::vm::call::ConstructorRef;

impl PromiseResume {
    pub(super) fn capability(
        self: Box<Self>,
        runtime: &Runtime,
        constructor: Option<ConstructorRef>,
    ) -> Result<PromiseStep, RuntimeError> {
        let Some(target) = constructor else {
            let capability = runtime.new_default_promise_capability(self.realm)?;
            return self.capability_ready(runtime, NativeConversion::Value(capability));
        };
        let executor = runtime.prepare_promise_capability_executor(self.realm)?;
        Ok({
            let __pending_field_target = target;
            let __pending_field_arguments = vec![Value::Object(executor.as_object().clone())];
            let __pending_field_resume = Box::new(Self {
                pending_effect: super::PromiseStepPending::default(),
                realm: self.realm,
                phase: Phase::Capability {
                    executor,
                    after: self,
                },
            });
            PromiseStep::request_construct(
                __pending_field_target,
                __pending_field_arguments,
                __pending_field_resume,
            )
        })
    }

    pub(super) fn capability_ready(
        self: Box<Self>,
        runtime: &Runtime,
        result: NativeConversion<RootedPromiseCapability>,
    ) -> Result<PromiseStep, RuntimeError> {
        let capability = match result {
            NativeConversion::Throw(value) => {
                return Ok(PromiseStep::Complete(Completion::Throw(value)));
            }
            NativeConversion::Value(capability) => capability,
        };
        match self.phase {
            Phase::AggregateCapability {
                constructor,
                iterable,
                kind,
            } => super::aggregate::ready(
                runtime,
                self.realm,
                constructor,
                iterable,
                kind,
                capability,
            ),
            Phase::ConvenienceCapability { kind, arguments } => {
                super::convenience::ready(runtime, self.realm, kind, arguments, capability)
            }
            Phase::StaticCapability { argument, kind } => {
                let target = if kind == crate::engine::builtins::native::PromiseNativeKind::Reject {
                    capability.reject
                } else {
                    capability.resolve
                };
                Ok({
                    let __pending_field_callable = target;
                    let __pending_field_receiver = Value::Undefined;
                    let __pending_field_arguments = vec![argument];
                    let __pending_field_resume = Box::new(Self {
                        pending_effect: super::PromiseStepPending::default(),
                        realm: self.realm,
                        phase: Phase::ReturnPromise(capability.promise),
                    });
                    PromiseStep::request_call(
                        __pending_field_callable,
                        __pending_field_receiver,
                        __pending_field_arguments,
                        __pending_field_resume,
                    )
                })
            }
            Phase::ThenCapability { promise, handlers } => handlers
                .finish(runtime, self.realm, promise, capability)
                .map(PromiseStep::Complete),
            _ => Err(RuntimeError::Invariant(
                "Promise capability reply has wrong phase",
            )),
        }
    }
}

pub(super) fn error(
    runtime: &Runtime,
    realm: ContextId,
    message: &'static str,
) -> Result<PromiseStep, RuntimeError> {
    Ok(PromiseStep::Complete(Completion::Throw(
        runtime.new_native_error(
            realm,
            crate::engine::api::error::NativeErrorKind::Type,
            message,
        )?,
    )))
}
