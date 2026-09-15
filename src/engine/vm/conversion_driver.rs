//! Scheduling for a pending addition or unary-plus conversion. Domain phases remain in
//! value/conversion; only frame installation and reply routing live here.
mod local_add;
use crate::engine::api::{error::Error, runtime::Runtime};
use crate::engine::code::function::metadata::FunctionKind;
use crate::engine::object::{CallableRef, OrdinaryRead};
use crate::engine::value::Value;
use crate::engine::value::conversion::primitive::{PrimitiveResume, PrimitiveStep};
use crate::engine::vm::call::{BytecodeCallRequest, CallableExecution};
use crate::engine::vm::exception::runtime_error_to_vm_error;
use crate::engine::vm::execution::RunningExecution;
use crate::engine::vm::frame::{FrameId, ReturnTarget};
use crate::engine::vm::{Completion, ToPrimitiveHint};
pub(super) use local_add::complete_local_add;

enum Finish {
    Predicate(Option<Box<super::predicate_driver::Input>>),
    SuperProperty(Option<Box<super::super_property_driver::Input>>),
    Plus,
    PropertyKey,
    PropertyWrite {
        base: Value,
        value: Value,
    },
    PropertyRead {
        base: Value,
        keep_receiver: bool,
        keep_key: bool,
    },
    AddLeft(Value),
    AddRight(Value),
}

