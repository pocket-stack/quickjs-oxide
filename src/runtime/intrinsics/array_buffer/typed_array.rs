//! The twelve concrete TypedArray classes over the shared ArrayBuffer store.
//!
//! Pinned QuickJS implements these classes through one fast-array kernel.  The
//! Rust representation follows that layout: every object stores a byte view
//! and one element selector, while detach and resizable-buffer bounds are
//! derived from the backing store for every observable operation.

use crate::atom::PropertyKeyKind;
use crate::heap::{
    ArrayBufferViewData, ArrayFindKind, ArrayIterationKind, ArrayIteratorKind, ArraySearchKind,
    ObjectData, ObjectPayload, TypedArrayData, TypedArrayElementKind, TypedArrayNativeKind,
    TypedArrayRealmData,
};

use super::*;

mod find;
mod iteration;
mod mutation;
mod search;
#[cfg(test)]
mod tests;

/// Classification of an ECMAScript CanonicalNumericIndexString.
///
/// `Invalid` is still a canonical numeric key, but it can never denote a
/// TypedArray element (`-0`, negative/fractional values, NaN, or infinities).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) enum CanonicalNumericIndex {
    Valid(u64),
    Invalid,
}

/// Durable TypedArray metadata copied out of the heap before observable work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) struct TypedArraySnapshot {
    pub buffer: ObjectId,
    pub byte_offset: u32,
    pub fixed_byte_length: Option<u32>,
    pub element: TypedArrayElementKind,
}

/// Current view state derived from a snapshot and its backing ArrayBuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) struct TypedArrayState {
    pub snapshot: TypedArraySnapshot,
    pub length: u32,
    pub byte_length: u32,
    pub out_of_bounds: bool,
    pub resizable: bool,
}

