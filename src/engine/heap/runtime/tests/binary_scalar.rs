use super::*;

#[test]
fn trusted_quickjs_scalar_script_uses_verified_runtime_publication() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;

    let function = context
        .read_trusted_scalar_script(QUICKJS_SCALAR_42_BC5)
        .unwrap();
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline + 1);
    assert_eq!(context.execute(&function).unwrap(), Value::Int(42));

    let mut same_runtime_realm = runtime.new_context();
    assert_eq!(
        same_runtime_realm.execute(&function).unwrap(),
        Value::Int(42)
    );

    let foreign_runtime = Runtime::new();
    let mut foreign_context = foreign_runtime.new_context();
    let foreign_objects = foreign_runtime.heap_counts().object_nodes;
    assert_eq!(
        foreign_context.execute(&function),
        Err(RuntimeError::WrongRuntime("function bytecode"))
    );
    assert_eq!(foreign_runtime.heap_counts().object_nodes, foreign_objects);
}

#[test]
fn trusted_quickjs_scalar_script_executes_the_full_direct_int32_family() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;
    let mut functions = Vec::new();
    let cases: &[(&[u8], i32)] = &[
        (&[0xb2, 0xcb, 0x28], -1),
        (&[0xb3, 0xcb, 0x28], 0),
        (&[0xba, 0xcb, 0x28], 7),
        (&[0xbb, 0x80, 0xcb, 0x28], -128),
        (&[0xbb, 0x7f, 0xcb, 0x28], 127),
        (&[0xbc, 0x7f, 0xff, 0xcb, 0x28], -129),
        (&[0xbc, 0xff, 0x7f, 0xcb, 0x28], 32_767),
        // Wider-than-canonical encodings remain valid QuickJS reader inputs.
        (&[0xbc, 0x01, 0x00, 0xcb, 0x28], 1),
        (&[0x01, 0xff, 0x7f, 0xff, 0xff, 0xcb, 0x28], -32_769),
        (&[0x01, 0x00, 0x80, 0x00, 0x00, 0xcb, 0x28], 32_768),
        (&[0x01, 0xff, 0xff, 0xff, 0x7f, 0xcb, 0x28], i32::MAX),
        (&[0x01, 0x00, 0x00, 0x00, 0x80, 0xcb, 0x28], i32::MIN),
        (&[0x01, 0x01, 0x00, 0x00, 0x00, 0xcb, 0x28], 1),
    ];

    for (code, expected) in cases {
        let image = quickjs_scalar_with_code(code);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(context.execute(&function).unwrap(), Value::Int(*expected));
        functions.push(function);
    }
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline + cases.len()
    );
}

#[test]
fn trusted_quickjs_scalar_script_executes_direct_atom_free_primitives() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_functions = runtime.heap_counts().function_bytecode_nodes;
    let baseline_atoms = runtime.test_atom_count();
    let mut functions = Vec::new();
    let cases = vec![
        (vec![0x06, 0xcb, 0x28], Value::Undefined),
        (vec![0x07, 0xcb, 0x28], Value::Null),
        (vec![0x09, 0xcb, 0x28], Value::Bool(false)),
        (vec![0x0a, 0xcb, 0x28], Value::Bool(true)),
        (
            vec![0xb0, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28],
            Value::BigInt(JsBigInt::zero()),
        ),
        (
            vec![0xb0, 0xff, 0xff, 0xff, 0xff, 0xcb, 0x28],
            Value::BigInt(JsBigInt::from(-1)),
        ),
        (
            vec![0xb0, 0xff, 0xff, 0xff, 0x7f, 0xcb, 0x28],
            Value::BigInt(JsBigInt::from(i32::MAX)),
        ),
        (
            vec![0xb0, 0x01, 0x00, 0x00, 0x80, 0xcb, 0x28],
            Value::BigInt(JsBigInt::from(-2_147_483_647)),
        ),
        (
            vec![0xb0, 0x00, 0x00, 0x00, 0x80, 0xcb, 0x28],
            Value::BigInt(JsBigInt::from(i32::MIN)),
        ),
    ];

    for (code, expected) in &cases {
        let image = quickjs_scalar_with_code(code);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(context.execute(&function).unwrap(), expected.clone());
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        functions.push(function);
    }
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline_functions + cases.len()
    );
}

