//! Deliver initial domain completion before constructing the generic waiting enum.
use super::NativeStep;
use crate::engine::vm::call::NativeInvokeOutcome;

pub(super) trait InitialOutput: Sized {
    fn split(self) -> Result<NativeInvokeOutcome, Self>;
    fn waiting(self) -> NativeStep;
}

#[inline]
pub(super) fn deliver<T: InitialOutput>(
    step: T,
    waiting: &mut impl FnMut(NativeStep),
) -> Option<NativeInvokeOutcome> {
    match step.split() {
        Ok(result) => {
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "native_domain_completed_without_waiting_payload",
            );
            Some(result)
        }
        Err(step) => {
            waiting(step.waiting());
            None
        }
    }
}

macro_rules! completion_domain {
    ($step:ty, $variant:ident) => {
        impl InitialOutput for $step {
            #[inline]
            fn split(self) -> Result<NativeInvokeOutcome, Self> {
                match self {
                    Self::Complete(result) => Ok(NativeInvokeOutcome::Completion(result)),
                    step => Err(step),
                }
            }
            #[inline]
            fn waiting(self) -> NativeStep {
                NativeStep::$variant(self)
            }
        }
    };
}
completion_domain!(crate::engine::builtins::MathStep, Math);
completion_domain!(crate::engine::builtins::GlobalStep, Global);
completion_domain!(crate::engine::builtins::ArrayMutationStep, ArrayMutation);
completion_domain!(
    crate::engine::builtins::PrimitiveConstructorStep,
    PrimitiveConstructor
);
completion_domain!(crate::engine::builtins::RegExpExecStep, RegExpExec);
completion_domain!(crate::engine::builtins::RegExpReplaceStep, RegExpReplace);
completion_domain!(crate::engine::builtins::StringReplaceStep, StringReplace);

impl InitialOutput for crate::engine::builtins::ArrayNextStep {
    #[inline]
    fn split(self) -> Result<NativeInvokeOutcome, Self> {
        match self {
            Self::Complete(result) => Ok(result),
            step => Err(step),
        }
    }
    #[inline]
    fn waiting(self) -> NativeStep {
        NativeStep::ArrayNext(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::api::{Runtime, Value};
    use crate::engine::vm::{Completion, call::NativeInvocation};

    #[test]
    fn domain_completion_does_not_construct_a_waiting_payload() {
        let result = deliver(
            crate::engine::builtins::MathStep::Complete(Completion::Return(Value::Int(7))),
            &mut |_| panic!("immediate result must not enter waiting sink"),
        );
        assert!(matches!(
            result,
            Some(NativeInvokeOutcome::Completion(Completion::Return(
                Value::Int(7)
            )))
        ));
        let result = deliver(
            crate::engine::builtins::ArrayNextStep::Complete(
                NativeInvokeOutcome::IteratorNextRaw {
                    value: Value::Int(9),
                    done: false,
                },
            ),
            &mut |_| panic!("raw immediate result must not enter waiting sink"),
        );
        assert!(matches!(
            result,
            Some(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Int(9),
                done: false
            })
        ));
    }

    #[test]
    fn waiting_domain_is_delivered_once_without_performing_its_read() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let iterator = context.eval("globalThis.readCount=0;Array.prototype.values.call({get length(){readCount++;return 1;},0:4})").unwrap();
        let step = crate::engine::builtins::ArrayNextStep::start(
            &runtime,
            context.realm,
            &NativeInvocation::Call {
                this_value: iterator,
            },
        )
        .unwrap();
        let mut delivered = None;
        assert!(
            deliver(step, &mut |step| {
                assert!(delivered.is_none());
                delivered = Some(step);
            })
            .is_none()
        );
        assert!(matches!(delivered, Some(NativeStep::ArrayNext(_))));
        assert_eq!(context.eval("readCount").unwrap(), Value::Int(0));
    }
}
