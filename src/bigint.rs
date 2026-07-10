//! `QuickJS`-compatible arbitrary-precision integer values.
//!
//! This module mirrors the two representations used by `QuickJS` 2026-06-04:
//! values fitting in one signed host limb use an immediate `i64`, while larger
//! values are reference-counted arbitrary-precision integers. Every operation
//! compacts its result back to the immediate representation when possible.
//!
//! `QuickJS` stores heap BigInts as normalized, little-endian two's-complement
//! limbs. `num_bigint::BigInt` is sign/magnitude internally, so the few places
//! where the representation is observable (notably `BigInt.asUintN`) explicitly
//! emulate the upstream signed-limb behavior.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::rc::Rc;
use std::str::FromStr;

use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{One, ToPrimitive, Zero};

/// `QuickJS` normally limits a BigInt allocation to
/// `(1024 * 1024) / JS_LIMB_BITS` limbs.
///
/// On the 64-bit compatibility target this is 16,384 limbs, or 1,048,576
/// signed representation bits. `js_bigint_extend` can append one sign limb
/// after that guarded allocation, so selected add/sub/neg/shift results may
/// temporarily occupy 16,385 limbs exactly as in the pinned release.
pub const MAX_BIGINT_BITS: u64 = 1024 * 1024;

/// The host limb width used by the `QuickJS` 64-bit value representation.
pub const BIGINT_LIMB_BITS: u64 = 64;

const MAX_BIGINT_LIMBS: u64 = MAX_BIGINT_BITS / BIGINT_LIMB_BITS;

/// A failure produced by the BigInt value layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BigIntError {
    /// A textual value is not a valid BigInt.
    InvalidSyntax,
    /// A formatting or parsing radix is outside `2..=36`.
    InvalidRadix(u32),
    /// Division or remainder by zero was requested.
    DivisionByZero,
    /// Exponentiation was requested with a negative exponent.
    NegativeExponent,
    /// The result would exceed the upstream BigInt allocation limit.
    BigIntTooLarge,
    /// An operation tried to allocate an input-sized BigInt whose normalized
    /// representation came from QuickJS's one-limb extension bypass.
    AllocationTooLarge,
    /// A left shift would exceed the upstream BigInt allocation limit.
    ShiftTooLarge,
}

impl fmt::Display for BigIntError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSyntax => formatter.write_str("invalid BigInt syntax"),
            Self::InvalidRadix(radix) => {
                write!(
                    formatter,
                    "invalid BigInt radix {radix}; expected 2 through 36"
                )
            }
            Self::DivisionByZero => formatter.write_str("BigInt division by zero"),
            Self::NegativeExponent => formatter.write_str("BigInt negative exponent"),
            Self::BigIntTooLarge => formatter.write_str("BigInt is too large"),
            Self::AllocationTooLarge => formatter.write_str("BigInt is too large to allocate"),
            Self::ShiftTooLarge => formatter.write_str("BigInt shift is too large"),
        }
    }
}

impl std::error::Error for BigIntError {}

#[derive(Clone, Eq, Hash, PartialEq)]
enum BigIntRepr {
    Short(i64),
    Heap(Rc<BigInt>),
}

/// An ECMAScript BigInt using the same short/heap split as `QuickJS`.
///
/// The representation is private so callers cannot construct a non-normalized
/// heap value that should have been a short BigInt.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct JsBigInt(BigIntRepr);

impl JsBigInt {
    /// Construct zero in the short representation.
    #[must_use]
    pub const fn zero() -> Self {
        Self(BigIntRepr::Short(0))
    }

    /// Construct one in the short representation.
    #[must_use]
    pub const fn one() -> Self {
        Self(BigIntRepr::Short(1))
    }

    /// Construct and normalize a `num_bigint::BigInt`.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::BigIntTooLarge`] when the signed limb
    /// representation exceeds the `QuickJS` allocation limit.
    pub fn from_bigint(value: BigInt) -> Result<Self, BigIntError> {
        if signed_limb_len(&value) > MAX_BIGINT_LIMBS {
            return Err(BigIntError::BigIntTooLarge);
        }
        Ok(Self::normalize(value))
    }

    /// Parse the string form accepted by the ECMAScript `BigInt` constructor.
    ///
    /// Leading and trailing ECMAScript whitespace is ignored. The empty string
    /// is zero. Decimal text may have a leading `+` or `-`; unsigned `0b`, `0o`,
    /// and `0x` prefixes are accepted. Numeric separators and a trailing `n`
    /// are deliberately rejected, as they are not part of `StringToBigInt`.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::InvalidSyntax`] for malformed input and
    /// [`BigIntError::BigIntTooLarge`] for values beyond the upstream limit.
    pub fn parse_js_string(source: &str) -> Result<Self, BigIntError> {
        let source = source.trim_matches(is_ecmascript_whitespace);
        if source.is_empty() {
            return Ok(Self::zero());
        }

        let bytes = source.as_bytes();
        match bytes[0] {
            b'+' | b'-' => {
                let negative = bytes[0] == b'-';
                let digits = &source[1..];
                // Signed non-decimal prefixes are rejected by StringToBigInt.
                parse_digits(digits, 10, negative)
            }
            _ if bytes.len() >= 2 && bytes[0] == b'0' => {
                let radix = match bytes[1] {
                    b'b' | b'B' => Some(2),
                    b'o' | b'O' => Some(8),
                    b'x' | b'X' => Some(16),
                    _ => None,
                };
                if let Some(radix) = radix {
                    parse_digits(&source[2..], radix, false)
                } else {
                    parse_digits(source, 10, false)
                }
            }
            _ => parse_digits(source, 10, false),
        }
    }