/// The same resident state moves between task and wait as one pointer.
pub(super) struct ConversionWait(ConversionTask);
pub(super) struct ConversionTask(Option<Box<ConversionState>>);
pub(super) struct ConversionState {
    finish: Finish,
    frame: FrameId,
    identity: u64,
    step: Option<PrimitiveStep>,
    resume: Option<PrimitiveResume>,
}
impl std::ops::Deref for ConversionTask {
    type Target = ConversionState;
    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("completed conversion has no resident state")
    }
}
impl std::ops::DerefMut for ConversionTask {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
            .as_mut()
            .expect("completed conversion has no resident state")
    }
}
impl std::ops::Deref for ConversionWait {
    type Target = ConversionState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
const _: () = assert!(size_of::<ConversionTask>() <= 8);
const _: () = assert!(size_of::<ConversionWait>() <= 8);

pub(super) enum Progress {
    Predicate(Box<super::predicate_driver::Input>),
    SuperProperty(Box<super::super_property_driver::Input>),
    Ready(ConversionTask),
    Entered,
    Complete(Completion),
    PropertyRead(Box<super::property_driver::ConvertedRead>),
    PropertyWrite(Box<super::property_write_driver::ConvertedWrite>),
}

fn property_key_primitive(runtime: &Runtime, value: Value) -> Result<Value, Error> {
    Ok(match value {
        Value::Symbol(symbol) => {
            if !symbol.belongs_to(runtime) {
                return Err(Error::internal(
                    "computed property symbol belongs to another runtime",
                ));
            }
            Value::Symbol(symbol)
        }
        Value::String(string) => Value::String(string),
        primitive => Value::String(primitive.to_js_string()?),
    })
}

fn add_completion(
    runtime: &Runtime,
    realm: crate::engine::heap::ContextId,
    left: Value,
    right: Value,
) -> Result<Completion, Error> {
    match super::numeric::add_primitives(left, right) {
        Ok(value) => Ok(Completion::Return(value)),
        Err(error) => {
            let Some(kind) =
                crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
            else {
                return Err(error);
            };
            Ok(Completion::Throw(
                runtime
                    .new_native_error_from_error(realm, kind, &error)
                    .map_err(runtime_error_to_vm_error)?,
            ))
        }
    }
}

pub(super) enum PrimitiveCompletion {
    Completed,
    Throw(Value),
    Declined,
    InvalidDomain,
}

/// The fault PC is published before entry. The input borrow owns no values
/// across parsing, allocation, error materialization or final-owner release.
pub(super) fn complete_primitives(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    addition: bool,
    next_operation: &mut u64,
) -> Result<PrimitiveCompletion, Error> {
    let frame = execution.frames.current_mut(id)?;
    let body = &mut *frame.cold;
    let executable = &*body.executable;
    let realm = executable.realm;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&body.window);
    let mut transaction = execution.slots.frame_transaction(&mut body.window)?;
    let (left, right, store) = {
        let mut slots = transaction.slots();
        // Preserve left-to-right domain validation, including checking a later
        // malformed slot after an earlier invalid domain, before identity issue.
        let mut invalid = false;
        for offset in (0..=usize::from(addition)).rev() {
            invalid |= runtime
                .validate_value_domain(slots.peek(offset)?, "conversion operand")
                .is_err();
        }
        if invalid {
            return Ok(PrimitiveCompletion::InvalidDomain);
        }
        *next_operation = next_operation
            .checked_add(1)
            .ok_or_else(|| Error::internal("conversion identity exhausted"))?;
        let store = if addition && executable.fusion.add_store(frame.fault_pc) {
            use crate::engine::code::bytecode::Instruction;
            match executable.code.get(frame.fault_pc + 1) {
                Some(
                    Instruction::PutLocal(index)
                    | Instruction::PutLocalCheck(index)
                    | Instruction::SetLocal(index)
                    | Instruction::SetLocalCheck(index),
                ) if matches!(
                    slots.local(*index)?,
                    super::bindings::FrameBinding::Direct(_)
                ) =>
                {
                    Some((
                        *index,
                        executable.fusion.add_store_span(frame.fault_pc),
                        matches!(
                            slots.local(*index)?,
                            super::bindings::FrameBinding::Direct(
                                Value::Object(_) | Value::Symbol(_)
                            )
                        ),
                    ))
                }
                _ => None,
            }
        } else {
            None
        };
        for offset in 0..=usize::from(addition) {
            if matches!(slots.peek(offset)?, Value::Object(_)) {
                return Ok(PrimitiveCompletion::Declined);
            }
        }
        let right = slots.pop().expect("validated primitive conversion operand");
        let left = if addition {
            Some(
                slots
                    .pop()
                    .expect("validated primitive conversion left operand"),
            )
        } else {
            None
        };
        (left, right, store)
    };
    // End the authenticated window before String/BigInt allocation or release.
    let completion = if let Some(left) = left {
        add_completion(runtime, realm, left, right)?
    } else {
        match super::numeric::unary_plus_primitive(right) {
            Ok(value) => Completion::Return(value),
            Err(error) => {
                let Some(kind) =
                    crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
                else {
                    return Err(error);
                };
                Completion::Throw(
                    runtime
                        .new_native_error_from_error(realm, kind, &error)
                        .map_err(runtime_error_to_vm_error)?,
                )
            }
        }
    };
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event(
        "conversion_completed_without_task",
    );
    Ok(match completion {
        Completion::Return(value) => {
            if let Some((index, span, observable_release)) = store {
                // Scalar/String/BigInt release cannot observe the runtime PC.
                // Object/Symbol release retains the canonical store publication
                // before replacement. No RunSlots borrow crosses either release.
                let store_pc = frame
                    .fault_pc
                    .checked_add(1)
                    .ok_or_else(|| Error::internal("conversion resume PC overflow"))?;
                if observable_release {
                    (frame.fault_pc, frame.resume_pc) = (store_pc, store_pc);
                    runtime
                        .update_active_bytecode_pc(
                            frame.active_frame,
                            super::BytecodePc::new(frame.fault_pc),
                        )
                        .map_err(runtime_error_to_vm_error)?;
                }
                let mut pending = Some(super::bindings::FrameBinding::Direct(value));
                let old = {
                    let mut slots = transaction.slots();
                    slots.replace_local_pending(index, &mut pending)
                };
                let old = match old {
                    Ok(old) => old,
                    Err(error) => {
                        if !observable_release {
                            (frame.fault_pc, frame.resume_pc) = (store_pc, store_pc);
                            runtime
                                .update_active_bytecode_pc(
                                    frame.active_frame,
                                    super::BytecodePc::new(frame.fault_pc),
                                )
                                .map_err(runtime_error_to_vm_error)?;
                        }
                        return Err(error);
                    }
                };
                drop(old);
                // The optional Drop only removes the assignment result while
                // the local keeps the value; it has no observable owner drain.
                let resume = store_pc
                    .checked_add(span - 1)
                    .ok_or_else(|| Error::internal("binding release resume PC overflow"))?;
                (frame.fault_pc, frame.resume_pc) = (resume - 1, resume);
                #[cfg(feature = "profiling")]
                {
                    crate::engine::api::profiling::record_owned_instruction(depth);
                    crate::engine::api::profiling::record_owned_instruction(depth - 1);
                    if span == 3 {
                        crate::engine::api::profiling::record_owned_instruction(depth - 1);
                    }
                    crate::engine::api::profiling::record_owned_execution_event(
                        "primitive_add_store_fused",
                    );
                }
            } else {
                let mut pending = Some(value);
                {
                    let mut slots = transaction.slots();
                    slots.push_pending(&mut pending)?;
                }
                frame.resume_pc = frame
                    .fault_pc
                    .checked_add(1)
                    .ok_or_else(|| Error::internal("conversion resume PC overflow"))?;
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_instruction(depth);
            }
            PrimitiveCompletion::Completed
        }
        Completion::Throw(value) => PrimitiveCompletion::Throw(value),
    })
}

