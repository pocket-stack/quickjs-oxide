use super::*;

fn allocate_private_callable_child(
    heap: &mut Heap,
    realm: ContextId,
    argument_count: u16,
    valid_home_object_metadata: bool,
    func_name: Option<JsString>,
    function_kind: FunctionKind,
    has_prototype: bool,
) -> FunctionBytecodeId {
    let code: Rc<[Instruction]> = if matches!(
        function_kind,
        FunctionKind::Generator | FunctionKind::AsyncGenerator
    ) {
        Rc::from([
            Instruction::InitialYield,
            Instruction::Undefined,
            Instruction::Return,
        ])
    } else {
        Rc::from([Instruction::Undefined, Instruction::Return])
    };
    let mut child = bytecode(&code, realm, Vec::new(), Vec::new());
    child.metadata.argument_count = argument_count;
    child.metadata.defined_argument_count = argument_count;
    child.metadata.strict = true;
    child.metadata.needs_home_object = valid_home_object_metadata;
    child.metadata.function_kind = function_kind;
    child.metadata.has_prototype = has_prototype;
    child.func_name = func_name;
    child.argument_definitions = (0..argument_count)
        .map(|_| VariableDefinition {
            name: None,
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        })
        .collect::<Vec<_>>()
        .into();
    heap.allocate_function_bytecode(child).unwrap()
}

fn allocate_private_accessor_child(
    heap: &mut Heap,
    realm: ContextId,
    argument_count: u16,
    valid_home_object_metadata: bool,
    func_name: Option<JsString>,
) -> FunctionBytecodeId {
    allocate_private_callable_child(
        heap,
        realm,
        argument_count,
        valid_home_object_metadata,
        func_name,
        FunctionKind::Normal,
        false,
    )
}

fn allocate_private_brand_child(heap: &mut Heap, realm: ContextId) -> FunctionBytecodeId {
    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let mut child = bytecode(&code, realm, Vec::new(), Vec::new());
    child.metadata.strict = true;
    child.metadata.super_allowed = true;
    child.metadata.arguments_forbidden = true;
    child.metadata.needs_home_object = true;
    child.metadata.class_initializer_kind = Some(ClassInitializerKind::InstanceFields);
    child.metadata.class_private_brand = true;
    heap.allocate_function_bytecode(child).unwrap()
}

#[derive(Clone, Copy)]
enum LinkedPrivateAccessorShape {
    Getter,
    Setter,
    Pair,
}

