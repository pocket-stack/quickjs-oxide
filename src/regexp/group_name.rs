//! QuickJS-shaped RegExp group-name parsing and lexical capture scans.
//!
//! Pinned QuickJS keeps this deliberately separate from the ordinary
//! JavaScript identifier lexer: group names always use Unicode identifier
//! semantics, accept only `\u` escapes, and are normalized before comparison.

use crate::value::JsString;

const GROUP_NAME_BUFFER_SIZE: usize = 128;
const UTF8_CHAR_LEN_MAX: usize = 6;
const CAPTURE_COUNT_MAX: u32 = u8::MAX as u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CaptureSummary {
    /// Includes capture zero, matching QuickJS's `re_count_captures` result.
    pub(super) capture_count: u32,
    /// Potential named syntax is enough here; validation belongs to the real
    /// parser, just as in QuickJS's lexical prepass.
    pub(super) has_named_captures: bool,
}

/// Parse a normalized `RegExpIdentifierName` starting immediately after `<`.
///
/// The returned cursor is immediately after `>`. Failure never advances the
/// caller's cursor, which is required by the non-Unicode Annex-B `\k`
/// fallback.
pub(super) fn parse(units: &[u16], start: usize) -> Option<(JsString, usize)> {
    let mut position = start;
    let mut normalized = Vec::new();
    let mut normalized_utf8_len = 0_usize;

    loop {
        let unit = *units.get(position)?;
        if unit == u16::from(b'>') {
            if normalized.is_empty() {
                return None;
            }
            let name = JsString::try_from_utf16(normalized)
                .expect("a bounded RegExp group name exceeded the String limit");
            return Some((name, position + 1));
        }

        let code_point = if unit == u16::from(b'\\') {
            if units.get(position + 1) != Some(&u16::from(b'u')) {
                return None;
            }
            position += 2;
            parse_unicode_escape(units, &mut position)?
        } else {
            position += 1;
            if is_high_surrogate(unit) {
                match units.get(position).copied() {
                    Some(low) if is_low_surrogate(low) => {
                        position += 1;
                        combine_surrogates(unit, low)
                    }
                    _ => u32::from(unit),
                }
            } else {
                u32::from(unit)
            }
        };

        let first = normalized.is_empty();
        if !(if first {
            is_identifier_start(code_point)
        } else {
            is_identifier_continue(code_point)
        }) {
            return None;
        }

        // QuickJS checks room for its historical maximum UTF-8 character
        // width plus the trailing NUL before writing every normalized code
        // point. Reproduce the check, including the resulting 122-byte ASCII
        // ceiling, rather than replacing it with a nominal 127-byte limit.
        if normalized_utf8_len + UTF8_CHAR_LEN_MAX + 1 > GROUP_NAME_BUFFER_SIZE {
            return None;
        }
        normalized_utf8_len += utf8_len(code_point);
        push_code_point(&mut normalized, code_point);
    }
}

/// QuickJS `re_count_captures`-style lexical prepass.
pub(super) fn capture_summary(units: &[u16]) -> CaptureSummary {
    let mut capture_index = 1_u32;
    let mut has_named_captures = false;
    let mut position = 0_usize;

    while position < units.len() {
        match units[position] {
            unit if unit == u16::from(b'(') => {
                let plain = units.get(position + 1) != Some(&u16::from(b'?'));
                let named = units.get(position + 1) == Some(&u16::from(b'?'))
                    && units.get(position + 2) == Some(&u16::from(b'<'))
                    && !matches!(units.get(position + 3).copied(), Some(0x3d | 0x21));
                if named {
                    has_named_captures = true;
                }
                if plain || named {
                    capture_index += 1;
                    if capture_index >= CAPTURE_COUNT_MAX {
                        break;
                    }
                }
            }
            unit if unit == u16::from(b'\\') => {
                position = position.saturating_add(1);
            }
            unit if unit == u16::from(b'[') => {
                position += 1;
                while position < units.len() && units[position] != u16::from(b']') {
                    if units[position] == u16::from(b'\\') {
                        position = position.saturating_add(1);
                    }
                    position = position.saturating_add(1);
                }
            }
            _ => {}
        }
        position = position.saturating_add(1);
    }

    CaptureSummary {
        capture_count: capture_index,
        has_named_captures,
    }
}

