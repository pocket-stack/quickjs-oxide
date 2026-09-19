//! Shared synchronous iterator calls with operation-specific close and record policies.
mod regions;
pub(super) mod suspension;
use super::{
    Completion,
    driver::CallStep,
    exception::runtime_error_to_vm_error,
    execution::RunningExecution,
    frame::{FrameId, OperationTarget, ReturnTarget},
};
use crate::engine::{
    api::{Error, ErrorKind, error::NativeErrorKind, runtime::Runtime},
    builtins::native::{ArrayIteratorKind, NativeFunctionId},
    heap::{ContextId, ObjectKind, ObjectPayload},
    object::{
        CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
        WellKnownSymbol, operations::PropertyDefineOutcome,
    },
    value::JsValue,
};
pub(super) use regions::unwind;

#[derive(Clone, Copy)]
enum Stage {
    Start,
    Close,
    Finish,
    Probe,
    Method,
    Iterator,
    NextMethod,
    Next,
    Value,
    ReturnMethod,
    ReturnResult,
    AsyncMethod,
    DelegateMethod,
    ResumeResult,
    DoneProperty,
    ValueProperty,
}

#[derive(Clone, Copy)]
enum Mode {
    Append,
    Start {
        asynchronous: bool,
        delegating: bool,
    },
    Invoke,
    Delegate(crate::engine::code::bytecode::IteratorCallKind),
    Parse {
        record_base: usize,
    },
    Next {
        record_base: usize,
    },
    Close {
        _instruction_depth: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Operation {
    Start,
    Suspend(suspension::Operation),
    Next(usize),
    Close,
    ClosePreserve,
    DropPreserve,
    DetachPreserve,
}

pub(super) struct PendingIterator(Box<PendingIteratorState>);
impl std::ops::Deref for PendingIterator {
    type Target = PendingIteratorState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for PendingIterator {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(size_of::<PendingIterator>() <= 8);
pub(super) struct PendingIteratorState {
    mode: Mode,
    yielded: JsValue,
    done: bool,
    frame: FrameId,
    pc: usize,
    generation: u64,
    realm: ContextId,
    array: Option<ObjectRef>,
    iterable: JsValue,
    position: u32,
    stage: Stage,
    builtin_probe: bool,
    iterator: JsValue,
    next: JsValue,
    fast: Option<std::vec::IntoIter<JsValue>>,
    ready: bool,
    abrupt: Option<JsValue>,
    argument: JsValue,
    sync_fallback: bool,
}

/// A query driver consumes these actions in its existing dispatch loop.
pub(super) enum IteratorAction {
    Read(JsValue, PropertyKey),
    Call(CallableRef, JsValue),
    Invoke(super::call::DirectCallTarget, JsValue, Vec<JsValue>),
    Next(CallableRef, JsValue),
    Finish,
}

enum Action {
    Read(JsValue, PropertyKey),
    Call(CallableRef, JsValue),
    Invoke(super::call::DirectCallTarget, JsValue, Vec<JsValue>),
    Next(CallableRef, JsValue),
    Reply(Completion),
    Finish,
}

#[inline(never)]
pub(super) fn start(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let JsValue::Object(array) = execution.slots.peek(&frame.window, 2)? else {
        return Ok(CallStep::Bridge);
    };
    {
        let state = runtime.0.state.borrow();
        let object = state
            .heap
            .object(*array)
            .map_err(|e| Error::internal(e.to_string()))?;
        if !matches!(
            (object.kind, &object.payload),
            (ObjectKind::Ordinary, ObjectPayload::Ordinary)
                | (ObjectKind::Array, ObjectPayload::Array { .. })
        ) {
            return Ok(CallStep::Bridge);
        }
    }
    let JsValue::Int(position) = execution.slots.peek(&frame.window, 1)? else {
        return Ok(CallStep::Bridge);
    };
    let array = crate::engine::object::ObjectRef::from_borrowed_handle(runtime.clone(), *array)
        .map_err(runtime_error_to_vm_error)?;
    let position = *position as u32;
    let iterable = runtime
        .dup_jsvalue(execution.slots.peek(&frame.window, 0)?)
        .map_err(runtime_error_to_vm_error)?;
    let mut pending = PendingIteratorState::new(frame, id, Mode::Append)?;
    pending.array = Some(array);
    pending.position = position;
    pending.iterable = iterable;
    drive_local(runtime, execution, pending, None)
}

#[inline(never)]
pub(super) fn operation(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    operation: Operation,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let pending = match operation {
        Operation::Suspend(operation) => {
            return suspension::start(runtime, execution, id, operation);
        }
        Operation::Start => {
            let mut pending = PendingIteratorState::new(
                frame,
                id,
                Mode::Start {
                    asynchronous: false,
                    delegating: false,
                },
            )?;
            pending.iterable = execution.slots.pop(&mut frame.window)?;
            return drive_local(runtime, execution, pending, None);
        }
        Operation::Next(offset) => {
            let Some(super::VmUnwindRegion::Iterator {
                record_base,
                enabled,
                asynchronous: false,
            }) = frame.cold.regions.last().copied()
            else {
                return Err(Error::internal(
                    "ForOfNext has no innermost synchronous iterator region",
                ));
            };
            let expected = record_base
                .checked_add(2)
                .and_then(|n| n.checked_add(offset))
                .ok_or_else(|| Error::internal("for-of offset overflow"))?;
            if execution.slots.depth(&frame.window) != expected {
                return Err(Error::internal(
                    "ForOfNext offset does not reach its iterator record",
                ));
            }
            if !enabled {
                return finish_next(
                    runtime,
                    execution,
                    id,
                    record_base,
                    JsValue::Undefined,
                    true,
                    None,
                );
            }
            // The record already owns receiver and captured method. Classify
            // before building a general iterator operation or its waiting box.
            let next = execution.slots.peek(&frame.window, offset)?;
            let metadata = if let JsValue::Object(method) = next {
                let state = runtime.0.state.borrow();
                match &state
                    .heap
                    .object(*method)
                    .map_err(|error| runtime_error_to_vm_error(error.into()))?
                    .payload
                {
                    ObjectPayload::NativeFunction { data, .. }
                        if data.target == NativeFunctionId::ArrayIteratorNext =>
                    {
                        Some(())
                    }
                    _ => None,
                }
            } else {
                None
            };
            if metadata.is_some() {
                let callable = callable(
                    runtime,
                    runtime
                        .dup_jsvalue(next)
                        .map_err(runtime_error_to_vm_error)?,
                    "not a function",
                )?;
                let (_, defining_realm, min_readable_args) = runtime
                    .direct_native_callable_metadata(&callable)
                    .map_err(runtime_error_to_vm_error)?
                    .ok_or_else(|| Error::internal("Array-next lost native metadata"))?;
                let iterator = runtime
                    .dup_jsvalue(execution.slots.peek(&frame.window, offset + 1)?)
                    .map_err(runtime_error_to_vm_error)?;
                return super::proxy_get_driver::start_array_next_without_pending(
                    runtime,
                    execution,
                    id,
                    record_base,
                    callable,
                    defining_realm,
                    min_readable_args,
                    iterator,
                );
            }
            let mut pending = PendingIteratorState::new(frame, id, Mode::Next { record_base })?;
            pending.iterator = runtime
                .dup_jsvalue(execution.slots.peek(&frame.window, offset + 1)?)
                .map_err(runtime_error_to_vm_error)?;
            pending.next = runtime
                .dup_jsvalue(execution.slots.peek(&frame.window, offset)?)
                .map_err(runtime_error_to_vm_error)?;
            pending.stage = if enabled { Stage::Next } else { Stage::Finish };
            pending.done = !enabled;
            pending
        }
        _ => {
            let preserve = operation != Operation::Close;
            let instruction_depth = execution.slots.depth(&frame.window);
            let (iterator, enabled, asynchronous) =
                regions::take(runtime, frame, &mut execution.slots, preserve)?;
            if asynchronous && !enabled {
                return Err(Error::internal(
                    "synchronous cleanup targeted a pending async iterator",
                ));
            }
            let mut pending = PendingIteratorState::new(
                frame,
                id,
                Mode::Close {
                    _instruction_depth: instruction_depth,
                },
            )?;
            if operation == Operation::DetachPreserve {
                if !enabled {
                    return Err(Error::internal(
                        "IteratorDetachPreserve targeted a disabled iterator region",
                    ));
                }
                execution.slots.push(&mut frame.window, iterator)?;
                pending.stage = Stage::Finish;
            } else {
                pending.iterator = iterator;
                pending.stage = if enabled && operation != Operation::DropPreserve {
                    Stage::Close
                } else {
                    Stage::Finish
                };
            }
            pending
        }
    };
    // These entries already have an iterator record; their first action has no prior JS reply.
    drive_local(
        runtime,
        execution,
        pending,
        Some(Completion::Return(JsValue::Undefined)),
    )
}

fn close_unwind(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    iterator: JsValue,
    value: JsValue,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let mut pending = PendingIteratorState::new(
        frame,
        id,
        Mode::Close {
            _instruction_depth: execution.slots.depth(&frame.window),
        },
    )?;
    pending.iterator = iterator;
    pending.abrupt = Some(value);
    pending.stage = Stage::Close;
    drive_local(
        runtime,
        execution,
        pending,
        Some(Completion::Return(JsValue::Undefined)),
    )
}

pub(super) fn finish(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    mut pending: PendingIterator,
) -> Result<CallStep, Error> {
    finish_local(runtime, execution, &mut pending.0)
}

fn finish_local(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: &mut PendingIteratorState,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(pending.frame)?;
    #[cfg(feature = "profiling")]
    let depth = match pending.mode {
        Mode::Close { _instruction_depth } => _instruction_depth,
        Mode::Start { .. } => execution.slots.depth(&frame.window) + 1,
        _ => execution.slots.depth(&frame.window),
    };
    match pending.mode {
        Mode::Append => {
            for _ in 0..3 {
                execution.slots.pop(&mut frame.window)?;
            }
            if pending.abrupt.is_none() {
                let array = pending
                    .array
                    .take()
                    .ok_or_else(|| Error::internal("Append lost its target"))?;
                let id = array.object_id();
                runtime
                    .retain_object_handle(id)
                    .map_err(runtime_error_to_vm_error)?;
                execution.slots.push(&mut frame.window, JsValue::Object(id))?;
                execution
                    .slots
                    .push(&mut frame.window, JsValue::Int(pending.position as i32))?;
            }
        }
        Mode::Start {
            asynchronous,
            delegating,
        } => {
            if pending.abrupt.is_none() {
                let record_base = execution.slots.depth(&frame.window);
                if !delegating {
                    #[cfg(feature = "profiling")]
                    let before = frame.cold.regions.capacity();
                    frame
                        .cold
                        .regions
                        .try_reserve(1)
                        .map_err(|_| Error::internal("iterator region allocation failed"))?;
                    #[cfg(feature = "profiling")]
                    crate::engine::api::profiling::record_call_buffer_capacity(
                        "cold.regions",
                        before,
                        frame.cold.regions.capacity(),
                        size_of::<super::VmUnwindRegion>(),
                    );
                }
                execution.slots.push(
                    &mut frame.window,
                    std::mem::replace(&mut pending.iterator, JsValue::Undefined),
                )?;
                execution.slots.push(
                    &mut frame.window,
                    std::mem::replace(&mut pending.next, JsValue::Undefined),
                )?;
                if delegating {
                    execution.slots.push(&mut frame.window, JsValue::Undefined)?;
                } else {
                    frame.cold.regions.push(super::VmUnwindRegion::Iterator {
                        record_base,
                        enabled: true,
                        asynchronous,
                    });
                }
            }
        }
        Mode::Invoke => {
            if pending.abrupt.is_none() {
                execution.slots.push(
                    &mut frame.window,
                    std::mem::replace(&mut pending.yielded, JsValue::Undefined),
                )?;
            }
        }
        Mode::Delegate(_) => {
            if pending.abrupt.is_none() {
                if !pending.done {
                    let old = execution.slots.replace_operand(
                        &frame.window,
                        0,
                        std::mem::replace(&mut pending.yielded, JsValue::Undefined),
                    )?;
                    runtime
                        .release_jsvalue(old)
                        .map_err(runtime_error_to_vm_error)?;
                }
                execution
                    .slots
                    .push(&mut frame.window, JsValue::Bool(pending.done))?;
            }
        }
        Mode::Parse { record_base } => {
            if pending.abrupt.is_none() {
                suspension::enable(frame, record_base)?;
                execution.slots.push(
                    &mut frame.window,
                    std::mem::replace(&mut pending.yielded, JsValue::Undefined),
                )?;
                execution
                    .slots
                    .push(&mut frame.window, JsValue::Bool(pending.done))?;
            }
        }
        Mode::Next { record_base } => {
            apply_next(
                runtime,
                frame,
                &mut execution.slots,
                record_base,
                std::mem::replace(&mut pending.yielded, JsValue::Undefined),
                pending.done,
                pending.abrupt.is_some(),
            )?;
        }
        Mode::Close { .. } => {}
    }
    if let Some(value) = pending.abrupt.take() {
        return Ok(CallStep::Complete(Completion::Throw(value)));
    }
    frame.resume_pc = pending
        .pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("iterator resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Entered)
}

fn apply_next(
    runtime: &Runtime,
    frame: &mut super::frame::Frame,
    slots: &mut super::stack::SlotStore,
    record_base: usize,
    value: JsValue,
    done: bool,
    abrupt: bool,
) -> Result<(), Error> {
    if done || abrupt {
        regions::disable(runtime, frame, slots, record_base)?;
    }
    if !abrupt {
        let mut window = slots.run_window(&mut frame.window)?;
        window.push(value)?;
        window.push(JsValue::Bool(done))?;
    }
    Ok(())
}

pub(super) fn finish_next(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    record_base: usize,
    value: JsValue,
    done: bool,
    abrupt: Option<JsValue>,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    apply_next(
        runtime,
        frame,
        &mut execution.slots,
        record_base,
        value,
        done,
        abrupt.is_some(),
    )?;
    if let Some(value) = abrupt {
        return Ok(CallStep::Complete(Completion::Throw(value)));
    }
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("iterator resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Entered)
}

pub(super) fn next_wait(
    execution: &mut RunningExecution,
    id: FrameId,
    record_base: usize,
) -> Result<PendingIterator, Error> {
    let frame = execution.frames.current_mut(id)?;
    let mut pending = PendingIteratorState::new(frame, id, Mode::Next { record_base })?;
    pending.stage = Stage::Next;
    // The live stack record and the native activation retain the receiver and
    // method. The suspended finish only consumes a raw value/done reply.
    Ok(pending.into_resident())
}

#[inline(never)]
pub(super) fn reply(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    target: ReturnTarget,
    completion: Completion,
) -> Result<CallStep, Error> {
    let pending = execution
        .frames
        .current_mut(target.frame()?)?
        .cold
        .iterator_wait
        .take()
        .ok_or_else(|| Error::internal("iterator reply has no owner"))?;
    if target.operation != Some(OperationTarget::Iterator(pending.generation))
        || pending.frame != target.frame()?
    {
        return Err(Error::internal("iterator reply identity mismatch"));
    }
    drive(runtime, execution, pending, Some(completion))
}

fn materialize(runtime: &Runtime, realm: ContextId, error: Error) -> Result<Completion, Error> {
    let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
        return Err(error);
    };
    Ok(Completion::Throw(
        runtime
            .new_native_error_from_error_jsvalue(realm, kind, &error)
            .map_err(runtime_error_to_vm_error)?,
    ))
}

#[inline(never)]
fn drive_local(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    mut state: PendingIteratorState,
    response: Option<Completion>,
) -> Result<CallStep, Error> {
    let action = state.advance_query(runtime, response)?;
    if matches!(action, IteratorAction::Finish) {
        return finish_local(runtime, execution, &mut state);
    }
    dispatch_action(runtime, execution, state.into_resident(), action)
}
fn drive(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    mut pending: PendingIterator,
    response: Option<Completion>,
) -> Result<CallStep, Error> {
    let action = pending.advance_query(runtime, response)?;
    dispatch_action(runtime, execution, pending, action)
}
fn dispatch_action(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    pending: PendingIterator,
    action: IteratorAction,
) -> Result<CallStep, Error> {
    match action {
        IteratorAction::Finish => finish(runtime, execution, pending),
        IteratorAction::Read(base, key) => {
            super::proxy_get_driver::start_iterator_read(runtime, execution, pending, base, key)
        }
        IteratorAction::Call(callable, receiver) => super::proxy_get_driver::start_iterator_call(
            runtime, execution, pending, callable, receiver,
        ),
        IteratorAction::Invoke(target, receiver, arguments) => {
            super::proxy_get_driver::start_iterator_invoke(
                runtime, execution, pending, target, receiver, arguments,
            )
        }
        IteratorAction::Next(callable, receiver) => super::proxy_get_driver::start_iterator_next(
            runtime, execution, pending, receiver, callable,
        ),
    }
}

impl PendingIteratorState {
    fn into_resident(self) -> PendingIterator {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("iterator_resident_allocated");
        PendingIterator(Box::new(self))
    }

    /// Fold pure iterator transitions and local errors without entering a query.
    pub(super) fn advance_query(
        &mut self,
        runtime: &Runtime,
        mut response: Option<Completion>,
    ) -> Result<IteratorAction, Error> {
        loop {
            let action = match self.advance(runtime, response.take()) {
                Ok(action) => action,
                Err(error) => {
                    response = Some(materialize(runtime, self.realm, error)?);
                    continue;
                }
            };
            match action {
                Action::Reply(completion) => response = Some(completion),
                Action::Finish => return Ok(IteratorAction::Finish),
                Action::Read(base, key) => {
                    if matches!(base, JsValue::Null | JsValue::Undefined) {
                        response = Some(materialize(
                            runtime,
                            self.realm,
                            Error::new(
                                ErrorKind::Type,
                                if matches!(base, JsValue::Null) {
                                    "cannot read property of null"
                                } else {
                                    "cannot read property of undefined"
                                },
                            ),
                        )?);
                        continue;
                    }
                    return Ok(IteratorAction::Read(base, key));
                }
                Action::Call(callable, receiver) => {
                    return Ok(IteratorAction::Call(callable, receiver));
                }
                Action::Invoke(target, receiver, arguments) => {
                    return Ok(IteratorAction::Invoke(target, receiver, arguments));
                }
                Action::Next(callable, receiver) => {
                    return Ok(IteratorAction::Next(callable, receiver));
                }
            }
        }
    }
    pub(super) fn is_next_iteration(&self) -> bool {
        matches!(self.mode, Mode::Next { .. })
    }

    pub(super) fn next_query(
        &mut self,
        runtime: &Runtime,
        reply: crate::engine::builtins::ObjectIteratorStep,
    ) -> Result<IteratorAction, Error> {
        use crate::engine::builtins::ObjectIteratorStep;
        match reply {
            ObjectIteratorStep::Throw(value) => {
                // The builtin iterator reply is a public root; transfer it into
                // the internal completion without a retain/release pair.
                let value = runtime
                    .into_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?;
                self.advance_query(runtime, Some(Completion::Throw(value)))
            }
            ObjectIteratorStep::Done => {
                self.done = true;
                Ok(IteratorAction::Finish)
            }
            ObjectIteratorStep::Yield(value) => {
                self.stage = Stage::Value;
                let value = runtime
                    .into_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?;
                self.advance_query(runtime, Some(Completion::Return(value)))
            }
        }
    }

    pub(super) fn frame(&self) -> FrameId {
        self.frame
    }
    pub(super) fn realm(&self) -> ContextId {
        self.realm
    }

    fn new(frame: &mut super::frame::Frame, id: FrameId, mode: Mode) -> Result<Self, Error> {
        frame.iterator_generation = frame
            .iterator_generation
            .checked_add(1)
            .ok_or_else(|| Error::internal("iterator operation identity exhausted"))?;
        let pending = Self {
            mode,
            yielded: JsValue::Undefined,
            done: false,
            frame: id,
            pc: frame.fault_pc,
            generation: frame.iterator_generation,
            realm: frame.executable.realm,
            array: None,
            iterable: JsValue::Undefined,
            position: 0,
            stage: Stage::Start,
            builtin_probe: false,
            iterator: JsValue::Undefined,
            next: JsValue::Undefined,
            fast: None,
            ready: false,
            abrupt: None,
            argument: JsValue::Undefined,
            sync_fallback: false,
        };
        Ok(pending)
    }

    fn key(
        &self,
        runtime: &Runtime,
        name: crate::engine::atom::pinned::PinnedAtom,
    ) -> Result<PropertyKey, Error> {
        runtime
            .pinned_property_key(name)
            .map_err(|e| Error::internal(e.to_string()))
    }

    fn advance(
        &mut self,
        runtime: &Runtime,
        response: Option<Completion>,
    ) -> Result<Action, Error> {
        let value = match response {
            Some(Completion::Throw(value)) => {
                if self.abrupt.is_some() {
                    return Ok(Action::Finish);
                }
                self.abrupt = Some(value);
                if !matches!(self.mode, Mode::Append) || !self.ready {
                    return Ok(Action::Finish);
                }
                self.stage = Stage::ReturnMethod;
                return Ok(Action::Read(
                    runtime
                        .dup_jsvalue(&self.iterator)
                        .map_err(runtime_error_to_vm_error)?,
                    self.key(runtime, crate::engine::atom::pinned::PinnedAtom::Return)?,
                ));
            }
            Some(Completion::Return(value)) => value,
            None if matches!(self.stage, Stage::Start) => JsValue::Undefined,
            None => return Err(Error::internal("iterator stage lost its reply")),
        };
        match self.stage {
            Stage::Finish => Ok(Action::Finish),
            Stage::Close => {
                self.stage = Stage::ReturnMethod;
                Ok(Action::Read(
                    runtime
                        .dup_jsvalue(&self.iterator)
                        .map_err(runtime_error_to_vm_error)?,
                    self.key(runtime, crate::engine::atom::pinned::PinnedAtom::Return)?,
                ))
            }
            Stage::AsyncMethod
            | Stage::DelegateMethod
            | Stage::ResumeResult
            | Stage::DoneProperty
            | Stage::ValueProperty => self.advance_suspension(runtime, value),
            Stage::Start
                if matches!(
                    self.mode,
                    Mode::Start {
                        asynchronous: true,
                        ..
                    }
                ) =>
            {
                self.stage = Stage::AsyncMethod;
                Ok(Action::Read(
                    runtime
                        .dup_jsvalue(&self.iterable)
                        .map_err(runtime_error_to_vm_error)?,
                    PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::AsyncIterator)),
                ))
            }
            Stage::Start => {
                self.stage = if matches!(self.mode, Mode::Append) {
                    Stage::Probe
                } else {
                    Stage::Method
                };
                Ok(Action::Read(
                    runtime
                        .dup_jsvalue(&self.iterable)
                        .map_err(runtime_error_to_vm_error)?,
                    PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator)),
                ))
            }
            Stage::Probe => {
                self.builtin_probe = super::iterator_support::is_direct_native_target(
                    runtime,
                    &value,
                    NativeFunctionId::ArrayPrototypeIterator(ArrayIteratorKind::Value),
                )?;
                // Release the first result before the second observable GetIterator lookup.
                runtime
                    .release_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?;
                self.stage = Stage::Method;
                Ok(Action::Read(
                    runtime
                        .dup_jsvalue(&self.iterable)
                        .map_err(runtime_error_to_vm_error)?,
                    PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator)),
                ))
            }
            Stage::Method => {
                let callable = callable(
                    runtime,
                    value,
                    if self.sync_fallback {
                        "not a function"
                    } else {
                        "value is not iterable"
                    },
                )?;
                self.stage = Stage::Iterator;
                let receiver = if matches!(self.mode, Mode::Start { .. }) {
                    std::mem::replace(&mut self.iterable, JsValue::Undefined)
                } else {
                    runtime
                        .dup_jsvalue(&self.iterable)
                        .map_err(runtime_error_to_vm_error)?
                };
                Ok(Action::Call(callable, receiver))
            }
            Stage::Iterator => {
                if !matches!(value, JsValue::Object(_)) {
                    return Err(Error::new(ErrorKind::Type, "not an object"));
                }
                self.iterator = value;
                self.stage = Stage::NextMethod;
                Ok(Action::Read(
                    runtime
                        .dup_jsvalue(&self.iterator)
                        .map_err(runtime_error_to_vm_error)?,
                    self.key(runtime, crate::engine::atom::pinned::PinnedAtom::Next)?,
                ))
            }
            Stage::NextMethod if self.sync_fallback => {
                let JsValue::Object(iterator) = self.iterator else {
                    return Err(Error::internal("async fallback lost its iterator"));
                };
                let wrapper = runtime
                    .new_async_from_sync_iterator_jsvalue(self.realm, iterator, &value)
                    .map_err(runtime_error_to_vm_error)?;
                runtime
                    .release_jsvalue(std::mem::replace(
                        &mut self.iterator,
                        JsValue::Undefined,
                    ))
                    .map_err(runtime_error_to_vm_error)?;
                let id = wrapper.object_id();
                runtime
                    .retain_object_handle(id)
                    .map_err(runtime_error_to_vm_error)?;
                self.iterator = JsValue::Object(id);
                self.sync_fallback = false;
                Ok(Action::Read(
                    runtime
                        .dup_jsvalue(&self.iterator)
                        .map_err(runtime_error_to_vm_error)?,
                    self.key(runtime, crate::engine::atom::pinned::PinnedAtom::Next)?,
                ))
            }
            Stage::NextMethod => {
                self.next = value;
                if matches!(self.mode, Mode::Start { .. }) {
                    return Ok(Action::Finish);
                }
                self.fast = super::iterator_support::append_fast_array_values(
                    runtime,
                    &self.iterable,
                    &self.next,
                    self.builtin_probe,
                )?
                .map(Vec::into_iter);
                self.ready = true;
                self.stage = Stage::Next;
                Ok(Action::Reply(Completion::Return(JsValue::Undefined)))
            }
            Stage::Next => {
                if let Some(values) = self.fast.as_mut() {
                    let Some(value) = values.next() else {
                        return Ok(Action::Finish);
                    };
                    self.stage = Stage::Value;
                    return Ok(Action::Reply(Completion::Return(value)));
                }
                let next = callable(
                    runtime,
                    runtime
                        .dup_jsvalue(&self.next)
                        .map_err(runtime_error_to_vm_error)?,
                    "not a function",
                )?;
                Ok(Action::Next(
                    next,
                    runtime
                        .dup_jsvalue(&self.iterator)
                        .map_err(runtime_error_to_vm_error)?,
                ))
            }

            Stage::Value => {
                // The iterator-result object is no longer live when defining the element or closing.
                if matches!(self.mode, Mode::Next { .. }) {
                    self.yielded = value;
                    return Ok(Action::Finish);
                }
                let key = runtime
                    .property_key_for_index(self.position as u64)
                    .map_err(|e| Error::internal(e.to_string()))?;
                let value_root = runtime.root_value(&value).map_err(runtime_error_to_vm_error)?;
                let outcome = runtime
                    .define_own_property_in_realm(
                        Some(self.realm),
                        self.array
                            .as_ref()
                            .ok_or_else(|| Error::internal("Append lost its target"))?,
                        &key,
                        &OrdinaryPropertyDescriptor {
                            value: DescriptorField::Present(value_root),
                            writable: DescriptorField::Present(true),
                            enumerable: DescriptorField::Present(true),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                    )
                    .map_err(runtime_error_to_vm_error)?;
                runtime
                    .release_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?;
                match outcome {
                    PropertyDefineOutcome::Defined(true) => {}
                    PropertyDefineOutcome::Defined(false) => {
                        return Err(Error::new(ErrorKind::Type, "property is not configurable"));
                    }
                    PropertyDefineOutcome::Throw(thrown) => {
                        let thrown = runtime
                            .into_jsvalue(thrown)
                            .map_err(runtime_error_to_vm_error)?;
                        return Ok(Action::Reply(Completion::Throw(thrown)));
                    }
                }
                self.position = self.position.wrapping_add(1);
                self.stage = Stage::Next;
                Ok(Action::Reply(Completion::Return(JsValue::Undefined)))
            }
            Stage::ReturnMethod => {
                if matches!(value, JsValue::Undefined | JsValue::Null) {
                    return Ok(Action::Finish);
                }
                let method = callable(runtime, value, "not a function")?;
                self.stage = Stage::ReturnResult;
                Ok(Action::Call(
                    method,
                    runtime
                        .dup_jsvalue(&self.iterator)
                        .map_err(runtime_error_to_vm_error)?,
                ))
            }
            // With an exception pending, even a primitive return result is ignored.
            Stage::ReturnResult => {
                if self.abrupt.is_none() && !matches!(value, JsValue::Object(_)) {
                    return Err(Error::new(ErrorKind::Type, "not an object"));
                }
                Ok(Action::Finish)
            }
        }
    }
}