    /// Parse an optionally signed digit sequence in `radix`.
    ///
    /// Unlike [`Self::parse_js_string`], this low-level entry point does not
    /// trim whitespace or recognize a base prefix.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::InvalidRadix`],
    /// [`BigIntError::InvalidSyntax`], or [`BigIntError::BigIntTooLarge`].
    pub fn parse_radix(source: &str, radix: u32) -> Result<Self, BigIntError> {
        validate_radix(radix)?;
        let (negative, digits) = match source.as_bytes().first().copied() {
            Some(b'-') => (true, &source[1..]),
            Some(b'+') => (false, &source[1..]),
            _ => (false, source),
        };
        parse_digits(digits, radix, negative)
    }

    /// Format the integer with lowercase digits in `radix`.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::InvalidRadix`] unless `radix` is in `2..=36`.
    pub fn to_string_radix(&self, radix: u32) -> Result<String, BigIntError> {
        validate_radix(radix)?;
        Ok(match &self.0 {
            BigIntRepr::Short(value) => short_to_string_radix(*value, radix),
            BigIntRepr::Heap(value) => value.to_str_radix(radix),
        })
    }

    /// Whether this value uses the `JS_TAG_SHORT_BIG_INT`-like fast path.
    #[must_use]
    pub const fn is_short(&self) -> bool {
        matches!(self.0, BigIntRepr::Short(_))
    }

