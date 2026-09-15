//! Delegation and async iteration keep the compiler's explicit operand protocol.
use super::{
    Action, CallStep, Error, FrameId, Mode, PendingIteratorState, RunningExecution, Runtime, Stage,
    callable, drive_local as drive, runtime_error_to_vm_error,
};
use crate::engine::api::ErrorKind;
use crate::engine::code::bytecode::IteratorCallKind;
use crate::engine::object::{PropertyKey, WellKnownSymbol};
use crate::engine::value::Value;
use crate::engine::vm::{VmUnwindRegion, frame::Frame};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::vm) enum Operation {
    Start {
        asynchronous: bool,
        delegating: bool,
    },
    Next,
    Call(IteratorCallKind),
    AwaitNext,
    Parse,
}

pub(super) fn start(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    op: Operation,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let mut pending = match op {
        Operation::Start {
            asynchronous,
            delegating,
        } => {
            let mut pending = PendingIteratorState::new(
                frame,
                id,
                Mode::Start {
                    asynchronous,
                    delegating,
                },
            )?;
            pending.iterable = execution.slots.pop(&mut frame.window)?;
            return drive(runtime, execution, pending, None);
        }
        Operation::Next | Operation::Call(_) => {
            let iterator = execution.slots.peek(&frame.window, 3)?.clone();
            let next = execution.slots.peek(&frame.window, 2)?.clone();
            let mode = if let Operation::Call(kind) = op {
                Mode::Delegate(kind)
            } else {
                Mode::Invoke
            };
            let mut pending = PendingIteratorState::new(frame, id, mode)?;
            pending.iterator = iterator;
            pending.next = next;
            pending.argument = if matches!(op, Operation::Next) {
                execution.slots.pop(&mut frame.window)?
            } else {
                execution.slots.peek(&frame.window, 0)?.clone()
            };
            pending
        }
        Operation::AwaitNext | Operation::Parse => {
            let Some(VmUnwindRegion::Iterator {
                record_base,
                enabled,
                asynchronous: true,
            }) = frame.cold.regions.last().copied()
            else {
                return Err(Error::internal(
                    "async iterator operation has no innermost async region",
                ));
            };
            let expected_enabled = matches!(op, Operation::AwaitNext);
            let extra = usize::from(matches!(op, Operation::Parse));
            if enabled != expected_enabled
                || record_base.checked_add(2 + extra) != Some(execution.slots.depth(&frame.window))
            {
                return Err(Error::internal(
                    "async iterator operation has invalid region state or stack depth",
                ));
            }
            let mode = if matches!(op, Operation::Parse) {
                Mode::Parse { record_base }
            } else {
                Mode::Invoke
            };
            let mut pending = PendingIteratorState::new(frame, id, mode)?;
            if matches!(op, Operation::Parse) {
                pending.iterator = execution.slots.pop(&mut frame.window)?;
                if !matches!(pending.iterator, Value::Object(_)) {
                    pending.stage = Stage::Finish;
                    let error = super::materialize(
                        runtime,
                        pending.realm,
                        Error::new(ErrorKind::Type, "iterator must return an object"),
                    )?;
                    return drive(runtime, execution, pending, Some(error));
                }
            } else {
                pending.iterator = execution.slots.peek(&frame.window, 1)?.clone();
                pending.next = execution.slots.peek(&frame.window, 0)?.clone();
                let Some(VmUnwindRegion::Iterator { enabled, .. }) = frame.cold.regions.last_mut()
                else {
                    unreachable!()
                };
                *enabled = false;
            }
            pending
        }
    };
    let action = match op {
        Operation::Next | Operation::AwaitNext => {
            pending.stage = Stage::ResumeResult;
            let target = match runtime
                .direct_call_target_from_value(pending.next.clone())
                .map_err(runtime_error_to_vm_error)
            {
                Ok(target) => target,
                Err(error) => {
                    let completion = super::materialize(runtime, pending.realm, error)?;
                    return drive(runtime, execution, pending, Some(completion));
                }
            };
            let arguments = if matches!(op, Operation::Next) {
                vec![std::mem::replace(&mut pending.argument, Value::Undefined)]
            } else {
                Vec::new()
            };
            let receiver = pending.iterator.clone();
            return crate::engine::vm::proxy_get_driver::start_iterator_invoke(
                runtime,
                execution,
                pending.into_resident(),
                target,
                receiver,
                arguments,
            );
        }
        Operation::Call(kind) => {
            pending.stage = Stage::DelegateMethod;
            let name = if kind == IteratorCallKind::ThrowWithValue {
                "throw"
            } else {
                "return"
            };
            pending.key(runtime, name)?
        }
        Operation::Parse => {
            pending.stage = Stage::DoneProperty;
            pending.key(runtime, "done")?
        }
        Operation::Start { .. } => unreachable!(),
    };
    let receiver = pending.iterator.clone();
    crate::engine::vm::proxy_get_driver::start_iterator_read(
        runtime,
        execution,
        pending.into_resident(),
        receiver,
        action,
    )
}

pub(super) fn enable(frame: &mut Frame, base: usize) -> Result<(), Error> {
    let Some(VmUnwindRegion::Iterator {
        record_base,
        enabled,
        asynchronous: true,
    }) = frame.cold.regions.last_mut()
    else {
        return Err(Error::internal("iterator parse lost its async region"));
    };
    if *record_base != base || *enabled {
        return Err(Error::internal("iterator parse region changed"));
    }
    *enabled = true;
    Ok(())
}

impl PendingIteratorState {
    pub(super) fn advance_suspension(
        &mut self,
        runtime: &Runtime,
        value: Value,
    ) -> Result<Action, Error> {
        match self.stage {
            Stage::AsyncMethod => {
                self.stage = Stage::Method;
                if matches!(value, Value::Undefined | Value::Null) {
                    self.sync_fallback = true;
                    Ok(Action::Read(
                        self.iterable.clone(),
                        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator)),
                    ))
                } else {
                    let method = callable(runtime, value, "value is not iterable")?;
                    self.stage = Stage::Iterator;
                    Ok(Action::Call(
                        method,
                        std::mem::replace(&mut self.iterable, Value::Undefined),
                    ))
                }
            }
            Stage::DelegateMethod => {
                if matches!(value, Value::Undefined | Value::Null) {
                    self.done = true;
                    return Ok(Action::Finish);
                }
                let Mode::Delegate(kind) = self.mode else {
                    return Err(Error::internal("delegate method has wrong mode"));
                };
                self.stage = Stage::ResumeResult;
                let target = runtime
                    .direct_call_target_from_value(value)
                    .map_err(runtime_error_to_vm_error)?;
                let arguments = if kind == IteratorCallKind::ReturnWithoutValue {
                    Vec::new()
                } else {
                    vec![std::mem::replace(&mut self.argument, Value::Undefined)]
                };
                Ok(Action::Invoke(target, self.iterator.clone(), arguments))
            }
            Stage::ResumeResult | Stage::ValueProperty => {
                self.yielded = value;
                Ok(Action::Finish)
            }
            Stage::DoneProperty => {
                self.done = runtime
                    .value_to_boolean(&value)
                    .map_err(runtime_error_to_vm_error)?;
                self.stage = Stage::ValueProperty;
                // Unlike sync ForOfNext, value is read even when done is true.
                Ok(Action::Read(
                    self.iterator.clone(),
                    self.key(runtime, "value")?,
                ))
            }
            _ => Err(Error::internal("unexpected suspension iterator reply")),
        }
    }
}
