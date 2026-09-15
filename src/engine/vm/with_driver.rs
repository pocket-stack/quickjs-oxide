//! HasBinding delegates its owned Has/Get sequence to the shared environment algorithm.
use super::{
    driver::CallStep,
    environment_bindings::operation::EnvironmentStep,
    execution::RunningExecution,
    frame::{FrameId, ReturnValue},
};
use crate::engine::api::{error::Error, runtime::Runtime};
use crate::engine::code::bytecode::DynamicEnvironmentSource;

#[inline(never)]
pub(super) fn start(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    source: DynamicEnvironmentSource,
    name: u32,
) -> Result<CallStep, Error> {
    let realm = execution.frames.current_mut(id)?.executable.realm;
    let result = (|| {
        let frame = execution.frames.current_mut(id)?;
        let object = super::environment_bindings::dynamic_object(
            runtime,
            &frame.executable,
            source,
            |index| execution.slots.local(&frame.window, index).ok(),
            &frame.cold.closure_slots,
        )?;
        let key = super::environment_driver::linked_key(runtime, &frame.executable, name)?;
        let depth = execution.slots.depth(&frame.window);
        let step = EnvironmentStep::has_binding(
            realm,
            object,
            key,
            matches!(source, DynamicEnvironmentSource::With(_)),
        );
        super::proxy_get_driver::start_environment(
            runtime,
            execution,
            id,
            step,
            ReturnValue::Push,
            depth,
        )
    })();
    match result {
        Ok(step) => Ok(step),
        Err(error) => super::property_driver::throw_error(runtime, realm, error),
    }
}
