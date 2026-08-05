//! Pinned QuickJS `%Atomics%` operations over integer TypedArrays.
//!
//! The existing non-shared milestone deliberately implements QuickJS's useful
//! behavior on ordinary ArrayBuffer-backed integer TypedArrays. Shared views
//! are now recognized but remain explicitly rejected until the separately
//! gated shared-Atomics and waiter milestones. `wait` rejects an ordinary view
//! before coercing its remaining arguments, while `notify` performs its
//! ordinary validation and coercions before returning zero.

use crate::heap::{AtomicsNativeKind, AtomicsOperationKind, TypedArrayElementKind};

use super::array_buffer::typed_array::TypedArraySnapshot;
use super::*;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtomicAccessMode {
    Ordinary,
    Notify,
    Wait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AtomicAccess {
    snapshot: TypedArraySnapshot,
    index: u64,
}

impl Runtime {
    /// Install QuickJS's lazy global `Atomics` `JS_OBJECT_DEF` equivalent.
    pub(in crate::runtime) fn initialize_atomics_intrinsic(
        &self,
        realm: ContextId,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key("Atomics")?;
        self.store_property_slot(
            global_object,
            &key,
            PropertyFlags::data(true, false, true),
            PropertySlot::AutoInit(AutoInitProperty::Atomics { realm }),
        )
    }

    /// Materialize the pinned `js_atomics_funcs` table in declaration order.
    pub(in crate::runtime) fn instantiate_atomics_intrinsic(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let atomics = self.new_ordinary_object_in_realm(realm)?;
        for (target, name, length, readable) in [
            (
                AtomicsNativeKind::Operation(AtomicsOperationKind::Add),
                "add",
                3,
                3,
            ),
            (
                AtomicsNativeKind::Operation(AtomicsOperationKind::And),
                "and",
                3,
                3,
            ),
            (
                AtomicsNativeKind::Operation(AtomicsOperationKind::Or),
                "or",
                3,
                3,
            ),
            (
                AtomicsNativeKind::Operation(AtomicsOperationKind::Sub),
                "sub",
                3,
                3,
            ),
            (
                AtomicsNativeKind::Operation(AtomicsOperationKind::Xor),
                "xor",
                3,
                3,
            ),
            (
                AtomicsNativeKind::Operation(AtomicsOperationKind::Exchange),
                "exchange",
                3,
                3,
            ),
            (
                AtomicsNativeKind::Operation(AtomicsOperationKind::CompareExchange),
                "compareExchange",
                4,
                4,
            ),
            (
                AtomicsNativeKind::Operation(AtomicsOperationKind::Load),
                "load",
                2,
                2,
            ),
            (AtomicsNativeKind::Store, "store", 3, 3),
            (AtomicsNativeKind::IsLockFree, "isLockFree", 1, 1),
            (AtomicsNativeKind::Pause, "pause", 0, 0),
            (AtomicsNativeKind::Wait, "wait", 4, 4),
            (AtomicsNativeKind::Notify, "notify", 3, 3),
        ] {
            self.define_native_builtin_auto_init(
                &atomics,
                realm,
                NativeFunctionId::Atomics(target),
                name,
                length,
                readable,
            )?;
        }

        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            &atomics,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("Atomics"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Atomics toStringTag definition was rejected",
            ));
        }
        Ok(atomics)
    }

    pub(in crate::runtime) fn call_atomics_native(
        &self,
        realm: ContextId,
        kind: AtomicsNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Atomics method did not receive a generic invocation",
            ));
        };
        match kind {
            AtomicsNativeKind::Operation(operation) => {
                self.call_atomics_operation(realm, operation, arguments)
            }
            AtomicsNativeKind::Store => self.call_atomics_store(realm, arguments),
            AtomicsNativeKind::IsLockFree => self.call_atomics_is_lock_free(realm, arguments),
            AtomicsNativeKind::Pause => self.call_atomics_pause(realm, arguments),
            AtomicsNativeKind::Wait => self.call_atomics_wait(realm, arguments),
            AtomicsNativeKind::Notify => self.call_atomics_notify(realm, arguments),
        }
    }

    /// Pinned `js_atomics_get_buf` validation and coercion ordering.
    ///
    /// `Notify` intentionally skips post-index detach/RAB revalidation, as
    /// upstream does before discovering that an ordinary ArrayBuffer has no
    /// waiter list. `Wait` rejects the non-shared backing store before index,
    /// expected-value, or timeout coercion.
    fn atomics_get_access(
        &self,
        realm: ContextId,
        typed_array: &Value,
        index: &Value,
        mode: AtomicAccessMode,
    ) -> Result<NativeConversion<AtomicAccess>, RuntimeError> {
        let Value::Object(object) = typed_array else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "integer TypedArray expected",
            )?));
        };
        let Some(snapshot) = self.typed_array_snapshot_if_branded(object)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "integer TypedArray expected",
            )?));
        };
        let valid_element = match mode {
            AtomicAccessMode::Ordinary => atomic_element_is_integer(snapshot.element),
            AtomicAccessMode::Notify | AtomicAccessMode::Wait => {
                atomic_element_is_waitable(snapshot.element)
            }
        };
        if !valid_element {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "integer TypedArray expected",
            )?));
        }

        let buffer_access = self.snapshot_buffer_access(snapshot.buffer)?;
        if buffer_access.is_shared() {
            // R3dh makes SharedArrayBuffer views usable throughout the binary
            // data stack. Shared Atomics are the next independently gated
            // milestone; reject them as JavaScript until their atomic/waiter
            // kernels are installed rather than leaking an ordinary-buffer
            // heap invariant through the public evaluator.
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "shared-memory Atomics are not implemented",
            )?));
        }

        // QuickJS performs this non-shared wait rejection before even its
        // initial detach check.
        if mode == AtomicAccessMode::Wait {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a SharedArrayBuffer TypedArray",
            )?));
        }

        let buffer = buffer_access.state;
        if buffer.detached {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }

        // QuickJS snapshots p->u.array.count before ToIndex because the
        // conversion may resize or detach the backing buffer.
        let old_length = self.typed_array_state_from_snapshot(snapshot)?.length;
        let index = match self.native_to_index(realm, index)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if index >= u64::from(old_length) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "out-of-bound access",
            )?));
        }

        if mode == AtomicAccessMode::Ordinary {
            let current = self.typed_array_state_from_snapshot(snapshot)?;
            if current.out_of_bounds {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "ArrayBuffer is detached or resized",
                )?));
            }
            if index >= u64::from(current.length) {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "out-of-bound access",
                )?));
            }
        }

        Ok(NativeConversion::Value(AtomicAccess { snapshot, index }))
    }

    /// Revalidate after operand coercion, which may detach or resize.
    fn atomics_revalidate_after_value(
        &self,
        realm: ContextId,
        access: AtomicAccess,
    ) -> Result<NativeConversion<()>, RuntimeError> {
        let current = self.typed_array_state_from_snapshot(access.snapshot)?;
        if current.out_of_bounds {
            // js_atomics_op/js_atomics_store explicitly use the detached
            // ArrayBuffer error here even when a RAB resize caused the state.
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        if access.index >= u64::from(current.length) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "out-of-bound access",
            )?));
        }
        Ok(NativeConversion::Value(()))
    }

    fn call_atomics_operation(
        &self,
        realm: ContextId,
        operation: AtomicsOperationKind,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let typed_array = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Atomics TypedArray was not readable",
        ))?;
        let index = arguments
            .readable
            .get(1)
            .ok_or(RuntimeError::Invariant("Atomics index was not readable"))?;
        let access =
            match self.atomics_get_access(realm, typed_array, index, AtomicAccessMode::Ordinary)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };

        if operation == AtomicsOperationKind::Load {
            return Ok(Completion::Return(self.atomics_load(access)?));
        }

        let operand = match self.typed_array_convert_element(
            realm,
            access.snapshot.element,
            arguments
                .readable
                .get(2)
                .ok_or(RuntimeError::Invariant("Atomics operand was not readable"))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let replacement = if operation == AtomicsOperationKind::CompareExchange {
            match self.typed_array_convert_element(
                realm,
                access.snapshot.element,
                arguments.readable.get(3).ok_or(RuntimeError::Invariant(
                    "Atomics replacement was not readable",
                ))?,
            )? {
                NativeConversion::Value(value) => Some(value),
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            None
        };

        match self.atomics_revalidate_after_value(realm, access)? {
            NativeConversion::Value(()) => {}
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        }
        Ok(Completion::Return(self.atomics_modify(
            access,
            operation,
            operand,
            replacement,
        )?))
    }

    fn atomics_load(&self, access: AtomicAccess) -> Result<Value, RuntimeError> {
        let width = usize::from(access.snapshot.element.byte_length());
        let offset = atomic_absolute_byte_offset(access)?;
        let bytes = self.0.state.borrow().heap.read_array_buffer_word(
            access.snapshot.buffer,
            offset,
            width,
        )?;
        Ok(atomic_decode_value(
            access.snapshot.element,
            atomic_decode_word(&bytes[..width]),
        ))
    }

    fn atomics_modify(
        &self,
        access: AtomicAccess,
        operation: AtomicsOperationKind,
        operand: [u8; 8],
        replacement: Option<[u8; 8]>,
    ) -> Result<Value, RuntimeError> {
        let width = usize::from(access.snapshot.element.byte_length());
        let offset = atomic_absolute_byte_offset(access)?;
        let operand = atomic_decode_word(&operand[..width]);
        let replacement = replacement.map(|bytes| atomic_decode_word(&bytes[..width]));
        let old = self.0.state.borrow_mut().heap.with_array_buffer_range_mut(
            access.snapshot.buffer,
            offset,
            width,
            |bytes| {
                let old = atomic_decode_word(bytes);
                let next = match operation {
                    AtomicsOperationKind::Add => old.wrapping_add(operand),
                    AtomicsOperationKind::And => old & operand,
                    AtomicsOperationKind::Or => old | operand,
                    AtomicsOperationKind::Sub => old.wrapping_sub(operand),
                    AtomicsOperationKind::Xor => old ^ operand,
                    AtomicsOperationKind::Exchange => operand,
                    AtomicsOperationKind::CompareExchange => {
                        if old == operand {
                            replacement.expect("compareExchange has a replacement")
                        } else {
                            old
                        }
                    }
                    AtomicsOperationKind::Load => {
                        unreachable!("Atomics.load does not mutate")
                    }
                };
                atomic_encode_word(bytes, next);
                old
            },
        )?;
        Ok(atomic_decode_value(access.snapshot.element, old))
    }

    fn call_atomics_store(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let access = match self.atomics_get_access(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Atomics.store view was not readable",
            ))?,
            arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "Atomics.store index was not readable",
            ))?,
            AtomicAccessMode::Ordinary,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let input = arguments.readable.get(2).ok_or(RuntimeError::Invariant(
            "Atomics.store value was not readable",
        ))?;
        let stored_value = if access.snapshot.element.is_bigint() {
            match self.native_to_bigint(realm, input)? {
                NativeConversion::Value(value) => Value::BigInt(value),
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            let number = match self.native_to_number(realm, input)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let integer = if number.is_nan() {
                0.0
            } else {
                let integer = number.trunc();
                if integer == 0.0 { 0.0 } else { integer }
            };
            Value::number(integer)
        };
        let bytes = match self.typed_array_convert_element(
            realm,
            access.snapshot.element,
            &stored_value,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "primitive Atomics.store value failed its second conversion",
                ));
            }
        };
        match self.atomics_revalidate_after_value(realm, access)? {
            NativeConversion::Value(()) => {}
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        }

        let width = usize::from(access.snapshot.element.byte_length());
        let offset = atomic_absolute_byte_offset(access)?;
        self.0.state.borrow_mut().heap.write_array_buffer_word(
            access.snapshot.buffer,
            offset,
            &bytes[..width],
        )?;
        Ok(Completion::Return(stored_value))
    }

    fn call_atomics_is_lock_free(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let number = match self.native_to_number(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Atomics.isLockFree size was not readable",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let size = atomic_to_int32_sat(number);
        Ok(Completion::Return(Value::Bool(matches!(
            size,
            1 | 2 | 4 | 8
        ))))
    }

    fn call_atomics_pause(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if arguments.actual_arg_count > 0 {
            let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Atomics.pause hint was not readable",
            ))?;
            let valid = match value {
                Value::Undefined | Value::Int(_) => true,
                Value::Float(value) => value.is_finite() && value.fract() == 0.0,
                Value::Null
                | Value::Bool(_)
                | Value::BigInt(_)
                | Value::String(_)
                | Value::Symbol(_)
                | Value::Object(_) => false,
            };
            if !valid {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an integral number",
                )?));
            }
        }
        std::hint::spin_loop();
        Ok(Completion::Return(Value::Undefined))
    }

    fn call_atomics_wait(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match self.atomics_get_access(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Atomics.wait view was not readable",
            ))?,
            arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "Atomics.wait index was not readable",
            ))?,
            AtomicAccessMode::Wait,
        )? {
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
            NativeConversion::Value(_) => Err(RuntimeError::Invariant(
                "non-shared Atomics.wait unexpectedly acquired an access",
            )),
        }
    }

    fn call_atomics_notify(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match self.atomics_get_access(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Atomics.notify view was not readable",
            ))?,
            arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "Atomics.notify index was not readable",
            ))?,
            AtomicAccessMode::Notify,
        )? {
            NativeConversion::Value(_) => {}
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        }

        let count = arguments.readable.get(2).ok_or(RuntimeError::Invariant(
            "Atomics.notify count was not readable",
        ))?;
        if !matches!(count, Value::Undefined) {
            let number = match self.native_to_number(realm, count)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let _count = atomic_to_int32_sat(number).clamp(0, i32::MAX);
        }

        // An ordinary ArrayBuffer can never own a QuickJS waiter list. The
        // coercions above remain observable even though the result is zero.
        Ok(Completion::Return(Value::Int(0)))
    }
}

