//! Original eval compiles and captures before entering an explicit child frame.
use super::{
    Completion, DirectEvalInvocation,
    call::{BytecodeCallRequest, CallableExecution},
    driver::{CallStep, push_frame},
    eval_bindings::{self, PreparedEvalEnvironment},
    exception::runtime_error_to_vm_error,
    execution::RunningExecution,
    frame::{FrameId, OperationTarget, ReturnTarget, ReturnValue},
};
use crate::engine::{
    api::{Error, runtime::Runtime},
    builtins::DirectEvalPreparation,
    code::function::metadata::EvalBindingSource,
    value::{Value, conversion::NativeConversion},
};

#[inline(never)]
pub(super) fn step(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    arguments: u16,
    environment: u16,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let function = execution
        .slots
        .peek(&frame.window, usize::from(arguments))?;
    if !runtime
        .is_original_eval(realm, function)
        .map_err(runtime_error_to_vm_error)?
    {
        return super::driver::enter_call(runtime, execution, id, arguments, false, false);
    }
    let result = prepare_and_enter(runtime, execution, id, arguments, environment);
    if !matches!(result, Ok(CallStep::Entered)) {
        execution.frames.current_mut(id)?.cold.eval_arguments = None;
    }
    let Err(error) = result else { return result };
    let Some(kind) =
        crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
    else {
        return Err(error);
    };
    Ok(CallStep::Complete(Completion::Throw(
        runtime
            .new_native_error_from_error(realm, kind, &error)
            .map_err(runtime_error_to_vm_error)?,
    )))
}

#[inline(never)]
fn prepare_and_enter(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    arguments: u16,
    environment: u16,
) -> Result<CallStep, Error> {
    let can_push = execution.frames.can_push();
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let input = if let Some(values) = &frame.cold.eval_arguments {
        values.first().cloned().unwrap_or(Value::Undefined)
    } else if arguments == 0 {
        Value::Undefined
    } else {
        execution
            .slots
            .peek(&frame.window, usize::from(arguments) - 1)?
            .clone()
    };
    let string = matches!(input, Value::String(_));
    let this_value = if !string {
        frame.cold.input.this_value.clone()
    } else if let Some(value) = frame
        .cold
        .rare
        .get()
        .and_then(|rare| rare.normalized_this.as_ref())
    {
        value.clone()
    } else if frame.executable.metadata.strict
        || matches!(frame.cold.input.this_value, Value::Object(_))
    {
        frame.cold.input.this_value.clone()
    } else if matches!(frame.cold.input.this_value, Value::Null | Value::Undefined) {
        Value::Object(frame.cold.input.callee_global(runtime, realm)?.clone())
    } else {
        let value = match runtime
            .native_to_object(realm, frame.cold.input.this_value.clone())
            .map_err(runtime_error_to_vm_error)?
        {
            NativeConversion::Value(object) => Value::Object(object),
            NativeConversion::Throw(value) => {
                return Ok(CallStep::Complete(Completion::Throw(value)));
            }
        };
        frame.cold.normalized_this = Some(value.clone());
        value
    };
    let prepared = if string {
        frame
            .executable
            .ensure_root(runtime)
            .map_err(runtime_error_to_vm_error)?;
        let descriptor = frame
            .executable
            .eval_environment(environment)
            .ok_or_else(|| Error::internal("eval environment index is out of bounds"))?;
        let (locals, parameters) = execution.slots.binding_counts(&frame.window)?;
        eval_bindings::validate(
            runtime,
            &frame.executable,
            &descriptor,
            frame.executable.metadata.strict,
            locals,
            parameters,
            &frame.cold.closure_slots,
        )?;
        Some(PreparedEvalEnvironment {
            index: environment,
            descriptor,
        })
    } else {
        None
    };
    let invocation = DirectEvalInvocation {
        input,
        environment,
        this_value,
        new_target: frame.cold.input.new_target.clone(),
        caller_strict: frame.executable.metadata.strict,
    };
    let prepared = runtime
        .prepare_direct_eval_original(realm, invocation, prepared, |prepared| {
            eval_bindings::materialize(prepared, &frame.cold.closure_slots, |source, descriptor| {
                let binding = match source {
                    EvalBindingSource::Local(index) => {
                        execution.slots.local_mut(&frame.window, index)?
                    }
                    EvalBindingSource::Argument(index) => {
                        execution.slots.parameter_mut(&frame.window, index)?
                    }
                    EvalBindingSource::Closure(_) => {
                        return Err(Error::internal("eval closure reached owned capture"));
                    }
                };
                super::bindings::capture_frame_binding(runtime, binding, descriptor)
            })
        })
        .map_err(runtime_error_to_vm_error)?;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let request = match prepared {
        DirectEvalPreparation::Complete(completion) => {
            frame.cold.eval_arguments = None;
            for _ in 0..=arguments {
                execution.slots.pop(&mut frame.window)?;
            }
            match completion {
                Completion::Return(value) => {
                    execution.slots.push(&mut frame.window, value)?;
                    None
                }
                completion => return Ok(CallStep::Complete(completion)),
            }
        }
        DirectEvalPreparation::Ready {
            callable,
            this_value,
        } => {
            let CallableExecution::Bytecode {
                bytecode,
                closure_slots,
            } = runtime
                .bytecode_for_callable(&callable)
                .map_err(runtime_error_to_vm_error)?
            else {
                return Err(Error::internal("prepared eval was not bytecode"));
            };
            if !can_push || runtime.bytecode_call_would_overflow() {
                return runtime
                    .bytecode_stack_overflow_completion(realm, &bytecode)
                    .map(CallStep::Complete)
                    .map_err(runtime_error_to_vm_error);
            }
            Some(BytecodeCallRequest {
                callable,
                receiver: this_value,
                new_target: Value::Undefined,
                arguments: Vec::new(),
                bytecode,
                closure_slots,
                caller_realm: realm,
                return_to: ReturnTarget {
                    value_use: ReturnValue::Push,
                    owner: crate::engine::vm::frame::ReturnOwner::Frame(id),
                    tail: false,
                    operation: Some(OperationTarget::Eval(arguments)),
                },
            })
        }
    };
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("eval resume PC overflow"))?;
    if let Some(request) = request {
        let entry = request.prepare(runtime, &mut execution.call_storage)?;
        push_frame(execution, entry)?;
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Entered)
}