impl ConversionTask {
    #[inline(always)]
    fn new(finish: Finish, frame: FrameId, identity: u64, step: PrimitiveStep) -> Self {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("conversion_task_allocated");
        Self(Some(Box::new(ConversionState {
            finish,
            frame,
            identity,
            step: Some(step),
            resume: None,
        })))
    }
    fn with_step(mut self, step: PrimitiveStep) -> Self {
        self.step = Some(step);
        self
    }
    fn waiting(mut self, resume: PrimitiveResume) -> ConversionWait {
        self.resume = Some(resume);
        ConversionWait(self)
    }

    #[cfg(feature = "profiling")]
    pub(super) fn operand_count(&self) -> usize {
        if self.0.is_none() {
            return 0;
        }
        match &self.finish {
            Finish::SuperProperty(input) => input
                .as_ref()
                .expect("super conversion input")
                .operand_count(),
            Finish::Plus | Finish::PropertyKey => 1,
            Finish::PropertyWrite { .. } => 3,
            _ => 2,
        }
    }

    pub(super) fn start(
        runtime: &Runtime,
        execution: &mut RunningExecution,
        frame: FrameId,
        identity: u64,
        addition: bool,
        property_key: bool,
    ) -> Result<Self, Error> {
        let parent = execution.frames.current_mut(frame)?;
        let right = execution.slots.pop(&mut parent.window)?;
        if property_key && !addition && !matches!(right, Value::Object(_)) {
            let value = property_key_primitive(runtime, right)?;
            #[cfg(feature = "profiling")]
            let depth = execution.slots.depth(&parent.window) + 1;
            execution.slots.push(&mut parent.window, value)?;
            parent.resume_pc = parent
                .fault_pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("conversion resume PC overflow"))?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(depth);
            return Ok(Self(None));
        }
        let (value, finish, hint) = if addition {
            (
                execution.slots.pop(&mut parent.window)?,
                Finish::AddLeft(right),
                ToPrimitiveHint::Default,
            )
        } else if property_key {
            (right, Finish::PropertyKey, ToPrimitiveHint::String)
        } else {
            (right, Finish::Plus, ToPrimitiveHint::Number)
        };
        Ok(Self::new(
            finish,
            frame,
            identity,
            PrimitiveResume::start(runtime, parent.executable.realm, value, hint),
        ))
    }

    pub(super) fn start_predicate(
        runtime: &Runtime,
        execution: &mut RunningExecution,
        frame: FrameId,
        identity: u64,
        input: Box<super::predicate_driver::Input>,
    ) -> Result<Self, Error> {
        let realm = execution.frames.current_mut(frame)?.executable.realm;
        let step =
            PrimitiveResume::start(runtime, realm, input.key.clone(), ToPrimitiveHint::String);
        Ok(Self::new(
            Finish::Predicate(Some(input)),
            frame,
            identity,
            step,
        ))
    }

    pub(super) fn start_super_property(
        runtime: &Runtime,
        execution: &mut RunningExecution,
        frame: FrameId,
        identity: u64,
        input: Box<super::super_property_driver::Input>,
    ) -> Result<Self, Error> {
        let realm = execution.frames.current_mut(frame)?.executable.realm;
        let step =
            PrimitiveResume::start(runtime, realm, input.key.clone(), ToPrimitiveHint::String);
        Ok(Self::new(
            Finish::SuperProperty(Some(input)),
            frame,
            identity,
            step,
        ))
    }

    pub(super) fn start_property_write(
        runtime: &Runtime,
        execution: &mut RunningExecution,
        frame: FrameId,
        identity: u64,
    ) -> Result<Self, Error> {
        let parent = execution.frames.current_mut(frame)?;
        for (offset, label) in [
            (2, "property receiver"),
            (1, "property key"),
            (0, "property value"),
        ] {
            runtime
                .validate_value_domain(execution.slots.peek(&parent.window, offset)?, label)
                .map_err(runtime_error_to_vm_error)?;
        }
        let value = execution.slots.pop(&mut parent.window)?;
        let key = execution.slots.pop(&mut parent.window)?;
        let base = execution.slots.pop(&mut parent.window)?;
        Ok(Self::new(
            Finish::PropertyWrite { base, value },
            frame,
            identity,
            PrimitiveResume::start(
                runtime,
                parent.executable.realm,
                key,
                ToPrimitiveHint::String,
            ),
        ))
    }

    pub(super) fn start_property_read(
        runtime: &Runtime,
        execution: &mut RunningExecution,
        frame: FrameId,
        identity: u64,
        keep_receiver: bool,
        keep_key: bool,
    ) -> Result<Self, Error> {
        let parent = execution.frames.current_mut(frame)?;
        runtime
            .validate_value_domain(
                execution.slots.peek(&parent.window, 1)?,
                "property receiver",
            )
            .map_err(runtime_error_to_vm_error)?;
        runtime
            .validate_value_domain(execution.slots.peek(&parent.window, 0)?, "property key")
            .map_err(runtime_error_to_vm_error)?;
        let key = execution.slots.pop(&mut parent.window)?;
        let base = execution.slots.pop(&mut parent.window)?;
        Ok(Self::new(
            Finish::PropertyRead {
                base,
                keep_receiver,
                keep_key,
            },
            frame,
            identity,
            PrimitiveResume::start(
                runtime,
                parent.executable.realm,
                key,
                ToPrimitiveHint::String,
            ),
        ))
    }

    pub(super) fn reply(
        runtime: &Runtime,
        execution: &mut RunningExecution,
        target: ReturnTarget,
        completion: Completion,
    ) -> Result<Self, Error> {
        let parent = execution.frames.current_mut(target.frame()?)?;
        let wait = parent
            .cold
            .conversion
            .take()
            .ok_or_else(|| Error::internal("conversion reply has no pending owner"))?;
        if target.operation != Some(super::frame::OperationTarget::Conversion(wait.identity)) {
            return Err(Error::internal("conversion reply identity mismatch"));
        }
        Self::from_wait(runtime, target.frame()?, wait, completion)
    }

    pub(super) fn from_wait(
        runtime: &Runtime,
        frame: FrameId,
        wait: ConversionWait,
        completion: Completion,
    ) -> Result<Self, Error> {
        let mut task = wait.0;
        task.frame = frame;
        let resume = task
            .resume
            .take()
            .ok_or_else(|| Error::internal("conversion wait lost its resume"))?;
        task.step = Some(
            resume
                .resume(runtime, completion)
                .map_err(runtime_error_to_vm_error)?,
        );
        Ok(task)
    }

    pub(super) fn advance(
        mut self,
        runtime: &Runtime,
        execution: &mut RunningExecution,
    ) -> Result<Progress, Error> {
        if self.0.is_none() {
            return Ok(Progress::Entered);
        }
        let frame = self.frame;
        let step = self
            .step
            .take()
            .ok_or_else(|| Error::internal("conversion task lost its step"))?;
        let realm = execution.frames.current_mut(frame)?.executable.realm;
        match step {
            PrimitiveStep::Complete(completion) => {
                let completion = match completion {
                    Completion::Throw(value) => Completion::Throw(value),
                    Completion::Return(value) => {
                        // Domain completion guarantees a primitive: this call
                        // cannot recursively perform another ToPrimitive.
                        if matches!(value, Value::Object(_)) {
                            return Err(Error::internal("conversion returned an object"));
                        }
                        match &mut self.finish {
                            Finish::AddLeft(right) => {
                                let right = std::mem::replace(right, Value::Undefined);
                                if !matches!(right, Value::Object(_)) {
                                    #[cfg(feature = "profiling")]
                                    crate::engine::api::profiling::record_owned_execution_event(
                                        "add_completed_with_primitive_rhs",
                                    );
                                    return Ok(Progress::Complete(add_completion(
                                        runtime, realm, value, right,
                                    )?));
                                }
                                self.finish = Finish::AddRight(value);
                                return Ok(Progress::Ready(self.with_step(
                                    PrimitiveResume::start(
                                        runtime,
                                        realm,
                                        right,
                                        ToPrimitiveHint::Default,
                                    ),
                                )));
                            }
                            Finish::AddRight(left) => add_completion(
                                runtime,
                                realm,
                                std::mem::replace(left, Value::Undefined),
                                value,
                            )?,
                            Finish::Predicate(input) => {
                                let mut input = input.take().expect("predicate conversion input");
                                input.key = value;
                                return Ok(Progress::Predicate(input));
                            }
                            Finish::SuperProperty(input) => {
                                let mut input = input.take().expect("super conversion input");
                                input.key = value;
                                return Ok(Progress::SuperProperty(input));
                            }
                            Finish::PropertyWrite {
                                base,
                                value: assigned,
                            } => {
                                let base = std::mem::replace(base, Value::Undefined);
                                let assigned = std::mem::replace(assigned, Value::Undefined);
                                return Ok(Progress::PropertyWrite(Box::new(
                                    super::property_write_driver::ConvertedWrite {
                                        base,
                                        key: value,
                                        value: assigned,
                                    },
                                )));
                            }
                            Finish::PropertyRead {
                                base,
                                keep_receiver,
                                keep_key,
                            } => {
                                let base = std::mem::replace(base, Value::Undefined);
                                let keep_receiver = *keep_receiver;
                                let keep_key = *keep_key;
                                return Ok(Progress::PropertyRead(Box::new(
                                    super::property_driver::ConvertedRead {
                                        base,
                                        key: value,
                                        keep_receiver,
                                        keep_key,
                                    },
                                )));
                            }
                            Finish::PropertyKey => {
                                Completion::Return(property_key_primitive(runtime, value)?)
                            }
                            Finish::Plus => match super::numeric::unary_plus_primitive(value) {
                                Ok(value) => Completion::Return(value),
                                Err(error) => {
                                    let Some(kind) = crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind()) else { return Err(error); };
                                    Completion::Throw(
                                        runtime
                                            .new_native_error_from_error(realm, kind, &error)
                                            .map_err(runtime_error_to_vm_error)?,
                                    )
                                }
                            },
                        }
                    }
                };
                Ok(Progress::Complete(completion))
            }
            PrimitiveStep::Get { mut resume } => {
                let (object, key) = resume.take_get();
                let read = runtime
                    .prepare_ordinary_read(&object, &key, Value::Object(object.clone()))
                    .map_err(runtime_error_to_vm_error)?;
                match read {
                    OrdinaryRead::Call { getter, receiver } => invoke(
                        runtime,
                        execution,
                        self,
                        getter,
                        receiver,
                        Vec::new(),
                        resume,
                    ),
                    OrdinaryRead::Complete(value) => Ok(Progress::Ready(
                        self.with_step(
                            resume
                                .resume(
                                    runtime,
                                    Completion::Return(value.unwrap_or(Value::Undefined)),
                                )
                                .map_err(runtime_error_to_vm_error)?,
                        ),
                    )),
                    OrdinaryRead::Special { .. } => {
                        match super::proxy_get_driver::start_conversion(
                            runtime,
                            execution,
                            frame,
                            object,
                            key,
                            self.waiting(resume),
                        )? {
                            super::proxy_get_driver::Progress::Conversion(task) => {
                                Ok(Progress::Ready(task))
                            }
                            super::proxy_get_driver::Progress::Call(
                                super::driver::CallStep::Entered,
                            ) => Ok(Progress::Entered),
                            super::proxy_get_driver::Progress::Call(
                                super::driver::CallStep::Complete(completion),
                            ) => Ok(Progress::Complete(completion)),
                            super::proxy_get_driver::Progress::Call(
                                super::driver::CallStep::Bridge,
                            ) => Err(Error::internal(
                                "conversion property query attempted replay",
                            )),
                        }
                    }
                }
            }
            PrimitiveStep::Call { mut resume } => {
                let callable = resume.take_callable();
                let receiver = resume.take_receiver();
                let arguments = resume.take_arguments();
                invoke(
                    runtime, execution, self, callable, receiver, arguments, resume,
                )
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn invoke(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    task: ConversionTask,
    callable: CallableRef,
    receiver: Value,
    arguments: Vec<Value>,
    resume: PrimitiveResume,
) -> Result<Progress, Error> {
    let frame = task.frame;
    let identity = task.identity;
    let realm = execution.frames.current_mut(frame)?.executable.realm;
    let super::call::NormalizedCallback {
        callable,
        receiver,
        arguments,
        classification,
    } = match super::call::normalize_callback(runtime, realm, callable, receiver, arguments)? {
        crate::engine::value::conversion::NativeConversion::Value(call) => call,
        crate::engine::value::conversion::NativeConversion::Throw(value) => {
            return Ok(Progress::Ready(
                task.with_step(
                    resume
                        .resume(runtime, Completion::Throw(value))
                        .map_err(runtime_error_to_vm_error)?,
                ),
            ));
        }
    };
    let is_proxy = matches!(classification, CallableExecution::Proxy);
    let is_owned_native = matches!(&classification, CallableExecution::Native { .. }
        if super::frames::native_operation(runtime, &callable).map_err(runtime_error_to_vm_error)?.is_some());
    let is_resumable = if let CallableExecution::Bytecode { bytecode, .. } = &classification {
        runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(bytecode.bytecode_id())
            .map_err(|error| Error::internal(error.to_string()))?
            .metadata
            .function_kind
            != FunctionKind::Normal
    } else {
        false
    };
    if is_proxy || is_owned_native || is_resumable {
        let wait = task.waiting(resume);
        let progress = if is_proxy {
            super::proxy_get_driver::start_conversion_call(
                runtime,
                execution,
                frame,
                callable.as_object().clone(),
                receiver,
                arguments,
                wait,
            )?
        } else {
            super::proxy_get_driver::start_native_conversion_call(
                runtime, execution, frame, callable, receiver, arguments, wait,
            )?
        };
        return match progress {
            super::proxy_get_driver::Progress::Conversion(task) => Ok(Progress::Ready(task)),
            super::proxy_get_driver::Progress::Call(super::driver::CallStep::Entered) => {
                Ok(Progress::Entered)
            }
            super::proxy_get_driver::Progress::Call(super::driver::CallStep::Complete(
                completion,
            )) => Ok(Progress::Complete(completion)),
            super::proxy_get_driver::Progress::Call(super::driver::CallStep::Bridge) => {
                Err(Error::internal("conversion callback attempted replay"))
            }
        };
    }
    if let CallableExecution::Bytecode {
        bytecode,
        closure_slots,
    } = classification
    {
        let kind = runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(bytecode.bytecode_id())
            .map_err(|error| Error::internal(error.to_string()))?
            .metadata
            .function_kind;
        if kind == FunctionKind::Normal {
            if !execution.frames.can_push() || runtime.bytecode_call_would_overflow() {
                let completion = runtime
                    .bytecode_stack_overflow_completion(realm, &bytecode)
                    .map_err(runtime_error_to_vm_error)?;
                return Ok(Progress::Ready(
                    task.with_step(
                        resume
                            .resume(runtime, completion)
                            .map_err(runtime_error_to_vm_error)?,
                    ),
                ));
            }
            let request = BytecodeCallRequest {
                callable,
                receiver,
                arguments,
                new_target: Value::Undefined,
                bytecode,
                closure_slots,
                caller_realm: realm,
                return_to: ReturnTarget {
                    value_use: crate::engine::vm::frame::ReturnValue::Push,
                    owner: crate::engine::vm::frame::ReturnOwner::Frame(frame),
                    tail: false,
                    operation: Some(super::frame::OperationTarget::Conversion(identity)),
                },
            };
            let entry = request.prepare(runtime, &mut execution.call_storage)?;
            let parent = execution.frames.current_mut(frame)?;
            if parent.cold.conversion.is_some() {
                return Err(Error::internal(
                    "conversion overwrote an unanswered request",
                ));
            }
            parent.cold.conversion = Some(task.waiting(resume));
            super::driver::push_frame(execution, entry)?;
            return Ok(Progress::Entered);
        }
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_sync_call_bridge();
    let completion = runtime
        .call_internal(realm, &callable, receiver, &arguments)
        .map_err(runtime_error_to_vm_error)?;
    Ok(Progress::Ready(
        task.with_step(
            resume
                .resume(runtime, completion)
                .map_err(runtime_error_to_vm_error)?,
        ),
    ))
}

#[cfg(all(test, feature = "profiling"))]
mod primitive_store_tests {
    use crate::engine::api::profiling::CostProfile;
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn conversion_task_resides_across_both_operands_and_skips_primitive_property_keys() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .eval("var residentLeft={valueOf(){return 1}},residentRight={valueOf(){return 2}}")
            .unwrap();
        let profile = CostProfile::start();
        assert_eq!(
            context.eval("residentLeft+residentRight").unwrap(),
            Value::Int(3)
        );
        assert_eq!(
            profile
                .snapshot()
                .owned_execution_events
                .get("conversion_task_allocated"),
            Some(&1)
        );
        drop(profile);
        let profile = CostProfile::start();
        assert_eq!(
            context.eval("({[true]:1,[1.25]:2,[null]:3}).true").unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            profile
                .snapshot()
                .owned_execution_events
                .get("conversion_task_allocated")
                .copied()
                .unwrap_or(0),
            0
        );
    }

    #[test]
    fn primitive_store_keeps_conversion_capture_and_throw_observations() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let s='', b=1n;
            for(let i=0;i<20;i++){ s+='x'; b+=2n; }
            let capture=()=>s;
            s+='y';
            let old=s,trace='';
            try { s+=Symbol(); } catch(e) { if(e instanceof TypeError)trace+='throw'; }
            finally { trace+='finally'; }
            let a='left';
            a+= {valueOf(){a='changed';return 'right';}};
            let constant='a', read=constant;
            try { const x='a'; x+='b'; } catch(e) { if(e instanceof TypeError)read+='!'; }
            return s===old && capture()===old && s.length===21 && b===41n
                && trace==='throwfinally' && a==='leftright' && read==='a!';
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(
            profile
                .snapshot()
                .owned_execution_events
                .get("primitive_add_store_fused")
                .copied()
                .unwrap_or(0)
                > 0
        );
    }
}
