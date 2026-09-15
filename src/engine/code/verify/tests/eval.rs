use super::*;

#[test]
fn eval_root_rejects_shadowed_super_pseudo_binding_origins() {
    let root = |derived_this: u16, active_function: u16, new_target: u16| {
        let names = [
            ("<this>", true),
            ("<this_active_func>", false),
            ("<new.target>", false),
            ("<this>", true),
            ("<this_active_func>", false),
            ("<new.target>", false),
        ];
        let constants = names
            .iter()
            .map(|(name, _)| {
                UnlinkedConstant::primitive(Value::String(JsString::from_static(name))).unwrap()
            })
            .collect::<Vec<_>>();
        let closure_variables = names
            .iter()
            .enumerate()
            .map(|(index, (_, is_lexical))| ClosureVariable {
                source: ClosureSource::EvalEnvironment(
                    u16::try_from(index).expect("test closure index fits u16"),
                ),
                name: ClosureVariableName::Constant(
                    u32::try_from(index).expect("test constant index fits u32"),
                ),
                is_lexical: *is_lexical,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            })
            .collect::<Vec<_>>();
        UnlinkedFunction::fixture_with_closure_variables(
            vec![
                Instruction::GetVarRef(active_function),
                Instruction::GetSuper,
                Instruction::GetVarRef(new_target),
                Instruction::MarkSuperCall,
                Instruction::ConstructSuper(0),
                Instruction::Dup,
                Instruction::InitializeDerivedVarRef(derived_this),
                Instruction::Return,
            ],
            constants,
            FunctionMetadata {
                closure_count: 6,
                max_stack: 2,
                strict: true,
                super_call_allowed: true,
                super_allowed: true,
                eval_kind: EvalKind::Direct,
                ..FunctionMetadata::default()
            },
            closure_variables,
        )
    };
    let expected_bindings = [
        eval_root_binding("<this>", 0, true, false, ClosureVariableKind::Normal),
        eval_root_binding(
            "<this_active_func>",
            0,
            false,
            false,
            ClosureVariableKind::Normal,
        ),
        eval_root_binding("<new.target>", 0, false, false, ClosureVariableKind::Normal),
        eval_root_binding("<this>", 1, true, false, ClosureVariableKind::Normal),
        eval_root_binding(
            "<this_active_func>",
            1,
            false,
            false,
            ClosureVariableKind::Normal,
        ),
        eval_root_binding("<new.target>", 1, false, false, ClosureVariableKind::Normal),
    ];

    verify_unlinked_eval_tree(
        &root(0, 1, 2),
        EvalKind::Direct,
        true,
        &expected_bindings,
        true,
        true,
    )
    .unwrap();

    for ((derived_this, active_function, new_target), expected_error) in [
        (
            (3, 1, 2),
            "captured derived initializer did not originate from derived this",
        ),
        (
            (0, 4, 2),
            "super-call marker did not read the authenticated active function",
        ),
        (
            (0, 1, 5),
            "super-call marker did not read the authenticated new.target",
        ),
    ] {
        let error = verify_unlinked_eval_tree(
            &root(derived_this, active_function, new_target),
            EvalKind::Direct,
            true,
            &expected_bindings,
            true,
            true,
        )
        .unwrap_err();
        assert!(error.to_string().contains(expected_error), "{error}");
    }
}

#[test]
fn eval_root_authenticates_ordered_caller_bindings_and_globals() {
    let expected = [
        eval_root_binding("outer", 3, false, false, ClosureVariableKind::Normal),
        eval_root_binding("inner", 0, true, true, ClosureVariableKind::Normal),
    ];
    let root = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![
            (
                ClosureSource::EvalEnvironment(0),
                "outer",
                false,
                false,
                ClosureVariableKind::Normal,
            ),
            (
                ClosureSource::EvalEnvironment(1),
                "inner",
                true,
                true,
                ClosureVariableKind::Normal,
            ),
            (
                ClosureSource::Global,
                "globalName",
                false,
                false,
                ClosureVariableKind::Normal,
            ),
        ],
    );

    verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &expected, false, false).unwrap();
}

#[test]
fn eval_root_authenticates_with_object_metadata_and_keeps_it_out_of_variable_targets() {
    let expected = [eval_root_binding(
        "<with>",
        0,
        false,
        false,
        ClosureVariableKind::WithObject,
    )];
    let root = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::EvalEnvironment(0),
            "<with>",
            false,
            false,
            ClosureVariableKind::WithObject,
        )],
    );
    let profile = EvalCallerProfile {
        scope_kinds: vec![EvalScopeKind::With].into_boxed_slice(),
        variable_target: EvalCallerVariableTarget::Global,
    };
    verify_unlinked_eval_tree_with_profile(
        &root,
        EvalKind::Direct,
        false,
        &expected,
        &profile,
        false,
        false,
    )
    .unwrap();

    let forged_target = EvalCallerProfile {
        scope_kinds: vec![EvalScopeKind::With].into_boxed_slice(),
        variable_target: EvalCallerVariableTarget::ExternalBinding(0),
    };
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &root,
            EvalKind::Direct,
            false,
            &expected,
            &forged_target,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("variable target is not authenticated")
    );

    let wrong_name = [eval_root_binding(
        "ordinary",
        0,
        false,
        false,
        ClosureVariableKind::WithObject,
    )];
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &root,
            EvalKind::Direct,
            false,
            &wrong_name,
            &profile,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("with-object binding has invalid")
    );
}

