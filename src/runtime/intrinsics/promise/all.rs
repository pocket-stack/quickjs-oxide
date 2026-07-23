//! `Promise.all` and its typed resolve-element callback.
//!
//! Pinned QuickJS represents every element callback with
//! `JS_NewCFunctionData`: each callback has its own first-call bit and index,
//! while all callbacks share the output Array, final resolve function, and a
//! remaining-elements counter initialized with one sentinel reference.

use std::{cell::Cell, rc::Rc};

use super::*;
use crate::runtime::intrinsics::object::ObjectIteratorStep;

impl Runtime {
    pub(super) fn call_promise_all(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.all received a constructor invocation",
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
                "Promise.all iterable argv was not padded",
            ))?;
        let (iterator, next_method) = match self.promise_iterator_record(realm, iterable)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => {
                return self.reject_promise_capability(realm, &capability, value);
            }
        };

        let values = self.new_array(realm)?;
        let remaining = Rc::new(Cell::new(1_u32));
        let mut index = 0_u32;
        loop {
            let item = match self.object_iterator_next(realm, &iterator, next_method.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    let count = remaining
                        .get()
                        .checked_sub(1)
                        .ok_or(RuntimeError::Invariant(
                            "Promise.all remaining-elements sentinel was already consumed",
                        ))?;
                    remaining.set(count);
                    if count == 0 {
                        match self.call_internal(
                            realm,
                            &capability.resolve,
                            Value::Undefined,
                            &[Value::Object(values)],
                        )? {
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
            let Some(count) = remaining.get().checked_add(1) else {
                let value = self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "too many Promise.all elements",
                )?;
                self.close_iterator_preserving_throw(realm, &iterator)?;
                return self.reject_promise_capability(realm, &capability, value);
            };
            remaining.set(count);

            let then_completion = self.invoke_promise_then(
                realm,
                next_promise,
                &[
                    Value::Object(resolve_element.as_object().clone()),
                    Value::Object(capability.reject.as_object().clone()),
                ],
            )?;
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
                        "too many Promise.all elements",
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

        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise.all resolve-element argv was not padded",
            ))?;
        let values = ObjectRef::from_borrowed_handle(self.clone(), values)?;
        if let Some(value) = self.create_array_data_property(realm, &values, index, value)? {
            return Ok(Completion::Throw(value));
        }

        let count = remaining
            .get()
            .checked_sub(1)
            .ok_or(RuntimeError::Invariant(
                "Promise.all resolve-element observed an exhausted counter",
            ))?;
        remaining.set(count);
        if count != 0 {
            return Ok(Completion::Return(Value::Undefined));
        }

        let resolve = ObjectRef::from_borrowed_handle(self.clone(), resolve)?;
        let resolve = self.as_callable(&resolve)?.ok_or(RuntimeError::Invariant(
            "Promise.all final resolve was no longer callable",
        ))?;
        match self.call_internal(realm, &resolve, Value::Undefined, &[Value::Object(values)])? {
            Completion::Return(_) => Ok(Completion::Return(Value::Undefined)),
            Completion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }
}
