//! Proxy prototype requests share their ordered extensibility/identity checks.
use super::{
    RootedProxy,
    method::{MethodResume, MethodStep},
};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::ContextId;
use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{Completion, call::DirectCallTarget};

pub(crate) enum ProxyPrototypeKind {
    Get,
    Set(Option<ObjectRef>),
}
pub(crate) enum ProxyPrototypeStep {
    Complete(Completion),
    Read { resume: ProxyPrototypeResume },
    Call { resume: ProxyPrototypeResume },
    Get { resume: ProxyPrototypeResume },
    Set { resume: ProxyPrototypeResume },
    Extensible { resume: ProxyPrototypeResume },
}
pub(crate) struct ProxyPrototypeResume(Box<ProxyPrototypeResumeState>);
impl std::ops::Deref for ProxyPrototypeResume {
    type Target = ProxyPrototypeResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ProxyPrototypeResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ProxyPrototypeResume>() <= 8);
pub(crate) struct ProxyPrototypeResumeState {
    pending_effect: ProxyPrototypeStepPending,
    realm: ContextId,
    phase: Phase,
}
enum Phase {
    Method {
        resume: MethodResume,
        kind: ProxyPrototypeKind,
    },
    Forward {
        _rooted: RootedProxy,
        kind: ProxyPrototypeKind,
    },
    Trap {
        rooted: RootedProxy,
        kind: ProxyPrototypeKind,
    },
    Extensible {
        rooted: RootedProxy,
        prototype: Option<ObjectRef>,
        setting: bool,
    },
    Compare {
        _rooted: RootedProxy,
        prototype: Option<ObjectRef>,
        setting: bool,
    },
}
impl ProxyPrototypeStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
        kind: ProxyPrototypeKind,
    ) -> Result<Self, RuntimeError> {
        let name = match &kind {
            ProxyPrototypeKind::Get => "getPrototypeOf",
            ProxyPrototypeKind::Set(prototype) => {
                if prototype
                    .as_ref()
                    .is_some_and(|object| !object.belongs_to(runtime))
                {
                    return Err(RuntimeError::WrongRuntime("Proxy prototype"));
                }
                "setPrototypeOf"
            }
        };
        method(
            runtime,
            realm,
            kind,
            MethodStep::start(runtime, realm, object, name)?,
        )
    }
}
fn method(
    _runtime: &Runtime,
    realm: ContextId,
    kind: ProxyPrototypeKind,
    step: MethodStep,
) -> Result<ProxyPrototypeStep, RuntimeError> {
    Ok(match step {
        MethodStep::Throw(value) => ProxyPrototypeStep::Complete(Completion::Throw(value)),
        MethodStep::Read { mut resume } => {
            let object = resume.take_read_object();
            let key = resume.take_read_key();
            let receiver = resume.take_read_receiver();
            ProxyPrototypeStep::request_read(
                object,
                key,
                receiver,
                ProxyPrototypeResume(Box::new(ProxyPrototypeResumeState {
                    pending_effect: ProxyPrototypeStepPending::default(),
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
                    let prototype = match &kind {
                        ProxyPrototypeKind::Set(prototype) => Some(prototype.clone()),
                        _ => None,
                    };
                    let resume = ProxyPrototypeResume(Box::new(ProxyPrototypeResumeState {
                        pending_effect: ProxyPrototypeStepPending::default(),
                        realm,
                        phase: Phase::Forward {
                            _rooted: rooted,
                            kind,
                        },
                    }));
                    match prototype {
                        Some(prototype) => {
                            ProxyPrototypeStep::request_set(object, prototype, resume)
                        }
                        None => ProxyPrototypeStep::request_get(object, resume),
                    }
                }
                Some(target) => {
                    let mut arguments = vec![Value::Object(rooted.target.clone())];
                    if let ProxyPrototypeKind::Set(prototype) = &kind {
                        arguments.push(prototype.clone().map_or(Value::Null, Value::Object));
                    }
                    ProxyPrototypeStep::request_call(
                        target,
                        Value::Object(rooted.handler.clone()),
                        arguments,
                        ProxyPrototypeResume(Box::new(ProxyPrototypeResumeState {
                            pending_effect: ProxyPrototypeStepPending::default(),
                            realm,
                            phase: Phase::Trap { rooted, kind },
                        })),
                    )
                }
            }
        }
    })
}
fn completed(prototype: Option<ObjectRef>, setting: bool) -> ProxyPrototypeStep {
    ProxyPrototypeStep::Complete(Completion::Return(if setting {
        Value::Bool(true)
    } else {
        prototype.map_or(Value::Null, Value::Object)
    }))
}
fn inconsistent(runtime: &Runtime, realm: ContextId) -> Result<ProxyPrototypeStep, RuntimeError> {
    Ok(ProxyPrototypeStep::Complete(
        match runtime.proxy_invariant_throw::<Value>(realm, "prototype")? {
            NativeConversion::Throw(value) => Completion::Throw(value),
            NativeConversion::Value(_) => {
                return Err(RuntimeError::Invariant(
                    "Proxy invariant rejection returned a value",
                ));
            }
        },
    ))
}
impl ProxyPrototypeResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<ProxyPrototypeStep, RuntimeError> {
        let value = match completion {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(ProxyPrototypeStep::Complete(Completion::Throw(value)));
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
                let (prototype, setting) = match kind {
                    ProxyPrototypeKind::Get => (
                        match value {
                            Value::Object(object) => Some(object),
                            Value::Null => None,
                            _ => return inconsistent(runtime, self.0.realm),
                        },
                        false,
                    ),
                    ProxyPrototypeKind::Set(prototype) => {
                        if !runtime.value_to_boolean(&value)? {
                            return Ok(ProxyPrototypeStep::Complete(Completion::Return(
                                Value::Bool(false),
                            )));
                        }
                        (prototype, true)
                    }
                };
                Ok(ProxyPrototypeStep::request_extensible(
                    rooted.target.clone(),
                    Self(Box::new(ProxyPrototypeResumeState {
                        pending_effect: ProxyPrototypeStepPending::default(),
                        realm: self.0.realm,
                        phase: Phase::Extensible {
                            rooted,
                            prototype,
                            setting,
                        },
                    })),
                ))
            }
            _ => Err(RuntimeError::Invariant(
                "Proxy prototype continuation received a value reply",
            )),
        }
    }
    pub(crate) fn boolean(
        self,
        _runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<ProxyPrototypeStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ProxyPrototypeStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Forward {
                kind: ProxyPrototypeKind::Set(_),
                ..
            } => Ok(ProxyPrototypeStep::Complete(Completion::Return(
                Value::Bool(value),
            ))),
            Phase::Extensible {
                rooted,
                prototype,
                setting,
            } => {
                if value {
                    return Ok(completed(prototype, setting));
                }
                Ok(ProxyPrototypeStep::request_get(
                    rooted.target.clone(),
                    Self(Box::new(ProxyPrototypeResumeState {
                        pending_effect: ProxyPrototypeStepPending::default(),
                        realm: self.0.realm,
                        phase: Phase::Compare {
                            _rooted: rooted,
                            prototype,
                            setting,
                        },
                    })),
                ))
            }
            _ => Err(RuntimeError::Invariant(
                "Proxy prototype continuation received a boolean reply",
            )),
        }
    }
    pub(crate) fn prototype(
        self,
        runtime: &Runtime,
        result: NativeConversion<Option<ObjectRef>>,
    ) -> Result<ProxyPrototypeStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ProxyPrototypeStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Forward {
                kind: ProxyPrototypeKind::Get,
                ..
            } => Ok(completed(value, false)),
            Phase::Compare {
                _rooted,
                prototype,
                setting,
            } => {
                if value != prototype {
                    return inconsistent(runtime, self.0.realm);
                }
                Ok(completed(prototype, setting))
            }
            _ => Err(RuntimeError::Invariant(
                "Proxy prototype continuation received a prototype reply",
            )),
        }
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ProxyPrototypeStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            ProxyPrototypeStep::Complete(result) => return Ok(result),
            ProxyPrototypeStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                resume.resume(
                    runtime,
                    runtime.internal_get(realm, &object, &key, receiver)?,
                )?
            }
            ProxyPrototypeStep::Call { mut resume } => {
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
            ProxyPrototypeStep::Get { mut resume } => {
                let object = resume.take_get_object();
                resume.prototype(runtime, runtime.internal_get_prototype_of(realm, &object)?)?
            }
            ProxyPrototypeStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let prototype = resume.take_set_prototype();
                resume.boolean(
                    runtime,
                    runtime.internal_set_prototype_of(realm, &object, prototype.as_ref())?,
                )?
            }
            ProxyPrototypeStep::Extensible { mut resume } => {
                let object = resume.take_extensible_object();
                resume.boolean(runtime, runtime.internal_is_extensible(realm, &object)?)?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn take_read(step: ProxyPrototypeStep) -> ProxyPrototypeResume {
        let ProxyPrototypeStep::Read { resume, .. } = step else {
            panic!("expected read")
        };
        resume
    }
    fn take_call(step: ProxyPrototypeStep) -> ProxyPrototypeResume {
        let ProxyPrototypeStep::Call { resume, .. } = step else {
            panic!("expected call")
        };
        resume
    }
    fn take_extensible(step: ProxyPrototypeStep) -> ProxyPrototypeResume {
        let ProxyPrototypeStep::Extensible { resume, .. } = step else {
            panic!("expected extensibility")
        };
        resume
    }
    fn take_get(step: ProxyPrototypeStep) -> ProxyPrototypeResume {
        let ProxyPrototypeStep::Get { resume, .. } = step else {
            panic!("expected prototype")
        };
        resume
    }
    #[test]
    fn prototypes_are_owned_across_requests_and_released_on_abandonment() {
        for setting in [false, true] {
            for compare in [false, true] {
                let runtime = Runtime::new();
                let weak = std::rc::Rc::downgrade(&runtime.0);
                let mut context = runtime.new_context();
                let Value::Object(proxy) = context.eval("new Proxy({}, {})").unwrap() else {
                    panic!("expected Proxy")
                };
                let rooted = runtime.proxy_snapshot_if_any(&proxy).unwrap().unwrap();
                let ids = [proxy.object_id(), rooted.target, rooted.handler];
                let prototype = runtime.new_object(None).unwrap();
                let prototype_id = prototype.object_id();
                let callable = context.eval("(function(){return true})").unwrap();
                let kind = if setting {
                    ProxyPrototypeKind::Set(Some(prototype.clone()))
                } else {
                    ProxyPrototypeKind::Get
                };
                let resume = take_read(
                    ProxyPrototypeStep::start(&runtime, context.realm, proxy, kind).unwrap(),
                );
                let resume = take_call(
                    resume
                        .resume(&runtime, Completion::Return(callable))
                        .unwrap(),
                );
                let reply = if setting {
                    drop(prototype);
                    Value::Bool(true)
                } else {
                    Value::Object(prototype)
                };
                let mut resume =
                    take_extensible(resume.resume(&runtime, Completion::Return(reply)).unwrap());
                if compare {
                    resume = take_get(
                        resume
                            .boolean(&runtime, NativeConversion::Value(false))
                            .unwrap(),
                    );
                }
                runtime.run_gc().unwrap();
                for id in ids.into_iter().chain([prototype_id]) {
                    assert!(runtime.0.state.borrow().heap.object(id).is_ok());
                }
                drop(resume);
                runtime.run_gc().unwrap();
                for id in ids.into_iter().chain([prototype_id]) {
                    assert!(runtime.0.state.borrow().heap.object(id).is_err());
                }
                assert_eq!(runtime.0.proxy_method_depth.get(), 0);
                drop(context);
                drop(runtime);
                assert!(weak.upgrade().is_none());
            }
        }
    }
}