const fn atomic_element_is_integer(element: TypedArrayElementKind) -> bool {
    matches!(
        element,
        TypedArrayElementKind::Int8
            | TypedArrayElementKind::Uint8
            | TypedArrayElementKind::Int16
            | TypedArrayElementKind::Uint16
            | TypedArrayElementKind::Int32
            | TypedArrayElementKind::Uint32
            | TypedArrayElementKind::BigInt64
            | TypedArrayElementKind::BigUint64
    )
}

const fn atomic_element_is_waitable(element: TypedArrayElementKind) -> bool {
    matches!(
        element,
        TypedArrayElementKind::Int32 | TypedArrayElementKind::BigInt64
    )
}

fn atomic_absolute_byte_offset(access: AtomicAccess) -> Result<usize, RuntimeError> {
    let relative = access
        .index
        .checked_mul(u64::from(access.snapshot.element.byte_length()))
        .ok_or(RuntimeError::Invariant(
            "Atomics relative byte offset overflowed u64",
        ))?;
    let absolute = u64::from(access.snapshot.byte_offset)
        .checked_add(relative)
        .ok_or(RuntimeError::Invariant(
            "Atomics absolute byte offset overflowed u64",
        ))?;
    usize::try_from(absolute)
        .map_err(|_| RuntimeError::Invariant("Atomics byte offset overflowed usize"))
}

