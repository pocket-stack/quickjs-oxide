//! Proxy ownKeys preserves list conversion, duplicate detection and invariant order.
use super::{
    RootedProxy,
    method::{MethodResume, MethodStep},
};
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    atom::Atom,
    heap::ContextId,
    object::{CompleteOrdinaryPropertyDescriptor, ObjectRef, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::{Completion, call::DirectCallTarget},
};
use std::collections::HashSet;

pub(crate) enum KeysStep {
    Complete(NativeConversion<Vec<PropertyKey>>),
    Read { resume: KeysResume },
    Call { resume: KeysResume },
    Number { resume: KeysResume },
    Keys { resume: KeysResume },
    Extensible { resume: KeysResume },
    Descriptor { resume: KeysResume },
}
pub(crate) struct KeysResume(Box<KeysResumeState>);
impl std::ops::Deref for KeysResume {
    type Target = KeysResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for KeysResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<KeysResume>() <= 8);
pub(crate) struct KeysResumeState {
    pending_effect: KeysStepPending,
    realm: ContextId,
    phase: Phase,
}
enum Phase {
    Method(MethodResume),
    Forward(RootedProxy),
    Trap(RootedProxy),
    Length {
        rooted: RootedProxy,
        list: Value,
    },
    Number {
        rooted: RootedProxy,
        list: Value,
    },
    Item {
        rooted: RootedProxy,
        list: Value,
        length: u32,
        keys: Vec<PropertyKey>,
    },
    Extensible {
        rooted: RootedProxy,
        keys: Vec<PropertyKey>,
        atoms: HashSet<Atom>,
    },
    TargetKeys {
        rooted: RootedProxy,
        keys: Vec<PropertyKey>,
        atoms: HashSet<Atom>,
        extensible: bool,
    },
    Descriptor {
        state: Check,
        key: PropertyKey,
    },
}
struct Check {
    rooted: RootedProxy,
    keys: Vec<PropertyKey>,
    atoms: HashSet<Atom>,
    extensible: bool,
    remaining: std::vec::IntoIter<PropertyKey>,
}
impl KeysStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
    ) -> Result<Self, RuntimeError> {
        method(realm, MethodStep::start(runtime, realm, object, "ownKeys")?)
    }
}
fn method(realm: ContextId, step: MethodStep) -> Result<KeysStep, RuntimeError> {
    Ok(match step {
        MethodStep::Read { mut resume } => {
            let object = resume.take_read_object();
            let key = resume.take_read_key();
            let _ = resume.take_read_receiver();
            KeysStep::request_read(
                Value::Object(object),
                key,
                KeysResume(Box::new(KeysResumeState {
                    pending_effect: KeysStepPending::default(),
                    realm,
                    phase: Phase::Method(resume),
                })),
            )
        }
        MethodStep::Throw(value) => KeysStep::Complete(NativeConversion::Throw(value)),
        MethodStep::Complete { mut resume } => {
            let rooted = resume.take_completed_rooted();
            let target = resume.take_completed_target();
            drop(resume);
            match target {
                None => KeysStep::request_keys(
                    rooted.target.clone(),
                    KeysResume(Box::new(KeysResumeState {
                        pending_effect: KeysStepPending::default(),
                        realm,
                        phase: Phase::Forward(rooted),
                    })),
                ),
                Some(target) => KeysStep::request_call(
                    target,
                    Value::Object(rooted.handler.clone()),
                    vec![Value::Object(rooted.target.clone())],
                    KeysResume(Box::new(KeysResumeState {
                        pending_effect: KeysStepPending::default(),
                        realm,
                        phase: Phase::Trap(rooted),
                    })),
                ),
            }
        }
    })
}
fn fail(runtime: &Runtime, realm: ContextId, message: &str) -> Result<KeysStep, RuntimeError> {
    Ok(KeysStep::Complete(NativeConversion::Throw(
        runtime.new_native_error(realm, NativeErrorKind::Type, message)?,
    )))
}
fn items(
    runtime: &Runtime,
    realm: ContextId,
    rooted: RootedProxy,
    list: Value,
    length: u32,
    keys: Vec<PropertyKey>,
) -> Result<KeysStep, RuntimeError> {
    if keys.len() < length as usize {
        let key = runtime.intern_property_key(&keys.len().to_string())?;
        return Ok(KeysStep::request_read(
            list.clone(),
            key,
            KeysResume(Box::new(KeysResumeState {
                pending_effect: KeysStepPending::default(),
                realm,
                phase: Phase::Item {
                    rooted,
                    list,
                    length,
                    keys,
                },
            })),
        ));
    }
    // Pinned QuickJS reads every list element before it reports duplicate keys.
    let mut atoms = HashSet::new();
    atoms
        .try_reserve(keys.len())
        .map_err(|_| RuntimeError::Invariant("Proxy ownKeys set allocation failed"))?;
    for key in &keys {
        if !atoms.insert(key.atom()) {
            return fail(runtime, realm, "proxy: duplicate property");
        }
    }
    Ok(KeysStep::request_extensible(
        rooted.target.clone(),
        KeysResume(Box::new(KeysResumeState {
            pending_effect: KeysStepPending::default(),
            realm,
            phase: Phase::Extensible {
                rooted,
                keys,
                atoms,
            },
        })),
    ))
}
fn check_next(
    runtime: &Runtime,
    realm: ContextId,
    mut state: Check,
) -> Result<KeysStep, RuntimeError> {
    if let Some(key) = state.remaining.next() {
        if runtime.proxy_is_revoked(&state.rooted.proxy)? {
            return Ok(KeysStep::Complete(runtime.proxy_revoked_throw(realm)?));
        }
        return Ok(KeysStep::request_descriptor(
            state.rooted.target.clone(),
            key.clone(),
            KeysResume(Box::new(KeysResumeState {
                pending_effect: KeysStepPending::default(),
                realm,
                phase: Phase::Descriptor { state, key },
            })),
        ));
    }
    if !state.extensible && !state.atoms.is_empty() {
        return fail(
            runtime,
            realm,
            "proxy: property not present in target were returned by non extensible proxy",
        );
    }
    Ok(KeysStep::Complete(NativeConversion::Value(state.keys)))
}
impl KeysResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<KeysStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(KeysStep::Complete(NativeConversion::Throw(value)));
            }
        };
        let realm = self.0.realm;
        match self.0.phase {
            Phase::Method(resume) => {
                method(realm, resume.resume(runtime, Completion::Return(value))?)
            }
            Phase::Trap(rooted) => Ok(KeysStep::request_read(
                value.clone(),
                runtime.intern_property_key("length")?,
                Self(Box::new(KeysResumeState {
                    pending_effect: KeysStepPending::default(),
                    realm,
                    phase: Phase::Length {
                        rooted,
                        list: value,
                    },
                })),
            )),
            Phase::Length { rooted, list } => Ok(KeysStep::request_number(
                value,
                Self(Box::new(KeysResumeState {
                    pending_effect: KeysStepPending::default(),
                    realm,
                    phase: Phase::Number { rooted, list },
                })),
            )),
            Phase::Item {
                rooted,
                list,
                length,
                mut keys,
            } => {
                let key = match value {
                    Value::String(value) => runtime.intern_property_key_js_string(&value)?,
                    Value::Symbol(value) => PropertyKey::from(value),
                    _ => {
                        return fail(
                            runtime,
                            realm,
                            "proxy: properties must be strings or symbols",
                        );
                    }
                };
                keys.push(key);
                items(runtime, realm, rooted, list, length, keys)
            }
            _ => Err(RuntimeError::Invariant("ownKeys received an untyped reply")),
        }
    }
    pub(crate) fn number(
        self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<KeysStep, RuntimeError> {
        let Phase::Number { rooted, list } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "ownKeys received unexpected numeric reply",
            ));
        };
        let length = match result {
            NativeConversion::Value(value) => Runtime::to_uint32_number(value),
            NativeConversion::Throw(value) => {
                return Ok(KeysStep::Complete(NativeConversion::Throw(value)));
            }
        };
        let mut keys = Vec::new();
        keys.try_reserve_exact(length as usize)
            .map_err(|_| RuntimeError::Invariant("Proxy ownKeys list allocation failed"))?;
        items(runtime, self.0.realm, rooted, list, length, keys)
    }
    pub(crate) fn boolean(
        self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<KeysStep, RuntimeError> {
        let Phase::Extensible {
            rooted,
            keys,
            atoms,
        } = self.0.phase
        else {
            return Err(RuntimeError::Invariant(
                "ownKeys received unexpected boolean reply",
            ));
        };
        let extensible = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(KeysStep::Complete(NativeConversion::Throw(value)));
            }
        };
        if runtime.proxy_is_revoked(&rooted.proxy)? {
            return Ok(KeysStep::Complete(
                runtime.proxy_revoked_throw(self.0.realm)?,
            ));
        }
        Ok(KeysStep::request_keys(
            rooted.target.clone(),
            Self(Box::new(KeysResumeState {
                pending_effect: KeysStepPending::default(),
                realm: self.0.realm,
                phase: Phase::TargetKeys {
                    rooted,
                    keys,
                    atoms,
                    extensible,
                },
            })),
        ))
    }
    pub(crate) fn keys(
        self,
        runtime: &Runtime,
        result: NativeConversion<Vec<PropertyKey>>,
    ) -> Result<KeysStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(KeysStep::Complete(NativeConversion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Forward(_rooted) => Ok(KeysStep::Complete(NativeConversion::Value(value))),
            Phase::TargetKeys {
                rooted,
                keys,
                atoms,
                extensible,
            } => check_next(
                runtime,
                self.0.realm,
                Check {
                    rooted,
                    keys,
                    atoms,
                    extensible,
                    remaining: value.into_iter(),
                },
            ),
            _ => Err(RuntimeError::Invariant(
                "ownKeys received unexpected key-list reply",
            )),
        }
    }
    pub(crate) fn descriptor(
        self,
        runtime: &Runtime,
        result: NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>,
    ) -> Result<KeysStep, RuntimeError> {
        let Phase::Descriptor { mut state, key } = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "ownKeys received unexpected descriptor reply",
            ));
        };
        let descriptor = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(KeysStep::Complete(NativeConversion::Throw(value)));
            }
        };
        if let Some(descriptor) = descriptor {
            let missing = if !state.extensible {
                !state.atoms.remove(&key.atom())
            } else {
                !descriptor.configurable() && !state.atoms.contains(&key.atom())
            };
            if missing {
                return fail(
                    runtime,
                    self.0.realm,
                    "proxy: target property must be present in proxy ownKeys",
                );
            }
        }
        check_next(runtime, self.0.realm, state)
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: KeysStep,
) -> Result<NativeConversion<Vec<PropertyKey>>, RuntimeError> {
    loop {
        step = match step {
            KeysStep::Complete(result) => return Ok(result),
            KeysStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            KeysStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                {
                    let completion = match target {
                        DirectCallTarget::Callable(callable) => {
                            runtime.call_internal(realm, &callable, receiver, &arguments)?
                        }
                        DirectCallTarget::NonCallableProxy(proxy) => {
                            runtime.call_proxy(realm, &proxy, receiver, &arguments)?
                        }
                    };
                    resume.resume(runtime, completion)?
                }
            }
            KeysStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            KeysStep::Keys { mut resume } => {
                let object = resume.take_keys_object();
                resume.keys(runtime, runtime.internal_own_property_keys(realm, &object)?)?
            }
            KeysStep::Extensible { mut resume } => {
                let object = resume.take_extensible_object();
                resume.boolean(runtime, runtime.internal_is_extensible(realm, &object)?)?
            }
            KeysStep::Descriptor { mut resume } => {
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
    #[test]
    fn list_and_key_roots_survive_gc_and_release_after_abandonment() {
        for after_list in [false, true] {
            let runtime = Runtime::new();
            let weak = std::rc::Rc::downgrade(&runtime.0);
            let mut context = runtime.new_context();
            let Value::Object(proxy) = context.eval("new Proxy({}, {})").unwrap() else {
                panic!("expected proxy")
            };
            let data = runtime.proxy_snapshot_if_any(&proxy).unwrap().unwrap();
            let ids = [proxy.object_id(), data.target, data.handler];
            let callable = context.eval("(function(){})").unwrap();
            let KeysStep::Read { resume, .. } =
                KeysStep::start(&runtime, context.realm, proxy).unwrap()
            else {
                panic!("expected handler read")
            };
            let KeysStep::Call { resume, .. } = resume
                .resume(&runtime, Completion::Return(callable))
                .unwrap()
            else {
                panic!("expected trap")
            };
            let list = runtime.new_object(None).unwrap();
            let list_id = list.object_id();
            let KeysStep::Read { resume, .. } = resume
                .resume(&runtime, Completion::Return(Value::Object(list)))
                .unwrap()
            else {
                panic!("expected length")
            };
            let KeysStep::Number { mut resume, .. } = resume
                .resume(&runtime, Completion::Return(Value::Int(1)))
                .unwrap()
            else {
                panic!("expected conversion")
            };
            let symbol = context.eval("Symbol('owned-key')").unwrap();
            let Value::Symbol(symbol) = symbol else {
                panic!("expected symbol")
            };
            let atom = symbol.atom();
            if after_list {
                let KeysStep::Read { resume: next, .. } = resume
                    .number(&runtime, NativeConversion::Value(1.0))
                    .unwrap()
                else {
                    panic!("expected item")
                };
                let KeysStep::Extensible { resume: next, .. } = next
                    .resume(&runtime, Completion::Return(Value::Symbol(symbol)))
                    .unwrap()
                else {
                    panic!("expected target query")
                };
                resume = next;
            } else {
                drop(symbol);
            }
            runtime.run_gc().unwrap();
            for id in ids {
                assert!(runtime.0.state.borrow().heap.object(id).is_ok());
            }
            assert_eq!(
                runtime.0.state.borrow().heap.object(list_id).is_ok(),
                !after_list
            );
            if after_list {
                assert!(
                    runtime
                        .0
                        .state
                        .borrow()
                        .atoms
                        .property_key_kind(atom)
                        .is_ok()
                );
            }
            drop(resume);
            runtime.run_gc().unwrap();
            for id in ids {
                assert!(runtime.0.state.borrow().heap.object(id).is_err());
            }
            assert!(runtime.0.state.borrow().heap.object(list_id).is_err());
            if after_list {
                assert!(
                    runtime
                        .0
                        .state
                        .borrow()
                        .atoms
                        .property_key_kind(atom)
                        .is_err()
                );
            }
            drop(context);
            drop(runtime);
            assert!(weak.upgrade().is_none());
        }
    }
}

#[derive(Default)]
struct KeysStepPending {
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    call_target: Option<DirectCallTarget>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    number_value: Option<Value>,
    keys_object: Option<ObjectRef>,
    extensible_object: Option<ObjectRef>,
    descriptor_object: Option<ObjectRef>,
    descriptor_key: Option<PropertyKey>,
}
impl KeysStep {
    pub(crate) fn request_read(receiver: Value, key: PropertyKey, mut resume: KeysResume) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        target: DirectCallTarget,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: KeysResume,
    ) -> Self {
        resume.0.pending_effect.call_target = Some(target);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: KeysResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_keys(object: ObjectRef, mut resume: KeysResume) -> Self {
        resume.0.pending_effect.keys_object = Some(object);
        Self::Keys { resume }
    }
    pub(crate) fn request_extensible(object: ObjectRef, mut resume: KeysResume) -> Self {
        resume.0.pending_effect.extensible_object = Some(object);
        Self::Extensible { resume }
    }
    pub(crate) fn request_descriptor(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: KeysResume,
    ) -> Self {
        resume.0.pending_effect.descriptor_object = Some(object);
        resume.0.pending_effect.descriptor_key = Some(key);
        Self::Descriptor { resume }
    }
}
impl KeysResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("KeysStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("KeysStep Read key")
    }
    pub(crate) fn take_call_target(&mut self) -> DirectCallTarget {
        self.0
            .pending_effect
            .call_target
            .take()
            .expect("KeysStep Call target")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("KeysStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("KeysStep Call arguments")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("KeysStep Number value")
    }
    pub(crate) fn take_keys_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .keys_object
            .take()
            .expect("KeysStep Keys object")
    }
    pub(crate) fn take_extensible_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .extensible_object
            .take()
            .expect("KeysStep Extensible object")
    }
    pub(crate) fn take_descriptor_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .descriptor_object
            .take()
            .expect("KeysStep Descriptor object")
    }
    pub(crate) fn take_descriptor_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .descriptor_key
            .take()
            .expect("KeysStep Descriptor key")
    }
}
const _: () = assert!(std::mem::size_of::<KeysStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<KeysStep>() <= 64);
