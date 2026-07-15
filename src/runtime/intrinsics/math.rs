//! Pinned QuickJS `Math` intrinsic algorithms.
//!
//! The public object table is bootstrapped by the parent runtime. This module
//! owns the native handlers and the numerical kernels so adding `Math` does not
//! grow the runtime facade or duplicate arithmetic behavior in the compiler.

use std::time::{SystemTime, UNIX_EPOCH};

use super::super::*;
use super::object::ObjectIteratorStep;

#[cfg(test)]
mod tests;

const SIGN_BIT: u64 = 1_u64 << 63;
const FRACTION_MASK: u64 = (1_u64 << 52) - 1;

const SP_LIMB_BITS: u32 = 56;
const SP_RND_BITS: u32 = SP_LIMB_BITS - 53;
const SP_LIMB_MASK: u64 = (1_u64 << SP_LIMB_BITS) - 1;
const SUM_PRECISE_ACC_LEN: usize = 39;
const SUM_PRECISE_COUNTER_INIT: u32 = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SumPreciseState {
    Finite,
    Infinity,
    MinusInfinity,
    Nan,
}

/// Exact port of pinned QuickJS's signed, redundant-limb accumulator.
///
/// The upstream build enables `-fwrapv`, and the algorithm observably relies
/// on signed wrapping before its 250-item renormalization interval. The
/// explicit wrapping operations below are therefore semantic compatibility,
/// not merely overflow hardening. In particular, replacing the limbs with an
/// `i128` accumulator changes pinned results for adversarial 129-item inputs.
struct SumPrecise {
    state: SumPreciseState,
    counter: u32,
    n_limbs: usize,
    acc: [i64; SUM_PRECISE_ACC_LEN],
}

impl SumPrecise {
    fn new() -> Self {
        Self {
            state: SumPreciseState::Finite,
            counter: SUM_PRECISE_COUNTER_INIT,
            n_limbs: 0,
            acc: [0; SUM_PRECISE_ACC_LEN],
        }
    }

    fn renormalize(&mut self) {
        let mut carry = 0_i64;
        for limb in &mut self.acc[..self.n_limbs] {
            let value = limb.wrapping_add(carry);
            *limb = (value as u64 & SP_LIMB_MASK) as i64;
            carry = value >> SP_LIMB_BITS;
        }
        // QuickJS calls this a failsafe which should not be reached in a
        // reasonable amount of time, but retaining it is part of the port.
        if carry != 0 && self.n_limbs < SUM_PRECISE_ACC_LEN {
            self.acc[self.n_limbs] = carry;
            self.n_limbs += 1;
        }
    }

    fn add(&mut self, value: f64) {
        let bits = value.to_bits();
        let is_negative = bits >> 63 != 0;
        let exponent = ((bits >> 52) & 0x7ff) as u32;
        let mut mantissa = bits & FRACTION_MASK;

        if exponent == 0x7ff {
            if mantissa == 0 {
                self.state = match (self.state, is_negative) {
                    (SumPreciseState::Nan, _)
                    | (SumPreciseState::MinusInfinity, false)
                    | (SumPreciseState::Infinity, true) => SumPreciseState::Nan,
                    (_, false) => SumPreciseState::Infinity,
                    (_, true) => SumPreciseState::MinusInfinity,
                };
            } else {
                self.state = SumPreciseState::Nan;
            }
            return;
        }

        let (position, shift) = if exponent == 0 {
            if mantissa == 0 {
                // n_limbs == 0 is the finite minus-zero sentinel. A positive
                // zero changes that sentinel to the positive-zero form.
                if self.n_limbs == 0 && !is_negative {
                    self.n_limbs = 1;
                }
                return;
            }
            (0_usize, 0_u32)
        } else {
            mantissa |= 1_u64 << 52;
            let bit_position = exponent - 1;
            (
                (bit_position / SP_LIMB_BITS) as usize,
                bit_position % SP_LIMB_BITS,
            )
        };

        let low = (mantissa << shift) & SP_LIMB_MASK;
        let high = mantissa >> (SP_LIMB_BITS - shift);
        if is_negative {
            self.acc[position] = self.acc[position].wrapping_sub(low as i64);
            self.acc[position + 1] = self.acc[position + 1].wrapping_sub(high as i64);
        } else {
            self.acc[position] = self.acc[position].wrapping_add(low as i64);
            self.acc[position + 1] = self.acc[position + 1].wrapping_add(high as i64);
        }
        self.n_limbs = self.n_limbs.max(position + 2);

        self.counter -= 1;
        if self.counter == 0 {
            self.counter = SUM_PRECISE_COUNTER_INIT;
            self.renormalize();
        }
    }

