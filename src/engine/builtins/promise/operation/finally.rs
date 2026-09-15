//! Finally retains QuickJS's absent-species sentinel through the later resolve.
use super::{PromiseResume, PromiseStep, capability};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::builtins::native::{NativeFunctionId, PromiseNativeKind};
use crate::engine::heap::PromiseReactionKind;
use crate::engine::heap::{ContextId, InternalCallableData};
use crate::engine::object::WellKnownSymbol;
use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{
    Completion,
    call::{NativeArguments, NativeInvocation},
};

pub(super) enum Phase {
    Constructor {
        receiver: ObjectRef,
        callback: Value,
    },
    Species {
        receiver: ObjectRef,
        callback: Value,
    },
    Callback {
        constructor: Value,
        settlement: Value,
        kind: PromiseReactionKind,
    },
    Resolved {
        settlement: Value,
        kind: PromiseReactionKind,
    },
}
fn continuation(realm: ContextId, phase: Phase) -> Box<PromiseResume> {
    Box::new(PromiseResume {
        pending_effect: super::PromiseStepPending::default(),
        realm,
        phase: super::Phase::Finally(phase),
    })
}
impl PromiseStep {
    pub(super) fn invoke_then(
        runtime: &Runtime,
        realm: ContextId,
        receiver: Value,
        arguments: Vec<Value>,
    ) -> Result<Self, RuntimeError> {
        Ok({
            let __pending_field_receiver = receiver.clone();
            let __pending_field_key = runtime.intern_property_key("then")?;
            let __pending_field_resume = Box::new(PromiseResume {
                pending_effect: super::PromiseStepPending::default(),
                realm,
                phase: super::Phase::InvokeThen {
                    receiver,
                    arguments,
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
pub(super) fn start(
    runtime: &Runtime,
    realm: ContextId,
    target: NativeFunctionId,
    invocation: &NativeInvocation,
    arguments: &NativeArguments,
) -> Result<PromiseStep, RuntimeError> {
    let NativeInvocation::Call { this_value } = invocation else {
        return Err(RuntimeError::Invariant(
            "Promise finally received constructor invocation",
        ));
    };
    let argument = arguments
        .readable
        .first()
        .cloned()
        .ok_or(RuntimeError::Invariant(
            "Promise finally argv was not padded",
        ))?;
    if target == NativeFunctionId::Promise(PromiseNativeKind::Finally) {
        let Value::Object(receiver) = this_value else {
            return capability::error(runtime, realm, "not an object");
        };
        return Ok({
            let __pending_field_receiver = this_value.clone();
            let __pending_field_key = runtime.intern_property_key("constructor")?;
            let __pending_field_resume = continuation(
                realm,
                Phase::Constructor {
                    receiver: receiver.clone(),
                    callback: argument,
                },
            );
            PromiseStep::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        });
    }
    let NativeFunctionId::PromiseFinallyHandler(kind) = target else {
        return Err(RuntimeError::Invariant("wrong finally operation"));
    };
    let active = runtime.active_function()?;
    let internal = runtime
        .0
        .state
        .borrow()
        .heap
        .native_internal_callable(active.object_id())?
        .ok_or(RuntimeError::Invariant(
            "Promise finally handler had no internal capture",
        ))?;
    let InternalCallableData::PromiseFinallyHandler {
        constructor,
        on_finally,
    } = internal
    else {
        return Err(RuntimeError::Invariant(
            "Promise finally handler had the wrong internal capture",
        ));
    };
    let callback = ObjectRef::from_borrowed_handle(runtime.clone(), on_finally)?;
    let callable = runtime
        .as_callable(&callback)?
        .ok_or(RuntimeError::Invariant(
            "Promise finally callback was no longer callable",
        ))?;
    // Root the capture before the callback can detach its last external owner.
    let constructor = match constructor {
        Some(id) => Value::Object(ObjectRef::from_borrowed_handle(runtime.clone(), id)?),
        None => Value::Undefined,
    };
    Ok({
        let __pending_field_callable = callable;
        let __pending_field_receiver = Value::Undefined;
        let __pending_field_arguments = Vec::new();
        let __pending_field_resume = continuation(
            realm,
            Phase::Callback {
                constructor,
                settlement: argument,
                kind,
            },
        );
        PromiseStep::request_call(
            __pending_field_callable,
            __pending_field_receiver,
            __pending_field_arguments,
            __pending_field_resume,
        )
    })
}
pub(super) fn resume(
    runtime: &Runtime,
    realm: ContextId,
    phase: Phase,
    completion: Completion,
) -> Result<PromiseStep, RuntimeError> {
    let value = match completion {
        Completion::Throw(value) => return Ok(PromiseStep::Complete(Completion::Throw(value))),
        Completion::Return(value) => value,
    };
    match phase {
        Phase::Constructor { receiver, callback } => match value {
            Value::Undefined => handlers(runtime, realm, receiver, callback, None),
            Value::Object(constructor) => Ok({
                let __pending_field_receiver = Value::Object(constructor);
                let __pending_field_key =
                    PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Species));
                let __pending_field_resume =
                    continuation(realm, Phase::Species { receiver, callback });
                PromiseStep::request_read(
                    __pending_field_receiver,
                    __pending_field_key,
                    __pending_field_resume,
                )
            }),
            _ => capability::error(runtime, realm, "not an object"),
        },
        Phase::Species { receiver, callback } => {
            let constructor = match value {
                Value::Undefined | Value::Null => None,
                value => match runtime.constructor_from_value(realm, value)? {
                    NativeConversion::Throw(value) => {
                        return Ok(PromiseStep::Complete(Completion::Throw(value)));
                    }
                    NativeConversion::Value(constructor) => Some(constructor),
                },
            };
            handlers(runtime, realm, receiver, callback, constructor)
        }
        Phase::Callback {
            constructor,
            settlement,
            kind,
        } => Ok({
            let __pending_field_step = Box::new(PromiseStep::static_resolve(
                runtime,
                realm,
                PromiseNativeKind::Resolve,
                constructor,
                value,
            )?);
            let __pending_field_resume = continuation(realm, Phase::Resolved { settlement, kind });
            PromiseStep::request_nested(__pending_field_step, __pending_field_resume)
        }),
        Phase::Resolved { settlement, kind } => {
            let raw = runtime.raw_property_value(&settlement)?;
            let thunk = runtime.new_internal_promise_function(
                realm,
                NativeFunctionId::PromiseFinallyThunk(kind),
                0,
                0,
                InternalCallableData::PromiseFinallyThunk { value: raw },
            )?;
            drop(settlement);
            PromiseStep::invoke_then(
                runtime,
                realm,
                value,
                vec![Value::Object(thunk.as_object().clone())],
            )
        }
    }
}
fn handlers(
    runtime: &Runtime,
    realm: ContextId,
    receiver: ObjectRef,
    callback: Value,
    constructor: Option<crate::engine::vm::call::ConstructorRef>,
) -> Result<PromiseStep, RuntimeError> {
    let handlers = runtime.prepare_promise_finally_handlers(realm, constructor, callback)?;
    PromiseStep::invoke_then(runtime, realm, Value::Object(receiver), handlers.into())
}
