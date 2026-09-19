//! AggregateError owns its unpublished errors array and closes after IteratorNext failure.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::{
        iterator::step::{CloseStep, NextStep, finish_close, finish_next},
        object::ObjectIteratorStep,
    },
    heap::ContextId,
    object::{
        CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
        WellKnownSymbol,
    },
    value::Value,
    vm::Completion,
};
pub(crate) enum AggregateStep {
    Complete(Completion),
    Read {
        receiver: Value,
        key: PropertyKey,
        resume: AggregateResume,
    },
    Call {
        callable: CallableRef,
        receiver: Value,
        resume: AggregateResume,
    },
    Next {
        iterator: ObjectRef,
        next: Value,
        resume: AggregateResume,
    },
    Close {
        iterator: ObjectRef,
        completion: Completion,
    },
}
enum Phase {
    Method,
    Iterator,
    NextMethod,
    Next,
}
pub(crate) struct AggregateResume(Box<AggregateResumeState>);
impl std::ops::Deref for AggregateResume {
    type Target = AggregateResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for AggregateResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<AggregateResume>() <= 8);
pub(crate) struct AggregateResumeState {
    realm: ContextId,
    phase: Phase,
    iterable: Value,
    iterator: Option<ObjectRef>,
    next: Value,
    result: Option<ObjectRef>,
    index: u64,
}
impl AggregateStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        iterable: Value,
    ) -> Result<Self, RuntimeError> {
        if matches!(iterable, Value::Null | Value::Undefined) {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(
                    realm,
                    NativeErrorKind::Type,
                    &format!(
                        "cannot read property 'Symbol.iterator' of {}",
                        if matches!(iterable, Value::Null) {
                            "null"
                        } else {
                            "undefined"
                        }
                    ),
                )?,
            )));
        }
        Ok(Self::Read {
            receiver: iterable.clone(),
            key: PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator)),
            resume: AggregateResume(Box::new(AggregateResumeState {
                realm,
                phase: Phase::Method,
                iterable,
                iterator: None,
                next: Value::Undefined,
                result: None,
                index: 0,
            })),
        })
    }
}
impl AggregateResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<AggregateStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(AggregateStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Method => {
                let callable = match value {
                    Value::Object(object) => runtime.as_callable(&object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return Ok(AggregateStep::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "value is not iterable",
                        )?,
                    )));
                };
                self.0.phase = Phase::Iterator;
                Ok(AggregateStep::Call {
                    callable,
                    receiver: self.0.iterable.clone(),
                    resume: self,
                })
            }
            Phase::Iterator => {
                let Value::Object(iterator) = value else {
                    return Ok(AggregateStep::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "not an object",
                        )?,
                    )));
                };
                self.0.iterable = Value::Undefined;
                self.0.iterator = Some(iterator.clone());
                self.0.phase = Phase::NextMethod;
                Ok(AggregateStep::Read {
                    receiver: Value::Object(iterator),
                    key: runtime
                        .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Next)?,
                    resume: self,
                })
            }
            Phase::NextMethod => {
                self.0.next = value;
                self.0.result = Some(runtime.new_array(self.0.realm)?);
                self.next()
            }
            _ => Err(RuntimeError::Invariant(
                "AggregateError value phase mismatch",
            )),
        }
    }
    fn next(mut self) -> Result<AggregateStep, RuntimeError> {
        self.0.phase = Phase::Next;
        Ok(AggregateStep::Next {
            iterator: self
                .0
                .iterator
                .clone()
                .ok_or(RuntimeError::Invariant("AggregateError iterator missing"))?,
            next: self.0.next.clone(),
            resume: self,
        })
    }
    fn close(self, value: Value) -> Result<AggregateStep, RuntimeError> {
        Ok(AggregateStep::Close {
            iterator: self
                .0
                .iterator
                .ok_or(RuntimeError::Invariant("AggregateError iterator missing"))?,
            completion: Completion::Throw(value),
        })
    }
    pub(crate) fn item(
        mut self,
        runtime: &Runtime,
        result: ObjectIteratorStep,
    ) -> Result<AggregateStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Next) {
            return Err(RuntimeError::Invariant(
                "AggregateError iterator phase mismatch",
            ));
        }
        let value = match result {
            ObjectIteratorStep::Yield(value) => value,
            ObjectIteratorStep::Done => {
                return Ok(AggregateStep::Complete(Completion::Return(Value::Object(
                    self.0
                        .result
                        .ok_or(RuntimeError::Invariant("AggregateError result missing"))?,
                ))));
            }
            ObjectIteratorStep::Throw(value) => return self.close(value),
        };
        let result = self
            .0
            .result
            .as_ref()
            .ok_or(RuntimeError::Invariant("AggregateError result missing"))?;
        let key = runtime.intern_property_key(&self.0.index.to_string())?;
        // The result has not been exposed to JavaScript: own data definition on this fresh Array is callback-free.
        match runtime.define_own_property(
            result,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            true => {}
            false => {
                return Err(RuntimeError::Invariant(
                    "fresh AggregateError Array rejected indexed property",
                ));
            }
        }
        self.0.index = self.0.index.checked_add(1).ok_or(RuntimeError::Invariant(
            "AggregateError iterable exceeded Uint64 indices",
        ))?;
        self.next()
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: AggregateStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            AggregateStep::Complete(result) => return Ok(result),
            AggregateStep::Read {
                receiver,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_value_property_in_realm(realm, receiver, &key)?,
            )?,
            AggregateStep::Call {
                callable,
                receiver,
                resume,
            } => resume.resume(
                runtime,
                runtime.call_internal(realm, &callable, receiver, &[])?,
            )?,
            AggregateStep::Next {
                iterator,
                next,
                resume,
            } => resume.item(
                runtime,
                finish_next(
                    runtime,
                    realm,
                    NextStep::start(runtime, realm, iterator, next)?,
                )?,
            )?,
            AggregateStep::Close {
                iterator,
                completion,
            } => {
                return finish_close(
                    runtime,
                    realm,
                    CloseStep::start(runtime, realm, iterator, completion)?,
                );
            }
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<AggregateStep>() <= 64);
