use super::*;
use crate::engine::code::function::metadata::{ParameterBodyStorage, ParameterDefaultSource};

#[test]
fn bytecode_allocation_rejects_eval_environments_across_parameter_phases() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();

    let value_name = Atom::from_raw(61);
    let scoped_name = Atom::from_raw(62);
    let argument_definitions: Rc<[VariableDefinition]> = Rc::from([VariableDefinition {
        name: Some(value_name),
        is_lexical: false,
        is_const: false,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::Normal,
    }]);
    let parameter_layout = |initialization_end| ParameterEnvironmentLayout {
        initialization_end,
        argument_cells: vec![ParameterArgumentCell {
            argument: 0,
            parameter_local: 0,
            body: ParameterBodyStorage::Argument(0),
        }]
        .into_boxed_slice(),
        pattern_copies: Box::new([]),
        default_sources: vec![ParameterDefaultSource::Argument(0)].into_boxed_slice(),
        synthetic_arguments_local: None,
        arg_eval_variable_object_local: None,
    };
    let binding = || EvalBinding {
        name: scoped_name,
        source: EvalBindingSource::Local(1),
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
        is_catch_parameter: false,
    };

    let body_eval_code: Rc<[Instruction]> = Rc::from([
        Instruction::SetLocalUninitialized(0),
        Instruction::GetArg(0),
        Instruction::Dup,
        Instruction::Undefined,
        Instruction::StrictEq,
        Instruction::IfFalse(14),
        Instruction::Drop,
        Instruction::SetLocalUninitialized(1),
        Instruction::Undefined,
        Instruction::InitializeLocal(1),
        Instruction::CloseLocal(1),
        Instruction::Undefined,
        Instruction::Dup,
        Instruction::PutArg(0),
        Instruction::InitializeLocal(0),
        Instruction::Nop,
        Instruction::Undefined,
        Instruction::Eval {
            argument_count: 0,
            environment: 0,
        },
        Instruction::Return,
    ]);
    let mut body_eval = bytecode(
        &body_eval_code,
        context,
        Vec::new(),
        vec![value_name, value_name, scoped_name, scoped_name],
    );
    body_eval.metadata = FunctionMetadata {
        argument_count: 1,
        defined_argument_count: 0,
        parameter_environment_local_count: 1,
        local_count: 2,
        max_stack: 3,
        strict: true,
        ..FunctionMetadata::default()
    };
    body_eval.parameter_environment = Some(parameter_layout(15));
    body_eval.argument_definitions = argument_definitions.clone();
    body_eval.local_definitions = Rc::from([
        VariableDefinition {
            name: Some(value_name),
            is_lexical: true,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        },
        VariableDefinition {
            name: Some(scoped_name),
            is_lexical: true,
            is_const: false,
            is_parameter_initializer: true,
            kind: ClosureVariableKind::Normal,
        },
    ]);
    body_eval.eval_environments = Rc::from([EvalEnvironment {
        scopes: vec![
            EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: vec![binding()].into_boxed_slice(),
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
    }]);
    assert_eq!(
        heap.allocate_function_bytecode(body_eval),
        Err(HeapError::Invariant(
            "function body eval captured a parameter-initializer local"
        ))
    );

    let initializer_eval_code: Rc<[Instruction]> = Rc::from([
        Instruction::SetLocalUninitialized(0),
        Instruction::GetArg(0),
        Instruction::Dup,
        Instruction::Undefined,
        Instruction::StrictEq,
        Instruction::IfFalse(12),
        Instruction::Drop,
        Instruction::Undefined,
        Instruction::Undefined,
        Instruction::ApplyEval { environment: 0 },
        Instruction::Dup,
        Instruction::PutArg(0),
        Instruction::InitializeLocal(0),
        Instruction::Nop,
        Instruction::Undefined,
        Instruction::Return,
    ]);
    let mut initializer_eval = bytecode(
        &initializer_eval_code,
        context,
        Vec::new(),
        vec![value_name, value_name, scoped_name, scoped_name],
    );
    initializer_eval.metadata = FunctionMetadata {
        argument_count: 1,
        defined_argument_count: 0,
        parameter_environment_local_count: 1,
        local_count: 2,
        max_stack: 3,
        strict: true,
        ..FunctionMetadata::default()
    };
    initializer_eval.parameter_environment = Some(parameter_layout(13));
    initializer_eval.argument_definitions = argument_definitions;
    initializer_eval.local_definitions = Rc::from([
        VariableDefinition {
            name: Some(value_name),
            is_lexical: true,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        },
        VariableDefinition {
            name: Some(scoped_name),
            is_lexical: true,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        },
    ]);
    initializer_eval.eval_environments = Rc::from([EvalEnvironment {
        scopes: vec![EvalScope {
            kind: EvalScopeKind::Parameter,
            bindings: vec![binding()].into_boxed_slice(),
        }]
        .into_boxed_slice(),
        variable_environment: EvalVariableEnvironment::StrictLocal(0),
        caller_strict: true,
        super_call_allowed: false,
        super_allowed: false,
    }]);
    assert_eq!(
        heap.allocate_function_bytecode(initializer_eval),
        Err(HeapError::Invariant(
            "parameter initializer eval captured a body-only local"
        ))
    );

    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn eval_variable_object_local_requires_exact_metadata_authentication() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();

    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let name = Atom::from_raw(43);
    let make_bytecode = |metadata_slot, definition_kind| {
        let mut bytecode = bytecode(&code, context, Vec::new(), vec![name]);
        bytecode.metadata.local_count = 1;
        bytecode.metadata.eval_variable_object_local = metadata_slot;
        bytecode.local_definitions = Rc::from([VariableDefinition {
            name: Some(name),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: definition_kind,
        }]);
        bytecode
    };

    assert_eq!(
        heap.allocate_function_bytecode(make_bytecode(Some(0), ClosureVariableKind::Normal)),
        Err(HeapError::Invariant(
            "eval variable-object definition disagrees with bytecode metadata"
        ))
    );
    assert_eq!(
        heap.allocate_function_bytecode(make_bytecode(
            None,
            ClosureVariableKind::EvalVariableObject
        )),
        Err(HeapError::Invariant(
            "ordinary local definition uses a non-local binding kind"
        ))
    );

    let published = heap
        .allocate_function_bytecode(make_bytecode(
            Some(0),
            ClosureVariableKind::EvalVariableObject,
        ))
        .unwrap();
    assert_eq!(
        heap.release_function_bytecode(published).unwrap().atoms,
        vec![name]
    );
    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
}

