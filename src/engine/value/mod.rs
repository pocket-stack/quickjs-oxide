pub mod bigint;
pub mod number;
pub mod number_parse;
use crate::engine::api::error::{Error, ErrorKind};

use crate::engine::object::{ObjectRef, SymbolRef};
use crate::engine::value::bigint::JsBigInt;

mod primitive;
pub use primitive::*;

#[derive(Clone, Debug)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Float(f64),
    BigInt(JsBigInt),
    String(JsString),
    Symbol(SymbolRef),
    Object(ObjectRef),
}

impl Value {
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::float_cmp)]
    pub fn number(value: f64) -> Self {
        if value == f64::from(value as i32) && !is_negative_zero(value) {
            Self::Int(value as i32)
        } else {
            Self::Float(value)
        }
    }

    /// Match QuickJS's representation-only `JSValue` comparison. This is
    /// narrower than JavaScript equality: heap-backed primitives must retain
    /// the same cell and floating-point payload bits must match exactly.
    #[must_use]
    pub(crate) fn same_quickjs_representation(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Undefined, Self::Undefined) | (Self::Null, Self::Null) => true,
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::Int(left), Self::Int(right)) => left == right,
            (Self::Float(left), Self::Float(right)) => left.to_bits() == right.to_bits(),
            (Self::BigInt(left), Self::BigInt(right)) => left.same_representation(right),
            (Self::String(left), Self::String(right)) => left.same_representation(right),
            (Self::Symbol(left), Self::Symbol(right)) => left == right,
            (Self::Object(left), Self::Object(right)) => left == right,
            _ => false,
        }
    }

    /// Apply ECMAScript `ToBoolean`, including QuickJS's Annex B
    /// `is_HTMLDDA` object exception.
    #[must_use]
    pub fn to_boolean(&self) -> bool {
        let Self::Object(object) = self else {
            return self.to_boolean_primitive();
        };
        !object
            .runtime()
            .value_is_html_dda(self)
            .expect("a rooted ObjectRef must resolve in its owning runtime")
    }

    /// Apply the representation-only primitive portion of `ToBoolean`.
    /// Runtime internals use this only after checking object metadata through
    /// the owning runtime.
    #[must_use]
    pub(crate) fn to_boolean_primitive(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::Int(value) => *value != 0,
            Self::Float(value) => *value != 0.0 && !value.is_nan(),
            Self::BigInt(value) => !value.is_zero(),
            Self::String(value) => !value.is_empty(),
            Self::Symbol(_) | Self::Object(_) => true,
            Self::Undefined | Self::Null => false,
        }
    }

    #[must_use]
    pub const fn as_number(&self) -> Option<f64> {
        match self {
            Self::Int(value) => Some(*value as f64),
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }

    /// Apply ECMAScript `ToNumber` to the value kinds implemented by the
    /// runtime kernel.
    ///
    /// # Errors
    /// Symbol and BigInt conversion throw, while object conversion must be
    /// routed through a context so `ToPrimitive` can execute user code.
    pub fn to_number(&self) -> Result<f64, Error> {
        match self {
            Self::Undefined => Ok(f64::NAN),
            Self::Null => Ok(0.0),
            Self::Bool(value) => Ok(f64::from(u8::from(*value))),
            Self::Int(value) => Ok(f64::from(*value)),
            Self::Float(value) => Ok(*value),
            Self::BigInt(_) => Err(Error::new(
                ErrorKind::Type,
                "cannot convert bigint to number",
            )),
            Self::String(value) => Ok(string_to_number(value)),
            Self::Symbol(_) => Err(Error::new(
                ErrorKind::Type,
                "cannot convert symbol to number",
            )),
            Self::Object(_) => Err(Error::new(
                ErrorKind::Internal,
                "object ToPrimitive requires an execution context",
            )),
        }
    }

    /// Apply ECMAScript `ToString` to the value kinds implemented by the
    /// runtime kernel.
    ///
    /// # Errors
    /// Symbol conversion throws, an extended-limit BigInt can fail the pinned
    /// QuickJS decimal-conversion allocation guard, and object conversion must
    /// be routed through a context so `ToPrimitive` can execute user code.
    pub fn to_js_string(&self) -> Result<JsString, Error> {
        let text = match self {
            Self::Undefined => "undefined".to_owned(),
            Self::Null => "null".to_owned(),
            Self::Bool(false) => "false".to_owned(),
            Self::Bool(true) => "true".to_owned(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => number_to_string(*value),
            Self::BigInt(value) => {
                if value.exceeds_allocation_limit() {
                    return Err(Error::new(
                        ErrorKind::Range,
                        "BigInt is too large to allocate",
                    ));
                }
                value.to_string()
            }
            Self::String(value) => return Ok(value.linearize()),
            Self::Symbol(_) => {
                return Err(Error::new(
                    ErrorKind::Type,
                    "cannot convert symbol to string",
                ));
            }
            Self::Object(_) => {
                return Err(Error::new(
                    ErrorKind::Internal,
                    "object ToPrimitive requires an execution context",
                ));
            }
        };
        Ok(JsString::try_from_utf8(&text)?)
    }

    #[must_use]
    #[allow(clippy::float_cmp)]
    pub fn strict_equal(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Undefined, Self::Undefined) | (Self::Null, Self::Null) => true,
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Symbol(left), Self::Symbol(right)) => left == right,
            (Self::Object(left), Self::Object(right)) => left == right,
            (Self::BigInt(left), Self::BigInt(right)) => left == right,
            (Self::Int(left), Self::Int(right)) => left == right,
            (left, right) => match (left.as_number(), right.as_number()) {
                (Some(left), Some(right)) => left == right,
                _ => false,
            },
        }
    }

    #[must_use]
    #[allow(clippy::float_cmp)]
    pub fn same_value(&self, other: &Self) -> bool {
        match (self.as_number(), other.as_number()) {
            (Some(left), Some(right)) if left.is_nan() && right.is_nan() => true,
            (Some(left), Some(right)) if left == 0.0 && right == 0.0 => {
                is_negative_zero(left) == is_negative_zero(right)
            }
            (Some(left), Some(right)) => left == right,
            _ => self.strict_equal(other),
        }
    }

    #[must_use]
    #[allow(clippy::float_cmp)]
    pub fn same_value_zero(&self, other: &Self) -> bool {
        match (self.as_number(), other.as_number()) {
            (Some(left), Some(right)) if left.is_nan() && right.is_nan() => true,
            (Some(left), Some(right)) => left == right,
            _ => self.strict_equal(other),
        }
    }

    #[must_use]
    /// Return the representation-only `typeof` tag.
    ///
    /// Object callability is runtime metadata, so the VM refines the object
    /// case through its runtime host and returns `"function"` for callables.
    pub const fn type_of(&self) -> &'static str {
        match self {
            Self::Null => "object",
            Self::Bool(_) => "boolean",
            Self::Int(_) | Self::Float(_) => "number",
            Self::BigInt(_) => "bigint",
            Self::String(_) => "string",
            Self::Symbol(_) => "symbol",
            Self::Object(_) => "object",
            Self::Undefined => "undefined",
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.strict_equal(other)
    }
}

