use super::*;

pub(in crate::engine::vm) enum NumericValue {
    Number(f64),
    BigInt(JsBigInt),
}

pub(in crate::engine::vm) fn abstract_equal(
    host: &mut impl VmHost,
    mut left: Value,
    mut right: Value,
) -> Result<OperationOutcome<bool>, Error> {
    loop {
        if left.strict_equal(&right) {
            return Ok(OperationOutcome::Value(true));
        }
        if (matches!(right, Value::Null | Value::Undefined) && host.is_html_dda(&left)?)
            || (matches!(left, Value::Null | Value::Undefined) && host.is_html_dda(&right)?)
        {
            return Ok(OperationOutcome::Value(true));
        }
        match (&left, &right) {
            (Value::Null, Value::Undefined) | (Value::Undefined, Value::Null) => {
                return Ok(OperationOutcome::Value(true));
            }
            (Value::Int(_) | Value::Float(_), Value::String(_)) => {
                right = Value::number(right.to_number()?);
            }
            (Value::String(_), Value::Int(_) | Value::Float(_)) => {
                left = Value::number(left.to_number()?);
            }
            (Value::BigInt(left_bigint), Value::String(right_string)) => {
                return Ok(OperationOutcome::Value(
                    string_to_bigint(right_string).is_some_and(|right| &right == left_bigint),
                ));
            }
            (Value::String(left_string), Value::BigInt(right_bigint)) => {
                return Ok(OperationOutcome::Value(
                    string_to_bigint(left_string).is_some_and(|left| &left == right_bigint),
                ));
            }
            (Value::BigInt(left_bigint), Value::Int(_) | Value::Float(_)) => {
                return Ok(OperationOutcome::Value(
                    compare_bigint_number(left_bigint, right.to_number()?)
                        == Some(std::cmp::Ordering::Equal),
                ));
            }
            (Value::Int(_) | Value::Float(_), Value::BigInt(right_bigint)) => {
                return Ok(OperationOutcome::Value(
                    compare_bigint_number(right_bigint, left.to_number()?)
                        == Some(std::cmp::Ordering::Equal),
                ));
            }
            (Value::Bool(_), _) => left = Value::number(left.to_number()?),
            (_, Value::Bool(_)) => right = Value::number(right.to_number()?),
            (
                Value::Object(_),
                Value::Int(_)
                | Value::Float(_)
                | Value::BigInt(_)
                | Value::String(_)
                | Value::Symbol(_),
            ) => match host.to_primitive(left, ToPrimitiveHint::Default)? {
                Completion::Return(value) => left = value,
                Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
            },
            (
                Value::Int(_)
                | Value::Float(_)
                | Value::BigInt(_)
                | Value::String(_)
                | Value::Symbol(_),
                Value::Object(_),
            ) => match host.to_primitive(right, ToPrimitiveHint::Default)? {
                Completion::Return(value) => right = value,
                Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
            },
            _ => return Ok(OperationOutcome::Value(false)),
        }
    }
}

pub(in crate::engine::vm) fn to_numeric(
    host: &mut impl VmHost,
    value: Value,
) -> Result<OperationOutcome<NumericValue>, Error> {
    match host.to_primitive(value, ToPrimitiveHint::Number)? {
        Completion::Return(value) => Ok(OperationOutcome::Value(to_numeric_primitive(value)?)),
        Completion::Throw(value) => Ok(OperationOutcome::Throw(value)),
    }
}

pub(in crate::engine::vm) fn to_numeric_primitive(value: Value) -> Result<NumericValue, Error> {
    match value {
        Value::BigInt(value) => Ok(NumericValue::BigInt(value)),
        value => Ok(NumericValue::Number(value.to_number()?)),
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
