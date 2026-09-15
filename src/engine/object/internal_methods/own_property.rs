//! Proxy [[GetOwnProperty]] and its observable invariant-query order.
use super::{
    RootedProxy,
    method::{MethodResume, MethodStep},
    proxy_gopd_descriptor_is_compatible,
};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::ContextId;
use crate::engine::object::operations::{
    descriptor_to_validation_record, validation_record_to_complete,
};
use crate::engine::object::property::validate_and_apply_property_descriptor;
use crate::engine::object::{
    CompleteOrdinaryPropertyDescriptor, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{Completion, call::DirectCallTarget};

type Descriptor = Option<CompleteOrdinaryPropertyDescriptor>;

pub(crate) enum ProxyOwnStep {
    Complete(NativeConversion<Descriptor>),
    Read { resume: ProxyOwnResume },
    Call { resume: ProxyOwnResume },
    Descriptor { resume: ProxyOwnResume },
    Extensible { resume: ProxyOwnResume },
    Convert { resume: ProxyOwnResume },
}

pub(crate) struct ProxyOwnResume(Box<ProxyOwnResumeState>);
impl std::ops::Deref for ProxyOwnResume {
    type Target = ProxyOwnResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ProxyOwnResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ProxyOwnResume>() <= 8);
pub(crate) struct ProxyOwnResumeState {
    pending_effect: ProxyOwnStepPending,
    realm: ContextId,
    phase: Phase,
}

enum Phase {
    Method {
        resume: MethodResume,
        key: PropertyKey,
    },
    Forward {
        _rooted: RootedProxy,
    },
    Trap {
        rooted: RootedProxy,
        key: PropertyKey,
    },
    Target {
        rooted: RootedProxy,
        result: Value,
    },
    Extensible {
        rooted: RootedProxy,
        result: Value,
        target: Descriptor,
    },
    Converted {
        _rooted: RootedProxy,
        target: Descriptor,
        extensible: bool,
    },
}

impl ProxyOwnStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
        key: PropertyKey,
    ) -> Result<Self, RuntimeError> {
        runtime.validate_object_and_key(&object, &key)?;
        let step = MethodStep::start(runtime, realm, object, "getOwnPropertyDescriptor")?;
        method(runtime, realm, key, step)
    }
}

fn method(
    runtime: &Runtime,
    realm: ContextId,
    key: PropertyKey,
    step: MethodStep,
) -> Result<ProxyOwnStep, RuntimeError> {
    Ok(match step {
        MethodStep::Throw(value) => ProxyOwnStep::Complete(NativeConversion::Throw(value)),
        MethodStep::Complete { mut resume } => {
            let rooted = resume.take_completed_rooted();
            let target = resume.take_completed_target();
            drop(resume);
            match target {
                None => ProxyOwnStep::request_descriptor(
                    rooted.target.clone(),
                    key,
                    ProxyOwnResume(Box::new(ProxyOwnResumeState {
                        pending_effect: ProxyOwnStepPending::default(),
                        realm,
                        phase: Phase::Forward { _rooted: rooted },
                    })),
                ),
                Some(target) => {
                    let key_value = runtime.property_key_value(&key)?;
                    ProxyOwnStep::request_call(
                        target,
                        Value::Object(rooted.handler.clone()),
                        vec![Value::Object(rooted.target.clone()), key_value],
                        ProxyOwnResume(Box::new(ProxyOwnResumeState {
                            pending_effect: ProxyOwnStepPending::default(),
                            realm,
                            phase: Phase::Trap { rooted, key },
                        })),
                    )
                }
            }
        }
        MethodStep::Read { mut resume } => {
            let object = resume.take_read_object();
            let method_key = resume.take_read_key();
            let receiver = resume.take_read_receiver();
            ProxyOwnStep::request_read(
                object,
                method_key,
                receiver,
                ProxyOwnResume(Box::new(ProxyOwnResumeState {
                    pending_effect: ProxyOwnStepPending::default(),
                    realm,
                    phase: Phase::Method { resume, key },
                })),
            )
        }
    })
}

