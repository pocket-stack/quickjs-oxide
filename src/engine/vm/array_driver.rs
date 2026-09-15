//! Literal definitions preserve the input key until the following stack step.
use super::{
    driver::CallStep, exception::runtime_error_to_vm_error, execution::RunningExecution,
    frame::FrameId,
};
use crate::engine::{
    api::{Error, ErrorKind, runtime::Runtime},
    object::object_literal::element::LiteralDefinitionStep,
    value::Value,
};

#[inline(never)]
pub(super) fn define_element(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let Value::Object(object) = execution.slots.peek(&frame.window, 2)? else {
        return super::property_driver::throw_error(
            runtime,
            realm,
            Error::new(ErrorKind::Type, "not an object"),
        );
    };
    let object = object.clone();
    let key = execution.slots.peek(&frame.window, 1)?.clone();
    let depth = execution.slots.depth(&frame.window);
    let value = execution.slots.pop(&mut frame.window)?;
    let step = match LiteralDefinitionStep::start(runtime, realm, object, key, value) {
        Ok(step) => step,
        Err(error) => {
            return super::property_driver::throw_error(
                runtime,
                realm,
                runtime_error_to_vm_error(error),
            );
        }
    };
    super::proxy_get_driver::start_literal_definition(runtime, execution, id, step, depth)
}
