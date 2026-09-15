//! Same-frame slot ownership transitions outside the resident RunSlots borrow.
use crate::engine::api::error::Error;
use crate::engine::vm::execution::RunningExecution;
use crate::engine::vm::frame::FrameId;
use crate::engine::vm::run::RunExit;

pub(in crate::engine::vm) fn complete(
    execution: &mut RunningExecution,
    id: FrameId,
    exit: RunExit,
) -> Result<bool, Error> {
    if let RunExit::ReplaceBinding {
        source,
        index,
        keep,
        uninitialized,
    } = exit
    {
        use crate::engine::vm::{bindings::FrameBinding, run::BindingSource};
        let frame = execution.frames.current_mut(id)?;
        #[cfg(feature = "profiling")]
        let depth = execution.slots.depth(&frame.window);
        let old = {
            let mut slots = execution.slots.run_window(&mut frame.window)?;
            let current = match source {
                BindingSource::Local => slots.local(index)?,
                BindingSource::Argument => slots.parameter(index)?,
                BindingSource::Closure => {
                    return Err(Error::internal("direct release received a closure"));
                }
            };
            if !matches!(
                current,
                FrameBinding::Direct(_) | FrameBinding::Uninitialized
            ) {
                return Err(Error::internal("direct release lost its slot owner"));
            }
            let next = if uninitialized {
                FrameBinding::Uninitialized
            } else if keep {
                FrameBinding::Direct(crate::engine::vm::stack::copy_value(slots.peek(0)?)?)
            } else {
                FrameBinding::Direct(slots.pop()?)
            };
            match source {
                BindingSource::Local => slots.replace_local(index, next)?,
                BindingSource::Argument => slots.replace_parameter(index, next)?,
                BindingSource::Closure => unreachable!(),
            }
        };
        if uninitialized {
            if let Some(flag) = frame
                .cold
                .reusable_captured_locals
                .get_mut(usize::from(index))
            {
                *flag = false;
            }
        }
        // Publish the replacement before releasing the displaced last root.
        // Ordinary Drop handles collection work outside the resident dispatch.
        drop(old);
        frame.resume_pc = frame
            .fault_pc
            .checked_add(1)
            .ok_or_else(|| Error::internal("binding release resume PC overflow"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_instruction(depth);
        return Ok(true);
    }
    if let RunExit::ReleaseOperand { keep_top } = exit {
        let frame = execution.frames.current_mut(id)?;
        // Hot preflight has not changed an owner. Validate both operands before
        // moving either, then let the ordinary Drop path drain deferred work.
        #[cfg(feature = "profiling")]
        let depth = execution.slots.depth(&frame.window);
        let released = {
            let mut slots = execution.slots.run_window(&mut frame.window)?;
            slots.peek(usize::from(keep_top))?;
            let kept = if keep_top { Some(slots.pop()?) } else { None };
            let released = slots.pop()?;
            if let Some(kept) = kept {
                slots.push(kept)?;
            }
            released
        };
        // Publish the surviving stack before dropping the last temporary root.
        drop(released);
        frame.resume_pc = frame
            .fault_pc
            .checked_add(1)
            .ok_or_else(|| Error::internal("release resume PC overflow"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_instruction(depth);
        return Ok(true);
    }
    Ok(false)
}
