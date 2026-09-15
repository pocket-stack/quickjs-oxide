use super::*;

#[test]
fn class_initializer_children_require_unique_matching_bridge_consumption() {
    verify_unlinked_tree(&script_installing_instance_initializer(
        empty_class_initializer(ClassInitializerKind::InstanceFields),
    ))
    .unwrap();

    let static_elements = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::CallClassStaticBlock,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::StaticBlock,
        ))],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            super_allowed: true,
            arguments_forbidden: true,
            needs_home_object: true,
            class_initializer_kind: Some(ClassInitializerKind::StaticElements),
            ..FunctionMetadata::default()
        },
    );
    verify_unlinked_tree(&script_running_static_initializer(static_elements)).unwrap();

    let escaped = UnlinkedFunction::fixture(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::InstanceFields,
        ))],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let error = verify_unlinked_tree(&escaped).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class initializer child escaped its matching bridge"),
        "{error}"
    );

    let repeated = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::FClosure(0),
            Instruction::InstallClassInstanceInitializer,
            Instruction::Drop,
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::FClosure(0),
            Instruction::InstallClassInstanceInitializer,
            Instruction::Drop,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::InstanceFields,
        ))],
        FunctionMetadata {
            max_stack: 3,
            ..FunctionMetadata::default()
        },
    );
    let error = verify_unlinked_tree(&repeated).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class initializer child did not have one unique closure site"),
        "{error}"
    );
}

#[test]
fn class_initializer_bridges_reject_forged_roles_and_parents() {
    let wrong_role = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::FClosure(0),
            Instruction::RunClassStaticInitializer,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::InstanceFields,
        ))],
        FunctionMetadata {
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    );
    let error = verify_unlinked_tree(&wrong_role).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class initializer bridge consumed a child with the wrong role"),
        "{error}"
    );

    let missing_child = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::InstallClassInstanceInitializer,
            Instruction::Drop,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            max_stack: 3,
            ..FunctionMetadata::default()
        },
    );
    let error = verify_unlinked_tree(&missing_child).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class initializer bridge did not consume an adjacent child closure"),
        "{error}"
    );

    let injected_bridge_operand = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::PushTrue,
            Instruction::Dup,
            Instruction::IfFalse(7),
            Instruction::Drop,
            Instruction::FClosure(0),
            Instruction::InstallClassInstanceInitializer,
            Instruction::Drop,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::InstanceFields,
        ))],
        FunctionMetadata {
            max_stack: 4,
            ..FunctionMetadata::default()
        },
    );
    let error = verify_unlinked_tree(&injected_bridge_operand).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class initializer closure/bridge pair has a non-fallthrough entry"),
        "{error}"
    );

    let wrong_static_parent = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::CallClassStaticBlock,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::StaticBlock,
        ))],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let error = verify_unlinked_tree(&wrong_static_parent).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class static block call escaped its static-elements parent"),
        "{error}"
    );

    let root_initializer = empty_class_initializer(ClassInitializerKind::InstanceFields);
    let error = verify_unlinked_tree(&root_initializer).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class initializer bytecode escaped the class publication tree"),
        "{error}"
    );
}

#[test]
fn class_initializer_pairs_reject_direct_control_flow_entry() {
    let instance = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::Goto(3),
            Instruction::FClosure(0),
            Instruction::InstallClassInstanceInitializer,
            Instruction::Drop,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::InstanceFields,
        ))],
        FunctionMetadata {
            max_stack: 3,
            ..FunctionMetadata::default()
        },
    );
    let error = verify_unlinked_tree(&instance).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class initializer closure/bridge pair has a non-fallthrough entry"),
        "{error}"
    );

    let static_initializer = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Goto(2),
            Instruction::FClosure(0),
            Instruction::RunClassStaticInitializer,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::StaticElements,
        ))],
        FunctionMetadata {
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    );
    let error = verify_unlinked_tree(&static_initializer).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class initializer closure/bridge pair has a non-fallthrough entry"),
        "{error}"
    );

    let static_elements = UnlinkedFunction::fixture(
        vec![
            Instruction::Goto(1),
            Instruction::FClosure(0),
            Instruction::CallClassStaticBlock,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::StaticBlock,
        ))],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            super_allowed: true,
            arguments_forbidden: true,
            needs_home_object: true,
            class_initializer_kind: Some(ClassInitializerKind::StaticElements),
            ..FunctionMetadata::default()
        },
    );
    let error =
        verify_unlinked_tree(&script_running_static_initializer(static_elements)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class initializer closure/bridge pair has a non-fallthrough entry"),
        "{error}"
    );
}

