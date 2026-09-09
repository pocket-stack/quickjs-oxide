use super::*;

#[test]
fn published_lexical_locals_use_named_tdz_errors_and_checked_mutation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let lexical = |code, is_const, max_stack| {
        UnlinkedFunction::fixture(
            code,
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack,
                ..FunctionMetadata::default()
            },
        )
        .with_fixture_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("namedLexical")),
                is_const,
            )],
        )
    };

    let tdz = runtime
        .publish_unlinked_function(
            context.realm,
            lexical(
                vec![Instruction::GetLocalCheck(0), Instruction::Return],
                false,
                1,
            ),
        )
        .unwrap();
    let tdz = runtime.new_bytecode_closure(context.realm, &tdz).unwrap();
    assert!(matches!(
        context.call(&tdz, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("lexical TDZ did not throw an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("namedLexical is not initialized"))
    );

    let mutable = runtime
        .publish_unlinked_function(
            context.realm,
            lexical(
                vec![
                    Instruction::SetLocalUninitialized(0),
                    Instruction::PushI32(40),
                    Instruction::InitializeLocal(0),
                    Instruction::PushI32(42),
                    Instruction::SetLocalCheck(0),
                    Instruction::Return,
                ],
                false,
                1,
            ),
        )
        .unwrap();
    let mutable = runtime
        .new_bytecode_closure(context.realm, &mutable)
        .unwrap();
    assert_eq!(
        context.call(&mutable, Value::Undefined, &[]).unwrap(),
        Value::Int(42)
    );

    let plain_put = runtime
        .publish_unlinked_function(
            context.realm,
            lexical(
                vec![
                    Instruction::PushI32(1),
                    Instruction::InitializeLocal(0),
                    Instruction::PushI32(2),
                    Instruction::InitializeLocal(0),
                    Instruction::GetLocalCheck(0),
                    Instruction::Return,
                ],
                false,
                1,
            ),
        )
        .unwrap();
    let plain_put = runtime
        .new_bytecode_closure(context.realm, &plain_put)
        .unwrap();
    assert_eq!(
        context.call(&plain_put, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn caught_throw_reuses_captured_lexical_without_close_local() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("exceptionReused")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::FClosure(0),
            Instruction::PutLocal(1),
            Instruction::Catch(9),
            Instruction::PushI32(0),
            Instruction::Throw,
            Instruction::Nop,
            Instruction::Drop,
            Instruction::SetLocalUninitialized(0),
            Instruction::PushI32(2),
            Instruction::InitializeLocal(0),
            Instruction::FClosure(0),
            Instruction::PutLocal(2),
            Instruction::GetLocal(1),
            Instruction::Call(0),
            Instruction::PushConst(1),
            Instruction::Add,
            Instruction::GetLocal(2),
            Instruction::Call(0),
            Instruction::Add,
            Instruction::PushConst(1),
            Instruction::Add,
            Instruction::GetLocal(1),
            Instruction::GetLocal(2),
            Instruction::StrictEq,
            Instruction::Add,
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::child(child),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("|"))).unwrap(),
        ],
        FunctionMetadata {
            local_count: 3,
            max_stack: 3,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![
            UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("exceptionReused")),
                false,
            ),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("first"))),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("second"))),
        ],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    assert_eq!(
        context.call(&parent, Value::Undefined, &[]).unwrap(),
        Value::String(JsString::from_static("2|2|false"))
    );
}

#[test]
fn finally_overridden_return_reuses_captured_lexical_without_close_local() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = concat!(
        "(function(){var f,g,i=0;while(i<2){i++;try{{let x=i;",
        "if(i===1)f=function(){return x};else g=function(){return x};",
        "return 9}}finally{continue}}return f()+'|'+g()})()"
    );

    assert_eq!(
        context.eval(source).unwrap(),
        Value::String(JsString::from_static("2|2"))
    );
}

