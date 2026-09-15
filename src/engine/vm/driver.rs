//! Own frames and advance ordinary bytecode calls without native recursion.

mod cold;
mod ordinary;
mod ready;

use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::code::function::metadata::FunctionKind;
use crate::engine::value::Value;
use crate::engine::value::conversion::NativeConversion;
#[cfg(all(test, feature = "profiling"))]
use crate::engine::vm::BytecodePc;
use crate::engine::vm::Completion;
use crate::engine::vm::call::{BytecodeCallRequest, CallableExecution};
use crate::engine::vm::exception::runtime_error_to_vm_error;
use crate::engine::vm::execution::{ExecutionLimits, RunningExecution};
use crate::engine::vm::frame::{Frame, FrameEntry, FrameId, ReturnTarget};
use crate::engine::vm::run::{RunExit, run};
#[cfg(test)]
use crate::engine::vm::stack::FrameStorage;

pub(super) fn push_frame(
    execution: &mut RunningExecution,
    entry: FrameEntry,
) -> Result<FrameId, Error> {
    execution
        .call_storage
        .reserve_depth(execution.frames.depth() + 1)?;
    let prepared = execution.frames.prepare_push()?;
    let window = if entry.initialize_bindings {
        execution.slots.push_initialized_frame(
            &entry.executable.frame_layout(),
            entry.storage,
            &entry.cold.function,
            entry.executable.metadata.function_name_local,
        )?
    } else {
        execution
            .slots
            .push_frame(&entry.executable.frame_layout(), entry.storage)?
    };
    let mut cold = entry.cold;
    cold.executable = entry.executable.into();
    cold.window = window.into();
    Ok(prepared.install(Frame {
        property_generation: entry.property_generation,
        iterator_generation: entry.iterator_generation,
        caller_realm: entry.caller_realm,
        active_frame: entry.active_frame,
        cold,
        fault_pc: 0,
        resume_pc: 0,
    }))
}

fn push_direct_call_frame(
    execution: &mut RunningExecution,
    parent: FrameId,
    entry: FrameEntry,
    count: usize,
    method: bool,
) -> Result<FrameId, Error> {
    if !entry.initialize_bindings || !entry.storage.original_arguments.is_empty() {
        return Err(Error::internal(
            "direct call received materialized arguments",
        ));
    }
    execution
        .call_storage
        .reserve_depth(execution.frames.depth() + 1)?;
    let mut prepared = execution.frames.prepare_push()?;
    let frame = prepared.current_mut(parent)?;
    let resume = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("call resume PC overflow"))?;
    let window = execution.slots.push_call_frame(
        &entry.executable.frame_layout(),
        &mut frame.window,
        count,
        method,
        &entry.cold.function,
        entry.executable.metadata.function_name_local,
    )?;
    frame.resume_pc = resume;
    let mut cold = entry.cold;
    cold.executable = entry.executable.into();
    cold.window = window.into();
    Ok(prepared.install(Frame {
        property_generation: entry.property_generation,
        iterator_generation: entry.iterator_generation,
        caller_realm: entry.caller_realm,
        active_frame: entry.active_frame,
        cold,
        fault_pc: 0,
        resume_pc: 0,
    }))
}

pub(super) fn prepare_captured_reuse(
    frame: &mut Frame,
    slots: &super::stack::SlotStore,
) -> Result<(), Error> {
    if !frame.executable.has_captured_locals && frame.cold.reusable_captured_locals.is_empty() {
        return Ok(());
    }
    if frame.cold.reusable_captured_locals.len() != frame.executable.local_definitions.len() {
        return Err(Error::internal(
            "reusable captured-local flags disagree with the frame",
        ));
    }
    let body = &mut *frame.cold;
    for (index, reusable) in body.owners.reusable_captured_locals.iter_mut().enumerate() {
        *reusable = matches!(
            slots.local(&body.window, index as u16)?,
            super::bindings::FrameBinding::Captured(_)
        );
    }
    Ok(())
}

pub(super) enum CallStep {
    Entered,
    Complete(Completion),
    Bridge,
}

/// Shared cold completion for a rejected call or constructor operand.
pub(super) fn rejected_call(
    runtime: &Runtime,
    realm: crate::engine::heap::ContextId,
    error: Error,
) -> Result<CallStep, Error> {
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

/// All classification and allocation occurs after publishing the caller's PC.
/// Unsupported callable kinds leave every operand and the resume PC untouched.
pub(super) fn enter_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    count: u16,
    method: bool,
    tail: bool,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let window = &mut frame.cold.window;
    let count = usize::from(count);
    execution.slots.peek(window, count + usize::from(method))?;
    let mut callable =
        match runtime.direct_call_target_from_value(execution.slots.peek(window, count)?.clone()) {
            Ok(super::call::DirectCallTarget::Callable(callable)) => callable,
            Ok(super::call::DirectCallTarget::NonCallableProxy(proxy)) => {
                // Pinned direct calls observe a non-callable Proxy's apply getter
                // before reporting its missing [[Call]] capability.
                let depth = execution.slots.depth(window);
                let mut arguments = Vec::new();
                arguments
                    .try_reserve_exact(count)
                    .map_err(|_| Error::internal("Proxy call arguments allocation failed"))?;
                for _ in 0..count {
                    arguments.push(execution.slots.pop(window)?);
                }
                arguments.reverse();
                execution.slots.pop(window)?;
                let receiver = if method {
                    execution.slots.pop(window)?
                } else {
                    Value::Undefined
                };
                return super::proxy_get_driver::start_call(
                    runtime, execution, id, proxy, receiver, arguments, tail, depth,
                );
            }
            Err(error) => return rejected_call(runtime, realm, runtime_error_to_vm_error(error)),
        };
    // Keep the existing rejection order and exception materialization until
    // general call errors join the owned unwind path. Nothing was consumed.
    if !execution
        .slots
        .validate_call_value_domains(window, runtime, count, method)?
    {
        return Ok(CallStep::Bridge);
    }
    let mut bound_arguments = None;
    let mut bound_receiver = None;
    let (bytecode, closure_slots) = loop {
        if let Some(mut selected) = super::frames::NativeClassification::select(runtime, &callable)
            .map_err(runtime_error_to_vm_error)?
            && let Some(kind) = selected.take_operation()
        {
            let target = selected.target();
            let defining_realm = selected.defining_realm();
            let min_readable_args = selected.minimum();
            let depth = execution.slots.depth(window);
            execution.slots.reserve_native_argument_depth(
                runtime.0.active_frame_depth.get().saturating_add(1),
            )?;
            let (arguments, receiver) = execution
                .slots
                .take_native_call_operands(window, count, method)?;
            return super::proxy_get_driver::start_native_with_classification(
                runtime,
                execution,
                id,
                callable,
                target,
                defining_realm,
                min_readable_args,
                bound_receiver.unwrap_or(receiver),
                bound_arguments.unwrap_or(arguments),
                tail,
                depth,
                Some(selected),
                Some(kind),
            );
        }

        match runtime
            .bytecode_for_callable(&callable)
            .map_err(runtime_error_to_vm_error)?
        {
            CallableExecution::Bytecode {
                bytecode,
                closure_slots,
            } => {
                break (bytecode, closure_slots);
            }
            CallableExecution::Bound {
                target,
                this_value,
                arguments,
            } => {
                let call_arguments = match bound_arguments.take() {
                    Some(arguments) => arguments,
                    None => {
                        let mut arguments = Vec::new();
                        arguments
                            .try_reserve_exact(count)
                            .map_err(|_| Error::internal("call arguments allocation failed"))?;
                        for offset in (0..count).rev() {
                            arguments.push(execution.slots.peek(window, offset)?.clone());
                        }
                        arguments
                    }
                };
                bound_arguments = Some(
                    match runtime
                        .concatenate_bound_arguments(realm, &arguments, &call_arguments)
                        .map_err(runtime_error_to_vm_error)?
                    {
                        NativeConversion::Value(arguments) => arguments,
                        NativeConversion::Throw(value) => {
                            return Ok(CallStep::Complete(Completion::Throw(value)));
                        }
                    },
                );
                bound_receiver = Some(this_value);
                callable = target;
            }
            CallableExecution::Proxy => {
                let depth = execution.slots.depth(window);
                let mut arguments = Vec::new();
                arguments
                    .try_reserve_exact(count)
                    .map_err(|_| Error::internal("Proxy call arguments allocation failed"))?;
                for _ in 0..count {
                    arguments.push(execution.slots.pop(window)?);
                }
                arguments.reverse();
                execution.slots.pop(window)?;
                let receiver = if method {
                    execution.slots.pop(window)?
                } else {
                    Value::Undefined
                };
                return super::proxy_get_driver::start_call(
                    runtime,
                    execution,
                    id,
                    callable.as_object().clone(),
                    bound_receiver.unwrap_or(receiver),
                    bound_arguments.unwrap_or(arguments),
                    tail,
                    depth,
                );
            }
            CallableExecution::Native {
                target,
                realm: defining_realm,
                min_readable_args,
            } if super::frames::native_operation(runtime, &callable)
                .map_err(runtime_error_to_vm_error)?
                .is_some() =>
            {
                let depth = execution.slots.depth(window);
                execution.slots.reserve_native_argument_depth(
                    runtime.0.active_frame_depth.get().saturating_add(1),
                )?;
                let (arguments, receiver) = execution
                    .slots
                    .take_native_call_operands(window, count, method)?;
                return super::proxy_get_driver::start_classified_native_call(
                    runtime,
                    execution,
                    id,
                    callable,
                    target,
                    defining_realm,
                    min_readable_args,
                    bound_receiver.unwrap_or(receiver),
                    bound_arguments.unwrap_or(arguments),
                    tail,
                    depth,
                );
            }
            _ => {
                return super::call_bridge::prepare(
                    runtime, execution, id, count, method, tail, None,
                );
            }
        }
    };
    let kind = runtime
        .0
        .state
        .borrow()
        .heap
        .function_bytecode(bytecode.bytecode_id())
        .map_err(|error| Error::internal(error.to_string()))?
        .metadata
        .function_kind;
    #[cfg(feature = "profiling")]
    let observed_depth = execution.slots.depth(window);
    if !execution.frames.can_push() || runtime.bytecode_call_would_overflow() {
        return runtime
            .bytecode_stack_overflow_completion(realm, &bytecode)
            .map(CallStep::Complete)
            .map_err(runtime_error_to_vm_error);
    }
    if kind == FunctionKind::Normal && bound_arguments.is_none() {
        let frame = execution.frames.current_mut(id)?;
        let receiver = if method {
            super::stack::copy_value(execution.slots.peek(&frame.window, count + 1)?)?
        } else {
            Value::Undefined
        };
        let request = BytecodeCallRequest {
            callable,
            receiver,
            new_target: Value::Undefined,
            arguments: Vec::new(),
            bytecode,
            closure_slots,
            caller_realm: realm,
            return_to: ReturnTarget {
                value_use: crate::engine::vm::frame::ReturnValue::Push,
                owner: crate::engine::vm::frame::ReturnOwner::Frame(id),
                tail,
                operation: None,
            },
        };
        let entry = request.prepare(runtime, &mut execution.call_storage)?;
        push_direct_call_frame(execution, id, entry, count, method)?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_instruction(observed_depth);
        return Ok(CallStep::Entered);
    }
    let mut arguments = execution.slots.take_argument_buffer(count)?;
    let frame = execution.frames.current_mut(id)?;
    for _ in 0..count {
        arguments.push(execution.slots.pop(&mut frame.window)?);
    }
    arguments.reverse();
    let function = execution.slots.pop(&mut frame.window)?;
    let receiver = if method {
        execution.slots.pop(&mut frame.window)?
    } else {
        Value::Undefined
    };
    // The checked callable root now owns the popped callee identity.
    drop(function);
    if let Some(normalized) = bound_arguments {
        arguments = normalized;
    }
    let receiver = bound_receiver.unwrap_or(receiver);
    if kind != FunctionKind::Normal {
        let depth = execution.slots.depth(&frame.window) + count + 1 + usize::from(method);
        return super::proxy_get_driver::start_callback_call(
            runtime, execution, id, callable, receiver, arguments, tail, depth,
        );
    }
    let request = BytecodeCallRequest {
        callable,
        receiver,
        new_target: Value::Undefined,
        arguments,
        bytecode,
        closure_slots,
        caller_realm: realm,
        return_to: ReturnTarget {
            value_use: crate::engine::vm::frame::ReturnValue::Push,
            owner: crate::engine::vm::frame::ReturnOwner::Frame(id),
            tail,
            operation: None,
        },
    };
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("call resume PC overflow"))?;
    let entry = request.prepare(runtime, &mut execution.call_storage)?;
    push_frame(execution, entry)?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(observed_depth);
    Ok(CallStep::Entered)
}

pub(super) fn execute(
    runtime: Runtime,
    entry: FrameEntry,
    limits: ExecutionLimits,
) -> Result<RunningExit, Error> {
    let mut execution = RunningExecution::new(&runtime, limits)?;
    push_frame(&mut execution, entry)?;
    run_frames(&runtime, execution)
}

pub(crate) enum RootOperation {
    Call {
        callable: crate::engine::object::CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
    },
    Construct(super::call::NormalizedConstructor),
    Get {
        object: crate::engine::object::ObjectRef,
        key: crate::engine::object::PropertyKey,
        receiver: Value,
    },
    Own {
        object: crate::engine::object::ObjectRef,
        key: crate::engine::object::PropertyKey,
    },
    Define {
        object: crate::engine::object::ObjectRef,
        key: crate::engine::object::PropertyKey,
        descriptor: crate::engine::object::OrdinaryPropertyDescriptor,
    },
    Set {
        object: crate::engine::object::ObjectRef,
        key: crate::engine::object::PropertyKey,
        value: Value,
        receiver: Value,
    },
    ModuleCallback(crate::engine::modules::callback::CallbackStep),
    ModuleEvaluation(crate::engine::modules::evaluation::EvaluationStep),
    ModuleLink(crate::engine::modules::link::LinkStep),
    AsyncGenerator(super::async_generator::AsyncGeneratorStep),
    FromSync(super::async_from_sync_iterator::FromSyncStep),
    Promise(crate::engine::builtins::promise::operation::PromiseStep),
    Async(super::async_function::AsyncStep),
}

pub(crate) fn execute_root(
    runtime: Runtime,
    realm: crate::engine::heap::ContextId,
    operation: RootOperation,
) -> Result<Completion, Error> {
    start_root(runtime.clone(), realm, operation)?.finish(runtime)
}
pub(super) fn execute_root_descriptor(
    runtime: Runtime,
    realm: crate::engine::heap::ContextId,
    operation: RootOperation,
) -> Result<super::entry::DescriptorReply, Error> {
    let mut exit = start_root(runtime.clone(), realm, operation)?;
    loop {
        match exit {
            RunningExit::RootDescriptor(result) => return Ok(result),
            RunningExit::Complete(Completion::Throw(value)) => {
                return Ok(crate::engine::value::conversion::NativeConversion::Throw(
                    value,
                ));
            }
            RunningExit::Call(mut continuation) => {
                let forwarded = continuation.invoke(&runtime)?;
                exit = continuation.resume(&runtime, forwarded)?;
            }
            _ => {
                return Err(Error::internal(
                    "descriptor entry returned an untyped terminal result",
                ));
            }
        }
    }
}
fn start_root(
    runtime: Runtime,
    realm: crate::engine::heap::ContextId,
    operation: RootOperation,
) -> Result<RunningExit, Error> {
    let mut execution = RunningExecution::new(&runtime, ExecutionLimits::default())?;
    match super::proxy_get_driver::start_root(&runtime, &mut execution, realm, operation)? {
        super::proxy_get_driver::Progress::Call(CallStep::Complete(completion)) => {
            Ok(RunningExit::Complete(completion))
        }
        super::proxy_get_driver::Progress::Call(CallStep::Entered) => {
            run_frames(&runtime, execution)
        }
        _ => Err(Error::internal(
            "root operation returned a bytecode-only continuation",
        )),
    }
}

pub(super) enum RunningExit {
    RootDescriptor(super::entry::DescriptorReply),
    Complete(Completion),
    RootHandoff(Box<super::frame_exit::RootHandoff>),
    Call(Box<CallContinuation>),
    Suspend(Box<super::suspend::OwnedSuspension>),
}

pub(super) struct CallContinuation {
    execution: RunningExecution,
    conversion: Option<super::conversion_driver::ConversionTask>,
    next_operation: u64,
}

impl CallContinuation {
    #[inline(never)]
    fn invoke(&mut self, runtime: &Runtime) -> Result<Option<Completion>, Error> {
        let call = self
            .execution
            .pending_call
            .take()
            .ok_or_else(|| Error::internal("call continuation has no request"))?;
        call.invoke(runtime, &mut self.execution)
    }

    #[inline(never)]
    fn resume(
        self: Box<Self>,
        runtime: &Runtime,
        forwarded: Option<Completion>,
    ) -> Result<RunningExit, Error> {
        let Self {
            execution,
            conversion,
            next_operation,
        } = *self;
        run_frames_with_state(runtime, execution, forwarded, conversion, next_operation)
    }
}

impl RunningExit {
    pub(super) fn finish_suspending(
        self,
        runtime: Runtime,
    ) -> Result<super::suspend::VmRunOutcome, Error> {
        let mut exit = self;
        loop {
            match exit {
                Self::RootDescriptor(_) => {
                    return Err(Error::internal("bytecode entry returned a root descriptor"));
                }
                Self::Complete(completion) => {
                    return Ok(super::suspend::VmRunOutcome::Complete(completion));
                }
                Self::Suspend(suspension) => {
                    return suspension
                        .freeze(runtime)
                        .map_err(runtime_error_to_vm_error);
                }
                Self::RootHandoff(handoff) => return handoff.execute_suspending(runtime),
                Self::Call(mut continuation) => {
                    let forwarded = continuation.invoke(&runtime)?;
                    exit = continuation.resume(&runtime, forwarded)?;
                }
            }
        }
    }

    pub(super) fn finish(self, runtime: Runtime) -> Result<Completion, Error> {
        let mut exit = self;
        loop {
            match exit {
                Self::RootDescriptor(_) => {
                    return Err(Error::internal("bytecode entry returned a root descriptor"));
                }
                Self::Suspend(_) => return Err(Error::internal("ordinary entry suspended")),
                Self::Complete(completion) => return Ok(completion),
                Self::RootHandoff(handoff) => return handoff.execute(runtime),
                Self::Call(mut continuation) => {
                    let forwarded = continuation.invoke(&runtime)?;
                    exit = continuation.resume(&runtime, forwarded)?;
                }
            }
        }
    }
}

/// Install an authenticated dormant frame and inject abrupt resumption into
/// the same unwinder used by ordinary child-frame throws.
pub(super) fn resume(runtime: Runtime, entry: FrameEntry, pc: usize) -> Result<RunningExit, Error> {
    let mut execution = RunningExecution::new(&runtime, ExecutionLimits::default())?;
    let id = push_frame(&mut execution, entry)?;
    let frame = execution.frames.current_mut(id)?;
    frame.resume_pc = pc;
    frame.fault_pc = pc.saturating_sub(1);
    run_frames_with_state(&runtime, execution, None, None, 0)
}

#[inline(never)]
fn run_frames(runtime: &Runtime, execution: RunningExecution) -> Result<RunningExit, Error> {
    run_frames_with_state(runtime, execution, None, None, 0)
}

#[inline(never)]
fn run_frames_with_state(
    runtime: &Runtime,
    mut execution: RunningExecution,
    mut forwarded: Option<Completion>,
    mut conversion: Option<super::conversion_driver::ConversionTask>,
    mut next_operation: u64,
) -> Result<RunningExit, Error> {
    loop {
        if let Some(result) = execution.root_descriptor.take() {
            if execution.frames.current_id().is_some()
                || execution.root_query.is_some()
                || execution.pending_call.is_some()
                || forwarded.is_some()
                || conversion.is_some()
            {
                return Err(Error::internal(
                    "root descriptor conflicts with an active continuation",
                ));
            }
            return Ok(RunningExit::RootDescriptor(result));
        }

        if execution.pending_call.is_some() {
            if forwarded.is_some() {
                return Err(Error::internal("pending call conflicts with a completion"));
            }
            return Ok(RunningExit::Call(Box::new(CallContinuation {
                execution,
                conversion,
                next_operation,
            })));
        }
        let mut id = execution
            .frames
            .current_id()
            .ok_or_else(|| Error::internal("driver lost its current frame"))?;
        if forwarded.is_none() {
            forwarded = execution
                .frames
                .current_mut(id)?
                .cold
                .resume_throw
                .take()
                .map(Completion::Throw);
        }
        let mut conversion_prepared = false;
        let mut exit = if let Some(task) = conversion.take() {
            use crate::engine::vm::conversion_driver::Progress;
            #[cfg(feature = "profiling")]
            let operands =
                crate::engine::vm::conversion_driver::ConversionTask::operand_count(&task);
            match crate::engine::vm::conversion_driver::ConversionTask::advance(
                task,
                runtime,
                &mut execution,
            )? {
                Progress::Ready(task) => {
                    conversion = Some(task);
                    continue;
                }
                Progress::Entered => continue,
                Progress::Complete(Completion::Return(value)) => {
                    let frame = execution.frames.current_mut(id)?;
                    #[cfg(feature = "profiling")]
                    let depth = execution.slots.depth(&frame.window) + operands;
                    execution.slots.push(&mut frame.window, value)?;
                    frame.resume_pc = frame
                        .fault_pc
                        .checked_add(1)
                        .ok_or_else(|| Error::internal("conversion resume PC overflow"))?;
                    #[cfg(feature = "profiling")]
                    crate::engine::api::profiling::record_owned_instruction(depth);
                    continue;
                }
                Progress::Complete(completion) => {
                    forwarded = Some(completion);
                    RunExit::Complete
                }
                Progress::Predicate(input) => {
                    match super::predicate_driver::converted(runtime, &mut execution, id, input)? {
                        CallStep::Entered => continue,
                        CallStep::Complete(completion) => {
                            forwarded = Some(completion);
                            RunExit::Complete
                        }
                        CallStep::Bridge => {
                            return Err(Error::internal("converted predicate attempted replay"));
                        }
                    }
                }
                Progress::SuperProperty(input) => {
                    match super::super_property_driver::converted(
                        runtime,
                        &mut execution,
                        id,
                        input,
                    )? {
                        CallStep::Entered => continue,
                        CallStep::Complete(completion) => {
                            forwarded = Some(completion);
                            RunExit::Complete
                        }
                        CallStep::Bridge => {
                            return Err(Error::internal(
                                "converted super property attempted replay",
                            ));
                        }
                    }
                }
                Progress::PropertyWrite(input) => {
                    match super::property_write_driver::converted(
                        runtime,
                        &mut execution,
                        id,
                        input,
                    )? {
                        CallStep::Entered => continue,
                        CallStep::Complete(completion) => {
                            forwarded = Some(completion);
                            RunExit::Complete
                        }
                        CallStep::Bridge => {
                            return Err(Error::internal(
                                "converted property write attempted replay",
                            ));
                        }
                    }
                }
                Progress::PropertyRead(input) => {
                    match super::property_driver::read_converted(
                        runtime,
                        &mut execution,
                        id,
                        input,
                    )? {
                        CallStep::Entered => continue,
                        CallStep::Complete(completion) => {
                            forwarded = Some(completion);
                            RunExit::Complete
                        }
                        CallStep::Bridge => {
                            return Err(Error::internal(
                                "converted property read attempted replay",
                            ));
                        }
                    }
                }
            }
        } else if forwarded.is_some() {
            RunExit::Complete
        } else {
            let boundary = ready::run(runtime, &mut execution, id, &mut next_operation)?;
            id = execution
                .frames
                .current_id()
                .ok_or_else(|| Error::internal("ordinary loop lost current frame"))?;
            match boundary {
                ready::Boundary::Exit(exit) => exit,
                ready::Boundary::Entered => continue,
                ready::Boundary::Conversion(exit) => {
                    conversion_prepared = true;
                    exit
                }
                ready::Boundary::Complete(completion) => {
                    forwarded = Some(completion);
                    RunExit::Complete
                }
            }
        };
        execution.frames.materialize(runtime)?;
        match cold::dispatch(
            runtime,
            &mut execution,
            id,
            exit,
            &mut forwarded,
            &mut conversion,
            &mut next_operation,
            conversion_prepared,
        )? {
            cold::Disposition::Entered => continue,
            cold::Disposition::Complete | cold::Disposition::Rethrow => exit = RunExit::Complete,
            cold::Disposition::Bridge => exit = RunExit::Bridge,
            cold::Disposition::Suspend(kind) => exit = RunExit::Suspend(kind),
        }
        if matches!(forwarded, Some(Completion::Throw(_))) {
            let Some(Completion::Throw(value)) = forwarded.take() else {
                unreachable!()
            };
            match super::iterator_driver::unwind(runtime, &mut execution, id, value)? {
                CallStep::Entered => continue,
                CallStep::Complete(completion) => forwarded = Some(completion),
                CallStep::Bridge => {
                    return Err(Error::internal("unwind attempted instruction replay"));
                }
            }
        }
        if let RunExit::Suspend(kind) = exit {
            let suspension = super::suspend::OwnedSuspension::detach(&mut execution, id, kind)?;
            if let Some(target) = suspension.return_to {
                let outcome = Box::new(suspension)
                    .freeze(runtime.clone())
                    .map_err(runtime_error_to_vm_error)?;
                match super::proxy_get_driver::reply_suspended(
                    runtime,
                    &mut execution,
                    target,
                    outcome,
                )? {
                    super::proxy_get_driver::Progress::Conversion(task) => conversion = Some(task),
                    super::proxy_get_driver::Progress::Call(CallStep::Entered) => {}
                    super::proxy_get_driver::Progress::Call(CallStep::Complete(completion)) => {
                        if matches!(target.owner, super::frame::ReturnOwner::Root) {
                            return Ok(RunningExit::Complete(completion));
                        }
                        forwarded = Some(completion);
                    }
                    super::proxy_get_driver::Progress::Call(CallStep::Bridge) => {
                        return Err(Error::internal("suspension reply attempted replay"));
                    }
                }
                continue;
            }
            drop(execution);
            return Ok(RunningExit::Suspend(Box::new(suspension)));
        }
        let (completion, return_to) =
            match super::frame_exit::finish(runtime, &mut execution, id, exit, forwarded.take())? {
                super::frame_exit::FrameExit::Complete {
                    completion,
                    return_to,
                } => (completion, return_to),
                super::frame_exit::FrameExit::Suspended { outcome, target } => {
                    match super::proxy_get_driver::reply_suspended(
                        runtime,
                        &mut execution,
                        target,
                        outcome,
                    )? {
                        super::proxy_get_driver::Progress::Conversion(task) => {
                            conversion = Some(task)
                        }
                        super::proxy_get_driver::Progress::Call(CallStep::Entered) => {}
                        super::proxy_get_driver::Progress::Call(CallStep::Complete(completion)) => {
                            if matches!(target.owner, super::frame::ReturnOwner::Root) {
                                return Ok(RunningExit::Complete(completion));
                            }
                            forwarded = Some(completion);
                        }
                        super::proxy_get_driver::Progress::Call(CallStep::Bridge) => {
                            return Err(Error::internal(
                                "handoff suspension reply attempted replay",
                            ));
                        }
                    }
                    continue;
                }
                super::frame_exit::FrameExit::RootHandoff(handoff) => {
                    drop(execution);
                    return Ok(RunningExit::RootHandoff(handoff));
                }
            };
        let Some(target) = return_to else {
            return Ok(RunningExit::Complete(completion));
        };
        if matches!(target.owner, super::frame::ReturnOwner::Root) {
            match super::proxy_get_driver::reply(runtime, &mut execution, target, completion)? {
                super::proxy_get_driver::Progress::Call(CallStep::Entered) => continue,
                super::proxy_get_driver::Progress::Call(CallStep::Complete(completion)) => {
                    return Ok(RunningExit::Complete(completion));
                }
                _ => {
                    return Err(Error::internal(
                        "root reply returned a bytecode-only continuation",
                    ));
                }
            }
        }
        execution.frames.current_mut(target.frame()?)?;
        if matches!(
            target.operation,
            Some(super::frame::OperationTarget::PropertyGet(_))
        ) {
            match super::proxy_get_driver::reply(runtime, &mut execution, target, completion)? {
                super::proxy_get_driver::Progress::Conversion(task) => {
                    conversion = Some(task);
                    continue;
                }
                super::proxy_get_driver::Progress::Call(CallStep::Entered) => continue,
                super::proxy_get_driver::Progress::Call(CallStep::Complete(completion)) => {
                    forwarded = Some(completion);
                    continue;
                }
                super::proxy_get_driver::Progress::Call(CallStep::Bridge) => {
                    return Err(Error::internal(
                        "property reply attempted instruction replay",
                    ));
                }
            }
        }

        if matches!(
            target.operation,
            Some(super::frame::OperationTarget::Iterator(_))
        ) {
            match super::environment_driver::reply(runtime, &mut execution, target, completion)? {
                CallStep::Entered => continue,
                CallStep::Complete(completion) => {
                    forwarded = Some(completion);
                    continue;
                }
                CallStep::Bridge => {
                    return Err(Error::internal("iterator reply attempted replay"));
                }
            }
        }
        if let Some(super::frame::OperationTarget::Eval(arguments)) = target.operation {
            let parent = execution.frames.current_mut(target.frame()?)?;
            parent.cold.eval_arguments = None;
            for _ in 0..=arguments {
                execution.slots.pop(&mut parent.window)?;
            }
            match completion {
                Completion::Return(value) => execution.slots.push(&mut parent.window, value)?,
                completion => forwarded = Some(completion),
            }
            continue;
        }
        if target.operation.is_some() {
            conversion = Some(crate::engine::vm::conversion_driver::ConversionTask::reply(
                runtime,
                &mut execution,
                target,
                completion,
            )?);
            continue;
        }
        match completion {
            Completion::Return(value) if !target.tail => {
                let parent = execution.frames.current_mut(target.frame()?)?;
                if matches!(target.value_use, super::frame::ReturnValue::Push) {
                    execution.slots.push(&mut parent.window, value)?;
                }
            }
            completion => forwarded = Some(completion),
        }
    }
}

