//! Pinned QuickJS `%Atomics%` operations over integer TypedArrays.
//!
//! QuickJS's useful ordinary-ArrayBuffer behavior is preserved alongside
//! shared-memory load/store/read-modify-write operations. Every memory access
//! first enters one process-wide sequencing gate and then the backing store,
//! conservatively reproducing QuickJS's default C11 sequentially-consistent
//! order even across distinct buffers. All observable coercion and
//! revalidation work finishes before either lock is acquired. Blocking waits
//! use the same process-wide coordinator for their final comparison,
//! registration, timeout race, and FIFO notification.

use super::array_buffer::typed_array::TypedArraySnapshot;
use crate::engine::builtins::buffer_access::BufferAccessToken;

use crate::engine::builtins::native::{
    AtomicsNativeKind, AtomicsOperationKind, TypedArrayElementKind,
};
use std::time::Duration;

use super::*;

mod operation;
#[cfg(test)]
mod tests;
mod waiter;
#[cfg(feature = "stack-vm")]
pub(crate) use operation::{AtomicsResume, AtomicsStep};

fn with_atomics_seq_cst<R>(operation: impl FnOnce() -> R) -> R {
    waiter::with_seq_cst(operation)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtomicAccessMode {
    Operation,
    Notify,
    Wait,
}

struct AtomicAccessPreparation {
    snapshot: TypedArraySnapshot,
    buffer: BufferAccessToken,
    old_length: u32,
}

struct AtomicAccess {
    snapshot: TypedArraySnapshot,
    buffer: BufferAccessToken,
    index: u64,
}

impl Runtime {
    /// Install QuickJS's lazy global `Atomics` `JS_OBJECT_DEF` equivalent.
    pub(crate) fn initialize_atomics_intrinsic(
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
    pub(crate) fn instantiate_atomics_intrinsic(
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

    pub(crate) fn call_atomics_native(
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
    /// waiter list. `Wait` rejects an ordinary backing before index,
    /// expected-value, or timeout coercion, while a shared backing continues
    /// through the pinned wait sequence.
    fn atomics_prepare_access(
        &self,
        realm: ContextId,
        typed_array: &Value,
        mode: AtomicAccessMode,
    ) -> Result<NativeConversion<AtomicAccessPreparation>, RuntimeError> {
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
            AtomicAccessMode::Operation => atomic_element_is_integer(snapshot.element),
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
        // QuickJS performs this non-shared wait rejection before even its
        // initial detach check.
        if mode == AtomicAccessMode::Wait && !buffer_access.is_shared() {
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
        Ok(NativeConversion::Value(AtomicAccessPreparation {
            snapshot,
            buffer: buffer_access,
            old_length,
        }))
    }

    fn atomics_finish_access(
        &self,
        realm: ContextId,
        prepared: AtomicAccessPreparation,
        index: u64,
        mode: AtomicAccessMode,
    ) -> Result<NativeConversion<AtomicAccess>, RuntimeError> {
        let AtomicAccessPreparation {
            snapshot,
            buffer: buffer_access,
            old_length,
        } = prepared;
        if index >= u64::from(old_length) {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "out-of-bound access",
            )?));
        }

        if mode == AtomicAccessMode::Operation {
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

        Ok(NativeConversion::Value(AtomicAccess {
            snapshot,
            buffer: buffer_access,
            index,
        }))
    }

    /// Revalidate after operand coercion, which may detach or resize.
    fn atomics_revalidate_after_value(
        &self,
        realm: ContextId,
        access: &AtomicAccess,
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
        operation::finish(
            self,
            realm,
            operation::AtomicsStep::start(
                self,
                realm,
                AtomicsNativeKind::Operation(operation),
                arguments,
            )?,
        )
    }

    fn atomics_load(&self, access: &AtomicAccess) -> Result<Value, RuntimeError> {
        let width = usize::from(access.snapshot.element.byte_length());
        let offset = atomic_absolute_byte_offset(access)?;
        let bytes = with_atomics_seq_cst(|| self.read_buffer_word(&access.buffer, offset, width))?;
        Ok(atomic_decode_value(
            access.snapshot.element,
            atomic_decode_word(&bytes[..width]),
        ))
    }

    fn atomics_modify(
        &self,
        access: &AtomicAccess,
        operation: AtomicsOperationKind,
        operand: [u8; 8],
        replacement: Option<[u8; 8]>,
    ) -> Result<Value, RuntimeError> {
        let width = usize::from(access.snapshot.element.byte_length());
        let offset = atomic_absolute_byte_offset(access)?;
        let operand = atomic_decode_word(&operand[..width]);
        let replacement = match replacement {
            Some(bytes) => atomic_decode_word(&bytes[..width]),
            None if operation == AtomicsOperationKind::CompareExchange => {
                return Err(RuntimeError::Invariant(
                    "Atomics.compareExchange reached its byte leaf without a replacement",
                ));
            }
            None => 0,
        };
        // All JavaScript calls, conversions, allocations, and live-state
        // checks have completed before this leaf. The global gate establishes
        // one SC order across buffers; the nested backing lock makes this
        // read-modify-write indivisible without caching a raw pointer.
        let old = with_atomics_seq_cst(|| {
            self.with_buffer_range_mut(&access.buffer, offset, width, |bytes| {
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
                            replacement
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
            })
        })?;
        Ok(atomic_decode_value(access.snapshot.element, old))
    }

    fn call_atomics_store(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::finish(
            self,
            realm,
            operation::AtomicsStep::start(self, realm, AtomicsNativeKind::Store, arguments)?,
        )
    }

    fn call_atomics_is_lock_free(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::finish(
            self,
            realm,
            operation::AtomicsStep::start(self, realm, AtomicsNativeKind::IsLockFree, arguments)?,
        )
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
        operation::finish(
            self,
            realm,
            operation::AtomicsStep::start(self, realm, AtomicsNativeKind::Wait, arguments)?,
        )
    }

    fn call_atomics_notify(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::finish(
            self,
            realm,
            operation::AtomicsStep::start(self, realm, AtomicsNativeKind::Notify, arguments)?,
        )
    }
}

impl Runtime {
    fn atomics_store_converted(
        &self,
        access: &AtomicAccess,
        stored_value: Value,
        bytes: [u8; 8],
    ) -> Result<Completion, RuntimeError> {
        let width = usize::from(access.snapshot.element.byte_length());
        let offset = atomic_absolute_byte_offset(&access)?;
        with_atomics_seq_cst(|| self.write_buffer_word(&access.buffer, offset, &bytes[..width]))?;
        Ok(Completion::Return(stored_value))
    }
    fn atomics_wait_converted(
        &self,
        realm: ContextId,
        access: &AtomicAccess,
        expected: [u8; 8],
        timeout: Option<Duration>,
    ) -> Result<Completion, RuntimeError> {
        // QuickJS deliberately checks the host policy after every observable
        // conversion, even when the current memory value would be unequal.
        if !self.can_block() {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot block in this thread",
            )?));
        }

        let width = usize::from(access.snapshot.element.byte_length());
        let byte_offset = atomic_absolute_byte_offset(&access)?;
        let backing_id = access
            .buffer
            .shared_backing_id()
            .ok_or(RuntimeError::Invariant(
                "Atomics.wait acquired a non-shared backing",
            ))?;
        let location = waiter::WaitLocation::new(backing_id, byte_offset);
        let expected = atomic_decode_word(&expected[..width]);
        let outcome = waiter::wait(location, timeout, || {
            let bytes = self.read_buffer_word(&access.buffer, byte_offset, width)?;
            Ok::<bool, RuntimeError>(atomic_decode_word(&bytes[..width]) == expected)
        })?;
        let result = match outcome {
            waiter::WaitOutcome::NotEqual => "not-equal",
            waiter::WaitOutcome::Ok => "ok",
            waiter::WaitOutcome::TimedOut => "timed-out",
        };
        Ok(Completion::Return(Value::String(JsString::from_static(
            result,
        ))))
    }
    fn atomics_notify_converted(
        &self,
        access: &AtomicAccess,
        count: i32,
    ) -> Result<Completion, RuntimeError> {
        if count == 0 || !access.buffer.is_shared() {
            return Ok(Completion::Return(Value::Int(0)));
        }
        let backing_id = access
            .buffer
            .shared_backing_id()
            .ok_or(RuntimeError::Invariant(
                "shared Atomics.notify lost its backing identity",
            ))?;
        let location = waiter::WaitLocation::new(backing_id, atomic_absolute_byte_offset(&access)?);
        let notified = waiter::notify(
            location,
            usize::try_from(count).expect("clamped Atomics.notify count fits usize"),
        );
        let notified = i32::try_from(notified)
            .map_err(|_| RuntimeError::Invariant("Atomics.notify waiter count overflowed i32"))?;
        Ok(Completion::Return(Value::Int(notified)))
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

fn atomic_absolute_byte_offset(access: &AtomicAccess) -> Result<usize, RuntimeError> {
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
            Value::BigInt(crate::engine::value::bigint::JsBigInt::from(word as i64))
        }
        TypedArrayElementKind::BigUint64 => {
            Value::BigInt(crate::engine::value::bigint::JsBigInt::from(word))
        }
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

fn atomic_wait_timeout(number: f64) -> Option<Duration> {
    const INFINITE_THRESHOLD: f64 = 9_223_372_036_854_775_808.0; // 2^63

    if number.is_nan() || number >= INFINITE_THRESHOLD {
        None
    } else if number < 0.0 {
        Some(Duration::ZERO)
    } else {
        Some(Duration::from_millis(number as u64))
    }
}
