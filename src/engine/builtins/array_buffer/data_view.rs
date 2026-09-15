//! `%DataView%` over the ArrayBuffer-family backing stores.
//!
//! QuickJS uses the same `JSTypedArray` descriptor for DataView and concrete
//! TypedArrays, but DataView itself is an ordinary (non integer-indexed
//! exotic) object.  Oxide keeps that split explicit: the payload owns one
//! traced ArrayBuffer-family edge and stores only byte offset plus
//! fixed/tracking length metadata. Every operation derives the current bounds
//! from the backing store, so detach and resizable/growable-buffer transitions
//! cannot leave a cached pointer or element count stale.

use crate::engine::builtins::native::{DataViewElementKind, DataViewNativeKind};
use crate::engine::heap::{ArrayBufferViewData, DataViewRealmData, ObjectData, ObjectPayload};

#[cfg(test)]
use crate::engine::api::context::Context;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::NativeFunctionId,
    heap::{ContextId, ObjectId},
    object::{
        DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
    },
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{
            ConstructorPrototypeSource, NativeArguments, NativeInvocation,
            prototype::{ProtoSourceStep, finish as finish_source},
        },
    },
};

#[cfg(test)]
mod tests;

const DATA_VIEW_ELEMENT_KINDS: [DataViewElementKind; 11] = [
    DataViewElementKind::Int8,
    DataViewElementKind::Uint8,
    DataViewElementKind::Int16,
    DataViewElementKind::Uint16,
    DataViewElementKind::Int32,
    DataViewElementKind::Uint32,
    DataViewElementKind::BigInt64,
    DataViewElementKind::BigUint64,
    DataViewElementKind::Float16,
    DataViewElementKind::Float32,
    DataViewElementKind::Float64,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DataViewSnapshot {
    buffer: ObjectId,
    byte_offset: u32,
    fixed_byte_length: Option<u32>,
}

impl Runtime {
    /// Install DataView after SharedArrayBuffer and the TypedArray family,
    /// matching pinned QuickJS global key order.
    pub(crate) fn initialize_data_view_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let prototype = self.new_object(Some(object_prototype))?;

        for (kind, name) in [
            (DataViewNativeKind::Buffer, "buffer"),
            (DataViewNativeKind::ByteLength, "byteLength"),
            (DataViewNativeKind::ByteOffset, "byteOffset"),
        ] {
            self.define_native_builtin_getter_on(
                &prototype,
                function_prototype,
                realm,
                NativeFunctionId::DataView(kind),
                name,
                &format!("get {name}"),
            )?;
        }

        for element in DATA_VIEW_ELEMENT_KINDS {
            let readable = if data_view_element_width(element) == 1 {
                1
            } else {
                2
            };
            self.define_native_builtin_auto_init(
                &prototype,
                realm,
                NativeFunctionId::DataView(DataViewNativeKind::Get(element)),
                data_view_get_name(element),
                1,
                readable,
            )?;
        }
        for element in DATA_VIEW_ELEMENT_KINDS {
            let readable = if data_view_element_width(element) == 1 {
                2
            } else {
                3
            };
            self.define_native_builtin_auto_init(
                &prototype,
                realm,
                NativeFunctionId::DataView(DataViewNativeKind::Set(element)),
                data_view_set_name(element),
                2,
                readable,
            )?;
        }

        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            &prototype,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("DataView"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "DataView toStringTag definition was rejected",
            ));
        }

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::DataView(DataViewNativeKind::Constructor),
            3,
            "DataView",
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "DataView",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, &prototype)?;
        self.0.state.borrow_mut().heap.attach_data_view_intrinsics(
            realm,
            constructor.as_object().object_id(),
            DataViewRealmData {
                prototype: prototype.object_id(),
            },
        )?;
        Ok(())
    }

    pub(crate) fn call_data_view_native(
        &self,
        realm: ContextId,
        kind: DataViewNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            DataViewNativeKind::Constructor => {
                self.call_data_view_constructor(realm, invocation, arguments)
            }
            DataViewNativeKind::Buffer
            | DataViewNativeKind::ByteLength
            | DataViewNativeKind::ByteOffset => self.call_data_view_getter(realm, kind, invocation),
            DataViewNativeKind::Get(element) => {
                self.call_data_view_get(realm, element, invocation, arguments)
            }
            DataViewNativeKind::Set(element) => {
                self.call_data_view_set(realm, element, invocation, arguments)
            }
        }
    }

    fn call_data_view_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish_constructor(
            self,
            realm,
            DataViewConstructorStep::start(self, realm, &invocation, arguments)?,
        )
    }
    fn finish_data_view_construction(
        &self,
        realm: ContextId,
        buffer: ObjectRef,
        byte_offset: u32,
        fixed_byte_length: Option<u32>,
        prototype: ObjectRef,
    ) -> Result<Completion, RuntimeError> {
        let current = self.snapshot_buffer_access(buffer.object_id())?.state;
        if current.detached {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        let bounds_are_invalid = byte_offset > current.byte_length
            || fixed_byte_length.is_some_and(|length| {
                byte_offset
                    .checked_add(length)
                    .is_none_or(|end| end > current.byte_length)
            });
        if bounds_are_invalid {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid byteOffset or byteLength",
            )?));
        }

        let object =
            self.new_data_view_object(&prototype, &buffer, byte_offset, fixed_byte_length)?;
        Ok(Completion::Return(Value::Object(object)))
    }

    fn call_data_view_getter(
        &self,
        realm: ContextId,
        kind: DataViewNativeKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "DataView prototype getter received a non-getter invocation",
            ));
        };
        let object = match self.require_data_view(realm, this_value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let view = self.data_view_snapshot(&object)?;
        if kind == DataViewNativeKind::Buffer {
            let buffer = ObjectRef::from_borrowed_handle(self.clone(), view.buffer)?;
            return Ok(Completion::Return(Value::Object(buffer)));
        }

        let buffer = self.snapshot_buffer_access(view.buffer)?.state;
        let Some(byte_length) = data_view_in_bounds_byte_length(view, buffer) else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached or resized",
            )?));
        };
        let value = match kind {
            DataViewNativeKind::ByteLength => Value::Int(
                i32::try_from(byte_length).expect("DataView length is bounded by i32::MAX"),
            ),
            DataViewNativeKind::ByteOffset => Value::Int(
                i32::try_from(view.byte_offset).expect("DataView offset is bounded by i32::MAX"),
            ),
            DataViewNativeKind::Constructor
            | DataViewNativeKind::Buffer
            | DataViewNativeKind::Get(_)
            | DataViewNativeKind::Set(_) => {
                return Err(RuntimeError::Invariant(
                    "non-getter DataView native reached getter dispatch",
                ));
            }
        };
        Ok(Completion::Return(value))
    }

    fn call_data_view_get(
        &self,
        realm: ContextId,
        element: DataViewElementKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish_access(
            self,
            realm,
            DataViewAccessStep::start(
                self,
                realm,
                DataViewNativeKind::Get(element),
                &invocation,
                arguments,
            )?,
        )
    }
    fn call_data_view_set(
        &self,
        realm: ContextId,
        element: DataViewElementKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish_access(
            self,
            realm,
            DataViewAccessStep::start(
                self,
                realm,
                DataViewNativeKind::Set(element),
                &invocation,
                arguments,
            )?,
        )
    }

    fn data_view_convert_set_value(
        &self,
        realm: ContextId,
        element: DataViewElementKind,
        value: &Value,
    ) -> Result<NativeConversion<u64>, RuntimeError> {
        match element {
            DataViewElementKind::Int8
            | DataViewElementKind::Uint8
            | DataViewElementKind::Int16
            | DataViewElementKind::Uint16
            | DataViewElementKind::Int32
            | DataViewElementKind::Uint32 => match self.native_to_number(realm, value)? {
                NativeConversion::Value(number) => Ok(NativeConversion::Value(u64::from(
                    Self::to_uint32_number(number),
                ))),
                NativeConversion::Throw(value) => Ok(NativeConversion::Throw(value)),
            },
            DataViewElementKind::BigInt64 | DataViewElementKind::BigUint64 => {
                let bigint = match self.native_to_bigint(realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                };
                let narrowed = bigint.as_int_n(64).map_err(|_| {
                    RuntimeError::Invariant("64-bit DataView BigInt conversion failed")
                })?;
                let signed = narrowed.as_i64().ok_or(RuntimeError::Invariant(
                    "64-bit DataView BigInt did not normalize to an i64",
                ))?;
                Ok(NativeConversion::Value(signed as u64))
            }
            DataViewElementKind::Float16 => match self.native_to_number(realm, value)? {
                NativeConversion::Value(number) => Ok(NativeConversion::Value(u64::from(
                    crate::engine::value::number::to_float16_bits(number),
                ))),
                NativeConversion::Throw(value) => Ok(NativeConversion::Throw(value)),
            },
            DataViewElementKind::Float32 => match self.native_to_number(realm, value)? {
                NativeConversion::Value(number) => Ok(NativeConversion::Value(u64::from(
                    (number as f32).to_bits(),
                ))),
                NativeConversion::Throw(value) => Ok(NativeConversion::Throw(value)),
            },
            DataViewElementKind::Float64 => match self.native_to_number(realm, value)? {
                NativeConversion::Value(number) => {
                    Ok(NativeConversion::Value(f64::to_bits(number)))
                }
                NativeConversion::Throw(value) => Ok(NativeConversion::Throw(value)),
            },
        }
    }

    fn data_view_read_word(
        &self,
        realm: ContextId,
        view: DataViewSnapshot,
        position: u64,
        element: DataViewElementKind,
    ) -> Result<NativeConversion<[u8; 8]>, RuntimeError> {
        let width = data_view_element_width(element);
        let access = self.snapshot_buffer_access(view.buffer)?;
        let absolute =
            match self.validate_data_view_access(realm, view, access.state, position, width)? {
                NativeConversion::Value(absolute) => absolute,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
        let word = self.read_buffer_word(&access, absolute, width)?;
        Ok(NativeConversion::Value(word))
    }

    fn data_view_write_word(
        &self,
        realm: ContextId,
        view: DataViewSnapshot,
        position: u64,
        element: DataViewElementKind,
        bytes: &[u8; 8],
    ) -> Result<NativeConversion<()>, RuntimeError> {
        let width = data_view_element_width(element);
        let access = self.snapshot_buffer_access(view.buffer)?;
        let absolute =
            match self.validate_data_view_access(realm, view, access.state, position, width)? {
                NativeConversion::Value(absolute) => absolute,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
        self.write_buffer_word(&access, absolute, &bytes[..width])?;
        Ok(NativeConversion::Value(()))
    }

    fn validate_data_view_access(
        &self,
        realm: ContextId,
        view: DataViewSnapshot,
        buffer: crate::engine::heap::ArrayBufferState,
        position: u64,
        width: usize,
    ) -> Result<NativeConversion<usize>, RuntimeError> {
        if buffer.detached {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }

        // QuickJS updates the cached length of a tracking view on every
        // resize. Deriving that value here is equivalent and naturally
        // restores a view after its RAB grows back in bounds.
        let range_length = view
            .fixed_byte_length
            .unwrap_or_else(|| buffer.byte_length.saturating_sub(view.byte_offset));
        let width = u64::try_from(width).expect("DataView element width fits u64");
        if position
            .checked_add(width)
            .is_none_or(|end| end > u64::from(range_length))
        {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "out of bound",
            )?));
        }

        // This second check intentionally follows the declared-range check.
        // A fixed view made OOB by RAB shrink therefore reports RangeError
        // for an index outside its declared range, and TypeError only for an
        // otherwise-valid index whose backing bytes disappeared.
        if view
            .byte_offset
            .checked_add(range_length)
            .is_none_or(|end| end > buffer.byte_length)
        {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "out of bound",
            )?));
        }

        let absolute =
            u64::from(view.byte_offset)
                .checked_add(position)
                .ok_or(RuntimeError::Invariant(
                    "DataView absolute byte position overflowed u64",
                ))?;
        let absolute = usize::try_from(absolute)
            .map_err(|_| RuntimeError::Invariant("DataView byte position overflowed usize"))?;
        Ok(NativeConversion::Value(absolute))
    }

    fn require_data_view_array_buffer(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer object expected",
            )?));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("DataView ArrayBuffer"));
        }
        let is_array_buffer = self.snapshot_buffer_access_if_branded(object)?.is_some();
        if !is_array_buffer {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer object expected",
            )?));
        }
        Ok(NativeConversion::Value(object.clone()))
    }

    fn require_data_view(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a DataView",
            )?));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("DataView"));
        }
        let is_data_view = {
            let state = self.0.state.borrow();
            matches!(
                state.heap.object(object.object_id())?.payload,
                ObjectPayload::DataView(_)
            )
        };
        if !is_data_view {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a DataView",
            )?));
        }
        Ok(NativeConversion::Value(object))
    }

    fn data_view_snapshot(&self, object: &ObjectRef) -> Result<DataViewSnapshot, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("DataView"));
        }
        let state = self.0.state.borrow();
        let ObjectPayload::DataView(view) = &state.heap.object(object.object_id())?.payload else {
            return Err(RuntimeError::Invariant(
                "validated DataView lost its class payload",
            ));
        };
        Ok(DataViewSnapshot {
            buffer: view.buffer,
            byte_offset: view.byte_offset,
            fixed_byte_length: view.fixed_byte_length,
        })
    }

    fn data_view_default_prototype(&self, realm: ContextId) -> Result<ObjectRef, RuntimeError> {
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .data_view
            .ok_or(RuntimeError::Invariant("realm has no DataView intrinsics"))?
            .prototype;
        Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
    }

    fn new_data_view_object(
        &self,
        prototype: &ObjectRef,
        buffer: &ObjectRef,
        byte_offset: u32,
        fixed_byte_length: Option<u32>,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) || !buffer.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("DataView allocation"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state.heap.allocate_object(ObjectData::data_view(
            shape,
            Vec::new(),
            ArrayBufferViewData {
                buffer: buffer.object_id(),
                byte_offset,
                fixed_byte_length,
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
}

fn data_view_in_bounds_byte_length(
    view: DataViewSnapshot,
    buffer: crate::engine::heap::ArrayBufferState,
) -> Option<u32> {
    if buffer.detached || view.byte_offset > buffer.byte_length {
        return None;
    }
    match view.fixed_byte_length {
        Some(length)
            if view
                .byte_offset
                .checked_add(length)
                .is_none_or(|end| end > buffer.byte_length) =>
        {
            None
        }
        Some(length) => Some(length),
        None => Some(buffer.byte_length - view.byte_offset),
    }
}

fn data_view_get_name(element: DataViewElementKind) -> &'static str {
    match element {
        DataViewElementKind::Int8 => "getInt8",
        DataViewElementKind::Uint8 => "getUint8",
        DataViewElementKind::Int16 => "getInt16",
        DataViewElementKind::Uint16 => "getUint16",
        DataViewElementKind::Int32 => "getInt32",
        DataViewElementKind::Uint32 => "getUint32",
        DataViewElementKind::BigInt64 => "getBigInt64",
        DataViewElementKind::BigUint64 => "getBigUint64",
        DataViewElementKind::Float16 => "getFloat16",
        DataViewElementKind::Float32 => "getFloat32",
        DataViewElementKind::Float64 => "getFloat64",
    }
}

fn data_view_set_name(element: DataViewElementKind) -> &'static str {
    match element {
        DataViewElementKind::Int8 => "setInt8",
        DataViewElementKind::Uint8 => "setUint8",
        DataViewElementKind::Int16 => "setInt16",
        DataViewElementKind::Uint16 => "setUint16",
        DataViewElementKind::Int32 => "setInt32",
        DataViewElementKind::Uint32 => "setUint32",
        DataViewElementKind::BigInt64 => "setBigInt64",
        DataViewElementKind::BigUint64 => "setBigUint64",
        DataViewElementKind::Float16 => "setFloat16",
        DataViewElementKind::Float32 => "setFloat32",
        DataViewElementKind::Float64 => "setFloat64",
    }
}

const fn data_view_element_width(element: DataViewElementKind) -> usize {
    match element {
        DataViewElementKind::Int8 | DataViewElementKind::Uint8 => 1,
        DataViewElementKind::Int16 | DataViewElementKind::Uint16 | DataViewElementKind::Float16 => {
            2
        }
        DataViewElementKind::Int32 | DataViewElementKind::Uint32 | DataViewElementKind::Float32 => {
            4
        }
        DataViewElementKind::BigInt64
        | DataViewElementKind::BigUint64
        | DataViewElementKind::Float64 => 8,
    }
}

fn data_view_encode(element: DataViewElementKind, raw: u64, little_endian: bool) -> [u8; 8] {
    let mut bytes = [0_u8; 8];
    match data_view_element_width(element) {
        1 => bytes[0] = raw as u8,
        2 => {
            let word = if little_endian {
                (raw as u16).to_le_bytes()
            } else {
                (raw as u16).to_be_bytes()
            };
            bytes[..2].copy_from_slice(&word);
        }
        4 => {
            let word = if little_endian {
                (raw as u32).to_le_bytes()
            } else {
                (raw as u32).to_be_bytes()
            };
            bytes[..4].copy_from_slice(&word);
        }
        8 => {
            bytes = if little_endian {
                raw.to_le_bytes()
            } else {
                raw.to_be_bytes()
            };
        }
        _ => unreachable!("DataView element widths are 1, 2, 4, or 8"),
    }
    bytes
}

fn data_view_decode(element: DataViewElementKind, bytes: [u8; 8], little_endian: bool) -> Value {
    let u16_value = || {
        let bytes = [bytes[0], bytes[1]];
        if little_endian {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    };
    let u32_value = || {
        let bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }
    };
    let u64_value = || {
        if little_endian {
            u64::from_le_bytes(bytes)
        } else {
            u64::from_be_bytes(bytes)
        }
    };
    match element {
        DataViewElementKind::Int8 => Value::Int(i32::from(bytes[0] as i8)),
        DataViewElementKind::Uint8 => Value::Int(i32::from(bytes[0])),
        DataViewElementKind::Int16 => Value::Int(i32::from(u16_value() as i16)),
        DataViewElementKind::Uint16 => Value::Int(i32::from(u16_value())),
        DataViewElementKind::Int32 => Value::Int(u32_value() as i32),
        DataViewElementKind::Uint32 => Runtime::array_length_value(u32_value()),
        DataViewElementKind::BigInt64 => Value::BigInt(
            crate::engine::value::bigint::JsBigInt::from(u64_value() as i64),
        ),
        DataViewElementKind::BigUint64 => {
            Value::BigInt(crate::engine::value::bigint::JsBigInt::from(u64_value()))
        }
        DataViewElementKind::Float16 => {
            Value::number(crate::engine::value::number::from_float16_bits(u16_value()))
        }
        DataViewElementKind::Float32 => Value::number(f64::from(f32::from_bits(u32_value()))),
        DataViewElementKind::Float64 => Value::number(f64::from_bits(u64_value())),
    }
}

/// A DataView keeps its branded owner alive, but never retains an access
/// certificate across user conversion. The immutable view metadata and the
/// current backing-store bounds are acquired again at the final access.
pub(crate) enum DataViewAccessStep {
    Complete(Completion),
    Primitive {
        value: Value,
        resume: DataViewAccessResume,
    },
}
pub(crate) struct DataViewAccessResume(Box<DataViewAccessResumeState>);
impl std::ops::Deref for DataViewAccessResume {
    type Target = DataViewAccessResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for DataViewAccessResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<DataViewAccessResume>() <= 8);
pub(crate) struct DataViewAccessResumeState {
    realm: ContextId,
    object: ObjectRef,
    element: DataViewElementKind,
    endian: Value,
    phase: AccessPhase,
}
enum AccessPhase {
    GetPosition,
    SetPosition(Value),
    SetValue(u64),
}
impl DataViewAccessStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: DataViewNativeKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let (element, set) = match kind {
            DataViewNativeKind::Get(element) => (element, false),
            DataViewNativeKind::Set(element) => (element, true),
            _ => {
                return Err(RuntimeError::Invariant(
                    "DataView access received a non-access operation",
                ));
            }
        };
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(if set {
                "DataView setter method received a constructor invocation"
            } else {
                "DataView getter method received a constructor invocation"
            }));
        };
        let object = match runtime.require_data_view(realm, this_value.clone())? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        // Preserve the original payload check before the first coercion.
        runtime.data_view_snapshot(&object)?;
        let value = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(if set {
                "DataView set byteOffset argument was not padded"
            } else {
                "DataView get byteOffset argument was not padded"
            }))?
            .clone();
        let phase = if set {
            AccessPhase::SetPosition(
                arguments
                    .readable
                    .get(1)
                    .ok_or(RuntimeError::Invariant(
                        "DataView set value argument was not padded",
                    ))?
                    .clone(),
            )
        } else {
            AccessPhase::GetPosition
        };
        let endian = arguments
            .readable
            .get(if set { 2 } else { 1 })
            .cloned()
            .unwrap_or(Value::Bool(false));
        Ok(Self::Primitive {
            value,
            resume: DataViewAccessResume(Box::new(DataViewAccessResumeState {
                realm,
                object,
                element,
                endian,
                phase,
            })),
        })
    }
}
impl DataViewAccessResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<DataViewAccessStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(DataViewAccessStep::Complete(Completion::Throw(value)));
            }
        };
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "DataView conversion returned an object",
            ));
        }
        match self.0.phase {
            AccessPhase::GetPosition | AccessPhase::SetPosition(_) => {
                let position = match runtime.native_to_index(self.0.realm, &value)? {
                    NativeConversion::Value(position) => position,
                    NativeConversion::Throw(value) => {
                        return Ok(DataViewAccessStep::Complete(Completion::Throw(value)));
                    }
                };
                if let AccessPhase::SetPosition(value) = self.0.phase {
                    return Ok(DataViewAccessStep::Primitive {
                        value,
                        resume: {
                            let updated_0 = AccessPhase::SetValue(position);
                            self.0.phase = updated_0;
                            self
                        },
                    });
                }
                let little_endian = runtime.value_to_boolean(&self.0.endian)?;
                let view = runtime.data_view_snapshot(&self.0.object)?;
                let bytes = match runtime.data_view_read_word(
                    self.0.realm,
                    view,
                    position,
                    self.0.element,
                )? {
                    NativeConversion::Value(bytes) => bytes,
                    NativeConversion::Throw(value) => {
                        return Ok(DataViewAccessStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok(DataViewAccessStep::Complete(Completion::Return(
                    data_view_decode(self.0.element, bytes, little_endian),
                )))
            }
            AccessPhase::SetValue(position) => {
                let converted = match runtime.data_view_convert_set_value(
                    self.0.realm,
                    self.0.element,
                    &value,
                )? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(DataViewAccessStep::Complete(Completion::Throw(value)));
                    }
                };
                let little_endian = runtime.value_to_boolean(&self.0.endian)?;
                let bytes = data_view_encode(self.0.element, converted, little_endian);
                let view = runtime.data_view_snapshot(&self.0.object)?;
                Ok(DataViewAccessStep::Complete(
                    match runtime.data_view_write_word(
                        self.0.realm,
                        view,
                        position,
                        self.0.element,
                        &bytes,
                    )? {
                        NativeConversion::Value(()) => Completion::Return(Value::Undefined),
                        NativeConversion::Throw(value) => Completion::Throw(value),
                    },
                ))
            }
        }
    }
}
fn finish_access(
    runtime: &Runtime,
    realm: ContextId,
    mut step: DataViewAccessStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            DataViewAccessStep::Complete(result) => return Ok(result),
            DataViewAccessStep::Primitive { value, resume } => {
                let result = if matches!(value, Value::Object(_)) {
                    runtime.to_primitive(realm, value, ToPrimitiveHint::Number)?
                } else {
                    Completion::Return(value)
                };
                resume.resume(runtime, result)?
            }
        };
    }
}