fn linked_private_accessor_bytecode(
    heap: &mut Heap,
    realm: ContextId,
    shape: LinkedPrivateAccessorShape,
) -> FunctionBytecodeData {
    let primary_name = Atom::from_raw(601);
    let setter_name = Atom::from_raw(602);
    let callable_arguments: &[u16] = match shape {
        LinkedPrivateAccessorShape::Getter => &[0],
        LinkedPrivateAccessorShape::Setter => &[1],
        LinkedPrivateAccessorShape::Pair => &[0, 1],
    };
    let mut constants = callable_arguments
        .iter()
        .map(|argument_count| {
            BytecodeConstant::Function(allocate_private_accessor_child(
                heap,
                realm,
                *argument_count,
                true,
                None,
            ))
        })
        .collect::<Vec<_>>();
    constants.push(BytecodeConstant::Function(allocate_private_brand_child(
        heap, realm,
    )));

    let (definitions, roles, code, names) = match shape {
        LinkedPrivateAccessorShape::Getter => (
            vec![VariableDefinition {
                name: Some(primary_name),
                is_lexical: true,
                is_const: true,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::PrivateGetter,
            }],
            vec![Some(PublishedPrivateBinding::primary(primary_name, None))],
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::Undefined,
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
                Instruction::Drop,
                Instruction::CloseLocal(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![primary_name],
        ),
        LinkedPrivateAccessorShape::Setter => (
            vec![
                VariableDefinition {
                    name: Some(primary_name),
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateSetter,
                },
                VariableDefinition {
                    name: Some(setter_name),
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateSetter,
                },
            ],
            vec![
                Some(PublishedPrivateBinding::primary(primary_name, Some(1))),
                Some(PublishedPrivateBinding::setter_storage(
                    setter_name,
                    Some(0),
                )),
            ],
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::SetLocalUninitialized(1),
                Instruction::Undefined,
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(1),
                Instruction::Drop,
                Instruction::CloseLocal(1),
                Instruction::CloseLocal(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![primary_name, setter_name],
        ),
        LinkedPrivateAccessorShape::Pair => (
            vec![
                VariableDefinition {
                    name: Some(primary_name),
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateGetterSetter,
                },
                VariableDefinition {
                    name: Some(setter_name),
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateSetter,
                },
            ],
            vec![
                Some(PublishedPrivateBinding::primary(primary_name, Some(1))),
                Some(PublishedPrivateBinding::setter_storage(
                    setter_name,
                    Some(0),
                )),
            ],
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::SetLocalUninitialized(1),
                Instruction::Undefined,
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::FClosure(1),
                Instruction::InitializePrivateAccessor(1),
                Instruction::Drop,
                Instruction::CloseLocal(1),
                Instruction::CloseLocal(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![primary_name, setter_name],
        ),
    };
    let mut parent = bytecode(&Rc::from(code), realm, constants, names);
    parent.metadata.local_count = u16::try_from(definitions.len()).unwrap();
    parent.metadata.max_stack = 2;
    parent.local_definitions = definitions.into();
    parent.private_bindings = PublishedPrivateBindings::authenticated(roles, Vec::new());
    parent
}

fn linked_private_method_bytecode(
    heap: &mut Heap,
    realm: ContextId,
    function_kind: FunctionKind,
    has_prototype: bool,
) -> FunctionBytecodeData {
    let name = Atom::from_raw(604);
    let method =
        allocate_private_callable_child(heap, realm, 0, true, None, function_kind, has_prototype);
    let brand = allocate_private_brand_child(heap, realm);
    let code: Rc<[Instruction]> = Rc::from([
        Instruction::SetLocalUninitialized(0),
        Instruction::Undefined,
        Instruction::FClosure(0),
        Instruction::InitializePrivateMethod(0),
        Instruction::Drop,
        Instruction::CloseLocal(0),
        Instruction::Undefined,
        Instruction::Return,
    ]);
    let mut parent = bytecode(
        &code,
        realm,
        vec![
            BytecodeConstant::Function(method),
            BytecodeConstant::Function(brand),
        ],
        vec![name],
    );
    parent.metadata.local_count = 1;
    parent.metadata.max_stack = 2;
    parent.local_definitions = Rc::from([VariableDefinition {
        name: Some(name),
        is_lexical: true,
        is_const: true,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::PrivateMethod,
    }]);
    parent.private_bindings = PublishedPrivateBindings::authenticated(
        vec![Some(PublishedPrivateBinding::primary(name, None))],
        Vec::new(),
    );
    parent
}

#[test]
fn linked_private_methods_accept_async_and_generator_callable_shapes() {
    for (function_kind, has_prototype) in [
        (FunctionKind::Normal, false),
        (FunctionKind::Async, false),
        (FunctionKind::Generator, true),
        (FunctionKind::AsyncGenerator, true),
    ] {
        let mut heap = Heap::new();
        let realm = bytecode_test_realm(&mut heap);
        let candidate =
            linked_private_method_bytecode(&mut heap, realm, function_kind, has_prototype);
        assert!(
            heap.allocate_function_bytecode(candidate).is_ok(),
            "{function_kind:?}/{has_prototype}"
        );
    }
}

#[test]
fn linked_private_callables_reject_cross_role_execution_shapes() {
    let mut heap = Heap::new();
    let realm = bytecode_test_realm(&mut heap);
    let candidate = linked_private_method_bytecode(&mut heap, realm, FunctionKind::Normal, true);
    assert_eq!(
        heap.allocate_function_bytecode(candidate),
        Err(HeapError::Invariant(
            "private-method child has invalid HomeObject metadata"
        ))
    );

    for function_kind in [FunctionKind::Generator, FunctionKind::AsyncGenerator] {
        let mut heap = Heap::new();
        let realm = bytecode_test_realm(&mut heap);
        let callable =
            allocate_private_callable_child(&mut heap, realm, 0, true, None, function_kind, true);
        let mut getter =
            linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
        getter.constants = Rc::from([
            BytecodeConstant::Function(callable),
            getter.constants[1].clone(),
        ]);
        assert_eq!(
            heap.allocate_function_bytecode(getter),
            Err(HeapError::Invariant(
                "private-accessor child has invalid HomeObject metadata"
            )),
            "{function_kind:?}"
        );
    }
}

#[test]
fn linked_private_accessors_accept_only_sealed_legal_lifecycles() {
    let mut heap = Heap::new();
    let realm = bytecode_test_realm(&mut heap);

    for shape in [
        LinkedPrivateAccessorShape::Getter,
        LinkedPrivateAccessorShape::Setter,
        LinkedPrivateAccessorShape::Pair,
    ] {
        let candidate = linked_private_accessor_bytecode(&mut heap, realm, shape);
        assert!(heap.allocate_function_bytecode(candidate).is_ok());
    }

    let mut unsealed =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
    unsealed.private_bindings = PublishedPrivateBindings::none();
    assert_eq!(
        heap.allocate_function_bytecode(unsealed),
        Err(HeapError::Invariant(
            "published private bindings are missing sealed role metadata"
        ))
    );
}

#[test]
fn linked_private_accessors_reject_forged_setter_capability_uses() {
    let mut heap = Heap::new();
    let realm = bytecode_test_realm(&mut heap);

    let mut initialized_primary =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Setter);
    initialized_primary.code = initialized_primary
        .code
        .iter()
        .map(|instruction| match instruction {
            Instruction::InitializePrivateAccessor(1) => Instruction::InitializePrivateAccessor(0),
            instruction => instruction.clone(),
        })
        .collect::<Vec<_>>()
        .into();
    assert_eq!(
        heap.allocate_function_bytecode(initialized_primary),
        Err(HeapError::Invariant(
            "private-accessor initializer referenced an incompatible binding"
        ))
    );

    let mut primary_put =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Setter);
    let mut code = primary_put.code.to_vec();
    code.insert(
        code.len() - 2,
        Instruction::PutPrivateField(PrivateNameSource::Local(0)),
    );
    primary_put.code = code.into();
    assert_eq!(
        heap.allocate_function_bytecode(primary_put),
        Err(HeapError::Invariant(
            "private put referenced an incompatible binding"
        ))
    );

    let mut synthetic_in =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Setter);
    let mut code = synthetic_in.code.to_vec();
    code.insert(
        code.len() - 2,
        Instruction::PrivateIn(PrivateNameSource::Local(1)),
    );
    synthetic_in.code = code.into();
    assert_eq!(
        heap.allocate_function_bytecode(synthetic_in),
        Err(HeapError::Invariant(
            "private-in referenced a synthetic setter binding"
        ))
    );

    let primary_name = Atom::from_raw(603);
    let code: Rc<[Instruction]> = Rc::from([
        Instruction::SetLocalUninitialized(0),
        Instruction::CloseLocal(0),
        Instruction::Undefined,
        Instruction::Return,
    ]);
    let mut unpaired = bytecode(&code, realm, Vec::new(), vec![primary_name]);
    unpaired.metadata.local_count = 1;
    unpaired.local_definitions = Rc::from([VariableDefinition {
        name: Some(primary_name),
        is_lexical: true,
        is_const: true,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::PrivateSetter,
    }]);
    unpaired.private_bindings = PublishedPrivateBindings::authenticated(
        vec![Some(PublishedPrivateBinding::primary(primary_name, None))],
        Vec::new(),
    );
    assert_eq!(
        heap.allocate_function_bytecode(unpaired),
        Err(HeapError::Invariant(
            "published private setter has malformed pair metadata"
        ))
    );
}

