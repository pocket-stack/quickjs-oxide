//! Move a suspended owned frame across the heap-publication boundary.
use super::{VmActivationResume, VmRunOutcome};
use crate::engine::api::{Error, runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::value::Value;
use crate::engine::vm::execution::RunningExecution;
use crate::engine::vm::frame::{FrameEntry, FrameId};
use crate::engine::vm::host_bridge::{RuntimeVmHost, owned};
use crate::engine::vm::{CallInput, VmResume, VmSuspendKind, VmSuspension};

pub(in crate::engine::vm) struct OwnedSuspension {
    entry: FrameEntry,
    pub(in crate::engine::vm) return_to: Option<crate::engine::vm::frame::ReturnTarget>,
    pc: usize,
    kind: VmSuspendKind,
}

impl OwnedSuspension {
    pub(in crate::engine::vm) fn detach(
        execution: &mut RunningExecution,
        id: FrameId,
        kind: VmSuspendKind,
    ) -> Result<Self, Error> {
        #[cfg(feature = "profiling")]
        let _profile_phase = crate::engine::api::profiling::PhaseTimer::start_vm("freeze.detach");
        let runtime = execution
            .frames
            .current_mut(id)?
            .cold
            .function
            .runtime()
            .clone();
        execution.frames.materialize(&runtime)?;
        let frame = execution.frames.current_mut(id)?;
        if frame.cold.has_pending_query()
            || frame.cold.iterator_wait.is_some()
            || frame.cold.conversion.is_some()
            || frame.cold.eval_arguments.is_some()
            || frame.cold.constructor_return.is_some()
        {
            return Err(Error::internal(
                "suspension has an unresolved frame continuation",
            ));
        }
        let mut frame = execution.frames.pop(id)?;
        let storage = execution.slots.take_frame(frame.window.take())?;
        if let Some(guard) = frame.cold.entry_guard.take() {
            guard
                .finish()
                .map_err(crate::engine::vm::exception::runtime_error_to_vm_error)?;
        }
        let return_to = frame.cold.return_to.take();
        Ok(Self {
            return_to,
            entry: FrameEntry {
                initialize_bindings: false,
                property_generation: frame.property_generation,
                iterator_generation: frame.iterator_generation,
                caller_realm: frame.caller_realm,
                active_frame: frame.active_frame,
                executable: frame.executable.take(),
                cold: frame.cold,
                storage,
            },
            pc: frame.resume_pc,
            kind,
        })
    }

    pub(in crate::engine::vm) fn freeze(
        self: Box<Self>,
        runtime: Runtime,
    ) -> Result<VmRunOutcome, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _profile_phase =
            crate::engine::api::profiling::PhaseTimer::start_vm("freeze.owned_export");
        let Self {
            entry,
            pc,
            kind,
            return_to: _,
        } = *self;
        // This is a representation adapter only; no old interpreter runs.
        let (host, activation, originals) =
            owned::detach_frame(runtime, entry, pc).map_err(RuntimeError::Engine)?;
        let suspension = VmSuspension::new(kind, activation).map_err(RuntimeError::Engine)?;
        super::finish_suspension(host, suspension, originals)
    }
}

pub(super) fn prepare(
    host: RuntimeVmHost,
    suspension: VmSuspension,
    original_arguments: Vec<Value>,
    resume: VmActivationResume,
) -> Result<PreparedResume, RuntimeError> {
    let (kind, mut parts) = suspension.into_parts().map_err(RuntimeError::Engine)?;
    let mut abrupt = None;
    let injection = match (kind, resume) {
        (VmSuspendKind::Initial, VmActivationResume::Initial) => None,
        (VmSuspendKind::Await, VmActivationResume::AwaitFulfill(value)) => Some((value, None)),
        (VmSuspendKind::Await, VmActivationResume::AwaitReject(value))
        | (VmSuspendKind::Yield, VmActivationResume::Generator(VmResume::Throw(value))) => {
            abrupt = Some(value);
            None
        }
        (
            VmSuspendKind::Yield | VmSuspendKind::YieldStar | VmSuspendKind::AsyncYieldStar,
            VmActivationResume::Generator(resume),
        ) => {
            let (value, magic) = match resume {
                VmResume::Next(value) => (value, 0),
                VmResume::Return(value) => (value, 1),
                VmResume::Throw(value) => (value, 2),
            };
            Some((value, Some(magic)))
        }
        _ => {
            return Err(RuntimeError::Invariant(
                "resume operation disagrees with the suspended VM state",
            ));
        }
    };
    if kind != VmSuspendKind::Initial && !matches!(parts.stack.last(), Some(Value::Undefined)) {
        return Err(RuntimeError::Invariant(
            "suspension resume operand was not cleared",
        ));
    }
    if let Some((value, magic)) = injection {
        *parts
            .stack
            .last_mut()
            .ok_or(RuntimeError::Invariant("suspension has no resume operand"))? = value;
        if let Some(magic) = magic {
            parts
                .stack
                .try_reserve(1)
                .map_err(|_| RuntimeError::Invariant("resume operand allocation failed"))?;
            parts.stack.push(Value::Int(magic));
        }
    }
    let input = CallInput {
        this_value: parts.this_value,
        new_target: parts.new_target,
        callee_global: Some(
            parts
                .callee_global
                .ok_or(RuntimeError::Invariant("suspension has no callee global"))?,
        ),
    };
    let (_runtime, mut entry) =
        owned::prepare(host, input, &original_arguments).map_err(RuntimeError::Engine)?;
    entry.cold.regions = parts.regions;
    entry.cold.normalized_this = parts.normalized_this;
    entry.storage.operands = parts.stack;
    entry.cold.resume_throw = abrupt;
    Ok(PreparedResume {
        entry,
        pc: parts.pc,
    })
}

pub(in crate::engine::vm) struct PreparedResume {
    pub entry: FrameEntry,
    pub pc: usize,
}
