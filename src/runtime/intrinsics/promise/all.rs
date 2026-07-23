//! QuickJS's shared `Promise.all`/`allSettled`/`any` aggregate loop.
//!
//! The entry algorithm is shared, but each callback family keeps a distinct
//! typed heap capture. In particular, pinned QuickJS copies `allSettled`'s
//! fulfill and reject CFunctionData records, so their first-call bits are
//! deliberately independent rather than one shared specification record.

use std::{cell::Cell, rc::Rc};

use super::*;
use crate::runtime::intrinsics::object::ObjectIteratorStep;

#[derive(Clone, Copy)]
enum AggregateTerminal {
    ResolveValues,
    RejectAggregate,
}

impl Runtime {
    pub(super) fn call_promise_aggregate(
        &self,
        kind: PromiseNativeKind,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if !matches!(
            kind,
            PromiseNativeKind::All | PromiseNativeKind::AllSettled | PromiseNativeKind::Any
        ) {
            return Err(RuntimeError::Invariant(
                "Promise aggregate received a non-aggregate selector",
            ));
        }
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise aggregate received a constructor invocation",
            ));
        };
        let Value::Object(constructor_object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let constructor =
            match self.constructor_from_value(realm, Value::Object(constructor_object.clone()))? {
                NativeConversion::Value(constructor) => constructor,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let capability = match self.new_promise_capability(realm, Some(&constructor))? {
            NativeConversion::Value(capability) => capability,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let resolve_key = self.intern_property_key("resolve")?;
        let promise_resolve =
            match self.get_property_in_realm(realm, &constructor_object, &resolve_key)? {
                Completion::Return(value) => match self.promise_callable(realm, value)? {
                    NativeConversion::Value(callable) => callable,
                    NativeConversion::Throw(value) => {
                        return self.reject_promise_capability(realm, &capability, value);
                    }
                },
                Completion::Throw(value) => {
                    return self.reject_promise_capability(realm, &capability, value);
                }
            };

        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise aggregate iterable argv was not padded",
            ))?;
        let (iterator, next_method) = match self.promise_iterator_record(realm, iterable)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => {
                return self.reject_promise_capability(realm, &capability, value);
            }
        };

        let values = self.new_array(realm)?;
        let remaining = Rc::new(Cell::new(1_i32));
        let mut index = 0_u32;
        loop {
            let item = match self.object_iterator_next(realm, &iterator, next_method.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    let count = remaining
                        .get()
                        .checked_sub(1)
                        .ok_or(RuntimeError::Invariant(
                            "Promise aggregate remaining-elements counter underflowed",
                        ))?;
                    remaining.set(count);
                    if count == 0 {
                        let (settle, value) = match kind {
                            PromiseNativeKind::Any => (
                                &capability.reject,
                                Value::Object(
                                    self.new_internal_aggregate_error(realm, values.clone())?,
                                ),
                            ),
                            PromiseNativeKind::All | PromiseNativeKind::AllSettled => {
                                (&capability.resolve, Value::Object(values.clone()))
                            }
                            _ => unreachable!("aggregate selector was validated above"),
                        };
                        match self.call_internal(realm, settle, Value::Undefined, &[value])? {
                            Completion::Return(_) => {}
                            Completion::Throw(value) => {
                                return self.reject_promise_capability(realm, &capability, value);
                            }
                        }
                    }
                    return Ok(Completion::Return(Value::Object(capability.promise)));
                }
                ObjectIteratorStep::Throw(value) => {
                    return self.reject_promise_capability(realm, &capability, value);
                }
            };

            let next_promise = self.call_internal(
                realm,
                &promise_resolve,
                Value::Object(constructor_object.clone()),
                std::slice::from_ref(&item),
            )?;
            drop(item);
            let next_promise = match next_promise {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return self.reject_promise_capability(realm, &capability, value);
                }
            };

            let then_arguments = match kind {
                PromiseNativeKind::All => {
                    let resolve_element = self.new_internal_promise_function(
                        realm,
                        NativeFunctionId::PromiseAllResolveElement,
                        1,
                        1,
                        InternalCallableData::PromiseAllResolveElement {
                            values: values.object_id(),
                            resolve: capability.resolve.as_object().object_id(),
                            remaining: remaining.clone(),
                            already_called: Rc::new(Cell::new(false)),
                            index,
                        },
                    )?;
                    [
                        Value::Object(resolve_element.as_object().clone()),
                        Value::Object(capability.reject.as_object().clone()),
                    ]
                }
                PromiseNativeKind::AllSettled => {
                    let make_element = |outcome| {
                        self.new_internal_promise_function(
                            realm,
                            NativeFunctionId::PromiseAllSettledElement(outcome),
                            1,
                            1,
                            InternalCallableData::PromiseAllSettledElement {
                                values: values.object_id(),
                                resolve: capability.resolve.as_object().object_id(),
                                remaining: remaining.clone(),
                                already_called: Rc::new(Cell::new(false)),
                                index,
                                outcome,
                            },
                        )
                    };
                    let fulfill_element = make_element(PromiseReactionKind::Fulfill)?;
                    let reject_element = make_element(PromiseReactionKind::Reject)?;
                    [
                        Value::Object(fulfill_element.as_object().clone()),
                        Value::Object(reject_element.as_object().clone()),
                    ]
                }
                PromiseNativeKind::Any => {
                    let reject_element = self.new_internal_promise_function(
                        realm,
                        NativeFunctionId::PromiseAnyRejectElement,
                        1,
                        1,
                        InternalCallableData::PromiseAnyRejectElement {
                            errors: values.object_id(),
                            reject: capability.reject.as_object().object_id(),
                            remaining: remaining.clone(),
                            already_called: Rc::new(Cell::new(false)),
                            index,
                        },
                    )?;
                    if let Some(value) = self.define_array_data_property_without_throw(
                        realm,
                        &values,
                        index,
                        Value::Undefined,
                    )? {
                        self.close_iterator_preserving_throw(realm, &iterator)?;
                        return self.reject_promise_capability(realm, &capability, value);
                    }
                    [
                        Value::Object(capability.resolve.as_object().clone()),
                        Value::Object(reject_element.as_object().clone()),
                    ]
                }
                _ => unreachable!("aggregate selector was validated above"),
            };

            let Some(count) = remaining.get().checked_add(1) else {
                let value = self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "too many Promise aggregate elements",
                )?;
                self.close_iterator_preserving_throw(realm, &iterator)?;
                return self.reject_promise_capability(realm, &capability, value);
            };
            remaining.set(count);

            let then_completion = self.invoke_promise_then(realm, next_promise, &then_arguments)?;
            if let Completion::Throw(value) = then_completion {
                self.close_iterator_preserving_throw(realm, &iterator)?;
                return self.reject_promise_capability(realm, &capability, value);
            }

            index = match index.checked_add(1).filter(|index| *index != u32::MAX) {
                Some(index) => index,
                None => {
                    let value = self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "too many Promise aggregate elements",
                    )?;
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return self.reject_promise_capability(realm, &capability, value);
                }
            };
        }
    }

    pub(in crate::runtime) fn call_promise_all_resolve_element(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.all resolve-element received a constructor invocation",
            ));
        };
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "Promise.all resolve-element had no internal capture",
            ))?;
        let InternalCallableData::PromiseAllResolveElement {
            values,
            resolve,
            remaining,
            already_called,
            index,
        } = internal
        else {
            return Err(RuntimeError::Invariant(
                "Promise.all resolve-element had the wrong internal capture",
            ));
        };
        if already_called.replace(true) {
            return Ok(Completion::Return(Value::Undefined));
        }

        let value = self.promise_aggregate_element_argument(arguments)?;
        self.finish_promise_aggregate_element(
            realm,
            values,
            resolve,
            remaining,
            index,
            value,
            AggregateTerminal::ResolveValues,
        )
    }

    pub(in crate::runtime) fn call_promise_all_settled_element(
        &self,
        target_outcome: PromiseReactionKind,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.allSettled element received a constructor invocation",
            ));
        };
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "Promise.allSettled element had no internal capture",
            ))?;
        let InternalCallableData::PromiseAllSettledElement {
            values,
            resolve,
            remaining,
            already_called,
            index,
            outcome,
        } = internal
        else {
            return Err(RuntimeError::Invariant(
                "Promise.allSettled element had the wrong internal capture",
            ));
        };
        if outcome != target_outcome {
            return Err(RuntimeError::Invariant(
                "Promise.allSettled element selector did not match its capture",
            ));
        }
        if already_called.replace(true) {
            return Ok(Completion::Return(Value::Undefined));
        }

        let value = self.promise_aggregate_element_argument(arguments)?;
        let result = self.new_ordinary_object_in_realm(realm)?;
        let (status, payload_name) = match outcome {
            PromiseReactionKind::Fulfill => ("fulfilled", "value"),
            PromiseReactionKind::Reject => ("rejected", "reason"),
        };
        self.define_fresh_aggregate_property(
            &result,
            "status",
            Value::String(JsString::from_static(status)),
        )?;
        self.define_fresh_aggregate_property(&result, payload_name, value)?;

        self.finish_promise_aggregate_element(
            realm,
            values,
            resolve,
            remaining,
            index,
            Value::Object(result),
            AggregateTerminal::ResolveValues,
        )
    }

    pub(in crate::runtime) fn call_promise_any_reject_element(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.any reject-element received a constructor invocation",
            ));
        };
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "Promise.any reject-element had no internal capture",
            ))?;
        let InternalCallableData::PromiseAnyRejectElement {
            errors,
            reject,
            remaining,
            already_called,
            index,
        } = internal
        else {
            return Err(RuntimeError::Invariant(
                "Promise.any reject-element had the wrong internal capture",
            ));
        };
        if already_called.replace(true) {
            return Ok(Completion::Return(Value::Undefined));
        }

        let reason = self.promise_aggregate_element_argument(arguments)?;
        self.finish_promise_aggregate_element(
            realm,
            errors,
            reject,
            remaining,
            index,
            reason,
            AggregateTerminal::RejectAggregate,
        )
    }

    fn promise_aggregate_element_argument(
        &self,
        arguments: &NativeArguments,
    ) -> Result<Value, RuntimeError> {
        arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise aggregate element argv was not padded",
            ))
    }

    fn define_fresh_aggregate_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        if !self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "fresh Promise.allSettled result rejected a data property",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_promise_aggregate_element(
        &self,
        realm: ContextId,
        values: ObjectId,
        settle: ObjectId,
        remaining: Rc<Cell<i32>>,
        index: u32,
        value: Value,
        terminal: AggregateTerminal,
    ) -> Result<Completion, RuntimeError> {
        let values = ObjectRef::from_borrowed_handle(self.clone(), values)?;
        if let Some(value) =
            self.define_array_data_property_without_throw(realm, &values, index, value)?
        {
            return Ok(Completion::Throw(value));
        }

        let count = remaining
            .get()
            .checked_sub(1)
            .ok_or(RuntimeError::Invariant(
                "Promise aggregate element counter underflowed",
            ))?;
        remaining.set(count);
        if count != 0 {
            return Ok(Completion::Return(Value::Undefined));
        }

        let argument = match terminal {
            AggregateTerminal::ResolveValues => Value::Object(values.clone()),
            AggregateTerminal::RejectAggregate => {
                Value::Object(self.new_internal_aggregate_error(realm, values.clone())?)
            }
        };
        let settle = ObjectRef::from_borrowed_handle(self.clone(), settle)?;
        let settle = self.as_callable(&settle)?.ok_or(RuntimeError::Invariant(
            "Promise aggregate final settlement function was no longer callable",
        ))?;
        match self.call_internal(realm, &settle, Value::Undefined, &[argument])? {
            Completion::Return(_) => Ok(Completion::Return(Value::Undefined)),
            Completion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }
}