/// Return every lexical capture index whose normalized name equals `name`.
///
/// This intentionally mirrors the rough second scan in QuickJS, including
/// its cursor movement after a successfully parsed name. It is used only when
/// a named backreference has no already-parsed matching capture.
pub(super) fn matching_capture_indices(units: &[u16], name: &JsString) -> Vec<u8> {
    let mut captures = Vec::new();
    let mut capture_index = 1_u32;
    let mut position = 0_usize;

    while position < units.len() {
        match units[position] {
            unit if unit == u16::from(b'(') => {
                let plain = units.get(position + 1) != Some(&u16::from(b'?'));
                let named = units.get(position + 1) == Some(&u16::from(b'?'))
                    && units.get(position + 2) == Some(&u16::from(b'<'))
                    && !matches!(units.get(position + 3).copied(), Some(0x3d | 0x21));
                if named {
                    if let Some((candidate, after_name)) = parse(units, position + 3) {
                        if candidate == *name {
                            captures.push(
                                u8::try_from(capture_index)
                                    .expect("capture prepass exceeded QuickJS's u8 limit"),
                            );
                        }
                        // `re_parse_captures` leaves its pointer after `>` and
                        // the surrounding `for` performs one more increment.
                        position = after_name;
                    } else {
                        // The caller has already moved its local pointer to
                        // the first unit after `<`. A failed parser does not
                        // update it, and the surrounding loop increments it.
                        position += 3;
                    }
                }
                if plain || named {
                    capture_index += 1;
                    if capture_index >= CAPTURE_COUNT_MAX {
                        break;
                    }
                }
            }
            unit if unit == u16::from(b'\\') => {
                position = position.saturating_add(1);
            }
            unit if unit == u16::from(b'[') => {
                position += 1;
                while position < units.len() && units[position] != u16::from(b']') {
                    if units[position] == u16::from(b'\\') {
                        position = position.saturating_add(1);
                    }
                    position = position.saturating_add(1);
                }
            }
            _ => {}
        }
        position = position.saturating_add(1);
    }

    captures
}

fn parse_unicode_escape(units: &[u16], position: &mut usize) -> Option<u32> {
    let mut code_point;
    if units.get(*position) == Some(&u16::from(b'{')) {
        *position += 1;
        code_point = 0_u32;
        let mut digits = 0_usize;
        loop {
            let digit = hex_value(*units.get(*position)?)?;
            digits += 1;
            code_point = code_point.checked_mul(16)?.checked_add(digit)?;
            if code_point > 0x10_ffff {
                return None;
            }
            *position += 1;
            if units.get(*position) == Some(&u16::from(b'}')) {
                *position += 1;
                break;
            }
        }
        debug_assert!(digits > 0);
    } else {
        code_point = parse_four_hex(units, position)?;
        if is_high_surrogate_u32(code_point)
            && units.get(*position) == Some(&u16::from(b'\\'))
            && units.get(*position + 1) == Some(&u16::from(b'u'))
        {
            let mut low_position = *position + 2;
            if let Some(low) = parse_four_hex(units, &mut low_position)
                && is_low_surrogate_u32(low)
            {
                *position = low_position;
                code_point = combine_surrogates_u32(code_point, low);
            }
        }
    }
    Some(code_point)
}

fn parse_four_hex(units: &[u16], position: &mut usize) -> Option<u32> {
    let mut value = 0_u32;
    for _ in 0..4 {
        value = value * 16 + hex_value(*units.get(*position)?)?;
        *position += 1;
    }
    Some(value)
}

const fn hex_value(unit: u16) -> Option<u32> {
    match unit {
        0x30..=0x39 => Some((unit - 0x30) as u32),
        0x41..=0x46 => Some((unit - 0x41 + 10) as u32),
        0x61..=0x66 => Some((unit - 0x61 + 10) as u32),
        _ => None,
    }
}

