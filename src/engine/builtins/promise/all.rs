//! QuickJS's shared `Promise.all`/`allSettled`/`any` aggregate loop.
//!
//! The entry algorithm is shared, but each callback family keeps a distinct
//! typed heap capture. In particular, pinned QuickJS copies `allSettled`'s
//! fulfill and reject CFunctionData records, so their first-call bits are
//! deliberately independent rather than one shared specification record.

use std::{cell::Cell, rc::Rc};

use super::*;

#[derive(Clone, Copy)]
enum AggregateTerminal {
    ResolveValues,
    RejectAggregate,
}

impl Runtime {
    pub(crate) fn call_promise_aggregate(
        &self,
        kind: PromiseNativeKind,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::aggregate(self, realm, kind, &invocation, arguments)?
            .finish(self, realm)
    }

    pub(super) fn prepare_promise_aggregate_handlers(
        &self,
        realm: ContextId,
        kind: PromiseNativeKind,
        values: &ObjectRef,
        capability: &RootedPromiseCapability,
        remaining: &Rc<Cell<i32>>,
        index: u32,
    ) -> Result<NativeConversion<[Value; 2]>, RuntimeError> {
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
                    return Ok(NativeConversion::Throw(value));
                }
                [
                    Value::Object(capability.resolve.as_object().clone()),
                    Value::Object(reject_element.as_object().clone()),
                ]
            }
            _ => unreachable!("aggregate selector was validated above"),
        };
        Ok(NativeConversion::Value(then_arguments))
    }

    pub(crate) fn call_promise_all_resolve_element(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.prepare_promise_all_resolve_element(realm, invocation, arguments)?
            .finish(self, realm)
    }

    pub(crate) fn prepare_promise_all_resolve_element(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<operation::PromiseStep, RuntimeError> {
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
            return Ok(operation::PromiseStep::Complete(Completion::Return(
                Value::Undefined,
            )));
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

    pub(crate) fn call_promise_all_settled_element(
        &self,
        target_outcome: PromiseReactionKind,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.prepare_promise_all_settled_element(target_outcome, realm, invocation, arguments)?
            .finish(self, realm)
    }

    pub(crate) fn prepare_promise_all_settled_element(
        &self,
        target_outcome: PromiseReactionKind,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<operation::PromiseStep, RuntimeError> {
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
            return Ok(operation::PromiseStep::Complete(Completion::Return(
                Value::Undefined,
            )));
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

    pub(crate) fn call_promise_any_reject_element(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.prepare_promise_any_reject_element(realm, invocation, arguments)?
            .finish(self, realm)
    }

    pub(crate) fn prepare_promise_any_reject_element(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<operation::PromiseStep, RuntimeError> {
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
            return Ok(operation::PromiseStep::Complete(Completion::Return(
                Value::Undefined,
            )));
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
    ) -> Result<operation::PromiseStep, RuntimeError> {
        let values = ObjectRef::from_borrowed_handle(self.clone(), values)?;
        if let Some(value) =
            self.define_array_data_property_without_throw(realm, &values, index, value)?
        {
            return Ok(operation::PromiseStep::Complete(Completion::Throw(value)));
        }

        let count = remaining
            .get()
            .checked_sub(1)
            .ok_or(RuntimeError::Invariant(
                "Promise aggregate element counter underflowed",
            ))?;
        remaining.set(count);
        if count != 0 {
            return Ok(operation::PromiseStep::Complete(Completion::Return(
                Value::Undefined,
            )));
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
        Ok(operation::PromiseStep::ignore_return(
            realm, settle, argument,
        ))
    }
}
