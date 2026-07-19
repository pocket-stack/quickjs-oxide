//! `%Set%`, Set Iterator, and the proposal-era Set methods shipped by QuickJS.
//!
//! The heap owns insertion-ordered records and stable tombstones. This module
//! owns the observable constructor, callback, iterator, set-like protocol,
//! realm, and descriptor behavior. The algorithms intentionally follow the
//! pinned QuickJS `JS_AddIntrinsicMapSet` implementation, including its
//! size-dependent branches and mutation-sensitive ordering.

use crate::heap::{MapNativeKind, SetIteratorKind, SetNativeKind, SetRealmData};

use super::super::*;
use super::object::ObjectIteratorStep;

struct SetLikeRecord {
    target: Value,
    size: i64,
    has: CallableRef,
    keys: CallableRef,
}

impl Runtime {
    pub(in crate::runtime) fn initialize_set_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        iterator_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let set_prototype = self.new_object(Some(object_prototype))?;
        let set_iterator_prototype = self.new_object(Some(iterator_prototype))?;

        for (kind, name, length, readable) in [
            (SetNativeKind::Add, "add", 1, 1),
            (SetNativeKind::Has, "has", 1, 1),
            (SetNativeKind::Delete, "delete", 1, 1),
            (SetNativeKind::Clear, "clear", 0, 0),
        ] {
            self.define_native_builtin_auto_init(
                &set_prototype,
                realm,
                NativeFunctionId::Set(kind),
                name,
                length,
                readable,
            )?;
        }
        self.define_native_builtin_getter_on(
            &set_prototype,
            function_prototype,
            realm,
            NativeFunctionId::Set(SetNativeKind::Size),
            "size",
            "get size",
        )?;
        for (kind, name, length, readable) in [
            (SetNativeKind::ForEach, "forEach", 1, 2),
            (SetNativeKind::IsDisjointFrom, "isDisjointFrom", 1, 1),
            (SetNativeKind::IsSubsetOf, "isSubsetOf", 1, 1),
            (SetNativeKind::IsSupersetOf, "isSupersetOf", 1, 1),
            (SetNativeKind::Intersection, "intersection", 1, 1),
            (SetNativeKind::Difference, "difference", 1, 1),
            (
                SetNativeKind::SymmetricDifference,
                "symmetricDifference",
                1,
                1,
            ),
            (SetNativeKind::Union, "union", 1, 1),
        ] {
            self.define_native_builtin_auto_init(
                &set_prototype,
                realm,
                NativeFunctionId::Set(kind),
                name,
                length,
                readable,
            )?;
        }

