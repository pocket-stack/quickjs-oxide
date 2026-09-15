//! Array Iterator next keeps the raw value/done ABI across callback requests.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::ArrayIteratorKind,
    heap::{ContextId, HeapError},
    object::{ObjectRef, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeInvocation, NativeInvokeOutcome},
    },
};
pub(crate) enum ArrayNextStep {
    Complete(NativeInvokeOutcome),
    #[cfg(feature = "stack-vm")]
    PreparedRead {
        resume: ArrayNextResume,
    },
    Read {
        resume: ArrayNextResume,
    },
    Number {
        resume: ArrayNextResume,
    },
}
const _: () = assert!(std::mem::size_of::<ArrayNextStep>() <= 64);
pub(crate) struct ArrayNextResume(Box<ArrayNextResumeState>);
impl std::ops::Deref for ArrayNextResume {
    type Target = ArrayNextResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ArrayNextResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ArrayNextResume>() <= 8);
pub(crate) struct ArrayNextResumeState {
    realm: ContextId,
    iterator: ObjectRef,
    source: ObjectRef,
    index: u32,
    kind: ArrayIteratorKind,
    phase: Phase,
    requested_object: Option<ObjectRef>,
    requested_key: Option<PropertyKey>,
    requested_value: Option<Value>,
    #[cfg(feature = "stack-vm")]
    requested_read: Option<crate::engine::object::OrdinaryRead>,
}
enum Phase {
    Length,
    Number,
    Value,
}
impl ArrayNextStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array Iterator next did not receive an iterator-next invocation",
            ));
        };
        let Value::Object(iterator) = this_value else {
            return Self::wrong_receiver(runtime, realm);
        };
        let state = runtime
            .0
            .state
            .borrow()
            .heap
            .array_iterator_state(iterator.object_id());
        let (source, index, kind) = match state {
            Ok(state) => state,
            Err(HeapError::Invariant(_)) => return Self::wrong_receiver(runtime, realm),
            Err(error) => return Err(error.into()),
        };
        let Some(source) = source else {
            return Ok(Self::Complete(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            }));
        };
        #[cfg(feature = "stack-vm")]
        if let Some(value) = Self::dense_immediate_next(runtime, iterator, source, index, kind)? {
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "array_next_dense_immediate_leaf",
            );
            return Ok(Self::Complete(NativeInvokeOutcome::IteratorNextRaw {
                value,
                done: false,
            }));
        }
        let source = ObjectRef::from_borrowed_handle(runtime.clone(), source)?;
        let mut resume = ArrayNextResume(Box::new(ArrayNextResumeState {
            realm,
            iterator: iterator.clone(),
            source,
            index,
            kind,
            phase: Phase::Length,
            requested_object: None,
            requested_key: None,
            requested_value: None,
            #[cfg(feature = "stack-vm")]
            requested_read: None,
        }));
        if runtime.typed_array_is_object(&resume.source)? {
            let action = match runtime.typed_array_validated_length(realm, &resume.source)? {
                NativeConversion::Value(length) => resume.length(runtime, length)?,
                NativeConversion::Throw(value) => {
                    NextAction::Complete(NativeInvokeOutcome::Completion(Completion::Throw(value)))
                }
            };
            return resume.drive(runtime, action);
        }
        let key = runtime.intern_property_key("length")?;
        resume.drive(runtime, NextAction::Read(key))
    }
    fn wrong_receiver(runtime: &Runtime, realm: ContextId) -> Result<Self, RuntimeError> {
        Ok(Self::Complete(NativeInvokeOutcome::Completion(
            Completion::Throw(runtime.new_native_error(
                realm,
                NativeErrorKind::Type,
                "Array Iterator object expected",
            )?),
        )))
    }
}
enum NextAction {
    Complete(NativeInvokeOutcome),
    Read(PropertyKey),
    Number(Value),
}

