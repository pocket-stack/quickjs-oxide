//! Owned private-field and method-identity operations. Accessors use a separate
//! call protocol; this step never invokes JavaScript or replays consumed operands.
use super::exception::runtime_error_to_vm_error;
use super::private_bindings::{self, PrivateSource};
use super::{execution::RunningExecution, frame::FrameId};
use crate::engine::api::{
    error::{Error, ErrorKind, NativeErrorKind},
    runtime::Runtime,
};
use crate::engine::code::bytecode::PrivateNameSource;
use crate::engine::code::function::metadata::ClosureVariableKind;
use crate::engine::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Access {
    Get,
    GetKeep,
    Put,
    Define,
    In,
}
pub(super) enum Outcome {
    Entered,
    Done,
    Throw(Value),
}

#[inline(never)]
pub(super) fn step(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    source: PrivateNameSource,
    access: Access,
) -> Result<Outcome, Error> {
    let frame = execution.frames.current_mut(id)?;
    let kind = match source {
        PrivateNameSource::Local(index) => {
            private_bindings::validate_definition(
                frame.executable.local_definitions[usize::from(index)],
            )?
            .1
        }
        PrivateNameSource::Closure(index) => private_bindings::validate_descriptor(
            frame.executable.closure_variables[usize::from(index)],
        )?,
    };
    let accessor_call = matches!(
        kind,
        ClosureVariableKind::PrivateGetter | ClosureVariableKind::PrivateGetterSetter
    ) && matches!(access, Access::Get | Access::GetKeep)
        || kind == ClosureVariableKind::PrivateSetter && access == Access::Put;
    if accessor_call {
        let realm = frame.executable.realm;
        return match enter_accessor(runtime, execution, id, source, access, kind) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(error);
                };
                Ok(Outcome::Throw(
                    runtime
                        .new_native_error_from_error(realm, kind, &error)
                        .map_err(runtime_error_to_vm_error)?,
                ))
            }
        };
    }
    let callable_identity_operation = super::bindings::is_private_callable_kind(kind)
        && match access {
            Access::In => true,
            Access::Put => kind != ClosureVariableKind::PrivateSetter,
            Access::Get | Access::GetKeep => matches!(
                kind,
                ClosureVariableKind::PrivateMethod | ClosureVariableKind::PrivateSetter
            ),
            Access::Define => false,
        };
    if kind != ClosureVariableKind::PrivateField && !callable_identity_operation {
        return Err(Error::internal(
            "private-field definition referenced a non-field binding",
        ));
    }
    let realm = frame.executable.realm;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let result = (|| -> Result<(), Error> {
        let value = if matches!(access, Access::Put | Access::Define) {
            Some(execution.slots.pop(&mut frame.window)?)
        } else {
            None
        };
        let base = execution.slots.pop(&mut frame.window)?;
        let source = match source {
            PrivateNameSource::Local(index) => PrivateSource::Local(
                frame.executable.local_definitions[usize::from(index)],
                execution.slots.local(&frame.window, index)?,
            ),
            PrivateNameSource::Closure(index) => PrivateSource::Closure(
                frame.executable.closure_variables[usize::from(index)],
                frame
                    .cold
                    .closure_slots
                    .get(usize::from(index))
                    .ok_or_else(|| Error::internal("private closure slot is out of bounds"))?,
            ),
        };
        if kind != ClosureVariableKind::PrivateField {
            if access == Access::Put
                || kind == ClosureVariableKind::PrivateSetter
                    && matches!(access, Access::Get | Access::GetKeep)
            {
                let name = match source {
                    PrivateSource::Local(definition, _) => definition.name,
                    PrivateSource::Closure(descriptor, _) => match descriptor.name {
                        crate::engine::code::function::metadata::ClosureVariableName::Atom(
                            name,
                        ) => Some(name),
                        _ => None,
                    },
                };
                return Err(super::bindings::lexical_read_only_error(runtime, name)?);
            }
            if access == Access::In {
                let Value::Object(receiver) = base else {
                    return Err(Error::new(ErrorKind::Type, "invalid 'in' operand"));
                };
                let present = if let Some(method) =
                    private_bindings::optional_callable(runtime, source, kind)?
                {
                    if kind == ClosureVariableKind::PrivateSetter {
                        return Err(Error::internal(
                            "private-in referenced a synthetic setter cell",
                        ));
                    }
                    runtime
                        .check_private_method_brand(&method, &receiver, kind)
                        .map_err(runtime_error_to_vm_error)?
                } else {
                    let key = runtime
                        .intern_property_key("[unsupported type]")
                        .map_err(|error| Error::internal(error.to_string()))?;
                    runtime
                        .has_own_property(&receiver, &key)
                        .map_err(runtime_error_to_vm_error)?
                };
                execution
                    .slots
                    .push(&mut frame.window, Value::Bool(present))?;
            } else {
                let method = private_bindings::optional_callable(runtime, source, kind)?
                    .ok_or_else(|| Error::new(ErrorKind::Type, "not an object"))?;
                let receiver = private_bindings::branded_receiver(runtime, &method, kind, base)?;
                if access == Access::GetKeep {
                    execution
                        .slots
                        .push(&mut frame.window, Value::Object(receiver))?;
                }
                execution
                    .slots
                    .push(&mut frame.window, Value::Object(method.as_object().clone()))?;
            }
            return Ok(());
        }
        let Value::Object(receiver) = base else {
            return Err(Error::new(
                ErrorKind::Type,
                if access == Access::In {
                    "invalid 'in' operand"
                } else {
                    "not an object"
                },
            ));
        };
        let name = private_bindings::optional_field_name(runtime, source)?;
        if access == Access::In {
            let present = if let Some(name) = name {
                runtime
                    .has_private_field_own(&receiver, &name)
                    .map_err(runtime_error_to_vm_error)?
            } else {
                let key = runtime
                    .intern_property_key("[unsupported type]")
                    .map_err(|error| Error::internal(error.to_string()))?;
                runtime
                    .has_own_property(&receiver, &key)
                    .map_err(runtime_error_to_vm_error)?
            };
            execution
                .slots
                .push(&mut frame.window, Value::Bool(present))?;
        } else {
            let name = name.ok_or_else(|| Error::new(ErrorKind::Type, "not a symbol"))?;
            match access {
                Access::Get | Access::GetKeep => {
                    let value = runtime
                        .get_private_field_own(&receiver, &name)
                        .map_err(runtime_error_to_vm_error)?;
                    if access == Access::GetKeep {
                        execution
                            .slots
                            .push(&mut frame.window, Value::Object(receiver))?;
                    }
                    execution.slots.push(&mut frame.window, value)?;
                }
                Access::Put => runtime
                    .set_private_field_own(&receiver, &name, value.unwrap())
                    .map_err(runtime_error_to_vm_error)?,
                Access::Define => {
                    runtime
                        .define_private_field_own(&receiver, &name, value.unwrap())
                        .map_err(runtime_error_to_vm_error)?;
                    execution
                        .slots
                        .push(&mut frame.window, Value::Object(receiver))?;
                }
                Access::In => unreachable!(),
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            frame.resume_pc = frame
                .fault_pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("private access resume PC overflow"))?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(depth);
            Ok(Outcome::Done)
        }
        Err(error) => {
            let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                return Err(error);
            };
            Ok(Outcome::Throw(
                runtime
                    .new_native_error_from_error(realm, kind, &error)
                    .map_err(runtime_error_to_vm_error)?,
            ))
        }
    }
}

