//! The twelve concrete TypedArray classes over the shared ArrayBuffer store.
//!
//! Pinned QuickJS implements these classes through one fast-array kernel.  The
//! Rust representation follows that layout: every object stores a byte view
//! and one element selector, while detach and resizable-buffer bounds are
//! derived from the backing store for every observable operation.

use crate::engine::atom::PropertyKeyKind;
use crate::engine::builtins::native::{
    ArrayFindKind, ArrayIterationKind, ArrayIteratorKind, ArrayJoinKind, ArrayReduceKind,
    ArraySearchKind, TypedArrayElementKind, TypedArrayNativeKind, Uint8ArrayCodecKind,
};
use crate::engine::heap::{
    ArrayBufferViewData, ObjectData, ObjectPayload, TypedArrayData, TypedArrayRealmData,
};

use crate::engine::object::builtin_properties::NativeBuiltinProperty;

use super::MAX_ARRAY_BUFFER_LENGTH;
#[cfg(test)]
use crate::engine::api::context::Context;
use crate::engine::{
    api::{
        error::{Error, ErrorKind, NativeErrorKind},
        runtime::Runtime,
        runtime_error::RuntimeError,
    },
    builtins::native::NativeFunctionId,
    heap::{ContextId, ObjectId},
    object::{
        AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef,
        OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
    },
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};

mod collect;
#[cfg(feature = "stack-vm")]
pub(crate) use collect::{
    TypedCollectResume, TypedCollectStep, TypedIteratorMethodResume, TypedIteratorMethodStep,
};

mod create;
#[cfg(feature = "stack-vm")]
pub(crate) use create::{TypedCreateResume, TypedCreateStep};
mod copying;
#[cfg(feature = "stack-vm")]
pub(crate) use copying::{TypedWithResume, TypedWithStep};
pub(crate) mod element;
mod find;
mod iteration;
#[cfg(feature = "stack-vm")]
pub(crate) use iteration::{TypedIterationResume, TypedIterationStep};
mod mutation;
#[cfg(feature = "stack-vm")]
pub(crate) use mutation::{TypedMutationKind, TypedMutationResume, TypedMutationStep};
mod reduce;
mod traversal;
#[cfg(feature = "stack-vm")]
pub(crate) use traversal::{TypedTraversalKind, TypedTraversalResume, TypedTraversalStep};
mod search;
#[cfg(feature = "stack-vm")]
pub(crate) use search::{TypedSearchKind, TypedSearchResume, TypedSearchStep};
mod set;
#[cfg(feature = "stack-vm")]
pub(crate) use set::{TypedSetResume, TypedSetStep};
mod slice;
#[cfg(feature = "stack-vm")]
pub(crate) use slice::{TypedSliceKind, TypedSliceResume, TypedSliceStep};
mod sort;
mod species;
#[cfg(feature = "stack-vm")]
pub(crate) use species::{TypedSpeciesResume, TypedSpeciesStep};
mod stringification;
#[cfg(feature = "stack-vm")]
pub(crate) use stringification::{TypedStringResume, TypedStringStep};
#[cfg(test)]
mod tests;
mod uint8_codec;
pub(crate) mod write;

/// Classification of an ECMAScript CanonicalNumericIndexString.
///
/// `Invalid` is still a canonical numeric key, but it can never denote a
/// TypedArray element (`-0`, negative/fractional values, NaN, or infinities).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CanonicalNumericIndex {
    Valid(u64),
    Invalid,
}

/// Durable TypedArray metadata copied out of the heap before observable work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TypedArraySnapshot {
    pub buffer: ObjectId,
    pub byte_offset: u32,
    pub fixed_byte_length: Option<u32>,
    pub element: TypedArrayElementKind,
}

/// Current view state derived from a snapshot and its ArrayBuffer-family backing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TypedArrayState {
    pub snapshot: TypedArraySnapshot,
    pub length: u32,
    pub byte_length: u32,
    pub out_of_bounds: bool,
    pub resizable: bool,
}