#[test]
fn close_local_detaches_a_captured_uninitialized_lexical_lifetime() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("closedLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::FClosure(0),
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::CloseLocal(0),
            Instruction::PushI32(2),
            Instruction::InitializeLocal(0),
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::PushI32(3),
            Instruction::InitializeLocal(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::lexical(
            Some(JsString::from_static("closedLexical")),
            false,
        )],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    assert!(runtime.test_atom_count() > baseline_atoms);
    let parent_callable = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    drop(parent);
    let Value::Object(child_object) = context
        .call(&parent_callable, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("parent did not return its captured child closure");
    };
    drop(parent_callable);
    let child_callable = runtime.as_callable(&child_object).unwrap().unwrap();
    drop(child_object);
    assert_eq!(
        context
            .call(&child_callable, Value::Undefined, &[])
            .unwrap(),
        Value::Int(1)
    );
    drop(child_callable);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn lexical_scope_entry_rejects_an_initialized_capture_without_close_local() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::Undefined, Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("reenteredLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::Undefined,
            Instruction::Gosub(11),
            Instruction::Drop,
            Instruction::SetLocalUninitialized(0),
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Ret,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::lexical(
            Some(JsString::from_static("reenteredLexical")),
            false,
        )],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let RuntimeError::Engine(error) = context
        .call(&parent, Value::Undefined, &[])
        .expect_err("initialized captured lifetime was accepted without CloseLocal")
    else {
        panic!("initialized captured lifetime did not report an engine invariant");
    };
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert_eq!(
        error.message(),
        "captured local entered a new lexical lifetime before CloseLocal"
    );
}

#[test]
fn close_local_exposes_direct_and_fresh_captured_plain_put_values() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("observedLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::FClosure(0),
            Instruction::PutLocal(1),
            Instruction::PushI32(1),
            Instruction::InitializeLocal(0),
            Instruction::CloseLocal(0),
            Instruction::PushI32(2),
            Instruction::InitializeLocal(0),
            Instruction::GetLocalCheck(0),
            Instruction::PutLocal(3),
            Instruction::FClosure(0),
            Instruction::PutLocal(2),
            Instruction::PushI32(3),
            Instruction::InitializeLocal(0),
            Instruction::GetLocal(3),
            Instruction::PushI32(100),
            Instruction::Mul,
            Instruction::GetLocal(1),
            Instruction::Call(0),
            Instruction::PushI32(10),
            Instruction::Mul,
            Instruction::Add,
            Instruction::GetLocal(2),
            Instruction::Call(0),
            Instruction::Add,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 4,
            max_stack: 3,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![
            UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("observedLexical")),
                false,
            ),
            UnlinkedVariableDefinition::ordinary(None),
            UnlinkedVariableDefinition::ordinary(None),
            UnlinkedVariableDefinition::ordinary(None),
        ],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();

    // Hundreds: the Direct value after CloseLocal. Tens: the old cell.
    // Ones: the fresh captured cell after a second plain initialization.
    assert_eq!(
        context.call(&parent, Value::Undefined, &[]).unwrap(),
        Value::Int(213)
    );
}

#[test]
fn escaped_uninitialized_lexical_cell_keeps_its_named_tdz() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("escapedLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::fixture(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::lexical(
            Some(JsString::from_static("escapedLexical")),
            false,
        )],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let Value::Object(child) = context.call(&parent, Value::Undefined, &[]).unwrap() else {
        panic!("parent did not return its uninitialized lexical closure");
    };
    let child = runtime.as_callable(&child).unwrap().unwrap();
    assert!(matches!(
        context.call(&child, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("escaped lexical TDZ did not throw an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("escapedLexical is not initialized"))
    );
}

#[test]
fn named_ordinary_definitions_capture_through_unnamed_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
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
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::PushI32(7),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::Return,
        ],
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
            JsString::from_static("ordinaryCapture"),
        ))],
    );
    let parent = runtime
        .publish_unlinked_function(context.realm, parent)
        .unwrap();
    let parent = runtime
        .new_bytecode_closure(context.realm, &parent)
        .unwrap();
    let Value::Object(child) = context.call(&parent, Value::Undefined, &[]).unwrap() else {
        panic!("parent did not return its child closure");
    };
    let child = runtime.as_callable(&child).unwrap().unwrap();
    assert_eq!(
        context.call(&child, Value::Undefined, &[]).unwrap(),
        Value::Int(7)
    );
}

#[test]
fn put_var_init_initializes_a_const_global_lexical_once() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .create_global_lexical_for_test("initializedLexical", true, None)
        .unwrap();
    let function = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::PushI32(9),
            Instruction::PutVarInit(0),
            Instruction::GetVar(0),
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("initializedLexical")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::Global,
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: true,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();
    let function = runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Int(9)
    );
    assert!(matches!(
        context.eval("initializedLexical = 10"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("const lexical reassignment did not throw an object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&exception, &message).unwrap(),
        Value::String(JsString::from_static("'initializedLexical' is read-only"))
    );
}