fn atomic_decode_word(bytes: &[u8]) -> u64 {
    match bytes {
        [value] => u64::from(*value),
        [a, b] => u64::from(u16::from_ne_bytes([*a, *b])),
        [a, b, c, d] => u64::from(u32::from_ne_bytes([*a, *b, *c, *d])),
        [a, b, c, d, e, f, g, h] => u64::from_ne_bytes([*a, *b, *c, *d, *e, *f, *g, *h]),
        _ => unreachable!("Atomics word width is 1, 2, 4, or 8"),
    }
}

fn atomic_encode_word(bytes: &mut [u8], value: u64) {
    match bytes {
        [slot] => *slot = value as u8,
        [a, b] => [*a, *b] = (value as u16).to_ne_bytes(),
        [a, b, c, d] => [*a, *b, *c, *d] = (value as u32).to_ne_bytes(),
        [a, b, c, d, e, f, g, h] => {
            [*a, *b, *c, *d, *e, *f, *g, *h] = value.to_ne_bytes();
        }
        _ => unreachable!("Atomics word width is 1, 2, 4, or 8"),
    }
}

fn atomic_decode_value(element: TypedArrayElementKind, word: u64) -> Value {
    match element {
        TypedArrayElementKind::Int8 => Value::Int(i32::from(word as u8 as i8)),
        TypedArrayElementKind::Uint8 => Value::Int(i32::from(word as u8)),
        TypedArrayElementKind::Int16 => Value::Int(i32::from(word as u16 as i16)),
        TypedArrayElementKind::Uint16 => Value::Int(i32::from(word as u16)),
        TypedArrayElementKind::Int32 => Value::Int(word as u32 as i32),
        TypedArrayElementKind::Uint32 => i32::try_from(word as u32)
            .map(Value::Int)
            .unwrap_or_else(|_| Value::number(f64::from(word as u32))),
        TypedArrayElementKind::BigInt64 => {
            Value::BigInt(crate::bigint::JsBigInt::from(word as i64))
        }
        TypedArrayElementKind::BigUint64 => Value::BigInt(crate::bigint::JsBigInt::from(word)),
        TypedArrayElementKind::Uint8Clamped
        | TypedArrayElementKind::Float16
        | TypedArrayElementKind::Float32
        | TypedArrayElementKind::Float64 => {
            unreachable!("non-integer TypedArray reached Atomics decoding")
        }
    }
}

fn atomic_to_int32_sat(number: f64) -> i32 {
    if number.is_nan() {
        0
    } else if number < f64::from(i32::MIN) {
        i32::MIN
    } else if number > f64::from(i32::MAX) {
        i32::MAX
    } else {
        number as i32
    }
}
