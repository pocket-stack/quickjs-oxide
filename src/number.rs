//! Exact numeric formatting primitives for the pinned QuickJS release.
//!
//! QuickJS does not delegate `Number.prototype` formatting to libc. Its
//! `dtoa.c` computes in an integer domain, uses round-to-nearest/ties-to-even
//! for the free shortest representation, and round-to-nearest/ties-away for
//! fixed digit counts. This module follows those rules with `BigUint` rational
//! arithmetic so the result is independent of Rust's or the platform's float
//! formatting implementation.

use std::cmp::Ordering;

use num_bigint::BigUint;
use num_traits::{One, ToPrimitive, Zero};

const MIN_RADIX: u32 = 2;
const MAX_RADIX: u32 = 36;
const MAX_DIGITS: i32 = 100;

/// Pinned QuickJS `js_pow` kernel shared by the `**` bytecode and
/// `Math.pow`.  C's `pow` result for a unit-magnitude base and an infinite
/// exponent is not the ECMAScript result, so QuickJS handles it explicitly.
#[must_use]
pub(crate) fn pow(base: f64, exponent: f64) -> f64 {
    if !exponent.is_finite() && base.abs() == 1.0 {
        f64::NAN
    } else {
        base.powf(exponent)
    }
}

/// Decode one IEEE-754 binary16 bit pattern with pinned QuickJS
/// `cutils.h::fromfp16` semantics.
///
/// The deliberately unusual construction mirrors QuickJS exactly: place the
/// half payload in a binary64, then scale it by `2^1008`. Besides avoiding a
/// platform conversion dependency, this preserves signed zero, infinities,
/// and the source NaN sign and payload behavior.
#[must_use]
pub(crate) fn from_float16_bits(value: u16) -> f64 {
    let mut magnitude = u32::from(value & 0x7fff);
    if magnitude >= 0x7c00 {
        magnitude += 0x1f_8000;
    }
    let bits =
        (u64::from(value >> 15) << 63) | (u64::from(magnitude) << (f64::MANTISSA_DIGITS - 11));
    const SCALE_2_POW_1008: f64 = f64::from_bits(0x7ef0_0000_0000_0000);
    f64::from_bits(bits) * SCALE_2_POW_1008
}

/// Encode a binary64 as IEEE-754 binary16 with pinned QuickJS
/// `cutils.h::tofp16` semantics.
///
/// Conversion is round-to-nearest, ties-to-even. Overflow becomes infinity;
/// every NaN is canonicalized to payload `1` while retaining its sign.
#[must_use]
pub(crate) fn to_float16_bits(value: f64) -> u16 {
    let bits = value.to_bits();
    let sign = u16::try_from((bits >> 63) << 15).expect("a binary64 sign always fits binary16");
    let mut magnitude = bits & 0x7fff_ffff_ffff_ffff;

    let encoded = if magnitude > 0x7ff0_0000_0000_0000 {
        0x7c01
    } else if magnitude < 0x3f10_0000_0000_0000 {
        if magnitude <= 0x3e60_0000_0000_0000 {
            0
        } else {
            let shift =
                u32::try_from(1051 - (magnitude >> 52)).expect("binary16 subnormal shift fits u32");
            magnitude = (1_u64 << 52) | (magnitude & ((1_u64 << 52) - 1));
            let addend = ((magnitude >> shift) & 1) + ((1_u64 << (shift - 1)) - 1);
            (magnitude + addend) >> shift
        }
    } else {
        magnitude -= 0x3f00_0000_0000_0000;
        let addend = ((magnitude >> (52 - 10)) & 1) + ((1_u64 << (52 - 11)) - 1);
        let rounded = (magnitude + addend) >> (52 - 10);
        if rounded > 0x7c00 { 0x7c00 } else { rounded }
    };

    u16::try_from(encoded).expect("binary16 encoding fits u16") | sign
}

