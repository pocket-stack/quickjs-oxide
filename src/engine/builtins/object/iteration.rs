//! Object iterator consumers preserve pinned acquisition and close boundaries.
use super::ObjectIteratorStep;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::{
        iterator::step::{CloseStep, NextStep, finish_close, finish_next},
        native::{ArrayPushKind, NativeFunctionId},
    },
    heap::ContextId,
    object::{
        CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
        WellKnownSymbol, operations::InternalDefineResult,
    },
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum IterationKind {
    Entries,
    Group,
    MapGroup,
}
impl IterationKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        match target {
            NativeFunctionId::ObjectFromEntries => Some(Self::Entries),
            NativeFunctionId::ObjectGroupBy => Some(Self::Group),
            NativeFunctionId::Map(crate::engine::builtins::native::MapNativeKind::GroupBy)
            | NativeFunctionId::Set(crate::engine::builtins::native::SetNativeKind::GroupBy) => {
                Some(Self::MapGroup)
            }
            _ => None,
        }
    }
}
pub(crate) enum IterationStep {
    Complete(Completion),
    Read {
        resume: IterationResume,
    },
    Call {
        resume: IterationResume,
    },
    Next {
        resume: IterationResume,
    },
    Key {
        resume: IterationResume,
    },
    Define {
        resume: IterationResume,
    },
    Push {
        resume: IterationResume,
    },
    Close {
        iterator: ObjectRef,
        completion: Completion,
    },
}
pub(crate) struct IterationResume(Box<IterationResumeState>);
impl std::ops::Deref for IterationResume {
    type Target = IterationResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for IterationResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<IterationResume>() <= 8);
