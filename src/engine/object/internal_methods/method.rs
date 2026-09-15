//! One get_proxy_method protocol for all Proxy operations.
//! Method lookup owns its depth guard until the selected method is delivered.
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

pub(super) enum MethodStep {
    Complete { resume: MethodResume },
    Throw(Value),
    Read { resume: MethodResume },
}

pub(super) struct MethodResume(Box<MethodResumeState>);
impl std::ops::Deref for MethodResume {
    type Target = MethodResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for MethodResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<MethodResume>() <= 8);
pub(super) struct MethodResumeState {
    pending_effect: MethodStepPending,
    rooted: Option<RootedProxy>,
    selected: Option<DirectCallTarget>,
    search: Search,
}

struct Search {
    realm: ContextId,
    key: PropertyKey,
    limit: Option<usize>,
    depth: usize,
    _guard: ProxyMethodStackGuard,
}

impl MethodStep {
    pub(super) fn start(
        runtime: &Runtime,
        realm: ContextId,
        proxy: ObjectRef,
        name: &'static str,
    ) -> Result<Self, RuntimeError> {
        if runtime.proxy_method_stack_would_overflow() {
            return overflow(runtime, realm);
        }
        let guard = ProxyMethodStackGuard::enter(runtime);
        let key = runtime.intern_property_key(name)?;
        Search {
            realm,
            key,
            limit: runtime.proxy_method_chain_limit(name),
            depth: 0,
            _guard: guard,
        }
        .read(runtime, proxy)
    }
}

fn overflow(runtime: &Runtime, realm: ContextId) -> Result<MethodStep, RuntimeError> {
    Ok(MethodStep::Throw(runtime.new_native_error(
        realm,
        NativeErrorKind::Internal,
        "stack overflow",
    )?))
}

impl Search {
    fn read(self, runtime: &Runtime, proxy: ObjectRef) -> Result<MethodStep, RuntimeError> {
        if self.limit.is_some_and(|limit| self.depth == limit) {
            return overflow(runtime, self.realm);
        }
        let data = runtime
            .proxy_snapshot_if_any(&proxy)?
            .ok_or(RuntimeError::Invariant(
                "Proxy method dispatch reached an ordinary object",
            ))?;
        if data.is_revoked {
            let NativeConversion::Throw(value) = runtime.proxy_revoked_throw::<()>(self.realm)?
            else {
                unreachable!("revoked proxy throws")
            };
            return Ok(MethodStep::Throw(value));
        }
        let rooted = runtime.root_proxy_snapshot(&proxy, data)?;
        Ok(MethodStep::request_read(
            rooted.handler.clone(),
            self.key.clone(),
            Value::Object(rooted.handler.clone()),
            MethodResume(Box::new(MethodResumeState {
                pending_effect: MethodStepPending::default(),
                rooted: Some(rooted),
                selected: None,
                search: self,
            })),
        ))
    }
}

impl MethodResume {
    pub(super) fn take_completed_rooted(&mut self) -> RootedProxy {
        self.0.rooted.take().expect("completed proxy owner")
    }
    pub(super) fn take_completed_target(&mut self) -> Option<DirectCallTarget> {
        self.0.selected.take()
    }
    pub(super) fn resume(
        mut self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<MethodStep, RuntimeError> {
        let value = match completion {
            Completion::Throw(value) => return Ok(MethodStep::Throw(value)),
            Completion::Return(value) => value,
        };
        if matches!(value, Value::Undefined | Value::Null) {
            let rooted = self.0.rooted.as_ref().expect("proxy owner");
            if let Some(data) = runtime.proxy_snapshot_if_any(&rooted.target)? {
                self.0.search.depth = self.0.search.depth.saturating_add(1);
                if self
                    .0
                    .search
                    .limit
                    .is_some_and(|limit| self.0.search.depth == limit)
                {
                    return overflow(runtime, self.0.search.realm);
                }
                if data.is_revoked {
                    let NativeConversion::Throw(value) =
                        runtime.proxy_revoked_throw::<()>(self.0.search.realm)?
                    else {
                        unreachable!("revoked proxy throws")
                    };
                    return Ok(MethodStep::Throw(value));
                }
                let next = runtime.root_proxy_snapshot(&rooted.target, data)?;
                let old = self.0.rooted.replace(next);
                let rooted = self.0.rooted.as_ref().unwrap();
                let object = rooted.handler.clone();
                let receiver = Value::Object(rooted.handler.clone());
                let key = self.0.search.key.clone();
                let step = MethodStep::request_read(object, key, receiver, self);
                drop(old);
                return Ok(step);
            }
            return Ok(MethodStep::Complete { resume: self });
        }
        let method = match runtime.direct_call_target_from_value(value) {
            Ok(method) => method,
            Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Type => {
                return Ok(MethodStep::Throw(runtime.new_native_error_from_error(
                    self.0.search.realm,
                    NativeErrorKind::Type,
                    &error,
                )?));
            }
            Err(error) => return Err(error),
        };
        self.0.selected = Some(method);
        Ok(MethodStep::Complete { resume: self })
    }
}

#[derive(Default)]
struct MethodStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    read_receiver: Option<Value>,
}
impl MethodStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        receiver: Value,
        mut resume: MethodResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        resume.0.pending_effect.read_receiver = Some(receiver);
        Self::Read { resume }
    }
}
impl MethodResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("MethodStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("MethodStep Read key")
    }
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("MethodStep Read receiver")
    }
}
const _: () = assert!(std::mem::size_of::<MethodStep>() <= 64);

#[cfg(test)]
mod resident_tests {
    use super::*;
    #[test]
    fn proxy_method_completion_reuses_the_pending_owner() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(proxy) = context.eval("new Proxy({}, {})").unwrap() else {
            panic!("proxy")
        };
        let MethodStep::Read { mut resume } =
            MethodStep::start(&runtime, context.realm, proxy.clone(), "get").unwrap()
        else {
            panic!("read")
        };
        let address = (&*resume.0) as *const MethodResumeState;
        drop(resume.take_read_object());
        drop(resume.take_read_key());
        drop(resume.take_read_receiver());
        let MethodStep::Complete { mut resume } = resume
            .resume(&runtime, Completion::Return(Value::Undefined))
            .unwrap()
        else {
            panic!("complete")
        };
        assert_eq!((&*resume.0) as *const MethodResumeState, address);
        assert_eq!(resume.take_completed_rooted().proxy, proxy);
        assert!(resume.take_completed_target().is_none());
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<MethodStep>() <= 64);
