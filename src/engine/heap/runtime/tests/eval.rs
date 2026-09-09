use super::*;

#[test]
fn direct_eval_identity_is_realm_local_and_independent_of_the_global_property() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_eval = global_callable(&runtime, &mut first, "eval");
    let second_eval = global_callable(&runtime, &mut second, "eval");
    let first_value = Value::Object(first_eval.as_object().clone());
    let second_value = Value::Object(second_eval.as_object().clone());

    let mut first_host = RuntimeVmHost::empty_for_test(runtime.clone(), first.realm);
    assert!(VmHost::is_original_eval(&mut first_host, &first_value).unwrap());
    assert!(!VmHost::is_original_eval(&mut first_host, &second_value).unwrap());

    let mut second_host = RuntimeVmHost::empty_for_test(runtime.clone(), second.realm);
    assert!(VmHost::is_original_eval(&mut second_host, &second_value).unwrap());
    assert!(!VmHost::is_original_eval(&mut second_host, &first_value).unwrap());

    let foreign_runtime = Runtime::new();
    let mut foreign_context = foreign_runtime.new_context();
    let foreign_eval = global_callable(&foreign_runtime, &mut foreign_context, "eval");
    assert_eq!(
        runtime.is_original_eval(
            first.realm,
            &Value::Object(foreign_eval.as_object().clone())
        ),
        Err(RuntimeError::WrongRuntime("eval function"))
    );

    assert_eq!(
        second.eval("delete globalThis.eval").unwrap(),
        Value::Bool(true)
    );
    assert!(VmHost::is_original_eval(&mut second_host, &second_value).unwrap());
    second
        .eval("globalThis.eval = function replacement() { return 17; }")
        .unwrap();
    let replacement = global_callable(&runtime, &mut second, "eval");
    assert!(
        !VmHost::is_original_eval(
            &mut second_host,
            &Value::Object(replacement.as_object().clone())
        )
        .unwrap()
    );
    assert!(VmHost::is_original_eval(&mut second_host, &second_value).unwrap());
}

#[test]
fn string_direct_eval_materializes_exact_caller_cells_but_non_string_stays_lazy() {
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
            UnlinkedVariableDefinition::lexical(Some(JsString::from_static("localBinding")), false),
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

    let mut non_string = RuntimeVmHost::eval_frame_for_test(
        runtime.clone(),
        context.realm,
        &child,
        vec![closure.clone()],
        vec![Value::Int(10)],
        vec![Value::Int(20), Value::Object(eval_variable_object.clone())],
    )
    .unwrap();
    assert_eq!(
        VmHost::direct_eval(
            &mut non_string,
            DirectEvalInvocation {
                input: Value::Int(42),
                environment: u16::MAX,
                this_value: Value::Int(1),
                new_target: Value::Int(2),
                caller_strict: false,
            },
        )
        .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert!(!non_string.eval_binding_is_captured_for_test(EvalBindingSource::Local(0)));
    assert!(!non_string.eval_binding_is_captured_for_test(EvalBindingSource::Argument(0)));

    let mut syntax_error = RuntimeVmHost::eval_frame_for_test(
        runtime.clone(),
        context.realm,
        &child,
        vec![closure.clone()],
        vec![Value::Int(10)],
        vec![Value::Int(20), Value::Object(eval_variable_object.clone())],
    )
    .unwrap();
    assert!(matches!(
        VmHost::direct_eval(
            &mut syntax_error,
            DirectEvalInvocation {
                input: Value::String(JsString::from_static(")")),
                environment: 0,
                this_value: Value::Int(1),
                new_target: Value::Int(2),
                caller_strict: false,
            },
        )
        .unwrap(),
        Completion::Throw(_)
    ));
    assert!(!syntax_error.eval_binding_is_captured_for_test(EvalBindingSource::Local(0)));
    assert!(!syntax_error.eval_binding_is_captured_for_test(EvalBindingSource::Argument(0)));

    let mut declaration = RuntimeVmHost::eval_frame_for_test(
        runtime.clone(),
        context.realm,
        &child,
        vec![closure.clone()],
        vec![Value::Int(10)],
        vec![Value::Int(20), Value::Object(eval_variable_object.clone())],
    )
    .unwrap();
    assert_eq!(
        VmHost::direct_eval(
            &mut declaration,
            DirectEvalInvocation {
                input: Value::String(JsString::from_static("var evalVar = 1")),
                environment: 0,
                this_value: Value::Int(1),
                new_target: Value::Int(2),
                caller_strict: false,
            },
        )
        .unwrap(),
        Completion::Return(Value::Undefined),
    );
    assert!(declaration.eval_binding_is_captured_for_test(EvalBindingSource::Local(0)));
    assert!(declaration.eval_binding_is_captured_for_test(EvalBindingSource::Local(1)));
    assert!(declaration.eval_binding_is_captured_for_test(EvalBindingSource::Argument(0)));

    let mut string = RuntimeVmHost::eval_frame_for_test(
        runtime.clone(),
        context.realm,
        &child,
        vec![closure.clone()],
        vec![Value::Int(10)],
        vec![Value::Int(20), Value::Object(eval_variable_object.clone())],
    )
    .unwrap();
    assert_eq!(
        VmHost::direct_eval(
            &mut string,
            DirectEvalInvocation {
                input: Value::String(JsString::from_static("40 + 2")),
                environment: 0,
                this_value: Value::Int(1),
                new_target: Value::Int(2),
                caller_strict: false,
            },
        )
        .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert!(string.eval_binding_is_captured_for_test(EvalBindingSource::Local(0)));
    assert!(string.eval_binding_is_captured_for_test(EvalBindingSource::Local(1)));
    assert!(string.eval_binding_is_captured_for_test(EvalBindingSource::Argument(0)));
    assert!(string.eval_binding_is_captured_for_test(EvalBindingSource::Closure(0)));
    assert_eq!(runtime.read_var_ref(&closure).unwrap(), Value::Int(30));
}