    fn result(mut self) -> f64 {
        match self.state {
            SumPreciseState::Infinity => return f64::INFINITY,
            SumPreciseState::MinusInfinity => return f64::NEG_INFINITY,
            SumPreciseState::Nan => return f64::NAN,
            SumPreciseState::Finite => {}
        }

        self.renormalize();
        let mut limb_count = self.n_limbs;
        if limb_count == 0 {
            return -0.0;
        }
        while limb_count > 0 && self.acc[limb_count - 1] == 0 {
            limb_count -= 1;
        }
        // A finite non-empty cancellation is always positive zero.
        if limb_count == 0 {
            return 0.0;
        }

        let is_negative = self.acc[limb_count - 1] < 0;
        if is_negative {
            let mut carry = 1_u64;
            for limb in &mut self.acc[..limb_count - 1] {
                let value = SP_LIMB_MASK.wrapping_sub(*limb as u64).wrapping_add(carry);
                carry = value >> SP_LIMB_BITS;
                *limb = (value & SP_LIMB_MASK) as i64;
            }
            let top = &mut self.acc[limb_count - 1];
            *top = top
                .wrapping_neg()
                .wrapping_add(carry as i64)
                .wrapping_sub(1);
            while limb_count > 1 && self.acc[limb_count - 1] == 0 {
                limb_count -= 1;
            }
        }

        if limb_count == 1 && self.acc[0] < (1_i64 << 52) {
            return f64::from_bits(
                (u64::from(is_negative) << 63) | u64::try_from(self.acc[0]).unwrap(),
            );
        }

        let mut exponent = (limb_count as i32) * (SP_LIMB_BITS as i32);
        let mut position = limb_count - 1;
        let mut mantissa = self.acc[position] as u64;
        let shift = mantissa.leading_zeros() - (64 - SP_LIMB_BITS);
        exponent -= shift as i32 + 52;

        if shift != 0 {
            mantissa <<= shift;
            if position > 0 {
                position -= 1;
                let lower_shift = SP_LIMB_BITS - shift;
                let discarded = self.acc[position] as u64 & ((1_u64 << lower_shift) - 1);
                mantissa |= (self.acc[position] as u64) >> lower_shift;
                mantissa |= u64::from(discarded != 0);
            }
        }

        let rounding_mask = (1_u64 << SP_RND_BITS) - 1;
        if mantissa & rounding_mask == 1_u64 << (SP_RND_BITS - 1) {
            // Add a sticky bit when any still-lower limb is non-zero.
            while position > 0 {
                position -= 1;
                if self.acc[position] != 0 {
                    mantissa |= 1;
                    break;
                }
            }
        }

        let addend = (1_u64 << (SP_RND_BITS - 1)) - 1 + ((mantissa >> SP_RND_BITS) & 1);
        mantissa = mantissa.wrapping_add(addend) >> SP_RND_BITS;
        if mantissa == 1_u64 << 53 {
            exponent += 1;
        }
        if exponent >= 0x7ff {
            return f64::from_bits((u64::from(is_negative) << 63) | (0x7ff_u64 << 52));
        }

        mantissa &= FRACTION_MASK;
        f64::from_bits((u64::from(is_negative) << 63) | ((exponent as u64) << 52) | mantissa)
    }
}