        self.define_native_builtin_auto_init(
            &set_prototype,
            realm,
            NativeFunctionId::Set(SetNativeKind::Iterator(SetIteratorKind::Value)),
            "values",
            0,
            0,
        )?;
        // QuickJS installs both aliases from the exact values-function object,
        // before `entries`; preserving that order is observable in ownKeys.
        let values_key = self.intern_property_key("values")?;
        let values = match self.get_property_in_realm(realm, &set_prototype, &values_key)? {
            Completion::Return(value @ Value::Object(_)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "Set.prototype.values was not callable during bootstrap",
                ));
            }
            Completion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "Set.prototype.values initialization threw during bootstrap",
                ));
            }
        };
        let keys_key = self.intern_property_key("keys")?;
        self.define_set_alias(&set_prototype, &keys_key, values.clone())?;
        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        self.define_set_alias(&set_prototype, &iterator_key, values)?;
        self.define_native_builtin_auto_init(
            &set_prototype,
            realm,
            NativeFunctionId::Set(SetNativeKind::Iterator(SetIteratorKind::KeyAndValue)),
            "entries",
            0,
            0,
        )?;
        self.define_set_to_string_tag(&set_prototype, "Set")?;

        self.define_native_builtin_auto_init(
            &set_iterator_prototype,
            realm,
            NativeFunctionId::SetIteratorNext,
            "next",
            0,
            0,
        )?;
        self.define_set_to_string_tag(&set_iterator_prototype, "Set Iterator")?;

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::Set(SetNativeKind::Constructor),
            1,
            "Set",
            0,
        )?;
        self.define_native_builtin_auto_init(
            constructor.as_object(),
            realm,
            NativeFunctionId::Set(SetNativeKind::GroupBy),
            "groupBy",
            2,
            2,
        )?;
        let species_getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::Set(SetNativeKind::Species),
            0,
            "get [Symbol.species]",
            0,
        )?;
        let species = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
        if !self.define_own_property(
            constructor.as_object(),
            &species,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(species_getter)),
                set: DescriptorField::Present(AccessorValue::Undefined),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Set species definition was rejected",
            ));
        }

        self.define_function_data_property(
            global_object,
            "Set",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, &set_prototype)?;
        self.0.state.borrow_mut().heap.attach_set_intrinsics(
            realm,
            SetRealmData {
                prototype: set_prototype.object_id(),
                iterator_prototype: set_iterator_prototype.object_id(),
            },
        )?;
        Ok(())
    }

    fn define_set_alias(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if !self.define_own_property(
            object,
            key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Set intrinsic alias definition was rejected",
            ));
        }
        Ok(())
    }

    fn define_set_to_string_tag(
        &self,
        object: &ObjectRef,
        value: &'static str,
    ) -> Result<(), RuntimeError> {
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static(value))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Set intrinsic toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    fn set_realm_data(&self, realm: ContextId) -> Result<SetRealmData, RuntimeError> {
        self.0
            .state
            .borrow()
            .heap
            .context(realm)?
            .set
            .ok_or(RuntimeError::Invariant("realm has no Set intrinsics"))
    }

    fn new_set_object(&self, prototype: &ObjectRef) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Set prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::set(shape, Vec::new()))
        {
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

    fn new_set_in_realm(&self, realm: ContextId) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.set_realm_data(realm)?.prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        self.new_set_object(&prototype)
    }

    fn set_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(new_target) = new_target else {
            return Err(RuntimeError::Invariant(
                "Set constructor new.target was not an object",
            ));
        };
        let key = self.intern_property_key("prototype")?;
        match self.get_property_in_realm(realm, &new_target, &key)? {
            Completion::Return(Value::Object(prototype)) => Ok(NativeConversion::Value(prototype)),
            Completion::Return(_) => {
                let callable = self.callable_from_value(Value::Object(new_target))?;
                let fallback_realm = self.callable_realm(&callable)?;
                let prototype = self.set_realm_data(fallback_realm)?.prototype;
                Ok(NativeConversion::Value(ObjectRef::from_borrowed_handle(
                    self.clone(),
                    prototype,
                )?))
            }
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(in crate::runtime) fn call_set_native(
        &self,
        realm: ContextId,
        kind: SetNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            SetNativeKind::Constructor => self.call_set_constructor(realm, invocation, arguments),
            SetNativeKind::Species => self.call_set_species(invocation),
            SetNativeKind::GroupBy => {
                self.call_map_native(realm, MapNativeKind::GroupBy, invocation, arguments)
            }
            SetNativeKind::Add => self.call_set_add(realm, invocation, arguments),
            SetNativeKind::Has => self.call_set_has(realm, invocation, arguments),
            SetNativeKind::Delete => self.call_set_delete(realm, invocation, arguments),
            SetNativeKind::Clear => self.call_set_clear(realm, invocation),
            SetNativeKind::Size => self.call_set_size(realm, invocation),
            SetNativeKind::ForEach => self.call_set_for_each(realm, invocation, arguments),
            SetNativeKind::IsDisjointFrom => {
                self.call_set_is_disjoint_from(realm, invocation, arguments)
            }
            SetNativeKind::IsSubsetOf => self.call_set_is_subset_of(realm, invocation, arguments),
            SetNativeKind::IsSupersetOf => {
                self.call_set_is_superset_of(realm, invocation, arguments)
            }
            SetNativeKind::Intersection => self.call_set_intersection(realm, invocation, arguments),
            SetNativeKind::Difference => self.call_set_difference(realm, invocation, arguments),
            SetNativeKind::SymmetricDifference => {
                self.call_set_symmetric_difference(realm, invocation, arguments)
            }
            SetNativeKind::Union => self.call_set_union(realm, invocation, arguments),
            SetNativeKind::Iterator(kind) => {
                self.call_set_iterator_factory(realm, invocation, kind)
            }
        }
    }

    fn call_set_species(&self, invocation: NativeInvocation) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Set species did not receive a getter invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    fn call_set_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Set constructor did not receive a constructor invocation",
            ));
        };
        let prototype = match self.set_prototype_from_new_target(realm, new_target)? {
            NativeConversion::Value(prototype) => prototype,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let set = self.new_set_object(&prototype)?;
        if arguments.actual_arg_count == 0 {
            return Ok(Completion::Return(Value::Object(set)));
        }
        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant("Set iterable argv was not padded"))?;
        if matches!(iterable, Value::Null | Value::Undefined) {
            return Ok(Completion::Return(Value::Object(set)));
        }

        let add_key = self.intern_property_key("add")?;
        let adder = match self.get_property_in_realm(realm, &set, &add_key)? {
            Completion::Return(Value::Object(adder)) => match self.as_callable(&adder)? {
                Some(adder) => adder,
                None => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "set/add is not a function",
                    )?));
                }
            },
            Completion::Return(_) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "set/add is not a function",
                )?));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        let method =
            match self.get_value_property_in_realm(realm, iterable.clone(), &iterator_key)? {
                Completion::Return(Value::Object(method)) => match self.as_callable(&method)? {
                    Some(method) => method,
                    None => {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "value is not iterable",
                        )?));
                    }
                },
                Completion::Return(_) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "value is not iterable",
                    )?));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let iterator = match self.call_internal(realm, &method, iterable, &[])? {
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
        let next = match self.get_property_in_realm(realm, &iterator, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        loop {
            let item = match self.object_iterator_next(realm, &iterator, next.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(set)));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let adder_completion = self.call_internal(
                realm,
                &adder,
                Value::Object(set.clone()),
                std::slice::from_ref(&item),
            )?;
            // QuickJS releases the current iterator value before performing
            // IteratorClose for an abrupt adder completion. Preserve that
            // weak-GC-observable lifetime boundary even though Set is strong.
            drop(item);
            match adder_completion {
                Completion::Return(_) => {}
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            }
        }
    }

    fn set_receiver(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        getter: bool,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let this_value = match (getter, invocation) {
            (false, NativeInvocation::Call { this_value })
            | (true, NativeInvocation::Getter { this_value }) => this_value,
            _ => {
                return Err(RuntimeError::Invariant(
                    "Set method received the wrong native invocation",
                ));
            }
        };
        let Value::Object(object) = this_value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "Set object expected",
            )?));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Set receiver"));
        }
        let is_set = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::Set { .. }
        );
        if !is_set {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "Set object expected",
            )?));
        }
        Ok(NativeConversion::Value(object))
    }

    fn normalized_set_key(value: Value) -> Value {
        match value {
            Value::Float(0.0) => Value::Int(0),
            value => value,
        }
    }

    fn find_set_record(&self, set: &ObjectRef, key: &Value) -> Result<Option<usize>, RuntimeError> {
        let records = self
            .0
            .state
            .borrow()
            .heap
            .set_records(set.object_id())?
            .iter()
            .enumerate()
            .filter_map(|(index, record)| record.key.as_ref().map(|key| (index, key.clone())))
            .collect::<Vec<_>>();
        for (index, candidate) in records {
            let candidate = self.root_raw_value(&candidate)?;
            if candidate.same_value_zero(key) {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    fn insert_set_record(&self, set: &ObjectRef, key: Value) -> Result<bool, RuntimeError> {
        self.validate_value_domain(&key, "Set value")?;
        let key = Self::normalized_set_key(key);
        if self.find_set_record(set, &key)?.is_some() {
            return Ok(false);
        }
        let raw_key = self.raw_property_value(&key)?;
        let mut state = self.0.state.borrow_mut();
        let retained = state.retain_raw_value_atoms([&raw_key])?;
        let cleanup = match state.heap.set_insert_record(set.object_id(), raw_key) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                state.release_atoms(retained)?;
                return Err(error.into());
            }
        };
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(key);
        Ok(true)
    }

    fn delete_set_record(&self, set: &ObjectRef, key: &Value) -> Result<bool, RuntimeError> {
        let key = Self::normalized_set_key(key.clone());
        let Some(index) = self.find_set_record(set, &key)? else {
            return Ok(false);
        };
        let mut state = self.0.state.borrow_mut();
        let cleanup = state.heap.set_delete_record(set.object_id(), index)?;
        state.apply_cleanup(cleanup)?;
        Ok(true)
    }

    fn set_size_value(&self, set: &ObjectRef) -> Result<usize, RuntimeError> {
        Ok(self.0.state.borrow().heap.set_size(set.object_id())?)
    }

    fn next_live_set_value(
        &self,
        set: &ObjectRef,
        index: &mut usize,
    ) -> Result<Option<Value>, RuntimeError> {
        loop {
            let record = self
                .0
                .state
                .borrow()
                .heap
                .set_records(set.object_id())?
                .get(*index)
                .map(|record| record.key.clone());
            let Some(key) = record else {
                return Ok(None);
            };
            *index = index
                .checked_add(1)
                .ok_or(RuntimeError::Invariant("Set record index overflowed"))?;
            let Some(key) = key else {
                continue;
            };
            return Ok(Some(self.root_raw_value(&key)?));
        }
    }

    fn call_set_add(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Set.prototype.add value argv was not padded",
            ))?;
        self.insert_set_record(&set, value)?;
        Ok(Completion::Return(Value::Object(set)))
    }

    fn call_set_has(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = Self::normalized_set_key(arguments.readable.first().cloned().ok_or(
            RuntimeError::Invariant("Set.prototype.has value argv was not padded"),
        )?);
        Ok(Completion::Return(Value::Bool(
            self.find_set_record(&set, &value)?.is_some(),
        )))
    }

    fn call_set_delete(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Set.prototype.delete value argv was not padded",
            ))?;
        Ok(Completion::Return(Value::Bool(
            self.delete_set_record(&set, &value)?,
        )))
    }

    fn call_set_clear(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut state = self.0.state.borrow_mut();
        let cleanup = state.heap.set_clear(set.object_id())?;
        state.apply_cleanup(cleanup)?;
        Ok(Completion::Return(Value::Undefined))
    }

    fn call_set_size(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, true)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::number(
            self.set_size_value(&set)? as f64,
        )))
    }

    fn call_set_for_each(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callback = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Set.prototype.forEach callback argv was not padded",
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
        let this_arg = arguments
            .readable
            .get(1)
            .cloned()
            .unwrap_or(Value::Undefined);
        let mut index = 0_usize;
        while let Some(value) = self.next_live_set_value(&set, &mut index)? {
            match self.call_internal(
                realm,
                &callback,
                this_arg.clone(),
                &[value.clone(), value, Value::Object(set.clone())],
            )? {
                Completion::Return(_) => {}
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        Ok(Completion::Return(Value::Undefined))
    }

    fn new_set_iterator(
        &self,
        realm: ContextId,
        set: &ObjectRef,
        kind: SetIteratorKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.set_realm_data(realm)?.iterator_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let iterator = match state.heap.allocate_object(ObjectData::set_iterator(
            shape,
            Vec::new(),
            set.object_id(),
            kind,
        )) {
            Ok(iterator) => iterator,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), iterator))
    }

    fn call_set_iterator_factory(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        kind: SetIteratorKind,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Object(
            self.new_set_iterator(realm, &set, kind)?,
        )))
    }

    pub(in crate::runtime) fn call_set_iterator_next(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        match self.call_set_iterator_next_raw(realm, invocation)? {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(Completion::Return(
                Value::Object(self.new_iterator_result(realm, value, done)?),
            )),
        }
    }

    pub(in crate::runtime) fn call_set_iterator_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Set Iterator next did not receive an iterator-next invocation",
            ));
        };
        let Value::Object(iterator) = this_value else {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "Set Iterator object expected",
                )?,
            )));
        };
        let state = self
            .0
            .state
            .borrow()
            .heap
            .set_iterator_state(iterator.object_id());
        let (set, mut index, kind) = match state {
            Ok(state) => state,
            Err(HeapError::Invariant(_)) => {
                return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                    self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "Set Iterator object expected",
                    )?,
                )));
            }
            Err(error) => return Err(error.into()),
        };
        let Some(set_id) = set else {
            return Ok(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            });
        };
        loop {
            let record = self
                .0
                .state
                .borrow()
                .heap
                .set_records(set_id)?
                .get(index)
                .map(|record| record.key.clone());
            let Some(key) = record else {
                let mut state = self.0.state.borrow_mut();
                let cleanup = state.heap.finish_set_iterator(iterator.object_id())?;
                state.apply_cleanup(cleanup)?;
                return Ok(NativeInvokeOutcome::IteratorNextRaw {
                    value: Value::Undefined,
                    done: true,
                });
            };
            index = index.checked_add(1).ok_or(RuntimeError::Invariant(
                "Set Iterator record index overflowed",
            ))?;
            self.0
                .state
                .borrow_mut()
                .heap
                .set_set_iterator_index(iterator.object_id(), index)?;
            let Some(key) = key else {
                continue;
            };
            let value = self.root_raw_value(&key)?;
            let value = match kind {
                SetIteratorKind::Value => value,
                SetIteratorKind::KeyAndValue => {
                    Value::Object(self.new_array_from_values(realm, vec![value.clone(), value])?)
                }
            };
            return Ok(NativeInvokeOutcome::IteratorNextRaw { value, done: false });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_backed_symbol_atoms_return_after_set_mutations() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context
            .eval(
                r#"(function(){
                    var set = new Set();
                    var value = Symbol("set-value");
                    set.add(value);
                    set.add(value);
                    set.delete(value);
                    set.add(Symbol("set-clear-value"));
                    set.clear();
                })"#,
            )
            .unwrap()
        else {
            panic!("Set Symbol ownership probe was not callable");
        };
        let function = runtime.as_callable(&function).unwrap().unwrap();

        context
            .call(&function, Value::Undefined, &[])
            .expect("warm Set Symbol ownership probe");
        let baseline = runtime.test_atom_count();
        for _ in 0..3 {
            context
                .call(&function, Value::Undefined, &[])
                .expect("repeat Set Symbol ownership probe");
            assert_eq!(runtime.test_atom_count(), baseline);
        }
    }
}

