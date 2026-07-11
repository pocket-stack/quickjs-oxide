//! QuickJS-compatible URI and legacy escape codecs.
//!
//! These operate on ECMAScript UTF-16 code units rather than Rust UTF-8
//! strings. That distinction is observable for lone surrogates, `%uXXXX`,
//! malformed UTF-8 percent sequences, and the reserved characters retained by
//! `decodeURI`.

use crate::value::{JsString, JsStringBuilder, JsStringError};

const HEX: &[u8; 16] = b"0123456789ABCDEF";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UriCodecError {
    ExpectingPercent,
    ExpectingHexDigit,
    MalformedUtf8,
    InvalidCharacter,
    ExpectingSurrogatePair,
    String(JsStringError),
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
            Self::String(JsStringError::TooLong) => "string too long",
        }
    }
}

impl From<JsStringError> for UriCodecError {
    fn from(error: JsStringError) -> Self {
        Self::String(error)
    }
}

/// Decode `decodeURI` (`component == false`) or `decodeURIComponent`.
pub(crate) fn decode(input: &JsString, component: bool) -> Result<JsString, UriCodecError> {
    decode_with_limit(input, component, JsString::MAX_LEN)
}

fn decode_with_limit(
    input: &JsString,
    component: bool,
    limit: usize,
) -> Result<JsString, UriCodecError> {
    let limit = limit.min(JsString::MAX_LEN);
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
    Ok(JsString::try_from_utf16_with_limit(output, limit)?)
}

/// Encode `encodeURI` (`component == false`) or `encodeURIComponent`.
pub(crate) fn encode(input: &JsString, component: bool) -> Result<JsString, UriCodecError> {
    encode_with_limit(input, component, JsString::MAX_LEN)
}

fn encode_with_limit(
    input: &JsString,
    component: bool,
    limit: usize,
) -> Result<JsString, UriCodecError> {
    let limit = limit.min(JsString::MAX_LEN);
    let output_len = encoded_uri_length(input, component, limit)?;
    let units = input.utf16_units().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(output_len);
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
    debug_assert_eq!(output.len(), output_len);
    Ok(JsString::try_from_utf16_with_limit(output, limit)?)
}

/// Apply Annex-B `escape`, preserving its code-unit-oriented `%uXXXX` form.
pub(crate) fn escape(input: &JsString) -> Result<JsString, UriCodecError> {
    escape_with_limit(input, JsString::MAX_LEN)
}

fn escape_with_limit(input: &JsString, limit: usize) -> Result<JsString, UriCodecError> {
    let limit = limit.min(JsString::MAX_LEN);
    let output_len = input.utf16_units().try_fold(0_usize, |length, unit| {
        let additional = if is_legacy_unescaped(unit) {
            1
        } else if unit <= 0xff {
            3
        } else {
            6
        };
        JsString::checked_length_with_limit(length, additional, limit).map_err(UriCodecError::from)
    })?;
    let mut output = JsStringBuilder::with_limit(output_len, limit);
    for unit in input.utf16_units() {
        if is_legacy_unescaped(unit) {
            output.push_code_point(u32::from(unit))?;
        } else if unit <= 0xff {
            output.push_code_point(u32::from(b'%'))?;
            output.push_code_point(u32::from(HEX[usize::from((unit >> 4) & 0x0f)]))?;
            output.push_code_point(u32::from(HEX[usize::from(unit & 0x0f)]))?;
        } else {
            output.push_utf8("%u")?;
            output.push_code_point(u32::from(HEX[usize::from((unit >> 12) & 0x0f)]))?;
            output.push_code_point(u32::from(HEX[usize::from((unit >> 8) & 0x0f)]))?;
            output.push_code_point(u32::from(HEX[usize::from((unit >> 4) & 0x0f)]))?;
            output.push_code_point(u32::from(HEX[usize::from(unit & 0x0f)]))?;
        }
    }
    Ok(output.finish()?)
}

/// Apply Annex-B `unescape`. Invalid escapes remain literal and never throw.
pub(crate) fn unescape(input: &JsString) -> Result<JsString, UriCodecError> {
    unescape_with_limit(input, JsString::MAX_LEN)
}

