//! Object constructor and prototype intrinsics.

use super::super::*;

#[cfg(test)]
mod tests;

enum ObjectGroupByIteratorStep {
    Yield(Value),
    Done,
    Throw(Value),
}

impl Runtime {
    /// QuickJS `js_object_groupBy(..., is_map = 0)`.
    ///
    /// The upstream routine deliberately closes the iterator only after an
    /// abrupt callback, property-key conversion, or element-count check. An
    /// abrupt iterator step or group-Array append takes the ordinary exception
    /// exit instead, so those branches must remain separate here.
    pub(in crate::runtime) fn call_object_group_by(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

        self.call_object_group_by_with_element_limit(realm, invocation, arguments, MAX_SAFE_INTEGER)
    }

    fn call_object_group_by_with_element_limit(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        element_limit: u64,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.groupBy did not receive a generic invocation",
            ));
        };

        // Pinned QuickJS checks the callback before it performs any operation
        // on the iterable.
        let callback = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "Object.groupBy callback argv was not padded",
        ))?;
        let Value::Object(callback) = callback else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(callback) = self.as_callable(callback)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object.groupBy iterable argv was not padded",
            ))?;
        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        let iterator_method = match &iterable {
            Value::Null | Value::Undefined => {
                let base = if matches!(iterable, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    &format!("cannot read property 'Symbol.iterator' of {base}"),
                )?));
            }
            _ => match self.get_value_property_in_realm(realm, iterable.clone(), &iterator_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            },
        };
        let Value::Object(iterator_method) = iterator_method else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "value is not iterable",
            )?));
        };
        let Some(iterator_method) = self.as_callable(&iterator_method)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "value is not iterable",
            )?));
        };
        let iterator = match self.call_internal(realm, &iterator_method, iterable, &[])? {
            Completion::Return(Value::Object(iterator)) => iterator,
            Completion::Return(_) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        // Cache `next` once before allocating the result, matching the exact
        // JS_GetIterator + Get(next) ordering in js_object_groupBy.
        let next_key = self.intern_property_key("next")?;
        let next_method = match self.get_property_in_realm(realm, &iterator, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let groups = self.new_object(None)?;
        let callback_this = Value::Object(self.global_object_for_realm(realm)?);

        let mut index = 0_u64;
        loop {
            if index >= element_limit {
                let exception =
                    self.new_native_error(realm, NativeErrorKind::Type, "too many elements")?;
                self.close_iterator_preserving_throw(realm, &iterator)?;
                return Ok(Completion::Throw(exception));
            }

            let value =
                match self.object_group_by_iterator_next(realm, &iterator, next_method.clone())? {
                    ObjectGroupByIteratorStep::Yield(value) => value,
                    ObjectGroupByIteratorStep::Done => {
                        return Ok(Completion::Return(Value::Object(groups)));
                    }
                    ObjectGroupByIteratorStep::Throw(value) => {
                        // IteratorNext failures use QuickJS's plain exception exit
                        // and therefore do not perform IteratorClose.
                        return Ok(Completion::Throw(value));
                    }
                };

            let key_value = match self.call_internal(
                realm,
                &callback,
                callback_this.clone(),
                &[value.clone(), Value::number(index as f64)],
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            };
            let key = match self.native_to_property_key(realm, key_value)? {
                NativeConversion::Value(key) => key,
                NativeConversion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            };

            let group = match self.get_property_in_realm(realm, &groups, &key)? {
                Completion::Return(Value::Undefined) => {
                    let group = self.new_array(realm)?;
                    match self.define_own_property_in_realm(
                        Some(realm),
                        &groups,
                        &key,
                        &OrdinaryPropertyDescriptor {
                            value: DescriptorField::Present(Value::Object(group.clone())),
                            writable: DescriptorField::Present(true),
                            enumerable: DescriptorField::Present(true),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                    )? {
                        PropertyDefineOutcome::Defined(true) => group,
                        PropertyDefineOutcome::Defined(false) => {
                            return Err(RuntimeError::Invariant(
                                "fresh Object.groupBy result rejected a group property",
                            ));
                        }
                        PropertyDefineOutcome::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                }
                Completion::Return(Value::Object(group)) => group,
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "Object.groupBy result contained a non-Array group",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };

            // Upstream calls js_array_push directly. Reuse the matching kernel
            // rather than CreateDataProperty: mutation of Array.prototype can
            // make an inherited index setter or a rejected Set observable.
            let push_arguments = NativeArguments {
                actual_arg_count: 1,
                readable: vec![value],
            };
            match self.call_array_prototype_push(
                realm,
                ArrayPushKind::Push,
                NativeInvocation::Call {
                    this_value: Value::Object(group),
                },
                &push_arguments,
            )? {
                Completion::Return(_) => {}
                Completion::Throw(value) => {
                    // js_object_groupBy does not close the iterator when its
                    // internal js_array_push fails.
                    return Ok(Completion::Throw(value));
                }
            }

            index = index.checked_add(1).ok_or(RuntimeError::Invariant(
                "Object.groupBy index overflowed Uint64",
            ))?;
        }
    }

    fn object_group_by_iterator_next(
        &self,
        realm: ContextId,
        iterator: &ObjectRef,
        next_method: Value,
    ) -> Result<ObjectGroupByIteratorStep, RuntimeError> {
        let Value::Object(next_method) = next_method else {
            return Ok(ObjectGroupByIteratorStep::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(next_method) = self.as_callable(&next_method)? else {
            return Ok(ObjectGroupByIteratorStep::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let result = match self
            .try_call_native_iterator_next_raw(&next_method, Value::Object(iterator.clone()))?
        {
            Some(NativeInvokeOutcome::IteratorNextRaw { value, done }) => {
                return Ok(if done {
                    ObjectGroupByIteratorStep::Done
                } else {
                    ObjectGroupByIteratorStep::Yield(value)
                });
            }
            Some(NativeInvokeOutcome::Completion(Completion::Throw(value))) => {
                return Ok(ObjectGroupByIteratorStep::Throw(value));
            }
            Some(NativeInvokeOutcome::Completion(Completion::Return(result))) => result,
            None => match self.call_internal(
                realm,
                &next_method,
                Value::Object(iterator.clone()),
                &[],
            )? {
                Completion::Return(result) => result,
                Completion::Throw(value) => {
                    return Ok(ObjectGroupByIteratorStep::Throw(value));
                }
            },
        };
        let Value::Object(result) = result else {
            return Ok(ObjectGroupByIteratorStep::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "iterator must return an object",
            )?));
        };

        let done_key = self.intern_property_key("done")?;
        let done = match self.get_property_in_realm(realm, &result, &done_key)? {
            Completion::Return(value) => value.to_boolean(),
            Completion::Throw(value) => return Ok(ObjectGroupByIteratorStep::Throw(value)),
        };
        if done {
            return Ok(ObjectGroupByIteratorStep::Done);
        }

        let value_key = self.intern_property_key("value")?;
        match self.get_property_in_realm(realm, &result, &value_key)? {
            Completion::Return(value) => Ok(ObjectGroupByIteratorStep::Yield(value)),
            Completion::Throw(value) => Ok(ObjectGroupByIteratorStep::Throw(value)),
        }
    }
}
