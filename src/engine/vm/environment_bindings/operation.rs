//! Object-environment lookup owns presence checks across unscopables and RHS callbacks.
use crate::engine::{
    api::{ErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, WellKnownSymbol, operations::InternalSetResult},
    value::{JsValue, Value, conversion::NativeConversion},
    vm::Completion,
};

pub(in crate::engine::vm) enum EnvironmentStep {
    Complete(Completion),
    Has { resume: EnvironmentResume },
    Read { resume: EnvironmentResume },
    Set { resume: EnvironmentResume },
    Delete { resume: EnvironmentResume },
}
pub(in crate::engine::vm) struct EnvironmentResume(Box<EnvironmentResumeState>);
impl std::ops::Deref for EnvironmentResume {
    type Target = EnvironmentResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for EnvironmentResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<EnvironmentResume>() <= 8);
pub(in crate::engine::vm) struct EnvironmentResumeState {
    pending_effect: EnvironmentStepPending,
    realm: ContextId,
    phase: Phase,
}
enum Phase {
    Binding {
        object: ObjectRef,
        key: PropertyKey,
        with: bool,
    },
    Unscopables {
        key: PropertyKey,
    },
    Excluded,
    Get {
        object: ObjectRef,
        key: PropertyKey,
        strict: bool,
    },
    Put {
        object: ObjectRef,
        key: PropertyKey,
        value: JsValue,
        strict: bool,
        reference: bool,
    },
    Reference {
        object: ObjectRef,
    },
    DeleteGlobal {
        object: ObjectRef,
        key: PropertyKey,
    },
    Set {
        key: PropertyKey,
        strict: bool,
    },
    Value,
    Boolean,
}
impl EnvironmentStep {
    pub(in crate::engine::vm) fn read(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
        receiver: JsValue,
    ) -> Self {
        Self::request_read(
            runtime,
            receiver,
            object,
            key,
            EnvironmentResume(Box::new(EnvironmentResumeState {
                pending_effect: EnvironmentStepPending::default(),
                realm,
                phase: Phase::Value,
            })),
        )
    }
    pub(in crate::engine::vm) fn has_binding(
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
        with: bool,
    ) -> Self {
        Self::request_has(
            object.clone(),
            key.clone(),
            EnvironmentResume(Box::new(EnvironmentResumeState {
                pending_effect: EnvironmentStepPending::default(),
                realm,
                phase: Phase::Binding { object, key, with },
            })),
        )
    }
    pub(in crate::engine::vm) fn get(
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
        strict: bool,
    ) -> Self {
        Self::request_has(
            object.clone(),
            key.clone(),
            EnvironmentResume(Box::new(EnvironmentResumeState {
                pending_effect: EnvironmentStepPending::default(),
                realm,
                phase: Phase::Get {
                    object,
                    key,
                    strict,
                },
            })),
        )
    }
    pub(in crate::engine::vm) fn put(
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
        value: JsValue,
        strict: bool,
        reference: bool,
    ) -> Self {
        Self::request_has(
            object.clone(),
            key.clone(),
            EnvironmentResume(Box::new(EnvironmentResumeState {
                pending_effect: EnvironmentStepPending::default(),
                realm,
                phase: Phase::Put {
                    object,
                    key,
                    value,
                    strict,
                    reference,
                },
            })),
        )
    }
    pub(in crate::engine::vm) fn set(
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
        value: JsValue,
        strict: bool,
    ) -> Self {
        Self::request_set(
            object,
            key.clone(),
            value,
            EnvironmentResume(Box::new(EnvironmentResumeState {
                pending_effect: EnvironmentStepPending::default(),
                realm,
                phase: Phase::Set { key, strict },
            })),
        )
    }
    pub(in crate::engine::vm) fn reference(
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
    ) -> Self {
        Self::request_has(
            object.clone(),
            key,
            EnvironmentResume(Box::new(EnvironmentResumeState {
                pending_effect: EnvironmentStepPending::default(),
                realm,
                phase: Phase::Reference { object },
            })),
        )
    }
    pub(in crate::engine::vm) fn delete_global(
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
    ) -> Self {
        Self::request_has(
            object.clone(),
            key.clone(),
            EnvironmentResume(Box::new(EnvironmentResumeState {
                pending_effect: EnvironmentStepPending::default(),
                realm,
                phase: Phase::DeleteGlobal { object, key },
            })),
        )
    }
    pub(in crate::engine::vm) fn delete(
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
    ) -> Self {
        Self::request_delete(
            object,
            key,
            EnvironmentResume(Box::new(EnvironmentResumeState {
                pending_effect: EnvironmentStepPending::default(),
                realm,
                phase: Phase::Boolean,
            })),
        )
    }
}
impl EnvironmentResume {
    pub(in crate::engine::vm) fn boolean(
        self,
        runtime: &Runtime,
        reply: NativeConversion<bool>,
    ) -> Result<EnvironmentStep, RuntimeError> {
        let present = match reply {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(EnvironmentStep::Complete(Completion::Throw(value)));
            }
        };
        let realm = self.0.realm;
        match self.0.phase {
            Phase::Binding { object, key, with } => {
                if !present || !with {
                    return Ok(EnvironmentStep::Complete(Completion::Return(
                        JsValue::Bool(present),
                    )));
                }
                Ok(EnvironmentStep::request_read(
                    runtime,
                    {
                        let id = object.object_id();
                        runtime.retain_object_handle(id)?;
                        JsValue::Object(id)
                    },
                    object,
                    PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Unscopables)),
                    EnvironmentResume(Box::new(EnvironmentResumeState {
                        pending_effect: EnvironmentStepPending::default(),
                        realm,
                        phase: Phase::Unscopables { key },
                    })),
                ))
            }
            Phase::Get {
                object,
                key,
                strict,
            } => {
                if !present {
                    if strict {
                        return Err(runtime
                            .native_atom_error(ErrorKind::Reference, "'", &key, "' is not defined")?
                            .into());
                    }
                    return Ok(EnvironmentStep::Complete(Completion::Return(
                        JsValue::Undefined,
                    )));
                }
                Ok(EnvironmentStep::request_read(
                    runtime,
                    {
                        let id = object.object_id();
                        runtime.retain_object_handle(id)?;
                        JsValue::Object(id)
                    },
                    object,
                    key,
                    EnvironmentResume(Box::new(EnvironmentResumeState {
                        pending_effect: EnvironmentStepPending::default(),
                        realm,
                        phase: Phase::Value,
                    })),
                ))
            }
            Phase::Put {
                object,
                key,
                value,
                strict,
                reference,
            } => {
                if !present && strict {
                    return Err(runtime
                        .native_atom_error(ErrorKind::Reference, "'", &key, "' is not defined")?
                        .into());
                }
                if reference && let Some(root) = runtime.own_var_ref_root(&object, &key)? {
                    let cell = runtime.0.state.borrow().heap.var_ref(root.id())?.clone();
                    if matches!(cell.value, crate::engine::heap::RawValue::Uninitialized) {
                        return Err(super::super::bindings::lexical_uninitialized_error(
                            runtime,
                            Some(key.atom()),
                            true,
                        )?
                        .into());
                    }
                    if cell.is_const && strict {
                        return Err(super::super::bindings::lexical_read_only_error(
                            runtime,
                            Some(key.atom()),
                        )?
                        .into());
                    }
                    if !cell.is_const {
                        runtime.write_var_ref(&root, value)?;
                    }
                    return Ok(EnvironmentStep::Complete(Completion::Return(
                        JsValue::Undefined,
                    )));
                }
                Ok(EnvironmentStep::set(realm, object, key, value, strict))
            }
            Phase::Reference { object } => {
                Ok(EnvironmentStep::Complete(Completion::Return(if present {
                    JsValue::Object(object.object_id())
                } else {
                    JsValue::Undefined
                })))
            }
            Phase::DeleteGlobal { object, key } => Ok(if present {
                EnvironmentStep::delete(realm, object, key)
            } else {
                EnvironmentStep::Complete(Completion::Return(JsValue::Bool(true)))
            }),
            Phase::Boolean => Ok(EnvironmentStep::Complete(Completion::Return(JsValue::Bool(
                present,
            )))),
            _ => Err(RuntimeError::Invariant(
                "environment Boolean reply has wrong phase",
            )),
        }
    }
    pub(in crate::engine::vm) fn resume(
        self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<EnvironmentStep, RuntimeError> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(EnvironmentStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Unscopables { key } => Ok(match value {
                JsValue::Object(object) => EnvironmentStep::request_read(
                    runtime,
                    JsValue::Object(object),
                    ObjectRef::from_borrowed_handle(runtime.clone(), object)?,
                    key,
                    EnvironmentResume(Box::new(EnvironmentResumeState {
                        pending_effect: EnvironmentStepPending::default(),
                        realm: self.0.realm,
                        phase: Phase::Excluded,
                    })),
                ),
                _ => EnvironmentStep::Complete(Completion::Return(JsValue::Bool(true))),
            }),
            Phase::Excluded => Ok(EnvironmentStep::Complete(Completion::Return(JsValue::Bool(
                !runtime.value_to_boolean_jsvalue(&value)?,
            )))),
            Phase::Value => Ok(EnvironmentStep::Complete(Completion::Return(value))),
            _ => Err(RuntimeError::Invariant(
                "environment value reply has wrong phase",
            )),
        }
    }
    pub(in crate::engine::vm) fn set(
        self,
        runtime: &Runtime,
        reply: NativeConversion<InternalSetResult>,
    ) -> Result<EnvironmentStep, RuntimeError> {
        let Phase::Set { key, strict } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "environment Set reply has wrong phase",
            ));
        };
        Ok(EnvironmentStep::Complete(
            runtime.finish_property_set(reply, &key, strict)?,
        ))
    }
}