#[test]
fn eval_root_rejects_incoherent_caller_variable_profiles() {
    let strict = eval_root_with_descriptors_and_strict(EvalKind::Direct, true, Vec::new());
    let global_profile = EvalCallerProfile {
        scope_kinds: Box::new([]),
        variable_target: EvalCallerVariableTarget::Global,
    };
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &strict,
            EvalKind::Direct,
            true,
            &[],
            &global_profile,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("variable target is not authenticated")
    );

    let expected = [eval_root_binding(
        "<var>",
        0,
        false,
        false,
        ClosureVariableKind::EvalVariableObject,
    )];
    let imported = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::EvalEnvironment(0),
            "<var>",
            false,
            false,
            ClosureVariableKind::EvalVariableObject,
        )],
    );
    let forged_global = EvalCallerProfile {
        scope_kinds: vec![EvalScopeKind::FunctionRoot].into_boxed_slice(),
        variable_target: EvalCallerVariableTarget::Global,
    };
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &imported,
            EvalKind::Direct,
            false,
            &expected,
            &forged_global,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("variable target is not authenticated")
    );
}

#[test]
fn recursive_eval_root_authenticates_imported_suffix_origins_and_target() {
    let expected = [
        eval_root_binding(
            "<var>",
            1,
            false,
            false,
            ClosureVariableKind::EvalVariableObject,
        ),
        eval_root_binding(
            "<var>",
            1,
            false,
            false,
            ClosureVariableKind::EvalVariableObject,
        ),
    ];
    let profile = EvalCallerProfile {
        scope_kinds: vec![EvalScopeKind::FunctionBody, EvalScopeKind::FunctionRoot]
            .into_boxed_slice(),
        variable_target: EvalCallerVariableTarget::ExternalBinding(1),
    };
    verify_unlinked_eval_tree_with_profile(
        &recursive_eval_root(EvalScopeKind::FunctionBody, [0, 1], 1),
        EvalKind::Direct,
        false,
        &expected,
        &profile,
        false,
        false,
    )
    .unwrap();

    let wrong_kind = recursive_eval_root(EvalScopeKind::Catch, [0, 1], 1);
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &wrong_kind,
            EvalKind::Direct,
            false,
            &expected,
            &profile,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("imported scope kind")
    );

    let wrong_origin = recursive_eval_root(EvalScopeKind::FunctionBody, [1, 0], 1);
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &wrong_origin,
            EvalKind::Direct,
            false,
            &expected,
            &profile,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("wrong caller origin")
    );

    let wrong_target = recursive_eval_root(EvalScopeKind::FunctionBody, [0, 1], 0);
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &wrong_target,
            EvalKind::Direct,
            false,
            &expected,
            &profile,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("variable target disagrees")
    );
}

#[test]
fn script_and_eval_root_publication_are_not_interchangeable() {
    let eval_root = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
    assert!(
        verify_unlinked_tree(&eval_root)
            .unwrap_err()
            .to_string()
            .contains("publication entry point")
    );

    let script_root = eval_root_with_descriptors(EvalKind::None, Vec::new());
    assert!(
        verify_unlinked_eval_tree(&script_root, EvalKind::Direct, false, &[], false, false)
            .unwrap_err()
            .to_string()
            .contains("publication entry point")
    );
    assert!(
        verify_unlinked_eval_tree(&script_root, EvalKind::None, false, &[], false, false).is_err()
    );
}

#[test]
fn eval_root_rejects_caller_binding_spoofs() {
    let expected = [
        eval_root_binding("outer", 1, false, false, ClosureVariableKind::Normal),
        eval_root_binding("inner", 0, true, false, ClosureVariableKind::Normal),
    ];

    let wrong_name = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::EvalEnvironment(0),
            "spoofed",
            false,
            false,
            ClosureVariableKind::Normal,
        )],
    );
    assert!(
        verify_unlinked_eval_tree(
            &wrong_name,
            EvalKind::Direct,
            false,
            &expected[..1],
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("name disagrees")
    );

    let wrong_flags = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::EvalEnvironment(0),
            "outer",
            true,
            false,
            ClosureVariableKind::Normal,
        )],
    );
    assert!(
        verify_unlinked_eval_tree(
            &wrong_flags,
            EvalKind::Direct,
            false,
            &expected[..1],
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("flags disagree")
    );

    let wrong_kind = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::EvalEnvironment(0),
            "outer",
            false,
            false,
            ClosureVariableKind::FunctionName,
        )],
    );
    assert!(
        verify_unlinked_eval_tree(
            &wrong_kind,
            EvalKind::Direct,
            false,
            &expected[..1],
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("flags disagree")
    );

    let wrong_order = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![
            (
                ClosureSource::EvalEnvironment(1),
                "inner",
                true,
                false,
                ClosureVariableKind::Normal,
            ),
            (
                ClosureSource::EvalEnvironment(0),
                "outer",
                false,
                false,
                ClosureVariableKind::Normal,
            ),
        ],
    );
    assert!(
        verify_unlinked_eval_tree(
            &wrong_order,
            EvalKind::Direct,
            false,
            &expected,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("exact prefix")
    );

    let missing = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::EvalEnvironment(0),
            "outer",
            false,
            false,
            ClosureVariableKind::Normal,
        )],
    );
    assert!(
        verify_unlinked_eval_tree(&missing, EvalKind::Direct, false, &expected, false, false,)
            .unwrap_err()
            .to_string()
            .contains("descriptor count")
    );
}

