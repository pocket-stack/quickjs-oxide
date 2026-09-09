use super::*;
use crate::engine::code::bytecode_validation::quickjs_copies_defined_argument_count;

#[test]
fn bytecode_debug_filename_requires_one_auxiliary_atom_ownership() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    let realm = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let filename = Atom::from_raw(19);
    let mut missing = bytecode(&code, realm, Vec::new(), Vec::new());
    missing.metadata.max_stack = 1;
    missing.debug = Some(FunctionDebugInfo {
        filename,
        pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
        source: None,
    });
    assert_eq!(
        heap.allocate_function_bytecode(missing),
        Err(HeapError::Invariant(
            "debug filename atom is not owned by bytecode metadata"
        ))
    );

    let mut owned = bytecode(&code, realm, Vec::new(), vec![filename]);
    owned.metadata.max_stack = 1;
    owned.debug = Some(FunctionDebugInfo {
        filename,
        pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
        source: None,
    });
    let bytecode = heap.allocate_function_bytecode(owned).unwrap();
    assert_eq!(
        heap.release_function_bytecode(bytecode).unwrap().atoms,
        vec![filename]
    );
}

#[test]
fn bytecode_allocation_rejects_mismatched_closure_descriptor_count() {
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
    let mut too_many_locals = bytecode(&code, context, Vec::new(), Vec::new());
    too_many_locals.metadata.local_count = u16::MAX;
    assert_eq!(
        heap.allocate_function_bytecode(too_many_locals),
        Err(HeapError::Invariant(
            "bytecode local count exceeds QuickJS JS_MAX_LOCAL_VARS"
        ))
    );

    let mut malformed = bytecode(&code, context, Vec::new(), Vec::new());
    malformed.metadata.closure_count = 1;
    assert!(matches!(
        heap.allocate_function_bytecode(malformed),
        Err(HeapError::Invariant(_))
    ));

    let mut lexical_argument = bytecode(&code, context, Vec::new(), Vec::new());
    lexical_argument.metadata.argument_count = 1;
    lexical_argument.metadata.defined_argument_count = 1;
    lexical_argument.argument_definitions = Rc::from([VariableDefinition {
        name: None,
        is_lexical: true,
        is_const: false,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::Normal,
    }]);
    assert_eq!(
        heap.allocate_function_bytecode(lexical_argument),
        Err(HeapError::Invariant(
            "argument definition is not an ordinary mutable binding"
        ))
    );
    assert_eq!(heap.counts().function_bytecode_nodes, 0);

    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn active_function_counts_as_a_quickjs_hidden_variable() {
    assert!(!quickjs_copies_defined_argument_count(0, 0, &[]));
    assert!(quickjs_copies_defined_argument_count(
        0,
        0,
        &[Instruction::PushActiveFunction],
    ));
}

#[test]
fn parameter_pseudo_prologue_accepts_active_function_in_fixed_order() {
    let metadata = FunctionMetadata {
        argument_count: 1,
        defined_argument_count: 0,
        rest_parameter: Some(0),
        local_count: 4,
        ..FunctionMetadata::default()
    };
    let ordered = vec![
        Instruction::PushHomeObject,
        Instruction::PutLocal(0),
        Instruction::PushActiveFunction,
        Instruction::PutLocal(1),
        Instruction::PushNewTarget,
        Instruction::PutLocal(2),
        Instruction::PushThis,
        Instruction::PutLocal(3),
        Instruction::Rest(0),
        Instruction::PutArg(0),
        Instruction::Undefined,
        Instruction::Return,
    ];
    assert_eq!(
        validate_parameter_bytecode_layout(&metadata, &ordered, &[false; 4], None),
        Ok(None),
    );

    let mut out_of_order = ordered;
    out_of_order.swap(2, 4);
    out_of_order.swap(3, 5);
    assert_eq!(
        validate_parameter_bytecode_layout(&metadata, &out_of_order, &[false; 4], None),
        Err("rest parameter contains a malformed pseudo-binding prologue"),
    );
}

#[test]
fn parameter_initializers_preserve_derived_this_authority() {
    let metadata = FunctionMetadata {
        argument_count: 1,
        defined_argument_count: 1,
        pattern_argument_count: 1,
        parameter_pattern_end: Some(6),
        local_count: 2,
        derived_this_local: Some(0),
        active_function_local: Some(1),
        constructor_kind: ConstructorKind::Derived,
        strict: true,
        super_call_allowed: true,
        super_allowed: true,
        ..FunctionMetadata::default()
    };
    let code = [
        Instruction::GetArg(0),
        Instruction::Drop,
        Instruction::Undefined,
        Instruction::InitializeDerivedLocal(0),
        Instruction::GetLocalCheck(0),
        Instruction::Drop,
        Instruction::Nop,
        Instruction::Undefined,
        Instruction::Return,
    ];
    assert_eq!(
        validate_pattern_parameter_bytecode_layout(
            &metadata,
            &code,
            &[true],
            &[true, false],
            &[false, false],
            None,
        ),
        Ok(()),
    );
    let mut escaped_protocol = code.clone();
    escaped_protocol[2] = Instruction::InitDerivedConstructor;
    assert_eq!(
        validate_pattern_parameter_bytecode_layout(
            &metadata,
            &escaped_protocol,
            &[true],
            &[true, false],
            &[false, false],
            None,
        ),
        Err("constructor completion protocol escaped into parameter initialization"),
    );

    let ordinary = FunctionMetadata {
        derived_this_local: None,
        active_function_local: None,
        constructor_kind: ConstructorKind::None,
        super_call_allowed: false,
        super_allowed: false,
        ..metadata
    };
    assert_eq!(
        validate_pattern_parameter_bytecode_layout(
            &ordinary,
            &code,
            &[true],
            &[true, false],
            &[false, false],
            None,
        ),
        Err("parameter BindingPattern bytecode accessed a body lexical local"),
    );

    let visibility_code = [
        Instruction::PushActiveFunction,
        Instruction::PutLocal(1),
        Instruction::Nop,
    ];
    let visibility_layout = ParameterEnvironmentLayout {
        initialization_end: 2,
        argument_cells: Box::new([]),
        pattern_copies: Box::new([]),
        default_sources: Box::new([]),
        synthetic_arguments_local: None,
        arg_eval_variable_object_local: None,
    };
    assert_eq!(
        parameter_initializer_visible_locals(
            &metadata,
            &visibility_code,
            Some(2),
            &[false, false],
            Some(&visibility_layout),
        ),
        Ok(Some(vec![true, true])),
    );

    let relay_metadata = FunctionMetadata {
        closure_count: 1,
        super_call_allowed: true,
        super_allowed: true,
        ..FunctionMetadata::default()
    };
    let relay = [
        Instruction::ConstructSuper(0),
        Instruction::Dup,
        Instruction::InitializeDerivedVarRef(0),
        Instruction::Undefined,
        Instruction::Return,
    ];
    let descriptor = ClosureVariable {
        source: ClosureSource::ParentLocal(0),
        name: ClosureVariableName::None,
        is_lexical: true,
        is_const: false,
        kind: ClosureVariableKind::Normal,
    };
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &relay_metadata,
            &relay,
            &[],
            &[],
            &[descriptor],
        ),
        Ok(()),
    );

    let mut generic_relay = relay;
    generic_relay[0] = Instruction::Construct(0);
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &relay_metadata,
            &generic_relay,
            &[],
            &[],
            &[descriptor],
        ),
        Err("captured derived initializer has no constructor result"),
    );
    let generic_apply_relay = [
        Instruction::Apply(crate::engine::code::bytecode::ApplyKind::Construct),
        Instruction::Dup,
        Instruction::InitializeDerivedVarRef(0),
        Instruction::Undefined,
        Instruction::Return,
    ];
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &relay_metadata,
            &generic_apply_relay,
            &[],
            &[],
            &[descriptor],
        ),
        Err("captured derived initializer has no constructor result"),
    );
    let injected_result = [
        Instruction::PushTrue,
        Instruction::IfFalse(6),
        Instruction::ConstructSuper(0),
        Instruction::Dup,
        Instruction::InitializeDerivedVarRef(0),
        Instruction::Return,
        Instruction::Object,
        Instruction::Goto(3),
    ];
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &relay_metadata,
            &injected_result,
            &[],
            &[],
            &[descriptor],
        ),
        Err("captured derived initializer protocol has a non-fallthrough entry"),
    );

    let ordinary_relay = FunctionMetadata {
        super_call_allowed: false,
        super_allowed: false,
        ..relay_metadata
    };
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &ordinary_relay,
            &[Instruction::MarkSuperCall],
            &[],
            &[],
            &[],
        ),
        Err("typed super-call opcode has no inherited super authority"),
    );
}

