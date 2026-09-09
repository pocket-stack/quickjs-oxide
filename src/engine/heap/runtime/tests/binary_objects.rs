use super::*;

#[test]
fn trusted_quickjs_ordinary_object_is_natural_fresh_and_defining_realm_owned() {
    assert_eq!(QUICKJS_ORDINARY_OBJECT_BC5.len(), 41);
    assert_eq!(fnv1a64(QUICKJS_ORDINARY_OBJECT_BC5), 0x3c41_af3f_ef8b_3a1e);

    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object_prototype = defining.object_prototype().unwrap();
    let caller_object_prototype = caller.object_prototype().unwrap();
    assert_ne!(defining_object_prototype, caller_object_prototype);
    let baseline = runtime.heap_counts();

    let function = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_OBJECT_BC5, 0)
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
        panic!("trusted raw11 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [Instruction::Object, Instruction::Return]
    ));
    assert!(snapshot.constants.is_empty());
    assert_eq!(snapshot.metadata.argument_count, 0);
    assert_eq!(snapshot.metadata.defined_argument_count, 0);
    assert_eq!(snapshot.metadata.local_count, 0);
    assert_eq!(snapshot.metadata.max_stack, 1);
    assert!(snapshot.metadata.strict);
    assert!(snapshot.metadata.strip_variable_debug);
    assert_eq!(snapshot.metadata.function_kind, FunctionKind::Normal);
    assert!(snapshot.metadata.has_prototype);
    assert_eq!(snapshot.metadata.constructor_kind, ConstructorKind::Base);
    assert!(!snapshot.metadata.arguments_forbidden);
    drop(snapshot);

    let Value::Object(first) = defining.call(&function, Value::Undefined, &[]).unwrap() else {
        panic!("first raw11 call did not return an Object");
    };
    let Value::Object(second) = defining.call(&function, Value::Undefined, &[]).unwrap() else {
        panic!("second raw11 call did not return an Object");
    };
    let Value::Object(cross_realm) = caller.call(&function, Value::Undefined, &[]).unwrap() else {
        panic!("cross-realm raw11 call did not return an Object");
    };
    let Value::Object(repeated_cross_realm) =
        caller.call(&function, Value::Undefined, &[]).unwrap()
    else {
        panic!("repeated cross-realm raw11 call did not return an Object");
    };

    let objects = [&first, &second, &cross_realm, &repeated_cross_realm];
    for (index, object) in objects.iter().enumerate() {
        assert_eq!(
            runtime.get_prototype_of(object).unwrap(),
            Some(defining_object_prototype.clone()),
            "raw11 result {index} used the wrong realm prototype"
        );
        assert_eq!(runtime.own_property_keys(object).unwrap(), []);
        assert!(runtime.is_extensible(object).unwrap());
        for other in objects.iter().skip(index + 1) {
            assert_ne!(object, other, "raw11 reused an Object allocation");
        }
    }
    assert_ne!(
        runtime.get_prototype_of(&cross_realm).unwrap(),
        Some(caller_object_prototype)
    );
    assert!(!defining.has_exception());
    assert!(!caller.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_to_object_is_natural_exact_and_realm_correct() {
    assert_eq!(QUICKJS_NATURAL_TO_OBJECT_BC5.len(), 56);
    assert_eq!(
        fnv1a64(QUICKJS_NATURAL_TO_OBJECT_BC5),
        0x65a8_b3d0_d7ed_115a
    );
    assert_eq!(QUICKJS_ORDINARY_TO_OBJECT_BC5.len(), 46);
    assert_eq!(
        fnv1a64(QUICKJS_ORDINARY_TO_OBJECT_BC5),
        0xc84f_8772_0cd0_9b16
    );
    assert_eq!(&QUICKJS_ORDINARY_TO_OBJECT_BC5[43..], &[0xcf, 0x6f, 0x28]);

    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let baseline = runtime.heap_counts().function_bytecode_nodes;
    let natural = defining
        .read_trusted_ordinary_function(QUICKJS_NATURAL_TO_OBJECT_BC5, 0)
        .unwrap();
    let function = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TO_OBJECT_BC5, 0)
        .unwrap();
    assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline + 2);
    assert_eq!(
        runtime.get_prototype_of(function.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap())
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted raw111 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::ToObject,
            Instruction::Return,
        ]
    ));
    assert!(snapshot.constants.is_empty());
    assert_eq!(snapshot.metadata.argument_count, 1);
    assert_eq!(snapshot.metadata.defined_argument_count, 1);
    assert_eq!(snapshot.metadata.local_count, 0);
    assert_eq!(snapshot.metadata.max_stack, 1);
    assert!(snapshot.metadata.strict);
    assert!(snapshot.metadata.strip_variable_debug);
    assert_eq!(snapshot.metadata.function_kind, FunctionKind::Normal);
    assert!(snapshot.metadata.has_prototype);
    assert_eq!(snapshot.metadata.constructor_kind, ConstructorKind::Base);
    assert!(!snapshot.metadata.arguments_forbidden);
    drop(snapshot);

    let Value::Object(probe) = caller
        .eval(
            r#"
                (function () {
                    globalThis.__raw111_coercions = 0;
                    var probe = {};
                    probe.valueOf = function () { __raw111_coercions++; return 1; };
                    probe.toString = function () { __raw111_coercions++; return "probe"; };
                    probe[Symbol.toPrimitive] = function () { __raw111_coercions++; return 2; };
                    return probe;
                })()
            "#,
        )
        .unwrap()
    else {
        panic!("raw111 coercion probe was not an object");
    };
    assert_eq!(
        caller
            .call(&function, Value::Undefined, &[Value::Object(probe.clone())],)
            .unwrap(),
        Value::Object(probe.clone())
    );
    assert_eq!(
        caller
            .call(&natural, Value::Undefined, &[Value::Object(probe.clone())],)
            .unwrap(),
        Value::Object(probe)
    );
    assert_eq!(caller.eval("__raw111_coercions").unwrap(), Value::Int(0));
    assert_eq!(
        caller
            .call(&natural, Value::Undefined, &[Value::Int(17)])
            .unwrap(),
        Value::Int(17),
        "the natural source returns its original argument after ToObject"
    );

    let symbol = runtime
        .new_symbol(Some(JsString::from_static("raw111")))
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
            Value::Int(7),
            defining.number_prototype().unwrap(),
            caller.number_prototype().unwrap(),
        ),
        (
            "floating Number",
            Value::Float(1.5),
            defining.number_prototype().unwrap(),
            caller.number_prototype().unwrap(),
        ),
        (
            "String",
            Value::String(JsString::from_static("raw111")),
            defining.string_prototype().unwrap(),
            caller.string_prototype().unwrap(),
        ),
        (
            "BigInt",
            Value::BigInt(JsBigInt::from(111)),
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
    for (label, primitive, defining_prototype, caller_prototype) in primitive_cases {
        assert_ne!(defining_prototype, caller_prototype, "{label} realm setup");
        let Value::Object(first) = caller
            .call(
                &function,
                Value::Undefined,
                std::slice::from_ref(&primitive),
            )
            .unwrap()
        else {
            panic!("raw111 did not box {label}");
        };
        let Value::Object(second) = caller
            .call(
                &function,
                Value::Undefined,
                std::slice::from_ref(&primitive),
            )
            .unwrap()
        else {
            panic!("raw111 did not box {label} on its repeated call");
        };
        assert_ne!(first, second, "raw111 reused a {label} wrapper");
        assert_eq!(
            runtime.get_prototype_of(&first).unwrap(),
            Some(defining_prototype.clone()),
            "raw111 boxed {label} in the wrong realm"
        );
        assert_eq!(
            runtime.get_prototype_of(&second).unwrap(),
            Some(defining_prototype.clone()),
            "repeated raw111 boxed {label} in the wrong realm"
        );
        assert_ne!(
            runtime.get_prototype_of(&first).unwrap(),
            Some(caller_prototype),
            "raw111 used the caller's {label} prototype"
        );
        let value_of = property_callable(&runtime, &mut defining, &defining_prototype, "valueOf");
        assert!(
            defining
                .call(&value_of, Value::Object(first), &[])
                .unwrap()
                .same_value(&primitive),
            "raw111 changed the boxed {label} payload"
        );
    }
    assert!(!defining.has_exception());
    assert!(!caller.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_to_object_nullish_is_pending_and_catchable() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let function = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TO_OBJECT_BC5, 0)
        .unwrap();
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

    let name_key = runtime.intern_property_key("name").unwrap();
    let message_key = runtime.intern_property_key("message").unwrap();
    for nullish in [Value::Null, Value::Undefined] {
        assert_eq!(
            caller.call(&function, Value::Undefined, &[nullish]),
            Err(RuntimeError::Exception)
        );
        assert!(caller.has_exception());
        let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
            panic!("raw111 nullish failure did not materialize a TypeError");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(defining_type_error_prototype.clone())
        );
        assert_ne!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(caller_type_error_prototype.clone())
        );
        assert_eq!(
            caller.get_property(&error, &name_key).unwrap(),
            Value::String(JsString::from_static("TypeError"))
        );
        assert_eq!(
            caller.get_property(&error, &message_key).unwrap(),
            Value::String(JsString::from_static("cannot convert to object"))
        );
        assert!(!caller.has_exception());
    }

    let global = caller.global_object().unwrap();
    let raw_to_object_key = runtime.intern_property_key("rawToObject").unwrap();
    assert!(
        caller
            .define_own_property(
                &global,
                &raw_to_object_key,
                &data_descriptor(
                    Value::Object(function.as_object().clone()),
                    true,
                    true,
                    true,
                ),
            )
            .unwrap()
    );
    assert_eq!(
        caller
            .eval(
                r#"
                    (function () {
                        var result = "";
                        try { rawToObject(null); }
                        catch (error) { result += error.name + ":" + error.message; }
                        try { rawToObject(undefined); }
                        catch (error) { result += "|" + error.name + ":" + error.message; }
                        return result;
                    })()
                "#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "TypeError:cannot convert to object|TypeError:cannot convert to object"
        ))
    );
    assert!(!caller.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_to_object_verification_rolls_back_and_retries() {
    let mut fallthrough = QUICKJS_ORDINARY_TO_OBJECT_BC5.to_vec();
    fallthrough[37] = 2;
    fallthrough.truncate(45);
    assert_eq!(&fallthrough[43..], &[0xcf, 0x6f]);

    let mut undersized_stack = QUICKJS_ORDINARY_TO_OBJECT_BC5.to_vec();
    undersized_stack[33] = 0;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    for (label, image) in [
        ("raw111 fallthrough", fallthrough),
        ("raw111 max-stack underdeclaration", undersized_stack),
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
            "{label}: {}",
            error.message()
        );
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TO_OBJECT_BC5, 0)
        .unwrap();
    assert!(matches!(
        context
            .call(&function, Value::Undefined, &[Value::Bool(false)])
            .unwrap(),
        Value::Object(_)
    ));
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_branch_targets_raw111_typed_index() {
    let mut image = QUICKJS_ORDINARY_TO_OBJECT_BC5.to_vec();
    image[37] = 5;
    image.truncate(43);
    image.extend_from_slice(&[0xcf, 0xea, 0x01, 0x6f, 0x28]);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let boolean_prototype = context.boolean_prototype().unwrap();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("branch-to-raw111 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::Goto(2),
            Instruction::ToObject,
            Instruction::Return,
        ]
    ));
    assert!(snapshot.constants.is_empty());
    drop(snapshot);

    let Value::Object(wrapper) = context
        .call(&function, Value::Undefined, &[Value::Bool(true)])
        .unwrap()
    else {
        panic!("branch-to-raw111 call did not box its Boolean");
    };
    assert_eq!(
        runtime.get_prototype_of(&wrapper).unwrap(),
        Some(boolean_prototype)
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_object_verification_rolls_back_and_retries() {
    let mut object_only = QUICKJS_ORDINARY_OBJECT_BC5.to_vec();
    object_only[37] = 1;
    object_only.truncate(40);
    assert_eq!(object_only[39], 0x0b);

    let mut undersized_stack = QUICKJS_ORDINARY_OBJECT_BC5.to_vec();
    undersized_stack[33] = 0;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    for (label, image) in [
        ("raw11-only fallthrough", object_only),
        ("raw11 max-stack underdeclaration", undersized_stack),
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
            "{label}: {}",
            error.message()
        );
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_OBJECT_BC5, 0)
        .unwrap();
    assert!(matches!(
        context.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Object(_)
    ));
    assert!(!context.has_exception());
}
