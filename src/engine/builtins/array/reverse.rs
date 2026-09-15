//! Reverse observes both indexed properties before performing either mutation.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, operations::InternalSetResult},
    value::{Value, conversion::NativeConversion},
    vm::{Completion, call::NativeInvocation},
};
pub(crate) enum ReverseStep {
    Complete(Completion),
    Read { resume: ReverseResume },
    Number { resume: ReverseResume },
    Has { resume: ReverseResume },
    Set { resume: ReverseResume },
    Delete { resume: ReverseResume },
}
enum Phase {
    Length,
    Number,
    LowerHas,
    LowerRead,
    UpperHas,
    UpperRead,
    LowerWrite,
    UpperWrite,
}
pub(crate) struct ReverseResume(Box<ReverseResumeState>);
impl std::ops::Deref for ReverseResume {
    type Target = ReverseResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ReverseResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ReverseResume>() <= 8);
pub(crate) struct ReverseResumeState {
    pending_effect: ReverseStepPending,
    scheduler_set_key: Option<PropertyKey>,
    realm: ContextId,
    object: ObjectRef,
    phase: Phase,
    lower: u64,
    upper: u64,
    lower_value: Option<Value>,
    upper_value: Option<Value>,
}
impl ReverseStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array reverse requires generic invocation",
            ));
        };
        let object = match runtime.native_to_object(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        Ok(Self::request_read(
            object.clone(),
            runtime.intern_property_key("length")?,
            ReverseResume(Box::new(ReverseResumeState {
                pending_effect: ReverseStepPending::default(),
                scheduler_set_key: None,
                realm,
                object,
                phase: Phase::Length,
                lower: 0,
                upper: 0,
                lower_value: None,
                upper_value: None,
            })),
        ))
    }
}
impl ReverseResume {
    pub(crate) fn with_scheduler_set_key(mut self, key: PropertyKey) -> Self {
        self.0.scheduler_set_key = Some(key);
        self
    }
    pub(crate) fn take_scheduler_set_key(&mut self) -> PropertyKey {
        self.0.scheduler_set_key.take().expect("waiting Set key")
    }

    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<ReverseStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(ReverseStep::Complete(Completion::Throw(value))),
        };
        match self.0.phase {
            Phase::Length => {
                self.0.phase = Phase::Number;
                Ok(ReverseStep::request_number(value, self))
            }
            Phase::LowerRead => {
                self.0.lower_value = Some(value);
                self.upper(runtime)
            }
            Phase::UpperRead => {
                self.0.upper_value = Some(value);
                self.write_lower(runtime)
            }
            _ => Err(RuntimeError::Invariant(
                "Array reverse value phase mismatch",
            )),
        }
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<ReverseStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Number) {
            return Err(RuntimeError::Invariant(
                "Array reverse number phase mismatch",
            ));
        }
        self.0.upper = match result {
            NativeConversion::Value(value) => Runtime::length_from_number(value).saturating_sub(1),
            NativeConversion::Throw(value) => {
                return Ok(ReverseStep::Complete(Completion::Throw(value)));
            }
        };
        self.next(runtime)
    }
    fn next(mut self, runtime: &Runtime) -> Result<ReverseStep, RuntimeError> {
        if self.0.lower >= self.0.upper {
            return Ok(ReverseStep::Complete(Completion::Return(Value::Object(
                self.0.object,
            ))));
        }
        self.0.phase = Phase::LowerHas;
        self.0.lower_value = None;
        self.0.upper_value = None;
        Ok(ReverseStep::request_has(
            self.0.object.clone(),
            runtime.property_key_for_index(self.0.lower)?,
            self,
        ))
    }
    fn upper(mut self, runtime: &Runtime) -> Result<ReverseStep, RuntimeError> {
        self.0.phase = Phase::UpperHas;
        Ok(ReverseStep::request_has(
            self.0.object.clone(),
            runtime.property_key_for_index(self.0.upper)?,
            self,
        ))
    }
    fn write_lower(mut self, runtime: &Runtime) -> Result<ReverseStep, RuntimeError> {
        self.0.phase = Phase::LowerWrite;
        if let Some(value) = self.0.upper_value.take() {
            Ok(ReverseStep::request_set(
                self.0.object.clone(),
                runtime.property_key_for_index(self.0.lower)?,
                value,
                self,
            ))
        } else if self.0.lower_value.is_some() {
            Ok(ReverseStep::request_delete(
                self.0.object.clone(),
                runtime.property_key_for_index(self.0.lower)?,
                self,
            ))
        } else {
            self.advance(runtime)
        }
    }
    fn write_upper(mut self, runtime: &Runtime) -> Result<ReverseStep, RuntimeError> {
        self.0.phase = Phase::UpperWrite;
        if let Some(value) = self.0.lower_value.take() {
            Ok(ReverseStep::request_set(
                self.0.object.clone(),
                runtime.property_key_for_index(self.0.upper)?,
                value,
                self,
            ))
        } else {
            Ok(ReverseStep::request_delete(
                self.0.object.clone(),
                runtime.property_key_for_index(self.0.upper)?,
                self,
            ))
        }
    }
    fn advance(mut self, runtime: &Runtime) -> Result<ReverseStep, RuntimeError> {
        self.0.lower += 1;
        self.0.upper -= 1;
        self.next(runtime)
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<ReverseStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ReverseStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::LowerHas | Phase::UpperHas => {
                let lower = matches!(self.0.phase, Phase::LowerHas);
                if value {
                    self.0.phase = if lower {
                        Phase::LowerRead
                    } else {
                        Phase::UpperRead
                    };
                    Ok(ReverseStep::request_read(
                        self.0.object.clone(),
                        runtime.property_key_for_index(if lower {
                            self.0.lower
                        } else {
                            self.0.upper
                        })?,
                        self,
                    ))
                } else if lower {
                    self.upper(runtime)
                } else {
                    self.write_lower(runtime)
                }
            }
            Phase::LowerWrite | Phase::UpperWrite => {
                if !value {
                    return Ok(ReverseStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "could not delete property",
                        )?,
                    )));
                }
                if matches!(self.0.phase, Phase::LowerWrite) {
                    self.write_upper(runtime)
                } else {
                    self.advance(runtime)
                }
            }
            _ => Err(RuntimeError::Invariant(
                "Array reverse boolean phase mismatch",
            )),
        }
    }
    pub(crate) fn set(
        self,
        runtime: &Runtime,
        key: PropertyKey,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<ReverseStep, RuntimeError> {
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            return Ok(ReverseStep::Complete(Completion::Throw(value)));
        }
        match self.0.phase {
            Phase::LowerWrite => self.write_upper(runtime),
            Phase::UpperWrite => self.advance(runtime),
            _ => Err(RuntimeError::Invariant("Array reverse set phase mismatch")),
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ReverseStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            ReverseStep::Complete(result) => return Ok(result),
            ReverseStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            ReverseStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            ReverseStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                resume.boolean(
                    runtime,
                    runtime.internal_has_property(realm, &object, &key)?,
                )?
            }
            ReverseStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                {
                    let result = runtime.internal_set(
                        realm,
                        &object,
                        &key,
                        value,
                        Value::Object(object.clone()),
                    )?;
                    resume.set(runtime, key, result)?
                }
            }
            ReverseStep::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                resume.boolean(
                    runtime,
                    runtime.internal_delete_property(realm, &object, &key)?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct ReverseStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    has_object: Option<ObjectRef>,
    has_key: Option<PropertyKey>,
    set_object: Option<ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<Value>,
    delete_object: Option<ObjectRef>,
    delete_key: Option<PropertyKey>,
}
impl ReverseStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ReverseResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: ReverseResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_has(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ReverseResume,
    ) -> Self {
        resume.0.pending_effect.has_object = Some(object);
        resume.0.pending_effect.has_key = Some(key);
        Self::Has { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: ReverseResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        Self::Set { resume }
    }
    pub(crate) fn request_delete(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ReverseResume,
    ) -> Self {
        resume.0.pending_effect.delete_object = Some(object);
        resume.0.pending_effect.delete_key = Some(key);
        Self::Delete { resume }
    }
}
impl ReverseResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ReverseStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ReverseStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("ReverseStep Number value")
    }
    pub(crate) fn take_has_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .has_object
            .take()
            .expect("ReverseStep Has object")
    }
    pub(crate) fn take_has_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .has_key
            .take()
            .expect("ReverseStep Has key")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("ReverseStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("ReverseStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("ReverseStep Set value")
    }
    pub(crate) fn take_delete_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .delete_object
            .take()
            .expect("ReverseStep Delete object")
    }
    pub(crate) fn take_delete_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .delete_key
            .take()
            .expect("ReverseStep Delete key")
    }
}
const _: () = assert!(std::mem::size_of::<ReverseStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ReverseStep>() <= 64);