#[test]
fn trusted_quickjs_scalar_script_preserves_constant_pool_string_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    let cases: &[(&[u16], bool)] = &[
        (&[], false),
        (&[u16::from(b'a'), 0, 0x00e9], false),
        (&[0x0100, 0xd83d, 0xde00, 0xd800], true),
        // Reader-compatible wide storage containing only Latin-1.
        (&[u16::from(b'4'), u16::from(b'2')], true),
    ];

    for &(units, wide) in cases {
        let image = quickjs_scalar_with_string_constant(&[0xbd, 0x00, 0xcb, 0x28], units, wide);
        let first_function = context.read_trusted_scalar_script(&image).unwrap();
        let first = expect_string_value(context.execute(&first_function).unwrap());
        let repeated = expect_string_value(context.execute(&first_function).unwrap());
        assert_eq!(first.utf16_units().collect::<Vec<_>>(), units);
        assert!(first.same_representation(&repeated));

        let second_function = context.read_trusted_scalar_script(&image).unwrap();
        let independent = expect_string_value(context.execute(&second_function).unwrap());
        assert_eq!(independent, first);
        assert!(!first.same_representation(&independent));
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }
}

#[test]
fn trusted_quickjs_scalar_script_republishes_ordinary_atom_strings_by_runtime_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut sibling = runtime.new_context();

    let pinned = quickjs_scalar_with_atom_value(50);
    let first_function = context.read_trusted_scalar_script(&pinned).unwrap();
    let second_function = sibling.read_trusted_scalar_script(&pinned).unwrap();
    let first = expect_string_value(context.execute(&first_function).unwrap());
    let repeated = expect_string_value(context.execute(&first_function).unwrap());
    let sibling_value = expect_string_value(sibling.execute(&second_function).unwrap());
    let source_value = expect_string_value(context.eval("'length'").unwrap());
    assert_eq!(first, JsString::from_static("length"));
    assert!(first.same_representation(&repeated));
    assert!(first.same_representation(&sibling_value));
    assert!(first.same_representation(&source_value));

    let dynamic_image = quickjs_scalar_with_atom_slot(
        "quickjs-oxide-string-scalar"
            .encode_utf16()
            .collect::<Vec<_>>()
            .as_slice(),
        false,
    );
    let dynamic_left = context.read_trusted_scalar_script(&dynamic_image).unwrap();
    let dynamic_right = sibling.read_trusted_scalar_script(&dynamic_image).unwrap();
    let dynamic_left = expect_string_value(context.execute(&dynamic_left).unwrap());
    let dynamic_right = expect_string_value(sibling.execute(&dynamic_right).unwrap());
    let dynamic_source =
        expect_string_value(context.eval("'quickjs-oxide-string-scalar'").unwrap());
    assert!(dynamic_left.same_representation(&dynamic_right));
    assert!(dynamic_left.same_representation(&dynamic_source));
}

#[test]
fn trusted_quickjs_empty_string_uses_the_runtime_canonical_atom() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut sibling = runtime.new_context();

    let direct = quickjs_scalar_with_code(&[0xbf, 0xcb, 0x28]);
    let atom_backed = quickjs_scalar_with_atom_value(47);
    let constant_pool = quickjs_scalar_with_string_constant(&[0xbd, 0, 0xcb, 0x28], &[], false);
    let direct_function = context.read_trusted_scalar_script(&direct).unwrap();
    let atom_function = sibling.read_trusted_scalar_script(&atom_backed).unwrap();
    let constant_pool_function = context.read_trusted_scalar_script(&constant_pool).unwrap();
    let direct_value = expect_string_value(context.execute(&direct_function).unwrap());
    let repeated = expect_string_value(context.execute(&direct_function).unwrap());
    let atom_value = expect_string_value(sibling.execute(&atom_function).unwrap());
    let constant_pool_value =
        expect_string_value(context.execute(&constant_pool_function).unwrap());
    let source_value = expect_string_value(context.eval("''").unwrap());
    assert!(direct_value.is_empty());
    assert!(direct_value.same_representation(&repeated));
    assert!(direct_value.same_representation(&atom_value));
    assert!(direct_value.same_representation(&source_value));
    assert!(!direct_value.same_representation(&constant_pool_value));

    let foreign_runtime = Runtime::new();
    let mut foreign_context = foreign_runtime.new_context();
    let foreign_value = expect_string_value(foreign_context.eval("''").unwrap());
    assert!(!direct_value.same_representation(&foreign_value));
}