impl Runtime {
    /// Install the hidden `%TypedArray%` constructor/prototype pair and the
    /// twelve public concrete classes in QuickJS class-id order.
    pub(in crate::runtime) fn initialize_typed_array_intrinsics(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let base_prototype = self.new_object(Some(object_prototype))?;

        self.define_native_builtin_getter_on(
            &base_prototype,
            function_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Length),
            "length",
            "get length",
        )?;
        self.define_native_builtin_auto_init(
            &base_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::At),
            "at",
            1,
            1,
        )?;
        for (kind, name) in [
            (TypedArrayNativeKind::Buffer, "buffer"),
            (TypedArrayNativeKind::ByteLength, "byteLength"),
            (TypedArrayNativeKind::ByteOffset, "byteOffset"),
        ] {
            self.define_native_builtin_getter_on(
                &base_prototype,
                function_prototype,
                realm,
                NativeFunctionId::TypedArray(kind),
                name,
                &format!("get {name}"),
            )?;
        }
        self.define_native_builtin_auto_init(
            &base_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Set),
            "set",
            1,
            2,
        )?;
        for (kind, name) in [
            (ArrayIteratorKind::Value, "values"),
            (ArrayIteratorKind::Key, "keys"),
            (ArrayIteratorKind::KeyAndValue, "entries"),
        ] {
            self.define_native_builtin_auto_init(
                &base_prototype,
                realm,
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Iterator(kind)),
                name,
                0,
                0,
            )?;
        }
        self.define_native_builtin_auto_init(
            &base_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::CopyWithin),
            "copyWithin",
            2,
            2,
        )?;
        for (kind, name) in [
            (ArrayIterationKind::Every, "every"),
            (ArrayIterationKind::Some, "some"),
        ] {
            self.define_native_builtin_auto_init(
                &base_prototype,
                realm,
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Iteration(kind)),
                name,
                1,
                1,
            )?;
        }
        self.define_native_builtin_auto_init(
            &base_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Fill),
            "fill",
            1,
            1,
        )?;
        for (kind, name) in [
            (ArrayFindKind::Find, "find"),
            (ArrayFindKind::FindIndex, "findIndex"),
            (ArrayFindKind::FindLast, "findLast"),
            (ArrayFindKind::FindLastIndex, "findLastIndex"),
        ] {
            self.define_native_builtin_auto_init(
                &base_prototype,
                realm,
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Find(kind)),
                name,
                1,
                1,
            )?;
        }
        self.define_native_builtin_auto_init(
            &base_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Reverse),
            "reverse",
            0,
            0,
        )?;
        for (kind, name) in [
            (ArraySearchKind::IndexOf, "indexOf"),
            (ArraySearchKind::LastIndexOf, "lastIndexOf"),
            (ArraySearchKind::Includes, "includes"),
        ] {
            self.define_native_builtin_auto_init(
                &base_prototype,
                realm,
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Search(kind)),
                name,
                1,
                1,
            )?;
        }

        let base_constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::BaseConstructor),
            0,
            "TypedArray",
            0,
        )?;
        self.define_native_builtin_auto_init(
            base_constructor.as_object(),
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::From),
            "from",
            1,
            3,
        )?;
        self.define_native_builtin_auto_init(
            base_constructor.as_object(),
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Of),
            "of",
            0,
            0,
        )?;
        let species_getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Species),
            0,
            "get [Symbol.species]",
            0,
        )?;
        let species = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
        if !self.define_own_property(
            base_constructor.as_object(),
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
                "TypedArray species definition was rejected",
            ));
        }
        self.define_constructor_relationship(&base_constructor, &base_prototype)?;

        let mut prototypes = Vec::with_capacity(TypedArrayElementKind::COUNT);
        let mut constructors = Vec::with_capacity(TypedArrayElementKind::COUNT);
        for element in TypedArrayElementKind::ALL {
            let prototype = self.new_object(Some(&base_prototype))?;
            self.define_function_data_property(
                &prototype,
                "BYTES_PER_ELEMENT",
                Value::Int(i32::from(element.byte_length())),
                false,
                false,
            )?;
            let constructor = self.new_native_builtin(
                base_constructor.as_object(),
                realm,
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Constructor(element)),
                3,
                element.name(),
                3,
            )?;
            self.define_function_data_property(
                constructor.as_object(),
                "BYTES_PER_ELEMENT",
                Value::Int(i32::from(element.byte_length())),
                false,
                false,
            )?;
            self.define_constructor_relationship(&constructor, &prototype)?;
            self.define_function_data_property(
                global_object,
                element.name(),
                Value::Object(constructor.as_object().clone()),
                true,
                true,
            )?;
            prototypes.push(prototype);
            constructors.push(constructor);
        }

        let array_prototype = {
            let id = self.0.state.borrow().heap.context(realm)?.array_prototype;
            ObjectRef::from_borrowed_handle(self.clone(), id)?
        };
        let to_string_key = self.intern_property_key("toString")?;
        let to_string = match self.get_property_in_realm(realm, &array_prototype, &to_string_key)? {
            Completion::Return(value @ Value::Object(_)) => value,
            Completion::Return(_) | Completion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "Array.prototype.toString was unavailable during TypedArray bootstrap",
                ));
            }
        };
        if !self.define_own_property(
            &base_prototype,
            &to_string_key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(to_string),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "TypedArray toString alias definition was rejected",
            ));
        }

        let values_key = self.intern_property_key("values")?;
        let values = match self.get_property_in_realm(realm, &base_prototype, &values_key)? {
            Completion::Return(value @ Value::Object(_)) => value,
            Completion::Return(_) | Completion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "TypedArray values was unavailable during iterator alias bootstrap",
                ));
            }
        };
        let iterator = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        if !self.define_own_property(
            &base_prototype,
            &iterator,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(values),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "TypedArray iterator alias definition was rejected",
            ));
        }

        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        let tag_getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::ToStringTag),
            0,
            "get [Symbol.toStringTag]",
            0,
        )?;
        if !self.define_own_property(
            &base_prototype,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(tag_getter)),
                set: DescriptorField::Present(AccessorValue::Undefined),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "TypedArray toStringTag definition was rejected",
            ));
        }

        let prototypes: [ObjectRef; TypedArrayElementKind::COUNT] =
            prototypes.try_into().map_err(|_| {
                RuntimeError::Invariant("TypedArray prototype table has the wrong length")
            })?;
        let constructors: [CallableRef; TypedArrayElementKind::COUNT] =
            constructors.try_into().map_err(|_| {
                RuntimeError::Invariant("TypedArray constructor table has the wrong length")
            })?;
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_typed_array_intrinsics(
                realm,
                base_constructor.as_object().object_id(),
                base_prototype.object_id(),
                constructors.map(|constructor| constructor.as_object().object_id()),
                TypedArrayRealmData {
                    prototypes: prototypes.map(|prototype| prototype.object_id()),
                },
            )?;
        Ok(())
    }

    pub(in crate::runtime) fn call_typed_array_native(
        &self,
        realm: ContextId,
        kind: TypedArrayNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            TypedArrayNativeKind::BaseConstructor => Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot be called",
            )?)),
            TypedArrayNativeKind::Constructor(element) => {
                self.call_typed_array_constructor(realm, element, invocation, arguments)
            }
            TypedArrayNativeKind::From => self.call_typed_array_from(realm, invocation, arguments),
            TypedArrayNativeKind::Of => self.call_typed_array_of(realm, invocation, arguments),
            TypedArrayNativeKind::Species => self.call_typed_array_species(invocation),
            TypedArrayNativeKind::Length
            | TypedArrayNativeKind::Buffer
            | TypedArrayNativeKind::ByteLength
            | TypedArrayNativeKind::ByteOffset
            | TypedArrayNativeKind::ToStringTag => {
                self.call_typed_array_getter(realm, kind, invocation)
            }
            TypedArrayNativeKind::Set => self.call_typed_array_set(realm, invocation, arguments),
            TypedArrayNativeKind::Iterator(kind) => {
                self.call_typed_array_iterator(realm, kind, invocation)
            }
            TypedArrayNativeKind::CopyWithin => {
                self.call_typed_array_copy_within(realm, invocation, arguments)
            }
            TypedArrayNativeKind::Iteration(kind) => {
                self.call_typed_array_iteration(realm, kind, invocation, arguments)
            }
            TypedArrayNativeKind::Fill => self.call_typed_array_fill(realm, invocation, arguments),
            TypedArrayNativeKind::Reverse => self.call_typed_array_reverse(realm, invocation),
            TypedArrayNativeKind::At => self.call_typed_array_at(realm, invocation, arguments),
            TypedArrayNativeKind::Search(kind) => {
                self.call_typed_array_search(realm, kind, invocation, arguments)
            }
            TypedArrayNativeKind::Find(kind) => {
                self.call_typed_array_find(realm, kind, invocation, arguments)
            }
            TypedArrayNativeKind::With
            | TypedArrayNativeKind::Reduce(_)
            | TypedArrayNativeKind::ToReversed
            | TypedArrayNativeKind::Slice
            | TypedArrayNativeKind::Subarray
            | TypedArrayNativeKind::Sort
            | TypedArrayNativeKind::ToSorted
            | TypedArrayNativeKind::Join(_) => Err(RuntimeError::Invariant(
                "unpublished TypedArray native reached dispatch",
            )),
        }
    }

    fn call_typed_array_constructor(
        &self,
        realm: ContextId,
        element: TypedArrayElementKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray constructor did not receive a constructor invocation",
            ));
        };
        let first = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "TypedArray constructor argv was not padded",
            ))?;

        let Value::Object(source) = first else {
            // ToIndex precedes the observable newTarget.prototype lookup, but
            // the backing-store size limit is checked only while allocating
            // after that lookup, matching js_typed_array_constructor.
            let length = match self.native_to_index(realm, &first)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let prototype =
                match self.typed_array_prototype_from_new_target(realm, new_target, element)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            let target =
                match self.new_typed_array_for_length(realm, &prototype, element, length)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            return Ok(Completion::Return(Value::Object(target)));
        };
        if !source.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("TypedArray constructor source"));
        }

        if self.array_buffer_snapshot_if_branded(&source)?.is_some() {
            return self
                .construct_typed_array_from_buffer(realm, element, new_target, &source, arguments);
        }
        if let Some(source_snapshot) = self.typed_array_snapshot_if_branded(&source)? {
            return self.construct_typed_array_from_typed_array(
                realm,
                element,
                new_target,
                &source,
                source_snapshot,
            );
        }
        self.construct_typed_array_from_object(realm, element, new_target, &source)
    }

    fn construct_typed_array_from_buffer(
        &self,
        realm: ContextId,
        element: TypedArrayElementKind,
        new_target: Value,
        buffer: &ObjectRef,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        // QuickJS gets the public instance prototype before coercing either
        // byteOffset or length in the raw-ArrayBuffer overload.
        let prototype =
            match self.typed_array_prototype_from_new_target(realm, new_target, element)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let byte_offset = if arguments.actual_arg_count > 1 {
            match self.native_to_index(
                realm,
                arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "TypedArray byteOffset argv was not padded",
                ))?,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            0
        };
        let width = u64::from(element.byte_length());
        // Alignment precedes the detached-buffer check in pinned QuickJS.
        if byte_offset % width != 0 {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid byteOffset",
            )?));
        }

        let explicit_length = arguments.actual_arg_count > 2
            && !matches!(arguments.readable.get(2), Some(Value::Undefined));
        let requested_length = if explicit_length {
            let length = match self.native_to_index(
                realm,
                arguments.readable.get(2).ok_or(RuntimeError::Invariant(
                    "TypedArray length argv was not padded",
                ))?,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            Some(length)
        } else {
            None
        };

        let backing = self.array_buffer_snapshot(buffer)?;
        if backing.detached {
            return Ok(Completion::Throw(self.new_native_error(
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
                return Ok(Completion::Throw(self.typed_array_invalid_length(realm)?));
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
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "invalid byteOffset",
                )?));
            }
            let byte_offset = u32::try_from(byte_offset)
                .map_err(|_| RuntimeError::Invariant("validated byteOffset overflowed u32"))?;
            let available = backing.byte_length - byte_offset;
            let fixed_byte_length = if backing.max_byte_length.is_some() {
                None
            } else {
                if u64::from(available) % width != 0 {
                    return Ok(Completion::Throw(self.typed_array_invalid_length(realm)?));
                }
                Some(available)
            };
            (byte_offset, fixed_byte_length)
        };
        let target = self.new_typed_array_object(
            &prototype,
            buffer,
            byte_offset,
            fixed_byte_length,
            element,
        )?;
        Ok(Completion::Return(Value::Object(target)))
    }

    fn construct_typed_array_from_typed_array(
        &self,
        realm: ContextId,
        element: TypedArrayElementKind,
        new_target: Value,
        source: &ObjectRef,
        source_snapshot: TypedArraySnapshot,
    ) -> Result<Completion, RuntimeError> {
        // QuickJS snapshots the current count before GetPrototypeFromConstructor.
        // An initially OOB tracking view therefore retains count zero even if
        // a prototype getter grows it back into bounds.
        let source_count = self
            .typed_array_state_from_snapshot(source_snapshot)?
            .length;
        let prototype =
            match self.typed_array_prototype_from_new_target(realm, new_target, element)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let current = self.typed_array_state_from_snapshot(source_snapshot)?;
        if current.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "out of bound",
            )?));
        }
        let target = match self.new_typed_array_for_length(
            realm,
            &prototype,
            element,
            u64::from(source_count),
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let current = self.typed_array_state(source)?;
        if current.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "out of bound",
            )?));
        }

        if source_snapshot.element == element && current.length >= source_count {
            // Same-class construction copies raw bytes, preserving NaN payloads
            // and negative zero. QuickJS only takes this path when the complete
            // snapshotted source range still exists after prototype lookup.
            if source_count != 0 {
                let target_snapshot = self.typed_array_snapshot(&target)?;
                let byte_count =
                    usize::try_from(u64::from(source_count) * u64::from(element.byte_length()))
                        .map_err(|_| {
                            RuntimeError::Invariant("TypedArray raw copy length overflowed")
                        })?;
                self.0.state.borrow_mut().heap.copy_array_buffer_range(
                    source_snapshot.buffer,
                    target_snapshot.buffer,
                    usize::try_from(source_snapshot.byte_offset).map_err(|_| {
                        RuntimeError::Invariant("TypedArray source byteOffset overflowed usize")
                    })?,
                    byte_count,
                )?;
            }
        } else {
            // A tracking source may shrink without becoming out of bounds while
            // GetPrototypeFromConstructor runs. The missing tail is then read
            // as undefined and converted element-by-element, just as it is for
            // a different element kind.
            for index in 0..u64::from(source_count) {
                let value = self
                    .typed_array_read_index(source, index)?
                    .unwrap_or(Value::Undefined);
                match self.typed_array_set_index(realm, &target, index, &value)? {
                    NativeConversion::Value(()) => {}
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    fn construct_typed_array_from_object(
        &self,
        realm: ContextId,
        element: TypedArrayElementKind,
        new_target: Value,
        source: &ObjectRef,
    ) -> Result<Completion, RuntimeError> {
        let prototype =
            match self.typed_array_prototype_from_new_target(realm, new_target, element)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let source_value = Value::Object(source.clone());
        let iterator = match self.typed_array_iterator_method(realm, source_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if let Some(iterator) = iterator {
            let values =
                match self.collect_typed_array_iterator(realm, source_value, &iterator, element)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            let target = match self.new_typed_array_for_length(
                realm,
                &prototype,
                element,
                u64::try_from(values.len()).map_err(|_| {
                    RuntimeError::Invariant("TypedArray iterable length overflowed u64")
                })?,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            for (index, value) in values.iter().enumerate() {
                match self.typed_array_set_index(realm, &target, index as u64, value)? {
                    NativeConversion::Value(()) => {}
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
            return Ok(Completion::Return(Value::Object(target)));
        }

        let length_key = self.intern_property_key("length")?;
        let length_value = match self.get_property_in_realm(realm, source, &length_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.native_to_length(realm, &length_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target = match self.new_typed_array_for_length(realm, &prototype, element, length)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        for index in 0..length {
            let key = self.intern_property_key(&index.to_string())?;
            let value = match self.get_property_in_realm(realm, source, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            match self.typed_array_set_index(realm, &target, index, &value)? {
                NativeConversion::Value(()) => {}
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    fn call_typed_array_species(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray species did not receive a getter invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    fn call_typed_array_getter(
        &self,
        realm: ContextId,
        kind: TypedArrayNativeKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray prototype getter received a non-getter invocation",
            ));
        };
        if kind == TypedArrayNativeKind::ToStringTag {
            let Value::Object(object) = this_value else {
                return Ok(Completion::Return(Value::Undefined));
            };
            let Some(snapshot) = self.typed_array_snapshot_if_branded(&object)? else {
                return Ok(Completion::Return(Value::Undefined));
            };
            return Ok(Completion::Return(Value::String(JsString::from_static(
                snapshot.element.name(),
            ))));
        }
        let object = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let state = self.typed_array_state(&object)?;
        let result = match kind {
            TypedArrayNativeKind::Buffer => Value::Object(ObjectRef::from_borrowed_handle(
                self.clone(),
                state.snapshot.buffer,
            )?),
            TypedArrayNativeKind::Length => typed_array_u32_value(state.length),
            TypedArrayNativeKind::ByteLength => typed_array_u32_value(state.byte_length),
            TypedArrayNativeKind::ByteOffset => typed_array_u32_value(if state.out_of_bounds {
                0
            } else {
                state.snapshot.byte_offset
            }),
            _ => {
                return Err(RuntimeError::Invariant(
                    "non-getter TypedArray native reached getter dispatch",
                ));
            }
        };
        Ok(Completion::Return(result))
    }

    fn call_typed_array_iterator(
        &self,
        realm: ContextId,
        kind: ArrayIteratorKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray iterator factory received a constructor invocation",
            ));
        };
        let object = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        match self.typed_array_validated_length(realm, &object)? {
            NativeConversion::Value(_) => {}
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        }
        self.call_array_prototype_iterator(
            realm,
            kind,
            NativeInvocation::Call {
                this_value: Value::Object(object),
            },
        )
    }

    fn call_typed_array_from(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.from received a constructor invocation",
            ));
        };
        let source = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "TypedArray.from argv was not padded",
            ))?;
        let mapping =
            arguments.actual_arg_count > 1 && !matches!(arguments.readable[1], Value::Undefined);
        let mapfn = if mapping {
            let Value::Object(object) = arguments.readable[1].clone() else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            let Some(callable) = self.as_callable(&object)? else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            Some(callable)
        } else {
            None
        };
        let map_this = if arguments.actual_arg_count > 2 {
            arguments.readable[2].clone()
        } else {
            Value::Undefined
        };
        let iterator = match self.typed_array_iterator_method(realm, source.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if let Some(iterator) = iterator {
            // TypedArray.from and the object constructor both materialize the
            // entire iterator before allocation and numeric conversion.
            let values = match self.collect_typed_array_iterator(
                realm,
                source,
                &iterator,
                TypedArrayElementKind::Uint8,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let target = match self.typed_array_create_from_constructor(
                realm,
                this_value,
                values.len() as u64,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            for (index, mut value) in values.into_iter().enumerate() {
                if let Some(mapfn) = &mapfn {
                    value = match self.call_internal(
                        realm,
                        mapfn,
                        map_this.clone(),
                        &[value, Value::number(index as f64)],
                    )? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                }
                match self.typed_array_set_index(realm, &target, index as u64, &value)? {
                    NativeConversion::Value(()) => {}
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
            return Ok(Completion::Return(Value::Object(target)));
        }

        let source = match self.native_to_object(realm, source)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length_key = self.intern_property_key("length")?;
        let length_value = match self.get_property_in_realm(realm, &source, &length_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.native_to_length(realm, &length_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target = match self.typed_array_create_from_constructor(realm, this_value, length)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        for index in 0..length {
            let key = self.intern_property_key(&index.to_string())?;
            let mut value = match self.get_property_in_realm(realm, &source, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if let Some(mapfn) = &mapfn {
                value = match self.call_internal(
                    realm,
                    mapfn,
                    map_this.clone(),
                    &[value, Value::number(index as f64)],
                )? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            }
            match self.typed_array_set_index(realm, &target, index, &value)? {
                NativeConversion::Value(()) => {}
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    fn call_typed_array_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.of received a constructor invocation",
            ));
        };
        let length = u64::try_from(arguments.actual_arg_count)
            .map_err(|_| RuntimeError::Invariant("TypedArray.of argc overflowed u64"))?;
        let target = match self.typed_array_create_from_constructor(realm, this_value, length)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        for (index, value) in arguments.readable[..arguments.actual_arg_count]
            .iter()
            .enumerate()
        {
            match self.typed_array_set_index(realm, &target, index as u64, value)? {
                NativeConversion::Value(()) => {}
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    fn typed_array_create_from_constructor(
        &self,
        realm: ContextId,
        constructor: Value,
        length: u64,
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
        let target = match self.construct_internal(
            realm,
            &constructor,
            &constructor,
            &[Value::number(length as f64)],
        )? {
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
        if u64::from(target_length) < length {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "typed array is too short",
            )?));
        }
        Ok(NativeConversion::Value(target))
    }

    fn call_typed_array_set(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.set received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let offset = match self.native_to_int64_sat(
            realm,
            arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "TypedArray.set offset argv was not padded",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if offset < 0 {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid offset",
            )?));
        }
        let offset = offset as u64;
        // Offset coercion is followed by an explicit validation. Later
        // array-like length/element getters use this cached count; detach or
        // shrink during those getters makes individual writes disappear
        // rather than retroactively throwing from `set`.
        let target_length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let source = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "TypedArray.set source argv was not padded",
            ))?;
        if let Value::Object(source_object) = &source
            && let Some(source_snapshot) = self.typed_array_snapshot_if_branded(source_object)?
        {
            return self.set_typed_array_from_typed_array(
                realm,
                &target,
                target_length,
                offset,
                source_object,
                source_snapshot,
            );
        }
        self.set_typed_array_from_array_like(realm, &target, target_length, offset, source)
    }

    fn set_typed_array_from_typed_array(
        &self,
        realm: ContextId,
        target: &ObjectRef,
        target_length: u32,
        offset: u64,
        source: &ObjectRef,
        source_snapshot: TypedArraySnapshot,
    ) -> Result<Completion, RuntimeError> {
        let source_state = self.typed_array_state_from_snapshot(source_snapshot)?;
        if source_state.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "out of bound",
            )?));
        }
        let target_element = self.typed_array_snapshot(target)?.element;
        if offset
            .checked_add(u64::from(source_state.length))
            .is_none_or(|end| end > u64::from(target_length))
        {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "out of bound",
            )?));
        }
        if source_snapshot.element == target_element {
            let target_snapshot = self.typed_array_snapshot(target)?;
            let source_start = typed_array_absolute_byte_offset(source_snapshot, 0)?;
            let target_start = typed_array_absolute_byte_offset(target_snapshot, offset)?;
            let byte_count = usize::try_from(
                u64::from(source_state.length) * u64::from(source_snapshot.element.byte_length()),
            )
            .map_err(|_| RuntimeError::Invariant("TypedArray set byte length overflowed usize"))?;
            self.0.state.borrow_mut().heap.move_array_buffer_range(
                source_snapshot.buffer,
                target_snapshot.buffer,
                source_start,
                target_start,
                byte_count,
            )?;
        } else {
            // This intentionally reads and writes one element at a time even
            // for overlapping views on the same backing buffer. Pinned
            // QuickJS exposes that non-temporary-copy behavior for different
            // element types, including its known overlap result.
            for index in 0..u64::from(source_state.length) {
                let value = self
                    .typed_array_read_index(source, index)?
                    .unwrap_or(Value::Undefined);
                match self.typed_array_set_index(realm, target, offset + index, &value)? {
                    NativeConversion::Value(()) => {}
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
        }
        Ok(Completion::Return(Value::Undefined))
    }

    fn set_typed_array_from_array_like(
        &self,
        realm: ContextId,
        target: &ObjectRef,
        target_length: u32,
        offset: u64,
        source: Value,
    ) -> Result<Completion, RuntimeError> {
        let source = match self.native_to_object(realm, source)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length_key = self.intern_property_key("length")?;
        let length_value = match self.get_property_in_realm(realm, &source, &length_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.native_to_length(realm, &length_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if offset
            .checked_add(length)
            .is_none_or(|end| end > u64::from(target_length))
        {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "out of bound",
            )?));
        }
        for index in 0..length {
            let key = self.intern_property_key(&index.to_string())?;
            let value = match self.get_property_in_realm(realm, &source, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            match self.typed_array_set_index(realm, target, offset + index, &value)? {
                NativeConversion::Value(()) => {}
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        Ok(Completion::Return(Value::Undefined))
    }

    fn require_typed_array(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a TypedArray",
            )?));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("TypedArray"));
        }
        if self.typed_array_snapshot_if_branded(&object)?.is_none() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a TypedArray",
            )?));
        }
        Ok(NativeConversion::Value(object))
    }

    fn typed_array_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
        element: TypedArrayElementKind,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(new_target_object) = new_target else {
            return Err(RuntimeError::Invariant(
                "TypedArray constructor new.target was not an object",
            ));
        };
        let prototype_key = self.intern_property_key("prototype")?;
        match self.get_property_in_realm(realm, &new_target_object, &prototype_key)? {
            Completion::Return(Value::Object(prototype)) => Ok(NativeConversion::Value(prototype)),
            Completion::Return(_) => {
                let new_target = self.callable_from_value(Value::Object(new_target_object))?;
                let fallback_realm = match self.function_realm(realm, &new_target)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                };
                Ok(NativeConversion::Value(
                    self.typed_array_default_prototype(fallback_realm, element)?,
                ))
            }
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    fn typed_array_default_prototype(
        &self,
        realm: ContextId,
        element: TypedArrayElementKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .typed_array
            .ok_or(RuntimeError::Invariant(
                "realm has no TypedArray intrinsics",
            ))?
            .prototypes[element as usize];
        Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
    }

    fn new_typed_array_for_length(
        &self,
        realm: ContextId,
        prototype: &ObjectRef,
        element: TypedArrayElementKind,
        length: u64,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        if !typed_array_length_is_supported(element, length) {
            return Ok(NativeConversion::Throw(
                self.typed_array_invalid_length(realm)?,
            ));
        }
        let byte_length = u32::try_from(length * u64::from(element.byte_length()))
            .map_err(|_| RuntimeError::Invariant("TypedArray byte length overflowed u32"))?;
        let array_buffer_prototype = self.array_buffer_default_prototype(realm)?;
        let Some(buffer) =
            self.new_array_buffer_object(&array_buffer_prototype, byte_length, None)?
        else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "out of memory",
            )?));
        };
        Ok(NativeConversion::Value(self.new_typed_array_object(
            prototype,
            &buffer,
            0,
            Some(byte_length),
            element,
        )?))
    }

    fn new_typed_array_object(
        &self,
        prototype: &ObjectRef,
        buffer: &ObjectRef,
        byte_offset: u32,
        fixed_byte_length: Option<u32>,
        element: TypedArrayElementKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) || !buffer.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("TypedArray allocation"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state.heap.allocate_object(ObjectData::typed_array(
            shape,
            Vec::new(),
            TypedArrayData {
                view: ArrayBufferViewData {
                    buffer: buffer.object_id(),
                    byte_offset,
                    fixed_byte_length,
                },
                element,
            },
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

    fn typed_array_invalid_length(&self, realm: ContextId) -> Result<Value, RuntimeError> {
        self.new_native_error(realm, NativeErrorKind::Range, "invalid typed array length")
    }

    fn typed_array_iterator_method(
        &self,
        realm: ContextId,
        source: Value,
    ) -> Result<NativeConversion<Option<CallableRef>>, RuntimeError> {
        if matches!(source, Value::Null | Value::Undefined) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot get iterator",
            )?));
        }
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        let method = match self.get_value_property_in_realm(realm, source, &key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(method, Value::Undefined | Value::Null) {
            return Ok(NativeConversion::Value(None));
        }
        let Value::Object(method_object) = method else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "value is not iterable",
            )?));
        };
        let Some(method) = self.as_callable(&method_object)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "value is not iterable",
            )?));
        };
        Ok(NativeConversion::Value(Some(method)))
    }

    fn collect_typed_array_iterator(
        &self,
        realm: ContextId,
        source: Value,
        method: &CallableRef,
        element: TypedArrayElementKind,
    ) -> Result<NativeConversion<Vec<Value>>, RuntimeError> {
        let iterator = match self.call_internal(realm, method, source, &[])? {
            Completion::Return(Value::Object(value)) => value,
            Completion::Return(_) => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            }
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let next_key = self.intern_property_key("next")?;
        let next = match self.get_property_in_realm(realm, &iterator, &next_key)? {
            Completion::Return(Value::Object(next)) => {
                let Some(next) = self.as_callable(&next)? else {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                };
                next
            }
            Completion::Return(_) => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            }
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let done_key = self.intern_property_key("done")?;
        let value_key = self.intern_property_key("value")?;
        let maximum = MAX_ARRAY_BUFFER_LENGTH / u64::from(element.byte_length());
        let mut values = Vec::new();
        // Pinned QuickJS collects through js_array_from_iterator, whose fail
        // path releases local values without calling iterator.return. Keep
        // that observable behavior for next/result/value/allocation failures.
        loop {
            let iteration =
                match self.call_internal(realm, &next, Value::Object(iterator.clone()), &[])? {
                    Completion::Return(Value::Object(value)) => value,
                    Completion::Return(_) => {
                        return Ok(NativeConversion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "iterator must return an object",
                        )?));
                    }
                    Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
                };
            let done = match self.get_property_in_realm(realm, &iteration, &done_key)? {
                Completion::Return(value) => value.to_boolean(),
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            if done {
                return Ok(NativeConversion::Value(values));
            }
            if values.len() as u64 == maximum {
                return Ok(NativeConversion::Throw(
                    self.typed_array_invalid_length(realm)?,
                ));
            }
            let value = match self.get_property_in_realm(realm, &iteration, &value_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            if values.try_reserve(1).is_err() {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
            values.push(value);
        }
    }

    pub(in crate::runtime) fn typed_array_is_object(
        &self,
        object: &ObjectRef,
    ) -> Result<bool, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("TypedArray"));
        }
        Ok(matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::TypedArray(_)
        ))
    }

    pub(in crate::runtime) fn typed_array_snapshot_if_branded(
        &self,
        object: &ObjectRef,
    ) -> Result<Option<TypedArraySnapshot>, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("TypedArray"));
        }
        let state = self.0.state.borrow();
        let ObjectPayload::TypedArray(data) = state.heap.object(object.object_id())?.payload else {
            return Ok(None);
        };
        Ok(Some(TypedArraySnapshot {
            buffer: data.view.buffer,
            byte_offset: data.view.byte_offset,
            fixed_byte_length: data.view.fixed_byte_length,
            element: data.element,
        }))
    }

    pub(in crate::runtime) fn typed_array_snapshot(
        &self,
        object: &ObjectRef,
    ) -> Result<TypedArraySnapshot, RuntimeError> {
        self.typed_array_snapshot_if_branded(object)?
            .ok_or(RuntimeError::Invariant(
                "validated TypedArray lost its class payload",
            ))
    }

    pub(in crate::runtime) fn typed_array_state(
        &self,
        object: &ObjectRef,
    ) -> Result<TypedArrayState, RuntimeError> {
        let snapshot = self.typed_array_snapshot(object)?;
        self.typed_array_state_from_snapshot(snapshot)
    }

    pub(in crate::runtime) fn typed_array_state_from_snapshot(
        &self,
        snapshot: TypedArraySnapshot,
    ) -> Result<TypedArrayState, RuntimeError> {
        let buffer = self
            .0
            .state
            .borrow()
            .heap
            .array_buffer_state(snapshot.buffer)?;
        let width = u32::from(snapshot.element.byte_length());
        let byte_length = if buffer.detached || snapshot.byte_offset > buffer.byte_length {
            None
        } else {
            match snapshot.fixed_byte_length {
                Some(length)
                    if snapshot
                        .byte_offset
                        .checked_add(length)
                        .is_none_or(|end| end > buffer.byte_length) =>
                {
                    None
                }
                Some(length) => Some(length),
                None => Some(buffer.byte_length - snapshot.byte_offset),
            }
        };
        let out_of_bounds = byte_length.is_none();
        let byte_length = byte_length.unwrap_or(0);
        // Length-tracking TypedArrays expose only complete elements when an
        // RAB grows to a byte length not divisible by the element width.
        let length = byte_length / width;
        let byte_length = length * width;
        Ok(TypedArrayState {
            snapshot,
            length,
            byte_length,
            out_of_bounds,
            resizable: buffer.max_byte_length.is_some(),
        })
    }

    pub(in crate::runtime) fn typed_array_current_length(
        &self,
        object: &ObjectRef,
    ) -> Result<u32, RuntimeError> {
        Ok(self.typed_array_state(object)?.length)
    }

    pub(in crate::runtime) fn typed_array_validated_length(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<u32>, RuntimeError> {
        let state = self.typed_array_state(object)?;
        if state.out_of_bounds {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached or resized",
            )?));
        }
        Ok(NativeConversion::Value(state.length))
    }

    pub(in crate::runtime) fn typed_array_has_rab_backing(
        &self,
        object: &ObjectRef,
    ) -> Result<bool, RuntimeError> {
        Ok(self.typed_array_state(object)?.resizable)
    }

    pub(in crate::runtime) fn typed_array_canonical_numeric_index(
        &self,
        key: &PropertyKey,
    ) -> Result<Option<CanonicalNumericIndex>, RuntimeError> {
        if self.0.state.borrow().atoms.property_key_kind(key.atom())? != PropertyKeyKind::String {
            return Ok(None);
        }
        let spelling = self.property_key_to_js_string(key)?;
        if spelling == JsString::from_static("-0") {
            return Ok(Some(CanonicalNumericIndex::Invalid));
        }
        let number = Value::String(spelling.clone())
            .to_number()
            .map_err(RuntimeError::Engine)?;
        if spelling != Value::number(number).to_js_string()? {
            return Ok(None);
        }
        if !number.is_finite() || number < 0.0 || number.fract() != 0.0 || number > u64::MAX as f64
        {
            return Ok(Some(CanonicalNumericIndex::Invalid));
        }
        Ok(Some(CanonicalNumericIndex::Valid(number as u64)))
    }

    pub(in crate::runtime) fn typed_array_read_index(
        &self,
        object: &ObjectRef,
        index: u64,
    ) -> Result<Option<Value>, RuntimeError> {
        let state = self.typed_array_state(object)?;
        if state.out_of_bounds || index >= u64::from(state.length) {
            return Ok(None);
        }
        let absolute = typed_array_absolute_byte_offset(state.snapshot, index)?;
        let width = usize::from(state.snapshot.element.byte_length());
        let bytes = self.0.state.borrow().heap.read_array_buffer_word(
            state.snapshot.buffer,
            absolute,
            width,
        )?;
        Ok(Some(typed_array_decode(state.snapshot.element, bytes)))
    }

    pub(in crate::runtime) fn typed_array_get_index_descriptor(
        &self,
        object: &ObjectRef,
        index: u64,
    ) -> Result<Option<CompleteOrdinaryPropertyDescriptor>, RuntimeError> {
        Ok(self.typed_array_read_index(object, index)?.map(|value| {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable: true,
                enumerable: true,
                configurable: true,
            }
        }))
    }

    pub(in crate::runtime) fn typed_array_convert_element(
        &self,
        realm: ContextId,
        element: TypedArrayElementKind,
        value: &Value,
    ) -> Result<NativeConversion<[u8; 8]>, RuntimeError> {
        if element.is_bigint() {
            let bigint = match self.native_to_bigint(realm, value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            return typed_array_encode_bigint(&bigint).map(NativeConversion::Value);
        }
        let number = match self.native_to_number(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        Ok(NativeConversion::Value(typed_array_encode_number(
            element, number,
        )))
    }

    /// Convert a primitive descriptor value for the public context-free
    /// property API. Object conversion stays on the realm-aware Context path.
    pub(in crate::runtime) fn typed_array_convert_primitive_element(
        &self,
        element: TypedArrayElementKind,
        value: &Value,
    ) -> Result<[u8; 8], RuntimeError> {
        if element.is_bigint() {
            let bigint = match value {
                Value::BigInt(value) => value.clone(),
                Value::Bool(value) => crate::bigint::JsBigInt::from(i64::from(*value)),
                Value::String(value) => {
                    let units = value.utf16_units().collect::<Vec<_>>();
                    let source = String::from_utf16(&units).map_err(|_| {
                        RuntimeError::Engine(Error::new(
                            ErrorKind::Syntax,
                            "invalid bigint literal",
                        ))
                    })?;
                    crate::bigint::JsBigInt::parse_js_string(&source).map_err(|error| {
                        let kind = match error {
                            crate::bigint::BigIntError::InvalidSyntax => ErrorKind::Syntax,
                            crate::bigint::BigIntError::InvalidRadix(_)
                            | crate::bigint::BigIntError::BigIntTooLarge
                            | crate::bigint::BigIntError::AllocationTooLarge
                            | crate::bigint::BigIntError::DivisionByZero
                            | crate::bigint::BigIntError::NegativeExponent
                            | crate::bigint::BigIntError::ShiftTooLarge => ErrorKind::Range,
                        };
                        RuntimeError::Engine(Error::new(kind, error.to_string()))
                    })?
                }
                Value::Undefined
                | Value::Null
                | Value::Int(_)
                | Value::Float(_)
                | Value::Symbol(_)
                | Value::Object(_) => {
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Type,
                        "cannot convert to bigint",
                    )));
                }
            };
            return typed_array_encode_bigint(&bigint);
        }
        let number = value.to_number().map_err(RuntimeError::Engine)?;
        Ok(typed_array_encode_number(element, number))
    }

    pub(in crate::runtime) fn typed_array_write_converted_index(
        &self,
        object: &ObjectRef,
        index: u64,
        bytes: &[u8; 8],
    ) -> Result<bool, RuntimeError> {
        let state = self.typed_array_state(object)?;
        if state.out_of_bounds || index >= u64::from(state.length) {
            return Ok(false);
        }
        let absolute = typed_array_absolute_byte_offset(state.snapshot, index)?;
        let width = usize::from(state.snapshot.element.byte_length());
        self.0.state.borrow_mut().heap.write_array_buffer_word(
            state.snapshot.buffer,
            absolute,
            &bytes[..width],
        )?;
        Ok(true)
    }

    pub(in crate::runtime) fn typed_array_set_index(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u64,
        value: &Value,
    ) -> Result<NativeConversion<()>, RuntimeError> {
        let element = self.typed_array_snapshot(object)?.element;
        let bytes = match self.typed_array_convert_element(realm, element, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let _ = self.typed_array_write_converted_index(object, index, &bytes)?;
        Ok(NativeConversion::Value(()))
    }

    pub(in crate::runtime) fn typed_array_define_index(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u64,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        if descriptor.get.is_present()
            || descriptor.set.is_present()
            || matches!(descriptor.writable, DescriptorField::Present(false))
            || matches!(descriptor.enumerable, DescriptorField::Present(false))
            || matches!(descriptor.configurable, DescriptorField::Present(false))
        {
            return Ok(NativeConversion::Value(false));
        }
        let state = self.typed_array_state(object)?;
        if state.out_of_bounds || index >= u64::from(state.length) {
            return Ok(NativeConversion::Value(false));
        }
        if let DescriptorField::Present(value) = &descriptor.value {
            let bytes =
                match self.typed_array_convert_element(realm, state.snapshot.element, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                };
            let _ = self.typed_array_write_converted_index(object, index, &bytes)?;
        }
        Ok(NativeConversion::Value(true))
    }

    pub(in crate::runtime) fn typed_array_delete_index(
        &self,
        object: &ObjectRef,
        index: u64,
    ) -> Result<bool, RuntimeError> {
        Ok(self.typed_array_read_index(object, index)?.is_none())
    }
}