#[inline(never)]
pub(super) fn apply(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    environment: u16,
) -> Result<CallStep, Error> {
    let can_push = execution.frames.can_push();
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let Value::Object(array) = execution.slots.peek(&frame.window, 0)? else {
        return Ok(CallStep::Complete(Completion::Throw(
            runtime
                .new_native_error(
                    realm,
                    crate::engine::api::error::NativeErrorKind::Type,
                    "not a object",
                )
                .map_err(runtime_error_to_vm_error)?,
        )));
    };
    let Some(values) = runtime
        .prepare_fast_array_arguments(realm, array)
        .map_err(runtime_error_to_vm_error)?
    else {
        return Ok(CallStep::Bridge);
    };
    let mut values = match values {
        NativeConversion::Value(values) => values,
        NativeConversion::Throw(value) => return Ok(CallStep::Complete(Completion::Throw(value))),
    };
    let function = execution.slots.peek(&frame.window, 1)?;
    if runtime
        .is_original_eval(realm, function)
        .map_err(runtime_error_to_vm_error)?
    {
        if frame.cold.eval_arguments.is_some() {
            return Err(Error::internal("eval argument snapshot was already active"));
        }
        frame.cold.eval_arguments = Some(values);
        return step(runtime, execution, id, 1, environment);
    }
    let Value::Object(function) = function else {
        return Ok(CallStep::Bridge);
    };
    let Some(mut callable) = runtime
        .as_callable(function)
        .map_err(runtime_error_to_vm_error)?
    else {
        return Ok(CallStep::Bridge);
    };
    let mut receiver = Value::Undefined;
    let (bytecode, closure_slots) = loop {
        match runtime
            .bytecode_for_callable(&callable)
            .map_err(runtime_error_to_vm_error)?
        {
            CallableExecution::Bytecode {
                bytecode,
                closure_slots,
            } => break (bytecode, closure_slots),
            CallableExecution::Bound {
                target,
                this_value,
                arguments,
            } => {
                values = match runtime
                    .concatenate_bound_arguments(realm, &arguments, &values)
                    .map_err(runtime_error_to_vm_error)?
                {
                    NativeConversion::Value(values) => values,
                    NativeConversion::Throw(value) => {
                        return Ok(CallStep::Complete(Completion::Throw(value)));
                    }
                };
                callable = target;
                receiver = this_value;
            }
            _ => return Ok(CallStep::Bridge),
        }
    };
    if runtime
        .0
        .state
        .borrow()
        .heap
        .function_bytecode(bytecode.bytecode_id())
        .map_err(|e| Error::internal(e.to_string()))?
        .metadata
        .function_kind
        != crate::engine::code::function::metadata::FunctionKind::Normal
    {
        return Ok(CallStep::Bridge);
    }
    if !can_push || runtime.bytecode_call_would_overflow() {
        return runtime
            .bytecode_stack_overflow_completion(realm, &bytecode)
            .map(CallStep::Complete)
            .map_err(runtime_error_to_vm_error);
    }
    let request = BytecodeCallRequest {
        callable,
        receiver,
        new_target: Value::Undefined,
        arguments: values,
        bytecode,
        closure_slots,
        caller_realm: realm,
        return_to: ReturnTarget {
            value_use: ReturnValue::Push,
            owner: crate::engine::vm::frame::ReturnOwner::Frame(id),
            tail: false,
            operation: Some(OperationTarget::Eval(1)),
        },
    };
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("apply eval resume PC overflow"))?;
    let entry = request.prepare(runtime, &mut execution.call_storage)?;
    push_frame(execution, entry)?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Entered)
}