/// Resolve and brand-check once, then retain the selected accessor in the
/// shared call driver across native, Proxy, bound, and bytecode targets.
#[inline(never)]
fn enter_accessor(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    source: PrivateNameSource,
    access: Access,
    kind: ClosureVariableKind,
) -> Result<Outcome, Error> {
    use super::frame::ReturnValue;
    let frame = execution.frames.current_mut(id)?;
    let binding = match source {
        PrivateNameSource::Local(index) => PrivateSource::Local(
            frame.executable.local_definitions[usize::from(index)],
            execution.slots.local(&frame.window, index)?,
        ),
        PrivateNameSource::Closure(index) => PrivateSource::Closure(
            frame.executable.closure_variables[usize::from(index)],
            frame
                .cold
                .closure_slots
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("private closure slot is out of bounds"))?,
        ),
    };
    let callable = private_bindings::optional_callable(runtime, binding, kind)?
        .ok_or_else(|| Error::new(ErrorKind::Type, "not an object"))?;
    let setter = access == Access::Put;
    let base = execution
        .slots
        .peek(&frame.window, usize::from(setter))?
        .clone();
    let receiver = private_bindings::branded_receiver(runtime, &callable, kind, base)?;
    if setter {
        runtime
            .validate_value_domain(execution.slots.peek(&frame.window, 0)?, "call argument")
            .map_err(runtime_error_to_vm_error)?;
    }
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let frame = execution.frames.current_mut(id)?;
    let mut arguments = Vec::new();
    if setter {
        arguments
            .try_reserve_exact(1)
            .map_err(|_| Error::internal("setter arguments allocation failed"))?;
        arguments.push(execution.slots.pop(&mut frame.window)?);
    }
    let base = execution.slots.pop(&mut frame.window)?;
    if access == Access::GetKeep {
        execution.slots.push(&mut frame.window, base)?;
    }
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("accessor resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    match super::proxy_get_driver::start_vm_call(
        runtime,
        execution,
        id,
        callable,
        Value::Object(receiver),
        arguments,
        if setter {
            ReturnValue::Discard
        } else {
            ReturnValue::Push
        },
    )? {
        super::driver::CallStep::Entered => Ok(Outcome::Entered),
        super::driver::CallStep::Complete(super::Completion::Throw(value)) => {
            Ok(Outcome::Throw(value))
        }
        _ => Err(Error::internal(
            "private accessor call returned an invalid transition",
        )),
    }
}
