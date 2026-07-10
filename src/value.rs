use std::fmt;
use std::rc::Rc;

use num_bigint::BigUint;
use num_traits::ToPrimitive;

use crate::bigint::JsBigInt;
use crate::error::{Error, ErrorKind};
use crate::object::{ObjectRef, SymbolRef};

/// ECMAScript strings are sequences of UTF-16 code units, not UTF-8 scalar
/// values. Keeping Latin-1 and UTF-16 representations mirrors `QuickJS`'s compact
/// 8/16-bit strings while preserving lone surrogates.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct JsString(Rc<StringRepr>);

#[derive(Eq, Hash, PartialEq)]
enum StringRepr {
    Latin1(Box<[u8]>),
    Utf16(Box<[u16]>),
}

impl JsString {
    #[must_use]
    pub fn from_utf8(value: &str) -> Self {
        Self::from_utf16(value.encode_utf16())
    }

    #[must_use]
    pub fn from_utf16(units: impl IntoIterator<Item = u16>) -> Self {
        let units: Vec<u16> = units.into_iter().collect();
        let latin1 = units
            .iter()
            .copied()
            .map(u8::try_from)
            .collect::<Result<Vec<_>, _>>();
        match latin1 {
            Ok(latin1) => Self(Rc::new(StringRepr::Latin1(latin1.into_boxed_slice()))),
            Err(_) => Self(Rc::new(StringRepr::Utf16(units.into_boxed_slice()))),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        match self.0.as_ref() {
            StringRepr::Latin1(units) => units.len(),
            StringRepr::Utf16(units) => units.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn utf16_units(&self) -> impl ExactSizeIterator<Item = u16> + '_ {
        enum Units<'a> {
            Latin1(std::slice::Iter<'a, u8>),
            Utf16(std::slice::Iter<'a, u16>),
        }

        impl Iterator for Units<'_> {
            type Item = u16;

            fn next(&mut self) -> Option<Self::Item> {
                match self {
                    Self::Latin1(iter) => iter.next().map(|unit| u16::from(*unit)),
                    Self::Utf16(iter) => iter.next().copied(),
                }
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                match self {
                    Self::Latin1(iter) => iter.size_hint(),
                    Self::Utf16(iter) => iter.size_hint(),
                }
            }
        }

        impl ExactSizeIterator for Units<'_> {}

        match self.0.as_ref() {
            StringRepr::Latin1(units) => Units::Latin1(units.iter()),
            StringRepr::Utf16(units) => Units::Utf16(units.iter()),
        }
    }

    /// Lossy conversion is suitable for terminal diagnostics. It must not be
    /// used for language-level string comparison or indexing.
    #[must_use]
    pub fn to_utf8_lossy(&self) -> String {
        char::decode_utf16(self.utf16_units())
            .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    }

    #[must_use]
    pub fn concat(&self, other: &Self) -> Self {
        Self::from_utf16(self.utf16_units().chain(other.utf16_units()))
    }
}

impl From<&str> for JsString {
    fn from(value: &str) -> Self {
        Self::from_utf8(value)
    }
}

impl fmt::Debug for JsString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("JsString")
            .field(&self.to_utf8_lossy())
            .finish()
    }
}

impl fmt::Display for JsString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_utf8_lossy())
    }
}

/// The currently materialized value tags follow `QuickJS`'s split between
/// immediate 32-bit integers and IEEE-754 doubles. Heap-backed tags are added
/// through the runtime heap rather than by changing source semantics.
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

    #[must_use]
    pub fn to_boolean(&self) -> bool {
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
    /// Symbol conversion throws, while object conversion must be routed
    /// through a context so `ToPrimitive` can execute user code.
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
    /// Symbol conversion throws, while object conversion must be routed
    /// through a context so `ToPrimitive` can execute user code.
    pub fn to_js_string(&self) -> Result<JsString, Error> {
        let text = match self {
            Self::Undefined => "undefined".to_owned(),
            Self::Null => "null".to_owned(),
            Self::Bool(false) => "false".to_owned(),
            Self::Bool(true) => "true".to_owned(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => number_to_string(*value),
            Self::BigInt(value) => value.to_string(),
            Self::String(value) => return Ok(value.clone()),
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
        Ok(JsString::from_utf8(&text))
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

#[must_use]
pub fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value == f64::INFINITY {
        return "Infinity".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-Infinity".to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    ryu_js::Buffer::new().format(value).to_owned()
}

fn string_to_number(value: &JsString) -> f64 {
    let units = value.utf16_units().collect::<Vec<_>>();
    let mut start = 0;
    let mut end = units.len();
    while start < end && is_ecmascript_whitespace(units[start]) {
        start += 1;
    }
    while end > start && is_ecmascript_whitespace(units[end - 1]) {
        end -= 1;
    }
    if start == end {
        return 0.0;
    }

    let Ok(text) = String::from_utf16(&units[start..end]) else {
        return f64::NAN;
    };
    match text.as_str() {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }

    if let Some(digits) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return parse_radix_number(digits, 16);
    }
    if let Some(digits) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) {
        return parse_radix_number(digits, 8);
    }
    if let Some(digits) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
        return parse_radix_number(digits, 2);
    }
    if is_decimal_number_text(&text) {
        text.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

fn parse_radix_number(digits: &str, radix: u32) -> f64 {
    if digits.is_empty()
        || !digits
            .bytes()
            .all(|byte| ascii_digit_value(byte).is_some_and(|digit| digit < radix))
    {
        return f64::NAN;
    }
    BigUint::parse_bytes(digits.as_bytes(), radix)
        .and_then(|value| value.to_f64())
        .unwrap_or(f64::NAN)
}

const fn ascii_digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some((byte - b'0') as u32),
        b'a'..=b'f' => Some((byte - b'a' + 10) as u32),
        b'A'..=b'F' => Some((byte - b'A' + 10) as u32),
        _ => None,
    }
}

fn is_decimal_number_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let mut integer_digits = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
        integer_digits += 1;
    }

    let mut fractional_digits = 0;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
            fractional_digits += 1;
        }
    }
    if integer_digits + fractional_digits == 0 {
        return false;
    }

    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