pub(crate) enum DataViewConstructorStep {
    Complete(Completion),
    Primitive {
        value: Value,
        resume: DataViewConstructorResume,
    },
    Prototype {
        new_target: Value,
        resume: DataViewConstructorResume,
    },
}
pub(crate) struct DataViewConstructorResume(Box<DataViewConstructorResumeState>);
impl std::ops::Deref for DataViewConstructorResume {
    type Target = DataViewConstructorResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for DataViewConstructorResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<DataViewConstructorResume>() <= 8);
pub(crate) struct DataViewConstructorResumeState {
    realm: ContextId,
    buffer: ObjectRef,
    new_target: Value,
    length: Option<Value>,
    phase: DataViewConstructorPhase,
}
enum DataViewConstructorPhase {
    Offset,
    Length { offset: u32, available: u32 },
    Prototype { offset: u32, length: Option<u32> },
}
impl DataViewConstructorStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "DataView constructor did not receive a constructor invocation",
            ));
        };
        let buffer = match runtime.require_data_view_array_buffer(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "DataView buffer argument was not padded",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let length = if arguments.actual_arg_count > 2
            && !matches!(arguments.readable.get(2), Some(Value::Undefined))
        {
            Some(
                arguments
                    .readable
                    .get(2)
                    .ok_or(RuntimeError::Invariant(
                        "DataView byteLength argument was not padded",
                    ))?
                    .clone(),
            )
        } else {
            None
        };
        let resume = DataViewConstructorResume(Box::new(DataViewConstructorResumeState {
            realm,
            buffer,
            new_target: new_target.clone(),
            length,
            phase: DataViewConstructorPhase::Offset,
        }));
        if arguments.actual_arg_count > 1 {
            Ok(Self::Primitive {
                value: arguments
                    .readable
                    .get(1)
                    .ok_or(RuntimeError::Invariant(
                        "DataView byteOffset argument was not padded",
                    ))?
                    .clone(),
                resume,
            })
        } else {
            resume.offset(runtime, 0)
        }
    }
}
impl DataViewConstructorResume {
    fn lookup(mut self, offset: u32, length: Option<u32>) -> DataViewConstructorStep {
        DataViewConstructorStep::Prototype {
            new_target: self.0.new_target.clone(),
            resume: {
                let updated_0 = DataViewConstructorPhase::Prototype { offset, length };
                self.0.phase = updated_0;
                self
            },
        }
    }
    fn offset(
        mut self,
        runtime: &Runtime,
        offset: u64,
    ) -> Result<DataViewConstructorStep, RuntimeError> {
        let initial = runtime
            .snapshot_buffer_access(self.0.buffer.object_id())?
            .state;
        if initial.detached {
            return Ok(DataViewConstructorStep::Complete(Completion::Throw(
                runtime.new_native_error(
                    self.0.realm,
                    NativeErrorKind::Type,
                    "ArrayBuffer is detached",
                )?,
            )));
        }
        if offset > u64::from(initial.byte_length) {
            return Ok(DataViewConstructorStep::Complete(Completion::Throw(
                runtime.new_native_error(
                    self.0.realm,
                    NativeErrorKind::Range,
                    "invalid byteOffset",
                )?,
            )));
        }
        let offset = u32::try_from(offset)
            .map_err(|_| RuntimeError::Invariant("validated DataView offset overflowed u32"))?;
        if let Some(length) = &self.0.length {
            Ok(DataViewConstructorStep::Primitive {
                value: length.clone(),
                resume: {
                    let updated_0 = DataViewConstructorPhase::Length {
                        offset,
                        available: initial.byte_length - offset,
                    };
                    self.0.phase = updated_0;
                    self
                },
            })
        } else {
            Ok(self.lookup(
                offset,
                if initial.max_byte_length.is_some() {
                    None
                } else {
                    Some(initial.byte_length - offset)
                },
            ))
        }
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<DataViewConstructorStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(DataViewConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "DataView constructor numeric conversion returned an object",
            ));
        }
        let value = match runtime.native_to_index(self.0.realm, &value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(DataViewConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            DataViewConstructorPhase::Offset => self.offset(runtime, value),
            DataViewConstructorPhase::Length { offset, available } => {
                if value > u64::from(available) {
                    return Ok(DataViewConstructorStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Range,
                            "invalid byteLength",
                        )?,
                    )));
                }
                let length = u32::try_from(value).map_err(|_| {
                    RuntimeError::Invariant("validated DataView length overflowed u32")
                })?;
                Ok(self.lookup(offset, Some(length)))
            }
            DataViewConstructorPhase::Prototype { .. } => Err(RuntimeError::Invariant(
                "DataView prototype request received an untyped reply",
            )),
        }
    }
    pub(crate) fn prototype(
        self,
        runtime: &Runtime,
        result: NativeConversion<ConstructorPrototypeSource>,
    ) -> Result<DataViewConstructorStep, RuntimeError> {
        let prototype = match result {
            NativeConversion::Value(ConstructorPrototypeSource::Explicit(value)) => value,
            NativeConversion::Value(ConstructorPrototypeSource::Realm(realm)) => {
                runtime.data_view_default_prototype(realm)?
            }
            NativeConversion::Throw(value) => {
                return Ok(DataViewConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        let DataViewConstructorPhase::Prototype { offset, length } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "DataView constructor received an unexpected prototype reply",
            ));
        };
        Ok(DataViewConstructorStep::Complete(
            runtime.finish_data_view_construction(
                self.0.realm,
                self.0.buffer,
                offset,
                length,
                prototype,
            )?,
        ))
    }
}
fn finish_constructor(
    runtime: &Runtime,
    realm: ContextId,
    mut step: DataViewConstructorStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            DataViewConstructorStep::Complete(result) => return Ok(result),
            DataViewConstructorStep::Primitive { value, resume } => {
                let result = if matches!(value, Value::Object(_)) {
                    runtime.to_primitive(realm, value, ToPrimitiveHint::Number)?
                } else {
                    Completion::Return(value)
                };
                resume.resume(runtime, result)?
            }
            DataViewConstructorStep::Prototype { new_target, resume } => resume.prototype(
                runtime,
                finish_source(
                    runtime,
                    realm,
                    ProtoSourceStep::start(runtime, realm, new_target)?,
                )?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<DataViewAccessStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<DataViewConstructorStep>() <= 64);
