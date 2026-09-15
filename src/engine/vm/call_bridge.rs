//! Temporary single-call boundary for callable families awaiting S07 entry integration.
//! The caller stays owned; no completed prefix is replayed by handing off its whole frame.
use super::{
    Completion, driver::CallStep, exception::runtime_error_to_vm_error,
    execution::RunningExecution, frame::FrameId,
};
use crate::engine::{
    api::{Error, runtime::Runtime},
    value::Value,
};

#[inline(never)]
pub(super) fn prepare(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    count: usize,
    method: bool,
    tail: bool,
    overflow: Option<crate::engine::code::rooted::FunctionBytecodeRef>,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let callable = runtime
        .callable_from_value(execution.slots.peek(&frame.window, count)?.clone())
        .map_err(runtime_error_to_vm_error)?;
    let mut arguments = Vec::new();
    arguments
        .try_reserve_exact(count)
        .map_err(|_| Error::internal("call boundary arguments allocation failed"))?;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    for _ in 0..count {
        arguments.push(execution.slots.pop(&mut frame.window)?);
    }
    arguments.reverse();
    #[cfg(feature = "profiling")]
    {
        crate::engine::api::profiling::record_call_buffer_capacity(
            "call.boundary_argv",
            0,
            arguments.capacity(),
            size_of::<Value>(),
        );
        crate::engine::api::profiling::record_call_buffer_moves(
            "call.boundary_argv",
            arguments.len(),
        );
    }
    execution.slots.pop(&mut frame.window)?;
    let receiver = if method {
        execution.slots.pop(&mut frame.window)?
    } else {
        Value::Undefined
    };
    if execution.pending_call.is_some() {
        return Err(Error::internal("call boundary overwrote a pending request"));
    }
    execution.pending_call = Some(Box::new(PendingCall {
        frame: id,
        realm,
        action: Action::Call {
            callable,
            receiver,
            arguments,
        },
        tail,
        overflow,
        #[cfg(feature = "profiling")]
        depth,
    }));
    Ok(CallStep::Entered)
}

/// Transitional internal call, owned while the resident dispatcher returns.
/// This is not an external-host delimiter or an owned domain continuation.
pub(super) struct PendingCall {
    frame: FrameId,
    realm: crate::engine::heap::ContextId,
    action: Action,
    tail: bool,
    overflow: Option<crate::engine::code::rooted::FunctionBytecodeRef>,
    #[cfg(feature = "profiling")]
    depth: usize,
}

pub(super) enum Action {
    Call {
        callable: crate::engine::object::CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
    },
}

pub(super) fn prepare_property(
    execution: &mut RunningExecution,
    frame: FrameId,
    realm: crate::engine::heap::ContextId,
    action: Action,
    _depth: usize,
) -> Result<CallStep, Error> {
    if execution.pending_call.is_some() {
        return Err(Error::internal(
            "property boundary overwrote a pending request",
        ));
    }
    execution.pending_call = Some(Box::new(PendingCall {
        frame,
        realm,
        action,
        tail: false,
        overflow: None,
        #[cfg(feature = "profiling")]
        depth: _depth,
    }));
    Ok(CallStep::Entered)
}

impl PendingCall {
    #[inline(never)]
    pub(super) fn invoke(
        self: Box<Self>,
        runtime: &Runtime,
        execution: &mut RunningExecution,
    ) -> Result<Option<Completion>, Error> {
        let Self {
            frame: id,
            realm,
            action,
            tail,
            overflow,
            #[cfg(feature = "profiling")]
            depth,
        } = *self;
        execution.frames.current_mut(id)?;
        // The request is independent of caller slots. The resident dispatcher
        // has returned before this remaining synchronous internal call begins.
        let completion = match overflow {
            Some(bytecode) => runtime.bytecode_stack_overflow_completion(realm, &bytecode),
            None => {
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_sync_call_bridge();
                match action {
                    Action::Call {
                        callable,
                        receiver,
                        arguments,
                    } => runtime.call_internal(realm, &callable, receiver, &arguments),
                }
            }
        };
        let completion = match completion {
            Ok(completion) => completion,
            Err(error) => {
                let error = runtime_error_to_vm_error(error);
                let Some(kind) =
                    crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
                else {
                    return Err(error);
                };
                Completion::Throw(
                    runtime
                        .new_native_error_from_error(realm, kind, &error)
                        .map_err(runtime_error_to_vm_error)?,
                )
            }
        };
        match completion {
            Completion::Return(value) if !tail => {
                let frame = execution.frames.current_mut(id)?;
                execution.slots.push(&mut frame.window, value)?;
                frame.resume_pc = frame
                    .fault_pc
                    .checked_add(1)
                    .ok_or_else(|| Error::internal("call boundary resume PC overflow"))?;
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_instruction(depth);
                Ok(None)
            }
            completion => Ok(Some(completion)),
        }
    }
}
