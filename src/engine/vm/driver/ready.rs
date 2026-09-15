//! Reenter the same frame after cold operations which cannot wait or call JS.
//! Opcode semantics remain in run and the shared cold completion helpers.
use super::{CallStep, RunExit};
use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::vm::BytecodePc;
use crate::engine::vm::Completion;
use crate::engine::vm::exception::runtime_error_to_vm_error;
use crate::engine::vm::execution::RunningExecution;
use crate::engine::vm::frame::FrameId;

pub(super) enum Boundary {
    Exit(RunExit),
    /// An existing property/query helper scheduled work; revisit the outer
    /// driver's pending-call/frame checks instead of assuming this frame ran.
    Entered,
    /// Operand domains were checked and next_operation was advanced once.
    /// The outer driver constructs the waiting task without repeating either.
    Conversion(RunExit),
    Complete(Completion),
}

#[inline(never)]
pub(super) fn run(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    mut id: FrameId,
    next_operation: &mut u64,
) -> Result<Boundary, Error> {
    loop {
        let result = super::run(execution, id);
        #[cfg(feature = "profiling")]
        record_exit(&result);
        // Ordinary Call/Return need no observable activation. Cold operations
        // may allocate an error, release an observable owner or invoke code.
        if !matches!(&result, Ok(RunExit::Call { .. } | RunExit::Complete)) {
            execution.frames.materialize(runtime)?;
        }
        let exit = result?;
        match exit {
            RunExit::Materialize => continue,
            RunExit::PrimitiveThrow => {
                let thrown = execution
                    .pending
                    .take()
                    .ok_or_else(|| invariant("resident arithmetic lost its exception"))?;
                return Ok(Boundary::Complete(Completion::Throw(thrown)));
            }
            RunExit::Call {
                arguments,
                method,
                tail,
            } => {
                let selected_native = execution.selected_native.take();
                if let Some(boundary) = enter_call(
                    runtime,
                    execution,
                    &mut id,
                    arguments,
                    method,
                    tail,
                    selected_native,
                )? {
                    return Ok(boundary);
                }
            }
            RunExit::Complete => {
                if !super::ordinary::finish(execution, id)? {
                    return Ok(Boundary::Exit(exit));
                }
                id = execution.frames.current_id().unwrap();
            }
            RunExit::ReplaceBinding { .. } | RunExit::ReleaseOperand { .. } => {
                if !crate::engine::vm::frame_operations::complete_owned_slot(execution, id, exit)? {
                    return Err(invariant(
                        "direct slot completion changed its frame protocol",
                    ));
                }
            }

            RunExit::Numeric(kind) => {
                use crate::engine::vm::frame_operations::NumericProgress;
                let Some(progress) =
                    crate::engine::vm::frame_operations::try_complete_primitive_numeric(
                        runtime, execution, id, kind,
                    )?
                else {
                    return Ok(Boundary::Exit(exit));
                };
                match progress {
                    NumericProgress::Completed => {
                        #[cfg(feature = "profiling")]
                        record_event("numeric_completed_in_same_frame");
                    }
                    NumericProgress::Deferred(CallStep::Entered) => return Ok(Boundary::Entered),
                    NumericProgress::Deferred(CallStep::Complete(completion)) => {
                        return Ok(Boundary::Complete(completion));
                    }
                    NumericProgress::Deferred(CallStep::Bridge) => {
                        return Err(invariant("numeric operation attempted replay"));
                    }
                }
            }
            RunExit::AddLocal => {
                use crate::engine::vm::conversion_driver::PrimitiveCompletion;
                match crate::engine::vm::conversion_driver::complete_local_add(
                    runtime,
                    execution,
                    id,
                    next_operation,
                )? {
                    PrimitiveCompletion::Completed => {}
                    PrimitiveCompletion::Throw(value) => {
                        return Ok(Boundary::Complete(Completion::Throw(value)));
                    }
                    _ => return Err(invariant("local addition lost its primitive guard")),
                }
            }
            RunExit::ConvertPlus | RunExit::ConvertAdd => {
                let addition = exit == RunExit::ConvertAdd;
                use crate::engine::vm::conversion_driver::PrimitiveCompletion;
                match crate::engine::vm::conversion_driver::complete_primitives(
                    runtime,
                    execution,
                    id,
                    addition,
                    next_operation,
                )? {
                    PrimitiveCompletion::Completed => {}
                    PrimitiveCompletion::Throw(value) => {
                        return Ok(Boundary::Complete(Completion::Throw(value)));
                    }
                    PrimitiveCompletion::InvalidDomain => {
                        return Ok(Boundary::Exit(RunExit::Bridge));
                    }
                    PrimitiveCompletion::Declined => return Ok(Boundary::Conversion(exit)),
                }
            }

            RunExit::GetField {
                index,
                keep_receiver,
            } => {
                let mut selected_native = None;
                let progress = crate::engine::vm::property_driver::read_progress_selected(
                    runtime,
                    execution,
                    id,
                    crate::engine::vm::property_driver::ReadKey::Static(index),
                    keep_receiver,
                    &mut selected_native,
                )?;
                if let crate::engine::vm::property_driver::PropertyProgress::MethodCall(arguments) =
                    progress
                {
                    let frame = execution.frames.current_mut(id)?;
                    frame.fault_pc = frame.resume_pc;
                    runtime
                        .update_active_bytecode_pc(
                            frame.active_frame,
                            BytecodePc::new(frame.fault_pc),
                        )
                        .map_err(runtime_error_to_vm_error)?;
                    if let Some(boundary) = enter_call(
                        runtime,
                        execution,
                        &mut id,
                        arguments,
                        true,
                        false,
                        selected_native,
                    )? {
                        return Ok(boundary);
                    }
                } else if let Some(boundary) = property_boundary(progress) {
                    return Ok(boundary);
                }
            }
            RunExit::GetElement {
                keep_receiver,
                keep_key,
            } => {
                let frame = execution.frames.current_mut(id)?;
                if !matches!(
                    execution.slots.peek(&frame.window, 1)?,
                    crate::engine::value::Value::Null | crate::engine::value::Value::Undefined
                ) && matches!(
                    execution.slots.peek(&frame.window, 0)?,
                    crate::engine::value::Value::Object(_)
                ) {
                    return Ok(Boundary::Exit(exit));
                }
                let progress = crate::engine::vm::property_driver::read_progress(
                    runtime,
                    execution,
                    id,
                    crate::engine::vm::property_driver::ReadKey::Computed { keep_key },
                    keep_receiver,
                )?;
                if let Some(boundary) = property_boundary(progress) {
                    return Ok(boundary);
                }
            }
            RunExit::SetProperty(key) => {
                let frame = execution.frames.current_mut(id)?;
                if key.is_none()
                    && matches!(
                        execution.slots.peek(&frame.window, 1)?,
                        crate::engine::value::Value::Object(_)
                    )
                {
                    return Ok(Boundary::Exit(exit));
                }
                let progress = crate::engine::vm::property_write_driver::write_progress(
                    runtime, execution, id, key,
                )?;
                if let Some(boundary) = property_boundary(progress) {
                    return Ok(boundary);
                }
            }
            _ => return Ok(Boundary::Exit(exit)),
        }
    }
}

