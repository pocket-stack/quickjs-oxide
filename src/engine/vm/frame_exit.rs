//! Cold frame destruction and temporary legacy handoff keep large owners off the driver stack.
use super::{
    Completion,
    exception::runtime_error_to_vm_error,
    execution::RunningExecution,
    frame::{ConstructorReturn, FrameEntry, FrameId, ReturnTarget},
    frames::ActiveFrameGuard,
    run::RunExit,
};
use crate::engine::{
    api::{Error, runtime::Runtime},
    value::Value,
};

pub(super) enum FrameExit {
    Complete {
        completion: Completion,
        return_to: Option<ReturnTarget>,
    },
    Suspended {
        outcome: super::suspend::VmRunOutcome,
        target: ReturnTarget,
    },
    RootHandoff(Box<RootHandoff>),
}

pub(super) struct RootHandoff {
    entry: FrameEntry,
    pc: usize,
    guard: Option<ActiveFrameGuard>,
    // Keep the same lifetime as the pre-extraction root-handoff branch.
    _constructor_return: Option<ConstructorReturn>,
}

impl RootHandoff {
    #[inline(never)]
    pub(super) fn execute_suspending(
        self: Box<Self>,
        runtime: Runtime,
    ) -> Result<super::suspend::VmRunOutcome, Error> {
        let Self {
            entry,
            pc,
            guard,
            _constructor_return,
        } = *self;
        let result = super::host_bridge::owned::run_frame(runtime, entry, pc);
        if let Some(guard) = guard {
            guard.finish().map_err(runtime_error_to_vm_error)?;
        }
        result
    }

    #[inline(never)]
    pub(super) fn execute(self: Box<Self>, runtime: Runtime) -> Result<Completion, Error> {
        let Self {
            entry,
            pc,
            guard,
            _constructor_return,
        } = *self;
        let result = super::host_bridge::owned::execute_frame(runtime, entry, pc);
        if let Some(guard) = guard {
            guard.finish().map_err(runtime_error_to_vm_error)?;
        }
        result
    }
}

#[inline(never)]
pub(super) fn finish(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    exit: RunExit,
    forwarded: Option<Completion>,
) -> Result<FrameExit, Error> {
    let mut frame = execution.frames.pop(id)?;
    let return_to = frame.cold.return_to;
    let constructor_return = frame.cold.constructor_return.take();
    let guard = frame.cold.entry_guard.take();
    let mut retired_cold = None;
    let result = if exit == RunExit::Complete {
        let completion = match forwarded {
            Some(completion) => completion,
            None => execution
                .pending
                .take()
                .map(Completion::Return)
                .ok_or_else(|| Error::internal("owned completion has no payload"))?,
        };
        // Install the completion owner before releasing any window root.
        execution.slots.clear_frame(frame.window.take())?;
        retired_cold = Some(frame.cold);
        Ok(super::suspend::VmRunOutcome::Complete(completion))
    } else {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_bridge();
        let storage = execution.slots.take_frame(frame.window.take())?;
        let entry = FrameEntry {
            initialize_bindings: false,
            property_generation: frame.property_generation,
            iterator_generation: frame.iterator_generation,
            caller_realm: frame.caller_realm,
            active_frame: frame.active_frame,
            executable: frame.executable.take(),
            cold: frame.cold,
            storage,
        };
        if return_to.is_none() {
            return Ok(FrameExit::RootHandoff(Box::new(RootHandoff {
                entry,
                pc: frame.resume_pc,
                guard,
                _constructor_return: constructor_return,
            })));
        }
        super::host_bridge::owned::run_frame(runtime.clone(), entry, frame.resume_pc)
    };
    if let Some(guard) = guard {
        guard.finish().map_err(runtime_error_to_vm_error)?;
    }
    if let Some(cold) = retired_cold {
        execution.call_storage.recycle(cold);
    }
    let completion = match result? {
        super::suspend::VmRunOutcome::Complete(completion) => completion,
        outcome @ super::suspend::VmRunOutcome::Suspend { .. } => {
            if constructor_return.is_some() {
                return Err(Error::internal("constructor handoff suspended"));
            }
            return Ok(FrameExit::Suspended {
                outcome,
                target: return_to
                    .ok_or_else(|| Error::internal("child suspension lost its return target"))?,
            });
        }
    };
    let completion = match (completion, constructor_return) {
        (Completion::Return(value), Some(ConstructorReturn::Base(receiver))) => {
            Completion::Return(if matches!(value, Value::Object(_)) {
                value
            } else {
                receiver
            })
        }
        (Completion::Return(value), Some(ConstructorReturn::Derived)) => {
            if !matches!(value, Value::Object(_)) {
                return Err(Error::internal(
                    "derived constructor bytecode returned an unvalidated primitive",
                ));
            }
            Completion::Return(value)
        }
        (completion, _) => completion,
    };
    Ok(FrameExit::Complete {
        completion,
        return_to,
    })
}
