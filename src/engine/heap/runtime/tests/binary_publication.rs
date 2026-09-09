use super::*;

#[test]
fn trusted_quickjs_ordinary_leaf_executes_real_control_flow_and_function_properties() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();

    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_LEAF_42_BC5, 0)
        .unwrap();
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + 1
    );
    assert_eq!(
        runtime.heap_counts().object_nodes,
        baseline.object_nodes + 1
    );
    assert!(runtime.is_constructor(function.as_object()).unwrap());
    assert_eq!(
        runtime.get_prototype_of(function.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );

    assert_eq!(
        context
            .call(&function, Value::Undefined, &[Value::Int(3), Value::Int(3)],)
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[Value::Int(3), Value::Int(4)],)
            .unwrap(),
        Value::Int(0)
    );

    let length = runtime.intern_property_key("length").unwrap();
    let name = runtime.intern_property_key("name").unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let constructor = runtime.intern_property_key("constructor").unwrap();
    let caller = runtime.intern_property_key("caller").unwrap();
    let arguments = runtime.intern_property_key("arguments").unwrap();
    assert_eq!(
        runtime.own_property_keys(function.as_object()).unwrap(),
        vec![length.clone(), name.clone(), prototype_key.clone()]
    );
    assert!(matches!(
        runtime
            .get_own_property(function.as_object(), &length)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(2),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::String(name_value),
        writable: false,
        enumerable: false,
        configurable: true,
    }) = runtime
        .get_own_property(function.as_object(), &name)
        .unwrap()
    else {
        panic!("trusted ordinary leaf has the wrong name descriptor");
    };
    assert!(name_value.is_empty());
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(prototype),
        writable: true,
        enumerable: false,
        configurable: false,
    }) = runtime
        .get_own_property(function.as_object(), &prototype_key)
        .unwrap()
    else {
        panic!("trusted ordinary leaf has the wrong prototype descriptor");
    };
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(constructor_value),
        writable: true,
        enumerable: false,
        configurable: true,
    }) = runtime.get_own_property(&prototype, &constructor).unwrap()
    else {
        panic!("trusted ordinary leaf prototype has the wrong constructor descriptor");
    };
    assert_eq!(constructor_value, function.as_object().clone());
    assert_eq!(
        context.get_property(function.as_object(), &caller).unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context
            .get_property(function.as_object(), &arguments)
            .unwrap(),
        Value::Undefined
    );
    let Value::Object(instance) = context
        .construct(&function, &[Value::Int(3), Value::Int(3)])
        .unwrap()
    else {
        panic!("trusted ordinary leaf constructor returned a primitive");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(prototype)
    );
}

#[test]
fn trusted_quickjs_ordinary_leaf_preserves_strict_and_capability_metadata() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let mut strict = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    strict[28] = 0x01;
    let strict = context.read_trusted_ordinary_function(&strict, 0).unwrap();
    for property in ["caller", "arguments"] {
        let key = runtime.intern_property_key(property).unwrap();
        assert_eq!(
            context.get_property(strict.as_object(), &key),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_name_and_message(&runtime, &mut context),
            (
                JsString::from_static("TypeError"),
                JsString::from_static("invalid property access"),
            )
        );
    }

    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let mut no_new_target = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    no_new_target[26] &= !0x40;
    let mut no_arguments = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    no_arguments[27] &= !0x02;
    for (label, image) in [
        ("new.target capability", no_new_target),
        ("arguments capability", no_arguments),
    ] {
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("ordinary leaf without {label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(!context.has_exception());
        assert_eq!(runtime.heap_counts(), baseline);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }
}