#[derive(Default)]
struct ProxyPrototypeStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    read_receiver: Option<Value>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    get_object: Option<ObjectRef>,
    set_object: Option<ObjectRef>,
    set_prototype: Option<Option<ObjectRef>>,
    extensible_object: Option<ObjectRef>,
}
impl ProxyPrototypeStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        receiver: Value,
        mut resume: ProxyPrototypeResume,
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
        mut resume: ProxyPrototypeResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_get(object: ObjectRef, mut resume: ProxyPrototypeResume) -> Self {
        resume.0.pending_effect.get_object = Some(object);
        Self::Get { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        prototype: Option<ObjectRef>,
        mut resume: ProxyPrototypeResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_prototype = Some(prototype);
        Self::Set { resume }
    }
    pub(crate) fn request_extensible(object: ObjectRef, mut resume: ProxyPrototypeResume) -> Self {
        resume.0.pending_effect.extensible_object = Some(object);
        Self::Extensible { resume }
    }
}
impl ProxyPrototypeResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ProxyPrototypeStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ProxyPrototypeStep Read key")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("ProxyPrototypeStep Read receiver")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("ProxyPrototypeStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ProxyPrototypeStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ProxyPrototypeStep Call arguments")
    }
    pub(crate) fn take_get_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .get_object
            .take()
            .expect("ProxyPrototypeStep Get object")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("ProxyPrototypeStep Set object")
    }
    pub(crate) fn take_set_prototype(&mut self) -> Option<ObjectRef> {
        self.0
            .pending_effect
            .set_prototype
            .take()
            .expect("ProxyPrototypeStep Set prototype")
    }
    pub(crate) fn take_extensible_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .extensible_object
            .take()
            .expect("ProxyPrototypeStep Extensible object")
    }
}
const _: () = assert!(std::mem::size_of::<ProxyPrototypeStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ProxyPrototypeStep>() <= 64);