    /// Return the immediate value, or `None` for a heap BigInt.
    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        match self.0 {
            BigIntRepr::Short(value) => Some(value),
            BigIntRepr::Heap(_) => None,
        }
    }

    /// Return a cloned arbitrary-precision value for external integrations.
    #[must_use]
    pub fn to_bigint(&self) -> BigInt {
        match &self.0 {
            BigIntRepr::Short(value) => BigInt::from(*value),
            BigIntRepr::Heap(value) => value.as_ref().clone(),
        }
    }

    /// The unsigned magnitude bit length (`0` for zero).
    #[must_use]
    pub fn magnitude_bits(&self) -> u64 {
        match &self.0 {
            BigIntRepr::Short(value) => {
                let magnitude = value.unsigned_abs();
                u64::from(u64::BITS - magnitude.leading_zeros())
            }
            BigIntRepr::Heap(value) => value.bits(),
        }
    }

    /// Number of normalized signed 64-bit limbs used by `QuickJS`.
    #[must_use]
    pub fn signed_limb_len(&self) -> u64 {
        match &self.0 {
            BigIntRepr::Short(_) => 1,
            BigIntRepr::Heap(value) => signed_limb_len(value),
        }
    }

    pub(crate) fn exceeds_allocation_limit(&self) -> bool {
        self.signed_limb_len() > MAX_BIGINT_LIMBS
    }

    /// Whether this value is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        match &self.0 {
            BigIntRepr::Short(value) => *value == 0,
            BigIntRepr::Heap(value) => value.is_zero(),
        }
    }

    /// Whether this value is negative.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        match &self.0 {
            BigIntRepr::Short(value) => *value < 0,
            BigIntRepr::Heap(value) => value.sign() == Sign::Minus,
        }
    }

    /// Add two BigInts, promoting on short overflow and compacting the result.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::BigIntTooLarge`] if the result exceeds the
    /// upstream allocation limit.
    pub fn add(&self, rhs: &Self) -> Result<Self, BigIntError> {
        let short_result = match (&self.0, &rhs.0) {
            (BigIntRepr::Short(left), BigIntRepr::Short(right)) => left.checked_add(*right),
            _ => None,
        };
        if let Some(result) = short_result {
            return Ok(Self::from(result));
        }
        self.ensure_operands_allocatable(rhs)?;
        self.checked_extending_binary(rhs, |left, right| left + right)
    }

    /// Subtract two BigInts, promoting on short overflow and compacting the
    /// result.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::BigIntTooLarge`] if the result exceeds the
    /// upstream allocation limit.
    pub fn sub(&self, rhs: &Self) -> Result<Self, BigIntError> {
        let short_result = match (&self.0, &rhs.0) {
            (BigIntRepr::Short(left), BigIntRepr::Short(right)) => left.checked_sub(*right),
            _ => None,
        };
        if let Some(result) = short_result {
            return Ok(Self::from(result));
        }
        self.ensure_operands_allocatable(rhs)?;
        self.checked_extending_binary(rhs, |left, right| left - right)
    }

    /// Multiply two BigInts, promoting on short overflow and compacting the
    /// result.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::BigIntTooLarge`] if the result exceeds the
    /// upstream allocation limit.
    pub fn mul(&self, rhs: &Self) -> Result<Self, BigIntError> {
        let short_result = match (&self.0, &rhs.0) {
            (BigIntRepr::Short(left), BigIntRepr::Short(right)) => left.checked_mul(*right),
            _ => None,
        };
        if let Some(result) = short_result {
            return Ok(Self::from(result));
        }
        self.ensure_operands_allocatable(rhs)?;
        let allocated_limbs = self
            .signed_limb_len()
            .checked_add(rhs.signed_limb_len())
            .ok_or(BigIntError::AllocationTooLarge)?;
        if allocated_limbs > MAX_BIGINT_LIMBS {
            return Err(BigIntError::AllocationTooLarge);
        }
        self.checked_binary(rhs, |left, right| left * right)
    }

    /// Divide with truncation toward zero, as required for ECMAScript BigInt.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::DivisionByZero`] when `rhs` is zero.
    pub fn div(&self, rhs: &Self) -> Result<Self, BigIntError> {
        if rhs.is_zero() {
            return Err(BigIntError::DivisionByZero);
        }
        let short_result = match (&self.0, &rhs.0) {
            (BigIntRepr::Short(left), BigIntRepr::Short(right)) => left.checked_div(*right),
            _ => None,
        };
        if let Some(result) = short_result {
            return Ok(Self::from(result));
        }
        if self
            .signed_limb_len()
            .checked_add(2)
            .is_none_or(|allocated_limbs| allocated_limbs > MAX_BIGINT_LIMBS)
        {
            return Err(BigIntError::AllocationTooLarge);
        }
        // The only overflowing i64 division is MIN / -1. The arbitrary-
        // precision path naturally promotes it instead of panicking.
        self.checked_binary(rhs, |left, right| left / right)
    }

    /// Compute a remainder whose sign follows the dividend.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::DivisionByZero`] when `rhs` is zero.
    pub fn rem(&self, rhs: &Self) -> Result<Self, BigIntError> {
        if rhs.is_zero() {
            return Err(BigIntError::DivisionByZero);
        }
        if let (BigIntRepr::Short(left), BigIntRepr::Short(right)) = (&self.0, &rhs.0) {
            // Rust reports MIN % -1 as overflow even though the mathematical
            // result is zero.
            if *left == i64::MIN && *right == -1 {
                return Ok(Self::zero());
            }
            return Ok(Self::from(left % right));
        }
        if self
            .signed_limb_len()
            .checked_add(2)
            .is_none_or(|allocated_limbs| allocated_limbs > MAX_BIGINT_LIMBS)
        {
            return Err(BigIntError::AllocationTooLarge);
        }
        self.checked_binary(rhs, |left, right| left % right)
    }

    /// Raise this value to a non-negative BigInt exponent.
    ///
    /// `0`, `1`, and `-1` retain the same constant-result shortcuts as
    /// `js_bigint_pow`, even when the exponent itself is too large to fit a
    /// machine integer.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::NegativeExponent`] for a negative exponent,
    /// [`BigIntError::BigIntTooLarge`] when the mathematical exponent exceeds
    /// the upstream limit, and [`BigIntError::AllocationTooLarge`] when an
    /// intermediate or final allocation crosses the nominal limb guard.
    pub fn pow(&self, exponent: &Self) -> Result<Self, BigIntError> {
        if exponent.is_negative() {
            return Err(BigIntError::NegativeExponent);
        }
        if exponent.is_zero() {
            return Ok(Self::one());
        }
        if exponent.is_one() {
            if self.exceeds_allocation_limit() {
                return Err(BigIntError::AllocationTooLarge);
            }
            return Ok(self.clone());
        }
        if self.is_zero() || self.is_one() {
            return Ok(self.clone());
        }
        if self.is_negative_one() {
            return Ok(if exponent.is_odd() {
                Self::from(-1)
            } else {
                Self::one()
            });
        }

        let exponent = exponent
            .to_u32()
            .filter(|value| *value <= i32::MAX as u32)
            .ok_or(BigIntError::BigIntTooLarge)?;

        // js_bigint_pow has an exact-allocation shortcut for one-limb powers
        // of two. Besides being faster, this is observable at the allocation
        // boundary because the generic multiplier reserves both input widths.
        if let Some(base) = self.as_i64() {
            let magnitude = base.unsigned_abs();
            if magnitude.is_power_of_two() {
                let base_shift = u64::from(magnitude.trailing_zeros());
                let result_shift = u64::from(exponent) * base_shift;
                if result_shift > MAX_BIGINT_BITS {
                    return Err(BigIntError::BigIntTooLarge);
                }
                let mut result = BigInt::one()
                    << usize::try_from(result_shift)
                        .expect("bounded QuickJS BigInt power shift fits usize");
                if base.is_negative() && exponent & 1 != 0 {
                    result = -result;
                }
                return Self::from_bigint(result).map_err(|error| match error {
                    BigIntError::BigIntTooLarge => BigIntError::AllocationTooLarge,
                    error => error,
                });
            }
        }

        // The generic upstream algorithm starts with `a`, then scans the
        // remaining exponent bits from most to least significant. Matching
        // that order avoids an observable final `1 * result` allocation near
        // the 16,384-limb boundary.
        if self.exceeds_allocation_limit() {
            return Err(BigIntError::AllocationTooLarge);
        }
        let highest_bit = u32::BITS - exponent.leading_zeros();
        let mut result = self.clone();
        for bit in (0..highest_bit - 1).rev() {
            result = result.mul(&result)?;
            if exponent & (1 << bit) != 0 {
                result = result.mul(self)?;
            }
        }
        Ok(result)
    }

    /// Arithmetic negation, promoting `i64::MIN` to a heap BigInt.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::BigIntTooLarge`] if the result exceeds the
    /// upstream allocation limit.
    pub fn neg(&self) -> Result<Self, BigIntError> {
        let short_result = match &self.0 {
            BigIntRepr::Short(value) => value.checked_neg(),
            BigIntRepr::Heap(_) => None,
        };
        if let Some(result) = short_result {
            return Ok(Self::from(result));
        }
        if self.exceeds_allocation_limit() {
            return Err(BigIntError::AllocationTooLarge);
        }
        let result = -self.as_bigint().as_ref();
        if signed_limb_len(&result) > MAX_BIGINT_LIMBS + 1 {
            return Err(BigIntError::AllocationTooLarge);
        }
        Ok(Self::normalize(result))
    }

    /// Infinite-width two's-complement bitwise AND.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::AllocationTooLarge`] when either operand came
    /// from QuickJS's unguarded one-sign-limb extension.
    pub fn bit_and(&self, rhs: &Self) -> Result<Self, BigIntError> {
        if let (BigIntRepr::Short(left), BigIntRepr::Short(right)) = (&self.0, &rhs.0) {
            return Ok(Self::from(left & right));
        }
        self.ensure_operands_allocatable(rhs)?;
        Ok(self.unchecked_binary(rhs, |left, right| left & right))
    }

    /// Infinite-width two's-complement bitwise OR.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::AllocationTooLarge`] when either operand came
    /// from QuickJS's unguarded one-sign-limb extension.
    pub fn bit_or(&self, rhs: &Self) -> Result<Self, BigIntError> {
        if let (BigIntRepr::Short(left), BigIntRepr::Short(right)) = (&self.0, &rhs.0) {
            return Ok(Self::from(left | right));
        }
        self.ensure_operands_allocatable(rhs)?;
        Ok(self.unchecked_binary(rhs, |left, right| left | right))
    }

    /// Infinite-width two's-complement bitwise XOR.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::AllocationTooLarge`] when either operand came
    /// from QuickJS's unguarded one-sign-limb extension.
    pub fn bit_xor(&self, rhs: &Self) -> Result<Self, BigIntError> {
        if let (BigIntRepr::Short(left), BigIntRepr::Short(right)) = (&self.0, &rhs.0) {
            return Ok(Self::from(left ^ right));
        }
        self.ensure_operands_allocatable(rhs)?;
        Ok(self.unchecked_binary(rhs, |left, right| left ^ right))
    }

    /// Infinite-width two's-complement bitwise NOT.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::AllocationTooLarge`] when the operand came from
    /// QuickJS's unguarded one-sign-limb extension.
    pub fn bit_not(&self) -> Result<Self, BigIntError> {
        if self.exceeds_allocation_limit() {
            return Err(BigIntError::AllocationTooLarge);
        }
        Ok(match &self.0 {
            BigIntRepr::Short(value) => Self::from(!value),
            BigIntRepr::Heap(value) => Self::normalize(!value.as_ref()),
        })
    }

    /// ECMAScript BigInt left shift. A negative count shifts right.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::ShiftTooLarge`] when a non-zero left-shift result
    /// would exceed the `QuickJS` allocation limit.
    pub fn shl(&self, count: &Self) -> Result<Self, BigIntError> {
        self.shift(count, true)
    }

    /// ECMAScript BigInt arithmetic right shift. A negative count shifts left.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::ShiftTooLarge`] when a reversed (left) shift
    /// would exceed the `QuickJS` allocation limit.
    pub fn shr(&self, count: &Self) -> Result<Self, BigIntError> {
        self.shift(count, false)
    }

    /// `QuickJS` 2026-06-04 behavior for `BigInt.asIntN(bits, value)`.
    #[must_use]
    pub fn as_int_n(&self, bits: u64) -> Self {
        self.as_n(bits, true)
    }

    /// `QuickJS` 2026-06-04 behavior for `BigInt.asUintN(bits, value)`.
    ///
    /// This intentionally preserves the upstream signed-limb behavior instead
    /// of silently substituting the ECMA-262 result. In this release, a short
    /// negative value is returned unchanged for `bits >= 64`; heap truncation
    /// at a multiple of 64 can likewise produce a negative value. These are
    /// observable compatibility requirements for the selected baseline.
    #[must_use]
    pub fn as_uint_n(&self, bits: u64) -> Self {
        self.as_n(bits, false)
    }

    fn normalize(value: BigInt) -> Self {
        if let Some(value) = value.to_i64() {
            Self(BigIntRepr::Short(value))
        } else {
            Self(BigIntRepr::Heap(Rc::new(value)))
        }
    }

    fn as_bigint(&self) -> Cow<'_, BigInt> {
        match &self.0 {
            BigIntRepr::Short(value) => Cow::Owned(BigInt::from(*value)),
            BigIntRepr::Heap(value) => Cow::Borrowed(value.as_ref()),
        }
    }

    fn checked_binary(
        &self,
        rhs: &Self,
        operation: impl FnOnce(&BigInt, &BigInt) -> BigInt,
    ) -> Result<Self, BigIntError> {
        let left = self.as_bigint();
        let right = rhs.as_bigint();
        Self::from_bigint(operation(left.as_ref(), right.as_ref())).map_err(|error| match error {
            BigIntError::BigIntTooLarge => BigIntError::AllocationTooLarge,
            error => error,
        })
    }

    fn checked_extending_binary(
        &self,
        rhs: &Self,
        operation: impl FnOnce(&BigInt, &BigInt) -> BigInt,
    ) -> Result<Self, BigIntError> {
        let left = self.as_bigint();
        let right = rhs.as_bigint();
        let result = operation(left.as_ref(), right.as_ref());
        if signed_limb_len(&result) > MAX_BIGINT_LIMBS + 1 {
            return Err(BigIntError::AllocationTooLarge);
        }
        Ok(Self::normalize(result))
    }

    fn ensure_operands_allocatable(&self, rhs: &Self) -> Result<(), BigIntError> {
        if self.exceeds_allocation_limit() || rhs.exceeds_allocation_limit() {
            Err(BigIntError::AllocationTooLarge)
        } else {
            Ok(())
        }
    }

    fn unchecked_binary(
        &self,
        rhs: &Self,
        operation: impl FnOnce(&BigInt, &BigInt) -> BigInt,
    ) -> Self {
        let left = self.as_bigint();
        let right = rhs.as_bigint();
        let result = operation(left.as_ref(), right.as_ref());
        debug_assert!(signed_limb_len(&result) <= MAX_BIGINT_LIMBS);
        Self::normalize(result)
    }

    fn is_one(&self) -> bool {
        matches!(self.0, BigIntRepr::Short(1))
    }

    fn is_negative_one(&self) -> bool {
        matches!(self.0, BigIntRepr::Short(-1))
    }

    fn is_odd(&self) -> bool {
        match &self.0 {
            BigIntRepr::Short(value) => value & 1 != 0,
            BigIntRepr::Heap(value) => value.magnitude().bit(0),
        }
    }

    fn to_u32(&self) -> Option<u32> {
        match &self.0 {
            BigIntRepr::Short(value) => u32::try_from(*value).ok(),
            BigIntRepr::Heap(value) => value.to_u32(),
        }
    }

    fn shift(&self, count: &Self, operator_shifts_left: bool) -> Result<Self, BigIntError> {
        let shifts_left = operator_shifts_left != count.is_negative();
        let count = count
            .magnitude_to_u64()
            .unwrap_or(u64::MAX)
            .min(i32::MAX as u64);
        if shifts_left {
            // js_bigint_shl returns zero before attempting an allocation.
            if self.is_zero() {
                return Ok(Self::zero());
            }
            let allocated_limbs = self
                .signed_limb_len()
                .checked_add(count / BIGINT_LIMB_BITS)
                .ok_or(BigIntError::ShiftTooLarge)?;
            if allocated_limbs > MAX_BIGINT_LIMBS {
                return Err(BigIntError::ShiftTooLarge);
            }
            let count = usize::try_from(count).map_err(|_| BigIntError::ShiftTooLarge)?;
            let result = self.as_bigint().as_ref() << count;
            if signed_limb_len(&result) > MAX_BIGINT_LIMBS + 1 {
                return Err(BigIntError::ShiftTooLarge);
            }
            Ok(Self::normalize(result))
        } else {
            let limb_offset = count / BIGINT_LIMB_BITS;
            if limb_offset >= self.signed_limb_len() {
                return Ok(self.right_shift_saturation());
            }
            if self.signed_limb_len() - limb_offset > MAX_BIGINT_LIMBS {
                return Err(BigIntError::ShiftTooLarge);
            }
            let count = usize::try_from(count).expect("bounded BigInt shift fits usize");
            Ok(Self::normalize(self.as_bigint().as_ref() >> count))
        }
    }

    fn right_shift_saturation(&self) -> Self {
        if self.is_negative() {
            Self::from(-1)
        } else {
            Self::zero()
        }
    }

    fn magnitude_to_u64(&self) -> Option<u64> {
        match &self.0 {
            BigIntRepr::Short(value) => Some(value.unsigned_abs()),
            BigIntRepr::Heap(value) => value.magnitude().to_u64(),
        }
    }

    fn as_n(&self, bits: u64, signed: bool) -> Self {
        if bits == 0 {
            return Self::zero();
        }

        // This early return is representation-dependent in upstream. In
        // particular, asUintN(128, -1n) returns -1n in QuickJS 2026-06-04.
        let storage_bits = self.signed_limb_len() * BIGINT_LIMB_BITS;
        if bits >= storage_bits {
            return self.clone();
        }

        // At this point bits < storage_bits <= MAX_BIGINT_BITS.
        let bits_usize = usize::try_from(bits).expect("bounded BigInt bit width fits usize");
        let modulus = BigInt::one() << bits_usize;
        let mut truncated = self.as_bigint().as_ref() % &modulus;
        if truncated.sign() == Sign::Minus {
            truncated += &modulus;
        }

        // js_bigint_asUintN writes a whole signed limb when bits is a multiple
        // of JS_LIMB_BITS. If that limb's top bit is set, normalization observes
        // a negative result even for asUintN. Preserve that release behavior.
        let interpret_as_signed = signed || bits & (BIGINT_LIMB_BITS - 1) == 0;
        if interpret_as_signed {
            let sign_threshold = &modulus >> 1usize;
            if truncated >= sign_threshold {
                truncated -= modulus;
            }
        }
        Self::normalize(truncated)
    }
}