#[test]
fn linked_private_accessor_initializer_authenticates_its_unique_child_edge() {
    let mut heap = Heap::new();
    let realm = bytecode_test_realm(&mut heap);

    let mut missing_closure =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
    missing_closure.code = missing_closure
        .code
        .iter()
        .map(|instruction| match instruction {
            Instruction::FClosure(0) => Instruction::Undefined,
            instruction => instruction.clone(),
        })
        .collect::<Vec<_>>()
        .into();
    assert_eq!(
        heap.allocate_function_bytecode(missing_closure),
        Err(HeapError::Invariant(
            "private-accessor initializer did not consume an adjacent closure"
        ))
    );

    let mut invalid_child =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
    let invalid = allocate_private_accessor_child(&mut heap, realm, 0, false, None);
    invalid_child.constants = Rc::from([
        BytecodeConstant::Function(invalid),
        invalid_child.constants[1].clone(),
    ]);
    assert_eq!(
        heap.allocate_function_bytecode(invalid_child),
        Err(HeapError::Invariant(
            "private-accessor child has invalid HomeObject metadata"
        ))
    );

    let mut duplicate_site =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
    let mut code = duplicate_site.code.to_vec();
    code.splice(1..1, [Instruction::FClosure(0), Instruction::Drop]);
    duplicate_site.code = code.into();
    assert_eq!(
        heap.allocate_function_bytecode(duplicate_site),
        Err(HeapError::Invariant(
            "private-accessor child did not have one unique closure site"
        ))
    );

    let mut non_fallthrough =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
    let mut code = non_fallthrough.code.to_vec();
    code.insert(1, Instruction::Goto(3));
    non_fallthrough.code = code.into();
    assert_eq!(
        heap.allocate_function_bytecode(non_fallthrough),
        Err(HeapError::Invariant(
            "private-accessor closure/initializer pair has a non-fallthrough entry"
        ))
    );

    let mut repeated_lifetime =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
    let mut code = repeated_lifetime.code.to_vec();
    code.insert(code.len() - 2, Instruction::Goto(1));
    repeated_lifetime.code = code.into();
    assert_eq!(
        heap.allocate_function_bytecode(repeated_lifetime),
        Err(HeapError::Invariant(
            "private-accessor initializer is reachable by a repeated-lifetime backedge"
        ))
    );
}

