//! Promise convenience constructors and the callback-free race combinator.
//!
//! These paths deliberately preserve pinned QuickJS's ordering: capability
//! creation precedes every observable callback, post-capability abrupt
//! completions reject through that capability, and `race` closes an acquired
//! iterator only after failures in `constructor.resolve` or the dynamic
//! `then` invocation. Iterator-step failures themselves do not close it.

use super::*;
use crate::runtime::intrinsics::object::ObjectIteratorStep;

impl Runtime {
    pub(super) fn call_promise_with_resolvers(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.withResolvers received a constructor invocation",
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
            match self.constructor_from_value(realm, Value::Object(constructor_object))? {
                NativeConversion::Value(constructor) => constructor,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let capability = match self.new_promise_capability(realm, Some(&constructor))? {
            NativeConversion::Value(capability) => capability,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let RootedPromiseCapability {
            promise,
            resolve,
            reject,
        } = capability;
        let result = self.new_ordinary_object_in_realm(realm)?;
        for (name, value) in [
            ("promise", Value::Object(promise)),
            ("resolve", Value::Object(resolve.as_object().clone())),
            ("reject", Value::Object(reject.as_object().clone())),
        ] {
            let key = self.intern_property_key(name)?;
            if !self.define_own_property(
                &result,
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
                    "fresh Promise.withResolvers result rejected a data property",
                ));
            }
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    pub(super) fn call_promise_try(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.try received a constructor invocation",
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
            match self.constructor_from_value(realm, Value::Object(constructor_object))? {
                NativeConversion::Value(constructor) => constructor,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let capability = match self.new_promise_capability(realm, Some(&constructor))? {
            NativeConversion::Value(capability) => capability,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let callback = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise.try callback argv was not padded",
            ))?;
        let callback_completion = match callback {
            Value::Object(callback) => match self.as_callable(&callback)? {
                Some(callback) => {
                    let end = arguments.actual_arg_count.max(1);
                    self.call_internal(
                        realm,
                        &callback,
                        Value::Undefined,
                        &arguments.readable[1..end],
                    )?
                }
                None => Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?),
            },
            _ => Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?),
        };
        let (target, value) = match callback_completion {
            Completion::Return(value) => (&capability.resolve, value),
            Completion::Throw(value) => (&capability.reject, value),
        };
        match self.call_internal(realm, target, Value::Undefined, &[value])? {
            Completion::Return(_) => Ok(Completion::Return(Value::Object(capability.promise))),
            Completion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    pub(super) fn call_promise_race(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.race received a constructor invocation",
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
                "Promise.race iterable argv was not padded",
            ))?;
        let (iterator, next_method) = match self.promise_iterator_record(realm, iterable)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => {
                return self.reject_promise_capability(realm, &capability, value);
            }
        };

        loop {
            let item = match self.object_iterator_next(realm, &iterator, next_method.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
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
            let then_completion = self.invoke_promise_then(
                realm,
                next_promise,
                &[
                    Value::Object(capability.resolve.as_object().clone()),
                    Value::Object(capability.reject.as_object().clone()),
                ],
            )?;
            if let Completion::Throw(value) = then_completion {
                self.close_iterator_preserving_throw(realm, &iterator)?;
                return self.reject_promise_capability(realm, &capability, value);
            }
        }
    }

    fn promise_callable(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<CallableRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        match self.as_callable(&object)? {
            Some(callable) => Ok(NativeConversion::Value(callable)),
            None => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?)),
        }
    }

    fn promise_iterator_record(
        &self,
        realm: ContextId,
        iterable: Value,
    ) -> Result<NativeConversion<(ObjectRef, Value)>, RuntimeError> {
        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        let iterator_method =
            match self.get_value_property_in_realm(realm, iterable.clone(), &iterator_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
        let iterator_method = match self.promise_callable(realm, iterator_method)? {
            NativeConversion::Value(callable) => callable,
            NativeConversion::Throw(_) => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "value is not iterable",
                )?));
            }
        };
        let iterator = match self.call_internal(realm, &iterator_method, iterable, &[])? {
            Completion::Return(Value::Object(iterator)) => iterator,
            Completion::Return(_) => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            }
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let next_key = self.intern_property_key("next")?;
        let next_method = match self.get_property_in_realm(realm, &iterator, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        Ok(NativeConversion::Value((iterator, next_method)))
    }

    fn invoke_promise_then(
        &self,
        realm: ContextId,
        receiver: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let then_key = self.intern_property_key("then")?;
        let then = match self.get_value_property_in_realm(realm, receiver.clone(), &then_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let then = match self.promise_callable(realm, then)? {
            NativeConversion::Value(callable) => callable,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.call_internal(realm, &then, receiver, arguments)
    }

    fn reject_promise_capability(
        &self,
        realm: ContextId,
        capability: &RootedPromiseCapability,
        reason: Value,
    ) -> Result<Completion, RuntimeError> {
        match self.call_internal(realm, &capability.reject, Value::Undefined, &[reason])? {
            Completion::Return(_) => Ok(Completion::Return(Value::Object(
                capability.promise.clone(),
            ))),
            Completion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }
}
