//! Proxy [[DefineOwnProperty]] shares its observable trap/invariant stages.
use super::{
    RootedProxy,
    method::{MethodResume, MethodStep},
    proxy_define_descriptor_is_compatible,
};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::ContextId;
use crate::engine::object::operations::InternalDefineResult;
use crate::engine::object::{
    CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor,
    PropertyKey,
};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{Completion, call::DirectCallTarget};

pub(crate) enum ProxyDefineStep {
    Complete(NativeConversion<InternalDefineResult>),
    Read { resume: ProxyDefineResume },
    Call { resume: ProxyDefineResume },
    Define { resume: ProxyDefineResume },
    Descriptor { resume: ProxyDefineResume },
}
pub(crate) struct ProxyDefineResume(Box<ProxyDefineResumeState>);
impl std::ops::Deref for ProxyDefineResume {
    type Target = ProxyDefineResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ProxyDefineResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ProxyDefineResume>() <= 8);
pub(crate) struct ProxyDefineResumeState {
    pending_effect: ProxyDefineStepPending,
    realm: ContextId,
    phase: Phase,
}
enum Phase {
    Method {
        resume: MethodResume,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
    },
    Forward {
        _rooted: RootedProxy,
    },
    Trap {
        rooted: RootedProxy,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
    },
    Invariant {
        rooted: RootedProxy,
        descriptor: OrdinaryPropertyDescriptor,
    },
}
impl ProxyDefineStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
    ) -> Result<Self, RuntimeError> {
        runtime.validate_object_and_key(&object, &key)?;
        runtime.validate_descriptor_domains(&descriptor)?;
        let step = MethodStep::start(runtime, realm, object, "defineProperty")?;
        method(runtime, realm, key, descriptor, step)
    }
}
fn method(
    runtime: &Runtime,
    realm: ContextId,
    key: PropertyKey,
    descriptor: OrdinaryPropertyDescriptor,
    step: MethodStep,
) -> Result<ProxyDefineStep, RuntimeError> {
    Ok(match step {
        MethodStep::Throw(value) => ProxyDefineStep::Complete(NativeConversion::Throw(value)),
        MethodStep::Read { mut resume } => {
            let object = resume.take_read_object();
            let method_key = resume.take_read_key();
            let receiver = resume.take_read_receiver();
            ProxyDefineStep::request_read(
                object,
                method_key,
                receiver,
                ProxyDefineResume(Box::new(ProxyDefineResumeState {
                    pending_effect: ProxyDefineStepPending::default(),
                    realm,
                    phase: Phase::Method {
                        resume,
                        key,
                        descriptor,
                    },
                })),
            )
        }
        MethodStep::Complete { mut resume } => {
            let rooted = resume.take_completed_rooted();
            let target = resume.take_completed_target();
            drop(resume);
            match target {
                None => ProxyDefineStep::request_define(
                    rooted.target.clone(),
                    key,
                    descriptor,
                    ProxyDefineResume(Box::new(ProxyDefineResumeState {
                        pending_effect: ProxyDefineStepPending::default(),
                        realm,
                        phase: Phase::Forward { _rooted: rooted },
                    })),
                ),
                Some(target) => {
                    let key_value = runtime.property_key_value(&key)?;
                    let descriptor_object = runtime.proxy_descriptor_object(realm, &descriptor)?;
                    ProxyDefineStep::request_call(
                        target,
                        Value::Object(rooted.handler.clone()),
                        vec![
                            Value::Object(rooted.target.clone()),
                            key_value,
                            Value::Object(descriptor_object),
                        ],
                        ProxyDefineResume(Box::new(ProxyDefineResumeState {
                            pending_effect: ProxyDefineStepPending::default(),
                            realm,
                            phase: Phase::Trap {
                                rooted,
                                key,
                                descriptor,
                            },
                        })),
                    )
                }
            }
        }
    })
}
impl ProxyDefineResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<ProxyDefineStep, RuntimeError> {
        let value = match completion {
            Completion::Throw(value) => {
                return Ok(ProxyDefineStep::Complete(NativeConversion::Throw(value)));
            }
            Completion::Return(value) => value,
        };
        match self.0.phase {
            Phase::Method {
                resume,
                key,
                descriptor,
            } => method(
                runtime,
                self.0.realm,
                key,
                descriptor,
                resume.resume(runtime, Completion::Return(value))?,
            ),
            Phase::Trap {
                rooted,
                key,
                descriptor,
            } => {
                if !runtime.value_to_boolean(&value)? {
                    return Ok(ProxyDefineStep::Complete(NativeConversion::Value(
                        InternalDefineResult::RejectedProxyTrap,
                    )));
                }
                Ok(ProxyDefineStep::request_descriptor(
                    rooted.target.clone(),
                    key,
                    Self(Box::new(ProxyDefineResumeState {
                        pending_effect: ProxyDefineStepPending::default(),
                        realm: self.0.realm,
                        phase: Phase::Invariant { rooted, descriptor },
                    })),
                ))
            }
            _ => Err(RuntimeError::Invariant(
                "Proxy Define continuation received a value reply",
            )),
        }
    }
    pub(crate) fn defined(
        self,
        result: NativeConversion<InternalDefineResult>,
    ) -> Result<ProxyDefineStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Forward { .. }) {
            return Err(RuntimeError::Invariant(
                "Proxy Define continuation received a Define reply",
            ));
        }
        Ok(ProxyDefineStep::Complete(result))
    }
    pub(crate) fn descriptor(
        self,
        runtime: &Runtime,
        result: NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>,
    ) -> Result<ProxyDefineStep, RuntimeError> {
        let Phase::Invariant { rooted, descriptor } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "Proxy Define continuation received a descriptor reply",
            ));
        };
        let target = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ProxyDefineStep::Complete(NativeConversion::Throw(value)));
            }
        };
        let compatible = if let Some(target) = target.as_ref() {
            proxy_define_descriptor_is_compatible(target, &descriptor)
        } else {
            runtime.raw_extensible_bit(&rooted.target)?
                && !matches!(descriptor.configurable, DescriptorField::Present(false))
        };
        Ok(ProxyDefineStep::Complete(if compatible {
            NativeConversion::Value(InternalDefineResult::Defined)
        } else {
            runtime.proxy_invariant_throw(self.0.realm, "defineProperty")?
        }))
    }
}