#[test]
fn trusted_quickjs_tagged_atom_strings_are_fresh_on_every_execution() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_atoms = runtime.test_atom_count();
    for (atom, expected) in [
        (0x8000_0000, "0"),
        (0x8000_002a, "42"),
        (0xffff_ffff, "2147483647"),
    ] {
        let image = quickjs_scalar_with_atom_value(atom);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        let first = expect_string_value(context.execute(&function).unwrap());
        let second = expect_string_value(context.execute(&function).unwrap());
        assert_eq!(first, JsString::from_static(expected));
        assert_eq!(second, first);
        assert!(!first.same_representation(&second));
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }

    let slot_alias = quickjs_scalar_with_atom_slot(&[u16::from(b'4'), u16::from(b'2')], false);
    let function = context.read_trusted_scalar_script(&slot_alias).unwrap();
    let first = expect_string_value(context.execute(&function).unwrap());
    let second = expect_string_value(context.execute(&function).unwrap());
    assert_eq!(first, JsString::from_static("42"));
    assert!(!first.same_representation(&second));
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn trusted_quickjs_non_string_atoms_remain_unsupported_without_publication() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let cases = [
        quickjs_scalar_with_atom_value(0),
        quickjs_scalar_with_atom_value(229),
        quickjs_scalar_with_atom_value(230),
        quickjs_scalar_with_unused_atom_slot(50, &[u16::from(b'x')], false),
        quickjs_scalar_with_two_atom_slots(),
    ];
    for image in cases {
        let RuntimeError::Engine(error) = context.read_trusted_scalar_script(&image).unwrap_err()
        else {
            panic!("non-String atom did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(!context.has_exception());
        assert_eq!(runtime.heap_counts(), baseline);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }
}

#[test]
fn trusted_quickjs_atom_string_publication_rolls_back_a_stale_realm() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let stale_realm = context.realm;
    drop(context);
    runtime.run_gc().unwrap();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let image = quickjs_scalar_with_atom_slot(
        &"quickjs-oxide-rollback-string-scalar"
            .encode_utf16()
            .collect::<Vec<_>>(),
        false,
    );

    assert!(
        runtime
            .read_trusted_scalar_script_in_realm(stale_realm, &image)
            .is_err()
    );
    assert_eq!(runtime.heap_counts(), baseline);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn trusted_quickjs_scalar_script_executes_exact_float64_constant_pairs() {
    const SHORT_INDEX_ZERO: &[u8] = &[0xbd, 0x00, 0xcb, 0x28];
    const WIDE_INDEX_ZERO: &[u8] = &[0x02, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28];
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let mut functions = Vec::new();
    let bits_cases = [
        0.5_f64.to_bits(),
        2_147_483_648_f64.to_bits(),
        1,
        f64::MAX.to_bits(),
        f64::INFINITY.to_bits(),
        f64::NEG_INFINITY.to_bits(),
        0.0_f64.to_bits(),
        (-0.0_f64).to_bits(),
        42.0_f64.to_bits(),
        0x7ff8_0000_0000_0042,
        0x7ff0_0000_0000_0042,
    ];

    for bits in bits_cases {
        let image = quickjs_scalar_with_float_constant(SHORT_INDEX_ZERO, bits);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        let Value::Float(actual) = context.execute(&function).unwrap() else {
            panic!("Float64 constant did not retain its runtime value kind");
        };
        assert_eq!(actual.to_bits(), bits);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
        functions.push(function);
    }

    let image = quickjs_scalar_with_float_constant(WIDE_INDEX_ZERO, 0.5_f64.to_bits());
    let function = context.read_trusted_scalar_script(&image).unwrap();
    let Value::Float(actual) = context.execute(&function).unwrap() else {
        panic!("wide Float64 constant did not retain its runtime value kind");
    };
    assert_eq!(actual.to_bits(), 0.5_f64.to_bits());
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
    functions.push(function);
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + bits_cases.len() + 1
    );
}

