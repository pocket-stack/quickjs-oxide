//! Numeric operators retain ordered conversion operands across JavaScript callbacks.
use super::{
    NumericValue, add_primitives, bigint_error, compare_bigint_number, mixed_numeric_type_error,
    number_to_int32, number_to_uint32, string_to_bigint, to_numeric_primitive,
    unary_plus_primitive,
};
use crate::engine::{
    api::{Error, ErrorKind},
    code::bytecode::Instruction,
    value::{Value, bigint::JsBigInt},
    vm::{Completion, ToPrimitiveHint},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::vm) enum NumericKind {
    Neg,
    Plus,
    BitNot,
    Inc,
    Dec,
    PostInc,
    PostDec,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Shl,
    Sar,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
}
impl NumericKind {
    pub(in crate::engine::vm) fn for_instruction(instruction: &Instruction) -> Option<Self> {
        Some(match instruction {
            Instruction::Neg => Self::Neg,
            Instruction::Plus => Self::Plus,
            Instruction::BitNot => Self::BitNot,
            Instruction::Inc => Self::Inc,
            Instruction::Dec => Self::Dec,
            Instruction::PostInc => Self::PostInc,
            Instruction::PostDec => Self::PostDec,
            Instruction::Add => Self::Add,
            Instruction::Sub => Self::Sub,
            Instruction::Mul => Self::Mul,
            Instruction::Div => Self::Div,
            Instruction::Mod => Self::Mod,
            Instruction::Pow => Self::Pow,
            Instruction::Shl => Self::Shl,
            Instruction::Sar => Self::Sar,
            Instruction::Shr => Self::Shr,
            Instruction::BitAnd => Self::BitAnd,
            Instruction::BitOr => Self::BitOr,
            Instruction::BitXor => Self::BitXor,
            Instruction::Eq => Self::Eq,
            Instruction::Neq => Self::Neq,
            Instruction::Lt => Self::Lt,
            Instruction::Lte => Self::Lte,
            Instruction::Gt => Self::Gt,
            Instruction::Gte => Self::Gte,
            _ => return None,
        })
    }
    pub(in crate::engine::vm) fn unary(self) -> bool {
        matches!(
            self,
            Self::Neg
                | Self::Plus
                | Self::BitNot
                | Self::Inc
                | Self::Dec
                | Self::PostInc
                | Self::PostDec
        )
    }
    pub(in crate::engine::vm) fn primitive_arithmetic(self) -> bool {
        !self.comparison() && !matches!(self, Self::Eq | Self::Neq)
    }
    fn comparison(self) -> bool {
        matches!(self, Self::Lt | Self::Lte | Self::Gt | Self::Gte)
    }
}
pub(in crate::engine::vm) struct NumericOutput {
    pub value: Value,
    pub previous: Option<Value>,
}
impl NumericOutput {
    fn value(value: Value) -> Self {
        Self {
            value,
            previous: None,
        }
    }
    fn into_step(self) -> NumericStep {
        NumericStep::Complete {
            value: self.value,
            previous: self.previous,
        }
    }
}

/// Primitive arithmetic uses the same conversion and operator kernels as resumes.
/// Parsing, allocation and final primitive-owner release require an ended
/// RunSlots borrow; the resident run helper is also such an owning boundary.
pub(in crate::engine::vm) fn primitive_output(
    kind: NumericKind,
    left: Value,
    right: Option<Value>,
) -> Result<NumericOutput, Error> {
    if kind.unary() {
        return unary_output(kind, left);
    }
    let right = right.ok_or_else(|| Error::internal("binary numeric operator lost RHS"))?;
    if kind == NumericKind::Add {
        return add_primitives(left, right).map(NumericOutput::value);
    }
    let left = to_numeric_primitive(left)?;
    let right = to_numeric_primitive(right)?;
    binary(kind, left, right).map(NumericOutput::value)
}

