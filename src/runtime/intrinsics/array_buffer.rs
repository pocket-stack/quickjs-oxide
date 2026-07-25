//! `%ArrayBuffer%` backing-store, constructor, resize, detach, and transfer.
//!
//! This mirrors the pinned QuickJS `JSArrayBuffer` boundary rather than
//! representing bytes as ordinary indexed properties. TypedArray and DataView
//! objects can therefore retain this branded object as their single backing
//! store in later milestones without changing ArrayBuffer identity or detach
//! semantics.

use crate::heap::{
    ArrayBufferData, ArrayBufferNativeKind, ArrayBufferRealmData, ObjectData, ObjectPayload,
};

use super::super::*;
use super::quickjs_to_int64_free;

#[cfg(test)]
mod tests;

const MAX_ARRAY_BUFFER_LENGTH: u64 = i32::MAX as u64;
const MAX_SAFE_INTEGER_I64: i64 = (1_i64 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ArrayBufferSnapshot {
    byte_length: u32,
    max_byte_length: Option<u32>,
    detached: bool,
}

impl Runtime {
    /// Install the complete pinned `%ArrayBuffer%` surface.
    ///
    /// QuickJS installs TypedArrays after Map/Set and before Promise. Keeping
    /// ArrayBuffer at that same bootstrap boundary preserves global own-key
    /// order while leaving TypedArray/DataView as explicit follow-up classes.
    pub(in crate::runtime) fn initialize_array_buffer_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let prototype = self.new_object(Some(object_prototype))?;

        for (kind, property_name, getter_name) in [
            (
                ArrayBufferNativeKind::ByteLength,
                "byteLength",
                "get byteLength",
            ),
            (
                ArrayBufferNativeKind::MaxByteLength,
                "maxByteLength",
                "get maxByteLength",
            ),
            (
                ArrayBufferNativeKind::Resizable,
                "resizable",
                "get resizable",
            ),
            (ArrayBufferNativeKind::Detached, "detached", "get detached"),
        ] {
            self.define_native_builtin_getter_on(
                &prototype,
                function_prototype,
                realm,
                NativeFunctionId::ArrayBuffer(kind),
                property_name,
                getter_name,
            )?;
        }

        for (kind, name, length, readable) in [
            (ArrayBufferNativeKind::Resize, "resize", 1, 1),
            (ArrayBufferNativeKind::Slice, "slice", 2, 2),
            (ArrayBufferNativeKind::Transfer, "transfer", 0, 1),
            (
                ArrayBufferNativeKind::TransferToFixedLength,
                "transferToFixedLength",
                0,
                1,
            ),
        ] {
            self.define_native_builtin_auto_init(
                &prototype,
                realm,
                NativeFunctionId::ArrayBuffer(kind),
                name,
                length,
                readable,
            )?;
        }

        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            &prototype,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static(
                    "ArrayBuffer",
                ))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer toStringTag definition was rejected",
            ));
        }

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ArrayBuffer(ArrayBufferNativeKind::Constructor),
            2,
            "ArrayBuffer",
            1,
        )?;
        self.define_native_builtin_auto_init(
            constructor.as_object(),
            realm,
            NativeFunctionId::ArrayBuffer(ArrayBufferNativeKind::IsView),
            "isView",
            1,
            1,
        )?;
        let species_getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ArrayBuffer(ArrayBufferNativeKind::Species),
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
                "ArrayBuffer species definition was rejected",
            ));
        }

        self.define_function_data_property(
            global_object,
            "ArrayBuffer",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, &prototype)?;
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_array_buffer_intrinsics(
                realm,
                constructor.as_object().object_id(),
                ArrayBufferRealmData {
                    prototype: prototype.object_id(),
                },
            )?;
        Ok(())
    }

    pub(in crate::runtime) fn call_array_buffer_native(
        &self,
        realm: ContextId,
        kind: ArrayBufferNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            ArrayBufferNativeKind::Constructor => {
                self.call_array_buffer_constructor(realm, invocation, arguments)
            }
            ArrayBufferNativeKind::IsView => self.call_array_buffer_is_view(invocation, arguments),
            ArrayBufferNativeKind::Species => self.call_array_buffer_species(invocation),
            ArrayBufferNativeKind::ByteLength
            | ArrayBufferNativeKind::MaxByteLength
            | ArrayBufferNativeKind::Resizable
            | ArrayBufferNativeKind::Detached => {
                self.call_array_buffer_getter(realm, kind, invocation)
            }
            ArrayBufferNativeKind::Resize => {
                self.call_array_buffer_resize(realm, invocation, arguments)
            }
            ArrayBufferNativeKind::Slice => {
                self.call_array_buffer_slice(realm, invocation, arguments)
            }
            ArrayBufferNativeKind::Transfer => {
                self.call_array_buffer_transfer(realm, invocation, arguments, false)
            }
            ArrayBufferNativeKind::TransferToFixedLength => {
                self.call_array_buffer_transfer(realm, invocation, arguments, true)
            }
        }
    }

    /// Pinned QuickJS `JS_ToInt64`: number-hint coercion followed by its
    /// representation-level modulo-2^64 conversion.
    fn native_to_int64(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<i64>, RuntimeError> {
        let number = match self.native_to_number(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        Ok(NativeConversion::Value(quickjs_to_int64_free(number)))
    }

    fn call_array_buffer_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer constructor did not receive a constructor invocation",
            ));
        };
        let length = match self.native_to_index(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "ArrayBuffer length argument was not padded",
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
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    };
                    // Pinned QuickJS compares the unsigned `len` with this
                    // signed result in C. Negative maxima therefore survive
                    // until the post-newTarget 2 GiB implementation limit.
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

        // Pinned QuickJS performs the observable newTarget.prototype lookup
        // before rejecting an otherwise unallocatable backing-store length.
        let prototype = match self.array_buffer_prototype_from_new_target(realm, new_target)? {
            NativeConversion::Value(prototype) => prototype,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        if length > MAX_ARRAY_BUFFER_LENGTH {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array buffer length",
            )?));
        }
        if max_byte_length.is_some_and(|maximum| maximum > MAX_ARRAY_BUFFER_LENGTH) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid max array buffer length",
            )?));
        }

        let length = u32::try_from(length)
            .map_err(|_| RuntimeError::Invariant("validated ArrayBuffer length overflowed u32"))?;
        let max_byte_length = max_byte_length
            .map(u32::try_from)
            .transpose()
            .map_err(|_| RuntimeError::Invariant("validated ArrayBuffer maximum overflowed u32"))?;
        let Some(object) = self.new_array_buffer_object(&prototype, length, max_byte_length)?
        else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "out of memory",
            )?));
        };
        Ok(Completion::Return(Value::Object(object)))
    }

    fn call_array_buffer_is_view(
        &self,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer.isView received a constructor invocation",
            ));
        };
        let Some(_value) = arguments.readable.first() else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer.isView argument was not padded",
            ));
        };
        // ArrayBuffer itself is not a view. Integer-indexed TypedArray and
        // DataView kinds will extend this exact class-brand predicate.
        Ok(Completion::Return(Value::Bool(false)))
    }

    fn call_array_buffer_species(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer species did not receive a getter invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    fn call_array_buffer_getter(
        &self,
        realm: ContextId,
        kind: ArrayBufferNativeKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer prototype getter received a non-getter invocation",
            ));
        };
        let object = match self.require_array_buffer(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let snapshot = self.array_buffer_snapshot(&object)?;
        let value = match kind {
            ArrayBufferNativeKind::ByteLength => Value::Int(
                i32::try_from(snapshot.byte_length)
                    .expect("ArrayBuffer length is bounded by i32::MAX"),
            ),
            ArrayBufferNativeKind::MaxByteLength => Value::Int(
                i32::try_from(snapshot.max_byte_length.unwrap_or(snapshot.byte_length))
                    .expect("ArrayBuffer maximum is bounded by i32::MAX"),
            ),
            ArrayBufferNativeKind::Resizable => Value::Bool(snapshot.max_byte_length.is_some()),
            ArrayBufferNativeKind::Detached => Value::Bool(snapshot.detached),
            ArrayBufferNativeKind::Constructor
            | ArrayBufferNativeKind::IsView
            | ArrayBufferNativeKind::Species
            | ArrayBufferNativeKind::Resize
            | ArrayBufferNativeKind::Slice
            | ArrayBufferNativeKind::Transfer
            | ArrayBufferNativeKind::TransferToFixedLength => {
                return Err(RuntimeError::Invariant(
                    "non-getter ArrayBuffer native reached getter dispatch",
                ));
            }
        };
        Ok(Completion::Return(value))
    }

    fn call_array_buffer_resize(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer.prototype.resize received a constructor invocation",
            ));
        };
        let object = match self.require_array_buffer(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let new_length = match self.native_to_int64(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "ArrayBuffer resize argument was not padded",
            ))?,
        )? {
            NativeConversion::Value(length) => length,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let current = self.array_buffer_snapshot(&object)?;
        if current.detached {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        let Some(maximum) = current.max_byte_length else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "array buffer is not resizable",
            )?));
        };
        if new_length < 0 || new_length > i64::from(maximum) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array buffer length",
            )?));
        }
        let new_length = usize::try_from(new_length)
            .map_err(|_| RuntimeError::Invariant("validated resize length overflowed usize"))?;
        let resized = self
            .0
            .state
            .borrow_mut()
            .heap
            .resize_array_buffer_bytes(object.object_id(), new_length)?;
        if !resized {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "out of memory",
            )?));
        }
        Ok(Completion::Return(Value::Undefined))
    }

    fn call_array_buffer_slice(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer.prototype.slice received a constructor invocation",
            ));
        };
        let source = match self.require_array_buffer(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let initial = self.array_buffer_snapshot(&source)?;
        if initial.detached {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        let length = i64::from(initial.byte_length);
        let start = match self.native_to_int64_clamp(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "ArrayBuffer slice start argument was not padded",
            ))?,
            0,
            length,
            length,
        )? {
            NativeConversion::Value(start) => start,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let end = if arguments.actual_arg_count < 2
            || matches!(arguments.readable.get(1), Some(Value::Undefined))
        {
            length
        } else {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "ArrayBuffer slice end argument was not padded",
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
            RuntimeError::Invariant("validated ArrayBuffer slice length overflowed u32")
        })?;

        let species = match self.array_buffer_species_constructor(realm, &source)? {
            NativeConversion::Value(species) => species,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target = if let Some(constructor) = species {
            match self.construct_internal(
                realm,
                &constructor,
                &constructor,
                &[Value::Int(i32::try_from(new_length).expect(
                    "ArrayBuffer slice length is bounded by i32::MAX",
                ))],
            )? {
                Completion::Return(Value::Object(object)) => object,
                Completion::Return(_) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "ArrayBuffer object expected",
                    )?));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            let prototype = self.array_buffer_default_prototype(realm)?;
            let Some(object) = self.new_array_buffer_object(&prototype, new_length, None)? else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            };
            object
        };

        if target.object_id() == source.object_id() {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot use identical ArrayBuffer",
            )?));
        }
        let target_snapshot = match self.array_buffer_snapshot_if_branded(&target)? {
            Some(snapshot) => snapshot,
            None => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "ArrayBuffer object expected",
                )?));
            }
        };
        if target_snapshot.detached {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        if target_snapshot.byte_length < new_length {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "new ArrayBuffer is too small",
            )?));
        }

        let start = usize::try_from(start)
            .map_err(|_| RuntimeError::Invariant("slice start overflowed usize"))?;
        let new_length_usize = usize::try_from(new_length)
            .map_err(|_| RuntimeError::Invariant("slice length overflowed usize"))?;
        let source_is_live = {
            let state = self.0.state.borrow();
            let ObjectPayload::ArrayBuffer(data) = &state.heap.object(source.object_id())?.payload
            else {
                return Err(RuntimeError::Invariant(
                    "validated ArrayBuffer lost its class payload",
                ));
            };
            let Some(end) = start.checked_add(new_length_usize) else {
                return Err(RuntimeError::Invariant(
                    "ArrayBuffer slice range overflowed usize",
                ));
            };
            !data.detached && end <= data.bytes.len()
        };
        if !source_is_live {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        self.0.state.borrow_mut().heap.copy_array_buffer_range(
            source.object_id(),
            target.object_id(),
            start,
            new_length_usize,
        )?;
        Ok(Completion::Return(Value::Object(target)))
    }

    fn call_array_buffer_transfer(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        to_fixed_length: bool,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer transfer received a constructor invocation",
            ));
        };
        let source = match self.require_array_buffer(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let initial = self.array_buffer_snapshot(&source)?;
        let new_length = if arguments.actual_arg_count == 0
            || matches!(arguments.readable.first(), Some(Value::Undefined))
        {
            u64::from(initial.byte_length)
        } else {
            match self.native_to_index(
                realm,
                arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "ArrayBuffer transfer argument was not padded",
                ))?,
            )? {
                NativeConversion::Value(length) => length,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        };

        let current = self.array_buffer_snapshot(&source)?;
        if current.detached {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        let result_maximum = if to_fixed_length {
            None
        } else {
            current.max_byte_length
        };
        if result_maximum.is_some_and(|maximum| new_length > u64::from(maximum)) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "invalid array buffer length",
            )?));
        }
        if new_length > MAX_ARRAY_BUFFER_LENGTH {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array buffer length",
            )?));
        }
        let new_length = usize::try_from(new_length)
            .map_err(|_| RuntimeError::Invariant("transfer length overflowed usize"))?;
        let prototype = self.array_buffer_default_prototype(realm)?;
        let Some(target) = self.new_array_buffer_object(&prototype, 0, result_maximum)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "out of memory",
            )?));
        };
        let transferred = self.0.state.borrow_mut().heap.transfer_array_buffer_bytes(
            source.object_id(),
            target.object_id(),
            new_length,
        )?;
        if !transferred {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "out of memory",
            )?));
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    fn array_buffer_species_constructor(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<Option<CallableRef>>, RuntimeError> {
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

    fn array_buffer_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(new_target_object) = new_target else {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer constructor new.target was not an object",
            ));
        };
        let prototype_key = self.intern_property_key("prototype")?;
        match self.get_property_in_realm(realm, &new_target_object, &prototype_key)? {
            Completion::Return(Value::Object(prototype)) => Ok(NativeConversion::Value(prototype)),
            Completion::Return(_) => {
                let new_target_callable =
                    self.callable_from_value(Value::Object(new_target_object))?;
                let fallback_realm = match self.function_realm(realm, &new_target_callable)? {
                    NativeConversion::Value(realm) => realm,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                };
                Ok(NativeConversion::Value(
                    self.array_buffer_default_prototype(fallback_realm)?,
                ))
            }
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    fn array_buffer_default_prototype(&self, realm: ContextId) -> Result<ObjectRef, RuntimeError> {
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .array_buffer
            .ok_or(RuntimeError::Invariant(
                "realm has no ArrayBuffer intrinsics",
            ))?
            .prototype;
        Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
    }

    fn require_array_buffer(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer object expected",
            )?));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("ArrayBuffer"));
        }
        if self.array_buffer_snapshot_if_branded(&object)?.is_none() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer object expected",
            )?));
        }
        Ok(NativeConversion::Value(object))
    }

    fn array_buffer_snapshot_if_branded(
        &self,
        object: &ObjectRef,
    ) -> Result<Option<ArrayBufferSnapshot>, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("ArrayBuffer"));
        }
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        let ObjectPayload::ArrayBuffer(data) = &object.payload else {
            return Ok(None);
        };
        Ok(Some(Self::array_buffer_snapshot_from_data(data)?))
    }

    fn array_buffer_snapshot(
        &self,
        object: &ObjectRef,
    ) -> Result<ArrayBufferSnapshot, RuntimeError> {
        self.array_buffer_snapshot_if_branded(object)?
            .ok_or(RuntimeError::Invariant(
                "validated ArrayBuffer lost its class payload",
            ))
    }

    fn array_buffer_snapshot_from_data(
        data: &ArrayBufferData,
    ) -> Result<ArrayBufferSnapshot, RuntimeError> {
        let byte_length = u32::try_from(data.bytes.len())
            .map_err(|_| RuntimeError::Invariant("ArrayBuffer byte length overflowed u32"))?;
        Ok(ArrayBufferSnapshot {
            byte_length,
            max_byte_length: data.max_byte_length,
            detached: data.detached,
        })
    }

    fn new_array_buffer_object(
        &self,
        prototype: &ObjectRef,
        byte_length: u32,
        max_byte_length: Option<u32>,
    ) -> Result<Option<ObjectRef>, RuntimeError> {
        if max_byte_length.is_some_and(|maximum| maximum < byte_length) {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer maximum is smaller than its initial length",
            ));
        }
        let byte_length = usize::try_from(byte_length)
            .map_err(|_| RuntimeError::Invariant("ArrayBuffer length overflowed usize"))?;
        let Some(bytes) = Self::try_zeroed_array_buffer_bytes(byte_length) else {
            return Ok(None);
        };
        self.new_array_buffer_from_bytes(prototype, bytes, max_byte_length)
            .map(Some)
    }

    fn try_zeroed_array_buffer_bytes(byte_length: usize) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(byte_length).ok()?;
        bytes.resize(byte_length, 0);
        Some(bytes)
    }

    fn new_array_buffer_from_bytes(
        &self,
        prototype: &ObjectRef,
        bytes: Vec<u8>,
        max_byte_length: Option<u32>,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("ArrayBuffer prototype"));
        }
        let byte_length = u32::try_from(bytes.len())
            .map_err(|_| RuntimeError::Invariant("ArrayBuffer byte length overflowed u32"))?;
        if byte_length > i32::MAX as u32
            || max_byte_length
                .is_some_and(|maximum| maximum < byte_length || maximum > i32::MAX as u32)
        {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer backing store exceeds the supported range",
            ));
        }

        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::array_buffer_from_bytes(
                shape,
                Vec::new(),
                bytes,
                max_byte_length,
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

    /// Pinned `JS_DetachArrayBuffer` core used by the Test262 host and later
    /// view classes. Non-ArrayBuffer values are rejected by callers before
    /// entering this branded mutation boundary.
    pub(in crate::runtime) fn detach_array_buffer_object(
        &self,
        object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("ArrayBuffer"));
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .detach_array_buffer(object.object_id())
            .map_err(RuntimeError::from)
    }

    /// Pinned `JS_DetachArrayBuffer` facade. QuickJS deliberately treats
    /// non-ArrayBuffer values and already-detached buffers as silent no-ops.
    pub(in crate::runtime) fn detach_array_buffer_value(
        &self,
        value: &Value,
    ) -> Result<(), RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(());
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("ArrayBuffer"));
        }
        if self.array_buffer_snapshot_if_branded(object)?.is_some() {
            self.detach_array_buffer_object(object)?;
        }
        Ok(())
    }

    pub(in crate::runtime) fn call_test262_detach_array_buffer(
        &self,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Test262 detachArrayBuffer received a constructor invocation",
            ));
        };
        let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Test262 detachArrayBuffer argument was not padded",
        ))?;
        self.detach_array_buffer_value(value)?;
        Ok(Completion::Return(Value::Undefined))
    }
}

impl Context {
    /// Create QuickJS's test262-only `$262.detachArrayBuffer` host function.
    ///
    /// The function is not an ECMAScript intrinsic. Embedders choose whether
    /// and where to publish it.
    pub fn new_detach_array_buffer_function(&mut self) -> Result<CallableRef, RuntimeError> {
        let function_prototype = self.function_prototype()?;
        self.runtime.new_native_builtin(
            &function_prototype,
            self.realm,
            NativeFunctionId::Test262DetachArrayBuffer,
            1,
            "detachArrayBuffer",
            1,
        )
    }

    /// Apply pinned QuickJS `JS_DetachArrayBuffer` semantics to one value.
    ///
    /// Values which are not ordinary ArrayBuffers are silent no-ops.
    pub fn detach_array_buffer(&mut self, value: &Value) -> Result<(), RuntimeError> {
        self.runtime.detach_array_buffer_value(value)
    }
}