#[test]
fn eval_root_allows_only_sloppy_global_variable_environment_declarations() {
    let direct_global = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::GlobalDeclaration,
            "declared",
            false,
            false,
            ClosureVariableKind::Normal,
        )],
    );
    verify_unlinked_eval_tree(&direct_global, EvalKind::Direct, false, &[], false, false).unwrap();

    let indirect_global = eval_root_with_descriptors(
        EvalKind::Indirect,
        vec![(
            ClosureSource::GlobalDeclaration,
            "declared",
            false,
            false,
            ClosureVariableKind::Normal,
        )],
    );
    verify_unlinked_eval_tree(
        &indirect_global,
        EvalKind::Indirect,
        false,
        &[],
        false,
        false,
    )
    .unwrap();

    for (kind, caller_strict) in [
        (EvalKind::Direct, false),
        (EvalKind::Direct, true),
        (EvalKind::Indirect, false),
    ] {
        let strict = eval_root_with_descriptors_and_strict(
            kind,
            true,
            vec![(
                ClosureSource::GlobalDeclaration,
                "declared",
                false,
                false,
                ClosureVariableKind::Normal,
            )],
        );
        assert!(
            verify_unlinked_eval_tree(&strict, kind, caller_strict, &[], false, false)
                .unwrap_err()
                .to_string()
                .contains("illegal global declaration")
        );
    }

    let lexical = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::GlobalDeclaration,
            "lexical",
            true,
            false,
            ClosureVariableKind::Normal,
        )],
    );
    assert!(
        verify_unlinked_eval_tree(&lexical, EvalKind::Direct, false, &[], false, false)
            .unwrap_err()
            .to_string()
            .contains("illegal global declaration")
    );
}

#[test]
fn eval_root_rejects_global_declarations_with_a_hidden_variable_object() {
    let expected = [eval_root_binding(
        "<var>",
        0,
        false,
        false,
        ClosureVariableKind::EvalVariableObject,
    )];
    let imported = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::HasEvalVariable {
                source: crate::engine::code::bytecode::EvalVariableSource::Closure(0),
                name: 0,
            },
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("<var>"))).unwrap()],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            eval_kind: EvalKind::Direct,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::EvalEnvironment(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::EvalVariableObject,
        }],
    );
    verify_unlinked_eval_tree(&imported, EvalKind::Direct, false, &expected, false, false).unwrap();

    let root = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![
            (
                ClosureSource::EvalEnvironment(0),
                "<var>",
                false,
                false,
                ClosureVariableKind::EvalVariableObject,
            ),
            (
                ClosureSource::GlobalDeclaration,
                "declared",
                false,
                false,
                ClosureVariableKind::Normal,
            ),
        ],
    );
    assert!(
        verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &expected, false, false,)
            .unwrap_err()
            .to_string()
            .contains("illegal global declaration")
    );
}

#[test]
fn eval_root_authenticates_the_parameter_variable_object_role_and_target() {
    let expected = [eval_root_binding(
        "<arg_var>",
        0,
        false,
        false,
        ClosureVariableKind::ArgEvalVariableObject,
    )];
    let imported = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::EvalEnvironment(0),
            "<arg_var>",
            false,
            false,
            ClosureVariableKind::ArgEvalVariableObject,
        )],
    );
    verify_unlinked_eval_tree(&imported, EvalKind::Direct, false, &expected, false, false).unwrap();

    let wrong_role = EvalCallerProfile {
        scope_kinds: vec![EvalScopeKind::FunctionRoot].into_boxed_slice(),
        variable_target: EvalCallerVariableTarget::ExternalBinding(0),
    };
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &imported,
            EvalKind::Direct,
            false,
            &expected,
            &wrong_role,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("variable target is not authenticated")
    );

    let wrong_sentinel = [eval_root_binding(
        "<var>",
        0,
        false,
        false,
        ClosureVariableKind::ArgEvalVariableObject,
    )];
    assert!(
        verify_unlinked_eval_tree(
            &imported,
            EvalKind::Direct,
            false,
            &wrong_sentinel,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("invalid binding metadata")
    );
}

