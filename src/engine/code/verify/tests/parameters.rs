use super::*;

#[test]
fn identifier_rest_metadata_authenticates_its_exact_entry_pair() {
    let valid = UnlinkedFunction::fixture(
        vec![
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 1,
            rest_parameter: Some(1),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    verify_unlinked_tree(&script_with_child(valid)).unwrap();

    let unauthenticated = UnlinkedFunction::fixture(
        vec![
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 2,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(unauthenticated))
            .unwrap_err()
            .to_string()
            .contains("no authenticated parameter metadata")
    );

    let misplaced = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Drop,
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 1,
            rest_parameter: Some(1),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(misplaced))
            .unwrap_err()
            .to_string()
            .contains("no exact entry initialization")
    );

    let wrong_target = UnlinkedFunction::fixture(
        vec![
            Instruction::Rest(1),
            Instruction::PutArg(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 1,
            rest_parameter: Some(1),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(wrong_target))
            .unwrap_err()
            .to_string()
            .contains("no exact entry initialization")
    );

    let duplicate = UnlinkedFunction::fixture(
        vec![
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Rest(1),
            Instruction::PutArg(1),
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 1,
            rest_parameter: Some(1),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(duplicate))
            .unwrap_err()
            .to_string()
            .contains("no exact entry initialization")
    );

    let mapped_arguments = UnlinkedFunction::fixture(
        vec![
            Instruction::Arguments(crate::engine::code::bytecode::ArgumentsKind::Mapped),
            Instruction::PutLocal(0),
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 1,
            rest_parameter: Some(1),
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(mapped_arguments))
            .unwrap_err()
            .to_string()
            .contains("malformed arguments prologue")
    );

    let forged_arguments_local = UnlinkedFunction::fixture(
        vec![
            Instruction::Arguments(crate::engine::code::bytecode::ArgumentsKind::Unmapped),
            Instruction::PutLocal(0),
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 1,
            rest_parameter: Some(1),
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(forged_arguments_local))
            .unwrap_err()
            .to_string()
            .contains("arguments binding is not authenticated")
    );

    let malformed_metadata = UnlinkedFunction::fixture(
        vec![
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            rest_parameter: Some(1),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(malformed_metadata))
            .unwrap_err()
            .to_string()
            .contains("metadata disagrees with argument slots")
    );

    let root_rest = UnlinkedFunction::fixture(
        vec![
            Instruction::Rest(1),
            Instruction::PutArg(1),
            Instruction::Undefined,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 1,
            rest_parameter: Some(1),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        verify_unlinked_tree(&root_rest)
            .unwrap_err()
            .to_string()
            .contains("metadata disagrees with argument slots")
    );
}

#[test]
fn parameter_binding_pattern_metadata_authenticates_anonymous_entry_segments() {
    let ordinary_metadata = || FunctionMetadata {
        argument_count: 1,
        defined_argument_count: 1,
        pattern_argument_count: 1,
        parameter_pattern_end: Some(2),
        local_count: 1,
        max_stack: 1,
        ..FunctionMetadata::default()
    };
    let ordinary_code = || {
        vec![
            Instruction::GetArg(0),
            Instruction::PutLocal(0),
            Instruction::Nop,
            Instruction::GetLocal(0),
            Instruction::Return,
        ]
    };
    let make_ordinary = |code, metadata| {
        UnlinkedFunction::fixture(code, Vec::new(), metadata).with_fixture_definitions(
            vec![UnlinkedVariableDefinition::ordinary(None)],
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("value"),
            ))],
        )
    };
    verify_unlinked_tree(&script_with_child(make_ordinary(
        ordinary_code(),
        ordinary_metadata(),
    )))
    .unwrap();

    let synthetic_metadata = FunctionMetadata {
        argument_count: 1,
        defined_argument_count: 1,
        pattern_argument_count: 1,
        parameter_pattern_end: Some(9),
        parameter_environment_local_count: 1,
        local_count: 4,
        max_stack: 2,
        ..FunctionMetadata::default()
    };
    let synthetic_layout = ParameterEnvironmentLayout {
        initialization_end: 9,
        argument_cells: Box::new([]),
        pattern_copies: vec![ParameterPatternCopy {
            parameter_local: 0,
            body_local: 2,
        }]
        .into_boxed_slice(),
        default_sources: Box::new([]),
        synthetic_arguments_local: Some(1),
        arg_eval_variable_object_local: None,
    };
    let synthetic_code = || {
        vec![
            Instruction::Arguments(crate::engine::code::bytecode::ArgumentsKind::Unmapped),
            Instruction::Dup,
            Instruction::InitializeLocal(1),
            Instruction::PutLocal(3),
            Instruction::SetLocalUninitialized(0),
            Instruction::GetArg(0),
            Instruction::InitializeLocal(0),
            Instruction::GetLocalCheck(0),
            Instruction::PutLocal(2),
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ]
    };
    let unnamed_arguments = [true];
    let lexical_locals = [true, true, false, false];
    let parameter_initializer_locals = [false; 4];
    validate_pattern_parameter_bytecode_layout(
        &synthetic_metadata,
        &synthetic_code(),
        &unnamed_arguments,
        &lexical_locals,
        &parameter_initializer_locals,
        Some(&synthetic_layout),
    )
    .unwrap();
    let mut checked_read = synthetic_code();
    checked_read[8] = Instruction::GetLocalCheck(1);
    validate_pattern_parameter_bytecode_layout(
        &synthetic_metadata,
        &checked_read,
        &unnamed_arguments,
        &lexical_locals,
        &parameter_initializer_locals,
        Some(&synthetic_layout),
    )
    .unwrap();
    for forged in [
        Instruction::SetLocalUninitialized(1),
        Instruction::CloseLocal(1),
        Instruction::InitializeLocal(1),
    ] {
        let mut code = synthetic_code();
        code[8] = forged;
        assert!(
            validate_pattern_parameter_bytecode_layout(
                &synthetic_metadata,
                &code,
                &unnamed_arguments,
                &lexical_locals,
                &parameter_initializer_locals,
                Some(&synthetic_layout),
            )
            .unwrap_err()
            .contains("body lexical local")
        );
    }

    let capture_child = |source, name: Option<&'static str>, is_lexical| {
        let constants = name
            .map(|name| {
                vec![
                    UnlinkedConstant::primitive(Value::String(JsString::from_static(name)))
                        .unwrap(),
                ]
            })
            .unwrap_or_default();
        UnlinkedFunction::fixture_with_closure_variables(
            vec![
                if is_lexical {
                    Instruction::GetVarRefCheck(0)
                } else {
                    Instruction::GetVarRef(0)
                },
                Instruction::Return,
            ],
            constants,
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source,
                name: if name.is_some() {
                    ClosureVariableName::Constant(0)
                } else {
                    ClosureVariableName::None
                },
                is_lexical,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
    };
    let pattern_closure_code = || {
        vec![
            Instruction::GetArg(0),
            Instruction::PutLocal(0),
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Nop,
            Instruction::GetLocal(0),
            Instruction::Return,
        ]
    };
    let mut closure_metadata = ordinary_metadata();
    closure_metadata.parameter_pattern_end = Some(4);

    let valid_root_capture = UnlinkedFunction::fixture(
        pattern_closure_code(),
        vec![UnlinkedConstant::child(capture_child(
            ClosureSource::ParentLocal(0),
            Some("value"),
            false,
        ))],
        closure_metadata,
    )
    .with_fixture_definitions(
        vec![UnlinkedVariableDefinition::ordinary(None)],
        vec![UnlinkedVariableDefinition::ordinary(Some(
            JsString::from_static("value"),
        ))],
    );
    verify_unlinked_tree(&script_with_child(valid_root_capture)).unwrap();

    let anonymous_argument_capture = UnlinkedFunction::fixture(
        pattern_closure_code(),
        vec![UnlinkedConstant::child(capture_child(
            ClosureSource::ParentArgument(0),
            None,
            false,
        ))],
        closure_metadata,
    )
    .with_fixture_definitions(
        vec![UnlinkedVariableDefinition::ordinary(None)],
        vec![UnlinkedVariableDefinition::ordinary(Some(
            JsString::from_static("value"),
        ))],
    );
    assert!(
        verify_unlinked_tree(&script_with_child(anonymous_argument_capture))
            .unwrap_err()
            .to_string()
            .contains("anonymous pattern argument slot")
    );

    let body_lexical_capture = UnlinkedFunction::fixture(
        pattern_closure_code(),
        vec![UnlinkedConstant::child(capture_child(
            ClosureSource::ParentLocal(1),
            Some("body"),
            true,
        ))],
        FunctionMetadata {
            local_count: 2,
            ..closure_metadata
        },
    )
    .with_fixture_definitions(
        vec![UnlinkedVariableDefinition::ordinary(None)],
        vec![
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
            UnlinkedVariableDefinition::lexical(Some(JsString::from_static("body")), false),
        ],
    );
    assert!(
        verify_unlinked_tree(&script_with_child(body_lexical_capture))
            .unwrap_err()
            .to_string()
            .contains("pattern initializer closure captured a body lexical local")
    );

    let direct_body_lexical_access = UnlinkedFunction::fixture(
        vec![
            Instruction::GetArg(0),
            Instruction::Drop,
            Instruction::GetLocalCheck(1),
            Instruction::Drop,
            Instruction::Nop,
            Instruction::GetLocal(0),
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            local_count: 2,
            parameter_pattern_end: Some(4),
            ..ordinary_metadata()
        },
    )
    .with_fixture_definitions(
        vec![UnlinkedVariableDefinition::ordinary(None)],
        vec![
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
            UnlinkedVariableDefinition::lexical(Some(JsString::from_static("body")), false),
        ],
    );
    assert!(
        verify_unlinked_tree(&script_with_child(direct_body_lexical_access))
            .unwrap_err()
            .to_string()
            .contains("accessed a body lexical local")
    );

    for (code, body_pc) in [
        (
            vec![
                Instruction::Undefined,
                Instruction::InitializeLocal(0),
                Instruction::Nop,
            ],
            3,
        ),
        (
            vec![Instruction::SetLocalUninitialized(0), Instruction::Nop],
            2,
        ),
    ] {
        assert_eq!(
            validate_parameter_initializer_scope_layout(
                &FunctionMetadata {
                    local_count: 1,
                    ..FunctionMetadata::default()
                },
                &code,
                Some(body_pc),
                &[true],
                &[true],
            ),
            Err("parameter-initializer local has no exact pre-boundary TDZ lifecycle")
        );
    }

    let initializer_capture = UnlinkedFunction::fixture(
        vec![
            Instruction::GetArg(0),
            Instruction::Drop,
            Instruction::SetLocalUninitialized(1),
            Instruction::Undefined,
            Instruction::InitializeLocal(1),
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::CloseLocal(1),
            Instruction::Nop,
            Instruction::FClosure(0),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(capture_child(
            ClosureSource::ParentLocal(1),
            Some("initializer"),
            true,
        ))],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            pattern_argument_count: 1,
            parameter_pattern_end: Some(8),
            local_count: 2,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        vec![UnlinkedVariableDefinition::ordinary(None)],
        vec![
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
            UnlinkedVariableDefinition::lexical(Some(JsString::from_static("initializer")), false)
                .with_parameter_initializer(true),
        ],
    );
    let error = verify_unlinked_tree(&script_with_child(initializer_capture)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("function body closure captured a parameter-initializer local"),
        "{error}"
    );

    let rest_pattern = UnlinkedFunction::fixture(
        vec![
            Instruction::Rest(0),
            Instruction::PutLocal(0),
            Instruction::Nop,
            Instruction::GetLocal(0),
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 0,
            defined_argument_count: 1,
            rest_pattern_start: Some(0),
            parameter_pattern_end: Some(2),
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        Vec::new(),
        vec![UnlinkedVariableDefinition::ordinary(Some(
            JsString::from_static("value"),
        ))],
    );
    verify_unlinked_tree(&script_with_child(rest_pattern)).unwrap();

    let empty_rest_metadata = |defined_argument_count| FunctionMetadata {
        defined_argument_count,
        rest_pattern_start: Some(0),
        parameter_pattern_end: Some(2),
        max_stack: 1,
        ..FunctionMetadata::default()
    };
    let empty_rest_code = |body_value| {
        vec![
            Instruction::Rest(0),
            Instruction::Drop,
            Instruction::Nop,
            body_value,
            Instruction::Return,
        ]
    };
    verify_unlinked_tree(&script_with_child(UnlinkedFunction::fixture(
        empty_rest_code(Instruction::Undefined),
        Vec::new(),
        empty_rest_metadata(0),
    )))
    .unwrap();
    for pseudo_read in [
        Instruction::PushHomeObject,
        Instruction::PushThis,
        Instruction::PushNewTarget,
    ] {
        let mut metadata = empty_rest_metadata(1);
        metadata.needs_home_object = matches!(&pseudo_read, Instruction::PushHomeObject);
        verify_unlinked_tree(&script_with_child(UnlinkedFunction::fixture(
            empty_rest_code(pseudo_read),
            Vec::new(),
            metadata,
        )))
        .unwrap();
    }
    assert!(
        verify_unlinked_tree(&script_with_child(UnlinkedFunction::fixture(
            empty_rest_code(Instruction::Undefined),
            Vec::new(),
            empty_rest_metadata(1),
        )))
        .unwrap_err()
        .to_string()
        .contains("metadata disagrees with function length")
    );
    assert!(
        verify_unlinked_tree(&script_with_child(UnlinkedFunction::fixture(
            empty_rest_code(Instruction::PushThis),
            Vec::new(),
            empty_rest_metadata(0),
        )))
        .unwrap_err()
        .to_string()
        .contains("metadata disagrees with function length")
    );

    let mut missing_marker = ordinary_metadata();
    missing_marker.parameter_pattern_end = None;
    assert!(
        verify_unlinked_tree(&script_with_child(make_ordinary(
            ordinary_code(),
            missing_marker,
        )))
        .unwrap_err()
        .to_string()
        .contains("no initialization marker")
    );

    let named_slot = UnlinkedFunction::fixture(ordinary_code(), Vec::new(), ordinary_metadata())
        .with_fixture_definitions(
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("forged"),
            ))],
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("value"),
            ))],
        );
    assert!(
        verify_unlinked_tree(&script_with_child(named_slot))
            .unwrap_err()
            .to_string()
            .contains("definitions disagree with bytecode metadata")
    );

    let mut missing_read = ordinary_code();
    missing_read[0] = Instruction::Undefined;
    assert!(
        verify_unlinked_tree(&script_with_child(make_ordinary(
            missing_read,
            ordinary_metadata(),
        )))
        .unwrap_err()
        .to_string()
        .contains("exact entry reads")
    );

    let mut body_read = ordinary_code();
    body_read.insert(3, Instruction::GetArg(0));
    body_read.insert(4, Instruction::Drop);
    assert!(
        verify_unlinked_tree(&script_with_child(make_ordinary(
            body_read,
            ordinary_metadata(),
        )))
        .unwrap_err()
        .to_string()
        .contains("function body reads")
    );

    let mut anonymous_write = ordinary_code();
    anonymous_write.insert(2, Instruction::Undefined);
    anonymous_write.insert(3, Instruction::PutArg(0));
    let mut write_metadata = ordinary_metadata();
    write_metadata.parameter_pattern_end = Some(4);
    assert!(
        verify_unlinked_tree(&script_with_child(make_ordinary(
            anonymous_write,
            write_metadata,
        )))
        .unwrap_err()
        .to_string()
        .contains("writes an anonymous")
    );

    let escaped_segment = vec![
        Instruction::GetArg(0),
        Instruction::Drop,
        Instruction::Goto(4),
        Instruction::Nop,
        Instruction::Undefined,
        Instruction::Return,
    ];
    let mut escaped_metadata = ordinary_metadata();
    escaped_metadata.parameter_pattern_end = Some(3);
    assert!(
        verify_unlinked_tree(&script_with_child(make_ordinary(
            escaped_segment,
            escaped_metadata,
        )))
        .unwrap_err()
        .to_string()
        .contains("escaped its initialization segment")
    );

    let body_reentry = vec![
        Instruction::GetArg(0),
        Instruction::Drop,
        Instruction::Nop,
        Instruction::Goto(2),
    ];
    assert!(
        verify_unlinked_tree(&script_with_child(make_ordinary(
            body_reentry,
            ordinary_metadata(),
        )))
        .unwrap_err()
        .to_string()
        .contains("jumps back into pattern initialization")
    );

    let mapped_arguments = UnlinkedFunction::fixture(
        vec![
            Instruction::Arguments(crate::engine::code::bytecode::ArgumentsKind::Mapped),
            Instruction::PutLocal(0),
            Instruction::GetArg(0),
            Instruction::PutLocal(1),
            Instruction::Nop,
            Instruction::GetLocal(1),
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            pattern_argument_count: 1,
            parameter_pattern_end: Some(4),
            local_count: 2,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        vec![UnlinkedVariableDefinition::ordinary(None)],
        vec![
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("arguments"))),
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
        ],
    );
    assert!(
        verify_unlinked_tree(&script_with_child(mapped_arguments))
            .unwrap_err()
            .to_string()
            .contains("malformed arguments prologue")
    );

    assert!(
        verify_unlinked_tree(&make_ordinary(ordinary_code(), ordinary_metadata()))
            .unwrap_err()
            .to_string()
            .contains("synthetic root contains formal-parameter metadata")
    );
}

