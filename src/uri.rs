//! QuickJS-compatible URI and legacy escape codecs.
//!
//! These operate on ECMAScript UTF-16 code units rather than Rust UTF-8
//! strings. That distinction is observable for lone surrogates, `%uXXXX`,
//! malformed UTF-8 percent sequences, and the reserved characters retained by
//! `decodeURI`.

use crate::value::JsString;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UriCodecError {
    ExpectingPercent,
    ExpectingHexDigit,
    MalformedUtf8,
    InvalidCharacter,
    ExpectingSurrogatePair,
}

impl UriCodecError {
    #[must_use]
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::ExpectingPercent => "expecting %",
            Self::ExpectingHexDigit => "expecting hex digit",
            Self::MalformedUtf8 => "malformed UTF-8",
            Self::InvalidCharacter => "invalid character",
            Self::ExpectingSurrogatePair => "expecting surrogate pair",
        }
    }
}

/// Decode `decodeURI` (`component == false`) or `decodeURIComponent`.
pub(crate) fn decode(input: &JsString, component: bool) -> Result<JsString, UriCodecError> {
    let units = input.utf16_units().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        if units[index] != u16::from(b'%') {
            output.push(units[index]);
            index += 1;
            continue;
        }

        let first = decode_percent_byte(&units, index)?;
        index += 3;
        if first < 0x80 {
            if !component && is_uri_reserved(first) {
                // QuickJS preserves the original hex digit spelling and case.
                output.push(u16::from(b'%'));
                output.push(units[index - 2]);
                output.push(units[index - 1]);
            } else {
                output.push(u16::from(first));
            }
            continue;
        }

        let (continuations, minimum, mut code_point) = match first {
            0xc0..=0xdf => (1, 0x80, u32::from(first & 0x1f)),
            0xe0..=0xef => (2, 0x800, u32::from(first & 0x0f)),
            0xf0..=0xf7 => (3, 0x1_0000, u32::from(first & 0x07)),
            _ => (0, 1, 0),
        };
        for _ in 0..continuations {
            let byte = decode_percent_byte(&units, index)?;
            index += 3;
            if byte & 0xc0 != 0x80 {
                code_point = 0;
                break;
            }
            code_point = (code_point << 6) | u32::from(byte & 0x3f);
        }
        if code_point < minimum || code_point > 0x10_ffff || (0xd800..=0xdfff).contains(&code_point)
        {
            return Err(UriCodecError::MalformedUtf8);
        }
        push_code_point(&mut output, code_point);
    }
    Ok(JsString::from_utf16(output))
}

/// Encode `encodeURI` (`component == false`) or `encodeURIComponent`.
pub(crate) fn encode(input: &JsString, component: bool) -> Result<JsString, UriCodecError> {
    let units = input.utf16_units().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        let first = units[index];
        index += 1;
        if is_uri_unescaped(first, component) {
            output.push(first);
            continue;
        }

        let code_point = if is_low_surrogate(first) {
            return Err(UriCodecError::InvalidCharacter);
        } else if is_high_surrogate(first) {
            let Some(&second) = units.get(index) else {
                return Err(UriCodecError::ExpectingSurrogatePair);
            };
            index += 1;
            if !is_low_surrogate(second) {
                return Err(UriCodecError::ExpectingSurrogatePair);
            }
            0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        } else {
            u32::from(first)
        };
        push_utf8_percent_encoding(&mut output, code_point);
    }
    Ok(JsString::from_utf16(output))
}

/// Apply Annex-B `escape`, preserving its code-unit-oriented `%uXXXX` form.
#[must_use]
pub(crate) fn escape(input: &JsString) -> JsString {
    let mut output = Vec::with_capacity(input.len());
    for unit in input.utf16_units() {
        if is_legacy_unescaped(unit) {
            output.push(unit);
        } else {
            push_legacy_escape(&mut output, unit);
        }
    }
    JsString::from_utf16(output)
}

/// Apply Annex-B `unescape`. Invalid escapes remain literal and never throw.
#[must_use]
pub(crate) fn unescape(input: &JsString) -> JsString {
    let units = input.utf16_units().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        if units[index] == u16::from(b'%') {
            if index + 6 <= units.len()
                && units[index + 1] == u16::from(b'u')
                && let Some(value) = decode_hex_units(&units[index + 2..index + 6])
            {
                output.push(value);
                index += 6;
                continue;
            }
            if index + 3 <= units.len()
                && let Some(value) = decode_hex_units(&units[index + 1..index + 3])
            {
                output.push(value);
                index += 3;
                continue;
            }
        }
        output.push(units[index]);
        index += 1;
    }
    JsString::from_utf16(output)
}

