//! Shared iterator result parsing and close policies, with owned callback replies.
use super::super::object::ObjectIteratorStep;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{CallableRef, ObjectRef, PropertyKey},
    value::Value,
    vm::{Completion, call::NativeInvokeOutcome},
};

pub(crate) enum NextStep {
    Complete(ObjectIteratorStep),
    Call {
        callable: CallableRef,
        iterator: ObjectRef,
        resume: NextResume,
    },
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: NextResume,
    },
}
pub(crate) struct NextResume(Box<NextResumeState>);
impl std::ops::Deref for NextResume {
    type Target = NextResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for NextResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<NextResume>() <= 8);
pub(crate) struct NextResumeState {
    realm: ContextId,
    phase: NextPhase,
}
enum NextPhase {
    Result,
    Done(ObjectRef),
    Value,
}
impl NextStep {
    pub(crate) fn parse_result(
        runtime: &Runtime,
        realm: ContextId,
        result: Completion,
    ) -> Result<Self, RuntimeError> {
        NextResume(Box::new(NextResumeState {
            realm,
            phase: NextPhase::Result,
        }))
        .resume(runtime, result)
    }

    /// The owned iterator record already holds a checked callable capability.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn start_callable(
        runtime: &Runtime,
        realm: ContextId,
        iterator: ObjectRef,
        callable: CallableRef,
    ) -> Result<Self, RuntimeError> {
        let _operation = runtime.operation();
        if !callable.belongs_to(runtime) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        Ok(Self::call(realm, iterator, callable))
    }

    fn call(realm: ContextId, iterator: ObjectRef, callable: CallableRef) -> Self {
        Self::Call {
            callable,
            iterator,
            resume: NextResume(Box::new(NextResumeState {
                realm,
                phase: NextPhase::Result,
            })),
        }
    }

    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        iterator: ObjectRef,
        method: Value,
    ) -> Result<Self, RuntimeError> {
        let callable = match method {
            Value::Object(ref object) => runtime.as_callable(object)?,
            _ => None,
        };
        let Some(callable) = callable else {
            return Ok(Self::Complete(ObjectIteratorStep::Throw(
                runtime.new_native_error(realm, NativeErrorKind::Type, "not a function")?,
            )));
        };
        Ok(Self::call(realm, iterator, callable))
    }
}
impl NextResume {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_raw(realm: ContextId) -> Self {
        Self(Box::new(NextResumeState {
            realm,
            phase: NextPhase::Result,
        }))
    }
    pub(crate) fn raw(
        self,
        runtime: &Runtime,
        result: NativeInvokeOutcome,
    ) -> Result<NextStep, RuntimeError> {
        match self.raw_completion(result)? {
            Ok(result) => Ok(NextStep::Complete(result)),
            Err(result) => self.resume(runtime, result),
        }
    }