fn typed_array_absolute_byte_offset(
    snapshot: TypedArraySnapshot,
    index: u64,
) -> Result<usize, RuntimeError> {
    let relative = index
        .checked_mul(u64::from(snapshot.element.byte_length()))
        .ok_or(RuntimeError::Invariant(
            "TypedArray relative byte offset overflowed u64",
        ))?;
    let absolute = u64::from(snapshot.byte_offset)
        .checked_add(relative)
        .ok_or(RuntimeError::Invariant(
            "TypedArray absolute byte offset overflowed u64",
        ))?;
    usize::try_from(absolute)
        .map_err(|_| RuntimeError::Invariant("TypedArray byte offset overflowed usize"))
}

const fn typed_array_length_is_supported(element: TypedArrayElementKind, length: u64) -> bool {
    length <= MAX_ARRAY_BUFFER_LENGTH / element.byte_length() as u64
}

fn typed_array_u32_value(value: u32) -> Value {
    i32::try_from(value).map_or_else(|_| Value::number(f64::from(value)), Value::Int)
}

fn typed_array_encode_bigint(bigint: &crate::bigint::JsBigInt) -> Result<[u8; 8], RuntimeError> {
    let narrowed = bigint
        .as_int_n(64)
        .map_err(|_| RuntimeError::Invariant("64-bit TypedArray BigInt conversion failed"))?;
    let signed = narrowed.as_i64().ok_or(RuntimeError::Invariant(
        "64-bit TypedArray BigInt did not normalize to an i64",
    ))?;
    Ok((signed as u64).to_ne_bytes())
}