#[test]
fn eval_root_requires_external_bindings_to_be_an_exact_prefix() {
    let expected = [eval_root_binding(
        "caller",
        0,
        false,
        false,
        ClosureVariableKind::Normal,
    )];
    let root = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![
            (
                ClosureSource::Global,
                "global",
                false,
                false,
                ClosureVariableKind::Normal,
            ),
            (
                ClosureSource::EvalEnvironment(0),
                "caller",
                false,
                false,
                ClosureVariableKind::Normal,
            ),
        ],
    );
    assert!(
        verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &expected, false, false,)
            .unwrap_err()
            .to_string()
            .contains("exact prefix")
    );
}

#[test]
fn eval_root_rejects_forged_special_and_catch_binding_metadata() {
    let global_special = eval_root_with_descriptors(
        EvalKind::Direct,
        vec![(
            ClosureSource::GlobalDeclaration,
            "<var>",
            false,
            false,
            ClosureVariableKind::EvalVariableObject,
        )],
    );
    assert!(
        verify_unlinked_eval_tree(&global_special, EvalKind::Direct, false, &[], false, false,)
            .unwrap_err()
            .to_string()
            .contains("non-global binding metadata")
    );

    let root = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
    let mut forged_catch =
        eval_root_binding("caught", 0, false, false, ClosureVariableKind::Normal);
    forged_catch.is_catch_parameter = true;
    assert!(
        verify_unlinked_eval_tree(
            &root,
            EvalKind::Direct,
            false,
            &[forged_catch],
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("catch binding")
    );
}

#[test]
fn eval_root_rejects_child_eval_sources() {
    let child = eval_root_with_descriptors(
        EvalKind::None,
        vec![(
            ClosureSource::EvalEnvironment(0),
            "escaped",
            false,
            false,
            ClosureVariableKind::Normal,
        )],
    );
    let root = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 1,
            eval_kind: EvalKind::Direct,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &[], false, false)
            .unwrap_err()
            .to_string()
            .contains("child bytecode directly referenced")
    );
}

#[test]
fn eval_variable_object_local_and_dynamic_sources_are_authenticated() {
    let special_source = crate::engine::code::bytecode::EvalVariableSource::Local(0);
    let valid = UnlinkedFunction::fixture(
        vec![
            Instruction::VariableEnvironment,
            Instruction::PutLocal(0),
            Instruction::HasEvalVariable {
                source: special_source,
                name: 0,
            },
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("created"))).unwrap()],
        FunctionMetadata {
            local_count: 1,
            eval_variable_object_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    verify_unlinked_tree(&script_with_child(valid)).unwrap();

    let no_prologue = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            local_count: 1,
            eval_variable_object_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(no_prologue))
            .unwrap_err()
            .to_string()
            .contains("exact entry prologue")
    );

    let exposed = UnlinkedFunction::fixture(
        vec![
            Instruction::VariableEnvironment,
            Instruction::PutLocal(0),
            Instruction::GetLocal(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            local_count: 1,
            eval_variable_object_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(exposed))
            .unwrap_err()
            .to_string()
            .contains("private eval variable object")
    );

    let forged_source = UnlinkedFunction::fixture(
        vec![
            Instruction::HasEvalVariable {
                source: special_source,
                name: 0,
            },
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("created"))).unwrap()],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(forged_source))
            .unwrap_err()
            .to_string()
            .contains("authenticated local")
    );
}

#[test]
fn dynamic_sources_and_reference_names_are_authenticated() {
    let forged_with = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Return,
            Instruction::DynamicEnvironmentObject(DynamicEnvironmentSource::With(
                WithObjectSource::Local(0),
            )),
        ],
        Vec::new(),
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(forged_with))
            .unwrap_err()
            .to_string()
            .contains("authenticated local")
    );

    let out_of_bounds = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Return,
            Instruction::DynamicEnvironmentObject(DynamicEnvironmentSource::With(
                WithObjectSource::Closure(0),
            )),
        ],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(out_of_bounds))
            .unwrap_err()
            .to_string()
            .contains("source closure is out of bounds")
    );

    for instruction in [
        Instruction::GetRefValue(0),
        Instruction::GetRefValueUndef(0),
        Instruction::PutRefValue(0),
    ] {
        let non_string_name = UnlinkedFunction::fixture(
            vec![Instruction::Undefined, Instruction::Return, instruction],
            vec![UnlinkedConstant::primitive(Value::Int(0)).unwrap()],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(non_string_name))
                .unwrap_err()
                .to_string()
                .contains("reference opcode referenced a non-string name constant")
        );
    }
}

