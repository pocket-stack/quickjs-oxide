use super::*;

impl Runtime {
    /// QuickJS `js_for_in_start`: box non-nullish primitives, snapshot the
    /// base object's own string keys, and return a hidden enumeration object.
    pub(crate) fn start_for_in(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<ObjectRef, RuntimeError> {
        let object = self.for_in_object(realm, value)?;
        let fast_array_count = object
            .as_ref()
            .map(|object| self.for_in_fast_array_count(object))
            .transpose()?
            .flatten();
        let properties = if fast_array_count.is_some() {
            Vec::new()
        } else {
            object
                .as_ref()
                .map(|object| self.snapshot_for_in_properties(object))
                .transpose()?
                .unwrap_or_default()
        };
        let data = ForInIteratorData {
            object: object.as_ref().map(ObjectRef::object_id),
            index: 0,
            properties,
            fast_array: fast_array_count.is_some(),
            array_count: fast_array_count.unwrap_or(0),
            in_prototype_chain: false,
            visited: std::collections::HashSet::new(),
        };

        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(None, &[])?;
        let iterator =
            match state
                .heap
                .allocate_object(ObjectData::for_in_iterator(shape, Vec::new(), data))
            {
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

    /// QuickJS `js_for_in_next`: retain the enumeration object, return one
    /// string key plus `done`, and snapshot prototypes only when reached.
    pub(crate) fn next_for_in(&self, iterator: &ObjectRef) -> Result<(Value, bool), RuntimeError> {
        if !iterator.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("for-in iterator"));
        }
        loop {
            let candidate = self
                .0
                .state
                .borrow_mut()
                .heap
                .next_for_in_candidate(iterator.object_id())?;
            match candidate {
                ForInCandidate::Done => return Ok((Value::Undefined, true)),
                ForInCandidate::BaseComplete { object, fast_array } => {
                    let current = ObjectRef::from_borrowed_handle(self.clone(), object)?;
                    if !self.for_in_prototype_chain_has_enumerable_property(&current)? {
                        self.store_for_in_level(iterator, None, Vec::new())?;
                        return Ok((Value::Undefined, true));
                    }
                    let refreshed_fast_properties = fast_array
                        .then(|| self.snapshot_for_in_properties(&current))
                        .transpose()?;
                    self.0
                        .state
                        .borrow_mut()
                        .heap
                        .enter_for_in_prototype_chain(
                            iterator.object_id(),
                            refreshed_fast_properties,
                        )?;
                    let prototype = self.get_prototype_of(&current)?;
                    let properties = prototype
                        .as_ref()
                        .map(|prototype| self.snapshot_for_in_properties(prototype))
                        .transpose()?
                        .unwrap_or_default();
                    let next_object = prototype.as_ref().map(ObjectRef::object_id);
                    self.store_for_in_level(iterator, next_object, properties)?;
                    if prototype.is_none() {
                        return Ok((Value::Undefined, true));
                    }
                }
                ForInCandidate::LevelComplete(current_id) => {
                    let current = ObjectRef::from_borrowed_handle(self.clone(), current_id)?;
                    let prototype = self.get_prototype_of(&current)?;
                    let properties = prototype
                        .as_ref()
                        .map(|prototype| self.snapshot_for_in_properties(prototype))
                        .transpose()?
                        .unwrap_or_default();
                    let next_object = prototype.as_ref().map(ObjectRef::object_id);
                    self.store_for_in_level(iterator, next_object, properties)?;
                    if prototype.is_none() {
                        return Ok((Value::Undefined, true));
                    }
                }
                ForInCandidate::ArrayIndex { object, index } => {
                    let current = ObjectRef::from_borrowed_handle(self.clone(), object)?;
                    let name = JsString::try_from_utf8(&index.to_string())?;
                    let key = self.intern_property_key_js_string(&name)?;
                    if self.has_own_property(&current, &key)? {
                        return Ok((Value::String(name), false));
                    }
                }
                ForInCandidate::Property { object, name } => {
                    let current = ObjectRef::from_borrowed_handle(self.clone(), object)?;
                    let key = self.intern_property_key_js_string(&name)?;
                    if self.has_own_property(&current, &key)? {
                        return Ok((Value::String(name), false));
                    }
                }
            }
        }
    }

    /// Mirror the representation-sensitive branch in
    /// `build_for_in_iterator`: a QuickJS fast Array or Arguments object stays
    /// count-only only when its ordinary shape has no other enumerable field.
    /// Rust stores dense elements in that shape too, so the tracked prefix is
    /// excluded from this check.
    fn for_in_fast_array_count(&self, object: &ObjectRef) -> Result<Option<u32>, RuntimeError> {
        let state = self.0.state.borrow();
        let object_data = state.heap.object(object.object_id())?;
        let fast_len = match &object_data.payload {
            ObjectPayload::Array {
                fast_len: Some(fast_len),
            }
            | ObjectPayload::Arguments {
                fast_len: Some(fast_len),
                ..
            } => *fast_len,
            ObjectPayload::Ordinary
            | ObjectPayload::RawJson
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::Array { fast_len: None }
            | ObjectPayload::Arguments { fast_len: None, .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::Map { .. }
            | ObjectPayload::MapIterator { .. }
            | ObjectPayload::Set { .. }
            | ObjectPayload::SetIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. } => return Ok(None),
        };
        let shape = state.heap.shape(object_data.shape)?;
        for entry in shape.entries() {
            if !entry.flags.enumerable {
                continue;
            }
            if state
                .atoms
                .array_index(entry.atom)?
                .is_some_and(|index| index < fast_len)
            {
                continue;
            }
            return Ok(None);
        }
        Ok(Some(fast_len))
    }

    /// QuickJS pre-scans the complete live prototype chain before it creates
    /// the hidden visited set. The actual traversal starts from the base again,
    /// so future Proxy support must preserve both prototype lookup passes.
    fn for_in_prototype_chain_has_enumerable_property(
        &self,
        object: &ObjectRef,
    ) -> Result<bool, RuntimeError> {
        let mut current = object.clone();
        while let Some(prototype) = self.get_prototype_of(&current)? {
            for key in self.own_property_keys(&prototype)? {
                if self.0.state.borrow().atoms.property_key_kind(key.atom())?
                    == PropertyKeyKind::String
                    && self.own_property_is_enumerable(&prototype, &key)?
                {
                    return Ok(true);
                }
            }
            current = prototype;
        }
        Ok(false)
    }

    fn for_in_object(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<Option<ObjectRef>, RuntimeError> {
        let kind = match &value {
            Value::Undefined | Value::Null => return Ok(None),
            Value::Object(object) => {
                if !object.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("for-in source"));
                }
                return Ok(Some(object.clone()));
            }
            Value::Bool(_) => PrimitiveKind::Boolean,
            Value::Int(_) | Value::Float(_) => PrimitiveKind::Number,
            Value::String(_) => PrimitiveKind::String,
            Value::Symbol(_) => PrimitiveKind::Symbol,
            Value::BigInt(_) => PrimitiveKind::BigInt,
        };
        let prototype = self.primitive_prototype_for_realm(realm, kind)?;
        self.new_primitive_object(&prototype, kind, value).map(Some)
    }

    fn snapshot_for_in_properties(
        &self,
        object: &ObjectRef,
    ) -> Result<Vec<ForInProperty>, RuntimeError> {
        let mut properties = Vec::new();
        for key in self.own_property_keys(object)? {
            let kind = self.0.state.borrow().atoms.property_key_kind(key.atom())?;
            if kind != PropertyKeyKind::String {
                continue;
            }
            properties.push(ForInProperty {
                name: self.property_key_to_js_string(&key)?,
                enumerable: self.own_property_is_enumerable(object, &key)?,
            });
        }
        Ok(properties)
    }

    fn store_for_in_level(
        &self,
        iterator: &ObjectRef,
        next_object: Option<ObjectId>,
        properties: Vec<ForInProperty>,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let cleanup =
            state
                .heap
                .replace_for_in_level(iterator.object_id(), next_object, properties)?;
        state.apply_cleanup(cleanup)
    }
}
