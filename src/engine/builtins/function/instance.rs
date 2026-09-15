//! Instanceof owns method selection and prototype walking across user callbacks.
use crate::engine::{
    api::{
        Error, ErrorKind, error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError,
    },
    builtins::native::NativeFunctionId,
    heap::{ContextId, ObjectPayload},
    object::{CallableRef, ObjectRef, PropertyKey, WellKnownSymbol},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum InstanceStep {
    Complete(Completion),
    Read { resume: InstanceResume },
    Call { resume: InstanceResume },
    Prototype { resume: InstanceResume },
}
pub(crate) struct InstanceResume(Box<InstanceResumeState>);
impl std::ops::Deref for InstanceResume {
    type Target = InstanceResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for InstanceResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<InstanceResume>() <= 8);
pub(crate) struct InstanceResumeState {
    pending_effect: InstanceStepPending,
    realm: ContextId,
    candidate: Value,
    target: ObjectRef,
    phase: Phase,
}
enum Phase {
    Method { delegate: bool },
    Result,
    Prototype,
    Walk(ObjectRef),
}
impl InstanceStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        candidate: Value,
        target: ObjectRef,
    ) -> Result<Self, RuntimeError> {
        Self::method(runtime, realm, candidate, target, false)
    }
    fn method(
        runtime: &Runtime,
        realm: ContextId,
        candidate: Value,
        target: ObjectRef,
        delegate: bool,
    ) -> Result<Self, RuntimeError> {
        Ok({
            let __pending_field_object = target.clone();
            let __pending_field_key =
                PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
            let __pending_field_resume = InstanceResume(Box::new(InstanceResumeState {
                pending_effect: InstanceStepPending::default(),
                realm,
                candidate,
                target,
                phase: Phase::Method { delegate },
            }));
            Self::request_read(
                __pending_field_object,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
    pub(crate) fn native(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "hasInstance requires generic invocation",
            ));
        };
        let target = match this_value {
            Value::Object(target) => runtime.as_callable(target)?,
            _ => None,
        };
        let Some(target) = target else {
            return Ok(Self::Complete(Completion::Return(Value::Bool(false))));
        };
        Self::ordinary(
            runtime,
            realm,
            &target,
            arguments
                .readable
                .first()
                .cloned()
                .unwrap_or(Value::Undefined),
        )
    }
    pub(crate) fn ordinary(
        runtime: &Runtime,
        realm: ContextId,
        target: &CallableRef,
        candidate: Value,
    ) -> Result<Self, RuntimeError> {
        let bound = {
            let state = runtime.0.state.borrow();
            match &state.heap.object(target.as_object().object_id())?.payload {
                ObjectPayload::BoundFunction { target, .. } => Some(*target),
                ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
                | ObjectPayload::Proxy(_) => None,
                _ => {
                    return Err(RuntimeError::Invariant(
                        "ordinary instanceof received a non-callable target",
                    ));
                }
            }
        };
        if let Some(bound) = bound {
            let target = ObjectRef::from_borrowed_handle(runtime.clone(), bound)?;
            return Self::method(runtime, realm, candidate, target, true);
        }
        if !matches!(candidate, Value::Object(_)) {
            return Ok(Self::Complete(Completion::Return(Value::Bool(false))));
        }
        Ok({
            let __pending_field_object = target.as_object().clone();
            let __pending_field_key = runtime.intern_property_key("prototype")?;
            let __pending_field_resume = InstanceResume(Box::new(InstanceResumeState {
                pending_effect: InstanceStepPending::default(),
                realm,
                candidate,
                target: target.as_object().clone(),
                phase: Phase::Prototype,
            }));
            Self::request_read(
                __pending_field_object,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
}
impl InstanceResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<InstanceStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            result @ Completion::Throw(_) => return Ok(InstanceStep::Complete(result)),
        };
        match self.0.phase {
            Phase::Method { delegate } => {
                if matches!(value, Value::Null | Value::Undefined) {
                    let Some(target) = runtime.as_callable(&self.0.target)? else {
                        return if delegate {
                            Ok(InstanceStep::Complete(Completion::Throw(
                                runtime.new_native_error(
                                    self.0.realm,
                                    NativeErrorKind::Type,
                                    "invalid 'instanceof' right operand",
                                )?,
                            )))
                        } else {
                            Err(RuntimeError::Engine(Error::new(
                                ErrorKind::Type,
                                "invalid 'instanceof' right operand",
                            )))
                        };
                    };
                    return InstanceStep::ordinary(
                        runtime,
                        self.0.realm,
                        &target,
                        self.0.candidate,
                    );
                }
                let callable = match runtime.callable_from_value(value) {
                    Ok(callable) => callable,
                    Err(RuntimeError::Engine(error))
                        if delegate && error.kind() == ErrorKind::Type =>
                    {
                        return Ok(InstanceStep::Complete(Completion::Throw(
                            runtime.new_native_error_from_error(
                                self.0.realm,
                                NativeErrorKind::Type,
                                &error,
                            )?,
                        )));
                    }
                    Err(error) => return Err(error),
                };
                Ok({
                    let __pending_field_callable = callable;
                    let __pending_field_receiver = Value::Object(self.0.target.clone());
                    let __pending_field_arguments = vec![self.0.candidate.clone()];
                    let __pending_field_delegate = delegate;
                    let __pending_field_resume = {
                        let updated_0 = Phase::Result;
                        self.0.phase = updated_0;
                        self
                    };
                    InstanceStep::request_call(
                        __pending_field_callable,
                        __pending_field_receiver,
                        __pending_field_arguments,
                        __pending_field_delegate,
                        __pending_field_resume,
                    )
                })
            }
            Phase::Result => Ok(InstanceStep::Complete(Completion::Return(Value::Bool(
                runtime.value_to_boolean(&value)?,
            )))),
            Phase::Prototype => {
                let Value::Object(prototype) = value else {
                    return Ok(InstanceStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "operand 'prototype' property is not an object",
                        )?,
                    )));
                };
                let Value::Object(candidate) = &self.0.candidate else {
                    return Err(RuntimeError::Invariant("instanceof lost object candidate"));
                };
                Ok({
                    let __pending_field_object = candidate.clone();
                    let __pending_field_resume = {
                        let updated_0 = Phase::Walk(prototype);
                        self.0.phase = updated_0;
                        self
                    };
                    InstanceStep::request_prototype(__pending_field_object, __pending_field_resume)
                })
            }
            Phase::Walk(_) => Err(RuntimeError::Invariant(
                "prototype walk received untyped reply",
            )),
        }
    }
    pub(crate) fn prototype(
        self,
        result: NativeConversion<Option<ObjectRef>>,
    ) -> Result<InstanceStep, RuntimeError> {
        let Phase::Walk(expected) = &self.0.phase else {
            return Err(RuntimeError::Invariant(
                "instanceof received unexpected prototype",
            ));
        };
        Ok(match result {
            NativeConversion::Throw(value) => InstanceStep::Complete(Completion::Throw(value)),
            NativeConversion::Value(None) => {
                InstanceStep::Complete(Completion::Return(Value::Bool(false)))
            }
            NativeConversion::Value(Some(object)) if &object == expected => {
                InstanceStep::Complete(Completion::Return(Value::Bool(true)))
            }
            NativeConversion::Value(Some(object)) => {
                let __pending_field_object = object;
                let __pending_field_resume = self;
                InstanceStep::request_prototype(__pending_field_object, __pending_field_resume)
            }
        })
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    mut realm: ContextId,
    mut step: InstanceStep,
) -> Result<Completion, RuntimeError> {
    // The old consumer retains its bound-chain trampoline and native backtraces.
    let mut frames = Vec::new();
    let result = (|| loop {
        step = match step {
            InstanceStep::Complete(result) => return Ok(result),
            InstanceStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            InstanceStep::Prototype { mut resume } => {
                let object = resume.take_prototype_object();
                resume.prototype(runtime.internal_get_prototype_of(realm, &object)?)?
            }
            InstanceStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                let delegate = resume.take_call_delegate();
                {
                    let standard = if delegate {
                        runtime.direct_native_callable_metadata(&callable)?
                    } else {
                        None
                    };
                    if let Some((
                        NativeFunctionId::FunctionPrototypeHasInstance,
                        defining_realm,
                        min,
                    )) = standard
                    {
                        frames.push(runtime.push_native_active_frame(
                            callable.as_object().clone(),
                            defining_realm,
                            NativeFunctionId::FunctionPrototypeHasInstance,
                            1,
                            1usize.max(usize::from(min)),
                        )?);
                        realm = defining_realm;
                        InstanceStep::native(
                            runtime,
                            realm,
                            &NativeInvocation::Call {
                                this_value: receiver,
                            },
                            &NativeArguments {
                                actual_arg_count: 1,
                                readable: arguments,
                            },
                        )?
                    } else {
                        resume.resume(
                            runtime,
                            runtime.call_internal(realm, &callable, receiver, &arguments)?,
                        )?
                    }
                }
            }
        };
    })();
    let mut frame_error = None;
    while let Some(frame) = frames.pop() {
        if let Err(error) = frame.finish() {
            frame_error.get_or_insert(error);
        }
    }
    frame_error.map_or(result, Err)
}

#[derive(Default)]
struct InstanceStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    call_delegate: Option<bool>,
    prototype_object: Option<ObjectRef>,
}
impl InstanceStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: InstanceResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
        delegate: bool,
        mut resume: InstanceResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        resume.0.pending_effect.call_delegate = Some(delegate);
        Self::Call { resume }
    }
    pub(crate) fn request_prototype(object: ObjectRef, mut resume: InstanceResume) -> Self {
        resume.0.pending_effect.prototype_object = Some(object);
        Self::Prototype { resume }
    }
}
impl InstanceResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("InstanceStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("InstanceStep Read key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("InstanceStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("InstanceStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("InstanceStep Call arguments")
    }
    pub(crate) fn take_call_delegate(&mut self) -> bool {
        self.0
            .pending_effect
            .call_delegate
            .take()
            .expect("InstanceStep Call delegate")
    }
    pub(crate) fn take_prototype_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .prototype_object
            .take()
            .expect("InstanceStep Prototype object")
    }
}
const _: () = assert!(std::mem::size_of::<InstanceStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<InstanceStep>() <= 64);