pub(in crate::engine::vm) enum NumericStep {
    Complete {
        value: Value,
        previous: Option<Value>,
    },
    Throw(Value),
    Primitive {
        value: Value,
        hint: ToPrimitiveHint,
        resume: NumericResume,
    },
    HtmlDda {
        value: Value,
        resume: NumericResume,
    },
}
pub(in crate::engine::vm) struct NumericResume(Box<NumericResumeState>);
impl std::ops::Deref for NumericResume {
    type Target = NumericResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for NumericResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<NumericResume>() <= 8);
pub(in crate::engine::vm) struct NumericResumeState {
    kind: NumericKind,
    phase: Phase,
}
enum Phase {
    Unary,
    Left(Value),
    RightPrimitive(Value),
    RightNumeric(NumericValue),
    EqualityLeft(Value),
    EqualityRight(Value),
    EqualityDda(Value, Value),
}
impl NumericStep {
    pub(in crate::engine::vm) fn start(
        kind: NumericKind,
        left: Value,
        right: Option<Value>,
    ) -> Result<Self, Error> {
        if kind.unary() {
            return primitive(
                left,
                ToPrimitiveHint::Number,
                NumericResume(Box::new(NumericResumeState {
                    kind,
                    phase: Phase::Unary,
                })),
            );
        }
        let right = right.ok_or_else(|| Error::internal("binary numeric operator lost RHS"))?;
        if matches!(kind, NumericKind::Eq | NumericKind::Neq) {
            return equality(kind, left, right, false);
        }
        primitive(
            left,
            if kind == NumericKind::Add {
                ToPrimitiveHint::Default
            } else {
                ToPrimitiveHint::Number
            },
            NumericResume(Box::new(NumericResumeState {
                kind,
                phase: Phase::Left(right),
            })),
        )
    }
}
fn primitive(
    value: Value,
    hint: ToPrimitiveHint,
    resume: NumericResume,
) -> Result<NumericStep, Error> {
    if matches!(value, Value::Object(_)) {
        Ok(NumericStep::Primitive {
            value,
            hint,
            resume,
        })
    } else {
        resume.resume(Completion::Return(value))
    }
}
fn complete(value: Value) -> NumericStep {
    NumericStep::Complete {
        value,
        previous: None,
    }
}
impl NumericResume {
    pub(in crate::engine::vm) fn resume(self, reply: Completion) -> Result<NumericStep, Error> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NumericStep::Throw(value)),
        };
        if matches!(value, Value::Object(_)) {
            return Err(Error::internal(
                "numeric ToPrimitive reply returned an object",
            ));
        }
        let kind = self.0.kind;
        match self.0.phase {
            Phase::Unary => unary(kind, value),
            Phase::Left(right) => {
                // Arithmetic converts the left primitive to Numeric before starting
                // the right callback. Relational comparison converts both primitives first.
                let phase = if kind == NumericKind::Add || kind.comparison() {
                    Phase::RightPrimitive(value)
                } else {
                    Phase::RightNumeric(to_numeric_primitive(value)?)
                };
                primitive(
                    right,
                    if kind == NumericKind::Add {
                        ToPrimitiveHint::Default
                    } else {
                        ToPrimitiveHint::Number
                    },
                    NumericResume(Box::new(NumericResumeState { kind, phase })),
                )
            }
            Phase::RightPrimitive(left) => Ok(complete(if kind == NumericKind::Add {
                add_primitives(left, value)?
            } else {
                Value::Bool(compare(kind, left, value)?)
            })),
            Phase::RightNumeric(left) => {
                Ok(complete(binary(kind, left, to_numeric_primitive(value)?)?))
            }
            Phase::EqualityLeft(right) => equality(kind, value, right, false),
            Phase::EqualityRight(left) => equality(kind, left, value, false),
            Phase::EqualityDda(..) => {
                Err(Error::internal("HTMLDDA check received a primitive reply"))
            }
        }
    }
    pub(in crate::engine::vm) fn html_dda(self, value: bool) -> Result<NumericStep, Error> {
        let Phase::EqualityDda(left, right) = self.0.phase else {
            return Err(Error::internal("HTMLDDA reply lost equality owner"));
        };
        if value {
            Ok(equal_result(self.0.kind, true))
        } else {
            equality(self.0.kind, left, right, true)
        }
    }
}
fn unary(kind: NumericKind, value: Value) -> Result<NumericStep, Error> {
    unary_output(kind, value).map(NumericOutput::into_step)
}
fn unary_output(kind: NumericKind, value: Value) -> Result<NumericOutput, Error> {
    if kind == NumericKind::Plus {
        return Ok(NumericOutput::value(unary_plus_primitive(value)?));
    }
    if kind == NumericKind::Neg {
        return Ok(NumericOutput::value(
            if let Some(number) = value.as_number_repr() {
                number.negate().into()
            } else {
                match value {
                    Value::BigInt(value) => Value::BigInt(value.neg().map_err(bigint_error)?),
                    value => Value::number(-value.to_number()?),
                }
            },
        ));
    }
    if kind == NumericKind::BitNot {
        return Ok(NumericOutput::value(match to_numeric_primitive(value)? {
            NumericValue::BigInt(value) => Value::BigInt(value.bit_not().map_err(bigint_error)?),
            NumericValue::Number(value) => Value::Int(!number_to_int32(value)),
        }));
    }
    let increment = matches!(kind, NumericKind::Inc | NumericKind::PostInc);
    let postfix = matches!(kind, NumericKind::PostInc | NumericKind::PostDec);
    let (old, next) = if let Some(number) = value.as_number_repr() {
        (value, number.update(increment).into())
    } else {
        match value {
            Value::BigInt(old) => {
                let next = if increment {
                    old.add(&JsBigInt::from(1_i32))
                } else {
                    old.update_decrement()
                }
                .map_err(bigint_error)?;
                (Value::BigInt(old), Value::BigInt(next))
            }
            value => {
                let old = value.to_number()?;
                (
                    Value::number(old),
                    Value::number(if increment { old + 1.0 } else { old - 1.0 }),
                )
            }
        }
    };
    Ok(NumericOutput {
        value: next,
        previous: postfix.then_some(old),
    })
}
fn binary(kind: NumericKind, left: NumericValue, right: NumericValue) -> Result<Value, Error> {
    if kind == NumericKind::Shr {
        let (NumericValue::Number(left), NumericValue::Number(right)) = (left, right) else {
            return Err(Error::new(
                ErrorKind::Type,
                "bigint operands are forbidden for >>>",
            ));
        };
        return Ok(Value::number(f64::from(
            number_to_uint32(left) >> (number_to_uint32(right) & 0x1f),
        )));
    }
    Ok(match (left, right) {
        (NumericValue::BigInt(left), NumericValue::BigInt(right)) => Value::BigInt(
            match kind {
                NumericKind::Sub => left.sub(&right),
                NumericKind::Mul => left.mul(&right),
                NumericKind::Div => left.div(&right),
                NumericKind::Mod => left.rem(&right),
                NumericKind::Pow => left.pow(&right),
                NumericKind::Shl => left.shl(&right),
                NumericKind::Sar => left.shr(&right),
                NumericKind::BitAnd => left.bit_and(&right),
                NumericKind::BitOr => left.bit_or(&right),
                NumericKind::BitXor => left.bit_xor(&right),
                _ => {
                    return Err(Error::internal(
                        "non-arithmetic operator entered binary Numeric",
                    ));
                }
            }
            .map_err(bigint_error)?,
        ),
        (NumericValue::Number(left), NumericValue::Number(right)) => Value::number(match kind {
            NumericKind::Sub => left - right,
            NumericKind::Mul => left * right,
            NumericKind::Div => left / right,
            NumericKind::Mod => left % right,
            NumericKind::Pow => crate::engine::value::number::pow(left, right),
            NumericKind::Shl => {
                f64::from(number_to_int32(left).wrapping_shl(number_to_uint32(right) & 0x1f))
            }
            NumericKind::Sar => {
                f64::from(number_to_int32(left) >> (number_to_uint32(right) & 0x1f))
            }
            NumericKind::BitAnd => f64::from(number_to_int32(left) & number_to_int32(right)),
            NumericKind::BitOr => f64::from(number_to_int32(left) | number_to_int32(right)),
            NumericKind::BitXor => f64::from(number_to_int32(left) ^ number_to_int32(right)),
            _ => {
                return Err(Error::internal(
                    "non-arithmetic operator entered binary Numeric",
                ));
            }
        }),
        _ => return Err(mixed_numeric_type_error()),
    })
}
fn compare(kind: NumericKind, left: Value, right: Value) -> Result<bool, Error> {
    let ordering = match (&left, &right) {
        (Value::String(left), Value::String(right)) => {
            Some(left.utf16_units().cmp(right.utf16_units()))
        }
        (Value::BigInt(left), Value::BigInt(right)) => Some(left.cmp(right)),
        (Value::BigInt(left), Value::String(right)) => {
            string_to_bigint(right).map(|right| left.cmp(&right))
        }
        (Value::String(left), Value::BigInt(right)) => {
            string_to_bigint(left).map(|left| left.cmp(right))
        }
        _ => match (to_numeric_primitive(left)?, to_numeric_primitive(right)?) {
            (NumericValue::BigInt(left), NumericValue::BigInt(right)) => Some(left.cmp(&right)),
            (NumericValue::BigInt(left), NumericValue::Number(right)) => {
                compare_bigint_number(&left, right)
            }
            (NumericValue::Number(left), NumericValue::BigInt(right)) => {
                compare_bigint_number(&right, left).map(std::cmp::Ordering::reverse)
            }
            (NumericValue::Number(left), NumericValue::Number(right)) => left.partial_cmp(&right),
        },
    };
    Ok(ordering.is_some_and(|ordering| match kind {
        NumericKind::Lt => ordering.is_lt(),
        NumericKind::Lte => ordering.is_le(),
        NumericKind::Gt => ordering.is_gt(),
        NumericKind::Gte => ordering.is_ge(),
        _ => false,
    }))
}
fn equal_result(kind: NumericKind, equal: bool) -> NumericStep {
    complete(Value::Bool(equal != (kind == NumericKind::Neq)))
}
fn equality(
    kind: NumericKind,
    mut left: Value,
    mut right: Value,
    mut checked_dda: bool,
) -> Result<NumericStep, Error> {
    loop {
        if left.strict_equal(&right) {
            return Ok(equal_result(kind, true));
        }
        if !checked_dda {
            if matches!(right, Value::Null | Value::Undefined) {
                return Ok(NumericStep::HtmlDda {
                    value: left.clone(),
                    resume: NumericResume(Box::new(NumericResumeState {
                        kind,
                        phase: Phase::EqualityDda(left, right),
                    })),
                });
            }
            if matches!(left, Value::Null | Value::Undefined) {
                return Ok(NumericStep::HtmlDda {
                    value: right.clone(),
                    resume: NumericResume(Box::new(NumericResumeState {
                        kind,
                        phase: Phase::EqualityDda(left, right),
                    })),
                });
            }
        }
        checked_dda = false;
        match (&left, &right) {
            (Value::Null, Value::Undefined) | (Value::Undefined, Value::Null) => {
                return Ok(equal_result(kind, true));
            }
            (Value::Int(_) | Value::Float(_), Value::String(_)) => {
                right = Value::number(right.to_number()?)
            }
            (Value::String(_), Value::Int(_) | Value::Float(_)) => {
                left = Value::number(left.to_number()?)
            }
            (Value::BigInt(a), Value::String(b)) => {
                return Ok(equal_result(
                    kind,
                    string_to_bigint(b).is_some_and(|b| &b == a),
                ));
            }
            (Value::String(a), Value::BigInt(b)) => {
                return Ok(equal_result(
                    kind,
                    string_to_bigint(a).is_some_and(|a| &a == b),
                ));
            }
            (Value::BigInt(a), Value::Int(_) | Value::Float(_)) => {
                return Ok(equal_result(
                    kind,
                    compare_bigint_number(a, right.to_number()?) == Some(std::cmp::Ordering::Equal),
                ));
            }
            (Value::Int(_) | Value::Float(_), Value::BigInt(b)) => {
                return Ok(equal_result(
                    kind,
                    compare_bigint_number(b, left.to_number()?) == Some(std::cmp::Ordering::Equal),
                ));
            }
            (Value::Bool(_), _) => left = Value::number(left.to_number()?),
            (_, Value::Bool(_)) => right = Value::number(right.to_number()?),
            (
                Value::Object(_),
                Value::Int(_)
                | Value::Float(_)
                | Value::BigInt(_)
                | Value::String(_)
                | Value::Symbol(_),
            ) => {
                return primitive(
                    left,
                    ToPrimitiveHint::Default,
                    NumericResume(Box::new(NumericResumeState {
                        kind,
                        phase: Phase::EqualityLeft(right),
                    })),
                );
            }
            (
                Value::Int(_)
                | Value::Float(_)
                | Value::BigInt(_)
                | Value::String(_)
                | Value::Symbol(_),
                Value::Object(_),
            ) => {
                return primitive(
                    right,
                    ToPrimitiveHint::Default,
                    NumericResume(Box::new(NumericResumeState {
                        kind,
                        phase: Phase::EqualityRight(left),
                    })),
                );
            }
            _ => return Ok(equal_result(kind, false)),
        }
    }
}