// QuickJS `dtoa_max_digits_table`. Each entry is a conservative count which
// guarantees that rounding a binary64 value in the corresponding radix and
// parsing it again can reproduce the original value.
const DTOA_MAX_DIGITS: [i32; 35] = [
    54, 35, 28, 24, 22, 20, 19, 18, 17, 17, 16, 16, 15, 15, 15, 14, 14, 14, 14, 14, 13, 13, 13, 13,
    13, 13, 13, 12, 12, 12, 12, 12, 12, 12, 12,
];

/// A range failure produced after the caller has performed JavaScript value
/// conversion. Runtime integration maps these two cases to the pinned
/// `RangeError` messages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NumberFormatError {
    InvalidRadix,
    InvalidDigits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoundingMode {
    /// QuickJS `JS_RNDN`: round to nearest, resolving a tie to an even integer.
    NearestEven,
    /// QuickJS `JS_RNDNA`: round to nearest, resolving a tie away from zero.
    NearestAway,
}

#[derive(Clone, Debug)]
struct BinaryMagnitude {
    significand: BigUint,
    exponent: i32,
}

#[derive(Clone, Debug)]
struct DigitSequence {
    mantissa: BigUint,
    digits: i32,
    /// The decimal/radix point position counted from the start of `mantissa`.
    /// The represented value is `mantissa * radix^(exponent - digits)`.
    exponent: i32,
}

/// Format a Number using the `Number.prototype.toString(radix)` FREE path.
///
/// Radix ten uses QuickJS's automatic decimal-exponent thresholds. Every
/// other radix disables exponential notation, exactly as `js_number_toString`
/// does before entering `js_dtoa`.
pub fn to_string_radix(value: f64, radix: u32) -> Result<String, NumberFormatError> {
    if !(MIN_RADIX..=MAX_RADIX).contains(&radix) {
        return Err(NumberFormatError::InvalidRadix);
    }
    Ok(format_free(value, radix, radix == 10))
}

/// Format `value` with exactly `fraction_digits` digits after the decimal
/// point, matching `Number.prototype.toFixed`.
pub fn to_fixed(value: f64, fraction_digits: i32) -> Result<String, NumberFormatError> {
    if !(0..=MAX_DIGITS).contains(&fraction_digits) {
        return Err(NumberFormatError::InvalidDigits);
    }
    if let Some(special) = format_non_finite(value) {
        return Ok(special);
    }

    // The pinned implementation switches back to FREE formatting at this
    // threshold instead of appending the requested fractional zeroes.
    if value.abs() >= 1e21 {
        return Ok(format_free(value, 10, true));
    }

    let negative = value.is_sign_negative() && value != 0.0;
    let magnitude = BinaryMagnitude::from_f64(value.abs());
    let rounded = round_scaled(&magnitude, 10, fraction_digits, RoundingMode::NearestAway);
    let mut digits = rounded.to_str_radix(10);
    if fraction_digits == 0 {
        return Ok(apply_sign(digits, negative));
    }

    let fraction_digits = usize::try_from(fraction_digits).expect("validated digit count");
    if digits.len() <= fraction_digits {
        let mut padded = String::with_capacity(fraction_digits + 2);
        padded.push('0');
        padded.push('.');
        padded.extend(std::iter::repeat_n('0', fraction_digits - digits.len()));
        padded.push_str(&digits);
        digits = padded;
    } else {
        digits.insert(digits.len() - fraction_digits, '.');
    }
    Ok(apply_sign(digits, negative))
}

/// Format `value` in exponential notation. `None` is the observable omitted
/// or `undefined` case and therefore requests the shortest mantissa. A present
/// count is checked only for finite values: pinned QuickJS still converts the
/// argument but returns `NaN`/`Infinity` before validating its range.
pub fn to_exponential(
    value: f64,
    fraction_digits: Option<i32>,
) -> Result<String, NumberFormatError> {
    if let Some(special) = format_non_finite(value) {
        return Ok(special);
    }
    if fraction_digits.is_some_and(|digits| !(0..=MAX_DIGITS).contains(&digits)) {
        return Err(NumberFormatError::InvalidDigits);
    }

    let negative = value.is_sign_negative() && value != 0.0;
    let sequence = match fraction_digits {
        Some(digits) => fixed_significant(value.abs(), digits + 1),
        None if value == 0.0 => DigitSequence {
            mantissa: BigUint::zero(),
            digits: 1,
            exponent: 1,
        },
        None => shortest_sequence(value.abs(), 10),
    };
    Ok(apply_sign(format_exponential(&sequence, 10), negative))
}