    /// Classify a native reply without transporting this continuation or a
    /// waiting NextStep. Only ordinary returned values need result parsing.
    pub(crate) fn raw_completion(
        &self,
        result: NativeInvokeOutcome,
    ) -> Result<Result<ObjectIteratorStep, Completion>, RuntimeError> {
        if !matches!(self.0.phase, NextPhase::Result) {
            return Err(RuntimeError::Invariant(
                "raw iterator reply has the wrong phase",
            ));
        }
        Ok(match result {
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(if done {
                ObjectIteratorStep::Done
            } else {
                ObjectIteratorStep::Yield(value)
            }),
            NativeInvokeOutcome::Completion(Completion::Throw(value)) => {
                Ok(ObjectIteratorStep::Throw(value))
            }
            NativeInvokeOutcome::Completion(result) => Err(result),
        })
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<NextStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(NextStep::Complete(ObjectIteratorStep::Throw(value)));
            }
        };
        let realm = self.0.realm;
        match self.0.phase {
            NextPhase::Result => {
                let Value::Object(object) = value else {
                    return Ok(NextStep::Complete(ObjectIteratorStep::Throw(
                        runtime.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "iterator must return an object",
                        )?,
                    )));
                };
                Ok(NextStep::Read {
                    object: object.clone(),
                    key: runtime.intern_property_key("done")?,
                    resume: Self(Box::new(NextResumeState {
                        realm,
                        phase: NextPhase::Done(object),
                    })),
                })
            }
            NextPhase::Done(object) => {
                if runtime.value_to_boolean(&value)? {
                    return Ok(NextStep::Complete(ObjectIteratorStep::Done));
                }
                Ok(NextStep::Read {
                    object,
                    key: runtime.intern_property_key("value")?,
                    resume: Self(Box::new(NextResumeState {
                        realm,
                        phase: NextPhase::Value,
                    })),
                })
            }
            NextPhase::Value => Ok(NextStep::Complete(ObjectIteratorStep::Yield(value))),
        }
    }
}
pub(crate) fn finish_next(
    runtime: &Runtime,
    realm: ContextId,
    mut step: NextStep,
) -> Result<ObjectIteratorStep, RuntimeError> {
    loop {
        step = match step {
            NextStep::Complete(result) => return Ok(result),
            NextStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            NextStep::Call {
                callable,
                iterator,
                resume,
            } => {
                let receiver = Value::Object(iterator);
                match runtime.try_call_native_iterator_next_raw(
                    realm,
                    &callable,
                    receiver.clone(),
                )? {
                    Some(result) => resume.raw(runtime, result)?,
                    None => resume.resume(
                        runtime,
                        runtime.call_internal(realm, &callable, receiver, &[])?,
                    )?,
                }
            }
        };
    }
}

pub(crate) enum CloseStep {
    Complete(Completion),
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: CloseResume,
    },
    Call {
        callable: CallableRef,
        iterator: ObjectRef,
        resume: CloseResume,
    },
}
pub(crate) struct CloseResume(Box<CloseResumeState>);
impl std::ops::Deref for CloseResume {
    type Target = CloseResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for CloseResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<CloseResume>() <= 8);
pub(crate) struct CloseResumeState {
    realm: ContextId,
    iterator: ObjectRef,
    completion: Completion,
    called: bool,
}
impl CloseStep {
    /// A pending Throw suppresses every JavaScript failure in close; a Return
    /// requires a callable return method and an object result.
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        iterator: ObjectRef,
        completion: Completion,
    ) -> Result<Self, RuntimeError> {
        Ok(Self::Read {
            object: iterator.clone(),
            key: runtime.intern_property_key("return")?,
            resume: CloseResume(Box::new(CloseResumeState {
                realm,
                iterator,
                completion,
                called: false,
            })),
        })
    }
}
impl CloseResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<CloseStep, RuntimeError> {
        let preserving = matches!(self.0.completion, Completion::Throw(_));
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(CloseStep::Complete(if preserving {
                    self.0.completion
                } else {
                    Completion::Throw(value)
                }));
            }
        };
        if self.0.called {
            return Ok(CloseStep::Complete(
                if preserving || matches!(value, Value::Object(_)) {
                    self.0.completion
                } else {
                    Completion::Throw(runtime.new_native_error(
                        self.0.realm,
                        NativeErrorKind::Type,
                        "not an object",
                    )?)
                },
            ));
        }
        if matches!(value, Value::Undefined | Value::Null) {
            return Ok(CloseStep::Complete(self.0.completion));
        }
        let callable = match value {
            Value::Object(ref object) => runtime.as_callable(object)?,
            _ => None,
        };
        let Some(callable) = callable else {
            return Ok(CloseStep::Complete(if preserving {
                self.0.completion
            } else {
                Completion::Throw(runtime.new_native_error(
                    self.0.realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?)
            }));
        };
        self.0.called = true;
        Ok(CloseStep::Call {
            callable,
            iterator: self.0.iterator.clone(),
            resume: self,
        })
    }
}
pub(crate) fn finish_close(
    runtime: &Runtime,
    realm: ContextId,
    mut step: CloseStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            CloseStep::Complete(result) => return Ok(result),
            CloseStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            CloseStep::Call {
                callable,
                iterator,
                resume,
            } => resume.resume(
                runtime,
                runtime.call_internal(realm, &callable, Value::Object(iterator), &[])?,
            )?,
        };
    }
}