fn typed_array_encode_number(element: TypedArrayElementKind, number: f64) -> [u8; 8] {
    let mut bytes = [0_u8; 8];
    match element {
        TypedArrayElementKind::Float16 => {
            bytes[..2].copy_from_slice(&crate::number::to_float16_bits(number).to_ne_bytes());
        }
        TypedArrayElementKind::Float32 => {
            bytes[..4].copy_from_slice(&(number as f32).to_bits().to_ne_bytes());
        }
        TypedArrayElementKind::Float64 => bytes = number.to_bits().to_ne_bytes(),
        TypedArrayElementKind::Uint8Clamped => bytes[0] = typed_array_to_uint8_clamp(number),
        TypedArrayElementKind::Int8
        | TypedArrayElementKind::Uint8
        | TypedArrayElementKind::Int16
        | TypedArrayElementKind::Uint16
        | TypedArrayElementKind::Int32
        | TypedArrayElementKind::Uint32 => {
            let integer = Runtime::to_uint32_number(number);
            match element.byte_length() {
                1 => bytes[0] = integer as u8,
                2 => bytes[..2].copy_from_slice(&(integer as u16).to_ne_bytes()),
                4 => bytes[..4].copy_from_slice(&integer.to_ne_bytes()),
                _ => unreachable!("integer TypedArray width is 1, 2, or 4"),
            }
        }
        TypedArrayElementKind::BigInt64 | TypedArrayElementKind::BigUint64 => {
            unreachable!("BigInt TypedArray values use the BigInt encoder")
        }
    }
    bytes
}

