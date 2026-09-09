use super::*;

#[test]
fn function_closures_share_runtime_rooted_var_ref_cells() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline_var_refs = runtime.heap_counts().var_ref_nodes;
    let child = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::GetVarRef(0),
            Instruction::PushI32(1),
            Instruction::Add,
            Instruction::SetVarRef(0),
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let root = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::fixture(
                vec![Instruction::Undefined, Instruction::Return],
                vec![UnlinkedConstant::child(child)],
                FunctionMetadata {
                    local_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            )
            .with_fixture_definitions(Vec::new(), vec![UnlinkedVariableDefinition::ordinary(None)]),
        )
        .unwrap();
    let function = runtime.test_child_function_bytecode(&root, 0).unwrap();
    let cell = runtime
        .new_var_ref(Value::Int(1), false, false, ClosureVariableKind::Normal)
        .unwrap();
    let cell_id = cell.id();
    let first = runtime
        .new_bytecode_closure_with_slots(context.realm, &function, std::slice::from_ref(&cell))
        .unwrap();
    let second = runtime
        .new_bytecode_closure_with_slots(context.realm, &function, std::slice::from_ref(&cell))
        .unwrap();
    assert_eq!(
        runtime.0.state.borrow().heap.var_ref_strong_count(cell_id),
        Ok(3)
    );

    let mut caller = context.clone();
    assert_eq!(
        caller.call(&first, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        caller.call(&second, Value::Undefined, &[]).unwrap(),
        Value::Int(3)
    );

    runtime.write_var_ref(&cell, Value::Int(7)).unwrap();
    assert_eq!(runtime.read_var_ref(&cell).unwrap(), Value::Int(7));
    drop(cell);
    assert_eq!(
        runtime.0.state.borrow().heap.var_ref_strong_count(cell_id),
        Ok(2)
    );
    let promoted = VarRefRoot::from_borrowed_handle(runtime.clone(), cell_id).unwrap();
    assert_eq!(runtime.read_var_ref(&promoted).unwrap(), Value::Int(7));
    drop(first);
    drop(second);
    assert_eq!(
        runtime.0.state.borrow().heap.var_ref_strong_count(cell_id),
        Ok(1)
    );
    drop(promoted);
    assert_eq!(runtime.heap_counts().var_ref_nodes, baseline_var_refs);
}

#[test]
fn fclosure_captures_parent_local_and_isolates_each_invocation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_var_refs = runtime.heap_counts().var_ref_nodes;
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::PushI32(10),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(incrementing_closure(
            ClosureSource::ParentLocal(0),
        ))],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();

    let first = context
        .call(&parent, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    let second = context
        .call(&parent, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    assert_eq!(
        context.call(&first, Value::Undefined, &[]).unwrap(),
        Value::Int(11)
    );
    assert_eq!(
        context.call(&first, Value::Undefined, &[]).unwrap(),
        Value::Int(12)
    );
    assert_eq!(
        context.call(&second, Value::Undefined, &[]).unwrap(),
        Value::Int(11)
    );

    drop(first);
    drop(second);
    assert_eq!(runtime.heap_counts().var_ref_nodes, baseline_var_refs);
}

#[test]
fn parent_local_writes_after_fclosure_update_the_shared_cell() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: crate::engine::code::function::metadata::ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::PushI32(1),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::PushI32(7),
            Instruction::PutLocal(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let child = context
        .call(&parent, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();

    assert_eq!(
        context.call(&child, Value::Undefined, &[]).unwrap(),
        Value::Int(7)
    );
}

#[test]
fn repeated_fclosure_in_one_frame_reuses_the_parent_cell() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::PushI32(0),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::FClosure(0),
            Instruction::Call(0),
            Instruction::Drop,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(incrementing_closure(
            ClosureSource::ParentLocal(0),
        ))],
        FunctionMetadata {
            local_count: 1,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let survivor = context
        .call(&parent, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();

    assert_eq!(
        context.call(&survivor, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn parent_argument_and_transitive_parent_closure_capture_share_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let inner = incrementing_closure(ClosureSource::ParentClosure(0));
    let middle = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(inner)],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentArgument(0),
            name: crate::engine::code::function::metadata::ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let outer = UnlinkedFunction::fixture(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(middle)],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let outer = runtime
        .publish_unlinked_function(context.realm, outer)
        .unwrap();
    let outer = runtime.new_bytecode_closure(context.realm, &outer).unwrap();
    let middle = context
        .call(&outer, Value::Undefined, &[Value::Int(40)])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    let first_inner = context
        .call(&middle, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    let second_inner = context
        .call(&middle, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();

    assert_eq!(
        context.call(&first_inner, Value::Undefined, &[]).unwrap(),
        Value::Int(41)
    );
    assert_eq!(
        context.call(&second_inner, Value::Undefined, &[]).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context.call(&first_inner, Value::Undefined, &[]).unwrap(),
        Value::Int(43)
    );

    let isolated_middle = context
        .call(&outer, Value::Undefined, &[Value::Int(40)])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    let isolated_inner = context
        .call(&isolated_middle, Value::Undefined, &[])
        .and_then(|value| runtime.callable_from_value(value))
        .unwrap();
    assert_eq!(
        context
            .call(&isolated_inner, Value::Undefined, &[])
            .unwrap(),
        Value::Int(41)
    );
}