#[test]
fn identifier_default_metadata_authenticates_parameter_environment_layout() {
    let parameter_environment = |code: &[Instruction], metadata: FunctionMetadata| {
        let initialization_end = code
            .iter()
            .position(|instruction| matches!(instruction, Instruction::Nop))
            .and_then(|pc| u32::try_from(pc).ok())
            .expect("parameter fixture has an initialization marker");
        ParameterEnvironmentLayout {
            initialization_end,
            argument_cells: (0..metadata.argument_count)
                .map(|argument| ParameterArgumentCell {
                    argument,
                    parameter_local: argument,
                    body: ParameterBodyStorage::Argument(argument),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            pattern_copies: Box::new([]),
            default_sources: (0..metadata.argument_count)
                .filter(|argument| *argument >= metadata.defined_argument_count)
                .filter(|argument| metadata.rest_parameter != Some(*argument))
                .map(ParameterDefaultSource::Argument)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            synthetic_arguments_local: None,
            arg_eval_variable_object_local: None,
        }
    };
    let metadata = || FunctionMetadata {
        argument_count: 2,
        defined_argument_count: 0,
        rest_parameter: Some(1),
        parameter_environment_local_count: 2,
        local_count: 2,
        max_stack: 3,
        ..FunctionMetadata::default()
    };
    let code = || {
        vec![
            Instruction::SetLocalUninitialized(1),
            Instruction::SetLocalUninitialized(0),
            Instruction::GetArg(0),
            Instruction::Dup,
            Instruction::Undefined,
            Instruction::StrictEq,
            Instruction::IfFalse(11),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Dup,
            Instruction::PutArg(0),
            Instruction::InitializeLocal(0),
            Instruction::Rest(1),
            Instruction::Dup,
            Instruction::PutArg(1),
            Instruction::InitializeLocal(1),
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ]
    };
    let make_with_constants = |code: Vec<Instruction>,
                               constants: Vec<UnlinkedConstant>,
                               metadata: FunctionMetadata| {
        let layout = (metadata.parameter_environment_local_count != 0)
            .then(|| parameter_environment(&code, metadata));
        UnlinkedFunction::fixture(code, constants, metadata)
            .with_parameter_environment(layout)
            .with_fixture_definitions(
                vec![
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("rest"))),
                ],
                vec![
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("a")), false),
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("rest")), false),
                ],
            )
    };
    let make = |code, metadata| make_with_constants(code, Vec::new(), metadata);
    let make_with_body_local = |code: Vec<Instruction>, metadata: FunctionMetadata| {
        let layout = parameter_environment(&code, metadata);
        UnlinkedFunction::fixture(code, Vec::new(), metadata)
            .with_parameter_environment(Some(layout))
            .with_fixture_definitions(
                vec![
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("rest"))),
                ],
                vec![
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("a")), false),
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("rest")), false),
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("body"))),
                ],
            )
    };
    let capture_child = |source, is_lexical| {
        UnlinkedFunction::fixture_with_closure_variables(
            vec![
                if is_lexical {
                    Instruction::GetVarRefCheck(0)
                } else {
                    Instruction::GetVarRef(0)
                },
                Instruction::Return,
            ],
            vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("a"))).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source,
                name: ClosureVariableName::Constant(0),
                is_lexical,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
    };

    verify_unlinked_tree(&script_with_child(make(code(), metadata()))).unwrap();

    let default_only_metadata = || FunctionMetadata {
        argument_count: 1,
        defined_argument_count: 0,
        parameter_environment_local_count: 1,
        local_count: 1,
        max_stack: 3,
        ..FunctionMetadata::default()
    };
    let default_only_code = || {
        vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::GetArg(0),
            Instruction::Dup,
            Instruction::Undefined,
            Instruction::StrictEq,
            Instruction::IfFalse(10),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Dup,
            Instruction::PutArg(0),
            Instruction::InitializeLocal(0),
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ]
    };
    let make_default_only =
        |code: Vec<Instruction>, constants: Vec<UnlinkedConstant>, metadata: FunctionMetadata| {
            let layout = (metadata.parameter_environment_local_count != 0)
                .then(|| parameter_environment(&code, metadata));
            UnlinkedFunction::fixture(code, constants, metadata)
                .with_parameter_environment(layout)
                .with_fixture_definitions(
                    vec![UnlinkedVariableDefinition::ordinary(Some(
                        JsString::from_static("value"),
                    ))],
                    vec![UnlinkedVariableDefinition::lexical(
                        Some(JsString::from_static("value")),
                        false,
                    )],
                )
        };
    verify_unlinked_tree(&script_with_child(make_default_only(
        default_only_code(),
        Vec::new(),
        default_only_metadata(),
    )))
    .unwrap();

    let mut missing_default_only_argument = default_only_code();
    missing_default_only_argument[1] = Instruction::Undefined;
    assert!(
        verify_unlinked_tree(&script_with_child(make_default_only(
            missing_default_only_argument,
            Vec::new(),
            default_only_metadata(),
        )))
        .unwrap_err()
        .to_string()
        .contains("default cell has no exact argument selection")
    );

    let mut missing_default_only_environment = default_only_metadata();
    missing_default_only_environment.parameter_environment_local_count = 0;
    assert!(
        verify_unlinked_tree(&script_with_child(make_default_only(
            default_only_code(),
            Vec::new(),
            missing_default_only_environment,
        )))
        .unwrap_err()
        .to_string()
        .contains("default parameter metadata has no parameter environment")
    );

    let eval_descendant_environment = EvalEnvironment {
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
    let plain_eval_descendant = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![eval_descendant_environment.clone()]);
    let plain_parent = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        vec![UnlinkedConstant::child(plain_eval_descendant)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    verify_unlinked_tree(&script_with_child(plain_parent)).unwrap();

    let eval_descendant = UnlinkedFunction::fixture(
        eval_code(0, false),
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_eval_environments(vec![eval_descendant_environment]);
    let parameter_parent = make_default_only(
        default_only_code(),
        vec![UnlinkedConstant::child(eval_descendant)],
        default_only_metadata(),
    );
    verify_unlinked_tree(&script_with_child(parameter_parent)).unwrap();

    let mut missing_environment = metadata();
    missing_environment.parameter_environment_local_count = 0;
    assert!(
        verify_unlinked_tree(&script_with_child(make(code(), missing_environment)))
            .unwrap_err()
            .to_string()
            .contains("metadata disagrees with argument slots")
    );

    let mut wrong_tdz = code();
    wrong_tdz.swap(0, 1);
    assert!(
        verify_unlinked_tree(&script_with_child(make(wrong_tdz, metadata())))
            .unwrap_err()
            .to_string()
            .contains("exact TDZ entry initialization")
    );

    let mut forged_pseudo_code = code();
    forged_pseudo_code.insert(0, Instruction::PushThis);
    forged_pseudo_code.insert(1, Instruction::PutLocal(2));
    forged_pseudo_code[8] = Instruction::IfFalse(13);
    let mut forged_pseudo_metadata = metadata();
    forged_pseudo_metadata.local_count = 3;
    let forged_pseudo_layout = parameter_environment(&forged_pseudo_code, forged_pseudo_metadata);
    let forged_pseudo =
        UnlinkedFunction::fixture(forged_pseudo_code, Vec::new(), forged_pseudo_metadata)
            .with_parameter_environment(Some(forged_pseudo_layout))
            .with_fixture_definitions(
                vec![
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("rest"))),
                ],
                vec![
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("a")), false),
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("rest")), false),
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("forged"))),
                ],
            );
    assert!(
        verify_unlinked_tree(&script_with_child(forged_pseudo))
            .unwrap_err()
            .to_string()
            .contains("pseudo-binding definition is not authenticated")
    );

    let mut pseudo_write_code = code();
    pseudo_write_code.insert(0, Instruction::PushThis);
    pseudo_write_code.insert(1, Instruction::PutLocal(2));
    pseudo_write_code[8] = Instruction::IfFalse(14);
    pseudo_write_code.insert(11, Instruction::SetLocal(2));
    let mut pseudo_write_metadata = metadata();
    pseudo_write_metadata.local_count = 3;
    let pseudo_write_layout = parameter_environment(&pseudo_write_code, pseudo_write_metadata);
    let pseudo_write =
        UnlinkedFunction::fixture(pseudo_write_code, Vec::new(), pseudo_write_metadata)
            .with_parameter_environment(Some(pseudo_write_layout))
            .with_fixture_definitions(
                vec![
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("rest"))),
                ],
                vec![
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("a")), false),
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("rest")), false),
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<this>"))),
                ],
            );
    assert!(
        verify_unlinked_tree(&script_with_child(pseudo_write))
            .unwrap_err()
            .to_string()
            .contains("initializer has an unauthenticated local access")
    );

    let mut wrong_rest_target = code();
    wrong_rest_target[14] = Instruction::PutArg(0);
    let error =
        verify_unlinked_tree(&script_with_child(make(wrong_rest_target, metadata()))).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("parameter default cell has no exact argument selection"),
        "{error}",
    );

    let missing_default = vec![
        Instruction::SetLocalUninitialized(1),
        Instruction::SetLocalUninitialized(0),
        Instruction::GetArg(0),
        Instruction::InitializeLocal(0),
        Instruction::Rest(1),
        Instruction::Dup,
        Instruction::PutArg(1),
        Instruction::InitializeLocal(1),
        Instruction::Nop,
        Instruction::Undefined,
        Instruction::Return,
    ];
    assert!(
        verify_unlinked_tree(&script_with_child(make(missing_default, metadata())))
            .unwrap_err()
            .to_string()
            .contains("parameter default cell has no exact argument selection")
    );

    let mut wrong_default_target = code();
    wrong_default_target[6] = Instruction::IfFalse(12);
    assert!(
        verify_unlinked_tree(&script_with_child(make(wrong_default_target, metadata())))
            .unwrap_err()
            .to_string()
            .contains("parameter default cell has no exact argument selection")
    );

    let mut missing_default_sync = code();
    missing_default_sync[10] = Instruction::Drop;
    assert!(
        verify_unlinked_tree(&script_with_child(make(missing_default_sync, metadata())))
            .unwrap_err()
            .to_string()
            .contains("parameter default cell has no exact argument selection")
    );

    let mut wrong_default_argument = code();
    wrong_default_argument[10] = Instruction::PutArg(1);
    assert!(
        verify_unlinked_tree(&script_with_child(make(wrong_default_argument, metadata())))
            .unwrap_err()
            .to_string()
            .contains("parameter default cell has no exact argument selection")
    );

    let mut raw_argument_rhs = code();
    raw_argument_rhs[8] = Instruction::GetArg(1);
    assert!(
        verify_unlinked_tree(&script_with_child(make(raw_argument_rhs, metadata())))
            .unwrap_err()
            .to_string()
            .contains("parameter rest cell has no exact initialization")
    );

    let mut unchecked_parameter_rhs = code();
    unchecked_parameter_rhs[8] = Instruction::GetLocal(0);
    assert!(
        verify_unlinked_tree(&script_with_child(make(
            unchecked_parameter_rhs,
            metadata()
        )))
        .unwrap_err()
        .to_string()
        .contains("initializer has an unauthenticated local access")
    );

    let mut body_local_rhs = code();
    body_local_rhs[8] = Instruction::GetLocal(2);
    let mut body_local_metadata = metadata();
    body_local_metadata.local_count = 3;
    assert!(
        verify_unlinked_tree(&script_with_child(make_with_body_local(
            body_local_rhs,
            body_local_metadata,
        )))
        .unwrap_err()
        .to_string()
        .contains("initializer has an unauthenticated local access")
    );

    let mut initializer_parameter_capture = code();
    initializer_parameter_capture[8] = Instruction::FClosure(0);
    initializer_parameter_capture.insert(17, Instruction::CloseLocal(0));
    verify_unlinked_tree(&script_with_child(make_with_constants(
        initializer_parameter_capture,
        vec![UnlinkedConstant::child(capture_child(
            ClosureSource::ParentLocal(0),
            true,
        ))],
        metadata(),
    )))
    .unwrap();

    let mut initializer_raw_argument_capture = code();
    initializer_raw_argument_capture[8] = Instruction::FClosure(0);
    assert!(
        verify_unlinked_tree(&script_with_child(make_with_constants(
            initializer_raw_argument_capture,
            vec![UnlinkedConstant::child(capture_child(
                ClosureSource::ParentArgument(0),
                false,
            ))],
            metadata(),
        )))
        .unwrap_err()
        .to_string()
        .contains("initializer closure captured a raw argument slot")
    );

    let mut body_raw_argument_capture = code();
    body_raw_argument_capture.insert(17, Instruction::FClosure(0));
    body_raw_argument_capture.insert(18, Instruction::Drop);
    verify_unlinked_tree(&script_with_child(make_with_constants(
        body_raw_argument_capture,
        vec![UnlinkedConstant::child(capture_child(
            ClosureSource::ParentArgument(0),
            false,
        ))],
        metadata(),
    )))
    .unwrap();

    let mut body_parameter_capture = code();
    body_parameter_capture.insert(17, Instruction::CloseLocal(0));
    body_parameter_capture.insert(18, Instruction::FClosure(0));
    body_parameter_capture.insert(19, Instruction::Drop);
    assert!(
        verify_unlinked_tree(&script_with_child(make_with_constants(
            body_parameter_capture,
            vec![UnlinkedConstant::child(capture_child(
                ClosureSource::ParentLocal(0),
                true,
            ))],
            metadata(),
        )))
        .unwrap_err()
        .to_string()
        .contains("body closure captured a parameter-initializer cell")
    );

    let mut body_parameter_cell = code();
    body_parameter_cell.insert(17, Instruction::GetLocalCheck(0));
    assert!(
        verify_unlinked_tree(&script_with_child(make(body_parameter_cell, metadata())))
            .unwrap_err()
            .to_string()
            .contains("function body accesses a parameter-environment cell")
    );

    let mut own_eval = code();
    own_eval.insert(
        own_eval.len() - 1,
        Instruction::Eval {
            argument_count: 0,
            environment: 0,
        },
    );
    assert!(
        verify_unlinked_tree(&script_with_child(make(own_eval, metadata())))
            .unwrap_err()
            .to_string()
            .contains("environment operand is out of bounds")
    );

    let ordinary_code = code();
    let ordinary_metadata = metadata();
    let ordinary_layout = parameter_environment(&ordinary_code, ordinary_metadata);
    let ordinary_locals = UnlinkedFunction::fixture(ordinary_code, Vec::new(), ordinary_metadata)
        .with_parameter_environment(Some(ordinary_layout));
    assert!(
        verify_unlinked_tree(&script_with_child(ordinary_locals))
            .unwrap_err()
            .to_string()
            .contains("parameter environment cell definition is not authenticated")
    );

    let mut root_metadata = metadata();
    root_metadata.rest_parameter = None;
    let error = verify_unlinked_tree(&make(code(), root_metadata)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("synthetic root contains parameter-environment metadata"),
        "{error}"
    );
}

