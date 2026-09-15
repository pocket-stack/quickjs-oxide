//! Aggregate and race loops retain iterator-close and capability ordering.
use super::{PromiseResume, PromiseStep, capability};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::builtins::{native::PromiseNativeKind, promise::RootedPromiseCapability};
use crate::engine::heap::ContextId;
use crate::engine::object::{CallableRef, ObjectRef, PropertyKey};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{
    Completion,
    call::{NativeArguments, NativeInvocation},
};
use crate::engine::{
    api::error::NativeErrorKind, builtins::object::ObjectIteratorStep, object::WellKnownSymbol,
};
use std::{cell::Cell, rc::Rc};

pub(super) enum Phase {
    Resolve(Acquire),
    Method {
        state: Acquire,
        resolve: CallableRef,
    },
    Iterator {
        state: Acquire,
        resolve: CallableRef,
    },
    NextMethod {
        state: Acquire,
        resolve: CallableRef,
        iterator: ObjectRef,
    },
    Next(Box<Loop>),
    Resolved(Box<Loop>),
    Then(Box<Loop>),
    Terminal(RootedPromiseCapability),
    Closed(RootedPromiseCapability),
}
pub(super) struct Acquire {
    constructor: ObjectRef,
    iterable: Value,
    kind: PromiseNativeKind,
    capability: RootedPromiseCapability,
}
pub(super) struct Loop {
    constructor: ObjectRef,
    kind: PromiseNativeKind,
    capability: RootedPromiseCapability,
    resolve: CallableRef,
    iterator: ObjectRef,
    method: Value,
    aggregate: Option<Elements>,
}
struct Elements {
    values: ObjectRef,
    remaining: Rc<Cell<i32>>,
    index: u32,
}
fn continuation(realm: ContextId, phase: Phase) -> Box<PromiseResume> {
    Box::new(PromiseResume {
        pending_effect: super::PromiseStepPending::default(),
        realm,
        phase: super::Phase::Aggregate(phase),
    })
}
fn reject(
    realm: ContextId,
    capability: RootedPromiseCapability,
    reason: Value,
) -> Result<PromiseStep, RuntimeError> {
    super::convenience::settle(realm, capability, Completion::Throw(reason))
}
impl PromiseStep {
    pub(in crate::engine::builtins::promise) fn aggregate(
        runtime: &Runtime,
        realm: ContextId,
        kind: PromiseNativeKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        if !matches!(
            kind,
            PromiseNativeKind::All
                | PromiseNativeKind::AllSettled
                | PromiseNativeKind::Any
                | PromiseNativeKind::Race
        ) {
            return Err(RuntimeError::Invariant(
                "Promise aggregate received non-aggregate selector",
            ));
        }
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise aggregate received constructor invocation",
            ));
        };
        let Value::Object(object) = this_value else {
            return capability::error(runtime, realm, "not an object");
        };
        let constructor = match runtime
            .constructor_from_value(realm, Value::Object(object.clone()))?
        {
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
            NativeConversion::Value(constructor) => constructor,
        };
        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise aggregate iterable argv was not padded",
            ))?;
        Box::new(PromiseResume {
            pending_effect: super::PromiseStepPending::default(),
            realm,
            phase: super::Phase::AggregateCapability {
                constructor: object.clone(),
                iterable,
                kind,
            },
        })
        .capability(runtime, Some(constructor))
    }
}
pub(super) fn ready(
    runtime: &Runtime,
    realm: ContextId,
    constructor: ObjectRef,
    iterable: Value,
    kind: PromiseNativeKind,
    capability: RootedPromiseCapability,
) -> Result<PromiseStep, RuntimeError> {
    Ok({
        let __pending_field_receiver = Value::Object(constructor.clone());
        let __pending_field_key = runtime.intern_property_key("resolve")?;
        let __pending_field_resume = continuation(
            realm,
            Phase::Resolve(Acquire {
                constructor,
                iterable,
                kind,
                capability,
            }),
        );
        PromiseStep::request_read(
            __pending_field_receiver,
            __pending_field_key,
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
    // Iterator acquisition errors reject without close; only the body closes.
    let value = match completion {
        Completion::Return(value) => value,
        Completion::Throw(reason) => {
            return match phase {
                Phase::Resolve(state)
                | Phase::Method { state, .. }
                | Phase::Iterator { state, .. }
                | Phase::NextMethod { state, .. } => reject(realm, state.capability, reason),
                Phase::Resolved(state) | Phase::Then(state) => state.close(realm, reason),
                Phase::Terminal(capability) | Phase::Closed(capability) => {
                    reject(realm, capability, reason)
                }
                Phase::Next(_) => Err(RuntimeError::Invariant(
                    "Promise next expected iterator reply",
                )),
            };
        }
    };
    match phase {
        Phase::Resolve(state) => match runtime.promise_callable(realm, value)? {
            NativeConversion::Throw(reason) => reject(realm, state.capability, reason),
            NativeConversion::Value(resolve) => Ok({
                let __pending_field_receiver = state.iterable.clone();
                let __pending_field_key =
                    PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
                let __pending_field_resume = continuation(realm, Phase::Method { state, resolve });
                PromiseStep::request_read(
                    __pending_field_receiver,
                    __pending_field_key,
                    __pending_field_resume,
                )
            }),
        },
        Phase::Method { state, resolve } => match runtime.promise_callable(realm, value)? {
            NativeConversion::Throw(_) => {
                let reason = runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "value is not iterable",
                )?;
                reject(realm, state.capability, reason)
            }
            NativeConversion::Value(callable) => Ok({
                let __pending_field_callable = callable;
                let __pending_field_receiver = state.iterable.clone();
                let __pending_field_arguments = Vec::new();
                let __pending_field_resume =
                    continuation(realm, Phase::Iterator { state, resolve });
                PromiseStep::request_call(
                    __pending_field_callable,
                    __pending_field_receiver,
                    __pending_field_arguments,
                    __pending_field_resume,
                )
            }),
        },
        Phase::Iterator { state, resolve } => {
            let Value::Object(iterator) = value else {
                let reason =
                    runtime.new_native_error(realm, NativeErrorKind::Type, "not an object")?;
                return reject(realm, state.capability, reason);
            };
            Ok({
                let __pending_field_receiver = Value::Object(iterator.clone());
                let __pending_field_key = runtime.intern_property_key("next")?;
                let __pending_field_resume = continuation(
                    realm,
                    Phase::NextMethod {
                        state,
                        resolve,
                        iterator,
                    },
                );
                PromiseStep::request_read(
                    __pending_field_receiver,
                    __pending_field_key,
                    __pending_field_resume,
                )
            })
        }
        Phase::NextMethod {
            state,
            resolve,
            iterator,
        } => {
            let aggregate = if state.kind == PromiseNativeKind::Race {
                None
            } else {
                Some(Elements {
                    values: runtime.new_array(realm)?,
                    remaining: Rc::new(Cell::new(1)),
                    index: 0,
                })
            };
            Ok(Box::new(Loop {
                constructor: state.constructor,
                kind: state.kind,
                capability: state.capability,
                resolve,
                iterator,
                method: value,
                aggregate,
            })
            .advance(realm))
        }
        Phase::Resolved(state) => {
            let arguments = if let Some(elements) = &state.aggregate {
                let handlers = match runtime.prepare_promise_aggregate_handlers(
                    realm,
                    state.kind,
                    &elements.values,
                    &state.capability,
                    &elements.remaining,
                    elements.index,
                )? {
                    NativeConversion::Value(handlers) => handlers,
                    NativeConversion::Throw(reason) => return state.close(realm, reason),
                };
                let Some(count) = elements.remaining.get().checked_add(1) else {
                    return state.overflow(runtime, realm);
                };
                elements.remaining.set(count);
                handlers.to_vec()
            } else {
                vec![
                    Value::Object(state.capability.resolve.as_object().clone()),
                    Value::Object(state.capability.reject.as_object().clone()),
                ]
            };
            Ok({
                let __pending_field_step =
                    Box::new(PromiseStep::invoke_then(runtime, realm, value, arguments)?);
                let __pending_field_resume = continuation(realm, Phase::Then(state));
                PromiseStep::request_nested(__pending_field_step, __pending_field_resume)
            })
        }
        Phase::Then(mut state) => {
            if let Some(elements) = &mut state.aggregate {
                let Some(index) = elements
                    .index
                    .checked_add(1)
                    .filter(|index| *index != u32::MAX)
                else {
                    return state.overflow(runtime, realm);
                };
                elements.index = index;
            }
            Ok(state.advance(realm))
        }
        Phase::Terminal(capability) => Ok(PromiseStep::Complete(Completion::Return(
            Value::Object(capability.promise),
        ))),
        Phase::Closed(_) | Phase::Next(_) => Err(RuntimeError::Invariant(
            "Promise aggregate unexpected reply",
        )),
    }
}
impl Loop {
    fn advance(self: Box<Self>, realm: ContextId) -> PromiseStep {
        {
            let __pending_field_iterator = self.iterator.clone();
            let __pending_field_method = self.method.clone();
            let __pending_field_resume = continuation(realm, Phase::Next(self));
            PromiseStep::request_next(
                __pending_field_iterator,
                __pending_field_method,
                __pending_field_resume,
            )
        }
    }
    fn close(
        self: Box<Self>,
        realm: ContextId,
        reason: Value,
    ) -> Result<PromiseStep, RuntimeError> {
        Ok({
            let __pending_field_iterator = self.iterator;
            let __pending_field_completion = Completion::Throw(reason);
            let __pending_field_resume = continuation(realm, Phase::Closed(self.capability));
            PromiseStep::request_close(
                __pending_field_iterator,
                __pending_field_completion,
                __pending_field_resume,
            )
        })
    }
    fn overflow(
        self: Box<Self>,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<PromiseStep, RuntimeError> {
        let reason = runtime.new_native_error(
            realm,
            NativeErrorKind::Range,
            "too many Promise aggregate elements",
        )?;
        self.close(realm, reason)
    }
    pub(super) fn next(
        self: Box<Self>,
        runtime: &Runtime,
        realm: ContextId,
        result: ObjectIteratorStep,
    ) -> Result<PromiseStep, RuntimeError> {
        match result {
            ObjectIteratorStep::Throw(reason) => reject(realm, self.capability, reason),
            ObjectIteratorStep::Yield(value) => Ok({
                let __pending_field_callable = self.resolve.clone();
                let __pending_field_receiver = Value::Object(self.constructor.clone());
                let __pending_field_arguments = vec![value];
                let __pending_field_resume = continuation(realm, Phase::Resolved(self));
                PromiseStep::request_call(
                    __pending_field_callable,
                    __pending_field_receiver,
                    __pending_field_arguments,
                    __pending_field_resume,
                )
            }),
            ObjectIteratorStep::Done => {
                if let Some(elements) = &self.aggregate {
                    let count =
                        elements
                            .remaining
                            .get()
                            .checked_sub(1)
                            .ok_or(RuntimeError::Invariant(
                                "Promise aggregate remaining-elements counter underflowed",
                            ))?;
                    elements.remaining.set(count);
                    if count == 0 {
                        let (callable, value) = if self.kind == PromiseNativeKind::Any {
                            (
                                self.capability.reject.clone(),
                                Value::Object(runtime.new_internal_aggregate_error(
                                    realm,
                                    elements.values.clone(),
                                )?),
                            )
                        } else {
                            (
                                self.capability.resolve.clone(),
                                Value::Object(elements.values.clone()),
                            )
                        };
                        return Ok({
                            let __pending_field_callable = callable;
                            let __pending_field_receiver = Value::Undefined;
                            let __pending_field_arguments = vec![value];
                            let __pending_field_resume =
                                continuation(realm, Phase::Terminal(self.capability));
                            PromiseStep::request_call(
                                __pending_field_callable,
                                __pending_field_receiver,
                                __pending_field_arguments,
                                __pending_field_resume,
                            )
                        });
                    }
                }
                Ok(PromiseStep::Complete(Completion::Return(Value::Object(
                    self.capability.promise,
                ))))
            }
        }
    }
}