#[test]
fn strict_script_global_eval_anchor_is_fail_closed_at_heap_boundary() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();

    let code: Rc<[Instruction]> = Rc::from([
        Instruction::Undefined,
        Instruction::Eval {
            argument_count: 0,
            environment: 0,
        },
        Instruction::Return,
    ]);
    let make_bytecode = |scopes: Vec<EvalScope<Atom>>, eval_kind, variable_environment| {
        let mut bytecode = bytecode(&code, context, Vec::new(), Vec::new());
        bytecode.metadata.strict = true;
        bytecode.metadata.eval_kind = eval_kind;
        bytecode.eval_environments = Rc::from([EvalEnvironment {
            scopes: scopes.into_boxed_slice(),
            variable_environment,
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        }]);
        bytecode
    };
    let script_scopes = || {
        vec![
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
    };

    let published = heap
        .allocate_function_bytecode(make_bytecode(
            script_scopes(),
            EvalKind::None,
            EvalVariableEnvironment::Global,
        ))
        .unwrap();
    heap.release_function_bytecode(published).unwrap();

    assert_eq!(
        heap.allocate_function_bytecode(make_bytecode(
            script_scopes(),
            EvalKind::None,
            EvalVariableEnvironment::StrictLocal(1),
        )),
        Err(HeapError::Invariant(
            "authored Script eval environment used a non-canonical strict-local target"
        ))
    );
    let synthetic = heap
        .allocate_function_bytecode(make_bytecode(
            script_scopes(),
            EvalKind::Direct,
            EvalVariableEnvironment::StrictLocal(1),
        ))
        .unwrap();
    heap.release_function_bytecode(synthetic).unwrap();

    let function_scopes = vec![
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
    ];
    assert_eq!(
        heap.allocate_function_bytecode(make_bytecode(
            function_scopes,
            EvalKind::None,
            EvalVariableEnvironment::Global,
        )),
        Err(HeapError::Invariant(
            "global eval variable environment escaped an authored Script root"
        ))
    );
    assert_eq!(
        heap.allocate_function_bytecode(make_bytecode(
            script_scopes(),
            EvalKind::Direct,
            EvalVariableEnvironment::Global,
        )),
        Err(HeapError::Invariant(
            "global eval variable environment escaped an authored Script root"
        ))
    );

    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
}

