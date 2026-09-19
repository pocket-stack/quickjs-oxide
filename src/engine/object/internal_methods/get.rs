//! Proxy [[Get]] phases shared by synchronous entry and the owned VM driver.
//! Each continuation owns the target/handler selected before observable work.
use super::{
    RootedProxy,
    method::{MethodResume, MethodStep},
};
use crate::engine::api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::heap::ContextId;
use crate::engine::object::{CompleteOrdinaryPropertyDescriptor, ObjectRef, PropertyKey};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{Completion, call::DirectCallTarget};

pub(crate) enum ProxyGetStep {
    Complete(Completion),
    Read { resume: ProxyGetResume },
    Call { resume: ProxyGetResume },
    Descriptor { resume: ProxyGetResume },
}

pub(crate) struct ProxyGetResume(super::reuse::PooledBox<ProxyGetResumeState>);
impl std::ops::Deref for ProxyGetResume {
    type Target = ProxyGetResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ProxyGetResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
thread_local! {
    // Preserve and reuse the continuation Box allocation across callbacks.
    #[allow(clippy::vec_box)]
    static EMPTY_CONTINUATIONS: std::cell::RefCell<Vec<Box<Option<ProxyGetResumeState>>>> = const { std::cell::RefCell::new(Vec::new()) };
}
impl super::reuse::Reusable for ProxyGetResumeState {
    #[cfg(feature = "profiling")]
    const EVENT: &'static str = "get_resume_allocation";
    fn pool() -> &'static super::reuse::EmptyPool<Self> {
        &EMPTY_CONTINUATIONS
    }
}
const _: () = assert!(std::mem::size_of::<ProxyGetResume>() <= 8);
pub(crate) struct ProxyGetResumeState {
    pending_effect: ProxyGetStepPending,
    realm: ContextId,
    phase: Phase,
}

enum Phase {
    Method {
        resume: MethodResume,
        key: PropertyKey,
        receiver: Value,
        arguments: Vec<Value>,
    },
    Forward {
        _rooted: RootedProxy,
    },
    Trap {
        rooted: RootedProxy,
        key: PropertyKey,
    },
    Invariant {
        _rooted: RootedProxy,
        result: Value,
    },
}

impl ProxyGetStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        proxy: ObjectRef,
        key: PropertyKey,
        receiver: Value,
    ) -> Result<Self, RuntimeError> {
        Self::start_buffered(runtime, realm, proxy, key, receiver, Vec::new())
    }
    pub(crate) fn start_buffered(
        runtime: &Runtime,
        realm: ContextId,
        proxy: ObjectRef,
        key: PropertyKey,
        receiver: Value,
        arguments: Vec<Value>,
    ) -> Result<Self, RuntimeError> {
        debug_assert!(arguments.is_empty());
        runtime.validate_object_and_key(&proxy, &key)?;
        runtime.validate_value_domain(&receiver, "property receiver")?;
        let step = MethodStep::start(runtime, realm, proxy, "get")?;
        method(runtime, realm, key, receiver, arguments, step)
    }
}