fn quickjs_min(left: f64, right: f64) -> f64 {
    if left == 0.0 && right == 0.0 {
        f64::from_bits(left.to_bits() | right.to_bits())
    } else {
        left.min(right)
    }
}

fn quickjs_max(left: f64, right: f64) -> f64 {
    if left == 0.0 && right == 0.0 {
        f64::from_bits(left.to_bits() & right.to_bits())
    } else {
        left.max(right)
    }
}

fn quickjs_round(value: f64) -> f64 {
    let mut bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as u32;
    if exponent < 1023 {
        if exponent == 1022 && bits != 0xbfe0_0000_0000_0000 {
            bits = (bits & SIGN_BIT) | (1023_u64 << 52);
        } else {
            bits &= SIGN_BIT;
        }
    } else if exponent < 1023 + 52 {
        let sign = bits >> 63;
        let one = 1_u64 << (52 - (exponent - 1023));
        let fraction_mask = one - 1;
        bits = bits.wrapping_add((one >> 1).wrapping_sub(sign));
        bits &= !fraction_mask;
    }
    f64::from_bits(bits)
}

fn quickjs_sign(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        value
    } else if value < 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn float16_to_f64(value: u16) -> f64 {
    let mut magnitude = u32::from(value & 0x7fff);
    if magnitude >= 0x7c00 {
        magnitude += 0x1f_8000;
    }
    let expanded = (u64::from(value >> 15) << 63) | (u64::from(magnitude) << (52 - 10));
    // Exact binary value 2^1008, used by QuickJS to normalize both ordinary
    // and subnormal binary16 encodings through one multiplication.
    f64::from_bits(expanded) * f64::from_bits(0x7ef0_0000_0000_0000)
}

fn f64_to_float16(value: f64) -> u16 {
    let bits = value.to_bits();
    let sign = (bits >> 63) as u16;
    let mut magnitude = bits & !SIGN_BIT;
    let encoded;

    if magnitude > 0x7ff0_0000_0000_0000 {
        encoded = 0x7c01_u64;
    } else if magnitude < 0x3f10_0000_0000_0000 {
        if magnitude <= 0x3e60_0000_0000_0000 {
            encoded = 0;
        } else {
            let shift = 1051_u32 - ((magnitude >> 52) as u32);
            magnitude = (1_u64 << 52) | (magnitude & FRACTION_MASK);
            let addend = ((magnitude >> shift) & 1) + ((1_u64 << (shift - 1)) - 1);
            encoded = magnitude.wrapping_add(addend) >> shift;
        }
    } else {
        magnitude -= 0x3f00_0000_0000_0000;
        let addend = ((magnitude >> (52 - 10)) & 1) + ((1_u64 << (52 - 11)) - 1);
        let rounded = magnitude.wrapping_add(addend) >> (52 - 10);
        encoded = rounded.min(0x7c00);
    }
    (encoded as u16) | (sign << 15)
}

fn quickjs_f16round(value: f64) -> f64 {
    float16_to_f64(f64_to_float16(value))
}