fn decode_percent_byte(units: &[u16], index: usize) -> Result<u8, UriCodecError> {
    if units.get(index).copied() != Some(u16::from(b'%')) {
        return Err(UriCodecError::ExpectingPercent);
    }
    let Some(digits) = units.get(index + 1..index + 3) else {
        return Err(UriCodecError::ExpectingHexDigit);
    };
    decode_hex_units(digits)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or(UriCodecError::ExpectingHexDigit)
}

fn decode_hex_units(units: &[u16]) -> Option<u16> {
    let mut value = 0_u16;
    for &unit in units {
        value = (value << 4) | u16::from(hex_value(unit)?);
    }
    Some(value)
}

fn hex_value(unit: u16) -> Option<u8> {
    match unit {
        0x30..=0x39 => Some(u8::try_from(unit - 0x30).expect("decimal hex digit fits u8")),
        0x41..=0x46 => Some(u8::try_from(unit - 0x41 + 10).expect("upper hex digit fits u8")),
        0x61..=0x66 => Some(u8::try_from(unit - 0x61 + 10).expect("lower hex digit fits u8")),
        _ => None,
    }
}

fn is_uri_reserved(byte: u8) -> bool {
    b";/?:@&=+$,#".contains(&byte)
}

fn is_uri_unescaped(unit: u16, component: bool) -> bool {
    let Ok(byte) = u8::try_from(unit) else {
        return false;
    };
    byte.is_ascii_alphanumeric()
        || b"-_.!~*'()".contains(&byte)
        || (!component && is_uri_reserved(byte))
}

fn is_legacy_unescaped(unit: u16) -> bool {
    let Ok(byte) = u8::try_from(unit) else {
        return false;
    };
    byte.is_ascii_alphanumeric() || b"@*_+-./".contains(&byte)
}

fn is_high_surrogate(unit: u16) -> bool {
    (0xd800..=0xdbff).contains(&unit)
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xdc00..=0xdfff).contains(&unit)
}

fn push_code_point(output: &mut Vec<u16>, code_point: u32) {
    if code_point < 0x1_0000 {
        output.push(u16::try_from(code_point).expect("validated BMP code point fits u16"));
    } else {
        let scalar = code_point - 0x1_0000;
        output.push(
            u16::try_from(0xd800 + (scalar >> 10))
                .expect("validated code point high surrogate fits u16"),
        );
        output.push(
            u16::try_from(0xdc00 + (scalar & 0x3ff))
                .expect("validated code point low surrogate fits u16"),
        );
    }
}

fn push_utf8_percent_encoding(output: &mut Vec<u16>, code_point: u32) {
    if code_point < 0x80 {
        push_percent_byte(
            output,
            u8::try_from(code_point).expect("ASCII code point fits u8"),
        );
    } else if code_point < 0x800 {
        push_percent_byte(
            output,
            u8::try_from((code_point >> 6) | 0xc0).expect("UTF-8 byte"),
        );
        push_percent_byte(
            output,
            u8::try_from((code_point & 0x3f) | 0x80).expect("UTF-8 byte"),
        );
    } else if code_point < 0x1_0000 {
        push_percent_byte(
            output,
            u8::try_from((code_point >> 12) | 0xe0).expect("UTF-8 byte"),
        );
        push_percent_byte(
            output,
            u8::try_from(((code_point >> 6) & 0x3f) | 0x80).expect("UTF-8 byte"),
        );
        push_percent_byte(
            output,
            u8::try_from((code_point & 0x3f) | 0x80).expect("UTF-8 byte"),
        );
    } else {
        push_percent_byte(
            output,
            u8::try_from((code_point >> 18) | 0xf0).expect("UTF-8 byte"),
        );
        push_percent_byte(
            output,
            u8::try_from(((code_point >> 12) & 0x3f) | 0x80).expect("UTF-8 byte"),
        );
        push_percent_byte(
            output,
            u8::try_from(((code_point >> 6) & 0x3f) | 0x80).expect("UTF-8 byte"),
        );
        push_percent_byte(
            output,
            u8::try_from((code_point & 0x3f) | 0x80).expect("UTF-8 byte"),
        );
    }
}