fn property_boundary(
    progress: crate::engine::vm::property_driver::PropertyProgress,
) -> Option<Boundary> {
    use crate::engine::vm::property_driver::PropertyProgress;
    match progress {
        PropertyProgress::Completed | PropertyProgress::MethodCall(_) => None,
        PropertyProgress::Deferred(CallStep::Entered) => Some(Boundary::Entered),
        PropertyProgress::Deferred(CallStep::Complete(completion)) => {
            Some(Boundary::Complete(completion))
        }
        PropertyProgress::Deferred(CallStep::Bridge) => Some(Boundary::Exit(RunExit::Bridge)),
    }
}

fn enter_call(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: &mut FrameId,
    arguments: u16,
    method: bool,
    tail: bool,
    selected_native: Option<crate::engine::object::LinkedNativeSelection>,
) -> Result<Option<Boundary>, Error> {
    Ok(
        match super::ordinary::enter_selected(
            runtime,
            execution,
            *id,
            arguments,
            method,
            tail,
            selected_native,
        )? {
            super::ordinary::Entry::Ordinary => {
                *id = execution.frames.current_id().unwrap();
                None
            }
            super::ordinary::Entry::NativeReady => None,
            super::ordinary::Entry::Native(CallStep::Entered) => Some(Boundary::Entered),
            super::ordinary::Entry::Native(CallStep::Complete(completion)) => {
                Some(Boundary::Complete(completion))
            }
            super::ordinary::Entry::Native(CallStep::Bridge) => {
                Some(Boundary::Exit(RunExit::Bridge))
            }
            super::ordinary::Entry::General => {
                execution.frames.materialize(runtime)?;
                Some(Boundary::Exit(RunExit::Call {
                    arguments,
                    method,
                    tail,
                }))
            }
        },
    )
}

/// Error allocation and diagnostic-only dispatch must not widen the ordinary
/// ready loop. Profiling helpers are absent from the ordinary build.
#[cold]
#[inline(never)]
fn invariant(message: &'static str) -> Error {
    Error::internal(message)
}

#[cfg(feature = "profiling")]
#[cold]
#[inline(never)]
fn record_exit(result: &Result<RunExit, Error>) {
    record_event(match result {
        Ok(exit) => exit.diagnostic_name(),
        Err(_) => "run_exit.EngineError",
    });
}

#[cfg(feature = "profiling")]
#[cold]
#[inline(never)]
fn record_event(event: &'static str) {
    crate::engine::api::profiling::record_owned_execution_event(event);
}
