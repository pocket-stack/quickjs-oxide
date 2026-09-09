use super::*;

#[test]
fn trusted_quickjs_ordinary_to_propkey_is_exact_typed_and_canonical() {
    for (label, wire, expected_len, expected_fnv) in [
        (
            "natural strict",
            QUICKJS_NATURAL_STRICT_TO_PROPKEY_BC5,
            50,
            0x83c3_3a69_f73e_737c,
        ),
        (
            "natural sloppy",
            QUICKJS_NATURAL_SLOPPY_TO_PROPKEY_BC5,
            50,
            0x6a17_06c9_ae12_6361,
        ),
        (
            "manual strict",
            QUICKJS_ORDINARY_STRICT_TO_PROPKEY_BC5,
            46,
            0xc7ed_0972_0c7c_faa1,
        ),
        (
            "manual sloppy",
            QUICKJS_ORDINARY_SLOPPY_TO_PROPKEY_BC5,
            46,
            0xdd8b_bd33_3d59_5b1c,
        ),
    ] {
        assert_eq!(wire.len(), expected_len, "{label}");
        assert_eq!(fnv1a64(wire), expected_fnv, "{label}");
    }
    assert_eq!(
        &QUICKJS_NATURAL_STRICT_TO_PROPKEY_BC5[43..],
        &[0x0b, 0xcf, 0x70, 0xb4, 0x4e, 0x0e, 0x28]
    );
    assert_eq!(
        &QUICKJS_NATURAL_SLOPPY_TO_PROPKEY_BC5[43..],
        &[0x0b, 0xcf, 0x70, 0xb4, 0x4e, 0x0e, 0x28]
    );
    assert_eq!(
        &QUICKJS_ORDINARY_STRICT_TO_PROPKEY_BC5[43..],
        &[0xcf, 0x70, 0x28]
    );
    assert_eq!(
        &QUICKJS_ORDINARY_SLOPPY_TO_PROPKEY_BC5[43..],
        &[0xcf, 0x70, 0x28]
    );

    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    // The natural compiler witnesses authenticate raw112 but still contain
    // raw78. Their rejection must identify that later operation, not raw112.
    for (label, wire) in [
        ("natural strict", QUICKJS_NATURAL_STRICT_TO_PROPKEY_BC5),
        ("natural sloppy", QUICKJS_NATURAL_SLOPPY_TO_PROPKEY_BC5),
    ] {
        let RuntimeError::Engine(error) = defining
            .read_trusted_ordinary_function(wire, 0)
            .unwrap_err()
        else {
            panic!("{label} unexpectedly entered the property-free cohort");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert!(
            error.message().contains("define_array_el"),
            "{label}: {error}"
        );
        assert!(!error.message().contains("to_propkey"), "{label}: {error}");
        assert!(!defining.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let strict = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_STRICT_TO_PROPKEY_BC5, 0)
        .unwrap();
    let sloppy = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_SLOPPY_TO_PROPKEY_BC5, 0)
        .unwrap();
    assert_eq!(
        runtime.get_prototype_of(strict.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + 2
    );

    for (label, function, expected_strict) in
        [("strict", &strict, true), ("sloppy", &sloppy, false)]
    {
        let CallableExecution::Bytecode { bytecode, .. } =
            runtime.bytecode_for_callable(function).unwrap()
        else {
            panic!("{label} raw112 function did not publish bytecode");
        };
        let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
        assert!(matches!(
            snapshot.code.as_ref(),
            [
                Instruction::GetArg(0),
                Instruction::ToPropKey,
                Instruction::Return,
            ]
        ));
        assert!(snapshot.constants.is_empty(), "{label}");
        assert_eq!(snapshot.metadata.argument_count, 1, "{label}");
        assert_eq!(snapshot.metadata.defined_argument_count, 1, "{label}");
        assert_eq!(snapshot.metadata.local_count, 0, "{label}");
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

    let primitive_cases = [
        (
            "undefined",
            Value::Undefined,
            Value::String(JsString::from_static("undefined")),
        ),
        (
            "null",
            Value::Null,
            Value::String(JsString::from_static("null")),
        ),
        (
            "Boolean",
            Value::Bool(false),
            Value::String(JsString::from_static("false")),
        ),
        ("integer", Value::Int(17), Value::Int(17)),
        (
            "floating Number",
            Value::Float(8.5),
            Value::String(JsString::from_static("8.5")),
        ),
        (
            "String",
            Value::String(JsString::from_static("raw112")),
            Value::String(JsString::from_static("raw112")),
        ),
        (
            "BigInt",
            Value::BigInt(JsBigInt::from(-112)),
            Value::String(JsString::from_static("-112")),
        ),
    ];
    for (function_label, function) in [("strict", &strict), ("sloppy", &sloppy)] {
        for (primitive_label, input, expected) in &primitive_cases {
            assert_eq!(
                caller
                    .call(function, Value::Undefined, &[input.clone()])
                    .unwrap(),
                expected.clone(),
                "{function_label} {primitive_label}"
            );
            assert!(
                !caller.has_exception(),
                "{function_label} {primitive_label}"
            );
        }
    }

    let symbol = runtime
        .new_symbol(Some(JsString::from_static("raw112")))
        .unwrap();
    for (label, function) in [("strict", &strict), ("sloppy", &sloppy)] {
        let converted = caller
            .call(function, Value::Undefined, &[Value::Symbol(symbol.clone())])
            .unwrap();
        assert!(
            converted.same_value(&Value::Symbol(symbol.clone())),
            "{label} changed Symbol identity"
        );
        assert!(!caller.has_exception(), "{label}");
    }
    assert!(!defining.has_exception());
    assert!(!caller.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_to_propkey_observability_and_reentry_match_quickjs() {
    assert_eq!(QUICKJS_DUPLICATE_TO_PROPKEY_BC5.len(), 47);
    assert_eq!(
        fnv1a64(QUICKJS_DUPLICATE_TO_PROPKEY_BC5),
        0xc3d5_f481_5e80_7dfc
    );
    assert_eq!(
        &QUICKJS_DUPLICATE_TO_PROPKEY_BC5[43..],
        &[0xcf, 0x70, 0x70, 0x28]
    );
    assert_eq!(QUICKJS_REENTER_TO_PROPKEY_BC5.len(), 65);
    assert_eq!(
        fnv1a64(QUICKJS_REENTER_TO_PROPKEY_BC5),
        0xedcc_1b5d_91f5_e46d
    );

    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let function = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_STRICT_TO_PROPKEY_BC5, 0)
        .unwrap();
    let sloppy = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_SLOPPY_TO_PROPKEY_BC5, 0)
        .unwrap();
    let duplicate = defining
        .read_trusted_ordinary_function(QUICKJS_DUPLICATE_TO_PROPKEY_BC5, 0)
        .unwrap();
    let reentry = defining
        .read_trusted_ordinary_function(QUICKJS_REENTER_TO_PROPKEY_BC5, 0)
        .unwrap();

    let CallableExecution::Bytecode {
        bytecode: duplicate_bytecode,
        ..
    } = runtime.bytecode_for_callable(&duplicate).unwrap()
    else {
        panic!("duplicate raw112 function did not publish bytecode");
    };
    let duplicate_snapshot = runtime
        .snapshot_function_bytecode(&duplicate_bytecode)
        .unwrap();
    assert!(matches!(
        duplicate_snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::ToPropKey,
            Instruction::ToPropKey,
            Instruction::Return,
        ]
    ));
    assert!(duplicate_snapshot.constants.is_empty());
    drop(duplicate_snapshot);

    let CallableExecution::Bytecode {
        bytecode: reentry_bytecode,
        ..
    } = runtime.bytecode_for_callable(&reentry).unwrap()
    else {
        panic!("re-entered raw112 function did not publish bytecode");
    };
    let reentry_snapshot = runtime
        .snapshot_function_bytecode(&reentry_bytecode)
        .unwrap();
    assert!(matches!(
        reentry_snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::ToPropKey,
            Instruction::GetArg(1),
            Instruction::IfFalse(9),
            Instruction::Drop,
            Instruction::PushFalse,
            Instruction::PutArg(1),
            Instruction::GetArg(0),
            Instruction::Goto(1),
            Instruction::Return,
        ]
    ));
    assert!(reentry_snapshot.constants.is_empty());
    drop(reentry_snapshot);

    let global = caller.global_object().unwrap();
    let raw_key = runtime.intern_property_key("rawToPropKey").unwrap();
    assert!(
        caller
            .define_own_property(
                &global,
                &raw_key,
                &data_descriptor(
                    Value::Object(function.as_object().clone()),
                    true,
                    true,
                    true,
                ),
            )
            .unwrap()
    );

    let Value::Object(to_primitive_probe) = caller
        .eval(
            r#"
                (function () {
                    globalThis.__raw112_log = "";
                    return {
                        [Symbol.toPrimitive]: function (hint) {
                            __raw112_log += "@@:" + hint + "|";
                            return "symbolic";
                        },
                        toString: function () { __raw112_log += "toString|"; return "wrong"; },
                        valueOf: function () { __raw112_log += "valueOf|"; return "wrong"; }
                    };
                })()
            "#,
        )
        .unwrap()
    else {
        panic!("raw112 @@toPrimitive probe was not an object");
    };
    assert_eq!(
        caller
            .call(
                &function,
                Value::Undefined,
                &[Value::Object(to_primitive_probe.clone())],
            )
            .unwrap(),
        Value::String(JsString::from_static("symbolic"))
    );
    assert_eq!(
        caller.eval("__raw112_log").unwrap(),
        Value::String(JsString::from_static("@@:string|"))
    );
    assert_eq!(
        caller.eval("__raw112_log = ''").unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert_eq!(
        caller
            .call(
                &sloppy,
                Value::Undefined,
                &[Value::Object(to_primitive_probe)],
            )
            .unwrap(),
        Value::String(JsString::from_static("symbolic"))
    );
    assert_eq!(
        caller.eval("__raw112_log").unwrap(),
        Value::String(JsString::from_static("@@:string|"))
    );

    let Value::Object(first_fallback_probe) = caller
        .eval(
            r#"
                (function () {
                    globalThis.__raw112_log = "";
                    return {
                        toString: function () { __raw112_log += "toString|"; return "first"; },
                        valueOf: function () { __raw112_log += "valueOf|"; return "wrong"; }
                    };
                })()
            "#,
        )
        .unwrap()
    else {
        panic!("raw112 first fallback probe was not an object");
    };
    assert_eq!(
        caller
            .call(
                &function,
                Value::Undefined,
                &[Value::Object(first_fallback_probe)],
            )
            .unwrap(),
        Value::String(JsString::from_static("first"))
    );
    assert_eq!(
        caller.eval("__raw112_log").unwrap(),
        Value::String(JsString::from_static("toString|"))
    );

    let Value::Object(fallback_probe) = caller
        .eval(
            r#"
                (function () {
                    globalThis.__raw112_log = "";
                    return {
                        toString: function () { __raw112_log += "toString|"; return {}; },
                        valueOf: function () { __raw112_log += "valueOf|"; return 73; }
                    };
                })()
            "#,
        )
        .unwrap()
    else {
        panic!("raw112 fallback probe was not an object");
    };
    assert_eq!(
        caller
            .call(
                &function,
                Value::Undefined,
                &[Value::Object(fallback_probe)],
            )
            .unwrap(),
        Value::String(JsString::from_static("73"))
    );
    assert_eq!(
        caller.eval("__raw112_log").unwrap(),
        Value::String(JsString::from_static("toString|valueOf|"))
    );

    let Value::Object(sentinel) = caller.eval("globalThis.__raw112_sentinel = {}").unwrap() else {
        panic!("raw112 throw sentinel was not an object");
    };
    let Value::Object(throwing_probe) = caller
        .eval("({ [Symbol.toPrimitive]: function () { throw __raw112_sentinel; } })")
        .unwrap()
    else {
        panic!("raw112 throwing probe was not an object");
    };
    assert_eq!(
        caller.call(
            &function,
            Value::Undefined,
            &[Value::Object(throwing_probe)],
        ),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(thrown)) = caller.take_exception().unwrap() else {
        panic!("raw112 did not preserve the thrown object");
    };
    assert_eq!(thrown, sentinel);
    assert!(!caller.has_exception());

    let Value::Object(defining_type_error_prototype) =
        defining.eval("TypeError.prototype").unwrap()
    else {
        panic!("defining TypeError.prototype was not an object");
    };
    let Value::Object(caller_type_error_prototype) = caller.eval("TypeError.prototype").unwrap()
    else {
        panic!("caller TypeError.prototype was not an object");
    };
    assert_ne!(defining_type_error_prototype, caller_type_error_prototype);
    let Value::Object(non_primitive_probe) = caller
        .eval("({ [Symbol.toPrimitive]: function () { return {}; } })")
        .unwrap()
    else {
        panic!("raw112 non-primitive probe was not an object");
    };
    assert_eq!(
        caller.call(
            &function,
            Value::Undefined,
            &[Value::Object(non_primitive_probe)],
        ),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(type_error)) = caller.take_exception().unwrap() else {
        panic!("raw112 non-primitive result did not materialize a TypeError");
    };
    assert_eq!(
        runtime.get_prototype_of(&type_error).unwrap(),
        Some(defining_type_error_prototype)
    );
    assert_ne!(
        runtime.get_prototype_of(&type_error).unwrap(),
        Some(caller_type_error_prototype)
    );
    assert!(!caller.has_exception());

    let Value::Object(nested_probe) = caller
        .eval(
            r#"
                (function () {
                    globalThis.__raw112_log = "";
                    var inner = {
                        [Symbol.toPrimitive]: function (hint) {
                            __raw112_log += "inner:" + hint + "|";
                            return "nested";
                        }
                    };
                    return {
                        [Symbol.toPrimitive]: function (hint) {
                            __raw112_log += "outer:" + hint + "|";
                            return rawToPropKey(inner);
                        }
                    };
                })()
            "#,
        )
        .unwrap()
    else {
        panic!("raw112 nested-reentry probe was not an object");
    };
    assert_eq!(
        caller
            .call(&function, Value::Undefined, &[Value::Object(nested_probe)],)
            .unwrap(),
        Value::String(JsString::from_static("nested"))
    );
    assert_eq!(
        caller.eval("__raw112_log").unwrap(),
        Value::String(JsString::from_static("outer:string|inner:string|"))
    );

    let Value::Object(duplicate_probe) = caller
        .eval(
            r#"
                (function () {
                    globalThis.__raw112_count = 0;
                    return {
                        [Symbol.toPrimitive]: function (hint) {
                            if (hint !== "string") throw "wrong hint";
                            __raw112_count++;
                            return "duplicate";
                        }
                    };
                })()
            "#,
        )
        .unwrap()
    else {
        panic!("duplicate raw112 probe was not an object");
    };
    assert_eq!(
        caller
            .call(
                &duplicate,
                Value::Undefined,
                &[Value::Object(duplicate_probe)],
            )
            .unwrap(),
        Value::String(JsString::from_static("duplicate"))
    );
    assert_eq!(caller.eval("__raw112_count").unwrap(), Value::Int(1));

    let Value::Object(reentry_probe) = caller
        .eval(
            r#"
                (function () {
                    globalThis.__raw112_count = 0;
                    return {
                        [Symbol.toPrimitive]: function (hint) {
                            if (hint !== "string") throw "wrong hint";
                            __raw112_count++;
                            return "loop";
                        }
                    };
                })()
            "#,
        )
        .unwrap()
    else {
        panic!("re-entered raw112 probe was not an object");
    };
    assert_eq!(
        caller
            .call(
                &reentry,
                Value::Undefined,
                &[Value::Object(reentry_probe), Value::Bool(true)],
            )
            .unwrap(),
        Value::String(JsString::from_static("loop"))
    );
    assert_eq!(caller.eval("__raw112_count").unwrap(), Value::Int(2));
    assert!(!defining.has_exception());
    assert!(!caller.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_to_propkey_verification_rolls_back_and_retries() {
    assert_eq!(QUICKJS_UNDERFLOW_TO_PROPKEY_BC5.len(), 45);
    assert_eq!(
        fnv1a64(QUICKJS_UNDERFLOW_TO_PROPKEY_BC5),
        0x72e4_9b7e_05fe_b73d
    );
    assert_eq!(&QUICKJS_UNDERFLOW_TO_PROPKEY_BC5[43..], &[0x70, 0x28]);

    let mut undersized_stack = QUICKJS_ORDINARY_STRICT_TO_PROPKEY_BC5.to_vec();
    undersized_stack[33] = 0;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    for (label, image, expected) in [
        (
            "raw112 stack underflow",
            QUICKJS_UNDERFLOW_TO_PROPKEY_BC5.to_vec(),
            "bytecode stack underflow",
        ),
        (
            "raw112 max-stack underdeclaration",
            undersized_stack,
            "declared maximum stack is smaller than required",
        ),
    ] {
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert!(
            error.message().starts_with(
                "trusted QuickJS ordinary leaf is not admitted by typed verification:"
            ),
            "{label}: {error}"
        );
        assert!(error.message().contains(expected), "{label}: {error}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let retry = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_STRICT_TO_PROPKEY_BC5, 0)
        .unwrap();
    assert_eq!(
        context
            .call(&retry, Value::Undefined, &[Value::Bool(false)])
            .unwrap(),
        Value::String(JsString::from_static("false"))
    );
    assert!(!context.has_exception());
}