#[test]
fn derived_active_function_initialization_is_an_entry_capability() {
    let metadata = FunctionMetadata {
        local_count: 2,
        derived_this_local: Some(0),
        active_function_local: Some(1),
        constructor_kind: ConstructorKind::Derived,
        strict: true,
        super_call_allowed: true,
        super_allowed: true,
        ..FunctionMetadata::default()
    };
    let entry = [
        Instruction::PushActiveFunction,
        Instruction::PutLocal(1),
        Instruction::Undefined,
        Instruction::ReturnDerived(0),
    ];
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &metadata,
            &entry,
            &[true, false],
            &[false, false],
            &[],
        ),
        Ok(()),
    );

    for ordinary_return in [
        Instruction::TailCall(0),
        Instruction::TailCallMethod(0),
        Instruction::Return,
        Instruction::ReturnUndefined,
    ] {
        let forged = [
            Instruction::PushActiveFunction,
            Instruction::PutLocal(1),
            ordinary_return,
        ];
        assert_eq!(
            validate_derived_constructor_bytecode_layout(
                &metadata,
                &forged,
                &[true, false],
                &[false, false],
                &[],
            ),
            Err("derived constructor contains an ordinary return"),
        );
    }

    let dead = [
        Instruction::Undefined,
        Instruction::ReturnDerived(0),
        Instruction::PushActiveFunction,
        Instruction::PutLocal(1),
    ];
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &metadata,
            &dead,
            &[true, false],
            &[false, false],
            &[],
        ),
        Err("derived constructor active-function initialization is not at entry"),
    );

    let default = [
        Instruction::PushActiveFunction,
        Instruction::PutLocal(1),
        Instruction::CheckCtor,
        Instruction::InitDerivedConstructor,
        Instruction::Dup,
        Instruction::InitializeDerivedLocal(0),
        Instruction::GetLocal(1),
        Instruction::CallClassInstanceInitializer,
        Instruction::ReturnDerived(0),
    ];
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &metadata,
            &default,
            &[true, false],
            &[false, false],
            &[],
        ),
        Ok(()),
    );

    let mut forged_default = default.to_vec();
    forged_default.insert(3, Instruction::Nop);
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &metadata,
            &forged_default,
            &[true, false],
            &[false, false],
            &[],
        ),
        Err("default-derived constructor has no exact synthesized shape"),
    );

    let arbitrary_this = [
        Instruction::PushActiveFunction,
        Instruction::PutLocal(1),
        Instruction::PushI32(1),
        Instruction::Dup,
        Instruction::InitializeDerivedLocal(0),
        Instruction::Undefined,
        Instruction::ReturnDerived(0),
    ];
    assert_eq!(
        validate_derived_constructor_bytecode_layout(
            &metadata,
            &arbitrary_this,
            &[true, false],
            &[false, false],
            &[],
        ),
        Err("derived local initializer has no constructor result"),
    );
}