#[test]
fn eval_environments_cannot_cross_a_binding_pattern_body_boundary() {
    let initializer_name = JsString::from_static("initializer");
    let initializer_binding = EvalBinding {
        name: initializer_name.clone(),
        source: EvalBindingSource::Local(1),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let body_eval = UnlinkedFunction::fixture(
        vec![
            Instruction::GetArg(0),
            Instruction::Drop,
            Instruction::SetLocalUninitialized(1),
            Instruction::Undefined,
            Instruction::InitializeLocal(1),
            Instruction::CloseLocal(1),
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Eval {
                argument_count: 0,
                environment: 0,
            },
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            pattern_argument_count: 1,
            parameter_pattern_end: Some(6),
            local_count: 2,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        vec![UnlinkedVariableDefinition::ordinary(None)],
        vec![
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
            UnlinkedVariableDefinition::lexical(Some(initializer_name), false)
                .with_parameter_initializer(true),
        ],
    )
    .with_eval_environments(vec![ordinary_environment(Some(initializer_binding))]);
    let error = verify_unlinked_tree(&script_with_child(body_eval)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("function body eval captured a parameter-initializer local"),
        "{error}"
    );

    let body_name = JsString::from_static("body");
    let body_binding = EvalBinding {
        name: body_name.clone(),
        source: EvalBindingSource::Local(1),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let initializer_apply_eval = UnlinkedFunction::fixture(
        vec![
            Instruction::GetArg(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::ApplyEval { environment: 0 },
            Instruction::Drop,
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            pattern_argument_count: 1,
            parameter_pattern_end: Some(6),
            local_count: 2,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        vec![UnlinkedVariableDefinition::ordinary(None)],
        vec![
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
            UnlinkedVariableDefinition::lexical(Some(body_name), false),
        ],
    )
    .with_eval_environments(vec![ordinary_environment(Some(body_binding))]);
    let error = verify_unlinked_tree(&script_with_child(initializer_apply_eval)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("pattern initializer eval captured a body lexical local"),
        "{error}"
    );
}

#[test]
fn eval_variable_object_can_relay_only_through_a_special_closure() {
    let name = JsString::from_static("<var>");
    let child = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::HasEvalVariable {
                source: crate::engine::code::bytecode::EvalVariableSource::Closure(0),
                name: 0,
            },
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::EvalVariableObject,
        }],
    );
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::VariableEnvironment,
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            eval_variable_object_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    verify_unlinked_tree(&script_with_child(parent)).unwrap();
}

#[test]
fn eval_kind_is_root_only_and_indirect_eval_has_no_caller_bindings() {
    let child = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
    let root = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 1,
            eval_kind: EvalKind::Direct,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &[], false, false)
            .unwrap_err()
            .to_string()
            .contains("non-root bytecode")
    );

    let indirect = eval_root_with_descriptors(
        EvalKind::Indirect,
        vec![(
            ClosureSource::Global,
            "globalName",
            false,
            false,
            ClosureVariableKind::Normal,
        )],
    );
    verify_unlinked_eval_tree(&indirect, EvalKind::Indirect, false, &[], false, false).unwrap();
    let caller_binding = eval_root_binding("caller", 0, false, false, ClosureVariableKind::Normal);
    assert!(
        verify_unlinked_eval_tree(
            &indirect,
            EvalKind::Indirect,
            false,
            &[caller_binding],
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("received caller bindings")
    );
    assert!(
        verify_unlinked_eval_tree(&indirect, EvalKind::Indirect, true, &[], false, false)
            .unwrap_err()
            .to_string()
            .contains("received caller strictness")
    );

    let sloppy_direct = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
    assert!(
        verify_unlinked_eval_tree(&sloppy_direct, EvalKind::Direct, true, &[], false, false)
            .unwrap_err()
            .to_string()
            .contains("lost inherited caller strictness")
    );
}

#[test]
fn eval_super_capabilities_are_authenticated_at_publication() {
    let malformed = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            super_call_allowed: true,
            super_allowed: false,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&malformed)
            .unwrap_err()
            .to_string()
            .contains("without SuperProperty")
    );

    let mut environment = ordinary_environment(None);
    environment.super_allowed = true;
    let caller = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![environment]);
    assert!(
        verify_unlinked_tree(&script_with_child(caller))
            .unwrap_err()
            .to_string()
            .contains("super capability disagrees with bytecode metadata")
    );

    let profile = EvalCallerProfile {
        scope_kinds: Box::new([]),
        variable_target: EvalCallerVariableTarget::Global,
    };
    let capability_root = |kind| {
        UnlinkedFunction::fixture(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                eval_kind: kind,
                super_allowed: true,
                ..FunctionMetadata::default()
            },
        )
    };

    let direct = capability_root(EvalKind::Direct);
    verify_unlinked_eval_tree_with_profile(
        &direct,
        EvalKind::Direct,
        false,
        &[],
        &profile,
        false,
        true,
    )
    .unwrap();
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &direct,
            EvalKind::Direct,
            false,
            &[],
            &profile,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("eval root super capability disagrees with its caller")
    );

    let indirect = capability_root(EvalKind::Indirect);
    assert!(
        verify_unlinked_eval_tree_with_profile(
            &indirect,
            EvalKind::Indirect,
            false,
            &[],
            &profile,
            false,
            false,
        )
        .unwrap_err()
        .to_string()
        .contains("eval root super capability disagrees with its caller")
    );
}

#[test]
fn eval_local_sources_count_as_captures_for_close_local() {
    let environment = ordinary_environment(Some(EvalBinding {
        name: JsString::from_static("binding"),
        source: EvalBindingSource::Local(0),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    }));
    let function = lexical_local_function(environment, eval_code(0, true));

    verify_unlinked_tree(&script_with_child(function)).unwrap();
}

