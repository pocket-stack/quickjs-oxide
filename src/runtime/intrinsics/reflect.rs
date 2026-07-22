//! Pinned QuickJS `Reflect` intrinsic algorithms.
//!
//! The implementation follows the 2026-06-04 `js_reflect_funcs` table and
//! intentionally retains QuickJS's validation order where it differs from a
//! tempting shared helper. In particular, `Reflect.construct` validates an
//! explicit `newTarget`, then materializes the argument list, and only then
//! validates the target constructor.

use super::super::*;

#[cfg(test)]
mod tests;

const MAX_APPLY_ARGUMENTS: u64 = 65_534;

impl Runtime {
    /// Snapshot QuickJS's fast Array/Arguments storage in numeric-index order.
    ///
    /// Oxide keeps dense values in ordinary shape slots, where named
    /// properties may be interleaved. Walk the shape once and reconstruct the
    /// numeric prefix instead of performing one observable-style property
    /// lookup per index. The owning object stays rooted while raw values are
    /// promoted to public roots.
    pub(in crate::runtime) fn fast_array_like_values(
        &self,
        object: &ObjectRef,
        expected_len: u32,
    ) -> Result<Option<Vec<Value>>, RuntimeError> {
        let raw_values = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            let (mapped, fast_len) = match &object_data.payload {
                ObjectPayload::Array { fast_len } => (false, *fast_len),
                ObjectPayload::Arguments { mapped, fast_len } => (*mapped, *fast_len),
                _ => return Ok(None),
            };
            if fast_len != Some(expected_len) {
                return Ok(None);
            }
            let shape = state.heap.shape(object_data.shape)?;
            let capacity = usize::try_from(expected_len)
                .map_err(|_| RuntimeError::Invariant("fast argument length does not fit usize"))?;
            let mut ordered = vec![None; capacity];
            for (entry, slot) in shape.entries().iter().zip(&object_data.slots) {
                let Some(index) = state.atoms.array_index(entry.atom)? else {
                    continue;
                };
                if index >= expected_len {
                    continue;
                }
                if !entry.flags.writable || !entry.flags.enumerable || !entry.flags.configurable {
                    return Err(RuntimeError::Invariant(
                        "fast argument index is not a C/W/E property",
                    ));
                }
                let value = match slot {
                    PropertySlot::Data(value) if !mapped => value.clone(),
                    PropertySlot::VarRef(var_ref) if mapped => {
                        state.heap.var_ref(*var_ref)?.value.clone()
                    }
                    _ => {
                        return Err(RuntimeError::Invariant(
                            "fast argument index has the wrong storage kind",
                        ));
                    }
                };
                let destination = ordered
                    .get_mut(usize::try_from(index).map_err(|_| {
                        RuntimeError::Invariant("fast argument index does not fit usize")
                    })?)
                    .ok_or(RuntimeError::Invariant(
                        "fast argument index escaped its dense prefix",
                    ))?;
                if destination.replace(value).is_some() {
                    return Err(RuntimeError::Invariant(
                        "fast argument prefix contains a duplicate index",
                    ));
                }
            }
            ordered
                .into_iter()
                .map(|value| {
                    value.ok_or(RuntimeError::Invariant(
                        "fast argument prefix is missing an indexed value",
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?
        };

        raw_values
            .iter()
            .map(|value| self.root_raw_value(value))
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    /// Install the global `Reflect` `JS_OBJECT_DEF` equivalent. The object is
    /// realm-owned and remains lazy until the global slot is first read.
    pub(in crate::runtime) fn initialize_reflect_intrinsic(
        &self,
        realm: ContextId,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key("Reflect")?;
        self.store_property_slot(
            global_object,
            &key,
            PropertyFlags::data(true, false, true),
            PropertySlot::AutoInit(AutoInitProperty::Reflect { realm }),
        )
    }

    /// Materialize pinned QuickJS's complete `js_reflect_funcs` table in its
    /// defining realm. Each method remains an AutoInit native property.
    pub(in crate::runtime) fn instantiate_reflect_intrinsic(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let reflect = self.new_ordinary_object_in_realm(realm)?;
        for (kind, name, length) in [
            (ReflectKind::Apply, "apply", 3),
            (ReflectKind::Construct, "construct", 2),
            (ReflectKind::DefineProperty, "defineProperty", 3),
            (ReflectKind::DeleteProperty, "deleteProperty", 2),
            (ReflectKind::Get, "get", 2),
            (
                ReflectKind::GetOwnPropertyDescriptor,
                "getOwnPropertyDescriptor",
                2,
            ),
            (ReflectKind::GetPrototypeOf, "getPrototypeOf", 1),
            (ReflectKind::Has, "has", 2),
            (ReflectKind::IsExtensible, "isExtensible", 1),
            (ReflectKind::OwnKeys, "ownKeys", 1),
            (ReflectKind::PreventExtensions, "preventExtensions", 1),
            (ReflectKind::Set, "set", 3),
            (ReflectKind::SetPrototypeOf, "setPrototypeOf", 2),
        ] {
            self.define_native_builtin_auto_init(
                &reflect,
                realm,
                NativeFunctionId::Reflect(kind),
                name,
                length,
                length,
            )?;
        }

        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            &reflect,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("Reflect"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Reflect toStringTag definition was rejected",
            ));
        }
        Ok(reflect)
    }

    /// QuickJS `build_arg_list`, shared by Function.prototype.apply and the
    /// two Reflect call/construct paths. Nullish exceptions remain a caller
    /// decision: this kernel always requires an object, as upstream does once
    /// it has entered `build_arg_list`.
    pub(in crate::runtime) fn build_array_like_argument_list(
        &self,
        realm: ContextId,
        array_argument: &Value,
    ) -> Result<NativeConversion<Vec<Value>>, RuntimeError> {
        let Value::Object(array_like) = array_argument else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a object",
            )?));
        };

        let length_key = self.intern_property_key("length")?;
        let length_value = match self.get_property_in_realm(realm, array_like, &length_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let length = match self.native_to_length(realm, &length_value)? {
            NativeConversion::Value(length) => length,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if length > MAX_APPLY_ARGUMENTS {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "too many arguments in function call (only 65534 allowed)",
            )?));
        }

        let length = usize::try_from(length)
            .map_err(|_| RuntimeError::Invariant("argument-list length does not fit usize"))?;
        let fast_len = u32::try_from(length)
            .map_err(|_| RuntimeError::Invariant("argument-list length does not fit u32"))?;
        if let Some(values) = self.fast_array_like_values(array_like, fast_len)? {
            return Ok(NativeConversion::Value(values));
        }
        let mut forwarded = Vec::with_capacity(length);
        for index in 0..length {
            let key = self.intern_property_key(&index.to_string())?;
            match self.get_property_in_realm(realm, array_like, &key)? {
                Completion::Return(value) => forwarded.push(value),
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        }
        Ok(NativeConversion::Value(forwarded))
    }

    pub(in crate::runtime) fn call_reflect(
        &self,
        realm: ContextId,
        kind: ReflectKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Reflect method did not receive a generic invocation",
            ));
        };
        match kind {
            ReflectKind::Apply => self.call_reflect_apply(realm, arguments),
            ReflectKind::Construct => self.call_reflect_construct(realm, arguments),
            ReflectKind::DefineProperty => self.call_reflect_define_property(realm, arguments),
            ReflectKind::DeleteProperty => self.call_reflect_delete_property(realm, arguments),
            ReflectKind::Get => self.call_reflect_get(realm, arguments),
            ReflectKind::GetOwnPropertyDescriptor => {
                self.call_reflect_get_own_property_descriptor(realm, arguments)
            }
            ReflectKind::GetPrototypeOf => self.call_reflect_get_prototype_of(realm, arguments),
            ReflectKind::Has => self.call_reflect_has(realm, arguments),
            ReflectKind::IsExtensible => self.call_reflect_is_extensible(realm, arguments),
            ReflectKind::OwnKeys => self.call_reflect_own_keys(realm, arguments),
            ReflectKind::PreventExtensions => {
                self.call_reflect_prevent_extensions(realm, arguments)
            }
            ReflectKind::Set => self.call_reflect_set(realm, arguments),
            ReflectKind::SetPrototypeOf => self.call_reflect_set_prototype_of(realm, arguments),
        }
    }

    fn reflect_object_argument(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        match arguments.readable.first() {
            Some(Value::Object(object)) => Ok(NativeConversion::Value(object.clone())),
            Some(_) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?)),
            None => Err(RuntimeError::Invariant(
                "Reflect target argv was not padded",
            )),
        }
    }

    fn call_reflect_apply(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let target = self.callable_from_value(arguments.readable[0].clone())?;
        let forwarded = match self.build_array_like_argument_list(realm, &arguments.readable[2])? {
            NativeConversion::Value(values) => values,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.call_internal(realm, &target, arguments.readable[1].clone(), &forwarded)
    }

    fn call_reflect_construct(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let target_value = arguments.readable[0].clone();

        // Pinned QuickJS validates an explicit third argument before touching
        // argsList, but leaves target validation until after argsList.
        let explicit_new_target = if arguments.actual_arg_count > 2 {
            let value = arguments
                .readable
                .get(2)
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "Reflect.construct newTarget argv was not readable",
                ))?;
            if !matches!(value, Value::Object(_)) {
                return Ok(Completion::Throw(
                    self.new_not_constructor_error(realm, &value)?,
                ));
            }
            Some(match self.constructor_from_value(realm, value)? {
                NativeConversion::Value(constructor) => constructor,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            })
        } else {
            None
        };

        let forwarded = match self.build_array_like_argument_list(realm, &arguments.readable[1])? {
            NativeConversion::Value(values) => values,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target = match self.constructor_from_value(realm, target_value)? {
            NativeConversion::Value(constructor) => constructor,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let new_target = explicit_new_target.unwrap_or_else(|| target.clone());
        self.construct_internal(realm, &target, &new_target, &forwarded)
    }

    fn call_reflect_define_property(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = match self.native_to_property_key(realm, arguments.readable[1].clone())? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let descriptor =
            match self.native_to_property_descriptor(realm, arguments.readable[2].clone())? {
                NativeConversion::Value(descriptor) => descriptor,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        match self.define_own_property_in_realm(Some(realm), &object, &key, &descriptor)? {
            PropertyDefineOutcome::Defined(defined) => Ok(Completion::Return(Value::Bool(defined))),
            PropertyDefineOutcome::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    fn call_reflect_delete_property(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = match self.native_to_property_key(realm, arguments.readable[1].clone())? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Bool(
            self.delete_property(&object, &key)?,
        )))
    }

    fn call_reflect_get(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let receiver = if arguments.actual_arg_count > 2 {
            arguments.readable[2].clone()
        } else {
            Value::Object(object.clone())
        };
        let key = match self.native_to_property_key(realm, arguments.readable[1].clone())? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        match self.prepare_get_property_with_receiver(&object, &key, receiver)? {
            PropertyGetAction::Complete(value) => Ok(Completion::Return(value)),
            PropertyGetAction::Call { getter, receiver } => {
                self.call_internal(realm, &getter, receiver, &[])
            }
        }
    }

    fn call_reflect_get_own_property_descriptor(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = match self.native_to_property_key(realm, arguments.readable[1].clone())? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(
            self.object_get_own_property_descriptor_value(realm, &object, &key)?,
        ))
    }

    fn call_reflect_get_prototype_of(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(
            self.get_prototype_of(&object)?
                .map_or(Value::Null, Value::Object),
        ))
    }

    fn call_reflect_has(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = match self.native_to_property_key(realm, arguments.readable[1].clone())? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.has_property_in_realm(realm, &object, &key)
    }

    fn call_reflect_is_extensible(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Bool(
            self.is_extensible(&object)?,
        )))
    }

    fn call_reflect_own_keys(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let values = self
            .own_property_keys(&object)?
            .iter()
            .map(|key| self.object_property_key_value(key))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Completion::Return(Value::Object(
            self.new_array_from_values(realm, values)?,
        )))
    }

    fn call_reflect_prevent_extensions(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.prevent_extensions(&object)?;
        Ok(Completion::Return(Value::Bool(true)))
    }

    fn call_reflect_set(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let receiver = if arguments.actual_arg_count > 3 {
            arguments.readable[3].clone()
        } else {
            Value::Object(object.clone())
        };
        let key = match self.native_to_property_key(realm, arguments.readable[1].clone())? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = arguments.readable[2].clone();
        match self.prepare_set_property_with_receiver_in_realm(
            Some(realm),
            &object,
            &key,
            value,
            receiver,
        )? {
            PropertySetAction::Complete => Ok(Completion::Return(Value::Bool(true))),
            PropertySetAction::Rejected(_) => Ok(Completion::Return(Value::Bool(false))),
            PropertySetAction::Throw(value) => Ok(Completion::Throw(value)),
            PropertySetAction::Call {
                setter,
                receiver,
                argument,
            } => match self.call_internal(realm, &setter, receiver, &[argument])? {
                Completion::Return(_) => Ok(Completion::Return(Value::Bool(true))),
                Completion::Throw(value) => Ok(Completion::Throw(value)),
            },
        }
    }

    fn call_reflect_set_prototype_of(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let object = match self.reflect_object_argument(realm, arguments)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let prototype = match &arguments.readable[1] {
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
        Ok(Completion::Return(Value::Bool(
            self.set_prototype_of(&object, prototype)?,
        )))
    }
}
