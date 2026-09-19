//! Original eval compiles and captures before entering an explicit child frame.
use super::{
    Completion, DirectEvalInvocation,
    call::{BytecodeCallRequest, CallableExecution},
    driver::{CallStep, push_frame},
    eval_bindings::{self, PreparedEvalEnvironment},
    exception::runtime_error_to_vm_error,
    execution::RunningExecution,
    frame::{FrameId, OperationTarget, ReturnTarget, ReturnValue},
};
use crate::engine::{
    api::{Error, runtime::Runtime},
    builtins::DirectEvalPreparation,
    code::function::metadata::EvalBindingSource,
    value::{JsValue, Value, conversion::NativeConversion},
};

#[inline(never)]
pub(super) fn step(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    arguments: u16,
    environment: u16,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let function = execution
        .slots
        .peek(&frame.window, usize::from(arguments))?;
    if !runtime
        .is_original_eval_jsvalue(realm, function)
        .map_err(runtime_error_to_vm_error)?
    {
        return super::driver::enter_call(runtime, execution, id, arguments, false, false);
    }
    let result = prepare_and_enter(runtime, execution, id, arguments, environment);
    if !matches!(result, Ok(CallStep::Entered)) {
        execution.frames.current_mut(id)?.cold.eval_arguments = None;
    }
    let Err(error) = result else { return result };
    let Some(kind) =
        crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
    else {
        return Err(error);
    };
    Ok(CallStep::Complete(Completion::Throw(
        runtime
            .new_native_error_from_error_jsvalue(realm, kind, &error)
            .map_err(runtime_error_to_vm_error)?,
    )))
}

#[inline(never)]
fn prepare_and_enter(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    arguments: u16,
    environment: u16,
) -> Result<CallStep, Error> {
    let can_push = execution.frames.can_push();
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    // `input` stays a public root: the direct-eval preparation consumes the
    // public invocation form, and the rare-cell cache is a public-root island.
    let input = if let Some(values) = &frame.cold.eval_arguments {
        values.first().cloned().unwrap_or(Value::Undefined)
    } else if arguments == 0 {
        Value::Undefined
    } else {
        runtime
            .root_value(
                execution
                    .slots
                    .peek(&frame.window, usize::from(arguments) - 1)?,
            )
            .map_err(runtime_error_to_vm_error)?
    };
    let string = matches!(input, Value::String(_));
    let this_value = if !string {
        frame.cold.input.this_value.clone()
    } else if let Some(value) = frame
        .cold
        .rare
        .get()
        .and_then(|rare| rare.normalized_this.as_ref())
    {
        value.clone()
    } else if frame.executable.metadata.strict
        || matches!(frame.cold.input.this_value, JsValue::Object(_))
    {
        frame.cold.input.this_value.clone()
    } else if matches!(frame.cold.input.this_value, JsValue::Null | JsValue::Undefined) {
        let id = frame.cold.input.callee_global(runtime, realm)?.object_id();
        runtime
            .retain_object_handle(id)
            .map_err(runtime_error_to_vm_error)?;
        JsValue::Object(id)
    } else {
        let value = match runtime
            .native_to_object_jsvalue(
                realm,
                runtime
                    .dup_jsvalue(&frame.cold.input.this_value)
                    .map_err(runtime_error_to_vm_error)?,
            )
            .map_err(runtime_error_to_vm_error)?
        {
            NativeConversion::Value(object) => {
                let id = object.object_id();
                runtime
                    .retain_object_handle(id)
                    .map_err(runtime_error_to_vm_error)?;
                JsValue::Object(id)
            }
            NativeConversion::Throw(value) => {
                let value = runtime
                    .into_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?;
                return Ok(CallStep::Complete(Completion::Throw(value)));
            }
        };
        frame.cold.normalized_this = Some(
            runtime
                .root_value(&value)
                .map_err(runtime_error_to_vm_error)?,
        );
        value
    };
    let prepared = if string {
        frame
            .executable
            .ensure_root(runtime)
            .map_err(runtime_error_to_vm_error)?;
        let descriptor = frame
            .executable
            .eval_environment(environment)
            .ok_or_else(|| Error::internal("eval environment index is out of bounds"))?;
        let (locals, parameters) = execution.slots.binding_counts(&frame.window)?;
        eval_bindings::validate(
            runtime,
            &frame.executable,
            &descriptor,
            frame.executable.metadata.strict,
            locals,
            parameters,
            &frame.cold.closure_slots,
        )?;
        Some(PreparedEvalEnvironment {
            index: environment,
            descriptor,
        })
    } else {
        None
    };
    let invocation = DirectEvalInvocation {
        input,
        environment,
        this_value,
        caller_strict: frame.executable.metadata.strict,
    };
    let prepared = runtime
        .prepare_direct_eval_original(realm, invocation, prepared, |prepared| {
            eval_bindings::materialize(prepared, &frame.cold.closure_slots, |source, descriptor| {
                let binding = match source {
                    EvalBindingSource::Local(index) => {
                        execution.slots.local_mut(&frame.window, index)?
                    }
                    EvalBindingSource::Argument(index) => {
                        execution.slots.parameter_mut(&frame.window, index)?
                    }
                    EvalBindingSource::Closure(_) => {
                        return Err(Error::internal("eval closure reached owned capture"));
                    }
                };
                super::bindings::capture_frame_binding(runtime, binding, descriptor)
            })
        })
        .map_err(runtime_error_to_vm_error)?;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let request = match prepared {
        DirectEvalPreparation::Complete(completion) => {
            frame.cold.eval_arguments = None;
            for _ in 0..=arguments {
                execution.slots.pop(&mut frame.window)?;
            }
            match completion {
                Completion::Return(value) => {
                    execution.slots.push(&mut frame.window, value)?;
                    None
                }
                completion => return Ok(CallStep::Complete(completion)),
            }
        }
        DirectEvalPreparation::Ready {
            callable,
            this_value,
        } => {
            let CallableExecution::Bytecode {
                bytecode,
                closure_slots,
            } = runtime
                .bytecode_for_callable(&callable)
                .map_err(runtime_error_to_vm_error)?
            else {
                return Err(Error::internal("prepared eval was not bytecode"));
            };
            if !can_push || runtime.bytecode_call_would_overflow() {
                return runtime
                    .bytecode_stack_overflow_completion(realm, &bytecode)
                    .map(CallStep::Complete)
                    .map_err(runtime_error_to_vm_error);
            }
            Some(BytecodeCallRequest {
                callable,
                receiver: this_value,
                new_target: Value::Undefined,
                arguments: Vec::new(),
                bytecode,
                closure_slots,
                caller_realm: realm,
                return_to: ReturnTarget {
                    value_use: ReturnValue::Push,
                    owner: crate::engine::vm::frame::ReturnOwner::Frame(id),
                    tail: false,
                    operation: Some(OperationTarget::Eval(arguments)),
                },
            })
        }
    };
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("eval resume PC overflow"))?;
    if let Some(request) = request {
        let entry = request.prepare(runtime, &mut execution.call_storage)?;
        push_frame(execution, entry)?;
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Entered)
}

