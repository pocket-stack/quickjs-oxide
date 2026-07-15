//! Byte-for-byte port of the pinned QuickJS `Date.parse` scanner.
//!
//! Observable coercion and conversion of the parsed fields to a time value
//! belong to the parent Date intrinsic. This module only reproduces
//! `quickjs.c:55266-55728`: the 127-code-unit byte view, ISO-first grammar,
//! legacy grammar, and the final field-range checks.

use crate::value::JsString;

const PARSE_BUFFER_LEN: usize = 128;
const PARSE_INPUT_LIMIT: usize = PARSE_BUFFER_LEN - 1;
const MONTH_NAMES: &[u8; 36] = b"JanFebMarAprMayJunJulAugSepOctNovDec";
const FIELD_MAX: [i32; 6] = [0, 11, 31, 24, 59, 59];

const TIME_ZONE_ABBREVIATIONS: [(&[u8], i32); 18] = [
    (b"GMT", 0),
    (b"UTC", 0),
    (b"UT", 0),
    (b"Z", 0),
    (b"EDT", -4 * 60),
    (b"EST", -5 * 60),
    (b"CDT", -5 * 60),
    (b"CST", -6 * 60),
    (b"MDT", -6 * 60),
    (b"MST", -7 * 60),
    (b"PDT", -7 * 60),
    (b"PST", -8 * 60),
    (b"WET", 0),
    (b"WEST", 60),
    (b"CET", 60),
    (b"CEST", 2 * 60),
    (b"EET", 2 * 60),
    (b"EEST", 3 * 60),
];

/// The nine integer slots produced by pinned QuickJS and whether the fields
/// must be interpreted in local time before applying `fields[8]` as the
/// explicit offset in minutes. Slot seven is retained for layout parity even
/// though the Date conversion consumes only slots zero through six and eight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ParsedDateString {
    pub(super) fields: [i32; 9],
    pub(super) is_local: bool,
}

/// Parse an already-coerced ECMAScript String using the release-pinned
/// QuickJS grammar. Calendar normalization and time clipping are deliberately
/// left to the parent Date implementation.
#[must_use]
pub(super) fn parse_date_string(input: &JsString) -> Option<ParsedDateString> {
    let bytes = quickjs_parse_bytes(input);

    // The C implementation uses `iso(...) || legacy(...)`; a syntactically
    // recognized ISO form which later fails range validation must therefore
    // not fall back to the permissive legacy grammar.
    let parsed = match parse_iso_string(&bytes) {
        Some(parsed) => parsed,
        None => parse_other_string(&bytes)?,
    };

    fields_are_valid(&parsed.fields).then_some(parsed)
}

fn map_code_unit(unit: u16) -> u8 {
    match u8::try_from(unit) {
        Ok(byte) => byte,
        Err(_) if unit == 0x2212 => b'-',
        Err(_) => b'x',
    }
}

fn quickjs_parse_bytes(input: &JsString) -> [u8; PARSE_BUFFER_LEN] {
    let mut bytes = [0; PARSE_BUFFER_LEN];
    for (index, unit) in input.utf16_units().take(PARSE_INPUT_LIMIT).enumerate() {
        bytes[index] = map_code_unit(unit);
    }
    bytes
}

fn skip_char(bytes: &[u8; PARSE_BUFFER_LEN], cursor: &mut usize, expected: u8) -> bool {
    if bytes[*cursor] == expected {
        *cursor += 1;
        true
    } else {
        false
    }
}

fn skip_spaces(bytes: &[u8; PARSE_BUFFER_LEN], cursor: &mut usize) -> u8 {
    while bytes[*cursor] == b' ' {
        *cursor += 1;
    }
    bytes[*cursor]
}

fn skip_separators(bytes: &[u8; PARSE_BUFFER_LEN], cursor: &mut usize) -> u8 {
    while matches!(bytes[*cursor], b'-' | b'/' | b'.' | b',') {
        *cursor += 1;
    }
    bytes[*cursor]
}