/// Bounded, borrow-free element snapshot for the qjs diagnostic printer.
/// Indexed values are copied while the ArrayBuffer-family access token is
/// live, so the printer never holds `RuntimeState` across an SAB lock or
/// recursively re-snapshots a resizable view element by element.
pub(crate) struct TypedArrayPrintSnapshot {
    pub element: TypedArrayElementKind,
    pub length: u32,
    pub values: Vec<Value>,
}

impl Runtime {
    /// Install the hidden `%TypedArray%` constructor/prototype pair and the
    /// twelve public concrete classes in QuickJS class-id order.
    pub(crate) fn initialize_typed_array_intrinsics(
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
        // Publish before the next getter group to preserve own-key order.
        let methods = [
            NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::At),
                "at",
                1,
                1,
            ),
            NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::With),
                "with",
                2,
                2,
            ),
        ];
        self.define_native_builtin_auto_init_batch(&base_prototype, realm, methods)?;

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
        let mut methods = vec![NativeBuiltinProperty::new(
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Set),
            "set",
            1,
            2,
        )];
        for (kind, name) in [
            (ArrayIteratorKind::Value, "values"),
            (ArrayIteratorKind::Key, "keys"),
            (ArrayIteratorKind::KeyAndValue, "entries"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Iterator(kind)),
                name,
                0,
                0,
            ));
        }
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::TypedArray(TypedArrayNativeKind::CopyWithin),
            "copyWithin",
            2,
            2,
        ));
        for (kind, name) in [
            (ArrayIterationKind::Every, "every"),
            (ArrayIterationKind::Some, "some"),
            (ArrayIterationKind::ForEach, "forEach"),
            (ArrayIterationKind::Map, "map"),
            (ArrayIterationKind::Filter, "filter"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Iteration(kind)),
                name,
                1,
                1,
            ));
        }
        for (kind, name) in [
            (ArrayReduceKind::Reduce, "reduce"),
            (ArrayReduceKind::ReduceRight, "reduceRight"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Reduce(kind)),
                name,
                1,
                1,
            ));
        }
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Fill),
            "fill",
            1,
            1,
        ));
        for (kind, name) in [
            (ArrayFindKind::Find, "find"),
            (ArrayFindKind::FindIndex, "findIndex"),
            (ArrayFindKind::FindLast, "findLast"),
            (ArrayFindKind::FindLastIndex, "findLastIndex"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Find(kind)),
                name,
                1,
                1,
            ));
        }
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Reverse),
            "reverse",
            0,
            0,
        ));
        methods.push(NativeBuiltinProperty::new(
            NativeFunctionId::TypedArray(TypedArrayNativeKind::ToReversed),
            "toReversed",
            0,
            0,
        ));
        for (kind, name) in [
            (TypedArrayNativeKind::Slice, "slice"),
            (TypedArrayNativeKind::Subarray, "subarray"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(kind),
                name,
                2,
                2,
            ));
        }
        for (kind, name) in [
            (TypedArrayNativeKind::Sort, "sort"),
            (TypedArrayNativeKind::ToSorted, "toSorted"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(kind),
                name,
                1,
                1,
            ));
        }
        for (kind, name, length) in [
            (ArrayJoinKind::Join, "join", 1),
            (ArrayJoinKind::ToLocaleString, "toLocaleString", 0),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Join(kind)),
                name,
                length,
                length,
            ));
        }
        for (kind, name) in [
            (ArraySearchKind::IndexOf, "indexOf"),
            (ArraySearchKind::LastIndexOf, "lastIndexOf"),
            (ArraySearchKind::Includes, "includes"),
        ] {
            methods.push(NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Search(kind)),
                name,
                1,
                1,
            ));
        }

        self.define_native_builtin_auto_init_batch(&base_prototype, realm, methods)?;

        let base_constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::BaseConstructor),
            0,
            "TypedArray",
            0,
        )?;
        let methods = [
            NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::From),
                "from",
                1,
                3,
            ),
            NativeBuiltinProperty::new(
                NativeFunctionId::TypedArray(TypedArrayNativeKind::Of),
                "of",
                0,
                0,
            ),
        ];
        self.define_native_builtin_auto_init_batch(base_constructor.as_object(), realm, methods)?;

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
            if element == TypedArrayElementKind::Uint8 {
                let mut methods = Vec::new();
                for (kind, name, length, min_readable_args) in [
                    (Uint8ArrayCodecKind::ToBase64, "toBase64", 0, 1),
                    (Uint8ArrayCodecKind::ToHex, "toHex", 0, 0),
                    (Uint8ArrayCodecKind::SetFromBase64, "setFromBase64", 1, 2),
                    (Uint8ArrayCodecKind::SetFromHex, "setFromHex", 1, 1),
                ] {
                    methods.push(NativeBuiltinProperty::new(
                        NativeFunctionId::TypedArray(TypedArrayNativeKind::Uint8Codec(kind)),
                        name,
                        length,
                        min_readable_args,
                    ));
                }
                self.define_native_builtin_auto_init_batch(&prototype, realm, methods)?;
            }
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
            if element == TypedArrayElementKind::Uint8 {
                let mut methods = Vec::new();
                for (kind, name, min_readable_args) in [
                    (Uint8ArrayCodecKind::FromBase64, "fromBase64", 2),
                    (Uint8ArrayCodecKind::FromHex, "fromHex", 1),
                ] {
                    methods.push(NativeBuiltinProperty::new(
                        NativeFunctionId::TypedArray(TypedArrayNativeKind::Uint8Codec(kind)),
                        name,
                        1,
                        min_readable_args,
                    ));
                }
                self.define_native_builtin_auto_init_batch(
                    constructor.as_object(),
                    realm,
                    methods,
                )?;
            }
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

    pub(crate) fn call_typed_array_native(
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
            TypedArrayNativeKind::Uint8Codec(kind) => {
                self.call_uint8_array_codec(realm, kind, invocation, arguments)
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
            TypedArrayNativeKind::Reduce(kind) => {
                self.call_typed_array_reduce(realm, kind, invocation, arguments)
            }
            TypedArrayNativeKind::Fill => self.call_typed_array_fill(realm, invocation, arguments),
            TypedArrayNativeKind::Reverse => self.call_typed_array_reverse(realm, invocation),
            TypedArrayNativeKind::At => self.call_typed_array_at(realm, invocation, arguments),
            TypedArrayNativeKind::With => self.call_typed_array_with(realm, invocation, arguments),
            TypedArrayNativeKind::ToReversed => {
                self.call_typed_array_to_reversed(realm, invocation)
            }
            TypedArrayNativeKind::Search(kind) => {
                self.call_typed_array_search(realm, kind, invocation, arguments)
            }
            TypedArrayNativeKind::Find(kind) => {
                self.call_typed_array_find(realm, kind, invocation, arguments)
            }
            TypedArrayNativeKind::Slice => {
                self.call_typed_array_slice(realm, invocation, arguments)
            }
            TypedArrayNativeKind::Subarray => {
                self.call_typed_array_subarray(realm, invocation, arguments)
            }
            TypedArrayNativeKind::Sort => self.call_typed_array_sort(realm, invocation, arguments),
            TypedArrayNativeKind::ToSorted => {
                self.call_typed_array_to_sorted(realm, invocation, arguments)
            }
            TypedArrayNativeKind::Join(kind) => {
                self.call_typed_array_join(realm, kind, invocation, arguments)
            }
        }
    }

    fn call_typed_array_constructor(
        &self,
        realm: ContextId,
        element: TypedArrayElementKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        create::finish(
            self,
            realm,
            create::TypedCreateStep::constructor(self, realm, element, &invocation, arguments)?,
        )
    }

    /// Finish the constructor's ArrayBuffer-family overload after prototype,
    /// byteOffset, and length coercion have completed. The access token owns
    /// all state needed to validate either backing class without carrying a
    /// runtime borrow into a shared-backing mutex operation.
    fn new_typed_array_constructor_view_from_coerced(
        &self,
        realm: ContextId,
        prototype: &ObjectRef,
        element: TypedArrayElementKind,
        buffer: &ObjectRef,
        byte_offset: u64,
        requested_length: Option<u64>,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let width = u64::from(element.byte_length());
        if byte_offset % width != 0 {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid offset",
            )?));
        }

        let backing = self.snapshot_buffer_access(buffer.object_id())?.state;
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

        Ok(NativeConversion::Value(self.new_typed_array_object(
            prototype,
            buffer,
            byte_offset,
            fixed_byte_length,
            element,
        )?))
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
        create::finish(
            self,
            realm,
            create::TypedCreateStep::from(self, realm, &invocation, arguments)?,
        )
    }

    fn call_typed_array_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        create::finish(
            self,
            realm,
            create::TypedCreateStep::of(self, realm, &invocation, arguments)?,
        )
    }

    fn call_typed_array_set(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        set::finish(
            self,
            realm,
            set::TypedSetStep::start(self, realm, &invocation, arguments)?,
        )
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
            // QuickJS uses memmove here so an overlapping same-backing source
            // is cached. The family-neutral leaf preserves that rule for AB,
            // SAB, and distinct SAB wrappers sharing one backing store.
            let source_access = self.snapshot_buffer_access(source_snapshot.buffer)?;
            let target_access = self.snapshot_buffer_access(target_snapshot.buffer)?;
            self.move_buffer_range(
                &source_access,
                &target_access,
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
        self.new_native_error(realm, NativeErrorKind::Range, "invalid array buffer length")
    }

    fn typed_array_iterator_method(
        &self,
        realm: ContextId,
        source: Value,
    ) -> Result<NativeConversion<Option<CallableRef>>, RuntimeError> {
        collect::finish_method(
            self,
            realm,
            collect::TypedIteratorMethodStep::start(self, realm, source)?,
        )
    }

    fn collect_typed_array_iterator(
        &self,
        realm: ContextId,
        source: Value,
        method: &CallableRef,
        element: TypedArrayElementKind,
    ) -> Result<NativeConversion<Vec<Value>>, RuntimeError> {
        collect::finish_collect(
            self,
            realm,
            collect::TypedCollectStep::start(realm, source, method.clone(), element),
        )
    }

    pub(crate) fn typed_array_is_object(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
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

    pub(crate) fn typed_array_snapshot_if_branded(
        &self,
        object: &ObjectRef,
    ) -> Result<Option<TypedArraySnapshot>, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("TypedArray"));
        }
        let state = self.0.state.borrow();
        Ok(typed_array_snapshot_from_payload(
            &state.heap.object(object.object_id())?.payload,
        ))
    }

    pub(crate) fn typed_array_snapshot(
        &self,
        object: &ObjectRef,
    ) -> Result<TypedArraySnapshot, RuntimeError> {
        self.typed_array_snapshot_if_branded(object)?
            .ok_or(RuntimeError::Invariant(
                "validated TypedArray lost its class payload",
            ))
    }

    pub(crate) fn typed_array_state(
        &self,
        object: &ObjectRef,
    ) -> Result<TypedArrayState, RuntimeError> {
        let snapshot = self.typed_array_snapshot(object)?;
        self.typed_array_state_from_snapshot(snapshot)
    }

    pub(crate) fn typed_array_state_from_snapshot(
        &self,
        snapshot: TypedArraySnapshot,
    ) -> Result<TypedArrayState, RuntimeError> {
        let buffer = self.snapshot_buffer_access(snapshot.buffer)?.state;
        Ok(Self::typed_array_state_with_buffer(snapshot, buffer))
    }

    // Pure bounds calculation, shared with element access. A caller holding
    // an access token must not perform observable work before consuming it.
    fn typed_array_state_with_buffer(
        snapshot: TypedArraySnapshot,
        buffer: crate::engine::heap::ArrayBufferState,
    ) -> TypedArrayState {
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
        TypedArrayState {
            snapshot,
            length,
            byte_length,
            out_of_bounds,
            resizable: buffer.max_byte_length.is_some(),
        }
    }

    pub(crate) fn typed_array_current_length(
        &self,
        object: &ObjectRef,
    ) -> Result<u32, RuntimeError> {
        Ok(self.typed_array_state(object)?.length)
    }

    pub(crate) fn qjs_typed_array_print_snapshot(
        &self,
        object: ObjectId,
        max_items: u32,
    ) -> Result<TypedArrayPrintSnapshot, RuntimeError> {
        let snapshot = {
            let state = self.0.state.borrow();
            let ObjectPayload::TypedArray(data) = &state.heap.object(object)?.payload else {
                return Err(RuntimeError::Invariant(
                    "qjs TypedArray printer reached another object class",
                ));
            };
            TypedArraySnapshot {
                buffer: data.view.buffer,
                byte_offset: data.view.byte_offset,
                fixed_byte_length: data.view.fixed_byte_length,
                element: data.element,
            }
        };
        let view = self.typed_array_state_from_snapshot(snapshot)?;
        if view.out_of_bounds || view.length == 0 || max_items == 0 {
            return Ok(TypedArrayPrintSnapshot {
                element: snapshot.element,
                length: view.length,
                values: Vec::new(),
            });
        }

        let count = view.length.min(max_items);
        let width = usize::from(snapshot.element.byte_length());
        let byte_offset = usize::try_from(snapshot.byte_offset)
            .map_err(|_| RuntimeError::Invariant("qjs TypedArray byte offset overflowed usize"))?;
        let byte_length = usize::try_from(count)
            .ok()
            .and_then(|count| count.checked_mul(width))
            .ok_or(RuntimeError::Invariant(
                "qjs TypedArray print range overflowed usize",
            ))?;
        let access = self.snapshot_buffer_access(snapshot.buffer)?;
        let values = self.with_buffer_range(&access, byte_offset, byte_length, |bytes| {
            let mut values = Vec::with_capacity(count as usize);
            for bytes in bytes.chunks_exact(width) {
                let mut word = [0_u8; 8];
                word[..width].copy_from_slice(bytes);
                values.push(typed_array_decode(snapshot.element, word));
            }
            values
        })?;
        Ok(TypedArrayPrintSnapshot {
            element: snapshot.element,
            length: view.length,
            values,
        })
    }

    pub(crate) fn typed_array_validated_length(
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

    /// Match QuickJS's TypedArray-specific `JS_PreventExtensions` guard.
    ///
    /// Every length-tracking view is rejected. Fixed-length views are also
    /// rejected over ordinary resizable ArrayBuffers because those buffers can
    /// shrink and later regrow, but they are safe to make non-extensible over
    /// growable SharedArrayBuffers, which cannot shrink.
    pub(crate) fn typed_array_prevent_extensions_is_rejected(
        &self,
        object: &ObjectRef,
    ) -> Result<bool, RuntimeError> {
        let snapshot = self.typed_array_snapshot(object)?;
        if snapshot.fixed_byte_length.is_none() {
            return Ok(true);
        }

        let state = self.0.state.borrow();
        match &state.heap.object(snapshot.buffer)?.payload {
            ObjectPayload::ArrayBuffer(data) => Ok(data.max_byte_length.is_some()),
            ObjectPayload::SharedArrayBuffer(_) => Ok(false),
            _ => Err(RuntimeError::Invariant(
                "TypedArray backing object is not an ArrayBuffer or SharedArrayBuffer",
            )),
        }
    }

    pub(crate) fn typed_array_canonical_numeric_index(
        &self,
        key: &PropertyKey,
    ) -> Result<Option<CanonicalNumericIndex>, RuntimeError> {
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("property key"));
        }
        // Immediate atoms are canonical nonnegative integer strings. Numeric
        // -0 has already become the zero atom; the string "-0" falls through.
        if let Some(index) = key.atom().immediate_integer() {
            return Ok(Some(CanonicalNumericIndex::Valid(u64::from(index))));
        }
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

    pub(crate) fn typed_array_read_index(
        &self,
        object: &ObjectRef,
        index: u64,
    ) -> Result<Option<Value>, RuntimeError> {
        let snapshot = self.typed_array_snapshot(object)?;
        #[cfg(feature = "stack-vm")]
        match self.ordinary_typed_array_word(snapshot, index, None)? {
            OrdinaryTypedWord::Missing => return Ok(None),
            OrdinaryTypedWord::Word(bytes) => {
                return Ok(Some(typed_array_decode(snapshot.element, bytes)));
            }
            OrdinaryTypedWord::Shared => {}
        }
        let access = self.snapshot_buffer_access(snapshot.buffer)?;
        let Some((absolute, width)) = typed_array_word_range(snapshot, access.state, index)? else {
            return Ok(None);
        };
        let bytes = self.read_buffer_word(&access, absolute, width)?;
        Ok(Some(typed_array_decode(snapshot.element, bytes)))
    }

    pub(crate) fn typed_array_get_index_descriptor(
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

    pub(crate) fn typed_array_convert_element(
        &self,
        realm: ContextId,
        element: TypedArrayElementKind,
        value: &Value,
    ) -> Result<NativeConversion<[u8; 8]>, RuntimeError> {
        element::ElementStep::start(self, realm, element, value.clone())?.finish_sync(self, realm)
    }

    /// Convert a primitive descriptor value for the public context-free
    /// property API. Object conversion stays on the realm-aware Context path.
    pub(crate) fn typed_array_convert_primitive_element(
        &self,
        element: TypedArrayElementKind,
        value: &Value,
    ) -> Result<[u8; 8], RuntimeError> {
        if element.is_bigint() {
            let bigint = match value {
                Value::BigInt(value) => value.clone(),
                Value::Bool(value) => {
                    crate::engine::value::bigint::JsBigInt::from(i64::from(*value))
                }
                Value::String(value) => {
                    let units = value.utf16_units().collect::<Vec<_>>();
                    let source = String::from_utf16(&units).map_err(|_| {
                        RuntimeError::Engine(Error::new(
                            ErrorKind::Syntax,
                            "invalid bigint literal",
                        ))
                    })?;
                    crate::engine::value::bigint::JsBigInt::parse_js_string(&source).map_err(
                        |error| {
                            let kind = match error {
                                crate::engine::value::bigint::BigIntError::InvalidSyntax => {
                                    ErrorKind::Syntax
                                }
                                crate::engine::value::bigint::BigIntError::InvalidRadix(_)
                                | crate::engine::value::bigint::BigIntError::BigIntTooLarge
                                | crate::engine::value::bigint::BigIntError::AllocationTooLarge
                                | crate::engine::value::bigint::BigIntError::DivisionByZero
                                | crate::engine::value::bigint::BigIntError::NegativeExponent
                                | crate::engine::value::bigint::BigIntError::ShiftTooLarge => {
                                    ErrorKind::Range
                                }
                            };
                            RuntimeError::Engine(Error::new(kind, error.to_string()))
                        },
                    )?
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

    pub(crate) fn typed_array_write_converted_index(
        &self,
        object: &ObjectRef,
        index: u64,
        bytes: &[u8; 8],
    ) -> Result<bool, RuntimeError> {
        let snapshot = self.typed_array_snapshot(object)?;
        #[cfg(feature = "stack-vm")]
        match self.ordinary_typed_array_word(snapshot, index, Some(bytes))? {
            OrdinaryTypedWord::Missing => return Ok(false),
            OrdinaryTypedWord::Word(_) => return Ok(true),
            OrdinaryTypedWord::Shared => {}
        }
        let access = self.snapshot_buffer_access(snapshot.buffer)?;
        let Some((absolute, width)) = typed_array_word_range(snapshot, access.state, index)? else {
            return Ok(false);
        };
        self.write_buffer_word(&access, absolute, &bytes[..width])?;
        Ok(true)
    }

    /// A rooted view owns its ordinary backing throughout this synchronous
    /// leaf. No token/root is needed when validation and word access consume
    /// the same state borrow. Conversion has already completed; this helper
    /// never calls user code, allocates a JS value, or releases an owner.
    /// Shared backing must leave the borrow before obtaining its access token.
    #[cfg(feature = "stack-vm")]
    fn ordinary_typed_array_word(
        &self,
        snapshot: TypedArraySnapshot,
        index: u64,
        write: Option<&[u8; 8]>,
    ) -> Result<OrdinaryTypedWord, RuntimeError> {
        let mut state = self.0.state.try_borrow_mut().map_err(|_| {
            RuntimeError::Invariant(
                "ArrayBuffer-family snapshot attempted during a runtime-state borrow",
            )
        })?;
        ordinary_typed_array_word_in_heap(&mut state.heap, snapshot, index, write)
    }

    /// Scoped numeric read shared by the resident indexed-read selector. The
    /// caller proves no-drain input release before taking this heap borrow.
    /// Decode allocates no owner because BigInt kinds are declined first.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn typed_array_number_read_in_heap(
        heap: &mut crate::engine::heap::Heap,
        object: ObjectId,
        index: u32,
    ) -> Option<Value> {
        let data = heap.object(object).ok()?;
        let snapshot = typed_array_snapshot_from_payload(&data.payload)?;
        if snapshot.element.is_bigint() {
            return None;
        }
        let OrdinaryTypedWord::Word(bytes) =
            ordinary_typed_array_word_in_heap(heap, snapshot, u64::from(index), None).ok()?
        else {
            return None;
        };
        Some(typed_array_decode(snapshot.element, bytes))
    }

    /// Resident VM leaf: every decline precedes the only byte write. The
    /// owning input may be dropped after success without running heap cleanup.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn try_typed_array_number_write(
        &self,
        base: &Value,
        index: u32,
        number: f64,
    ) -> bool {
        use crate::engine::heap::SlotReleaseReadiness;
        let Value::Object(object) = base else {
            return false;
        };
        if !matches!(
            self.slot_value_release_readiness(base),
            Ok(SlotReleaseReadiness::Ready)
        ) {
            return false;
        }
        let Ok(mut state) = self.0.state.try_borrow_mut() else {
            return false;
        };
        let Ok(data) = state.heap.object(object.object_id()) else {
            return false;
        };
        let Some(snapshot) = typed_array_snapshot_from_payload(&data.payload) else {
            return false;
        };
        if snapshot.element.is_bigint() {
            return false;
        }
        let bytes = typed_array_encode_number(snapshot.element, number);
        matches!(
            ordinary_typed_array_word_in_heap(
                &mut state.heap,
                snapshot,
                u64::from(index),
                Some(&bytes)
            ),
            Ok(OrdinaryTypedWord::Word(_))
        )
    }

    pub(crate) fn typed_array_set_index(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u64,
        value: &Value,
    ) -> Result<NativeConversion<()>, RuntimeError> {
        Ok(
            match write::TypedWriteStep::set(self, object.clone(), Some(index), value.clone())?
                .finish_sync(self, realm)?
            {
                NativeConversion::Value(_) => NativeConversion::Value(()),
                NativeConversion::Throw(value) => NativeConversion::Throw(value),
            },
        )
    }

    pub(crate) fn typed_array_define_index(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        index: u64,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        write::TypedWriteStep::define(self, object.clone(), index, descriptor)?
            .finish_sync(self, realm)
    }

    pub(crate) fn typed_array_delete_index(
        &self,
        object: &ObjectRef,
        index: u64,
    ) -> Result<bool, RuntimeError> {
        Ok(self.typed_array_read_index(object, index)?.is_none())
    }
}

fn typed_array_snapshot_from_payload(payload: &ObjectPayload) -> Option<TypedArraySnapshot> {
    let ObjectPayload::TypedArray(data) = payload else {
        return None;
    };
    Some(TypedArraySnapshot {
        buffer: data.view.buffer,
        byte_offset: data.view.byte_offset,
        fixed_byte_length: data.view.fixed_byte_length,
        element: data.element,
    })
}

#[cfg(feature = "stack-vm")]
fn ordinary_typed_array_word_in_heap(
    heap: &mut crate::engine::heap::Heap,
    snapshot: TypedArraySnapshot,
    index: u64,
    write: Option<&[u8; 8]>,
) -> Result<OrdinaryTypedWord, RuntimeError> {
    match heap.object(snapshot.buffer)?.kind {
        crate::engine::heap::ObjectKind::SharedArrayBuffer => {
            return Ok(OrdinaryTypedWord::Shared);
        }
        crate::engine::heap::ObjectKind::ArrayBuffer => {}
        _ => {
            return Err(RuntimeError::Invariant(
                "ArrayBuffer-family access reached another object class",
            ));
        }
    }
    let buffer = heap.buffer_state(snapshot.buffer)?;
    let Some((absolute, width)) = typed_array_word_range(snapshot, buffer, index)? else {
        return Ok(OrdinaryTypedWord::Missing);
    };
    let bytes = if let Some(bytes) = write {
        heap.write_array_buffer_word(snapshot.buffer, absolute, &bytes[..width])?;
        [0; 8]
    } else {
        heap.read_array_buffer_word(snapshot.buffer, absolute, width)?
    };
    Ok(OrdinaryTypedWord::Word(bytes))
}

#[cfg(feature = "stack-vm")]
enum OrdinaryTypedWord {
    Shared,
    Missing,
    Word([u8; 8]),
}

// One range calculation for rooted-token and scoped ordinary word access.
// Bounds precede offset arithmetic, including detached and resized views.
fn typed_array_word_range(
    snapshot: TypedArraySnapshot,
    buffer: crate::engine::heap::ArrayBufferState,
    index: u64,
) -> Result<Option<(usize, usize)>, RuntimeError> {
    let state = Runtime::typed_array_state_with_buffer(snapshot, buffer);
    if state.out_of_bounds || index >= u64::from(state.length) {
        return Ok(None);
    }
    Ok(Some((
        typed_array_absolute_byte_offset(snapshot, index)?,
        usize::from(snapshot.element.byte_length()),
    )))
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

fn typed_array_encode_bigint(
    bigint: &crate::engine::value::bigint::JsBigInt,
) -> Result<[u8; 8], RuntimeError> {
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
            bytes[..2].copy_from_slice(
                &crate::engine::value::number::to_float16_bits(number).to_ne_bytes(),
            );
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
        TypedArrayElementKind::BigInt64 => Value::BigInt(
            crate::engine::value::bigint::JsBigInt::from(i64::from_ne_bytes(bytes)),
        ),
        TypedArrayElementKind::BigUint64 => Value::BigInt(
            crate::engine::value::bigint::JsBigInt::from(u64::from_ne_bytes(bytes)),
        ),
        TypedArrayElementKind::Float16 => {
            Value::number(crate::engine::value::number::from_float16_bits(
                u16::from_ne_bytes([bytes[0], bytes[1]]),
            ))
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

#[cfg(feature = "stack-vm")]
pub(crate) use sort::{TypedSortResume, TypedSortStep};

#[cfg(feature = "stack-vm")]
pub(crate) use uint8_codec::{Uint8CodecResume, Uint8CodecStep};