impl From<PrimitiveValue> for Value {
    fn from(value: PrimitiveValue) -> Self {
        match value {
            PrimitiveValue::Undefined => Value::Undefined,
            PrimitiveValue::Null => Value::Null,
            PrimitiveValue::Bool(value) => Value::Bool(value),
            PrimitiveValue::Int(value) => Value::Int(value),
            PrimitiveValue::Float(value) => Value::Float(value),
            PrimitiveValue::BigInt(value) => Value::BigInt(value),
            PrimitiveValue::String(value) => Value::String(value),
        }
    }
}

impl TryFrom<Value> for PrimitiveValue {
    type Error = crate::engine::code::function::UnlinkedConstantError;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Undefined => Ok(PrimitiveValue::Undefined),
            Value::Null => Ok(PrimitiveValue::Null),
            Value::Bool(value) => Ok(PrimitiveValue::Bool(value)),
            Value::Int(value) => Ok(PrimitiveValue::Int(value)),
            Value::Float(value) => Ok(PrimitiveValue::Float(value)),
            Value::BigInt(value) => Ok(PrimitiveValue::BigInt(value)),
            Value::String(value) => Ok(PrimitiveValue::String(value)),
            Value::Object(_) => Err(Self::Error::RuntimeBoundObject),
            Value::Symbol(_) => Err(Self::Error::RuntimeBoundSymbol),
        }
    }
}
impl PartialEq<Value> for PrimitiveValue {
    fn eq(&self, other: &Value) -> bool {
        Value::from(self.clone()).eq(other)
    }
}
impl PartialEq<PrimitiveValue> for Value {
    fn eq(&self, other: &PrimitiveValue) -> bool {
        self.eq(&Value::from(other.clone()))
    }
}

pub(crate) mod conversion;

#[cfg(test)]
impl crate::engine::code::bytecode::TestConstant for Value {
    fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }
}
