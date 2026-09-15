//! Proxy [[Set]] stages, including the target descriptor invariant.
use super::{
    RootedProxy,
    method::{MethodResume, MethodStep},
};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::ContextId;
use crate::engine::object::operations::InternalSetResult;
use crate::engine::object::{CompleteOrdinaryPropertyDescriptor, ObjectRef, PropertyKey};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{Completion, call::DirectCallTarget};

pub(crate) enum ProxySetStep {
    Complete(NativeConversion<InternalSetResult>),
    Read { resume: ProxySetResume },
    Call { resume: ProxySetResume },
    Set { resume: ProxySetResume },
    Descriptor { resume: ProxySetResume },
}
pub(crate) struct ProxySetResume(Box<ProxySetResumeState>);
impl std::ops::Deref for ProxySetResume {
    type Target = ProxySetResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ProxySetResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ProxySetResume>() <= 8);
pub(crate) struct ProxySetResumeState {
    pending_effect: ProxySetStepPending,
    realm: ContextId,
    phase: Phase,
}
enum Phase {
    Method {
        resume: MethodResume,
        key: PropertyKey,
        value: Value,
        receiver: Value,
    },
    Forward {
        _rooted: RootedProxy,
    },
    Trap {
        rooted: RootedProxy,
        key: PropertyKey,
        value: Value,
    },
    Invariant {
        _rooted: RootedProxy,
        value: Value,
    },
}
impl ProxySetStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<Self, RuntimeError> {
        runtime.validate_object_and_key(&object, &key)?;
        runtime.validate_value_domain(&value, "property value")?;
        runtime.validate_value_domain(&receiver, "property receiver")?;
        let step = MethodStep::start(runtime, realm, object, "set")?;
        method(runtime, realm, key, value, receiver, step)
    }
}
fn method(
    runtime: &Runtime,
    realm: ContextId,
    key: PropertyKey,
    value: Value,
    receiver: Value,
    step: MethodStep,
) -> Result<ProxySetStep, RuntimeError> {
    Ok(match step {
        MethodStep::Throw(value) => ProxySetStep::Complete(NativeConversion::Throw(value)),
        MethodStep::Read { mut resume } => {
            let object = resume.take_read_object();
            let method_key = resume.take_read_key();
            let method_receiver = resume.take_read_receiver();
            ProxySetStep::request_read(
                object,
                method_key,
                method_receiver,
                ProxySetResume(Box::new(ProxySetResumeState {
                    pending_effect: ProxySetStepPending::default(),
                    realm,
                    phase: Phase::Method {
                        resume,
                        key,
                        value,
                        receiver,
                    },
                })),
            )
        }
        MethodStep::Complete { mut resume } => {
            let rooted = resume.take_completed_rooted();
            let target = resume.take_completed_target();
            drop(resume);
            match target {
                None => ProxySetStep::request_set(
                    rooted.target.clone(),
                    key,
                    value,
                    receiver,
                    ProxySetResume(Box::new(ProxySetResumeState {
                        pending_effect: ProxySetStepPending::default(),
                        realm,
                        phase: Phase::Forward { _rooted: rooted },
                    })),
                ),
                Some(target) => {
                    let key_value = runtime.property_key_value(&key)?;
                    ProxySetStep::request_call(
                        target,
                        Value::Object(rooted.handler.clone()),
                        vec![
                            Value::Object(rooted.target.clone()),
                            key_value,
                            value.clone(),
                            receiver,
                        ],
                        ProxySetResume(Box::new(ProxySetResumeState {
                            pending_effect: ProxySetStepPending::default(),
                            realm,
                            phase: Phase::Trap { rooted, key, value },
                        })),
                    )
                }
            }
        }
    })
}
impl ProxySetResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<ProxySetStep, RuntimeError> {
        let result = match completion {
            Completion::Throw(value) => {
                return Ok(ProxySetStep::Complete(NativeConversion::Throw(value)));
            }
            Completion::Return(value) => value,
        };
        match self.0.phase {
            Phase::Method {
                resume,
                key,
                value,
                receiver,
            } => method(
                runtime,
                self.0.realm,
                key,
                value,
                receiver,
                resume.resume(runtime, Completion::Return(result))?,
            ),
            Phase::Trap { rooted, key, value } => {
                if !runtime.value_to_boolean(&result)? {
                    return Ok(ProxySetStep::Complete(NativeConversion::Value(
                        InternalSetResult::RejectedProxyTrap,
                    )));
                }
                Ok(ProxySetStep::request_descriptor(
                    rooted.target.clone(),
                    key,
                    Self(Box::new(ProxySetResumeState {
                        pending_effect: ProxySetStepPending::default(),
                        realm: self.0.realm,
                        phase: Phase::Invariant {
                            _rooted: rooted,
                            value,
                        },
                    })),
                ))
            }
            _ => Err(RuntimeError::Invariant(
                "Proxy Set continuation received a value reply",
            )),
        }
    }
    pub(crate) fn set(
        self,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<ProxySetStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Forward { .. }) {
            return Err(RuntimeError::Invariant(
                "Proxy Set continuation received a Set reply",
            ));
        }
        Ok(ProxySetStep::Complete(result))
    }
    pub(crate) fn descriptor(
        self,
        runtime: &Runtime,
        result: NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>,
    ) -> Result<ProxySetStep, RuntimeError> {
        let Phase::Invariant { _rooted, value } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "Proxy Set continuation received a descriptor reply",
            ));
        };
        let target = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ProxySetStep::Complete(NativeConversion::Throw(value)));
            }
        };
        let invalid = match target {
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: target_value,
                writable: false,
                configurable: false,
                ..
            }) => !value.same_value(&target_value),
            Some(CompleteOrdinaryPropertyDescriptor::Accessor {
                set: None,
                configurable: false,
                ..
            }) => true,
            _ => false,
        };
        Ok(ProxySetStep::Complete(if invalid {
            runtime.proxy_invariant_throw(self.0.realm, "set")?
        } else {
            NativeConversion::Value(InternalSetResult::Accepted)
        }))
    }
}

#[derive(Default)]
struct ProxySetStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    read_receiver: Option<Value>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    set_object: Option<ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<Value>,
    set_receiver: Option<Value>,
    descriptor_object: Option<ObjectRef>,
    descriptor_key: Option<PropertyKey>,
}
impl ProxySetStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        receiver: Value,
        mut resume: ProxySetResume,
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
        mut resume: ProxySetResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        receiver: Value,
        mut resume: ProxySetResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        resume.0.pending_effect.set_receiver = Some(receiver);
        Self::Set { resume }
    }
    pub(crate) fn request_descriptor(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ProxySetResume,
    ) -> Self {
        resume.0.pending_effect.descriptor_object = Some(object);
        resume.0.pending_effect.descriptor_key = Some(key);
        Self::Descriptor { resume }
    }
}
impl ProxySetResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ProxySetStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ProxySetStep Read key")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("ProxySetStep Read receiver")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("ProxySetStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ProxySetStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ProxySetStep Call arguments")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("ProxySetStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("ProxySetStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("ProxySetStep Set value")
    }
    pub(crate) fn take_set_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .set_receiver
            .take()
            .expect("ProxySetStep Set receiver")
    }
    pub(crate) fn take_descriptor_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .descriptor_object
            .take()
            .expect("ProxySetStep Descriptor object")
    }
    pub(crate) fn take_descriptor_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .descriptor_key
            .take()
            .expect("ProxySetStep Descriptor key")
    }
}
const _: () = assert!(std::mem::size_of::<ProxySetStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ProxySetStep>() <= 64);
