//! Object.create/defineProperties: snapshot enumerable keys, then convert and publish each.
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{
        ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, operations::InternalDefineResult,
    },
    value::{Value, conversion::NativeConversion},
    vm::{Completion, call::NativeArguments},
};
#[derive(Clone, Copy)]
pub(crate) enum DefinitionsKind {
    Create,
    Define,
}
impl DefinitionsKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::ObjectCreate => Self::Create,
            NativeFunctionId::ObjectDefineProperties => Self::Define,
            _ => return None,
        })
    }
}
pub(crate) enum DefinitionsStep {
    Complete(Completion),
    Keys { resume: DefinitionsResume },
    Enumerable { resume: DefinitionsResume },
    Read { resume: DefinitionsResume },
    Convert { resume: DefinitionsResume },
    Define { resume: DefinitionsResume },
}
pub(crate) struct DefinitionsResume(Box<DefinitionsResumeState>);
impl std::ops::Deref for DefinitionsResume {
    type Target = DefinitionsResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for DefinitionsResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<DefinitionsResume>() <= 8);
pub(crate) struct DefinitionsResumeState {
    pending_effect: DefinitionsStepPending,
    realm: ContextId,
    target: ObjectRef,
    source: ObjectRef,
    phase: Phase,
}
enum Phase {
    Keys,
    Snapshot {
        remaining: std::vec::IntoIter<PropertyKey>,
        selected: Vec<PropertyKey>,
        key: PropertyKey,
    },
    Read {
        remaining: std::vec::IntoIter<PropertyKey>,
        key: PropertyKey,
    },
    Convert {
        remaining: std::vec::IntoIter<PropertyKey>,
        key: PropertyKey,
    },
    Define {
        remaining: std::vec::IntoIter<PropertyKey>,
        key: PropertyKey,
    },
}
impl DefinitionsStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: DefinitionsKind,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Object definitions target argv was not padded",
        ))?;
        let target = match kind {
            DefinitionsKind::Create => match value {
                Value::Object(prototype) => runtime.new_object(Some(prototype))?,
                Value::Null => runtime.new_object(None)?,
                _ => {
                    return Ok(Self::Complete(Completion::Throw(
                        runtime.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "not a prototype",
                        )?,
                    )));
                }
            },
            DefinitionsKind::Define => match value {
                Value::Object(object) => object.clone(),
                _ => {
                    return Ok(Self::Complete(Completion::Throw(
                        runtime.new_native_error(realm, NativeErrorKind::Type, "not an object")?,
                    )));
                }
            },
        };
        let value = arguments
            .readable
            .get(1)
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object definitions properties argv was not padded",
            ))?;
        if matches!(kind, DefinitionsKind::Create) && matches!(value, Value::Undefined) {
            return Ok(Self::Complete(Completion::Return(Value::Object(target))));
        }
        let source = match runtime.native_to_object(realm, value)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        Ok(Self::request_keys(
            source.clone(),
            DefinitionsResume(Box::new(DefinitionsResumeState {
                pending_effect: DefinitionsStepPending::default(),
                realm,
                target,
                source,
                phase: Phase::Keys,
            })),
        ))
    }
}
impl DefinitionsResume {
    pub(crate) fn keys(
        self,
        runtime: &Runtime,
        result: NativeConversion<Vec<PropertyKey>>,
    ) -> Result<DefinitionsStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Keys) {
            return Err(RuntimeError::Invariant(
                "Object definitions received unexpected keys",
            ));
        }
        let keys = match result {
            NativeConversion::Value(keys) => keys,
            NativeConversion::Throw(value) => {
                return Ok(DefinitionsStep::Complete(Completion::Throw(value)));
            }
        };
        let mut selected = Vec::new();
        selected.try_reserve_exact(keys.len()).map_err(|_| {
            RuntimeError::Invariant("Object definitions key snapshot allocation failed")
        })?;
        self.snapshot(runtime, keys.into_iter(), selected)
    }
    fn snapshot(
        mut self,
        runtime: &Runtime,
        mut remaining: std::vec::IntoIter<PropertyKey>,
        selected: Vec<PropertyKey>,
    ) -> Result<DefinitionsStep, RuntimeError> {
        let Some(key) = remaining.next() else {
            return self.next(runtime, selected.into_iter());
        };
        Ok(DefinitionsStep::request_enumerable(
            self.0.source.clone(),
            key.clone(),
            {
                let updated_0 = Phase::Snapshot {
                    remaining,
                    selected,
                    key,
                };
                self.0.phase = updated_0;
                self
            },
        ))
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<DefinitionsStep, RuntimeError> {
        let Phase::Snapshot {
            remaining,
            mut selected,
            key,
        } = self.0.phase
        else {
            return Err(RuntimeError::Invariant(
                "Object definitions received unexpected enumerable reply",
            ));
        };
        match result {
            NativeConversion::Throw(value) => {
                return Ok(DefinitionsStep::Complete(Completion::Throw(value)));
            }
            NativeConversion::Value(true) => selected.push(key),
            NativeConversion::Value(false) => {}
        }
        {
            let updated_0 = Phase::Keys;
            self.0.phase = updated_0;
            self
        }
        .snapshot(runtime, remaining, selected)
    }
    fn next(
        mut self,
        _runtime: &Runtime,
        mut remaining: std::vec::IntoIter<PropertyKey>,
    ) -> Result<DefinitionsStep, RuntimeError> {
        let Some(key) = remaining.next() else {
            return Ok(DefinitionsStep::Complete(Completion::Return(
                Value::Object(self.0.target),
            )));
        };
        Ok(DefinitionsStep::request_read(
            self.0.source.clone(),
            key.clone(),
            {
                let updated_0 = Phase::Read { remaining, key };
                self.0.phase = updated_0;
                self
            },
        ))
    }
    pub(crate) fn read(mut self, result: Completion) -> Result<DefinitionsStep, RuntimeError> {
        let Phase::Read { remaining, key } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "Object definitions received unexpected value reply",
            ));
        };
        Ok(match result {
            Completion::Throw(value) => DefinitionsStep::Complete(Completion::Throw(value)),
            Completion::Return(value) => DefinitionsStep::request_convert(value, {
                let updated_0 = Phase::Convert { remaining, key };
                self.0.phase = updated_0;
                self
            }),
        })
    }
    pub(crate) fn converted(
        mut self,
        result: NativeConversion<OrdinaryPropertyDescriptor>,
    ) -> Result<DefinitionsStep, RuntimeError> {
        let Phase::Convert { remaining, key } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "Object definitions received unexpected conversion reply",
            ));
        };
        Ok(match result {
            NativeConversion::Throw(value) => DefinitionsStep::Complete(Completion::Throw(value)),
            NativeConversion::Value(descriptor) => {
                DefinitionsStep::request_define(self.0.target.clone(), key.clone(), descriptor, {
                    let updated_0 = Phase::Define { remaining, key };
                    self.0.phase = updated_0;
                    self
                })
            }
        })
    }
    pub(crate) fn defined(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<InternalDefineResult>,
    ) -> Result<DefinitionsStep, RuntimeError> {
        let Phase::Define { remaining, key } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "Object definitions received unexpected definition reply",
            ));
        };
        if let Some(value) = runtime.finish_define_property_or_throw(self.0.realm, &key, result)? {
            return Ok(DefinitionsStep::Complete(Completion::Throw(value)));
        }
        {
            let updated_0 = Phase::Keys;
            self.0.phase = updated_0;
            self
        }
        .next(runtime, remaining)
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: DefinitionsStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            DefinitionsStep::Complete(result) => return Ok(result),
            DefinitionsStep::Keys { mut resume } => {
                let object = resume.take_keys_object();
                resume.keys(runtime, runtime.internal_own_property_keys(realm, &object)?)?
            }
            DefinitionsStep::Enumerable { mut resume } => {
                let object = resume.take_enumerable_object();
                let key = resume.take_enumerable_key();
                resume.boolean(
                    runtime,
                    runtime.internal_snapshot_own_property_is_enumerable(realm, &object, &key)?,
                )?
            }
            DefinitionsStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.read(runtime.get_property_in_realm(realm, &object, &key)?)?
            }
            DefinitionsStep::Convert { mut resume } => {
                let value = resume.take_convert_value();
                resume.converted(runtime.native_to_property_descriptor(realm, value)?)?
            }
            DefinitionsStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                resume.defined(
                    runtime,
                    runtime.internal_define_own_property(realm, &object, &key, &descriptor)?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct DefinitionsStepPending {
    keys_object: Option<ObjectRef>,
    enumerable_object: Option<ObjectRef>,
    enumerable_key: Option<PropertyKey>,
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    convert_value: Option<Value>,
    define_object: Option<ObjectRef>,
    define_key: Option<PropertyKey>,
    define_descriptor: Option<OrdinaryPropertyDescriptor>,
}
impl DefinitionsStep {
    pub(crate) fn request_keys(object: ObjectRef, mut resume: DefinitionsResume) -> Self {
        resume.0.pending_effect.keys_object = Some(object);
        Self::Keys { resume }
    }
    pub(crate) fn request_enumerable(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: DefinitionsResume,
    ) -> Self {
        resume.0.pending_effect.enumerable_object = Some(object);
        resume.0.pending_effect.enumerable_key = Some(key);
        Self::Enumerable { resume }
    }
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: DefinitionsResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_convert(value: Value, mut resume: DefinitionsResume) -> Self {
        resume.0.pending_effect.convert_value = Some(value);
        Self::Convert { resume }
    }
    pub(crate) fn request_define(
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
        mut resume: DefinitionsResume,
    ) -> Self {
        resume.0.pending_effect.define_object = Some(object);
        resume.0.pending_effect.define_key = Some(key);
        resume.0.pending_effect.define_descriptor = Some(descriptor);
        Self::Define { resume }
    }
}
impl DefinitionsResume {
    pub(crate) fn take_keys_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .keys_object
            .take()
            .expect("DefinitionsStep Keys object")
    }
    pub(crate) fn take_enumerable_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .enumerable_object
            .take()
            .expect("DefinitionsStep Enumerable object")
    }
    pub(crate) fn take_enumerable_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .enumerable_key
            .take()
            .expect("DefinitionsStep Enumerable key")
    }
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("DefinitionsStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("DefinitionsStep Read key")
    }
    pub(crate) fn take_convert_value(&mut self) -> Value {
        self.0
            .pending_effect
            .convert_value
            .take()
            .expect("DefinitionsStep Convert value")
    }
    pub(crate) fn take_define_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .define_object
            .take()
            .expect("DefinitionsStep Define object")
    }
    pub(crate) fn take_define_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .define_key
            .take()
            .expect("DefinitionsStep Define key")
    }
    pub(crate) fn take_define_descriptor(&mut self) -> OrdinaryPropertyDescriptor {
        self.0
            .pending_effect
            .define_descriptor
            .take()
            .expect("DefinitionsStep Define descriptor")
    }
}
const _: () = assert!(std::mem::size_of::<DefinitionsStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<DefinitionsStep>() <= 64);