/// Format `value` with an optional significant-digit count, matching
/// `Number.prototype.toPrecision` and its automatic exponent threshold.
pub fn to_precision(value: f64, precision: Option<i32>) -> Result<String, NumberFormatError> {
    let Some(precision) = precision else {
        return Ok(format_free(value, 10, true));
    };
    if let Some(special) = format_non_finite(value) {
        return Ok(special);
    }
    if !(1..=MAX_DIGITS).contains(&precision) {
        return Err(NumberFormatError::InvalidDigits);
    }

    let negative = value.is_sign_negative() && value != 0.0;
    let sequence = fixed_significant(value.abs(), precision);
    let body = if sequence.exponent <= -6 || sequence.exponent > precision {
        format_exponential(&sequence, 10)
    } else {
        format_positional(&sequence, 10)
    };
    Ok(apply_sign(body, negative))
}

/// The pure numeric part of QuickJS `JS_ToInt32Sat` after `ToNumber` has
/// completed. NaN becomes zero, finite values truncate toward zero, and both
/// finite overflow and infinities saturate.
#[must_use]
pub fn to_int32_sat(value: f64) -> i32 {
    if value.is_nan() {
        0
    } else if value < f64::from(i32::MIN) {
        i32::MIN
    } else if value > f64::from(i32::MAX) {
        i32::MAX
    } else {
        value as i32
    }
}

/// The numeric kernel of ECMAScript `ToInt32` after `ToNumber` completes.
///
/// Unlike [`to_int32_sat`], this path truncates and then reduces modulo 2^32;
/// global `parseInt` uses it for its radix argument.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn to_int32(value: f64) -> i32 {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }

    const TWO_TO_31: f64 = 2_147_483_648.0;
    const TWO_TO_32: f64 = 4_294_967_296.0;
    let modulo = value.trunc() % TWO_TO_32;
    let unsigned = if modulo < 0.0 {
        modulo + TWO_TO_32
    } else {
        modulo
    };
    if unsigned >= TWO_TO_31 {
        (unsigned - TWO_TO_32) as i32
    } else {
        unsigned as i32
    }
}

fn format_free(value: f64, radix: u32, auto_exponent: bool) -> String {
    if let Some(special) = format_non_finite(value) {
        return special;
    }
    if value == 0.0 {
        return "0".to_owned();
    }

    let negative = value.is_sign_negative();
    let sequence = shortest_sequence(value.abs(), radix);
    let use_exponent = auto_exponent
        && (sequence.exponent <= -6 || sequence.exponent > DTOA_MAX_DIGITS[radix_index(radix)] + 4);
    let body = if use_exponent {
        format_exponential(&sequence, radix)
    } else {
        format_positional(&sequence, radix)
    };
    apply_sign(body, negative)
}

fn shortest_sequence(value: f64, radix: u32) -> DigitSequence {
    debug_assert!(value.is_finite() && value > 0.0);
    let bits = value.to_bits();
    let magnitude = BinaryMagnitude::from_f64(value);
    let base_exponent = radix_exponent(&magnitude, radix, value);
    let mut digits = DTOA_MAX_DIGITS[radix_index(radix)];
    let mut best = None;

    loop {
        let mut exponent = base_exponent;
        let mut mantissa;
        loop {
            mantissa = round_scaled(
                &magnitude,
                radix,
                digits - exponent,
                RoundingMode::NearestEven,
            );
            if mantissa < radix_power(radix, digits) {
                break;
            }
            exponent += 1;
        }

        while (&mantissa % radix).is_zero() {
            mantissa /= radix;
            digits -= 1;
        }

        let candidate = DigitSequence {
            mantissa,
            digits,
            exponent,
        };
        let roundtrips = best.is_none() || sequence_to_f64_bits(&candidate, radix) == bits;
        if !roundtrips {
            break;
        }
        best = Some(candidate);
        if digits == 1 {
            break;
        }
        digits -= 1;
    }

    best.expect("the QuickJS maximum digit table guarantees a roundtrip")
}

