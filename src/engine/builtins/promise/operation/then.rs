//! Public then preserves species/capability effects before inspecting handlers.
use super::{
    Completion, ContextId, NativeArguments, NativeConversion, NativeInvocation, ObjectRef, Phase,
    PromiseResume, PromiseStep, Runtime, RuntimeError, Value,
};
use crate::engine::{
    heap::ObjectPayload,
    object::{PropertyKey, WellKnownSymbol},
};
impl PromiseStep {
    pub(super) fn then(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.prototype.then received a constructor invocation",
            ));
        };
        let Value::Object(promise) = this_value else {
            return super::capability::error(runtime, realm, "not a promise");
        };
        if !matches!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(promise.object_id())?
                .payload,
            ObjectPayload::Promise(_)
        ) {
            return super::capability::error(runtime, realm, "not a promise");
        }
        let handlers = ThenHandlers::Public([
            arguments
                .readable
                .first()
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Promise.then fulfill argv was not padded",
                ))?,
            arguments
                .readable
                .get(1)
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Promise.then reject argv was not padded",
                ))?,
        ]);
        Ok({
            let __pending_field_receiver = Value::Object(promise.clone());
            let __pending_field_key = runtime.intern_property_key("constructor")?;
            let __pending_field_resume = Box::new(PromiseResume {
                pending_effect: super::PromiseStepPending::default(),
                realm,
                phase: Phase::ThenConstructor {
                    promise: promise.clone(),
                    handlers,
                },
            });
            Self::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
}
pub(super) fn constructor(
    runtime: &Runtime,
    realm: ContextId,
    promise: ObjectRef,
    handlers: ThenHandlers,
    result: Completion,
) -> Result<PromiseStep, RuntimeError> {
    match result {
        Completion::Throw(value) => Ok(PromiseStep::Complete(Completion::Throw(value))),
        Completion::Return(Value::Undefined) => Box::new(PromiseResume {
            pending_effect: super::PromiseStepPending::default(),
            realm,
            phase: Phase::ThenCapability { promise, handlers },
        })
        .capability(runtime, None),
        Completion::Return(Value::Object(constructor)) => Ok({
            let __pending_field_receiver = Value::Object(constructor);
            let __pending_field_key =
                PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Species));
            let __pending_field_resume = Box::new(PromiseResume {
                pending_effect: super::PromiseStepPending::default(),
                realm,
                phase: Phase::ThenSpecies { promise, handlers },
            });
            PromiseStep::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        }),
        Completion::Return(_) => super::capability::error(runtime, realm, "not an object"),
    }
}
pub(super) fn species(
    runtime: &Runtime,
    realm: ContextId,
    promise: ObjectRef,
    handlers: ThenHandlers,
    result: Completion,
) -> Result<PromiseStep, RuntimeError> {
    let constructor = match result {
        Completion::Throw(value) => return Ok(PromiseStep::Complete(Completion::Throw(value))),
        Completion::Return(Value::Undefined | Value::Null) => None,
        Completion::Return(value) => match runtime.constructor_from_value(realm, value)? {
            NativeConversion::Throw(value) => {
                return Ok(PromiseStep::Complete(Completion::Throw(value)));
            }
            NativeConversion::Value(constructor) => Some(constructor),
        },
    };
    Box::new(PromiseResume {
        pending_effect: super::PromiseStepPending::default(),
        realm,
        phase: Phase::ThenCapability { promise, handlers },
    })
    .capability(runtime, constructor)
}

/// Internal handlers are prepared at their original point after species and
/// capability creation. Their roots remain owned while user code is suspended.
pub(super) enum ThenHandlers {
    Public([Value; 2]),
    Module {
        fulfill: crate::engine::object::CallableRef,
        reject: crate::engine::object::CallableRef,
    },
    Dynamic {
        module: crate::engine::heap::RawModuleRef,
        _root: crate::engine::modules::ModuleBytecodeRef,
        resolve: ObjectRef,
        reject: ObjectRef,
    },
}
impl PromiseStep {
    pub(crate) fn module_then(
        runtime: &Runtime,
        realm: ContextId,
        promise: ObjectRef,
        fulfill: crate::engine::object::CallableRef,
        reject: crate::engine::object::CallableRef,
    ) -> Result<Self, RuntimeError> {
        Self::internal_then(
            runtime,
            realm,
            promise,
            ThenHandlers::Module { fulfill, reject },
        )
    }
    pub(crate) fn dynamic_import_then(
        runtime: &Runtime,
        realm: ContextId,
        promise: ObjectRef,
        module: crate::engine::heap::RawModuleRef,
        resolve: crate::engine::heap::ObjectId,
        reject: crate::engine::heap::ObjectId,
    ) -> Result<Self, RuntimeError> {
        if module.cache != realm {
            return Err(RuntimeError::Invariant(
                "dynamic import module belongs to another Context cache",
            ));
        }
        Self::internal_then(
            runtime,
            realm,
            promise,
            ThenHandlers::Dynamic {
                module,
                _root: runtime.root_module(module)?,
                resolve: ObjectRef::from_borrowed_handle(runtime.clone(), resolve)?,
                reject: ObjectRef::from_borrowed_handle(runtime.clone(), reject)?,
            },
        )
    }
    fn internal_then(
        runtime: &Runtime,
        realm: ContextId,
        promise: ObjectRef,
        handlers: ThenHandlers,
    ) -> Result<Self, RuntimeError> {
        Ok({
            let __pending_field_receiver = Value::Object(promise.clone());
            let __pending_field_key = runtime.intern_property_key("constructor")?;
            let __pending_field_resume = Box::new(PromiseResume {
                pending_effect: super::PromiseStepPending::default(),
                realm,
                phase: Phase::ThenConstructor { promise, handlers },
            });
            Self::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
}
impl ThenHandlers {
    pub(super) fn finish(
        self,
        runtime: &Runtime,
        realm: ContextId,
        promise: ObjectRef,
        capability: super::RootedPromiseCapability,
    ) -> Result<Completion, RuntimeError> {
        use crate::engine::{
            builtins::native::{DynamicImportHandlerKind, NativeFunctionId},
            heap::InternalCallableData,
        };
        let (fulfill, reject) = match self {
            Self::Public(handlers) => {
                return runtime.finish_promise_then(realm, promise, handlers, capability);
            }
            Self::Module { fulfill, reject } => (fulfill, reject),
            Self::Dynamic {
                module,
                _root,
                resolve,
                reject,
            } => {
                let make_handler = |kind| {
                    runtime.new_internal_promise_function(
                        realm,
                        NativeFunctionId::DynamicImportHandler(kind),
                        1,
                        0,
                        InternalCallableData::DynamicImportHandler {
                            module,
                            resolve: resolve.object_id(),
                            reject: reject.object_id(),
                            kind,
                        },
                    )
                };
                let fulfill = make_handler(DynamicImportHandlerKind::Fulfill)?;
                let reject = make_handler(DynamicImportHandlerKind::Reject)?;
                (fulfill, reject)
            }
        };
        runtime.perform_promise_then_with_capability(
            realm,
            &promise,
            Some(&fulfill),
            Some(&reject),
            &capability,
        )?;
        Ok(Completion::Return(Value::Undefined))
    }
}
