//! `%Map%`, Map Iterator, and strong ordered-record semantics.
//!
//! This follows the pinned QuickJS `JS_AddIntrinsicMapSet`/`js_map_*`
//! boundary.  The heap owns the insertion-ordered records and tombstones;
//! this module owns all observable iteration, callback, realm, and descriptor
//! behavior.  `Set` and the weak collections deliberately remain separate
//! follow-up slices rather than weakening the Map brand here.

use crate::heap::{MapIteratorKind, MapNativeKind, MapRealmData};

use super::super::*;
use super::object::ObjectIteratorStep;

impl Runtime {
    pub(in crate::runtime) fn initialize_map_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        iterator_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let map_prototype = self.new_object(Some(object_prototype))?;
        let map_iterator_prototype = self.new_object(Some(iterator_prototype))?;

        for (kind, name, length, readable) in [
            (MapNativeKind::Set, "set", 2, 2),
            (MapNativeKind::Get, "get", 1, 1),
            (MapNativeKind::GetOrInsert, "getOrInsert", 2, 2),
            (
                MapNativeKind::GetOrInsertComputed,
                "getOrInsertComputed",
                2,
                2,
            ),
            (MapNativeKind::Has, "has", 1, 1),
            (MapNativeKind::Delete, "delete", 1, 1),
            (MapNativeKind::Clear, "clear", 0, 0),
        ] {
            self.define_native_builtin_auto_init(
                &map_prototype,
                realm,
                NativeFunctionId::Map(kind),
                name,
                length,
                readable,
            )?;
        }
        self.define_native_builtin_getter_on(
            &map_prototype,
            function_prototype,
            realm,
            NativeFunctionId::Map(MapNativeKind::Size),
            "size",
            "get size",
        )?;
        for (kind, name, length, readable) in [
            (MapNativeKind::ForEach, "forEach", 1, 2),
            (
                MapNativeKind::Iterator(MapIteratorKind::Value),
                "values",
                0,
                0,
            ),
            (MapNativeKind::Iterator(MapIteratorKind::Key), "keys", 0, 0),
            (
                MapNativeKind::Iterator(MapIteratorKind::KeyAndValue),
                "entries",
                0,
                0,
            ),
        ] {
            self.define_native_builtin_auto_init(
                &map_prototype,
                realm,
                NativeFunctionId::Map(kind),
                name,
                length,
                readable,
            )?;
        }

