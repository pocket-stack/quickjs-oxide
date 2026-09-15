//! Closure allocation and capture temporaries do not live in the resident driver.
use super::exception::runtime_error_to_vm_error;
use super::execution::RunningExecution;
use super::frame::FrameId;
use crate::engine::api::{error::Error, runtime::Runtime};
use crate::engine::value::Value;

#[inline(never)]
pub(super) fn instantiate(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: u32,
) -> Result<(), Error> {
    use super::bindings::{capture_frame_binding, capture_local_binding, reuse_frame_capture};
    use crate::engine::code::function::metadata::ClosureSource;
    let frame = execution.frames.current_mut(id)?;
    let Some(crate::engine::heap::BytecodeConstant::Function(child_id)) =
        frame.executable.constant(index)
    else {
        return Err(Error::internal(
            "function-closure opcode referenced a non-function constant",
        ));
    };
    let child_id = *child_id;
    let descriptors = runtime
        .0
        .state
        .borrow()
        .heap
        .function_bytecode(child_id)
        .map_err(|error| Error::internal(error.to_string()))?
        .closure_variables
        .clone();
    let bytecode = crate::engine::code::rooted::FunctionBytecodeRef::from_borrowed_handle(
        runtime.clone(),
        child_id,
    )
    .map_err(|error| Error::internal(error.to_string()))?;
    let mut captured = Vec::new();
    captured
        .try_reserve_exact(descriptors.len())
        .map_err(|_| Error::internal("closure captures allocation failed"))?;
    for descriptor in descriptors.iter().copied() {
        let root = match descriptor.source {
            ClosureSource::ParentLocal(index) => capture_local_binding(
                &runtime,
                execution.slots.local_mut(&frame.window, index)?,
                frame.executable.local_definitions[usize::from(index)],
                descriptor,
            )?,
            ClosureSource::ParentArgument(index) => capture_frame_binding(
                &runtime,
                execution.slots.parameter_mut(&frame.window, index)?,
                descriptor,
            )?,
            ClosureSource::ParentClosure(index) => reuse_frame_capture(
                &runtime,
                &frame
                    .cold
                    .closure_slots
                    .get(usize::from(index))
                    .ok_or_else(|| {
                        Error::internal("captured parent closure index is out of bounds")
                    })?,
                descriptor,
            )?,
            ClosureSource::ParentGlobal(index) => frame
                .cold
                .closure_slots
                .get(usize::from(index))
                .ok_or_else(|| {
                    Error::internal("relayed parent global closure index is out of bounds")
                })?
                .clone(),
            _ => {
                return Err(Error::internal(
                    "child closure attempted to resolve a root descriptor",
                ));
            }
        };
        captured.push(root);
    }
    let callable = runtime
        .new_bytecode_closure_with_slots(frame.executable.realm, &bytecode, &captured)
        .map_err(runtime_error_to_vm_error)?;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    execution
        .slots
        .push(&mut frame.window, Value::Object(callable.into_object()))?;
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("closure resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(())
}
