//! Pure numeric-prefix parsing used by the global `parseInt` and `parseFloat`
//! intrinsics.
//!
//! The caller is responsible for the observable coercions: `parseInt` receives
//! the result of `ToString` and `ToInt32`, while `parseFloat` receives the
//! result of `ToString`.  Keeping coercion out of this module makes these
//! helpers deterministic and prevents parsing from accidentally invoking user
//! code a second time.

use num_bigint::BigUint;
use num_traits::ToPrimitive;

use crate::value::JsString;

// QuickJS `dtoa.c:atod_max_digits_table`, indexed by `radix - 2`.  The pinned
// parser keeps only this many significant input digits. For a power-of-two
// radix it folds discarded non-zero digits into a sticky low bit, which still
// guarantees exact binary64 rounding. For every other radix the discarded
// digits are deliberately truncated; reproducing that observable behavior is
// necessary for feature parity with the pinned release.
const ATOD_MAX_DIGITS: [usize; 35] = [
    64, 80, 32, 55, 49, 45, 21, 40, 38, 37, 35, 34, 33, 32, 16, 31, 30, 30, 29, 29, 28, 28, 27, 27,
    27, 26, 26, 26, 26, 25, 12, 25, 25, 24, 24,
];

const EXPONENT_LIMIT: i64 = 10_000;

/// Parse the longest integer prefix using the semantics of the pinned
/// `js_parseInt`/`js_atof` path.
///
/// `input` has already passed through `ToString`, and `radix` has already
/// passed through `ToInt32`.  Invalid non-zero radices return `NaN` rather than
/// throwing. The retained QuickJS mantissa is converted with a single
/// round-to-nearest, ties-to-even conversion to `f64`.
#[must_use]
pub fn parse_int(input: &JsString, radix: i32) -> f64 {
    if radix != 0 && !(2..=36).contains(&radix) {
        return f64::NAN;
    }

    let units = input.utf16_units().collect::<Vec<_>>();
    let mut cursor = skip_leading_space(&units);
    let negative = match units.get(cursor).copied() {
        Some(unit) if unit == u16::from(b'+') => {
            cursor += 1;
            false
        }
        Some(unit) if unit == u16::from(b'-') => {
            cursor += 1;
            true
        }
        _ => false,
    };

    let mut effective_radix = if radix == 0 { 10 } else { radix as u32 };
    if (radix == 0 || radix == 16)
        && matches!(units.get(cursor), Some(&unit) if unit == u16::from(b'0'))
        && matches!(units.get(cursor + 1), Some(&unit) if unit == u16::from(b'x') || unit == u16::from(b'X'))
    {
        cursor += 2;
        effective_radix = 16;
    }

    let digits_start = cursor;
    while units
        .get(cursor)
        .copied()
        .and_then(ascii_digit_value)
        .is_some_and(|digit| digit < effective_radix)
    {
        cursor += 1;
    }
    if cursor == digits_start {
        return f64::NAN;
    }

    // Every accepted digit is ASCII, so this conversion is lossless. BigUint
    // keeps the pinned bounded mantissa exact and avoids the double-rounding
    // caused by incrementally updating an f64.
    let digits = units[digits_start..cursor]
        .iter()
        .map(|unit| u8::try_from(*unit).expect("accepted parseInt digits are ASCII"))
        .collect::<Vec<_>>();
    let number = radix_integer_to_f64(&digits, effective_radix);

    if negative { -number } else { number }
}