#[test]
fn with_object_metadata_is_fail_closed_at_the_heap_boundary() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();

    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let eval_code: Rc<[Instruction]> = Rc::from([
        Instruction::Undefined,
        Instruction::Eval {
            argument_count: 0,
            environment: 0,
        },
        Instruction::Return,
    ]);
    let name = Atom::from_raw(45);
    let mut local = bytecode(&code, context, Vec::new(), vec![name]);
    local.metadata.local_count = 1;
    local.local_definitions = Rc::from([VariableDefinition {
        name: Some(name),
        is_lexical: false,
        is_const: false,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::WithObject,
    }]);
    let published = heap.allocate_function_bytecode(local).unwrap();
    assert_eq!(
        heap.release_function_bytecode(published).unwrap().atoms,
        vec![name]
    );

    let mut lexical = bytecode(&code, context, Vec::new(), vec![name]);
    lexical.metadata.local_count = 1;
    lexical.local_definitions = Rc::from([VariableDefinition {
        name: Some(name),
        is_lexical: true,
        is_const: false,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::WithObject,
    }]);
    assert_eq!(
        heap.allocate_function_bytecode(lexical),
        Err(HeapError::Invariant(
            "strict or malformed bytecode contains a with-object local"
        ))
    );

    let mut strict = bytecode(&code, context, Vec::new(), vec![name]);
    strict.metadata.local_count = 1;
    strict.metadata.strict = true;
    strict.local_definitions = Rc::from([VariableDefinition {
        name: Some(name),
        is_lexical: false,
        is_const: false,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::WithObject,
    }]);
    assert_eq!(
        heap.allocate_function_bytecode(strict),
        Err(HeapError::Invariant(
            "strict or malformed bytecode contains a with-object local"
        ))
    );

    let mut argument_source = bytecode(&code, context, Vec::new(), vec![name]);
    argument_source.metadata.argument_count = 1;
    argument_source.metadata.defined_argument_count = 1;
    argument_source.argument_definitions = Rc::from([VariableDefinition {
        name: Some(name),
        is_lexical: false,
        is_const: false,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::Normal,
    }]);
    argument_source.code = eval_code.clone();
    argument_source.eval_environments = Rc::from([EvalEnvironment {
        scopes: Box::new([
            EvalScope {
                kind: EvalScopeKind::With,
                bindings: Box::new([EvalBinding {
                    name,
                    source: EvalBindingSource::Argument(0),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::WithObject,
                    is_catch_parameter: false,
                }]),
            },
            EvalScope {
                kind: EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]),
        variable_environment: EvalVariableEnvironment::Global,
        caller_strict: false,
        super_call_allowed: false,
        super_allowed: false,
    }]);
    assert_eq!(
        heap.allocate_function_bytecode(argument_source),
        Err(HeapError::Invariant(
            "eval with-object binding metadata disagrees with its scope"
        ))
    );

    let other_name = Atom::from_raw(46);
    let with_scope = |source| {
        Rc::from([EvalEnvironment {
            scopes: Box::new([
                EvalScope {
                    kind: EvalScopeKind::With,
                    bindings: Box::new([EvalBinding {
                        name: other_name,
                        source,
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::WithObject,
                        is_catch_parameter: false,
                    }]),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]),
            variable_environment: EvalVariableEnvironment::Global,
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        }])
    };

    let mut local_name_mismatch = bytecode(&code, context, Vec::new(), vec![name, other_name]);
    local_name_mismatch.metadata.local_count = 1;
    local_name_mismatch.local_definitions = Rc::from([VariableDefinition {
        name: Some(name),
        is_lexical: false,
        is_const: false,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::WithObject,
    }]);
    local_name_mismatch.code = eval_code.clone();
    local_name_mismatch.eval_environments = with_scope(EvalBindingSource::Local(0));
    assert_eq!(
        heap.allocate_function_bytecode(local_name_mismatch),
        Err(HeapError::Invariant(
            "eval binding name atom disagrees with its source metadata"
        ))
    );

    let mut closure_name_mismatch = bytecode(&code, context, Vec::new(), vec![name, other_name]);
    closure_name_mismatch.metadata.closure_count = 1;
    closure_name_mismatch.closure_variables = Rc::from([ClosureVariable {
        source: ClosureSource::ParentLocal(0),
        name: ClosureVariableName::Atom(name),
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::WithObject,
    }]);
    closure_name_mismatch.code = eval_code;
    closure_name_mismatch.eval_environments = with_scope(EvalBindingSource::Closure(0));
    assert_eq!(
        heap.allocate_function_bytecode(closure_name_mismatch),
        Err(HeapError::Invariant(
            "eval binding name atom disagrees with its source metadata"
        ))
    );

    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
}

#[test]
fn bytecode_allocation_requires_published_closure_name_atom_ownership() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();
    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let name = Atom::from_raw(47);

    let descriptor = |name| ClosureVariable {
        source: ClosureSource::Global,
        name,
        is_lexical: false,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    for (name_kind, atoms) in [
        (ClosureVariableName::Constant(0), Vec::new()),
        (ClosureVariableName::Atom(name), Vec::new()),
    ] {
        let mut malformed = bytecode(&code, context, Vec::new(), atoms);
        malformed.metadata.closure_count = 1;
        malformed.closure_variables = vec![descriptor(name_kind)].into();
        assert!(matches!(
            heap.allocate_function_bytecode(malformed),
            Err(HeapError::Invariant(_))
        ));
    }

    let mut non_lexical_const = bytecode(&code, context, Vec::new(), vec![name]);
    non_lexical_const.metadata.closure_count = 1;
    non_lexical_const.closure_variables = vec![ClosureVariable {
        source: ClosureSource::GlobalDeclaration,
        name: ClosureVariableName::Atom(name),
        is_lexical: false,
        is_const: true,
        kind: ClosureVariableKind::Normal,
    }]
    .into();
    assert_eq!(
        heap.allocate_function_bytecode(non_lexical_const),
        Err(HeapError::Invariant(
            "a const closure descriptor must also be lexical"
        ))
    );

    let mut published = bytecode(&code, context, Vec::new(), vec![name]);
    published.metadata.closure_count = 1;
    published.closure_variables = vec![descriptor(ClosureVariableName::Atom(name))].into();
    let published = heap.allocate_function_bytecode(published).unwrap();
    assert_eq!(
        heap.release_function_bytecode(published).unwrap().atoms,
        vec![name]
    );
    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
}

#[test]
fn eval_environment_closure_is_confined_to_direct_eval_root() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();

    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let name = Atom::from_raw(59);
    let make_bytecode = |kind| {
        let mut bytecode = bytecode(&code, context, Vec::new(), vec![name]);
        bytecode.metadata.eval_kind = kind;
        bytecode.metadata.closure_count = 1;
        bytecode.closure_variables = Rc::from([ClosureVariable {
            source: ClosureSource::EvalEnvironment(0),
            name: ClosureVariableName::Atom(name),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }]);
        bytecode
    };

    for kind in [EvalKind::None, EvalKind::Indirect] {
        assert_eq!(
            heap.allocate_function_bytecode(make_bytecode(kind)),
            Err(HeapError::Invariant(
                "eval-environment closure escaped a direct-eval root"
            ))
        );
    }
    let direct = heap
        .allocate_function_bytecode(make_bytecode(EvalKind::Direct))
        .unwrap();
    assert_eq!(
        heap.release_function_bytecode(direct).unwrap().atoms,
        vec![name]
    );
    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
}

