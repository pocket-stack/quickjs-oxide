//! Constructor dispatch returns owned requests; bytecode bodies use ordinary child frames.
use super::{
    BytecodeCallRequest, CallableExecution, Completion, Error, NativeConversion, Next,
    OperationTarget, Query, Resume, ReturnOwner, ReturnTarget, ReturnValue, RunningExecution,
    Runtime, Step, Value, overflow, runtime_error_to_vm_error,
};
use crate::engine::{
    code::function::metadata::ConstructorKind,
    vm::call::{ConstructNewTarget, ConstructorRef, ConstructorTarget, NormalizedConstructor},
};

pub(super) fn start(
    runtime: &Runtime,
    owner: ReturnOwner,
    identity: u64,
    realm: crate::engine::heap::ContextId,
    constructor: ConstructorRef,
    new_target: ConstructNewTarget,
    arguments: Vec<Value>,
    resume: Resume,
) -> Result<Step, Error> {
    let normalized = match runtime
        .normalize_constructor(realm, constructor, new_target, arguments)
        .map_err(runtime_error_to_vm_error)?
    {
        NativeConversion::Value(result) => result,
        NativeConversion::Throw(value) => {
            return resume
                .resume(runtime, Completion::Throw(value))
                .map_err(runtime_error_to_vm_error);
        }
    };
    prepared(runtime, owner, identity, realm, normalized, resume)
}

pub(super) fn prepared(
    runtime: &Runtime,
    owner: ReturnOwner,
    identity: u64,
    realm: crate::engine::heap::ContextId,
    normalized: NormalizedConstructor,
    resume: Resume,
) -> Result<Step, Error> {
    let NormalizedConstructor {
        target,
        new_target,
        arguments,
    } = normalized;
    let (callable, classification) = match target {
        ConstructorTarget::Proxy(target) => {
            return Ok(Step::ConstructProxy {
                target: Some(target),
                new_target: Some(new_target),
                arguments: Some(arguments),
                resume: Some(resume),
            });
        }
        ConstructorTarget::Ordinary {
            callable,
            classification,
        } => (callable, classification),
    };
    match classification {
        CallableExecution::Native {
            target,
            realm: defining_realm,
            min_readable_args,
        } => Ok(Step::Native {
            mode: Some(crate::engine::vm::call::NativeInvokeMode::Ordinary),
            callable: Some(callable),
            target: Some(target),
            defining_realm: Some(defining_realm),
            min_readable_args: Some(min_readable_args),
            invocation: Some(crate::engine::vm::call::NativeInvocation::Construct {
                new_target: new_target.value(),
            }),
            arguments: Some(arguments),
            resume: Some(resume),
        }),
        CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        } => {
            let kind = runtime
                .0
                .state
                .borrow()
                .heap
                .function_bytecode(bytecode.bytecode_id())
                .map_err(|error| Error::internal(error.to_string()))?
                .metadata
                .constructor_kind;
            let request = Box::new(BytecodeCallRequest {
                callable,
                receiver: Value::Undefined,
                new_target: new_target.value(),
                arguments,
                bytecode,
                closure_slots,
                caller_realm: realm,
                return_to: ReturnTarget {
                    owner,
                    value_use: ReturnValue::Push,
                    tail: false,
                    operation: Some(OperationTarget::PropertyGet(identity)),
                },
            });
            match kind {
                ConstructorKind::None => Err(Error::internal(
                    "constructor bit disagrees with bytecode constructor metadata",
                )),
                ConstructorKind::Derived => Ok(Step::ConstructorReady {
                    request: Some(request),
                    receiver: Some(Completion::Return(Value::Undefined)),
                    derived: Some(true),
                    resume: Some(resume),
                }),
                ConstructorKind::Base if matches!(request.new_target, Value::Undefined) => {
                    prototype(
                        runtime,
                        request,
                        Completion::Return(Value::Undefined),
                        resume,
                    )
                }
                ConstructorKind::Base => Ok(Step::ReadValue {
                    receiver: Some(request.new_target.clone()),
                    key: Some(
                        runtime
                            .intern_property_key("prototype")
                            .map_err(|error| Error::internal(error.to_string()))?,
                    ),
                    resume: Some(Resume::ConstructorPrototype {
                        request,
                        resume: Box::new(resume),
                    }),
                }),
            }
        }
        _ => Err(Error::internal("constructor dispatch was not normalized")),
    }
}
pub(super) fn prototype(
    runtime: &Runtime,
    request: Box<BytecodeCallRequest>,
    completion: Completion,
    resume: Resume,
) -> Result<Step, Error> {
    let receiver = runtime
        .create_from_constructor_prototype_reply(
            request.caller_realm,
            &request.new_target,
            completion,
        )
        .map_err(runtime_error_to_vm_error)?;
    Ok(Step::ConstructorReady {
        request: Some(request),
        receiver: Some(receiver),
        derived: Some(false),
        resume: Some(resume),
    })
}
pub(super) fn ready(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    query: &Query,
    mut request: Box<BytecodeCallRequest>,
    receiver: Completion,
    derived: bool,
    resume: Resume,
) -> Result<Result<Next, Step>, Error> {
    let receiver = match receiver {
        Completion::Throw(value) => {
            return Ok(Err(resume
                .resume(runtime, Completion::Throw(value))
                .map_err(runtime_error_to_vm_error)?));
        }
        Completion::Return(value) => value,
    };
    if !execution
        .frames
        .can_push_with_continuations(query.continuation_depth())
        || runtime.bytecode_call_would_overflow()
    {
        return Ok(Err(resume
            .resume(runtime, overflow(runtime, request.caller_realm)?)
            .map_err(runtime_error_to_vm_error)?));
    }
    request.receiver = receiver.clone();
    let mut entry = request.prepare(runtime, &mut execution.call_storage)?;
    entry.cold.constructor_return = Some(if derived {
        crate::engine::vm::frame::ConstructorReturn::Derived
    } else {
        crate::engine::vm::frame::ConstructorReturn::Base(receiver)
    });
    Ok(Ok(Next::Call {
        entry: Box::new(entry),
        pc: 0,
        resume,
    }))
}
