//! TypedArray species construction shared by copying prototype methods.
//!
//! QuickJS uses `JS_SpeciesConstructor` with an undefined default sentinel,
//! then either constructs the authored species or allocates the source element
//! class directly in the builtin's defining realm.

use super::*;

enum TypedArraySpeciesConstructor {
    Default,
    Custom(Value),
}

impl Runtime {
    pub(super) fn typed_array_species_create(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        source_element: TypedArrayElementKind,
        length: u64,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        match self.typed_array_species_constructor(realm, source)? {
            NativeConversion::Value(TypedArraySpeciesConstructor::Default) => {
                let prototype = self.typed_array_default_prototype(realm, source_element)?;
                self.new_typed_array_for_length(realm, &prototype, source_element, length)
            }
            NativeConversion::Value(TypedArraySpeciesConstructor::Custom(species)) => {
                self.typed_array_create_from_constructor(realm, species, length)
            }
            NativeConversion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(super) fn typed_array_species_create_subarray(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        source_element: TypedArrayElementKind,
        buffer: &ObjectRef,
        byte_offset: u64,
        length: Option<u64>,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let species = match self.typed_array_species_constructor(realm, source)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(species, TypedArraySpeciesConstructor::Default) {
            let prototype = self.typed_array_default_prototype(realm, source_element)?;
            return self.new_typed_array_view_from_coerced(
                realm,
                &prototype,
                source_element,
                buffer,
                byte_offset,
                length,
            );
        }

        let TypedArraySpeciesConstructor::Custom(species) = species else {
            unreachable!("default TypedArray species was handled above");
        };
        let mut arguments = vec![
            Value::Object(buffer.clone()),
            Value::number(byte_offset as f64),
        ];
        if let Some(length) = length {
            arguments.push(Value::number(length as f64));
        }
        self.typed_array_create_from_constructor_arguments(realm, species, &arguments, None)
    }

    fn typed_array_species_constructor(
        &self,
        realm: ContextId,
        source: &ObjectRef,
    ) -> Result<NativeConversion<TypedArraySpeciesConstructor>, RuntimeError> {
        let constructor_key = self.intern_property_key("constructor")?;
        let constructor = match self.get_property_in_realm(realm, source, &constructor_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let species = if matches!(&constructor, Value::Undefined) {
            Value::Undefined
        } else {
            let Value::Object(constructor) = constructor else {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            };
            let species_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
            match self.get_property_in_realm(realm, &constructor, &species_key)? {
                Completion::Return(Value::Null | Value::Undefined) => Value::Undefined,
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        };

        if matches!(&species, Value::Undefined) {
            return Ok(NativeConversion::Value(
                TypedArraySpeciesConstructor::Default,
            ));
        }
        Ok(NativeConversion::Value(
            TypedArraySpeciesConstructor::Custom(species),
        ))
    }

    pub(super) fn typed_array_create_from_constructor(
        &self,
        realm: ContextId,
        constructor: Value,
        length: u64,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        self.typed_array_create_from_constructor_arguments(
            realm,
            constructor,
            &[Value::number(length as f64)],
            Some(length),
        )
    }

    /// Construct and validate a species result with the exact authored
    /// argument vector. QuickJS only enforces a minimum result length for its
    /// one-argument create form; `subarray` deliberately uses two or three
    /// arguments and therefore accepts any live TypedArray result.
    fn typed_array_create_from_constructor_arguments(
        &self,
        realm: ContextId,
        constructor: Value,
        arguments: &[Value],
        minimum_length: Option<u64>,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(object) = constructor else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a constructor",
            )?));
        };
        if !self.is_constructor(&object)? {
            return Ok(NativeConversion::Throw(
                self.new_not_constructor_error(realm, &Value::Object(object))?,
            ));
        }
        let constructor = self.callable_from_value(Value::Object(object))?;
        let target = match self.construct_internal(realm, &constructor, &constructor, arguments)? {
            Completion::Return(Value::Object(value)) => value,
            Completion::Return(_) => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a TypedArray",
                )?));
            }
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(_) = self.typed_array_snapshot_if_branded(&target)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a TypedArray",
            )?));
        };
        let target_length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if minimum_length.is_some_and(|length| u64::from(target_length) < length) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "TypedArray length is too small",
            )?));
        }
        Ok(NativeConversion::Value(target))
    }

    /// Finish the ArrayBuffer overload after every observable prototype and
    /// numeric conversion. `subarray` reuses this exact non-observable tail
    /// for its default-species path, bypassing mutable public constructors.
    pub(super) fn new_typed_array_view_from_coerced(
        &self,
        realm: ContextId,
        prototype: &ObjectRef,
        element: TypedArrayElementKind,
        buffer: &ObjectRef,
        byte_offset: u64,
        requested_length: Option<u64>,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let width = u64::from(element.byte_length());
        // Alignment precedes the detached-buffer check in pinned QuickJS.
        if byte_offset % width != 0 {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid offset",
            )?));
        }
        let backing = self.array_buffer_snapshot(buffer)?;
        if backing.detached {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        let (byte_offset, fixed_byte_length) = if let Some(length) = requested_length {
            let bytes = length.checked_mul(width).ok_or(RuntimeError::Invariant(
                "ToIndex TypedArray byte length overflowed u64",
            ))?;
            let end = byte_offset
                .checked_add(bytes)
                .ok_or(RuntimeError::Invariant(
                    "ToIndex TypedArray end offset overflowed u64",
                ))?;
            if end > u64::from(backing.byte_length) {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "invalid length",
                )?));
            }
            (
                u32::try_from(byte_offset)
                    .map_err(|_| RuntimeError::Invariant("validated byteOffset overflowed u32"))?,
                Some(u32::try_from(bytes).map_err(|_| {
                    RuntimeError::Invariant("validated TypedArray byte length overflowed u32")
                })?),
            )
        } else {
            if byte_offset > u64::from(backing.byte_length) {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "invalid offset",
                )?));
            }
            let byte_offset = u32::try_from(byte_offset)
                .map_err(|_| RuntimeError::Invariant("validated byteOffset overflowed u32"))?;
            let available = backing.byte_length - byte_offset;
            let fixed_byte_length = if backing.max_byte_length.is_some() {
                None
            } else {
                if u64::from(available) % width != 0 {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "invalid length",
                    )?));
                }
                Some(available)
            };
            (byte_offset, fixed_byte_length)
        };
        let target = self.new_typed_array_object(
            prototype,
            buffer,
            byte_offset,
            fixed_byte_length,
            element,
        )?;
        Ok(NativeConversion::Value(target))
    }

    pub(super) fn typed_array_filter_result(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        source_element: TypedArrayElementKind,
        selected: ObjectRef,
        selected_length: u64,
    ) -> Result<Completion, RuntimeError> {
        let target = match self.typed_array_species_create(
            realm,
            source,
            source_element,
            selected_length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let set_key = self.intern_property_key("set")?;
        let set = match self.get_property_in_realm(realm, &target, &set_key)? {
            Completion::Return(value) => self.callable_from_value(value)?,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        match self.call_internal(
            realm,
            &set,
            Value::Object(target.clone()),
            &[Value::Object(selected)],
        )? {
            Completion::Return(_) => Ok(Completion::Return(Value::Object(target))),
            Completion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }
}