fn quickjs_unary(selector: MathUnaryKind, value: f64) -> f64 {
    match selector {
        MathUnaryKind::Abs => value.abs(),
        MathUnaryKind::Floor => value.floor(),
        MathUnaryKind::Ceil => value.ceil(),
        MathUnaryKind::Round => quickjs_round(value),
        MathUnaryKind::Sqrt => value.sqrt(),
        MathUnaryKind::Acos => value.acos(),
        MathUnaryKind::Asin => value.asin(),
        MathUnaryKind::Atan => value.atan(),
        MathUnaryKind::Cos => value.cos(),
        MathUnaryKind::Exp => value.exp(),
        MathUnaryKind::Log => value.ln(),
        MathUnaryKind::Sin => value.sin(),
        MathUnaryKind::Tan => value.tan(),
        MathUnaryKind::Trunc => value.trunc(),
        MathUnaryKind::Sign => quickjs_sign(value),
        MathUnaryKind::Cosh => value.cosh(),
        MathUnaryKind::Sinh => value.sinh(),
        MathUnaryKind::Tanh => value.tanh(),
        MathUnaryKind::Acosh => value.acosh(),
        MathUnaryKind::Asinh => value.asinh(),
        MathUnaryKind::Atanh => value.atanh(),
        MathUnaryKind::Expm1 => value.exp_m1(),
        MathUnaryKind::Log1p => value.ln_1p(),
        MathUnaryKind::Log2 => value.log2(),
        MathUnaryKind::Log10 => value.log10(),
        MathUnaryKind::Cbrt => value.cbrt(),
        MathUnaryKind::F16Round => quickjs_f16round(value),
        MathUnaryKind::FRound => f64::from(value as f32),
    }
}

fn quickjs_binary(selector: MathBinaryKind, left: f64, right: f64) -> f64 {
    match selector {
        MathBinaryKind::Atan2 => left.atan2(right),
        MathBinaryKind::Pow => crate::number::pow(left, right),
    }
}

fn quickjs_random_fraction(random: u64) -> f64 {
    let bits = (0x3ff_u64 << 52) | (random >> 12);
    f64::from_bits(bits) - 1.0
}

impl Runtime {
    /// Seed the realm-local random stream and install the global `Math`
    /// `JS_OBJECT_DEF` equivalent. The object itself remains lazy until the
    /// first operation which materializes the AutoInit property.
    pub(in crate::runtime) fn initialize_math_intrinsic(
        &self,
        realm: ContextId,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let seed = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_micros() as u64,
            Err(error) => 0_u64.wrapping_sub(error.duration().as_micros() as u64),
        };
        self.0
            .state
            .borrow_mut()
            .heap
            .initialize_math_random_state(realm, seed)?;

