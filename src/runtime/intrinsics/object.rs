//! Object constructor and prototype intrinsics.

use super::super::*;

#[cfg(test)]
mod tests;

pub(in crate::runtime) enum ObjectIteratorStep {
    Yield(Value),
    Done,
    Throw(Value),
}

impl Runtime {
    /// QuickJS `JS_ToPrimitive(..., HINT_FORCE_ORDINARY)`: probe the ordinary
    /// conversion methods without consulting `Symbol.toPrimitive`. Date's
    /// standard exotic method delegates here after translating its hint.
    pub(in crate::runtime) fn ordinary_to_primitive(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        hint: ToPrimitiveHint,
    ) -> Result<Completion, RuntimeError> {
        let methods = match hint {
            ToPrimitiveHint::String => ["toString", "valueOf"],
            ToPrimitiveHint::Number | ToPrimitiveHint::Default => ["valueOf", "toString"],
        };
        for name in methods {
            let key = self.intern_property_key(name)?;
            let method = match self.get_property_in_realm(realm, object, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let Value::Object(method_object) = method else {
                continue;
            };
            let Some(method) = self.as_callable(&method_object)? else {
                continue;
            };
            match self.call_internal(realm, &method, Value::Object(object.clone()), &[])? {
                Completion::Return(Value::Object(_)) => {}
                completion => return Ok(completion),
            }
        }
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            "toPrimitive",
        )?))
    }

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

            let value = match self.object_iterator_next(realm, &iterator, next_method.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(groups)));
                }
                ObjectIteratorStep::Throw(value) => {
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

    /// QuickJS `js_object_fromEntries`.
    ///
    /// Unlike `Object.groupBy`, the pinned implementation allocates its
    /// defining-realm result before touching the iterable and closes an
    /// acquired iterator after every subsequent abrupt completion, including
    /// `next` lookup and iterator-step failures. `JS_IteratorClose(..., TRUE)`
    /// always restores the original pending exception, so close failures are
    /// deliberately ignored by `close_iterator_preserving_throw`.
    pub(in crate::runtime) fn call_object_from_entries(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.fromEntries did not receive a generic invocation",
            ));
        };

        let result = self.new_ordinary_object_in_realm(realm)?;
        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object.fromEntries iterable argv was not padded",
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

        let next_key = self.intern_property_key("next")?;
        let next_method = match self.get_property_in_realm(realm, &iterator, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                self.close_iterator_preserving_throw(realm, &iterator)?;
                return Ok(Completion::Throw(value));
            }
        };
        let zero_key = self.intern_property_key("0")?;
        let one_key = self.intern_property_key("1")?;

        loop {
            let item = match self.object_iterator_next(realm, &iterator, next_method.clone())? {
                ObjectIteratorStep::Yield(Value::Object(item)) => item,
                ObjectIteratorStep::Yield(_) => {
                    let exception =
                        self.new_native_error(realm, NativeErrorKind::Type, "not an object")?;
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(exception));
                }
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(result)));
                }
                ObjectIteratorStep::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            };

            let key_value = match self.get_property_in_realm(realm, &item, &zero_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            };
            let value = match self.get_property_in_realm(realm, &item, &one_key)? {
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
            let descriptor = OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            };
            if let Some(value) = self.define_property_or_throw(realm, &result, &key, &descriptor)? {
                self.close_iterator_preserving_throw(realm, &iterator)?;
                return Ok(Completion::Throw(value));
            }
        }
    }

    /// QuickJS `js_object_hasOwn`.
    ///
    /// The static method converts its target before its key (the reverse of
    /// `Object.prototype.hasOwnProperty`) and then performs the descriptor-free
    /// `JS_GetOwnPropertyInternal` presence check. The local property kernel
    /// preserves that check without materializing AutoInit payloads.
    pub(in crate::runtime) fn call_object_has_own(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.hasOwn did not receive a generic invocation",
            ));
        };
        let target = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object.hasOwn target argv was not padded",
            ))?;
        let object = match self.native_to_object(realm, target)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = match self.native_to_property_key(
            realm,
            arguments
                .readable
                .get(1)
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Object.hasOwn key argv was not padded",
                ))?,
        )? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Bool(
            self.has_own_property(&object, &key)?,
        )))
    }

    pub(in crate::runtime) fn object_iterator_next(
        &self,
        realm: ContextId,
        iterator: &ObjectRef,
        next_method: Value,
    ) -> Result<ObjectIteratorStep, RuntimeError> {
        let Value::Object(next_method) = next_method else {
            return Ok(ObjectIteratorStep::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(next_method) = self.as_callable(&next_method)? else {
            return Ok(ObjectIteratorStep::Throw(self.new_native_error(
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
                    ObjectIteratorStep::Done
                } else {
                    ObjectIteratorStep::Yield(value)
                });
            }
            Some(NativeInvokeOutcome::Completion(Completion::Throw(value))) => {
                return Ok(ObjectIteratorStep::Throw(value));
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
                    return Ok(ObjectIteratorStep::Throw(value));
                }
            },
        };
        let Value::Object(result) = result else {
            return Ok(ObjectIteratorStep::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "iterator must return an object",
            )?));
        };

        let done_key = self.intern_property_key("done")?;
        let done = match self.get_property_in_realm(realm, &result, &done_key)? {
            Completion::Return(value) => value.to_boolean(),
            Completion::Throw(value) => return Ok(ObjectIteratorStep::Throw(value)),
        };
        if done {
            return Ok(ObjectIteratorStep::Done);
        }

        let value_key = self.intern_property_key("value")?;
        match self.get_property_in_realm(realm, &result, &value_key)? {
            Completion::Return(value) => Ok(ObjectIteratorStep::Yield(value)),
            Completion::Throw(value) => Ok(ObjectIteratorStep::Throw(value)),
        }
    }
}