fn push_percent_byte(output: &mut Vec<u16>, byte: u8) {
    output.push(u16::from(b'%'));
    output.push(u16::from(HEX[usize::from(byte >> 4)]));
    output.push(u16::from(HEX[usize::from(byte & 0x0f)]));
}

fn push_legacy_escape(output: &mut Vec<u16>, unit: u16) {
    output.push(u16::from(b'%'));
    if unit >= 0x100 {
        output.push(u16::from(b'u'));
        output.push(u16::from(HEX[usize::from((unit >> 12) & 0x0f)]));
        output.push(u16::from(HEX[usize::from((unit >> 8) & 0x0f)]));
    }
    output.push(u16::from(HEX[usize::from((unit >> 4) & 0x0f)]));
    output.push(u16::from(HEX[usize::from(unit & 0x0f)]));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> JsString {
        JsString::from(value)
    }

    #[test]
    fn uri_ascii_sets_and_reserved_decode_split_match_quickjs() {
        assert_eq!(
            encode(&text(";/?:@&=+$,#-_.!~*'()"), false).unwrap(),
            text(";/?:@&=+$,#-_.!~*'()")
        );
        assert_eq!(
            encode(&text(";/?:@&=+$,#-_.!~*'()"), true).unwrap(),
            text("%3B%2F%3F%3A%40%26%3D%2B%24%2C%23-_.!~*'()")
        );
        assert_eq!(
            decode(&text("%2f%3F%23"), false).unwrap(),
            text("%2f%3F%23")
        );
        assert_eq!(decode(&text("%2f%3F%23"), true).unwrap(), text("/?#"));
    }

    #[test]
    fn uri_unicode_surrogates_and_malformed_sequences_are_exact() {
        let emoji = JsString::from_utf16([0xd83d, 0xde00]);
        assert_eq!(encode(&emoji, true).unwrap(), text("%F0%9F%98%80"));
        assert_eq!(decode(&text("%F0%9F%98%80"), true).unwrap(), emoji);
        let raw_lone_surrogates = JsString::from_utf16([0xd800, u16::from(b'A'), 0xdfff]);
        assert_eq!(
            decode(&raw_lone_surrogates, false).unwrap(),
            raw_lone_surrogates
        );
        assert_eq!(
            decode(&raw_lone_surrogates, true).unwrap(),
            raw_lone_surrogates
        );
        assert_eq!(
            encode(&JsString::from_utf16([0xdc00]), false),
            Err(UriCodecError::InvalidCharacter)
        );
        assert_eq!(
            encode(&JsString::from_utf16([0xd800]), false),
            Err(UriCodecError::ExpectingSurrogatePair)
        );
        assert_eq!(
            decode(&text("%E0%A0"), true),
            Err(UriCodecError::ExpectingPercent)
        );
        for malformed in ["%80", "%C0%80", "%ED%A0%80", "%F4%90%80%80", "%F8"] {
            assert_eq!(
                decode(&text(malformed), true),
                Err(UriCodecError::MalformedUtf8),
                "{malformed}"
            );
        }
    }

    #[test]
    fn legacy_escape_operates_on_code_units_and_unescape_is_permissive() {
        assert_eq!(
            escape(&text("AZaz09@*_+-./ !~'()")),
            text("AZaz09@*_+-./%20%21%7E%27%28%29")
        );
        assert_eq!(
            escape(&JsString::from_utf16([0x00e9, 0x0100, 0xd800])),
            text("%E9%u0100%uD800")
        );
        assert_eq!(
            unescape(&text("%41%u0042%uD83D%uDE00%U0043%zz%")),
            JsString::from_utf16([
                0x41, 0x42, 0xd83d, 0xde00, 0x25, 0x55, 0x30, 0x30, 0x34, 0x33, 0x25, 0x7a, 0x7a,
                0x25,
            ])
        );
    }

    #[test]
    fn legacy_escape_roundtrips_every_utf16_code_unit() {
        let all_units = JsString::from_utf16(0_u16..=u16::MAX);
        assert_eq!(unescape(&escape(&all_units)), all_units);
    }
}
