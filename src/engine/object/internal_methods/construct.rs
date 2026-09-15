//! Proxy [[Construct]] keeps each selected handler and the incoming new.target rooted.
use super::{ProxyMethodStackGuard, RootedProxy};
use crate::engine::{
    api::{
        error::{ErrorKind, NativeErrorKind},
        runtime::Runtime,
        runtime_error::RuntimeError,
    },
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{ConstructNewTarget, ConstructorRef, DirectCallTarget},
    },
};
pub(crate) enum ProxyConstructStep {
    Complete(Completion),
    Read { resume: ProxyConstructResume },
    Call { resume: ProxyConstructResume },
    Construct { resume: ProxyConstructResume },
}
pub(crate) struct ProxyConstructResume(Box<ProxyConstructResumeState>);
impl std::ops::Deref for ProxyConstructResume {
    type Target = ProxyConstructResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ProxyConstructResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ProxyConstructResume>() <= 8);
pub(crate) struct ProxyConstructResumeState {
    pending_effect: ProxyConstructStepPending,
    phase: Phase,
}
enum Phase {
    Method {
        rooted: RootedProxy,
        search: Search,
    },
    Result {
        realm: ContextId,
        trap: bool,
        _rooted: RootedProxy,
        _guard: ProxyMethodStackGuard,
    },
}
struct Search {
    realm: ContextId,
    key: PropertyKey,
    limit: Option<usize>,
    depth: usize,
    guard: ProxyMethodStackGuard,
    new_target: ConstructNewTarget,
    arguments: Vec<Value>,
}
fn overflow(runtime: &Runtime, realm: ContextId) -> Result<ProxyConstructStep, RuntimeError> {
    Ok(ProxyConstructStep::Complete(Completion::Throw(
        runtime.new_native_error(realm, NativeErrorKind::Internal, "stack overflow")?,
    )))
}
impl ProxyConstructStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        proxy: ConstructorRef,
        new_target: ConstructNewTarget,
        arguments: Vec<Value>,
    ) -> Result<Self, RuntimeError> {
        if runtime.proxy_method_stack_would_overflow() {
            return overflow(runtime, realm);
        }
        Search {
            realm,
            key: runtime.intern_property_key("construct")?,
            limit: runtime.proxy_method_chain_limit("construct"),
            depth: 0,
            guard: ProxyMethodStackGuard::enter(runtime),
            new_target,
            arguments,
        }
        .read(runtime, proxy)
    }
}
impl Search {
    fn read(
        self,
        runtime: &Runtime,
        proxy: ConstructorRef,
    ) -> Result<ProxyConstructStep, RuntimeError> {
        if self.limit.is_some_and(|limit| self.depth == limit) {
            return overflow(runtime, self.realm);
        }
        let data =
            runtime
                .proxy_snapshot_if_any(proxy.as_object())?
                .ok_or(RuntimeError::Invariant(
                    "Proxy construct dispatch reached an ordinary object",
                ))?;
        if data.is_revoked {
            return match runtime.proxy_revoked_throw(self.realm)? {
                NativeConversion::Throw(value) => {
                    Ok(ProxyConstructStep::Complete(Completion::Throw(value)))
                }
                NativeConversion::Value(()) => Err(RuntimeError::Invariant(
                    "revoked Proxy construct returned a value",
                )),
            };
        }
        let rooted = runtime.root_proxy_snapshot(proxy.as_object(), data)?;
        Ok(ProxyConstructStep::request_read(
            rooted.handler.clone(),
            self.key.clone(),
            ProxyConstructResume(Box::new(ProxyConstructResumeState {
                pending_effect: ProxyConstructStepPending::default(),
                phase: Phase::Method {
                    rooted,
                    search: self,
                },
            })),
        ))
    }
}
impl ProxyConstructResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<ProxyConstructStep, RuntimeError> {
        let (rooted, mut search) = match self.0.phase {
            Phase::Method { rooted, search } => (rooted, search),
            Phase::Result { realm, trap, .. } => {
                return Ok(ProxyConstructStep::Complete(match completion {
                    Completion::Return(value) if trap && !matches!(value, Value::Object(_)) => {
                        Completion::Throw(runtime.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "not an object",
                        )?)
                    }
                    result => result,
                }));
            }
        };
        let method = match completion {
            Completion::Return(value) => value,
            result @ Completion::Throw(_) => return Ok(ProxyConstructStep::Complete(result)),
        };
        // Validate the immediate target after Get(trap), even for missing traps.
        let target = match runtime
            .constructor_from_value(search.realm, Value::Object(rooted.target.clone()))?
        {
            NativeConversion::Value(target) => target,
            NativeConversion::Throw(value) => {
                return Ok(ProxyConstructStep::Complete(Completion::Throw(value)));
            }
        };
        if matches!(method, Value::Null | Value::Undefined) {
            if runtime.is_proxy_object(target.as_object())? {
                search.depth = search.depth.saturating_add(1);
                return search.read(runtime, target);
            }
            return Ok(ProxyConstructStep::request_construct(
                target,
                search.new_target,
                search.arguments,
                Self(Box::new(ProxyConstructResumeState {
                    pending_effect: ProxyConstructStepPending::default(),
                    phase: Phase::Result {
                        realm: search.realm,
                        trap: false,
                        _rooted: rooted,
                        _guard: search.guard,
                    },
                })),
            ));
        }
        let array = runtime.new_array_from_values(search.realm, search.arguments)?;
        let method = match runtime.direct_call_target_from_value(method) {
            Ok(method) => method,
            Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Type => {
                return Ok(ProxyConstructStep::Complete(Completion::Throw(
                    runtime.new_native_error_from_error(
                        search.realm,
                        NativeErrorKind::Type,
                        &error,
                    )?,
                )));
            }
            Err(error) => return Err(error),
        };
        Ok(ProxyConstructStep::request_call(
            method,
            Value::Object(rooted.handler.clone()),
            vec![
                Value::Object(rooted.target.clone()),
                Value::Object(array),
                search.new_target.value(),
            ],
            Self(Box::new(ProxyConstructResumeState {
                pending_effect: ProxyConstructStepPending::default(),
                phase: Phase::Result {
                    realm: search.realm,
                    trap: true,
                    _rooted: rooted,
                    _guard: search.guard,
                },
            })),
        ))
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ProxyConstructStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            ProxyConstructStep::Complete(result) => return Ok(result),
            ProxyConstructStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.internal_get(realm, &object, &key, Value::Object(object.clone()))?,
                )?
            }
            ProxyConstructStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                {
                    let result = match target {
                        DirectCallTarget::Callable(target) => {
                            runtime.call_internal(realm, &target, receiver, &arguments)?
                        }
                        DirectCallTarget::NonCallableProxy(proxy) => {
                            runtime.call_proxy(realm, &proxy, receiver, &arguments)?
                        }
                    };
                    resume.resume(runtime, result)?
                }
            }
            ProxyConstructStep::Construct { mut resume } => {
                let target = resume.take_construct_target();
                let new_target = resume.take_construct_new_target();
                let arguments = resume.take_construct_arguments();
                resume.resume(
                    runtime,
                    runtime.construct_internal_with_new_target(
                        realm, &target, new_target, &arguments,
                    )?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct ProxyConstructStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    construct_target: Option<ConstructorRef>,
    construct_new_target: Option<ConstructNewTarget>,
    construct_arguments: Option<Vec<Value>>,
}
impl ProxyConstructStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ProxyConstructResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        target: DirectCallTarget,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: ProxyConstructResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_construct(
        target: ConstructorRef,
        new_target: ConstructNewTarget,
        arguments: Vec<Value>,
        mut resume: ProxyConstructResume,
    ) -> Self {
        resume.0.pending_effect.construct_target = Some(target);
        resume.0.pending_effect.construct_new_target = Some(new_target);
        resume.0.pending_effect.construct_arguments = Some(arguments);
        Self::Construct { resume }
    }
}
impl ProxyConstructResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ProxyConstructStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ProxyConstructStep Read key")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("ProxyConstructStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ProxyConstructStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ProxyConstructStep Call arguments")
    }
    pub(crate) fn take_construct_target(&mut self) -> ConstructorRef {
        self.0
            .pending_effect
            .construct_target
            .take()
            .expect("ProxyConstructStep Construct target")
    }
    pub(crate) fn take_construct_new_target(&mut self) -> ConstructNewTarget {
        self.0
            .pending_effect
            .construct_new_target
            .take()
            .expect("ProxyConstructStep Construct new_target")
    }
    pub(crate) fn take_construct_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .construct_arguments
            .take()
            .expect("ProxyConstructStep Construct arguments")
    }
}
const _: () = assert!(std::mem::size_of::<ProxyConstructStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ProxyConstructStep>() <= 64);