#[derive(Default)]
struct EnvironmentStepPending {
    has_object: Option<ObjectRef>,
    has_key: Option<PropertyKey>,
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    read_receiver: Option<JsValue>,
    set_object: Option<ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<JsValue>,
    delete_object: Option<ObjectRef>,
    delete_key: Option<PropertyKey>,
}
impl EnvironmentStep {
    pub(crate) fn request_has(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: EnvironmentResume,
    ) -> Self {
        resume.0.pending_effect.has_object = Some(object);
        resume.0.pending_effect.has_key = Some(key);
        Self::Has { resume }
    }
    pub(crate) fn request_read(
        _runtime: &Runtime,
        receiver: JsValue,
        object: ObjectRef,
        key: PropertyKey,
        mut resume: EnvironmentResume,
    ) -> Self {
        // The receiver carries an owned edge: either the caller retained it
        // above, or the completion value already owned its edge.
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        resume.0.pending_effect.read_receiver = Some(receiver);
        Self::Read { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        key: PropertyKey,
        value: JsValue,
        mut resume: EnvironmentResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        Self::Set { resume }
    }
    pub(crate) fn request_delete(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: EnvironmentResume,
    ) -> Self {
        resume.0.pending_effect.delete_object = Some(object);
        resume.0.pending_effect.delete_key = Some(key);
        Self::Delete { resume }
    }
}
impl EnvironmentResume {
    pub(crate) fn take_has_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .has_object
            .take()
            .expect("EnvironmentStep Has object")
    }
    pub(crate) fn take_has_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .has_key
            .take()
            .expect("EnvironmentStep Has key")
    }
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("EnvironmentStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("EnvironmentStep Read key")
    }
    pub(crate) fn take_read_receiver(&mut self) -> JsValue {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("EnvironmentStep Read receiver")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("EnvironmentStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("EnvironmentStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> JsValue {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("EnvironmentStep Set value")
    }
    pub(crate) fn take_delete_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .delete_object
            .take()
            .expect("EnvironmentStep Delete object")
    }
    pub(crate) fn take_delete_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .delete_key
            .take()
            .expect("EnvironmentStep Delete key")
    }
}
const _: () = assert!(std::mem::size_of::<EnvironmentStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<EnvironmentStep>() <= 64);