#[test]
fn trusted_quickjs_ordinary_leaf_preserves_frontier_and_malformed_provenance() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    let RuntimeError::Engine(error) = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_LEAF_42_BC5, 1)
        .unwrap_err()
    else {
        panic!("out-of-range root selector did not return an engine error");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let RuntimeError::Engine(error) = context
        .read_trusted_ordinary_function(QUICKJS_SCALAR_42_BC5, 0)
        .unwrap_err()
    else {
        panic!("scalar root did not preserve the ordinary-leaf frontier");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let mut verifier_rejected = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    verifier_rejected[57] = 0x9b; // `add` is typed but underflows this CFG.
    let RuntimeError::Engine(error) = context
        .read_trusted_ordinary_function(&verifier_rejected, 0)
        .unwrap_err()
    else {
        panic!("typed-verifier rejection did not return an engine error");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().contains("typed verification"));
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let mut truncated = QUICKJS_ORDINARY_LEAF_42_BC5.to_vec();
    truncated.pop();
    assert_eq!(
        context.read_trusted_ordinary_function(&truncated, 0),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("read after the end of the buffer"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes
    );
}

#[test]
fn trusted_quickjs_ordinary_leaf_publication_rolls_back_a_stale_realm() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let stale_realm = context.realm;
    drop(context);
    runtime.run_gc().unwrap();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    assert!(
        runtime
            .read_trusted_ordinary_function_in_realm(stale_realm, QUICKJS_ORDINARY_LEAF_42_BC5, 0,)
            .is_err()
    );
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn trusted_quickjs_ordinary_leaf_synthesizes_bigint_and_canonical_empty_atom_constants() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut sibling = runtime.new_context();

    let mut bigint_code = vec![0xb0];
    bigint_code.extend_from_slice(&42_i32.to_le_bytes());
    bigint_code.push(0x28);
    let bigint_image = quickjs_ordinary_with_code_and_constants(&bigint_code, &[]);
    let bigint_function = context
        .read_trusted_ordinary_function(&bigint_image, 0)
        .unwrap();
    assert_eq!(
        context
            .call(&bigint_function, Value::Undefined, &[])
            .unwrap(),
        Value::BigInt(JsBigInt::from(42))
    );
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&bigint_function).unwrap()
    else {
        panic!("trusted ordinary leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [Instruction::PushConst(0), Instruction::Return]
    ));
    assert!(matches!(
        snapshot.constants.as_ref(),
        [BytecodeConstant::Value(RawValue::BigInt(value))]
            if value == &JsBigInt::from(42)
    ));

    let direct_image = quickjs_ordinary_with_code_and_constants(&[0xbf, 0x28], &[]);
    let empty_entry = quickjs_string_constant_entry(&[], false);
    let constant_image =
        quickjs_ordinary_with_code_and_constants(&[0xbd, 0x00, 0x28], &[empty_entry.as_slice()]);
    let direct_function = context
        .read_trusted_ordinary_function(&direct_image, 0)
        .unwrap();
    let sibling_function = sibling
        .read_trusted_ordinary_function(&direct_image, 0)
        .unwrap();
    let constant_function = context
        .read_trusted_ordinary_function(&constant_image, 0)
        .unwrap();
    let direct = expect_string_value(
        context
            .call(&direct_function, Value::Undefined, &[])
            .unwrap(),
    );
    let repeated = expect_string_value(
        context
            .call(&direct_function, Value::Undefined, &[])
            .unwrap(),
    );
    let sibling_value = expect_string_value(
        sibling
            .call(&sibling_function, Value::Undefined, &[])
            .unwrap(),
    );
    let constant = expect_string_value(
        context
            .call(&constant_function, Value::Undefined, &[])
            .unwrap(),
    );
    let source = expect_string_value(context.eval("''").unwrap());
    assert!(direct.is_empty());
    assert!(direct.same_representation(&repeated));
    assert!(direct.same_representation(&sibling_value));
    assert!(direct.same_representation(&source));
    assert!(!direct.same_representation(&constant));

    let original = quickjs_float_constant_entry(0.5_f64.to_bits());
    let mut mixed_code = vec![0xbd, 0x00, 0x0e, 0xb0];
    mixed_code.extend_from_slice(&7_i32.to_le_bytes());
    mixed_code.extend_from_slice(&[0x0e, 0xbf, 0x0e, 0xb0]);
    mixed_code.extend_from_slice(&(-3_i32).to_le_bytes());
    mixed_code.extend_from_slice(&[0x0e, 0xbf, 0x28]);
    let mut mixed_image =
        quickjs_ordinary_with_code_and_constants(&mixed_code, &[original.as_slice()]);
    mixed_image[33] = 1;
    let mixed_function = context
        .read_trusted_ordinary_function(&mixed_image, 0)
        .unwrap();
    let mixed_result = expect_string_value(
        context
            .call(&mixed_function, Value::Undefined, &[])
            .unwrap(),
    );
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&mixed_function).unwrap()
    else {
        panic!("trusted ordinary leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::PushConst(0),
            Instruction::Drop,
            Instruction::PushConst(1),
            Instruction::Drop,
            Instruction::PushConst(2),
            Instruction::Drop,
            Instruction::PushConst(3),
            Instruction::Drop,
            Instruction::PushConst(4),
            Instruction::Return,
        ]
    ));
    assert_eq!(snapshot.constants.len(), 5);
    assert!(matches!(
        &snapshot.constants[0],
        BytecodeConstant::Value(RawValue::Float(value))
            if value.to_bits() == 0.5_f64.to_bits()
    ));
    assert!(matches!(
        &snapshot.constants[1],
        BytecodeConstant::Value(RawValue::BigInt(value)) if value == &JsBigInt::from(7)
    ));
    assert!(matches!(
        &snapshot.constants[3],
        BytecodeConstant::Value(RawValue::BigInt(value)) if value == &JsBigInt::from(-3)
    ));
    let BytecodeConstant::Value(RawValue::String(first_empty)) = &snapshot.constants[2] else {
        panic!("first synthesized empty atom lost its String payload");
    };
    let BytecodeConstant::Value(RawValue::String(second_empty)) = &snapshot.constants[4] else {
        panic!("second synthesized empty atom lost its String payload");
    };
    assert!(first_empty.same_representation(second_empty));
    assert!(mixed_result.same_representation(first_empty));
}

