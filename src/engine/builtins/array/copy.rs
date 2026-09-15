//! One directional Has/Get/Set-or-Delete range algorithm for Array mutations.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, operations::InternalSetResult},
    value::{Value, conversion::NativeConversion},
    vm::Completion,
};
pub(crate) enum CopyStep {
    Complete(Completion),
    Has { resume: CopyResume },
    Read { resume: CopyResume },
    Set { resume: CopyResume },
    Delete { resume: CopyResume },
}
enum Phase {
    Has,
    Read,
    Write,
}
pub(crate) struct CopyResume(Box<CopyResumeState>);
impl std::ops::Deref for CopyResume {
    type Target = CopyResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for CopyResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<CopyResume>() <= 8);
pub(crate) struct CopyResumeState {
    pending_effect: CopyStepPending,
    scheduler_set_key: Option<PropertyKey>,
    realm: ContextId,
    object: ObjectRef,
    to: u64,
    from: u64,
    count: u64,
    backwards: bool,
    offset: u64,
    phase: Phase,
}
impl CopyStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
        to: u64,
        from: u64,
        count: u64,
        backwards: bool,
    ) -> Result<Self, RuntimeError> {
        CopyResume(Box::new(CopyResumeState {
            pending_effect: CopyStepPending::default(),
            scheduler_set_key: None,
            realm,
            object,
            to,
            from,
            count,
            backwards,
            offset: 0,
            phase: Phase::Has,
        }))
        .next(runtime)
    }
}
impl CopyResume {
    pub(crate) fn with_scheduler_set_key(mut self, key: PropertyKey) -> Self {
        self.0.scheduler_set_key = Some(key);
        self
    }
    pub(crate) fn take_scheduler_set_key(&mut self) -> PropertyKey {
        self.0.scheduler_set_key.take().expect("waiting Set key")
    }

    fn relative(&self) -> u64 {
        if self.0.backwards {
            self.0.count - self.0.offset - 1
        } else {
            self.0.offset
        }
    }
    fn to_key(&self, runtime: &Runtime) -> Result<PropertyKey, RuntimeError> {
        Ok(
            runtime.property_key_for_index(self.0.to.checked_add(self.relative()).ok_or(
                RuntimeError::Invariant("Array copy target index overflowed"),
            )?)?,
        )
    }
    fn from_key(&self, runtime: &Runtime) -> Result<PropertyKey, RuntimeError> {
        Ok(
            runtime.property_key_for_index(self.0.from.checked_add(self.relative()).ok_or(
                RuntimeError::Invariant("Array copy source index overflowed"),
            )?)?,
        )
    }
    fn next(mut self, runtime: &Runtime) -> Result<CopyStep, RuntimeError> {
        if self.0.offset == self.0.count {
            return Ok(CopyStep::Complete(Completion::Return(Value::Undefined)));
        }
        self.0.phase = Phase::Has;
        Ok(CopyStep::request_has(
            self.0.object.clone(),
            self.from_key(runtime)?,
            self,
        ))
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<CopyStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(CopyStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Has if value => {
                self.0.phase = Phase::Read;
                Ok(CopyStep::request_read(
                    self.0.object.clone(),
                    self.from_key(runtime)?,
                    self,
                ))
            }
            Phase::Has => {
                self.0.phase = Phase::Write;
                Ok(CopyStep::request_delete(
                    self.0.object.clone(),
                    self.to_key(runtime)?,
                    self,
                ))
            }
            Phase::Write => {
                if !value {
                    return Ok(CopyStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "could not delete property",
                        )?,
                    )));
                }
                self.0.offset += 1;
                self.next(runtime)
            }
            _ => Err(RuntimeError::Invariant("Array copy boolean phase mismatch")),
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<CopyStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Read) {
            return Err(RuntimeError::Invariant("Array copy value phase mismatch"));
        }
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(CopyStep::Complete(Completion::Throw(value))),
        };
        self.0.phase = Phase::Write;
        Ok(CopyStep::request_set(
            self.0.object.clone(),
            self.to_key(runtime)?,
            value,
            self,
        ))
    }
    pub(crate) fn set(
        mut self,
        runtime: &Runtime,
        key: PropertyKey,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<CopyStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Write) {
            return Err(RuntimeError::Invariant("Array copy set phase mismatch"));
        }
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            return Ok(CopyStep::Complete(Completion::Throw(value)));
        }
        self.0.offset += 1;
        self.next(runtime)
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: CopyStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            CopyStep::Complete(result) => return Ok(result),
            CopyStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            CopyStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                resume.boolean(
                    runtime,
                    runtime.internal_has_property(realm, &object, &key)?,
                )?
            }
            CopyStep::Set { mut resume } => {
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
            CopyStep::Delete { mut resume } => {
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
struct CopyStepPending {
    has_object: Option<ObjectRef>,
    has_key: Option<PropertyKey>,
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    set_object: Option<ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<Value>,
    delete_object: Option<ObjectRef>,
    delete_key: Option<PropertyKey>,
}
impl CopyStep {
    pub(crate) fn request_has(object: ObjectRef, key: PropertyKey, mut resume: CopyResume) -> Self {
        resume.0.pending_effect.has_object = Some(object);
        resume.0.pending_effect.has_key = Some(key);
        Self::Has { resume }
    }
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: CopyResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: CopyResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        Self::Set { resume }
    }
    pub(crate) fn request_delete(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: CopyResume,
    ) -> Self {
        resume.0.pending_effect.delete_object = Some(object);
        resume.0.pending_effect.delete_key = Some(key);
        Self::Delete { resume }
    }
}
impl CopyResume {
    pub(crate) fn take_has_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .has_object
            .take()
            .expect("CopyStep Has object")
    }
    pub(crate) fn take_has_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .has_key
            .take()
            .expect("CopyStep Has key")
    }
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("CopyStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("CopyStep Read key")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("CopyStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("CopyStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("CopyStep Set value")
    }
    pub(crate) fn take_delete_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .delete_object
            .take()
            .expect("CopyStep Delete object")
    }
    pub(crate) fn take_delete_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .delete_key
            .take()
            .expect("CopyStep Delete key")
    }
}
const _: () = assert!(std::mem::size_of::<CopyStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<CopyStep>() <= 64);
