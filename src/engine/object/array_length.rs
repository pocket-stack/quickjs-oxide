//! Pinned Array length conversion: non-numbers undergo two observable ToNumbers.
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::ContextId;
use crate::engine::object::operations::ArrayLengthConversion;
use crate::engine::value::{Value, conversion::NativeConversion};

pub(crate) enum ArrayLengthStep {
    Complete(ArrayLengthConversion),
    Number {
        value: Value,
        resume: ArrayLengthResume,
    },
}
pub(crate) struct ArrayLengthResume(Box<ArrayLengthResumeState>);
impl std::ops::Deref for ArrayLengthResume {
    type Target = ArrayLengthResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ArrayLengthResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ArrayLengthResume>() <= 8);
pub(crate) struct ArrayLengthResumeState {
    realm: Option<ContextId>,
    phase: Phase,
}
enum Phase {
    First(Value),
    Second { _original: Value, uint32: u32 },
}
impl ArrayLengthStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: Option<ContextId>,
        value: Value,
    ) -> Result<Self, RuntimeError> {
        Ok(match value {
            Value::Int(value) if value >= 0 => {
                Self::Complete(ArrayLengthConversion::Length(value as u32))
            }
            Value::Bool(value) => Self::Complete(ArrayLengthConversion::Length(u32::from(value))),
            Value::Null => Self::Complete(ArrayLengthConversion::Length(0)),
            Value::Float(value) => {
                Self::Complete(runtime.validate_array_length_number(realm, value, None)?)
            }
            Value::Int(_) => Self::Complete(runtime.invalid_array_length(realm)?),
            value => Self::Number {
                value: value.clone(),
                resume: ArrayLengthResume(Box::new(ArrayLengthResumeState {
                    realm,
                    phase: Phase::First(value),
                })),
            },
        })
    }
}
impl ArrayLengthResume {
    pub(crate) fn number(
        self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<ArrayLengthStep, RuntimeError> {
        let number = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ArrayLengthStep::Complete(ArrayLengthConversion::Throw(
                    value,
                )));
            }
        };
        Ok(match self.0.phase {
            Phase::First(original) => ArrayLengthStep::Number {
                value: original.clone(),
                resume: Self(Box::new(ArrayLengthResumeState {
                    realm: self.0.realm,
                    phase: Phase::Second {
                        _original: original,
                        uint32: Runtime::to_uint32_number(number),
                    },
                })),
            },
            Phase::Second { _original, uint32 } => ArrayLengthStep::Complete(
                runtime.validate_array_length_number(self.0.realm, number, Some(uint32))?,
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take_number(step: ArrayLengthStep) -> ArrayLengthResume {
        let ArrayLengthStep::Number { resume, .. } = step else {
            panic!("expected ToNumber request")
        };
        resume
    }

    #[test]
    fn both_array_length_requests_root_original_until_reply_or_abandonment() {
        for second in [false, true] {
            let runtime = Runtime::new();
            let weak = std::rc::Rc::downgrade(&runtime.0);
            let context = runtime.new_context();
            let original = runtime.new_object(None).unwrap();
            let id = original.object_id();
            let mut resume = take_number(
                ArrayLengthStep::start(&runtime, Some(context.realm), Value::Object(original))
                    .unwrap(),
            );
            if second {
                resume = take_number(
                    resume
                        .number(&runtime, NativeConversion::Value(1.0))
                        .unwrap(),
                );
            }
            runtime.run_gc().unwrap();
            assert!(runtime.0.state.borrow().heap.object(id).is_ok());
            drop(resume);
            runtime.run_gc().unwrap();
            assert!(runtime.0.state.borrow().heap.object(id).is_err());
            drop(context);
            drop(runtime);
            assert!(weak.upgrade().is_none());
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ArrayLengthStep>() <= 64);