fn fixed_significant(value: f64, digits: i32) -> DigitSequence {
    debug_assert!(value.is_finite() && value >= 0.0 && digits >= 1);
    if value == 0.0 {
        return DigitSequence {
            mantissa: BigUint::zero(),
            digits,
            exponent: 1,
        };
    }

    let magnitude = BinaryMagnitude::from_f64(value);
    let mut exponent = radix_exponent(&magnitude, 10, value);
    loop {
        let mantissa = round_scaled(&magnitude, 10, digits - exponent, RoundingMode::NearestAway);
        if mantissa < radix_power(10, digits) {
            return DigitSequence {
                mantissa,
                digits,
                exponent,
            };
        }
        exponent += 1;
    }
}

fn format_positional(sequence: &DigitSequence, radix: u32) -> String {
    let digits = padded_digits(sequence, radix);
    let exponent = sequence.exponent;
    if exponent <= 0 {
        let zeroes = usize::try_from(-exponent).expect("finite f64 exponent fits usize");
        let mut output = String::with_capacity(2 + zeroes + digits.len());
        output.push('0');
        output.push('.');
        output.extend(std::iter::repeat_n('0', zeroes));
        output.push_str(&digits);
        output
    } else {
        let exponent = usize::try_from(exponent).expect("finite f64 exponent fits usize");
        if exponent >= digits.len() {
            let mut output = digits;
            output.extend(std::iter::repeat_n('0', exponent - output.len()));
            output
        } else {
            let mut output = digits;
            output.insert(exponent, '.');
            output
        }
    }
}

fn format_exponential(sequence: &DigitSequence, radix: u32) -> String {
    let digits = padded_digits(sequence, radix);
    let mut output = String::with_capacity(digits.len() + 8);
    output.push(digits.as_bytes()[0] as char);
    if digits.len() > 1 {
        output.push('.');
        output.push_str(&digits[1..]);
    }
    output.push('e');
    let exponent = sequence.exponent - 1;
    if exponent < 0 {
        output.push('-');
        output.push_str(&(-exponent).to_string());
    } else {
        output.push('+');
        output.push_str(&exponent.to_string());
    }
    output
}

fn padded_digits(sequence: &DigitSequence, radix: u32) -> String {
    let mut digits = sequence.mantissa.to_str_radix(radix);
    let requested = usize::try_from(sequence.digits).expect("positive digit count");
    if digits.len() < requested {
        let mut padded = String::with_capacity(requested);
        padded.extend(std::iter::repeat_n('0', requested - digits.len()));
        padded.push_str(&digits);
        digits = padded;
    }
    debug_assert_eq!(digits.len(), requested);
    digits
}

fn format_non_finite(value: f64) -> Option<String> {
    if value.is_nan() {
        Some("NaN".to_owned())
    } else if value == f64::INFINITY {
        Some("Infinity".to_owned())
    } else if value == f64::NEG_INFINITY {
        Some("-Infinity".to_owned())
    } else {
        None
    }
}

fn apply_sign(mut body: String, negative: bool) -> String {
    if negative {
        body.insert(0, '-');
    }
    body
}

impl BinaryMagnitude {
    fn from_f64(value: f64) -> Self {
        debug_assert!(value.is_finite() && value >= 0.0);
        let bits = value.to_bits();
        let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
        let fraction = bits & ((1_u64 << 52) - 1);
        if exponent_bits == 0 {
            Self {
                significand: BigUint::from(fraction),
                exponent: -1074,
            }
        } else {
            Self {
                significand: BigUint::from((1_u64 << 52) | fraction),
                exponent: exponent_bits - 1023 - 52,
            }
        }
    }

