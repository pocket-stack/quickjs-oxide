//! Iterator records stay in operand slots; regions hold only validated indices.
use super::{CallStep, Completion, Error, FrameId, RunningExecution, Runtime};
use crate::engine::value::JsValue;
use crate::engine::vm::{VmUnwindRegion, frame::Frame, stack::SlotStore};

pub(super) fn disable(
    runtime: &Runtime,
    frame: &mut Frame,
    slots: &mut SlotStore,
    base: usize,
) -> Result<(), Error> {
    let body = &mut *frame.cold;
    let Some(VmUnwindRegion::Iterator {
        record_base,
        enabled,
        asynchronous: false,
    }) = body.owners.regions.last_mut()
    else {
        return Err(Error::internal(
            "iterator operation has no innermost synchronous region",
        ));
    };
    if *record_base != base {
        return Err(Error::internal("iterator region changed during operation"));
    }
    let offset = slots
        .depth(&body.window)
        .checked_sub(base + 1)
        .ok_or_else(|| Error::internal("iterator record is truncated"))?;
    let old = slots.replace_operand(&body.window, offset, JsValue::Undefined)?;
    runtime
        .release_jsvalue(old)
        .map_err(super::runtime_error_to_vm_error)?;
    *enabled = false;
    Ok(())
}

pub(super) fn take(
    runtime: &Runtime,
    frame: &mut Frame,
    slots: &mut SlotStore,
    preserve: bool,
) -> Result<(JsValue, bool, bool), Error> {
    let Some(VmUnwindRegion::Iterator {
        record_base,
        enabled,
        asynchronous,
    }) = frame.cold.regions.last().copied()
    else {
        return Err(Error::internal(
            "iterator cleanup has no innermost iterator region",
        ));
    };
    let end = record_base
        .checked_add(2)
        .ok_or_else(|| Error::internal("iterator record depth overflow"))?;
    let depth = slots.depth(&frame.window);
    if (preserve && depth <= end) || (!preserve && depth != end) {
        return Err(Error::internal(
            "iterator cleanup did not reach its record/preserved value",
        ));
    }
    let iterator = {
        let peeked = slots.peek(&frame.window, depth - record_base - 1)?;
        runtime
            .dup_jsvalue(peeked)
            .map_err(super::runtime_error_to_vm_error)?
    };
    let value = if preserve {
        Some(slots.pop(&mut frame.window)?)
    } else {
        None
    };
    while slots.depth(&frame.window) > record_base {
        let discarded = slots.pop(&mut frame.window)?;
        runtime
            .release_jsvalue(discarded)
            .map_err(super::runtime_error_to_vm_error)?;
    }
    if let Some(value) = value {
        slots.push(&mut frame.window, value)?;
    }
    frame.cold.regions.pop();
    Ok((iterator, enabled, asynchronous))
}

#[inline(never)]
pub(in crate::engine::vm) fn unwind(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    mut value: JsValue,
) -> Result<CallStep, Error> {
    runtime
        .ensure_error_backtrace_jsvalue(&value, false, None)
        .map_err(super::runtime_error_to_vm_error)?;
    loop {
        let frame = execution.frames.current_mut(id)?;
        let Some(region) = frame.cold.regions.last().copied() else {
            return Ok(CallStep::Complete(Completion::Throw(value)));
        };
        match region {
            VmUnwindRegion::Catch {
                target,
                stack_depth,
            } => {
                crate::engine::vm::driver::prepare_captured_reuse(frame, &execution.slots)?;
                if execution.slots.depth(&frame.window) < stack_depth {
                    return Err(Error::internal(
                        "exception handler stack depth exceeds the VM stack",
                    ));
                }
                while execution.slots.depth(&frame.window) > stack_depth {
                    let discarded = execution.slots.pop(&mut frame.window)?;
                    runtime
                        .release_jsvalue(discarded)
                        .map_err(super::runtime_error_to_vm_error)?;
                }
                execution.slots.push(&mut frame.window, value)?;
                frame.cold.regions.pop();
                frame.resume_pc = target;
                return Ok(CallStep::Entered);
            }
            VmUnwindRegion::Iterator {
                record_base,
                enabled,
                ..
            } => {
                let end = record_base
                    .checked_add(2)
                    .ok_or_else(|| Error::internal("iterator record depth overflow"))?;
                let depth = execution.slots.depth(&frame.window);
                if depth < end {
                    return Err(Error::internal(
                        "iterator unwind region exceeds the VM stack",
                    ));
                }
                let iterator = {
                    let peeked = execution.slots.peek(&frame.window, depth - record_base - 1)?;
                    runtime
                        .dup_jsvalue(peeked)
                        .map_err(super::runtime_error_to_vm_error)?
                };
                while execution.slots.depth(&frame.window) > record_base {
                    let discarded = execution.slots.pop(&mut frame.window)?;
                    runtime
                        .release_jsvalue(discarded)
                        .map_err(super::runtime_error_to_vm_error)?;
                }
                frame.cold.regions.pop();
                if enabled {
                    match super::close_unwind(runtime, execution, id, iterator, value)? {
                        CallStep::Entered => return Ok(CallStep::Entered),
                        CallStep::Complete(Completion::Throw(original)) => value = original,
                        _ => return Err(Error::internal("iterator unwind lost its throw")),
                    }
                }
            }
        }
    }
}