#[test]
fn eval_instruction_rejects_an_out_of_bounds_environment() {
    let function = UnlinkedFunction::fixture(
        eval_code(1, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![ordinary_environment(None)]);

    let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("environment operand is out of bounds")
    );

    let spread = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Undefined,
            Instruction::ApplyEval { environment: 1 },
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![ordinary_environment(None)]);
    let error = verify_unlinked_tree(&script_with_child(spread)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("environment operand is out of bounds")
    );
}

#[test]
fn unreferenced_eval_environment_cannot_manufacture_a_local_capture() {
    let environment = ordinary_environment(Some(EvalBinding {
        name: JsString::from_static("binding"),
        source: EvalBindingSource::Local(0),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    }));
    let function = lexical_local_function(
        environment,
        vec![
            Instruction::CloseLocal(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
    );

    let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
    assert!(error.to_string().contains("not referenced by bytecode"));
}

#[test]
fn eval_environment_rejects_malformed_binding_metadata() {
    let cases = [
        EvalBinding {
            name: JsString::from_static("binding"),
            source: EvalBindingSource::Local(1),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        },
        EvalBinding {
            name: JsString::from_static("wrong"),
            source: EvalBindingSource::Local(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        },
        EvalBinding {
            name: JsString::from_static("binding"),
            source: EvalBindingSource::Local(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        },
    ];

    for binding in cases {
        let function =
            lexical_local_function(ordinary_environment(Some(binding)), eval_code(0, false));
        assert!(verify_unlinked_tree(&script_with_child(function)).is_err());
    }
}

#[test]
fn eval_environment_authenticates_local_with_object_metadata() {
    let binding = EvalBinding {
        name: JsString::from_static("<with>"),
        source: EvalBindingSource::Local(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::WithObject,
        is_catch_parameter: false,
    };
    verify_unlinked_tree(&script_with_child(local_with_environment(
        binding.clone(),
        "<with>",
    )))
    .unwrap();

    assert!(
        verify_unlinked_tree(&script_with_child(local_with_environment(
            binding.clone(),
            "ordinary",
        )))
        .unwrap_err()
        .to_string()
        .contains("with-object local")
    );

    let mut lexical = binding.clone();
    lexical.is_lexical = true;
    assert!(
        verify_unlinked_tree(&script_with_child(local_with_environment(
            lexical, "<with>",
        )))
        .unwrap_err()
        .to_string()
        .contains("with-object binding metadata")
    );

    let mut argument = binding;
    argument.source = EvalBindingSource::Argument(0);
    assert!(
        verify_unlinked_tree(&script_with_child(local_with_environment(
            argument, "<with>",
        )))
        .unwrap_err()
        .to_string()
        .contains("with-object binding metadata")
    );
}

#[test]
fn eval_environment_authenticates_simple_and_pattern_catch_provenance() {
    let catch_binding = EvalBinding {
        name: JsString::from_static("binding"),
        source: EvalBindingSource::Local(0),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: true,
    };
    let environment = EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::Catch,
                bindings: vec![catch_binding.clone()].into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::StrictLocal(2),
        caller_strict: true,
        super_call_allowed: false,
        super_allowed: false,
    };
    verify_unlinked_tree(&script_with_child(lexical_local_function(
        environment.clone(),
        eval_code(0, false),
    )))
    .unwrap();

    let mut pattern_environment = environment.clone();
    pattern_environment.scopes[0].bindings = vec![EvalBinding {
        is_catch_parameter: false,
        ..catch_binding.clone()
    }]
    .into_boxed_slice();
    verify_unlinked_tree(&script_with_child(lexical_local_function(
        pattern_environment,
        eval_code(0, false),
    )))
    .unwrap();

    let mut forged = environment;
    forged.scopes[0].bindings = Box::new([]);
    forged.scopes[1].bindings = vec![catch_binding].into_boxed_slice();
    assert!(
        verify_unlinked_tree(&script_with_child(lexical_local_function(
            forged,
            eval_code(0, false),
        )))
        .unwrap_err()
        .to_string()
        .contains("catch binding metadata")
    );
}

#[test]
fn eval_environment_rejects_sources_from_the_wrong_function_segment() {
    let binding = EvalBinding {
        name: JsString::from_static("binding"),
        source: EvalBindingSource::Local(0),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };
    let mut ancestor_local = ordinary_environment(None);
    ancestor_local.scopes[2].bindings = vec![binding.clone()].into_boxed_slice();
    let function = lexical_local_function(ancestor_local, eval_code(0, false));
    assert!(
        verify_unlinked_tree(&script_with_child(function))
            .unwrap_err()
            .to_string()
            .contains("function scope segment")
    );

    let mut current_closure = ordinary_environment(None);
    current_closure.scopes[0].bindings = vec![EvalBinding {
        source: EvalBindingSource::Closure(0),
        ..binding
    }]
    .into_boxed_slice();
    let function = lexical_local_function(current_closure, eval_code(0, false));
    assert!(
        verify_unlinked_tree(&script_with_child(function))
            .unwrap_err()
            .to_string()
            .contains("function scope segment")
    );
}

#[test]
fn eval_variable_object_target_cannot_forge_an_ancestor_function_anchor() {
    let variable_name = JsString::from_static("<var>");
    let current_binding = EvalBinding {
        name: variable_name.clone(),
        source: EvalBindingSource::Local(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::EvalVariableObject,
        is_catch_parameter: false,
    };
    let ancestor_binding = EvalBinding {
        source: EvalBindingSource::Closure(0),
        ..current_binding.clone()
    };
    let environment = EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: vec![current_binding].into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: vec![ancestor_binding].into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::VariableObject {
            scope: 3,
            source: EvalBindingSource::Closure(0),
        },
        caller_strict: false,
        super_call_allowed: false,
        super_allowed: false,
    };
    let mut inner_code = vec![Instruction::VariableEnvironment, Instruction::PutLocal(0)];
    inner_code.extend(eval_code(0, false));
    let inner = UnlinkedFunction::fixture_with_closure_variables(
        inner_code,
        vec![UnlinkedConstant::primitive(Value::String(variable_name.clone())).unwrap()],
        FunctionMetadata {
            local_count: 1,
            closure_count: 1,
            eval_variable_object_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::EvalVariableObject,
        }],
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition {
            name: Some(variable_name.clone()),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::EvalVariableObject,
        }],
    )
    .with_eval_environments(vec![environment]);
    let outer = UnlinkedFunction::fixture(
        vec![
            Instruction::VariableEnvironment,
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(inner)],
        FunctionMetadata {
            local_count: 1,
            eval_variable_object_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition {
            name: Some(variable_name),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::EvalVariableObject,
        }],
    );

    let error = verify_unlinked_tree(&script_with_child(outer)).unwrap_err();
    assert!(
        error.to_string().contains("wrong current function segment"),
        "{error}"
    );
}

#[test]
fn eval_variable_object_target_cannot_forge_an_ancestor_parameter_anchor() {
    let body_name = JsString::from_static("<var>");
    let parameter_name = JsString::from_static("<arg_var>");
    let environment = EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: vec![EvalBinding {
                    name: body_name.clone(),
                    source: EvalBindingSource::Local(0),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::EvalVariableObject,
                    is_catch_parameter: false,
                }]
                .into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::Parameter,
                bindings: vec![EvalBinding {
                    name: parameter_name.clone(),
                    source: EvalBindingSource::Closure(0),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::ArgEvalVariableObject,
                    is_catch_parameter: false,
                }]
                .into_boxed_slice(),
            },
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::VariableObject {
            scope: 2,
            source: EvalBindingSource::Closure(0),
        },
        caller_strict: false,
        super_call_allowed: false,
        super_allowed: false,
    };
    let mut inner_code = vec![Instruction::VariableEnvironment, Instruction::PutLocal(0)];
    inner_code.extend(eval_code(0, false));
    let inner = UnlinkedFunction::fixture_with_closure_variables(
        inner_code,
        vec![UnlinkedConstant::primitive(Value::String(parameter_name.clone())).unwrap()],
        FunctionMetadata {
            local_count: 1,
            closure_count: 1,
            eval_variable_object_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(1),
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::ArgEvalVariableObject,
        }],
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition {
            name: Some(body_name.clone()),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::EvalVariableObject,
        }],
    )
    .with_eval_environments(vec![environment]);
    let outer = UnlinkedFunction::fixture(
        vec![
            Instruction::VariableEnvironment,
            Instruction::PutLocal(0),
            Instruction::VariableEnvironment,
            Instruction::PutLocal(1),
            Instruction::Nop,
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(inner)],
        FunctionMetadata {
            local_count: 2,
            eval_variable_object_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_parameter_environment(Some(ParameterEnvironmentLayout {
        initialization_end: 4,
        argument_cells: Box::new([]),
        pattern_copies: Box::new([]),
        default_sources: Box::new([]),
        synthetic_arguments_local: None,
        arg_eval_variable_object_local: Some(1),
    }))
    .with_fixture_definitions(
        Vec::new(),
        vec![
            UnlinkedVariableDefinition {
                name: Some(body_name),
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::EvalVariableObject,
            },
            UnlinkedVariableDefinition {
                name: Some(parameter_name),
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::ArgEvalVariableObject,
            },
        ],
    );

    let error = verify_unlinked_tree(&script_with_child(outer)).unwrap_err();
    assert!(
        error.to_string().contains("wrong current function segment"),
        "{error}"
    );
}

#[test]
fn strict_script_global_eval_anchor_does_not_leak_to_strict_functions() {
    let strict_script = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::Global,
        caller_strict: true,
        super_call_allowed: false,
        super_allowed: false,
    }]);
    verify_unlinked_tree(&strict_script).unwrap();

    let mut non_canonical_script_environment = strict_script.eval_environments()[0].clone();
    non_canonical_script_environment.variable_environment = EvalVariableEnvironment::StrictLocal(1);
    let non_canonical_script = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![non_canonical_script_environment.clone()]);
    let error = verify_unlinked_tree(&non_canonical_script).unwrap_err();
    assert!(
        error.to_string().contains("non-canonical strict-local"),
        "{error}"
    );

    let synthetic_strict_eval = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            eval_kind: EvalKind::Direct,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![non_canonical_script_environment]);
    verify_unlinked_eval_tree(
        &synthetic_strict_eval,
        EvalKind::Direct,
        false,
        &[],
        false,
        false,
    )
    .unwrap();

    let mut forged_function = ordinary_environment(None);
    forged_function.variable_environment = EvalVariableEnvironment::Global;
    let function = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![forged_function]);
    let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("escaped an authored Script root"),
        "{error}"
    );
}

