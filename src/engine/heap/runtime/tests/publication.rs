use super::*;

#[test]
fn bytecode_is_rooted_and_calls_separate_caller_from_callee_realm() {
    let runtime = Runtime::new();
    let mut compiler_context = runtime.new_context();
    let compiler_realm = compiler_context.realm;
    let intrinsic_realm_roots = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(compiler_realm)
        .unwrap();
    let function = compiler_context.compile("this").unwrap();
    let bytecode_id = function.bytecode_id();

    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode_strong_count(bytecode_id),
        Ok(1)
    );
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(compiler_realm),
        Ok(intrinsic_realm_roots + 1)
    );
    let duplicate = function.clone();
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode_strong_count(bytecode_id),
        Ok(2)
    );
    drop(duplicate);

    let mut caller_context = runtime.new_context();
    let caller_global = caller_context.global_object().unwrap();
    drop(compiler_context);
    assert_eq!(runtime.heap_counts().context_nodes, 2);

    let snapshot = runtime.snapshot_function_bytecode(&function).unwrap();
    assert_eq!(snapshot.realm, compiler_realm);
    drop(snapshot);
    assert_eq!(
        caller_context.execute(&function).unwrap(),
        Value::Object(caller_global)
    );

    drop(function);
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.heap_counts().context_nodes, 2);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
}

#[test]
fn publication_rejects_value_opcode_for_child_bytecode_before_heap_changes() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let child = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let root = UnlinkedFunction::fixture(
        vec![Instruction::PushConst(0), Instruction::Return],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );

    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, root),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn publication_rejects_mismatched_regexp_constants_even_in_dead_code() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let pattern = JsString::from_static("a");
    let flags = JsString::from_static("g");
    let program = std::rc::Rc::new(crate::regexp::compile(&pattern, &flags).unwrap());

    let regexp_as_value = UnlinkedFunction::fixture(
        vec![Instruction::PushConst(0), Instruction::Return],
        vec![UnlinkedConstant::regexp(pattern, program)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, regexp_as_value),
        Err(RuntimeError::Engine(_))
    ));

    let value_as_regexp = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::Return,
            Instruction::RegExp(0),
        ],
        vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("a"))).unwrap()],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, value_as_regexp),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn publication_rejects_string_key_opcodes_with_non_string_constants() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    for (code, max_stack) in [
        (
            vec![
                Instruction::Undefined,
                Instruction::SetName(0),
                Instruction::Return,
            ],
            1,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::GetField(0),
                Instruction::Return,
            ],
            1,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::GetField2(0),
                Instruction::Drop,
                Instruction::Return,
            ],
            2,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::PutField(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            2,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::DefineField(0),
                Instruction::Return,
            ],
            2,
        ),
        (
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::DefineMethod {
                    key: 0,
                    kind: crate::engine::code::bytecode::DefineMethodKind::Method,
                    enumerable: true,
                },
                Instruction::Return,
            ],
            2,
        ),
    ] {
        let function = UnlinkedFunction::fixture(
            code,
            vec![UnlinkedConstant::primitive(Value::Int(1)).unwrap()],
            FunctionMetadata {
                max_stack,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    }

    let child = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let function = UnlinkedFunction::fixture(
        vec![
            Instruction::Undefined,
            Instruction::GetField(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn publication_rejects_malformed_exception_and_gosub_regions() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;

    for code in [
        vec![
            Instruction::Catch(99),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Gosub(99),
        ],
        vec![Instruction::PushI32(0), Instruction::Ret],
        vec![
            Instruction::Gosub(3),
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Return,
        ],
    ] {
        let malformed = UnlinkedFunction::fixture(
            code,
            Vec::new(),
            FunctionMetadata {
                max_stack: 2,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, malformed),
            Err(RuntimeError::Engine(_))
        ));
    }
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);
}

#[test]
fn publication_rejects_vardef_and_checked_opcode_mismatches_before_allocation() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let reject = |function| {
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    };

    reject(
        UnlinkedFunction::fixture(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_fixture_definitions(Vec::new(), Vec::new()),
    );
    reject(
        UnlinkedFunction::fixture(
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
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("rejectedLexicalArgument")),
                false,
            )],
            Vec::new(),
        ),
    );
    reject(
        UnlinkedFunction::fixture(
            vec![Instruction::GetLocal(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_fixture_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("rejectedLexicalRead")),
                false,
            )],
        ),
    );
    reject(
        UnlinkedFunction::fixture(
            vec![Instruction::GetLocalCheck(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_fixture_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("rejectedOrdinaryRead"),
            ))],
        ),
    );
    reject(
        UnlinkedFunction::fixture(
            vec![
                Instruction::PushI32(1),
                Instruction::PutLocalCheck(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_fixture_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("rejectedConstWrite")),
                true,
            )],
        ),
    );

    let mismatched_child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRefCheck(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static(
                "differentLexicalName",
            )))
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
    reject(
        UnlinkedFunction::fixture(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(mismatched_child)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_fixture_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("expectedLexicalName")),
                false,
            )],
        ),
    );
}

