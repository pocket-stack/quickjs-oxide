//! `%SharedArrayBuffer%` constructor, grow, slice, and shared-backing bridge.
//!
//! Pinned QuickJS uses a distinct class/prototype/native family from
//! `%ArrayBuffer%`. Growable shared buffers commit their maximum capacity up
//! front, while each wrapper keeps its own current length. The safe public
//! handle bridge below preserves that intentionally off-spec wrapper-local
//! behavior without exporting runtime-local `Value` or arena identities.

use crate::heap::{
    ObjectData, ObjectPayload, SharedArrayBufferNativeKind, SharedArrayBufferRealmData,
};
use crate::shared_memory::{
    MAX_SHARED_ARRAY_BUFFER_BYTE_LENGTH, SharedBufferHandle, SharedMemoryError,
};

use super::super::*;

const MAX_SAFE_INTEGER_I64: i64 = (1_i64 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SharedArrayBufferSnapshot {
    byte_length: u32,
    max_byte_length: Option<u32>,
}

impl Runtime {
    /// Install pinned QuickJS's independent `%SharedArrayBuffer%` intrinsic.
    pub(in crate::runtime) fn initialize_shared_array_buffer_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let prototype = self.new_object(Some(object_prototype))?;

        for (kind, property_name, getter_name) in [
            (
                SharedArrayBufferNativeKind::ByteLength,
                "byteLength",
                "get byteLength",
            ),
            (
                SharedArrayBufferNativeKind::MaxByteLength,
                "maxByteLength",
                "get maxByteLength",
            ),
            (
                SharedArrayBufferNativeKind::Growable,
                "growable",
                "get growable",
            ),
        ] {
            self.define_native_builtin_getter_on(
                &prototype,
                function_prototype,
                realm,
                NativeFunctionId::SharedArrayBuffer(kind),
                property_name,
                getter_name,
            )?;
        }

        for (kind, name, length) in [
            (SharedArrayBufferNativeKind::Grow, "grow", 1),
            (SharedArrayBufferNativeKind::Slice, "slice", 2),
        ] {
            self.define_native_builtin_auto_init(
                &prototype,
                realm,
                NativeFunctionId::SharedArrayBuffer(kind),
                name,
                length,
                length,
            )?;
        }

        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            &prototype,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static(
                    "SharedArrayBuffer",
                ))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "SharedArrayBuffer toStringTag definition was rejected",
            ));
        }

        // Two readable arguments preserve QuickJS's padded argv[0]/argv[1]
        // contract while the public constructor length remains one.
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::SharedArrayBuffer(SharedArrayBufferNativeKind::Constructor),
            2,
            "SharedArrayBuffer",
            1,
        )?;
        let species_getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::SharedArrayBuffer(SharedArrayBufferNativeKind::Species),
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
                "SharedArrayBuffer species definition was rejected",
            ));
        }

        self.define_function_data_property(
            global_object,
            "SharedArrayBuffer",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, &prototype)?;
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_shared_array_buffer_intrinsics(
                realm,
                constructor.as_object().object_id(),
                SharedArrayBufferRealmData {
                    prototype: prototype.object_id(),
                },
            )?;
        Ok(())
    }

    pub(in crate::runtime) fn call_shared_array_buffer_native(
        &self,
        realm: ContextId,
        kind: SharedArrayBufferNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            SharedArrayBufferNativeKind::Constructor => {
                self.call_shared_array_buffer_constructor(realm, invocation, arguments)
            }
            SharedArrayBufferNativeKind::Species => {
                self.call_shared_array_buffer_species(invocation)
            }
            SharedArrayBufferNativeKind::ByteLength
            | SharedArrayBufferNativeKind::MaxByteLength
            | SharedArrayBufferNativeKind::Growable => {
                self.call_shared_array_buffer_getter(realm, kind, invocation)
            }
            SharedArrayBufferNativeKind::Grow => {
                self.call_shared_array_buffer_grow(realm, invocation, arguments)
            }
            SharedArrayBufferNativeKind::Slice => {
                self.call_shared_array_buffer_slice(realm, invocation, arguments)
            }
        }
    }

    fn call_shared_array_buffer_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "SharedArrayBuffer constructor did not receive a constructor invocation",
            ));
        };
        let length = match self.native_to_index(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "SharedArrayBuffer length argument was not padded",
            ))?,
        )? {
            NativeConversion::Value(length) => length,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let mut max_byte_length = None;
        if arguments.actual_arg_count >= 2 {
            if let Some(Value::Object(options)) = arguments.readable.get(1) {
                let key = self.intern_property_key("maxByteLength")?;
                let maximum = match self.get_property_in_realm(realm, options, &key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if !matches!(maximum, Value::Undefined) {
                    let maximum = match self.native_to_int64(realm, &maximum)? {
                        NativeConversion::Value(maximum) => maximum,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    // Preserve QuickJS's unsigned/signed C comparison. A
                    // negative maximum is converted to a huge u64 and reaches
                    // the post-newTarget implementation-limit check below.
                    if maximum > MAX_SAFE_INTEGER_I64 || length > maximum as u64 {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Range,
                            "invalid array buffer max length",
                        )?));
                    }
                    max_byte_length = Some(maximum as u64);
                }
            }
        }

        // `js_create_from_ctor` precedes QuickJS's implementation-limit and
        // backing allocation checks, making newTarget.prototype observable.
        let prototype =
            match self.shared_array_buffer_prototype_from_new_target(realm, new_target)? {
                NativeConversion::Value(prototype) => prototype,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };

        if length > u64::from(MAX_SHARED_ARRAY_BUFFER_BYTE_LENGTH) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array buffer length",
            )?));
        }
        if max_byte_length
            .is_some_and(|maximum| maximum > u64::from(MAX_SHARED_ARRAY_BUFFER_BYTE_LENGTH))
        {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid max array buffer length",
            )?));
        }

        let length = u32::try_from(length).map_err(|_| {
            RuntimeError::Invariant("validated SharedArrayBuffer length overflowed u32")
        })?;
        let max_byte_length = max_byte_length
            .map(u32::try_from)
            .transpose()
            .map_err(|_| {
                RuntimeError::Invariant("validated SharedArrayBuffer maximum overflowed u32")
            })?;
        let handle = match SharedBufferHandle::new(length, max_byte_length) {
            Ok(handle) => handle,
            Err(SharedMemoryError::Allocation) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
            Err(error) => return Err(shared_memory_runtime_error(error)),
        };
        let object = self.new_shared_array_buffer_from_handle(&prototype, handle)?;
        Ok(Completion::Return(Value::Object(object)))
    }

    fn call_shared_array_buffer_species(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "SharedArrayBuffer species did not receive a getter invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    fn call_shared_array_buffer_getter(
        &self,
        realm: ContextId,
        kind: SharedArrayBufferNativeKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "SharedArrayBuffer prototype getter received a non-getter invocation",
            ));
        };
        let object = match self.require_shared_array_buffer(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let snapshot = self.shared_array_buffer_snapshot(&object)?;
        let value = match kind {
            SharedArrayBufferNativeKind::ByteLength => Value::Int(
                i32::try_from(snapshot.byte_length)
                    .expect("SharedArrayBuffer length is bounded by i32::MAX"),
            ),
            SharedArrayBufferNativeKind::MaxByteLength => Value::Int(
                i32::try_from(snapshot.max_byte_length.unwrap_or(snapshot.byte_length))
                    .expect("SharedArrayBuffer maximum is bounded by i32::MAX"),
            ),
            SharedArrayBufferNativeKind::Growable => {
                Value::Bool(snapshot.max_byte_length.is_some())
            }
            SharedArrayBufferNativeKind::Constructor
            | SharedArrayBufferNativeKind::Species
            | SharedArrayBufferNativeKind::Grow
            | SharedArrayBufferNativeKind::Slice => {
                return Err(RuntimeError::Invariant(
                    "non-getter SharedArrayBuffer native reached getter dispatch",
                ));
            }
        };
        Ok(Completion::Return(value))
    }

    fn call_shared_array_buffer_grow(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "SharedArrayBuffer.prototype.grow received a constructor invocation",
            ));
        };
        // QuickJS performs the exact brand check before any observable length
        // coercion, but performs coercion before checking growability.
        let object = match self.require_shared_array_buffer(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let new_length = match self.native_to_int64(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "SharedArrayBuffer grow argument was not padded",
            ))?,
        )? {
            NativeConversion::Value(length) => length,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let current = self.shared_array_buffer_snapshot(&object)?;
        let Some(maximum) = current.max_byte_length else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "array buffer is not resizable",
            )?));
        };
        if new_length < 0
            || new_length > i64::from(maximum)
            || new_length < i64::from(current.byte_length)
        {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array buffer length",
            )?));
        }
        let new_length = u32::try_from(new_length).map_err(|_| {
            RuntimeError::Invariant("validated SharedArrayBuffer grow length overflowed u32")
        })?;
        self.0
            .state
            .borrow_mut()
            .heap
            .grow_shared_array_buffer(object.object_id(), new_length)?;
        Ok(Completion::Return(Value::Undefined))
    }

    fn call_shared_array_buffer_slice(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "SharedArrayBuffer.prototype.slice received a constructor invocation",
            ));
        };
        let source = match self.require_shared_array_buffer(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let initial = self.shared_array_buffer_snapshot(&source)?;
        let length = i64::from(initial.byte_length);
        let start = match self.native_to_int64_clamp(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "SharedArrayBuffer slice start argument was not padded",
            ))?,
            0,
            length,
            length,
        )? {
            NativeConversion::Value(start) => start,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let end = if matches!(arguments.readable.get(1), Some(Value::Undefined)) {
            length
        } else {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "SharedArrayBuffer slice end argument was not padded",
                ))?,
                0,
                length,
                length,
            )? {
                NativeConversion::Value(end) => end,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        };
        let new_length = u32::try_from((end - start).max(0)).map_err(|_| {
            RuntimeError::Invariant("validated SharedArrayBuffer slice length overflowed u32")
        })?;

        let species = match self.shared_array_buffer_species_constructor(realm, &source)? {
            NativeConversion::Value(species) => species,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target = if let Some(constructor) = species {
            match self.construct_constructor_internal(
                realm,
                &constructor,
                &constructor,
                &[Value::Int(i32::try_from(new_length).expect(
                    "SharedArrayBuffer slice length is bounded by i32::MAX",
                ))],
            )? {
                Completion::Return(Value::Object(object)) => object,
                Completion::Return(_) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "SharedArrayBuffer object expected",
                    )?));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            let prototype = self.shared_array_buffer_default_prototype(realm)?;
            let handle = match SharedBufferHandle::new(new_length, None) {
                Ok(handle) => handle,
                Err(SharedMemoryError::Allocation) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        "out of memory",
                    )?));
                }
                Err(error) => return Err(shared_memory_runtime_error(error)),
            };
            self.new_shared_array_buffer_from_handle(&prototype, handle)?
        };

        if target.object_id() == source.object_id() {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot use identical ArrayBuffer",
            )?));
        }
        let target_snapshot = match self.shared_array_buffer_snapshot_if_branded(&target)? {
            Some(snapshot) => snapshot,
            None => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "SharedArrayBuffer object expected",
                )?));
            }
        };
        if target_snapshot.byte_length < new_length {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "new ArrayBuffer is too small",
            )?));
        }

        // Species construction is re-entrant, so re-read the source wrapper's
        // current length before acquiring either backing-store lock.
        let source_snapshot = self.shared_array_buffer_snapshot(&source)?;
        let start = u32::try_from(start)
            .map_err(|_| RuntimeError::Invariant("SharedArrayBuffer slice start overflowed u32"))?;
        let Some(end) = start.checked_add(new_length) else {
            return Err(RuntimeError::Invariant(
                "SharedArrayBuffer slice range overflowed u32",
            ));
        };
        if end > source_snapshot.byte_length {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }

        let (source_handle, target_handle) = {
            let state = self.0.state.borrow();
            (
                state
                    .heap
                    .clone_shared_array_buffer_handle(source.object_id())?,
                state
                    .heap
                    .clone_shared_array_buffer_handle(target.object_id())?,
            )
        };
        target_handle
            .copy_range_from(&source_handle, start, 0, new_length)
            .map_err(shared_memory_runtime_error)?;
        Ok(Completion::Return(Value::Object(target)))
    }

    fn shared_array_buffer_species_constructor(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<Option<ConstructorRef>>, RuntimeError> {
        let constructor_key = self.intern_property_key("constructor")?;
        let constructor = match self.get_property_in_realm(realm, object, &constructor_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(constructor, Value::Undefined) {
            return Ok(NativeConversion::Value(None));
        }
        let Value::Object(constructor) = constructor else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let species_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
        let species = match self.get_property_in_realm(realm, &constructor, &species_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(species, Value::Undefined | Value::Null) {
            return Ok(NativeConversion::Value(None));
        }
        let Value::Object(_) = &species else {
            return Ok(NativeConversion::Throw(
                self.new_not_constructor_error(realm, &species)?,
            ));
        };
        self.constructor_from_value(realm, species)
            .map(|result| match result {
                NativeConversion::Value(constructor) => NativeConversion::Value(Some(constructor)),
                NativeConversion::Throw(value) => NativeConversion::Throw(value),
            })
    }

    fn shared_array_buffer_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        self.prototype_from_constructor_value(realm, &new_target, |fallback_realm| {
            self.shared_array_buffer_default_prototype(fallback_realm)
        })
    }

    fn shared_array_buffer_default_prototype(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .shared_array_buffer
            .ok_or(RuntimeError::Invariant(
                "realm has no SharedArrayBuffer intrinsics",
            ))?
            .prototype;
        Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
    }

    fn require_shared_array_buffer(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "SharedArrayBuffer object expected",
            )?));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("SharedArrayBuffer"));
        }
        if self
            .shared_array_buffer_snapshot_if_branded(&object)?
            .is_none()
        {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "SharedArrayBuffer object expected",
            )?));
        }
        Ok(NativeConversion::Value(object))
    }

    fn shared_array_buffer_snapshot_if_branded(
        &self,
        object: &ObjectRef,
    ) -> Result<Option<SharedArrayBufferSnapshot>, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("SharedArrayBuffer"));
        }
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        let ObjectPayload::SharedArrayBuffer(data) = &object.payload else {
            return Ok(None);
        };
        Ok(Some(SharedArrayBufferSnapshot {
            byte_length: data.handle.byte_length(),
            max_byte_length: data.handle.max_byte_length_option(),
        }))
    }

    fn shared_array_buffer_snapshot(
        &self,
        object: &ObjectRef,
    ) -> Result<SharedArrayBufferSnapshot, RuntimeError> {
        self.shared_array_buffer_snapshot_if_branded(object)?
            .ok_or(RuntimeError::Invariant(
                "validated SharedArrayBuffer lost its class payload",
            ))
    }

    pub(in crate::runtime) fn shared_array_buffer_handle_if_branded(
        &self,
        object: &ObjectRef,
    ) -> Result<Option<SharedBufferHandle>, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("SharedArrayBuffer"));
        }
        let state = self.0.state.borrow();
        let ObjectPayload::SharedArrayBuffer(_) = &state.heap.object(object.object_id())?.payload
        else {
            return Ok(None);
        };
        Ok(Some(
            state
                .heap
                .clone_shared_array_buffer_handle(object.object_id())?,
        ))
    }

    fn new_shared_array_buffer_from_handle(
        &self,
        prototype: &ObjectRef,
        handle: SharedBufferHandle,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("SharedArrayBuffer prototype"));
        }
        let maximum = handle.max_byte_length_option();
        if handle.byte_length() > MAX_SHARED_ARRAY_BUFFER_BYTE_LENGTH
            || maximum.is_some_and(|maximum| {
                maximum < handle.byte_length()
                    || maximum > MAX_SHARED_ARRAY_BUFFER_BYTE_LENGTH
                    || maximum != handle.backing_capacity()
            })
            || maximum.is_none() && handle.backing_capacity() != handle.byte_length()
        {
            return Err(RuntimeError::Invariant(
                "SharedArrayBuffer handle has invalid wrapper or backing state",
            ));
        }

        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state.heap.allocate_object(ObjectData::shared_array_buffer(
            shape,
            Vec::new(),
            handle,
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
}

impl Context {
    /// Clone a safe, sendable handle from one genuine SharedArrayBuffer.
    ///
    /// The returned handle contains no `Value`, `ObjectId`, realm, or heap
    /// edge. Its bytes remain shared, while its current length is a snapshot
    /// local to this exported wrapper handle, matching pinned QuickJS.
    pub fn export_shared_array_buffer(
        &self,
        object: &ObjectRef,
    ) -> Result<SharedBufferHandle, RuntimeError> {
        self.runtime
            .shared_array_buffer_handle_if_branded(object)?
            .ok_or_else(|| {
                RuntimeError::Engine(Error::new(
                    ErrorKind::Type,
                    "SharedArrayBuffer object expected",
                ))
            })
    }

    /// Import a safe shared-backing handle as a new wrapper in this realm.
    ///
    /// Import never reuses a foreign heap identity. The new object receives
    /// this realm's `%SharedArrayBuffer.prototype%`, shares only backing bytes,
    /// and owns an independent copy of the handle's visible length metadata.
    pub fn import_shared_array_buffer(
        &mut self,
        handle: SharedBufferHandle,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self
            .runtime
            .shared_array_buffer_default_prototype(self.realm)?;
        self.runtime
            .new_shared_array_buffer_from_handle(&prototype, handle)
    }
}

fn shared_memory_runtime_error(error: SharedMemoryError) -> RuntimeError {
    RuntimeError::Invariant(match error {
        SharedMemoryError::InvalidLength => "SharedArrayBuffer has an invalid length",
        SharedMemoryError::Allocation => "SharedArrayBuffer backing allocation failed",
        SharedMemoryError::NotGrowable => "SharedArrayBuffer wrapper is not growable",
        SharedMemoryError::CannotShrink => "SharedArrayBuffer wrapper cannot shrink",
        SharedMemoryError::RangeOverflow => "SharedArrayBuffer range overflowed",
        SharedMemoryError::OutOfBounds => "SharedArrayBuffer range is out of bounds",
        SharedMemoryError::InvalidWordLength => {
            "SharedArrayBuffer word has an unsupported byte length"
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_descriptors_and_key_order_match_pinned_quickjs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
                    var globalDesc=Object.getOwnPropertyDescriptor(globalThis,"SharedArrayBuffer");
                    var ctorProto=Object.getOwnPropertyDescriptor(SharedArrayBuffer,"prototype");
                    var species=Object.getOwnPropertyDescriptor(SharedArrayBuffer,Symbol.species);
                    var tag=Object.getOwnPropertyDescriptor(SharedArrayBuffer.prototype,Symbol.toStringTag);
                    [
                      Reflect.ownKeys(SharedArrayBuffer).map(String).join(","),
                      Reflect.ownKeys(SharedArrayBuffer.prototype).map(String).join(","),
                      SharedArrayBuffer.length,SharedArrayBuffer.name,
                      globalDesc.writable,globalDesc.enumerable,globalDesc.configurable,
                      ctorProto.writable,ctorProto.enumerable,ctorProto.configurable,
                      species.get.length,species.get.name,species.set,species.enumerable,species.configurable,
                      tag.value,tag.writable,tag.enumerable,tag.configurable,
                      Object.getOwnPropertyNames(globalThis).filter(function(x){
                        return x==="ArrayBuffer"||x==="SharedArrayBuffer"||x==="Uint8ClampedArray";
                      }).join(",")
                    ].join("|")
                    "#,
                )
                .unwrap(),
            Value::String(JsString::from_static(
                "length,name,prototype,Symbol(Symbol.species)|byteLength,maxByteLength,growable,grow,slice,constructor,Symbol(Symbol.toStringTag)|1|SharedArrayBuffer|true|false|true|false|false|false|0|get [Symbol.species]||false|true|SharedArrayBuffer|false|false|true|ArrayBuffer,SharedArrayBuffer,Uint8ClampedArray",
            )),
        );
    }

    #[test]
    fn constructor_and_grow_preserve_quickjs_observable_order_and_errors() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
                    function errorName(callback){try{callback();return "none"}catch(error){return error.name}}
                    var log=[];
                    var target=function(){}.bind(null);
                    Object.defineProperty(target,"prototype",{get:function(){log.push("prototype");return null}});
                    var tooSmall={get maxByteLength(){log.push("maximum");return 0}};
                    var compare=errorName(function(){Reflect.construct(SharedArrayBuffer,[1,tooSmall],target)});
                    var compareLog=log.join(",");
                    log=[];
                    var limit=errorName(function(){Reflect.construct(SharedArrayBuffer,[2147483648],target)});
                    var limitLog=log.join(",");
                    var touched=0;
                    var coercible={valueOf:function(){touched++;return 2}};
                    var callError=errorName(function(){SharedArrayBuffer(coercible)});
                    var callTouched=touched;
                    touched=0;
                    var wrongBrand=errorName(function(){SharedArrayBuffer.prototype.grow.call(new ArrayBuffer(1),coercible)});
                    var wrongBrandTouched=touched;
                    touched=0;
                    var fixed=errorName(function(){new SharedArrayBuffer(1).grow(coercible)});
                    var fixedTouched=touched;
                    var shared=new SharedArrayBuffer(2,{maxByteLength:6});
                    var same=shared.grow(2);
                    shared.grow(5);
                    [compare,compareLog,limit,limitLog,callError,callTouched,
                     wrongBrand,wrongBrandTouched,fixed,fixedTouched,
                     same,shared.byteLength,shared.maxByteLength,shared.growable,
                     errorName(function(){shared.grow(4)}),
                     errorName(function(){shared.grow(7)}),
                     Object.prototype.toString.call(shared)].join("|")
                    "#,
                )
                .unwrap(),
            Value::String(JsString::from_static(
                "RangeError|maximum|RangeError|prototype|TypeError|0|TypeError|0|TypeError|1||5|6|true|RangeError|RangeError|[object SharedArrayBuffer]",
            )),
        );
    }

    #[test]
    fn slice_species_branding_and_detach_isolation_match_quickjs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source = eval_object(
            &mut context,
            "globalThis.__shared=new SharedArrayBuffer(4,{maxByteLength:8});__shared",
        );
        let source_handle = context.export_shared_array_buffer(&source).unwrap();
        source_handle.write_range(0, &[1, 2, 3, 4]).unwrap();

        let sliced = eval_object(&mut context, "__shared.slice(1,3)");
        let sliced_handle = context.export_shared_array_buffer(&sliced).unwrap();
        assert_eq!(sliced_handle.read_range(0, 2).unwrap(), [2, 3]);
        assert_eq!(sliced_handle.max_byte_length_option(), None);
        assert!(!sliced_handle.shares_backing_with(&source_handle));

        assert_eq!(
            context
                .eval(
                    r#"
                    function errorName(callback){try{callback();return "none"}catch(error){return error.name}}
                    var source=new SharedArrayBuffer(4);
                    var larger=new SharedArrayBuffer(8);
                    source.constructor={};
                    source.constructor[Symbol.species]=function(){return larger};
                    var accepted=source.slice(0,2)===larger;
                    source.constructor[Symbol.species]=function(){return new ArrayBuffer(4)};
                    var arrayBuffer=errorName(function(){source.slice(0,2)});
                    source.constructor[Symbol.species]=function(){return source};
                    var same=errorName(function(){source.slice(0,2)});
                    source.constructor[Symbol.species]=function(){return new SharedArrayBuffer(1)};
                    var small=errorName(function(){source.slice(0,2)});
                    var sabOnAb=errorName(function(){SharedArrayBuffer.prototype.slice.call(new ArrayBuffer(4),0)});
                    var abOnSab=errorName(function(){ArrayBuffer.prototype.slice.call(source,0)});
                    [accepted,arrayBuffer,same,small,sabOnAb,abOnSab].join("|")
                    "#,
                )
                .unwrap(),
            Value::String(JsString::from_static(
                "true|TypeError|TypeError|TypeError|TypeError|TypeError",
            )),
        );

        context
            .detach_array_buffer(&Value::Object(source.clone()))
            .unwrap();
        assert_eq!(
            context.eval("[__shared.byteLength,__shared.growable].join('|')"),
            Ok(Value::String(JsString::from_static("4|true"))),
        );
    }

    #[test]
    fn safe_handles_share_bytes_across_runtimes_but_not_wrapper_lengths_and_survive_gc() {
        let first_runtime = Runtime::new();
        let mut first_context = first_runtime.new_context();
        let source = eval_object(
            &mut first_context,
            "new SharedArrayBuffer(2,{maxByteLength:8})",
        );
        let exported = first_context.export_shared_array_buffer(&source).unwrap();
        exported.write_range(0, &[11, 22]).unwrap();
        drop(source);
        first_runtime.run_gc().unwrap();

        let first_realm_prototype = eval_object(&mut first_context, "SharedArrayBuffer.prototype");
        let mut sibling_context = first_runtime.new_context();
        let sibling_imported = sibling_context
            .import_shared_array_buffer(exported.clone())
            .unwrap();
        let sibling_realm_prototype =
            eval_object(&mut sibling_context, "SharedArrayBuffer.prototype");
        assert_ne!(first_realm_prototype, sibling_realm_prototype);
        assert_eq!(
            first_runtime.get_prototype_of(&sibling_imported).unwrap(),
            Some(sibling_realm_prototype)
        );

        let second_runtime = Runtime::new();
        let mut second_context = second_runtime.new_context();
        let imported = second_context
            .import_shared_array_buffer(exported.clone())
            .unwrap();
        let second_realm_prototype =
            eval_object(&mut second_context, "SharedArrayBuffer.prototype");
        assert_eq!(
            second_runtime.get_prototype_of(&imported).unwrap(),
            Some(second_realm_prototype)
        );
        let imported_initial = second_context
            .export_shared_array_buffer(&imported)
            .unwrap();
        assert!(exported.shares_backing_with(&imported_initial));
        assert_eq!(imported_initial.read_range(0, 2).unwrap(), [11, 22]);

        let global = second_context.global_object().unwrap();
        let key = second_runtime.intern_property_key("__imported").unwrap();
        assert!(
            second_context
                .define_own_property(
                    &global,
                    &key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Object(imported.clone())),
                        writable: DescriptorField::Present(true),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        second_context.eval("__imported.grow(4)").unwrap();
        let imported_grown = second_context
            .export_shared_array_buffer(&imported)
            .unwrap();
        assert_eq!(exported.byte_length(), 2);
        assert_eq!(imported_initial.byte_length(), 2);
        assert_eq!(imported_grown.byte_length(), 4);
        assert_eq!(imported_grown.read_range(0, 4).unwrap(), [11, 22, 0, 0]);
        imported_grown.write_range(0, &[33, 44]).unwrap();
        assert_eq!(exported.read_range(0, 2).unwrap(), [33, 44]);

        second_context.eval("delete globalThis.__imported").unwrap();
        drop(imported);
        second_runtime.run_gc().unwrap();
        assert_eq!(imported_grown.read_range(0, 4).unwrap(), [33, 44, 0, 0]);

        let reimported = first_context
            .import_shared_array_buffer(imported_grown.clone())
            .unwrap();
        let reexported = first_context
            .export_shared_array_buffer(&reimported)
            .unwrap();
        assert_eq!(reexported.byte_length(), 4);
        assert_eq!(reexported.read_range(0, 4).unwrap(), [33, 44, 0, 0]);
    }

    fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
        let Value::Object(object) = context.eval(source).unwrap() else {
            panic!("SharedArrayBuffer test source did not return an object");
        };
        object
    }
}