#[cfg(all(test, feature = "profiling"))]
mod tests {
    use super::*;
    use crate::engine::api::profiling::CostProfile;
    use crate::engine::vm::frame::FrameCold;

    #[test]
    fn same_frame_property_completion_keeps_getters_proxy_traps_and_error_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let costs = CostProfile::start();
        let result = context
            .eval(
                "(function(){let events=[]; let plain={x:1}; plain.x=2; let value=plain.x;
              let accessor={get x(){events.push('g');return plain.x},
                            set x(v){events.push('s'+v);plain.x=v}};
              value+=accessor.x; accessor.x=3;
              let proxy=new Proxy(plain,{get(t,k){events.push('p');return t[k]},
                                        set(t,k,v){events.push('q');t[k]=v;return true}});
              value+=proxy.x; proxy.x=4;
              let key={toString(){events.push('k');return 'x'}};
              try {null[key]} catch(e){events.push('n')}
              try {null[key]=9} catch(e){events.push('w')}
              try {new Proxy({}, {get(){throw 'boom'}}).x}
              catch(e){events.push(e)} finally {events.push('f')}
              return JSON.stringify([value,plain.x,events])})()",
            )
            .unwrap();
        assert_eq!(
            result,
            Value::String(crate::engine::value::JsString::from_static(
                "[7,4,[\"g\",\"s3\",\"p\",\"q\",\"n\",\"k\",\"w\",\"boom\",\"f\"]]"
            ))
        );
        let costs = costs.snapshot();
        assert_eq!(costs.legacy_dispatches, 0);
        assert_eq!(costs.owned_bridge_exits, 0);
        assert_eq!(costs.owned_sync_call_bridges, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn same_frame_write_rejection_is_a_throw_not_a_completed_instruction() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context
            .eval(
                "(function(){'use strict';let events='';
              let plain={};Object.defineProperty(plain,'x',{value:1,writable:false});
              try{plain.x=2;events+='bad'}catch(e){events+=e instanceof TypeError?'t':'?'}
              let proxy=new Proxy({}, {set(){events+='s';return false}});
              try{proxy.x=2;events+='bad'}catch(e){events+=e instanceof TypeError?'t':'?'}
              finally{events+='f'}return events})()",
            )
            .unwrap();
        assert_eq!(
            result,
            Value::String(crate::engine::value::JsString::from_static("tstf"))
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn same_frame_cold_loop_preserves_object_conversion_and_primitive_throw_finally() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let costs = CostProfile::start();
        let result = context
            .eval(
                "(function(){let events=''; let text='a';
              for(let i=0;i<3;i++) text+='b';
              let object={valueOf(){events+='v'; return 2}};
              let result=text+object;
              try {result+=Symbol('x')} catch(e) {events+=e instanceof TypeError?'t':'?'}
              finally {events+='f'}
              return result+':'+events})()",
            )
            .unwrap();
        assert_eq!(
            result,
            Value::String(crate::engine::value::JsString::from_static("abbb2:vtf"))
        );
        let costs = costs.snapshot();
        assert!(
            costs
                .owned_execution_events
                .get("conversion_completed_without_task")
                .copied()
                .unwrap_or(0)
                >= 4
        );
        assert_eq!(costs.legacy_dispatches, 0);
        assert_eq!(costs.owned_bridge_exits, 0);
        assert_eq!(costs.owned_sync_call_bridges, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn same_frame_conversion_identity_failure_keeps_operands_and_fault_pc() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(){return 'a'+'b'})",
            Vec::new(),
        );
        let mut execution = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let id = push_frame(&mut execution, entry).unwrap();
        let mut identity = u64::MAX;
        let result = super::ready::run(&runtime, &mut execution, id, &mut identity);
        assert!(matches!(result, Err(error) if error.message() == "conversion identity exhausted"));
        assert_eq!(identity, u64::MAX);
        let frame = execution.frames.current_mut(id).unwrap();
        assert!(matches!(
            frame.executable.code[frame.fault_pc],
            crate::engine::code::bytecode::Instruction::Add
        ));
        assert_eq!(frame.resume_pc, frame.fault_pc);
        assert_eq!(execution.slots.depth(&frame.window), 2);
        assert_eq!(
            execution.slots.peek(&frame.window, 0).unwrap(),
            &Value::String(crate::engine::value::JsString::from_static("b"))
        );
        assert_eq!(
            execution.slots.peek(&frame.window, 1).unwrap(),
            &Value::String(crate::engine::value::JsString::from_static("a"))
        );
        drop(execution);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    fn execute(
        runtime: Runtime,
        entry: FrameEntry,
        limits: ExecutionLimits,
    ) -> Result<Completion, Error> {
        super::execute(runtime.clone(), entry, limits)?.finish(runtime)
    }

    fn execute_running(runtime: Runtime, execution: RunningExecution) -> Result<Completion, Error> {
        super::run_frames(&runtime, execution)?.finish(runtime)
    }

    fn entry(
        runtime: &Runtime,
        context: &mut crate::engine::api::Context,
        source: &str,
        arguments: Vec<Value>,
    ) -> FrameEntry {
        let Value::Object(function) = context.eval(source).unwrap() else {
            panic!("expected function");
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        callable_entry(runtime, context, callable, arguments)
    }

    fn callable_entry(
        runtime: &Runtime,
        context: &mut crate::engine::api::Context,
        callable: crate::engine::object::CallableRef,
        arguments: Vec<Value>,
    ) -> FrameEntry {
        let function = callable.as_object().clone();
        let CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("expected bytecode");
        };
        let prepared = runtime
            .prepare_bytecode_frame(
                &callable,
                Value::Undefined,
                Value::Undefined,
                &arguments,
                bytecode,
            )
            .unwrap();
        let locals = prepared.locals.len();
        FrameEntry {
            initialize_bindings: false,
            executable: prepared.executable,
            property_generation: 0,
            iterator_generation: 0,
            caller_realm: context.realm,
            active_frame: prepared.active_frame.token(),
            cold: crate::engine::vm::frame::ColdFrame::new(FrameCold {
                rare: std::cell::OnceCell::new(),
                return_to: None,
                entry_guard: Some(prepared.active_frame),
                function: (function).into(),
                closure_slots,
                reusable_captured_locals: vec![false; locals],
                input: (prepared.input).into(),
            }),
            storage: FrameStorage {
                original_arguments: arguments,
                parameters: prepared.arguments,
                locals: prepared.locals,
                operands: Vec::new(),
            },
        }
    }

    #[test]
    fn lexical_initialization_and_tdz_use_owned_slots() {
        for (source, error_message) in [
            (
                "(function(){var sum=0,i=0; while(i<3){let x=i+13; sum=sum+x; i=i+1} return sum})",
                None,
            ),
            (
                "(function(){var i=0; while(i<2){if(i===1)return x; let x=42; i=i+1}})",
                Some("x is not initialized"),
            ),
            ("(function(){let x=40; x=x+1; const y=1; return x+y})", None),
            (
                "(function(){return x; let x=42})",
                Some("x is not initialized"),
            ),
            ("(function(){x=42; let x})", Some("x is not initialized")),
            (
                "(function(){let x=x; return x})",
                Some("x is not initialized"),
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(&runtime, &mut context, source, Vec::new());
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            match (completion, error_message) {
                (Completion::Return(value), None) => assert_eq!(value, Value::Int(42)),
                (Completion::Throw(Value::Object(error)), Some(message)) => {
                    for (key, expected) in [("name", "ReferenceError"), ("message", message)] {
                        assert_eq!(
                            context
                                .get_property(&error, &runtime.intern_property_key(key).unwrap())
                                .unwrap(),
                            Value::String(crate::engine::value::JsString::from_static(expected))
                        );
                    }
                }
                _ => panic!("unexpected lexical completion: {source}"),
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn closure_reads_and_writes_preserve_live_cells_and_tdz() {
        for (source, throws) in [
            (
                "(function(){var x=40;return function(){x=x+1;return x}})()",
                false,
            ),
            (
                "(function(){let x=40;return function(){x=x+1;return x}})()",
                false,
            ),
            (
                "(function(){const x=42;return function(){return x}})()",
                false,
            ),
            ("(function(){return function(){return x};let x})()", true),
            ("(function(){return function(){x=42};let x})()", true),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let closure = context.eval(source).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(f){f();return f()})",
                vec![closure],
            );
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_storage.maximum_frame_depth, 2);
            match (completion, throws) {
                (Completion::Return(value), false) => assert_eq!(value, Value::Int(42)),
                (Completion::Throw(Value::Object(error)), true) => {
                    for (key, expected) in [
                        ("name", "ReferenceError"),
                        ("message", "x is not initialized"),
                    ] {
                        assert_eq!(
                            context
                                .get_property(&error, &runtime.intern_property_key(key).unwrap())
                                .unwrap(),
                            Value::String(crate::engine::value::JsString::from_static(expected))
                        );
                    }
                }
                _ => panic!("unexpected closure completion: {source}"),
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn catch_and_finally_resume_owned_frames() {
        for (source, bridge) in [
            ("(function(){try{throw 40}catch(e){return e+2}})", false),
            ("(function(f){try{return f()}catch(e){return e+2}})", false),
            ("(function(){try{return 1}finally{return 42}})", false),
            (
                "(function(){var x=0;try{try{throw 40}finally{x=2}}catch(e){return e+x}})",
                false,
            ),
            (
                "(function(){try{String(1);throw 40}catch(e){return e+2}})",
                false,
            ),
            (
                "(function(){try{'x' in {};throw 40}catch(e){return e+2}})",
                false,
            ),
            (
                "(function(){try{[] instanceof Array;throw 40}catch(e){return e+2}})",
                false,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let throwing = context.eval("(function(){throw 40})").unwrap();
            let entry = entry(&runtime, &mut context, source, vec![throwing]);
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = completion else {
                panic!("expected caught return: {source}")
            };
            assert_eq!(value, Value::Int(42), "{source}");
            let costs = profile.snapshot();
            assert_eq!(costs.owned_bridge_exits != 0, bridge, "{source}: {costs:?}");
            if !bridge {
                assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn created_closures_keep_captured_values_after_scope_and_frame_exit() {
        for source in [
            "(function(){let x=42;return function(){return x}})",
            "(function(x){return function(){return x}})",
            "(function(){var f; {let x=42; f=function(){return x}} return f})",
            "(function(){var f,g,i=41;while(i<43){let x=i;if(i===41)f=function(){return x};else g=function(){return x};i=i+1}return function(){return f()+g()-41}})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let call_entry = entry(&runtime, &mut context, source, vec![Value::Int(42)]);
            let code = call_entry.executable.code.clone();
            let profile = CostProfile::start();
            let completion =
                execute(runtime.clone(), call_entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(Value::Object(closure)) = completion else {
                panic!("expected closure")
            };
            let costs = profile.snapshot();
            assert_eq!(
                costs.owned_bridge_exits, 0,
                "{source}: {costs:?}, code: {code:?}"
            );
            assert_eq!(
                costs.legacy_dispatches, 0,
                "{source}: {costs:?}, code: {code:?}"
            );
            drop(profile);
            let entry = entry(
                &runtime,
                &mut context,
                "(function(f){return f()})",
                vec![Value::Object(closure)],
            );
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = completion else {
                panic!("expected closure value")
            };
            assert_eq!(value, Value::Int(42));
            assert_eq!(profile.snapshot().legacy_dispatches, 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn captured_parent_slots_share_writes_initialization_and_abrupt_reuse() {
        for source in [
            "(function(){let x=40;var f=function(){x=x+1};f();x=x+1;return x})",
            "(function(x){var f=function(){x=x+1};f();x=x+1;return x})",
            "(function(){function f(){return x}let x=42;return f()})",
            "(function(){function f(){return x}try{return x}catch(e){return 42}let x})",
            "(function(){var i=0,f;while(i<2){try{let x=40+i;if(i===0)f=function(){return x};i=i+1;throw 0}catch(e){}}return f()+1})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(&runtime, &mut context, source, vec![Value::Int(40)]);
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = completion else {
                panic!("expected captured return: {source}")
            };
            assert_eq!(value, Value::Int(42), "{source}");
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn arguments_and_rest_preserve_actual_arity_and_parameter_aliasing() {
        for (source, arguments, expected) in [
            (
                "(function(a){'use strict';a=2;return arguments})",
                vec![Value::Int(1)],
                vec![Value::Int(1)],
            ),
            (
                "(function(a){a=2;return arguments})",
                vec![Value::Int(1), Value::Int(40)],
                vec![Value::Int(2), Value::Int(40)],
            ),
            (
                "(function(a,b){return arguments})",
                vec![Value::Int(42)],
                vec![Value::Int(42)],
            ),
            (
                "(function(a,...rest){return rest})",
                vec![Value::Int(1), Value::Int(40), Value::Int(2)],
                vec![Value::Int(40), Value::Int(2)],
            ),
            (
                "(function(a,b,...rest){return rest})",
                vec![Value::Int(1)],
                vec![],
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(&runtime, &mut context, source, arguments);
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(Value::Object(object)) = completion else {
                panic!("expected arguments or rest object: {source}")
            };
            let costs = profile.snapshot();
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            drop(profile);
            assert_eq!(
                context
                    .get_property(&object, &runtime.intern_property_key("length").unwrap())
                    .unwrap(),
                Value::Int(expected.len() as i32)
            );
            for (index, value) in expected.into_iter().enumerate() {
                assert_eq!(
                    context
                        .get_property(
                            &object,
                            &runtime.intern_property_key(&index.to_string()).unwrap()
                        )
                        .unwrap(),
                    value,
                    "{source}: {index}"
                );
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn default_parameters_keep_order_tdz_and_supplied_values() {
        for (source, arguments) in [
            (
                "(function(a=missing){return a===null?42:0})",
                vec![Value::Null],
            ),
            (
                "(function(a=missing){return a===false?42:0})",
                vec![Value::Bool(false)],
            ),
            ("(function(a=40,b=a+2){return b})", vec![]),
            ("(function(a=40,b=a+2){return b})", vec![Value::Undefined]),
            (
                "(function(a=40,b=a+2){return b})",
                vec![Value::Int(1), Value::Int(42)],
            ),
            (
                "(function(a=40,b=function(){return a}){a=2;return a+b()})",
                vec![],
            ),
            ("(function(f){try{return f()}catch(e){return e}})", vec![]),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let arguments = if source.contains("try{") {
                vec![context.eval("(function(a=b,b=42){return a})").unwrap()]
            } else {
                arguments
            };
            let entry = entry(&runtime, &mut context, source, arguments);
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = completion else {
                panic!("expected parameter return: {source}")
            };
            let costs = profile.snapshot();
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            drop(profile);
            if source.contains("try{") {
                let Value::Object(error) = value else {
                    panic!("expected TDZ error")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("name").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(
                        "ReferenceError"
                    ))
                );
            } else {
                assert_eq!(value, Value::Int(42));
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn readonly_binding_errors_preserve_tdz_and_rhs_order() {
        for (source, name, message) in [
            (
                "(function(){const x=1;x=42})",
                "TypeError",
                "'x' is read-only",
            ),
            (
                "(function f(){'use strict';f=42})",
                "TypeError",
                "'f' is read-only",
            ),
            (
                "(function(){const x=1;return function(){x=42}})()",
                "TypeError",
                "'x' is read-only",
            ),
            (
                "(function(){x=42;const x=1})",
                "TypeError",
                "'x' is read-only",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(&runtime, &mut context, source, vec![]);
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Throw(Value::Object(error)) = completion else {
                panic!("expected binding error: {source}")
            };
            let costs = profile.snapshot();
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            drop(profile);
            for (key, expected) in [("name", name), ("message", message)] {
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key(key).unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(expected))
                );
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(){const x=0;function rhs(){throw 42}try{x=rhs()}catch(e){return e}})",
            vec![],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        let Completion::Return(value) = result else {
            panic!("expected RHS throw to win")
        };
        assert_eq!(value, Value::Int(42));
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
    }

    #[test]
    fn private_field_initialization_uses_fresh_identity_in_published_frames() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let mut names = Vec::new();
        for _ in 0..2 {
            let entry = entry(
                &runtime,
                &mut context,
                "(function(){return class {#x=42; read(){return this.#x}}})",
                vec![],
            );
            let pc = entry
                .executable
                .code
                .iter()
                .position(|op| {
                    matches!(
                        op,
                        crate::engine::code::bytecode::Instruction::InitializePrivateName(_)
                    )
                })
                .unwrap();
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            execution.frames.current_mut(id).unwrap().resume_pc = pc;
            let RunExit::PrivateInitialize { index, kind } = run(&mut execution, id).unwrap()
            else {
                panic!("expected private initialization boundary")
            };
            let frame = execution.frames.current_mut(id).unwrap();
            runtime
                .update_active_bytecode_pc(frame.active_frame, BytecodePc::new(frame.fault_pc))
                .unwrap();
            assert!(
                super::super::private_bindings::step(&runtime, &mut execution, id, index, kind)
                    .unwrap()
                    .is_none()
            );
            let frame = execution.frames.current_mut(id).unwrap();
            let super::super::bindings::FrameBinding::Private(name) =
                execution.slots.local(&frame.window, index).unwrap()
            else {
                panic!("expected private identity owner")
            };
            names.push(name.clone());
            assert_eq!(frame.resume_pc, pc + 1);
            drop(execution);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
        assert_ne!(names[0], names[1]);
    }

    #[test]
    fn private_field_reads_writes_and_membership_use_owned_frames() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let instance = context.eval("new (class {#x=40;#f=function(){return this};bump(){this.#x=this.#x+2;return this.#x}has(o){return #x in o}read(o){return o.#x}call(){return this.#f()}})").unwrap();
        let foreign = context.eval("({})").unwrap();
        for (source, argument, expected) in [
            (
                "(function(o,x){return o.bump()})",
                Value::Undefined,
                Value::Int(42),
            ),
            (
                "(function(o,x){return o.has(x)})",
                instance.clone(),
                Value::Bool(true),
            ),
            (
                "(function(o,x){return o.has(x)})",
                foreign.clone(),
                Value::Bool(false),
            ),
            (
                "(function(o,x){return o.call()})",
                Value::Undefined,
                instance.clone(),
            ),
        ] {
            let entry = entry(
                &runtime,
                &mut context,
                source,
                vec![instance.clone(), argument],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = result else {
                panic!("expected private result")
            };
            assert_eq!(value, expected);
            let costs = profile.snapshot();
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
        }
        let entry = entry(
            &runtime,
            &mut context,
            "(function(o,x){try{return o.read(x)}catch(e){return e}})",
            vec![instance, foreign],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        let Completion::Return(Value::Object(error)) = result else {
            panic!("expected private brand error")
        };
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        drop(profile);
        assert_eq!(
            context
                .get_property(&error, &runtime.intern_property_key("name").unwrap())
                .unwrap(),
            Value::String(crate::engine::value::JsString::from_static("TypeError"))
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn private_methods_preserve_identity_receiver_brand_and_readonly() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let instance = context.eval("new (class {#m(){return this}call(){return this.#m()}get(){return this.#m}has(x){return #m in x}read(x){return x.#m}write(){this.#m=1}})").unwrap();
        let foreign = context.eval("({})").unwrap();
        for (source, argument, expected) in [
            (
                "(function(o,x){return o.call()})",
                Value::Undefined,
                instance.clone(),
            ),
            (
                "(function(o,x){return o.get()===o.get()})",
                Value::Undefined,
                Value::Bool(true),
            ),
            (
                "(function(o,x){return o.has(x)})",
                instance.clone(),
                Value::Bool(true),
            ),
            (
                "(function(o,x){return o.has(x)})",
                foreign.clone(),
                Value::Bool(false),
            ),
        ] {
            let entry = entry(
                &runtime,
                &mut context,
                source,
                vec![instance.clone(), argument],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = result else {
                panic!("expected private method result")
            };
            assert_eq!(value, expected);
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
        }
        for source in [
            "(function(o,x){try{return o.read(x)}catch(e){return e}})",
            "(function(o,x){try{return o.write()}catch(e){return e}})",
        ] {
            let entry = entry(
                &runtime,
                &mut context,
                source,
                vec![instance.clone(), foreign.clone()],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(Value::Object(error)) = result else {
                panic!("expected private error")
            };
            assert_eq!(profile.snapshot().legacy_dispatches, 0);
            drop(profile);
            assert_eq!(
                context
                    .get_property(&error, &runtime.intern_property_key("name").unwrap())
                    .unwrap(),
                Value::String(crate::engine::value::JsString::from_static("TypeError"))
            );
        }
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn private_accessors_use_child_frames_and_discard_setter_returns() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let instance = context.eval("new (class {#x=40;get #value(){return this.#x}set #value(v){this.#x=v;return this}run(){try{this.#value=this.#value+2}catch(e){throw e}return this.#value}})").unwrap();
        let call_entry = entry(
            &runtime,
            &mut context,
            "(function(o){return o.run()})",
            vec![instance],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), call_entry, ExecutionLimits::default()).unwrap();
        let Completion::Return(value) = result else {
            panic!("expected accessor result")
        };
        assert_eq!(value, Value::Int(42));
        let costs = profile.snapshot();
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert_eq!(costs.owned_storage.maximum_frame_depth, 3);
        assert_eq!(costs.owned_storage.frames_pushed, 5);
        drop(profile);
        for source in [
            "new (class {get #value(){throw 42}run(){return this.#value}})",
            "new (class {set #value(v){throw v}run(){this.#value=42;return 0}})",
        ] {
            let instance = context.eval(source).unwrap();
            let call_entry = entry(
                &runtime,
                &mut context,
                "(function(o){try{return o.run()}catch(e){return e}})",
                vec![instance],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), call_entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = result else {
                panic!("expected accessor throw to reach caller")
            };
            assert_eq!(value, Value::Int(42));
            assert_eq!(profile.snapshot().legacy_dispatches, 0);
            assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        }
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn private_getter_returned_function_keeps_receiver_and_runs_once() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let instance = context.eval("new (class {#count=0;get #fn(){this.#count=this.#count+1;return function(){return this}}run(){return this.#fn()}count(){return this.#count}})").unwrap();
        let call_entry = entry(
            &runtime,
            &mut context,
            "(function(o){var result=o.run();return result===o?o.count():0})",
            vec![instance],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), call_entry, ExecutionLimits::default()).unwrap();
        let Completion::Return(value) = result else {
            panic!("expected getter method result")
        };
        assert_eq!(value, Value::Int(1));
        let costs = profile.snapshot();
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert_eq!(costs.owned_storage.frames_pushed, 5);
        assert_eq!(costs.owned_storage.maximum_frame_depth, 3);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn instance_initializers_run_as_owned_children_and_propagate_throws() {
        for (source, wrapper) in [
            (
                "(class {#x=42;#m(){return this.#x}read(){return this.#m()}})",
                "(function(C){return new C().read()})",
            ),
            (
                "(class extends (function B(){}) {#x=42;read(){return this.#x}})",
                "(function(C){return new C().read()})",
            ),
            (
                "(class {#x=(function(){throw 42})()})",
                "(function(C){try{return new C()}catch(e){return e}})",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let constructor = context.eval(source).unwrap();
            let entry = entry(&runtime, &mut context, wrapper, vec![constructor]);
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = completion else {
                panic!("expected instance initializer result: {source}")
            };
            assert_eq!(value, Value::Int(42), "{source}");
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(costs.owned_storage.maximum_frame_depth >= 3);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn base_class_creation_and_initialization_throw_through_owned_frames() {
        for source in [
            "(function(){try{class C {static{throw 42}}}catch(e){return e}})",
            "(function(){try{class C {#x=(function(){throw 42})()}return new C()}catch(e){return e}})",
            "(function(){class C extends null {}return 42})",
            "(function(){try{class C extends 1 {}}catch(e){return e}})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(&runtime, &mut context, source, vec![]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected class result: {source}")
            };
            let costs = profile.snapshot();
            if let Value::Object(error) = value {
                assert!(source.contains("extends 1"), "{source}");
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("name").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static("TypeError"))
                );
            } else {
                assert!(!source.contains("extends 1"), "{source}");
                assert_eq!(value, Value::Int(42), "{source}");
            }
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn class_methods_are_published_and_called_without_handoff() {
        for source in [
            "(function(){class C {#x=42;read(){return this.#x}}return new C().read()})",
            "(function(){class C {get x(){return 42}}return new C().x})",
            "(function(){class B {read(){return 42}}class C extends B {}return new C().read()})",
            "(function(){class C {#x=42;get x(){return this.#x}set x(v){this.#x=v}}return new C().x})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(&runtime, &mut context, source, vec![]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected method result: {source}")
            };
            assert_eq!(value, Value::Int(42));
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn public_fields_define_own_data_and_skip_inherited_setters() {
        for source in [
            "(function(){class C {x=42}return new C().x})",
            "(function(){class B {set x(v){throw 99}}class C extends B {x=42}return new C().x})",
            "(function(){class C {x=40;y=this.x+2}return new C().y})",
            "(function(){class C {f=function(){return this.x};x=42}return new C().f()})",
            "(function(){try{class C {x=(function(){throw 42})()}return new C()}catch(e){return e}})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(&runtime, &mut context, source, vec![]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected field result: {source}")
            };
            assert_eq!(value, Value::Int(42), "{source}");
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn computed_class_keys_convert_once_before_definition() {
        for source in [
            "(function(k,count){class C {[k]=41}return new C().x+count()})",
            "(function(k,count){class C {[k](){return 41}}return new C().x()+count()})",
            "(function(k,count){class C {get [k](){return 41}set x(v){throw 99}}return new C().x+count()})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let Value::Object(pair) = context.eval("(function(){var n=0;return {key:{toString:function(){n=n+1;return 'x'},valueOf:function(){throw 99}},count:function(){return n}}})()").unwrap()
            else {panic!("expected key setup")};
            let key = context
                .get_property(&pair, &runtime.intern_property_key("key").unwrap())
                .unwrap();
            let count = context
                .get_property(&pair, &runtime.intern_property_key("count").unwrap())
                .unwrap();
            let entry = entry(&runtime, &mut context, source, vec![key, count]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected computed definition: {source}")
            };
            assert_eq!(value, Value::Int(42), "{source}");
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
        for (key_source, source) in [
            (
                "Symbol('x')",
                "(function(k){class C {[k]=42}return new C()})",
            ),
            ("1.5", "(function(k){class C {[k]=42}return new C()})"),
            ("-1", "(function(k){class C {[k]=42}return new C()})"),
            (
                "({toString:function(){throw 42}})",
                "(function(k){try{class C {[k]=(function(){throw 99})()}return new C()}catch(e){return e}})",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let key = context.eval(key_source).unwrap();
            let expected_key = if matches!(key, Value::Object(_)) {
                None
            } else {
                let NativeConversion::Value(key) = runtime
                    .native_to_property_key(context.realm, key.clone())
                    .unwrap()
                else {
                    panic!("expected primitive key")
                };
                Some(key)
            };
            let entry = entry(&runtime, &mut context, source, vec![key]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected computed result")
            };
            let costs = profile.snapshot();
            if let Some(key) = expected_key {
                let Value::Object(instance) = value else {
                    panic!("expected computed instance")
                };
                assert_eq!(
                    context.get_property(&instance, &key).unwrap(),
                    Value::Int(42)
                );
            } else {
                assert_eq!(value, Value::Int(42));
            }
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn computed_function_field_names_keep_key_identity_and_existing_names() {
        for (key_source, expected, initializer) in [
            ("'x'", "x", "function(){}"),
            ("-1", "-1", "function(){}"),
            ("Symbol('x')", "[x]", "function(){}"),
            ("Symbol()", "", "function(){}"),
            ("Symbol('')", "[]", "function(){}"),
            ("'x'", "keep", "function keep(){}"),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let key = context.eval(key_source).unwrap();
            let NativeConversion::Value(property) = runtime
                .native_to_property_key(context.realm, key.clone())
                .unwrap()
            else {
                panic!("expected canonical key")
            };
            let source = format!("(function(k){{class C {{[k]={initializer}}}return new C()}})");
            let entry = entry(&runtime, &mut context, &source, vec![key]);
            let profile = CostProfile::start();
            let Completion::Return(Value::Object(instance)) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected instance")
            };
            let costs = profile.snapshot();
            let Value::Object(function) = context.get_property(&instance, &property).unwrap()
            else {
                panic!("expected function field")
            };
            assert_eq!(
                context
                    .get_property(&function, &runtime.intern_property_key("name").unwrap())
                    .unwrap(),
                Value::String(crate::engine::value::JsString::from_static(expected))
            );
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn with_entry_boxes_primitives_and_unwinds_nullish_errors() {
        for value in [
            Value::Int(1),
            Value::Bool(true),
            Value::String(crate::engine::value::JsString::from_static("x")),
            Value::Null,
            Value::Undefined,
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let nullish = matches!(value, Value::Null | Value::Undefined);
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){try{with(o){return 42}}catch(e){return e}})",
                vec![value],
            );
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected with completion")
            };
            let costs = profile.snapshot();
            if nullish {
                let Value::Object(error) = value else {
                    panic!("expected nullish error")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(
                        "cannot convert to object"
                    ))
                );
            } else {
                assert_eq!(value, Value::Int(42))
            }
            assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn variable_environment_creation_uses_authenticated_null_prototype() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(){eval('');return 42})",
            vec![],
        );
        let pc = entry
            .executable
            .code
            .iter()
            .position(|op| {
                matches!(
                    op,
                    crate::engine::code::bytecode::Instruction::VariableEnvironment
                )
            })
            .unwrap();
        let mut execution = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let id = push_frame(&mut execution, entry).unwrap();
        execution.frames.current_mut(id).unwrap().resume_pc = pc;
        let op = super::super::environment_driver::Operation::CreateVariable;
        assert_eq!(run(&mut execution, id).unwrap(), RunExit::Environment(op));
        let before = execution.frames.current_mut(id).unwrap().fault_pc;
        assert!(matches!(
            super::super::environment_driver::step(&runtime, &mut execution, id, op).unwrap(),
            CallStep::Entered
        ));
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(frame.resume_pc, before + 1);
        let Value::Object(environment) = execution.slots.peek(&frame.window, 0).unwrap() else {
            panic!("expected variable environment")
        };
        assert!(runtime.get_prototype_of(environment).unwrap().is_none());
        drop(execution);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn dynamic_data_reads_handle_unscopables_and_captured_with_receivers() {
        for (setup, source) in [
            ("({x:42})", "(function(o){with(o){return x}})"),
            (
                "({x:99,[Symbol.unscopables]:{x:true}})",
                "(function(o){let x=42;with(o){return x}})",
            ),
            (
                "({x:42,[Symbol.unscopables]:{x:false}})",
                "(function(o){with(o){return (function(){return x})()}})",
            ),
            (
                "({x:42,m:function(){return this.x}})",
                "(function(o){with(o){return m()}})",
            ),
            ("({})", "(function(o){let x=42;with(o){return x}})"),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(&runtime, &mut context, source, vec![object]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected dynamic read")
            };
            assert_eq!(value, Value::Int(42), "{source}");
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn dynamic_getters_run_once_and_keep_the_with_receiver() {
        for (setup, source) in [
            (
                "(function(){var n=0;return {get x(){n=n+1;return 41},count:function(){return n}}})()",
                "(function(o){with(o){return x+count()}})",
            ),
            (
                "({get x(){throw 42}})",
                "(function(o){try{with(o){return x}}catch(e){return e}})",
            ),
            (
                "(function(){var n=0;return {x:41,get m(){n=n+1;return function(){return this.x+n}}}})()",
                "(function(o){with(o){return m()}})",
            ),
            (
                "({get x(){return 42}})",
                "(function(o){with(o){return (function(){return x})()}})",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(&runtime, &mut context, source, vec![object]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected getter result")
            };
            assert_eq!(value, Value::Int(42));
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(costs.owned_storage.maximum_frame_depth >= 2);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn unscopables_getters_resume_in_order_and_propagate_abrupt_completion() {
        for (setup, source, expected) in [
            (
                "(function(){var n=0;var excluded={get x(){n=n*10+2;return false}};return [{get x(){n=n*10+3;return 41},get [Symbol.unscopables](){n=n*10+1;return excluded}},function(){return n}]})()",
                "(function(o,count){var v=(function(){with(o){return x}})();return v+count()})",
                164,
            ),
            (
                "(function(){var n=0;var excluded={get x(){n=n*10+2;return true}};return [{get x(){throw 99},get [Symbol.unscopables](){n=n*10+1;return excluded}},function(){return n}]})()",
                "(function(o,count){let x=41;var v=(function(){with(o){return x}})();return v+count()})",
                53,
            ),
            (
                "[{x:99,get [Symbol.unscopables](){throw 42}},function(){return 0}]",
                "(function(o){try{with(o){return x}}catch(e){return e}})",
                42,
            ),
            (
                "[{x:99,get [Symbol.unscopables](){return {get x(){throw 42}}}},function(){return 0}]",
                "(function(o){try{with(o){return x}}catch(e){return e}})",
                42,
            ),
            (
                "[{get [Symbol.unscopables](){throw 99}},function(){return 0}]",
                "(function(o){let x=42;with(o){return x}})",
                42,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let Value::Object(pair) = context.eval(setup).unwrap() else {
                panic!("expected setup pair")
            };
            let object = context
                .get_property(&pair, &runtime.intern_property_key("0").unwrap())
                .unwrap();
            let count = context
                .get_property(&pair, &runtime.intern_property_key("1").unwrap())
                .unwrap();
            let entry = entry(&runtime, &mut context, source, vec![object, count]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected HasBinding result")
            };
            assert_eq!(value, Value::Int(expected), "{setup}");
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{setup}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{setup}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn selected_native_calls_return_or_throw_into_the_owned_caller() {
        for (callee, source) in [
            ("parseInt", "(function(f){let x=2;return f('40',10)+x})"),
            (
                "Reflect.get",
                "(function(f){let x=40;try{f(0,'x')}catch(e){return x+2}})",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callee = context.eval(callee).unwrap();
            let entry = entry(&runtime, &mut context, source, vec![callee]);
            let profile = CostProfile::start();
            assert!(matches!(
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                Completion::Return(Value::Int(42))
            ));
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_json_and_function_domains_keep_callback_order_without_bridges() {
        for source in [
            "(function(){let log='';let result=JSON.parse('{\"a\":1,\"b\":[2]}',function(k,v,c){log+=k;if(k==='a')return c.source==='1'?3:0;return v});return result.a===3&&result.b[0]===2&&log==='a0b'?42:0})",
            "(function(){let log='';let value={get a(){log+='a';return {toJSON(k){log+='j'+k;return 2}}}};let text=JSON.stringify(value,function(k,v){log+='r'+k;return v},1);return text==='{'+'\\n '+ '\"a\": 2'+'\\n}'&&log==='rajara'?42:0})",
            "(function(){let log='';let value={toString(){log+='s';return '12'}};return JSON.stringify({x:JSON.rawJSON(value)})==='{'+'\"x\":12}'&&log==='s'?42:0})",
            "(function(){let log='';let f=function(a,b){return this.x+a+b};Object.defineProperty(f,'length',{get(){log+='l';return 2}});Object.defineProperty(f,'name',{get(){log+='n';return 'f'}});let g=f.bind({x:30},5);return g(7)===42&&g.length===1&&g.name==='bound f'&&log==='ln'?42:0})",
            "(function(){let log='';let f=Function({toString(){log+='p';return 'x'}},{toString(){log+='b';return 'return x+2'}});return f(40)===42&&log==='pb'?42:0})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(&runtime, &mut context, source, vec![]);
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{source}: {result:?}"
            );
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_sync_call_bridges, 0, "{source}: {costs:?}");
        }
    }

    #[test]
    fn selected_native_callback_reentry_preserves_captured_bindings() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let array = context
            .eval("(function(){let xs=[10,20,12];xs.f=Array.prototype.map;return xs})()")
            .unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(xs){let sum=0;let mapped=xs.f(function(value){sum=sum+value;return value});return sum})",
            vec![array],
        );
        let profile = CostProfile::start();
        assert!(matches!(
            execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
            Completion::Return(Value::Int(42))
        ));
        let costs = profile.snapshot();
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        // Mapping, species lookup and result construction all resume in this execution.
        assert_eq!(costs.owned_sync_call_bridges, 0);
    }

    #[test]
    fn named_array_prototype_reads_reach_owned_methods() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.eval("Object.defineProperty(Array.prototype,'owned',{value:function(){return 42},configurable:true})").unwrap();
        let array = context.eval("[]").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(xs){return xs.owned()})",
            vec![array],
        );
        let profile = CostProfile::start();
        assert!(matches!(
            execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
            Completion::Return(Value::Int(42))
        ));
        let costs = profile.snapshot();
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn program_global_initialization_and_assignments_use_owned_cells() {
        for source in [
            "let x=40;x=x+2;x",
            "const x=42;x",
            "var x=40;x=x+2;x",
            "function f(){return 42}f()",
            "x=40;x=x+2;x",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let profile = CostProfile::start();
            assert_eq!(context.eval(source).unwrap(), Value::Int(42), "{source}");
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn global_writes_preserve_tdz_const_and_rhs_order() {
        for (setup, body, message) in [
            ("const x=1", "x=42;return x", Some("'x' is read-only")),
            (
                "throw 0;let x",
                "x=42;return x",
                Some("x is not initialized"),
            ),
            ("throw 0;let x", "x=(function(){throw 42})()", None),
            ("const x=1", "x=(function(){throw 42})()", None),
            (
                "delete globalThis.x",
                "'use strict';x=42",
                Some("'x' is not defined"),
            ),
            (
                "Object.defineProperty(globalThis,'x',{value:1,writable:false,configurable:true})",
                "'use strict';x=42",
                Some("'x' is read-only"),
            ),
            (
                "Object.defineProperty(globalThis,'x',{value:42,writable:false,configurable:true})",
                "x=99;return x",
                None,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            if context.eval(setup).is_err() {
                context.take_exception().unwrap();
            }
            let source = format!("(function(){{{body}}})");
            let entry = entry(&runtime, &mut context, &source, vec![]);
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let costs = profile.snapshot();
            if let Some(message) = message {
                let Completion::Throw(Value::Object(error)) = completion else {
                    panic!("expected global binding error: {source}: {completion:?}")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::try_from_utf8(message).unwrap())
                );
            } else if body.contains("throw 42") {
                assert!(
                    matches!(completion, Completion::Throw(Value::Int(42))),
                    "{source}: {completion:?}"
                );
            } else {
                assert!(
                    matches!(completion, Completion::Return(Value::Int(42))),
                    "{source}: {completion:?}"
                );
            }
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn global_property_setters_run_once_without_getter_reads() {
        for inherited in [false, true] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let target = if inherited {
                "Object.getPrototypeOf(globalThis)"
            } else {
                "globalThis"
            };
            context.eval(&format!("(function(){{let stored=0;globalThis.readStored=function(){{return stored}};Object.defineProperty({target},'x',{{get(){{throw 99}},set(value){{if(this!==globalThis)throw 99;stored=stored+value}},configurable:true}})}})()" )).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(){'use strict';x=40;x=2;return 42})",
                vec![],
            );
            let profile = CostProfile::start();
            assert!(matches!(
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                Completion::Return(Value::Int(42))
            ));
            let costs = profile.snapshot();
            drop(profile);
            assert_eq!(context.eval("readStored()").unwrap(), Value::Int(42));
            assert_eq!(
                costs.legacy_dispatches, 0,
                "inherited={inherited}: {costs:?}"
            );
            assert_eq!(
                costs.owned_bridge_exits, 0,
                "inherited={inherited}: {costs:?}"
            );
            assert!(costs.owned_storage.maximum_frame_depth >= 2);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn delete_global_binding_probes_presence_without_reading_accessors() {
        for (setup, expected) in [
            ("let x=1", false),
            ("throw 0;let x", false),
            ("var x=1", false),
            ("globalThis.x=1", true),
            ("delete globalThis.x", true),
            (
                "Object.defineProperty(globalThis,'x',{get(){throw 99},configurable:true})",
                true,
            ),
            (
                "Object.setPrototypeOf(globalThis,{get x(){throw 99}})",
                true,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            if context.eval(setup).is_err() {
                context.take_exception().unwrap();
            }
            let entry = entry(
                &runtime,
                &mut context,
                "(function(){return delete x})",
                vec![],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(execute(runtime.clone(),entry,ExecutionLimits::default()).unwrap(),Completion::Return(Value::Bool(value)) if value==expected),
                "{setup}"
            );
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{setup}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{setup}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn spread_calls_preserve_receiver_bound_arguments_and_explicit_frames() {
        for (callee, source) in [
            (
                "(function(a,b){return a+b})",
                "(function(f){return f(...[40,2])})",
            ),
            (
                "({base:40,f:function(x){return this.base+x}})",
                "(function(o){return o.f(...[2])})",
            ),
            (
                "(function(a,b){return this.base+a+b}).bind({base:1},40)",
                "(function(f){return f(...[1])})",
            ),
            ("(function(){return 42})", "(function(f){return f(...[])})"),
            (
                "(function(){throw 42})",
                "(function(f){try{return f(...[1])}catch(e){return e}})",
            ),
            (
                "(function loop(n){if(n===0)return 42;return loop(...[n-1])})",
                "(function(f){return f(...[256])})",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callee = context.eval(callee).unwrap();
            let entry = entry(&runtime, &mut context, source, vec![callee]);
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(completion, Completion::Return(Value::Int(42))),
                "{source}: {completion:?}"
            );
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn spread_constructors_and_super_use_existing_constructor_continuations() {
        for (callee, arguments) in [
            (
                "(function Base(a,b){if(new.target===Base)return {value:a+b};throw 99})",
                "[40,2]",
            ),
            ("(function(a,b){return {value:a+b}}).bind(null,40)", "[2]"),
            (
                "(class Derived extends (class Base{constructor(a,b){return {value:a+b}}}) {constructor(...args){super(...args)}})",
                "[40,2]",
            ),
            (
                "(class Derived extends (class Base{constructor(a,b){return {value:a+b}}}) {})",
                "[40,2]",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callee = context.eval(callee).unwrap();
            let args = context.eval(arguments).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(C,args){return (new C(...args)).value})",
                vec![callee, args],
            );
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(completion, Completion::Return(Value::Int(42))),
                "{arguments}: {completion:?}"
            );
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn apply_construct_snapshot_survives_prototype_getter_and_carrier_mutation() {
        for new_target_source in [
            "({get prototype(){return {}}})",
            "new Proxy({}, {get(t,k){return {}}})",
            "Object.defineProperty({},'prototype',{get:Object.getPrototypeOf.bind(undefined,{})})",
        ] {
            use crate::engine::code::bytecode::{ApplyKind, Instruction};
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let constructor = context.eval("(function(value){return value})").unwrap();
            let new_target = context.eval(new_target_source).unwrap();
            let Value::Object(carrier) = context.eval("[{}]").unwrap() else {
                panic!("expected argument array")
            };
            let argument_id = {
                let values = runtime
                    .fast_array_like_values(&carrier, 1)
                    .unwrap()
                    .unwrap();
                let Value::Object(object) = &values[0] else {
                    panic!("expected object argument")
                };
                object.object_id()
            };
            let entry = entry(
                &runtime,
                &mut context,
                "(function(C){return new C(...[])})",
                vec![],
            );
            let pc = entry
                .executable
                .code
                .iter()
                .position(|op| matches!(op, Instruction::Apply(ApplyKind::Construct)))
                .unwrap();
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let frame = execution.frames.current_mut(id).unwrap();
            frame.resume_pc = pc;
            for value in [constructor, new_target, Value::Object(carrier.clone())] {
                execution.slots.push(&mut frame.window, value).unwrap();
            }
            let profile = CostProfile::start();
            assert!(matches!(
                run(&mut execution, id).unwrap(),
                RunExit::Apply(ApplyKind::Construct)
            ));
            assert!(matches!(
                super::super::apply_driver::step(
                    &runtime,
                    &mut execution,
                    id,
                    ApplyKind::Construct,
                    1
                )
                .unwrap(),
                CallStep::Entered
            ));
            assert_ne!(execution.frames.current_id(), Some(id));
            assert!(
                runtime
                    .delete_property(&carrier, &runtime.intern_property_key("0").unwrap())
                    .unwrap()
            );
            assert!(
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .object_strong_count(argument_id)
                    .unwrap()
                    > 0
            );
            let Completion::Return(Value::Object(result)) =
                execute_running(runtime.clone(), execution).unwrap()
            else {
                panic!("expected original argument")
            };
            let costs = profile.snapshot();
            assert_eq!(result.object_id(), argument_id);
            drop(result);
            assert_eq!(
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .object_strong_count(argument_id)
                    .unwrap_or(0),
                0
            );
            assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn apply_checkpoint_preserves_callability_list_and_constructor_error_order() {
        use crate::engine::code::bytecode::{ApplyKind, Instruction};
        for (function, array, construct, error_name, message) in [
            (
                "0",
                "({get length(){throw 99}})",
                false,
                Some("TypeError"),
                Some("not a function"),
            ),
            (
                "0",
                "new Array(65535)",
                true,
                Some("TypeError"),
                Some("not a function"),
            ),
            (
                "(()=>42)",
                "new Array(65535)",
                true,
                Some("RangeError"),
                None,
            ),
            ("(()=>42)", "[]", true, Some("TypeError"), None),
            (
                "(()=>42)",
                "1",
                false,
                Some("TypeError"),
                Some("not a object"),
            ),
            ("(()=>42)", "null", true, None, None),
            ("(()=>42)", "undefined", false, None, None),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let function = context.eval(function).unwrap();
            let array = context.eval(array).unwrap();
            let source = if construct {
                "(function(C){return new C(...[])})"
            } else {
                "(function(f){return f(...[])})"
            };
            let entry = entry(&runtime, &mut context, source, vec![]);
            let kind = if construct {
                ApplyKind::Construct
            } else {
                ApplyKind::Call
            };
            let pc = entry
                .executable
                .code
                .iter()
                .position(|op| matches!(op, Instruction::Apply(found) if *found==kind))
                .unwrap();
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let frame = execution.frames.current_mut(id).unwrap();
            frame.resume_pc = pc;
            for value in [function, Value::Undefined, array] {
                execution.slots.push(&mut frame.window, value).unwrap();
            }
            let profile = CostProfile::start();
            let completion = execute_running(runtime.clone(), execution).unwrap();
            let costs = profile.snapshot();
            if let Some(error_name) = error_name {
                let Completion::Throw(Value::Object(error)) = completion else {
                    panic!("expected Apply error: {completion:?}")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("name").unwrap())
                        .unwrap(),
                    Value::String(
                        crate::engine::value::JsString::try_from_utf8(error_name).unwrap()
                    )
                );
                if let Some(message) = message {
                    assert_eq!(
                        context
                            .get_property(&error, &runtime.intern_property_key("message").unwrap())
                            .unwrap(),
                        Value::String(
                            crate::engine::value::JsString::try_from_utf8(message).unwrap()
                        )
                    );
                }
            } else {
                assert!(
                    matches!(completion, Completion::Return(Value::Int(42))),
                    "{completion:?}"
                );
            }
            assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn for_of_owns_records_and_per_iteration_bindings() {
        for source in [
            "(function(xs){let sum=0;for(let x of xs){sum=sum+x}return sum})",
            "(function(xs){let sum=0;for(const x of xs){if(x===40){sum=x;continue}sum=sum+x}return sum})",
            "(function(xs){let f;for(let x of xs){if(x===40)f=function(){return x};else return f()+x}})",
            "(function(xs){let [a,b]=xs;return a+b})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let xs = context.eval("[40,2]").unwrap();
            let entry = entry(&runtime, &mut context, source, vec![xs]);
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{source}: {result:?}"
            );
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn for_of_callback_order_uses_one_iterator_get_and_cached_next() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let xs = context.eval(r#"(function(){let trace='',n=0;globalThis.readTrace=function(){return trace};return {
            get [Symbol.iterator](){trace=trace+'i';return function(){trace=trace+'c';return {
                get next(){trace=trace+'n';return function(){trace=trace+'s';n=n+1;return {
                    get done(){trace=trace+'d';return n>2},
                    get value(){trace=trace+'v';if(n>2)throw 99;return n===1?40:2}
                }}}, get return(){throw 99}
            }}}
        }})()"#).unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(xs){let sum=0;for(let x of xs){sum=sum+x}return sum})",
            vec![xs],
        );
        let profile = CostProfile::start();
        assert!(matches!(
            execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
            Completion::Return(Value::Int(42))
        ));
        let costs = profile.snapshot();
        drop(profile);
        assert_eq!(
            context.eval("readTrace()").unwrap(),
            Value::String(crate::engine::value::JsString::from_static("icnsdvsdvsd"))
        );
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn for_of_closes_body_abrupt_completions_but_not_next_failures() {
        for (next, close, body, expected, trace) in [
            (
                "next(){trace=trace+'n';return {done:false,value:40}}",
                "return {}",
                "return x+2",
                42,
                "irc",
            ),
            (
                "next(){trace=trace+'n';return {done:false,value:40}}",
                "return {}",
                "break",
                42,
                "irc",
            ),
            (
                "next(){trace=trace+'n';return {done:false,value:40}}",
                "throw 99",
                "throw 42",
                42,
                "irc",
            ),
            (
                "next(){trace=trace+'n';return {done:false,value:40}}",
                "throw 99",
                "return 42",
                99,
                "irc",
            ),
            (
                "next(){trace=trace+'n';return {done:false,value:40}}",
                "return 0",
                "throw 42",
                42,
                "irc",
            ),
            (
                "next(){trace=trace+'n';throw 42}",
                "throw 99",
                "return 0",
                42,
                "i",
            ),
            (
                "next(){trace=trace+'n';return {get done(){throw 42}}}",
                "throw 99",
                "return 0",
                42,
                "i",
            ),
            (
                "next(){trace=trace+'n';return {done:false,get value(){throw 42}}}",
                "throw 99",
                "return 0",
                42,
                "i",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let setup = r#"(function(){let trace='';globalThis.readTrace=function(){return trace};
                let iter={NEXT,get return(){trace=trace+'r';return function(){trace=trace+'c';CLOSE}}};
                return {[Symbol.iterator](){trace=trace+'i';return iter}};
            })()"#.replace("NEXT", next).replace("CLOSE", close);
            let xs = context.eval(&setup).unwrap();
            let source = format!(
                "(function(xs){{try{{for(let x of xs){{{body}}}return 42}}catch(e){{return e}}}})"
            );
            let entry = entry(&runtime, &mut context, &source, vec![xs]);
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(value)) if value == expected),
                "{source}: {result:?}"
            );
            let costs = profile.snapshot();
            drop(profile);
            let expected_trace = trace.replacen('i', "in", 1);
            assert_eq!(
                context.eval("readTrace()").unwrap(),
                Value::String(
                    crate::engine::value::JsString::try_from_utf8(&expected_trace).unwrap()
                ),
                "{source}"
            );
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn for_of_normal_close_rejects_primitive_results_and_unwinds_nested_regions() {
        for (source, expected, expected_trace, message) in [
            (
                "(function(xs,ys){try{for(let x of xs){break}}catch(e){return e}})",
                0,
                "xo",
                Some("not an object"),
            ),
            (
                "(function(xs,ys){try{for(let x of xs){for(let y of ys){throw 42}}}catch(e){return e}})",
                42,
                "xyio",
                None,
            ),
            (
                "(function(xs,ys){try{outer:for(let x of xs){for(let y of ys){break outer}}}catch(e){return 42}})",
                42,
                "xyio",
                None,
            ),
            (
                "(function(xs,ys){try{for(let x of xs){try{throw 42}finally{throw 43}}}catch(e){return e}})",
                43,
                "xo",
                None,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let Value::Object(pair) = context.eval(r#"(function(){let trace='';globalThis.readTrace=function(){return trace};return [
                {[Symbol.iterator](){trace=trace+'x';return {next(){return {done:false,value:40}},return(){trace=trace+'o';return 0}}}},
                {[Symbol.iterator](){trace=trace+'y';return {next(){return {done:false,value:2}},return(){trace=trace+'i';return {}}}}}
            ]})()"#).unwrap() else { panic!("expected pair") };
            let arguments = runtime.fast_array_like_values(&pair, 2).unwrap().unwrap();
            let entry = entry(&runtime, &mut context, source, arguments);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected caught completion")
            };
            let costs = profile.snapshot();
            drop(profile);
            if let Some(message) = message {
                let Value::Object(error) = value else {
                    panic!("expected close error")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::try_from_utf8(message).unwrap())
                );
            } else {
                assert_eq!(value, Value::Int(expected), "{source}");
            }
            assert_eq!(
                context.eval("readTrace()").unwrap(),
                Value::String(
                    crate::engine::value::JsString::try_from_utf8(expected_trace).unwrap()
                ),
                "{source}"
            );
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn append_spreads_dense_arrays_strings_and_eval_arguments_in_owned_frames() {
        for (source, argument, expected) in [
            ("(function(xs){return [...xs,2]})", "[40,41]", "[40,41,2]"),
            ("(function(xs){return [...xs]})", "'ab'", "['a','b']"),
            ("(function(xs){return [...xs]})", "[]", "[]"),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let argument = context.eval(argument).unwrap();
            let Value::Object(expected) = context.eval(expected).unwrap() else {
                panic!("expected array")
            };
            let len = runtime.array_length_state(&expected).unwrap().0;
            let expected = runtime
                .fast_array_like_values(&expected, len)
                .unwrap()
                .unwrap();
            let entry = entry(&runtime, &mut context, source, vec![argument]);
            let profile = CostProfile::start();
            let Completion::Return(Value::Object(array)) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected spread array")
            };
            let costs = profile.snapshot();
            assert_eq!(runtime.array_length_state(&array).unwrap().0, len);
            assert_eq!(
                runtime
                    .fast_array_like_values(&array, len)
                    .unwrap()
                    .unwrap(),
                expected
            );
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(){return eval(...['40+2'])})",
            vec![],
        );
        let profile = CostProfile::start();
        assert!(matches!(
            execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
            Completion::Return(Value::Int(42))
        ));
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn append_callbacks_preserve_lookup_order_cache_next_and_skip_done_value() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let iterable = context.eval(r#"(function(){let trace='',n=0;globalThis.readTrace=function(){return trace};return {
                get [Symbol.iterator](){trace=trace+'i';return function(){trace=trace+'c';return {
                    get next(){trace=trace+'n';return function(){trace=trace+'s';n=n+1;
                        if(n===1)return {get done(){trace=trace+'d';return false},get value(){trace=trace+'v';return 40}};
                        return {get done(){trace=trace+'d';return true},get value(){throw 99}};
                    }},
                    get return(){throw 99}
                }}}
            }})()"#).unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(xs){return [...xs,2]})",
            vec![iterable],
        );
        let profile = CostProfile::start();
        let Completion::Return(Value::Object(array)) =
            execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
        else {
            panic!("expected spread array")
        };
        let costs = profile.snapshot();
        drop(profile);
        assert_eq!(
            runtime.fast_array_like_values(&array, 2).unwrap().unwrap(),
            vec![Value::Int(40), Value::Int(2)]
        );
        assert_eq!(
            context.eval("readTrace()").unwrap(),
            Value::String(crate::engine::value::JsString::from_static("iicnsdvsd"))
        );
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert!(costs.owned_storage.maximum_frame_depth >= 2);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn append_closes_iteration_errors_and_preserves_original_throw() {
        for (next, close, expected) in [
            (
                "get next(){trace=trace+'n';return function(){trace=trace+'s';throw 42}}",
                "get return(){trace=trace+'r';throw 99}",
                "iicnsr",
            ),
            (
                "get next(){trace=trace+'n';return function(){trace=trace+'s';throw 42}}",
                "get return(){trace=trace+'r';return function(){trace=trace+'c';throw 99}}",
                "iicnsrc",
            ),
            (
                "get next(){trace=trace+'n';return function(){trace=trace+'s';return {get done(){trace=trace+'d';throw 42}}}}",
                "get return(){trace=trace+'r';return function(){trace=trace+'c';return 0}}",
                "iicnsdrc",
            ),
            (
                "get next(){trace=trace+'n';return function(){trace=trace+'s';return {done:false,get value(){trace=trace+'v';throw 42}}}}",
                "get return(){trace=trace+'r';return 0}",
                "iicnsvr",
            ),
            (
                "get next(){trace=trace+'n';throw 42}",
                "get return(){trace=trace+'r';throw 99}",
                "iicn",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let setup = format!(
                "(function(){{let trace='';globalThis.readTrace=function(){{return trace}};let iter={{{next},{close}}};return {{get [Symbol.iterator](){{trace=trace+'i';return function(){{trace=trace+'c';return iter}}}}}}}})()"
            );
            let iterable = context.eval(&setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(xs){try{return [...xs]}catch(e){return e}})",
                vec![iterable],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(
                    execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                    Completion::Return(Value::Int(42))
                ),
                "{setup}"
            );
            let costs = profile.snapshot();
            drop(profile);
            assert_eq!(
                context.eval("readTrace()").unwrap(),
                Value::String(crate::engine::value::JsString::try_from_utf8(expected).unwrap()),
                "{setup}"
            );
            assert_eq!(costs.legacy_dispatches, 0, "{setup}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{setup}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn append_reuses_bounded_frame_storage_across_many_iterator_calls() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let iterable = context
            .eval(
                r#"(function(){let n=0;return {
            [Symbol.iterator](){return {next(){n=n+1;return {done:n>2048,value:n}}}}
        }})()"#,
            )
            .unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(xs){return [...xs]})",
            vec![iterable],
        );
        let profile = CostProfile::start();
        let Completion::Return(Value::Object(array)) = execute(
            runtime.clone(),
            entry,
            ExecutionLimits {
                frames: 2,
                ..ExecutionLimits::default()
            },
        )
        .unwrap() else {
            panic!("expected finite iterable")
        };
        let costs = profile.snapshot();
        assert_eq!(runtime.array_length_state(&array).unwrap().0, 2048);
        assert_eq!(
            context
                .get_property(&array, &runtime.intern_property_key("2047").unwrap())
                .unwrap(),
            Value::Int(2048)
        );
        assert_eq!(costs.owned_storage.maximum_frame_depth, 2);
        assert!(costs.owned_storage.frames_pushed > 2048);
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn append_checkpoint_closes_failed_writes_and_wraps_its_u32_index() {
        for fail in [false, true] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let target = context
                .eval(if fail {
                    "Object.preventExtensions([])"
                } else {
                    "[]"
                })
                .unwrap();
            let iterable = context.eval(if fail {
                r#"(function(){let trace='';globalThis.readTrace=function(){return trace};return {
                    [Symbol.iterator](){return {
                        next(){return {done:false,value:40}},
                        get return(){trace=trace+'r';return function(){trace=trace+'c';throw 99}}
                    }}
                }})()"#
            } else { "[40,2]" }).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(xs){return [...xs]})",
                vec![],
            );
            let pc = entry
                .executable
                .code
                .iter()
                .position(|op| matches!(op, crate::engine::code::bytecode::Instruction::Append))
                .unwrap();
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let frame = execution.frames.current_mut(id).unwrap();
            frame.resume_pc = pc;
            execution.slots.push(&mut frame.window, target).unwrap();
            execution
                .slots
                .push(&mut frame.window, Value::Int(if fail { 0 } else { -1 }))
                .unwrap();
            execution.slots.push(&mut frame.window, iterable).unwrap();
            let profile = CostProfile::start();
            let completion = execute_running(runtime.clone(), execution).unwrap();
            let costs = profile.snapshot();
            drop(profile);
            if fail {
                let Completion::Throw(Value::Object(error)) = completion else {
                    panic!("expected original write error")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(
                        "property is not configurable"
                    ))
                );
                assert_eq!(
                    context.eval("readTrace()").unwrap(),
                    Value::String(crate::engine::value::JsString::from_static("rc"))
                );
            } else {
                let Completion::Return(Value::Object(array)) = completion else {
                    panic!("expected wrapped-index array")
                };
                for (key, expected) in [("4294967295", 40), ("0", 2), ("length", 1)] {
                    assert_eq!(
                        context
                            .get_property(&array, &runtime.intern_property_key(key).unwrap())
                            .unwrap(),
                        Value::Int(expected)
                    );
                }
            }
            assert_eq!(costs.legacy_dispatches, 0, "fail={fail}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "fail={fail}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn array_element_checkpoint_defines_own_properties_without_setters() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .eval("Object.defineProperty(Array.prototype,'1',{set(){throw 99},configurable:true})")
            .unwrap();
        let array = context.eval("[]").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(){return [...[],40]})",
            vec![],
        );
        let pc = entry
            .executable
            .code
            .iter()
            .position(|op| {
                matches!(
                    op,
                    crate::engine::code::bytecode::Instruction::DefineArrayEl
                )
            })
            .unwrap();
        let mut execution = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let id = push_frame(&mut execution, entry).unwrap();
        let frame = execution.frames.current_mut(id).unwrap();
        // Start at the published element write; Append itself is not exercised here.
        frame.resume_pc = pc;
        execution
            .slots
            .push(&mut frame.window, array.clone())
            .unwrap();
        execution
            .slots
            .push(&mut frame.window, Value::Int(1))
            .unwrap();
        execution
            .slots
            .push(&mut frame.window, Value::Int(40))
            .unwrap();
        let profile = CostProfile::start();
        assert!(matches!(
            run(&mut execution, id).unwrap(),
            RunExit::Environment(super::super::environment_driver::Operation::DefineArrayElement)
        ));
        assert!(matches!(
            super::super::environment_driver::step(
                &runtime,
                &mut execution,
                id,
                super::super::environment_driver::Operation::DefineArrayElement
            )
            .unwrap(),
            CallStep::Entered
        ));
        let frame = execution.frames.current_mut(id).unwrap();
        assert_eq!(frame.resume_pc, pc + 1);
        assert_eq!(execution.slots.depth(&frame.window), 2);
        assert_eq!(
            execution.slots.peek(&frame.window, 0).unwrap(),
            &Value::Int(1)
        );
        assert_eq!(execution.slots.peek(&frame.window, 1).unwrap(), &array);
        let Completion::Return(Value::Object(array)) =
            execute_running(runtime.clone(), execution).unwrap()
        else {
            panic!("expected sparse array")
        };
        let costs = profile.snapshot();
        assert_eq!(
            context
                .get_property(&array, &runtime.intern_property_key("length").unwrap())
                .unwrap(),
            Value::Int(2)
        );
        assert!(
            runtime
                .get_own_property(&array, &runtime.intern_property_key("0").unwrap())
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            runtime
                .get_own_property(&array, &runtime.intern_property_key("1").unwrap())
                .unwrap(),
            Some(
                crate::engine::object::CompleteOrdinaryPropertyDescriptor::Data {
                    value: Value::Int(40),
                    writable: true,
                    enumerable: true,
                    configurable: true
                }
            )
        ));
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn array_literals_preserve_values_and_callee_realm_without_setters() {
        let runtime = Runtime::new();
        let mut caller = runtime.new_context();
        let mut callee = runtime.new_context();
        callee
            .eval("Object.defineProperty(Array.prototype,'0',{set(){throw 99},configurable:true})")
            .unwrap();
        let prototype = callee.eval("Array.prototype").unwrap();
        for (source, expected) in [
            ("(function(){return []})", vec![]),
            (
                "(function(a){return [40,a,2]})",
                vec![Value::Int(40), Value::Int(41), Value::Int(2)],
            ),
            (
                "(function(){return eval('[40,2]')})",
                vec![Value::Int(40), Value::Int(2)],
            ),
        ] {
            let Value::Object(function) = callee.eval(source).unwrap() else {
                panic!("expected function")
            };
            let callable = runtime.as_callable(&function).unwrap().unwrap();
            let entry = callable_entry(&runtime, &mut caller, callable, vec![Value::Int(41)]);
            let profile = CostProfile::start();
            let Completion::Return(Value::Object(array)) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected array")
            };
            let costs = profile.snapshot();
            assert_eq!(
                runtime
                    .fast_array_like_values(&array, expected.len() as u32)
                    .unwrap(),
                Some(expected)
            );
            assert_eq!(
                runtime.get_prototype_of(&array).unwrap().map(Value::Object),
                Some(prototype.clone())
            );
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn apply_eval_checkpoint_runs_children_and_cleans_argument_snapshots() {
        for (callee_source, array_source, expected, throws) in [
            ("eval", "['40+2',{}]", 42, false),
            ("eval", "[42,{}]", 42, false),
            ("eval", "[]", 0, false),
            ("eval", "['throw 42',{}]", 42, true),
            ("(function(a,b){return a+b})", "[40,2]", 42, false),
            (
                "(function(a,b){return a+b}).bind(null,40)",
                "[2]",
                42,
                false,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callee = context.eval(callee_source).unwrap();
            let array = context.eval(array_source).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(){'use strict';return eval(...[])})",
                vec![callee.clone()],
            );
            let pc = entry
                .executable
                .code
                .iter()
                .position(|op| {
                    matches!(
                        op,
                        crate::engine::code::bytecode::Instruction::ApplyEval { .. }
                    )
                })
                .unwrap();
            let retained = if array_source == "['40+2',{}]" {
                let Value::Object(carrier) = &array else {
                    unreachable!()
                };
                let Value::Object(extra) = context
                    .get_property(carrier, &runtime.intern_property_key("1").unwrap())
                    .unwrap()
                else {
                    panic!("expected extra argument")
                };
                Some(extra.object_id())
            } else {
                None
            };
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let frame = execution.frames.current_mut(id).unwrap();
            frame.resume_pc = pc;
            execution.slots.push(&mut frame.window, callee).unwrap();
            execution
                .slots
                .push(&mut frame.window, array.clone())
                .unwrap();
            let profile = CostProfile::start();
            if let Some(extra) = retained {
                let RunExit::ApplyEval(environment) = run(&mut execution, id).unwrap() else {
                    panic!("expected ApplyEval")
                };
                assert!(matches!(
                    super::super::eval_driver::apply(&runtime, &mut execution, id, environment)
                        .unwrap(),
                    CallStep::Entered
                ));
                assert_ne!(execution.frames.current_id(), Some(id));
                let Value::Object(carrier) = &array else {
                    unreachable!()
                };
                assert!(
                    runtime
                        .delete_property(carrier, &runtime.intern_property_key("1").unwrap())
                        .unwrap()
                );
                assert!(
                    runtime
                        .0
                        .state
                        .borrow()
                        .heap
                        .object_strong_count(extra)
                        .unwrap()
                        > 0,
                    "snapshot must survive carrier mutation"
                );
            }
            let completion = execute_running(runtime.clone(), execution).unwrap();
            let costs = profile.snapshot();
            if let Some(extra) = retained {
                assert_eq!(
                    runtime
                        .0
                        .state
                        .borrow()
                        .heap
                        .object_strong_count(extra)
                        .unwrap_or(0),
                    0,
                    "snapshot must be released after reply"
                );
            }

            if throws {
                assert!(
                    matches!(completion, Completion::Throw(Value::Int(value)) if value == expected)
                );
            } else if array_source == "[]" {
                assert!(matches!(completion, Completion::Return(Value::Undefined)));
            } else {
                assert!(
                    matches!(completion, Completion::Return(Value::Int(value)) if value == expected),
                    "{callee_source}: {completion:?}"
                );
            }
            assert_eq!(costs.legacy_dispatches, 0, "{callee_source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{callee_source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn direct_eval_keeps_boxed_this_identity_in_bound_callers() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let bound = context
            .eval("(function(){return eval('this')===this?42:0}).bind(7)")
            .unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(f){return f()})",
            vec![bound],
        );
        let profile = CostProfile::start();
        let Completion::Return(value) =
            execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
        else {
            panic!("expected eval this result")
        };
        let costs = profile.snapshot();
        assert_eq!(value, Value::Int(42));
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert!(costs.owned_storage.maximum_frame_depth >= 3, "{costs:?}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn direct_eval_runs_in_owned_children_and_shares_caller_bindings() {
        for (source, expected, message) in [
            ("(function(){let x=40;return eval('x+2')})", 42, None),
            ("(function(){let x=1;eval('x=42');return x})", 42, None),
            ("(function(){eval('var x=42');return x})", 42, None),
            (
                "(function(){'use strict';let x=40;return eval('x+2')})",
                42,
                None,
            ),
            (
                "(function(){let x=40;return eval('eval(\"x+2\")')})",
                42,
                None,
            ),
            (
                "(function(){try{return eval('throw 42')}catch(e){return e}})",
                42,
                None,
            ),
            (
                "(function(){try{return eval('let =')}catch(e){return 42}})",
                42,
                None,
            ),
            ("(function(){return eval(42)})", 42, None),
            ("(function(){var n=0;return eval(41,n=n+1)+n})", 42, None),
            (
                "(function(){let x=eval('x');return x})",
                0,
                Some("x is not initialized"),
            ),
            (
                "(function(){let eval=function(a,b){return a+b};return eval(40,2)})",
                42,
                None,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(&runtime, &mut context, source, vec![]);
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let costs = profile.snapshot();
            if let Some(message) = message {
                let Completion::Throw(Value::Object(error)) = completion else {
                    panic!("expected eval TDZ error")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(message))
                );
            } else {
                assert!(
                    matches!(completion, Completion::Return(ref value) if value == &Value::Int(expected)),
                    "{source}: {completion:?}"
                );
            }
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn global_reference_accessors_and_new_bindings_use_owned_calls() {
        for (setup, source, expected, stored, error_message) in [
            (
                "delete globalThis.x",
                "(function(o){with(o){return x=(function(){return 42})()}})",
                42,
                Some(42),
                None,
            ),
            (
                "(function(){let n=1;Object.defineProperty(globalThis,'x',{get(){return n},set(v){n=v;return 99},configurable:true})})()",
                "(function(o){with(o){return x+=(function(){return 41})()}})",
                42,
                Some(42),
                None,
            ),
            (
                "(function(){let n=0;globalThis.marker=41;Object.defineProperty(globalThis,'x',{get(){return n},set(v){n=n+v+this.marker;return 99},configurable:true})})()",
                "(function(o){with(o){return x+=(function(){return 1})()}})",
                1,
                Some(42),
                None,
            ),
            (
                "Object.defineProperty(globalThis,'x',{get(){throw 42},configurable:true})",
                "(function(o){try{with(o){x+=(function(){throw 99})()}}catch(e){return e}})",
                42,
                None,
                None,
            ),
            (
                "Object.defineProperty(globalThis,'x',{value:1,writable:false,configurable:true})",
                "(function(o){with(o){return x=(function(){return 42})()}})",
                42,
                Some(1),
                None,
            ),
            (
                "delete globalThis.x",
                "(function(o){with(o){return function(){'use strict';try{x=(function(){return 42})()}catch(e){return e}}}})({})",
                0,
                None,
                Some("'x' is not defined"),
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context.eval(setup).unwrap();
            let object = context.eval("({})").unwrap();
            let entry = entry(&runtime, &mut context, source, vec![object]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected global access result")
            };
            let costs = profile.snapshot();
            if let Some(message) = error_message {
                let Value::Object(error) = value else {
                    panic!("expected strict unresolved error")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(message))
                );
            } else {
                assert_eq!(value, Value::Int(expected), "{source}");
            }
            if let Some(stored) = stored {
                assert_eq!(
                    context
                        .get_property(
                            &runtime.global_object_for_realm(context.realm).unwrap(),
                            &runtime.intern_property_key("x").unwrap()
                        )
                        .unwrap(),
                    Value::Int(stored)
                );
            }
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn global_reference_checkpoint_checks_presence_without_invoking_getters() {
        for (setup, present) in [
            ("globalThis.x=1", true),
            (
                "Object.defineProperty(globalThis,'x',{get(){throw 99},configurable:true})",
                true,
            ),
            (
                "Object.setPrototypeOf(globalThis,{get x(){throw 99}})",
                true,
            ),
            ("delete globalThis.x", false),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){with(o){x=(function(){return 42})()}})",
                vec![Value::Undefined],
            );
            let pc = entry
                .executable
                .code
                .iter()
                .position(|op| {
                    matches!(
                        op,
                        crate::engine::code::bytecode::Instruction::GlobalReference(_)
                    )
                })
                .unwrap();
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            execution.frames.current_mut(id).unwrap().resume_pc = pc;
            let RunExit::Environment(
                op @ super::super::environment_driver::Operation::GlobalReference(_),
            ) = run(&mut execution, id).unwrap()
            else {
                panic!("expected global reference")
            };
            assert!(matches!(
                super::super::environment_driver::step(&runtime, &mut execution, id, op).unwrap(),
                CallStep::Entered
            ));
            let frame = execution.frames.current_mut(id).unwrap();
            assert_eq!(frame.resume_pc, pc + 1);
            assert_eq!(execution.slots.depth(&frame.window), 1);
            let expected = if present {
                Value::Object(runtime.global_object_for_realm(context.realm).unwrap())
            } else {
                Value::Undefined
            };
            assert_eq!(execution.slots.peek(&frame.window, 0).unwrap(), &expected);
            drop(execution);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn global_references_prefer_late_lexical_cells_before_evaluating_rhs() {
        for (lexical, is_const, initial, expected_error) in [
            (false, false, Some(Value::Int(1)), None),
            (true, false, Some(Value::Int(1)), None),
            (true, true, Some(Value::Int(1)), Some("'x' is read-only")),
            (true, false, None, Some("x is not initialized")),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context.eval("globalThis.x=1").unwrap();
            let object = context.eval("({})").unwrap();
            let source = if expected_error.is_some() {
                "(function(o){try{with(o){return x=(function(){throw 99})()}}catch(e){return e}})"
            } else {
                "(function(o){with(o){return x=(function(){return 42})()}})"
            };
            let entry = entry(&runtime, &mut context, source, vec![object]);
            assert!(entry.executable.code.iter().any(|op| matches!(
                op,
                crate::engine::code::bytecode::Instruction::GlobalReference(_)
            )));
            // Publish against the property first, then introduce a live lexical cell.
            if lexical {
                context
                    .create_global_lexical_for_test("x", is_const, initial)
                    .unwrap();
            }
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected global reference result")
            };
            let costs = profile.snapshot();
            let key = runtime.intern_property_key("x").unwrap();
            if let Some(message) = expected_error {
                let Value::Object(error) = value else {
                    panic!("RHS ran before lexical validation")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(message))
                );
            } else {
                assert_eq!(value, Value::Int(42));
                if lexical {
                    let root = runtime
                        .own_var_ref_root(&context.global_var_object().unwrap(), &key)
                        .unwrap()
                        .unwrap();
                    assert_eq!(runtime.read_var_ref(&root).unwrap(), Value::Int(42));
                }
            }
            let global = runtime.global_object_for_realm(context.realm).unwrap();
            assert_eq!(
                context.get_property(&global, &key).unwrap(),
                Value::Int(if lexical { 1 } else { 42 })
            );
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn reference_read_checkpoint_preserves_cells_and_reports_unresolved_names() {
        use crate::engine::vm::environment_driver::Operation;
        for (initial, unresolved, expected_error) in [
            (Some(Value::Int(42)), false, None),
            (None, false, Some("x is not initialized")),
            (Some(Value::Int(42)), true, Some("'x' is not defined")),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context
                .create_global_lexical_for_test("x", false, initial)
                .unwrap();
            let object = context.global_var_object().unwrap();
            let key = runtime.intern_property_key("x").unwrap();
            let root = runtime.own_var_ref_root(&object, &key).unwrap().unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){with(o){x+=(function(){return 1})()}})",
                vec![Value::Object(object.clone())],
            );
            let pc = entry
                .executable
                .code
                .iter()
                .position(|op| {
                    matches!(
                        op,
                        crate::engine::code::bytecode::Instruction::GetRefValue(_)
                            | crate::engine::code::bytecode::Instruction::GetRefValueUndef(_)
                    )
                })
                .expect("expected published reference read");
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let frame = execution.frames.current_mut(id).unwrap();
            frame.resume_pc = pc;
            let base = if unresolved {
                Value::Undefined
            } else {
                Value::Object(object.clone())
            };
            execution
                .slots
                .push(&mut frame.window, base.clone())
                .unwrap();
            let RunExit::Environment(op @ Operation::ReadReference { .. }) =
                run(&mut execution, id).unwrap()
            else {
                panic!("expected reference read")
            };
            let profile = CostProfile::start();
            let result =
                super::super::environment_driver::step(&runtime, &mut execution, id, op).unwrap();
            let costs = profile.snapshot();
            if let Some(message) = expected_error {
                let CallStep::Complete(Completion::Throw(Value::Object(error))) = result else {
                    panic!("expected reference error")
                };
                drop(execution);
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(message))
                );
            } else {
                assert!(matches!(result, CallStep::Entered));
                let frame = execution.frames.current_mut(id).unwrap();
                assert_eq!(frame.resume_pc, pc + 1);
                assert_eq!(execution.slots.depth(&frame.window), 2);
                assert_eq!(
                    execution.slots.peek(&frame.window, 0).unwrap(),
                    &Value::Int(42)
                );
                assert_eq!(execution.slots.peek(&frame.window, 1).unwrap(), &base);
                drop(execution);
            }
            assert_eq!(
                runtime
                    .own_var_ref_root(&object, &key)
                    .unwrap()
                    .unwrap()
                    .id(),
                root.id()
            );
            assert_eq!(costs.legacy_dispatches, 0);
            assert_eq!(costs.owned_bridge_exits, 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn reference_write_checkpoint_preserves_live_lexical_cells() {
        use crate::engine::vm::environment_driver::{Operation, WriteTarget};
        for (is_const, initial, strict, expected_error) in [
            (false, Some(Value::Int(1)), false, None),
            (false, Some(Value::Int(1)), true, None),
            (true, Some(Value::Int(1)), false, None),
            (true, Some(Value::Int(1)), true, Some("'x' is read-only")),
            (false, None, false, Some("x is not initialized")),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context
                .create_global_lexical_for_test("x", is_const, initial.clone())
                .unwrap();
            let object = context.global_var_object().unwrap();
            let key = runtime.intern_property_key("x").unwrap();
            let root = runtime.own_var_ref_root(&object, &key).unwrap().unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                if strict {
                    "(function(o){with(o){return function(){'use strict';x=(function(){return 42})()}}})({})"
                } else {
                    "(function(o){with(o){x=(function(){return 42})()}})"
                },
                vec![Value::Object(object.clone())],
            );
            let (pc, name) = entry
                .executable
                .code
                .iter()
                .enumerate()
                .find_map(|(pc, op)| match op {
                    crate::engine::code::bytecode::Instruction::PutRefValue(name) => {
                        Some((pc, *name))
                    }
                    _ => None,
                })
                .expect("expected published reference write");
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let frame = execution.frames.current_mut(id).unwrap();
            frame.resume_pc = pc;
            execution
                .slots
                .push(&mut frame.window, Value::Object(object.clone()))
                .unwrap();
            execution
                .slots
                .push(&mut frame.window, Value::Int(42))
                .unwrap();
            let op = Operation::Put {
                source: WriteTarget::Reference,
                name,
                strict,
                check_presence: true,
            };
            assert_eq!(run(&mut execution, id).unwrap(), RunExit::Environment(op));
            let result = super::super::environment_driver::step(
                &runtime,
                &mut execution,
                id,
                Operation::Put {
                    source: WriteTarget::Reference,
                    name,
                    strict,
                    check_presence: true,
                },
            )
            .unwrap();
            if let Some(message) = expected_error {
                let CallStep::Complete(Completion::Throw(Value::Object(error))) = result else {
                    panic!("expected lexical rejection")
                };
                drop(execution);
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(message))
                );
            } else {
                assert!(matches!(result, CallStep::Entered));
                let frame = execution.frames.current_mut(id).unwrap();
                assert_eq!(frame.resume_pc, pc + 1);
                assert_eq!(execution.slots.depth(&frame.window), 0);
                assert_eq!(
                    runtime.read_var_ref(&root).unwrap(),
                    if is_const {
                        Value::Int(1)
                    } else {
                        Value::Int(42)
                    }
                );
                drop(execution);
            }
            assert_eq!(
                runtime
                    .own_var_ref_root(&object, &key)
                    .unwrap()
                    .unwrap()
                    .id(),
                root.id()
            );
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn dynamic_assignments_preserve_setter_receiver_and_rejection_rules() {
        for (setup, body, expected_error) in [
            ("({x:0})", "with(o){x=42;return x}", None),
            ("Object.create({x:0})", "with(o){x=42;return x}", None),
            (
                "(function(){let n=0;return {y:1,get x(){return n},set x(v){n=n+v+this.y;return 99}}})()",
                "with(o){x=41;return x}",
                None,
            ),
            (
                "(function(){let n=0;return Object.create({get x(){return n},set x(v){n=n+v+this.y;return 99}}, {y:{value:1}})})()",
                "with(o){x=41;return x}",
                None,
            ),
            (
                "({set x(v){throw v}})",
                "try{with(o){x=42}}catch(e){return e}",
                None,
            ),
            (
                "Object.defineProperty({},'x',{value:42})",
                "with(o){x=99;return x}",
                None,
            ),
            ("({get x(){return 42}})", "with(o){x=99;return x}", None),
            (
                "({x:1})",
                "with(o){x=(function(){delete x;return 42})()}return o.x",
                None,
            ),
            (
                "Object.defineProperty({},'x',{value:42})",
                "try{with(o){return (function(){'use strict';x=99})()}}catch(e){return e}",
                Some("'x' is read-only"),
            ),
            (
                "({get x(){return 42}})",
                "try{with(o){return (function(){'use strict';x=99})()}}catch(e){return e}",
                Some("no setter for property"),
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected assignment result: {body}")
            };
            let costs = profile.snapshot();
            if let Some(message) = expected_error {
                let Value::Object(error) = value else {
                    panic!("expected error: {body}")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(message))
                );
            } else {
                assert_eq!(value, Value::Int(42), "{body}");
            }
            assert_eq!(costs.legacy_dispatches, 0, "{body}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{body}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn published_eval_declarations_and_reads_use_owned_environment() {
        use crate::engine::code::debug::DebugInfoMode;
        use crate::engine::code::function::metadata::{ClosureVariableKind, EvalRootBinding};
        use crate::engine::compiler::{EvalCompileContext, compile_unlinked_eval_with_filename};
        use crate::engine::value::JsString;

        for (source, setup, expected, error_message) in [
            ("var x; x", "Object.create(null)", Value::Undefined, None),
            (
                "var x; x=42; x",
                "Object.create(null)",
                Value::Int(42),
                None,
            ),
            (
                "function x(){return 42} x()",
                "Object.create(null)",
                Value::Int(42),
                None,
            ),
            (
                "var x; function x(){return 42} x()",
                "Object.create(null)",
                Value::Int(42),
                None,
            ),
            (
                "var x; delete x",
                "Object.create(null)",
                Value::Bool(true),
                None,
            ),
            (
                "x",
                "({x:42,get [Symbol.unscopables](){throw 99}})",
                Value::Int(42),
                None,
            ),
            (
                "x",
                "Object.create({get x(){return this.y}}, {y:{value:42}})",
                Value::Int(42),
                None,
            ),
            (
                "try{x}catch(e){e}",
                "({get x(){throw 42}})",
                Value::Int(42),
                None,
            ),
            ("var x; x", "({get x(){throw 99}})", Value::Undefined, None),
            (
                "var x; x",
                "Object.defineProperty({},'x',{value:42})",
                Value::Undefined,
                Some("property is not configurable"),
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let eval_context = EvalCompileContext::direct(
                false,
                vec![EvalRootBinding {
                    name: JsString::from_static("<var>"),
                    scope: 0,
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::EvalVariableObject,
                    is_catch_parameter: false,
                }],
            );
            let unlinked = compile_unlinked_eval_with_filename(
                source,
                "<eval>",
                DebugInfoMode::Full,
                eval_context.clone(),
            )
            .unwrap();
            let verified = crate::engine::code::bytecode_publish::VerifiedFunction::eval(
                unlinked,
                crate::engine::api::compile::eval_publication_input(&eval_context),
            )
            .unwrap();
            let bytecode = runtime
                .publish_verified_unlinked_function(context.realm, verified)
                .unwrap();
            let Value::Object(object) = context.eval(setup).unwrap() else {
                panic!("expected eval environment")
            };
            let root = runtime
                .new_var_ref(
                    Value::Object(object),
                    false,
                    false,
                    ClosureVariableKind::EvalVariableObject,
                )
                .unwrap();
            let mut roots = vec![root];
            let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
            for descriptor in snapshot.closure_variables.iter().skip(1) {
                use crate::engine::code::function::metadata::{ClosureSource, ClosureVariableName};
                assert_eq!(descriptor.source, ClosureSource::Global);
                let ClosureVariableName::Atom(name) = descriptor.name else {
                    panic!("expected global atom")
                };
                roots.push(runtime.resolve_global_var(context.realm, name).unwrap());
            }
            let callable = runtime
                .new_bytecode_closure_with_slots(context.realm, &bytecode, &roots)
                .unwrap();
            let entry = callable_entry(&runtime, &mut context, callable, Vec::new());
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let costs = profile.snapshot();
            if let Some(message) = error_message {
                let Completion::Throw(Value::Object(error)) = completion else {
                    panic!("{source}: expected definition error")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(JsString::from_static(message))
                );
            } else {
                assert!(
                    matches!(completion, Completion::Return(ref value) if value == &expected),
                    "{source}: {completion:?}"
                );
            }
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn dynamic_deletion_respects_own_descriptors_and_unscopables() {
        for (setup, expected, own_after) in [
            ("({x:42})", true, false),
            (
                "Object.defineProperty({},'x',{value:42,configurable:false})",
                false,
                true,
            ),
            ("({get x(){throw 99}})", true, false),
            ("Object.create({x:42})", true, false),
            ("({x:42,[Symbol.unscopables]:{x:true}})", false, true),
            ("({})", false, false),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let Value::Object(object) = context.eval(setup).unwrap() else {
                panic!("expected delete target")
            };
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){let x=41;with(o){return delete x}})",
                vec![Value::Object(object.clone())],
            );
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected delete result")
            };
            assert_eq!(value, Value::Bool(expected), "{setup}");
            let costs = profile.snapshot();
            let key = runtime.intern_property_key("x").unwrap();
            assert_eq!(
                runtime.get_own_property(&object, &key).unwrap().is_some(),
                own_after,
                "{setup}"
            );
            assert_eq!(costs.legacy_dispatches, 0, "{setup}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{setup}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn class_heritage_getter_replies_once_before_publication() {
        for (getter, expected_error) in [
            ("function(){n=n+1;return prototype}", false),
            ("function(){n=n+1;throw 41}", false),
            ("function(){n=n+1;return 1}", true),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let parent = context.eval(&format!("(function(){{var n=0,prototype={{}};var B=(function(){{}}).bind(null);Object.defineProperty(B,'prototype',{{get:{getter}}});return [B,function(){{return n}}]}})()")).unwrap();
            let Value::Object(pair) = parent else {
                panic!("expected parent pair")
            };
            let parent = context
                .get_property(&pair, &runtime.intern_property_key("0").unwrap())
                .unwrap();
            let counter = context
                .get_property(&pair, &runtime.intern_property_key("1").unwrap())
                .unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(B,count){try{class C extends B {}return 41+count()}catch(e){if(e===41)return e+count();return e}})",
                vec![parent, counter],
            );
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("expected class heritage completion")
            };
            let costs = profile.snapshot();
            if expected_error {
                let Value::Object(error) = value else {
                    panic!("expected invalid prototype error")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("name").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static("TypeError"))
                );
            } else {
                assert_eq!(value, Value::Int(42))
            }
            assert_eq!(costs.legacy_dispatches, 0, "{getter}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{getter}: {costs:?}");
            assert_eq!(costs.owned_storage.maximum_frame_depth, 2);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn class_initializer_installation_and_static_calls_do_not_replay() {
        use super::super::construct_driver::{InitializerKind, initializer};
        use crate::engine::code::bytecode::Instruction;
        use crate::engine::code::function::metadata::ClassInitializerKind;
        use crate::engine::code::rooted::FunctionBytecodeRef;
        use crate::engine::heap::BytecodeConstant;

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(){return class {x=42; static {throw 42}}})",
            vec![],
        );
        let pc = entry
            .executable
            .code
            .iter()
            .position(|op| matches!(op, Instruction::RunClassStaticInitializer))
            .unwrap();
        let mut constructor = None;
        let mut static_initializer = None;
        let mut instance_initializer = None;
        for constant in entry.executable.constants.iter() {
            let BytecodeConstant::Function(child) = constant else {
                continue;
            };
            let metadata = runtime
                .0
                .state
                .borrow()
                .heap
                .function_bytecode(*child)
                .unwrap()
                .metadata;
            let bytecode =
                FunctionBytecodeRef::from_borrowed_handle(runtime.clone(), *child).unwrap();
            let callable = runtime
                .new_bytecode_closure(context.realm, &bytecode)
                .unwrap();
            if metadata.class_initializer_kind == Some(ClassInitializerKind::StaticElements) {
                static_initializer = Some(Value::Object(callable.into_object()));
            } else if metadata.class_initializer_kind == Some(ClassInitializerKind::InstanceFields)
            {
                instance_initializer = Some(Value::Object(callable.into_object()));
            } else if metadata.constructor_kind
                != crate::engine::code::function::metadata::ConstructorKind::None
            {
                constructor = Some(Value::Object(callable.into_object()));
            }
        }
        let crate::engine::vm::DefineClassOutcome::Defined {
            constructor,
            prototype,
        } = runtime
            .define_class_pair(
                context.realm,
                Value::Undefined,
                constructor.unwrap(),
                &crate::engine::value::JsString::from_static(""),
                false,
            )
            .unwrap()
        else {
            panic!("expected fresh class pair")
        };
        let static_initializer = static_initializer.unwrap();
        let install_pc = entry
            .executable
            .code
            .iter()
            .position(|op| matches!(op, Instruction::InstallClassInstanceInitializer))
            .unwrap();
        let mut execution = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let parent = push_frame(&mut execution, entry).unwrap();
        let frame = execution.frames.current_mut(parent).unwrap();
        frame.resume_pc = install_pc;
        execution
            .slots
            .push(&mut frame.window, constructor.clone())
            .unwrap();
        execution
            .slots
            .push(&mut frame.window, prototype.clone())
            .unwrap();
        execution
            .slots
            .push(&mut frame.window, instance_initializer.unwrap())
            .unwrap();
        let profile = CostProfile::start();
        assert_eq!(
            run(&mut execution, parent).unwrap(),
            RunExit::ClassInitializer(InitializerKind::Install)
        );
        assert!(matches!(
            initializer(&runtime, &mut execution, parent, InitializerKind::Install).unwrap(),
            CallStep::Entered
        ));
        assert_eq!(execution.frames.current_id(), Some(parent));
        let frame = execution.frames.current_mut(parent).unwrap();
        assert_eq!(frame.resume_pc, install_pc + 1);
        assert_eq!(execution.slots.depth(&frame.window), 2);
        assert_eq!(
            execution.slots.peek(&frame.window, 1).unwrap(),
            &constructor
        );
        assert_eq!(execution.slots.pop(&mut frame.window).unwrap(), prototype);
        frame.resume_pc = pc;
        execution
            .slots
            .push(&mut frame.window, static_initializer.clone())
            .unwrap();
        assert_eq!(
            run(&mut execution, parent).unwrap(),
            RunExit::ClassInitializer(InitializerKind::Static)
        );
        assert!(matches!(
            initializer(&runtime, &mut execution, parent, InitializerKind::Static).unwrap(),
            CallStep::Entered
        ));
        let child = execution.frames.current_id().unwrap();
        assert_ne!(child, parent);
        let RunExit::InstantiateClosure(index) = run(&mut execution, child).unwrap() else {
            panic!("expected static block closure")
        };
        super::super::closure_driver::instantiate(&runtime, &mut execution, child, index).unwrap();
        assert_eq!(
            run(&mut execution, child).unwrap(),
            RunExit::ClassInitializer(InitializerKind::Block)
        );
        assert!(matches!(
            initializer(&runtime, &mut execution, child, InitializerKind::Block).unwrap(),
            CallStep::Entered
        ));
        let block = execution.frames.current_id().unwrap();
        assert_ne!(block, child);
        assert_eq!(run(&mut execution, block).unwrap(), RunExit::Throw);
        let frame = execution.frames.current_mut(block).unwrap();
        assert_eq!(
            execution.slots.pop(&mut frame.window).unwrap(),
            Value::Int(42)
        );
        // Preparation is irreversible even when the body throws.
        assert!(
            runtime
                .begin_class_static_initializer(context.realm, constructor, static_initializer)
                .is_err()
        );
        let costs = profile.snapshot();
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert_eq!(costs.owned_storage.maximum_frame_depth, 3);
        drop(execution);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn logical_limit_returns_a_throw_and_unwinds_every_active_frame() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let entry = entry(
            &runtime,
            &mut context,
            "(function f(){return f()})",
            Vec::new(),
        );
        let profile = CostProfile::start();
        let completion = execute(
            runtime.clone(),
            entry,
            ExecutionLimits {
                frames: 8,
                slots: 1024,
            },
        )
        .unwrap();
        let Completion::Throw(Value::Object(error)) = completion else {
            panic!("expected catchable overflow");
        };
        assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, 8);
        assert_eq!(profile.snapshot().owned_storage.frames_pushed, 8);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        assert_eq!(
            context
                .get_property(&error, &runtime.intern_property_key("message").unwrap())
                .unwrap(),
            Value::String(crate::engine::value::JsString::from_static(
                "stack overflow"
            ))
        );
    }
    #[test]
    fn nested_bound_calls_keep_argument_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callee = context
            .eval("(function(a,b,c){return a*100+b*10+c}).bind(1000,1).bind(9000,2)")
            .unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(f){return f(3)})",
            vec![callee],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(matches!(result, Completion::Return(Value::Int(123))));
        let costs = profile.snapshot();
        assert_eq!(costs.owned_storage.maximum_frame_depth, 2);
        assert_eq!(costs.legacy_dispatches, 0);
        assert_eq!(costs.owned_bridge_exits, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn bound_receiver_and_native_fallback_preserve_original_call() {
        for (source, expected, depth) in [
            (
                "(function(a,b,c){'use strict';return this+a*100+b*10+c}).bind(1000,1).bind(9000,2)",
                Value::Int(1123),
                2,
            ),
            (
                "String.fromCharCode.bind(null,65).bind(null,66)",
                Value::String(crate::engine::value::JsString::from_static("ABC")),
                1,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callee = context.eval(source).unwrap();
            let argument = if depth == 1 { 67 } else { 3 };
            let entry = entry(
                &runtime,
                &mut context,
                "(function root(f,x){return f(x)})",
                vec![callee, Value::Int(argument)],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = result else {
                panic!("expected return")
            };
            assert_eq!(value, expected);
            assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, depth);
            assert_eq!(profile.snapshot().owned_bridge_exits, 0);
            assert_eq!(profile.snapshot().legacy_dispatches, 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn nonobject_definition_rejections_preserve_static_computed_and_field_errors() {
        use crate::engine::code::bytecode::DefineMethodKind;
        for (computed, method, message) in [
            (
                false,
                true,
                "object-literal method target was not an Object",
            ),
            (
                true,
                true,
                "computed object-literal method target was not an Object",
            ),
            (false, false, "not an object"),
            (true, false, "not an object"),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(k){return {[k](){}}})",
                vec![],
            );
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let frame = execution.frames.current_mut(id).unwrap();
            execution
                .slots
                .push(&mut frame.window, Value::Int(1))
                .unwrap();
            if computed {
                execution
                    .slots
                    .push(&mut frame.window, Value::Int(0))
                    .unwrap();
            }
            execution
                .slots
                .push(&mut frame.window, Value::Int(2))
                .unwrap();
            let profile = CostProfile::start();
            let result = super::super::construct_driver::define_property(
                &runtime,
                &mut execution,
                id,
                (!computed).then_some(0),
                method.then_some((DefineMethodKind::Method, true)),
            );
            drop(execution);
            if method {
                let Err(error) = result else {
                    panic!("invalid method target must remain an internal error")
                };
                assert_eq!(error.kind(), crate::engine::api::ErrorKind::Internal);
                assert!(error.to_string().contains(message));
            } else {
                let Ok(CallStep::Complete(Completion::Throw(Value::Object(error)))) = result else {
                    panic!("expected field TypeError")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(message))
                );
            }
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0);
            assert_eq!(costs.owned_bridge_exits, 0);
            assert_eq!(costs.owned_sync_call_bridges, 0);
        }
    }

    #[test]
    fn method_definition_checkpoints_keep_exotic_coercions_and_native_values_owned() {
        use crate::engine::code::bytecode::Instruction;
        for (target_source, function_source, key, expected, hits) in [
            (
                "[]",
                "(function(){var f=function(){};f.valueOf=function(){hits++;return 2};return f})()",
                "length",
                Some(0),
                2,
            ),
            (
                "new Uint8Array(1)",
                "(function(){var f=function(){};f.valueOf=function(){hits++;return 42};return f})()",
                "0",
                Some(42),
                1,
            ),
            (
                "new Proxy({}, {defineProperty(){throw 99}})",
                "Math.abs",
                "x",
                None,
                0,
            ),
            ("[]", "Math.abs.bind(null)", "x", None, 0),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context.eval("var hits=0").unwrap();
            let target = context.eval(target_source).unwrap();
            let function = context.eval(function_source).unwrap();
            let Value::Object(old_target) = context.eval(target_source).unwrap() else {
                unreachable!()
            };
            let old = runtime
                .define_object_literal_method(
                    context.realm,
                    &old_target,
                    &runtime.intern_property_key(key).unwrap(),
                    function.clone(),
                    crate::engine::code::bytecode::DefineMethodKind::Method,
                    true,
                )
                .unwrap();
            assert!(
                matches!(old, crate::engine::object::operations::PropertyDefineOutcome::Defined(defined) if defined == (key != "length"))
            );
            assert_eq!(context.eval("hits").unwrap(), Value::Int(hits));
            context.eval("hits=0").unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(k){return {[k](){}}})",
                vec![],
            );
            let pc = entry
                .executable
                .code
                .iter()
                .position(|op| matches!(op, Instruction::DefineMethodComputed { .. }))
                .unwrap();
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let frame = execution.frames.current_mut(id).unwrap();
            frame.resume_pc = pc;
            execution
                .slots
                .push(&mut frame.window, target.clone())
                .unwrap();
            execution
                .slots
                .push(
                    &mut frame.window,
                    Value::String(crate::engine::value::JsString::from_static(key)),
                )
                .unwrap();
            execution
                .slots
                .push(&mut frame.window, function.clone())
                .unwrap();
            let profile = CostProfile::start();
            let completion = execute_running(runtime.clone(), execution).unwrap();
            if key == "length" {
                let Completion::Throw(Value::Object(error)) = completion else {
                    panic!("method cannot reconfigure Array length")
                };
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(
                        "property is not configurable"
                    ))
                );
            } else {
                assert_eq!(
                    completion,
                    Completion::Return(target.clone()),
                    "{target_source}"
                );
            }
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{target_source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{target_source}: {costs:?}");
            assert_eq!(
                costs.owned_sync_call_bridges, 0,
                "{target_source}: {costs:?}"
            );
            drop(profile);
            let Value::Object(target) = target else {
                unreachable!()
            };
            let descriptor = runtime
                .get_own_property(&target, &runtime.intern_property_key(key).unwrap())
                .unwrap()
                .unwrap();
            let crate::engine::object::CompleteOrdinaryPropertyDescriptor::Data { value, .. } =
                descriptor
            else {
                panic!("expected method data")
            };
            assert_eq!(value, expected.map(Value::Int).unwrap_or(function));
            assert_eq!(context.eval("hits").unwrap(), Value::Int(hits));
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn boxed_this_identity_survives_owned_frames() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callee = context
            .eval("(function(){return this === ([] instanceof Array,this)}).bind(3)")
            .unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(f){return f()})",
            vec![callee],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(matches!(result, Completion::Return(Value::Bool(true))));
        assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, 2);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert_eq!(profile.snapshot().owned_sync_call_bridges, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn named_accessors_on_non_proxy_storage_use_owned_getter_frames() {
        for expression in [
            "(function(){return arguments})(7)",
            "new Uint8Array(2)",
            "new String('text')",
            "new Number(7)",
            "new Map()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(&format!(
                "var hits=0;var object={expression};object.marker=42;Object.defineProperty(object,'x',{{get:function(){{hits++;return this.marker}}}});object"
            )).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){return o.x})",
                vec![object],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{expression}"
            );
            let costs = profile.snapshot();
            assert_eq!(costs.owned_storage.frames_pushed, 2, "{expression}");
            assert_eq!(costs.legacy_dispatches, 0, "{expression}");
            assert_eq!(costs.owned_bridge_exits, 0, "{expression}");
            assert_eq!(costs.owned_sync_call_bridges, 0, "{expression}");
            drop(profile);
            assert_eq!(context.eval("hits").unwrap(), Value::Int(1));
        }
    }

    #[test]
    fn computed_reads_keep_receivers_holes_and_typed_index_terminals() {
        for (setup, source, expected, calls) in [
            (
                "var hits=0;var object=[,];object.marker=42;Object.setPrototypeOf(object,{get 0(){hits++;return this.marker}});object",
                "(function(o){return o[0]})",
                Value::Int(42),
                1,
            ),
            (
                "var hits=0;var object=(function(){return arguments})(7);object.marker=42;Object.defineProperty(object,'0',{get:function(){hits++;return function(){return this.marker}}});object",
                "(function(o){return o[0]()})",
                Value::Int(42),
                1,
            ),
            (
                "var hits=0;var object=new Uint8Array([7]);Object.setPrototypeOf(object,{get '-0'(){hits++;throw 99},get '1'(){hits++;throw 98}});object",
                "(function(o){var k='-0';return o[k]===undefined && o[1]===undefined})",
                Value::Bool(true),
                0,
            ),
            (
                "var hits=0;var object={};object[Symbol.iterator]=42;object",
                "(function(o){return o[Symbol.iterator]})",
                Value::Int(42),
                0,
            ),
            (
                "var hits=0;var object={'true':42,'1.5':42};object",
                "(function(o){return o[true] + o[1.5]})",
                Value::Int(84),
                0,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(&runtime, &mut context, source, vec![object]);
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = result else {
                panic!("read threw: {source}");
            };
            assert_eq!(value, expected, "{source}");
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{source}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}");
            assert_eq!(costs.owned_sync_call_bridges, 0, "{source}");
            drop(profile);
            assert_eq!(context.eval("hits").unwrap(), Value::Int(calls));
        }
    }

    #[test]
    fn pending_native_call_can_be_abandoned_without_invocation_or_runtime_cycle() {
        let profile = CostProfile::start();
        let (weak, pending) = {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callee = Value::Object(
                runtime
                    .new_bound_native_function(
                        &context.function_prototype().unwrap(),
                        context.realm,
                        crate::engine::builtins::native::NativeFunctionId::ActiveFrameProbe,
                        2,
                    )
                    .unwrap()
                    .as_object()
                    .clone(),
            );
            let callback = context.eval("(function(){throw 42})").unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(f,callback){return f(callback)})",
                vec![callee, callback],
            );
            let weak = std::rc::Rc::downgrade(&runtime.0);
            let mut pending = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let parent = push_frame(&mut pending, entry).unwrap();
            let RunExit::Call {
                arguments,
                method,
                tail,
            } = run(&mut pending, parent).unwrap()
            else {
                panic!("expected native probe call");
            };
            assert!(matches!(
                super::enter_call(&runtime, &mut pending, parent, arguments, method, tail).unwrap(),
                CallStep::Entered
            ));
            let child = pending.frames.current_id().unwrap();
            assert_ne!(child, parent);
            assert_eq!(pending.frames.current_mut(child).unwrap().resume_pc, 0);
            assert!(!runtime.0.state.borrow().active_frames.is_empty());
            (weak, pending)
        };
        assert!(weak.upgrade().is_some());
        assert_eq!(profile.snapshot().owned_sync_call_bridges, 0);
        drop(pending);
        assert!(weak.upgrade().is_none());
        assert_eq!(profile.snapshot().owned_sync_call_bridges, 0);
    }

    #[test]
    fn computed_update_retains_the_original_key_across_owned_getter_and_setter() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = context.eval("var key='x',hits=0,written=0;({get x(){hits++;key='y';return 40},set x(v){written=v},set y(v){throw 99}})").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(o){return o[key]++})",
            vec![object],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(matches!(result, Completion::Return(Value::Int(40))));
        let costs = profile.snapshot();
        assert_eq!(costs.owned_storage.maximum_frame_depth, 2);
        assert_eq!(costs.legacy_dispatches, 0);
        assert_eq!(costs.owned_bridge_exits, 0);
        assert_eq!(costs.owned_sync_call_bridges, 0);
        drop(profile);
        assert_eq!(context.eval("hits").unwrap(), Value::Int(1));
        assert_eq!(context.eval("written").unwrap(), Value::Int(41));
        assert_eq!(
            context.eval("key").unwrap(),
            Value::String(crate::engine::value::JsString::from_static("y"))
        );
    }

    #[test]
    fn nullish_computed_reads_reject_before_key_callbacks() {
        for (source, expected) in [
            (
                "(function(o,k){try{return o[k]}catch(e){return e.message}})",
                "cannot read property of null",
            ),
            (
                "(function(o,k){try{return o[k]++}catch(e){return e.message}})",
                "value has no property",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let key = context
                .eval("var hits=0;({toString(){hits++;throw 99}})")
                .unwrap();
            let entry = entry(&runtime, &mut context, source, vec![Value::Null, key]);
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(value) = result else {
                panic!("read escaped catch");
            };
            assert_eq!(
                value,
                Value::String(crate::engine::value::JsString::from_static(expected))
            );
            let costs = profile.snapshot();
            assert_eq!(costs.owned_bridge_exits, 0);
            assert_eq!(costs.legacy_dispatches, 0);
            assert_eq!(costs.owned_sync_call_bridges, 0);
            drop(profile);
            assert_eq!(context.eval("hits").unwrap(), Value::Int(0));
        }
    }

    #[test]
    fn object_property_keys_resume_string_hint_and_late_method_reads() {
        for (setup, expected_events) in [
            (
                "var events='';var target={marker:42,get x(){events+='G';return this.marker}};var key={get [Symbol.toPrimitive](){events+='M';return function(hint){events+=hint;return 'x'}}}",
                "MstringG",
            ),
            (
                "var events='';var later=function(){throw 99};var target={get x(){events+='G';return 42}};var key={toString(){events+='S';later=function(){events+='V';return 'x'};return this},get valueOf(){events+='M';return later}}",
                "SMVG",
            ),
            (
                "var events='';var target={};Object.defineProperty(target,'x',{get:(function(a,b){events+='G';return this.marker+a+b}).bind({marker:39},1).bind({marker:0},2)});var key={toString:(function(k){events+=this.marker;return k}).bind({marker:'B'},'x')}",
                "BG",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context.eval(setup).unwrap();
            let target = context.eval("target").unwrap();
            let key = context.eval("key").unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o,k){return o[k]})",
                vec![target, key],
            );
            let profile = CostProfile::start();
            assert!(matches!(
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                Completion::Return(Value::Int(42))
            ));
            let costs = profile.snapshot();
            assert_eq!(costs.owned_storage.maximum_frame_depth, 2);
            assert_eq!(costs.legacy_dispatches, 0, "{setup}");
            assert_eq!(costs.owned_bridge_exits, 0, "{setup}");
            assert_eq!(costs.owned_sync_call_bridges, 0, "{setup}");
            drop(profile);
            assert_eq!(
                context.eval("events").unwrap(),
                Value::String(crate::engine::value::JsString::from_static(expected_events))
            );
        }
    }

    #[test]
    fn converted_property_keys_keep_the_evaluated_base_and_throw_identity() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let key = context.eval("var current={x:42};var replacement={x:99};({toString(){current=replacement;return 'x'}})").unwrap();
        let initial_entry = entry(
            &runtime,
            &mut context,
            "(function(k){return current[k]})",
            vec![key],
        );
        let profile = CostProfile::start();
        assert!(matches!(
            execute(runtime.clone(), initial_entry, ExecutionLimits::default()).unwrap(),
            Completion::Return(Value::Int(42))
        ));
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert_eq!(profile.snapshot().owned_sync_call_bridges, 0);
        drop(profile);
        assert_eq!(context.eval("current.x").unwrap(), Value::Int(99));
        for key_source in [
            "({get toString(){hits++;throw token}})",
            "({toString(){hits++;throw token}})",
        ] {
            let token = context.eval("var hits=0;var token={};token").unwrap();
            let key = context.eval(key_source).unwrap();
            let target = context.eval("({get x(){throw 98}})").unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o,k){return o[k]})",
                vec![target, key],
            );
            let profile = CostProfile::start();
            let Completion::Throw(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("key did not throw");
            };
            assert_eq!(value, token);
            assert_eq!(profile.snapshot().owned_bridge_exits, 0);
            assert_eq!(profile.snapshot().owned_sync_call_bridges, 0);
            drop(profile);
            assert_eq!(context.eval("hits").unwrap(), Value::Int(1));
        }
    }

    #[test]
    fn converted_key_and_native_getter_remain_owned() {
        for target_source in [
            "new Proxy({x:42},{get(t,k,r){traps++;return t[k]}})",
            "Object.defineProperty({},'x',{get:Number.prototype.valueOf.bind(42)})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let key = context
                .eval("var keys=0,traps=0;({toString(){keys++;return 'x'}})")
                .unwrap();
            let target = context.eval(target_source).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o,k){return o[k]})",
                vec![target, key],
            );
            let profile = CostProfile::start();
            assert!(matches!(
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                Completion::Return(Value::Int(42))
            ));
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0);
            assert_eq!(costs.owned_bridge_exits, 0);
            assert_eq!(costs.owned_sync_call_bridges, 0);
            drop(profile);
            assert_eq!(context.eval("keys").unwrap(), Value::Int(1));
            assert_eq!(
                context.eval("traps").unwrap(),
                Value::Int(i32::from(target_source.starts_with("new Proxy")))
            );
        }
    }

    #[test]
    fn proxy_get_resumes_nested_handler_reads_and_preserves_receiver() {
        for (setup, expected_trace) in [
            (
                "var trace=0;var p=new Proxy({x:42},{get get(){trace=trace*10+1;return function(t,k,r){trace=trace*10+2;return t[k]}}});p",
                12,
            ),
            (
                "var trace=0;var handler=new Proxy({get get(){trace=trace*10+2;return function(t,k,r){trace=trace*10+3;return t[k]}}},{get(t,k,r){trace=trace*10+1;return t[k]}});var p=new Proxy({x:42},handler);p",
                123,
            ),
            (
                "var trace=0;var target={get x(){trace=trace*10+3;return this===p?42:0}};var inner=new Proxy(target,{get get(){trace=trace*10+2;return null}});var p=new Proxy(inner,{get get(){trace=trace*10+1;return undefined}});p",
                123,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let proxy = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){return o.x})",
                vec![proxy],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(
                    execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                    Completion::Return(Value::Int(42))
                ),
                "{setup}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            assert!(cost.owned_storage.maximum_frame_depth >= 2);
            drop(profile);
            assert_eq!(context.eval("trace").unwrap(), Value::Int(expected_trace));
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn proxy_call_and_callback_traps_use_owned_replies() {
        for (setup, body, expected_hits) in [
            (
                "var hits=0; var p=new Proxy(function(a,b){hits++;return this.n+a+b},{get apply(){hits++;return undefined}}); p.bind({n:10},20)",
                "return o(12)",
                2,
            ),
            (
                "var hits=0; new Proxy(function(){},{apply(t,r,a){hits++;return a[0]+a[1]}})",
                "var x=o(20,21);return x+1",
                1,
            ),
            (
                "var hits=0; var f=new Proxy(function(){},{apply(t,r,a){hits++;return a[0].x}}); new Proxy({x:42},{get:f})",
                "return o.x",
                1,
            ),
            (
                "var hits=0; var f=new Proxy(function(){},{apply(t,r,a){hits++;return r.x}}); var o={x:42}; Object.defineProperty(o,'answer',{get:f});o",
                "return o.answer",
                1,
            ),
            (
                "var hits=0; ({valueOf:new Proxy(function(){},{apply(){hits++;return 42}})})",
                "return +o",
                1,
            ),
            (
                "var hits=0,token={}; new Proxy(function(){},{get apply(){hits++;throw token}})",
                "try{o()}catch(e){return e===token?42:0}",
                1,
            ),
            (
                "var hits=0,token={}; new Proxy(function(){},{apply(){hits++;throw token}})",
                "try{o()}catch(e){return e===token?42:0}",
                1,
            ),
            (
                "var hits=0; var bad=new Proxy({},{get apply(){hits++;return function(){hits+=100}}}); new Proxy({x:42},{get:bad})",
                "try{return o.x}catch(e){return hits===1?42:0}",
                1,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(
                    execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                    Completion::Return(Value::Int(42))
                ),
                "{setup}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            drop(profile);
            assert_eq!(
                context.eval("hits").unwrap(),
                Value::Int(expected_hits),
                "{setup}"
            );
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn cyclic_proxy_apply_obeys_the_execution_limit_without_bytecode_recursion() {
        for (setup, body) in [
            (
                "var h={};var p=new Proxy(function(){},h);h.apply=p;p",
                "return o()",
            ),
            (
                "var h={};var p=new Proxy(function(){},h);h.apply=p;new Proxy({},{get:p})",
                "return o.x",
            ),
            (
                "var h={};var p=new Proxy(function(){},h);h.apply=p;({valueOf:p})",
                "return +o",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!(
                    "(function(o){{try{{{body}}}catch(e){{return e.message==='stack overflow'?42:0}}}})"
                ),
                vec![object],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(
                    execute(
                        runtime.clone(),
                        entry,
                        ExecutionLimits {
                            frames: 32,
                            slots: 4096
                        }
                    )
                    .unwrap(),
                    Completion::Return(Value::Int(42))
                ),
                "{setup}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0);
            assert_eq!(cost.owned_bridge_exits, 0);
            assert_eq!(cost.owned_sync_call_bridges, 0);
            assert_eq!(cost.owned_storage.maximum_frame_depth, 1);
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn proxy_get_throws_once_and_releases_pending_method_guards() {
        for setup in [
            "var hits=0;var token={};new Proxy({},{get get(){hits++;throw token}})",
            "var hits=0;var token={};new Proxy({},{get(){hits++;throw token}})",
            "var hits=0;var token={};new Proxy({},new Proxy({},{get(){hits++;throw token}}))",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let proxy = context.eval(setup).unwrap();
            let token = context.eval("token").unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){try{return o.x}catch(e){return e}})",
                vec![proxy],
            );
            let profile = CostProfile::start();
            let Completion::Return(result) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("throw escaped catch");
            };
            assert_eq!(result, token);
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0);
            assert_eq!(cost.owned_bridge_exits, 0);
            assert_eq!(cost.owned_sync_call_bridges, 0);
            drop(profile);
            assert_eq!(context.eval("hits").unwrap(), Value::Int(1));
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn proxy_get_checks_same_value_and_getterless_invariants_in_reading_realm() {
        for (descriptor, result, throws) in [
            ("{value:42,writable:false,configurable:false}", "42", false),
            ("{value:42,writable:false,configurable:false}", "41", true),
            (
                "{value:NaN,writable:false,configurable:false}",
                "NaN",
                false,
            ),
            ("{value:0,writable:false,configurable:false}", "-0", true),
            ("{get:undefined,configurable:false}", "undefined", false),
            ("{get:undefined,configurable:false}", "42", true),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let mut foreign = runtime.new_context();
            let proxy = foreign.eval(&format!("new Proxy(Object.defineProperty({{}},'x',{descriptor}),{{get(){{return {result}}}}})")).unwrap();
            let expected = foreign.eval(result).unwrap();
            let type_error = context.eval("TypeError.prototype").unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){return o.x})",
                vec![proxy],
            );
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0);
            assert_eq!(cost.owned_bridge_exits, 0);
            assert_eq!(cost.owned_sync_call_bridges, 0);
            drop(profile);
            match completion {
                Completion::Return(value) if !throws => assert!(value.same_value(&expected)),
                Completion::Throw(Value::Object(error)) if throws => {
                    assert_eq!(
                        runtime.get_prototype_of(&error).unwrap().map(Value::Object),
                        Some(type_error)
                    );
                    assert_eq!(
                        context
                            .get_property(&error, &runtime.intern_property_key("message").unwrap())
                            .unwrap(),
                        Value::String(crate::engine::value::JsString::from_static(
                            "proxy: inconsistent get"
                        ))
                    );
                }
                _ => panic!("unexpected invariant result for {descriptor} -> {result}"),
            }
        }
    }

    #[test]
    fn proxy_get_drives_descriptor_fields_and_nested_boolean_queries() {
        for (setup, expected_trace, expected_has, expected_get) in [
            (
                r#"var trace=0,hasCount=0,getCount=0;
                var d={get enumerable(){trace=trace*10+4;return true},get configurable(){trace=trace*10+5;return true},get value(){trace=trace*10+6;return 42},get writable(){trace=trace*10+7;return true}};
                var target=new Proxy({x:42},{get getOwnPropertyDescriptor(){trace=trace*10+2;return function(){trace=trace*10+3;return d}}});
                new Proxy(target,{get(){trace=trace*10+1;return 42}})"#,
                1234567,
                0,
                0,
            ),
            (
                r#"var trace=0,hasCount=0,getCount=0;
                var d={get enumerable(){trace=trace*10+5;return true},get configurable(){trace=trace*10+6;return true},get value(){trace=trace*10+7;return 42},get writable(){trace=trace*10+8;return true}};
                var inner=new Proxy({x:42},{getOwnPropertyDescriptor(){trace=trace*10+1;return undefined},isExtensible(){trace=trace*10+2;return true}});
                var target=new Proxy(inner,{getOwnPropertyDescriptor(){trace=trace*10+3;return d}});
                new Proxy(target,{get(){trace=trace*10+4;return 42}})"#,
                43125678,
                0,
                0,
            ),
            (
                r#"var trace=0,hasCount=0,getCount=0;
                var d=new Proxy({},{has(t,k){hasCount++;return k==='value'||k==='configurable'||k==='writable'},get(t,k){getCount++;return k==='value'?42:true}});
                var target=new Proxy({x:42},{getOwnPropertyDescriptor(){return d}});
                new Proxy(target,{get(){return 42}})"#,
                0,
                6,
                3,
            ),
            (
                r#"var trace=0,hasCount=0,getCount=0,token={};
                var d=new Proxy({},{has(t,k){hasCount++;if(k==='value'||k==='writable')return false;throw token},get(t,k){getCount++;return k==='configurable'?true:undefined}});
                var target=new Proxy({},{getOwnPropertyDescriptor(){return d}});
                new Proxy(target,{get(){return 42}})"#,
                0,
                6,
                4,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let proxy = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){return o.x})",
                vec![proxy],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(
                    execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                    Completion::Return(Value::Int(42))
                ),
                "{setup}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            drop(profile);
            assert_eq!(context.eval("trace").unwrap(), Value::Int(expected_trace));
            assert_eq!(context.eval("hasCount").unwrap(), Value::Int(expected_has));
            assert_eq!(context.eval("getCount").unwrap(), Value::Int(expected_get));
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn proxy_descriptor_conversion_preserves_or_replaces_throw_in_reading_realm() {
        for (field, expected_message) in [
            ("value", None),
            ("get", Some("invalid getter")),
            ("set", Some("invalid setter")),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let mut foreign = runtime.new_context();
            let proxy = foreign.eval(&format!("var hits=0,token={{}};var d={{get {field}(){{hits++;throw token}}}};var target=new Proxy({{}},{{getOwnPropertyDescriptor(){{return d}}}});new Proxy(target,{{get(){{return 42}}}})")).unwrap();
            let token = foreign.eval("token").unwrap();
            let error_prototype = context.eval("TypeError.prototype").unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o){try{return o.x}catch(e){return e}})",
                vec![proxy],
            );
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("throw escaped parent catch");
            };
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0);
            assert_eq!(cost.owned_bridge_exits, 0);
            assert_eq!(cost.owned_sync_call_bridges, 0);
            drop(profile);
            if let Some(message) = expected_message {
                let Value::Object(error) = value else {
                    panic!("missing descriptor TypeError");
                };
                assert_eq!(
                    runtime.get_prototype_of(&error).unwrap().map(Value::Object),
                    Some(error_prototype)
                );
                assert_eq!(
                    context
                        .get_property(&error, &runtime.intern_property_key("message").unwrap())
                        .unwrap(),
                    Value::String(crate::engine::value::JsString::from_static(message))
                );
            } else {
                assert_eq!(value, token);
            }
            assert_eq!(foreign.eval("hits").unwrap(), Value::Int(1));
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn proxy_descriptor_checks_extensibility_before_touching_the_result() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let proxy = context.eval("var reads=0,extensibility=0,token={};var d={get value(){reads++;return 42}};var inner=new Proxy({}, {isExtensible(){extensibility++;throw token}});var target=new Proxy(inner,{getOwnPropertyDescriptor(){return d}});new Proxy(target,{get(){return 42}})").unwrap();
        let token = context.eval("token").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(o){return o.x})",
            vec![proxy],
        );
        let profile = CostProfile::start();
        let Completion::Throw(value) =
            execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
        else {
            panic!("expected extensibility throw");
        };
        assert_eq!(value, token);
        let cost = profile.snapshot();
        assert_eq!(cost.legacy_dispatches, 0);
        assert_eq!(cost.owned_bridge_exits, 0);
        assert_eq!(cost.owned_sync_call_bridges, 0);
        drop(profile);
        assert_eq!(context.eval("reads").unwrap(), Value::Int(0));
        assert_eq!(context.eval("extensibility").unwrap(), Value::Int(1));
        assert_eq!(runtime.0.proxy_method_depth.get(), 0);
    }

    #[test]
    fn primitive_property_receivers_and_string_units_use_owned_reads() {
        for (setup, source, input, expected) in [
            (
                "Object.defineProperty(Number.prototype,'x',{get:function(){'use strict';return this===42}})",
                "(function(o){return o.x})",
                Value::Int(42),
                Value::Bool(true),
            ),
            (
                "Object.defineProperty(String.prototype,'x',{get:function(){'use strict';return this==='hi'}})",
                "(function(o){return o.x})",
                Value::String(crate::engine::value::JsString::from_static("hi")),
                Value::Bool(true),
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context.eval(setup).unwrap();
            let entry = entry(&runtime, &mut context, source, vec![input]);
            let profile = CostProfile::start();
            let Completion::Return(value) =
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
            else {
                panic!("primitive read threw");
            };
            assert_eq!(value, expected);
            assert_eq!(profile.snapshot().legacy_dispatches, 0);
            assert_eq!(profile.snapshot().owned_bridge_exits, 0);
            assert_eq!(profile.snapshot().owned_sync_call_bridges, 0);
        }
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(s){return s[0]+':'+s.length})",
            vec![Value::String(crate::engine::value::JsString::from_static(
                "hi",
            ))],
        );
        let profile = CostProfile::start();
        assert!(
            matches!(execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(), Completion::Return(Value::String(value)) if value==crate::engine::value::JsString::from_static("h:2"))
        );
        assert_eq!(profile.snapshot().owned_storage.frames_pushed, 1);
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert_eq!(profile.snapshot().owned_sync_call_bridges, 0);
    }

    #[test]
    fn object_key_updates_keep_one_conversion_across_owned_getter_and_setter() {
        for selected in ["0", "Symbol.iterator"] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context.eval(&format!("var keys=0,getters=0,written=0;var selected={selected};var target={{}};Object.defineProperty(target,selected,{{get:function(){{getters++;return 41}},set:function(v){{written=v}}}});var key={{toString(){{keys++;return selected}}}};")).unwrap();
            let target = context.eval("target").unwrap();
            let key = context.eval("key").unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(o,k){return o[k]++})",
                vec![target, key],
            );
            let profile = CostProfile::start();
            assert!(matches!(
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                Completion::Return(Value::Int(41))
            ));
            assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, 2);
            assert_eq!(profile.snapshot().owned_bridge_exits, 0);
            assert_eq!(profile.snapshot().legacy_dispatches, 0);
            assert_eq!(profile.snapshot().owned_sync_call_bridges, 0);
            drop(profile);
            assert_eq!(
                context.eval("keys*100+getters*10+written").unwrap(),
                Value::Int(152)
            );
        }
    }

    #[test]
    fn object_key_conversion_failure_uses_the_reading_realm() {
        let runtime = Runtime::new();
        let mut caller = runtime.new_context();
        let mut foreign = runtime.new_context();
        let Value::Object(expected) = caller.eval("TypeError.prototype").unwrap() else {
            panic!("missing error prototype");
        };
        let key = foreign
            .eval("({toString(){return {}},valueOf(){return {}}})")
            .unwrap();
        let target = caller.eval("({})").unwrap();
        let entry = entry(
            &runtime,
            &mut caller,
            "(function(o,k){return o[k]})",
            vec![target, key],
        );
        let profile = CostProfile::start();
        let Completion::Throw(Value::Object(error)) =
            execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap()
        else {
            panic!("key unexpectedly succeeded");
        };
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert_eq!(profile.snapshot().owned_sync_call_bridges, 0);
        drop(profile);
        assert_eq!(runtime.get_prototype_of(&error).unwrap(), Some(expected));
        assert_eq!(
            caller
                .get_property(&error, &runtime.intern_property_key("message").unwrap())
                .unwrap(),
            Value::String(crate::engine::value::JsString::from_static("toPrimitive"))
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn owned_reference_write_rechecks_typed_array_prototype_presence_after_rhs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callable = context.eval("(function(o){with(o){return function(){'use strict';NaN=(delete o.NaN,0)}}})(Object.defineProperty(Object.create(new Int32Array(1)),'NaN',{value:100,configurable:true}))").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(f){try{f()}catch(e){return e.name==='ReferenceError'?42:0}})",
            vec![callable],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(
            matches!(result, Completion::Return(Value::Int(42))),
            "{result:?}"
        );
        let cost = profile.snapshot();
        assert_eq!(cost.legacy_dispatches, 0);
        assert_eq!(cost.owned_bridge_exits, 0);
        assert_eq!(cost.owned_sync_call_bridges, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn owned_in_and_delete_preserve_conversion_order_and_proxy_invariants() {
        for (setup, body, trace) in [
            (
                "var trace=0;var key={toString(){trace++;return 'x'}};({x:42})",
                "return key in o?42:0",
                1,
            ),
            (
                "var trace=0;var key={toString(){trace++;return 'x'}};null",
                r#"try{return key in o}catch(e){return e.message==="invalid 'in' operand"?42:0}"#,
                0,
            ),
            (
                "var trace=0;var token={};var key={toString(){trace++;throw token}};null",
                "try{delete o[key]}catch(e){return e===token?42:0}",
                1,
            ),
            (
                "var trace=0;var key={toString(){trace++;return 'x'}};null",
                "try{delete o[key]}catch(e){return e.message==='cannot convert to object'?42:0}",
                1,
            ),
            (
                "var trace=0;var key={toString(){trace++;return '0'}};'abc'",
                "return delete o[key]?0:42",
                1,
            ),
            (
                "var trace=0;var key={toString(){trace++;return '0'}};'abc'",
                "'use strict';try{delete o[key]}catch(e){return e.message==='could not delete property'?42:0}",
                1,
            ),
            (
                "var trace=0;var key='x';new Proxy({x:1},{get deleteProperty(){trace=trace*10+1;return function(t,k){trace=trace*10+2;return delete t[k]}}})",
                "return delete o[key]?42:0",
                12,
            ),
            (
                "var trace=0;var key='x';new Proxy({x:1},{deleteProperty(){trace++;return false}})",
                "return delete o[key]?0:42",
                1,
            ),
            (
                "var trace=0;var key='x';new Proxy({x:1},{deleteProperty(){trace++;return false}})",
                "'use strict';try{delete o[key]}catch(e){return e.message==='could not delete property'?42:0}",
                1,
            ),
            (
                "var trace=0;var key='x';new Proxy(Object.defineProperty({},'x',{value:1}),{deleteProperty(){trace++;return true}})",
                "try{delete o[key]}catch(e){return e.name==='TypeError'?42:0}",
                1,
            ),
            (
                "var trace=0;var key='x';var t=new Proxy({x:1},{getOwnPropertyDescriptor(){trace=trace*10+2;return {value:1,writable:true,enumerable:true,configurable:true}},isExtensible(){trace=trace*10+3;return true}});new Proxy(t,{deleteProperty(){trace=trace*10+1;return true}})",
                "return delete o[key]?42:0",
                123,
            ),
            (
                "var trace=0;var key='x';var t=new Proxy(Object.preventExtensions({x:1}),{isExtensible(){trace=trace*10+2;return false}});new Proxy(t,{deleteProperty(){trace=trace*10+1;return true}})",
                "try{delete o[key]}catch(e){return e.name==='TypeError'?42:0}",
                12,
            ),
            (
                "var trace=0;var key='x';var t=new Proxy({}, {isExtensible(){trace=99;throw 1}});new Proxy(t,{deleteProperty(){trace++;return true}})",
                "return delete o[key]?42:0",
                1,
            ),
            (
                "var trace=0;var key='x';new Proxy({x:42},{get has(){trace=trace*10+1;return function(t,k){trace=trace*10+2;return k in t}}})",
                "return key in o?42:0",
                12,
            ),
            (
                "var trace=0;var key='x';new Proxy(Object.defineProperty({},'x',{value:1}),{has(){trace++;return false}})",
                "try{return key in o}catch(e){return e.name==='TypeError'?42:0}",
                1,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{setup}: {result:?}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            drop(profile);
            assert_eq!(context.eval("trace").unwrap(), Value::Int(trace), "{setup}");
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_prevent_extensions_checks_nested_target_after_trap() {
        use crate::engine::object::ProxyBooleanKind;
        for (setup, expected, trace) in [
            (
                "var trace=0;new Proxy({}, {preventExtensions(){trace++;return false}})",
                Some(false),
                1,
            ),
            (
                "var trace=0;new Proxy(Object.preventExtensions({}), {preventExtensions(){trace++;return true}})",
                Some(true),
                1,
            ),
            (
                "var trace=0;new Proxy({}, {preventExtensions(){trace++;return true}})",
                None,
                1,
            ),
            (
                "var trace=0;var t=new Proxy(Object.preventExtensions({}),{isExtensible(){trace=trace*10+2;return false}});new Proxy(t,{preventExtensions(){trace=trace*10+1;return true}})",
                Some(true),
                12,
            ),
            (
                "var trace=0;var t=new Proxy({}, {preventExtensions(){trace=trace*10+2;return false}});new Proxy(t,{get preventExtensions(){trace=trace*10+1;return undefined}})",
                Some(false),
                12,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let Value::Object(object) = context.eval(setup).unwrap() else {
                panic!("expected Proxy")
            };
            let entry = entry(&runtime, &mut context, "(function(){return false})", vec![]);
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let profile = CostProfile::start();
            let result = match super::super::proxy_get_driver::start_boolean(
                &runtime,
                &mut execution,
                id,
                object,
                ProxyBooleanKind::PreventExtensions,
                false,
                0,
            )
            .unwrap()
            {
                CallStep::Entered => super::run_frames(&runtime, execution)
                    .unwrap()
                    .finish(runtime.clone())
                    .unwrap(),
                CallStep::Complete(result) => result,
                CallStep::Bridge => panic!("preventExtensions attempted handoff"),
            };
            match expected {
                Some(expected) => assert!(
                    matches!(result, Completion::Return(Value::Bool(value)) if value == expected),
                    "{setup}: {result:?}"
                ),
                None => assert!(
                    matches!(result, Completion::Throw(_)),
                    "{setup}: {result:?}"
                ),
            }
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            drop(profile);
            assert_eq!(context.eval("trace").unwrap(), Value::Int(trace), "{setup}");
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_native_prototype_entries_drive_nested_callbacks_and_bound_calls() {
        for (setup, body, trace) in [
            (
                "var trace=0;var p={};var target=new Proxy({}, {getPrototypeOf(){trace++;return p}});Object.defineProperty({},'x',{get:Object.getPrototypeOf.bind(undefined,target)})",
                "return o.x===p?42:0",
                1,
            ),
            (
                "var trace=0;var target=new Proxy({}, {getPrototypeOf(){trace++;return null}});({valueOf:Object.getPrototypeOf.bind(undefined,target)})",
                "return +o===0?42:0",
                1,
            ),
            (
                "var trace=0;Object.create(null)",
                "return Object.getPrototypeOf(o)===null?42:0",
                0,
            ),
            (
                "var trace=0;42",
                "return Object.getPrototypeOf(o)===Number.prototype?42:0",
                0,
            ),
            ("var trace=0;42", "return Object.setPrototypeOf(o,null)", 0),
            (
                "var trace=0;42",
                "try{Object.setPrototypeOf(o,1)}catch(e){return e.message==='not an object'?42:0}",
                0,
            ),
            (
                "var trace=0;42",
                "try{Reflect.getPrototypeOf(o)}catch(e){return e.message==='not an object'?42:0}",
                0,
            ),
            (
                "var trace=0;new Proxy({}, {setPrototypeOf(){trace++;return false}})",
                "return Reflect.setPrototypeOf(o,null)===false?42:0",
                1,
            ),
            (
                "var trace=0;new Proxy({}, {setPrototypeOf(){trace++;return false}})",
                "try{Object.setPrototypeOf(o,null)}catch(e){return e.message==='proxy: bad prototype'?42:0}",
                1,
            ),
            (
                "var trace=0;var p={};var t=new Proxy({}, {getPrototypeOf(){trace++;return p}});new Proxy(t,{getPrototypeOf:Reflect.getPrototypeOf})",
                "return Object.getPrototypeOf(o)===p?42:0",
                1,
            ),
            (
                "var trace=0;var p={};var t=new Proxy({}, {getPrototypeOf(){trace++;return p}});new Proxy(t,{get getPrototypeOf(){trace=trace*10+1;return Reflect.getPrototypeOf}})",
                "return Object.getPrototypeOf(o)===p?42:0",
                2,
            ),
            (
                "var trace=0;var p={};var t=new Proxy({}, {getPrototypeOf(){trace++;return p}});new Proxy({}, {getPrototypeOf(){trace=trace*10+1;return Object.getPrototypeOf(t)}})",
                "return Reflect.getPrototypeOf(o)===p?42:0",
                2,
            ),
            (
                "var trace=0;var t=new Proxy({}, {setPrototypeOf(t,p){trace++;return Reflect.setPrototypeOf(t,p)}});new Proxy(t,{setPrototypeOf:Reflect.setPrototypeOf})",
                "return Object.setPrototypeOf(o,null)===o?42:0",
                1,
            ),
            (
                "var trace=0;var p={};var o=new Proxy({}, {getPrototypeOf(){trace++;return p}});Object.getPrototypeOf.bind(undefined,o)",
                "return o()===p?42:0",
                1,
            ),
            (
                "var trace=0;var n=64;var o=new Proxy({}, {getPrototypeOf(){trace++;if(n--===0)return null;return Object.getPrototypeOf(o)}});o",
                "return Object.getPrototypeOf(o)===null?42:0",
                65,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{setup}: {result:?}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            drop(profile);
            assert_eq!(context.eval("trace").unwrap(), Value::Int(trace), "{setup}");
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_native_properties_preserve_order_receivers_and_throw_identity() {
        for (setup, body) in [
            (
                "(function(a,b){return this.x+a+b})",
                "return o.call({x:20},10,12)",
            ),
            (
                "(function(){return arguments.length})",
                "return o.call()===0?42:0",
            ),
            (
                "(function(a,b){return this.x+a+b})",
                "return o.apply({x:20},{get length(){return {valueOf(){return 2}}},get 0(){return 10},get 1(){return 12}})",
            ),
            (
                "(function(){return arguments.length})",
                "return o.apply(null,null)===0?42:0",
            ),
            (
                "(function(a,b){return a+b})",
                "return Reflect.apply(o,null,new Proxy({length:2,0:20,1:22},{get(t,k){return t[k]}}))",
            ),
            (
                "new Proxy(function(x){return x},{apply(t,r,a){return a[0]+2}})",
                "return o.call(null,40)",
            ),
            (
                "var trace=0;({get length(){trace++;return 1},get 0(){throw 42}})",
                "try{Reflect.apply(function(){},null,o)}catch(e){return trace===1?e:0}",
            ),
            (
                "var trace=0;({get length(){trace++;return 1}})",
                "try{Function.prototype.apply.call(1,null,o)}catch(e){return trace===0?42:0}",
            ),
            (
                "var trace=0;({get length(){trace++;return 65535},get 0(){trace+=10}})",
                "try{Reflect.apply(function(){},null,o)}catch(e){return trace===1?42:0}",
            ),
            (
                "({})",
                "o.__defineGetter__({toString(){return 'x'}},function(){return 42});return o.x",
            ),
            (
                "new Proxy({}, {defineProperty(t,k,d){return Reflect.defineProperty(t,k,d)}})",
                "o.__defineSetter__('x',function(v){this.y=v});o.x=42;return o.y",
            ),
            (
                "var getter=function(){return 42};Object.create({get x(){return 1},y:1})",
                "o.__defineGetter__('x',getter);return o.__lookupGetter__('x')===getter?42:0",
            ),
            (
                "var getter=function(){return 42};var p=Object.defineProperty({},'x',{get:getter});new Proxy(Object.create(p),{getOwnPropertyDescriptor(t,k){return Reflect.getOwnPropertyDescriptor(t,k)},getPrototypeOf(t){return Object.getPrototypeOf(t)}})",
                "return o.__lookupGetter__({toString(){return 'x'}})===getter?42:0",
            ),
            (
                "var trace=0;({toString(){trace++;return 'x'}})",
                "try{({}).__defineGetter__(o,1)}catch(e){return trace===0?42:0}",
            ),
            (
                "({tag:Object.prototype.toString,get [Symbol.toStringTag](){return 'owned'}})",
                "return o.tag()==='[object owned]'?42:0",
            ),
            (
                "new Proxy({}, {get(t,k){if(k===Symbol.toStringTag)return 'proxy';return Object.prototype.toString}})",
                "return o.tag()==='[object proxy]'?42:0",
            ),
            (
                "var calls=0;Object.defineProperty(Number.prototype,Symbol.toStringTag,{get(){calls++;return this===1?'bad':'boxed'},configurable:true});Object.prototype.toString.bind(1)",
                "return o()==='[object boxed]'&&calls===1?42:0",
            ),
            (
                "({locale:Object.prototype.toLocaleString,get toString(){return function(){return this.x}},x:42})",
                "return o.locale()",
            ),
            (
                "Number.prototype.toString=function(){'use strict';return this===1?42:0};Object.prototype.toLocaleString.bind(1)",
                "return o()",
            ),
            (
                "Object.prototype.toString.bind(null)",
                "return o()==='[object Null]'?42:0",
            ),
            (
                "var r=Proxy.revocable({},{});var f=Object.prototype.toString.bind(r.proxy);r.revoke();f",
                "try{o()}catch(e){return e.name==='TypeError'?42:0}",
            ),
            (
                "({})",
                "return Object.is(NaN,NaN)&&!Object.is(0,-0)&&o.valueOf()===o?42:0",
            ),
            (
                "var proto={};new Proxy({}, {getPrototypeOf(){return proto}})",
                "return o.__proto__===proto?42:0",
            ),
            (
                "var proto={};new Proxy({}, {setPrototypeOf(t,p){return Reflect.setPrototypeOf(t,p)}})",
                "o.__proto__=proto;return Object.getPrototypeOf(o)===proto?42:0",
            ),
            (
                "var proto={};var o=new Proxy({}, {getPrototypeOf(){return proto}});({check:Object.prototype.isPrototypeOf,proto:proto,o:o})",
                "o.proto.check=o.check;return o.proto.check(o.o)?42:0",
            ),
            (
                "Object.prototype.isPrototypeOf.bind(null,1)",
                "return o()===false?42:0",
            ),
            (
                "({})",
                "try{Object.defineProperties(o,{x:{value:42},get y(){throw o}})}catch(e){return e===o?o.x:0}",
            ),
            (
                "var trace=0;new Proxy({x:{value:42},y:{value:1}},{ownKeys(t){return ['x','y']},getOwnPropertyDescriptor(t,k){trace=trace*10+(k==='x'?1:2);return Reflect.getOwnPropertyDescriptor(t,k)},get(t,k){trace=trace*10+(k==='x'?3:4);return t[k]}})",
                "var t=Object.defineProperties({},o);return trace===1234?t.x:0",
            ),
            (
                "({get x(){return {get value(){return 42}}}})",
                "return Object.create(null,o).x",
            ),
            (
                "({})",
                "return Object.getPrototypeOf(Object.create(o))===o?42:0",
            ),
            (
                "({get x(){throw 42}})",
                "try{Object.create(0,o)}catch(e){return e.message==='not a prototype'?42:0}",
            ),
            (
                "new Proxy({}, {getOwnPropertyDescriptor(t,k){return {value:1,enumerable:true,configurable:true}}})",
                "return Object.hasOwn(o,{toString(){return 'x'}})?42:0",
            ),
            (
                "({x:42,check:Object.prototype.hasOwnProperty})",
                "return o.check({toString(){return 'x'}})?42:0",
            ),
            (
                "({x:42,check:Object.prototype.propertyIsEnumerable})",
                "return o.check({toString(){return 'x'}})?42:0",
            ),
            (
                "var calls=0;var marker={};Object.prototype.hasOwnProperty.bind(null,{toString(){calls++;throw marker}})",
                "try{o()}catch(e){return e===marker&&calls===1?42:0}",
            ),
            (
                "var calls=0;var marker={};Object.prototype.propertyIsEnumerable.bind(null,{toString(){calls++;throw marker}})",
                "try{o()}catch(e){return e===marker&&calls===1?42:0}",
            ),
            (
                "({toString(){throw 99}})",
                "try{Object.hasOwn(null,o)}catch(e){return e===99?0:42}",
            ),
            ("'x'", "return Object.hasOwn(o,'length')?42:0"),
            (
                "({get x(){Object.defineProperty(this,'y',{enumerable:false});return 40},y:2})",
                "var t=Object.assign({},null,o);return t.x+t.y",
            ),
            (
                "new Proxy({get x(){Object.defineProperty(this,'y',{enumerable:false});return 42},y:2},{ownKeys(t){return Reflect.ownKeys(t)},getOwnPropertyDescriptor(t,k){return Reflect.getOwnPropertyDescriptor(t,k)}})",
                "var t=Object.assign({},o);return t.y===undefined?t.x:0",
            ),
            (
                "({set x(v){this.y=v}})",
                "return Object.assign(o,{get x(){return 42}}).y",
            ),
            (
                "({})",
                "var t={};try{Object.assign(t,{get x(){return 42},get y(){throw o}})}catch(e){return e===o?t.x:0}",
            ),
            (
                "new Proxy({x:42},{preventExtensions(t){return Reflect.preventExtensions(t)},ownKeys(t){return Reflect.ownKeys(t)},getOwnPropertyDescriptor(t,k){return Reflect.getOwnPropertyDescriptor(t,k)},defineProperty(t,k,d){return Reflect.defineProperty(t,k,d)}})",
                "return Object.freeze(o)===o&&Object.isFrozen(o)&&Object.isSealed(o)?o.x:0",
            ),
            (
                "({x:42})",
                "Object.seal(o);return Object.isSealed(o)&&!Object.isFrozen(o)?o.x:0",
            ),
            (
                "var trace=0;new Proxy({x:1},{getOwnPropertyDescriptor(t,k){trace++;return Reflect.getOwnPropertyDescriptor(t,k)},isExtensible(){trace+=100;return true}})",
                "return !Object.isFrozen(o)&&trace===1?42:0",
            ),
            (
                "new Proxy({}, {preventExtensions(){throw 42}})",
                "try{Object.seal(o)}catch(e){return e}",
            ),
            (
                "({get x(){delete this.y;return 42},y:1})",
                "var a=Object.values(o);return a.length===1?a[0]:0",
            ),
            (
                "({get x(){Object.defineProperty(this,'y',{enumerable:false});return 42},y:1})",
                "var a=Object.entries(o);return a.length===1&&a[0][0]==='x'?a[0][1]:0",
            ),
            (
                "var symbol=Symbol('k');({x:1,[symbol]:42})",
                "return Object.getOwnPropertyNames(o)[0]==='x'&&Object.getOwnPropertySymbols(o)[0]===symbol?42:0",
            ),
            (
                "var symbol=Symbol('k');new Proxy({x:1,[symbol]:42},{getOwnPropertyDescriptor(t,k){return Reflect.getOwnPropertyDescriptor(t,k)},ownKeys(t){return Reflect.ownKeys(t)}})",
                "return Object.getOwnPropertyDescriptors(o)[symbol].value",
            ),
            (
                "new Proxy({x:1,y:2},{ownKeys(){return ['y','x']},getOwnPropertyDescriptor(t,k){return {enumerable:k==='x',configurable:true}}})",
                "var a=Object.keys(o);return a.length===1&&a[0]==='x'?42:0",
            ),
            (
                "({get x(){throw 42}})",
                "try{Object.entries(o)}catch(e){return e}",
            ),
            (
                "new Proxy({x:42},{ownKeys(){return {get length(){return {valueOf(){return 1}}},get 0(){return 'x'}}}})",
                "return Reflect.ownKeys(o)[0]==='x'?42:0",
            ),
            (
                "new Proxy(new Proxy({x:1},{ownKeys(t){return Reflect.ownKeys(t)},getOwnPropertyDescriptor(t,k){return Reflect.getOwnPropertyDescriptor(t,k)}}),{ownKeys(){return ['x']}})",
                "return Reflect.ownKeys(o)[0]==='x'?42:0",
            ),
            (
                "new Proxy({}, {ownKeys(){return 'ab'}})",
                "var a=Reflect.ownKeys(o);return a.length===2&&a[0]==='a'&&a[1]==='b'?42:0",
            ),
            (
                "new Proxy({}, {ownKeys(){return null}})",
                r#"try{Reflect.ownKeys(o)}catch(e){return e.message==="cannot read property 'length' of null"?42:0}"#,
            ),
            (
                "var trace=0;new Proxy({}, {ownKeys(){return {length:3,get 0(){trace=trace*10+1;return 'a'},get 1(){trace=trace*10+2;return 'a'},get 2(){trace=trace*10+3;return 'b'}}}})",
                "try{Reflect.ownKeys(o)}catch(e){return e.message==='proxy: duplicate property'&&trace===123?42:0}",
            ),
            (
                "var trace=0;new Proxy(new Proxy(Object.preventExtensions({x:1}),{isExtensible(t){trace=trace*10+1;return false},ownKeys(t){trace=trace*10+2;return ['x']},getOwnPropertyDescriptor(t,k){trace=trace*10+3;return Reflect.getOwnPropertyDescriptor(t,k)}}),{ownKeys(){return []}})",
                "try{Reflect.ownKeys(o)}catch(e){return e.message==='proxy: target property must be present in proxy ownKeys'&&trace===123?42:0}",
            ),
            (
                "new Proxy(Object.preventExtensions({}),{ownKeys(){return ['x']}})",
                "try{Reflect.ownKeys(o)}catch(e){return e.message==='proxy: property not present in target were returned by non extensible proxy'?42:0}",
            ),
            (
                "new Proxy({}, {ownKeys(){return {get length(){throw 42}}}})",
                "try{Reflect.ownKeys(o)}catch(e){return e}",
            ),
            (
                "({get x(){return this.y}})",
                "return Reflect.get(o,{toString(){return 'x'}},{y:42})",
            ),
            (
                "({set x(v){this.y=v}})",
                "var r={};return Reflect.set(o,'x',42,r)&&r.y",
            ),
            (
                "new Proxy({}, {get(t,k,r){return Reflect.get({get x(){return this.y}},k,r)}})",
                "return Reflect.get(o,'x',{y:42})",
            ),
            (
                "new Proxy({}, {defineProperty(t,k,d){return Reflect.defineProperty(t,k,d)}})",
                "return Reflect.defineProperty(o,{toString(){return 'x'}},{get value(){return 42}})&&o.x",
            ),
            (
                "({})",
                "return Object.defineProperty(o,'x',{get value(){return 42},get writable(){return true}}).x",
            ),
            (
                "new Proxy({}, {getOwnPropertyDescriptor(){return {get value(){return 42},configurable:true}}})",
                "return Reflect.getOwnPropertyDescriptor(o,'x').value",
            ),
            (
                "'abc'",
                "return Object.getOwnPropertyDescriptor(o,'length').value===3?42:0",
            ),
            (
                "new Proxy({}, {has(t,k){return k==='x'}})",
                "return Reflect.has(o,{toString(){return 'x'}})?42:0",
            ),
            (
                "new Proxy({x:1}, {deleteProperty(t,k){return Reflect.deleteProperty(t,k)}})",
                "return Reflect.deleteProperty(o,'x')&&!Reflect.has(o,'x')?42:0",
            ),
            (
                "new Proxy({}, {isExtensible(t){return Reflect.isExtensible(t)}})",
                "return Object.isExtensible(o)?42:0",
            ),
            (
                "new Proxy({}, {preventExtensions(t){return Reflect.preventExtensions(t)}})",
                "return Object.preventExtensions(o)===o&&!Reflect.isExtensible(o)?42:0",
            ),
            (
                "new Proxy({}, {preventExtensions(){return false}})",
                "if(Reflect.preventExtensions(o))return 0;try{Object.preventExtensions(o)}catch(e){return e.message==='proxy preventExtensions handler returned false'?42:0}",
            ),
            (
                "new Proxy({}, {defineProperty(){return false}})",
                "if(Reflect.defineProperty(o,'x',{}))return 0;try{Object.defineProperty(o,'x',{})}catch(e){return e.message==='proxy: defineProperty exception'?42:0}",
            ),
            (
                "({})",
                "var n=0,k={toString(){n++;throw o}};try{Reflect.get(0,k)}catch(e){}if(n)return 0;try{Reflect.get({},k)}catch(e){return e===o&&n===1?42:0}",
            ),
            (
                "({})",
                "var n=0;try{Object.defineProperty({}, {toString(){n++;throw o}}, {get value(){n+=10}})}catch(e){return e===o&&n===1?42:0}",
            ),
            (
                "({})",
                "var n=0;try{Reflect.defineProperty({},'x',{get enumerable(){n++;throw o},get value(){n+=10}})}catch(e){return e===o&&n===1?42:0}",
            ),
            (
                "({})",
                "return Object.preventExtensions(42)===42&&!Object.isExtensible(null)?42:0",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{body}: {result:?}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{body}");
            assert_eq!(cost.owned_bridge_exits, 0, "{body}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{body}");
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_native_enumeration_uses_vm_limits_without_native_family_charges() {
        for (frames, depth) in [(256, 64), (32, 64)] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(depth){function go(n){if(n===0)return 42;return Object.values({get x(){return go(n-1)}})[0]}try{return go(depth)}catch(e){return e.message==='stack overflow'?43:0}})",
                vec![Value::Int(depth)],
            );
            let profile = CostProfile::start();
            let result = execute(
                runtime.clone(),
                entry,
                ExecutionLimits {
                    frames,
                    ..ExecutionLimits::default()
                },
            )
            .unwrap();
            let expected = if frames == 256 { 42 } else { 43 };
            assert!(
                matches!(result, Completion::Return(Value::Int(value)) if value == expected),
                "{result:?}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0);
            assert_eq!(cost.owned_bridge_exits, 0);
            assert_eq!(cost.owned_sync_call_bridges, 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
        }
    }

    #[test]
    fn owned_native_prototype_recursion_uses_existing_frame_budget_and_recovers() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = context.eval("var calls=0;var p=new Proxy({}, {getPrototypeOf(){calls++;return Object.getPrototypeOf(p)}});p").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function(p){try{Object.getPrototypeOf(p)}catch(e){return e.message==='stack overflow'?42:0}})",
            vec![object],
        );
        let profile = CostProfile::start();
        let result = execute(
            runtime.clone(),
            entry,
            ExecutionLimits {
                frames: 32,
                ..ExecutionLimits::default()
            },
        )
        .unwrap();
        assert!(
            matches!(result, Completion::Return(Value::Int(42))),
            "{result:?}"
        );
        let cost = profile.snapshot();
        assert_eq!(cost.legacy_dispatches, 0);
        assert_eq!(cost.owned_bridge_exits, 0);
        assert_eq!(cost.owned_sync_call_bridges, 0);
        drop(profile);
        assert!(
            matches!(context.eval("calls").unwrap(), Value::Int(calls) if calls > 1 && calls < 32)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        assert_eq!(runtime.0.proxy_method_depth.get(), 0);
    }

    #[test]
    fn owned_native_prototype_error_uses_defining_realm_and_keeps_native_stack() {
        let runtime = Runtime::new();
        let mut caller = runtime.new_context();
        let mut defining = runtime.new_context();
        let function = defining.eval("Object.getPrototypeOf").unwrap();
        let prototype = defining.eval("TypeError.prototype").unwrap();
        let object = caller
            .eval("new Proxy({}, {getPrototypeOf(){return 1}})")
            .unwrap();
        let entry = entry(
            &runtime,
            &mut caller,
            "(function(f,p){return f(p)})",
            vec![function, object],
        );
        drop(defining);
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        let cost = profile.snapshot();
        assert_eq!(cost.legacy_dispatches, 0);
        assert_eq!(cost.owned_bridge_exits, 0);
        assert_eq!(cost.owned_sync_call_bridges, 0);
        drop(profile);
        let Completion::Throw(Value::Object(error)) = result else {
            panic!("expected error")
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap().map(Value::Object),
            Some(prototype)
        );
        let stack = caller
            .get_property(&error, &runtime.intern_property_key("stack").unwrap())
            .unwrap();
        assert!(
            matches!(stack, Value::String(ref text) if text.to_string().contains("getPrototypeOf (native)")),
            "{stack:?}"
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn owned_native_prototype_pending_call_keeps_extra_arguments_until_abandonment() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("Object.getPrototypeOf").unwrap())
            .unwrap();
        let object = context
            .eval("new Proxy({}, {getPrototypeOf(){return null}})")
            .unwrap();
        let extra = runtime.new_object(None).unwrap();
        let extra_id = extra.object_id();
        let entry = entry(&runtime, &mut context, "(function(){return false})", vec![]);
        let mut execution = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let id = push_frame(&mut execution, entry).unwrap();
        assert!(matches!(
            super::super::proxy_get_driver::start_callback_call(
                &runtime,
                &mut execution,
                id,
                callable,
                Value::Undefined,
                vec![object, Value::Object(extra)],
                false,
                0
            )
            .unwrap(),
            CallStep::Entered
        ));
        assert_ne!(execution.frames.current_id(), Some(id));
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(extra_id).is_ok());
        drop(execution);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(extra_id).is_err());
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        assert_eq!(runtime.0.proxy_method_depth.get(), 0);
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn owned_proxy_prototypes_preserve_order_and_identity() {
        use crate::engine::object::ProxyPrototypeKind;
        // `expected` is evaluated outside the measured owned query.
        for (setup, setting, expected, trace) in [
            (
                "var trace=0;new Proxy({}, {getPrototypeOf(){trace++;return null}})",
                false,
                Some("null"),
                1,
            ),
            (
                "var trace=0;new Proxy({}, {getPrototypeOf(){trace++;return 1}})",
                false,
                None,
                1,
            ),
            (
                "var trace=0;var p={};new Proxy({}, {getPrototypeOf(){trace++;return p}})",
                false,
                Some("p"),
                1,
            ),
            (
                "var trace=0;var p={};var t=Object.preventExtensions(Object.create(p));new Proxy(new Proxy(t,{isExtensible(){trace=trace*10+2;return false},getPrototypeOf(){trace=trace*10+3;return p}}),{getPrototypeOf(){trace=trace*10+1;return p}})",
                false,
                Some("p"),
                123,
            ),
            (
                "var trace=0;var p={};var t=Object.preventExtensions(Object.create(p));new Proxy(new Proxy(t,{isExtensible(){trace=trace*10+2;return false},getPrototypeOf(){trace=trace*10+3;return p}}),{getPrototypeOf(){trace=trace*10+1;return null}})",
                false,
                None,
                123,
            ),
            (
                "var trace=0;new Proxy(new Proxy({}, {isExtensible(){trace=trace*10+2;return true}}),{getPrototypeOf(){trace=trace*10+1;return 1}})",
                false,
                None,
                1,
            ),
            (
                "var trace=0;new Proxy(new Proxy({}, {getPrototypeOf(){trace=trace*10+2;return null}}),{get getPrototypeOf(){trace=trace*10+1;return undefined}})",
                false,
                Some("null"),
                12,
            ),
            (
                "var trace=0;new Proxy(new Proxy({}, {isExtensible(){trace=trace*10+2;return true}}),{setPrototypeOf(){trace=trace*10+1;return false}})",
                true,
                Some("false"),
                1,
            ),
            (
                "var trace=0;new Proxy({}, {setPrototypeOf(t,p){trace++;return p===null}})",
                true,
                Some("true"),
                1,
            ),
            (
                "var trace=0;var t=Object.preventExtensions(Object.create(null));new Proxy(new Proxy(t,{isExtensible(){trace=trace*10+2;return false},getPrototypeOf(){trace=trace*10+3;return null}}),{setPrototypeOf(){trace=trace*10+1;return true}})",
                true,
                Some("true"),
                123,
            ),
            (
                "var trace=0;var t=Object.preventExtensions({});new Proxy(new Proxy(t,{isExtensible(){trace=trace*10+2;return false},getPrototypeOf(){trace=trace*10+3;return Object.prototype}}),{setPrototypeOf(){trace=trace*10+1;return true}})",
                true,
                None,
                123,
            ),
            (
                "var trace=0;new Proxy(new Proxy({}, {setPrototypeOf(){trace=trace*10+2;return false}}),{get setPrototypeOf(){trace=trace*10+1;return null}})",
                true,
                Some("false"),
                12,
            ),
            (
                "var trace=0;var r=Proxy.revocable({},{});r.revoke();r.proxy",
                false,
                None,
                0,
            ),
            (
                "var trace=0;var r=Proxy.revocable({},{});r.revoke();r.proxy",
                true,
                None,
                0,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let Value::Object(object) = context.eval(setup).unwrap() else {
                panic!("expected Proxy")
            };
            let expected = expected.map(|source| context.eval(source).unwrap());
            let entry = entry(&runtime, &mut context, "(function(){return false})", vec![]);
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let profile = CostProfile::start();
            let result = match super::super::proxy_get_driver::start_prototype(
                &runtime,
                &mut execution,
                id,
                object,
                if setting {
                    ProxyPrototypeKind::Set(None)
                } else {
                    ProxyPrototypeKind::Get
                },
            )
            .unwrap()
            {
                CallStep::Entered => super::run_frames(&runtime, execution)
                    .unwrap()
                    .finish(runtime.clone())
                    .unwrap(),
                CallStep::Complete(result) => {
                    drop(execution);
                    result
                }
                CallStep::Bridge => panic!("prototype query attempted handoff"),
            };
            match expected {
                Some(expected) => assert!(
                    matches!(result, Completion::Return(ref value) if value == &expected),
                    "{setup}: {result:?}"
                ),
                None => assert!(
                    matches!(result, Completion::Throw(_)),
                    "{setup}: {result:?}"
                ),
            }
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            drop(profile);
            assert_eq!(context.eval("trace").unwrap(), Value::Int(trace), "{setup}");
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_proxy_prototypes_preserve_thrown_values_at_each_request() {
        use crate::engine::object::ProxyPrototypeKind;
        for setting in [false, true] {
            for stage in ["method", "trap", "extensible", "prototype"] {
                let runtime = Runtime::new();
                let mut context = runtime.new_context();
                let name = if setting {
                    "setPrototypeOf"
                } else {
                    "getPrototypeOf"
                };
                let source = format!(
                    r#"var trace=0, sentinel={{}};
                    var t=new Proxy(Object.preventExtensions(Object.create(null)),{{
                        isExtensible(){{trace=trace*10+3;{extensible}return false}},
                        getPrototypeOf(){{trace=trace*10+4;{prototype}return null}}
                    }});
                    new Proxy(t,{{get {name}(){{trace=trace*10+1;{method}
                        return function(){{trace=trace*10+2;{trap}return {result}}}
                    }}}})"#,
                    extensible = if stage == "extensible" {
                        "throw sentinel;"
                    } else {
                        ""
                    },
                    prototype = if stage == "prototype" {
                        "throw sentinel;"
                    } else {
                        ""
                    },
                    method = if stage == "method" {
                        "throw sentinel;"
                    } else {
                        ""
                    },
                    trap = if stage == "trap" {
                        "throw sentinel;"
                    } else {
                        ""
                    },
                    result = if setting { "true" } else { "null" },
                );
                let Value::Object(object) = context.eval(&source).unwrap() else {
                    panic!("expected Proxy")
                };
                let sentinel = context.eval("sentinel").unwrap();
                let entry = entry(&runtime, &mut context, "(function(){return false})", vec![]);
                let mut execution =
                    RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
                let id = push_frame(&mut execution, entry).unwrap();
                let profile = CostProfile::start();
                let result = match super::super::proxy_get_driver::start_prototype(
                    &runtime,
                    &mut execution,
                    id,
                    object,
                    if setting {
                        ProxyPrototypeKind::Set(None)
                    } else {
                        ProxyPrototypeKind::Get
                    },
                )
                .unwrap()
                {
                    CallStep::Entered => super::run_frames(&runtime, execution)
                        .unwrap()
                        .finish(runtime.clone())
                        .unwrap(),
                    CallStep::Complete(result) => {
                        drop(execution);
                        result
                    }
                    CallStep::Bridge => panic!("prototype query attempted handoff"),
                };
                assert!(
                    matches!(result, Completion::Throw(ref value) if value == &sentinel),
                    "{source}: {result:?}"
                );
                let cost = profile.snapshot();
                assert_eq!(cost.legacy_dispatches, 0, "{source}");
                assert_eq!(cost.owned_bridge_exits, 0, "{source}");
                assert_eq!(cost.owned_sync_call_bridges, 0, "{source}");
                drop(profile);
                let trace = match stage {
                    "method" => 1,
                    "trap" => 12,
                    "extensible" => 123,
                    _ => 1234,
                };
                assert_eq!(
                    context.eval("trace").unwrap(),
                    Value::Int(trace),
                    "{source}"
                );
                assert_eq!(runtime.0.proxy_method_depth.get(), 0);
                assert!(runtime.0.state.borrow().active_frames.is_empty());
            }
        }
    }

    #[test]
    fn super_lookup_keeps_frozen_base_independent_of_getter_receiver() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let Value::Object(base) = context
            .eval("Object.defineProperty({},'x',{get:function(){return 42}})")
            .unwrap()
        else {
            panic!("expected base")
        };
        let base_id = base.object_id();
        let receiver = runtime.new_object(None).unwrap();
        let receiver_id = receiver.object_id();
        let entry = entry(&runtime, &mut context, "(function(){return 42})", vec![]);
        let mut execution = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let id = push_frame(&mut execution, entry).unwrap();
        assert!(matches!(
            super::super::proxy_get_driver::start_owned_read(
                &runtime,
                &mut execution,
                id,
                base,
                runtime.intern_property_key("x").unwrap(),
                Value::Object(receiver),
                0
            )
            .unwrap(),
            CallStep::Entered
        ));
        assert_ne!(execution.frames.current_id(), Some(id));
        runtime.run_gc().unwrap();
        for id in [base_id, receiver_id] {
            assert!(runtime.0.state.borrow().heap.object(id).is_ok());
        }
        drop(execution);
        runtime.run_gc().unwrap();
        for id in [base_id, receiver_id] {
            assert!(runtime.0.state.borrow().heap.object(id).is_err());
        }
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn owned_super_properties_keep_pinned_receivers_and_key_order() {
        for (setup, body, trace, native_calls) in [
            (
                "var trace=0;var p={get x(){trace++;return this.answer}};({__proto__:p,answer:42,read(){return super.x}})",
                "return o.read()",
                1,
                0,
            ),
            (
                "var trace=0;var p={answer:0,get x(){trace+=this===p?1:100;return function(){return this.answer}}};({__proto__:p,answer:42,read(){return super.x()}})",
                "return o.read()",
                1,
                0,
            ),
            (
                "var trace=0;var p={set x(v){trace++;this.answer=v}};({__proto__:p,write(){super.x=42;return this.answer}})",
                "return o.write()",
                1,
                0,
            ),
            (
                "var trace=0;var p={x:42};var key={toString(){trace++;Object.setPrototypeOf(o,{x:0});return 'x'}};var o={__proto__:p,read(){return super[key]}};o",
                "return o.read()",
                1,
                0,
            ),
            (
                "var trace=0;var key={toString(){trace=trace*10+2;return 'x'}};var rhs=function(){trace=trace*10+1;return 42};({__proto__:null,write(){super[key]=rhs()}})",
                "try{o.write()}catch(e){return e.message==='not an object'?42:0}",
                1,
                0,
            ),
            (
                "var trace=0;var key={toString(){trace++;return 'x'}};({__proto__:null,read(){return super[key]}})",
                r#"try{o.read()}catch(e){return e.message==="cannot read property 'x' of null"?42:0}"#,
                1,
                0,
            ),
            (
                "var trace=0;var key={toString(){trace++;return 'x'}};({__proto__:null,read(){return super[key]()}})",
                "try{o.read()}catch(e){return e.message==='cannot read property of null'?42:0}",
                0,
                0,
            ),
            (
                "var trace=0;var p=new Proxy({},{get(t,k,r){trace++;return r.answer}});({__proto__:p,answer:42,read(){return super.x}})",
                "return o.read()",
                1,
                0,
            ),
            (
                "var trace=0;var p=new Proxy({},{set(t,k,v,r){trace++;r.answer=v;return true}});({__proto__:p,answer:0,write(){super.x=42;return this.answer}})",
                "return o.write()",
                1,
                0,
            ),
            (
                "var trace=0;var key={toString(){trace=trace*10+1;return 'x'}};var p={get x(){trace=trace*10+2;return 40},set x(v){trace=trace*10+3;this.answer=v}};({__proto__:p,write(){super[key]+=2;return this.answer}})",
                "return o.write()",
                123,
                0,
            ),
            (
                "var trace=0;var token={};var p={get x(){trace++;throw token}};({__proto__:p,read(){return super.x}})",
                "try{o.read()}catch(e){return e===token?42:0}",
                1,
                0,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{setup}: {result:?}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, native_calls, "{setup}");
            drop(profile);
            assert_eq!(context.eval("trace").unwrap(), Value::Int(trace), "{setup}");
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn cold_operand_release_preserves_surviving_roots_and_commits_once() {
        for keep_top in [false, true] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(a,b){return a+b})",
                vec![],
            );
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let released = runtime.new_object(None).unwrap();
            let released_id = released.object_id();
            let frame = execution.frames.current_mut(id).unwrap();
            execution
                .slots
                .push(&mut frame.window, Value::Object(released))
                .unwrap();
            let kept_id = if keep_top {
                let kept = runtime.new_object(None).unwrap();
                let kept_id = kept.object_id();
                execution
                    .slots
                    .push(&mut frame.window, Value::Object(kept))
                    .unwrap();
                Some(kept_id)
            } else {
                None
            };
            assert!(
                !execution
                    .slots
                    .release_operand(&frame.window, usize::from(keep_top), &runtime)
                    .unwrap()
            );
            assert_eq!(
                execution.slots.depth(&frame.window),
                1 + usize::from(keep_top)
            );
            assert!(matches!(
                super::super::frame_operations::complete_owned_slot(
                    &mut execution,
                    id,
                    RunExit::ReleaseOperand { keep_top }
                )
                .unwrap(),
                true
            ));
            let frame = execution.frames.current_mut(id).unwrap();
            assert_eq!(frame.resume_pc, frame.fault_pc + 1);
            assert_eq!(execution.slots.depth(&frame.window), usize::from(keep_top));
            runtime.run_gc().unwrap();
            assert!(runtime.0.state.borrow().heap.object(released_id).is_err());
            if let Some(kept_id) = kept_id {
                assert!(runtime.0.state.borrow().heap.object(kept_id).is_ok());
            }
            drop(execution);
            runtime.run_gc().unwrap();
            if let Some(kept_id) = kept_id {
                assert!(runtime.0.state.borrow().heap.object(kept_id).is_err());
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_typed_writes_convert_once_and_reacquire_buffer_after_callbacks() {
        for (setup, body, hits, native_calls) in [
            (
                "var trace=0;var v={valueOf(){trace++;return 42}};new Uint8Array(1)",
                "o[0]=v;return o[0]",
                1,
                0,
            ),
            (
                "var trace=0;var a=new Uint8Array(1);var v={valueOf(){trace++;return 42}};new Proxy(a,{})",
                "o[0]=v;return a[0]",
                1,
                0,
            ),
            (
                "var trace=0;var v={valueOf(){trace++;return 42}};new Uint8ClampedArray(1)",
                "o[0]=v;return o[0]",
                1,
                0,
            ),
            (
                "var trace=0;var v={[Symbol.toPrimitive](h){trace++;return h==='number'?42n:0n}};new BigInt64Array(1)",
                "o[0]=v;return o[0]===42n?42:0",
                1,
                0,
            ),
            (
                "var trace=0;var v={valueOf(){trace++;return true}};new BigUint64Array(1)",
                "o[0]=v;return o[0]===1n?42:0",
                1,
                0,
            ),
            (
                "var trace=0;var v={valueOf(){trace++;return 42}};new Uint8Array(1)",
                "o['-0']=v;return o['-0']===undefined&&o[0]===0?42:0",
                1,
                0,
            ),
            (
                "var trace=0;var v={valueOf(){trace++;return 42}};new Uint8Array(1)",
                "o[99]=v;return o[99]===undefined?42:0",
                1,
                0,
            ),
            (
                "var trace=0;var v={valueOf(){trace++;return 42}};new Proxy(new Uint8Array(1),{})",
                "o[99]=v;return 42",
                0,
                0,
            ),
            (
                "var trace=0;var v={valueOf(){trace++;return 42}};new BigInt64Array(1)",
                "try{o[99]=v}catch(e){return e.message==='cannot convert to bigint'?42:0}",
                1,
                0,
            ),
            (
                "var trace=0;var token={};var v={valueOf(){trace++;throw token}};new Uint8Array(1)",
                "try{o[0]=v}catch(e){return e===token&&o[0]===0?42:0}",
                1,
                0,
            ),
            (
                "var trace=0;var b=new ArrayBuffer(1,{maxByteLength:2});var v={valueOf(){trace++;b.resize(0);return 42}};new Uint8Array(b)",
                "o[0]=v;return o[0]===undefined?42:0",
                1,
                0,
            ),
            (
                "var trace=0;var b=new ArrayBuffer(0,{maxByteLength:2});var v={valueOf(){trace++;b.resize(1);return 42}};new Uint8Array(b)",
                "o[0]=v;return o[0]",
                1,
                0,
            ),
            (
                "var trace=0;var b=new ArrayBuffer(1);var v={valueOf(){trace++;b.transfer();return 42}};new Uint8Array(b)",
                "o[0]=v;return o[0]===undefined?42:0",
                1,
                0,
            ),
            (
                "var trace=0;var b=new ArrayBuffer(1);var a=new Uint8Array(b);var v={valueOf(){trace++;b.transfer();return 42}};new Proxy(a,{})",
                "'use strict';o[0]=v;return a[0]===undefined?42:0",
                1,
                0,
            ),
            (
                "var trace=0;var b=new SharedArrayBuffer(1);var v={valueOf(){trace++;return 42}};new Uint8Array(b)",
                "o[0]=v;return o[0]",
                1,
                0,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{setup}: {result:?}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, native_calls, "{setup}");
            drop(profile);
            assert_eq!(context.eval("trace").unwrap(), Value::Int(hits), "{setup}");
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_array_length_runs_both_conversions_and_preserves_write_failures() {
        for (setup, body, expected_trace, native_calls) in [
            (
                "var trace=0;var a=[1,2,3];var second=function(){trace=trace*10+2;return 1};var v={valueOf(){trace=trace*10+1;this.valueOf=second;return 1}};a",
                "o.length=v;return o.length===1&&o[1]===undefined?42:0",
                12,
                0,
            ),
            (
                "var trace=0;var a=[1,2,3];var v={valueOf(){trace++;a.length=5;return 2}};new Proxy(a,{})",
                "o.length=v;return a.length===2&&a[2]===undefined?42:0",
                2,
                0,
            ),
            (
                "var trace=0;var token={};var v={valueOf(){trace++;if(trace===2)throw token;return 1}};[1,2,3]",
                "try{o.length=v}catch(e){return e===token&&o.length===3?42:0}",
                2,
                0,
            ),
            (
                "var trace=0;var token={};var v={get valueOf(){trace++;throw token}};[1,2,3]",
                "try{o.length=v}catch(e){return e===token&&o.length===3?42:0}",
                1,
                0,
            ),
            (
                "var trace=0;var v={[Symbol.toPrimitive](hint){trace++;return hint==='number'?1:99}};[1,2,3]",
                "o.length=v;return o.length===1?42:0",
                2,
                0,
            ),
            (
                "var trace=0;var v={valueOf(){trace++;return trace===1?1:2}};[1,2,3]",
                "try{o.length=v}catch(e){return e.name==='RangeError'&&o.length===3?42:0}",
                2,
                0,
            ),
            (
                "var trace=0;var a=[1,2,3];Object.defineProperty(a,'1',{configurable:false});var v={valueOf(){trace++;return 0}};a",
                "'use strict';try{o.length=v}catch(e){return e.name==='TypeError'&&o.length===2&&o[2]===undefined?42:0}",
                2,
                0,
            ),
            (
                "var trace=0;var a=[1,2,3];var v={valueOf(){trace++;if(trace===2)Object.defineProperty(a,'length',{writable:false});return 3}};a",
                "'use strict';try{o.length=v}catch(e){return e.name==='TypeError'&&o.length===3?42:0}",
                2,
                0,
            ),
            (
                "var trace=0;var a=[1,2,3];var v={valueOf(){trace++;if(trace===2)Object.defineProperty(a,'length',{writable:false});return 1}};new Proxy(a,{})",
                "'use strict';try{o.length=v}catch(e){return e.name==='TypeError'&&a.length===3?42:0}",
                2,
                0,
            ),
            (
                "var trace=0;var v={valueOf(){trace++;return 1n}};[1,2,3]",
                "try{o.length=v}catch(e){return e.name==='TypeError'&&o.length===3?42:0}",
                1,
                0,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{setup}: {result:?}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, native_calls, "{setup}");
            drop(profile);
            assert_eq!(
                context.eval("trace").unwrap(),
                Value::Int(expected_trace),
                "{setup}"
            );
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_writes_keep_receiver_and_proxy_descriptor_define_order() {
        for (setup, body, expected_trace) in [
            (
                "var trace=0;({n:0,set x(v){trace++;this.n=v}})",
                "o.x=42;return o.n",
                1,
            ),
            (
                "var trace=0;var target={};var o=new Proxy(target,{get set(){trace=trace*10+1;return function(t,k,v,r){trace=trace*10+2;t[k]=v;return r===o}}});o",
                "o.x=42;return target.x",
                12,
            ),
            (
                "var trace=0;var target={};new Proxy(target,{getOwnPropertyDescriptor(t,k){trace=trace*10+1;return undefined},defineProperty(t,k,d){trace=trace*10+2;t[k]=d.value;return d.writable&&d.enumerable&&d.configurable}})",
                "o.x=42;return target.x",
                12,
            ),
            (
                "var trace=0;var t=new Proxy({x:0},{getOwnPropertyDescriptor(){trace=trace*10+2;return {value:42,writable:true,enumerable:true,configurable:true}}});new Proxy(t,{set(){trace=trace*10+1;return true}})",
                "o.x=42;return 42",
                12,
            ),
            (
                "var trace=0;var setter=new Proxy(function(){},{apply(t,r,a){trace++;r.n=a[0]}});var o={n:0};Object.defineProperty(o,'x',{set:setter});o",
                "o.x=42;return o.n",
                1,
            ),
            (
                "var trace=0;var target={};var proto={set x(v){trace++;this.answer=v}};Object.create(proto)",
                "o.x=42;return o.answer",
                1,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(
                    execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                    Completion::Return(Value::Int(42))
                ),
                "{setup}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            drop(profile);
            assert_eq!(
                context.eval("trace").unwrap(),
                Value::Int(expected_trace),
                "{setup}"
            );
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_computed_writes_convert_after_rhs_and_before_nullish_rejection() {
        for (setup, body, expected_trace) in [
            (
                "var trace=0;var key={toString(){trace=trace*10+2;return 'x'}};({set x(v){trace=trace*10+3},rhs(){trace=trace*10+1;return 42}})",
                "o[key]=o.rhs();return 42",
                123,
            ),
            (
                "var trace=0;var token={};var key={toString(){trace++;throw token}};({})",
                "try{null[key]=42}catch(e){return e===token?42:0}",
                1,
            ),
            (
                "var trace=0;var key={toString(){trace++;return 'x'}};({})",
                "try{null[key]=42}catch(e){return e.message===\"cannot set property 'x' of null\"?42:0}",
                1,
            ),
            (
                "var trace=0;var token={};var key='x';({set x(v){trace++;throw token}})",
                "try{o[key]=42}catch(e){return e===token?42:0}",
                1,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(
                    execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                    Completion::Return(Value::Int(42))
                ),
                "{setup}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            drop(profile);
            assert_eq!(
                context.eval("trace").unwrap(),
                Value::Int(expected_trace),
                "{setup}"
            );
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn owned_write_rejections_keep_strictness_and_proxy_same_value_rules() {
        for (setup, body) in [
            ("new Proxy({},{set(){return false}})", "o.x=1;return 42"),
            (
                "new Proxy({},{set(){return false}})",
                "'use strict';try{o.x=1}catch(e){return e.message==='proxy: cannot set property'?42:0}",
            ),
            (
                "Object.freeze({x:1})",
                "'use strict';try{o.x=2}catch(e){return e.message===\"'x' is read-only\"?42:0}",
            ),
            (
                "new Proxy(Object.freeze({x:NaN}),{set(){return true}})",
                "'use strict';o.x=NaN;return 42",
            ),
            (
                "new Proxy(Object.freeze({x:0}),{set(){return true}})",
                "try{o.x=-0}catch(e){return e.message==='proxy: inconsistent set'?42:0}",
            ),
            (
                "var x={};Object.defineProperty(x,'x',{get(){return 1},configurable:false});new Proxy(x,{set(){return true}})",
                "try{o.x=1}catch(e){return e.message==='proxy: inconsistent set'?42:0}",
            ),
            (
                "var trace=0;Object.defineProperty(Number.prototype,'x',{set:function(v){'use strict';trace=this===7?v:0},configurable:true});({})",
                "'use strict';(7).x=42;return trace",
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(setup).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(
                    execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                    Completion::Return(Value::Int(42))
                ),
                "{setup}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{setup}");
            assert_eq!(cost.owned_bridge_exits, 0, "{setup}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{setup}");
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn ordinary_getter_resumes_in_the_same_execution() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = context
            .eval("Object.create({get x(){return this.y}}, {y:{value:41}})")
            .unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(o){return o.x+1})",
            vec![object],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(matches!(result, Completion::Return(Value::Int(42))));
        assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, 2);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn getter_method_call_keeps_the_original_receiver() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = context
            .eval("({y:52, fn:function(){return this.y}, get m(){return this.fn}})")
            .unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(o){return o.m()})",
            vec![object],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(matches!(result, Completion::Return(Value::Int(52))));
        assert_eq!(profile.snapshot().owned_storage.frames_pushed, 3);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn plus_conversion_getter_and_nested_value_of_use_explicit_replies() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = context.eval("({get valueOf(){return this.method}, method:function(){return +this.inner}, inner:{valueOf:function(){return 41}}})").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(o){return +o+1})",
            vec![object],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(matches!(result, Completion::Return(Value::Int(42))));
        assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, 3);
        assert_eq!(profile.snapshot().owned_storage.frames_pushed, 4);
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn conversion_proxy_reads_resume_without_replaying_the_parent_operation() {
        for (source, body, expected_hits) in [
            (
                "var hits=0; new Proxy({}, {get(t,k){hits++;if(k==='valueOf')return function(){hits++;return 41}}})",
                "return +o+1",
                3,
            ),
            (
                "var hits=0; new Proxy({}, {get(t,k){hits++;if(k==='toString')return function(){hits++;return 'answer'}}})",
                "return {answer:42}[o]",
                3,
            ),
            (
                "var hits=0; var p=new Proxy({}, {get(){hits++;return function(){hits++;return 41}}}); new Proxy(p,{})",
                "return +o+1",
                2,
            ),
            (
                "var hits=0; var token={}; new Proxy({}, {get(){hits++;throw token}})",
                "try {return +o} catch(e){return e===token?42:0}",
                1,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let object = context.eval(source).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                &format!("(function(o){{{body}}})"),
                vec![object],
            );
            let profile = CostProfile::start();
            assert!(
                matches!(
                    execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                    Completion::Return(Value::Int(42))
                ),
                "{source}"
            );
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{source}");
            assert_eq!(cost.owned_bridge_exits, 0, "{source}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{source}");
            drop(profile);
            assert_eq!(
                context.eval("hits").unwrap(),
                Value::Int(expected_hits),
                "{source}"
            );
            assert_eq!(runtime.0.proxy_method_depth.get(), 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn plus_conversion_preserves_callback_numeric_tags_and_bigint_diagnostic() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(factory) = context
            .eval("(function(v){return {valueOf:function(){return v}}})")
            .unwrap()
        else {
            panic!("expected factory");
        };
        let factory = runtime.as_callable(&factory).unwrap().unwrap();
        for value in [
            Value::Float(42.0),
            Value::Float(-0.0),
            Value::Float(f64::from_bits(0x7ff8_0000_0000_0042)),
            Value::BigInt(crate::engine::value::bigint::JsBigInt::from(1)),
        ] {
            let object = context
                .call(&factory, Value::Undefined, std::slice::from_ref(&value))
                .unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function root(o){return +o})",
                vec![object],
            );
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let costs = profile.snapshot();
            assert_eq!(costs.owned_storage.frames_pushed, 2);
            assert_eq!(costs.legacy_dispatches, 0);
            assert_eq!(costs.owned_bridge_exits, 0);
            drop(profile);
            match (value, completion) {
                (Value::Float(expected), Completion::Return(Value::Float(actual))) => {
                    assert_eq!(actual.to_bits(), expected.to_bits());
                }
                (Value::BigInt(_), Completion::Throw(Value::Object(error))) => {
                    for (key, expected) in [
                        ("name", "TypeError"),
                        ("message", "bigint argument with unary +"),
                    ] {
                        assert_eq!(
                            context
                                .get_property(&error, &runtime.intern_property_key(key).unwrap())
                                .unwrap(),
                            Value::String(crate::engine::value::JsString::from_static(expected))
                        );
                    }
                }
                _ => panic!("unexpected unary plus completion"),
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn conversion_child_handoff_does_not_repeat_getter_or_method() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = context
            .eval("var hits=0; ({get valueOf(){hits++;return function(){hits++;return 5}}})")
            .unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(o){return +o})",
            vec![object],
        );
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(matches!(result, Completion::Return(Value::Int(5))));
        assert_eq!(context.eval("hits").unwrap(), Value::Int(2));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn conversion_getter_and_method_throws_keep_identity_and_run_once() {
        for source in [
            "({get valueOf(){hits++;throw token}})",
            "({valueOf:function(){hits++;throw token}})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let token = context.eval("var hits=0; var token={}; token").unwrap();
            let object = context.eval(source).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function root(o){return +o})",
                vec![object],
            );
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Throw(value) = result else {
                panic!("expected throw")
            };
            assert_eq!(value, token);
            assert_eq!(context.eval("hits").unwrap(), Value::Int(1));
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn addition_keeps_evaluated_left_when_right_reassigns_parameter() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = context.eval("({valueOf:function(){return 40}})").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(x){return x+(x=2)})",
            vec![object],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(matches!(result, Completion::Return(Value::Int(42))));
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn addition_reads_right_conversion_after_left_callback() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let right = context
            .eval("var log='';var right={valueOf:function(){return 2}};right")
            .unwrap();
        let left = context.eval("({valueOf:function(){log+='L';right.valueOf=function(){log+='R';return 9};return 1}})").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(a,b){return a+b})",
            vec![left, right],
        );
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        assert!(matches!(result, Completion::Return(Value::Int(10))));
        assert_eq!(
            context.eval("log").unwrap(),
            Value::String(crate::engine::value::JsString::from_static("LR"))
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn constructor_primitive_and_object_returns_use_explicit_child_frames() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let constructor = context.eval("(function C(x){return x})").unwrap();
        for argument in [Value::Int(7), context.eval("({marker:42})").unwrap()] {
            let expected = argument.clone();
            let entry = entry(
                &runtime,
                &mut context,
                "(function root(C,x){return new C(x)})",
                vec![constructor.clone(), argument],
            );
            let profile = CostProfile::start();
            let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let Completion::Return(Value::Object(object)) = result else {
                panic!("expected constructed object")
            };
            if matches!(expected, Value::Object(_)) {
                assert_eq!(Value::Object(object), expected);
            }
            assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, 2);
            assert_eq!(profile.snapshot().legacy_dispatches, 0);
            assert_eq!(profile.snapshot().owned_bridge_exits, 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn bound_constructor_retargets_new_target_to_original_function() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let original = context
            .eval("var C=function(){return new.target};C")
            .unwrap();
        let bound = context.eval("C.bind(null,1).bind(null,2)").unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(C){return new C()})",
            vec![bound],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
        let Completion::Return(value) = result else {
            panic!("expected return")
        };
        assert_eq!(value, original);
        assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, 2);
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn derived_return_without_super_preserves_object_and_rejects_primitives() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let constructor = context
            .eval("(class D extends null {constructor(x){return x}})")
            .unwrap();
        for (value, expected_error) in [
            (context.eval("({marker:42})").unwrap(), None),
            (Value::Undefined, Some("ReferenceError")),
            (Value::Int(1), Some("TypeError")),
        ] {
            let expected = value.clone();
            let entry = entry(
                &runtime,
                &mut context,
                "(function root(C,x){return new C(x)})",
                vec![constructor.clone(), value],
            );
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let costs = profile.snapshot();
            assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
            match (completion, expected_error) {
                (Completion::Return(value), None) => assert_eq!(value, expected),
                (Completion::Throw(Value::Object(error)), Some(name)) => {
                    assert_eq!(
                        context
                            .get_property(&error, &runtime.intern_property_key("name").unwrap())
                            .unwrap(),
                        Value::String(crate::engine::value::JsString::from_static(name))
                    );
                }
                _ => panic!("unexpected derived completion"),
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn explicit_super_initializes_this_once_after_calling_the_base() {
        for (source, frames, throws) in [
            (
                "(class D extends (function B(){}) {constructor(){super()}})",
                3,
                false,
            ),
            (
                "(class D extends (function B(){}) {constructor(){super();super()}})",
                4,
                true,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let constructor = context.eval(source).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function root(C){return new C()})",
                vec![constructor],
            );
            let profile = CostProfile::start();
            let completion = execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap();
            let costs = profile.snapshot();
            assert_eq!(costs.owned_storage.frames_pushed, frames, "{costs:?}");
            assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
            match (completion, throws) {
                (Completion::Return(Value::Object(_)), false) => {}
                (Completion::Throw(Value::Object(error)), true) => {
                    assert_eq!(
                        context
                            .get_property(&error, &runtime.intern_property_key("message").unwrap())
                            .unwrap(),
                        Value::String(crate::engine::value::JsString::from_static(
                            "'this' can be initialized only once"
                        ))
                    );
                }
                _ => panic!("unexpected super completion"),
            }
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn default_derived_forwards_actual_arguments_and_live_super_new_target() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let constructor = context
            .eval("var D=class extends (function B(a,b,c){return c}) {}; D")
            .unwrap();
        let marker = context.eval("({marker:42})").unwrap();
        let call_entry = entry(
            &runtime,
            &mut context,
            "(function root(C,x){return new C(1,2,x)})",
            vec![constructor.clone(), marker.clone()],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), call_entry, ExecutionLimits::default()).unwrap();
        let Completion::Return(value) = result else {
            panic!("expected return")
        };
        assert_eq!(value, marker);
        assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, 3);
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        drop(profile);
        context
            .eval("Object.setPrototypeOf(D,function Replacement(){return new.target})")
            .unwrap();
        let call_entry = entry(
            &runtime,
            &mut context,
            "(function root(C){return new C()})",
            vec![constructor.clone()],
        );
        let profile = CostProfile::start();
        let result = execute(runtime.clone(), call_entry, ExecutionLimits::default()).unwrap();
        let Completion::Return(value) = result else {
            panic!("expected return")
        };
        assert_eq!(value, constructor);
        assert_eq!(profile.snapshot().owned_storage.maximum_frame_depth, 3);
        assert_eq!(profile.snapshot().legacy_dispatches, 0);
        assert_eq!(profile.snapshot().owned_bridge_exits, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn selected_direct_calls_validate_domains_once_and_reuse_native_facts() {
        for (callee_source, native) in [
            ("Math.min", true),
            ("(function(x,y){return x<y?x:y})", false),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callee = context.eval(callee_source).unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function root(f){return f(42,99)})",
                vec![callee],
            );
            let profile = CostProfile::start();
            assert!(matches!(
                execute(runtime.clone(), entry, ExecutionLimits::default()).unwrap(),
                Completion::Return(Value::Int(42))
            ));
            let report = profile.snapshot();
            assert_eq!(
                report
                    .owned_execution_events
                    .get("call_value_domain_validation"),
                Some(&1)
            );
            assert_eq!(
                report
                    .owned_execution_events
                    .get("native_classification_reused")
                    .copied()
                    .unwrap_or(0),
                u64::from(native)
            );
            assert_eq!(
                report
                    .owned_execution_events
                    .get("native_publication_checked")
                    .copied()
                    .unwrap_or(0),
                0
            );
            assert_eq!(report.legacy_dispatches, 0);
            assert_eq!(report.owned_bridge_exits, 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn selected_native_metadata_error_follows_foreign_argument_rejection() {
        assert_selected_native_metadata_error(true, false);
    }

    #[test]
    fn selected_native_metadata_error_is_reported_after_valid_domains() {
        assert_selected_native_metadata_error(false, false);
    }

    #[test]
    fn selected_native_missing_receiver_precedes_metadata_error() {
        assert_selected_native_metadata_error(false, true);
    }

    fn assert_selected_native_metadata_error(foreign_argument: bool, missing_receiver: bool) {
        let runtime = Runtime::new();
        let foreign = Runtime::new();
        let mut context = runtime.new_context();
        let callee = context.eval("Math.min").unwrap();
        let Value::Object(function) = &callee else {
            panic!("native")
        };
        let object_id = function.object_id();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(f,x){return f(x)})",
            vec![
                callee,
                if foreign_argument {
                    Value::Object(foreign.new_object(None).unwrap())
                } else {
                    Value::Int(42)
                },
            ],
        );
        let old_realm = runtime
            .0
            .state
            .borrow_mut()
            .heap
            .replace_native_realm_for_test(object_id, None)
            .unwrap();
        let result = if missing_receiver {
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let frame = push_frame(&mut execution, entry).unwrap();
            let RunExit::Call {
                arguments, tail, ..
            } = run(&mut execution, frame).unwrap()
            else {
                panic!("call")
            };
            // The actual call has [callee, argument], so requesting a method
            // receiver must fail the unchanged leading range check first.
            super::ordinary::enter(&runtime, &mut execution, frame, arguments, true, tail)
                .map(|_| Completion::Return(Value::Undefined))
        } else {
            execute(runtime.clone(), entry, ExecutionLimits::default())
        };
        runtime
            .0
            .state
            .borrow_mut()
            .heap
            .replace_native_realm_for_test(object_id, old_realm)
            .unwrap();
        let error = result.err().expect("call rejection");
        let expected = if missing_receiver {
            "owned operand stack underflow"
        } else if foreign_argument {
            "call argument"
        } else {
            "native function was called before its defining realm was attached"
        };
        assert!(error.to_string().contains(expected), "{error}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn direct_child_call_preserves_foreign_argument_rejection() {
        let runtime = Runtime::new();
        let foreign = Runtime::new();
        let mut context = runtime.new_context();
        let callee = context.eval("(function ignore(x){return 1})").unwrap();
        let object = foreign.new_object(None).unwrap();
        let entry = entry(
            &runtime,
            &mut context,
            "(function root(f,x){return f(x)})",
            vec![callee, Value::Object(object.clone())],
        );
        let result = execute(runtime.clone(), entry, ExecutionLimits::default());
        let Err(error) = result else {
            panic!("expected domain rejection, got {result:?}");
        };
        assert!(error.to_string().contains("call argument"), "{error}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        assert!(
            foreign
                .0
                .state
                .borrow()
                .heap
                .object(object.object_id())
                .is_ok()
        );
    }
    #[test]
    fn literal_element_accepts_object_keys_once_and_keeps_ordinary_definition() {
        use crate::engine::code::bytecode::Instruction;
        use crate::engine::value::JsString;

        for (key_source, proxy, throws) in [
            (
                "({get [Symbol.toPrimitive](){trace+='g';return function(hint){trace+=hint;return 'answer'}}})",
                false,
                false,
            ),
            (
                "({get [Symbol.toPrimitive](){trace+='g';throw marker}})",
                false,
                true,
            ),
            (
                "({get [Symbol.toPrimitive](){trace+='g';return function(hint){trace+=hint;return 'answer'}}})",
                true,
                false,
            ),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context
                .eval("var trace='',traps=0,marker={};var target={};")
                .unwrap();
            let object = context
                .eval(if proxy {
                    "new Proxy(target,{defineProperty(){traps++;throw 99}})"
                } else {
                    "target"
                })
                .unwrap();
            let key = context.eval(key_source).unwrap();
            let marker = context.eval("marker").unwrap();
            let entry = entry(
                &runtime,
                &mut context,
                "(function(){return {[Symbol.species]:40}})",
                vec![],
            );
            let pc = entry
                .executable
                .code
                .iter()
                .position(|op| matches!(op, Instruction::DefineArrayEl))
                .unwrap();
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            let frame = execution.frames.current_mut(id).unwrap();
            // Supply a raw object key at this accepted instruction, without the
            // compiler's preceding ToPropKey. The following Drop must still
            // consume the retained key, then Return must receive the base.
            frame.resume_pc = pc;
            for value in [object.clone(), key, Value::Int(40)] {
                execution.slots.push(&mut frame.window, value).unwrap();
            }
            let profile = CostProfile::start();
            let result = execute_running(runtime.clone(), execution).unwrap();
            let costs = profile.snapshot();
            match result {
                Completion::Throw(value) if throws => assert_eq!(value, marker),
                Completion::Return(value) if !throws => assert_eq!(value, object),
                result => panic!("unexpected literal completion: {result:?}"),
            }
            assert_eq!(
                context.eval("trace").unwrap(),
                Value::String(JsString::from_static(if throws { "g" } else { "gstring" }))
            );
            assert_eq!(context.eval("traps").unwrap(), Value::Int(0));
            assert_eq!(
                context.eval("target.answer").unwrap(),
                if throws || proxy {
                    Value::Undefined
                } else {
                    Value::Int(40)
                }
            );
            assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
            assert_eq!(costs.owned_sync_call_bridges, 0, "{costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<CallStep>() <= 64);