#[test]
fn trusted_quickjs_scalar_script_executes_exact_bigint_constant_pairs() {
    const SHORT_INDEX_ZERO: &[u8] = &[0xbd, 0x00, 0xcb, 0x28];
    const WIDE_INDEX_ZERO: &[u8] = &[0x02, 0x00, 0x00, 0x00, 0x00, 0xcb, 0x28];
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let cases = vec![
        (Vec::new(), JsBigInt::zero()),
        (vec![0x01], JsBigInt::one()),
        (vec![0xff], JsBigInt::from(-1)),
        (
            vec![0x00, 0x00, 0x00, 0x80, 0x00],
            JsBigInt::from(2_147_483_648_i64),
        ),
        (
            vec![0xff, 0xff, 0xff, 0x7f, 0xff],
            JsBigInt::from(-2_147_483_649_i64),
        ),
        (
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
            JsBigInt::from(i64::MAX),
        ),
        (
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00],
            JsBigInt::parse_js_string("9223372036854775808").unwrap(),
        ),
        (
            vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff],
            JsBigInt::parse_js_string("-9223372036854775809").unwrap(),
        ),
        (
            vec![
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x01,
            ],
            JsBigInt::parse_js_string("340282366920938463463374607431768211456").unwrap(),
        ),
        // Compatible BC5 accepts redundant sign extension; the archival
        // reader normalizes it before runtime publication.
        (vec![0x00], JsBigInt::zero()),
        (vec![0x01, 0x00], JsBigInt::one()),
        (vec![0xff, 0xff], JsBigInt::from(-1)),
    ];
    let mut functions = Vec::new();

    for (payload, expected) in &cases {
        let image = quickjs_scalar_with_bigint_constant(SHORT_INDEX_ZERO, payload);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(
            context.execute(&function).unwrap(),
            Value::BigInt(expected.clone())
        );
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
        functions.push(function);
    }

    let image =
        quickjs_scalar_with_bigint_constant(WIDE_INDEX_ZERO, &[0x00, 0x00, 0x00, 0x80, 0x00]);
    let function = context.read_trusted_scalar_script(&image).unwrap();
    assert_eq!(
        context.execute(&function).unwrap(),
        Value::BigInt(JsBigInt::from(2_147_483_648_i64))
    );
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
    functions.push(function);
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + cases.len() + 1
    );
}

#[test]
fn trusted_quickjs_scalar_script_executes_bigint_negation_at_runtime() {
    const SHORT_INDEX_ZERO_NEG: &[u8] = &[0xbd, 0x00, 0x8a, 0xcb, 0x28];
    const WIDE_INDEX_ZERO_NEG: &[u8] = &[0x02, 0x00, 0x00, 0x00, 0x00, 0x8a, 0xcb, 0x28];
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let constant_cases = vec![
        (Vec::new(), JsBigInt::zero()),
        (vec![0x01], JsBigInt::from(-1)),
        (vec![0xff], JsBigInt::one()),
        (
            vec![0x00, 0x00, 0x00, 0x80, 0x00],
            JsBigInt::from(-2_147_483_648_i64),
        ),
        (
            vec![0xff, 0xff, 0xff, 0x7f, 0xff],
            JsBigInt::from(2_147_483_649_i64),
        ),
        // Negating the largest-magnitude short BigInt must promote rather
        // than overflow: i64::MIN becomes the positive heap value 2^63.
        (
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80],
            JsBigInt::parse_js_string("9223372036854775808").unwrap(),
        ),
        (
            vec![
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x01,
            ],
            JsBigInt::parse_js_string("-340282366920938463463374607431768211456").unwrap(),
        ),
        // Compatible redundant sign extension is normalized before the
        // runtime receives the value, while negation still executes in the VM.
        (vec![0x00], JsBigInt::zero()),
        (vec![0x01, 0x00], JsBigInt::from(-1)),
        (vec![0xff, 0xff], JsBigInt::one()),
    ];
    let direct_cases = [
        (0_i32, JsBigInt::zero()),
        (1, JsBigInt::from(-1)),
        (-1, JsBigInt::one()),
        (i32::MAX, JsBigInt::from(-i64::from(i32::MAX))),
        (i32::MIN, JsBigInt::parse_js_string("2147483648").unwrap()),
    ];
    let mut functions = Vec::new();

    for (payload, expected) in &constant_cases {
        let image = quickjs_scalar_with_bigint_constant(SHORT_INDEX_ZERO_NEG, payload);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(
            context.execute(&function).unwrap(),
            Value::BigInt(expected.clone())
        );
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
        functions.push(function);
    }

    let image =
        quickjs_scalar_with_bigint_constant(WIDE_INDEX_ZERO_NEG, &[0x00, 0x00, 0x00, 0x80, 0x00]);
    let function = context.read_trusted_scalar_script(&image).unwrap();
    assert_eq!(
        context.execute(&function).unwrap(),
        Value::BigInt(JsBigInt::from(-2_147_483_648_i64))
    );
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
    assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
    functions.push(function);

    for (input, expected) in direct_cases {
        let mut code = vec![0xb0];
        code.extend_from_slice(&input.to_le_bytes());
        code.extend_from_slice(&[0x8a, 0xcb, 0x28]);
        let image = quickjs_scalar_with_code(&code);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(context.execute(&function).unwrap(), Value::BigInt(expected));
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        assert_eq!(runtime.heap_counts().object_nodes, baseline.object_nodes);
        functions.push(function);
    }

    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + functions.len()
    );
}

