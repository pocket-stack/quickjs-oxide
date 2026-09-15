//! Apply uses the shared argument-list and call/construct continuations.
use super::{driver::CallStep, execution::RunningExecution, frame::FrameId};
use crate::engine::{
    api::{Error, runtime::Runtime},
    code::bytecode::ApplyKind,
};

#[inline(never)]
pub(super) fn step(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    kind: ApplyKind,
    _identity: u64,
) -> Result<CallStep, Error> {
    super::proxy_get_driver::start_apply(runtime, execution, id, kind)
}