#[test]
fn base_class_initializer_hook_is_unique_and_entry_only() {
    let metadata = FunctionMetadata {
        constructor_kind: ConstructorKind::Base,
        strict: true,
        ..FunctionMetadata::default()
    };
    let canonical = [
        Instruction::CheckCtor,
        Instruction::PushThis,
        Instruction::PushActiveFunction,
        Instruction::CallClassInstanceInitializer,
        Instruction::Drop,
        Instruction::Undefined,
        Instruction::Return,
    ];
    assert_eq!(
        validate_derived_constructor_bytecode_layout(&metadata, &canonical, &[], &[], &[]),
        Ok(()),
    );

    let mut duplicate = canonical.to_vec();
    duplicate.splice(
        5..5,
        [
            Instruction::PushThis,
            Instruction::PushActiveFunction,
            Instruction::CallClassInstanceInitializer,
            Instruction::Drop,
        ],
    );
    assert_eq!(
        validate_derived_constructor_bytecode_layout(&metadata, &duplicate, &[], &[], &[]),
        Err("active-function opcode escaped a derived constructor"),
    );

    let mut backedge = canonical.to_vec();
    backedge.splice(5..5, [Instruction::Goto(0)]);
    assert_eq!(
        validate_derived_constructor_bytecode_layout(&metadata, &backedge, &[], &[], &[]),
        Err("base class initializer protocol has a non-fallthrough entry"),
    );
}

