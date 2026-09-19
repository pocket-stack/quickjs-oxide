//! Outlined frame and binding operations. Keeping their temporary values out
//! of the resident driver frame leaves native stack room for callback reentry.

#[cfg(test)]
mod direct;
mod numeric;
#[cfg(test)]
pub(super) use direct::complete as complete_owned_slot;
pub(super) use numeric::{
    NumericProgress, commit_output as commit_numeric_output, complete as complete_numeric,
    try_complete_primitive as try_complete_primitive_numeric,
};

use super::Completion;
use super::driver::{CallStep, prepare_captured_reuse};
use super::exception::runtime_error_to_vm_error;
use super::execution::RunningExecution;
use super::frame::FrameId;
use super::run::RunExit;
use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::value::Value;
use crate::engine::value::conversion::NativeConversion;

#[inline(never)]
pub(super) fn pure(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    operation: super::pure_operations::PureOperation,
) -> Result<CallStep, Error> {
    super::pure_operations::step(runtime, execution, id, operation)
}

#[inline(never)]
pub(super) fn copy_data(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    target: usize,
    source: usize,
    excluded: Option<usize>,
) -> Result<CallStep, Error> {
    super::proxy_get_driver::start_object_copy(runtime, execution, id, target, source, excluded)
}

#[inline(never)]
pub(super) fn home_object(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let home = runtime
        .bytecode_function_home_object(&frame.cold.function)
        .map_err(runtime_error_to_vm_error)?
        .ok_or_else(|| Error::internal("bytecode requested an uninstalled HomeObject"))?;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    execution
        .slots
        .push(&mut frame.window, Value::Object(home))?;
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("HomeObject resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Entered)
}

#[inline(never)]
pub(super) fn get_super(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let value = execution.slots.peek(&frame.window, 0)?;
    if let Value::Object(object) = value {
        if object.belongs_to(runtime) {
            let value = runtime
                .get_prototype_of(object)
                .map_err(runtime_error_to_vm_error)?
                .map_or(Value::Null, Value::Object);
            #[cfg(feature = "profiling")]
            let depth = execution.slots.depth(&frame.window);
            execution.slots.pop(&mut frame.window)?;
            execution.slots.push(&mut frame.window, value)?;
            frame.resume_pc = frame
                .fault_pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("super resume PC overflow"))?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(depth);
            return Ok(CallStep::Entered);
        }
    }
    Ok(CallStep::Bridge)
}

#[cold]
#[inline(never)]
pub(super) fn binding_error(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: u32,
    redeclaration: bool,
) -> Result<CallStep, Error> {
    Ok(CallStep::Complete(super::exception::binding_error(
        runtime,
        execution,
        id,
        index,
        redeclaration,
    )?))
}

#[inline(never)]
pub(super) fn private_initialize(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: u16,
    kind: super::private_bindings::Initialization,
) -> Result<CallStep, Error> {
    match super::private_bindings::step(runtime, execution, id, index, kind)? {
        None => Ok(CallStep::Entered),
        Some(completion) => Ok(CallStep::Complete(completion)),
    }
}

#[inline(never)]
pub(super) fn private_access(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    source: crate::engine::code::bytecode::PrivateNameSource,
    access: super::private_access::Access,
) -> Result<CallStep, Error> {
    match super::private_access::step(runtime, execution, id, source, access)? {
        super::private_access::Outcome::Done | super::private_access::Outcome::Entered => {
            Ok(CallStep::Entered)
        }
        super::private_access::Outcome::Throw(value) => {
            Ok(CallStep::Complete(Completion::Throw(value)))
        }
    }
}

#[inline(never)]
pub(super) fn for_in(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    next: bool,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let depth = execution.slots.depth(&frame.window);
    let step = if next {
        let Value::Object(iterator) = execution.slots.peek(&frame.window, 0)? else {
            return Err(Error::internal(
                "for-in next received a non-object iterator",
            ));
        };
        super::for_in::operation::ForInStep::next(runtime, realm, iterator)
    } else {
        let value = execution.slots.pop(&mut frame.window)?;
        super::for_in::operation::ForInStep::start(runtime, realm, value)
    };
    match step {
        Ok(step) => {
            super::proxy_get_driver::start_for_in_query(runtime, execution, id, step, depth)
        }
        Err(error) => {
            super::property_driver::throw_error(runtime, realm, runtime_error_to_vm_error(error))
        }
    }
}

#[inline(never)]
pub(super) fn numeric(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    kind: super::numeric::operation::NumericKind,
) -> Result<CallStep, Error> {
    complete_numeric(runtime, execution, id, kind).map(NumericProgress::into_call_step)
}

#[inline(never)]
pub(super) fn strict_equality(
    _runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    negate: bool,
) -> Result<CallStep, Error> {
    super::run::strict_comparison(execution, id, negate)?;
    Ok(CallStep::Entered)
}

#[inline(never)]
pub(super) fn arguments(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    exit: RunExit,
) -> Result<CallStep, Error> {
    match super::arguments_driver::step(runtime, execution, id, exit)? {
        None => Ok(CallStep::Entered),
        Some(completion) => Ok(CallStep::Complete(completion)),
    }
}