impl Default for JsBigInt {
    fn default() -> Self {
        Self::zero()
    }
}

impl From<i64> for JsBigInt {
    fn from(value: i64) -> Self {
        Self(BigIntRepr::Short(value))
    }
}

impl From<i32> for JsBigInt {
    fn from(value: i32) -> Self {
        Self::from(i64::from(value))
    }
}

impl From<u32> for JsBigInt {
    fn from(value: u32) -> Self {
        Self::from(i64::from(value))
    }
}

impl From<u64> for JsBigInt {
    fn from(value: u64) -> Self {
        if let Ok(value) = i64::try_from(value) {
            Self::from(value)
        } else {
            Self::normalize(BigInt::from(value))
        }
    }
}

impl From<i128> for JsBigInt {
    fn from(value: i128) -> Self {
        Self::normalize(BigInt::from(value))
    }
}

impl From<u128> for JsBigInt {
    fn from(value: u128) -> Self {
        Self::normalize(BigInt::from(value))
    }
}

impl TryFrom<BigInt> for JsBigInt {
    type Error = BigIntError;

    fn try_from(value: BigInt) -> Result<Self, Self::Error> {
        Self::from_bigint(value)
    }
}

impl FromStr for JsBigInt {
    type Err = BigIntError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::parse_js_string(source)
    }
}