#[test]
fn class_initializers_require_a_home_object_slot() {
    let mut metadata = FunctionMetadata {
        class_initializer_kind: Some(ClassInitializerKind::InstanceFields),
        strict: true,
        super_allowed: true,
        arguments_forbidden: true,
        ..FunctionMetadata::default()
    };
    let code = [Instruction::Undefined, Instruction::Return];
    assert_eq!(
        validate_class_initializer_bytecode_layout(&metadata, &code),
        Err("class initializer function metadata is malformed"),
    );
    metadata.needs_home_object = true;
    assert_eq!(
        validate_class_initializer_bytecode_layout(&metadata, &code),
        Ok(()),
    );
}

#[test]
fn bytecode_allocation_rejects_stack_underflow_in_privileged_class_initializer() {
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
        Instruction::Drop,
        Instruction::Undefined,
        Instruction::Return,
    ]);
    let mut initializer = bytecode(&code, context, Vec::new(), Vec::new());
    initializer.metadata.class_initializer_kind = Some(ClassInitializerKind::InstanceFields);
    initializer.metadata.strict = true;
    initializer.metadata.super_allowed = true;
    initializer.metadata.arguments_forbidden = true;
    initializer.metadata.needs_home_object = true;
    assert_eq!(
        heap.allocate_function_bytecode(initializer),
        Err(HeapError::Invariant(
            "function bytecode failed generic verification"
        ))
    );
    assert_eq!(heap.counts().function_bytecode_nodes, 0);

    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn class_static_initializer_claim_is_one_shot_per_constructor() {
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
        Instruction::CheckCtor,
        Instruction::PushThis,
        Instruction::PushActiveFunction,
        Instruction::CallClassInstanceInitializer,
        Instruction::Drop,
        Instruction::Undefined,
        Instruction::Return,
    ]);
    let mut owner = bytecode(&code, context, Vec::new(), Vec::new());
    owner.metadata.constructor_kind = ConstructorKind::Base;
    owner.metadata.strict = true;
    owner.metadata.max_stack = 2;
    let owner = heap.allocate_function_bytecode(owner).unwrap();
    let constructor = heap
        .allocate_object(ObjectData::bytecode_function(
            shape,
            Vec::new(),
            owner,
            None,
            true,
        ))
        .unwrap();

    assert_eq!(
        heap.begin_bytecode_class_static_initializer(constructor),
        Ok(())
    );
    assert_eq!(
        heap.begin_bytecode_class_static_initializer(constructor),
        Err(HeapError::Invariant(
            "class static initializer was already started"
        ))
    );

    heap.release_object(constructor).unwrap();
    heap.release_function_bytecode(owner).unwrap();
    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn bytecode_allocation_accepts_typed_class_heritage() {
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
        Instruction::PushI32(7),
        Instruction::DefineClass {
            name: 0,
            has_heritage: true,
        },
        Instruction::Drop,
        Instruction::Return,
    ]);
    let mut candidate = bytecode(
        &code,
        context,
        vec![BytecodeConstant::Value(RawValue::String(
            JsString::from_static("Derived"),
        ))],
        Vec::new(),
    );
    candidate.metadata.max_stack = 2;
    let function = heap.allocate_function_bytecode(candidate).unwrap();
    heap.release_function_bytecode(function).unwrap();

    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn bytecode_allocation_rejects_pattern_access_to_body_lexical() {
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
        Instruction::GetArg(0),
        Instruction::Drop,
        Instruction::GetLocalCheck(1),
        Instruction::Drop,
        Instruction::Nop,
        Instruction::GetLocal(0),
        Instruction::Return,
    ]);
    let mut malformed = bytecode(&code, context, Vec::new(), Vec::new());
    malformed.metadata.argument_count = 1;
    malformed.metadata.defined_argument_count = 1;
    malformed.metadata.pattern_argument_count = 1;
    malformed.metadata.parameter_pattern_end = Some(4);
    malformed.metadata.local_count = 2;
    malformed.metadata.max_stack = 1;
    malformed.argument_definitions = Rc::from([VariableDefinition {
        name: None,
        is_lexical: false,
        is_const: false,
        is_parameter_initializer: false,
        kind: ClosureVariableKind::Normal,
    }]);
    malformed.local_definitions = Rc::from([
        VariableDefinition {
            name: None,
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        },
        VariableDefinition {
            name: None,
            is_lexical: true,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        },
    ]);
    assert_eq!(
        heap.allocate_function_bytecode(malformed),
        Err(HeapError::Invariant(
            "parameter BindingPattern bytecode accessed a body lexical local"
        ))
    );

    let empty_rest = |body_value, defined_argument_count| {
        let code: Rc<[Instruction]> = Rc::from([
            Instruction::Rest(0),
            Instruction::Drop,
            Instruction::Nop,
            body_value,
            Instruction::Return,
        ]);
        let mut candidate = bytecode(&code, context, Vec::new(), Vec::new());
        candidate.metadata.defined_argument_count = defined_argument_count;
        candidate.metadata.rest_pattern_start = Some(0);
        candidate.metadata.parameter_pattern_end = Some(2);
        candidate.metadata.max_stack = 1;
        candidate
    };
    let direct_this = heap
        .allocate_function_bytecode(empty_rest(Instruction::PushThis, 1))
        .unwrap();
    heap.release_function_bytecode(direct_this).unwrap();
    let direct_new_target = heap
        .allocate_function_bytecode(empty_rest(Instruction::PushNewTarget, 1))
        .unwrap();
    heap.release_function_bytecode(direct_new_target).unwrap();
    let mut direct_home_object = empty_rest(Instruction::PushHomeObject, 1);
    direct_home_object.metadata.needs_home_object = true;
    let direct_home_object = heap.allocate_function_bytecode(direct_home_object).unwrap();
    heap.release_function_bytecode(direct_home_object).unwrap();
    assert_eq!(
        heap.allocate_function_bytecode(empty_rest(Instruction::Undefined, 1)),
        Err(HeapError::Invariant(
            "rest BindingPattern metadata disagrees with function length"
        ))
    );
    assert_eq!(
        heap.allocate_function_bytecode(empty_rest(Instruction::PushThis, 0)),
        Err(HeapError::Invariant(
            "rest BindingPattern metadata disagrees with function length"
        ))
    );

    heap.release_context(context).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}