#[inline(never)]
pub(super) fn apply(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    environment: u16,
) -> Result<CallStep, Error> {
    let can_push = execution.frames.can_push();
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let array = runtime
        .root_value(execution.slots.peek(&frame.window, 0)?)
        .map_err(runtime_error_to_vm_error)?;
    let Value::Object(array) = array else {
        return Ok(CallStep::Complete(Completion::Throw(
            runtime
                .new_native_error_jsvalue(
                    realm,
                    crate::engine::api::error::NativeErrorKind::Type,
                    "not a object",
                )
                .map_err(runtime_error_to_vm_error)?,
        )));
    };
    let Some(values) = runtime
        .prepare_fast_array_arguments(realm, &array)
        .map_err(runtime_error_to_vm_error)?
    else {
        return Ok(CallStep::Bridge);
    };
    // The fast-array snapshot stays public-rooted: the eval-arguments rare
    // cell is a public-root island, and the apply request re-enters the
    // internal convention at its boundary below.
    let mut values = match values {
        NativeConversion::Value(values) => values,
        NativeConversion::Throw(value) => return Ok(CallStep::Complete(Completion::Throw(value))),
    };
    let function = execution.slots.peek(&frame.window, 1)?;
    if runtime
        .is_original_eval_jsvalue(realm, function)
        .map_err(runtime_error_to_vm_error)?
    {
        if frame.cold.eval_arguments.is_some() {
            return Err(Error::internal("eval argument snapshot was already active"));
        }
        frame.cold.eval_arguments = Some(values);
        return step(runtime, execution, id, 1, environment);
    }
    let JsValue::Object(function) = function else {
        return Ok(CallStep::Bridge);
    };
    let Some(mut callable) = runtime
        .as_callable_object(*function)
        .map_err(runtime_error_to_vm_error)?
    else {
        return Ok(CallStep::Bridge);
    };
    let mut receiver = Value::Undefined;
    let (bytecode, closure_slots) = loop {
        match runtime
            .bytecode_for_callable(&callable)
            .map_err(runtime_error_to_vm_error)?
        {
            CallableExecution::Bytecode {
                bytecode,
                closure_slots,
            } => break (bytecode, closure_slots),
            CallableExecution::Bound {
                target,
                this_value,
                arguments,
            } => {
                values = match runtime
                    .concatenate_bound_arguments(realm, &arguments, &values)
                    .map_err(runtime_error_to_vm_error)?
                {
                    NativeConversion::Value(values) => values,
                    NativeConversion::Throw(value) => {
                        return Ok(CallStep::Complete(Completion::Throw(value)));
                    }
                };
                callable = target;
                receiver = this_value;
            }
            _ => return Ok(CallStep::Bridge),
        }
    };
    if runtime
        .0
        .state
        .borrow()
        .heap
        .function_bytecode(bytecode.bytecode_id())
        .map_err(|e| Error::internal(e.to_string()))?
        .metadata
        .function_kind
        != crate::engine::code::function::metadata::FunctionKind::Normal
    {
        return Ok(CallStep::Bridge);
    }
    if !can_push || runtime.bytecode_call_would_overflow() {
        return runtime
            .bytecode_stack_overflow_completion(realm, &bytecode)
            .map(CallStep::Complete)
            .map_err(runtime_error_to_vm_error);
    }
    let request = BytecodeCallRequest {
        callable,
        receiver: runtime
            .unroot_value(&receiver)
            .map_err(runtime_error_to_vm_error)?,
        new_target: JsValue::Undefined,
        arguments: values
            .iter()
            .map(|value| runtime.unroot_value(value))
            .collect::<Result<Vec<_>, _>>()
            .map_err(runtime_error_to_vm_error)?,
        bytecode,
        closure_slots,
        caller_realm: realm,
        return_to: ReturnTarget {
            value_use: ReturnValue::Push,
            owner: crate::engine::vm::frame::ReturnOwner::Frame(id),
            tail: false,
            operation: Some(OperationTarget::Eval(1)),
        },
    };
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    frame.resume_pc = frame
        .fault_pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("apply eval resume PC overflow"))?;
    let entry = request.prepare(runtime, &mut execution.call_storage)?;
    push_frame(execution, entry)?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(depth);
    Ok(CallStep::Entered)
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::engine::code::bytecode::Instruction;
    use crate::engine::code::function::metadata::*;
    use crate::engine::code::function::{
        UnlinkedConstant, UnlinkedFunction, UnlinkedVariableDefinition,
    };
    use crate::engine::value::JsString;
    use crate::engine::vm::{
        bindings::FrameBinding,
        execution::ExecutionLimits,
        frame::{ColdFrame, FrameCold, FrameEntry},
        stack::FrameStorage,
    };
    #[test]
    fn direct_eval_preparation_captures_exact_cells_only_after_successful_string_compile() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let environment = EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::Block,
                    bindings: vec![EvalBinding {
                        name: JsString::from_static("localBinding"),
                        source: EvalBindingSource::Local(0),
                        is_lexical: true,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                        is_catch_parameter: false,
                    }]
                    .into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: vec![
                        EvalBinding {
                            name: JsString::from_static("argumentBinding"),
                            source: EvalBindingSource::Argument(0),
                            is_lexical: false,
                            is_const: false,
                            kind: ClosureVariableKind::Normal,
                            is_catch_parameter: false,
                        },
                        EvalBinding {
                            name: JsString::from_static("<var>"),
                            source: EvalBindingSource::Local(1),
                            is_lexical: false,
                            is_const: false,
                            kind: ClosureVariableKind::EvalVariableObject,
                            is_catch_parameter: false,
                        },
                    ]
                    .into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: vec![EvalBinding {
                        name: JsString::from_static("outerBinding"),
                        source: EvalBindingSource::Closure(0),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                        is_catch_parameter: false,
                    }]
                    .into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::VariableObject {
                scope: 2,
                source: EvalBindingSource::Local(1),
            },
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        let child = UnlinkedFunction::fixture_with_closure_variables(
            vec![
                Instruction::VariableEnvironment,
                Instruction::PutLocal(1),
                Instruction::Undefined,
                Instruction::Eval {
                    argument_count: 0,
                    environment: 0,
                },
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("outerBinding")))
                    .unwrap(),
            ],
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                local_count: 2,
                eval_variable_object_local: Some(1),
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
        .with_fixture_definitions(
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("argumentBinding"),
            ))],
            vec![
                UnlinkedVariableDefinition::lexical(
                    Some(JsString::from_static("localBinding")),
                    false,
                ),
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<var>"))),
            ],
        )
        .with_eval_environments(vec![environment]);
        let parent = UnlinkedFunction::fixture(
            vec![Instruction::Undefined, Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_fixture_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("outerBinding"),
            ))],
        );
        let parent = runtime
            .publish_unlinked_function(context.realm, parent)
            .unwrap();
        let child = runtime.test_child_function_bytecode(&parent, 0).unwrap();
        let closure = runtime
            .new_var_ref(Value::Int(30), false, false, ClosureVariableKind::Normal)
            .unwrap();
        let eval_variable_object = runtime.new_object(None).unwrap();

        let callable = runtime
            .new_bytecode_closure_with_slots(context.realm, &child, std::slice::from_ref(&closure))
            .unwrap();
        for (input, environment, captured, throws) in [
            (Value::Int(42), u16::MAX, false, false),
            (Value::String(JsString::from_static(")")), 0, false, true),
            (
                Value::String(JsString::from_static("var evalVar = 1")),
                0,
                true,
                false,
            ),
            (
                Value::String(JsString::from_static("40 + 2")),
                0,
                true,
                false,
            ),
        ] {
            let prepared = runtime
                .prepare_bytecode_frame(
                    &callable,
                    Value::Int(1),
                    Value::Int(2),
                    &[Value::Int(10)],
                    child.clone(),
                )
                .unwrap();
            let entry = FrameEntry {
                property_generation: 0,
                iterator_generation: 0,
                caller_realm: context.realm,
                active_frame: prepared.active_frame.token(),
                initialize_bindings: false,
                executable: prepared.executable,
                cold: ColdFrame::new(FrameCold {
                    rare: std::cell::OnceCell::new(),
                    return_to: None,
                    entry_guard: Some(prepared.active_frame),
                    function: callable.as_object().clone().into(),
                    closure_slots: vec![closure.clone()].into(),
                    reusable_captured_locals: vec![false; 2],
                    input: prepared.input.into(),
                }),
                storage: FrameStorage {
                    original_arguments: vec![Value::Int(10)],
                    parameters: prepared.arguments,
                    locals: vec![
                        FrameBinding::Direct(Value::Int(20)),
                        FrameBinding::Direct(Value::Object(eval_variable_object.clone())),
                    ],
                    operands: vec![Value::Undefined],
                },
            };
            let mut execution =
                RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            let id = push_frame(&mut execution, entry).unwrap();
            execution
                .frames
                .current_mut(id)
                .unwrap()
                .cold
                .eval_arguments = Some(vec![input]);
            let outcome = prepare_and_enter(&runtime, &mut execution, id, 0, environment).unwrap();
            assert_eq!(
                matches!(outcome, CallStep::Complete(Completion::Throw(_))),
                throws
            );
            if captured {
                // Preparation entered the compiled child. Retire it without running
                // it so the caller's cell representation can be inspected in place.
                let child_id = execution.frames.current_id().unwrap();
                assert_ne!(child_id, id);
                super::super::frame_exit::finish(
                    &runtime,
                    &mut execution,
                    child_id,
                    super::super::run::RunExit::Complete,
                    Some(Completion::Return(Value::Undefined)),
                )
                .unwrap();
            }
            let frame = execution.frames.current_mut(id).unwrap();
            for index in 0..2 {
                assert_eq!(
                    matches!(
                        execution.slots.local(&frame.window, index).unwrap(),
                        FrameBinding::Captured(_)
                    ),
                    captured
                );
            }
            assert_eq!(
                matches!(
                    execution.slots.parameter(&frame.window, 0).unwrap(),
                    FrameBinding::Captured(_)
                ),
                captured
            );
            assert_eq!(runtime.read_var_ref(&closure).unwrap(), Value::Int(30));
            if !captured && !throws {
                assert_eq!(
                    execution.slots.peek(&frame.window, 0).unwrap(),
                    &Value::Int(42)
                );
            }
        }
    }
}