impl Runtime {
    pub(in crate::runtime) fn initialize_object_prototype_intrinsics(
        &self,
        realm: ContextId,
        object_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        for (target, name, length, min_readable_args) in [
            (NativeFunctionId::ObjectPrototypeToString, "toString", 0, 0),
            (
                NativeFunctionId::ObjectPrototypeToLocaleString,
                "toLocaleString",
                0,
                0,
            ),
            (NativeFunctionId::ObjectPrototypeValueOf, "valueOf", 0, 0),
            (
                NativeFunctionId::ObjectPrototypeHasOwnProperty,
                "hasOwnProperty",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectPrototypeIsPrototypeOf,
                "isPrototypeOf",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectPrototypePropertyIsEnumerable,
                "propertyIsEnumerable",
                1,
                1,
            ),
        ] {
            self.define_native_builtin_auto_init(
                object_prototype,
                realm,
                target,
                name,
                length,
                min_readable_args,
            )?;
        }

        let function_prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .function_prototype;
        let function_prototype = ObjectRef::from_borrowed_handle(self.clone(), function_prototype)?;
        let getter = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::ObjectPrototypeProtoGetter,
            0,
            "get __proto__",
            0,
        )?;
        let setter = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::ObjectPrototypeProtoSetter,
            1,
            "set __proto__",
            1,
        )?;
        let proto = self.intern_property_key("__proto__")?;
        if !self.define_own_property(
            object_prototype,
            &proto,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(getter)),
                set: DescriptorField::Present(AccessorValue::Callable(setter)),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Object.prototype __proto__ definition was rejected",
            ));
        }

        for (target, name, length, min_readable_args) in [
            (
                NativeFunctionId::ObjectPrototypeDefineAccessor(ObjectAccessorKind::Getter),
                "__defineGetter__",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectPrototypeDefineAccessor(ObjectAccessorKind::Setter),
                "__defineSetter__",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectPrototypeLookupAccessor(ObjectAccessorKind::Getter),
                "__lookupGetter__",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectPrototypeLookupAccessor(ObjectAccessorKind::Setter),
                "__lookupSetter__",
                1,
                1,
            ),
        ] {
            self.define_native_builtin_auto_init(
                object_prototype,
                realm,
                target,
                name,
                length,
                min_readable_args,
            )?;
        }
        Ok(())
    }

    pub(in crate::runtime) fn initialize_object_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ObjectConstructor,
            1,
            "Object",
            1,
        )?;
        for (target, name, length, min_readable_args) in [
            (NativeFunctionId::ObjectCreate, "create", 2, 2),
            (
                NativeFunctionId::ObjectGetPrototypeOf,
                "getPrototypeOf",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectSetPrototypeOf,
                "setPrototypeOf",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectDefineProperty,
                "defineProperty",
                3,
                3,
            ),
            (
                NativeFunctionId::ObjectDefineProperties,
                "defineProperties",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind::Names),
                "getOwnPropertyNames",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind::Symbols),
                "getOwnPropertySymbols",
                1,
                1,
            ),
            (NativeFunctionId::ObjectGroupBy, "groupBy", 2, 2),
            (
                NativeFunctionId::ObjectKeys(ObjectKeysKind::Keys),
                "keys",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectKeys(ObjectKeysKind::Values),
                "values",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectKeys(ObjectKeysKind::Entries),
                "entries",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectExtensibility(ObjectExtensibilityKind::IsExtensible),
                "isExtensible",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectExtensibility(ObjectExtensibilityKind::PreventExtensions),
                "preventExtensions",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectGetOwnPropertyDescriptor,
                "getOwnPropertyDescriptor",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectGetOwnPropertyDescriptors,
                "getOwnPropertyDescriptors",
                1,
                1,
            ),
            (NativeFunctionId::ObjectIs, "is", 2, 2),
            (NativeFunctionId::ObjectAssign, "assign", 2, 2),
            (
                NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::Seal),
                "seal",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::Freeze),
                "freeze",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::IsSealed),
                "isSealed",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::IsFrozen),
                "isFrozen",
                1,
                1,
            ),
            (NativeFunctionId::ObjectFromEntries, "fromEntries", 1, 1),
            (NativeFunctionId::ObjectHasOwn, "hasOwn", 2, 2),
        ] {
            self.define_native_builtin_auto_init(
                constructor.as_object(),
                realm,
                target,
                name,
                length,
                min_readable_args,
            )?;
        }
        self.define_function_data_property(
            global_object,
            "Object",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, object_prototype)
    }

    fn object_to_string_tag(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        let default_tag = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            match &object_data.payload {
                ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. } => JsString::from_static("Function"),
                ObjectPayload::Error => JsString::from_static("Error"),
                ObjectPayload::Primitive(PrimitiveObjectData::Number(_)) => {
                    JsString::from_static("Number")
                }
                ObjectPayload::Primitive(PrimitiveObjectData::String(_)) => {
                    JsString::from_static("String")
                }
                ObjectPayload::Primitive(PrimitiveObjectData::Boolean(_)) => {
                    JsString::from_static("Boolean")
                }
                // QuickJS's built-in class fallback has no Symbol- or
                // BigInt-wrapper case. Their standard tags come exclusively
                // from inherited configurable @@toStringTag properties.
                ObjectPayload::Primitive(
                    PrimitiveObjectData::Symbol(_) | PrimitiveObjectData::BigInt(_),
                ) => JsString::from_static("Object"),
                ObjectPayload::Array { .. } => JsString::from_static("Array"),
                ObjectPayload::Arguments { .. } => JsString::from_static("Arguments"),
                ObjectPayload::Date(_) => JsString::from_static("Date"),
                ObjectPayload::Ordinary
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::GlobalObject { .. } => JsString::from_static("Object"),
                ObjectPayload::ArrayIterator { .. } | ObjectPayload::StringIterator { .. } => {
                    JsString::from_static("Object")
                }
            }
        };
        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        match self.get_property_in_realm(realm, object, &to_string_tag)? {
            Completion::Return(Value::String(tag)) => Ok(NativeConversion::Value(tag)),
            Completion::Return(_) => Ok(NativeConversion::Value(default_tag)),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(in crate::runtime) fn call_object_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.toString did not receive a generic invocation",
            ));
        };
        let tag = match this_value {
            Value::Undefined => JsString::from_static("Undefined"),
            Value::Null => JsString::from_static("Null"),
            Value::Bool(value) => {
                let prototype =
                    self.primitive_prototype_for_realm(realm, PrimitiveKind::Boolean)?;
                let object = self.new_primitive_object(
                    &prototype,
                    PrimitiveKind::Boolean,
                    Value::Bool(value),
                )?;
                match self.object_to_string_tag(realm, &object)? {
                    NativeConversion::Value(tag) => tag,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
            value @ (Value::Int(_) | Value::Float(_)) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::Number)?;
                let object = self.new_primitive_object(&prototype, PrimitiveKind::Number, value)?;
                match self.object_to_string_tag(realm, &object)? {
                    NativeConversion::Value(tag) => tag,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
            value @ Value::BigInt(_) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::BigInt)?;
                let object = self.new_primitive_object(&prototype, PrimitiveKind::BigInt, value)?;
                match self.object_to_string_tag(realm, &object)? {
                    NativeConversion::Value(tag) => tag,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
            value @ Value::Symbol(_) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::Symbol)?;
                let object = self.new_primitive_object(&prototype, PrimitiveKind::Symbol, value)?;
                match self.object_to_string_tag(realm, &object)? {
                    NativeConversion::Value(tag) => tag,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
            value @ Value::String(_) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::String)?;
                let object = self.new_primitive_object(&prototype, PrimitiveKind::String, value)?;
                match self.object_to_string_tag(realm, &object)? {
                    NativeConversion::Value(tag) => tag,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
            Value::Object(object) => match self.object_to_string_tag(realm, &object)? {
                NativeConversion::Value(tag) => tag,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
        };
        let result = JsString::from_static("[object ")
            .try_concat(&tag)?
            .try_concat(&JsString::from_static("]"))?;
        Ok(Completion::Return(Value::String(result)))
    }

    pub(in crate::runtime) fn call_object_prototype_to_locale_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.toLocaleString did not receive a generic invocation",
            ));
        };
        if matches!(this_value, Value::Null | Value::Undefined) {
            let message = if matches!(this_value, Value::Null) {
                "cannot read property 'toString' of null"
            } else {
                "cannot read property 'toString' of undefined"
            };
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                message,
            )?));
        }
        let to_string = self.intern_property_key("toString")?;
        let method =
            match self.get_value_property_in_realm(realm, this_value.clone(), &to_string)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let Value::Object(method) = method else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(method) = self.as_callable(&method)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        self.call_internal(realm, &method, this_value, &[])
    }

    pub(in crate::runtime) fn call_object_prototype_value_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.valueOf did not receive a generic invocation",
            ));
        };
        match this_value {
            value @ Value::Object(_) => Ok(Completion::Return(value)),
            Value::Undefined | Value::Null => Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot convert to object",
            )?)),
            value @ Value::Bool(_) => {
                let prototype =
                    self.primitive_prototype_for_realm(realm, PrimitiveKind::Boolean)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::Boolean, value)?,
                )))
            }
            value @ (Value::Int(_) | Value::Float(_)) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::Number)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::Number, value)?,
                )))
            }
            value @ Value::String(_) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::String)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::String, value)?,
                )))
            }
            value @ Value::BigInt(_) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::BigInt)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::BigInt, value)?,
                )))
            }
            value @ Value::Symbol(_) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::Symbol)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::Symbol, value)?,
                )))
            }
        }
    }

    pub(in crate::runtime) fn call_object_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object constructor did not receive constructor-or-function invocation",
            ));
        };
        let active = self.active_function()?;
        if let Value::Object(new_target) = &new_target
            && new_target != &active
        {
            let new_target = self.callable_from_value(Value::Object(new_target.clone()))?;
            return self.create_from_constructor(realm, &new_target);
        }
        if !matches!(new_target, Value::Undefined | Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "Object constructor new.target was neither undefined nor an object",
            ));
        }
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Object constructor argv was not padded",
        ))?;
        if matches!(argument, Value::Undefined | Value::Null) {
            let prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
            let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
            return Ok(Completion::Return(Value::Object(
                self.new_object(Some(&prototype))?,
            )));
        }
        match self.native_to_object(realm, argument.clone())? {
            NativeConversion::Value(object) => Ok(Completion::Return(Value::Object(object))),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    pub(in crate::runtime) fn call_object_create(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.create did not receive a generic invocation",
            ));
        };
        let prototype = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Object.create prototype argv was not padded",
        ))?;
        let object = match prototype {
            Value::Object(prototype) => self.new_object(Some(prototype))?,
            Value::Null => self.new_object(None)?,
            _ => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a prototype",
                )?));
            }
        };
        let properties = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "Object.create properties argv was not padded",
        ))?;
        if !matches!(properties, Value::Undefined)
            && let Some(value) =
                self.object_define_properties(realm, &object, properties.clone())?
        {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(object)))
    }

    pub(in crate::runtime) fn call_object_get_prototype_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.getPrototypeOf did not receive a generic invocation",
            ));
        };
        let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Object.getPrototypeOf argv was not padded",
        ))?;
        if matches!(value, Value::Null | Value::Undefined) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        }
        let object = match self.native_to_object(realm, value.clone())? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(
            self.get_prototype_of(&object)?
                .map_or(Value::Null, Value::Object),
        ))
    }

    fn set_prototype_or_throw(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        prototype: Option<&ObjectRef>,
    ) -> Result<Option<Value>, RuntimeError> {
        if self.set_prototype_of(object, prototype)? {
            return Ok(None);
        }
        let (immutable, extensible) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object.object_id())?;
            (object.immutable_prototype, object.extensible)
        };
        let message = if immutable {
            "prototype is immutable"
        } else if !extensible {
            "object is not extensible"
        } else {
            "circular prototype chain"
        };
        Ok(Some(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            message,
        )?))
    }

    pub(in crate::runtime) fn call_object_set_prototype_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.setPrototypeOf did not receive a generic invocation",
            ));
        };
        let target = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Object.setPrototypeOf target argv was not padded",
        ))?;
        if matches!(target, Value::Undefined | Value::Null) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        }
        let prototype = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "Object.setPrototypeOf prototype argv was not padded",
        ))?;
        let prototype = match prototype {
            Value::Object(prototype) => Some(prototype),
            Value::Null => None,
            _ => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            }
        };
        let Value::Object(target_object) = target else {
            return Ok(Completion::Return(target.clone()));
        };
        if let Some(value) = self.set_prototype_or_throw(realm, target_object, prototype)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(target.clone()))
    }

    fn property_define_rejection(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Value, RuntimeError> {
        if let ArrayOwnKey::Index(index) = self.array_own_key(object, key)? {
            let (length, writable) = self.array_length_state(object)?;
            if index >= length && !writable {
                let length = self.intern_property_key("length")?;
                let error =
                    self.native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")?;
                return self.new_native_error_from_error(realm, NativeErrorKind::Type, &error);
            }
        }
        let message = if !self.has_own_property(object, key)? && !self.is_extensible(object)? {
            "object is not extensible"
        } else {
            "property is not configurable"
        };
        self.new_native_error(realm, NativeErrorKind::Type, message)
    }

    fn define_property_or_throw(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<Option<Value>, RuntimeError> {
        match self.define_own_property_in_realm(Some(realm), object, key, descriptor)? {
            PropertyDefineOutcome::Defined(true) => Ok(None),
            PropertyDefineOutcome::Defined(false) => {
                self.property_define_rejection(realm, object, key).map(Some)
            }
            PropertyDefineOutcome::Throw(value) => Ok(Some(value)),
        }
    }

    pub(in crate::runtime) fn call_object_define_property(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.defineProperty did not receive a generic invocation",
            ));
        };
        let Some(Value::Object(object)) = arguments.readable.first() else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let key = match self.native_to_property_key(
            realm,
            arguments
                .readable
                .get(1)
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Object.defineProperty key argv was not padded",
                ))?,
        )? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let descriptor = match self.native_to_property_descriptor(
            realm,
            arguments
                .readable
                .get(2)
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Object.defineProperty descriptor argv was not padded",
                ))?,
        )? {
            NativeConversion::Value(descriptor) => descriptor,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if let Some(value) = self.define_property_or_throw(realm, object, &key, &descriptor)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(object.clone())))
    }

    fn object_define_properties(
        &self,
        realm: ContextId,
        target: &ObjectRef,
        properties: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let properties = match self.native_to_object(realm, properties)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Some(value)),
        };
        // Pinned QuickJS snapshots enumerable own keys, then immediately
        // converts and defines each descriptor instead of using the spec's
        // two-phase descriptor list.
        let mut keys = Vec::new();
        for key in self.own_property_keys(&properties)? {
            if self.own_property_is_enumerable(&properties, &key)? {
                keys.push(key);
            }
        }
        for key in keys {
            let descriptor = match self.get_property_in_realm(realm, &properties, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Some(value)),
            };
            let descriptor = match self.native_to_property_descriptor(realm, descriptor)? {
                NativeConversion::Value(descriptor) => descriptor,
                NativeConversion::Throw(value) => return Ok(Some(value)),
            };
            if let Some(value) = self.define_property_or_throw(realm, target, &key, &descriptor)? {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    pub(in crate::runtime) fn call_object_define_properties(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.defineProperties did not receive a generic invocation",
            ));
        };
        let Some(Value::Object(target)) = arguments.readable.first() else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let properties = arguments
            .readable
            .get(1)
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object.defineProperties properties argv was not padded",
            ))?;
        if let Some(value) = self.object_define_properties(realm, target, properties)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(target.clone())))
    }

    pub(in crate::runtime) fn call_object_get_own_property_keys(
        &self,
        realm: ContextId,
        kind: ObjectOwnPropertyKeysKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object own-key method did not receive a generic invocation",
            ));
        };
        let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Object own-key argv was not padded",
        ))?;
        let object = match self.native_to_object(realm, value.clone())? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut values = Vec::new();
        for key in self.own_property_keys(&object)? {
            let key_kind = self.0.state.borrow().atoms.property_key_kind(key.atom())?;
            match (kind, key_kind) {
                (ObjectOwnPropertyKeysKind::Names, PropertyKeyKind::String) => {
                    values.push(Value::String(
                        self.0.state.borrow().atoms.to_js_string(key.atom())?,
                    ));
                }
                (ObjectOwnPropertyKeysKind::Symbols, PropertyKeyKind::Symbol) => {
                    values.push(Value::Symbol(SymbolRef::from_borrowed_atom(
                        self.clone(),
                        key.atom(),
                    )?));
                }
                (
                    ObjectOwnPropertyKeysKind::Names,
                    PropertyKeyKind::Symbol | PropertyKeyKind::Private,
                )
                | (
                    ObjectOwnPropertyKeysKind::Symbols,
                    PropertyKeyKind::String | PropertyKeyKind::Private,
                ) => {}
            }
        }
        Ok(Completion::Return(Value::Object(
            self.new_array_from_values(realm, values)?,
        )))
    }

    pub(in crate::runtime) fn call_object_keys(
        &self,
        realm: ContextId,
        kind: ObjectKeysKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object keys method did not receive a generic invocation",
            ));
        };
        let value = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant("Object keys argv was not padded"))?;
        let object = match self.native_to_object(realm, value.clone())? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        // QuickJS snapshots every own string key first, then rechecks the
        // descriptor immediately before emitting each result. A preceding
        // getter may therefore delete or make a later snapshotted key
        // non-enumerable, while newly added keys remain absent.
        let mut keys = Vec::new();
        for key in self.own_property_keys(&object)? {
            if self.0.state.borrow().atoms.property_key_kind(key.atom())? == PropertyKeyKind::String
            {
                keys.push(key);
            }
        }
        let result = self.new_array(realm)?;
        let mut result_index = 0_u32;
        for key in keys {
            let Some(descriptor) = self.get_own_property(&object, &key)? else {
                continue;
            };
            let enumerable = match descriptor {
                CompleteOrdinaryPropertyDescriptor::Data { enumerable, .. }
                | CompleteOrdinaryPropertyDescriptor::Accessor { enumerable, .. } => enumerable,
            };
            if !enumerable {
                continue;
            }

            let value = match kind {
                ObjectKeysKind::Keys => {
                    Value::String(self.0.state.borrow().atoms.to_js_string(key.atom())?)
                }
                ObjectKeysKind::Values => {
                    match self.get_property_in_realm(realm, &object, &key)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                ObjectKeysKind::Entries => {
                    let entry = self.new_array(realm)?;
                    let key_value =
                        Value::String(self.0.state.borrow().atoms.to_js_string(key.atom())?);
                    self.define_fresh_object_keys_array_element(
                        &entry,
                        0,
                        key_value,
                        "fresh Object.entries pair rejected its key",
                    )?;
                    let value = match self.get_property_in_realm(realm, &object, &key)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    self.define_fresh_object_keys_array_element(
                        &entry,
                        1,
                        value,
                        "fresh Object.entries pair rejected its value",
                    )?;
                    Value::Object(entry)
                }
            };
            self.define_fresh_object_keys_array_element(
                &result,
                result_index,
                value,
                "fresh Object keys result rejected an element",
            )?;
            result_index = result_index.checked_add(1).ok_or_else(|| {
                RuntimeError::Engine(Error::new(ErrorKind::Range, "invalid array length"))
            })?;
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    fn define_fresh_object_keys_array_element(
        &self,
        array: &ObjectRef,
        index: u32,
        value: Value,
        rejection: &'static str,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(&index.to_string())?;
        if !self.define_own_property(
            array,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(rejection));
        }
        Ok(())
    }

    pub(in crate::runtime) fn call_object_extensibility(
        &self,
        kind: ObjectExtensibilityKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object extensibility method did not receive a generic invocation",
            ));
        };
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object extensibility argv was not padded",
            ))?;

        // Unlike most Object statics these routines deliberately do not box
        // primitives. This matches QuickJS's initial tag test and preserves
        // the exact primitive for Object.preventExtensions.
        let Value::Object(object) = &value else {
            return Ok(Completion::Return(match kind {
                ObjectExtensibilityKind::IsExtensible => Value::Bool(false),
                ObjectExtensibilityKind::PreventExtensions => value,
            }));
        };
        match kind {
            ObjectExtensibilityKind::IsExtensible => {
                Ok(Completion::Return(Value::Bool(self.is_extensible(object)?)))
            }
            ObjectExtensibilityKind::PreventExtensions => {
                self.prevent_extensions(object)?;
                Ok(Completion::Return(value))
            }
        }
    }

    /// Allocate the ordinary Object produced by QuickJS `OP_object` in the
    /// executing realm, without routing VM allocation through Context's public
    /// convenience layer.
    pub(in crate::runtime) fn new_ordinary_object_in_realm(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        self.new_object(Some(&prototype))
    }

    fn define_fresh_object_descriptor_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        rejection: &'static str,
    ) -> Result<(), RuntimeError> {
        if !self.define_own_property(
            object,
            key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(rejection));
        }
        Ok(())
    }

    fn complete_descriptor_to_object(
        &self,
        realm: ContextId,
        descriptor: CompleteOrdinaryPropertyDescriptor,
    ) -> Result<ObjectRef, RuntimeError> {
        let object = self.new_ordinary_object_in_realm(realm)?;
        let mut fields = Vec::with_capacity(4);
        match descriptor {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => {
                fields.push(("value", value));
                fields.push(("writable", Value::Bool(writable)));
                fields.push(("enumerable", Value::Bool(enumerable)));
                fields.push(("configurable", Value::Bool(configurable)));
            }
            CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => {
                fields.push((
                    "get",
                    get.map_or(Value::Undefined, |value| Value::Object(value.into_object())),
                ));
                fields.push((
                    "set",
                    set.map_or(Value::Undefined, |value| Value::Object(value.into_object())),
                ));
                fields.push(("enumerable", Value::Bool(enumerable)));
                fields.push(("configurable", Value::Bool(configurable)));
            }
        }
        for (name, value) in fields {
            let key = self.intern_property_key(name)?;
            self.define_fresh_object_descriptor_property(
                &object,
                &key,
                value,
                "fresh property descriptor object rejected a field",
            )?;
        }
        Ok(object)
    }

    pub(super) fn object_get_own_property_descriptor_value(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Value, RuntimeError> {
        let Some(descriptor) = self.get_own_property(object, key)? else {
            return Ok(Value::Undefined);
        };
        Ok(Value::Object(
            self.complete_descriptor_to_object(realm, descriptor)?,
        ))
    }

    pub(in crate::runtime) fn call_object_get_own_property_descriptor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.getOwnPropertyDescriptor did not receive a generic invocation",
            ));
        };
        let target = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object.getOwnPropertyDescriptor target argv was not padded",
            ))?;
        let object = match self.native_to_object(realm, target)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = match self.native_to_property_key(
            realm,
            arguments
                .readable
                .get(1)
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Object.getOwnPropertyDescriptor key argv was not padded",
                ))?,
        )? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(
            self.object_get_own_property_descriptor_value(realm, &object, &key)?,
        ))
    }

    pub(super) fn object_property_key_value(
        &self,
        key: &PropertyKey,
    ) -> Result<Value, RuntimeError> {
        let kind = self.0.state.borrow().atoms.property_key_kind(key.atom())?;
        match kind {
            PropertyKeyKind::String => Ok(Value::String(
                self.0.state.borrow().atoms.to_js_string(key.atom())?,
            )),
            PropertyKeyKind::Symbol => Ok(Value::Symbol(SymbolRef::from_borrowed_atom(
                self.clone(),
                key.atom(),
            )?)),
            PropertyKeyKind::Private => Err(RuntimeError::Invariant(
                "private key escaped into Object.getOwnPropertyDescriptors",
            )),
        }
    }

    pub(in crate::runtime) fn call_object_get_own_property_descriptors(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.getOwnPropertyDescriptors did not receive a generic invocation",
            ));
        };
        let target = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object.getOwnPropertyDescriptors argv was not padded",
            ))?;
        let object = match self.native_to_object(realm, target)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let keys = self.own_property_keys(&object)?;
        let result = self.new_ordinary_object_in_realm(realm)?;
        for key in keys {
            // QuickJS routes every snapshotted atom back through the singular
            // helper, so the current descriptor is re-read before publication.
            let key_value = self.object_property_key_value(&key)?;
            let descriptor_key = match self.native_to_property_key(realm, key_value)? {
                NativeConversion::Value(key) => key,
                NativeConversion::Throw(_) => {
                    return Err(RuntimeError::Invariant(
                        "snapshotted property key conversion threw",
                    ));
                }
            };
            let descriptor =
                self.object_get_own_property_descriptor_value(realm, &object, &descriptor_key)?;
            if matches!(descriptor, Value::Undefined) {
                continue;
            }
            self.define_fresh_object_descriptor_property(
                &result,
                &key,
                descriptor,
                "fresh Object.getOwnPropertyDescriptors result rejected a property",
            )?;
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    pub(in crate::runtime) fn call_object_is(
        &self,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.is did not receive a generic invocation",
            ));
        };
        let left = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant("Object.is lhs argv was not padded"))?;
        let right = arguments
            .readable
            .get(1)
            .ok_or(RuntimeError::Invariant("Object.is rhs argv was not padded"))?;
        Ok(Completion::Return(Value::Bool(left.same_value(right))))
    }

    /// Pinned QuickJS `JS_CopyDataProperties(..., setprop = 0)` as used by an
    /// Object literal spread. This intentionally preserves two upstream
    /// details which differ from a naive spec helper reuse:
    ///
    /// - primitive sources are ignored instead of being boxed;
    /// - ordinary sources snapshot their enumerable key set before any getter
    ///   runs, while each value lookup remains live and may reach a prototype
    ///   after an earlier getter deletes an own property.
    pub(in crate::runtime) fn copy_object_literal_data_properties(
        &self,
        realm: ContextId,
        target: &ObjectRef,
        source: Value,
    ) -> Result<Completion, RuntimeError> {
        let Value::Object(source) = source else {
            return Ok(Completion::Return(Value::Undefined));
        };
        if !target.belongs_to(self) || !source.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object-literal spread object"));
        }

        let mut keys = Vec::new();
        for key in self.own_property_keys(&source)? {
            let kind = self.0.state.borrow().atoms.property_key_kind(key.atom())?;
            if matches!(kind, PropertyKeyKind::String | PropertyKeyKind::Symbol)
                && self.own_property_is_enumerable(&source, &key)?
            {
                keys.push(key);
            }
        }

        for key in keys {
            let value = match self.get_property_in_realm(realm, &source, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            self.define_fresh_object_descriptor_property(
                target,
                &key,
                value,
                "fresh Object literal rejected a spread data property",
            )?;
        }
        Ok(Completion::Return(Value::Undefined))
    }

    pub(in crate::runtime) fn call_object_assign(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.assign did not receive a generic invocation",
            ));
        };
        let target = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object.assign target argv was not padded",
            ))?;
        let target = match self.native_to_object(realm, target)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        for source in arguments
            .readable
            .iter()
            .skip(1)
            .take(arguments.actual_arg_count.saturating_sub(1))
        {
            if matches!(source, Value::Null | Value::Undefined) {
                continue;
            }
            let source = match self.native_to_object(realm, source.clone())? {
                NativeConversion::Value(object) => object,
                NativeConversion::Throw(_) => {
                    return Err(RuntimeError::Invariant(
                        "non-nullish Object.assign source failed ToObject",
                    ));
                }
            };

            // QuickJS's ordinary-object fast path applies ENUM_ONLY while it
            // snapshots all string and Symbol keys. Later getters therefore
            // cannot add initially hidden keys or remove an initially visible
            // key from this source's copy list.
            let mut keys = Vec::new();
            for key in self.own_property_keys(&source)? {
                let kind = self.0.state.borrow().atoms.property_key_kind(key.atom())?;
                if matches!(kind, PropertyKeyKind::String | PropertyKeyKind::Symbol)
                    && self.own_property_is_enumerable(&source, &key)?
                {
                    keys.push(key);
                }
            }
            for key in keys {
                let value = match self.get_property_in_realm(realm, &source, &key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if let Some(value) = self.set_property_or_throw(realm, &target, &key, value)? {
                    return Ok(Completion::Throw(value));
                }
            }
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    pub(in crate::runtime) fn call_object_integrity(
        &self,
        realm: ContextId,
        kind: ObjectIntegrityKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object integrity method did not receive a generic invocation",
            ));
        };
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object integrity argv was not padded",
            ))?;
        let Value::Object(object) = &value else {
            return Ok(Completion::Return(match kind {
                ObjectIntegrityKind::Seal | ObjectIntegrityKind::Freeze => value,
                ObjectIntegrityKind::IsSealed | ObjectIntegrityKind::IsFrozen => Value::Bool(true),
            }));
        };

        let mut keys = Vec::new();
        match kind {
            ObjectIntegrityKind::Seal | ObjectIntegrityKind::Freeze => {
                // QuickJS prevents extensions before it snapshots any key.
                self.prevent_extensions(object)?;
                for key in self.own_property_keys(object)? {
                    let key_kind = self.0.state.borrow().atoms.property_key_kind(key.atom())?;
                    if matches!(key_kind, PropertyKeyKind::String | PropertyKeyKind::Symbol) {
                        keys.push(key);
                    }
                }
                for key in keys {
                    let mut descriptor = OrdinaryPropertyDescriptor {
                        configurable: DescriptorField::Present(false),
                        ..OrdinaryPropertyDescriptor::new()
                    };
                    if kind == ObjectIntegrityKind::Freeze
                        && matches!(
                            self.get_own_property(object, &key)?,
                            Some(CompleteOrdinaryPropertyDescriptor::Data { writable: true, .. })
                        )
                    {
                        descriptor.writable = DescriptorField::Present(false);
                    }
                    if let Some(value) =
                        self.define_property_or_throw(realm, object, &key, &descriptor)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Ok(Completion::Return(value))
            }
            ObjectIntegrityKind::IsSealed | ObjectIntegrityKind::IsFrozen => {
                for key in self.own_property_keys(object)? {
                    let key_kind = self.0.state.borrow().atoms.property_key_kind(key.atom())?;
                    if matches!(key_kind, PropertyKeyKind::String | PropertyKeyKind::Symbol) {
                        keys.push(key);
                    }
                }
                for key in keys {
                    let Some(descriptor) = self.get_own_property(object, &key)? else {
                        continue;
                    };
                    let violates = match descriptor {
                        CompleteOrdinaryPropertyDescriptor::Data {
                            writable,
                            configurable,
                            ..
                        } => configurable || (kind == ObjectIntegrityKind::IsFrozen && writable),
                        CompleteOrdinaryPropertyDescriptor::Accessor { configurable, .. } => {
                            configurable
                        }
                    };
                    if violates {
                        return Ok(Completion::Return(Value::Bool(false)));
                    }
                }
                Ok(Completion::Return(Value::Bool(
                    !self.is_extensible(object)?,
                )))
            }
        }
    }

    pub(in crate::runtime) fn call_object_prototype_has_own_property(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.hasOwnProperty did not receive a generic invocation",
            ));
        };
        // QuickJS converts the key before checking the receiver.
        let key = match self.native_to_property_key(
            realm,
            arguments
                .readable
                .first()
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "hasOwnProperty argv was not padded",
                ))?,
        )? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let object = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Bool(
            self.has_own_property(&object, &key)?,
        )))
    }

    pub(in crate::runtime) fn call_object_prototype_property_is_enumerable(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.propertyIsEnumerable did not receive a generic invocation",
            ));
        };
        let key = match self.native_to_property_key(
            realm,
            arguments
                .readable
                .first()
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "propertyIsEnumerable argv was not padded",
                ))?,
        )? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let object = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let enumerable = self
            .get_own_property(&object, &key)?
            .is_some_and(|descriptor| match descriptor {
                CompleteOrdinaryPropertyDescriptor::Data { enumerable, .. }
                | CompleteOrdinaryPropertyDescriptor::Accessor { enumerable, .. } => enumerable,
            });
        Ok(Completion::Return(Value::Bool(enumerable)))
    }

    pub(in crate::runtime) fn call_object_prototype_is_prototype_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.isPrototypeOf did not receive a generic invocation",
            ));
        };
        let Some(Value::Object(candidate)) = arguments.readable.first() else {
            return Ok(Completion::Return(Value::Bool(false)));
        };
        let prototype = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut cursor = self.get_prototype_of(candidate)?;
        while let Some(current) = cursor {
            if current == prototype {
                return Ok(Completion::Return(Value::Bool(true)));
            }
            cursor = self.get_prototype_of(&current)?;
        }
        Ok(Completion::Return(Value::Bool(false)))
    }

    pub(in crate::runtime) fn call_object_prototype_proto_getter(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype __proto__ getter received the wrong invocation",
            ));
        };
        let object = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(
            self.get_prototype_of(&object)?
                .map_or(Value::Null, Value::Object),
        ))
    }

    pub(in crate::runtime) fn call_object_prototype_proto_setter(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Setter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype __proto__ setter received the wrong invocation",
            ));
        };
        if matches!(this_value, Value::Undefined | Value::Null) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        }
        let prototype = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Object.prototype __proto__ setter argv was not padded",
        ))?;
        let prototype = match prototype {
            Value::Object(prototype) => Some(prototype),
            Value::Null => None,
            _ => return Ok(Completion::Return(Value::Undefined)),
        };
        let Value::Object(object) = this_value else {
            return Ok(Completion::Return(Value::Undefined));
        };
        if let Some(value) = self.set_prototype_or_throw(realm, &object, prototype)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Undefined))
    }

    pub(in crate::runtime) fn call_object_prototype_define_accessor(
        &self,
        realm: ContextId,
        kind: ObjectAccessorKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype __define*__ did not receive a generic invocation",
            ));
        };
        let object = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let accessor = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "Object.prototype __define*__ accessor argv was not padded",
        ))?;
        let Value::Object(accessor) = accessor else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(accessor) = self.as_callable(accessor)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let key = match self.native_to_property_key(
            realm,
            arguments
                .readable
                .first()
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Object.prototype __define*__ key argv was not padded",
                ))?,
        )? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut descriptor = OrdinaryPropertyDescriptor {
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        match kind {
            ObjectAccessorKind::Getter => {
                descriptor.get = DescriptorField::Present(AccessorValue::Callable(accessor));
            }
            ObjectAccessorKind::Setter => {
                descriptor.set = DescriptorField::Present(AccessorValue::Callable(accessor));
            }
        }
        if let Some(value) = self.define_property_or_throw(realm, &object, &key, &descriptor)? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Undefined))
    }

    pub(in crate::runtime) fn call_object_prototype_lookup_accessor(
        &self,
        realm: ContextId,
        kind: ObjectAccessorKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype __lookup*__ did not receive a generic invocation",
            ));
        };
        let object = match self.native_to_object(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = match self.native_to_property_key(
            realm,
            arguments
                .readable
                .first()
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Object.prototype __lookup*__ key argv was not padded",
                ))?,
        )? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut cursor = Some(object);
        while let Some(current) = cursor {
            if let Some(descriptor) = self.get_own_property(&current, &key)? {
                let value = match descriptor {
                    CompleteOrdinaryPropertyDescriptor::Accessor { get, set, .. } => match kind {
                        ObjectAccessorKind::Getter => get,
                        ObjectAccessorKind::Setter => set,
                    }
                    .map_or(Value::Undefined, |callable| {
                        Value::Object(callable.as_object().clone())
                    }),
                    CompleteOrdinaryPropertyDescriptor::Data { .. } => Value::Undefined,
                };
                return Ok(Completion::Return(value));
            }
            cursor = self.get_prototype_of(&current)?;
        }
        Ok(Completion::Return(Value::Undefined))
    }
}