fn method(
    runtime: &Runtime,
    realm: ContextId,
    key: PropertyKey,
    receiver: Value,
    mut arguments: Vec<Value>,
    step: MethodStep,
) -> Result<ProxyGetStep, RuntimeError> {
    Ok(match step {
        MethodStep::Throw(value) => ProxyGetStep::Complete(Completion::Throw(value)),
        MethodStep::Complete { mut resume } => {
            let rooted = resume.take_completed_rooted();
            let target = resume.take_completed_target();
            drop(resume);
            match target {
                None => ProxyGetStep::request_read(
                    rooted.target.clone(),
                    key,
                    receiver,
                    ProxyGetResume(super::reuse::PooledBox::new(ProxyGetResumeState {
                        pending_effect: ProxyGetStepPending::default(),
                        realm,
                        phase: Phase::Forward { _rooted: rooted },
                    })),
                ),
                Some(target) => {
                    let key_value = runtime.property_key_value(&key)?;
                    arguments.extend([Value::Object(rooted.target.clone()), key_value, receiver]);
                    ProxyGetStep::request_call(
                        target,
                        Value::Object(rooted.handler.clone()),
                        arguments,
                        ProxyGetResume(super::reuse::PooledBox::new(ProxyGetResumeState {
                            pending_effect: ProxyGetStepPending::default(),
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
            let method_receiver = resume.take_read_receiver();
            ProxyGetStep::request_read(
                object,
                method_key,
                method_receiver,
                ProxyGetResume(super::reuse::PooledBox::new(ProxyGetResumeState {
                    pending_effect: ProxyGetStepPending::default(),
                    realm,
                    phase: Phase::Method {
                        resume,
                        key,
                        receiver,
                        arguments,
                    },
                })),
            )
        }
    })
}

impl ProxyGetResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<ProxyGetStep, RuntimeError> {
        let Completion::Return(value) = completion else {
            return Ok(ProxyGetStep::Complete(completion));
        };
        let state = self.0.into_inner();
        let realm = state.realm;
        match state.phase {
            Phase::Method {
                resume,
                key,
                receiver,
                arguments,
            } => method(
                runtime,
                realm,
                key,
                receiver,
                arguments,
                resume.resume(runtime, Completion::Return(value))?,
            ),
            Phase::Forward { .. } => Ok(ProxyGetStep::Complete(Completion::Return(value))),
            Phase::Trap { rooted, key } => {
                // An ordinary target's [[GetOwnProperty]] is synchronous, so the
                // invariant comparison runs in place instead of a full scheduler
                // descriptor round. A Proxy target may re-enter JavaScript and
                // keeps the descriptor round.
                if runtime.is_proxy_object(&rooted.target)? {
                    return Ok(ProxyGetStep::request_descriptor(
                        rooted.target.clone(),
                        key,
                        Self(super::reuse::PooledBox::new(ProxyGetResumeState {
                            pending_effect: ProxyGetStepPending::default(),
                            realm,
                            phase: Phase::Invariant {
                                _rooted: rooted,
                                result: value,
                            },
                        })),
                    ));
                }
                let descriptor =
                    NativeConversion::Value(runtime.get_own_property(&rooted.target, &key)?);
                complete_get_invariant(runtime, realm, value, descriptor)
            }
            Phase::Invariant { .. } => Err(RuntimeError::Invariant(
                "Proxy Get descriptor continuation received a value reply",
            )),
        }
    }

    pub(crate) fn descriptor(
        self,
        runtime: &Runtime,
        descriptor: NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>,
    ) -> Result<ProxyGetStep, RuntimeError> {
        let state = self.0.into_inner();
        let Phase::Invariant { _rooted, result } = state.phase else {
            return Err(RuntimeError::Invariant(
                "Proxy Get value continuation received a descriptor reply",
            ));
        };
        complete_get_invariant(runtime, state.realm, result, descriptor)
    }
}

/// Shared `[[Get]]` invariant test: a non-configurable, non-writable data
/// property must return the same value, and a setter-less non-configurable
/// accessor must return `undefined`.
fn get_invariant_violation(
    result: &Value,
    descriptor: &Option<CompleteOrdinaryPropertyDescriptor>,
) -> bool {
    match descriptor {
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable: false,
            configurable: false,
            ..
        }) => !result.same_value(value),
        Some(CompleteOrdinaryPropertyDescriptor::Accessor {
            get: None,
            configurable: false,
            ..
        }) => !matches!(result, Value::Undefined),
        _ => false,
    }
}

fn complete_get_invariant(
    runtime: &Runtime,
    realm: ContextId,
    result: Value,
    descriptor: NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>,
) -> Result<ProxyGetStep, RuntimeError> {
    let descriptor = match descriptor {
        NativeConversion::Value(descriptor) => descriptor,
        NativeConversion::Throw(value) => {
            return Ok(ProxyGetStep::Complete(Completion::Throw(value)));
        }
    };
    Ok(ProxyGetStep::Complete(
        if get_invariant_violation(&result, &descriptor) {
            Completion::Throw(runtime.new_native_error_jsvalue(
                realm,
                NativeErrorKind::Type,
                "proxy: inconsistent get",
            )?)
        } else {
            Completion::Return(result)
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abandoned_proxy_method_releases_roots_and_depth_guard() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let context = runtime.new_context();
        let target = runtime.new_object(None).unwrap();
        let handler = runtime.new_object(None).unwrap();
        let target_id = target.object_id();
        let handler_id = handler.object_id();
        let NativeConversion::Value(proxy) = runtime
            .new_proxy(context.realm, Value::Object(target), Value::Object(handler))
            .unwrap()
        else {
            panic!("proxy allocation failed");
        };
        let step = ProxyGetStep::start(
            &runtime,
            context.realm,
            proxy,
            runtime.intern_property_key("x").unwrap(),
            Value::Undefined,
        )
        .unwrap();
        assert_eq!(runtime.0.proxy_method_depth.get(), 1);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(target_id).is_ok());
        assert!(runtime.0.state.borrow().heap.object(handler_id).is_ok());
        drop(step);
        assert_eq!(runtime.0.proxy_method_depth.get(), 0);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(target_id).is_err());
        assert!(runtime.0.state.borrow().heap.object(handler_id).is_err());
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn mismatched_proxy_reply_rejects_and_releases_the_method_guard() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let NativeConversion::Value(proxy) = runtime
            .new_proxy(
                context.realm,
                Value::Object(runtime.new_object(None).unwrap()),
                Value::Object(runtime.new_object(None).unwrap()),
            )
            .unwrap()
        else {
            panic!("proxy allocation failed");
        };
        let ProxyGetStep::Read { resume, .. } = ProxyGetStep::start(
            &runtime,
            context.realm,
            proxy,
            runtime.intern_property_key("x").unwrap(),
            Value::Undefined,
        )
        .unwrap() else {
            panic!("expected method read");
        };
        assert_eq!(runtime.0.proxy_method_depth.get(), 1);
        assert!(
            resume
                .descriptor(&runtime, NativeConversion::Value(None))
                .is_err()
        );
        assert_eq!(runtime.0.proxy_method_depth.get(), 0);
    }
}

#[derive(Default)]
struct ProxyGetStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    read_receiver: Option<Value>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    descriptor_object: Option<ObjectRef>,
    descriptor_key: Option<PropertyKey>,
}
impl ProxyGetStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        receiver: Value,
        mut resume: ProxyGetResume,
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
        mut resume: ProxyGetResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_descriptor(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ProxyGetResume,
    ) -> Self {
        resume.0.pending_effect.descriptor_object = Some(object);
        resume.0.pending_effect.descriptor_key = Some(key);
        Self::Descriptor { resume }
    }
}
impl ProxyGetResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ProxyGetStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ProxyGetStep Read key")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("ProxyGetStep Read receiver")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("ProxyGetStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ProxyGetStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ProxyGetStep Call arguments")
    }
    pub(crate) fn take_descriptor_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .descriptor_object
            .take()
            .expect("ProxyGetStep Descriptor object")
    }
    pub(crate) fn take_descriptor_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .descriptor_key
            .take()
            .expect("ProxyGetStep Descriptor key")
    }
}
const _: () = assert!(std::mem::size_of::<ProxyGetStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ProxyGetStep>() <= 64);