#[test]
fn publication_rejects_out_of_range_frame_operands() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let function = UnlinkedFunction::fixture(
        vec![Instruction::GetArg(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );

    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);

    let function = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            local_count: u16::MAX,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);

    let function = UnlinkedFunction::fixture(
        vec![Instruction::DeleteVar(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);

    let function = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            function_name_local: Some(0),
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn publication_rejects_malformed_function_name_metadata_and_writes() {
    fn descriptor(
        source: ClosureSource,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> ClosureVariable {
        ClosureVariable {
            source,
            name: ClosureVariableName::None,
            is_lexical,
            is_const,
            kind,
        }
    }

    fn child(code: Vec<Instruction>, mut descriptor: ClosureVariable) -> UnlinkedFunction {
        let constants = if descriptor.kind == ClosureVariableKind::FunctionName {
            descriptor.name = ClosureVariableName::Constant(0);
            vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("self"))).unwrap()]
        } else {
            Vec::new()
        };
        UnlinkedFunction::fixture_with_closure_variables(
            code,
            constants,
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![descriptor],
        )
    }

    fn parent(
        child: UnlinkedFunction,
        metadata: FunctionMetadata,
        name: Option<&str>,
    ) -> UnlinkedFunction {
        let function = UnlinkedFunction::fixture(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            metadata,
        );
        function.with_name(name.map(|name| JsString::try_from_utf8(name).unwrap()))
    }

    fn named_parent(child: UnlinkedFunction, strict: bool) -> UnlinkedFunction {
        parent(
            child,
            FunctionMetadata {
                local_count: 1,
                function_name_local: Some(0),
                max_stack: 1,
                strict,
                ..FunctionMetadata::default()
            },
            Some("self"),
        )
    }

    let runtime = Runtime::new();
    let context = runtime.new_context();
    let reject = |function| {
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    };

    for name in [None, Some("")] {
        reject(
            UnlinkedFunction::fixture(
                vec![Instruction::GetLocal(0), Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    local_count: 1,
                    function_name_local: Some(0),
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            )
            .with_name(name.map(|name| JsString::try_from_utf8(name).unwrap())),
        );
    }

    reject(parent(
        child(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            descriptor(
                ClosureSource::ParentArgument(0),
                false,
                false,
                ClosureVariableKind::FunctionName,
            ),
        ),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        None,
    ));
    reject(parent(
        child(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            descriptor(
                ClosureSource::ParentLocal(0),
                false,
                false,
                ClosureVariableKind::FunctionName,
            ),
        ),
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        None,
    ));
    reject(named_parent(
        child(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            descriptor(
                ClosureSource::ParentLocal(0),
                false,
                false,
                ClosureVariableKind::Normal,
            ),
        ),
        false,
    ));
    reject(named_parent(
        UnlinkedFunction::fixture_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("other"))).unwrap(),
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
                kind: ClosureVariableKind::FunctionName,
            }],
        ),
        false,
    ));

    for (strict, is_lexical, is_const) in [
        (true, false, false),
        (false, false, true),
        (false, true, false),
    ] {
        reject(named_parent(
            child(
                vec![Instruction::GetVarRef(0), Instruction::Return],
                descriptor(
                    ClosureSource::ParentLocal(0),
                    is_lexical,
                    is_const,
                    ClosureVariableKind::FunctionName,
                ),
            ),
            strict,
        ));
    }

    let inner = child(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        descriptor(
            ClosureSource::ParentClosure(0),
            false,
            false,
            ClosureVariableKind::Normal,
        ),
    );
    let middle = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![
            UnlinkedConstant::child(inner),
            UnlinkedConstant::primitive(Value::String(JsString::from_static("self"))).unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(1),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::FunctionName,
        }],
    );
    reject(named_parent(middle, false));

    for code in [
        vec![
            Instruction::PushI32(1),
            Instruction::PutLocal(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::SetLocal(0),
            Instruction::Return,
        ],
    ] {
        reject(
            UnlinkedFunction::fixture(
                code,
                Vec::new(),
                FunctionMetadata {
                    local_count: 1,
                    function_name_local: Some(0),
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            )
            .with_name(Some(JsString::from_static("self"))),
        );
    }

    for code in [
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarRef(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::SetVarRef(0),
            Instruction::Return,
        ],
    ] {
        reject(named_parent(
            child(
                code,
                descriptor(
                    ClosureSource::ParentLocal(0),
                    false,
                    false,
                    ClosureVariableKind::FunctionName,
                ),
            ),
            false,
        ));
    }
}

#[test]
fn publication_rejects_mixed_global_and_lexical_closure_opcodes() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let reject = |function| {
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    };

    for code in [
        vec![Instruction::GetVar(0), Instruction::Return],
        vec![Instruction::GetVarUndef(0), Instruction::Return],
        vec![Instruction::DeleteVar(0), Instruction::Return],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVar(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarInit(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
    ] {
        let child = UnlinkedFunction::fixture_with_closure_variables(
            code,
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
        reject(UnlinkedFunction::fixture(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        ));
    }

    for code in [
        vec![Instruction::GetVarRef(0), Instruction::Return],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarRef(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::SetVarRef(0),
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarInit(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
    ] {
        reject(UnlinkedFunction::fixture_with_closure_variables(
            code,
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("global")))
                    .unwrap(),
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
        ));
    }
}

#[test]
fn publication_rejects_duplicate_lexical_global_declaration_descriptors() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let descriptor = ClosureVariable {
        source: ClosureSource::GlobalDeclaration,
        name: ClosureVariableName::Constant(0),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    let function = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::Undefined, Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("duplicate"))).unwrap(),
        ],
        FunctionMetadata {
            closure_count: 2,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![descriptor, descriptor],
    );

    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, function),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn publication_accepts_duplicate_program_var_declaration_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let descriptor = ClosureVariable {
        source: ClosureSource::GlobalDeclaration,
        name: ClosureVariableName::Constant(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    let function = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVar(0), Instruction::Return],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("duplicateVar")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 2,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![descriptor, descriptor],
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();

    assert_eq!(context.execute(&function).unwrap(), Value::Undefined);
    let key = runtime.intern_property_key("duplicateVar").unwrap();
    let global = context.global_object().unwrap();
    assert_eq!(
        context.get_own_property(&global, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: true,
            configurable: false,
        })
    );
}