fn typed_array_decode(element: TypedArrayElementKind, bytes: [u8; 8]) -> Value {
    match element {
        TypedArrayElementKind::Uint8Clamped | TypedArrayElementKind::Uint8 => {
            Value::Int(i32::from(bytes[0]))
        }
        TypedArrayElementKind::Int8 => Value::Int(i32::from(bytes[0] as i8)),
        TypedArrayElementKind::Int16 => {
            Value::Int(i32::from(i16::from_ne_bytes([bytes[0], bytes[1]])))
        }
        TypedArrayElementKind::Uint16 => {
            Value::Int(i32::from(u16::from_ne_bytes([bytes[0], bytes[1]])))
        }
        TypedArrayElementKind::Int32 => {
            Value::Int(i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        TypedArrayElementKind::Uint32 => Runtime::array_length_value(u32::from_ne_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ])),
        TypedArrayElementKind::BigInt64 => {
            Value::BigInt(crate::bigint::JsBigInt::from(i64::from_ne_bytes(bytes)))
        }
        TypedArrayElementKind::BigUint64 => {
            Value::BigInt(crate::bigint::JsBigInt::from(u64::from_ne_bytes(bytes)))
        }
        TypedArrayElementKind::Float16 => {
            Value::number(crate::number::from_float16_bits(u16::from_ne_bytes([
                bytes[0], bytes[1],
            ])))
        }
        TypedArrayElementKind::Float32 => {
            Value::number(f64::from(f32::from_bits(u32::from_ne_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ]))))
        }
        TypedArrayElementKind::Float64 => Value::number(f64::from_bits(u64::from_ne_bytes(bytes))),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn typed_array_to_uint8_clamp(number: f64) -> u8 {
    if number.is_nan() || number <= 0.0 {
        return 0;
    }
    if number >= 255.0 {
        return 255;
    }
    let floor = number.floor();
    let midpoint = floor + 0.5;
    if number > midpoint || (number == midpoint && (floor as u64) % 2 == 1) {
        (floor as u8) + 1
    } else {
        floor as u8
    }
}
