pub(super) mod operation;
#[cfg(test)]
use super::{Completion, ToPrimitiveHint, VmHost};
use crate::engine::{
    api::{Error, ErrorKind},
    value::{
        Value,
        bigint::{BigIntError, JsBigInt},
    },
};
use num_bigint::BigInt;
use num_traits::FromPrimitive;

pub(in crate::engine::vm) enum NumericValue {
    Number(f64),
    BigInt(JsBigInt),
}

/// Apply ToPrimitive at the VM boundary. Primitive operands keep their exact
/// representation and need no host services; only objects can execute user code.
#[cfg(test)]
#[inline]
pub(in crate::engine::vm) fn to_primitive(
    host: &mut impl VmHost,
    value: Value,
    hint: ToPrimitiveHint,
) -> Result<Completion, Error> {
    match value {
        Value::Object(_) => host.to_primitive(value, hint),
        primitive => Ok(Completion::Return(primitive)),
    }
}

pub(in crate::engine::vm) fn to_numeric_primitive(value: Value) -> Result<NumericValue, Error> {
    match value {
        Value::BigInt(value) => Ok(NumericValue::BigInt(value)),
        value => Ok(NumericValue::Number(value.to_number()?)),
    }
}

/// OP_plus after ToPrimitive: preserve numeric tags and its specific BigInt
/// diagnostic. This step cannot invoke user code.
pub(in crate::engine::vm) fn unary_plus_primitive(value: Value) -> Result<Value, Error> {
    match value {
        Value::BigInt(_) => Err(Error::new(ErrorKind::Type, "bigint argument with unary +")),
        value @ (Value::Int(_) | Value::Float(_)) => Ok(value),
        value => Ok(Value::number(value.to_number()?)),
    }
}

/// ECMAScript `ToInt32`, matching QuickJS's modulo-2^32 conversion for every
/// finite IEEE-754 input and its zero result for NaN and infinities.
pub(in crate::engine::vm) fn number_to_int32(value: f64) -> i32 {
    crate::engine::value::number::to_int32(value)
}

pub(in crate::engine::vm) fn number_to_uint32(value: f64) -> u32 {
    u32::from_ne_bytes(number_to_int32(value).to_ne_bytes())
}

pub(in crate::engine::vm) fn compare_bigint_number(
    bigint: &JsBigInt,
    number: f64,
) -> Option<std::cmp::Ordering> {
    if number.is_nan() {
        return None;
    }
    if number == f64::INFINITY {
        return Some(std::cmp::Ordering::Less);
    }
    if number == f64::NEG_INFINITY {
        return Some(std::cmp::Ordering::Greater);
    }

    let truncated = BigInt::from_f64(number.trunc())?;
    let ordering = bigint.to_bigint().cmp(&truncated);
    if !ordering.is_eq() {
        return Some(ordering);
    }
    if number.fract().is_sign_positive() && number.fract() != 0.0 {
        Some(std::cmp::Ordering::Less)
    } else if number.fract().is_sign_negative() && number.fract() != 0.0 {
        Some(std::cmp::Ordering::Greater)
    } else {
        Some(std::cmp::Ordering::Equal)
    }
}

pub(in crate::engine::vm) fn string_to_bigint(
    value: &crate::engine::value::JsString,
) -> Option<JsBigInt> {
    let text = String::from_utf16(&value.utf16_units().collect::<Vec<_>>()).ok()?;
    JsBigInt::parse_js_string(&text).ok()
}

pub(in crate::engine::vm) fn mixed_numeric_type_error() -> Error {
    Error::new(ErrorKind::Type, "cannot convert bigint to number")
}

pub(in crate::engine::vm) fn bigint_error(error: BigIntError) -> Error {
    let message = match error {
        BigIntError::ShiftTooLarge => "BigInt is too large to allocate".to_owned(),
        error => error.to_string(),
    };
    Error::new(ErrorKind::Range, message)
}

/// Addition after both operands have completed ToPrimitive, in order.
pub(in crate::engine::vm) fn add_primitives(left: Value, right: Value) -> Result<Value, Error> {
    add_primitives_ref(&left, &right)
}

/// Same primitive kernel with owners retained by the caller. No user code can
/// execute; callers may borrow frame locals without cloning temporary roots.
pub(in crate::engine::vm) fn add_primitives_ref(
    left: &Value,
    right: &Value,
) -> Result<Value, Error> {
    if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
        use std::borrow::Cow;
        let left = match left {
            Value::String(value) => Cow::Borrowed(value),
            value => Cow::Owned(value.to_js_string()?),
        };
        let right = match right {
            Value::String(value) => Cow::Borrowed(value),
            value => Cow::Owned(value.to_js_string()?),
        };
        return Ok(Value::String(left.try_concat(&right).map_err(Error::from)?));
    }
    match (left, right) {
        (Value::BigInt(left), Value::BigInt(right)) => {
            Ok(Value::BigInt(left.add(right).map_err(bigint_error)?))
        }
        (Value::BigInt(_), right) => {
            right.to_number()?;
            Err(mixed_numeric_type_error())
        }
        (left, Value::BigInt(_)) => {
            left.to_number()?;
            Err(mixed_numeric_type_error())
        }
        (left, right) => {
            let left = left.to_number()?;
            let right = right.to_number()?;
            Ok(Value::number(left + right))
        }
    }
}
