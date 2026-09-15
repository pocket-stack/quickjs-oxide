//! Math argument conversion advances in source order while numerical kernels stay pure.
//!
//! Owned primitive calls borrow NativeActivation argv and allocate no Math argv.
//! At the first object, only the remaining suffix becomes continuation-owned;
//! NativeActivation retains the original arguments across callbacks. Profiling
//! distinguishes these two starts, without claiming native argument storage or
//! every primitive conversion is allocation-free.
use super::{quickjs_binary, quickjs_max, quickjs_min, quickjs_unary};
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::{MathBinaryKind, MathMinMaxKind, MathUnaryKind},
    heap::ContextId,
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum MathKind {
    Unary(MathUnaryKind),
    Binary(MathBinaryKind),
    MinMax(MathMinMaxKind),
    Hypot,
    Imul,
    Clz32,
}
impl MathKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::MathUnary(kind) => Self::Unary(kind),
            NativeFunctionId::MathBinary(kind) => Self::Binary(kind),
            NativeFunctionId::MathMinMax(kind) => Self::MinMax(kind),
            NativeFunctionId::MathHypot => Self::Hypot,
            NativeFunctionId::MathImul => Self::Imul,
            NativeFunctionId::MathClz32 => Self::Clz32,
            _ => return None,
        })
    }
}
pub(crate) enum MathStep {
    Complete(Completion),
    Number { value: Value, resume: MathResume },
}
pub(crate) struct MathResume(Box<MathResumeState>);
impl std::ops::Deref for MathResume {
    type Target = MathResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for MathResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<MathResume>() <= 8);
pub(crate) struct MathResumeState {
    kind: MathKind,
    arguments: std::vec::IntoIter<Value>,
    result: Option<f64>,
    count: usize,
}
impl MathStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: MathKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        if !matches!(invocation, NativeInvocation::Call { .. }) {
            return Err(RuntimeError::Invariant("Math requires generic invocation"));
        }
        let count = match kind {
            MathKind::Unary(_) | MathKind::Clz32 => 1,
            MathKind::Binary(_) | MathKind::Imul => 2,
            _ => arguments.actual_arg_count,
        };
        let values = arguments
            .readable
            .get(..count)
            .ok_or(RuntimeError::Invariant("Math argv was not padded"))?;
        #[cfg(feature = "stack-vm")]
        {
            let mut resume = MathResume(Box::new(MathResumeState {
                kind,
                arguments: Vec::new().into_iter(),
                result: None,
                count,
            }));
            for (index, value) in values.iter().enumerate() {
                if matches!(value, Value::Object(_)) {
                    // The native activation owns original argv. A suspended
                    // continuation needs only the not-yet-converted suffix.
                    resume.arguments = values[index..].to_vec().into_iter();
                    #[cfg(feature = "profiling")]
                    crate::engine::api::profiling::record_owned_execution_event(
                        "math_remaining_arguments_owned",
                    );
                    return resume.next();
                }
                // Object arguments above retain the shared waiting protocol.
                // NativeActivation already owns this primitive: borrow it in
                // the same conversion kernel used by NumberStep completion.
                let result = runtime.number_from_primitive(realm, value)?;
                if let Some(completion) = resume.accept_number(result)? {
                    #[cfg(feature = "profiling")]
                    crate::engine::api::profiling::record_owned_execution_event(
                        "math_completed_without_argument_storage",
                    );
                    return Ok(Self::Complete(completion));
                }
            }
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "math_completed_without_argument_storage",
            );
            resume.next()
        }
        #[cfg(not(feature = "stack-vm"))]
        {
            let _ = (runtime, realm);
            MathResume(Box::new(MathResumeState {
                kind,
                arguments: values.to_vec().into_iter(),
                result: None,
                count,
            }))
            .next()
        }
    }
}
impl MathResume {
    fn next(mut self) -> Result<MathStep, RuntimeError> {
        if let Some(value) = self.0.arguments.next() {
            return Ok(MathStep::Number {
                value,
                resume: self,
            });
        }
        let value = match self.0.kind {
            MathKind::MinMax(kind) if self.0.result.is_none() => {
                return Ok(MathStep::Complete(Completion::Return(Value::Float(
                    match kind {
                        MathMinMaxKind::Min => f64::INFINITY,
                        MathMinMaxKind::Max => f64::NEG_INFINITY,
                    },
                ))));
            }
            MathKind::Hypot if self.0.count == 0 => {
                return Ok(MathStep::Complete(Completion::Return(Value::Int(0))));
            }
            MathKind::Hypot if self.0.count == 1 => self
                .0
                .result
                .ok_or(RuntimeError::Invariant("Math hypot result missing"))?
                .abs(),
            _ => self
                .0
                .result
                .ok_or(RuntimeError::Invariant("Math result missing"))?,
        };
        Ok(MathStep::Complete(Completion::Return(Value::number(value))))
    }
    pub(crate) fn number(
        mut self,
        result: NativeConversion<f64>,
    ) -> Result<MathStep, RuntimeError> {
        if let Some(completion) = self.accept_number(result)? {
            Ok(MathStep::Complete(completion))
        } else {
            self.next()
        }
    }

    /// One numerical accumulation kernel for immediate and suspended inputs.
    fn accept_number(
        &mut self,
        result: NativeConversion<f64>,
    ) -> Result<Option<Completion>, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(Some(Completion::Throw(value)));
            }
        };
        self.0.result = Some(match self.0.kind {
            MathKind::Unary(kind) => quickjs_unary(kind, value),
            MathKind::Clz32 => {
                return Ok(Some(Completion::Return(Value::Int(
                    Runtime::to_uint32_number(value).leading_zeros() as i32,
                ))));
            }
            MathKind::Binary(kind) => {
                if let Some(left) = self.0.result {
                    quickjs_binary(kind, left, value)
                } else {
                    value
                }
            }
            MathKind::Imul => {
                if let Some(left) = self.0.result {
                    let product = Runtime::to_uint32_number(left)
                        .wrapping_mul(Runtime::to_uint32_number(value));
                    return Ok(Some(Completion::Return(Value::Int(i32::from_ne_bytes(
                        product.to_ne_bytes(),
                    )))));
                } else {
                    value
                }
            }
            MathKind::MinMax(kind) => {
                if let Some(left) = self.0.result {
                    if left.is_nan() {
                        left
                    } else if value.is_nan() {
                        value
                    } else {
                        match kind {
                            MathMinMaxKind::Min => quickjs_min(left, value),
                            MathMinMaxKind::Max => quickjs_max(left, value),
                        }
                    }
                } else {
                    value
                }
            }
            MathKind::Hypot => {
                if let Some(left) = self.0.result {
                    left.hypot(value)
                } else {
                    value
                }
            }
        });
        Ok(None)
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: MathStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            MathStep::Complete(result) => return Ok(result),
            MathStep::Number { value, resume } => {
                resume.number(runtime.native_to_number(realm, &value)?)?
            }
        };
    }
}

#[cfg(all(test, feature = "stack-vm", feature = "profiling"))]
mod tests;

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<MathStep>() <= 64);