pub(crate) struct IterationResumeState {
    pending_effect: IterationStepPending,
    realm: ContextId,
    kind: IterationKind,
    result: Option<ObjectRef>,
    callback: Option<CallableRef>,
    iterator: Option<ObjectRef>,
    next: Value,
    index: u64,
    limit: u64,
    phase: Phase,
}
enum Phase {
    IteratorMethod(Value),
    Iterator,
    NextMethod,
    Next,
    EntryKey(ObjectRef),
    EntryValue(Value),
    Key(Value),
    Callback(Value),
    Group(Value, PropertyKey),
    GroupDefined(Value, ObjectRef),
    EntryDefined(PropertyKey),
    Push,
}
impl IterationStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: IterationKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        Self::start_with_limit(
            runtime,
            realm,
            kind,
            invocation,
            arguments,
            (1u64 << 53) - 1,
        )
    }
    pub(super) fn start_with_limit(
        runtime: &Runtime,
        realm: ContextId,
        kind: IterationKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
        limit: u64,
    ) -> Result<Self, RuntimeError> {
        if !matches!(invocation, NativeInvocation::Call { .. }) {
            return Err(RuntimeError::Invariant(
                "Object iterator consumer requires a generic invocation",
            ));
        }
        // groupBy validates callback first; fromEntries allocates its result first.
        let callback = if !matches!(kind, IterationKind::Entries) {
            let value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "groupBy callback argv was not padded",
            ))?;
            let callback = match value {
                Value::Object(object) => runtime.as_callable(object)?,
                _ => None,
            };
            let Some(callback) = callback else {
                return Ok(Self::Complete(Completion::Throw(
                    runtime.new_native_error(realm, NativeErrorKind::Type, "not a function")?,
                )));
            };
            Some(callback)
        } else {
            None
        };
        let result = if matches!(kind, IterationKind::Entries) {
            Some(runtime.new_ordinary_object_in_realm(realm)?)
        } else {
            None
        };
        let iterable = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Object iterator argv was not padded",
            ))?;
        if matches!(iterable, Value::Null | Value::Undefined) {
            let base = if matches!(iterable, Value::Null) {
                "null"
            } else {
                "undefined"
            };
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    &format!("cannot read property 'Symbol.iterator' of {base}"),
                )?,
            )));
        }
        Ok(Self::request_read(
            iterable.clone(),
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator)),
            IterationResume(Box::new(IterationResumeState {
                pending_effect: IterationStepPending::default(),
                realm,
                kind,
                result,
                callback,
                iterator: None,
                next: Value::Undefined,
                index: 0,
                limit,
                phase: Phase::IteratorMethod(iterable),
            })),
        ))
    }
}
impl IterationResume {
    fn iterator(&self) -> Result<ObjectRef, RuntimeError> {
        self.0
            .iterator
            .clone()
            .ok_or(RuntimeError::Invariant("Object iterator not acquired"))
    }
    fn result(&self) -> Result<ObjectRef, RuntimeError> {
        self.0.result.clone().ok_or(RuntimeError::Invariant(
            "Object iterator result not allocated",
        ))
    }
    fn abrupt(self, value: Value) -> IterationStep {
        let close = matches!(self.0.kind, IterationKind::Entries)
            || matches!(self.0.phase, Phase::Callback(_) | Phase::Key(_));
        if close && let Some(iterator) = self.0.iterator {
            IterationStep::Close {
                iterator,
                completion: Completion::Throw(value),
            }
        } else {
            IterationStep::Complete(Completion::Throw(value))
        }
    }
    fn next_step(mut self, runtime: &Runtime) -> Result<IterationStep, RuntimeError> {
        if !matches!(self.0.kind, IterationKind::Entries) && self.0.index >= self.0.limit {
            return Ok(IterationStep::Close {
                iterator: self.iterator()?,
                completion: Completion::Throw(runtime.new_native_error(
                    self.0.realm,
                    NativeErrorKind::Type,
                    "too many elements",
                )?),
            });
        }
        self.0.phase = Phase::Next;
        Ok(IterationStep::request_next(
            self.iterator()?,
            self.0.next.clone(),
            self,
        ))
    }
    pub(crate) fn next(
        mut self,
        runtime: &Runtime,
        reply: ObjectIteratorStep,
    ) -> Result<IterationStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Next) {
            return Err(RuntimeError::Invariant(
                "Object iterator step has wrong reply phase",
            ));
        }
        let value = match reply {
            ObjectIteratorStep::Throw(value) => return Ok(self.abrupt(value)),
            ObjectIteratorStep::Done => {
                return Ok(IterationStep::Complete(Completion::Return(Value::Object(
                    self.result()?,
                ))));
            }
            ObjectIteratorStep::Yield(value) => value,
        };
        match self.0.kind {
            IterationKind::Entries => {
                let Value::Object(item) = value else {
                    let value = runtime.new_native_error(
                        self.0.realm,
                        NativeErrorKind::Type,
                        "not an object",
                    )?;
                    return Ok(self.abrupt(value));
                };
                self.0.phase = Phase::EntryKey(item.clone());
                Ok(IterationStep::request_read(
                    Value::Object(item),
                    runtime.intern_property_key("0")?,
                    self,
                ))
            }
            IterationKind::Group | IterationKind::MapGroup => {
                let callable = self
                    .0
                    .callback
                    .clone()
                    .ok_or(RuntimeError::Invariant("groupBy callback missing"))?;
                let arguments = vec![value.clone(), Value::number(self.0.index as f64)];
                self.0.phase = Phase::Callback(value);
                Ok(IterationStep::request_call(
                    callable,
                    Value::Object(runtime.global_object_for_realm(self.0.realm)?),
                    arguments,
                    self,
                ))
            }
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<IterationStep, RuntimeError> {
        let value = match reply {
            Completion::Throw(value) => return Ok(self.abrupt(value)),
            Completion::Return(value) => value,
        };
        let phase = std::mem::replace(&mut self.0.phase, Phase::Next);
        match phase {
            Phase::IteratorMethod(iterable) => {
                let callable = match value {
                    Value::Object(ref object) => runtime.as_callable(object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return Ok(IterationStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "value is not iterable",
                        )?,
                    )));
                };
                self.0.phase = Phase::Iterator;
                Ok(IterationStep::request_call(
                    callable,
                    iterable,
                    Vec::new(),
                    self,
                ))
            }
            Phase::Iterator => {
                let Value::Object(iterator) = value else {
                    return Ok(IterationStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "not an object",
                        )?,
                    )));
                };
                self.0.iterator = Some(iterator.clone());
                self.0.phase = Phase::NextMethod;
                Ok(IterationStep::request_read(
                    Value::Object(iterator),
                    runtime.intern_property_key("next")?,
                    self,
                ))
            }
            Phase::NextMethod => {
                self.0.next = value;
                match self.0.kind {
                    IterationKind::Group => self.0.result = Some(runtime.new_object(None)?),
                    IterationKind::MapGroup => {
                        self.0.result = Some(runtime.new_map_in_realm(self.0.realm)?)
                    }
                    IterationKind::Entries => {}
                }
                self.next_step(runtime)
            }
            Phase::EntryKey(item) => {
                self.0.phase = Phase::EntryValue(value);
                Ok(IterationStep::request_read(
                    Value::Object(item),
                    runtime.intern_property_key("1")?,
                    self,
                ))
            }
            Phase::EntryValue(key) => {
                self.0.phase = Phase::Key(value);
                Ok(IterationStep::request_key(key, self))
            }
            Phase::Callback(item) => {
                if matches!(self.0.kind, IterationKind::MapGroup) {
                    let key = Runtime::normalized_map_key(value);
                    let groups = self.result()?;
                    let group = match runtime.find_map_record(&groups, &key)? {
                        Some((_, value)) => match runtime.root_raw_value(&value)? {
                            Value::Object(group) => group,
                            _ => {
                                return Err(RuntimeError::Invariant(
                                    "Map.groupBy result contained a non-Array group",
                                ));
                            }
                        },
                        None => {
                            let group = runtime.new_array(self.0.realm)?;
                            runtime.set_map_record(&groups, key, Value::Object(group.clone()))?;
                            group
                        }
                    };
                    self.0.phase = Phase::Push;
                    return Ok(IterationStep::request_push(group, item, self));
                }
                self.0.phase = Phase::Key(item);
                Ok(IterationStep::request_key(value, self))
            }
            Phase::Group(item, key) => {
                let group = match value {
                    Value::Undefined => {
                        let group = runtime.new_array(self.0.realm)?;
                        self.0.phase = Phase::GroupDefined(item, group.clone());
                        return Ok(IterationStep::request_define(
                            self.result()?,
                            key,
                            descriptor(Value::Object(group)),
                            self,
                        ));
                    }
                    Value::Object(group) => group,
                    _ => {
                        return Err(RuntimeError::Invariant(
                            "Object.groupBy result contained a non-Array group",
                        ));
                    }
                };
                self.0.phase = Phase::Push;
                Ok(IterationStep::request_push(group, item, self))
            }
            Phase::Push => {
                self.0.index = self.0.index.checked_add(1).ok_or(RuntimeError::Invariant(
                    "Object.groupBy index overflowed Uint64",
                ))?;
                self.next_step(runtime)
            }
            _ => Err(RuntimeError::Invariant(
                "Object iterator consumer received wrong completion reply",
            )),
        }
    }
    pub(crate) fn key(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<IterationStep, RuntimeError> {
        let value = match reply {
            Completion::Throw(value) => return Ok(self.abrupt(value)),
            Completion::Return(value) => value,
        };
        let key = match runtime.property_key_from_primitive(self.0.realm, value)? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
        };
        let Phase::Key(item) = std::mem::replace(&mut self.0.phase, Phase::Next) else {
            return Err(RuntimeError::Invariant(
                "Object iterator key has wrong phase",
            ));
        };
        match self.0.kind {
            IterationKind::Entries => {
                self.0.phase = Phase::EntryDefined(key.clone());
                Ok(IterationStep::request_define(
                    self.result()?,
                    key,
                    descriptor(item),
                    self,
                ))
            }
            IterationKind::Group => {
                self.0.phase = Phase::Group(item, key.clone());
                Ok(IterationStep::request_read(
                    Value::Object(self.result()?),
                    key,
                    self,
                ))
            }
            IterationKind::MapGroup => Err(RuntimeError::Invariant(
                "Map.groupBy received a property-key reply",
            )),
        }
    }
    pub(crate) fn defined(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<InternalDefineResult>,
    ) -> Result<IterationStep, RuntimeError> {
        let result = match reply {
            NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
            NativeConversion::Value(result) => result,
        };
        match std::mem::replace(&mut self.0.phase, Phase::Next) {
            Phase::EntryDefined(key) => {
                if let Some(value) = runtime.finish_define_property_or_throw(
                    self.0.realm,
                    &key,
                    NativeConversion::Value(result),
                )? {
                    return Ok(self.abrupt(value));
                }
                self.next_step(runtime)
            }
            Phase::GroupDefined(value, group) => {
                if !matches!(result, InternalDefineResult::Defined) {
                    return Err(RuntimeError::Invariant(
                        "fresh Object.groupBy result rejected a group property",
                    ));
                }
                self.0.phase = Phase::Push;
                Ok(IterationStep::request_push(group, value, self))
            }
            _ => Err(RuntimeError::Invariant(
                "Object iterator definition has wrong phase",
            )),
        }
    }
}
fn descriptor(value: Value) -> OrdinaryPropertyDescriptor {
    OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: IterationStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            IterationStep::Complete(result) => return Ok(result),
            IterationStep::Close {
                iterator,
                completion,
            } => {
                return finish_close(
                    runtime,
                    realm,
                    CloseStep::start(runtime, realm, iterator, completion)?,
                );
            }
            IterationStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            IterationStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &arguments)?,
                )?
            }
            IterationStep::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                resume.next(
                    runtime,
                    finish_next(
                        runtime,
                        realm,
                        NextStep::start(runtime, realm, iterator, method)?,
                    )?,
                )?
            }
            IterationStep::Key { mut resume } => {
                let value = resume.take_key_value();
                {
                    let result = runtime.to_primitive(
                        realm,
                        value,
                        crate::engine::vm::ToPrimitiveHint::String,
                    )?;
                    resume.key(runtime, result)?
                }
            }
            IterationStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                resume.defined(
                    runtime,
                    runtime.internal_define_own_property(realm, &object, &key, &descriptor)?,
                )?
            }
            IterationStep::Push { mut resume } => {
                let object = resume.take_push_object();
                let value = resume.take_push_value();
                resume.resume(
                    runtime,
                    runtime.call_array_prototype_push(
                        realm,
                        ArrayPushKind::Push,
                        NativeInvocation::Call {
                            this_value: Value::Object(object),
                        },
                        &NativeArguments {
                            actual_arg_count: 1,
                            readable: vec![value],
                        },
                    )?,
                )?
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn from_entries_unpublished_result_is_owned_until_abandonment() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let iterable = runtime.new_object(None).unwrap();
        let iterable_id = iterable.object_id();
        let arguments = NativeArguments {
            actual_arg_count: 1,
            readable: vec![Value::Object(iterable)],
        };
        let IterationStep::Read { mut resume } = IterationStep::start(
            &runtime,
            context.realm,
            IterationKind::Entries,
            &NativeInvocation::Call {
                this_value: Value::Undefined,
            },
            &arguments,
        )
        .unwrap() else {
            panic!("iterator method read expected")
        };
        let _ = resume.take_read_receiver();
        let _ = resume.take_read_key();

        let result_id = resume.result.as_ref().unwrap().object_id();
        drop(arguments);
        runtime.run_gc().unwrap();
        for id in [iterable_id, result_id] {
            assert!(runtime.0.state.borrow().heap.object(id).is_ok());
        }
        drop(resume);
        runtime.run_gc().unwrap();
        for id in [iterable_id, result_id] {
            assert!(runtime.0.state.borrow().heap.object(id).is_err());
        }
    }
}