#[test]
fn publication_accepts_annex_masked_mixed_global_declaration_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let ordinary = ClosureVariable {
        source: ClosureSource::GlobalDeclaration,
        name: ClosureVariableName::Constant(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    let lexical_let = ClosureVariable {
        is_lexical: true,
        ..ordinary
    };
    let lexical_const = ClosureVariable {
        is_const: true,
        ..lexical_let
    };
    let function = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::PushI32(7),
            Instruction::PutVarInit(0),
            Instruction::GetVar(0),
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("annexMaskedMixed")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 4,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ordinary, lexical_let, lexical_const, ordinary],
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();

    assert_eq!(context.execute(&function).unwrap(), Value::Int(7));
}

#[test]
fn publication_restricts_global_function_initializer_to_the_first_name_slot() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let ordinary = ClosureVariable {
        source: ClosureSource::GlobalDeclaration,
        name: ClosureVariableName::Constant(0),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    let global_function = ClosureVariable {
        kind: ClosureVariableKind::GlobalFunction,
        ..ordinary
    };
    let draft = |code| {
        let child = UnlinkedFunction::fixture(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_name(Some(JsString::from_static("functionSlot")));
        UnlinkedFunction::fixture_with_closure_variables(
            code,
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("functionSlot")))
                    .unwrap(),
                UnlinkedConstant::child(child),
            ],
            FunctionMetadata {
                closure_count: 3,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ordinary, ordinary, global_function],
        )
    };

    for code in [
        vec![
            Instruction::FClosure(1),
            Instruction::PutVarInit(1),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![
            Instruction::PushI32(1),
            Instruction::PutVarInit(0),
            Instruction::Undefined,
            Instruction::Return,
        ],
        vec![Instruction::FClosure(1), Instruction::Return],
    ] {
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, draft(code)),
            Err(RuntimeError::Engine(_))
        ));
    }
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    runtime
        .publish_unlinked_function(
            context.realm,
            draft(vec![
                Instruction::FClosure(1),
                Instruction::PutVarInit(0),
                Instruction::Undefined,
                Instruction::Return,
            ]),
        )
        .expect("the first same-name declaration slot should be a valid raw target");
}

