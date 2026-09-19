//! Call forwarding shares validation and argument ownership across both VM consumers.
use super::arguments::{ArgumentsStep, finish as finish_arguments};
use crate::engine::builtins::native::ReflectKind;
use crate::engine::vm::call::{ConstructNewTarget, ConstructorRef, DirectCallTarget};
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::NativeFunctionId,
    heap::ContextId,
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};

#[derive(Clone, Copy)]
pub(crate) enum InvokeKind {
    Call,
    Apply,
    ReflectApply,
    ReflectConstruct,
}
impl InvokeKind {
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::FunctionPrototypeCall => Self::Call,
            NativeFunctionId::FunctionPrototypeApply => Self::Apply,
            NativeFunctionId::Reflect(ReflectKind::Apply) => Self::ReflectApply,
            NativeFunctionId::Reflect(ReflectKind::Construct) => Self::ReflectConstruct,
            _ => return None,
        })
    }
}
pub(crate) enum InvokeStep {
    Construct(Box<InvokeConstruct>),
    Complete(Completion),
    Arguments { resume: InvokeResume },
    Call(Box<InvokeCall>),
}
pub(crate) struct InvokeResume(Box<InvokeResumeState>);
impl std::ops::Deref for InvokeResume {
    type Target = InvokeResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for InvokeResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<InvokeResume>() <= 8);
