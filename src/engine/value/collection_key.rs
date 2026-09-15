//! Pure SameValueZero key operations on validated, runtime-local raw values.
//!
//! No rooting, heap access, coercion or user callbacks occur here. Callers must
//! reject internal sentinels and foreign identities at their storage boundary.

use crate::engine::heap::RawValue;
use std::hash::{Hash, Hasher};

fn number(value: &RawValue) -> Option<f64> {
    match value {
        RawValue::Int(value) => Some(f64::from(*value)),
        RawValue::Float(value) => Some(*value),
        _ => None,
    }
}

pub(crate) fn same_value_zero(left: &RawValue, right: &RawValue) -> bool {
    if let (Some(left), Some(right)) = (number(left), number(right)) {
        return left == right || (left.is_nan() && right.is_nan());
    }
    match (left, right) {
        (RawValue::Undefined, RawValue::Undefined) | (RawValue::Null, RawValue::Null) => true,
        (RawValue::Bool(left), RawValue::Bool(right)) => left == right,
        (RawValue::String(left), RawValue::String(right)) => left == right,
        (RawValue::BigInt(left), RawValue::BigInt(right)) => left == right,
        (RawValue::Symbol(left), RawValue::Symbol(right)) => left == right,
        (RawValue::Object(left), RawValue::Object(right)) => left == right,
        _ => false,
    }
}

pub(crate) fn hash<H: Hasher>(key: &RawValue, state: &mut H) {
    match key {
        RawValue::Undefined => state.write_u8(0),
        RawValue::Null => state.write_u8(1),
        RawValue::Bool(value) => {
            state.write_u8(2);
            value.hash(state);
        }
        RawValue::Int(_) | RawValue::Float(_) => {
            state.write_u8(3);
            let value = number(key).expect("numeric key variant");
            // Canonicalize both zero signs and every NaN payload. Int and Float
            // representations of the same Number must select the same bucket.
            let bits = if value == 0.0 {
                0
            } else if value.is_nan() {
                f64::NAN.to_bits()
            } else {
                value.to_bits()
            };
            state.write_u64(bits);
        }
        RawValue::String(value) => {
            state.write_u8(4);
            value.len().hash(state);
            // Hash actual content with the index's randomized hasher, not the
            // existing unseeded 32-bit content fingerprint.
            value.hash_code_units(state);
        }
        RawValue::BigInt(value) => {
            state.write_u8(5);
            value.hash(state);
        }
        RawValue::Symbol(value) => {
            state.write_u8(6);
            value.hash(state);
        }
        RawValue::Object(value) => {
            state.write_u8(7);
            value.hash(state);
        }
        RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception => {
            unreachable!("internal sentinel reached a validated collection key")
        }
    }
}
