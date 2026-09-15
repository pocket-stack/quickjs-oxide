//! IEEE-754 binary16 conversion; no JavaScript conversion or Runtime state.

/// Decode one IEEE-754 binary16 bit pattern with pinned QuickJS
/// `cutils.h::fromfp16` semantics.
///
/// The deliberately unusual construction mirrors QuickJS exactly: place the
/// half payload in a binary64, then scale it by `2^1008`. Besides avoiding a
/// platform conversion dependency, this preserves signed zero, infinities,
/// and the source NaN sign and payload behavior.
#[must_use]
pub fn from_float16_bits(value: u16) -> f64 {
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
pub fn to_float16_bits(value: f64) -> u16 {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