    fn fraction(&self) -> (BigUint, BigUint) {
        if self.exponent >= 0 {
            (
                &self.significand << usize::try_from(self.exponent).expect("nonnegative exponent"),
                BigUint::one(),
            )
        } else {
            (
                self.significand.clone(),
                BigUint::one()
                    << usize::try_from(-self.exponent).expect("negative exponent magnitude"),
            )
        }
    }
}

fn radix_exponent(magnitude: &BinaryMagnitude, radix: u32, approximate: f64) -> i32 {
    let (numerator, denominator) = magnitude.fraction();
    let mut exponent = (approximate.log(f64::from(radix)).floor() as i32) + 1;
    while compare_fraction_to_radix_power(&numerator, &denominator, radix, exponent)
        != Ordering::Less
    {
        exponent += 1;
    }
    while compare_fraction_to_radix_power(&numerator, &denominator, radix, exponent - 1)
        == Ordering::Less
    {
        exponent -= 1;
    }
    exponent
}

fn compare_fraction_to_radix_power(
    numerator: &BigUint,
    denominator: &BigUint,
    radix: u32,
    exponent: i32,
) -> Ordering {
    if exponent >= 0 {
        numerator.cmp(&(denominator * radix_power(radix, exponent)))
    } else {
        (numerator * radix_power(radix, -exponent)).cmp(denominator)
    }
}

fn round_scaled(
    magnitude: &BinaryMagnitude,
    radix: u32,
    radix_exponent: i32,
    mode: RoundingMode,
) -> BigUint {
    let (mut numerator, mut denominator) = magnitude.fraction();
    if radix_exponent >= 0 {
        numerator *= radix_power(radix, radix_exponent);
    } else {
        denominator *= radix_power(radix, -radix_exponent);
    }
    round_ratio(&numerator, &denominator, mode)
}

fn round_ratio(numerator: &BigUint, denominator: &BigUint, mode: RoundingMode) -> BigUint {
    let quotient = numerator / denominator;
    let remainder = numerator - (&quotient * denominator);
    let comparison = (&remainder << 1usize).cmp(denominator);
    let round_up = comparison == Ordering::Greater
        || (comparison == Ordering::Equal
            && (mode == RoundingMode::NearestAway || (&quotient & BigUint::one()).is_one()));
    if round_up {
        quotient + BigUint::one()
    } else {
        quotient
    }
}

fn radix_power(radix: u32, exponent: i32) -> BigUint {
    debug_assert!(exponent >= 0);
    BigUint::from(radix).pow(u32::try_from(exponent).expect("nonnegative exponent"))
}

fn sequence_to_f64_bits(sequence: &DigitSequence, radix: u32) -> u64 {
    let exponent = sequence.exponent - sequence.digits;
    let (numerator, denominator) = if exponent >= 0 {
        (
            &sequence.mantissa * radix_power(radix, exponent),
            BigUint::one(),
        )
    } else {
        (sequence.mantissa.clone(), radix_power(radix, -exponent))
    };
    rational_to_f64_bits(&numerator, &denominator)
}

fn rational_to_f64_bits(numerator: &BigUint, denominator: &BigUint) -> u64 {
    if numerator.is_zero() {
        return 0;
    }

    let mut exponent = i32::try_from(numerator.bits()).expect("BigUint bit count fits i32")
        - i32::try_from(denominator.bits()).expect("BigUint bit count fits i32");
    let below_guess = if exponent >= 0 {
        numerator < &(denominator << usize::try_from(exponent).expect("nonnegative exponent"))
    } else {
        &(numerator << usize::try_from(-exponent).expect("negative exponent magnitude"))
            < denominator
    };
    if below_guess {
        exponent -= 1;
    }

    if exponent < -1022 {
        let rounded = round_ratio(
            &(numerator << 1074usize),
            denominator,
            RoundingMode::NearestEven,
        );
        return rounded.to_u64().expect("subnormal mantissa fits u64");
    }
    if exponent > 1023 {
        return f64::INFINITY.to_bits();
    }

    let shift = 52 - exponent;
    let mut significand = if shift >= 0 {
        round_ratio(
            &(numerator << usize::try_from(shift).expect("nonnegative significand shift")),
            denominator,
            RoundingMode::NearestEven,
        )
    } else {
        round_ratio(
            numerator,
            &(denominator
                << usize::try_from(-shift).expect("negative significand shift magnitude")),
            RoundingMode::NearestEven,
        )
    };
    if significand.bits() > 53 {
        significand >>= 1usize;
        exponent += 1;
        if exponent > 1023 {
            return f64::INFINITY.to_bits();
        }
    }
    let significand = significand
        .to_u64()
        .expect("normal binary64 significand fits u64");
    let biased = u64::try_from(exponent + 1023).expect("normal exponent is nonnegative");
    (biased << 52) | (significand & ((1_u64 << 52) - 1))
}