/// Parse the longest decimal or `Infinity` prefix using the semantics of the
/// pinned global `parseFloat` scanner.
///
/// The scanner deliberately excludes Rust-only float spellings such as `inf`
/// and `NaN`. The conversion retains the first 38 significant decimal digits,
/// as pinned QuickJS does, and correctly rounds that retained value to
/// binary64 exactly once.
#[must_use]
pub fn parse_float(input: &JsString) -> f64 {
    let units = input.utf16_units().collect::<Vec<_>>();
    let start = skip_leading_space(&units);
    let mut cursor = start;
    let negative = match units.get(cursor).copied() {
        Some(unit) if unit == u16::from(b'+') => {
            cursor += 1;
            false
        }
        Some(unit) if unit == u16::from(b'-') => {
            cursor += 1;
            true
        }
        _ => false,
    };

    if has_ascii_prefix(&units[cursor..], b"Infinity") {
        return if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }

    let mantissa_start = cursor;
    let integer_start = cursor;
    while units.get(cursor).copied().is_some_and(is_ascii_digit) {
        cursor += 1;
    }
    let integer_digits = cursor - integer_start;

    let mut fractional_digits = 0;
    if matches!(units.get(cursor), Some(&unit) if unit == u16::from(b'.')) {
        cursor += 1;
        let fractional_start = cursor;
        while units.get(cursor).copied().is_some_and(is_ascii_digit) {
            cursor += 1;
        }
        fractional_digits = cursor - fractional_start;
    }

    if integer_digits == 0 && fractional_digits == 0 {
        return f64::NAN;
    }

    let mantissa_end = cursor;
    let mut explicit_exponent = 0;

    // An exponent marker is part of the longest valid prefix only when at
    // least one decimal exponent digit follows its optional sign.
    if matches!(units.get(cursor), Some(&unit) if unit == u16::from(b'e') || unit == u16::from(b'E'))
    {
        cursor += 1;
        let exponent_is_negative = if matches!(units.get(cursor), Some(&unit) if unit == u16::from(b'+'))
        {
            cursor += 1;
            false
        } else if matches!(units.get(cursor), Some(&unit) if unit == u16::from(b'-')) {
            cursor += 1;
            true
        } else {
            false
        };
        let exponent_start = cursor;
        while units.get(cursor).copied().is_some_and(is_ascii_digit) {
            cursor += 1;
        }
        if cursor != exponent_start {
            // Keep enough explicit exponent to cancel any decimal-point
            // offset present in this allocation-sized input. Only the final
            // combined scale may be clamped: pinned js_atod accepts cases such
            // as 10,000 fractional zeroes followed by `1e10001`.
            let cancellation_limit = i64::try_from(units.len())
                .unwrap_or(i64::MAX - EXPONENT_LIMIT)
                .saturating_add(EXPONENT_LIMIT);
            explicit_exponent = decimal_exponent(
                &units[exponent_start..cursor],
                exponent_is_negative,
                cancellation_limit,
            );
        }
    }

    let mantissa_digits = units[mantissa_start..mantissa_end]
        .iter()
        .copied()
        .filter(|unit| is_ascii_digit(*unit))
        .collect::<Vec<_>>();
    let Some(significant_start) = mantissa_digits
        .iter()
        .position(|unit| *unit != u16::from(b'0'))
    else {
        return if negative { -0.0 } else { 0.0 };
    };
    let significant_digits = &mantissa_digits[significant_start..];
    let retained_count = significant_digits
        .len()
        .min(ATOD_MAX_DIGITS[usize::from(10_u8 - 2)]);
    let retained = significant_digits[..retained_count]
        .iter()
        .map(|unit| u8::try_from(*unit).expect("validated parseFloat digits are ASCII"))
        .collect::<Vec<_>>();

    // QuickJS represents the retained decimal as M * 10^scale. Positions are
    // counted in digits, so the decimal point itself does not consume one.
    let decimal_point = i64::try_from(integer_digits).unwrap_or(i64::MAX);
    let significant_start = i64::try_from(significant_start).unwrap_or(i64::MAX);
    let retained_count = i64::try_from(retained_count).expect("retained digit count fits i64");
    let scale = explicit_exponent
        .saturating_add(decimal_point)
        .saturating_sub(significant_start)
        .saturating_sub(retained_count)
        .clamp(-EXPONENT_LIMIT, EXPONENT_LIMIT);
    let number = retained_decimal_to_f64(&retained, scale);

    // Preserve the sign even on implementations where decimal underflow is
    // returned as an unsigned zero.
    if negative { -number } else { number }
}

fn radix_integer_to_f64(digits: &[u8], radix: u32) -> f64 {
    let Some(significant_start) = digits.iter().position(|digit| *digit != b'0') else {
        return 0.0;
    };
    let significant = &digits[significant_start..];
    let max_digits = ATOD_MAX_DIGITS[usize::try_from(radix - 2).expect("validated radix")];
    let retained_count = significant.len().min(max_digits);
    let mut retained = BigUint::parse_bytes(&significant[..retained_count], radix)
        .expect("a non-empty validated digit prefix must parse as BigUint");

    if radix.is_power_of_two()
        && significant[retained_count..]
            .iter()
            .any(|digit| *digit != b'0')
    {
        retained |= BigUint::from(1_u8);
    }

    let discarded_count = significant.len() - retained_count;
    // Any radix^discarded_count above this conservative bound is far beyond
    // binary64 overflow. Avoid constructing a BigUint proportional to an
    // attacker-controlled input length.
    if discarded_count > 2_048 {
        return f64::INFINITY;
    }
    if discarded_count != 0 {
        retained *= BigUint::from(radix)
            .pow(u32::try_from(discarded_count).expect("bounded discarded count fits u32"));
    }
    retained
        .to_f64()
        .expect("BigUint to f64 conversion returns a finite value or infinity")
}

