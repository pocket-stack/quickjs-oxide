use super::*;

#[test]
fn trusted_quickjs_ordinary_push_this_is_exact_typed_and_realm_correct() {
    for (label, wire, expected_fnv) in [
        (
            "natural strict",
            QUICKJS_NATURAL_STRICT_PUSH_THIS_BC5,
            0x4ec7_e018_7375_d810,
        ),
        (
            "natural sloppy",
            QUICKJS_NATURAL_SLOPPY_PUSH_THIS_BC5,
            0x4e7f_8f98_adff_8463,
        ),
        (
            "minimal strict",
            QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5,
            0x3c3e_393f_ef88_3bc5,
        ),
        (
            "minimal sloppy",
            QUICKJS_ORDINARY_SLOPPY_PUSH_THIS_BC5,
            0x0e24_85c9_7eea_9cfa,
        ),
    ] {
        assert_eq!(
            wire.len(),
            if label.starts_with("natural") { 47 } else { 41 }
        );
        assert_eq!(fnv1a64(wire), expected_fnv, "{label}");
    }
    assert_eq!(
        &QUICKJS_NATURAL_STRICT_PUSH_THIS_BC5[43..],
        &[8, 199, 195, 40]
    );
    assert_eq!(
        &QUICKJS_NATURAL_SLOPPY_PUSH_THIS_BC5[43..],
        &[8, 199, 195, 40]
    );
    assert_eq!(&QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5[39..], &[8, 40]);
    assert_eq!(&QUICKJS_ORDINARY_SLOPPY_PUSH_THIS_BC5[39..], &[8, 40]);

    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_global = defining.global_object().unwrap();
    let caller_global = caller.global_object().unwrap();
    assert_ne!(defining_global, caller_global);
    let baseline = runtime.heap_counts().function_bytecode_nodes;

    let strict_natural = defining
        .read_trusted_ordinary_function(QUICKJS_NATURAL_STRICT_PUSH_THIS_BC5, 0)
        .unwrap();
    let strict_minimal = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5, 0)
        .unwrap();
    let sloppy_natural = defining
        .read_trusted_ordinary_function(QUICKJS_NATURAL_SLOPPY_PUSH_THIS_BC5, 0)
        .unwrap();
    let sloppy_minimal = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_SLOPPY_PUSH_THIS_BC5, 0)
        .unwrap();
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline + 4);

    for (label, function, expected_strict, natural) in [
        ("natural strict", &strict_natural, true, true),
        ("minimal strict", &strict_minimal, true, false),
        ("natural sloppy", &sloppy_natural, false, true),
        ("minimal sloppy", &sloppy_minimal, false, false),
    ] {
        assert_eq!(
            runtime.get_prototype_of(function.as_object()).unwrap(),
            Some(defining.function_prototype().unwrap()),
            "{label}"
        );
        let CallableExecution::Bytecode { bytecode, .. } =
            runtime.bytecode_for_callable(function).unwrap()
        else {
            panic!("{label} raw8 function did not publish bytecode");
        };
        let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
        if natural {
            assert!(matches!(
                snapshot.code.as_ref(),
                [
                    Instruction::PushThis,
                    Instruction::PutLocal(0),
                    Instruction::GetLocal(0),
                    Instruction::Return,
                ]
            ));
            assert_eq!(snapshot.metadata.local_count, 1, "{label}");
        } else {
            assert!(matches!(
                snapshot.code.as_ref(),
                [Instruction::PushThis, Instruction::Return]
            ));
            assert_eq!(snapshot.metadata.local_count, 0, "{label}");
        }
        assert!(snapshot.constants.is_empty(), "{label}");
        assert_eq!(snapshot.metadata.argument_count, 0, "{label}");
        assert_eq!(snapshot.metadata.defined_argument_count, 0, "{label}");
        assert_eq!(snapshot.metadata.max_stack, 1, "{label}");
        assert_eq!(snapshot.metadata.strict, expected_strict, "{label}");
        assert!(snapshot.metadata.strip_variable_debug, "{label}");
        assert_eq!(
            snapshot.metadata.function_kind,
            FunctionKind::Normal,
            "{label}"
        );
        assert!(snapshot.metadata.has_prototype, "{label}");
        assert_eq!(
            snapshot.metadata.constructor_kind,
            ConstructorKind::Base,
            "{label}"
        );
        assert!(!snapshot.metadata.arguments_forbidden, "{label}");
    }

    let Value::Object(receiver) = caller.eval("({ raw8: true })").unwrap() else {
        panic!("raw8 receiver probe was not an object");
    };
    for (label, function) in [
        ("natural strict", &strict_natural),
        ("minimal strict", &strict_minimal),
    ] {
        assert_eq!(
            caller.call(function, Value::Undefined, &[]).unwrap(),
            Value::Undefined,
            "{label} undefined"
        );
        assert_eq!(
            caller.call(function, Value::Null, &[]).unwrap(),
            Value::Null,
            "{label} null"
        );
        assert_eq!(
            caller.call(function, Value::Int(8), &[]).unwrap(),
            Value::Int(8),
            "{label} primitive"
        );
        assert_eq!(
            caller
                .call(function, Value::Object(receiver.clone()), &[])
                .unwrap(),
            Value::Object(receiver.clone()),
            "{label} object identity"
        );
    }

    let symbol = runtime
        .new_symbol(Some(JsString::from_static("raw8")))
        .unwrap();
    let primitive_cases = [
        (
            "Boolean",
            Value::Bool(true),
            defining.boolean_prototype().unwrap(),
            caller.boolean_prototype().unwrap(),
        ),
        (
            "integer Number",
            Value::Int(8),
            defining.number_prototype().unwrap(),
            caller.number_prototype().unwrap(),
        ),
        (
            "floating Number",
            Value::Float(8.5),
            defining.number_prototype().unwrap(),
            caller.number_prototype().unwrap(),
        ),
        (
            "String",
            Value::String(JsString::from_static("raw8")),
            defining.string_prototype().unwrap(),
            caller.string_prototype().unwrap(),
        ),
        (
            "BigInt",
            Value::BigInt(JsBigInt::from(8)),
            defining.bigint_prototype().unwrap(),
            caller.bigint_prototype().unwrap(),
        ),
        (
            "Symbol",
            Value::Symbol(symbol),
            defining.symbol_prototype().unwrap(),
            caller.symbol_prototype().unwrap(),
        ),
    ];
    for (function_label, function) in [
        ("natural sloppy", &sloppy_natural),
        ("minimal sloppy", &sloppy_minimal),
    ] {
        for nullish in [Value::Undefined, Value::Null] {
            assert_eq!(
                caller.call(function, nullish, &[]).unwrap(),
                Value::Object(defining_global.clone()),
                "{function_label} nullish receiver"
            );
        }
        assert_eq!(
            caller
                .call(function, Value::Object(receiver.clone()), &[])
                .unwrap(),
            Value::Object(receiver.clone()),
            "{function_label} object identity"
        );
        for (primitive_label, primitive, defining_prototype, caller_prototype) in &primitive_cases {
            assert_ne!(
                defining_prototype, caller_prototype,
                "{primitive_label} realms"
            );
            let Value::Object(first) = caller.call(function, primitive.clone(), &[]).unwrap()
            else {
                panic!("{function_label} did not box {primitive_label}");
            };
            let Value::Object(second) = caller.call(function, primitive.clone(), &[]).unwrap()
            else {
                panic!("{function_label} did not box {primitive_label} twice");
            };
            assert_ne!(
                first, second,
                "{function_label} reused a {primitive_label} wrapper"
            );
            assert_eq!(
                runtime.get_prototype_of(&first).unwrap(),
                Some(defining_prototype.clone()),
                "{function_label} {primitive_label} defining realm"
            );
            assert_ne!(
                runtime.get_prototype_of(&first).unwrap(),
                Some(caller_prototype.clone()),
                "{function_label} {primitive_label} caller realm"
            );
            let value_of =
                property_callable(&runtime, &mut defining, defining_prototype, "valueOf");
            assert!(
                defining
                    .call(&value_of, Value::Object(first), &[])
                    .unwrap()
                    .same_value(primitive),
                "{function_label} changed the boxed {primitive_label} payload"
            );
        }
    }
    assert_ne!(defining_global, caller_global);
    assert!(!defining.has_exception());
    assert!(!caller.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_push_this_protocol_rejects_transactionally_and_retries() {
    assert_eq!(QUICKJS_DUPLICATE_PUSH_THIS_BC5.len(), 43);
    assert_eq!(
        fnv1a64(QUICKJS_DUPLICATE_PUSH_THIS_BC5),
        0x920d_e09a_af63_833e
    );
    assert_eq!(&QUICKJS_DUPLICATE_PUSH_THIS_BC5[39..], &[8, 8, 169, 40]);
    assert_eq!(QUICKJS_REENTER_PUSH_THIS_BC5.len(), 65);
    assert_eq!(
        fnv1a64(QUICKJS_REENTER_PUSH_THIS_BC5),
        0xfa10_0ff2_b085_4673
    );

    let mut nonzero = QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5.to_vec();
    nonzero[37] = 3;
    nonzero.insert(39, 177);
    assert_eq!(&nonzero[39..], &[177, 8, 40]);

    let mut cases = vec![
        (
            "duplicate raw8",
            QUICKJS_DUPLICATE_PUSH_THIS_BC5.to_vec(),
            "push_this must occur exactly once",
        ),
        (
            "re-enter raw8",
            QUICKJS_REENTER_PUSH_THIS_BC5.to_vec(),
            "must not explicitly target the push_this prologue",
        ),
    ];
    cases.push((
        "nonzero raw8",
        nonzero,
        "push_this must be typed instruction zero",
    ));
    let branch_encodings = [
        ("if_false", vec![104, 0xfe, 0xff, 0xff, 0xff]),
        ("if_true", vec![105, 0xfe, 0xff, 0xff, 0xff]),
        ("goto", vec![106, 0xfe, 0xff, 0xff, 0xff]),
        ("if_false8", vec![232, 0xfe]),
        ("if_true8", vec![233, 0xfe]),
        ("goto8", vec![234, 0xfe]),
        ("goto16", vec![235, 0xfe, 0xff]),
    ];
    for (label, branch_encoding) in branch_encodings {
        let mut branch = QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5.to_vec();
        branch[37] = u8::try_from(branch_encoding.len() + 2).unwrap();
        branch.truncate(39);
        branch.push(8);
        branch.extend_from_slice(&branch_encoding);
        branch.push(41);
        cases.push((
            label,
            branch,
            "must not explicitly target the push_this prologue",
        ));
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    for (label, wire, expected) in cases {
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&wire, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert!(error.message().contains(expected), "{label}: {error}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let retry = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5, 0)
        .unwrap();
    assert_eq!(
        context.call(&retry, Value::Int(42), &[]).unwrap(),
        Value::Int(42)
    );
    assert!(!context.has_exception());
}
