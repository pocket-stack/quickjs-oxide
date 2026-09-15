//! Branded scalar formatting and BigInt widths preserve argument conversion order.
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::{BigIntAsNKind, NumberFormatKind, PrimitiveKind},
    heap::ContextId,
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum NumericKind {
    ToString(PrimitiveKind),
    Format(NumberFormatKind),
    BigIntAsN(BigIntAsNKind),
}
impl NumericKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        match target {
            NativeFunctionId::PrimitivePrototypeToString(kind) => Some(Self::ToString(kind)),
            NativeFunctionId::NumberPrototypeFormat(kind) => Some(Self::Format(kind)),
            NativeFunctionId::BigIntAsN(kind) => Some(Self::BigIntAsN(kind)),
            _ => None,
        }
    }
}
pub(crate) enum NumericStep {
    Complete(Completion),
    Number { value: Value, resume: NumericResume },
    Primitive { value: Value, resume: NumericResume },
}
enum Phase {
    Radix,
    Digits,
    Width,
    BigInt,
}
pub(crate) struct NumericResume(Box<NumericResumeState>);
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
pub(crate) struct NumericResumeState {
    realm: ContextId,
    kind: NumericKind,
    value: Value,
    argument: Value,
    phase: Phase,
    bits: u64,
}
impl NumericStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: NumericKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "scalar numeric method requires generic invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .unwrap_or(Value::Undefined);
        let value = match kind {
            NumericKind::BigIntAsN(_) => arguments
                .readable
                .get(1)
                .cloned()
                .ok_or(RuntimeError::Invariant("BigInt width argv was not padded"))?,
            _ => {
                let brand = match kind {
                    NumericKind::ToString(kind) => kind,
                    _ => PrimitiveKind::Number,
                };
                match runtime.primitive_this_value(realm, brand, this_value.clone())? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(Self::Complete(Completion::Throw(value)));
                    }
                }
            }
        };
        if let NumericKind::ToString(brand) = kind {
            if !matches!(brand, PrimitiveKind::Number | PrimitiveKind::BigInt)
                || matches!(argument, Value::Undefined)
            {
                return Ok(Self::Complete(
                    runtime.finish_branded_to_string(realm, brand, value, 10)?,
                ));
            }
        }
        let phase = match kind {
            NumericKind::ToString(_) => Phase::Radix,
            NumericKind::Format(_) => Phase::Digits,
            NumericKind::BigIntAsN(_) => Phase::Width,
        };
        let resume = NumericResume(Box::new(NumericResumeState {
            realm,
            kind,
            value,
            argument: argument.clone(),
            phase,
            bits: 0,
        }));
        match kind {
            NumericKind::Format(NumberFormatKind::LocaleString) => resume.format(runtime, 0),
            NumericKind::Format(NumberFormatKind::Precision)
                if matches!(argument, Value::Undefined) =>
            {
                resume.format(runtime, 0)
            }
            _ => Ok(Self::Number {
                value: argument,
                resume,
            }),
        }
    }
}
impl NumericResume {
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<NumericStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(NumericStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Radix => {
                let radix = crate::engine::value::number::to_int32_sat(value);
                if !(2..=36).contains(&radix) {
                    return Ok(NumericStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Range,
                            "radix must be between 2 and 36",
                        )?,
                    )));
                }
                let NumericKind::ToString(kind) = self.0.kind else {
                    return Err(RuntimeError::Invariant("scalar radix kind mismatch"));
                };
                Ok(NumericStep::Complete(runtime.finish_branded_to_string(
                    self.0.realm,
                    kind,
                    self.0.value,
                    radix as u32,
                )?))
            }
            Phase::Digits => {
                self.format(runtime, crate::engine::value::number::to_int32_sat(value))
            }
            Phase::Width => {
                self.0.bits = match runtime.index_from_number(self.0.realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NumericStep::Complete(Completion::Throw(value)));
                    }
                };
                self.0.phase = Phase::BigInt;
                let value = std::mem::replace(&mut self.0.value, Value::Undefined);
                Ok(NumericStep::Primitive {
                    value,
                    resume: self,
                })
            }
            _ => Err(RuntimeError::Invariant(
                "scalar numeric reply phase mismatch",
            )),
        }
    }
    fn format(self, runtime: &Runtime, digits: i32) -> Result<NumericStep, RuntimeError> {
        let NumericKind::Format(kind) = self.0.kind else {
            return Err(RuntimeError::Invariant("scalar formatter kind mismatch"));
        };
        let number = self.0.value.as_number().ok_or(RuntimeError::Invariant(
            "Number formatter brand returned non-number",
        ))?;
        let result = match kind {
            NumberFormatKind::LocaleString => {
                crate::engine::value::number::to_string_radix(number, 10)
            }
            NumberFormatKind::Fixed => crate::engine::value::number::to_fixed(number, digits),
            NumberFormatKind::Exponential => crate::engine::value::number::to_exponential(
                number,
                (!matches!(self.0.argument, Value::Undefined)).then_some(digits),
            ),
            NumberFormatKind::Precision => crate::engine::value::number::to_precision(
                number,
                (!matches!(self.0.argument, Value::Undefined)).then_some(digits),
            ),
        };
        Ok(NumericStep::Complete(
            runtime.finish_number_format(self.0.realm, result)?,
        ))
    }
    pub(crate) fn primitive(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<NumericStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::BigInt) {
            return Err(RuntimeError::Invariant(
                "BigInt width primitive phase mismatch",
            ));
        }
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NumericStep::Complete(Completion::Throw(value))),
        };
        let value = match runtime.bigint_from_primitive(self.0.realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(NumericStep::Complete(Completion::Throw(value)));
            }
        };
        let NumericKind::BigIntAsN(kind) = self.0.kind else {
            return Err(RuntimeError::Invariant("BigInt width kind mismatch"));
        };
        let result = match kind {
            BigIntAsNKind::AsUintN => value.as_uint_n(self.0.bits),
            BigIntAsNKind::AsIntN => value.as_int_n(self.0.bits),
        };
        Ok(NumericStep::Complete(match result {
            Ok(value) => Completion::Return(Value::BigInt(value)),
            Err(_) => Completion::Throw(runtime.new_native_error(
                self.0.realm,
                NativeErrorKind::Range,
                "BigInt is too large to allocate",
            )?),
        }))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: NumericStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            NumericStep::Complete(result) => return Ok(result),
            NumericStep::Number { value, resume } => {
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            NumericStep::Primitive { value, resume } => resume.primitive(
                runtime,
                runtime.to_primitive(realm, value, crate::engine::vm::ToPrimitiveHint::Number)?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<NumericStep>() <= 64);