fn is_identifier_start(code_point: u32) -> bool {
    if code_point < 0x80 {
        matches!(code_point, 0x24 | 0x41..=0x5a | 0x5f | 0x61..=0x7a)
    } else {
        crate::unicode::is_id_start(code_point)
    }
}

fn is_identifier_continue(code_point: u32) -> bool {
    if code_point < 0x80 {
        is_identifier_start(code_point) || (0x30..=0x39).contains(&code_point)
    } else {
        matches!(code_point, 0x200c | 0x200d) || crate::unicode::is_id_continue(code_point)
    }
}

const fn utf8_len(code_point: u32) -> usize {
    match code_point {
        0..=0x7f => 1,
        0x80..=0x7ff => 2,
        0x800..=0xffff => 3,
        _ => 4,
    }
}

fn push_code_point(output: &mut Vec<u16>, code_point: u32) {
    if code_point <= 0xffff {
        output.push(code_point as u16);
    } else {
        let scalar = code_point - 0x1_0000;
        output.push(0xd800 | (scalar >> 10) as u16);
        output.push(0xdc00 | (scalar & 0x3ff) as u16);
    }
}

const fn is_high_surrogate(unit: u16) -> bool {
    unit >= 0xd800 && unit <= 0xdbff
}

const fn is_low_surrogate(unit: u16) -> bool {
    unit >= 0xdc00 && unit <= 0xdfff
}

const fn is_high_surrogate_u32(code_point: u32) -> bool {
    code_point >= 0xd800 && code_point <= 0xdbff
}

const fn is_low_surrogate_u32(code_point: u32) -> bool {
    code_point >= 0xdc00 && code_point <= 0xdfff
}

const fn combine_surrogates(high: u16, low: u16) -> u32 {
    combine_surrogates_u32(high as u32, low as u32)
}

const fn combine_surrogates_u32(high: u32, low: u32) -> u32 {
    0x1_0000 + ((high - 0xd800) << 10) + (low - 0xdc00)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> Option<JsString> {
        let units = source.encode_utf16().collect::<Vec<_>>();
        parse(&units, 0).map(|(name, after)| {
            assert_eq!(after, units.len());
            name
        })
    }

    #[test]
    fn group_names_normalize_raw_and_escaped_unicode() {
        for source in ["a$𐒤_\u{200C}>", r"\u0061$\uD801\uDCA4_\u200C>"] {
            assert_eq!(
                parsed(source).unwrap().to_utf8_lossy(),
                "a$𐒤_\u{200c}",
                "{source}",
            );
        }
        assert_eq!(parsed(r"\u{41}>").unwrap().to_utf8_lossy(), "A");
    }

    #[test]
    fn group_names_reject_invalid_identifier_forms_and_escapes() {
        for source in [
            ">",
            "0a>",
            "🦊>",
            r"\x61>",
            r"\u{}>",
            r"\u{110000}>",
            r"\uD800>",
            "a->",
            "unterminated",
        ] {
            assert!(parsed(source).is_none(), "{source}");
        }
    }

    #[test]
    fn group_name_uses_quickjs_historical_128_byte_guard() {
        assert!(parsed(&format!("{}>", "a".repeat(122))).is_some());
        assert!(parsed(&format!("{}>", "a".repeat(123))).is_none());
        assert!(parsed(&format!("{}𐒤>", "a".repeat(121))).is_some());
    }

    #[test]
    fn capture_scans_skip_escapes_and_classes_and_normalize_names() {
        let units = r"\((?<a>x)[(](b)(?<\u0061>y)"
            .encode_utf16()
            .collect::<Vec<_>>();
        assert_eq!(
            capture_summary(&units),
            CaptureSummary {
                capture_count: 4,
                has_named_captures: true,
            },
        );
        let name = JsString::try_from_utf8("a").unwrap();
        assert_eq!(matching_capture_indices(&units, &name), vec![1, 3]);
    }
}