#[test]
fn publication_preflights_closure_descriptors_before_heap_changes() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let missing_descriptor = UnlinkedFunction::fixture(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, missing_descriptor),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let root_parent_local = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            local_count: 1,
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
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, root_parent_local),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let out_of_bounds_argument_child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentArgument(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let parent = UnlinkedFunction::fixture(
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(out_of_bounds_argument_child)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, parent),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let out_of_bounds_child = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
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
        vec![Instruction::FClosure(0), Instruction::Return],
        vec![UnlinkedConstant::child(out_of_bounds_child)],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, parent),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn publication_rejects_inconsistent_closure_metadata() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let child = |is_lexical| {
        UnlinkedFunction::fixture_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: crate::engine::code::function::metadata::ClosureVariableName::None,
                is_lexical,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
    };
    let inconsistent_siblings = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        vec![
            UnlinkedConstant::child(child(false)),
            UnlinkedConstant::child(child(true)),
        ],
        FunctionMetadata {
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, inconsistent_siblings),
        Err(RuntimeError::Engine(_))
    ));

    let illegal_const = UnlinkedFunction::fixture_with_closure_variables(
        vec![Instruction::GetVarRef(0), Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            closure_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::ParentClosure(0),
            name: crate::engine::code::function::metadata::ClosureVariableName::None,
            is_lexical: false,
            is_const: true,
            kind: ClosureVariableKind::Normal,
        }],
    );
    assert!(matches!(
        runtime.publish_unlinked_function(context.realm, illegal_const),
        Err(RuntimeError::Engine(_))
    ));
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
}

#[test]
fn deeply_nested_child_publication_and_release_are_iterative() {
    const DEPTH: usize = 50_000;

    let runtime = Runtime::new();
    let context = runtime.new_context();
    let metadata = FunctionMetadata {
        max_stack: 1,
        ..FunctionMetadata::default()
    };
    let mut function = UnlinkedFunction::fixture(
        vec![Instruction::Undefined, Instruction::Return],
        Vec::new(),
        metadata,
    );
    for _ in 0..DEPTH {
        function = UnlinkedFunction::fixture(
            vec![Instruction::Undefined, Instruction::Return],
            vec![UnlinkedConstant::child(function)],
            metadata,
        );
    }

    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, DEPTH + 1);
    drop(function);
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    assert_eq!(runtime.heap_counts().context_nodes, 1);
}