fn skip_until(bytes: &[u8; PARSE_BUFFER_LEN], cursor: &mut usize, stop_list: &[u8]) -> u8 {
    while bytes[*cursor] != 0 && !stop_list.contains(&bytes[*cursor]) {
        *cursor += 1;
    }
    bytes[*cursor]
}

/// Parse one QuickJS numeric field. Failure leaves `cursor` unchanged.
fn get_digits(
    bytes: &[u8; PARSE_BUFFER_LEN],
    cursor: &mut usize,
    min_digits: usize,
    max_digits: usize,
) -> Option<i32> {
    let mut value = 0_i32;
    let start = *cursor;
    let mut position = start;

    while bytes[position].is_ascii_digit() {
        // This is the upstream parser's deliberate arbitrary nine-digit cap,
        // checked before incorporating the next digit.
        if value >= 100_000_000 {
            return None;
        }
        value = value * 10 + i32::from(bytes[position] - b'0');
        position += 1;
        if position - start == max_digits {
            break;
        }
    }

    if position - start < min_digits {
        return None;
    }
    *cursor = position;
    Some(value)
}

/// Parse an optional fractional second, truncating after milliseconds while
/// consuming at most nine digits just like `string_get_milliseconds`.
fn get_milliseconds(bytes: &[u8; PARSE_BUFFER_LEN], cursor: &mut usize) -> Option<i32> {
    if !matches!(bytes[*cursor], b'.' | b',') {
        return None;
    }

    let mut position = *cursor + 1;
    let start = position;
    let mut multiplier = 100_i32;
    let mut milliseconds = 0_i32;
    while bytes[position].is_ascii_digit() {
        milliseconds += i32::from(bytes[position] - b'0') * multiplier;
        multiplier /= 10;
        position += 1;
        if position - start == 9 {
            break;
        }
    }

    if position == start {
        return None;
    }
    *cursor = position;
    Some(milliseconds)
}

fn upper_ascii(byte: u8) -> u8 {
    byte.to_ascii_uppercase()
}

/// Parse `Z`, a strict ISO offset, or a permissive legacy offset. Failure
/// leaves both the cursor and destination state unchanged.
fn get_time_zone_offset(
    bytes: &[u8; PARSE_BUFFER_LEN],
    cursor: &mut usize,
    strict: bool,
) -> Option<i32> {
    let mut position = *cursor;
    let sign = bytes[position];
    position += 1;

    let offset = if matches!(sign, b'+' | b'-') {
        let digits_start = position;
        let mut hours = get_digits(bytes, &mut position, 1, 0)?;
        let mut digit_count = position - digits_start;
        if strict && !matches!(digit_count, 2 | 4) {
            return None;
        }

        // The legacy grammar accepts arbitrarily shaped offsets up to the
        // numeric scanner's cap, discarding trailing digit pairs until only a
        // HH or HHMM-shaped prefix remains.
        while digit_count > 4 {
            digit_count -= 2;
            hours /= 100;
        }

        let minutes;
        if digit_count > 2 {
            minutes = hours % 100;
            hours /= 100;
        } else {
            if skip_char(bytes, &mut position, b':') {
                minutes = get_digits(bytes, &mut position, 2, 2)?;
            } else {
                if strict {
                    return None;
                }
                minutes = 0;
            }
        }

        if hours > 23 || minutes > 59 {
            return None;
        }
        let absolute = hours * 60 + minutes;
        if sign == b'+' { absolute } else { -absolute }
    } else if sign == b'Z' {
        0
    } else {
        return None;
    };

    *cursor = position;
    Some(offset)
}

/// Match ASCII case-insensitively. Failure leaves `cursor` unchanged.
fn string_match(bytes: &[u8; PARSE_BUFFER_LEN], cursor: &mut usize, text: &[u8]) -> bool {
    let position = *cursor;
    for (index, expected) in text.iter().copied().enumerate() {
        if upper_ascii(bytes.get(position + index).copied().unwrap_or(0)) != upper_ascii(expected) {
            return false;
        }
    }
    *cursor += text.len();
    true
}

