//! `%WeakMap%` / `%WeakSet%` and QuickJS-compatible weak-key behavior.
//!
//! The heap stores generational object/atom identities without retaining the
//! key. WeakMap values remain ordinary strong edges. Dead records are skipped
//! by lookup and reclaimed at the explicit-GC boundary, matching pinned
//! QuickJS's weak-object removal pass.

use crate::heap::{
    WeakCollectionKey, WeakMapNativeKind, WeakMapRealmData, WeakSetNativeKind, WeakSetRealmData,
};

use super::super::*;
use super::object::ObjectIteratorStep;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeakCollectionKind {
    Map,
    Set,
}

impl WeakCollectionKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Map => "WeakMap",
            Self::Set => "WeakSet",
        }
    }

    const fn adder(self) -> &'static str {
        match self {
            Self::Map => "set",
            Self::Set => "add",
        }
    }
}

impl Runtime {
    fn weak_collection_mutation_error(error: HeapError) -> RuntimeError {
        match error {
            HeapError::Allocation { .. } => {
                RuntimeError::Engine(Error::new(ErrorKind::JsInternal, "out of memory"))
            }
            error => RuntimeError::Heap(error),
        }
    }

    pub(in crate::runtime) fn initialize_weak_collection_intrinsics(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        self.initialize_weak_map_intrinsic(
            realm,
            function_prototype,
            object_prototype,
            global_object,
        )?;
        self.initialize_weak_set_intrinsic(
            realm,
            function_prototype,
            object_prototype,
            global_object,
        )
    }