#[test]
fn trusted_quickjs_ordinary_leaf_return_undefined_is_a_zero_stack_terminal() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut image = quickjs_ordinary_with_code_and_constants(&[0x29], &[]);
    image[33] = 0;

    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    );
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 0);
    assert!(matches!(
        snapshot.code.as_ref(),
        [Instruction::ReturnUndefined]
    ));
}

#[test]
fn trusted_quickjs_ordinary_branch_targets_raw11_typed_index() {
    let mut image = QUICKJS_ORDINARY_OBJECT_BC5.to_vec();
    image[37] = 4;
    image.truncate(39);
    image.extend_from_slice(&[0xea, 0x01, 0x0b, 0x28]);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object_prototype = context.object_prototype().unwrap();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("branch-to-raw11 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::Goto(1),
            Instruction::Object,
            Instruction::Return,
        ]
    ));
    drop(snapshot);

    let Value::Object(first) = context.call(&function, Value::Undefined, &[]).unwrap() else {
        panic!("branch-to-raw11 call did not return an Object");
    };
    let Value::Object(second) = context.call(&function, Value::Undefined, &[]).unwrap() else {
        panic!("repeated branch-to-raw11 call did not return an Object");
    };
    assert_ne!(first, second);
    assert_eq!(
        runtime.get_prototype_of(&first).unwrap(),
        Some(object_prototype.clone())
    );
    assert_eq!(
        runtime.get_prototype_of(&second).unwrap(),
        Some(object_prototype)
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_nop_preserves_exact_metadata_realm_and_zero_effect() {
    assert_eq!(QUICKJS_ORDINARY_NOP_BC5.len(), 41);
    assert_eq!(fnv1a64(QUICKJS_ORDINARY_NOP_BC5), 0x1c52_2736_e3cb_ef92);

    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let baseline = runtime.heap_counts();
    let function = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_NOP_BC5, 0)
        .unwrap();
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + 1
    );
    assert_eq!(
        runtime.get_prototype_of(function.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap())
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted raw177 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [Instruction::Nop, Instruction::ReturnUndefined]
    ));
    assert!(snapshot.constants.is_empty());
    assert_eq!(snapshot.metadata.argument_count, 0);
    assert_eq!(snapshot.metadata.defined_argument_count, 0);
    assert_eq!(snapshot.metadata.local_count, 0);
    assert_eq!(snapshot.metadata.max_stack, 0);
    assert!(snapshot.metadata.strict);
    assert!(snapshot.metadata.strip_variable_debug);
    assert_eq!(snapshot.metadata.function_kind, FunctionKind::Normal);
    assert!(snapshot.metadata.has_prototype);
    assert_eq!(snapshot.metadata.constructor_kind, ConstructorKind::Base);
    assert!(!snapshot.metadata.arguments_forbidden);
    drop(snapshot);

    for _ in 0..2 {
        assert_eq!(
            caller.call(&function, Value::Undefined, &[]).unwrap(),
            Value::Undefined
        );
        assert!(!caller.has_exception());
    }
}