impl Ord for JsBigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (BigIntRepr::Short(left), BigIntRepr::Short(right)) => left.cmp(right),
            _ => self.as_bigint().cmp(&other.as_bigint()),
        }
    }
}

impl PartialOrd for JsBigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for JsBigInt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            BigIntRepr::Short(value) => value.fmt(formatter),
            BigIntRepr::Heap(value) => value.fmt(formatter),
        }
    }
}

impl fmt::Debug for JsBigInt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            BigIntRepr::Short(value) => formatter.debug_tuple("ShortBigInt").field(value).finish(),
            BigIntRepr::Heap(value) => formatter.debug_tuple("BigInt").field(value).finish(),
        }
    }
}

fn validate_radix(radix: u32) -> Result<(), BigIntError> {
    if (2..=36).contains(&radix) {
        Ok(())
    } else {
        Err(BigIntError::InvalidRadix(radix))
    }
}

fn parse_digits(digits: &str, radix: u32, negative: bool) -> Result<JsBigInt, BigIntError> {
    validate_radix(radix)?;
    if digits.is_empty()
        || !digits
            .as_bytes()
            .iter()
            .all(|byte| is_radix_digit(*byte, radix))
    {
        return Err(BigIntError::InvalidSyntax);
    }

    // js_bigint_from_string discards leading zeroes before its conservative
    // source-length guard, so an arbitrarily long spelling of zero stays zero.
    let significant_digits = digits.trim_start_matches('0');
    if significant_digits.is_empty() {
        return Ok(JsBigInt::zero());
    }
    if u64::try_from(significant_digits.len()).unwrap_or(u64::MAX) > MAX_BIGINT_BITS {
        return Err(BigIntError::BigIntTooLarge);
    }

    let magnitude = BigUint::parse_bytes(significant_digits.as_bytes(), radix)
        .ok_or(BigIntError::InvalidSyntax)?;
    let sign = if negative && !magnitude.is_zero() {
        Sign::Minus
    } else if magnitude.is_zero() {
        Sign::NoSign
    } else {
        Sign::Plus
    };
    JsBigInt::from_bigint(BigInt::from_biguint(sign, magnitude))
}

