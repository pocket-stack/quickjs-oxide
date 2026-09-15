//! The owned stack's one instruction match. Dynamic coercion and unimplemented
//! protocols exit before consuming their operands; S04–S07 replace that bridge.

use crate::engine::api::error::Error;
use crate::engine::code::bytecode::Instruction;
use crate::engine::heap::{BytecodeConstant, RawValue, SlotReleaseReadiness};
use crate::engine::value::Value;
use crate::engine::value::number::operations::Number;
use crate::engine::vm::bindings::FrameBinding;
use crate::engine::vm::exception::runtime_error_to_vm_error;
use crate::engine::vm::execution::RunningExecution;
use crate::engine::vm::frame::FrameId;
use crate::engine::vm::stack::{RunSlots, copy_value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindingSource {
    Closure,
    Local,
    Argument,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunExit {
    Import,
    Pure(super::pure_operations::PureOperation),
    ApplyEval(u16),
    Apply(crate::engine::code::bytecode::ApplyKind),
    Eval {
        arguments: u16,
        environment: u16,
    },
    Call {
        arguments: u16,
        method: bool,
        tail: bool,
    },
    SetProperty(Option<u32>),
    GetField {
        index: u32,
        keep_receiver: bool,
    },
    GetElement {
        keep_receiver: bool,
        keep_key: bool,
    },
    InitializeDerived(u16),
    LexicalUninitialized(u16),
    Binding {
        source: BindingSource,
        index: u16,
        write: bool,
        checked: bool,
        keep: bool,
    },
    ClassInitializer(super::construct_driver::InitializerKind),
    DefineClass {
        name: u32,
        has_heritage: bool,
    },
    DefineProperty {
        key: Option<u32>,
        method: Option<(crate::engine::code::bytecode::DefineMethodKind, bool)>,
    },
    Environment(super::environment_driver::Operation),
    GetSuper,
    Predicate(super::predicate_driver::Kind),
    HomeObject,
    SuperProperty(super::super_property_driver::Kind),
    ReturnDerived(u16),
    InitDerivedConstructor,
    Construct(u16),
    ConvertAdd,
    AddLocal,
    ConvertPlus,
    ConvertPropertyKey,
    NormalizeThis,
    Arguments(crate::engine::code::bytecode::ArgumentsKind),
    Rest(u16),
    InstantiateClosure(u32),
    SetName(Option<u32>),
    CloseCaptured(u16),
    ResetCaptured(u16),
    Catch(u32),
    DropCatch,
    NipCatch,
    Throw,
    /// Owned arithmetic error in execution.pending; operands already consumed.
    PrimitiveThrow,
    Materialize,
    BindingError {
        index: u32,
        redeclaration: bool,
    },
    PrivateInitialize {
        index: u16,
        kind: super::private_bindings::Initialization,
    },
    PrivateAccess {
        source: crate::engine::code::bytecode::PrivateNameSource,
        access: super::private_access::Access,
    },
    StrictEquality(bool),
    Numeric(super::numeric::operation::NumericKind),
    ForIn(bool),
    LogicalNot,
    CopyData {
        target: u8,
        source: u8,
        excluded: Option<u8>,
    },
    ReplaceBinding {
        source: BindingSource,
        index: u16,
        keep: bool,
        uninitialized: bool,
    },
    ReleaseOperand {
        keep_top: bool,
    },
    Complete,
    Suspend(super::VmSuspendKind),
    Bridge,
}

#[cfg(feature = "profiling")]
impl RunExit {
    pub(super) fn diagnostic_name(self) -> &'static str {
        match self {
            Self::Import => "run_exit.Import",
            Self::Pure(..) => "run_exit.Pure",
            Self::ApplyEval(..) => "run_exit.ApplyEval",
            Self::Apply(..) => "run_exit.Apply",
            Self::Eval { .. } => "run_exit.Eval",
            Self::Call { .. } => "run_exit.Call",
            Self::SetProperty(..) => "run_exit.SetProperty",
            Self::GetField { .. } => "run_exit.GetField",
            Self::GetElement { .. } => "run_exit.GetElement",
            Self::InitializeDerived(..) => "run_exit.InitializeDerived",
            Self::LexicalUninitialized(..) => "run_exit.LexicalUninitialized",
            Self::Binding { .. } => "run_exit.Binding",
            Self::ClassInitializer(..) => "run_exit.ClassInitializer",
            Self::DefineClass { .. } => "run_exit.DefineClass",
            Self::DefineProperty { .. } => "run_exit.DefineProperty",
            Self::Environment(..) => "run_exit.Environment",
            Self::GetSuper => "run_exit.GetSuper",
            Self::Predicate(..) => "run_exit.Predicate",
            Self::HomeObject => "run_exit.HomeObject",
            Self::SuperProperty(..) => "run_exit.SuperProperty",
            Self::ReturnDerived(..) => "run_exit.ReturnDerived",
            Self::InitDerivedConstructor => "run_exit.InitDerivedConstructor",
            Self::Construct(..) => "run_exit.Construct",
            Self::ConvertAdd => "run_exit.ConvertAdd",
            Self::AddLocal => "run_exit.AddLocal",
            Self::ConvertPlus => "run_exit.ConvertPlus",
            Self::ConvertPropertyKey => "run_exit.ConvertPropertyKey",
            Self::NormalizeThis => "run_exit.NormalizeThis",
            Self::Arguments(..) => "run_exit.Arguments",
            Self::Rest(..) => "run_exit.Rest",
            Self::InstantiateClosure(..) => "run_exit.InstantiateClosure",
            Self::SetName(..) => "run_exit.SetName",
            Self::CloseCaptured(..) => "run_exit.CloseCaptured",
            Self::ResetCaptured(..) => "run_exit.ResetCaptured",
            Self::Catch(..) => "run_exit.Catch",
            Self::DropCatch => "run_exit.DropCatch",
            Self::NipCatch => "run_exit.NipCatch",
            Self::Throw => "run_exit.Throw",
            Self::PrimitiveThrow => "run_exit.PrimitiveThrow",
            Self::Materialize => "run_exit.Materialize",
            Self::BindingError { .. } => "run_exit.BindingError",
            Self::PrivateInitialize { .. } => "run_exit.PrivateInitialize",
            Self::PrivateAccess { .. } => "run_exit.PrivateAccess",
            Self::StrictEquality(..) => "run_exit.StrictEquality",
            Self::Numeric(..) => "run_exit.Numeric",
            Self::ForIn(..) => "run_exit.ForIn",
            Self::LogicalNot => "run_exit.LogicalNot",
            Self::CopyData { .. } => "run_exit.CopyData",
            Self::ReplaceBinding { .. } => "run_exit.ReplaceBinding",
            Self::ReleaseOperand { .. } => "run_exit.ReleaseOperand",
            Self::Complete => "run_exit.Complete",
            Self::Suspend(..) => "run_exit.Suspend",
            Self::Bridge => "run_exit.Bridge",
        }
    }
}

mod cold;
mod fusion;
mod numeric;
mod program_counter;
use program_counter::ProgramCounter;

#[cfg(test)]
pub(super) fn test_supported_numeric(
    slots: &RunSlots<'_>,
    kind: super::numeric::operation::NumericKind,
) -> bool {
    numeric::supported(slots, kind)
}

#[cfg(test)]
pub(super) fn test_complete_numeric(
    runtime: &crate::engine::api::runtime::Runtime,
    realm: crate::engine::heap::ContextId,
    transaction: &mut super::stack::FrameTransaction<'_>,
    kind: super::numeric::operation::NumericKind,
    thrown: &mut Option<Value>,
) -> Result<bool, Error> {
    numeric::complete(runtime, realm, transaction, kind, thrown)
}

fn number(value: &Value) -> Option<Number> {
    value.as_number_repr()
}
fn value(number: Number) -> Value {
    number.into()
}
fn immediate(value: &Value) -> bool {
    matches!(
        value,
        Value::Undefined | Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_)
    )
}
fn binary(
    slots: &mut RunSlots<'_>,
    operation: impl FnOnce(Number, Number) -> Value,
) -> Result<bool, Error> {
    slots.binary_number(operation)
}

fn release_displaced(
    runtime: &crate::engine::api::runtime::Runtime,
    old: FrameBinding,
) -> Result<(), Error> {
    let FrameBinding::Direct(mut old) = old else {
        return Err(cold::internal(
            "non-direct binding passed a direct release preflight",
        ));
    };
    // Between the initial proof and this commit, only moves and possibly one
    // retain occurred. Neither can invalidate the no-drain proof.
    if !runtime
        .try_release_slot_value(&mut old)
        .map_err(runtime_error_to_vm_error)?
    {
        return Err(cold::internal(
            "slot release proof changed without a callback",
        ));
    }
    Ok(())
}