fn decimal_exponent(digits: &[u16], negative: bool, limit: i64) -> i64 {
    let mut exponent = 0_i64;
    for digit in digits {
        exponent = exponent
            .saturating_mul(10)
            .saturating_add(i64::from(*digit - u16::from(b'0')))
            .min(limit);
    }
    if negative { -exponent } else { exponent }
}

fn retained_decimal_to_f64(digits: &[u8], scale: i64) -> f64 {
    let mantissa = std::str::from_utf8(digits).expect("validated decimal digits are UTF-8");
    let normalized = format!("{mantissa}e{scale}");
    normalized
        .parse::<f64>()
        .expect("normalized retained decimal must parse as f64")
}

fn skip_leading_space(units: &[u16]) -> usize {
    units
        .iter()
        .position(|unit| !is_ecmascript_whitespace(*unit))
        .unwrap_or(units.len())
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

fn has_ascii_prefix(units: &[u16], prefix: &[u8]) -> bool {
    units.len() >= prefix.len()
        && units
            .iter()
            .zip(prefix)
            .all(|(unit, byte)| *unit == u16::from(*byte))
}

const fn is_ascii_digit(unit: u16) -> bool {
    unit >= 0x30 && unit <= 0x39
}

const fn ascii_digit_value(unit: u16) -> Option<u32> {
    match unit {
        0x30..=0x39 => Some((unit - 0x30) as u32),
        0x61..=0x7a => Some((unit - 0x61) as u32 + 10),
        0x41..=0x5a => Some((unit - 0x41) as u32 + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_float, parse_int};
    use crate::value::JsString;

    fn string(source: &str) -> JsString {
        JsString::from_utf8(source)
    }

    fn assert_same_number(actual: f64, expected: f64) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "actual {actual:?}, expected {expected:?}"
        );
    }

    #[test]
    fn parse_int_trims_space_selects_radix_and_stops_at_the_longest_prefix() {
        assert_same_number(parse_int(&string("  -0x10tail"), 0), -16.0);
        assert_same_number(parse_int(&string("+0Xf"), 16), 15.0);
        assert_same_number(parse_int(&string("0x10"), 10), 0.0);
        assert_same_number(parse_int(&string("0b11"), 0), 0.0);
        assert_same_number(parse_int(&string("10102"), 2), 10.0);
        assert_same_number(parse_int(&string("zZ!"), 36), 1_295.0);
        assert_same_number(parse_int(&string("10"), 2), 2.0);
        assert_same_number(parse_int(&string("10"), 8), 8.0);
        assert_same_number(parse_int(&string("10"), 36), 36.0);
    }

    #[test]
    fn parse_int_rejects_missing_digits_and_invalid_radices() {
        for source in ["", " ", "+", "-", "xyz", "0x", "-0x"] {
            assert!(parse_int(&string(source), 0).is_nan(), "source {source:?}");
        }
        assert!(parse_int(&string("10"), 1).is_nan());
        assert!(parse_int(&string("10"), 37).is_nan());
        assert!(parse_int(&string("10"), -2).is_nan());
    }

    #[test]
    fn parse_int_rounds_large_integers_once_and_preserves_negative_zero() {
        assert_same_number(
            parse_int(&string("9007199254740993"), 10),
            9_007_199_254_740_992.0,
        );
        assert_same_number(
            parse_int(&string("9007199254740995"), 10),
            9_007_199_254_740_996.0,
        );
        assert_same_number(
            parse_int(&string("20000000000001"), 16),
            9_007_199_254_740_992.0,
        );
        assert_same_number(parse_int(&string("-000tail"), 10), -0.0);

        let overflowing = string(&format!("1{}", "0".repeat(400)));
        assert_same_number(parse_int(&overflowing, 10), f64::INFINITY);
    }

    #[test]
    fn parse_int_preserves_the_pinned_non_power_of_two_digit_limit() {
        // The exact 39-digit integer lies just above a binary64 midpoint.
        // QuickJS retains 38 decimal digits and truncates the last one, so it
        // selects the lower neighbor rather than the specification-level
        // exact-integer result selected by engines without this bound.
        let parsed = parse_int(&string("300000000000000031025361333325263798273"), 10);
        assert_eq!(parsed.to_bits(), 0x47ec_363c_bf21_f28a);
    }

    #[test]
    fn parse_float_accepts_infinity_and_the_longest_decimal_prefix() {
        assert_same_number(parse_float(&string("  +Infinitytail")), f64::INFINITY);
        assert_same_number(
            parse_float(&string("-Infinity and beyond")),
            f64::NEG_INFINITY,
        );
        assert_same_number(parse_float(&string("-1.25e2tail")), -125.0);
        assert_same_number(parse_float(&string("1.e2")), 100.0);
        assert_same_number(parse_float(&string(".5e+2")), 50.0);
        assert_same_number(parse_float(&string("0x10")), 0.0);
        assert_same_number(parse_float(&string("1e")), 1.0);
        assert_same_number(parse_float(&string("1e+")), 1.0);
        assert_same_number(parse_float(&string("1e-")), 1.0);
    }

    #[test]
    fn parse_float_rejects_non_decimal_starts() {
        for source in ["", " ", "+", "-", ".", ".e2", "NaN", "infinity"] {
            assert!(parse_float(&string(source)).is_nan(), "source {source:?}");
        }
    }

    #[test]
    fn parse_float_rounds_boundaries_and_preserves_signed_underflow() {
        assert_same_number(
            parse_float(&string("9007199254740993")),
            9_007_199_254_740_992.0,
        );
        assert_same_number(
            parse_float(&string("9007199254740995")),
            9_007_199_254_740_996.0,
        );
        assert_same_number(parse_float(&string("2.4703282292062327e-324")), 0.0);
        assert_same_number(
            parse_float(&string("2.4703282292062328e-324")),
            f64::from_bits(1),
        );
        assert_same_number(parse_float(&string("1.7976931348623158e308")), f64::MAX);
        assert_same_number(
            parse_float(&string("1.7976931348623159e308")),
            f64::INFINITY,
        );
        assert_same_number(parse_float(&string("-0x")), -0.0);
        assert_same_number(parse_float(&string("-1e-9999")), -0.0);
    }

    #[test]
    fn parse_float_preserves_the_pinned_38_digit_decimal_truncation() {
        // This is just above the exact midpoint between 1 and nextUp(1).
        // Pinned QuickJS discards the digits after its 38-digit atod mantissa
        // and therefore returns the even lower neighbor.
        assert_same_number(
            parse_float(&string(
                "1.0000000000000001110223024625156540423631668090820313",
            )),
            1.0,
        );
    }

    #[test]
    fn parse_float_combines_large_exponents_before_final_scale_clamping() {
        let positive_cancellation = format!("0.{}1e10001", "0".repeat(10_000));
        assert_same_number(parse_float(&string(&positive_cancellation)), 1.0);

        let negative_cancellation = format!("1{}e-10000", "0".repeat(10_000));
        assert_same_number(parse_float(&string(&negative_cancellation)), 1.0);
    }

    #[test]
    fn both_parsers_use_the_pinned_ecmascript_space_set() {
        for whitespace in [
            0x0009, 0x000a, 0x000b, 0x000c, 0x000d, 0x0020, 0x00a0, 0x1680, 0x2000, 0x200a, 0x2028,
            0x2029, 0x202f, 0x205f, 0x3000, 0xfeff,
        ] {
            let input = JsString::from_utf16([whitespace, u16::from(b'1')]);
            assert_same_number(parse_int(&input, 10), 1.0);
            assert_same_number(parse_float(&input), 1.0);
        }

        for non_whitespace in [0x0085, 0x180e] {
            let input = JsString::from_utf16([non_whitespace, u16::from(b'1')]);
            assert!(parse_int(&input, 10).is_nan());
            assert!(parse_float(&input).is_nan());
        }
    }

    #[test]
    fn embedded_nul_terminates_the_prefix_without_losing_prior_digits() {
        let input = JsString::from_utf16([u16::from(b'1'), u16::from(b'2'), 0, u16::from(b'3')]);
        assert_same_number(parse_int(&input, 10), 12.0);
        assert_same_number(parse_float(&input), 12.0);
    }
}