#[test]
fn eval_environment_rejects_malformed_function_segments() {
    let mut environment = ordinary_environment(None);
    environment.scopes[2].kind = EvalScopeKind::FunctionBody;
    let function = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![environment]);
    assert!(
        verify_unlinked_tree(&script_with_child(function))
            .unwrap_err()
            .to_string()
            .contains("wrong body scope")
    );

    let mut missing_body = ordinary_environment(None);
    missing_body.scopes[0].kind = EvalScopeKind::FunctionRoot;
    let function = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![missing_body]);
    assert!(
        verify_unlinked_tree(&script_with_child(function))
            .unwrap_err()
            .to_string()
            .contains("wrong body scope")
    );
}

#[test]
fn eval_environment_rejects_global_and_nameless_lexical_closure_sources() {
    let global_name = JsString::from_static("globalName");
    let global = UnlinkedFunction::fixture_with_closure_variables(
        eval_code(0, false),
        vec![UnlinkedConstant::primitive(Value::String(global_name.clone())).unwrap()],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::Global,
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    )
    .with_eval_environments(vec![EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: vec![EvalBinding {
                    name: global_name,
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
        variable_environment: EvalVariableEnvironment::Global,
        caller_strict: false,
        super_call_allowed: false,
        super_allowed: false,
    }]);
    assert!(
        verify_unlinked_tree(&global)
            .unwrap_err()
            .to_string()
            .contains("function scope segment")
    );

    let mut environment = ordinary_environment(None);
    environment.scopes[2].bindings = vec![EvalBinding {
        name: JsString::from_static("outerLexical"),
        source: EvalBindingSource::Closure(0),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    }]
    .into_boxed_slice();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::None,
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    )
    .with_eval_environments(vec![environment]);
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
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
        vec![UnlinkedVariableDefinition::lexical(None, false)],
    );
    assert!(
        verify_unlinked_tree(&parent)
            .unwrap_err()
            .to_string()
            .contains("closure descriptor")
    );
}

