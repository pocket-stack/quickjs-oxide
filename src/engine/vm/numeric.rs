pub(super) mod operation;
use crate::engine::{
    api::{Error, ErrorKind},
    api::runtime::Runtime,
    heap::{BigIntId, StringId},
    value::{
        JsString, JsValue,
        bigint::{BigIntError, JsBigInt},
    },
};
use num_bigint::BigInt;
use num_traits::FromPrimitive;

pub(in crate::engine::vm) enum NumericValue {
    Number(f64),
    BigInt(JsBigInt),
}

/// Read one string node's payload. The borrowed value keeps its edge.
pub(in crate::engine::vm) fn string_payload(
    runtime: &Runtime,
    id: StringId,
) -> Result<JsString, Error> {
    Ok(runtime
        .0
        .state
        .borrow()
        .heap
        .string(id)
        .map_err(|error| Error::internal(error.to_string()))?
        .clone())
}

/// Read one BigInt node's payload. The borrowed value keeps its edge.
pub(in crate::engine::vm) fn bigint_payload(
    runtime: &Runtime,
    id: BigIntId,
) -> Result<JsBigInt, Error> {
    Ok(runtime
        .0
        .state
        .borrow()
        .heap
        .bigint(id)
        .map_err(|error| Error::internal(error.to_string()))?
        .clone())
}

/// Publish a freshly produced string payload as an owned internal value.
/// Concatenation and primitive formatting are genuine string creation points.
pub(in crate::engine::vm) fn allocate_string_jsvalue(
    runtime: &Runtime,
    string: JsString,
) -> Result<JsValue, Error> {
    let id = runtime
        .0
        .state
        .borrow_mut()
        .heap
        .allocate_string(string)
        .map_err(|error| Error::internal(error.to_string()))?;
    Ok(JsValue::String(id))
}

/// Publish a freshly produced BigInt payload as an owned internal value.
/// BigInt arithmetic results are genuine BigInt creation points.
pub(in crate::engine::vm) fn allocate_bigint_jsvalue(
    runtime: &Runtime,
    bigint: JsBigInt,
) -> Result<JsValue, Error> {
    let id = runtime
        .0
        .state
        .borrow_mut()
        .heap
        .allocate_bigint(bigint)
        .map_err(|error| Error::internal(error.to_string()))?;
    Ok(JsValue::BigInt(id))
}