#[cfg(all(test, feature = "stack-vm", feature = "profiling"))]
mod tests {
    use crate::engine::{
        api::{profiling::CostProfile, runtime::Runtime},
        value::Value,
        vm::Completion,
    };

    #[test]
    fn numeric_callbacks_preserve_order_abrupt_completion_and_postfix_values_without_bridges() {
        for source in [
            "(function(){var log='';var a={[Symbol.toPrimitive](h){log+='L'+h;return 6}},b={[Symbol.toPrimitive](h){log+='R'+h;return 2}};return function(){var result=a-b;return result===4&&log==='LnumberRnumber'?42:0}})()",
            "(function(){var log='';var symbol=Symbol(),a={valueOf(){log+='L';return symbol}},b={valueOf(){log+='R';return 1}};return function(){try{a*b}catch(e){return log==='L'?42:0}return 0}})()",
            "(function(){var log='';var symbol=Symbol(),a={valueOf(){log+='L';return symbol}},b={valueOf(){log+='R';return 1}};return function(){try{a<b}catch(e){return log==='LR'?42:0}return 0}})()",
            "(function(){var log='';var a={valueOf(){log+='L';return 1n}},b={valueOf(){log+='R';return 1}};return function(){try{a/b}catch(e){return log==='LR'?42:0}return 0}})()",
            "(function(){var n=0,a={[Symbol.toPrimitive](h){if(h!=='number')throw 99;n++;return '6'}};return function(){var v=a,old=v++;return old===6&&v===7&&n===1?42:0}})()",
            "(function(){var n=0,a={[Symbol.toPrimitive](h){if(h!=='number')throw 99;n++;return 6n}};return function(){var v=a,old=v--;return old===6n&&v===5n&&n===1?42:0}})()",
            "(function(){var n=0,a={[Symbol.toPrimitive](h){if(h!=='default')throw 99;n++;return '42'}};return function(){return a==42&&n===1?42:0}})()",
            "(function(){var a={valueOf(){return 40n}},b={valueOf(){return 2n}};return function(){return a|b}})()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callable = runtime
                .callable_from_value(context.eval(source).unwrap())
                .unwrap();
            let profile = CostProfile::start();
            let completion = runtime
                .call_internal(context.realm, &callable, Value::Undefined, &[])
                .unwrap();
            let costs = profile.snapshot();
            assert!(
                matches!(completion, Completion::Return(Value::Int(42)))
                    || matches!(&completion,Completion::Return(Value::BigInt(value)) if value == &crate::engine::value::bigint::JsBigInt::from(42_i32)),
                "{source}: {completion:?}"
            );
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_sync_call_bridges, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<NumericStep>() <= 64);