#[test]
fn trusted_quickjs_ordinary_nop_only_fallthrough_rolls_back_and_retries() {
    const FALLTHROUGH: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode ended without return";

    let mut nop_only = QUICKJS_ORDINARY_NOP_BC5.to_vec();
    nop_only[37] = 1;
    nop_only.truncate(40);
    assert_eq!(nop_only.len(), 40);
    assert_eq!(nop_only[39], 0xb1);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let RuntimeError::Engine(error) = context
        .read_trusted_ordinary_function(&nop_only, 0)
        .unwrap_err()
    else {
        panic!("raw177-only fallthrough did not return an engine error");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(error.message(), FALLTHROUGH);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_NOP_BC5, 0)
        .unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_branch_targets_raw177_typed_index() {
    let mut image = QUICKJS_ORDINARY_NOP_BC5.to_vec();
    image[37] = 4;
    image.truncate(39);
    image.extend_from_slice(&[0xea, 0x01, 0xb1, 0x29]);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("branch-to-raw177 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::Goto(1),
            Instruction::Nop,
            Instruction::ReturnUndefined,
        ]
    ));
    drop(snapshot);
    assert_eq!(
        context.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_branch_targets_follow_the_first_expanded_instruction() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    // The taken label8 jump skips a raw rot3l. rot3l expands to Perm3, Swap,
    // so the native target at source instruction 6 becomes IR 7.
    let code = [0xbb, 1, 0xbb, 2, 0xbb, 3, 0x0a, 0xe9, 2, 0x1d, 0x28];
    let mut image = quickjs_ordinary_with_code_and_constants(&code, &[]);
    image[33] = 4;

    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Int(3)
    );
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::PushTrue,
            Instruction::IfTrue(7),
            Instruction::Perm3,
            Instruction::Swap,
            Instruction::Return,
        ]
    ));
}

#[test]
fn trusted_quickjs_ordinary_branch_targets_can_land_on_plain_call_after_expansion() {
    let code = [0xcf, 0xea, 0x05, 0xb4, 0xb5, 0xb6, 0x1d, 0xec, 0x28];
    let mut image = quickjs_ordinary_with_code_and_constants(&code, &[]);
    image[33] = 1;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let callback = context
        .eval("(0, function () { 'use strict'; return this === undefined; })")
        .unwrap();
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[callback])
            .unwrap(),
        Value::Bool(true)
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary branch-to-call leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 1);
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::Goto(7),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::Perm3,
            Instruction::Swap,
            Instruction::Call(0),
            Instruction::Return,
        ]
    ));
}

#[cfg(feature = "test262-host")]
#[test]
fn trusted_quickjs_ordinary_fused_predicates_distinguish_htmldda() {
    fn predicate(
        context: &mut crate::engine::api::context::Context,
        raw: u8,
        value: Value,
    ) -> Value {
        let image = quickjs_ordinary_with_code_and_constants(&[0xcf, raw, 0x28], &[]);
        let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
        context.call(&function, Value::Undefined, &[value]).unwrap()
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(normal_callable) = context.eval("(function () {})").unwrap() else {
        panic!("function expression did not produce an Object");
    };
    let Value::Object(callable_proxy) = context.eval("new Proxy(function () {}, {})").unwrap()
    else {
        panic!("callable Proxy did not produce an Object");
    };
    let Value::Object(html_dda_callable) = context.eval("(function () {})").unwrap() else {
        panic!("function expression did not produce an Object");
    };
    runtime.set_object_is_html_dda(&html_dda_callable).unwrap();
    let Value::Object(ordinary_object) = context.eval("({})").unwrap() else {
        panic!("object literal did not produce an Object");
    };

    assert_eq!(
        predicate(&mut context, 0xf0, Value::Undefined),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf0, Value::Object(html_dda_callable.clone())),
        Value::Bool(false)
    );
    assert_eq!(
        predicate(&mut context, 0xf1, Value::Null),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf1, Value::Object(html_dda_callable.clone())),
        Value::Bool(false)
    );
    assert_eq!(
        predicate(&mut context, 0xf2, Value::Undefined),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf2, Value::Object(html_dda_callable.clone())),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf3, Value::Object(normal_callable)),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf3, Value::Object(callable_proxy)),
        Value::Bool(true)
    );
    assert_eq!(
        predicate(&mut context, 0xf3, Value::Object(html_dda_callable)),
        Value::Bool(false)
    );
    assert_eq!(
        predicate(&mut context, 0xf3, Value::Object(ordinary_object)),
        Value::Bool(false)
    );
}
