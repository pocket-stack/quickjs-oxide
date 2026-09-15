//! Completion-aware Proxy boolean internal methods and their distinct invariants.
use super::{
    RootedProxy,
    method::{MethodResume, MethodStep},
};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::ContextId;
use crate::engine::object::{CompleteOrdinaryPropertyDescriptor, ObjectRef, PropertyKey};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{Completion, call::DirectCallTarget};

pub(crate) enum ProxyBooleanKind {
    Has(PropertyKey),
    Delete(PropertyKey),
    Extensible,
    PreventExtensions,
}
pub(crate) enum ProxyBooleanStep {
    Delete { resume: ProxyBooleanResume },
    PreventExtensions { resume: ProxyBooleanResume },
    Complete(NativeConversion<bool>),
    Read { resume: ProxyBooleanResume },
    Call { resume: ProxyBooleanResume },
    Has { resume: ProxyBooleanResume },
    Extensible { resume: ProxyBooleanResume },
    Descriptor { resume: ProxyBooleanResume },
}
pub(crate) struct ProxyBooleanResume(Box<ProxyBooleanResumeState>);
impl std::ops::Deref for ProxyBooleanResume {
    type Target = ProxyBooleanResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ProxyBooleanResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ProxyBooleanResume>() <= 8);
pub(crate) struct ProxyBooleanResumeState {
    pending_effect: ProxyBooleanStepPending,
    realm: ContextId,
    phase: Phase,
}
enum Phase {
    Method {
        resume: MethodResume,
        kind: ProxyBooleanKind,
    },
    Forward {
        _rooted: RootedProxy,
        _key: Option<PropertyKey>,
    },
    Trap {
        rooted: RootedProxy,
        kind: ProxyBooleanKind,
    },
    DeleteInvariant {
        rooted: RootedProxy,
        key: PropertyKey,
    },
    RequiredExtensibility {
        _rooted: RootedProxy,
        _key: Option<PropertyKey>,
        name: &'static str,
        expected: bool,
    },
    HasInvariant {
        rooted: RootedProxy,
        key: PropertyKey,
    },
    ExtensibleInvariant {
        _rooted: RootedProxy,
        result: bool,
    },
}

impl ProxyBooleanStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
        kind: ProxyBooleanKind,
    ) -> Result<Self, RuntimeError> {
        let name = match &kind {
            ProxyBooleanKind::Has(key) => {
                runtime.validate_object_and_key(&object, key)?;
                "has"
            }
            ProxyBooleanKind::Delete(key) => {
                runtime.validate_object_and_key(&object, key)?;
                "deleteProperty"
            }
            ProxyBooleanKind::Extensible => "isExtensible",
            ProxyBooleanKind::PreventExtensions => "preventExtensions",
        };
        let step = MethodStep::start(runtime, realm, object, name)?;
        method(runtime, realm, kind, step)
    }
}

fn method(
    runtime: &Runtime,
    realm: ContextId,
    kind: ProxyBooleanKind,
    step: MethodStep,
) -> Result<ProxyBooleanStep, RuntimeError> {
    Ok(match step {
        MethodStep::Throw(value) => ProxyBooleanStep::Complete(NativeConversion::Throw(value)),
        MethodStep::Read { mut resume } => {
            let object = resume.take_read_object();
            let key = resume.take_read_key();
            let receiver = resume.take_read_receiver();
            ProxyBooleanStep::request_read(
                object,
                key,
                receiver,
                ProxyBooleanResume(Box::new(ProxyBooleanResumeState {
                    pending_effect: ProxyBooleanStepPending::default(),
                    realm,
                    phase: Phase::Method { resume, kind },
                })),
            )
        }
        MethodStep::Complete { mut resume } => {
            let rooted = resume.take_completed_rooted();
            let target = resume.take_completed_target();
            drop(resume);
            match target {
                None => {
                    let object = rooted.target.clone();
                    let resume = ProxyBooleanResume(Box::new(ProxyBooleanResumeState {
                        pending_effect: ProxyBooleanStepPending::default(),
                        realm,
                        phase: Phase::Forward {
                            _rooted: rooted,
                            _key: match &kind {
                                ProxyBooleanKind::Has(key) | ProxyBooleanKind::Delete(key) => {
                                    Some(key.clone())
                                }
                                _ => None,
                            },
                        },
                    }));
                    match kind {
                        ProxyBooleanKind::Has(key) => {
                            ProxyBooleanStep::request_has(object, key, resume)
                        }
                        ProxyBooleanKind::Extensible => {
                            ProxyBooleanStep::request_extensible(object, resume)
                        }
                        ProxyBooleanKind::Delete(key) => {
                            ProxyBooleanStep::request_delete(object, key, resume)
                        }
                        ProxyBooleanKind::PreventExtensions => {
                            ProxyBooleanStep::request_prevent_extensions(object, resume)
                        }
                    }
                }
                Some(target) => {
                    let mut arguments = vec![Value::Object(rooted.target.clone())];
                    if let ProxyBooleanKind::Has(key) | ProxyBooleanKind::Delete(key) = &kind {
                        arguments.push(runtime.property_key_value(key)?);
                    }
                    ProxyBooleanStep::request_call(
                        target,
                        Value::Object(rooted.handler.clone()),
                        arguments,
                        ProxyBooleanResume(Box::new(ProxyBooleanResumeState {
                            pending_effect: ProxyBooleanStepPending::default(),
                            realm,
                            phase: Phase::Trap { rooted, kind },
                        })),
                    )
                }
            }
        }
    })
}

