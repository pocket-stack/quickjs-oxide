//! Integer conversion kernels after JavaScript ToNumber has completed.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
