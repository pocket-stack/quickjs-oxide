use super::*;

#[test]
fn trusted_quickjs_ordinary_read_only_uses_exact_zero_stack_wire_and_type_error() {
    assert_eq!(QUICKJS_ORDINARY_READ_ONLY_BC5.len(), 47);
    assert_eq!(
        fnv1a64(QUICKJS_ORDINARY_READ_ONLY_BC5),
        0xb4c1_126c_2830_93af
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_READ_ONLY_BC5, 0)
        .unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted raw49 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [Instruction::ThrowReadOnly(0)]
    ));
    assert_eq!(snapshot.metadata.argument_count, 0);
    assert_eq!(snapshot.metadata.local_count, 0);
    assert_eq!(snapshot.metadata.max_stack, 0);
    assert!(snapshot.metadata.strict);
    let [BytecodeConstant::Value(RawValue::String(name))] = snapshot.constants.as_ref() else {
        panic!("raw49 name was not published as one verified String constant");
    };
    assert_eq!(name, &JsString::from_static("x"));
    drop(snapshot);

    assert_eq!(
        context.call(&function, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    assert!(context.has_exception());
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("'x' is read-only")
        )
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_read_only_reenters_catch_and_resets_pending_state() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_READ_ONLY_BC5, 0)
        .unwrap();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("rawReadOnly").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &key,
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
        expect_string_value(
            context
                .eval("try { rawReadOnly() } catch (error) { error.name + ':' + error.message }")
                .unwrap()
        ),
        JsString::from_static("TypeError:'x' is read-only")
    );
    assert!(!context.has_exception());

    assert_eq!(
        expect_string_value(
            context
                .eval(
                    r#"
                        (function () {
                            var log = "";
                            var iterable = {};
                            iterable[Symbol.iterator] = function () {
                                return {
                                    next: function () { return { value: 1, done: false }; },
                                    return: function () {
                                        log += "close";
                                        throw new Error("close failure");
                                    }
                                };
                            };
                            try {
                                for (var value of iterable) rawReadOnly();
                            } catch (error) {
                                return log + ":" + error.name + ":" + error.message;
                            }
                            return "missing throw";
                        })()
                    "#,
                )
                .unwrap()
        ),
        JsString::from_static("close:TypeError:'x' is read-only")
    );
    assert!(!context.has_exception());

    // The source compiler leaves attempted/postfix values below this terminal;
    // catch entry must discard the entire abandoned frame stack.
    assert_eq!(
        expect_string_value(
            context
                .eval("(function(){try{const x=0;x++;}catch(error){return error.message}})()")
                .unwrap()
        ),
        JsString::from_static("'x' is read-only")
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_read_only_uses_bytecode_realm_and_attaches_backtrace() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let function = defining
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_READ_ONLY_BC5, 0)
        .unwrap();
    let Value::Object(defining_type_error_prototype) =
        defining.eval("TypeError.prototype").unwrap()
    else {
        panic!("defining TypeError.prototype was not an object");
    };

    assert_eq!(
        caller.call(&function, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("raw49 did not materialize a TypeError object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error_prototype)
    );
    assert!(
        own_stack_string(&runtime, &error)
            .to_utf8_lossy()
            .contains("<anonymous>")
    );
    assert!(!caller.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_read_only_preserves_wide_lone_surrogate_name_units() {
    let mut image = QUICKJS_ORDINARY_READ_ONLY_BC5.to_vec();
    // Replace the narrow `x` header atom with wide `A` + an unpaired high
    // surrogate. The relocated raw243 operand remains the sole input slot.
    image.splice(2..4, [0x05, 0x41, 0x00, 0x00, 0xd8]);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context)
            .utf16_units()
            .collect::<Vec<_>>(),
        [
            vec![u16::from(b'\''), u16::from(b'A'), 0xd800],
            "' is read-only".encode_utf16().collect(),
        ]
        .concat()
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_read_only_rejections_are_transactional_and_retryable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    let mut subtype_one = QUICKJS_ORDINARY_READ_ONLY_BC5.to_vec();
    subtype_one[46] = 1;
    let mut subtype_max = QUICKJS_ORDINARY_READ_ONLY_BC5.to_vec();
    subtype_max[46] = u8::MAX;

    let mut unused_atom = QUICKJS_ORDINARY_READ_ONLY_BC5.to_vec();
    unused_atom[39] = 1;
    unused_atom.truncate(41);
    unused_atom.push(0x29); // return_undef

    let mut multiple_atoms = QUICKJS_ORDINARY_READ_ONLY_BC5.to_vec();
    multiple_atoms[1] = 2;
    multiple_atoms.splice(4..4, [0x02, b'y']);

    let mut non_string = QUICKJS_ORDINARY_READ_ONLY_BC5.to_vec();
    non_string[42..46].copy_from_slice(&0x8000_002a_u32.to_le_bytes());

    for (label, image) in [
        ("subtype one", subtype_one),
        ("subtype max", subtype_max),
        ("unused atom", unused_atom),
        ("multiple atoms", multiple_atoms),
        ("non-String atom", non_string),
        (
            "natural lexical wire",
            QUICKJS_NATURAL_READ_ONLY_BC5.to_vec(),
        ),
    ] {
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_READ_ONLY_BC5, 0)
        .unwrap();
    assert_eq!(
        runtime.heap_counts().function_bytecode_nodes,
        baseline.function_bytecode_nodes + 1
    );
    assert_eq!(
        context.call(&function, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("'x' is read-only")
        )
    );
}