pub(super) fn run(execution: &mut RunningExecution, id: FrameId) -> Result<RunExit, Error> {
    let frame = execution.frames.current_mut(id)?;
    let body = &mut *frame.cold;
    let executable = &*body.executable;
    let mut transaction = execution.slots.frame_transaction(&mut body.window)?;
    let mut slots = transaction.slots();
    let cold = &mut body.owners;
    let runtime = cold.function.runtime();
    let mut pc = ProgramCounter::new(&mut frame.fault_pc, &mut frame.resume_pc);
    // Preserve the cold path's observation order, but keep this authenticated
    // frame resident. No slot borrow crosses active-PC publication or Drop.
    macro_rules! release_outside_slots {
        ($operation:expr) => {{
            if !frame.active_frame.is_materialized() {
                return Ok(RunExit::Materialize);
            }
            drop(slots);
            pc.publish_fault();
            runtime
                .update_active_bytecode_pc(frame.active_frame, super::BytecodePc::new(pc.fault))
                .map_err(runtime_error_to_vm_error)?;
            slots = transaction.slots();
            let released = $operation;
            drop(slots);
            drop(released);
            slots = transaction.slots();
        }};
    }
    loop {
        pc.fault = pc.resume;
        let instruction = executable
            .code
            .get(pc.fault)
            .ok_or_else(|| cold::internal("owned bytecode ended without return"))?;
        #[cfg(feature = "profiling")]
        let observed_depth = slots.depth();
        let mut next_pc = pc
            .fault
            .checked_add(1)
            .ok_or_else(|| cold::internal("owned program counter overflow"))?;
        let handled = match instruction {
            Instruction::Call(arguments)
            | Instruction::TailCall(arguments)
            | Instruction::CallMethod(arguments)
            | Instruction::TailCallMethod(arguments) => {
                return Ok(RunExit::Call {
                    arguments: *arguments,
                    method: matches!(
                        instruction,
                        Instruction::CallMethod(_) | Instruction::TailCallMethod(_)
                    ),
                    tail: matches!(
                        instruction,
                        Instruction::TailCall(_) | Instruction::TailCallMethod(_)
                    ),
                });
            }
            Instruction::Apply(kind) => return Ok(RunExit::Apply(*kind)),
            Instruction::ApplySuper => {
                return Ok(RunExit::Apply(
                    crate::engine::code::bytecode::ApplyKind::Construct,
                ));
            }
            Instruction::ApplyEval { environment } => return Ok(RunExit::ApplyEval(*environment)),
            Instruction::Eval {
                argument_count,
                environment,
            } => {
                return Ok(RunExit::Eval {
                    arguments: *argument_count,
                    environment: *environment,
                });
            }
            Instruction::PushThis => {
                let value = if let Some(value) = cold
                    .rare
                    .get()
                    .and_then(|rare| rare.normalized_this.as_ref())
                {
                    copy_value(value)?
                } else if executable.metadata.strict
                    || matches!(cold.input.this_value, Value::Object(_))
                {
                    copy_value(&cold.input.this_value)?
                } else if matches!(cold.input.this_value, Value::Undefined | Value::Null) {
                    copy_value(&Value::Object(
                        cold.input.callee_global(runtime, executable.realm)?.clone(),
                    ))?
                } else {
                    return Ok(RunExit::NormalizeThis);
                };
                slots.push(value)?;
                true
            }
            Instruction::PutField(index) => {
                let Some(identity) = frame.property_generation.checked_add(1) else {
                    return Ok(RunExit::SetProperty(Some(*index)));
                };
                if !slots.ordinary_field_immediate_write(runtime, &executable, *index)? {
                    return Ok(RunExit::SetProperty(Some(*index)));
                }
                frame.property_generation = identity;
                true
            }
            Instruction::PutArrayEl => {
                let Some(identity) = frame.property_generation.checked_add(1) else {
                    return Ok(RunExit::SetProperty(None));
                };
                if !slots.typed_array_number_write(runtime)? {
                    return Ok(RunExit::SetProperty(None));
                }
                frame.property_generation = identity;
                true
            }
            Instruction::GetField(index) => {
                let mut native = None;
                if !slots.property_ic_read(
                    runtime,
                    executable,
                    pc.fault,
                    *index,
                    false,
                    &mut native,
                )? && !slots.ordinary_field_immediate_read(runtime, &executable, *index)?
                {
                    return Ok(RunExit::GetField {
                        index: *index,
                        keep_receiver: false,
                    });
                }
                true
            }
            Instruction::GetField2(index) => {
                let mut native = None;
                if !slots.property_ic_read(
                    runtime,
                    executable,
                    pc.fault,
                    *index,
                    true,
                    &mut native,
                )? {
                    return Ok(RunExit::GetField {
                        index: *index,
                        keep_receiver: true,
                    });
                }
                let candidate = executable.fusion.method_call(pc.fault).filter(|count| {
                    slots.has_operand_capacity(*count)
                        && super::method_arguments::available(
                            &slots,
                            &executable.code[pc.fault + 1..pc.fault + count + 1],
                        )
                });
                if let Some(count) = candidate {
                    #[cfg(feature = "profiling")]
                    cold::instruction(observed_depth);
                    let start = pc.fault;
                    for offset in 0..count {
                        pc.fault = start + offset + 1;
                        pc.resume = pc.fault;
                        let argument =
                            super::method_arguments::argument(&slots, &executable.code[pc.fault])?;
                        slots.push(argument)?;
                        #[cfg(feature = "profiling")]
                        cold::instruction(observed_depth + offset + 1);
                    }
                    pc.fault = start + count + 1;
                    pc.resume = pc.fault;
                    #[cfg(feature = "profiling")]
                    cold::event("method_call_span");
                    execution.selected_native = native;
                    return Ok(RunExit::Call {
                        arguments: count as u16,
                        method: true,
                        tail: matches!(executable.code[pc.fault], Instruction::TailCallMethod(_)),
                    });
                }
                true
            }
            Instruction::GetArrayEl => {
                if !slots.array_immediate_read(runtime)? {
                    return Ok(RunExit::GetElement {
                        keep_receiver: false,
                        keep_key: false,
                    });
                }
                true
            }
            Instruction::GetArrayEl2 | Instruction::GetArrayEl3 => {
                return Ok(RunExit::GetElement {
                    keep_receiver: !matches!(instruction, Instruction::GetArrayEl),
                    keep_key: matches!(instruction, Instruction::GetArrayEl3),
                });
            }
            Instruction::Construct(count) | Instruction::ConstructSuper(count) => {
                return Ok(RunExit::Construct(*count));
            }
            Instruction::PushNewTarget => {
                slots.push(copy_value(&cold.input.new_target)?)?;
                true
            }
            Instruction::InitializeDerivedLocal(index) => {
                return Ok(RunExit::InitializeDerived(*index));
            }
            Instruction::CopyDataProperties => {
                return Ok(RunExit::CopyData {
                    target: 1,
                    source: 0,
                    excluded: None,
                });
            }
            Instruction::CopyDataPropertiesExcluded {
                target_depth,
                source_depth,
                excluded_depth,
            } => {
                return Ok(RunExit::CopyData {
                    target: *target_depth,
                    source: *source_depth,
                    excluded: Some(*excluded_depth),
                });
            }
            Instruction::InstanceOf => {
                return Ok(RunExit::Predicate(super::predicate_driver::Kind::Instance));
            }
            Instruction::In => return Ok(RunExit::Predicate(super::predicate_driver::Kind::Has)),
            Instruction::Delete => {
                return Ok(RunExit::Predicate(super::predicate_driver::Kind::Delete));
            }
            Instruction::GetSuper => return Ok(RunExit::GetSuper),
            Instruction::PushHomeObject => return Ok(RunExit::HomeObject),
            Instruction::GetSuperValue => {
                return Ok(RunExit::SuperProperty(
                    super::super_property_driver::Kind::Read,
                ));
            }
            Instruction::GetSuperValueForCall => {
                return Ok(RunExit::SuperProperty(
                    super::super_property_driver::Kind::Call,
                ));
            }
            Instruction::PutSuperValue => {
                return Ok(RunExit::SuperProperty(
                    super::super_property_driver::Kind::Write,
                ));
            }
            Instruction::ReturnDerived(index) => return Ok(RunExit::ReturnDerived(*index)),
            Instruction::CheckCtor if !matches!(cold.input.new_target, Value::Undefined) => true,
            Instruction::CheckCtor => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::ConstructorWithoutNew,
                ));
            }
            Instruction::PushActiveFunction => {
                slots.push(Value::Object(cold.function.clone()))?;
                #[cfg(feature = "profiling")]
                cold::storage(crate::engine::api::profiling::OwnedStorageEvent::Copy {
                    heap_root: true,
                });
                true
            }
            Instruction::InitDerivedConstructor => return Ok(RunExit::InitDerivedConstructor),
            Instruction::PutVar(index) | Instruction::PutVarInit(index) => {
                let root = executable
                    .closure_variables
                    .get(usize::from(*index))
                    .filter(|descriptor| {
                        !descriptor.kind.is_private()
                            && matches!(
                                descriptor.name,
                                crate::engine::code::function::metadata::ClosureVariableName::Atom(
                                    _
                                )
                            )
                    })
                    .and_then(|_| cold.closure_slots.get(usize::from(*index)));
                let stored = if matches!(instruction, Instruction::PutVar(_)) {
                    if let Some(root) = root {
                        super::bindings::try_write_immediate_cell(
                            runtime,
                            &root,
                            slots.peek(0)?,
                            None,
                        )
                    } else {
                        false
                    }
                } else {
                    false
                };
                if stored {
                    slots.pop()?;
                    #[cfg(feature = "profiling")]
                    cold::event("global_immediate_cell_write");
                    true
                } else {
                    return Ok(RunExit::Environment(
                        super::environment_driver::Operation::Put {
                            source: super::environment_driver::WriteTarget::Global {
                                index: *index,
                                initialize: matches!(instruction, Instruction::PutVarInit(_)),
                            },
                            name: 0, // Global names come from the authenticated closure descriptor.
                            strict: executable.metadata.strict,
                            check_presence: true,
                        },
                    ));
                }
            }
            Instruction::DeleteVar(index) => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::GlobalDelete(*index),
                ));
            }
            Instruction::GetVar(index) | Instruction::GetVarUndef(index) => {
                let immediate = executable
                    .closure_variables
                    .get(usize::from(*index))
                    .filter(|descriptor| {
                        !descriptor.kind.is_private()
                            && matches!(
                                descriptor.name,
                                crate::engine::code::function::metadata::ClosureVariableName::Atom(
                                    _
                                )
                            )
                    })
                    .and_then(|_| cold.closure_slots.get(usize::from(*index)))
                    .map(|root| super::bindings::read_run_cell(runtime, &root))
                    .transpose()?
                    .flatten();
                if let Some((value, _owned)) = immediate {
                    slots.push(value)?;
                    #[cfg(feature = "profiling")]
                    cold::event(if _owned {
                        "global_owned_cell_read"
                    } else {
                        "global_immediate_cell_read"
                    });
                    true
                } else if let Some(value) = super::environment_driver::try_global_own_read(
                    runtime,
                    &executable,
                    &cold.closure_slots,
                    *index,
                )? {
                    slots.push(value)?;
                    true
                } else {
                    return Ok(RunExit::Environment(
                        super::environment_driver::Operation::GlobalGet {
                            index: *index,
                            strict: matches!(instruction, Instruction::GetVar(_)),
                        },
                    ));
                }
            }
            Instruction::GlobalReference(index) => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::GlobalReference(*index),
                ));
            }
            Instruction::GetRefValue(name) | Instruction::GetRefValueUndef(name) => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::ReadReference {
                        name: *name,
                        strict: matches!(instruction, Instruction::GetRefValue(_))
                            && executable.metadata.strict,
                    },
                ));
            }
            Instruction::DeleteDynamicBinding { source, name } => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Delete {
                        source: *source,
                        name: *name,
                    },
                ));
            }
            Instruction::DeleteEvalVariable { source, name } => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Delete {
                        source: crate::engine::code::bytecode::DynamicEnvironmentSource::Eval(
                            *source,
                        ),
                        name: *name,
                    },
                ));
            }
            Instruction::DynamicEnvironmentObject(source) => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Object(*source),
                ));
            }
            Instruction::HasDynamicBinding { source, name } => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Has {
                        source: *source,
                        name: *name,
                    },
                ));
            }
            Instruction::PutDynamicBinding { source, name } => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Put {
                        source: super::environment_driver::WriteTarget::Dynamic(*source),
                        name: *name,
                        strict: executable.metadata.strict,
                        check_presence: true,
                    },
                ));
            }
            Instruction::PutEvalVariable { source, name } => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Put {
                        source: super::environment_driver::WriteTarget::Dynamic(
                            crate::engine::code::bytecode::DynamicEnvironmentSource::Eval(*source),
                        ),
                        name: *name,
                        strict: false,
                        check_presence: false,
                    },
                ));
            }
            Instruction::PutRefValue(name) => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Put {
                        source: super::environment_driver::WriteTarget::Reference,
                        name: *name,
                        strict: executable.metadata.strict,
                        check_presence: true,
                    },
                ));
            }
            Instruction::HasEvalVariable { source, name } => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Has {
                        source: crate::engine::code::bytecode::DynamicEnvironmentSource::Eval(
                            *source,
                        ),
                        name: *name,
                    },
                ));
            }
            Instruction::GetEvalVariable { source, name } => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Get {
                        source: crate::engine::code::bytecode::DynamicEnvironmentSource::Eval(
                            *source,
                        ),
                        name: *name,
                        strict: false,
                    },
                ));
            }
            Instruction::DefineEvalVariable { source, name } => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Define {
                        source: *source,
                        name: *name,
                    },
                ));
            }
            Instruction::GetDynamicBinding { source, name } => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Get {
                        source: *source,
                        name: *name,
                        strict: executable.metadata.strict,
                    },
                ));
            }
            Instruction::ForInStart => return Ok(RunExit::ForIn(false)),
            Instruction::ForInNext => return Ok(RunExit::ForIn(true)),
            Instruction::IteratorStart
            | Instruction::AsyncIteratorStart
            | Instruction::ForAwaitOfStart
            | Instruction::ForAwaitOfNext
            | Instruction::IteratorNext
            | Instruction::IteratorCall(_)
            | Instruction::IteratorGetValueDone => {
                use super::iterator_driver::suspension::Operation;
                let operation = match instruction {
                    Instruction::IteratorStart => Operation::Start {
                        asynchronous: false,
                        delegating: true,
                    },
                    Instruction::AsyncIteratorStart => Operation::Start {
                        asynchronous: true,
                        delegating: true,
                    },
                    Instruction::ForAwaitOfStart => Operation::Start {
                        asynchronous: true,
                        delegating: false,
                    },
                    Instruction::ForAwaitOfNext => Operation::AwaitNext,
                    Instruction::IteratorNext => Operation::Next,
                    Instruction::IteratorCall(kind) => Operation::Call(*kind),
                    Instruction::IteratorGetValueDone => Operation::Parse,
                    _ => unreachable!(),
                };
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Iterator(
                        super::iterator_driver::Operation::Suspend(operation),
                    ),
                ));
            }
            Instruction::ForOfStart
            | Instruction::ForOfNext(_)
            | Instruction::IteratorClose
            | Instruction::IteratorClosePreserve
            | Instruction::IteratorDropPreserve
            | Instruction::IteratorDetachPreserve => {
                use super::iterator_driver::Operation;
                let op = match instruction {
                    Instruction::ForOfStart => Operation::Start,
                    Instruction::ForOfNext(offset) => Operation::Next(usize::from(*offset)),
                    Instruction::IteratorClose => Operation::Close,
                    Instruction::IteratorClosePreserve => Operation::ClosePreserve,
                    Instruction::IteratorDropPreserve => Operation::DropPreserve,
                    Instruction::IteratorDetachPreserve => Operation::DetachPreserve,
                    _ => unreachable!(),
                };
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Iterator(op),
                ));
            }
            Instruction::Append => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::Append,
                ));
            }
            Instruction::DefineArrayEl => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::DefineArrayElement,
                ));
            }
            Instruction::ArrayFrom(count) => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::CreateArray(*count),
                ));
            }
            Instruction::Object => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::CreateObject,
                ));
            }
            Instruction::VariableEnvironment => {
                return Ok(RunExit::Environment(
                    super::environment_driver::Operation::CreateVariable,
                ));
            }
            Instruction::ToObject => {
                if matches!(slots.peek(0)?, Value::Object(_)) {
                    true
                } else {
                    return Ok(RunExit::Environment(
                        super::environment_driver::Operation::ToObject,
                    ));
                }
            }
            Instruction::ToPropKey => match slots.peek(0)? {
                Value::Int(_) | Value::String(_) => true,
                Value::Symbol(symbol) if symbol.belongs_to(runtime) => true,
                _ => return Ok(RunExit::ConvertPropertyKey),
            },
            Instruction::DefineFieldComputed => {
                return Ok(RunExit::DefineProperty {
                    key: None,
                    method: None,
                });
            }
            Instruction::DefineMethodComputed { kind, enumerable } => {
                return Ok(RunExit::DefineProperty {
                    key: None,
                    method: Some((*kind, *enumerable)),
                });
            }
            Instruction::DefineField(key) => {
                return Ok(RunExit::DefineProperty {
                    key: Some(*key),
                    method: None,
                });
            }
            Instruction::DefineMethod {
                key,
                kind,
                enumerable,
            } => {
                return Ok(RunExit::DefineProperty {
                    key: Some(*key),
                    method: Some((*kind, *enumerable)),
                });
            }
            Instruction::DefineClass { name, has_heritage } => {
                return Ok(RunExit::DefineClass {
                    name: *name,
                    has_heritage: *has_heritage,
                });
            }
            Instruction::InstallClassInstanceInitializer => {
                return Ok(RunExit::ClassInitializer(
                    super::construct_driver::InitializerKind::Install,
                ));
            }
            Instruction::CallClassInstanceInitializer => {
                return Ok(RunExit::ClassInitializer(
                    super::construct_driver::InitializerKind::Instance,
                ));
            }
            Instruction::RunClassStaticInitializer => {
                return Ok(RunExit::ClassInitializer(
                    super::construct_driver::InitializerKind::Static,
                ));
            }
            Instruction::CallClassStaticBlock => {
                return Ok(RunExit::ClassInitializer(
                    super::construct_driver::InitializerKind::Block,
                ));
            }
            Instruction::PushAtomValueIndex(value) => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::AtomValue(*value),
                ));
            }
            Instruction::RegExp(index) => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::RegExp(*index),
                ));
            }
            Instruction::ThrowDeleteSuper => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::DeleteSuper,
                ));
            }
            Instruction::Import => return Ok(RunExit::Import),
            Instruction::InitializeModuleImportCollision(index) => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::InitializeModuleImportCollision(*index),
                ));
            }
            Instruction::InitializeVarRef(index) | Instruction::InitializeDerivedVarRef(index) => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::InitializeClosure {
                        index: *index,
                        derived: matches!(instruction, Instruction::InitializeDerivedVarRef(_)),
                    },
                ));
            }
            Instruction::SetProto => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::SetPrototype,
                ));
            }
            Instruction::IteratorCheckObject => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::IteratorCheckObject,
                ));
            }
            Instruction::ThrowIteratorMissingThrow => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::IteratorMissingThrow,
                ));
            }
            Instruction::TypeOf
            | Instruction::IsUndefinedOrNull
            | Instruction::IsUndefined
            | Instruction::IsNull
            | Instruction::TypeOfIsUndefined
            | Instruction::TypeOfIsFunction => {
                use super::pure_operations::PureOperation as P;
                let kind = match instruction {
                    Instruction::TypeOf => P::TypeOf,
                    Instruction::IsUndefinedOrNull => P::IsUndefinedOrNull,
                    Instruction::IsUndefined => P::IsUndefined,
                    Instruction::IsNull => P::IsNull,
                    Instruction::TypeOfIsUndefined => P::TypeOfIsUndefined,
                    _ => P::TypeOfIsFunction,
                };
                return Ok(RunExit::Pure(kind));
            }
            Instruction::Nop | Instruction::MarkSuperCall => true,
            Instruction::PushI32(number) => {
                slots.push(Value::Int(*number))?;
                true
            }
            Instruction::Undefined => {
                slots.push(Value::Undefined)?;
                true
            }
            Instruction::Null => {
                slots.push(Value::Null)?;
                true
            }
            Instruction::PushTrue => {
                slots.push(Value::Bool(true))?;
                true
            }
            Instruction::PushFalse => {
                slots.push(Value::Bool(false))?;
                true
            }
            Instruction::PushConst(index) => {
                let result = match executable.constant(*index) {
                    Some(BytecodeConstant::Value(RawValue::Int(number))) => {
                        Some(Value::Int(*number))
                    }
                    Some(BytecodeConstant::Value(RawValue::Float(number))) => {
                        Some(Value::Float(*number))
                    }
                    Some(BytecodeConstant::Value(RawValue::Undefined)) => Some(Value::Undefined),
                    Some(BytecodeConstant::Value(RawValue::Null)) => Some(Value::Null),
                    Some(BytecodeConstant::Value(RawValue::Bool(value))) => {
                        Some(Value::Bool(*value))
                    }
                    Some(BytecodeConstant::Value(RawValue::String(value))) => {
                        Some(Value::String(value.clone()))
                    }
                    Some(BytecodeConstant::Value(RawValue::BigInt(value))) => {
                        Some(Value::BigInt(value.clone()))
                    }
                    _ => None,
                };
                if let Some(value) = result {
                    #[cfg(feature = "profiling")]
                    cold::storage(crate::engine::api::profiling::OwnedStorageEvent::Copy {
                        heap_root: false,
                    });
                    slots.push(value)?;
                    true
                } else {
                    return Ok(RunExit::Pure(
                        super::pure_operations::PureOperation::Constant(*index),
                    ));
                }
            }
            Instruction::GetVarRef(index) | Instruction::GetVarRefCheck(index) => {
                if let Some((value, _owned)) = cold
                    .closure_slots
                    .get(usize::from(*index))
                    .map(|root| super::bindings::read_run_cell(runtime, &root))
                    .transpose()?
                    .flatten()
                {
                    slots.push(value)?;
                    #[cfg(feature = "profiling")]
                    cold::event(if _owned {
                        "captured_owned_cell_read"
                    } else {
                        "captured_immediate_cell_read"
                    });
                    true
                } else {
                    return Ok(RunExit::Binding {
                        source: BindingSource::Closure,
                        index: *index,
                        write: false,
                        checked: matches!(instruction, Instruction::GetVarRefCheck(_)),
                        keep: false,
                    });
                }
            }
            Instruction::PutVarRef(index)
            | Instruction::SetVarRef(index)
            | Instruction::PutVarRefCheck(index) => {
                let stored = if let (Some(root), Some(descriptor)) = (
                    cold.closure_slots.get(usize::from(*index)),
                    executable.closure_variables.get(usize::from(*index)),
                ) {
                    super::bindings::try_write_immediate_cell(
                        runtime,
                        &root,
                        slots.peek(0)?,
                        Some((descriptor.is_lexical, descriptor.is_const, descriptor.kind)),
                    )
                } else {
                    false
                };
                if stored {
                    if !matches!(instruction, Instruction::SetVarRef(_)) {
                        slots.pop()?;
                    }
                    #[cfg(feature = "profiling")]
                    cold::event("captured_immediate_cell_write");
                    true
                } else {
                    return Ok(RunExit::Binding {
                        source: BindingSource::Closure,
                        index: *index,
                        write: true,
                        checked: matches!(instruction, Instruction::PutVarRefCheck(_)),
                        keep: matches!(instruction, Instruction::SetVarRef(_)),
                    });
                }
            }
            Instruction::PutLocal(index)
            | Instruction::SetLocal(index)
            | Instruction::PutLocalCheck(index)
            | Instruction::SetLocalCheck(index)
                if matches!(slots.local(*index)?, FrameBinding::Captured(_)) =>
            {
                let stored = if let (FrameBinding::Captured(root), Some(definition)) = (
                    slots.local(*index)?,
                    executable.local_definitions.get(usize::from(*index)),
                ) {
                    super::bindings::try_write_immediate_cell(
                        runtime,
                        &root,
                        slots.peek(0)?,
                        Some((definition.is_lexical, definition.is_const, definition.kind)),
                    )
                } else {
                    false
                };
                if stored {
                    if !matches!(
                        instruction,
                        Instruction::SetLocal(_) | Instruction::SetLocalCheck(_)
                    ) {
                        slots.pop()?;
                    }
                    #[cfg(feature = "profiling")]
                    cold::event("captured_immediate_cell_write");
                    true
                } else {
                    return Ok(RunExit::Binding {
                        source: BindingSource::Local,
                        index: *index,
                        write: true,
                        checked: matches!(
                            instruction,
                            Instruction::PutLocalCheck(_) | Instruction::SetLocalCheck(_)
                        ),
                        keep: matches!(
                            instruction,
                            Instruction::SetLocal(_) | Instruction::SetLocalCheck(_)
                        ),
                    });
                }
            }
            Instruction::GetArg(index)
            | Instruction::PutArg(index)
            | Instruction::SetArg(index)
                if matches!(slots.parameter(*index)?, FrameBinding::Captured(_)) =>
            {
                let immediate = if matches!(instruction, Instruction::GetArg(_)) {
                    match slots.parameter(*index)? {
                        FrameBinding::Captured(root) => {
                            super::bindings::read_run_cell(runtime, &root)?
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some((value, _owned)) = immediate {
                    slots.push(value)?;
                    #[cfg(feature = "profiling")]
                    cold::event(if _owned {
                        "captured_owned_cell_read"
                    } else {
                        "captured_immediate_cell_read"
                    });
                    true
                } else if !matches!(instruction, Instruction::GetArg(_))
                    && match (
                        slots.parameter(*index)?,
                        executable.argument_definitions.get(usize::from(*index)),
                    ) {
                        (FrameBinding::Captured(root), Some(definition)) => {
                            super::bindings::try_write_immediate_cell(
                                runtime,
                                &root,
                                slots.peek(0)?,
                                Some((definition.is_lexical, definition.is_const, definition.kind)),
                            )
                        }
                        _ => false,
                    }
                {
                    if !matches!(instruction, Instruction::SetArg(_)) {
                        slots.pop()?;
                    }
                    #[cfg(feature = "profiling")]
                    cold::event("captured_immediate_cell_write");
                    true
                } else {
                    return Ok(RunExit::Binding {
                        source: BindingSource::Argument,
                        index: *index,
                        write: !matches!(instruction, Instruction::GetArg(_)),
                        checked: false,
                        keep: matches!(instruction, Instruction::SetArg(_)),
                    });
                }
            }
            Instruction::InitializeLocal(index)
                if matches!(slots.local(*index)?, FrameBinding::Captured(_))
                    && executable.local_definitions[usize::from(*index)].kind
                        == crate::engine::code::function::metadata::ClosureVariableKind::Normal =>
            {
                return Ok(RunExit::Binding {
                    source: BindingSource::Local,
                    index: *index,
                    write: true,
                    checked: false,
                    keep: false,
                });
            }
            Instruction::GetLocal(index) | Instruction::GetLocalCheck(index) => {
                if executable.fusion.local_add_span(pc.fault).is_some() {
                    if let Some(Instruction::GetLocal(right) | Instruction::GetLocalCheck(right)) =
                        executable.code.get(pc.fault + 1)
                    {
                        if slots.local_add_supported(runtime, *index, *right)? {
                            return Ok(RunExit::AddLocal);
                        }
                    }
                }
                if let Some(update) = executable.fusion.update(pc.fault) {
                    if fusion::update_local(&mut slots, *index, update)? {
                        #[cfg(feature = "profiling")]
                        fusion::record_span(
                            &executable.code[pc.fault..pc.fault + update.instructions],
                            observed_depth,
                        );
                        pc.resume = pc.fault + update.instructions;
                        continue;
                    }
                }
                match slots.local(*index)? {
                    FrameBinding::Direct(value) => {
                        let copied = copy_value(value)?;
                        slots.push(copied)?;
                        true
                    }
                    FrameBinding::Captured(root) => {
                        if let Some((value, _owned)) =
                            super::bindings::read_run_cell(runtime, &root)?
                        {
                            slots.push(value)?;
                            #[cfg(feature = "profiling")]
                            cold::event(if _owned {
                                "captured_owned_cell_read"
                            } else {
                                "captured_immediate_cell_read"
                            });
                            true
                        } else {
                            return Ok(RunExit::Binding {
                                source: BindingSource::Local,
                                index: *index,
                                write: false,
                                checked: matches!(instruction, Instruction::GetLocalCheck(_)),
                                keep: false,
                            });
                        }
                    }
                    FrameBinding::Uninitialized
                        if matches!(instruction, Instruction::GetLocalCheck(_)) =>
                    {
                        return Ok(RunExit::LexicalUninitialized(*index));
                    }
                    _ => false,
                }
            }

            Instruction::ThrowReadOnly(index) | Instruction::ThrowRedeclaration(index) => {
                return Ok(RunExit::BindingError {
                    index: *index,
                    redeclaration: matches!(instruction, Instruction::ThrowRedeclaration(_)),
                });
            }
            Instruction::InitializePrivateName(index)
            | Instruction::InitializePrivateMethod(index)
            | Instruction::InitializePrivateAccessor(index) => {
                use super::private_bindings::Initialization;
                let kind = match instruction {
                    Instruction::InitializePrivateName(_) => Initialization::Name,
                    Instruction::InitializePrivateMethod(_) => Initialization::Method,
                    _ => Initialization::Accessor,
                };
                return Ok(RunExit::PrivateInitialize {
                    index: *index,
                    kind,
                });
            }
            Instruction::GetPrivateField(source)
            | Instruction::GetPrivateField2(source)
            | Instruction::PutPrivateField(source)
            | Instruction::DefinePrivateField(source)
            | Instruction::PrivateIn(source) => {
                use super::private_access::Access;
                let access = match instruction {
                    Instruction::GetPrivateField(_) => Access::Get,
                    Instruction::GetPrivateField2(_) => Access::GetKeep,
                    Instruction::PutPrivateField(_) => Access::Put,
                    Instruction::DefinePrivateField(_) => Access::Define,
                    _ => Access::In,
                };
                return Ok(RunExit::PrivateAccess {
                    source: *source,
                    access,
                });
            }
            Instruction::Arguments(kind) => return Ok(RunExit::Arguments(*kind)),
            Instruction::Rest(start) => return Ok(RunExit::Rest(*start)),
            Instruction::SetName(index) => return Ok(RunExit::SetName(Some(*index))),
            Instruction::SetNameComputed => return Ok(RunExit::SetName(None)),
            Instruction::FClosure(index) => return Ok(RunExit::InstantiateClosure(*index)),
            Instruction::CloseLocal(index) => {
                // An uncaptured local keeps its value until the next scope entry.
                // Captured cells must first root and detach their shared value.
                if matches!(slots.local(*index)?, FrameBinding::Captured(_)) {
                    return Ok(RunExit::CloseCaptured(*index));
                } else {
                    if let Some(flag) = cold.reusable_captured_locals.get_mut(usize::from(*index)) {
                        *flag = false;
                    }
                    true
                }
            }
            Instruction::SetLocalUninitialized(index) => {
                if matches!(slots.local(*index)?, FrameBinding::Captured(_)) {
                    return Ok(RunExit::ResetCaptured(*index));
                }
                let ready = match slots.local(*index)? {
                    FrameBinding::Uninitialized => true,
                    FrameBinding::Direct(old) => {
                        runtime
                            .slot_value_release_readiness(old)
                            .map_err(runtime_error_to_vm_error)?
                            == SlotReleaseReadiness::Ready
                    }
                    _ => false,
                };
                if ready {
                    let old = slots.replace_local(*index, FrameBinding::Uninitialized)?;
                    if let Some(flag) = cold.reusable_captured_locals.get_mut(usize::from(*index)) {
                        *flag = false;
                    }
                    if matches!(old, FrameBinding::Direct(_)) {
                        release_displaced(runtime, old)?;
                    }
                }
                if !ready {
                    release_outside_slots!({
                        let old = slots.replace_local(*index, FrameBinding::Uninitialized)?;
                        if let Some(flag) =
                            cold.reusable_captured_locals.get_mut(usize::from(*index))
                        {
                            *flag = false;
                        }
                        old
                    });
                }
                true
            }
            Instruction::InitializeLocal(index) => {
                let definition = executable.local_definitions[usize::from(*index)];
                if definition.kind
                    == crate::engine::code::function::metadata::ClosureVariableKind::WithObject
                {
                    return Ok(RunExit::Environment(
                        super::environment_driver::Operation::InitializeWith(*index),
                    ));
                }
                let ready = definition.is_lexical
                    && definition.kind
                        == crate::engine::code::function::metadata::ClosureVariableKind::Normal
                    && match slots.local(*index)? {
                        FrameBinding::Uninitialized => true,
                        FrameBinding::Direct(old) => {
                            runtime
                                .slot_value_release_readiness(old)
                                .map_err(runtime_error_to_vm_error)?
                                == SlotReleaseReadiness::Ready
                        }
                        _ => false,
                    };
                if ready {
                    let next = slots.pop()?;
                    let old = slots.replace_local(*index, FrameBinding::Direct(next))?;
                    if matches!(old, FrameBinding::Direct(_)) {
                        release_displaced(runtime, old)?;
                    }
                }
                if !ready
                    && definition.is_lexical
                    && definition.kind
                        == crate::engine::code::function::metadata::ClosureVariableKind::Normal
                    && matches!(slots.local(*index)?, FrameBinding::Direct(_))
                {
                    release_outside_slots!({
                        let next = slots.pop()?;
                        slots.replace_local(*index, FrameBinding::Direct(next))?
                    });
                    true
                } else {
                    ready
                }
            }
            Instruction::PutLocalCheck(index) | Instruction::SetLocalCheck(index)
                if matches!(slots.local(*index)?, FrameBinding::Uninitialized) =>
            {
                return Ok(RunExit::LexicalUninitialized(*index));
            }
            Instruction::PutLocal(index)
            | Instruction::SetLocal(index)
            | Instruction::PutLocalCheck(index)
            | Instruction::SetLocalCheck(index) => {
                if matches!(slots.local(*index)?, FrameBinding::Direct(old) if runtime.slot_value_release_readiness(old).map_err(runtime_error_to_vm_error)? == SlotReleaseReadiness::Ready)
                {
                    let next = if matches!(
                        instruction,
                        Instruction::SetLocal(_) | Instruction::SetLocalCheck(_)
                    ) {
                        copy_value(slots.peek(0)?)?
                    } else {
                        slots.pop()?
                    };
                    let old = slots.replace_local(*index, FrameBinding::Direct(next))?;
                    release_displaced(runtime, old)?;
                    true
                } else if matches!(slots.local(*index)?, FrameBinding::Direct(_)) {
                    release_outside_slots!({
                        let next = if matches!(
                            instruction,
                            Instruction::SetLocal(_) | Instruction::SetLocalCheck(_)
                        ) {
                            copy_value(slots.peek(0)?)?
                        } else {
                            slots.pop()?
                        };
                        slots.replace_local(*index, FrameBinding::Direct(next))?
                    });
                    true
                } else {
                    false
                }
            }
            Instruction::GetArg(index) => {
                if let FrameBinding::Direct(value) = slots.parameter(*index)? {
                    let copied = copy_value(value)?;
                    slots.push(copied)?;
                    true
                } else {
                    false
                }
            }
            Instruction::PutArg(index) | Instruction::SetArg(index) => {
                if matches!(slots.parameter(*index)?, FrameBinding::Direct(old) if runtime.slot_value_release_readiness(old).map_err(runtime_error_to_vm_error)? == SlotReleaseReadiness::Ready)
                {
                    let next = if matches!(instruction, Instruction::SetArg(_)) {
                        copy_value(slots.peek(0)?)?
                    } else {
                        slots.pop()?
                    };
                    let old = slots.replace_parameter(*index, FrameBinding::Direct(next))?;
                    release_displaced(runtime, old)?;
                    true
                } else if matches!(slots.parameter(*index)?, FrameBinding::Direct(_)) {
                    release_outside_slots!({
                        let next = if matches!(instruction, Instruction::SetArg(_)) {
                            copy_value(slots.peek(0)?)?
                        } else {
                            slots.pop()?
                        };
                        slots.replace_parameter(*index, FrameBinding::Direct(next))?
                    });
                    true
                } else {
                    false
                }
            }
            Instruction::Dup => {
                slots.insert_copy(0, 0)?;
                true
            }
            Instruction::Dup1 => {
                slots.insert_copy(1, 1)?;
                true
            }
            Instruction::Dup3 => {
                slots.duplicate_operands(3)?;
                true
            }
            Instruction::Insert2 | Instruction::Insert3 | Instruction::Insert4 => {
                let count = match instruction {
                    Instruction::Insert2 => 2,
                    Instruction::Insert3 => 3,
                    _ => 4,
                };
                slots.peek(count - 1)?;
                slots.insert_copy(0, count)?;
                true
            }
            Instruction::Perm3 | Instruction::Perm4 | Instruction::Perm5 => {
                let count = match instruction {
                    Instruction::Perm3 => 2,
                    Instruction::Perm4 => 3,
                    _ => 4,
                };
                slots.rotate_operands(1, count, false)?;
                true
            }
            Instruction::Rot4Left => {
                slots.rotate_operands(0, 4, true)?;
                true
            }
            Instruction::Drop => {
                if slots.release_operand(0, runtime)? {
                    slots.pop()?;
                    true
                } else {
                    release_outside_slots!(slots.pop()?);
                    true
                }
            }
            Instruction::Swap => {
                slots.rotate_operands(0, 2, false)?;
                true
            }
            Instruction::Nip => {
                if slots.release_operand(1, runtime)? {
                    let right = slots.pop()?;
                    slots.pop()?;
                    slots.push(right)?;
                    true
                } else {
                    release_outside_slots!({
                        slots.peek(1)?;
                        let kept = slots.pop()?;
                        let released = slots.pop()?;
                        slots.push(kept)?;
                        released
                    });
                    true
                }
            }
            Instruction::Add => {
                if binary(&mut slots, |a, b| value(a.add(b)))? {
                    true
                } else {
                    return Ok(RunExit::ConvertAdd);
                }
            }
            Instruction::Sub => binary(&mut slots, |a, b| value(a.sub(b)))?,
            Instruction::Mul => binary(&mut slots, |a, b| value(a.mul(b)))?,
            Instruction::Div => binary(&mut slots, |a, b| value(a.div(b)))?,
            Instruction::Mod => binary(&mut slots, |a, b| value(a.rem(b)))?,
            Instruction::Pow => binary(&mut slots, |a, b| value(a.pow(b)))?,
            Instruction::Shl => binary(&mut slots, |a, b| {
                Value::Int(a.int32().wrapping_shl(b.int32() as u32 & 31))
            })?,
            Instruction::Sar => binary(&mut slots, |a, b| {
                Value::Int(a.int32() >> (b.int32() as u32 & 31))
            })?,
            Instruction::Shr => binary(&mut slots, |a, b| {
                value(Number::compact(f64::from(
                    (a.int32() as u32) >> (b.int32() as u32 & 31),
                )))
            })?,
            Instruction::BitAnd => binary(&mut slots, |a, b| Value::Int(a.int32() & b.int32()))?,
            Instruction::BitOr => binary(&mut slots, |a, b| Value::Int(a.int32() | b.int32()))?,
            Instruction::BitXor => binary(&mut slots, |a, b| Value::Int(a.int32() ^ b.int32()))?,
            Instruction::Lt
            | Instruction::Lte
            | Instruction::Gt
            | Instruction::Gte
            | Instruction::Eq
            | Instruction::Neq
            | Instruction::StrictEq
            | Instruction::StrictNeq
                if executable.fusion.compare_branch(pc.fault) =>
            {
                let branch_pc = pc.fault + 1;
                let Some(target) =
                    fusion::compare_branch(&mut slots, instruction, &executable.code[branch_pc])?
                else {
                    if matches!(instruction, Instruction::StrictEq | Instruction::StrictNeq) {
                        return Ok(RunExit::StrictEquality(matches!(
                            instruction,
                            Instruction::StrictNeq
                        )));
                    }
                    return Ok(RunExit::Numeric(
                        super::numeric::operation::NumericKind::for_instruction(instruction)
                            .ok_or_else(|| cold::internal("comparison has no numeric operation"))?,
                    ));
                };
                #[cfg(feature = "profiling")]
                fusion::record_span(&executable.code[pc.fault..pc.fault + 2], observed_depth);
                pc.resume = if target == usize::MAX {
                    pc.fault + 2
                } else {
                    target
                };
                continue;
            }
            Instruction::Lt => binary(&mut slots, |a, b| Value::Bool(a.float() < b.float()))?,
            Instruction::Lte => binary(&mut slots, |a, b| Value::Bool(a.float() <= b.float()))?,
            Instruction::Gt => binary(&mut slots, |a, b| Value::Bool(a.float() > b.float()))?,
            Instruction::Gte => binary(&mut slots, |a, b| Value::Bool(a.float() >= b.float()))?,
            Instruction::StrictEq | Instruction::StrictNeq => {
                let negate = matches!(instruction, Instruction::StrictNeq);
                if binary(&mut slots, |a, b| {
                    Value::Bool((a.float() == b.float()) != negate)
                })? {
                    true
                } else {
                    return Ok(RunExit::StrictEquality(negate));
                }
            }
            Instruction::Eq => binary(&mut slots, |a, b| Value::Bool(a.float() == b.float()))?,
            Instruction::Neq => binary(&mut slots, |a, b| Value::Bool(a.float() != b.float()))?,
            Instruction::Not => return Ok(RunExit::LogicalNot),
            Instruction::Neg
            | Instruction::Plus
            | Instruction::BitNot
            | Instruction::Inc
            | Instruction::Dec
            | Instruction::PostInc
            | Instruction::PostDec => {
                if let Some(old) = number(slots.peek(0)?) {
                    let next = match instruction {
                        Instruction::Neg => old.negate(),
                        Instruction::Plus => old,
                        Instruction::BitNot => Number::Int(!old.int32()),
                        _ => old.update(matches!(
                            instruction,
                            Instruction::Inc | Instruction::PostInc
                        )),
                    };
                    if !matches!(instruction, Instruction::PostInc | Instruction::PostDec) {
                        slots.pop()?;
                    }
                    slots.push(value(next))?;
                    true
                } else if matches!(instruction, Instruction::Plus) {
                    return Ok(RunExit::ConvertPlus);
                } else {
                    false
                }
            }
            Instruction::Goto(target) => {
                next_pc = *target as usize;
                true
            }
            Instruction::IfTrue(target) | Instruction::IfFalse(target)
                if immediate(slots.peek(0)?) =>
            {
                let truthy = slots.pop()?.to_boolean_primitive();
                if truthy == matches!(instruction, Instruction::IfTrue(_)) {
                    next_pc = *target as usize;
                }
                true
            }
            Instruction::IfTrue(target) | Instruction::IfFalse(target) => {
                return Ok(RunExit::Pure(
                    super::pure_operations::PureOperation::Branch {
                        target: *target,
                        when: matches!(instruction, Instruction::IfTrue(_)),
                    },
                ));
            }
            Instruction::Catch(target) => return Ok(RunExit::Catch(*target)),
            Instruction::DropCatch => return Ok(RunExit::DropCatch),
            Instruction::NipCatch => return Ok(RunExit::NipCatch),
            Instruction::Throw => return Ok(RunExit::Throw),
            Instruction::Gosub(target) => {
                let pc = i32::try_from(next_pc)
                    .map_err(|_| cold::internal("gosub return PC does not fit Int"))?;
                slots.push(Value::Int(pc))?;
                next_pc = *target as usize;
                true
            }
            Instruction::Ret => {
                let Value::Int(target) = slots.pop()? else {
                    return Err(cold::internal("invalid ret value"));
                };
                next_pc =
                    usize::try_from(target).map_err(|_| cold::internal("invalid ret value"))?;
                if next_pc >= executable.code.len() {
                    return Err(cold::internal("invalid ret value"));
                }
                true
            }
            Instruction::DropGosub => {
                if !matches!(slots.pop()?, Value::Int(_)) {
                    return Err(cold::internal("invalid gosub cleanup value"));
                }
                true
            }
            Instruction::InitialYield
            | Instruction::Yield
            | Instruction::YieldStar
            | Instruction::AsyncYieldStar
            | Instruction::Await => {
                let kind = match instruction {
                    Instruction::InitialYield => super::VmSuspendKind::Initial,
                    Instruction::Yield => super::VmSuspendKind::Yield,
                    Instruction::YieldStar => super::VmSuspendKind::YieldStar,
                    Instruction::AsyncYieldStar => super::VmSuspendKind::AsyncYieldStar,
                    Instruction::Await => super::VmSuspendKind::Await,
                    _ => unreachable!(),
                };
                if kind != super::VmSuspendKind::Initial {
                    slots.peek(0)?;
                }
                pc.resume = next_pc;
                #[cfg(feature = "profiling")]
                cold::instruction(observed_depth);
                return Ok(RunExit::Suspend(kind));
            }
            Instruction::Return => {
                execution.pending = Some(slots.pop()?);
                pc.resume = next_pc;
                #[cfg(feature = "profiling")]
                cold::instruction(observed_depth);
                return Ok(RunExit::Complete);
            }
            Instruction::ReturnUndefined => {
                execution.pending = Some(Value::Undefined);
                pc.resume = next_pc;
                #[cfg(feature = "profiling")]
                cold::instruction(observed_depth);
                return Ok(RunExit::Complete);
            }
        };
        if !handled {
            if let Some(kind) = super::numeric::operation::NumericKind::for_instruction(instruction)
            {
                if numeric::supported(&slots, kind) {
                    // Symbol release and BigInt errors may observe the stack.
                    // Number/String/bool coercions cannot construct a JS error.
                    if !frame.active_frame.is_materialized()
                        && (0..if kind.unary() { 1 } else { 2 }).any(|i| {
                            matches!(slots.peek(i), Ok(Value::Symbol(_) | Value::BigInt(_)))
                        })
                    {
                        return Ok(RunExit::Materialize);
                    }
                    // Preserve active-PC admission before consuming operands,
                    // then keep this transaction and run frame across parsing.
                    drop(slots);
                    pc.publish_fault();
                    if frame.active_frame.is_materialized() {
                        runtime
                            .update_active_bytecode_pc(
                                frame.active_frame,
                                super::BytecodePc::new(pc.fault),
                            )
                            .map_err(runtime_error_to_vm_error)?;
                    }
                    if !numeric::complete(
                        runtime,
                        executable.realm,
                        &mut transaction,
                        kind,
                        &mut execution.pending,
                    )? {
                        return Ok(RunExit::PrimitiveThrow);
                    }
                    slots = transaction.slots();
                } else {
                    return Ok(RunExit::Numeric(kind));
                }
            } else {
                return Ok(RunExit::Bridge);
            }
        }
        #[cfg(feature = "profiling")]
        cold::instruction(observed_depth);
        pc.resume = next_pc;
    }
}

