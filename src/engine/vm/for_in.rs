pub(super) mod operation;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::builtins::native::PrimitiveKind;

use crate::engine::heap::{
    ContextId, ForInIteratorData, ForInProperty, ObjectData, ObjectId, ObjectPayload,
};
use crate::engine::object::ObjectRef;

use crate::engine::value::Value;

impl Runtime {
    fn allocate_for_in_iterator(
        &self,
        object: Option<&ObjectRef>,
        fast_array_count: Option<u32>,
        properties: Vec<ForInProperty>,
    ) -> Result<ObjectRef, RuntimeError> {
        let data = ForInIteratorData {
            object: object.map(ObjectRef::object_id),
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

    /// Mirror the representation-sensitive branch in
    /// `build_for_in_iterator`: a QuickJS fast Array, Arguments, or TypedArray
    /// stays count-only only when its ordinary shape has no other enumerable
    /// field. Array and TypedArray integer indices are virtual to the ordinary
    /// shape; Arguments keeps its mapped/unmapped indexed slots there, so only
    /// that tracked prefix is excluded from the named-field check.
    fn for_in_fast_array_count(&self, object: &ObjectRef) -> Result<Option<u32>, RuntimeError> {
        let (dense_length, typed_array, shape_id) = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            let (dense_length, typed_array) = match &object_data.payload {
                ObjectPayload::Array { dense: Some(dense) } => (
                    Some(u32::try_from(dense.len()).map_err(|_| {
                        RuntimeError::Invariant("fast Array count exceeded Uint32")
                    })?),
                    None,
                ),
                ObjectPayload::Arguments {
                    fast_len: Some(fast_len),
                    ..
                } => (Some(*fast_len), None),
                ObjectPayload::TypedArray(data) => (None, Some(*data)),
                ObjectPayload::Ordinary
                | ObjectPayload::Proxy(_)
                | ObjectPayload::RawJson
                | ObjectPayload::Promise(_)
                | ObjectPayload::Date(_)
                | ObjectPayload::RegExp(_)
                | ObjectPayload::ArrayBuffer(_)
                | ObjectPayload::SharedArrayBuffer(_)
                | ObjectPayload::DataView(_)
                | ObjectPayload::Array { dense: None }
                | ObjectPayload::Arguments { fast_len: None, .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::IteratorHelper(_)
                | ObjectPayload::IteratorWrap(_)
                | ObjectPayload::AsyncFromSyncIterator(_)
                | ObjectPayload::IteratorConcat(_)
                | ObjectPayload::Map { .. }
                | ObjectPayload::MapIterator { .. }
                | ObjectPayload::Set { .. }
                | ObjectPayload::WeakMap { .. }
                | ObjectPayload::WeakSet { .. }
                | ObjectPayload::WeakRef { .. }
                | ObjectPayload::FinalizationRegistry(_)
                | ObjectPayload::SetIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. }
                | ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
                | ObjectPayload::AsyncFunctionState(_)
                | ObjectPayload::Generator { .. }
                | ObjectPayload::AsyncGenerator(_) => return Ok(None),
            };
            (dense_length, typed_array, object_data.shape)
        };

        let fast_len = if let Some(data) = typed_array {
            let buffer = self.snapshot_buffer_access(data.view.buffer)?.state;
            let byte_length = if buffer.detached || data.view.byte_offset > buffer.byte_length {
                0
            } else {
                match data.view.fixed_byte_length {
                    Some(length)
                        if data
                            .view
                            .byte_offset
                            .checked_add(length)
                            .is_none_or(|end| end > buffer.byte_length) =>
                    {
                        0
                    }
                    Some(length) => length,
                    None => buffer.byte_length - data.view.byte_offset,
                }
            };
            byte_length / u32::from(data.element.byte_length())
        } else {
            dense_length.expect("fast dense payload recorded its length")
        };

        let state = self.0.state.borrow();
        let shape = state.heap.shape(shape_id)?;
        for entry in shape.entries() {
            if !entry.flags.enumerable {
                continue;
            }
            if state
                .atoms
                .array_index(state.atoms.brand(entry.atom)?)?
                .is_some_and(|index| index < fast_len)
            {
                continue;
            }
            return Ok(None);
        }
        Ok(Some(fast_len))
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