#[test]
fn eval_environment_authenticates_ordinary_closure_names_to_parent_definitions() {
    let spoofed = JsString::from_static("spoofed");
    let mut environment = ordinary_environment(None);
    environment.scopes[2].bindings = vec![EvalBinding {
        name: spoofed.clone(),
        source: EvalBindingSource::Closure(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    }]
    .into_boxed_slice();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        eval_code(0, false),
        vec![UnlinkedConstant::primitive(Value::String(spoofed.clone())).unwrap()],
        FunctionMetadata {
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
    .with_eval_environments(vec![environment]);
    let parent = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
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
            JsString::from_static("real"),
        ))],
    );
    assert!(
        verify_unlinked_tree(&parent)
            .unwrap_err()
            .to_string()
            .contains("parent local definition")
    );
}

#[test]
fn eval_environment_rejects_a_local_relay_of_a_global_parent_slot() {
    let name = JsString::from_static("globalName");
    let mut environment = ordinary_environment(None);
    environment.scopes[2].bindings = vec![EvalBinding {
        name: name.clone(),
        source: EvalBindingSource::Closure(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    }]
    .into_boxed_slice();
    let child = UnlinkedFunction::fixture_with_closure_variables(
        eval_code(0, false),
        vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentClosure(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    )
    .with_eval_environments(vec![environment]);
    let root = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::FClosure(1),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(name)).unwrap(),
            UnlinkedConstant::child(child),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::Global,
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    assert!(
        verify_unlinked_tree(&root)
            .unwrap_err()
            .to_string()
            .contains("global parent slot")
    );
}

#[test]
fn eval_variable_scope_must_select_the_current_function_root() {
    let mut environment = ordinary_environment(None);
    environment.variable_environment = EvalVariableEnvironment::StrictLocal(0);
    let function = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![environment]);

    let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
    assert!(error.to_string().contains("wrong function anchor"));
}

#[test]
fn eval_variable_environment_matches_publication_tree_topology() {
    let root_with_function_scope = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![ordinary_environment(None)]);
    assert!(
        verify_unlinked_tree(&root_with_function_scope)
            .unwrap_err()
            .to_string()
            .contains("segment count")
    );

    let child_with_global = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::Global,
        caller_strict: false,
        super_call_allowed: false,
        super_allowed: false,
    }]);
    assert!(
        verify_unlinked_tree(&script_with_child(child_with_global))
            .unwrap_err()
            .to_string()
            .contains("segment count")
    );
}

#[test]
fn nested_eval_environment_may_contain_ancestor_function_roots() {
    let environment = EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::StrictLocal(1),
        caller_strict: true,
        super_call_allowed: false,
        super_allowed: false,
    };
    let function = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![environment]);

    verify_unlinked_tree(&script_with_child(function)).unwrap();
}
