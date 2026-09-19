//! Constructor dispatch returns owned requests; bytecode bodies use ordinary child frames.
use super::{
    BytecodeCallRequest, CallableExecution, Completion, Error, NativeConversion, Next,
    OperationTarget, Query, Resume, ReturnOwner, ReturnTarget, ReturnValue, RunningExecution,
    Runtime, Step, Value, overflow, runtime_error_to_vm_error,
};
use crate::engine::value::JsValue;
use crate::engine::{
    code::function::metadata::ConstructorKind,
    vm::call::{ConstructNewTarget, ConstructorRef, ConstructorTarget, NormalizedConstructor},
};

// Keep the return owner, reply identity and original constructor operands explicit at this suspension boundary.
#[allow(clippy::too_many_arguments)]
pub(super) fn start(
    runtime: &Runtime,
    owner: ReturnOwner,
    identity: u64,
    realm: crate::engine::heap::ContextId,
    constructor: ConstructorRef,
    new_target: ConstructNewTarget,
    arguments: Vec<JsValue>,
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
                new_target: new_target.into_value(),
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
                receiver: JsValue::Undefined,
                new_target: new_target.into_value(),
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
                    receiver: Some(Completion::Return(JsValue::Undefined)),
                    derived: Some(true),
                    resume: Some(resume),
                }),
                ConstructorKind::Base if matches!(request.new_target, JsValue::Undefined) => {
                    prototype(
                        runtime,
                        request,
                        Completion::Return(JsValue::Undefined),
                        resume,
                    )
                }
                ConstructorKind::Base => Ok(Step::ReadValue {
                    receiver: Some(
                        runtime
                            .root_and_release_jsvalue(std::mem::replace(
                                &mut request.new_target,
                                JsValue::Undefined,
                            ))
                            .map_err(runtime_error_to_vm_error)?,
                    ),
                    key: Some(
                        runtime
                            .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Prototype)
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
    // The child frame request owns one edge; the cold constructor-return
    // record owns a duplicate.
    request.receiver = runtime
        .dup_jsvalue(&receiver)
        .map_err(runtime_error_to_vm_error)?;
    let mut entry = request.prepare(runtime, &mut execution.call_storage)?;
    entry.cold.constructor_return = Some(if derived {
        crate::engine::vm::frame::ConstructorReturn::Derived
    } else {
        crate::engine::vm::frame::ConstructorReturn::Base(
            runtime
                .root_and_release_jsvalue(receiver)
                .map_err(runtime_error_to_vm_error)?,
        )
    });
    Ok(Ok(Next::Call {
        entry: Box::new(entry),
        pc: 0,
        resume,
    }))
}
