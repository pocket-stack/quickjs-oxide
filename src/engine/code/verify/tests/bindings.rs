use super::*;

#[test]
fn global_reference_operand_accepts_only_named_global_closures() {
    let name = JsString::from_static("globalName");
    let valid = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::GlobalReference(0),
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
            source: ClosureSource::Global,
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    verify_unlinked_tree(&valid).unwrap();

    let local_child = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::GlobalReference(0),
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
            kind: ClosureVariableKind::Normal,
        }],
    );
    let local_parent = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(local_child)],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::ordinary(Some(name))],
    );
    assert!(
        verify_unlinked_tree(&local_parent)
            .unwrap_err()
            .to_string()
            .contains("global closure opcode referenced a non-global closure descriptor")
    );

    let out_of_bounds = UnlinkedFunction::fixture(
        vec![Instruction::GlobalReference(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&out_of_bounds)
            .unwrap_err()
            .to_string()
            .contains("closure variable bytecode operand is out of bounds")
    );
}

#[test]
fn captured_with_object_uses_only_initialize_and_close_local_lifecycle_ops() {
    verify_unlinked_tree(&script_with_child(captured_with_object_function(false))).unwrap();

    for instruction in [
        Instruction::GetLocal(0),
        Instruction::PutLocal(0),
        Instruction::SetLocal(0),
    ] {
        let code = match instruction {
            Instruction::GetLocal(_) => vec![instruction, Instruction::Return],
            Instruction::PutLocal(_) => vec![
                Instruction::Undefined,
                instruction,
                Instruction::Undefined,
                Instruction::Return,
            ],
            Instruction::SetLocal(_) => {
                vec![Instruction::Undefined, instruction, Instruction::Return]
            }
            _ => unreachable!(),
        };
        let forged = UnlinkedFunction::fixture(
            code,
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_fixture_definitions(Vec::new(), vec![UnlinkedVariableDefinition::with_object()]);
        assert!(
            verify_unlinked_tree(&script_with_child(forged))
                .unwrap_err()
                .to_string()
                .contains("ordinary local opcode referenced a private with object")
        );
    }
}

#[test]
fn strict_bytecode_cannot_publish_a_local_with_object() {
    assert!(
        verify_unlinked_tree(&script_with_child(captured_with_object_function(true)))
            .unwrap_err()
            .to_string()
            .contains("strict or malformed bytecode contains a with-object local")
    );
}

#[test]
fn with_object_closure_descriptor_rejects_non_local_sources() {
    for (source, diagnostic) in [
        (ClosureSource::ParentArgument(0), "with-object descriptor"),
        (ClosureSource::Global, "non-global binding metadata"),
    ] {
        let child = UnlinkedFunction::fixture_with_closure_variables(
            vec![Instruction::Undefined, Instruction::Return],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<with>")))
                    .unwrap(),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source,
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::WithObject,
            }],
        );
        assert!(
            verify_unlinked_tree(&child)
                .unwrap_err()
                .to_string()
                .contains(diagnostic)
        );
    }
}

#[test]
fn function_name_metadata_erasure_requires_a_direct_eval_child() {
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
    let named_parent = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            local_count: 1,
            function_name_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_name(Some(JsString::from_static("named")));
    let script = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(named_parent)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );

    assert!(
        verify_unlinked_tree(&script)
            .unwrap_err()
            .to_string()
            .contains("parent local definition"),
        "a child without direct eval erased FunctionName metadata",
    );
}

#[test]
fn erased_function_name_view_cannot_stick_into_a_plain_descendant() {
    let name = JsString::from_static("named");
    let leaf = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
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
    );
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
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: vec![EvalBinding {
                    name: name.clone(),
                    source: EvalBindingSource::Closure(0),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
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
        variable_environment: EvalVariableEnvironment::StrictLocal(1),
        caller_strict: true,
        super_call_allowed: false,
        super_allowed: false,
    };
    let middle = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::Undefined,
            Instruction::Eval {
                argument_count: 0,
                environment: 0,
            },
            Instruction::Drop,
            Instruction::FClosure(1),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(name.clone())).unwrap(),
            UnlinkedConstant::child(leaf),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            strict: true,
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
    let named_parent = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(middle)],
        FunctionMetadata {
            local_count: 1,
            function_name_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_name(Some(name));
    let script = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(named_parent)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );

    assert!(
        verify_unlinked_tree(&script)
            .unwrap_err()
            .to_string()
            .contains("not the first source request"),
        "a plain descendant retained an erased FunctionName view",
    );
}

#[test]
fn later_eval_child_cannot_rewrite_an_earlier_function_name_request() {
    let name = JsString::from_static("named");
    let plain_child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
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
            kind: ClosureVariableKind::FunctionName,
        }],
    );
    let eval_environment = EvalEnvironment {
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
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: vec![EvalBinding {
                    name: name.clone(),
                    source: EvalBindingSource::Closure(0),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
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
        variable_environment: EvalVariableEnvironment::StrictLocal(1),
        caller_strict: true,
        super_call_allowed: false,
        super_allowed: false,
    };
    let eval_child = UnlinkedFunction::fixture_with_closure_variables(
        eval_code(0, false),
        vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            strict: true,
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
    .with_eval_environments(vec![eval_environment]);
    let middle = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::FClosure(1),
            Instruction::Drop,
            Instruction::FClosure(2),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(name.clone())).unwrap(),
            UnlinkedConstant::child(plain_child),
            UnlinkedConstant::child(eval_child),
        ],
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
    );
    let named_parent = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(middle)],
        FunctionMetadata {
            local_count: 1,
            function_name_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_name(Some(name));
    let script = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(named_parent)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );

    assert!(
        verify_unlinked_tree(&script)
            .unwrap_err()
            .to_string()
            .contains("not the first source request"),
        "a later eval child authenticated an impossible parent Normal view",
    );
}