        let key = self.intern_property_key("Math")?;
        self.store_property_slot(
            global_object,
            &key,
            PropertyFlags::data(true, false, true),
            PropertySlot::AutoInit(AutoInitProperty::Math { realm }),
        )
    }

    /// Instantiate pinned QuickJS's complete `js_math_funcs` table for the
    /// AutoInit callback. Methods remain lazy native properties while constants
    /// are ordinary immutable data properties. The symbol is inserted before
    /// the constants as upstream does; ordinary own-key ordering still reports
    /// it after every string key.
    pub(in crate::runtime) fn instantiate_math_intrinsic(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let math = self.new_ordinary_object_in_realm(realm)?;
        for (target, name, length) in [
            (NativeFunctionId::MathMinMax(MathMinMaxKind::Min), "min", 2),
            (NativeFunctionId::MathMinMax(MathMinMaxKind::Max), "max", 2),
            (NativeFunctionId::MathUnary(MathUnaryKind::Abs), "abs", 1),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::Floor),
                "floor",
                1,
            ),
            (NativeFunctionId::MathUnary(MathUnaryKind::Ceil), "ceil", 1),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::Round),
                "round",
                1,
            ),
            (NativeFunctionId::MathUnary(MathUnaryKind::Sqrt), "sqrt", 1),
            (NativeFunctionId::MathUnary(MathUnaryKind::Acos), "acos", 1),
            (NativeFunctionId::MathUnary(MathUnaryKind::Asin), "asin", 1),
            (NativeFunctionId::MathUnary(MathUnaryKind::Atan), "atan", 1),
            (
                NativeFunctionId::MathBinary(MathBinaryKind::Atan2),
                "atan2",
                2,
            ),
            (NativeFunctionId::MathUnary(MathUnaryKind::Cos), "cos", 1),
            (NativeFunctionId::MathUnary(MathUnaryKind::Exp), "exp", 1),
            (NativeFunctionId::MathUnary(MathUnaryKind::Log), "log", 1),
            (NativeFunctionId::MathBinary(MathBinaryKind::Pow), "pow", 2),
            (NativeFunctionId::MathUnary(MathUnaryKind::Sin), "sin", 1),
            (NativeFunctionId::MathUnary(MathUnaryKind::Tan), "tan", 1),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::Trunc),
                "trunc",
                1,
            ),
            (NativeFunctionId::MathUnary(MathUnaryKind::Sign), "sign", 1),
            (NativeFunctionId::MathUnary(MathUnaryKind::Cosh), "cosh", 1),
            (NativeFunctionId::MathUnary(MathUnaryKind::Sinh), "sinh", 1),
            (NativeFunctionId::MathUnary(MathUnaryKind::Tanh), "tanh", 1),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::Acosh),
                "acosh",
                1,
            ),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::Asinh),
                "asinh",
                1,
            ),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::Atanh),
                "atanh",
                1,
            ),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::Expm1),
                "expm1",
                1,
            ),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::Log1p),
                "log1p",
                1,
            ),
            (NativeFunctionId::MathUnary(MathUnaryKind::Log2), "log2", 1),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::Log10),
                "log10",
                1,
            ),
            (NativeFunctionId::MathUnary(MathUnaryKind::Cbrt), "cbrt", 1),
            (NativeFunctionId::MathHypot, "hypot", 2),
            (NativeFunctionId::MathRandom, "random", 0),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::F16Round),
                "f16round",
                1,
            ),
            (
                NativeFunctionId::MathUnary(MathUnaryKind::FRound),
                "fround",
                1,
            ),
            (NativeFunctionId::MathImul, "imul", 2),
            (NativeFunctionId::MathClz32, "clz32", 1),
            (NativeFunctionId::MathSumPrecise, "sumPrecise", 1),
        ] {
            self.define_native_builtin_auto_init(&math, realm, target, name, length, length)?;
        }

        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            &math,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("Math"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Math toStringTag definition was rejected",
            ));
        }

        for (name, value) in [
            ("E", std::f64::consts::E),
            ("LN10", std::f64::consts::LN_10),
            ("LN2", std::f64::consts::LN_2),
            ("LOG2E", std::f64::consts::LOG2_E),
            ("LOG10E", std::f64::consts::LOG10_E),
            ("PI", std::f64::consts::PI),
            ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
            ("SQRT2", std::f64::consts::SQRT_2),
        ] {
            self.define_function_data_property(&math, name, Value::Float(value), false, false)?;
        }
        Ok(math)
    }

    pub(in crate::runtime) fn call_math_min_max(
        &self,
        realm: ContextId,
        selector: MathMinMaxKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Math min/max did not receive a generic-magic invocation",
            ));
        };
        if arguments.actual_arg_count == 0 {
            return Ok(Completion::Return(Value::Float(match selector {
                MathMinMaxKind::Min => f64::INFINITY,
                MathMinMaxKind::Max => f64::NEG_INFINITY,
            })));
        }

        let mut result = match self.native_to_number(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Math min/max first argument was not readable",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        for argument in arguments
            .readable
            .iter()
            .take(arguments.actual_arg_count)
            .skip(1)
        {
            let value = match self.native_to_number(realm, argument)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if !result.is_nan() {
                if value.is_nan() {
                    result = value;
                } else {
                    result = match selector {
                        MathMinMaxKind::Min => quickjs_min(result, value),
                        MathMinMaxKind::Max => quickjs_max(result, value),
                    };
                }
            }
        }
        Ok(Completion::Return(Value::number(result)))
    }

    pub(in crate::runtime) fn call_math_unary(
        &self,
        realm: ContextId,
        selector: MathUnaryKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Math unary function did not receive an f_f invocation",
            ));
        };
        let value = match self.native_to_number(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Math unary argument was not readable",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::number(quickjs_unary(
            selector, value,
        ))))
    }

    pub(in crate::runtime) fn call_math_binary(
        &self,
        realm: ContextId,
        selector: MathBinaryKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Math binary function did not receive an f_f_f invocation",
            ));
        };
        let left = match self.native_to_number(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Math binary first argument was not readable",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let right = match self.native_to_number(
            realm,
            arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "Math binary second argument was not readable",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::number(quickjs_binary(
            selector, left, right,
        ))))
    }

    pub(in crate::runtime) fn call_math_hypot(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Math.hypot did not receive a generic invocation",
            ));
        };
        if arguments.actual_arg_count == 0 {
            return Ok(Completion::Return(Value::Int(0)));
        }
        let mut result = match self.native_to_number(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Math.hypot first argument was not readable",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if arguments.actual_arg_count == 1 {
            result = result.abs();
        } else {
            for argument in arguments
                .readable
                .iter()
                .take(arguments.actual_arg_count)
                .skip(1)
            {
                let value = match self.native_to_number(realm, argument)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                result = result.hypot(value);
            }
        }
        Ok(Completion::Return(Value::number(result)))
    }

    pub(in crate::runtime) fn call_math_random(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Math.random did not receive a generic invocation",
            ));
        };
        let random = self.0.state.borrow_mut().heap.next_math_random_u64(realm)?;
        Ok(Completion::Return(Value::Float(quickjs_random_fraction(
            random,
        ))))
    }

    pub(in crate::runtime) fn call_math_imul(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Math.imul did not receive a generic invocation",
            ));
        };
        let left = match self.native_to_number(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Math.imul first argument was not readable",
            ))?,
        )? {
            NativeConversion::Value(value) => Self::to_uint32_number(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let right = match self.native_to_number(
            realm,
            arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "Math.imul second argument was not readable",
            ))?,
        )? {
            NativeConversion::Value(value) => Self::to_uint32_number(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let product = left.wrapping_mul(right);
        Ok(Completion::Return(Value::Int(i32::from_ne_bytes(
            product.to_ne_bytes(),
        ))))
    }

    pub(in crate::runtime) fn call_math_clz32(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Math.clz32 did not receive a generic invocation",
            ));
        };
        let value = match self.native_to_number(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Math.clz32 argument was not readable",
            ))?,
        )? {
            NativeConversion::Value(value) => Self::to_uint32_number(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Int(value.leading_zeros() as i32)))
    }

    pub(in crate::runtime) fn call_math_sum_precise(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Math.sumPrecise did not receive a generic invocation",
            ));
        };
        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Math.sumPrecise iterable argument was not readable",
            ))?;
        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        let iterator_method = match &iterable {
            Value::Null | Value::Undefined => {
                let base = if matches!(iterable, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    &format!("cannot read property 'Symbol.iterator' of {base}"),
                )?));
            }
            _ => match self.get_value_property_in_realm(realm, iterable.clone(), &iterator_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            },
        };
        let Value::Object(iterator_method) = iterator_method else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "value is not iterable",
            )?));
        };
        let Some(iterator_method) = self.as_callable(&iterator_method)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "value is not iterable",
            )?));
        };
        let iterator = match self.call_internal(realm, &iterator_method, iterable, &[])? {
            Completion::Return(Value::Object(iterator)) => iterator,
            Completion::Return(_) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        // Pinned QuickJS obtains and caches `next` once. Failure here or in a
        // subsequent IteratorNext follows its plain exception exit and does
        // not perform IteratorClose.
        let next_key = self.intern_property_key("next")?;
        let next_method = match self.get_property_in_realm(realm, &iterator, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let mut sum = SumPrecise::new();
        loop {
            let item = match self.object_iterator_next(realm, &iterator, next_method.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Float(sum.result())));
                }
                ObjectIteratorStep::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let number = match item {
                Value::Int(value) => f64::from(value),
                Value::Float(value) => value,
                _ => {
                    let exception =
                        self.new_native_error(realm, NativeErrorKind::Type, "not a number")?;
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(exception));
                }
            };
            sum.add(number);
        }
    }
}