#[test]
fn bytecode_allocation_requires_one_atom_reference_per_eval_binding_name() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();

    let code: Rc<[Instruction]> = Rc::from([
        Instruction::Undefined,
        Instruction::Eval {
            argument_count: 0,
            environment: 0,
        },
        Instruction::Return,
    ]);
    let name = Atom::from_raw(53);
    let make_bytecode = |owned_names: Vec<Atom>| {
        let mut bytecode = bytecode(&code, context, Vec::new(), owned_names);
        bytecode.metadata.local_count = 1;
        bytecode.local_definitions = Rc::from([VariableDefinition {
            name: Some(name),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }]);
        bytecode.eval_environments = Rc::from([EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: vec![EvalBinding {
                        name,
                        source: EvalBindingSource::Local(0),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                        is_catch_parameter: false,
                    }]
                    .into_boxed_slice(),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Global,
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        }]);
        bytecode
    };

    assert_eq!(
        heap.allocate_function_bytecode(make_bytecode(vec![name])),
        Err(HeapError::Invariant(
            "eval binding name atom ownership multiplicity is too small"
        ))
    );
    let published = heap
        .allocate_function_bytecode(make_bytecode(vec![name, name]))
        .unwrap();
    assert_eq!(
        heap.release_function_bytecode(published).unwrap().atoms,
        vec![name, name]
    );
    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
}