fn callable(runtime: &Runtime, value: JsValue, message: &str) -> Result<CallableRef, Error> {
    if let JsValue::Object(object) = value {
        if let Some(callable) = runtime
            .as_callable_object(object)
            .map_err(runtime_error_to_vm_error)?
        {
            return Ok(callable);
        }
    }
    Err(Error::new(ErrorKind::Type, message))
}

#[cfg(all(test, feature = "profiling"))]
#[test]
fn one_resident_owner_per_iterator_operation_and_none_for_disabled_close() {
    use crate::engine::api::profiling::CostProfile;
    for count in [0, 4] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.eval("function collect(n){let i=0;let it={ [Symbol.iterator](){return this},next(){return {get value(){return i},get done(){return i++>=n}}}}; let total=0;for(let x of it)total+=x;return total}").unwrap();
        let profile = CostProfile::start();
        assert_eq!(
            context.eval(&format!("collect({count})")).unwrap(),
            Value::Int(count * (count + 1) / 2)
        );
        let costs = profile.snapshot();
        assert_eq!(
            costs
                .owned_execution_events
                .get("iterator_resident_allocated")
                .copied()
                .unwrap_or(0),
            (count + 2) as u64
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

#[cfg(test)]
mod resident_next_tests {
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn synchronous_array_next_keeps_live_cursor_and_captured_method() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            const a=[1,2,3], it=a.values(); let nested=0;
            Object.defineProperty(a,'0',{get(){nested=it.next().value;return 1;}});
            const iterable={[Symbol.iterator](){return it;}};
            let result='';
            for(const x of iterable){result+=x;if(x===1)it.next=()=>({done:true});}
            const b=[4], seen=[];
            for(const x of b){seen.push(x);if(x===4)b.push(5);}
            return result==='13' && nested===2 && seen.join(',')==='4,5';
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn selected_array_next_getter_is_not_replayed_and_close_policy_is_kept() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let reads=0, closes=0; const marker={}, a=[1], it=a.values();
            Object.defineProperty(a,'0',{get(){reads++;throw marker;}});
            it.return=()=>{closes++;return {};};
            let caught=false;
            try{for(const x of {[Symbol.iterator](){return it;}}){}}catch(e){caught=e===marker;}
            if(!caught || reads!==1 || closes!==0)return false;
            const b=[2], jt=b.values();jt.return=()=>{closes++;return {};};
            try{for(const x of {[Symbol.iterator](){return jt;}}){throw marker;}}catch(e){if(e!==marker)return false;}
            const bad={next:Object.getPrototypeOf([].values()).next,return(){closes++;return {};},[Symbol.iterator](){return this;}};
            let brand=false;try{for(const x of bad){}}catch(e){brand=e instanceof TypeError;}
            return closes===1 && brand;
        })()"#).unwrap(), Value::Bool(true));
    }

    #[test]
    fn resident_non_array_iterator_preserves_getters_close_and_nested_calls() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let log='', n=0;
            const it={
                [Symbol.iterator](){return this;},
                next(){const i=++n;return {
                    get done(){log+='d'+i;return i>2;},
                    get value(){log+='v'+i;return Array.from('x').length+i;}
                };},
                return(){log+='r';return {};}
            };
            let first;[first]=it;
            if(first!==2 || log!=='d1v1r')return false;
            log='';n=0;
            const values=[...it];
            if(values.join(',')!=='2,3' || log!=='d1v1d2v2d3')return false;
            const marker={};it.next=()=>({get done(){throw marker;}});
            try{[first]=it;}catch(e){return e===marker && log==='d1v1d2v2d3';}
            return false;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn array_destructuring_keeps_elision_rest_and_early_close() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let count=0, closed=0;
            function make(){let n=0;return {[Symbol.iterator](){return this;},next(){count++;return {value:++n,done:n>4};},return(){closed++;return {};}};}
            let a,rest; [a,,...rest]=make();
            if(a!==1 || rest.join(',')!=='3,4' || count!==5 || closed!==0)return false;
            [a]=make();
            let x,y,tail;[x,,y,...tail]=[7];
            return a===1 && closed===1 && x===7 && y===undefined && tail.length===0;
        })()"#).unwrap(), Value::Bool(true));
    }
}