fn get_month(bytes: &[u8; PARSE_BUFFER_LEN], cursor: &mut usize) -> Option<i32> {
    for (month, spelling) in MONTH_NAMES.chunks_exact(3).enumerate() {
        if spelling
            .iter()
            .copied()
            .enumerate()
            .all(|(index, expected)| {
                upper_ascii(bytes.get(*cursor + index).copied().unwrap_or(0))
                    == upper_ascii(expected)
            })
        {
            *cursor += 3;
            return Some(month as i32 + 1);
        }
    }
    None
}

fn parse_iso_string(bytes: &[u8; PARSE_BUFFER_LEN]) -> Option<ParsedDateString> {
    let mut fields = [0_i32; 9];
    fields[2] = 1;
    let mut is_local = false;
    let mut cursor = 0_usize;

    let sign = bytes[cursor];
    if matches!(sign, b'-' | b'+') {
        cursor += 1;
        fields[0] = get_digits(bytes, &mut cursor, 6, 6)?;
        if sign == b'-' {
            if fields[0] == 0 {
                return None;
            }
            fields[0] = -fields[0];
        }
    } else {
        fields[0] = get_digits(bytes, &mut cursor, 4, 4)?;
    }

    if skip_char(bytes, &mut cursor, b'-') {
        fields[1] = get_digits(bytes, &mut cursor, 2, 2)?;
        if fields[1] < 1 {
            return None;
        }
        fields[1] -= 1;
        if skip_char(bytes, &mut cursor, b'-') {
            fields[2] = get_digits(bytes, &mut cursor, 2, 2)?;
            if fields[2] < 1 {
                return None;
            }
        }
    }

    if skip_char(bytes, &mut cursor, b'T') {
        is_local = true;
        let time_prefix_is_valid = if let Some(hours) = get_digits(bytes, &mut cursor, 2, 2) {
            fields[3] = hours;
            if skip_char(bytes, &mut cursor, b':') {
                if let Some(minutes) = get_digits(bytes, &mut cursor, 2, 2) {
                    fields[4] = minutes;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if !time_prefix_is_valid {
            // Upstream intentionally returns TRUE with an impossible hour so
            // the outer range check rejects the string without legacy retry.
            fields[3] = 100;
            return Some(ParsedDateString { fields, is_local });
        }

        if skip_char(bytes, &mut cursor, b':') {
            fields[5] = get_digits(bytes, &mut cursor, 2, 2)?;
            if let Some(milliseconds) = get_milliseconds(bytes, &mut cursor) {
                fields[6] = milliseconds;
            }
        }
    }

    if bytes[cursor] != 0 {
        is_local = false;
        fields[8] = get_time_zone_offset(bytes, &mut cursor, true)?;
    }

    (bytes[cursor] == 0).then_some(ParsedDateString { fields, is_local })
}

fn legacy_year(value: i32) -> i32 {
    value + if value < 100 { 1900 } else { 0 } + if value < 50 { 100 } else { 0 }
}

fn parse_other_string(bytes: &[u8; PARSE_BUFFER_LEN]) -> Option<ParsedDateString> {
    let mut fields = [0_i32; 9];
    fields[0] = 2001;
    fields[1] = 1;
    fields[2] = 1;
    let mut is_local = true;

    let mut has_year = false;
    let mut has_month = false;
    let mut has_time = false;
    let mut numbers = [0_i32; 3];
    let mut number_count = 0_usize;
    let mut cursor = 0_usize;

    while skip_spaces(bytes, &mut cursor) != 0 {
        let token_start = cursor;
        let first = bytes[cursor];

        if matches!(first, b'+' | b'-') {
            let parsed_offset = if has_time {
                get_time_zone_offset(bytes, &mut cursor, false)
            } else {
                None
            };
            if let Some(offset) = parsed_offset {
                fields[8] = offset;
                is_local = false;
            } else {
                cursor += 1;
                if let Some(mut value) = get_digits(bytes, &mut cursor, 1, 0) {
                    if first == b'-' {
                        if value == 0 {
                            return None;
                        }
                        value = -value;
                    }
                    fields[0] = value;
                    has_year = true;
                }
            }
        } else if let Some(value) = get_digits(bytes, &mut cursor, 1, 0) {
            if skip_char(bytes, &mut cursor, b':') {
                fields[3] = value;
                fields[4] = get_digits(bytes, &mut cursor, 1, 2)?;
                if skip_char(bytes, &mut cursor, b':') {
                    fields[5] = get_digits(bytes, &mut cursor, 1, 2)?;
                    if let Some(milliseconds) = get_milliseconds(bytes, &mut cursor) {
                        fields[6] = milliseconds;
                    }
                }
                has_time = true;
                if matches!(bytes[cursor], b'+' | b'-') {
                    if let Some(offset) = get_time_zone_offset(bytes, &mut cursor, false) {
                        fields[8] = offset;
                        is_local = false;
                    }
                }
            } else if cursor - token_start > 2 && !has_year {
                fields[0] = value;
                has_year = true;
            } else if !(1..=31).contains(&value) && !has_year {
                fields[0] = legacy_year(value);
                has_year = true;
            } else {
                if number_count == numbers.len() {
                    return None;
                }
                numbers[number_count] = value;
                number_count += 1;
            }
        } else if let Some(month) = get_month(bytes, &mut cursor) {
            fields[1] = month;
            has_month = true;
            skip_until(bytes, &mut cursor, b"0123456789 -/(");
        } else if has_time && string_match(bytes, &mut cursor, b"PM") {
            if fields[3] < 12 {
                fields[3] += 12;
            }
            continue;
        } else if has_time && string_match(bytes, &mut cursor, b"AM") {
            if fields[3] == 12 {
                fields[3] -= 12;
            }
            continue;
        } else if let Some(offset) = get_time_zone_abbreviation(bytes, &mut cursor) {
            fields[8] = offset;
            is_local = false;
            continue;
        } else if first == b'(' {
            let mut level = 0_i32;
            while bytes[cursor] != 0 {
                let character = bytes[cursor];
                cursor += 1;
                level += i32::from(character == b'(');
                level -= i32::from(character == b')');
                if level == 0 {
                    break;
                }
            }
            if level > 0 {
                return None;
            }
        } else if first == b')' {
            return None;
        } else {
            if usize::from(has_year) + usize::from(has_month) + usize::from(has_time) + number_count
                != 0
            {
                return None;
            }
            skip_until(bytes, &mut cursor, b" -/(");
        }
        skip_separators(bytes, &mut cursor);
    }

    if number_count + usize::from(has_year) + usize::from(has_month) > 3 {
        return None;
    }

    match number_count {
        0 => {
            if !has_year {
                return None;
            }
        }
        1 => {
            if has_month {
                fields[2] = numbers[0];
            } else {
                fields[1] = numbers[0];
            }
        }
        2 => {
            if has_year {
                fields[1] = numbers[0];
                fields[2] = numbers[1];
            } else if has_month {
                fields[0] = legacy_year(numbers[1]);
                fields[2] = numbers[0];
            } else {
                fields[1] = numbers[0];
                fields[2] = numbers[1];
            }
        }
        3 => {
            fields[0] = legacy_year(numbers[2]);
            fields[1] = numbers[0];
            fields[2] = numbers[1];
        }
        _ => return None,
    }

    if fields[1] < 1 || fields[2] < 1 {
        return None;
    }
    fields[1] -= 1;
    Some(ParsedDateString { fields, is_local })
}

fn get_time_zone_abbreviation(bytes: &[u8; PARSE_BUFFER_LEN], cursor: &mut usize) -> Option<i32> {
    for (name, offset) in TIME_ZONE_ABBREVIATIONS {
        if string_match(bytes, cursor, name) {
            return Some(offset);
        }
    }
    None
}

fn fields_are_valid(fields: &[i32; 9]) -> bool {
    if fields[1..6]
        .iter()
        .zip(FIELD_MAX[1..6].iter())
        .any(|(field, maximum)| field > maximum)
    {
        return false;
    }

    // 24:00:00.000 is the only accepted representation with hour 24.
    fields[3] != 24 || fields[4..=6].iter().all(|field| *field == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn js_string(text: &str) -> JsString {
        JsString::try_from_utf8(text).unwrap()
    }

    fn parse(text: &str) -> Option<ParsedDateString> {
        parse_date_string(&js_string(text))
    }

    fn expected(fields: [i32; 9], is_local: bool) -> Option<ParsedDateString> {
        Some(ParsedDateString { fields, is_local })
    }

    #[test]
    fn utf16_mapping_is_exhaustive_and_input_is_nul_terminated_at_127_units() {
        for unit in 0_u16..=u16::MAX {
            let expected = match u8::try_from(unit) {
                Ok(byte) => byte,
                Err(_) if unit == 0x2212 => b'-',
                Err(_) => b'x',
            };
            assert_eq!(map_code_unit(unit), expected, "U+{unit:04X}");
        }

        let minus =
            JsString::try_from_utf16([0x2212].into_iter().chain("000001-01-01".encode_utf16()))
                .unwrap();
        assert_eq!(
            parse_date_string(&minus),
            expected([-1, 0, 1, 0, 0, 0, 0, 0, 0], false)
        );

        let embedded_nul = JsString::try_from_utf16(
            "2000"
                .encode_utf16()
                .chain([0])
                .chain("junk".encode_utf16()),
        )
        .unwrap();
        assert_eq!(
            parse_date_string(&embedded_nul),
            expected([2000, 0, 1, 0, 0, 0, 0, 0, 0], false)
        );

        let truncated = format!("2000-01-01{}junk", " ".repeat(117));
        assert_eq!(truncated.encode_utf16().take(127).count(), 127);
        assert_eq!(
            parse_date_string(&js_string(&truncated)),
            // The inserted terminator hides the suffix, but the preceding
            // spaces force ISO parsing to fail and legacy parsing to win.
            expected([2000, 0, 1, 0, 0, 0, 0, 0, 0], true)
        );

        assert_eq!(parse("2000-01-\u{0100}"), None);
    }

    #[test]
    fn iso_forms_match_pinned_quickjs_fields_and_locality() {
        let cases = [
            ("2000", [2000, 0, 1, 0, 0, 0, 0, 0, 0], false),
            ("2000-02", [2000, 1, 1, 0, 0, 0, 0, 0, 0], false),
            ("2000-02-03", [2000, 1, 3, 0, 0, 0, 0, 0, 0], false),
            ("2000-02-03T04:05", [2000, 1, 3, 4, 5, 0, 0, 0, 0], true),
            ("2000-02-03T04:05Z", [2000, 1, 3, 4, 5, 0, 0, 0, 0], false),
            (
                "2000-02-03T04:05:06.123456789Z",
                [2000, 1, 3, 4, 5, 6, 123, 0, 0],
                false,
            ),
            (
                "2000-02-03T04:05:06,9876+01:30",
                [2000, 1, 3, 4, 5, 6, 987, 0, 90],
                false,
            ),
            (
                "2000-02-03T04:05:06-0130",
                [2000, 1, 3, 4, 5, 6, 0, 0, -90],
                false,
            ),
            (
                "+000000-01-01T00:00:00Z",
                [0, 0, 1, 0, 0, 0, 0, 0, 0],
                false,
            ),
            (
                "-000001-01-01T00:00:00Z",
                [-1, 0, 1, 0, 0, 0, 0, 0, 0],
                false,
            ),
            (
                "+275760-09-13T00:00:00.000Z",
                [275760, 8, 13, 0, 0, 0, 0, 0, 0],
                false,
            ),
            (
                "2000-01-01T24:00:00.000Z",
                [2000, 0, 1, 24, 0, 0, 0, 0, 0],
                false,
            ),
            ("2000Z", [2000, 0, 1, 0, 0, 0, 0, 0, 0], false),
        ];

        for (source, fields, is_local) in cases {
            assert_eq!(parse(source), expected(fields, is_local), "{source}");
        }
    }

    #[test]
    fn fractional_seconds_are_consumed_to_nine_digits_and_truncated_to_millis() {
        for width in 1..=9 {
            let digits = "123456789"[..width].to_owned();
            let source = format!("2000-01-01T00:00:00.{digits}Z");
            let parsed = parse(&source).unwrap();
            let milliseconds = match width {
                1 => 100,
                2 => 120,
                _ => 123,
            };
            assert_eq!(parsed.fields[6], milliseconds, "{source}");
        }
    }

    #[test]
    fn iso_rejections_include_range_offset_and_negative_zero_boundaries() {
        let rejected = [
            "",
            "-000000-01-01T00:00:00Z",
            "2000-00-01",
            "2000-13-01",
            "2000-01-00",
            "2000-01-32",
            "2000-01-01T",
            "2000-01-01T00Z",
            "2000-01-01T25:00Z",
            "2000-01-01T24:01Z",
            "2000-01-01T24:00:01Z",
            "2000-01-01T24:00:00.001Z",
            "2000-01-01T00:60Z",
            "2000-01-01T00:00:60Z",
            "2000-01-01T00:00:00.Z",
            "2000-01-01T00:00:00.1234567890Z",
            "2000-01-01T00:00:00+01",
            "2000-01-01T00:00:00+1:00",
            "2000-01-01T00:00:00+010",
            "2000-01-01T00:00:00+24:00",
            "2000-01-01T00:00:00+00:60",
            "2000-01-01T00:00:00z",
            "2000-01-01T00:00:00Zjunk",
        ];

        for source in rejected {
            assert_eq!(parse(source), None, "{source}");
        }
    }

    #[test]
    fn legacy_parser_accepts_upstream_string_utc_string_and_common_forms() {
        let cases = [
            (
                "Thu Jan 01 1970 00:00:00 GMT+0000",
                [1970, 0, 1, 0, 0, 0, 0, 0, 0],
                false,
            ),
            (
                "Thu, 01 Jan 1970 00:00:00 GMT",
                [1970, 0, 1, 0, 0, 0, 0, 0, 0],
                false,
            ),
            (
                "Sat Jan 1 2000 00:00:00 GMT+0100",
                [2000, 0, 1, 0, 0, 0, 0, 0, 60],
                false,
            ),
            (
                "nonsense weekday (nested (comment)) January---1//2000 1:02:03,45 pm PST",
                [2000, 0, 1, 13, 2, 3, 450, 0, -480],
                false,
            ),
            ("Jan 1 2000 12:34 AM", [2000, 0, 1, 0, 34, 0, 0, 0, 0], true),
            (
                "Jan 1 2000 12:34 PM",
                [2000, 0, 1, 12, 34, 0, 0, 0, 0],
                true,
            ),
            ("Jan 1 2000 24:00", [2000, 0, 1, 24, 0, 0, 0, 0, 0], true),
            (
                // An invalid positive offset is retried as a signed year.
                "Jan 1 2000 00:00+24",
                [24, 0, 1, 0, 0, 0, 0, 0, 0],
                true,
            ),
            (
                // Month recognition is a three-byte prefix match, so `junk`
                // overwrites the earlier January with June.
                "Jan 1 2000 junk",
                [2000, 5, 1, 0, 0, 0, 0, 0, 0],
                true,
            ),
            (
                // A later time token overwrites an earlier one.
                "Jan 1 2000 01:02 03:04",
                [2000, 0, 1, 3, 4, 0, 0, 0, 0],
                true,
            ),
            ("2000-", [2000, 0, 1, 0, 0, 0, 0, 0, 0], true),
            ("Jan 1", [2001, 0, 1, 0, 0, 0, 0, 0, 0], true),
            ("1/2", [2001, 0, 2, 0, 0, 0, 0, 0, 0], true),
            ("1", [2001, 0, 1, 0, 0, 0, 0, 0, 0], true),
            ("+49", [49, 0, 1, 0, 0, 0, 0, 0, 0], true),
            ("-1", [-1, 0, 1, 0, 0, 0, 0, 0, 0], true),
        ];

        for (source, fields, is_local) in cases {
            assert_eq!(parse(source), expected(fields, is_local), "{source}");
        }
    }

    #[test]
    fn all_month_abbreviations_and_suffixes_are_case_insensitive() {
        let names = [
            "january",
            "FEBRUARY",
            "March",
            "aPrIl",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        for (month, name) in names.into_iter().enumerate() {
            let source = format!("{name} 1 2000");
            let parsed = parse(&source).unwrap();
            assert_eq!(parsed.fields[..3], [2000, month as i32, 1], "{source}");
        }
    }

    #[test]
    fn legacy_two_digit_year_pivot_is_exhaustive() {
        for year in 0..=99 {
            let source = format!("Jan 1 {year:02}");
            let parsed = parse(&source).unwrap();
            let expected_year = if year < 50 { 2000 + year } else { 1900 + year };
            assert_eq!(parsed.fields[0], expected_year, "{source}");
        }

        assert_eq!(parse("32").unwrap().fields[0], 2032);
        assert_eq!(parse("49").unwrap().fields[0], 2049);
        assert_eq!(parse("50").unwrap().fields[0], 1950);
        assert_eq!(parse("99").unwrap().fields[0], 1999);
        assert_eq!(parse("100").unwrap().fields[0], 100);
    }

    #[test]
    fn every_pinned_time_zone_abbreviation_has_its_exact_offset() {
        for (name, offset) in TIME_ZONE_ABBREVIATIONS {
            let name = std::str::from_utf8(name).unwrap();
            let source = format!("Jan 1 2000 00:00 {name}");
            let parsed = parse(&source).unwrap();
            assert_eq!(parsed.fields[8], offset, "{source}");
            assert!(!parsed.is_local, "{source}");
        }
    }

    #[test]
    fn legacy_numeric_time_zone_offsets_retain_quickjs_permissiveness() {
        let cases = [
            ("+1", 60),
            ("+01", 60),
            ("+1:02", 62),
            ("+0102", 62),
            ("+102", 62),
            ("-530", -330),
            ("+12345", 83),
            ("+123456", 12 * 60 + 34),
            ("+2359", 23 * 60 + 59),
        ];
        for (suffix, offset) in cases {
            let source = format!("Jan 1 2000 00:00{suffix}");
            let parsed = parse(&source).unwrap();
            assert_eq!(parsed.fields[8], offset, "{source}");
            assert!(!parsed.is_local, "{source}");
        }
    }

    #[test]
    fn legacy_rejections_cover_comments_extra_tokens_and_field_limits() {
        let rejected = [
            "word only",
            "Jan 1 2000 garbage",
            "Jan 1 2000 (unclosed",
            "Jan 1 2000 )",
            "Jan 0 2000",
            "13/1/2000",
            "1/32/2000",
            "Jan 1 2000 24:01",
            "Jan 1 2000 25:00",
            "Jan 1 2000 00:60",
            "Jan 1 2000 00:00:60",
            "Jan 1 2000 00:00-24",
            "Jan 1 2000 00:00+1:2",
            "1/2/3/4",
            "1000000000",
        ];
        for source in rejected {
            assert_eq!(parse(source), None, "{source}");
        }
    }
}
