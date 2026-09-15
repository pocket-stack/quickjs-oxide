//! Proxy [[Call]] owns the trap lookup and the final invocation separately.
use super::{ProxyMethodStackGuard, RootedProxy};
use crate::engine::api::{
    error::{ErrorKind, NativeErrorKind},
    runtime::Runtime,
    runtime_error::RuntimeError,
};
use crate::engine::heap::ContextId;
use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{Completion, call::DirectCallTarget};

pub(crate) enum ProxyCallStep {
    Complete(Completion),
    Read { resume: ProxyCallResume },
    Call { resume: ProxyCallResume },
}

pub(crate) struct ProxyCallResume(Box<ProxyCallResumeState>);
impl std::ops::Deref for ProxyCallResume {
    type Target = ProxyCallResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ProxyCallResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ProxyCallResume>() <= 8);
pub(crate) struct ProxyCallResumeState {
    pending_effect: ProxyCallStepPending,
    phase: Phase,
}
enum Phase {
    Method {
        rooted: RootedProxy,
        search: Search,
    },
    Result {
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
    receiver: Value,
    arguments: Vec<Value>,
}

fn overflow(runtime: &Runtime, realm: ContextId) -> Result<ProxyCallStep, RuntimeError> {
    Ok(ProxyCallStep::Complete(Completion::Throw(
        runtime.new_native_error(realm, NativeErrorKind::Internal, "stack overflow")?,
    )))
}

impl ProxyCallStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        proxy: ObjectRef,
        receiver: Value,
        arguments: Vec<Value>,
    ) -> Result<Self, RuntimeError> {
        if runtime.proxy_method_stack_would_overflow() {
            return overflow(runtime, realm);
        }
        let guard = ProxyMethodStackGuard::enter(runtime);
        Search {
            realm,
            key: runtime.intern_property_key("apply")?,
            limit: runtime.proxy_method_chain_limit("apply"),
            depth: 0,
            guard,
            receiver,
            arguments,
        }
        .read(runtime, proxy)
    }
}

impl Search {
    fn read(self, runtime: &Runtime, proxy: ObjectRef) -> Result<ProxyCallStep, RuntimeError> {
        if self.limit.is_some_and(|limit| self.depth == limit) {
            return overflow(runtime, self.realm);
        }
        let data = runtime
            .proxy_snapshot_if_any(&proxy)?
            .ok_or(RuntimeError::Invariant(
                "Proxy call dispatch reached an ordinary object",
            ))?;
        if data.is_revoked {
            return match runtime.proxy_revoked_throw(self.realm)? {
                NativeConversion::Throw(value) => {
                    Ok(ProxyCallStep::Complete(Completion::Throw(value)))
                }
                NativeConversion::Value(()) => Err(RuntimeError::Invariant(
                    "revoked Proxy call returned a value",
                )),
            };
        }
        let rooted = runtime.root_proxy_snapshot(&proxy, data)?;
        Ok(ProxyCallStep::request_read(
            rooted.handler.clone(),
            self.key.clone(),
            Value::Object(rooted.handler.clone()),
            ProxyCallResume(Box::new(ProxyCallResumeState {
                pending_effect: ProxyCallStepPending::default(),
                phase: Phase::Method {
                    rooted,
                    search: self,
                },
            })),
        ))
    }
}