impl ProxyBooleanResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<ProxyBooleanStep, RuntimeError> {
        let value = match completion {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(ProxyBooleanStep::Complete(NativeConversion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Method { resume, kind } => method(
                runtime,
                self.0.realm,
                kind,
                resume.resume(runtime, Completion::Return(value))?,
            ),
            Phase::Trap { rooted, kind } => {
                let result = runtime.value_to_boolean(&value)?;
                match kind {
                    ProxyBooleanKind::Has(_) if result => {
                        Ok(ProxyBooleanStep::Complete(NativeConversion::Value(true)))
                    }
                    ProxyBooleanKind::Has(key) => Ok(ProxyBooleanStep::request_descriptor(
                        rooted.target.clone(),
                        key.clone(),
                        Self(Box::new(ProxyBooleanResumeState {
                            pending_effect: ProxyBooleanStepPending::default(),
                            realm: self.0.realm,
                            phase: Phase::HasInvariant { rooted, key },
                        })),
                    )),
                    ProxyBooleanKind::Delete(_) | ProxyBooleanKind::PreventExtensions
                        if !result =>
                    {
                        Ok(ProxyBooleanStep::Complete(NativeConversion::Value(false)))
                    }
                    ProxyBooleanKind::Delete(key) => Ok(ProxyBooleanStep::request_descriptor(
                        rooted.target.clone(),
                        key.clone(),
                        Self(Box::new(ProxyBooleanResumeState {
                            pending_effect: ProxyBooleanStepPending::default(),
                            realm: self.0.realm,
                            phase: Phase::DeleteInvariant { rooted, key },
                        })),
                    )),
                    ProxyBooleanKind::PreventExtensions => {
                        Ok(ProxyBooleanStep::request_extensible(
                            rooted.target.clone(),
                            Self(Box::new(ProxyBooleanResumeState {
                                pending_effect: ProxyBooleanStepPending::default(),
                                realm: self.0.realm,
                                phase: Phase::RequiredExtensibility {
                                    _rooted: rooted,
                                    name: "preventExtensions",
                                    expected: false,
                                    _key: None,
                                },
                            })),
                        ))
                    }
                    ProxyBooleanKind::Extensible => Ok(ProxyBooleanStep::request_extensible(
                        rooted.target.clone(),
                        Self(Box::new(ProxyBooleanResumeState {
                            pending_effect: ProxyBooleanStepPending::default(),
                            realm: self.0.realm,
                            phase: Phase::ExtensibleInvariant {
                                _rooted: rooted,
                                result,
                            },
                        })),
                    )),
                }
            }
            _ => Err(RuntimeError::Invariant(
                "Proxy boolean continuation received a value reply",
            )),
        }
    }

    pub(crate) fn boolean(
        self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<ProxyBooleanStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ProxyBooleanStep::Complete(NativeConversion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Forward { .. } => Ok(ProxyBooleanStep::Complete(NativeConversion::Value(value))),
            Phase::RequiredExtensibility {
                _rooted,
                name,
                expected,
                _key,
            } => Ok(ProxyBooleanStep::Complete(if value != expected {
                runtime.proxy_invariant_throw(self.0.realm, name)?
            } else {
                NativeConversion::Value(true)
            })),
            Phase::ExtensibleInvariant { _rooted, result } => {
                if result != value {
                    return Ok(ProxyBooleanStep::Complete(
                        runtime.proxy_invariant_throw(self.0.realm, "isExtensible")?,
                    ));
                }
                Ok(ProxyBooleanStep::Complete(NativeConversion::Value(result)))
            }
            _ => Err(RuntimeError::Invariant(
                "Proxy value continuation received a boolean reply",
            )),
        }
    }

    pub(crate) fn descriptor(
        self,
        runtime: &Runtime,
        descriptor: NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>,
    ) -> Result<ProxyBooleanStep, RuntimeError> {
        let (rooted, key, deleting) = match self.0.phase {
            Phase::HasInvariant { rooted, key } => (rooted, key, false),
            Phase::DeleteInvariant { rooted, key } => (rooted, key, true),
            _ => {
                return Err(RuntimeError::Invariant(
                    "Proxy boolean continuation received a descriptor reply",
                ));
            }
        };
        let descriptor = match descriptor {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ProxyBooleanStep::Complete(NativeConversion::Throw(value)));
            }
        };
        if deleting {
            let Some(target) = descriptor else {
                return Ok(ProxyBooleanStep::Complete(NativeConversion::Value(true)));
            };
            if !target.configurable() {
                return Ok(ProxyBooleanStep::Complete(
                    runtime.proxy_invariant_throw(self.0.realm, "deleteProperty")?,
                ));
            }
            // Delete consults nested [[IsExtensible]], unlike Has's pinned raw bit.
            return Ok(ProxyBooleanStep::request_extensible(
                rooted.target.clone(),
                Self(Box::new(ProxyBooleanResumeState {
                    pending_effect: ProxyBooleanStepPending::default(),
                    realm: self.0.realm,
                    phase: Phase::RequiredExtensibility {
                        _rooted: rooted,
                        name: "deleteProperty",
                        expected: true,
                        _key: Some(key),
                    },
                })),
            ));
        }
        if let Some(target) = descriptor
            && (!target.configurable() || !runtime.raw_extensible_bit(&rooted.target)?)
        {
            return Ok(ProxyBooleanStep::Complete(
                runtime.proxy_invariant_throw(self.0.realm, "has")?,
            ));
        }
        Ok(ProxyBooleanStep::Complete(NativeConversion::Value(false)))
    }
}

pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ProxyBooleanStep,
) -> Result<NativeConversion<bool>, RuntimeError> {
    loop {
        step = match step {
            ProxyBooleanStep::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                resume.boolean(
                    runtime,
                    runtime.internal_delete_property(realm, &object, &key)?,
                )?
            }
            ProxyBooleanStep::PreventExtensions { mut resume } => {
                let object = resume.take_prevent_extensions_object();
                resume.boolean(
                    runtime,
                    runtime.internal_prevent_extensions(realm, &object)?,
                )?
            }
            ProxyBooleanStep::Complete(result) => return Ok(result),
            ProxyBooleanStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                resume.resume(
                    runtime,
                    runtime.internal_get(realm, &object, &key, receiver)?,
                )?
            }
            ProxyBooleanStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                {
                    let result = match target {
                        DirectCallTarget::Callable(callable) => {
                            runtime.call_internal(realm, &callable, receiver, &arguments)?
                        }
                        DirectCallTarget::NonCallableProxy(object) => {
                            runtime.call_proxy(realm, &object, receiver, &arguments)?
                        }
                    };
                    resume.resume(runtime, result)?
                }
            }
            ProxyBooleanStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                resume.boolean(
                    runtime,
                    runtime.internal_has_property(realm, &object, &key)?,
                )?
            }
            ProxyBooleanStep::Extensible { mut resume } => {
                let object = resume.take_extensible_object();
                resume.boolean(runtime, runtime.internal_is_extensible(realm, &object)?)?
            }
            ProxyBooleanStep::Descriptor { mut resume } => {
                let object = resume.take_descriptor_object();
                let key = resume.take_descriptor_key();
                resume.descriptor(
                    runtime,
                    runtime.internal_get_own_property(realm, &object, &key)?,
                )?
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take_read(step: ProxyBooleanStep) -> ProxyBooleanResume {
        let ProxyBooleanStep::Read { resume, .. } = step else {
            panic!("expected method read")
        };
        resume
    }
    fn take_call(step: ProxyBooleanStep) -> ProxyBooleanResume {
        let ProxyBooleanStep::Call { resume, .. } = step else {
            panic!("expected trap call")
        };
        resume
    }
    fn take_descriptor(step: ProxyBooleanStep) -> ProxyBooleanResume {
        let ProxyBooleanStep::Descriptor { resume, .. } = step else {
            panic!("expected descriptor query")
        };
        resume
    }
    fn take_extensible(step: ProxyBooleanStep) -> ProxyBooleanResume {
        let ProxyBooleanStep::Extensible { resume, .. } = step else {
            panic!("expected extensibility query")
        };
        resume
    }

    #[test]
    fn delete_keeps_symbol_key_across_both_invariant_queries_and_abandonment() {
        for after_descriptor in [false, true] {
            let runtime = Runtime::new();
            let weak = std::rc::Rc::downgrade(&runtime.0);
            let mut context = runtime.new_context();
            let Value::Object(proxy) = context.eval("new Proxy({}, {})").unwrap() else {
                panic!("expected Proxy")
            };
            let callable = context.eval("(function(){return true})").unwrap();
            let symbol = runtime.new_symbol(None).unwrap();
            let key = PropertyKey::from(symbol);
            let atom = key.atom();
            let resume = take_read(
                ProxyBooleanStep::start(
                    &runtime,
                    context.realm,
                    proxy,
                    ProxyBooleanKind::Delete(key),
                )
                .unwrap(),
            );
            let resume = take_call(
                resume
                    .resume(&runtime, Completion::Return(callable))
                    .unwrap(),
            );
            let mut resume = take_descriptor(
                resume
                    .resume(&runtime, Completion::Return(Value::Bool(true)))
                    .unwrap(),
            );
            if after_descriptor {
                resume = take_extensible(
                    resume
                        .descriptor(
                            &runtime,
                            NativeConversion::Value(Some(
                                CompleteOrdinaryPropertyDescriptor::Data {
                                    value: Value::Int(1),
                                    writable: true,
                                    enumerable: true,
                                    configurable: true,
                                },
                            )),
                        )
                        .unwrap(),
                );
            }
            runtime.run_gc().unwrap();
            assert!(runtime.0.state.borrow().atoms.is_live(atom));
            drop(resume);
            runtime.run_gc().unwrap();
            assert!(!runtime.0.state.borrow().atoms.is_live(atom));
            drop(context);
            drop(runtime);
            assert!(weak.upgrade().is_none());
        }
    }
}