    fn initialize_weak_map_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let prototype = self.new_object(Some(object_prototype))?;
        for (kind, name, length, readable) in [
            (WeakMapNativeKind::Set, "set", 2, 2),
            (WeakMapNativeKind::Get, "get", 1, 1),
            (WeakMapNativeKind::GetOrInsert, "getOrInsert", 2, 2),
            (
                WeakMapNativeKind::GetOrInsertComputed,
                "getOrInsertComputed",
                2,
                2,
            ),
            (WeakMapNativeKind::Has, "has", 1, 1),
            (WeakMapNativeKind::Delete, "delete", 1, 1),
        ] {
            self.define_native_builtin_auto_init(
                &prototype,
                realm,
                NativeFunctionId::WeakMap(kind),
                name,
                length,
                readable,
            )?;
        }
        self.define_weak_collection_to_string_tag(&prototype, "WeakMap")?;

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::WeakMap(WeakMapNativeKind::Constructor),
            1,
            "WeakMap",
            0,
        )?;
        self.define_function_data_property(
            global_object,
            "WeakMap",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, &prototype)?;
        self.0.state.borrow_mut().heap.attach_weak_map_intrinsics(
            realm,
            WeakMapRealmData {
                prototype: prototype.object_id(),
            },
        )?;
        Ok(())
    }

    fn initialize_weak_set_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let prototype = self.new_object(Some(object_prototype))?;
        for (kind, name, length, readable) in [
            (WeakSetNativeKind::Add, "add", 1, 1),
            (WeakSetNativeKind::Has, "has", 1, 1),
            (WeakSetNativeKind::Delete, "delete", 1, 1),
        ] {
            self.define_native_builtin_auto_init(
                &prototype,
                realm,
                NativeFunctionId::WeakSet(kind),
                name,
                length,
                readable,
            )?;
        }
        self.define_weak_collection_to_string_tag(&prototype, "WeakSet")?;

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::WeakSet(WeakSetNativeKind::Constructor),
            1,
            "WeakSet",
            0,
        )?;
        self.define_function_data_property(
            global_object,
            "WeakSet",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, &prototype)?;
        self.0.state.borrow_mut().heap.attach_weak_set_intrinsics(
            realm,
            WeakSetRealmData {
                prototype: prototype.object_id(),
            },
        )?;
        Ok(())
    }

    fn define_weak_collection_to_string_tag(
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
                "weak collection toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    fn weak_map_realm_data(&self, realm: ContextId) -> Result<WeakMapRealmData, RuntimeError> {
        self.0
            .state
            .borrow()
            .heap
            .context(realm)?
            .weak_map
            .ok_or(RuntimeError::Invariant("realm has no WeakMap intrinsics"))
    }

    fn weak_set_realm_data(&self, realm: ContextId) -> Result<WeakSetRealmData, RuntimeError> {
        self.0
            .state
            .borrow()
            .heap
            .context(realm)?
            .weak_set
            .ok_or(RuntimeError::Invariant("realm has no WeakSet intrinsics"))
    }

    fn weak_collection_prototype(
        &self,
        realm: ContextId,
        kind: WeakCollectionKind,
    ) -> Result<ObjectId, RuntimeError> {
        Ok(match kind {
            WeakCollectionKind::Map => self.weak_map_realm_data(realm)?.prototype,
            WeakCollectionKind::Set => self.weak_set_realm_data(realm)?.prototype,
        })
    }

    fn new_weak_collection_object(
        &self,
        prototype: &ObjectRef,
        kind: WeakCollectionKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("weak collection prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let data = match kind {
            WeakCollectionKind::Map => ObjectData::weak_map(shape, Vec::new()),
            WeakCollectionKind::Set => ObjectData::weak_set(shape, Vec::new()),
        };
        let object = match state.heap.allocate_object(data) {
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

    fn weak_collection_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
        kind: WeakCollectionKind,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        self.prototype_from_constructor_value(realm, &new_target, |fallback_realm| {
            let prototype = self.weak_collection_prototype(fallback_realm, kind)?;
            Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
        })
    }

    fn call_weak_collection_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        kind: WeakCollectionKind,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "weak collection constructor did not receive a constructor invocation",
            ));
        };
        let prototype =
            match self.weak_collection_prototype_from_new_target(realm, new_target, kind)? {
                NativeConversion::Value(prototype) => prototype,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let collection = self.new_weak_collection_object(&prototype, kind)?;
        if arguments.actual_arg_count == 0 {
            return Ok(Completion::Return(Value::Object(collection)));
        }
        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "weak collection iterable argv was not padded",
            ))?;
        if matches!(iterable, Value::Null | Value::Undefined) {
            return Ok(Completion::Return(Value::Object(collection)));
        }

        let adder_key = self.intern_property_key(kind.adder())?;
        let adder = match self.get_property_in_realm(realm, &collection, &adder_key)? {
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
                ObjectIteratorStep::Yield(item) => item,
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(collection)));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let arguments = match kind {
                WeakCollectionKind::Set => vec![item],
                WeakCollectionKind::Map => {
                    let item = match item {
                        Value::Object(item) => item,
                        item => {
                            let value = self.new_native_error(
                                realm,
                                NativeErrorKind::Type,
                                "not an object",
                            )?;
                            // QuickJS releases the yielded value before
                            // IteratorClose, which is observable when return()
                            // performs a collection.
                            drop(item);
                            self.close_iterator_preserving_throw(realm, &iterator)?;
                            return Ok(Completion::Throw(value));
                        }
                    };
                    let key = match self.get_property_in_realm(realm, &item, &zero)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => {
                            drop(item);
                            self.close_iterator_preserving_throw(realm, &iterator)?;
                            return Ok(Completion::Throw(value));
                        }
                    };
                    let value = match self.get_property_in_realm(realm, &item, &one)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => {
                            drop(key);
                            drop(item);
                            self.close_iterator_preserving_throw(realm, &iterator)?;
                            return Ok(Completion::Throw(value));
                        }
                    };
                    vec![key, value]
                }
            };
            let adder_completion =
                self.call_internal(realm, &adder, Value::Object(collection.clone()), &arguments)?;
            // Match QuickJS's lifetime boundary: the current element (or
            // WeakMap key/value pair) is released before IteratorClose.
            drop(arguments);
            match adder_completion {
                Completion::Return(_) => {}
                Completion::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            }
        }
    }

    fn weak_collection_receiver(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        kind: WeakCollectionKind,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "weak collection method received the wrong native invocation",
            ));
        };
        let Value::Object(object) = this_value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                &format!("{} object expected", kind.name()),
            )?));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("weak collection receiver"));
        }
        let matches_brand = matches!(
            (
                kind,
                &self
                    .0
                    .state
                    .borrow()
                    .heap
                    .object(object.object_id())?
                    .payload,
            ),
            (WeakCollectionKind::Map, ObjectPayload::WeakMap { .. })
                | (WeakCollectionKind::Set, ObjectPayload::WeakSet { .. })
        );
        if !matches_brand {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                &format!("{} object expected", kind.name()),
            )?));
        }
        Ok(NativeConversion::Value(object))
    }

    fn weak_collection_key(
        &self,
        value: &Value,
        role: &'static str,
    ) -> Result<Option<WeakCollectionKey>, RuntimeError> {
        match value {
            Value::Object(object) => {
                if !object.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime(role));
                }
                Ok(Some(WeakCollectionKey::Object(object.object_id())))
            }
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime(role));
                }
                let atom = symbol.atom();
                let can_be_held_weakly =
                    self.0.state.borrow().atoms.kind(atom)? == AtomKind::Symbol;
                Ok(can_be_held_weakly.then_some(WeakCollectionKey::Symbol(atom)))
            }
            _ => Ok(None),
        }
    }

    fn invalid_weak_key(
        &self,
        realm: ContextId,
        kind: WeakCollectionKind,
    ) -> Result<Completion, RuntimeError> {
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            &format!("invalid value used as {} key", kind.name()),
        )?))
    }

    fn find_weak_map_record(
        &self,
        map: &ObjectRef,
        key: WeakCollectionKey,
    ) -> Result<Option<RawValue>, RuntimeError> {
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .weak_map_get(map.object_id(), key)?
            .cloned())
    }

    fn set_weak_map_record(
        &self,
        map: &ObjectRef,
        key: WeakCollectionKey,
        value: Value,
    ) -> Result<(), RuntimeError> {
        self.validate_value_domain(&value, "WeakMap value")?;
        let raw_value = self.raw_property_value(&value)?;
        let mut state = self.0.state.borrow_mut();
        let retained = state.retain_raw_value_atoms([&raw_value])?;
        let result = state.heap.weak_map_set(map.object_id(), key, raw_value);
        let cleanup = match result {
            Ok(cleanup) => cleanup,
            Err(error) => {
                state.release_atoms(retained)?;
                return Err(Self::weak_collection_mutation_error(error));
            }
        };
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(value);
        Ok(())
    }

    fn delete_weak_map_record(
        &self,
        map: &ObjectRef,
        key: WeakCollectionKey,
    ) -> Result<bool, RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let (deleted, cleanup) = state.heap.weak_map_delete(map.object_id(), key)?;
        state.apply_cleanup(cleanup)?;
        Ok(deleted)
    }

    fn has_weak_set_record(
        &self,
        set: &ObjectRef,
        key: WeakCollectionKey,
    ) -> Result<bool, RuntimeError> {
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .weak_set_has(set.object_id(), key)?)
    }

    fn insert_weak_set_record(
        &self,
        set: &ObjectRef,
        key: WeakCollectionKey,
    ) -> Result<(), RuntimeError> {
        self.0
            .state
            .borrow_mut()
            .heap
            .weak_set_add(set.object_id(), key)
            .map_err(Self::weak_collection_mutation_error)?;
        Ok(())
    }

    fn delete_weak_set_record(
        &self,
        set: &ObjectRef,
        key: WeakCollectionKey,
    ) -> Result<bool, RuntimeError> {
        Ok(self
            .0
            .state
            .borrow_mut()
            .heap
            .weak_set_delete(set.object_id(), key)?)
    }

    pub(in crate::runtime) fn call_weak_map_native(
        &self,
        realm: ContextId,
        kind: WeakMapNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if kind == WeakMapNativeKind::Constructor {
            return self.call_weak_collection_constructor(
                realm,
                invocation,
                arguments,
                WeakCollectionKind::Map,
            );
        }
        let map = match self.weak_collection_receiver(realm, invocation, WeakCollectionKind::Map)? {
            NativeConversion::Value(map) => map,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key_value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant("WeakMap key argv was not padded"))?;

        if kind == WeakMapNativeKind::GetOrInsertComputed {
            let callback_value =
                arguments
                    .readable
                    .get(1)
                    .cloned()
                    .ok_or(RuntimeError::Invariant(
                        "WeakMap computed value argv was not padded",
                    ))?;
            let Value::Object(callback) = callback_value else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            let Some(callback) = self.as_callable(&callback)? else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            let Some(key) = self.weak_collection_key(&key_value, "WeakMap key")? else {
                return self.invalid_weak_key(realm, WeakCollectionKind::Map);
            };
            if let Some(value) = self.find_weak_map_record(&map, key)? {
                return Ok(Completion::Return(self.root_raw_value(&value)?));
            }
            let value = match self.call_internal(
                realm,
                &callback,
                Value::Undefined,
                std::slice::from_ref(&key_value),
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            self.delete_weak_map_record(&map, key)?;
            self.set_weak_map_record(&map, key, value.clone())?;
            return Ok(Completion::Return(value));
        }

        let key = self.weak_collection_key(&key_value, "WeakMap key")?;
        match kind {
            WeakMapNativeKind::Set => {
                let Some(key) = key else {
                    return self.invalid_weak_key(realm, WeakCollectionKind::Map);
                };
                let value = arguments
                    .readable
                    .get(1)
                    .cloned()
                    .ok_or(RuntimeError::Invariant("WeakMap value argv was not padded"))?;
                self.set_weak_map_record(&map, key, value)?;
                Ok(Completion::Return(Value::Object(map)))
            }
            WeakMapNativeKind::Get => {
                let value = match key {
                    Some(key) => match self.find_weak_map_record(&map, key)? {
                        Some(value) => self.root_raw_value(&value)?,
                        None => Value::Undefined,
                    },
                    None => Value::Undefined,
                };
                Ok(Completion::Return(value))
            }
            WeakMapNativeKind::GetOrInsert => {
                let Some(key) = key else {
                    return self.invalid_weak_key(realm, WeakCollectionKind::Map);
                };
                if let Some(value) = self.find_weak_map_record(&map, key)? {
                    return Ok(Completion::Return(self.root_raw_value(&value)?));
                }
                let value = arguments
                    .readable
                    .get(1)
                    .cloned()
                    .ok_or(RuntimeError::Invariant("WeakMap value argv was not padded"))?;
                self.set_weak_map_record(&map, key, value.clone())?;
                Ok(Completion::Return(value))
            }
            WeakMapNativeKind::Has => {
                let present = match key {
                    Some(key) => self.find_weak_map_record(&map, key)?.is_some(),
                    None => false,
                };
                Ok(Completion::Return(Value::Bool(present)))
            }
            WeakMapNativeKind::Delete => Ok(Completion::Return(Value::Bool(match key {
                Some(key) => self.delete_weak_map_record(&map, key)?,
                None => false,
            }))),
            WeakMapNativeKind::Constructor | WeakMapNativeKind::GetOrInsertComputed => {
                unreachable!("constructor/computed handled before ordinary WeakMap dispatch")
            }
        }
    }

    pub(in crate::runtime) fn call_weak_set_native(
        &self,
        realm: ContextId,
        kind: WeakSetNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if kind == WeakSetNativeKind::Constructor {
            return self.call_weak_collection_constructor(
                realm,
                invocation,
                arguments,
                WeakCollectionKind::Set,
            );
        }
        let set = match self.weak_collection_receiver(realm, invocation, WeakCollectionKind::Set)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let key_value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant("WeakSet value argv was not padded"))?;
        let key = self.weak_collection_key(&key_value, "WeakSet key")?;
        match kind {
            WeakSetNativeKind::Add => {
                let Some(key) = key else {
                    return self.invalid_weak_key(realm, WeakCollectionKind::Set);
                };
                self.insert_weak_set_record(&set, key)?;
                Ok(Completion::Return(Value::Object(set)))
            }
            WeakSetNativeKind::Has => Ok(Completion::Return(Value::Bool(match key {
                Some(key) => self.has_weak_set_record(&set, key)?,
                None => false,
            }))),
            WeakSetNativeKind::Delete => Ok(Completion::Return(Value::Bool(match key {
                Some(key) => self.delete_weak_set_record(&set, key)?,
                None => false,
            }))),
            WeakSetNativeKind::Constructor => {
                unreachable!("constructor handled before ordinary WeakSet dispatch")
            }
        }
    }
}