/// Representation-only `ToNumber` for internal values. Object conversion must
/// be routed through a context; Symbol and BigInt conversion throw here.
pub(in crate::engine::vm) fn to_number_jsvalue(
    runtime: &Runtime,
    value: &JsValue,
) -> Result<f64, Error> {
    Ok(match value {
        JsValue::Undefined => f64::NAN,
        JsValue::Null => 0.0,
        JsValue::Bool(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        JsValue::Int(value) => f64::from(*value),
        JsValue::Float(value) => *value,
        JsValue::String(id) => string_payload(runtime, *id)?.to_number()?,
        JsValue::BigInt(_) => {
            return Err(Error::new(ErrorKind::Type, "cannot convert bigint to number"));
        }
        JsValue::Symbol(_) => {
            return Err(Error::new(
                ErrorKind::Type,
                "cannot convert symbol to number",
            ));
        }
        JsValue::Object(_) => {
            return Err(Error::internal(
                "object ToNumber requires an execution context",
            ));
        }
    })
}

/// Primitive `ToString` payload for internal values (no object conversion).
pub(crate) fn to_js_string_jsvalue(
    runtime: &Runtime,
    value: &JsValue,
) -> Result<JsString, Error> {
    Ok(match value {
        JsValue::String(id) => string_payload(runtime, *id)?,
        JsValue::Undefined => JsString::from_static("undefined"),
        JsValue::Null => JsString::from_static("null"),
        JsValue::Bool(true) => JsString::from_static("true"),
        JsValue::Bool(false) => JsString::from_static("false"),
        value => crate::engine::value::Value::number(to_number_jsvalue(runtime, value)?)
            .to_js_string()?,
    })
}

pub(in crate::engine::vm) fn to_numeric_primitive(
    runtime: &Runtime,
    value: &JsValue,
) -> Result<NumericValue, Error> {
    match value {
        JsValue::BigInt(id) => Ok(NumericValue::BigInt(bigint_payload(runtime, *id)?)),
        value => Ok(NumericValue::Number(to_number_jsvalue(runtime, value)?)),
    }
}

/// OP_plus after ToPrimitive: preserve numeric tags and its specific BigInt
/// diagnostic. This step cannot invoke user code.
pub(in crate::engine::vm) fn unary_plus_primitive(
    runtime: &Runtime,
    value: JsValue,
) -> Result<JsValue, Error> {
    match value {
        JsValue::BigInt(_) => Err(Error::new(ErrorKind::Type, "bigint argument with unary +")),
        value @ (JsValue::Int(_) | JsValue::Float(_)) => Ok(value),
        value => Ok(jsvalue_number(to_number_jsvalue(runtime, &value)?)),
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

/// Compact a numeric payload into the internal number representation.
pub(in crate::engine::vm) fn jsvalue_number(value: f64) -> JsValue {
    jsvalue_from_number(crate::engine::value::number::operations::Number::compact(value))
}

/// Project an already-compacted numeric representation without recompacting.
pub(in crate::engine::vm) fn jsvalue_from_number(
    number: crate::engine::value::number::operations::Number,
) -> JsValue {
    match number {
        crate::engine::value::number::operations::Number::Int(value) => JsValue::Int(value),
        crate::engine::value::number::operations::Number::Float(value) => JsValue::Float(value),
    }
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
pub(in crate::engine::vm) fn add_primitives(
    runtime: &Runtime,
    left: JsValue,
    right: JsValue,
) -> Result<JsValue, Error> {
    if matches!(left, JsValue::String(_)) || matches!(right, JsValue::String(_)) {
        let left = match left {
            JsValue::String(id) => string_payload(runtime, id)?,
            value => to_js_string_jsvalue(runtime, &value)?,
        };
        let right = match right {
            JsValue::String(id) => string_payload(runtime, id)?,
            value => to_js_string_jsvalue(runtime, &value)?,
        };
        return allocate_string_jsvalue(runtime, left.concat_owned(&right)?);
    }
    add_primitives_ref(runtime, &left, &right)
}

/// Same primitive kernel with owners retained by the caller. No user code can
/// execute; callers may borrow frame locals without cloning temporary roots.
pub(in crate::engine::vm) fn add_primitives_ref(
    runtime: &Runtime,
    left: &JsValue,
    right: &JsValue,
) -> Result<JsValue, Error> {
    if matches!(left, JsValue::String(_)) || matches!(right, JsValue::String(_)) {
        let left = match left {
            JsValue::String(id) => string_payload(runtime, *id)?,
            value => to_js_string_jsvalue(runtime, value)?,
        };
        let right = match right {
            JsValue::String(id) => string_payload(runtime, *id)?,
            value => to_js_string_jsvalue(runtime, value)?,
        };
        return allocate_string_jsvalue(runtime, left.try_concat(&right).map_err(Error::from)?);
    }
    match (left, right) {
        (JsValue::BigInt(left), JsValue::BigInt(right)) => {
            let left = bigint_payload(runtime, *left)?;
            let right = bigint_payload(runtime, *right)?;
            allocate_bigint_jsvalue(runtime, left.add(&right).map_err(bigint_error)?)
        }
        (JsValue::BigInt(_), right) => {
            to_number_jsvalue(runtime, right)?;
            Err(mixed_numeric_type_error())
        }
        (left, JsValue::BigInt(_)) => {
            to_number_jsvalue(runtime, left)?;
            Err(mixed_numeric_type_error())
        }
        (left, right) => {
            let left = to_number_jsvalue(runtime, left)?;
            let right = to_number_jsvalue(runtime, right)?;
            Ok(jsvalue_number(left + right))
        }
    }
}