impl Runtime {
    fn get_set_like_record(
        &self,
        realm: ContextId,
        target: Value,
    ) -> Result<NativeConversion<SetLikeRecord>, RuntimeError> {
        if matches!(target, Value::Null | Value::Undefined) {
            let kind = if matches!(target, Value::Null) {
                "null"
            } else {
                "undefined"
            };
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                &format!("cannot read property 'size' of {kind}"),
            )?));
        }
        let genuine_size = if let Value::Object(object) = &target {
            if !object.belongs_to(self) {
                return Err(RuntimeError::WrongRuntime("set-like object"));
            }
            let state = self.0.state.borrow();
            if matches!(
                state.heap.object(object.object_id())?.payload,
                ObjectPayload::Set { .. }
            ) {
                Some(state.heap.set_size(object.object_id())?)
            } else {
                None
            }
        } else {
            None
        };

        let size = if let Some(size) = genuine_size {
            i64::try_from(size).map_err(|_| {
                RuntimeError::Invariant("genuine Set size exceeded signed 64-bit range")
            })?
        } else {
            let size_key = self.intern_property_key("size")?;
            let size = match self.get_value_property_in_realm(realm, target.clone(), &size_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            let size = match self.native_to_number(realm, &size)? {
                NativeConversion::Value(size) => size,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            if size.is_nan() {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    ".size is not a number",
                )?));
            }
            let size = if size < i64::MIN as f64 {
                i64::MIN
            } else if size >= 2_f64.powi(63) {
                i64::MAX
            } else {
                size as i64
            };
            if size < 0 {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    ".size must be positive",
                )?));
            }
            size
        };

        let has_key = self.intern_property_key("has")?;
        let has = match self.get_value_property_in_realm(realm, target.clone(), &has_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(has, Value::Undefined) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                ".has is undefined",
            )?));
        }
        let has = match has {
            Value::Object(has) => match self.as_callable(&has)? {
                Some(has) => has,
                None => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        ".has is not a function",
                    )?));
                }
            },
            _ => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    ".has is not a function",
                )?));
            }
        };

        let keys_key = self.intern_property_key("keys")?;
        let keys = match self.get_value_property_in_realm(realm, target.clone(), &keys_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(keys, Value::Undefined) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                ".keys is undefined",
            )?));
        }
        let keys = match keys {
            Value::Object(keys) => match self.as_callable(&keys)? {
                Some(keys) => keys,
                None => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        ".keys is not a function",
                    )?));
                }
            },
            _ => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    ".keys is not a function",
                )?));
            }
        };

        Ok(NativeConversion::Value(SetLikeRecord {
            target,
            size,
            has,
            keys,
        }))
    }

    fn start_set_like_keys_iterator(
        &self,
        realm: ContextId,
        target: &Value,
        keys: &CallableRef,
    ) -> Result<NativeConversion<(Value, Value)>, RuntimeError> {
        let iterator = match self.call_internal(realm, keys, target.clone(), &[])? {
            Completion::Return(iterator) => iterator,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(iterator, Value::Null | Value::Undefined) {
            let kind = if matches!(iterator, Value::Null) {
                "null"
            } else {
                "undefined"
            };
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                &format!("cannot read property 'next' of {kind}"),
            )?));
        }
        let next_key = self.intern_property_key("next")?;
        let next = match self.get_value_property_in_realm(realm, iterator.clone(), &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        Ok(NativeConversion::Value((iterator, next)))
    }

    fn set_like_iterator_next(
        &self,
        realm: ContextId,
        iterator: &Value,
        next: Value,
    ) -> Result<ObjectIteratorStep, RuntimeError> {
        let next = match next {
            Value::Object(next) => match self.as_callable(&next)? {
                Some(next) => next,
                None => {
                    return Ok(ObjectIteratorStep::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                }
            },
            _ => {
                return Ok(ObjectIteratorStep::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            }
        };
        let result = match self.call_internal(realm, &next, iterator.clone(), &[])? {
            Completion::Return(Value::Object(result)) => result,
            Completion::Return(_) => {
                return Ok(ObjectIteratorStep::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "iterator must return an object",
                )?));
            }
            Completion::Throw(value) => return Ok(ObjectIteratorStep::Throw(value)),
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

    /// Mirror the pinned QuickJS Set-method call sites: they invoke
    /// `JS_IteratorClose(iter, FALSE)` for an early boolean result but ignore
    /// its status. The `return` getter/call remains observable, while any
    /// getter, call, callability, or result-brand failure is swallowed.
    fn close_set_iterator_for_set_method(
        &self,
        realm: ContextId,
        iterator: &Value,
    ) -> Result<(), RuntimeError> {
        let return_key = self.intern_property_key("return")?;
        let method = match self.get_value_property_in_realm(realm, iterator.clone(), &return_key)? {
            Completion::Return(value) => value,
            Completion::Throw(_) => return Ok(()),
        };
        if matches!(method, Value::Undefined | Value::Null) {
            return Ok(());
        }
        let method = match method {
            Value::Object(method) => match self.as_callable(&method)? {
                Some(method) => method,
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
        let _ = self.call_internal(realm, &method, iterator.clone(), &[])?;
        Ok(())
    }

    fn call_set_like_has(
        &self,
        realm: ContextId,
        record: &SetLikeRecord,
        value: Value,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        match self.call_internal(
            realm,
            &record.has,
            record.target.clone(),
            std::slice::from_ref(&value),
        )? {
            Completion::Return(value) => Ok(NativeConversion::Value(value.to_boolean())),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    fn copy_set_in_realm(
        &self,
        realm: ContextId,
        source: &ObjectRef,
    ) -> Result<ObjectRef, RuntimeError> {
        let copy = self.new_set_in_realm(realm)?;
        let mut index = 0_usize;
        while let Some(value) = self.next_live_set_value(source, &mut index)? {
            self.insert_set_record(&copy, value)?;
        }
        Ok(copy)
    }

    fn set_method_operand(
        &self,
        arguments: &NativeArguments,
        operation: &'static str,
    ) -> Result<Value, RuntimeError> {
        arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(operation))
    }

    fn call_set_is_disjoint_from(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let operand = self.set_method_operand(
            arguments,
            "Set.prototype.isDisjointFrom operand argv was not padded",
        )?;
        let other = match self.get_set_like_record(realm, operand)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        if i64::try_from(self.set_size_value(&set)?).unwrap_or(i64::MAX) <= other.size {
            let mut index = 0_usize;
            while let Some(value) = self.next_live_set_value(&set, &mut index)? {
                match self.call_set_like_has(realm, &other, value)? {
                    NativeConversion::Value(true) => {
                        return Ok(Completion::Return(Value::Bool(false)));
                    }
                    NativeConversion::Value(false) => {}
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
            return Ok(Completion::Return(Value::Bool(true)));
        }

        let (iterator, next) =
            match self.start_set_like_keys_iterator(realm, &other.target, &other.keys)? {
                NativeConversion::Value(iterator) => iterator,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        loop {
            let value = match self.set_like_iterator_next(realm, &iterator, next.clone())? {
                ObjectIteratorStep::Yield(value) => Self::normalized_set_key(value),
                ObjectIteratorStep::Done => return Ok(Completion::Return(Value::Bool(true))),
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let present = self.find_set_record(&set, &value)?.is_some();
            // Pinned QuickJS frees the yielded key before invoking return().
            drop(value);
            if present {
                self.close_set_iterator_for_set_method(realm, &iterator)?;
                return Ok(Completion::Return(Value::Bool(false)));
            }
        }
    }

    fn call_set_is_subset_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let operand = self.set_method_operand(
            arguments,
            "Set.prototype.isSubsetOf operand argv was not padded",
        )?;
        let other = match self.get_set_like_record(realm, operand)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if i64::try_from(self.set_size_value(&set)?).unwrap_or(i64::MAX) > other.size {
            return Ok(Completion::Return(Value::Bool(false)));
        }
        let mut index = 0_usize;
        while let Some(value) = self.next_live_set_value(&set, &mut index)? {
            match self.call_set_like_has(realm, &other, value)? {
                NativeConversion::Value(true) => {}
                NativeConversion::Value(false) => {
                    return Ok(Completion::Return(Value::Bool(false)));
                }
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        Ok(Completion::Return(Value::Bool(true)))
    }

    fn call_set_is_superset_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let operand = self.set_method_operand(
            arguments,
            "Set.prototype.isSupersetOf operand argv was not padded",
        )?;
        let other = match self.get_set_like_record(realm, operand)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if i64::try_from(self.set_size_value(&set)?).unwrap_or(i64::MAX) < other.size {
            return Ok(Completion::Return(Value::Bool(false)));
        }
        let (iterator, next) =
            match self.start_set_like_keys_iterator(realm, &other.target, &other.keys)? {
                NativeConversion::Value(iterator) => iterator,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        loop {
            let value = match self.set_like_iterator_next(realm, &iterator, next.clone())? {
                ObjectIteratorStep::Yield(value) => Self::normalized_set_key(value),
                ObjectIteratorStep::Done => return Ok(Completion::Return(Value::Bool(true))),
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let present = self.find_set_record(&set, &value)?.is_some();
            // Pinned QuickJS frees the yielded key before invoking return().
            drop(value);
            if !present {
                self.close_set_iterator_for_set_method(realm, &iterator)?;
                return Ok(Completion::Return(Value::Bool(false)));
            }
        }
    }

    fn call_set_intersection(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let operand = self.set_method_operand(
            arguments,
            "Set.prototype.intersection operand argv was not padded",
        )?;
        let other = match self.get_set_like_record(realm, operand)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        if i64::try_from(self.set_size_value(&set)?).unwrap_or(i64::MAX) > other.size {
            let (iterator, next) =
                match self.start_set_like_keys_iterator(realm, &other.target, &other.keys)? {
                    NativeConversion::Value(iterator) => iterator,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            let result = self.new_set_in_realm(realm)?;
            loop {
                let value = match self.set_like_iterator_next(realm, &iterator, next.clone())? {
                    ObjectIteratorStep::Yield(value) => Self::normalized_set_key(value),
                    ObjectIteratorStep::Done => {
                        return Ok(Completion::Return(Value::Object(result)));
                    }
                    ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if self.find_set_record(&set, &value)?.is_some()
                    && self.find_set_record(&result, &value)?.is_none()
                {
                    self.insert_set_record(&result, value)?;
                }
            }
        }

        let result = self.new_set_in_realm(realm)?;
        let mut index = 0_usize;
        while let Some(value) = self.next_live_set_value(&set, &mut index)? {
            let present = match self.call_set_like_has(realm, &other, value.clone())? {
                NativeConversion::Value(present) => present,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if present && self.find_set_record(&result, &value)?.is_none() {
                self.insert_set_record(&result, value)?;
            }
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    fn call_set_difference(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let operand = self.set_method_operand(
            arguments,
            "Set.prototype.difference operand argv was not padded",
        )?;
        let other = match self.get_set_like_record(realm, operand)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = self.copy_set_in_realm(realm, &set)?;

        if i64::try_from(self.set_size_value(&set)?).unwrap_or(i64::MAX) <= other.size {
            let mut index = 0_usize;
            while let Some(value) = self.next_live_set_value(&result, &mut index)? {
                let present = match self.call_set_like_has(realm, &other, value.clone())? {
                    NativeConversion::Value(present) => present,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if present {
                    self.delete_set_record(&result, &value)?;
                }
            }
            return Ok(Completion::Return(Value::Object(result)));
        }

        let (iterator, next) =
            match self.start_set_like_keys_iterator(realm, &other.target, &other.keys)? {
                NativeConversion::Value(iterator) => iterator,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        loop {
            let value = match self.set_like_iterator_next(realm, &iterator, next.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(result)));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            self.delete_set_record(&result, &value)?;
        }
    }

    fn call_set_symmetric_difference(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let operand = self.set_method_operand(
            arguments,
            "Set.prototype.symmetricDifference operand argv was not padded",
        )?;
        let other = match self.get_set_like_record(realm, operand)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let SetLikeRecord {
            target, has, keys, ..
        } = other;
        // QuickJS releases this otherwise-unused method before calling keys().
        drop(has);
        // QuickJS starts the foreign iterator before copying `this`; getters
        // and the keys call can therefore mutate what is copied.
        let (iterator, next) = match self.start_set_like_keys_iterator(realm, &target, &keys)? {
            NativeConversion::Value(iterator) => iterator,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = self.copy_set_in_realm(realm, &set)?;
        loop {
            let value = match self.set_like_iterator_next(realm, &iterator, next.clone())? {
                ObjectIteratorStep::Yield(value) => Self::normalized_set_key(value),
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(result)));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            // The first lookup is deliberately against the current original,
            // not the copy. Mutating foreign iterators make this observable.
            if self.find_set_record(&set, &value)?.is_some() {
                self.delete_set_record(&result, &value)?;
            } else if self.find_set_record(&result, &value)?.is_none() {
                self.insert_set_record(&result, value)?;
            }
        }
    }

    fn call_set_union(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let set = match self.set_receiver(realm, invocation, false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let operand =
            self.set_method_operand(arguments, "Set.prototype.union operand argv was not padded")?;
        let other = match self.get_set_like_record(realm, operand)? {
            NativeConversion::Value(record) => record,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let SetLikeRecord {
            target, has, keys, ..
        } = other;
        // Match QuickJS's explicit JS_FreeValue(has) before invoking keys().
        drop(has);
        let (iterator, next) = match self.start_set_like_keys_iterator(realm, &target, &keys)? {
            NativeConversion::Value(iterator) => iterator,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = self.copy_set_in_realm(realm, &set)?;
        loop {
            let value = match self.set_like_iterator_next(realm, &iterator, next.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(result)));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            self.insert_set_record(&result, value)?;
        }
    }
}