#[cfg(all(test, feature = "profiling"))]
mod tests {
    use crate::engine::api::profiling::CostProfile;
    use crate::engine::api::{Runtime, Value};
    use crate::engine::heap::SlotReleaseReadiness;

    #[test]
    fn property_ic_resides_for_own_prototype_and_method_reads() {
        for holder in ["({x:{answer:42}})", "Object.create({x:{answer:42}})"] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context.eval(&format!("var icHolder={holder}; function icRead(n){{var r;for(var i=0;i<n;i++)r=icHolder.x;return r;}} icRead(2)")).unwrap();
            let expected = context.eval("icHolder.x").unwrap();
            let profile = CostProfile::start();
            assert_eq!(context.eval("icRead(20)").unwrap(), expected);
            let costs = profile.snapshot();
            assert_eq!(
                costs.owned_execution_events.get("property_ic.hit"),
                Some(&20)
            );
            assert_eq!(
                costs
                    .owned_execution_events
                    .get("run_exit.GetField")
                    .copied()
                    .unwrap_or(0),
                0
            );
            assert_eq!(costs.owned_bridge_exits, 0);
        }
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.eval("var icMethodHolder={min:Math.min};function icMethod(n){var r;for(var i=0;i<n;i++)r=icMethodHolder.min(42,43);return r;}icMethod(2)").unwrap();
        let profile = CostProfile::start();
        assert_eq!(context.eval("icMethod(20)").unwrap(), Value::Int(42));
        let costs = profile.snapshot();
        assert_eq!(
            costs.owned_execution_events.get("property_ic.hit"),
            Some(&20)
        );
        assert_eq!(
            costs.owned_execution_events.get("method_call_span"),
            Some(&20)
        );
        assert_eq!(
            costs
                .owned_execution_events
                .get("run_exit.GetField")
                .copied()
                .unwrap_or(0),
            0
        );
        drop(profile);
        context.eval("icMethodHolder.min=Math.max").unwrap();
        assert_eq!(context.eval("icMethod(3)").unwrap(), Value::Int(43));
    }

    #[test]
    fn property_ic_invalidations_preserve_accessor_receiver_and_current_value() {
        for source in [
            "var o={x:1};function r(){return o.x;}r();r();o.x=42;r()",
            "var p={x:1},o=Object.create(p);function r(){return o.x;}r();r();Object.defineProperty(p,'x',{get(){return this.y},configurable:true});o.y=42;r()",
            "var p={x:1},m=Object.create(p),o=Object.create(m);function r(){return o.x;}r();r();Object.setPrototypeOf(m,{x:42});r()",
            "var o={x:1};function r(){return o.x;}r();r();delete o.x;Object.setPrototypeOf(o,{x:42});r()",
            "var o={x:1};function r(){return o.x;}r();r();Object.defineProperty(o,'x',{get(){return 42}});r()",
            "var o={x:1};function r(){return o.x;}r();r();for(var i=0;i<40;i++)o['k'+i]=i;delete o.k0;o.x=42;r()",
            "var o={v:42,f(){return this.v}};function r(){return o.f()}r();r();r()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            assert_eq!(context.eval(source).unwrap(), Value::Int(42), "{source}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn resident_ordinary_fields_keep_scalar_updates_and_following_pc() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        assert_eq!(context.eval("(function(){var o={x:0,u:undefined,n:null,b:true,f:1.5,z:-0};for(var i=0;i<8;i++){o.x=i;o.x+=1;if(o.x!==i+1)return 0;}if(o.u!==undefined||o.n!==null||o.b!==true||o.f!==1.5||!Object.is(o.z,-0))return 0;try{throw o.x}catch(e){return e+34}})()").unwrap(),Value::Int(42));
        let costs = profile.snapshot();
        assert!(
            costs
                .owned_execution_events
                .get("ordinary_field_immediate_read_in_run")
                .copied()
                .unwrap_or(0)
                + costs
                    .owned_execution_events
                    .get("property_ic.hit")
                    .copied()
                    .unwrap_or(0)
                >= 8
        );
        for event in ["ordinary_field_immediate_write_in_run"] {
            assert!(
                costs
                    .owned_execution_events
                    .get(event)
                    .copied()
                    .unwrap_or(0)
                    >= 8,
                "{event}: {costs:?}"
            );
        }
        assert_eq!(costs.legacy_dispatches, 0);
        assert_eq!(costs.owned_bridge_exits, 0);
        assert_eq!(costs.owned_sync_call_bridges, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn resident_ordinary_field_fallback_preserves_accessor_proxy_strict_and_receiver_rules() {
        for source in [
            "(function(){var n=0,o={get x(){n++;return 41}};return o.x+n})()",
            "(function(){var n=0,o={set x(v){n++;this.y=v}};o.x=41;return o.y+n})()",
            "(function(){var n=0,o=new Proxy({x:40},{get(t,k,r){n++;return Reflect.get(t,k,r)},set(t,k,v,r){n++;return Reflect.set(t,k,v,r)}});o.x=40;return o.x+n})()",
            "(function(){var marker={},n=0,o={get x(){n++;throw marker}};try{o.x;return 0}catch(e){return e===marker&&n===1?42:0}})()",
            "(function(){'use strict';var o=Object.freeze({x:7});try{o.x=17;return 0}catch(e){return e instanceof TypeError&&o.x===7?42:0}})()",
            "(function(){var o=Object.create({x:7});o.x=42;return o.x})()",
            "(function(){var marker={},o={x:marker};if(o.x!==marker)return 0;o.x=42;return o.x})()",
            "(function(){var o={x:42,f(){return this.x}};return o.f()})()",
            "(function(){return ({x:42}).x})()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            assert_eq!(context.eval(source).unwrap(), Value::Int(42), "{source}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn resident_typed_reads_share_decoding_and_following_exception_pc() {
        for kind in [
            "Int8",
            "Uint8",
            "Uint8Clamped",
            "Int16",
            "Uint16",
            "Int32",
            "Uint32",
            "Float16",
            "Float32",
            "Float64",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let source = format!(
                "(function(){{var a=new {kind}Array(1),n=0;for(var value of [257.5,-129.5,NaN,Infinity,-0]){{a[0]=value;var x=a[0];if(!Object.is(x,Reflect.get(a,'0')))return 0;n++}}a[0]=37;try{{throw a[0]}}catch(e){{return e+n}}}})()"
            );
            let profile = CostProfile::start();
            assert_eq!(context.eval(&source).unwrap(), Value::Int(42), "{kind}");
            let costs = profile.snapshot();
            assert!(
                costs
                    .owned_execution_events
                    .get("typed_array_number_read_leaf")
                    .copied()
                    .unwrap_or(0)
                    >= 6,
                "{kind}: {costs:?}"
            );
            assert_eq!(costs.legacy_dispatches, 0);
            assert_eq!(costs.owned_bridge_exits, 0);
            assert_eq!(costs.owned_sync_call_bridges, 0);
        }
    }

    #[test]
    fn resident_typed_read_fallback_keeps_resizing_conversion_and_proxy_once() {
        for source in [
            "(function(){var b=new ArrayBuffer(4,{maxByteLength:8}),a=new Uint8Array(b,2,2),track=new Uint8Array(b,2);a[0]=42;b.resize(1);if(a[0]!==undefined||track[0]!==undefined)return 0;b.resize(8);a[0]=42;return a[0]===42&&track[0]===42&&track[5]===0?42:0})()",
            "(function(){var a=new Uint8Array([42]),n=0,key={toString(){n++;a.buffer.transfer();return '0'}};return a[key]===undefined&&n===1?42:0})()",
            "(function(){var b=new ArrayBuffer(4,{maxByteLength:8}),a=new Uint8Array(b),n=0,key={toString(){n++;b.resize(0);return '0'}};return a[key]===undefined&&n===1?42:0})()",
            "(function(){var a=new Uint8Array([42]),n=0,marker={},key={toString(){n++;throw marker}};try{a[key];return 0}catch(e){return e===marker&&n===1?42:0}})()",
            "(function(){var n=0,a=new Proxy(new Uint8Array([41]),{get(t,k){n++;return Reflect.get(t,k)}});return a[0]+n})()",
            "(function(){var a=new Uint8Array(new SharedArrayBuffer(1));a[0]=42;return a[0]})()",
            "(function(){var a=new BigInt64Array([42n]);return a[0]===42n?42:0})()",
            "(function(){return new Uint8Array([42])[0]})()",
            "(function(){var a=new Uint8Array([42]);return a[-1]===undefined&&a['-0']===undefined&&a[1]===undefined?42:0})()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            assert_eq!(context.eval(source).unwrap(), Value::Int(42), "{source}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn resident_dense_reads_keep_scalar_results_and_following_pc() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        assert_eq!(context.eval("(function(){var a=[undefined,null,true,42,1.5,-0];if(a[0]!==undefined||a[1]!==null||a[2]!==true||a[3]!==42||a[4]!==1.5||!Object.is(a[5],-0))return 0;try{var x=a[3];throw x}catch(e){return e}})()").unwrap(), Value::Int(42));
        let costs = profile.snapshot();
        assert!(
            costs
                .owned_execution_events
                .get("array_immediate_read_in_run")
                .copied()
                .unwrap_or(0)
                >= 7,
            "{costs:?}"
        );
        assert_eq!(costs.legacy_dispatches, 0);
        assert_eq!(costs.owned_bridge_exits, 0);
        assert_eq!(costs.owned_sync_call_bridges, 0);
    }

    #[test]
    fn resident_dense_read_fallback_keeps_getter_proxy_and_method_effects_once() {
        for source in [
            "(function(){var n=0,a=[];Object.defineProperty(a,0,{get(){n++;return 41}});return a[0]+n})()",
            "(function(){var n=0,a=new Proxy([41],{get(t,k,r){n++;return Reflect.get(t,k,r)}});return a[0]+n})()",
            "(function(){var n=0,marker={},a=[];Object.defineProperty(a,0,{get(){n++;throw marker}});try{a[0];return 0}catch(e){return n===1&&e===marker?42:0}})()",
            "(function(){var a=[,];Object.setPrototypeOf(a,{0:42});return a[0]})()",
            "(function(){var marker={},a=[marker];return a[0]===marker?42:0})()",
            "(function(){var a=[function(){return this.x}];a.x=42;return a[0]()})()",
            "(function(){var a=[40];a[0]++;return a[0]+1})()",
            "(function(){return [42][0]})()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            assert_eq!(context.eval(source).unwrap(), Value::Int(42), "{source}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn resident_typed_writes_share_numeric_encoding_and_preserve_following_pc() {
        for kind in [
            "Int8",
            "Uint8",
            "Uint8Clamped",
            "Int16",
            "Uint16",
            "Int32",
            "Uint32",
            "Float16",
            "Float32",
            "Float64",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let source = format!(
                "(function(){{var C={kind}Array, a=new C(1), b=new C(1),n=0;for(var value of [257.5,-129.5,NaN,Infinity,-0]){{a[0]=value;Reflect.set(b,'0',value);if(!Object.is(a[0],b[0]))return 0;n++}}try{{a[0]=7;throw 35}}catch(e){{return a[0]+e+n}}}})()"
            );
            let profile = CostProfile::start();
            assert_eq!(context.eval(&source).unwrap(), Value::Int(47), "{kind}");
            let costs = profile.snapshot();
            assert!(
                costs
                    .owned_execution_events
                    .get("typed_array_number_write_in_run")
                    .copied()
                    .unwrap_or(0)
                    >= 6,
                "{kind}: {costs:?}"
            );
            assert_eq!(costs.legacy_dispatches, 0);
            assert_eq!(costs.owned_bridge_exits, 0);
            assert_eq!(costs.owned_sync_call_bridges, 0);
        }
    }

    #[test]
    fn resident_typed_write_fallback_keeps_conversion_exception_and_receiver_rules() {
        for source in [
            "(function(){var a=new Uint8Array(1),n=0;a[0]={valueOf(){n++;return 42}};return a[0]===42&&n===1?42:0})()",
            "(function(){var a=new Uint8Array(1),marker={},n=0;try{a[0]={valueOf(){n++;throw marker}};return 0}catch(e){return e===marker&&n===1&&a[0]===0?42:0}})()",
            "(function(){'use strict';var a=new Uint8Array(1);a[3]=42;a[-1]=42;a['-0']=42;return a[0]===0&&a[3]===undefined&&a['-0']===undefined?42:0})()",
            "(function(){var a=new Uint8Array(1),n=0;a.buffer.transfer();a[0]=7;a[0]={valueOf(){n++;return 42}};return a[0]===undefined&&n===1?42:0})()",
            "(function(){var a=new Uint8Array(new SharedArrayBuffer(1));a[0]=42;return a[0]})()",
            "(function(){var a=new BigInt64Array(1);try{a[0]=42;return 0}catch(e){a[0]=42n;return e instanceof TypeError&&a[0]===42n?42:0}})()",
            "(function(){var n=0,p=new Proxy(new Uint8Array(1),{set(t,k,v){n++;return Reflect.set(t,k,v,t)}});p[0]=42;return n===1?42:0})()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            assert_eq!(context.eval(source).unwrap(), Value::Int(42), "{source}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn ordinary_recursion_uses_one_execution_on_a_two_mib_native_stack() {
        std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let runtime = Runtime::new();
                let mut context = runtime.new_context();
                let Value::Object(function) = context
                    .eval("(function f(n){if(n===0)return 0;return 1+f(n-1)})")
                    .unwrap()
                else {
                    panic!("expected function");
                };
                let callable = runtime.as_callable(&function).unwrap().unwrap();
                let profile = CostProfile::start();
                assert_eq!(
                    context
                        .call(&callable, Value::Undefined, &[Value::Int(1000)])
                        .unwrap(),
                    Value::Int(1000)
                );
                let costs = profile.snapshot();
                assert_eq!(costs.owned_storage.maximum_frame_depth, 1001, "{costs:?}");
                assert_eq!(costs.owned_storage.frames_pushed, 1001, "{costs:?}");
                assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
                assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
                assert!(runtime.0.state.borrow().active_frames.is_empty());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn tail_calls_return_through_owned_parents() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context
            .eval("(function f(n){if(n===0)return 7;return f(n-1)})")
            .unwrap()
        else {
            panic!("expected function");
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        let profile = CostProfile::start();
        assert_eq!(
            context
                .call(&callable, Value::Undefined, &[Value::Int(64)])
                .unwrap(),
            Value::Int(7)
        );
        let costs = profile.snapshot();
        assert_eq!(costs.owned_storage.maximum_frame_depth, 65);
        assert_eq!(costs.legacy_dispatches, 0);
        assert_eq!(costs.owned_bridge_exits, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn child_throw_propagates_through_owned_parent_once() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        let result = context.eval("var calls=0;function outer(f){return 1+f()} function inner(){calls++; [] instanceof Array;throw 42} var result;try{outer(inner)}catch(e){result=e} result*10+calls").unwrap();
        assert_eq!(result, Value::Int(421));
        let costs = profile.snapshot();
        assert_eq!(costs.owned_storage.maximum_frame_depth, 3, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn ordinary_loop_finishes_in_the_owned_core() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context
            .eval("(function(n) { var s=0; for(var i=0;i<n;i++) s=s+i; return s; })")
            .unwrap()
        else {
            panic!("expected function");
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        let profile = CostProfile::start();
        let result = context
            .call(&callable, Value::Undefined, &[Value::Int(100)])
            .unwrap();
        assert_eq!(result, Value::Int(4950));
        let costs = profile.snapshot();
        assert!(costs.owned_instructions > 1000, "{costs:?}");
        // Both frame PCs are materialized once when the owned loop exits.
        let fault_writes = costs.owned_execution_events["run_frame_fault_pc_write"];
        assert_eq!(fault_writes, 1);
        assert_eq!(costs.owned_execution_events["run_frame_resume_pc_write"], 1);
        assert_eq!(costs.owned_execution_events["runtime_pc_publication"], 1);
        assert!(
            costs.owned_execution_events["slot_authentication"] < 20,
            "{costs:?}"
        );
        assert_eq!(
            costs.owned_bridge_exits, 0,
            "the ordinary numeric loop must not use the bridge"
        );
        assert_eq!(
            costs.legacy_dispatches, 0,
            "the measured call must finish entirely in the owned core"
        );
    }

    #[test]
    fn last_object_binding_replacement_releases_in_owned_cold_operations() {
        for source in [
            "(function(){var x=new ArrayBuffer(16);x=42;return x})",
            "(function(){var x=new ArrayBuffer(16);return x=42})",
            "(function(a){a=new ArrayBuffer(16);a=42;return a})",
            "(function(a){a=new ArrayBuffer(16);return a=42})",
            "(function(){for(var i=0;i<2;i++){let x=new ArrayBuffer(16);if(i===1)return 42}})",
            "(function(){var x={child:new ArrayBuffer(16)};x=42;return x})",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let Value::Object(function) = context.eval(source).unwrap() else {
                panic!("function expected")
            };
            let callable = runtime.as_callable(&function).unwrap().unwrap();
            let profile = CostProfile::start();
            assert_eq!(
                context.call(&callable, Value::Undefined, &[]).unwrap(),
                Value::Int(42),
                "{source}"
            );
            let costs = profile.snapshot();
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_sync_call_bridges, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn shared_object_binding_replacement_finishes_without_a_bridge() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context
            .eval("(function(a){var x=a;x=0;a=0;return 1})")
            .unwrap()
        else {
            panic!("expected function");
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        let root = runtime.new_object(None).unwrap();
        let argument = Value::Object(root.try_clone().unwrap());
        let profile = CostProfile::start();
        assert_eq!(
            context
                .call(&callable, Value::Undefined, &[argument])
                .unwrap(),
            Value::Int(1)
        );
        let costs = profile.snapshot();
        assert!(costs.owned_instructions > 0, "{costs:?}");
        assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
        assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        assert_eq!(costs.owned_storage.frames_pushed, 1);
        assert!(costs.owned_storage.copied_heap_roots > 0);
        assert!(costs.owned_storage.hot_heap_root_releases > 0);
        assert!(
            matches!(
                runtime
                    .slot_value_release_readiness(&Value::Object(root))
                    .unwrap(),
                SlotReleaseReadiness::QueueCapacity | SlotReleaseReadiness::Drain
            ),
            "the completed call must leave no extra root for its object argument"
        );
    }

    #[test]
    fn numeric_edges_use_the_owned_core() {
        for (expression, expected) in [
            ("2147483647+1", 2147483648.0_f64),
            ("-2147483648-1", -2147483649.0),
            ("0*-1", -0.0),
            ("1/0", f64::INFINITY),
            ("-0%3", -0.0),
            ("(-1)**(1/0)", f64::NAN),
            ("-1>>>0", 4294967295.0),
            ("1<<33", 2.0),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let Value::Object(function) = context
                .eval(&format!("(function(){{return {expression}}})"))
                .unwrap()
            else {
                panic!("expected function");
            };
            let callable = runtime.as_callable(&function).unwrap().unwrap();
            let profile = CostProfile::start();
            let result = context
                .call(&callable, Value::Undefined, &[])
                .unwrap()
                .as_number()
                .unwrap();
            if expected.is_nan() {
                assert!(result.is_nan());
            } else {
                assert_eq!(result.to_bits(), expected.to_bits(), "{expression}");
            }
            assert!(profile.snapshot().owned_instructions > 0, "{expression}");
            assert_eq!(profile.snapshot().owned_bridge_exits, 0, "{expression}");
            assert_eq!(profile.snapshot().legacy_dispatches, 0, "{expression}");
        }
    }

    #[test]
    fn shared_primitive_literals_return_without_a_bridge() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for (expression, expected) in [
            (
                "'owned literal'",
                Value::String(crate::engine::value::JsString::from_static("owned literal")),
            ),
            (
                "170141183460469231731687303715884105727n",
                Value::BigInt(i128::MAX.into()),
            ),
        ] {
            let Value::Object(function) = context
                .eval(&format!("(function(){{return {expression}}})"))
                .unwrap()
            else {
                panic!("expected function");
            };
            let callable = runtime.as_callable(&function).unwrap().unwrap();
            let profile = CostProfile::start();
            let result = context.call(&callable, Value::Undefined, &[]).unwrap();
            let costs = profile.snapshot();
            assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
            assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
            assert!(costs.owned_instructions > 0, "{costs:?}");
            drop(callable);
            drop(function);
            assert_eq!(result, expected);
        }
    }

    #[test]
    fn string_and_bigint_addition_use_owned_conversion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context.eval("(function(a,b){return a+b})").unwrap() else {
            panic!("expected function");
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        for (arguments, expected) in [
            (
                [
                    Value::String(crate::engine::value::JsString::from_static("1")),
                    Value::Int(2),
                ],
                Value::String(crate::engine::value::JsString::from_static("12")),
            ),
            (
                [Value::BigInt(1.into()), Value::BigInt(2.into())],
                Value::BigInt(3.into()),
            ),
        ] {
            let profile = CostProfile::start();
            assert_eq!(
                context
                    .call(&callable, Value::Undefined, &arguments)
                    .unwrap(),
                expected
            );
            let costs = profile.snapshot();
            assert!(costs.owned_instructions > 0, "{costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{costs:?}");
            assert_eq!(costs.legacy_dispatches, 0, "{costs:?}");
        }
    }

    #[test]
    fn owned_conversion_preserves_operands_pc_and_one_callback() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        let result = context.eval_with_filename("var calls=0; function f(a){var x=3;return (x+a)+(x=2)} var v=f({valueOf(){calls++; [] instanceof Array;return 4}}); v*10+calls", "owned-handoff.js").unwrap();
        assert_eq!(result, Value::Int(91));
        let costs = profile.snapshot();
        assert!(costs.owned_instructions > 0);
        assert_eq!(costs.owned_bridge_exits, 0);
        assert_eq!(costs.legacy_dispatches, 0);
    }
}

/// Strict equality has no user conversion. Retain both operands until the
/// result is known, then release them outside the resident instruction match.
#[inline(never)]
pub(super) fn strict_comparison(
    execution: &mut RunningExecution,
    id: FrameId,
    negate: bool,
) -> Result<(), Error> {
    let frame = execution.frames.current_mut(id)?;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let right = execution.slots.pop(&mut frame.window)?;
    let left = execution.slots.pop(&mut frame.window)?;
    let equal = left.strict_equal(&right);
    execution
        .slots
        .push(&mut frame.window, Value::Bool(equal != negate))?;
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| cold::internal("comparison resume PC overflow"))?;
    #[cfg(feature = "profiling")]
    cold::instruction(depth);
    Ok(())
}