pub(crate) struct InvokeResumeState {
    pending_effect: InvokeStepPending,
    realm: ContextId,
    target: ForwardTarget,
}
enum ForwardTarget {
    Call {
        target: DirectCallTarget,
        receiver: Value,
    },
    Construct {
        target: Value,
        new_target: Option<ConstructNewTarget>,
    },
}
impl InvokeStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: InvokeKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "call forwarding requires a generic invocation",
            ));
        };
        if matches!(kind, InvokeKind::ReflectConstruct) {
            let new_target = if arguments.actual_arg_count > 2 {
                let value = arguments
                    .readable
                    .get(2)
                    .cloned()
                    .ok_or(RuntimeError::Invariant(
                        "Reflect.construct newTarget argv was not readable",
                    ))?;
                if !matches!(value, Value::Object(_)) {
                    return Ok(Self::Complete(Completion::Throw(
                        runtime.new_not_constructor_error(realm, &value)?,
                    )));
                }
                Some(match runtime.constructor_from_value(realm, value)? {
                    NativeConversion::Value(target) => ConstructNewTarget::Validated(target),
                    NativeConversion::Throw(value) => {
                        return Ok(Self::Complete(Completion::Throw(value)));
                    }
                })
            } else {
                None
            };
            return Ok({
                let __pending_field_value = arguments.readable[1].clone();
                let __pending_field_resume = InvokeResume(Box::new(InvokeResumeState {
                    pending_effect: InvokeStepPending::default(),
                    realm,
                    target: ForwardTarget::Construct {
                        target: arguments.readable[0].clone(),
                        new_target,
                    },
                }));
                Self::request_arguments(__pending_field_value, __pending_field_resume)
            });
        }
        let (target, receiver, list) = match kind {
            InvokeKind::ReflectConstruct => unreachable!("constructor validation already handled"),
            InvokeKind::Call => {
                let actual = &arguments.readable[..arguments.actual_arg_count];
                let (target, receiver) = match runtime.forward_function_prototype_call(
                    realm,
                    this_value.clone(),
                    actual,
                )? {
                    NativeConversion::Value(result) => result,
                    NativeConversion::Throw(value) => {
                        return Ok(Self::Complete(Completion::Throw(value)));
                    }
                };
                let forwarded = actual.get(1..).unwrap_or(&[]).to_vec();
                #[cfg(feature = "profiling")]
                {
                    crate::engine::api::profiling::record_call_buffer_capacity(
                        "function.call_suffix",
                        0,
                        forwarded.capacity(),
                        size_of::<Value>(),
                    );
                    crate::engine::api::profiling::record_call_buffer_copies(
                        "function.call_suffix",
                        &forwarded,
                    );
                }
                return Ok(Self::Call(Box::new(InvokeCall {
                    target,
                    receiver,
                    arguments: forwarded,
                })));
            }
            InvokeKind::Apply => {
                let target = match this_value {
                    Value::Object(object) => runtime.as_callable(object)?,
                    _ => None,
                };
                let Some(target) = target else {
                    return Ok(Self::Complete(Completion::Throw(
                        runtime.new_native_error_jsvalue(realm, NativeErrorKind::Type, "not a function")?,
                    )));
                };
                (
                    DirectCallTarget::Callable(target),
                    arguments.readable[0].clone(),
                    arguments.readable[1].clone(),
                )
            }
            InvokeKind::ReflectApply => (
                DirectCallTarget::Callable(
                    runtime.callable_from_value(arguments.readable[0].clone())?,
                ),
                arguments.readable[1].clone(),
                arguments.readable[2].clone(),
            ),
        };
        if matches!(kind, InvokeKind::Apply) && matches!(list, Value::Null | Value::Undefined) {
            return Ok(Self::Call(Box::new(InvokeCall {
                target,
                receiver,
                arguments: Vec::new(),
            })));
        }
        Ok({
            let __pending_field_value = list;
            let __pending_field_resume = InvokeResume(Box::new(InvokeResumeState {
                pending_effect: InvokeStepPending::default(),
                realm,
                target: ForwardTarget::Call { target, receiver },
            }));
            Self::request_arguments(__pending_field_value, __pending_field_resume)
        })
    }

    pub(crate) fn start_spread(
        runtime: &Runtime,
        realm: ContextId,
        kind: crate::engine::code::bytecode::ApplyKind,
        target: Value,
        receiver: Value,
        value: Value,
    ) -> Result<Self, RuntimeError> {
        // OP_apply validates callability even for construct mode, before argsList.
        let callable = runtime.callable_from_value(target.clone())?;
        if matches!(value, Value::Null | Value::Undefined) {
            return Ok(Self::Call(Box::new(InvokeCall {
                target: DirectCallTarget::Callable(callable),
                receiver,
                arguments: Vec::new(),
            })));
        }
        let target = match kind {
            crate::engine::code::bytecode::ApplyKind::Call => ForwardTarget::Call {
                target: DirectCallTarget::Callable(callable),
                receiver,
            },
            crate::engine::code::bytecode::ApplyKind::Construct => ForwardTarget::Construct {
                target,
                new_target: Some(ConstructNewTarget::Raw(receiver)),
            },
        };
        Ok({
            let __pending_field_value = value;
            let __pending_field_resume = InvokeResume(Box::new(InvokeResumeState {
                pending_effect: InvokeStepPending::default(),
                realm,
                target,
            }));
            Self::request_arguments(__pending_field_value, __pending_field_resume)
        })
    }
}
impl InvokeResume {
    pub(crate) fn arguments(
        self,
        runtime: &Runtime,
        result: NativeConversion<Vec<Value>>,
    ) -> Result<InvokeStep, RuntimeError> {
        let arguments = match result {
            NativeConversion::Throw(value) => {
                return Ok(InvokeStep::Complete(Completion::Throw(value)));
            }
            NativeConversion::Value(arguments) => arguments,
        };
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_observed(
            "invoke.argv_carrier",
            arguments.capacity(),
            size_of::<Value>(),
        );
        Ok(match self.0.target {
            ForwardTarget::Call { target, receiver } => InvokeStep::Call(Box::new(InvokeCall {
                target,
                receiver,
                arguments,
            })),
            ForwardTarget::Construct { target, new_target } => {
                let target = match runtime.constructor_from_value(self.0.realm, target)? {
                    NativeConversion::Value(target) => target,
                    NativeConversion::Throw(value) => {
                        return Ok(InvokeStep::Complete(Completion::Throw(value)));
                    }
                };
                let new_target =
                    new_target.unwrap_or_else(|| ConstructNewTarget::Validated(target.clone()));
                InvokeStep::Construct(Box::new(InvokeConstruct {
                    target,
                    new_target,
                    arguments,
                }))
            }
        })
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: InvokeStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            InvokeStep::Complete(result) => return Ok(result),
            InvokeStep::Construct(request) => {
                let target = request.target;
                let new_target = request.new_target;
                let arguments = request.arguments;
                {
                    return runtime.construct_internal_with_new_target(
                        realm, &target, new_target, &arguments,
                    );
                }
            }
            InvokeStep::Arguments { mut resume } => {
                let value = resume.take_arguments_value();
                resume.arguments(
                    runtime,
                    finish_arguments(runtime, realm, ArgumentsStep::start(runtime, realm, value)?)?,
                )?
            }
            InvokeStep::Call(request) => {
                let target = request.target;
                let receiver = request.receiver;
                let arguments = request.arguments;
                {
                    return match target {
                        DirectCallTarget::Callable(target) => {
                            runtime.call_internal(realm, &target, receiver, &arguments)
                        }
                        DirectCallTarget::NonCallableProxy(proxy) => {
                            runtime.call_proxy(realm, &proxy, receiver, &arguments)
                        }
                    };
                }
            }
        };
    }
}

#[derive(Default)]
struct InvokeStepPending {
    arguments_value: Option<Value>,
}
impl InvokeStep {
    pub(crate) fn request_arguments(value: Value, mut resume: InvokeResume) -> Self {
        resume.0.pending_effect.arguments_value = Some(value);
        Self::Arguments { resume }
    }
}
impl InvokeResume {
    pub(crate) fn take_arguments_value(&mut self) -> Value {
        self.0
            .pending_effect
            .arguments_value
            .take()
            .expect("InvokeStep Arguments value")
    }
}
const _: () = assert!(std::mem::size_of::<InvokeStep>() <= 64);

pub(crate) struct InvokeCall {
    pub(crate) target: DirectCallTarget,
    pub(crate) receiver: Value,
    pub(crate) arguments: Vec<Value>,
}

pub(crate) struct InvokeConstruct {
    pub(crate) target: ConstructorRef,
    pub(crate) new_target: ConstructNewTarget,
    pub(crate) arguments: Vec<Value>,
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<InvokeStep>() <= 64);