#[derive(Default)]
struct IterationStepPending {
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    next_iterator: Option<ObjectRef>,
    next_method: Option<Value>,
    key_value: Option<Value>,
    define_object: Option<ObjectRef>,
    define_key: Option<PropertyKey>,
    define_descriptor: Option<OrdinaryPropertyDescriptor>,
    push_object: Option<ObjectRef>,
    push_value: Option<Value>,
}
impl IterationStep {
    pub(crate) fn request_read(
        receiver: Value,
        key: PropertyKey,
        mut resume: IterationResume,
    ) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: IterationResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_next(
        iterator: ObjectRef,
        method: Value,
        mut resume: IterationResume,
    ) -> Self {
        resume.0.pending_effect.next_iterator = Some(iterator);
        resume.0.pending_effect.next_method = Some(method);
        Self::Next { resume }
    }
    pub(crate) fn request_key(value: Value, mut resume: IterationResume) -> Self {
        resume.0.pending_effect.key_value = Some(value);
        Self::Key { resume }
    }
    pub(crate) fn request_define(
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
        mut resume: IterationResume,
    ) -> Self {
        resume.0.pending_effect.define_object = Some(object);
        resume.0.pending_effect.define_key = Some(key);
        resume.0.pending_effect.define_descriptor = Some(descriptor);
        Self::Define { resume }
    }
    pub(crate) fn request_push(
        object: ObjectRef,
        value: Value,
        mut resume: IterationResume,
    ) -> Self {
        resume.0.pending_effect.push_object = Some(object);
        resume.0.pending_effect.push_value = Some(value);
        Self::Push { resume }
    }
}
impl IterationResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("IterationStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("IterationStep Read key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("IterationStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("IterationStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("IterationStep Call arguments")
    }
    pub(crate) fn take_next_iterator(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .next_iterator
            .take()
            .expect("IterationStep Next iterator")
    }
    pub(crate) fn take_next_method(&mut self) -> Value {
        self.0
            .pending_effect
            .next_method
            .take()
            .expect("IterationStep Next method")
    }
    pub(crate) fn take_key_value(&mut self) -> Value {
        self.0
            .pending_effect
            .key_value
            .take()
            .expect("IterationStep Key value")
    }
    pub(crate) fn take_define_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .define_object
            .take()
            .expect("IterationStep Define object")
    }
    pub(crate) fn take_define_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .define_key
            .take()
            .expect("IterationStep Define key")
    }
    pub(crate) fn take_define_descriptor(&mut self) -> OrdinaryPropertyDescriptor {
        self.0
            .pending_effect
            .define_descriptor
            .take()
            .expect("IterationStep Define descriptor")
    }
    pub(crate) fn take_push_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .push_object
            .take()
            .expect("IterationStep Push object")
    }
    pub(crate) fn take_push_value(&mut self) -> Value {
        self.0
            .pending_effect
            .push_value
            .take()
            .expect("IterationStep Push value")
    }
}
const _: () = assert!(std::mem::size_of::<IterationStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<IterationStep>() <= 64);