impl ArrayNextResume {
    fn resume_once(
        &mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<NextAction, RuntimeError> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(NextAction::Complete(NativeInvokeOutcome::Completion(
                    Completion::Throw(value),
                )));
            }
        };
        match self.0.phase {
            Phase::Length => {
                self.0.phase = Phase::Number;
                Ok(NextAction::Number(value))
            }
            Phase::Value => {
                let value = if self.0.kind == ArrayIteratorKind::KeyAndValue {
                    Value::Object(runtime.new_array_from_values(
                        self.0.realm,
                        vec![Runtime::array_length_value(self.0.index), value],
                    )?)
                } else {
                    value
                };
                Ok(NextAction::Complete(NativeInvokeOutcome::IteratorNextRaw {
                    value,
                    done: false,
                }))
            }
            Phase::Number => Err(RuntimeError::Invariant(
                "Array Iterator number phase received completion",
            )),
        }
    }
    fn number_once(
        &mut self,
        runtime: &Runtime,
        reply: NativeConversion<f64>,
    ) -> Result<NextAction, RuntimeError> {
        if !matches!(self.0.phase, Phase::Number) {
            return Err(RuntimeError::Invariant(
                "Array Iterator numeric reply has wrong phase",
            ));
        }
        match reply {
            NativeConversion::Value(value) => {
                self.length(runtime, Runtime::to_uint32_number(value))
            }
            NativeConversion::Throw(value) => Ok(NextAction::Complete(
                NativeInvokeOutcome::Completion(Completion::Throw(value)),
            )),
        }
    }
    fn length(&mut self, runtime: &Runtime, length: u32) -> Result<NextAction, RuntimeError> {
        let Some(next_index) = live_next_index(self.0.index, length) else {
            let mut state = runtime.0.state.borrow_mut();
            let cleanup = state
                .heap
                .finish_array_iterator(self.0.iterator.object_id())?;
            state.apply_cleanup(cleanup)?;
            return Ok(NextAction::Complete(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            }));
        };
        runtime
            .0
            .state
            .borrow_mut()
            .heap
            .set_array_iterator_index(self.0.iterator.object_id(), next_index)?;
        if self.0.kind == ArrayIteratorKind::Key {
            return Ok(NextAction::Complete(NativeInvokeOutcome::IteratorNextRaw {
                value: Runtime::array_length_value(self.0.index),
                done: false,
            }));
        }
        self.0.phase = Phase::Value;
        Ok(NextAction::Read(
            runtime.property_key_for_index(self.0.index as u64)?,
        ))
    }
}
impl ArrayNextResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<ArrayNextStep, RuntimeError> {
        let action = self.resume_once(runtime, reply)?;
        self.drive(runtime, action)
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<f64>,
    ) -> Result<ArrayNextStep, RuntimeError> {
        let action = self.number_once(runtime, reply)?;
        self.drive(runtime, action)
    }
    fn drive(
        mut self,
        runtime: &Runtime,
        mut action: NextAction,
    ) -> Result<ArrayNextStep, RuntimeError> {
        loop {
            #[cfg(feature = "stack-vm")]
            {
                use crate::engine::object::OrdinaryRead;
                use crate::engine::value::conversion::number::NumberStep;
                action = match action {
                    NextAction::Read(key) => {
                        let receiver = Value::Object(self.0.source.clone());
                        match runtime.prepare_ordinary_read_borrowed(
                            &self.0.source,
                            &key,
                            &receiver,
                        )? {
                            OrdinaryRead::Complete(value) => self.resume_once(
                                runtime,
                                Completion::Return(value.unwrap_or(Value::Undefined)),
                            )?,
                            read => {
                                return Ok(self.prepared(read, key));
                            }
                        }
                    }
                    NextAction::Number(value) if !matches!(value, Value::Object(_)) => {
                        let NumberStep::Complete(reply) =
                            NumberStep::start(runtime, self.0.realm, value)?
                        else {
                            return Err(RuntimeError::Invariant(
                                "primitive iterator number suspended",
                            ));
                        };
                        self.number_once(runtime, reply)?
                    }
                    action => return Ok(self.wait(action)),
                };
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    "array_next_resident_stage",
                );
            }
            #[cfg(not(feature = "stack-vm"))]
            return Ok(self.wait(action));
        }
    }
    #[cfg(feature = "stack-vm")]
    fn prepared(
        mut self,
        read: crate::engine::object::OrdinaryRead,
        key: PropertyKey,
    ) -> ArrayNextStep {
        self.requested_read = Some(read);
        self.requested_key = Some(key);
        ArrayNextStep::PreparedRead { resume: self }
    }
    #[cfg(feature = "stack-vm")]
    pub(crate) fn take_prepared(&mut self) -> crate::engine::object::OrdinaryRead {
        self.requested_read
            .take()
            .expect("array next prepared read")
    }
    #[cfg(feature = "stack-vm")]
    pub(crate) fn take_key(&mut self) -> PropertyKey {
        self.requested_key.take().expect("array next key")
    }
    pub(crate) fn take_read(&mut self) -> (ObjectRef, PropertyKey) {
        (
            self.requested_object.take().expect("array next object"),
            self.requested_key.take().expect("array next key"),
        )
    }
    pub(crate) fn take_number(&mut self) -> Value {
        self.requested_value.take().expect("array next number")
    }
    fn wait(mut self, action: NextAction) -> ArrayNextStep {
        match action {
            NextAction::Complete(result) => ArrayNextStep::Complete(result),
            NextAction::Read(key) => {
                self.requested_object = Some(self.source.clone());
                self.requested_key = Some(key);
                ArrayNextStep::Read { resume: self }
            }
            NextAction::Number(value) => {
                self.requested_value = Some(value);
                ArrayNextStep::Number { resume: self }
            }
        }
    }
}

// Shared advance/completion decision. An in-range Uint32 index always has a
// representable successor; neither caller can overflow at the last element.
fn live_next_index(index: u32, length: u32) -> Option<u32> {
    (index < length).then(|| index + 1)
}

pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ArrayNextStep,
) -> Result<NativeInvokeOutcome, RuntimeError> {
    loop {
        step = match step {
            ArrayNextStep::Complete(result) => return Ok(result),
            #[cfg(feature = "stack-vm")]
            ArrayNextStep::PreparedRead { mut resume } => {
                let read = resume.take_prepared();
                let key = resume.take_key();
                let completion = match runtime.finish_prepared_read(realm, &key, read)? {
                    NativeConversion::Value(value) => {
                        Completion::Return(value.unwrap_or(Value::Undefined))
                    }
                    NativeConversion::Throw(value) => Completion::Throw(value),
                };
                resume.resume(runtime, completion)?
            }
            ArrayNextStep::Read { mut resume } => {
                let (object, key) = resume.take_read();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            ArrayNextStep::Number { mut resume } => {
                let value = resume.take_number();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
        };
    }
}

#[cfg(feature = "stack-vm")]
mod local;

#[cfg(all(test, feature = "stack-vm", feature = "profiling"))]
mod tests;

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ArrayNextStep>() <= 64);