#[cfg(test)]
mod raw_completion_tests {
    use super::*;

    #[test]
    fn raw_completion_checks_phase_and_releases_ignored_done_value() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let resume = NextResume(Box::new(NextResumeState {
            realm: context.realm,
            phase: NextPhase::Result,
        }));
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        assert!(matches!(
            resume
                .raw_completion(NativeInvokeOutcome::IteratorNextRaw {
                    value: Value::Object(object),
                    done: true,
                })
                .unwrap(),
            Ok(ObjectIteratorStep::Done)
        ));
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
        let wrong = NextResume(Box::new(NextResumeState {
            realm: context.realm,
            phase: NextPhase::Value,
        }));
        for reply in [
            NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            },
            NativeInvokeOutcome::Completion(Completion::Throw(Value::Int(7))),
        ] {
            assert!(matches!(
                wrong.raw_completion(reply),
                Err(RuntimeError::Invariant(
                    "raw iterator reply has the wrong phase"
                ))
            ));
        }
    }

    #[test]
    fn consuming_raw_wrapper_drops_wrong_phase_result_owner() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        let wrong = NextResume(Box::new(NextResumeState {
            realm: context.realm,
            phase: NextPhase::Done(object),
        }));
        assert!(
            wrong
                .raw(
                    &runtime,
                    NativeInvokeOutcome::IteratorNextRaw {
                        value: Value::Undefined,
                        done: false,
                    }
                )
                .is_err()
        );
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
    }

    #[test]
    fn raw_completion_preserves_yield_and_throw_identity() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let resume = NextResume(Box::new(NextResumeState {
            realm: context.realm,
            phase: NextPhase::Result,
        }));
        let marker = Value::Object(runtime.new_object(None).unwrap());
        let Ok(ObjectIteratorStep::Yield(value)) = resume
            .raw_completion(NativeInvokeOutcome::IteratorNextRaw {
                value: marker.clone(),
                done: false,
            })
            .unwrap()
        else {
            panic!("yield lost");
        };
        assert_eq!(value, marker);
        let Ok(ObjectIteratorStep::Throw(value)) = resume
            .raw_completion(NativeInvokeOutcome::Completion(Completion::Throw(
                marker.clone(),
            )))
            .unwrap()
        else {
            panic!("throw lost");
        };
        assert_eq!(value, marker);
    }

    #[test]
    fn raw_completion_ordinary_results_reuse_done_value_parsing_once() {
        for wrapper in [false, true] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let result = context.eval("globalThis.trace='';globalThis.marker={};({get done(){trace+='d';return false},get value(){trace+='v';return marker}})").unwrap();
            let marker = context.eval("marker").unwrap();
            let resume = NextResume(Box::new(NextResumeState {
                realm: context.realm,
                phase: NextPhase::Result,
            }));
            let reply = NativeInvokeOutcome::Completion(Completion::Return(result));
            let step = if wrapper {
                resume.raw(&runtime, reply).unwrap()
            } else {
                let Err(result) = resume.raw_completion(reply).unwrap() else {
                    panic!("ordinary result skipped parsing");
                };
                assert_eq!(
                    context.eval("trace").unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(""))
                );
                resume.resume(&runtime, result).unwrap()
            };
            let ObjectIteratorStep::Yield(value) =
                finish_next(&runtime, context.realm, step).unwrap()
            else {
                panic!("result lost");
            };
            assert_eq!(value, marker);
            assert_eq!(
                context.eval("trace").unwrap(),
                Value::String(crate::engine::value::JsString::from_static("dv"))
            );
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<CloseStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<NextStep>() <= 64);