#[test]
fn static_block_pair_rejects_a_crossing_backedge() {
    let static_elements = UnlinkedFunction::fixture(
        vec![
            Instruction::Nop,
            Instruction::FClosure(0),
            Instruction::CallClassStaticBlock,
            Instruction::PushFalse,
            Instruction::IfFalse(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(empty_class_initializer(
            ClassInitializerKind::StaticBlock,
        ))],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            super_allowed: true,
            arguments_forbidden: true,
            needs_home_object: true,
            class_initializer_kind: Some(ClassInitializerKind::StaticElements),
            ..FunctionMetadata::default()
        },
    );
    let error =
        verify_unlinked_tree(&script_running_static_initializer(static_elements)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("class static block closure/bridge pair is reentrant"),
        "{error}"
    );
}

#[test]
fn authored_class_initializers_inside_a_loop_remain_publishable() {
    let function = crate::engine::compiler::compile_unlinked_script(
        "while (again) { class C { field = 1; static value = 2; static {} } }",
    )
    .unwrap();
    assert!(
        function
            .code()
            .iter()
            .enumerate()
            .any(|(source_pc, instruction)| {
                explicit_control_flow_target(instruction)
                    .is_some_and(|target_pc| source_pc > target_pc)
            })
    );
    verify_unlinked_tree(&function).unwrap();
}