#[test]
fn linked_private_accessor_initializer_authenticates_role_arity_and_empty_name() {
    let mut heap = Heap::new();
    let realm = bytecode_test_realm(&mut heap);

    let mut zero_argument_setter =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Setter);
    let forged_setter = allocate_private_accessor_child(&mut heap, realm, 0, true, None);
    zero_argument_setter.constants = Rc::from([
        BytecodeConstant::Function(forged_setter),
        zero_argument_setter.constants[1].clone(),
    ]);
    assert_eq!(
        heap.allocate_function_bytecode(zero_argument_setter),
        Err(HeapError::Invariant(
            "private-accessor child has invalid authored arity"
        ))
    );

    let mut one_argument_getter =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
    let forged_getter = allocate_private_accessor_child(&mut heap, realm, 1, true, None);
    one_argument_getter.constants = Rc::from([
        BytecodeConstant::Function(forged_getter),
        one_argument_getter.constants[1].clone(),
    ]);
    assert_eq!(
        heap.allocate_function_bytecode(one_argument_getter),
        Err(HeapError::Invariant(
            "private-accessor child has invalid authored arity"
        ))
    );

    let mut named_getter =
        linked_private_accessor_bytecode(&mut heap, realm, LinkedPrivateAccessorShape::Getter);
    let forged_name = allocate_private_accessor_child(
        &mut heap,
        realm,
        0,
        true,
        Some(JsString::from_static("#value")),
    );
    named_getter.constants = Rc::from([
        BytecodeConstant::Function(forged_name),
        named_getter.constants[1].clone(),
    ]);
    assert_eq!(
        heap.allocate_function_bytecode(named_getter),
        Err(HeapError::Invariant(
            "private-accessor child retained a non-empty intrinsic name"
        ))
    );
}