impl ProxyOwnResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<ProxyOwnStep, RuntimeError> {
        let value = match completion {
            Completion::Throw(value) => {
                return Ok(ProxyOwnStep::Complete(NativeConversion::Throw(value)));
            }
            Completion::Return(value) => value,
        };
        match self.0.phase {
            Phase::Method { resume, key } => method(
                runtime,
                self.0.realm,
                key,
                resume.resume(runtime, Completion::Return(value))?,
            ),
            Phase::Trap { rooted, key } => {
                if !matches!(value, Value::Undefined | Value::Object(_)) {
                    return Ok(ProxyOwnStep::Complete(runtime.proxy_invariant_throw(
                        self.0.realm,
                        "getOwnPropertyDescriptor",
                    )?));
                }
                Ok(ProxyOwnStep::request_descriptor(
                    rooted.target.clone(),
                    key,
                    Self(Box::new(ProxyOwnResumeState {
                        pending_effect: ProxyOwnStepPending::default(),
                        realm: self.0.realm,
                        phase: Phase::Target {
                            rooted,
                            result: value,
                        },
                    })),
                ))
            }
            _ => Err(RuntimeError::Invariant(
                "Proxy descriptor continuation received a value reply",
            )),
        }
    }

    pub(crate) fn descriptor(
        self,
        runtime: &Runtime,
        descriptor: NativeConversion<Descriptor>,
    ) -> Result<ProxyOwnStep, RuntimeError> {
        let target = match descriptor {
            NativeConversion::Value(target) => target,
            NativeConversion::Throw(value) => {
                return Ok(ProxyOwnStep::Complete(NativeConversion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Forward { .. } => Ok(ProxyOwnStep::Complete(NativeConversion::Value(target))),
            Phase::Target { rooted, result } => {
                if matches!(result, Value::Undefined) {
                    if let Some(target) = target
                        && (!target.configurable()
                            || !runtime.raw_extensible_bit(&rooted.target)?)
                    {
                        return Ok(ProxyOwnStep::Complete(runtime.proxy_invariant_throw(
                            self.0.realm,
                            "getOwnPropertyDescriptor",
                        )?));
                    }
                    return Ok(ProxyOwnStep::Complete(NativeConversion::Value(None)));
                }
                // QuickJS queries target extensibility before reading any
                // fields from the descriptor returned by the trap.
                Ok(ProxyOwnStep::request_extensible(
                    rooted.target.clone(),
                    Self(Box::new(ProxyOwnResumeState {
                        pending_effect: ProxyOwnStepPending::default(),
                        realm: self.0.realm,
                        phase: Phase::Extensible {
                            rooted,
                            result,
                            target,
                        },
                    })),
                ))
            }
            _ => Err(RuntimeError::Invariant(
                "Proxy value continuation received a descriptor reply",
            )),
        }
    }

    pub(crate) fn extensible(
        self,
        result: NativeConversion<bool>,
    ) -> Result<ProxyOwnStep, RuntimeError> {
        let Phase::Extensible {
            rooted,
            result: value,
            target,
        } = self.0.phase
        else {
            return Err(RuntimeError::Invariant(
                "Proxy descriptor continuation received an extensibility reply",
            ));
        };
        match result {
            NativeConversion::Throw(value) => {
                Ok(ProxyOwnStep::Complete(NativeConversion::Throw(value)))
            }
            NativeConversion::Value(extensible) => Ok(ProxyOwnStep::request_convert(
                value,
                Self(Box::new(ProxyOwnResumeState {
                    pending_effect: ProxyOwnStepPending::default(),
                    realm: self.0.realm,
                    phase: Phase::Converted {
                        _rooted: rooted,
                        target,
                        extensible,
                    },
                })),
            )),
        }
    }

    pub(crate) fn converted(
        self,
        runtime: &Runtime,
        result: NativeConversion<OrdinaryPropertyDescriptor>,
    ) -> Result<ProxyOwnStep, RuntimeError> {
        let Phase::Converted {
            _rooted,
            target,
            extensible,
        } = self.0.phase
        else {
            return Err(RuntimeError::Invariant(
                "Proxy descriptor continuation received a conversion reply",
            ));
        };
        let result = match result {
            NativeConversion::Throw(value) => {
                return Ok(ProxyOwnStep::Complete(NativeConversion::Throw(value)));
            }
            NativeConversion::Value(result) => result,
        };
        let result = descriptor_to_validation_record(&result);
        let complete = validate_and_apply_property_descriptor(
            true,
            &result,
            None,
            &Value::Undefined,
            Value::same_value,
        )
        .map_err(|_| {
            RuntimeError::Invariant("validated Proxy descriptor could not be completed")
        })?;
        let result = validation_record_to_complete(complete)?;
        if !proxy_gopd_descriptor_is_compatible(target.as_ref(), &result, extensible) {
            return Ok(ProxyOwnStep::Complete(runtime.proxy_invariant_throw(
                self.0.realm,
                "getOwnPropertyDescriptor",
            )?));
        }
        Ok(ProxyOwnStep::Complete(NativeConversion::Value(Some(
            result,
        ))))
    }
}

#[derive(Default)]
struct ProxyOwnStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    read_receiver: Option<Value>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    descriptor_object: Option<ObjectRef>,
    descriptor_key: Option<PropertyKey>,
    extensible_object: Option<ObjectRef>,
    convert_value: Option<Value>,
}
impl ProxyOwnStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        receiver: Value,
        mut resume: ProxyOwnResume,
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
        mut resume: ProxyOwnResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_descriptor(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ProxyOwnResume,
    ) -> Self {
        resume.0.pending_effect.descriptor_object = Some(object);
        resume.0.pending_effect.descriptor_key = Some(key);
        Self::Descriptor { resume }
    }
    pub(crate) fn request_extensible(object: ObjectRef, mut resume: ProxyOwnResume) -> Self {
        resume.0.pending_effect.extensible_object = Some(object);
        Self::Extensible { resume }
    }
    pub(crate) fn request_convert(value: Value, mut resume: ProxyOwnResume) -> Self {
        resume.0.pending_effect.convert_value = Some(value);
        Self::Convert { resume }
    }
}
impl ProxyOwnResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ProxyOwnStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ProxyOwnStep Read key")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("ProxyOwnStep Read receiver")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("ProxyOwnStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ProxyOwnStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ProxyOwnStep Call arguments")
    }
    pub(crate) fn take_descriptor_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .descriptor_object
            .take()
            .expect("ProxyOwnStep Descriptor object")
    }
    pub(crate) fn take_descriptor_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .descriptor_key
            .take()
            .expect("ProxyOwnStep Descriptor key")
    }
    pub(crate) fn take_extensible_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .extensible_object
            .take()
            .expect("ProxyOwnStep Extensible object")
    }
    pub(crate) fn take_convert_value(&mut self) -> Value {
        self.0
            .pending_effect
            .convert_value
            .take()
            .expect("ProxyOwnStep Convert value")
    }
}
const _: () = assert!(std::mem::size_of::<ProxyOwnStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ProxyOwnStep>() <= 64);