#[test]
fn trusted_quickjs_scalar_script_executes_table_driven_unary_chains() {
    const NEG: u8 = 0x8a;
    const PLUS: u8 = 0x8b;
    const DEC: u8 = 0x8c;
    const INC: u8 = 0x8d;
    const BIT_NOT: u8 = 0x93;
    const LOGICAL_NOT: u8 = 0x94;
    const TYPEOF: u8 = 0x95;

    fn direct_code(push: &[u8], unary_ops: &[u8]) -> Vec<u8> {
        let mut code = Vec::with_capacity(push.len() + unary_ops.len() + 2);
        code.extend_from_slice(push);
        code.extend_from_slice(unary_ops);
        code.extend_from_slice(&[0xcb, 0x28]);
        code
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let direct_cases = [
        (direct_code(&[0xb4], &[NEG]), Value::Int(-1)),
        (direct_code(&[0x09], &[PLUS]), Value::Int(0)),
        (direct_code(&[0xb3], &[DEC]), Value::Int(-1)),
        (direct_code(&[0xb3], &[INC]), Value::Int(1)),
        (direct_code(&[0xb3], &[BIT_NOT]), Value::Int(-1)),
        (direct_code(&[0x0a], &[LOGICAL_NOT]), Value::Bool(false)),
        // Exercises all six value-transforming operations before `typeof`;
        // reader and publisher snapshots separately pin their exact order.
        (
            direct_code(
                &[0x0a],
                &[PLUS, INC, DEC, NEG, BIT_NOT, LOGICAL_NOT, TYPEOF],
            ),
            Value::String(JsString::from_static("boolean")),
        ),
        (direct_code(&[0xb4], &[NEG, NEG]), Value::Int(1)),
    ];
    for (code, expected) in &direct_cases {
        let function = context
            .read_trusted_scalar_script(&quickjs_scalar_with_code(code))
            .unwrap();
        assert_eq!(context.execute(&function).unwrap(), expected.clone());
    }

    for (bits, op, expected_bits) in [
        (42.0_f64.to_bits(), NEG, (-42.0_f64).to_bits()),
        ((-0.0_f64).to_bits(), PLUS, (-0.0_f64).to_bits()),
        (
            2_147_483_647.0_f64.to_bits(),
            INC,
            2_147_483_648.0_f64.to_bits(),
        ),
        (42.0_f64.to_bits(), DEC, 41.0_f64.to_bits()),
    ] {
        let code = [0xbd, 0x00, op, 0xcb, 0x28];
        let image = quickjs_scalar_with_float_constant(&code, bits);
        let function = context.read_trusted_scalar_script(&image).unwrap();
        let Value::Float(actual) = context.execute(&function).unwrap() else {
            panic!("Float64 scalar unary result lost its QuickJS value tag");
        };
        assert_eq!(actual.to_bits(), expected_bits);
    }

    let bitnot_nan = quickjs_scalar_with_float_constant(
        &[0xbd, 0x00, BIT_NOT, 0xcb, 0x28],
        0x7ff8_0000_0000_0042,
    );
    let bitnot_nan = context.read_trusted_scalar_script(&bitnot_nan).unwrap();
    assert_eq!(context.execute(&bitnot_nan).unwrap(), Value::Int(-1));

    let string_plus = quickjs_scalar_with_string_constant(
        &[0xbd, 0x00, PLUS, 0xcb, 0x28],
        &[u16::from(b'4'), u16::from(b'2')],
        false,
    );
    let string_plus = context.read_trusted_scalar_script(&string_plus).unwrap();
    assert_eq!(context.execute(&string_plus).unwrap(), Value::Int(42));

    let int_zero_neg = context
        .read_trusted_scalar_script(&quickjs_scalar_with_code(&direct_code(&[0xb3], &[NEG])))
        .unwrap();
    let Value::Float(negative_zero) = context.execute(&int_zero_neg).unwrap() else {
        panic!("negating Int32 zero did not produce QuickJS Float64 negative zero");
    };
    assert_eq!(negative_zero.to_bits(), (-0.0_f64).to_bits());

    for (op, expected) in [
        (NEG, JsBigInt::from(-1)),
        (DEC, JsBigInt::zero()),
        (INC, JsBigInt::from(2)),
        (BIT_NOT, JsBigInt::from(-2)),
    ] {
        let code = direct_code(&[0xb0, 0x01, 0x00, 0x00, 0x00], &[op]);
        let function = context
            .read_trusted_scalar_script(&quickjs_scalar_with_code(&code))
            .unwrap();
        assert_eq!(context.execute(&function).unwrap(), Value::BigInt(expected));
    }

    // Pinned QuickJS performs unsigned opcode arithmetic in the heap-BigInt
    // decrement slow path, adding UINT32_MAX instead of subtracting one.
    // This is release behavior, not eager decoder normalization.
    let heap_bigint_dec = quickjs_scalar_with_bigint_constant(
        &[0xbd, 0x00, DEC, 0xcb, 0x28],
        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80],
    );
    let heap_bigint_dec = context
        .read_trusted_scalar_script(&heap_bigint_dec)
        .unwrap();
    assert_eq!(
        context.execute(&heap_bigint_dec).unwrap(),
        Value::BigInt(JsBigInt::parse_js_string("-9223372032559808513").unwrap())
    );

    let typeof_image = quickjs_scalar_with_string_constant(
        &[0xbd, 0x00, TYPEOF, 0xcb, 0x28],
        &[u16::from(b'x')],
        false,
    );
    let typeof_function = context.read_trusted_scalar_script(&typeof_image).unwrap();
    let first_type = expect_string_value(context.execute(&typeof_function).unwrap());
    let repeated_type = expect_string_value(context.execute(&typeof_function).unwrap());
    let atoms_after_typeof = runtime.test_atom_count();
    let string_key = runtime.intern_property_key("string").unwrap();
    let canonical_type = runtime.property_key_to_js_string(&string_key).unwrap();
    assert_eq!(first_type, JsString::from_static("string"));
    assert!(!first_type.is_wide());
    assert!(first_type.same_representation(&repeated_type));
    assert!(first_type.same_representation(&canonical_type));
    assert_eq!(runtime.test_atom_count(), atoms_after_typeof);

    // Publication is independent from execution. QuickJS accepts OP_plus in
    // the bytecode and raises TypeError only when it executes on a BigInt.
    let plus_bigint =
        quickjs_scalar_with_code(&direct_code(&[0xb0, 0x01, 0x00, 0x00, 0x00], &[PLUS, NEG]));
    let function = context.read_trusted_scalar_script(&plus_bigint).unwrap();
    for _ in 0..2 {
        assert_eq!(context.execute(&function), Err(RuntimeError::Exception));
        let (name, message) = take_error_name_and_message(&runtime, &mut context);
        assert_eq!(name, JsString::from_static("TypeError"));
        assert_eq!(
            message,
            JsString::from_static("bigint argument with unary +")
        );
        assert!(!context.has_exception());
    }
}

