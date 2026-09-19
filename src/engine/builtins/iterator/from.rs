//! Iterator.from acquires next before the ordinary instance check.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{CallableRef, ObjectRef, PropertyKey, WellKnownSymbol},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum FromStep {
    Complete(Completion),
    Read { resume: FromResume },
    Call { resume: FromResume },
    Instance { resume: FromResume },
}
pub(crate) struct FromResume(Box<FromResumeState>);
impl std::ops::Deref for FromResume {
    type Target = FromResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for FromResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<FromResume>() <= 8);
pub(crate) struct FromResumeState {
    pending_effect: FromStepPending,
    realm: ContextId,
    phase: Phase,
}
enum Phase {
    Method(Value),
    Iterator,
    Next(Value),
    Instance { iterator: Value, next: Value },
}
impl FromStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        if !matches!(invocation, NativeInvocation::Call { .. }) {
            return Err(RuntimeError::Invariant(
                "Iterator.from did not receive a generic invocation",
            ));
        }
        let input = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Iterator.from argument was not padded",
            ))?;
        if !matches!(input, Value::Object(_) | Value::String(_)) {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(
                    realm,
                    NativeErrorKind::Type,
                    "Iterator.from called on non-object",
                )?,
            )));
        }
        Ok({
            let __pending_field_receiver = input.clone();
            let __pending_field_key =
                PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
            let __pending_field_resume = FromResume(Box::new(FromResumeState {
                pending_effect: FromStepPending::default(),
                realm,
                phase: Phase::Method(input),
            }));
            Self::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
}
impl FromResume {
    fn next(mut self, runtime: &Runtime, iterator: Value) -> Result<FromStep, RuntimeError> {
        self.0.phase = Phase::Next(iterator.clone());
        Ok({
            let __pending_field_receiver = iterator;
            let __pending_field_key =
                runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Next)?;
            let __pending_field_resume = self;
            FromStep::request_read(
                __pending_field_receiver,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<FromStep, RuntimeError> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(FromStep::Complete(Completion::Throw(value))),
        };
        match std::mem::replace(&mut self.0.phase, Phase::Iterator) {
            Phase::Method(input) => {
                if matches!(value, Value::Undefined | Value::Null) {
                    return self.next(runtime, input);
                }
                let callable = match runtime.iterator_callable_value(self.0.realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(FromStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok({
                    let __pending_field_callable = callable;
                    let __pending_field_receiver = input;
                    let __pending_field_resume = self;
                    FromStep::request_call(
                        __pending_field_callable,
                        __pending_field_receiver,
                        __pending_field_resume,
                    )
                })
            }
            Phase::Iterator => {
                if !matches!(value, Value::Object(_)) {
                    return Ok(FromStep::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "not an object",
                        )?,
                    )));
                }
                self.next(runtime, value)
            }
            Phase::Next(iterator) => {
                let constructor = runtime.iterator_realm_data(self.0.realm)?.constructor;
                let constructor = CallableRef::from_validated_object(
                    ObjectRef::from_borrowed_handle(runtime.clone(), constructor)?,
                );
                self.0.phase = Phase::Instance {
                    iterator: iterator.clone(),
                    next: value,
                };
                Ok({
                    let __pending_field_constructor = constructor;
                    let __pending_field_value = iterator;
                    let __pending_field_resume = self;
                    FromStep::request_instance(
                        __pending_field_constructor,
                        __pending_field_value,
                        __pending_field_resume,
                    )
                })
            }
            Phase::Instance { iterator, next } => Ok(FromStep::Complete(Completion::Return(
                if runtime.value_to_boolean(&value)? {
                    iterator
                } else {
                    Value::Object(runtime.new_iterator_wrap(self.0.realm, &iterator, &next)?)
                },
            ))),
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: FromStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            FromStep::Complete(result) => return Ok(result),
            FromStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            FromStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &[])?,
                )?
            }
            FromStep::Instance { mut resume } => {
                let constructor = resume.take_instance_constructor();
                let value = resume.take_instance_value();
                resume.resume(
                    runtime,
                    runtime.ordinary_is_instance_of(realm, &constructor, value)?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct FromStepPending {
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    instance_constructor: Option<CallableRef>,
    instance_value: Option<Value>,
}
impl FromStep {
    pub(crate) fn request_read(receiver: Value, key: PropertyKey, mut resume: FromResume) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        mut resume: FromResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        Self::Call { resume }
    }
    pub(crate) fn request_instance(
        constructor: CallableRef,
        value: Value,
        mut resume: FromResume,
    ) -> Self {
        resume.0.pending_effect.instance_constructor = Some(constructor);
        resume.0.pending_effect.instance_value = Some(value);
        Self::Instance { resume }
    }
}
impl FromResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("FromStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("FromStep Read key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("FromStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("FromStep Call receiver")
    }
    pub(crate) fn take_instance_constructor(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .instance_constructor
            .take()
            .expect("FromStep Instance constructor")
    }
    pub(crate) fn take_instance_value(&mut self) -> Value {
        self.0
            .pending_effect
            .instance_value
            .take()
            .expect("FromStep Instance value")
    }
}
const _: () = assert!(std::mem::size_of::<FromStep>() <= 56);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<FromStep>() <= 64);