const fn is_ecmascript_whitespace(unit: u16) -> bool {
    matches!(
        unit,
        0x0009..=0x000d
            | 0x0020
            | 0x00a0
            | 0x1680
            | 0x2000..=0x200a
            | 0x2028
            | 0x2029
            | 0x202f
            | 0x205f
            | 0x3000
            | 0xfeff
    )
}

#[allow(clippy::float_cmp)]
fn is_negative_zero(value: f64) -> bool {
    value == 0.0 && value.is_sign_negative()
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.strict_equal(other)
    }
}

#[cfg(test)]
mod tests {
    use super::{JsString, Value, number_to_string};
    use crate::bigint::JsBigInt;

    #[test]
    fn string_length_counts_utf16_code_units() {
        let text = JsString::from("a🚀");
        assert_eq!(text.len(), 3);
        assert_eq!(
            text.utf16_units().collect::<Vec<_>>(),
            vec![0x61, 0xd83d, 0xde80]
        );
    }

    #[test]
    fn strings_preserve_lone_surrogates() {
        let text = JsString::from_utf16([0xd800, 0x61]);
        assert_eq!(text.utf16_units().collect::<Vec<_>>(), vec![0xd800, 0x61]);
        assert_eq!(text.to_utf8_lossy(), "�a");
    }

    #[test]
    fn number_uses_int_fast_path_without_losing_negative_zero() {
        assert!(matches!(Value::number(42.0), Value::Int(42)));
        assert!(matches!(Value::number(-0.0), Value::Float(value) if value.is_sign_negative()));
    }

    #[test]
    fn equality_variants_handle_nan_and_zero() {
        let nan = Value::Float(f64::NAN);
        assert!(!nan.strict_equal(&nan));
        assert!(nan.same_value(&nan));
        assert!(nan.same_value_zero(&nan));

        let positive_zero = Value::Int(0);
        let negative_zero = Value::Float(-0.0);
        assert!(positive_zero.strict_equal(&negative_zero));
        assert!(!positive_zero.same_value(&negative_zero));
        assert!(positive_zero.same_value_zero(&negative_zero));
    }

    #[test]
    fn primitive_coercions_follow_ecmascript() {
        assert_eq!(
            Value::String(JsString::from("  \u{feff}  "))
                .to_number()
                .unwrap(),
            0.0
        );
        assert_eq!(
            Value::String(JsString::from("0xff")).to_number().unwrap(),
            255.0
        );
        assert!(
            Value::String(JsString::from("-0x1"))
                .to_number()
                .unwrap()
                .is_nan()
        );
        for invalid in ["0x1_", "0x+1", "0b1_", "0o7_"] {
            assert!(
                Value::String(JsString::from(invalid))
                    .to_number()
                    .unwrap()
                    .is_nan(),
                "{invalid}"
            );
        }
        assert_eq!(Value::Bool(true).to_number().unwrap(), 1.0);
        assert_eq!(Value::Null.to_number().unwrap(), 0.0);
        assert!(Value::Undefined.to_number().unwrap().is_nan());
    }

    #[test]
    fn number_formatting_uses_ecmascript_thresholds() {
        assert_eq!(number_to_string(-0.0), "0");
        assert_eq!(number_to_string(f64::NAN), "NaN");
        assert_eq!(number_to_string(1e20), "100000000000000000000");
        assert_eq!(number_to_string(1e21), "1e+21");
    }

    #[test]
    fn bigint_has_distinct_primitive_coercion_rules() {
        let zero = Value::BigInt(JsBigInt::zero());
        let one = Value::BigInt(JsBigInt::one());
        assert!(!zero.to_boolean());
        assert!(one.to_boolean());
        assert!(one.to_number().is_err());
        assert_eq!(one.to_js_string().unwrap(), JsString::from("1"));
        assert_eq!(one.type_of(), "bigint");
    }
}