fn is_radix_digit(byte: u8, radix: u32) -> bool {
    let value = match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'z' => u32::from(byte - b'a') + 10,
        b'A'..=b'Z' => u32::from(byte - b'A') + 10,
        _ => return false,
    };
    value < radix
}

fn short_to_string_radix(value: i64, radix: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

    if value == 0 {
        return "0".to_owned();
    }

    let negative = value < 0;
    let mut magnitude = value.unsigned_abs();
    let radix = u64::from(radix);
    let mut reversed = Vec::with_capacity(65);
    while magnitude != 0 {
        let digit = usize::try_from(magnitude % radix).expect("radix digit fits usize");
        reversed.push(DIGITS[digit]);
        magnitude /= radix;
    }
    if negative {
        reversed.push(b'-');
    }
    reversed.reverse();
    String::from_utf8(reversed).expect("BigInt formatting only emits ASCII")
}

fn signed_limb_len(value: &BigInt) -> u64 {
    let signed_bits = match value.sign() {
        Sign::NoSign => 1,
        Sign::Plus => value.bits() + 1,
        Sign::Minus => {
            let mut below_magnitude = value.magnitude().clone();
            below_magnitude -= 1u8;
            below_magnitude.bits() + 1
        }
    };
    signed_bits.div_ceil(BIGINT_LIMB_BITS).max(1)
}