#[test]
fn mixed_parameter_environment_metadata_authenticates_cells_copies_and_defaults() {
    let metadata = |marker| FunctionMetadata {
        argument_count: 2,
        defined_argument_count: 2,
        parameter_environment_local_count: 2,
        pattern_argument_count: 1,
        parameter_pattern_end: Some(marker),
        local_count: 3,
        max_stack: 1,
        ..FunctionMetadata::default()
    };
    let layout = |marker| ParameterEnvironmentLayout {
        initialization_end: marker,
        argument_cells: vec![ParameterArgumentCell {
            argument: 0,
            parameter_local: 0,
            body: ParameterBodyStorage::Argument(0),
        }]
        .into_boxed_slice(),
        pattern_copies: vec![ParameterPatternCopy {
            parameter_local: 1,
            body_local: 2,
        }]
        .into_boxed_slice(),
        default_sources: Box::new([]),
        synthetic_arguments_local: None,
        arg_eval_variable_object_local: None,
    };
    let code = || {
        vec![
            Instruction::SetLocalUninitialized(1),
            Instruction::SetLocalUninitialized(0),
            Instruction::GetArg(0),
            Instruction::InitializeLocal(0),
            Instruction::GetArg(1),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::InitializeLocal(1),
            Instruction::GetLocalCheck(1),
            Instruction::PutLocal(2),
            Instruction::Nop,
            Instruction::GetLocal(2),
            Instruction::Return,
        ]
    };
    let definitions = |function: UnlinkedFunction, target_name, target_lexical| {
        function.with_fixture_definitions(
            vec![
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                UnlinkedVariableDefinition::ordinary(None),
            ],
            vec![
                UnlinkedVariableDefinition::lexical(Some(JsString::from_static("a")), false),
                UnlinkedVariableDefinition::lexical(Some(JsString::from_static("b")), false),
                if target_lexical {
                    UnlinkedVariableDefinition::lexical(Some(target_name), false)
                } else {
                    UnlinkedVariableDefinition::ordinary(Some(target_name))
                },
            ],
        )
    };
    let make = |code, metadata, layout| {
        definitions(
            UnlinkedFunction::fixture(code, Vec::new(), metadata)
                .with_parameter_environment(layout),
            JsString::from_static("b"),
            false,
        )
    };

    verify_unlinked_tree(&script_with_child(make(
        code(),
        metadata(10),
        Some(layout(10)),
    )))
    .unwrap();

    let error =
        verify_unlinked_tree(&script_with_child(make(code(), metadata(10), None))).unwrap_err();
    assert!(error.to_string().contains("immutable layout"), "{error}");

    let mut wrong_copy_source = layout(10);
    wrong_copy_source.pattern_copies[0].parameter_local = 0;
    let error = verify_unlinked_tree(&script_with_child(make(
        code(),
        metadata(10),
        Some(wrong_copy_source),
    )))
    .unwrap_err();
    assert!(
        error.to_string().contains("copy source overlaps"),
        "{error}"
    );

    let mut wrong_tdz = code();
    wrong_tdz.swap(0, 1);
    let error = verify_unlinked_tree(&script_with_child(make(
        wrong_tdz,
        metadata(10),
        Some(layout(10)),
    )))
    .unwrap_err();
    assert!(error.to_string().contains("exact TDZ"), "{error}");

    let mut duplicate_initializer = code();
    duplicate_initializer[7] = Instruction::InitializeLocal(0);
    let error = verify_unlinked_tree(&script_with_child(make(
        duplicate_initializer,
        metadata(10),
        Some(layout(10)),
    )))
    .unwrap_err();
    assert!(
        error.to_string().contains("one exact initializer"),
        "{error}"
    );

    let mut wrong_copy_code = code();
    wrong_copy_code[8] = Instruction::GetLocalCheck(0);
    let error = verify_unlinked_tree(&script_with_child(make(
        wrong_copy_code,
        metadata(10),
        Some(layout(10)),
    )))
    .unwrap_err();
    assert!(error.to_string().contains("copy phase"), "{error}");

    let mut body_cell_read = code();
    body_cell_read.insert(11, Instruction::GetLocalCheck(1));
    body_cell_read.insert(12, Instruction::Drop);
    let error = verify_unlinked_tree(&script_with_child(make(
        body_cell_read,
        metadata(10),
        Some(layout(10)),
    )))
    .unwrap_err();
    assert!(error.to_string().contains("body accesses"), "{error}");

    let mut bypassed_plain_cell = code();
    bypassed_plain_cell[2] = Instruction::Undefined;
    let error = verify_unlinked_tree(&script_with_child(make(
        bypassed_plain_cell,
        metadata(10),
        Some(layout(10)),
    )))
    .unwrap_err();
    assert!(
        error.to_string().contains("plain parameter argument cell"),
        "{error}"
    );

    let mut raw_named_bypass = code();
    raw_named_bypass.insert(8, Instruction::GetArg(0));
    raw_named_bypass.insert(9, Instruction::Drop);
    let error = verify_unlinked_tree(&script_with_child(make(
        raw_named_bypass,
        metadata(12),
        Some(layout(12)),
    )))
    .unwrap_err();
    assert!(
        error.to_string().contains("plain parameter argument cell"),
        "{error}"
    );

    let wrong_target_definition = definitions(
        UnlinkedFunction::fixture(code(), Vec::new(), metadata(10))
            .with_parameter_environment(Some(layout(10))),
        JsString::from_static("not_b"),
        false,
    );
    let error = verify_unlinked_tree(&script_with_child(wrong_target_definition)).unwrap_err();
    assert!(error.to_string().contains("same-name"), "{error}");

    let lexical_target_definition = definitions(
        UnlinkedFunction::fixture(code(), Vec::new(), metadata(10))
            .with_parameter_environment(Some(layout(10))),
        JsString::from_static("b"),
        true,
    );
    let error = verify_unlinked_tree(&script_with_child(lexical_target_definition)).unwrap_err();
    assert!(error.to_string().contains("body lexical local"), "{error}");

    let default_metadata = FunctionMetadata {
        argument_count: 1,
        defined_argument_count: 0,
        pattern_argument_count: 1,
        parameter_pattern_end: Some(10),
        max_stack: 3,
        ..FunctionMetadata::default()
    };
    let default_layout = ParameterEnvironmentLayout {
        initialization_end: 10,
        argument_cells: Box::new([]),
        pattern_copies: Box::new([]),
        default_sources: vec![ParameterDefaultSource::Argument(0)].into_boxed_slice(),
        synthetic_arguments_local: None,
        arg_eval_variable_object_local: None,
    };
    let default_code = vec![
        Instruction::GetArg(0),
        Instruction::Dup,
        Instruction::Undefined,
        Instruction::StrictEq,
        Instruction::IfTrue(7),
        Instruction::Drop,
        Instruction::Goto(10),
        Instruction::Drop,
        Instruction::Undefined,
        Instruction::Goto(5),
        Instruction::Nop,
        Instruction::Undefined,
        Instruction::Return,
    ];
    let make_default = |code, metadata, layout| {
        UnlinkedFunction::fixture(code, Vec::new(), metadata)
            .with_parameter_environment(Some(layout))
            .with_fixture_definitions(vec![UnlinkedVariableDefinition::ordinary(None)], Vec::new())
    };
    verify_unlinked_tree(&script_with_child(make_default(
        default_code,
        default_metadata,
        default_layout.clone(),
    )))
    .unwrap();

    let missing_selection = vec![
        Instruction::GetArg(0),
        Instruction::Drop,
        Instruction::Nop,
        Instruction::Undefined,
        Instruction::Return,
    ];
    let missing_selection_metadata = FunctionMetadata {
        parameter_pattern_end: Some(2),
        max_stack: 1,
        ..default_metadata
    };
    let mut missing_selection_layout = default_layout;
    missing_selection_layout.initialization_end = 2;
    let error = verify_unlinked_tree(&script_with_child(make_default(
        missing_selection,
        missing_selection_metadata,
        missing_selection_layout,
    )))
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("default has no exact argument selection"),
        "{error}"
    );
}

#[test]
fn argument_definition_cannot_claim_parameter_initializer_provenance() {
    let function = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    )
    .with_fixture_definitions(
        vec![
            UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("argument")))
                .with_parameter_initializer(true),
        ],
        Vec::new(),
    );

    let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("argument definition is not an ordinary mutable binding"),
        "{error}"
    );
}
