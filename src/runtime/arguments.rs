//! QuickJS-compatible mapped and unmapped Arguments exotic objects.
//!
//! The compiler/VM boundary decides when an implicit binding is materialized
//! and supplies either copied actual values or shared argument VarRefs. This
//! module owns the class shape, cached realm intrinsics, representation state,
//! and the mapped `[[DefineOwnProperty]]` transitions.

use super::*;

impl Runtime {
    /// Build QuickJS `JS_CLASS_ARGUMENTS` from the exact actual arguments.
    /// Formal-parameter padding must never be included in `values`.
    pub(in crate::runtime) fn new_unmapped_arguments_object(
        &self,
        realm: ContextId,
        values: Vec<Value>,
    ) -> Result<ObjectRef, RuntimeError> {
        for value in &values {
            self.validate_value_domain(value, "unmapped arguments element")?;
        }
        let length = u32::try_from(values.len()).map_err(|_| {
            RuntimeError::Invariant("actual argument count exceeded QuickJS Uint32 storage")
        })?;
        let object = self.new_arguments_object_base(realm, false, length)?;
        for (index, value) in values.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| {
                RuntimeError::Invariant("actual argument index exceeded QuickJS Uint32 storage")
            })?;
            let key = self.intern_property_key(&index.to_string())?;
            self.store_property_slot(
                &object,
                &key,
                PropertyFlags::data(true, true, true),
                PropertySlot::Data(self.raw_property_value(&value)?),
            )?;
        }
        self.install_arguments_common_properties(realm, &object, length, None)?;
        Ok(object)
    }

    /// Build QuickJS `JS_CLASS_MAPPED_ARGUMENTS`. Each supplied root is one
    /// actual indexed element. Roots corresponding to formal parameters share
    /// the frame cell; roots for extra actual arguments are detached cells
    /// allocated by the VM before this call.
    pub(in crate::runtime) fn new_mapped_arguments_object(
        &self,
        realm: ContextId,
        current_function: &ObjectRef,
        roots: Vec<VarRefRoot>,
    ) -> Result<ObjectRef, RuntimeError> {
        if !current_function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("mapped arguments callee"));
        }
        if self.as_callable(current_function)?.is_none() {
            return Err(RuntimeError::Invariant(
                "mapped arguments callee has no [[Call]] method",
            ));
        }
        for root in &roots {
            if !root.belongs_to(self) {
                return Err(RuntimeError::WrongRuntime("mapped arguments element"));
            }
        }
        let length = u32::try_from(roots.len()).map_err(|_| {
            RuntimeError::Invariant("actual argument count exceeded QuickJS Uint32 storage")
        })?;
        let object = self.new_arguments_object_base(realm, true, length)?;
        for (index, root) in roots.iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| {
                RuntimeError::Invariant("actual argument index exceeded QuickJS Uint32 storage")
            })?;
            let key = self.intern_property_key(&index.to_string())?;
            self.store_property_slot(
                &object,
                &key,
                PropertyFlags::data(true, true, true),
                PropertySlot::VarRef(root.id()),
            )?;
        }
        self.install_arguments_common_properties(realm, &object, length, Some(current_function))?;
        Ok(object)
    }

    fn new_arguments_object_base(
        &self,
        realm: ContextId,
        mapped: bool,
        fast_len: u32,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype), &[])?;
        let object = match state.heap.allocate_object(ObjectData::arguments(
            shape,
            Vec::new(),
            mapped,
            fast_len,
        )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn install_arguments_common_properties(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        length: u32,
        current_function: Option<&ObjectRef>,
    ) -> Result<(), RuntimeError> {
        let (array_values, thrower) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            (
                context
                    .array_prototype_values
                    .ok_or(RuntimeError::Invariant(
                        "realm has no cached Array.prototype.values root",
                    ))?,
                context.throw_type_error.ok_or(RuntimeError::Invariant(
                    "realm has no shared %ThrowTypeError% root",
                ))?,
            )
        };

        let length_key = self.intern_property_key("length")?;
        self.store_property_slot(
            object,
            &length_key,
            PropertyFlags::data(true, false, true),
            PropertySlot::Data(self.raw_property_value(&Self::array_length_value(length))?),
        )?;

        let callee = self.intern_property_key("callee")?;
        if let Some(current_function) = current_function {
            self.store_property_slot(
                object,
                &callee,
                PropertyFlags::data(true, false, true),
                PropertySlot::Data(RawValue::Object(current_function.object_id())),
            )?;
        } else {
            self.store_property_slot(
                object,
                &callee,
                PropertyFlags::accessor(false, false),
                PropertySlot::Accessor {
                    get: Some(thrower),
                    set: Some(thrower),
                },
            )?;
        }

        let iterator = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        self.store_property_slot(
            object,
            &iterator,
            PropertyFlags::data(true, false, true),
            PropertySlot::Data(RawValue::Object(array_values)),
        )?;
        Ok(())
    }

    /// Return the class state and numeric index for one Arguments own-key
    /// operation. `None` means either a non-Arguments receiver or a non-index
    /// property key.
    pub(super) fn arguments_index_state(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<(u32, bool, Option<u32>)>, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let state = self.0.state.borrow();
        let object_data = state.heap.object(object.object_id())?;
        let ObjectPayload::Arguments { mapped, fast_len } = object_data.payload else {
            return Ok(None);
        };
        Ok(state
            .atoms
            .array_index(key.atom())?
            .map(|index| (index, mapped, fast_len)))
    }

    #[cfg(test)]
    pub(super) fn arguments_fast_len(
        &self,
        object: &ObjectRef,
    ) -> Result<Option<u32>, RuntimeError> {
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .arguments_state(object.object_id())?
            .1)
    }

    pub(super) fn set_arguments_fast_len(
        &self,
        object: &ObjectRef,
        fast_len: Option<u32>,
    ) -> Result<(), RuntimeError> {
        self.0
            .state
            .borrow_mut()
            .heap
            .set_arguments_fast_len(object.object_id(), fast_len)?;
        Ok(())
    }

    /// QuickJS's arguments exotic hook converts a fast existing numeric field
    /// to slow storage before applying an explicit DefineOwnProperty. Mapped
    /// VarRef slots remain aliases unless the descriptor makes them accessors
    /// or non-writable.
    pub(super) fn define_arguments_index(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<Option<bool>, RuntimeError> {
        let Some((index, mapped, fast_len)) = self.arguments_index_state(object, key)? else {
            return Ok(None);
        };
        if fast_len.is_some_and(|fast_len| index < fast_len) {
            self.set_arguments_fast_len(object, None)?;
        }

        let var_ref = self.own_var_ref_root(object, key)?;
        if var_ref.is_none() {
            return self
                .define_ordinary_own_property(object, key, descriptor)
                .map(Some);
        }
        if !mapped {
            return Err(RuntimeError::Invariant(
                "unmapped Arguments object contains a mapped VarRef slot",
            ));
        }
        let var_ref = var_ref.expect("mapped VarRef presence was checked");
        let current = self
            .get_own_property(object, key)?
            .ok_or(RuntimeError::Invariant(
                "mapped Arguments VarRef lost its property",
            ))?;
        let descriptor_record = descriptor_to_validation_record(descriptor);
        let current_record = complete_to_validation_record(&current);
        let complete = match validate_and_apply_property_descriptor(
            self.is_extensible(object)?,
            &descriptor_record,
            Some(&current_record),
            &Value::Undefined,
            Value::same_value,
        ) {
            Ok(complete) => validation_record_to_complete(complete)?,
            Err(PropertyDefinitionError::InvalidDescriptor) => {
                return Err(PropertyDefinitionError::InvalidDescriptor.into());
            }
            Err(_) => return Ok(Some(false)),
        };

        match complete {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable: true,
                enumerable,
                configurable,
            } => {
                self.write_var_ref(&var_ref, value)?;
                self.store_property_slot(
                    object,
                    key,
                    PropertyFlags::data(true, enumerable, configurable),
                    PropertySlot::VarRef(var_ref.id()),
                )?;
            }
            complete @ CompleteOrdinaryPropertyDescriptor::Data {
                writable: false, ..
            } => {
                let CompleteOrdinaryPropertyDescriptor::Data { value, .. } = &complete else {
                    unreachable!()
                };
                self.write_var_ref(&var_ref, value.clone())?;
                self.store_complete_property(object, key, complete)?;
            }
            complete @ CompleteOrdinaryPropertyDescriptor::Accessor { .. } => {
                self.store_complete_property(object, key, complete)?;
            }
        }
        Ok(Some(true))
    }

    /// Direct ordinary assignment to an existing Arguments index uses the
    /// fast element path and must not trigger the explicit-define conversion.
    pub(super) fn set_arguments_index_value(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: &Value,
    ) -> Result<bool, RuntimeError> {
        if self.arguments_index_state(object, key)?.is_none() {
            return Ok(false);
        }
        let (flags, slot) = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            let shape = state.heap.shape(object_data.shape)?;
            let Some(index) = shape.find(key.atom()) else {
                return Ok(false);
            };
            let index = usize::try_from(index)
                .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            (
                shape.entries()[index].flags,
                object_data.slots[index].clone(),
            )
        };
        if !flags.writable {
            return Ok(false);
        }
        match slot {
            PropertySlot::VarRef(id) => {
                let root = VarRefRoot::from_borrowed_handle(self.clone(), id)?;
                self.write_var_ref(&root, value.clone())?;
            }
            PropertySlot::Data(_) => {
                self.store_property_slot(
                    object,
                    key,
                    flags,
                    PropertySlot::Data(self.raw_property_value(value)?),
                )?;
            }
            PropertySlot::Accessor { .. } | PropertySlot::AutoInit(_) => return Ok(false),
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_descriptor(
        descriptor: Option<CompleteOrdinaryPropertyDescriptor>,
    ) -> (Value, bool, bool, bool) {
        let Some(CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        }) = descriptor
        else {
            panic!("expected an own data property")
        };
        (value, writable, enumerable, configurable)
    }

    #[test]
    fn unmapped_arguments_use_exact_values_realm_roots_and_quickjs_descriptors() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let array_prototype = context.array_prototype().unwrap();
        let values_key = runtime.intern_property_key("values").unwrap();
        let original_values = context.get_property(&array_prototype, &values_key).unwrap();
        assert!(
            context
                .set_property(&array_prototype, &values_key, Value::Int(99))
                .unwrap()
        );

        let arguments = runtime
            .new_unmapped_arguments_object(context.realm, vec![Value::Int(10), Value::Int(20)])
            .unwrap();
        assert_eq!(
            runtime
                .get_prototype_of(&arguments)
                .unwrap()
                .as_ref()
                .map(ObjectRef::object_id),
            Some(context.object_prototype().unwrap().object_id())
        );
        assert_eq!(runtime.arguments_fast_len(&arguments), Ok(Some(2)));
        assert!(matches!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(arguments.object_id())
                .unwrap()
                .payload,
            ObjectPayload::Arguments {
                mapped: false,
                fast_len: Some(2)
            }
        ));

        let zero = runtime.intern_property_key("0").unwrap();
        assert_eq!(
            data_descriptor(runtime.get_own_property(&arguments, &zero).unwrap()),
            (Value::Int(10), true, true, true)
        );
        let length = runtime.intern_property_key("length").unwrap();
        assert_eq!(
            data_descriptor(runtime.get_own_property(&arguments, &length).unwrap()),
            (Value::Int(2), true, false, true)
        );

        let callee = runtime.intern_property_key("callee").unwrap();
        let Some(CompleteOrdinaryPropertyDescriptor::Accessor {
            get: Some(get),
            set: Some(set),
            enumerable,
            configurable,
        }) = runtime.get_own_property(&arguments, &callee).unwrap()
        else {
            panic!("unmapped callee was not the poison accessor")
        };
        assert_eq!(get.as_object(), set.as_object());
        assert!(!enumerable);
        assert!(!configurable);

        let iterator = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
        assert_eq!(
            data_descriptor(runtime.get_own_property(&arguments, &iterator).unwrap()),
            (original_values, true, false, true)
        );
        assert_eq!(
            runtime.own_property_keys(&arguments).unwrap(),
            [
                runtime.intern_property_key("0").unwrap(),
                runtime.intern_property_key("1").unwrap(),
                length,
                callee,
                iterator,
            ]
        );
    }

    #[test]
    fn arguments_intrinsic_properties_are_cached_per_realm() {
        let runtime = Runtime::new();
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let values = runtime.intern_property_key("values").unwrap();
        let first_values = first
            .get_property(&first.array_prototype().unwrap(), &values)
            .unwrap();
        let second_values = second
            .get_property(&second.array_prototype().unwrap(), &values)
            .unwrap();
        assert_ne!(first_values, second_values);

        let first_arguments = runtime
            .new_unmapped_arguments_object(first.realm, Vec::new())
            .unwrap();
        let second_arguments = runtime
            .new_unmapped_arguments_object(second.realm, Vec::new())
            .unwrap();
        let iterator = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
        assert_eq!(
            data_descriptor(
                runtime
                    .get_own_property(&first_arguments, &iterator)
                    .unwrap()
            )
            .0,
            first_values
        );
        assert_eq!(
            data_descriptor(
                runtime
                    .get_own_property(&second_arguments, &iterator)
                    .unwrap()
            )
            .0,
            second_values
        );

        let callee = runtime.intern_property_key("callee").unwrap();
        let poison = |object: &ObjectRef| {
            let Some(CompleteOrdinaryPropertyDescriptor::Accessor {
                get: Some(get),
                set: Some(set),
                ..
            }) = runtime.get_own_property(object, &callee).unwrap()
            else {
                panic!("unmapped arguments lost its poison callee")
            };
            assert_eq!(get.as_object(), set.as_object());
            get
        };
        assert_ne!(
            poison(&first_arguments).as_object(),
            poison(&second_arguments).as_object()
        );
    }

    #[test]
    fn mapped_arguments_keep_aliases_until_delete_accessor_or_read_only_transition() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callee = context.function_prototype().unwrap();
        let root = runtime
            .new_var_ref(Value::Int(1), false, false, ClosureVariableKind::Normal)
            .unwrap();
        let arguments = runtime
            .new_mapped_arguments_object(context.realm, &callee, vec![root.clone()])
            .unwrap();
        let zero = runtime.intern_property_key("0").unwrap();

        assert!(
            context
                .set_property(&arguments, &zero, Value::Int(2))
                .unwrap()
        );
        assert_eq!(runtime.read_var_ref(&root).unwrap(), Value::Int(2));
        assert_eq!(runtime.arguments_fast_len(&arguments), Ok(Some(1)));

        runtime.write_var_ref(&root, Value::Int(3)).unwrap();
        assert_eq!(
            context.get_property(&arguments, &zero).unwrap(),
            Value::Int(3)
        );
        assert!(
            context
                .define_own_property(
                    &arguments,
                    &zero,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(4)),
                        enumerable: DescriptorField::Present(false),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(runtime.read_var_ref(&root).unwrap(), Value::Int(4));
        assert_eq!(runtime.arguments_fast_len(&arguments), Ok(None));
        runtime.write_var_ref(&root, Value::Int(5)).unwrap();
        assert_eq!(
            context.get_property(&arguments, &zero).unwrap(),
            Value::Int(5)
        );

        assert!(
            context
                .define_own_property(
                    &arguments,
                    &zero,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(6)),
                        writable: DescriptorField::Present(false),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(runtime.read_var_ref(&root).unwrap(), Value::Int(6));
        runtime.write_var_ref(&root, Value::Int(7)).unwrap();
        assert_eq!(
            context.get_property(&arguments, &zero).unwrap(),
            Value::Int(6)
        );
        assert_eq!(
            data_descriptor(runtime.get_own_property(&arguments, &zero).unwrap()),
            (Value::Int(6), false, false, true)
        );

        let callee_key = runtime.intern_property_key("callee").unwrap();
        assert_eq!(
            data_descriptor(runtime.get_own_property(&arguments, &callee_key).unwrap()),
            (Value::Object(callee), true, false, true)
        );
    }

    #[test]
    fn arguments_delete_updates_fast_state_and_never_reconnects_a_mapping() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callee = context.function_prototype().unwrap();
        let first = runtime
            .new_var_ref(Value::Int(1), false, false, ClosureVariableKind::Normal)
            .unwrap();
        let second = runtime
            .new_var_ref(Value::Int(2), false, false, ClosureVariableKind::Normal)
            .unwrap();
        let tail = runtime
            .new_mapped_arguments_object(
                context.realm,
                &callee,
                vec![first.clone(), second.clone()],
            )
            .unwrap();
        let one = runtime.intern_property_key("1").unwrap();
        assert!(runtime.delete_property(&tail, &one).unwrap());
        assert_eq!(runtime.arguments_fast_len(&tail), Ok(Some(1)));

        let middle = runtime
            .new_mapped_arguments_object(
                context.realm,
                &callee,
                vec![first.clone(), second.clone()],
            )
            .unwrap();
        let zero = runtime.intern_property_key("0").unwrap();
        assert!(runtime.delete_property(&middle, &zero).unwrap());
        assert_eq!(runtime.arguments_fast_len(&middle), Ok(None));
        assert!(context.set_property(&middle, &zero, Value::Int(8)).unwrap());
        runtime.write_var_ref(&first, Value::Int(9)).unwrap();
        assert_eq!(context.get_property(&middle, &zero).unwrap(), Value::Int(8));
    }
}