const fn radix_index(radix: u32) -> usize {
    (radix - MIN_RADIX) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_decimal_matches_pinned_thresholds_and_edges() {
        for (value, expected) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.0, "-1"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (0.1 + 0.2, "0.30000000000000004"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
            (f64::from_bits(1), "5e-324"),
        ] {
            assert_eq!(to_string_radix(value, 10).unwrap(), expected);
        }
        assert_eq!(to_string_radix(f64::NAN, 10).unwrap(), "NaN");
        assert_eq!(to_string_radix(f64::INFINITY, 10).unwrap(), "Infinity");
        assert_eq!(to_string_radix(f64::NEG_INFINITY, 10).unwrap(), "-Infinity");
    }

    #[test]
    fn free_non_decimal_uses_shortest_roundtrip_without_exponents() {
        for (value, radix, expected) in [
            (255.0, 2, "11111111"),
            (255.0, 8, "377"),
            (255.0, 16, "ff"),
            (255.0, 36, "73"),
            (
                0.1,
                2,
                "0.0001100110011001100110011001100110011001100110011001101",
            ),
            (1.0 / 3.0, 36, "0.c"),
        ] {
            assert_eq!(to_string_radix(value, radix).unwrap(), expected);
        }

        let minimum_binary = to_string_radix(f64::from_bits(1), 2).unwrap();
        assert_eq!(minimum_binary.len(), 1076);
        assert!(minimum_binary.starts_with("0."));
        assert!(minimum_binary.ends_with('1'));
        assert!(
            minimum_binary[2..minimum_binary.len() - 1]
                .bytes()
                .all(|byte| byte == b'0')
        );
    }

    #[test]
    fn fixed_uses_quickjs_ties_away_and_large_value_fallback() {
        for (value, digits, expected) in [
            (0.0, 0, "0"),
            (-0.0, 2, "0.00"),
            (-0.1, 0, "-0"),
            (1.005, 2, "1.00"),
            (2.55, 1, "2.5"),
            (1.25, 1, "1.3"),
            (-1.25, 1, "-1.3"),
            (999_999_999_999_999_900_000.0, 2, "999999999999999868928.00"),
            (1e21, 2, "1e+21"),
        ] {
            assert_eq!(to_fixed(value, digits).unwrap(), expected);
        }
        assert_eq!(to_fixed(1.0, -1), Err(NumberFormatError::InvalidDigits));
        assert_eq!(
            to_fixed(f64::INFINITY, 101),
            Err(NumberFormatError::InvalidDigits)
        );
    }

    #[test]
    fn exponential_preserves_non_finite_validation_order() {
        for (value, digits, expected) in [
            (0.0, None, "0e+0"),
            (-0.0, None, "0e+0"),
            (1.0, None, "1e+0"),
            (1.25, Some(1), "1.3e+0"),
            (1234.0, Some(2), "1.23e+3"),
            (1e21, None, "1e+21"),
            (1e-7, None, "1e-7"),
        ] {
            assert_eq!(to_exponential(value, digits).unwrap(), expected);
        }
        assert_eq!(
            to_exponential(f64::INFINITY, Some(101)).unwrap(),
            "Infinity"
        );
        assert_eq!(
            to_exponential(1.0, Some(101)),
            Err(NumberFormatError::InvalidDigits)
        );
    }

    #[test]
    fn precision_uses_significant_digits_and_auto_exponents() {
        for (value, precision, expected) in [
            (0.0, None, "0"),
            (0.0, Some(1), "0"),
            (-0.0, Some(3), "0.00"),
            (1.25, Some(2), "1.3"),
            (1234.0, Some(2), "1.2e+3"),
            (0.0001234, Some(2), "0.00012"),
            (1e21, Some(4), "1.000e+21"),
        ] {
            assert_eq!(to_precision(value, precision).unwrap(), expected);
        }
        assert_eq!(to_precision(f64::INFINITY, Some(101)).unwrap(), "Infinity");
        assert_eq!(
            to_precision(1.0, Some(0)),
            Err(NumberFormatError::InvalidDigits)
        );
    }

    #[test]
    fn int32_sat_matches_quickjs_numeric_kernel() {
        assert_eq!(to_int32_sat(f64::NAN), 0);
        assert_eq!(to_int32_sat(-0.0), 0);
        assert_eq!(to_int32_sat(2.9), 2);
        assert_eq!(to_int32_sat(-2.9), -2);
        assert_eq!(to_int32_sat(f64::INFINITY), i32::MAX);
        assert_eq!(to_int32_sat(f64::NEG_INFINITY), i32::MIN);
        assert_eq!(to_int32_sat(4_294_967_298.0), i32::MAX);
        assert_eq!(to_int32_sat(-4_294_967_298.0), i32::MIN);
    }

    #[test]
    fn int32_wraps_for_parse_int_radices() {
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(f64::INFINITY), 0);
        assert_eq!(to_int32(2.9), 2);
        assert_eq!(to_int32(-2.9), -2);
        assert_eq!(to_int32(4_294_967_298.0), 2);
        assert_eq!(to_int32(-4_294_967_294.0), 2);
        assert_eq!(to_int32(2_147_483_648.0), i32::MIN);
    }

    #[test]
    fn pow_matches_quickjs_unit_base_infinity_rule() {
        assert!(pow(1.0, f64::INFINITY).is_nan());
        assert!(pow(-1.0, f64::NEG_INFINITY).is_nan());
        assert_eq!(pow(2.0, 10.0), 1024.0);
    }

    #[test]
    fn fp16_special_values_preserve_sign_and_canonicalize_nan() {
        for (half, expected) in [
            (0x0000, 0.0_f64),
            (0x8000, -0.0),
            (0x7c00, f64::INFINITY),
            (0xfc00, f64::NEG_INFINITY),
        ] {
            assert_eq!(from_float16_bits(half).to_bits(), expected.to_bits());
            assert_eq!(to_float16_bits(expected), half);
        }

        for bits in [
            0x7ff0_0000_0000_0001,
            0x7ff8_0000_0000_0000,
            0x7fff_ffff_ffff_ffff,
            0xfff0_0000_0000_0001,
            0xfff8_0000_0000_0000,
            0xffff_ffff_ffff_ffff,
        ] {
            let value = f64::from_bits(bits);
            let sign = if value.is_sign_negative() { 0x8000 } else { 0 };
            assert!(value.is_nan());
            assert_eq!(to_float16_bits(value), sign | 0x7c01);
        }

        for half in [0x7c01, 0x7dff, 0x7e00, 0x7fff, 0xfc01, 0xfe00, 0xffff] {
            let value = from_float16_bits(half);
            assert!(value.is_nan());
            assert_eq!(value.is_sign_negative(), half & 0x8000 != 0);
            assert_eq!(to_float16_bits(value), (half & 0x8000) | 0x7c01);
        }
    }

    #[test]
    fn fp16_normal_and_subnormal_boundaries_match_quickjs() {
        for (half, expected) in [
            (0x0001, 2.0_f64.powi(-24)),
            (0x03ff, 1023.0 * 2.0_f64.powi(-24)),
            (0x0400, 2.0_f64.powi(-14)),
            (0x3c00, 1.0),
            (0x7bff, 65_504.0),
            (0x8001, -2.0_f64.powi(-24)),
            (0x83ff, -1023.0 * 2.0_f64.powi(-24)),
            (0x8400, -2.0_f64.powi(-14)),
            (0xbbff, -0.999_511_718_75),
            (0xfbff, -65_504.0),
        ] {
            assert_eq!(from_float16_bits(half).to_bits(), expected.to_bits());
            assert_eq!(to_float16_bits(expected), half);
        }
    }

    #[test]
    fn fp16_rounds_every_tie_to_even() {
        let zero_subnormal_tie = 2.0_f64.powi(-25);
        assert_eq!(to_float16_bits(zero_subnormal_tie), 0x0000);
        assert_eq!(
            to_float16_bits(f64::from_bits(zero_subnormal_tie.to_bits() + 1)),
            0x0001
        );
        assert_eq!(to_float16_bits(3.0 * 2.0_f64.powi(-25)), 0x0002);

        let subnormal_normal_tie = (from_float16_bits(0x03ff) + from_float16_bits(0x0400)) / 2.0;
        assert_eq!(to_float16_bits(subnormal_normal_tie), 0x0400);

        assert_eq!(to_float16_bits(1.0 + 2.0_f64.powi(-11)), 0x3c00);
        assert_eq!(to_float16_bits(1.0 + 3.0 * 2.0_f64.powi(-11)), 0x3c02);
        assert_eq!(to_float16_bits(-1.0 - 2.0_f64.powi(-11)), 0xbc00);
        assert_eq!(to_float16_bits(-1.0 - 3.0 * 2.0_f64.powi(-11)), 0xbc02);

        assert_eq!(to_float16_bits(65_519.0), 0x7bff);
        assert_eq!(to_float16_bits(65_520.0), 0x7c00);
        assert_eq!(to_float16_bits(-65_520.0), 0xfc00);
    }

    #[test]
    fn fp16_exhaustive_decode_formula_and_reencode_invariant() {
        for half in 0_u16..=u16::MAX {
            let sign = half & 0x8000;
            let exponent = (half >> 10) & 0x1f;
            let fraction = half & 0x03ff;
            let decoded = from_float16_bits(half);

            if exponent == 0x1f {
                if fraction == 0 {
                    let expected = if sign == 0 {
                        f64::INFINITY
                    } else {
                        f64::NEG_INFINITY
                    };
                    assert_eq!(decoded.to_bits(), expected.to_bits());
                    assert_eq!(to_float16_bits(decoded), half);
                } else {
                    assert!(decoded.is_nan(), "{half:#06x} did not decode as NaN");
                    assert_eq!(decoded.is_sign_negative(), sign != 0);
                    assert_eq!(to_float16_bits(decoded), sign | 0x7c01);
                }
                continue;
            }

            let magnitude = if exponent == 0 {
                f64::from(fraction) * 2.0_f64.powi(-24)
            } else {
                f64::from(1024 + fraction) * 2.0_f64.powi(i32::from(exponent) - 25)
            };
            let expected = if sign == 0 { magnitude } else { -magnitude };
            assert_eq!(
                decoded.to_bits(),
                expected.to_bits(),
                "wrong decode for {half:#06x}"
            );
            assert_eq!(
                to_float16_bits(decoded),
                half,
                "wrong re-encode for {half:#06x}"
            );
        }
    }

    #[test]
    fn shortest_candidates_roundtrip_for_a_deterministic_bit_corpus() {
        let mut state = 0x243f_6a88_85a3_08d3_u64;
        for _ in 0..2_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let bits = state & !(1_u64 << 63);
            let value = f64::from_bits(bits);
            if !value.is_finite() || value == 0.0 {
                continue;
            }
            for radix in [2, 3, 8, 10, 16, 36] {
                let sequence = shortest_sequence(value, radix);
                assert_eq!(
                    sequence_to_f64_bits(&sequence, radix),
                    value.to_bits(),
                    "radix {radix} failed for bits {bits:#018x}"
                );
            }
        }
    }
}