impl ProxyCallResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<ProxyCallStep, RuntimeError> {
        let Phase::Method { rooted, mut search } = self.0.phase else {
            return Ok(ProxyCallStep::Complete(completion));
        };
        let method = match completion {
            Completion::Throw(value) => {
                return Ok(ProxyCallStep::Complete(Completion::Throw(value)));
            }
            Completion::Return(value) => value,
        };
        // Pinned callability validation occurs after the observable trap Get.
        if !rooted.data.is_callable {
            return Ok(ProxyCallStep::Complete(Completion::Throw(
                runtime.new_native_error(search.realm, NativeErrorKind::Type, "not a function")?,
            )));
        }
        let (target, receiver, arguments) = if matches!(method, Value::Undefined | Value::Null) {
            if runtime.is_proxy_object(&rooted.target)? {
                search.depth = search.depth.saturating_add(1);
                return search.read(runtime, rooted.target);
            }
            (
                runtime.direct_call_target_from_value(Value::Object(rooted.target.clone()))?,
                search.receiver,
                search.arguments,
            )
        } else {
            // Allocate the argument array before validating the trap, as in C.
            let array = runtime.new_array_from_values(search.realm, search.arguments)?;
            let method = match runtime.direct_call_target_from_value(method) {
                Ok(method) => method,
                Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Type => {
                    return Ok(ProxyCallStep::Complete(Completion::Throw(
                        runtime.new_native_error_from_error(
                            search.realm,
                            NativeErrorKind::Type,
                            &error,
                        )?,
                    )));
                }
                Err(error) => return Err(error),
            };
            (
                method,
                Value::Object(rooted.handler.clone()),
                vec![
                    Value::Object(rooted.target.clone()),
                    search.receiver,
                    Value::Object(array),
                ],
            )
        };
        Ok(ProxyCallStep::request_call(
            target,
            receiver,
            arguments,
            Self(Box::new(ProxyCallResumeState {
                pending_effect: ProxyCallStepPending::default(),
                phase: Phase::Result {
                    _rooted: rooted,
                    _guard: search.guard,
                },
            })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take_read(step: ProxyCallStep) -> ProxyCallResume {
        let ProxyCallStep::Read { resume, .. } = step else {
            panic!("expected apply lookup")
        };
        resume
    }

    #[test]
    fn abandoned_proxy_call_releases_roots_and_guard_in_both_phases() {
        for after_lookup in [false, true] {
            let runtime = Runtime::new();
            let weak = std::rc::Rc::downgrade(&runtime.0);
            let mut context = runtime.new_context();
            let target = context.eval("(function(){return 42})").unwrap();
            let handler = runtime.new_object(None).unwrap();
            let handler_id = handler.object_id();
            let receiver = runtime.new_object(None).unwrap();
            let receiver_id = receiver.object_id();
            let argument = runtime.new_object(None).unwrap();
            let argument_id = argument.object_id();
            let NativeConversion::Value(proxy) = runtime
                .new_proxy(context.realm, target, Value::Object(handler))
                .unwrap()
            else {
                panic!("proxy allocation failed")
            };
            let mut step = ProxyCallStep::start(
                &runtime,
                context.realm,
                proxy,
                Value::Object(receiver),
                vec![Value::Object(argument)],
            )
            .unwrap();
            if after_lookup {
                step = take_read(step)
                    .resume(&runtime, Completion::Return(Value::Undefined))
                    .unwrap();
            }
            assert_eq!(runtime.0.proxy_method_depth.get(), 1);
            runtime.run_gc().unwrap();
            for id in [handler_id, receiver_id, argument_id] {
                assert!(runtime.0.state.borrow().heap.object(id).is_ok());
            }
            drop(step);
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            runtime.run_gc().unwrap();
            for id in [handler_id, receiver_id, argument_id] {
                assert!(runtime.0.state.borrow().heap.object(id).is_err());
            }
            drop(context);
            drop(runtime);
            assert!(weak.upgrade().is_none());
        }
    }
}

#[derive(Default)]
struct ProxyCallStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    read_receiver: Option<Value>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
}
impl ProxyCallStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        receiver: Value,
        mut resume: ProxyCallResume,
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
        mut resume: ProxyCallResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
}
impl ProxyCallResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ProxyCallStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ProxyCallStep Read key")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("ProxyCallStep Read receiver")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("ProxyCallStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ProxyCallStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ProxyCallStep Call arguments")
    }
}
const _: () = assert!(std::mem::size_of::<ProxyCallStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ProxyCallStep>() <= 64);