#[derive(Default)]
struct ProxyDefineStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    read_receiver: Option<Value>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    define_object: Option<ObjectRef>,
    define_key: Option<PropertyKey>,
    define_descriptor: Option<OrdinaryPropertyDescriptor>,
    descriptor_object: Option<ObjectRef>,
    descriptor_key: Option<PropertyKey>,
}
impl ProxyDefineStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        receiver: Value,
        mut resume: ProxyDefineResume,
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
        mut resume: ProxyDefineResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_define(
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
        mut resume: ProxyDefineResume,
    ) -> Self {
        resume.0.pending_effect.define_object = Some(object);
        resume.0.pending_effect.define_key = Some(key);
        resume.0.pending_effect.define_descriptor = Some(descriptor);
        Self::Define { resume }
    }
    pub(crate) fn request_descriptor(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ProxyDefineResume,
    ) -> Self {
        resume.0.pending_effect.descriptor_object = Some(object);
        resume.0.pending_effect.descriptor_key = Some(key);
        Self::Descriptor { resume }
    }
}
impl ProxyDefineResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ProxyDefineStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ProxyDefineStep Read key")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("ProxyDefineStep Read receiver")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("ProxyDefineStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ProxyDefineStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ProxyDefineStep Call arguments")
    }
    pub(crate) fn take_define_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .define_object
            .take()
            .expect("ProxyDefineStep Define object")
    }
    pub(crate) fn take_define_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .define_key
            .take()
            .expect("ProxyDefineStep Define key")
    }
    pub(crate) fn take_define_descriptor(&mut self) -> OrdinaryPropertyDescriptor {
        self.0
            .pending_effect
            .define_descriptor
            .take()
            .expect("ProxyDefineStep Define descriptor")
    }
    pub(crate) fn take_descriptor_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .descriptor_object
            .take()
            .expect("ProxyDefineStep Descriptor object")
    }
    pub(crate) fn take_descriptor_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .descriptor_key
            .take()
            .expect("ProxyDefineStep Descriptor key")
    }
}
const _: () = assert!(std::mem::size_of::<ProxyDefineStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ProxyDefineStep>() <= 64);