fn unescape_with_limit(input: &JsString, limit: usize) -> Result<JsString, UriCodecError> {
    let limit = limit.min(JsString::MAX_LEN);
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
    Ok(JsString::try_from_utf16_with_limit(output, limit)?)
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

fn encoded_uri_length(
    input: &JsString,
    component: bool,
    limit: usize,
) -> Result<usize, UriCodecError> {
    let mut units = input.utf16_units();
    let mut output_len = 0;
    let mut too_long = false;
    while let Some(first) = units.next() {
        let encoded_len = if is_uri_unescaped(first, component) {
            1
        } else {
            let code_point = if is_low_surrogate(first) {
                return Err(UriCodecError::InvalidCharacter);
            } else if is_high_surrogate(first) {
                let Some(second) = units.next() else {
                    return Err(UriCodecError::ExpectingSurrogatePair);
                };
                if !is_low_surrogate(second) {
                    return Err(UriCodecError::ExpectingSurrogatePair);
                }
                0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
            } else {
                u32::from(first)
            };
            match code_point {
                0..=0x7f => 3,
                0x80..=0x7ff => 6,
                0x800..=0xffff => 9,
                _ => 12,
            }
        };
        if !too_long {
            match JsString::checked_length_with_limit(output_len, encoded_len, limit) {
                Ok(length) => output_len = length,
                Err(JsStringError::TooLong) => too_long = true,
            }
        }
    }
    if too_long {
        Err(UriCodecError::String(JsStringError::TooLong))
    } else {
        Ok(output_len)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> JsString {
        JsString::try_from_utf8(value).unwrap()
    }

    const fn too_long() -> UriCodecError {
        UriCodecError::String(JsStringError::TooLong)
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
        let emoji = JsString::try_from_utf16([0xd83d, 0xde00]).unwrap();
        assert_eq!(encode(&emoji, true).unwrap(), text("%F0%9F%98%80"));
        assert_eq!(decode(&text("%F0%9F%98%80"), true).unwrap(), emoji);
        let raw_lone_surrogates =
            JsString::try_from_utf16([0xd800, u16::from(b'A'), 0xdfff]).unwrap();
        assert_eq!(
            decode(&raw_lone_surrogates, false).unwrap(),
            raw_lone_surrogates
        );
        assert_eq!(
            decode(&raw_lone_surrogates, true).unwrap(),
            raw_lone_surrogates
        );
        assert_eq!(
            encode(&JsString::try_from_utf16([0xdc00]).unwrap(), false),
            Err(UriCodecError::InvalidCharacter)
        );
        assert_eq!(
            encode(&JsString::try_from_utf16([0xd800]).unwrap(), false),
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
            escape(&text("AZaz09@*_+-./ !~'()")).unwrap(),
            text("AZaz09@*_+-./%20%21%7E%27%28%29")
        );
        assert_eq!(
            escape(&JsString::try_from_utf16([0x00e9, 0x0100, 0xd800]).unwrap()).unwrap(),
            text("%E9%u0100%uD800")
        );
        assert_eq!(
            unescape(&text("%41%u0042%uD83D%uDE00%U0043%zz%")).unwrap(),
            JsString::try_from_utf16([
                0x41, 0x42, 0xd83d, 0xde00, 0x25, 0x55, 0x30, 0x30, 0x34, 0x33, 0x25, 0x7a, 0x7a,
                0x25,
            ])
            .unwrap()
        );
    }

    #[test]
    fn legacy_escape_roundtrips_every_utf16_code_unit() {
        let all_units = JsString::try_from_utf16(0_u16..=u16::MAX).unwrap();
        assert_eq!(unescape(&escape(&all_units).unwrap()).unwrap(), all_units);
    }

    #[test]
    fn uri_small_limits_accept_exact_lengths_and_reject_the_next_unit() {
        let empty = text("");
        assert_eq!(encode_with_limit(&empty, true, 0), Ok(empty.clone()));
        assert_eq!(decode_with_limit(&empty, true, 0), Ok(empty.clone()));

        for (input, component, limit, expected) in [
            ("A", true, 1, "A"),
            ("/", false, 1, "/"),
            (" ", true, 3, "%20"),
            ("\u{e9}", true, 6, "%C3%A9"),
            ("\u{100}", true, 6, "%C4%80"),
            ("\u{800}", true, 9, "%E0%A0%80"),
        ] {
            let input = text(input);
            assert_eq!(
                encode_with_limit(&input, component, limit),
                Ok(text(expected)),
                "encode exact limit {limit}"
            );
            assert_eq!(
                encode_with_limit(&input, component, limit - 1),
                Err(too_long()),
                "encode overflow after limit {}",
                limit - 1
            );
        }

        let emoji = JsString::try_from_utf16([0xd83d, 0xde00]).unwrap();
        assert_eq!(
            encode_with_limit(&emoji, true, 12),
            Ok(text("%F0%9F%98%80"))
        );
        assert_eq!(encode_with_limit(&emoji, true, 11), Err(too_long()));

        for (input, component, limit, expected) in [
            ("%41", true, 1, text("A")),
            ("%2f", false, 3, text("%2f")),
            ("%2f", true, 1, text("/")),
            ("%F0%9F%98%80", true, 2, emoji.clone()),
        ] {
            let input = text(input);
            assert_eq!(
                decode_with_limit(&input, component, limit),
                Ok(expected),
                "decode exact limit {limit}"
            );
            assert_eq!(
                decode_with_limit(&input, component, limit - 1),
                Err(too_long()),
                "decode overflow after limit {}",
                limit - 1
            );
        }

        assert_eq!(encode_with_limit(&text("A "), true, 1), Err(too_long()));
        assert_eq!(decode_with_limit(&text("A%41"), true, 1), Err(too_long()));
    }

    #[test]
    fn later_uri_validation_overrides_an_earlier_small_limit_overflow() {
        for (input, expected) in [
            (
                JsString::try_from_utf16([u16::from(b'A'), 0xdc00]).unwrap(),
                UriCodecError::InvalidCharacter,
            ),
            (
                JsString::try_from_utf16([u16::from(b'A'), 0xd800]).unwrap(),
                UriCodecError::ExpectingSurrogatePair,
            ),
            (
                JsString::try_from_utf16([u16::from(b'A'), 0xd800, u16::from(b'B')]).unwrap(),
                UriCodecError::ExpectingSurrogatePair,
            ),
        ] {
            assert_eq!(encode_with_limit(&input, true, 0), Err(expected));
        }

        for (input, expected) in [
            ("A%", UriCodecError::ExpectingHexDigit),
            ("A%E2A0", UriCodecError::ExpectingPercent),
            ("A%E2%GG", UriCodecError::ExpectingHexDigit),
            ("A%80", UriCodecError::MalformedUtf8),
            ("A%ED%A0%80", UriCodecError::MalformedUtf8),
        ] {
            assert_eq!(
                decode_with_limit(&text(input), true, 0),
                Err(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn annex_b_small_limits_cover_exact_and_overflow_results() {
        let empty = text("");
        assert_eq!(escape_with_limit(&empty, 0), Ok(empty.clone()));
        assert_eq!(unescape_with_limit(&empty, 0), Ok(empty));

        for (input, limit, expected) in [
            (text("A"), 1, text("A")),
            (text(" "), 3, text("%20")),
            (text("\u{e9}"), 3, text("%E9")),
            (
                JsString::try_from_utf16([0x0100]).unwrap(),
                6,
                text("%u0100"),
            ),
            (
                JsString::try_from_utf16([0xd800]).unwrap(),
                6,
                text("%uD800"),
            ),
        ] {
            assert_eq!(escape_with_limit(&input, limit), Ok(expected));
            assert_eq!(escape_with_limit(&input, limit - 1), Err(too_long()));
        }

        for (input, limit, expected) in [
            ("%41", 1, text("A")),
            ("%u0100", 1, JsString::try_from_utf16([0x0100]).unwrap()),
            (
                "%uD83D%uDE00",
                2,
                JsString::try_from_utf16([0xd83d, 0xde00]).unwrap(),
            ),
            ("%zz", 3, text("%zz")),
        ] {
            let input = text(input);
            assert_eq!(unescape_with_limit(&input, limit), Ok(expected));
            assert_eq!(unescape_with_limit(&input, limit - 1), Err(too_long()));
        }
    }
}