#[test]
fn trusted_quickjs_scalar_script_accepts_compatible_wire_spellings() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (operand, expected) in [(0x29, 41), (0xff, -1)] {
        let mut scalar = QUICKJS_SCALAR_42_BC5.to_vec();
        scalar[22] = operand;
        let function = context.read_trusted_scalar_script(&scalar).unwrap();
        assert_eq!(context.execute(&function).unwrap(), Value::Int(expected));
    }

    let mut trailing = QUICKJS_SCALAR_42_BC5.to_vec();
    trailing.extend_from_slice(&[0xde, 0xad]);
    let function = context.read_trusted_scalar_script(&trailing).unwrap();
    assert_eq!(context.execute(&function).unwrap(), Value::Int(42));

    let mut non_minimal_header = vec![0x05, 0x80, 0x00];
    non_minimal_header.extend_from_slice(&QUICKJS_SCALAR_42_BC5[2..]);
    let function = context
        .read_trusted_scalar_script(&non_minimal_header)
        .unwrap();
    assert_eq!(context.execute(&function).unwrap(), Value::Int(42));
}

#[test]
fn trusted_quickjs_scalar_script_preserves_frontier_and_malformed_provenance() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;
    let baseline_objects = runtime.heap_counts().object_nodes;
    let baseline_atoms = runtime.test_atom_count();

    // A valid BC5 Int32 data root is not a scalar Script. The public API must
    // preserve that implementation frontier instead of fabricating a parse
    // failure or publishing any heap bytecode.
    let RuntimeError::Engine(error) = context
        .read_trusted_scalar_script(&[0x05, 0x00, 0x05, 0x54])
        .unwrap_err()
    else {
        panic!("valid but unadmitted BC5 root did not return an engine error");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut unused_atom_slot = QUICKJS_SCALAR_42_BC5.to_vec();
    unused_atom_slot.splice(1..2, [0x01, 0x00]);
    let float_entry = quickjs_float_constant_entry(0.5_f64.to_bits());
    let other_float_entry = quickjs_float_constant_entry(1.5_f64.to_bits());
    let bigint_entry = quickjs_bigint_constant_entry(&[0x00, 0x00, 0x00, 0x80, 0x00]);
    let other_bigint_entry = quickjs_bigint_constant_entry(&[0x01]);
    let well_formed_unadmitted = [
        (
            "push_this scalar Script",
            quickjs_scalar_with_code(&[0x08, 0xcb, 0x28]),
        ),
        (
            "scalar Script with await outside the unary table",
            quickjs_scalar_with_code(&[0xb3, 0x89, 0xcb, 0x28]),
        ),
        (
            "scalar Script with postfix decrement outside the unary table",
            quickjs_scalar_with_code(&[0xb3, 0x8e, 0xcb, 0x28]),
        ),
        (
            "scalar Script with postfix increment outside the unary table",
            quickjs_scalar_with_code(&[0xb3, 0x8f, 0xcb, 0x28]),
        ),
        (
            "scalar Script with delete outside the unary table",
            quickjs_scalar_with_code(&[0xb3, 0x96, 0xcb, 0x28]),
        ),
        (
            "scalar Script with an unused input atom slot",
            unused_atom_slot,
        ),
        (
            "push_const8 scalar Script with an empty constant pool",
            quickjs_scalar_with_code(&[0xbd, 0x00, 0xcb, 0x28]),
        ),
        (
            "direct scalar Script with a Float64 constant",
            quickjs_scalar_with_constants(&[0xb3, 0xcb, 0x28], &[float_entry.as_slice()]),
        ),
        (
            "push_const8 scalar Script with a nonzero index",
            quickjs_scalar_with_constants(&[0xbd, 0x01, 0xcb, 0x28], &[float_entry.as_slice()]),
        ),
        (
            "push_const8 scalar Script with an extra constant",
            quickjs_scalar_with_constants(
                &[0xbd, 0x00, 0xcb, 0x28],
                &[float_entry.as_slice(), other_float_entry.as_slice()],
            ),
        ),
        (
            "push_const8 scalar Script with an Int32 constant",
            quickjs_scalar_with_constants(&[0xbd, 0x00, 0xcb, 0x28], &[&[0x05, 0x54]]),
        ),
        (
            "direct scalar Script with a BigInt constant",
            quickjs_scalar_with_constants(&[0xb3, 0xcb, 0x28], &[bigint_entry.as_slice()]),
        ),
        (
            "push_const8 BigInt scalar Script with a nonzero index",
            quickjs_scalar_with_constants(&[0xbd, 0x01, 0xcb, 0x28], &[bigint_entry.as_slice()]),
        ),
        (
            "push_const8 BigInt scalar Script with an extra constant",
            quickjs_scalar_with_constants(
                &[0xbd, 0x00, 0xcb, 0x28],
                &[bigint_entry.as_slice(), other_bigint_entry.as_slice()],
            ),
        ),
        (
            "negated BigInt constant scalar Script with a nonzero index",
            quickjs_scalar_with_constants(
                &[0xbd, 0x01, 0x8a, 0xcb, 0x28],
                &[bigint_entry.as_slice()],
            ),
        ),
        (
            "negated BigInt constant scalar Script with an extra constant",
            quickjs_scalar_with_constants(
                &[0xbd, 0x00, 0x8a, 0xcb, 0x28],
                &[bigint_entry.as_slice(), other_bigint_entry.as_slice()],
            ),
        ),
        (
            "negated direct BigInt scalar Script with a constant",
            quickjs_scalar_with_constants(
                &[0xb0, 0x01, 0x00, 0x00, 0x00, 0x8a, 0xcb, 0x28],
                &[bigint_entry.as_slice()],
            ),
        ),
    ];
    for (label, image) in well_formed_unadmitted {
        let RuntimeError::Engine(error) = context.read_trusted_scalar_script(&image).unwrap_err()
        else {
            panic!("valid {label} did not preserve the admission frontier");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(!context.has_exception());
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }

    let mut wrapping_scope_link = QUICKJS_SCALAR_42_BC5.to_vec();
    wrapping_scope_link.splice(18..19, [0x80, 0x80, 0x80, 0x80, 0x08]);
    let RuntimeError::Engine(error) = context
        .read_trusted_scalar_script(&wrapping_scope_link)
        .unwrap_err()
    else {
        panic!("compatible wrapping scope link did not preserve the admission frontier");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let RuntimeError::Engine(error) = context
        .read_trusted_scalar_script(QUICKJS_SELF_CONTAINED_MODULE_BC5)
        .unwrap_err()
    else {
        panic!("valid Module root did not preserve the admission frontier");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut wrapping_module_field = QUICKJS_SELF_CONTAINED_MODULE_BC5.to_vec();
    wrapping_module_field.splice(58..59, [0x80, 0x80, 0x80, 0x80, 0x08]);
    let RuntimeError::Engine(error) = context
        .read_trusted_scalar_script(&wrapping_module_field)
        .unwrap_err()
    else {
        panic!("compatible wrapping Module field did not preserve the admission frontier");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut truncated = QUICKJS_SCALAR_42_BC5.to_vec();
    truncated.pop();
    assert_eq!(
        context.read_trusted_scalar_script(&truncated),
        Err(RuntimeError::Exception)
    );
    assert!(context.has_exception());
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("read after the end of the buffer"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut wrong_version = QUICKJS_SCALAR_42_BC5.to_vec();
    wrong_version[0] = 4;
    assert_eq!(
        context.read_trusted_scalar_script(&wrong_version),
        Err(RuntimeError::Exception)
    );
    assert!(context.has_exception());
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("invalid version (4 expected=5)"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    assert_eq!(
        context.read_trusted_scalar_script(&[0x05, 0x80, 0x80, 0x80, 0x80, 0x80]),
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
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut invalid_atom = QUICKJS_SCALAR_42_BC5.to_vec();
    invalid_atom.splice(6..8, [0xe6, 0x03]);
    assert_eq!(
        context.read_trusted_scalar_script(&invalid_atom),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("invalid atom index (pos=8)"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    assert_eq!(
        context.read_trusted_scalar_script(&[0x05, 0x01, 0x80, 0x80, 0x80, 0x80, 0x08]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("InternalError"),
            JsString::from_static("string too long"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut negative_bytecode_length = QUICKJS_SCALAR_42_BC5.to_vec();
    negative_bytecode_length.splice(15..16, [0x80, 0x80, 0x80, 0x80, 0x08]);
    assert_eq!(
        context.read_trusted_scalar_script(&negative_bytecode_length),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("InternalError"),
            JsString::from_static("out of memory"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    let mut negative_module_count = QUICKJS_SELF_CONTAINED_MODULE_BC5.to_vec();
    negative_module_count.splice(55..56, [0x80, 0x80, 0x80, 0x80, 0x08]);
    assert_eq!(
        context.read_trusted_scalar_script(&negative_module_count),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("InternalError"),
            JsString::from_static("out of memory"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);

    assert_eq!(
        context.read_trusted_scalar_script(&[0x05, 0x00, 0x13, 0x00]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("SyntaxError"),
            JsString::from_static("invalid object reference (0 >= 0)"),
        )
    );
    assert!(!context.has_exception());
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);
}

#[test]
fn trusted_quickjs_scalar_script_preserves_pinned_data_reader_errors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;
    let cases: [(&[u8], &str, &str); 10] = [
        (
            &[
                0x05, 0x00, 0x12, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00,
                0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
            ],
            "TypeError",
            "cannot convert to object",
        ),
        (
            &[
                0x05, 0x00, 0x11, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00,
                0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
            ],
            "TypeError",
            "Number tag expected for date",
        ),
        (
            &[
                0x05, 0x00, 0x0e, 0x02, 0x01, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01,
                0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb,
                0x28,
            ],
            "TypeError",
            "ArrayBuffer object expected",
        ),
        (
            &[0x05, 0x00, 0x0f, 0x01, 0x00],
            "TypeError",
            "invalid array buffer",
        ),
        (
            &[0x05, 0x00, 0x0e, 0xff],
            "TypeError",
            "invalid typed array",
        ),
        (
            &[0x05, 0x00, 0x0e, 0x02, 0x00, 0x00, 0x01],
            "TypeError",
            "ArrayBuffer object expected",
        ),
        (
            &[0x05, 0x00, 0x12, 0x01],
            "TypeError",
            "cannot convert to object",
        ),
        (
            &[0x05, 0x00, 0x11, 0x01],
            "TypeError",
            "Number tag expected for date",
        ),
        (
            &[0x05, 0x00, 0x0e, 0x04, 0x00, 0x01, 0x0f, 0x00, 0x00],
            "RangeError",
            "invalid offset",
        ),
        (
            &[0x05, 0x00, 0x0e, 0x02, 0x01, 0x01, 0x0f, 0x01, 0x01, 0x00],
            "RangeError",
            "invalid length",
        ),
    ];

    for (object, expected_name, expected_message) in cases {
        assert_eq!(
            context.read_trusted_scalar_script(object),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_name_and_message(&runtime, &mut context),
            (
                JsString::try_from_utf8(expected_name).unwrap(),
                JsString::try_from_utf8(expected_message).unwrap(),
            )
        );
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline);
    }
}