        // QuickJS's alias table preserves the exact entries-function identity.
        let entries_key = self.intern_property_key("entries")?;
        let entries = match self.get_property_in_realm(realm, &map_prototype, &entries_key)? {
            Completion::Return(value @ Value::Object(_)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "Map.prototype.entries was not callable during bootstrap",
                ));
            }
            Completion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "Map.prototype.entries initialization threw during bootstrap",
                ));
            }
        };
        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        if !self.define_own_property(
            &map_prototype,
            &iterator_key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(entries),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Map iterator alias definition was rejected",
            ));
        }
        self.define_to_string_tag(&map_prototype, "Map")?;

        self.define_native_builtin_auto_init(
            &map_iterator_prototype,
            realm,
            NativeFunctionId::MapIteratorNext,
            "next",
            0,
            0,
        )?;
        self.define_to_string_tag(&map_iterator_prototype, "Map Iterator")?;

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::Map(MapNativeKind::Constructor),
            1,
            "Map",
            0,
        )?;
        self.define_native_builtin_auto_init(
            constructor.as_object(),
            realm,
            NativeFunctionId::Map(MapNativeKind::GroupBy),
            "groupBy",
            2,
            2,
        )?;
        let species_getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::Map(MapNativeKind::Species),
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
                "Map species definition was rejected",
            ));
        }

        self.define_function_data_property(
            global_object,
            "Map",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, &map_prototype)?;
        self.0.state.borrow_mut().heap.attach_map_intrinsics(
            realm,
            MapRealmData {
                prototype: map_prototype.object_id(),
                iterator_prototype: map_iterator_prototype.object_id(),
            },
        )?;
        Ok(())
    }

    fn define_to_string_tag(
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
                "Map intrinsic toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    fn map_realm_data(&self, realm: ContextId) -> Result<MapRealmData, RuntimeError> {
        self.0
            .state
            .borrow()
            .heap
            .context(realm)?
            .map
            .ok_or(RuntimeError::Invariant("realm has no Map intrinsics"))
    }

    fn new_map_object(&self, prototype: &ObjectRef) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Map prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::map(shape, Vec::new()))
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

    fn new_map_in_realm(&self, realm: ContextId) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.map_realm_data(realm)?.prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        self.new_map_object(&prototype)
    }

    fn map_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(new_target) = new_target else {
            return Err(RuntimeError::Invariant(
                "Map constructor new.target was not an object",
            ));
        };
        let key = self.intern_property_key("prototype")?;
        match self.get_property_in_realm(realm, &new_target, &key)? {
            Completion::Return(Value::Object(prototype)) => Ok(NativeConversion::Value(prototype)),
            Completion::Return(_) => {
                let callable = self.callable_from_value(Value::Object(new_target))?;
                let fallback_realm = self.callable_realm(&callable)?;
                let prototype = self.map_realm_data(fallback_realm)?.prototype;
                Ok(NativeConversion::Value(ObjectRef::from_borrowed_handle(
                    self.clone(),
                    prototype,
                )?))
            }
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(in crate::runtime) fn call_map_native(
        &self,
        realm: ContextId,
        kind: MapNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            MapNativeKind::Constructor => self.call_map_constructor(realm, invocation, arguments),
            MapNativeKind::Species => self.call_map_species(invocation),
            MapNativeKind::GroupBy => self.call_map_group_by(realm, invocation, arguments),
            MapNativeKind::Set => self.call_map_set(realm, invocation, arguments),
            MapNativeKind::Get => self.call_map_get(realm, invocation, arguments),
            MapNativeKind::GetOrInsert => {
                self.call_map_get_or_insert(realm, invocation, arguments, false)
            }
            MapNativeKind::GetOrInsertComputed => {
                self.call_map_get_or_insert(realm, invocation, arguments, true)
            }
            MapNativeKind::Has => self.call_map_has(realm, invocation, arguments),
            MapNativeKind::Delete => self.call_map_delete(realm, invocation, arguments),
            MapNativeKind::Clear => self.call_map_clear(realm, invocation),
            MapNativeKind::Size => self.call_map_size(realm, invocation),
            MapNativeKind::ForEach => self.call_map_for_each(realm, invocation, arguments),
            MapNativeKind::Iterator(kind) => {
                self.call_map_iterator_factory(realm, invocation, kind)
            }
        }
    }

    fn call_map_species(&self, invocation: NativeInvocation) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Map species did not receive a getter invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    fn call_map_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Map constructor did not receive a constructor invocation",
            ));
        };
        let prototype = match self.map_prototype_from_new_target(realm, new_target)? {
            NativeConversion::Value(prototype) => prototype,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let map = self.new_map_object(&prototype)?;
        if arguments.actual_arg_count == 0 {
            return Ok(Completion::Return(Value::Object(map)));
        }
        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant("Map iterable argv was not padded"))?;
        if matches!(iterable, Value::Null | Value::Undefined) {
            return Ok(Completion::Return(Value::Object(map)));
        }

        let set_key = self.intern_property_key("set")?;
        let adder = match self.get_property_in_realm(realm, &map, &set_key)? {
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
        let zero = self.intern_property_key("0")?;
        let one = self.intern_property_key("1")?;
        loop {
            let item = match self.object_iterator_next(realm, &iterator, next.clone())? {
                ObjectIteratorStep::Yield(Value::Object(item)) => item,
                ObjectIteratorStep::Yield(_) => {
                    let value =
                        self.new_native_error(realm, NativeErrorKind::Type, "not an object")?;
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(map)));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let key = match self.get_property_in_realm(realm, &item, &zero)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            };
            let value = match self.get_property_in_realm(realm, &item, &one)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            };
            match self.call_internal(realm, &adder, Value::Object(map.clone()), &[key, value])? {
                Completion::Return(_) => {}
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            }
        }
    }

    fn map_receiver(
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
                    "Map method received the wrong native invocation",
                ));
            }
        };
        let Value::Object(object) = this_value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "Map object expected",
            )?));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Map receiver"));
        }
        let is_map = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::Map { .. }
        );
        if !is_map {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "Map object expected",
            )?));
        }
        Ok(NativeConversion::Value(object))
    }

    fn normalized_map_key(value: Value) -> Value {
        match value {
            Value::Float(0.0) => Value::Int(0),
            value => value,
        }
    }

    fn find_map_record(
        &self,
        map: &ObjectRef,
        key: &Value,
    ) -> Result<Option<(usize, RawValue)>, RuntimeError> {
        let records = self
            .0
            .state
            .borrow()
            .heap
            .map_records(map.object_id())?
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                record
                    .key
                    .as_ref()
                    .map(|key| (index, key.clone(), record.value.clone()))
            })
            .collect::<Vec<_>>();
        for (index, candidate, value) in records {
            let candidate = self.root_raw_value(&candidate)?;
            if candidate.same_value_zero(key) {
                return Ok(Some((index, value)));
            }
        }
        Ok(None)
    }

    fn set_map_record(
        &self,
        map: &ObjectRef,
        key: Value,
        value: Value,
    ) -> Result<(), RuntimeError> {
        self.validate_value_domain(&key, "Map key")?;
        self.validate_value_domain(&value, "Map value")?;
        let key = Self::normalized_map_key(key);
        let existing = self.find_map_record(map, &key)?.map(|(index, _)| index);
        let raw_key = self.raw_property_value(&key)?;
        let raw_value = self.raw_property_value(&value)?;
        let mut state = self.0.state.borrow_mut();
        let retained = if existing.is_some() {
            state.retain_raw_value_atoms([&raw_value])?
        } else {
            state.retain_raw_value_atoms([&raw_key, &raw_value])?
        };
        let result = if let Some(index) = existing {
            state
                .heap
                .map_replace_record_value(map.object_id(), index, raw_value)
        } else {
            state
                .heap
                .map_insert_record(map.object_id(), raw_key, raw_value)
        };
        let cleanup = match result {
            Ok(cleanup) => cleanup,
            Err(error) => {
                state.release_atoms(retained)?;
                return Err(error.into());
            }
        };
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(key);
        drop(value);
        Ok(())
    }

    fn delete_map_record(&self, map: &ObjectRef, key: &Value) -> Result<bool, RuntimeError> {
        let key = Self::normalized_map_key(key.clone());
        let Some((index, _)) = self.find_map_record(map, &key)? else {
            return Ok(false);
        };
        let mut state = self.0.state.borrow_mut();
        let cleanup = state.heap.map_delete_record(map.object_id(), index)?;
        state.apply_cleanup(cleanup)?;
        Ok(true)
    }

    fn call_map_set(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let map = match self.map_receiver(realm, invocation, false)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Map.prototype.set key argv was not padded",
            ))?;
        let value = arguments
            .readable
            .get(1)
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Map.prototype.set value argv was not padded",
            ))?;
        self.set_map_record(&map, key, value)?;
        Ok(Completion::Return(Value::Object(map)))
    }

    fn call_map_get(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let map = match self.map_receiver(realm, invocation, false)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = Self::normalized_map_key(arguments.readable.first().cloned().ok_or(
            RuntimeError::Invariant("Map.prototype.get key argv was not padded"),
        )?);
        let value = match self.find_map_record(&map, &key)? {
            Some((_, value)) => self.root_raw_value(&value)?,
            None => Value::Undefined,
        };
        Ok(Completion::Return(value))
    }

    fn call_map_has(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let map = match self.map_receiver(realm, invocation, false)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = Self::normalized_map_key(arguments.readable.first().cloned().ok_or(
            RuntimeError::Invariant("Map.prototype.has key argv was not padded"),
        )?);
        Ok(Completion::Return(Value::Bool(
            self.find_map_record(&map, &key)?.is_some(),
        )))
    }

    fn call_map_delete(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let map = match self.map_receiver(realm, invocation, false)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Map.prototype.delete key argv was not padded",
            ))?;
        Ok(Completion::Return(Value::Bool(
            self.delete_map_record(&map, &key)?,
        )))
    }

    fn call_map_clear(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let map = match self.map_receiver(realm, invocation, false)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut state = self.0.state.borrow_mut();
        let cleanup = state.heap.map_clear(map.object_id())?;
        state.apply_cleanup(cleanup)?;
        Ok(Completion::Return(Value::Undefined))
    }

    fn call_map_size(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let map = match self.map_receiver(realm, invocation, true)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let size = self.0.state.borrow().heap.map_size(map.object_id())?;
        Ok(Completion::Return(Value::number(size as f64)))
    }

    fn call_map_get_or_insert(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        computed: bool,
    ) -> Result<Completion, RuntimeError> {
        let map = match self.map_receiver(realm, invocation, false)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key = Self::normalized_map_key(arguments.readable.first().cloned().ok_or(
            RuntimeError::Invariant("Map getOrInsert key argv was not padded"),
        )?);
        let second = arguments
            .readable
            .get(1)
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Map getOrInsert value argv was not padded",
            ))?;
        let callback = if computed {
            let Value::Object(callback) = &second else {
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
            Some(callback)
        } else {
            None
        };
        if let Some((_, value)) = self.find_map_record(&map, &key)? {
            return Ok(Completion::Return(self.root_raw_value(&value)?));
        }
        let value = if let Some(callback) = callback {
            match self.call_internal(
                realm,
                &callback,
                Value::Undefined,
                std::slice::from_ref(&key),
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            second
        };
        if computed {
            // Pinned QuickJS removes a callback-created entry before appending
            // the callback result, so insertion order and overwrite behavior
            // are both observable.
            self.delete_map_record(&map, &key)?;
        }
        self.set_map_record(&map, key, value.clone())?;
        Ok(Completion::Return(value))
    }

    fn call_map_for_each(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let map = match self.map_receiver(realm, invocation, false)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callback = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Map.prototype.forEach callback argv was not padded",
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
        loop {
            let record = self
                .0
                .state
                .borrow()
                .heap
                .map_records(map.object_id())?
                .get(index)
                .map(|record| (record.key.clone(), record.value.clone()));
            let Some((key, value)) = record else {
                break;
            };
            index = index.checked_add(1).ok_or(RuntimeError::Invariant(
                "Map forEach record index overflowed",
            ))?;
            let Some(key) = key else {
                continue;
            };
            let key = self.root_raw_value(&key)?;
            let value = self.root_raw_value(&value)?;
            match self.call_internal(
                realm,
                &callback,
                this_arg.clone(),
                &[value, key, Value::Object(map.clone())],
            )? {
                Completion::Return(_) => {}
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        Ok(Completion::Return(Value::Undefined))
    }

    fn new_map_iterator(
        &self,
        realm: ContextId,
        map: &ObjectRef,
        kind: MapIteratorKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.map_realm_data(realm)?.iterator_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let iterator = match state.heap.allocate_object(ObjectData::map_iterator(
            shape,
            Vec::new(),
            map.object_id(),
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

    fn call_map_iterator_factory(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        kind: MapIteratorKind,
    ) -> Result<Completion, RuntimeError> {
        let map = match self.map_receiver(realm, invocation, false)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Object(
            self.new_map_iterator(realm, &map, kind)?,
        )))
    }

    pub(in crate::runtime) fn call_map_iterator_next(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        match self.call_map_iterator_next_raw(realm, invocation)? {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(Completion::Return(
                Value::Object(self.new_iterator_result(realm, value, done)?),
            )),
        }
    }

    pub(in crate::runtime) fn call_map_iterator_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Map Iterator next did not receive an iterator-next invocation",
            ));
        };
        let Value::Object(iterator) = this_value else {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "Map Iterator object expected",
                )?,
            )));
        };
        let state = self
            .0
            .state
            .borrow()
            .heap
            .map_iterator_state(iterator.object_id());
        let (map, mut index, kind) = match state {
            Ok(state) => state,
            Err(HeapError::Invariant(_)) => {
                return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                    self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "Map Iterator object expected",
                    )?,
                )));
            }
            Err(error) => return Err(error.into()),
        };
        let Some(map_id) = map else {
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
                .map_records(map_id)?
                .get(index)
                .map(|record| (record.key.clone(), record.value.clone()));
            let Some((key, value)) = record else {
                let mut state = self.0.state.borrow_mut();
                let cleanup = state.heap.finish_map_iterator(iterator.object_id())?;
                state.apply_cleanup(cleanup)?;
                return Ok(NativeInvokeOutcome::IteratorNextRaw {
                    value: Value::Undefined,
                    done: true,
                });
            };
            index = index.checked_add(1).ok_or(RuntimeError::Invariant(
                "Map Iterator record index overflowed",
            ))?;
            self.0
                .state
                .borrow_mut()
                .heap
                .set_map_iterator_index(iterator.object_id(), index)?;
            let Some(key) = key else {
                continue;
            };
            let key = self.root_raw_value(&key)?;
            let value = match kind {
                MapIteratorKind::Key => key,
                MapIteratorKind::Value => self.root_raw_value(&value)?,
                MapIteratorKind::KeyAndValue => Value::Object(
                    self.new_array_from_values(realm, vec![key, self.root_raw_value(&value)?])?,
                ),
            };
            return Ok(NativeInvokeOutcome::IteratorNextRaw { value, done: false });
        }
    }

    fn call_map_group_by(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Map.groupBy did not receive a generic invocation",
            ));
        };
        let callback = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "Map.groupBy callback argv was not padded",
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
                "Map.groupBy iterable argv was not padded",
            ))?;
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
        let groups = self.new_map_in_realm(realm)?;
        let callback_this = Value::Object(self.global_object_for_realm(realm)?);
        let mut index = 0_u64;
        loop {
            if index >= MAX_SAFE_INTEGER {
                let exception =
                    self.new_native_error(realm, NativeErrorKind::Type, "too many elements")?;
                self.close_iterator_preserving_throw(realm, &iterator)?;
                return Ok(Completion::Throw(exception));
            }
            let value = match self.object_iterator_next(realm, &iterator, next.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(groups)));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let key = match self.call_internal(
                realm,
                &callback,
                callback_this.clone(),
                &[value.clone(), Value::number(index as f64)],
            )? {
                Completion::Return(value) => Self::normalized_map_key(value),
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            };
            let group = match self.find_map_record(&groups, &key)? {
                Some((_, value)) => match self.root_raw_value(&value)? {
                    Value::Object(group) => group,
                    _ => {
                        return Err(RuntimeError::Invariant(
                            "Map.groupBy result contained a non-Array group",
                        ));
                    }
                },
                None => {
                    let group = self.new_array(realm)?;
                    self.set_map_record(&groups, key, Value::Object(group.clone()))?;
                    group
                }
            };
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
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
            index = index.checked_add(1).ok_or(RuntimeError::Invariant(
                "Map.groupBy index overflowed Uint64",
            ))?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_backed_symbol_atoms_return_after_map_mutations() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context
            .eval(
                r#"(function(){
                    var map = new Map();
                    var key = Symbol("map-key");
                    map.set(key, Symbol("map-first-value"));
                    map.set(key, Symbol("map-replacement-value"));
                    map.delete(key);
                    map.set(Symbol("map-clear-key"), Symbol("map-clear-value"));
                    map.clear();
                })"#,
            )
            .unwrap()
        else {
            panic!("Map Symbol ownership probe was not callable");
        };
        let function = runtime.as_callable(&function).unwrap().unwrap();

        context
            .call(&function, Value::Undefined, &[])
            .expect("warm Map Symbol ownership probe");
        let baseline = runtime.test_atom_count();
        for _ in 0..3 {
            context
                .call(&function, Value::Undefined, &[])
                .expect("repeat Map Symbol ownership probe");
            assert_eq!(runtime.test_atom_count(), baseline);
        }
    }
}
