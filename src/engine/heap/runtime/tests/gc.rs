use super::*;

#[test]
fn object_property_cycle_is_collected_only_by_explicit_gc() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let self_key = runtime.intern_property_key("self").unwrap();
    assert!(set_property(&runtime, &object, &self_key, Value::Object(object.clone())).unwrap());
    assert_eq!(runtime.heap_counts().object_nodes, 1);
    let state = runtime.0.state.borrow_mut();
    drop(object);
    drop(state);
    let stats = runtime.run_gc().unwrap();
    assert_eq!(stats.cleanup.finalized_objects, 1);
    assert_eq!(runtime.heap_counts().object_nodes, 0);
}

#[test]
fn named_function_self_capture_cycle_is_collected() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let closure = context
        .eval(
            "(function() {\
                var child;\
                var owner = function self() {\
                    child = function() { return self; };\
                    return child;\
                };\
                return owner();\
            })()",
        )
        .unwrap();
    let retained = runtime.heap_counts();
    assert!(retained.object_nodes >= baseline.object_nodes + 2);
    assert!(retained.var_ref_nodes >= baseline.var_ref_nodes + 2);
    drop(closure);
    assert!(runtime.heap_counts().object_nodes > baseline.object_nodes);

    let stats = runtime.run_gc().unwrap();
    assert!(stats.cleanup.finalized_objects >= 2);
    assert!(stats.cleanup.finalized_var_refs >= 2);
    let collected = runtime.heap_counts();
    assert_eq!(collected.object_nodes, baseline.object_nodes);
    assert_eq!(collected.var_ref_nodes, baseline.var_ref_nodes);
    assert_eq!(
        collected.function_bytecode_nodes,
        baseline.function_bytecode_nodes
    );
}

#[test]
fn exceptional_vm_exit_releases_local_frame_roots_immediately() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let function = DetachedBytecode::<Value> {
        code: vec![
            Instruction::PushConst(0),
            Instruction::PushConst(1),
            Instruction::PushI32(1),
            Instruction::Add,
            Instruction::Drop,
            Instruction::Return,
        ],
        constants: vec![
            Value::Object(object.clone()),
            Value::BigInt(JsBigInt::one()),
        ],
        local_count: 0,
        max_stack: 3,
    };
    let before = runtime
        .0
        .state
        .borrow()
        .heap
        .object_strong_count(object.object_id())
        .unwrap();
    assert!(Vm::new().execute(&function).is_err());
    let after = runtime
        .0
        .state
        .borrow()
        .heap
        .object_strong_count(object.object_id())
        .unwrap();
    assert_eq!(after, before);
}

#[test]
fn drops_during_runtime_borrow_are_deferred_to_the_next_safe_point() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let key = runtime.intern_property_key("queued").unwrap();
    let state = runtime.0.state.borrow_mut();
    drop(object);
    drop(key);
    assert_eq!(runtime.0.deferred_references.borrow().len(), 2);
    drop(state);

    let context = runtime.new_context();
    assert!(runtime.0.deferred_references.borrow().is_empty());
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, 0);
}