#[test]
fn derived_this_initializer_requires_authenticated_parent_lineage() {
    let no_capture_arrow = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            super_call_allowed: true,
            super_allowed: true,
            ..FunctionMetadata::default()
        },
    );
    verify_unlinked_tree(&script_with_child(derived_parent(no_capture_arrow))).unwrap();

    verify_unlinked_tree(&script_with_child(derived_parent(
        derived_this_initializer(
            ClosureSource::ParentLocal(0),
            ClosureSource::ParentLocal(1),
            ClosureSource::ParentLocal(2),
        ),
    )))
    .unwrap();

    let injected_pair = derived_this_initializer_with_code(
        ClosureSource::ParentLocal(0),
        ClosureSource::ParentLocal(1),
        ClosureSource::ParentLocal(2),
        vec![
            Instruction::PushTrue,
            Instruction::IfFalse(10),
            Instruction::GetVarRef(1),
            Instruction::GetSuper,
            Instruction::GetVarRef(2),
            Instruction::MarkSuperCall,
            Instruction::ConstructSuper(0),
            Instruction::Dup,
            Instruction::InitializeDerivedVarRef(0),
            Instruction::Return,
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::Goto(5),
        ],
        2,
    );
    assert!(
        verify_unlinked_tree(&script_with_child(derived_parent(injected_pair)))
            .unwrap_err()
            .to_string()
            .contains("super-call operand protocol has a non-fallthrough entry")
    );

    let forged = script_with_child(derived_parent(derived_this_initializer(
        ClosureSource::ParentLocal(3),
        ClosureSource::ParentLocal(1),
        ClosureSource::ParentLocal(2),
    )));
    assert!(
        verify_unlinked_tree(&forged)
            .unwrap_err()
            .to_string()
            .contains("did not originate from derived this")
    );

    let forged_active = script_with_child(derived_parent(derived_this_initializer(
        ClosureSource::ParentLocal(0),
        ClosureSource::ParentLocal(4),
        ClosureSource::ParentLocal(2),
    )));
    assert!(
        verify_unlinked_tree(&forged_active)
            .unwrap_err()
            .to_string()
            .contains("did not read the authenticated active function")
    );

    let forged_new_target = script_with_child(derived_parent(derived_this_initializer(
        ClosureSource::ParentLocal(0),
        ClosureSource::ParentLocal(1),
        ClosureSource::ParentLocal(5),
    )));
    assert!(
        verify_unlinked_tree(&forged_new_target)
            .unwrap_err()
            .to_string()
            .contains("did not read the authenticated new.target")
    );

    let relay = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::FClosure(3),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("<this>"))).unwrap(),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("<this_active_func>")))
                .unwrap(),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("<new.target>")))
                .unwrap(),
            UnlinkedConstant::child(derived_this_initializer(
                ClosureSource::ParentClosure(0),
                ClosureSource::ParentClosure(1),
                ClosureSource::ParentClosure(2),
            )),
        ],
        FunctionMetadata {
            closure_count: 3,
            max_stack: 1,
            strict: true,
            super_call_allowed: true,
            super_allowed: true,
            ..FunctionMetadata::default()
        },
        vec![
            ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: true,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
            ClosureVariable {
                source: ClosureSource::ParentLocal(1),
                name: ClosureVariableName::Constant(1),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
            ClosureVariable {
                source: ClosureSource::ParentLocal(2),
                name: ClosureVariableName::Constant(2),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
        ],
    );
    verify_unlinked_tree(&script_with_child(derived_parent(relay))).unwrap();

    let eval_root = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::GetVarRef(1),
            Instruction::GetSuper,
            Instruction::GetVarRef(2),
            Instruction::MarkSuperCall,
            Instruction::ConstructSuper(0),
            Instruction::Dup,
            Instruction::InitializeDerivedVarRef(0),
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("<this>"))).unwrap(),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("<this_active_func>")))
                .unwrap(),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("<new.target>")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 3,
            max_stack: 2,
            strict: true,
            super_call_allowed: true,
            super_allowed: true,
            eval_kind: EvalKind::Direct,
            ..FunctionMetadata::default()
        },
        vec![
            ClosureVariable {
                source: ClosureSource::EvalEnvironment(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: true,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
            ClosureVariable {
                source: ClosureSource::EvalEnvironment(1),
                name: ClosureVariableName::Constant(1),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
            ClosureVariable {
                source: ClosureSource::EvalEnvironment(2),
                name: ClosureVariableName::Constant(2),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            },
        ],
    );
    let derived_binding = EvalRootBinding {
        name: JsString::from_static("<this>"),
        scope: 0,
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let active_function_binding = EvalRootBinding {
        name: JsString::from_static("<this_active_func>"),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let new_target_binding = EvalRootBinding {
        name: JsString::from_static("<new.target>"),
        scope: 0,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let eval_bindings = [
        derived_binding.clone(),
        active_function_binding.clone(),
        new_target_binding.clone(),
    ];
    verify_unlinked_eval_tree(
        &eval_root,
        EvalKind::Direct,
        true,
        &eval_bindings,
        true,
        true,
    )
    .unwrap();

    let ordinary_binding = EvalRootBinding {
        is_lexical: false,
        ..derived_binding
    };
    let ordinary_bindings = [
        ordinary_binding,
        active_function_binding,
        new_target_binding,
    ];
    assert!(
        verify_unlinked_eval_tree(
            &eval_root,
            EvalKind::Direct,
            true,
            &ordinary_bindings,
            true,
            true,
        )
        .unwrap_err()
        .to_string()
        .contains("did not originate from derived this")
    );
}

#[test]
fn class_instance_initializer_relay_requires_active_function_provenance() {
    let relay = |active_read| {
        UnlinkedFunction::fixture_with_closure_variables(
            vec![
                Instruction::GetVarRef(1),
                Instruction::GetSuper,
                Instruction::GetVarRef(2),
                Instruction::MarkSuperCall,
                Instruction::ConstructSuper(0),
                Instruction::Dup,
                Instruction::InitializeDerivedVarRef(0),
                Instruction::GetVarRef(active_read),
                Instruction::CallClassInstanceInitializer,
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<this>")))
                    .unwrap(),
                UnlinkedConstant::primitive(Value::String(JsString::from_static(
                    "<this_active_func>",
                )))
                .unwrap(),
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<new.target>")))
                    .unwrap(),
            ],
            FunctionMetadata {
                closure_count: 4,
                max_stack: 2,
                strict: true,
                super_call_allowed: true,
                super_allowed: true,
                ..FunctionMetadata::default()
            },
            vec![
                ClosureVariable {
                    source: ClosureSource::ParentLocal(0),
                    name: ClosureVariableName::Constant(0),
                    is_lexical: true,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
                ClosureVariable {
                    source: ClosureSource::ParentLocal(1),
                    name: ClosureVariableName::Constant(1),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
                ClosureVariable {
                    source: ClosureSource::ParentLocal(2),
                    name: ClosureVariableName::Constant(2),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
                // This slot deliberately copies the sentinel spelling but
                // originates from an unauthenticated ordinary parent local.
                ClosureVariable {
                    source: ClosureSource::ParentLocal(4),
                    name: ClosureVariableName::Constant(1),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
            ],
        )
    };

    verify_unlinked_tree(&script_with_child(derived_parent(relay(1)))).unwrap();

    let error = verify_unlinked_tree(&script_with_child(derived_parent(relay(3)))).unwrap_err();
    assert!(
        error.to_string().contains(
            "class instance initializer relay did not read the authenticated active function"
        ),
        "{error}"
    );
}