#[derive(Default)]
struct ProxyBooleanStepPending {
    delete_object: Option<ObjectRef>,
    delete_key: Option<PropertyKey>,
    prevent_extensions_object: Option<ObjectRef>,
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    read_receiver: Option<Value>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    has_object: Option<ObjectRef>,
    has_key: Option<PropertyKey>,
    extensible_object: Option<ObjectRef>,
    descriptor_object: Option<ObjectRef>,
    descriptor_key: Option<PropertyKey>,
}
impl ProxyBooleanStep {
    pub(crate) fn request_delete(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ProxyBooleanResume,
    ) -> Self {
        resume.0.pending_effect.delete_object = Some(object);
        resume.0.pending_effect.delete_key = Some(key);
        Self::Delete { resume }
    }
    pub(crate) fn request_prevent_extensions(
        object: ObjectRef,
        mut resume: ProxyBooleanResume,
    ) -> Self {
        resume.0.pending_effect.prevent_extensions_object = Some(object);
        Self::PreventExtensions { resume }
    }
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        receiver: Value,
        mut resume: ProxyBooleanResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        resume.0.pending_effect.read_receiver = Some(receiver);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        target: DirectCallTarget,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: ProxyBooleanResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_has(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ProxyBooleanResume,
    ) -> Self {
        resume.0.pending_effect.has_object = Some(object);
        resume.0.pending_effect.has_key = Some(key);
        Self::Has { resume }
    }
    pub(crate) fn request_extensible(object: ObjectRef, mut resume: ProxyBooleanResume) -> Self {
        resume.0.pending_effect.extensible_object = Some(object);
        Self::Extensible { resume }
    }
    pub(crate) fn request_descriptor(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ProxyBooleanResume,
    ) -> Self {
        resume.0.pending_effect.descriptor_object = Some(object);
        resume.0.pending_effect.descriptor_key = Some(key);
        Self::Descriptor { resume }
    }
}
impl ProxyBooleanResume {
    pub(crate) fn take_delete_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .delete_object
            .take()
            .expect("ProxyBooleanStep Delete object")
    }
    pub(crate) fn take_delete_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .delete_key
            .take()
            .expect("ProxyBooleanStep Delete key")
    }
    pub(crate) fn take_prevent_extensions_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .prevent_extensions_object
            .take()
            .expect("ProxyBooleanStep PreventExtensions object")
    }
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ProxyBooleanStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ProxyBooleanStep Read key")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("ProxyBooleanStep Read receiver")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("ProxyBooleanStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ProxyBooleanStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ProxyBooleanStep Call arguments")
    }
    pub(crate) fn take_has_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .has_object
            .take()
            .expect("ProxyBooleanStep Has object")
    }
    pub(crate) fn take_has_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .has_key
            .take()
            .expect("ProxyBooleanStep Has key")
    }
    pub(crate) fn take_extensible_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .extensible_object
            .take()
            .expect("ProxyBooleanStep Extensible object")
    }
    pub(crate) fn take_descriptor_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .descriptor_object
            .take()
            .expect("ProxyBooleanStep Descriptor object")
    }
    pub(crate) fn take_descriptor_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .descriptor_key
            .take()
            .expect("ProxyBooleanStep Descriptor key")
    }
}
const _: () = assert!(std::mem::size_of::<ProxyBooleanStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ProxyBooleanStep>() <= 64);
