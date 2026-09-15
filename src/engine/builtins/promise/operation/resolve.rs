//! Static and intrinsic PromiseResolve share the constructor identity fast path.
use super::{
    Completion, ContextId, NativeConversion, ObjectRef, Phase, PromiseNativeKind, PromiseResume,
    PromiseStep, Runtime, RuntimeError, Value,
};
use crate::engine::heap::ObjectPayload;
impl PromiseStep {
    pub(crate) fn static_resolve(
        runtime: &Runtime,
        realm: ContextId,
        kind: PromiseNativeKind,
        receiver: Value,
        argument: Value,
    ) -> Result<Self, RuntimeError> {
        let Value::Object(constructor) = receiver else {
            return super::capability::error(runtime, realm, "not an object");
        };
        if kind == PromiseNativeKind::Resolve
            && let Value::Object(promise) = &argument
            && matches!(
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .object(promise.object_id())?
                    .payload,
                ObjectPayload::Promise(_)
            )
        {
            return Ok({
                let __pending_field_receiver = Value::Object(promise.clone());
                let __pending_field_key = runtime.intern_property_key("constructor")?;
                let __pending_field_resume = Box::new(PromiseResume {
                    pending_effect: super::PromiseStepPending::default(),
                    realm,
                    phase: Phase::StaticConstructor {
                        constructor,
                        argument,
                        kind,
                    },
                });
                Self::request_read(
                    __pending_field_receiver,
                    __pending_field_key,
                    __pending_field_resume,
                )
            });
        }
        create(runtime, realm, constructor, argument, kind)
    }
}
pub(super) fn constructor(
    runtime: &Runtime,
    realm: ContextId,
    constructor: ObjectRef,
    argument: Value,
    kind: PromiseNativeKind,
    completion: Completion,
) -> Result<PromiseStep, RuntimeError> {
    match completion {
        Completion::Throw(value) => Ok(PromiseStep::Complete(Completion::Throw(value))),
        Completion::Return(value) if value.same_value(&Value::Object(constructor.clone())) => {
            Ok(PromiseStep::Complete(Completion::Return(argument)))
        }
        Completion::Return(_) => create(runtime, realm, constructor, argument, kind),
    }
}
fn create(
    runtime: &Runtime,
    realm: ContextId,
    constructor: ObjectRef,
    argument: Value,
    kind: PromiseNativeKind,
) -> Result<PromiseStep, RuntimeError> {
    let constructor = match runtime.constructor_from_value(realm, Value::Object(constructor))? {
        NativeConversion::Throw(value) => {
            return Ok(PromiseStep::Complete(Completion::Throw(value)));
        }
        NativeConversion::Value(constructor) => constructor,
    };
    Box::new(PromiseResume {
        pending_effect: super::PromiseStepPending::default(),
        realm,
        phase: Phase::StaticCapability { argument, kind },
    })
    .capability(runtime, Some(constructor))
}
