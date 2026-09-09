use super::*;

#[test]
fn unified_active_frames_preserve_order_caller_pc_and_defining_realms() {
    let runtime = Runtime::new();
    let outer_context = runtime.new_context();
    let native_context = runtime.new_context();
    let callback_context = runtime.new_context();

    let function_prototype = native_context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            native_context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    let callback = bytecode_callable(
        &runtime,
        &callback_context,
        vec![
            Instruction::GetArg(0),
            Instruction::Call(0),
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: false,
            ..FunctionMetadata::default()
        },
    );
    let outer = bytecode_callable(
        &runtime,
        &outer_context,
        vec![
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::Call(1),
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 2,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let (outer_bytecode, callback_bytecode) = {
        let state = runtime.0.state.borrow();
        let ObjectPayload::BytecodeFunction { bytecode, .. } = &state
            .heap
            .object(outer.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("outer probe caller was not bytecode");
        };
        let outer_bytecode = *bytecode;
        let ObjectPayload::BytecodeFunction { bytecode, .. } = &state
            .heap
            .object(callback.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("probe callback was not bytecode");
        };
        (outer_bytecode, *bytecode)
    };

    assert_eq!(
        runtime
            .call_internal(
                outer_context.realm,
                &outer,
                Value::Undefined,
                &[
                    Value::Object(probe.as_object().clone()),
                    Value::Object(callback.as_object().clone()),
                ],
            )
            .unwrap(),
        Completion::Return(Value::Undefined)
    );

    let snapshot = runtime
        .0
        .state
        .borrow_mut()
        .active_frame_probe_snapshots
        .pop()
        .expect("deep native probe should capture the active chain");
    assert_eq!(snapshot.len(), 4);
    assert_eq!(snapshot[0].function, outer.as_object().object_id());
    assert_eq!(snapshot[0].realm, outer_context.realm);
    assert!(snapshot[0].flags.strict);
    assert!(matches!(
        snapshot[0].kind,
        ActiveFrameKind::Bytecode {
            bytecode,
            pc: Some(pc),
        } if bytecode == outer_bytecode && pc.index() == 2
    ));
    assert_eq!(snapshot[1].function, probe.as_object().object_id());
    assert_eq!(snapshot[1].realm, native_context.realm);
    assert!(matches!(
        snapshot[1].kind,
        ActiveFrameKind::Native {
            target: NativeFunctionId::ActiveFrameProbe,
            actual_arg_count: 1,
            readable_arg_count: 1,
        }
    ));
    assert_eq!(snapshot[2].function, callback.as_object().object_id());
    assert_eq!(snapshot[2].realm, callback_context.realm);
    assert!(!snapshot[2].flags.strict);
    assert!(matches!(
        snapshot[2].kind,
        ActiveFrameKind::Bytecode {
            bytecode,
            pc: Some(pc),
        } if bytecode == callback_bytecode && pc.index() == 1
    ));
    assert_eq!(snapshot[3].function, probe.as_object().object_id());
    assert_eq!(snapshot[3].realm, native_context.realm);
    assert!(matches!(
        snapshot[3].kind,
        ActiveFrameKind::Native {
            target: NativeFunctionId::ActiveFrameProbe,
            actual_arg_count: 0,
            readable_arg_count: 0,
        }
    ));
    assert!(
        snapshot
            .windows(2)
            .all(|frames| frames[0].token != frames[1].token)
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn unified_active_frames_restore_after_return_throw_and_engine_error() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    let no_argument_call = bytecode_callable(
        &runtime,
        &context,
        vec![
            Instruction::GetArg(0),
            Instruction::Call(0),
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let command_call = bytecode_callable(
        &runtime,
        &context,
        vec![
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::Call(1),
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 2,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );

    assert_eq!(
        context
            .call(
                &no_argument_call,
                Value::Undefined,
                &[Value::Object(probe.as_object().clone())],
            )
            .unwrap(),
        Value::Undefined
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert_eq!(
        context.call(
            &command_call,
            Value::Undefined,
            &[Value::Object(probe.as_object().clone()), Value::Bool(false),],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static(
            "active frame probe throw"
        )))
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert!(matches!(
        context.call(
            &command_call,
            Value::Undefined,
            &[
                Value::Object(probe.as_object().clone()),
                Value::Bool(true),
            ],
        ),
        Err(RuntimeError::Engine(error))
            if error.message().contains("active frame probe engine error")
    ));
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn active_frame_guard_roots_function_and_bytecode_through_gc_and_drop_fallback() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let bytecode = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::fixture(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let bytecode_id = bytecode.bytecode_id();
    let callable = runtime
        .new_bytecode_closure(context.realm, &bytecode)
        .unwrap();
    let function_id = callable.as_object().object_id();
    let guard = runtime
        .push_bytecode_active_frame(
            callable.as_object().clone(),
            bytecode.clone(),
            context.realm,
            true,
        )
        .unwrap();
    drop(callable);
    drop(bytecode);

    assert_eq!(runtime.run_gc().unwrap().cleanup.finalized_objects, 0);
    {
        let state = runtime.0.state.borrow();
        assert!(state.heap.object(function_id).is_ok());
        assert!(state.heap.function_bytecode(bytecode_id).is_ok());
        assert_eq!(state.active_frames.len(), 1);
    }

    drop(guard);
    assert!(runtime.0.state.borrow().active_frames.is_empty());
    assert!(runtime.0.state.borrow().heap.object(function_id).is_err());
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(bytecode_id)
            .is_err()
    );
}

#[test]
fn bytecode_active_frame_rejects_a_realm_other_than_the_bytecode_realm() {
    let runtime = Runtime::new();
    let defining_context = runtime.new_context();
    let other_context = runtime.new_context();
    let bytecode = runtime
        .publish_unlinked_function(
            defining_context.realm,
            UnlinkedFunction::fixture(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let callable = runtime
        .new_bytecode_closure(defining_context.realm, &bytecode)
        .unwrap();

    assert!(matches!(
        runtime.push_bytecode_active_frame(
            callable.as_object().clone(),
            bytecode.clone(),
            other_context.realm,
            true,
        ),
        Err(RuntimeError::Invariant(
            "bytecode active frame realm disagrees with its bytecode"
        ))
    ));
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn active_frame_drop_defers_nested_pops_until_the_state_borrow_ends() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let bytecode = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::fixture(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let bytecode_id = bytecode.bytecode_id();
    let callable = runtime
        .new_bytecode_closure(context.realm, &bytecode)
        .unwrap();
    let function_id = callable.as_object().object_id();
    let outer_guard = runtime
        .push_bytecode_active_frame(
            callable.as_object().clone(),
            bytecode.clone(),
            context.realm,
            true,
        )
        .unwrap();
    let outer_token = outer_guard.token();
    let inner_guard = runtime
        .push_bytecode_active_frame(
            callable.as_object().clone(),
            bytecode.clone(),
            context.realm,
            true,
        )
        .unwrap();
    let inner_token = inner_guard.token();
    drop(callable);
    drop(bytecode);

    let state_borrow = runtime.0.state.borrow();
    drop(inner_guard);
    drop(outer_guard);

    // The state borrow forces both guard drops through the deferred path.
    // `push_front` reverses unwind order so the outer pop removes the whole
    // nested suffix before either guard releases its raw heap roots.
    assert_eq!(state_borrow.active_frames.len(), 2);
    let deferred = runtime.0.deferred_references.borrow();
    let frame_pops = deferred
        .iter()
        .filter_map(|operation| match operation {
            DeferredRefOp::ActiveFramePop { token, .. } => Some(*token),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(frame_pops, vec![outer_token, inner_token]);
    assert!(matches!(
        deferred.front(),
        Some(DeferredRefOp::ActiveFramePop { token, .. }) if *token == outer_token
    ));
    drop(deferred);
    drop(state_borrow);

    runtime.drain_deferred_references().unwrap();
    assert!(runtime.0.deferred_references.borrow().is_empty());
    let state = runtime.0.state.borrow();
    assert!(state.active_frames.is_empty());
    assert!(state.heap.object(function_id).is_err());
    assert!(state.heap.function_bytecode(bytecode_id).is_err());
}