#[inline(never)]
pub(super) fn set_name(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: Option<u32>,
) -> Result<CallStep, Error> {
    match super::property_keys::set_name(runtime, execution, id, index)? {
        None => Ok(CallStep::Entered),
        Some(value) => Ok(CallStep::Complete(Completion::Throw(value))),
    }
}

#[inline(never)]
pub(super) fn instantiate_closure(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: u32,
) -> Result<CallStep, Error> {
    super::closure_driver::instantiate(runtime, execution, id, index)?;
    Ok(CallStep::Entered)
}

#[inline(never)]
pub(super) fn reset_captured(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: u16,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let reusable = std::mem::take(&mut frame.cold.reusable_captured_locals[usize::from(index)]);
    let super::bindings::FrameBinding::Captured(root) =
        execution.slots.local(&frame.window, index)?
    else {
        return Err(Error::internal("captured reset lost its cell"));
    };
    super::bindings::reset_captured_binding(runtime, root, reusable)?;
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("reset resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(execution.slots.depth(&frame.window));
    Ok(CallStep::Entered)
}

#[inline(never)]
pub(super) fn close_captured(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: u16,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    if let Some(flag) = frame
        .cold
        .reusable_captured_locals
        .get_mut(usize::from(index))
    {
        *flag = false;
    }
    super::bindings::close_frame_binding(
        runtime,
        execution.slots.local_mut(&frame.window, index)?,
        frame.executable.local_definitions[usize::from(index)].kind,
    )?;
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("close resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(execution.slots.depth(&frame.window));
    Ok(CallStep::Entered)
}

#[inline(never)]
pub(super) fn catch(
    _runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    exit: RunExit,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let depth = execution.slots.depth(&frame.window);
    if let RunExit::Catch(target) = exit {
        if target as usize >= frame.executable.code.len() {
            return Err(Error::internal("catch target is out of bounds"));
        }
        #[cfg(feature = "profiling")]
        let before = frame.cold.regions.capacity();
        frame
            .cold
            .regions
            .try_reserve(1)
            .map_err(|_| Error::internal("catch region allocation failed"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_capacity(
            "cold.regions",
            before,
            frame.cold.regions.capacity(),
            size_of::<super::VmUnwindRegion>(),
        );
        frame.cold.regions.push(super::VmUnwindRegion::Catch {
            target: target as usize,
            stack_depth: depth,
        });
    } else {
        let Some(super::VmUnwindRegion::Catch { stack_depth, .. }) =
            frame.cold.regions.last().copied()
        else {
            return Err(Error::internal(
                "catch cleanup has no innermost catch region",
            ));
        };
        if exit == RunExit::DropCatch {
            if depth != stack_depth {
                return Err(Error::internal(
                    "DropCatch did not reach its catch entry depth",
                ));
            }
        } else {
            if depth <= stack_depth {
                return Err(Error::internal(
                    "NipCatch has no value above its catch marker",
                ));
            }
            prepare_captured_reuse(frame, &execution.slots)?;
            let value = execution.slots.pop(&mut frame.window)?;
            while execution.slots.depth(&frame.window) > stack_depth {
                drop(execution.slots.pop(&mut frame.window)?);
            }
            execution.slots.push(&mut frame.window, value)?;
        }
        frame.cold.regions.pop();
    }
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("catch resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Entered)
}

#[inline(never)]
pub(super) fn throw(
    _runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let completion = Completion::Throw(execution.slots.pop(&mut frame.window)?);
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Complete(completion))
}

#[inline(never)]
// Binding opcode flags travel with the existing execution borrow instead of creating a second operation representation.
#[allow(clippy::too_many_arguments)]
pub(super) fn binding(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    source: super::run::BindingSource,
    index: u16,
    write: bool,
    checked: bool,
    keep: bool,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    use super::run::BindingSource;
    use crate::engine::code::function::metadata::{
        ClosureSource, ClosureVariable, ClosureVariableName,
    };
    let (root, descriptor) = match source {
        BindingSource::Closure => (
            frame
                .cold
                .closure_slots
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?
                .clone(),
            frame.executable.closure_variables[usize::from(index)],
        ),
        BindingSource::Local | BindingSource::Argument => {
            let (binding, definition, source) = if source == BindingSource::Local {
                (
                    execution.slots.local(&frame.window, index)?,
                    frame.executable.local_definitions[usize::from(index)],
                    ClosureSource::ParentLocal(index),
                )
            } else {
                (
                    execution.slots.parameter(&frame.window, index)?,
                    frame.executable.argument_definitions[usize::from(index)],
                    ClosureSource::ParentArgument(index),
                )
            };
            let super::bindings::FrameBinding::Captured(root) = binding else {
                return Err(Error::internal("captured access lost its cell"));
            };
            (
                root.clone(),
                ClosureVariable {
                    source,
                    name: definition
                        .name
                        .map_or(ClosureVariableName::None, ClosureVariableName::Atom),
                    is_lexical: definition.is_lexical,
                    is_const: definition.is_const,
                    kind: definition.kind,
                },
            )
        }
    };
    let strip_debug = frame.executable.metadata.strip_variable_debug;
    let realm = frame.executable.realm;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let value = if write {
        Some(if keep {
            super::stack::copy_value(execution.slots.peek(&frame.window, 0)?)?
        } else {
            execution.slots.pop(&mut frame.window)?
        })
    } else {
        None
    };
    // Own the cell and input before touching heap storage; no window or
    // Runtime state borrow survives the operation.
    let result = if let Some(value) = value {
        if checked {
            super::bindings::write_checked_closure(runtime, &root, descriptor, strip_debug, value)
        } else {
            runtime
                .write_var_ref(&root, value)
                .map_err(|error| Error::internal(error.to_string()))
        }
        .map(|()| None)
    } else if checked {
        super::bindings::read_checked_closure(runtime, &root, descriptor, strip_debug).map(Some)
    } else {
        runtime
            .read_var_ref(&root)
            .map(Some)
            .map_err(|error| Error::internal(error.to_string()))
    };
    match result {
        Ok(value) => {
            let frame = execution.frames.current_mut(id)?;
            if let Some(value) = value {
                execution.slots.push(&mut frame.window, value)?;
            }
            frame.resume_pc = frame
                .fault_pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("closure access resume PC overflow"))?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(depth);
            Ok(CallStep::Entered)
        }
        Err(error) => {
            let Some(kind) =
                crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
            else {
                return Err(error);
            };
            let value = runtime
                .new_native_error_from_error_jsvalue(realm, kind, &error)
                .map_err(runtime_error_to_vm_error)?;
            Ok(CallStep::Complete(Completion::Throw(value)))
        }
    }
}

#[cold]
#[inline(never)]
pub(super) fn lexical_uninitialized(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: u16,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let definition = frame.executable.local_definitions[usize::from(index)];
    let error = super::bindings::lexical_uninitialized_error(
        runtime,
        definition.name,
        !frame.executable.metadata.strip_variable_debug,
    )?;
    let value = runtime
        .new_native_error_from_error_jsvalue(
            frame.executable.realm,
            crate::engine::api::error::NativeErrorKind::Reference,
            &error,
        )
        .map_err(runtime_error_to_vm_error)?;
    Ok(CallStep::Complete(Completion::Throw(value)))
}

#[inline(never)]
pub(super) fn initialize_derived(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: u16,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let definition = frame
        .executable
        .frame_layout()
        .locals()
        .get(usize::from(index))
        .copied()
        .ok_or_else(|| Error::internal("local definition index is out of bounds"))?;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let value = execution.slots.pop(&mut frame.window)?;
    match super::bindings::initialize_derived_binding(
        runtime,
        definition,
        Some(execution.slots.local(&frame.window, index)?),
        value,
    ) {
        Ok(replacement) => {
            if let Some(binding) = replacement {
                let old = execution
                    .slots
                    .replace_local(&frame.window, index, binding)?;
                if !matches!(old, super::bindings::FrameBinding::Uninitialized) {
                    return Err(Error::internal(
                        "derived initialization replaced an initialized slot",
                    ));
                }
            }
            frame.resume_pc = frame
                .fault_pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("initialization resume PC overflow"))?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(depth);
            Ok(CallStep::Entered)
        }
        Err(error) => {
            let Some(kind) =
                crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
            else {
                return Err(error);
            };
            let value = runtime
                .new_native_error_from_error_jsvalue(frame.executable.realm, kind, &error)
                .map_err(runtime_error_to_vm_error)?;
            Ok(CallStep::Complete(Completion::Throw(value)))
        }
    }
}

#[inline(never)]
pub(super) fn return_derived(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    index: u16,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let definition = frame
        .executable
        .frame_layout()
        .locals()
        .get(usize::from(index))
        .copied()
        .ok_or_else(|| Error::internal("local definition index is out of bounds"))?;
    let value = execution.slots.pop(&mut frame.window)?;
    let completion = super::bindings::finish_derived_return(
        runtime,
        frame.caller_realm,
        definition,
        Some(execution.slots.local(&frame.window, index)?),
        value,
    )?;
    #[cfg(feature = "profiling")]
    if matches!(completion, Completion::Return(_)) {
        crate::engine::api::profiling::record_owned_instruction(
            execution.slots.depth(&frame.window) + 1,
        );
    }
    Ok(CallStep::Complete(completion))
}

#[inline(never)]
pub(super) fn normalize_this(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    // This conversion only allocates a primitive wrapper; it cannot
    // call JavaScript. Keep its identity across every later handoff.
    let value = runtime
        .native_to_object(frame.executable.realm, frame.cold.input.this_value.clone())
        .map_err(runtime_error_to_vm_error)?;
    let NativeConversion::Value(object) = value else {
        return Err(Error::internal("non-null primitive this boxing threw"));
    };
    frame.cold.normalized_this = Some(Value::Object(object));
    Ok(CallStep::Entered)
}