fn is_ecmascript_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::{BigIntError, JsBigInt, MAX_BIGINT_BITS, MAX_BIGINT_LIMBS};

    fn bigint(source: &str) -> JsBigInt {
        JsBigInt::parse_js_string(source).unwrap()
    }

    #[test]
    fn normalizes_short_values_and_promotes_overflow() {
        let max = JsBigInt::from(i64::MAX);
        let min = JsBigInt::from(i64::MIN);
        assert!(max.is_short());
        assert!(min.is_short());

        let above_max = max.add(&JsBigInt::one()).unwrap();
        let below_min = min.sub(&JsBigInt::one()).unwrap();
        assert!(!above_max.is_short());
        assert!(!below_min.is_short());
        assert_eq!(above_max.to_string(), "9223372036854775808");
        assert_eq!(below_min.to_string(), "-9223372036854775809");

        assert_eq!(above_max.sub(&JsBigInt::one()).unwrap(), max);
        assert_eq!(below_min.add(&JsBigInt::one()).unwrap(), min);
        assert!(above_max.sub(&JsBigInt::one()).unwrap().is_short());
    }

    #[test]
    fn signed_limb_width_matches_quickjs_twos_complement_normalization() {
        assert_eq!(JsBigInt::from(i64::MAX).signed_limb_len(), 1);
        assert_eq!(JsBigInt::from(i64::MIN).signed_limb_len(), 1);
        assert_eq!(bigint("9223372036854775808").signed_limb_len(), 2);
        assert_eq!(bigint("-9223372036854775809").signed_limb_len(), 2);
        assert_eq!(
            bigint("170141183460469231731687303715884105727").signed_limb_len(),
            2
        );
        assert_eq!(
            bigint("170141183460469231731687303715884105728").signed_limb_len(),
            3
        );
        assert_eq!(
            bigint("-170141183460469231731687303715884105728").signed_limb_len(),
            2
        );
        assert_eq!(
            bigint("-170141183460469231731687303715884105729").signed_limb_len(),
            3
        );
    }

    #[test]
    fn parses_string_to_bigint_forms_and_formats_all_radices() {
        assert_eq!(bigint(""), JsBigInt::zero());
        assert_eq!(bigint("\u{00a0}  +123 \u{feff}"), JsBigInt::from(123));
        assert_eq!(bigint("-00123"), JsBigInt::from(-123));
        assert_eq!(bigint("0b101010"), JsBigInt::from(42));
        assert_eq!(bigint("0O77"), JsBigInt::from(63));
        assert_eq!(bigint("0xabcdef"), JsBigInt::from(0x00ab_cdef));

        let large = bigint("1267650600228229401496703205376");
        assert_eq!(
            large.to_string_radix(10).unwrap(),
            "1267650600228229401496703205376"
        );
        assert_eq!(
            large.to_string_radix(8).unwrap(),
            "2000000000000000000000000000000000"
        );
        assert_eq!(
            large.to_string_radix(16).unwrap(),
            "10000000000000000000000000"
        );
        assert_eq!(
            bigint("-1267650600228229401496703205376")
                .to_string_radix(36)
                .unwrap(),
            "-3ewfdnca0n6ld1ggvfgg"
        );

        assert_eq!(JsBigInt::parse_radix("z", 36).unwrap(), JsBigInt::from(35));
        assert_eq!(
            JsBigInt::parse_radix("+ff", 16).unwrap(),
            JsBigInt::from(255)
        );
        assert_eq!(
            JsBigInt::parse_radix("-ff", 16).unwrap(),
            JsBigInt::from(-255)
        );
    }

    #[test]
    fn rejects_invalid_string_to_bigint_forms() {
        for source in ["+", "-", "-0x1", "+0x1", "0x", "1_0", "1n", "\0a", " 12 r "] {
            assert_eq!(
                JsBigInt::parse_js_string(source),
                Err(BigIntError::InvalidSyntax)
            );
        }
        assert_eq!(
            JsBigInt::parse_radix("2", 2),
            Err(BigIntError::InvalidSyntax)
        );
        assert_eq!(
            JsBigInt::parse_radix("1", 1),
            Err(BigIntError::InvalidRadix(1))
        );
        assert_eq!(
            JsBigInt::one().to_string_radix(37),
            Err(BigIntError::InvalidRadix(37))
        );
    }

    #[test]
    fn arithmetic_matches_upstream_large_integer_vectors() {
        let three_to_100 = JsBigInt::from(3).pow(&JsBigInt::from(100)).unwrap();
        assert_eq!(
            three_to_100.to_string(),
            "515377520732011331036461129765621272702107522001"
        );

        let dividend = bigint("3213213213213213432453243");
        let divisor = bigint("123434343439");
        assert_eq!(
            dividend.div(&divisor).unwrap().to_string(),
            "26031760073331"
        );
        assert_eq!(dividend.rem(&divisor).unwrap().to_string(), "26953727934");
        let negative = dividend.neg().unwrap();
        assert_eq!(
            negative.div(&divisor).unwrap().to_string(),
            "-26031760073331"
        );
        assert_eq!(negative.rem(&divisor).unwrap().to_string(), "-26953727934");

        assert_eq!(
            JsBigInt::from(-2)
                .pow(&JsBigInt::from(127))
                .unwrap()
                .to_string(),
            "-170141183460469231731687303715884105728"
        );
        assert_eq!(
            JsBigInt::from(7)
                .pow(&JsBigInt::from(20))
                .unwrap()
                .to_string(),
            "79792266297612001"
        );
    }

    #[test]
    fn division_promotes_min_overflow_and_reports_zero() {
        let promoted = JsBigInt::from(i64::MIN).div(&JsBigInt::from(-1)).unwrap();
        assert_eq!(promoted.to_string(), "9223372036854775808");
        assert!(!promoted.is_short());
        assert_eq!(
            JsBigInt::from(i64::MIN).rem(&JsBigInt::from(-1)).unwrap(),
            JsBigInt::zero()
        );
        assert_eq!(
            JsBigInt::one().div(&JsBigInt::zero()),
            Err(BigIntError::DivisionByZero)
        );
        assert_eq!(
            JsBigInt::one().rem(&JsBigInt::zero()),
            Err(BigIntError::DivisionByZero)
        );
    }

    #[test]
    fn pow_handles_sign_shortcuts_and_explicit_errors() {
        assert_eq!(
            JsBigInt::from(123).pow(&JsBigInt::zero()).unwrap(),
            JsBigInt::one()
        );
        assert_eq!(
            JsBigInt::from(-1)
                .pow(&bigint("999999999999999999999999999999999999999"))
                .unwrap(),
            JsBigInt::from(-1)
        );
        assert_eq!(
            JsBigInt::from(2).pow(&JsBigInt::from(-1)),
            Err(BigIntError::NegativeExponent)
        );
        assert_eq!(
            JsBigInt::from(2).pow(&bigint("2147483648")),
            Err(BigIntError::BigIntTooLarge)
        );

        let half_width_base = JsBigInt::one()
            .shl(&JsBigInt::from(MAX_BIGINT_BITS / 2 - 2))
            .unwrap()
            .add(&JsBigInt::one())
            .unwrap();
        assert_eq!(
            half_width_base.pow(&JsBigInt::from(2)).unwrap(),
            half_width_base.mul(&half_width_base).unwrap()
        );

        let max_power_of_two = JsBigInt::from(2)
            .pow(&JsBigInt::from(MAX_BIGINT_BITS - 2))
            .unwrap();
        assert_eq!(max_power_of_two.signed_limb_len(), MAX_BIGINT_LIMBS);
        assert_eq!(
            JsBigInt::from(2).pow(&JsBigInt::from(MAX_BIGINT_BITS - 1)),
            Err(BigIntError::AllocationTooLarge)
        );
        assert_eq!(
            JsBigInt::from(-2).pow(&JsBigInt::from(MAX_BIGINT_BITS)),
            Err(BigIntError::AllocationTooLarge)
        );
        assert_eq!(
            JsBigInt::from(2).pow(&JsBigInt::from(MAX_BIGINT_BITS + 1)),
            Err(BigIntError::BigIntTooLarge)
        );
        assert!(
            JsBigInt::from(-2)
                .pow(&JsBigInt::from(MAX_BIGINT_BITS - 1))
                .unwrap()
                .is_negative()
        );
    }

    #[test]
    fn shifts_match_arithmetic_and_negative_count_semantics() {
        let value = bigint("0x5a4653ca673768565b41f775");
        assert_eq!(
            value.shl(&JsBigInt::from(78)).unwrap().to_string(),
            "8443945299673273647701379149826607537748959488376832"
        );
        assert_eq!(
            value.shr(&JsBigInt::from(78)).unwrap(),
            JsBigInt::from(92441)
        );
        assert_eq!(
            value.neg().unwrap().shr(&JsBigInt::from(78)).unwrap(),
            JsBigInt::from(-92442)
        );
        assert_eq!(
            JsBigInt::from(8).shl(&JsBigInt::from(-1)).unwrap(),
            JsBigInt::from(4)
        );
        assert_eq!(
            JsBigInt::from(8).shr(&JsBigInt::from(-2)).unwrap(),
            JsBigInt::from(32)
        );
    }

    #[test]
    fn excessive_left_shift_is_an_explicit_error_but_right_shift_saturates() {
        let huge = JsBigInt::from(MAX_BIGINT_BITS + 1);
        assert_eq!(JsBigInt::one().shl(&huge), Err(BigIntError::ShiftTooLarge));
        assert_eq!(JsBigInt::zero().shl(&huge).unwrap(), JsBigInt::zero());
        assert_eq!(JsBigInt::one().shr(&huge).unwrap(), JsBigInt::zero());
        assert_eq!(JsBigInt::from(-1).shr(&huge).unwrap(), JsBigInt::from(-1));
        assert_eq!(
            JsBigInt::one().shr(&huge.neg().unwrap()),
            Err(BigIntError::ShiftTooLarge)
        );
    }

    #[test]
    fn shift_preserves_quickjs_unguarded_sign_limb_extension() {
        let boundary_count = JsBigInt::from(MAX_BIGINT_BITS - 1);
        let boundary = JsBigInt::one().shl(&boundary_count).unwrap();
        assert_eq!(boundary.signed_limb_len(), MAX_BIGINT_LIMBS + 1);
        assert_eq!(boundary.shr(&boundary_count).unwrap(), JsBigInt::one());

        assert_eq!(
            boundary.shl(&JsBigInt::zero()),
            Err(BigIntError::ShiftTooLarge)
        );
        assert_eq!(
            boundary.shr(&JsBigInt::zero()),
            Err(BigIntError::ShiftTooLarge)
        );
        assert_eq!(
            boundary.bit_and(&JsBigInt::from(-1)),
            Err(BigIntError::AllocationTooLarge)
        );
        assert_eq!(boundary.bit_not(), Err(BigIntError::AllocationTooLarge));
        assert_eq!(
            boundary.add(&JsBigInt::zero()),
            Err(BigIntError::AllocationTooLarge)
        );
        assert_eq!(boundary.neg(), Err(BigIntError::AllocationTooLarge));

        assert_eq!(
            JsBigInt::one().shl(&JsBigInt::from(MAX_BIGINT_BITS)),
            Err(BigIntError::ShiftTooLarge)
        );
        assert_eq!(
            JsBigInt::zero().shl(&JsBigInt::from(u64::MAX)).unwrap(),
            JsBigInt::zero()
        );

        let near_boundary = JsBigInt::one()
            .shl(&JsBigInt::from(MAX_BIGINT_BITS - 2))
            .unwrap();
        assert_eq!(near_boundary.add(&JsBigInt::zero()).unwrap(), near_boundary);
        assert_eq!(
            near_boundary.bit_and(&JsBigInt::from(-1)).unwrap(),
            near_boundary
        );
        assert_eq!(
            near_boundary.mul(&JsBigInt::zero()),
            Err(BigIntError::AllocationTooLarge)
        );
        assert_eq!(
            near_boundary.div(&JsBigInt::one()),
            Err(BigIntError::AllocationTooLarge)
        );
        assert_eq!(
            near_boundary.rem(&JsBigInt::one()),
            Err(BigIntError::AllocationTooLarge)
        );
        assert_eq!(near_boundary.pow(&JsBigInt::one()).unwrap(), near_boundary);
        let added_boundary = near_boundary.add(&near_boundary).unwrap();
        assert_eq!(added_boundary, boundary);

        let negative_boundary = JsBigInt::from(-1)
            .shl(&JsBigInt::from(MAX_BIGINT_BITS - 1))
            .unwrap();
        assert_eq!(negative_boundary.neg().unwrap(), boundary);
        assert_eq!(
            boundary.pow(&JsBigInt::one()),
            Err(BigIntError::AllocationTooLarge)
        );
        assert_eq!(boundary.pow(&JsBigInt::zero()).unwrap(), JsBigInt::one());
    }

    #[test]
    fn bitwise_operations_use_infinite_twos_complement() {
        assert_eq!(
            JsBigInt::from(0x5a65_3ca6).bit_not().unwrap(),
            JsBigInt::from(-1_516_584_103)
        );
        let left = JsBigInt::from(0x5a46_3ca6);
        let right = JsBigInt::from(0x6737_6856);
        assert_eq!(left.bit_or(&right).unwrap(), JsBigInt::from(2_138_537_206));
        assert_eq!(left.bit_and(&right).unwrap(), JsBigInt::from(1_107_699_718));
        assert_eq!(left.bit_xor(&right).unwrap(), JsBigInt::from(1_030_837_488));

        let huge = bigint("1208925819614629174706176"); // 2^80
        assert_eq!(
            huge.bit_or(&JsBigInt::from(-1)).unwrap(),
            JsBigInt::from(-1)
        );
        assert_eq!(huge.bit_and(&JsBigInt::from(-1)).unwrap(), huge);
    }

    #[test]
    fn comparison_is_numeric_across_storage_classes() {
        let short = JsBigInt::from(i64::MAX);
        let heap = short.add(&JsBigInt::one()).unwrap();
        assert!(short < heap);
        assert!(bigint("-9223372036854775809") < JsBigInt::from(i64::MIN));
        assert_eq!(
            bigint("123456789012345678901234567890"),
            bigint("123456789012345678901234567890")
        );
    }

    #[test]
    fn as_int_n_truncates_and_sign_extends() {
        assert_eq!(JsBigInt::from(257).as_int_n(8), JsBigInt::one());
        assert_eq!(JsBigInt::from(255).as_int_n(8), JsBigInt::from(-1));
        assert_eq!(JsBigInt::from(-129).as_int_n(8), JsBigInt::from(127));
        assert_eq!(
            bigint("18446744073709551616").as_int_n(65),
            bigint("-18446744073709551616")
        );
        assert_eq!(
            bigint("36893488147419103231").as_int_n(65),
            JsBigInt::from(-1)
        );
        assert_eq!(
            bigint("999999999999999999999").as_int_n(0),
            JsBigInt::zero()
        );
    }

    #[test]
    fn quickjs_2026_as_uint_n_preserves_signed_limb_quirks() {
        // These results were verified against qjs 2026-06-04. They differ from
        // ECMA-262/Node, but observable baseline parity takes precedence here.
        assert_eq!(JsBigInt::from(-1).as_uint_n(64), JsBigInt::from(-1));
        assert_eq!(JsBigInt::from(-1).as_uint_n(128), JsBigInt::from(-1));
        assert_eq!(
            bigint("9223372036854775808").as_uint_n(64),
            JsBigInt::from(i64::MIN)
        );
        assert_eq!(
            bigint("18446744073709551615").as_uint_n(64),
            JsBigInt::from(-1)
        );
        assert_eq!(
            bigint("-18446744073709551616").as_uint_n(128),
            bigint("-18446744073709551616")
        );
        // Non-limb-aligned truncation clears the spare sign bits and is
        // positive, matching js_bigint_asUintN's logical shift path.
        assert_eq!(
            bigint("-18446744073709551616").as_uint_n(65),
            bigint("18446744073709551616")
        );
    }

    #[test]
    fn arbitrary_precision_is_not_capped_at_i128() {
        let value = JsBigInt::from(3).pow(&JsBigInt::from(1000)).unwrap();
        assert!(value.magnitude_bits() > 128);
        let text = value.to_string();
        assert_eq!(text.len(), 478);
        assert!(text.starts_with("1322070819480806636890455259752144365965"));
        assert_eq!(JsBigInt::parse_js_string(&text).unwrap(), value);
    }
}
