use super::*;

#[test]
fn trusted_quickjs_ordinary_throw_uses_the_exact_wire_metadata_and_value_identity() {
    assert_eq!(QUICKJS_ORDINARY_THROW_BC5.len(), 45);
    assert_eq!(fnv1a64(QUICKJS_ORDINARY_THROW_BC5), 0x73cf_217e_06c5_fee2);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)
        .unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted raw48 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [Instruction::GetArg(0), Instruction::Throw]
    ));
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

    assert_eq!(
        context.call(&function, Value::Undefined, &[Value::Int(9)]),
        Err(RuntimeError::Exception)
    );
    assert!(context.has_exception());
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
    assert!(!context.has_exception());

    let object = context.new_object().unwrap();
    assert_eq!(
        context.call(
            &function,
            Value::Undefined,
            &[Value::Object(object.clone())],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::Object(object.clone()))
    );
    assert!(!context.has_exception());

    let Value::Object(error) = context.eval("new Error('raw48 identity')").unwrap() else {
        panic!("Error construction did not return an object");
    };
    let stack = runtime.intern_property_key("stack").unwrap();
    assert!(runtime.delete_property(&error, &stack).unwrap());
    assert_eq!(
        context.call(&function, Value::Undefined, &[Value::Object(error.clone())],),
        Err(RuntimeError::Exception)
    );
    let Value::Object(thrown_error) = context.take_exception().unwrap().unwrap() else {
        panic!("trusted raw48 throw lost its Error object");
    };
    assert_eq!(thrown_error, error);
    assert!(
        own_stack_string(&runtime, &error)
            .to_utf8_lossy()
            .contains("<anonymous>")
    );
    assert!(!context.has_exception());

    // Taking the pending completion restores the public API for a clean retry.
    assert_eq!(
        context.call(&function, Value::Undefined, &[Value::Int(42)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_throw_reenters_caller_catch_backtrace_and_iterator_close() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)
        .unwrap();
    let global = context.global_object().unwrap();
    {
        let (name, value) = ("rawThrow", Value::Object(function.as_object().clone()));
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(&global, &key, &data_descriptor(value, true, true, true),)
                .unwrap()
        );
    }

    let Value::Object(error) = context.eval("new Error('caught raw48')").unwrap() else {
        panic!("Error construction did not return an object");
    };
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert!(runtime.delete_property(&error, &stack_key).unwrap());
    let held_key = runtime.intern_property_key("heldThrowError").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &held_key,
                &data_descriptor(Value::Object(error.clone()), true, true, true),
            )
            .unwrap()
    );

    assert_eq!(
        context
            .eval_with_filename(
                "(function caller(){try{rawThrow(heldThrowError)}catch(caught){return caught===heldThrowError}})()",
                "raw-throw-caller.js",
            )
            .unwrap(),
        Value::Bool(true)
    );
    assert!(!context.has_exception());
    let stack = own_stack_string(&runtime, &error).to_utf8_lossy();
    assert!(stack.contains("    at <anonymous>\n"), "{stack}");
    assert!(stack.contains("caller (raw-throw-caller.js:"), "{stack}");

    assert_eq!(
        expect_string_value(
            context
                .eval(
                    r#"
                        (function () {
                            globalThis.__qjo_throw_close_log = "";
                            var original = { kind: "raw48" };
                            var closeFailure = { kind: "iterator-close" };
                            var iterable = {};
                            iterable[Symbol.iterator] = function () {
                                var delivered = false;
                                return {
                                    next: function () {
                                        if (delivered) return { done: true };
                                        delivered = true;
                                        return { value: 1, done: false };
                                    },
                                    return: function () {
                                        __qjo_throw_close_log += "close";
                                        throw closeFailure;
                                    }
                                };
                            };
                            try {
                                for (var value of iterable) rawThrow(original);
                            } catch (caught) {
                                return __qjo_throw_close_log + ":" +
                                    (caught === original) + ":" +
                                    (caught === closeFailure);
                            }
                            return "missing throw";
                        })()
                    "#,
                )
                .unwrap(),
        ),
        JsString::from_static("close:true:false")
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_throw_is_terminal_and_branch_targetable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    // The underflowing Drop is translated and published but is unreachable
    // after the explicit throw completion.
    let terminal = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_one_argument_with_code(&[0xcf, 0x30, 0x0e], 1),
            0,
        )
        .unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&terminal).unwrap()
    else {
        panic!("terminal raw48 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::Throw,
            Instruction::Drop,
        ]
    ));
    drop(snapshot);
    assert_eq!(
        context.call(&terminal, Value::Undefined, &[Value::Int(17)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(17)));

    // goto8 lands directly on raw48 while retaining the argument value. The
    // skipped push proves target rewriting uses the throw's typed IR index.
    let branch = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_one_argument_with_code(&[0xcf, 0xea, 0x02, 0xb4, 0x30], 1),
            0,
        )
        .unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&branch).unwrap()
    else {
        panic!("branch-to-raw48 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::Goto(3),
            Instruction::PushI32(1),
            Instruction::Throw,
        ]
    ));
    drop(snapshot);
    let object = context.new_object().unwrap();
    assert_eq!(
        context.call(&branch, Value::Undefined, &[Value::Object(object.clone())],),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::Object(object))
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_throw_verification_rejects_transactionally_and_retries() {
    const UNDERFLOW: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow";
    const DECLARED_MAXIMUM: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required";

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    for (label, code, max_stack, expected) in [
        ("throw underflow", &[0x30][..], 0, UNDERFLOW),
        (
            "throw declared maximum",
            &[0xcf, 0x30][..],
            0,
            DECLARED_MAXIMUM,
        ),
        (
            "branch-to-throw underflow",
            &[0xea, 0x02, 0xb4, 0x30][..],
            1,
            UNDERFLOW,
        ),
    ] {
        let image = quickjs_ordinary_one_argument_with_code(code, max_stack);
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(error.message(), expected, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let retry = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)
        .unwrap();
    assert_eq!(
        context.call(&retry, Value::Undefined, &[Value::Int(42)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_throw_rejects_nonordinary_metadata_transactionally() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    let mut async_function = QUICKJS_ORDINARY_THROW_BC5.to_vec();
    async_function[28] |= 1 << 2; // strict + async JS mode
    let mut generator = QUICKJS_ORDINARY_THROW_BC5.to_vec();
    generator[26] |= 1 << 4; // normal -> generator FunctionKind
    let mut derived = QUICKJS_ORDINARY_THROW_BC5.to_vec();
    derived[26] |= 1 << 2; // ordinary -> derived class constructor

    for (label, image) in [
        ("async", async_function),
        ("generator", generator),
        ("derived constructor", derived),
    ] {
        assert_eq!(
            &image[43..],
            [0xcf, 0x30],
            "{label} mutation changed the natural raw48 body"
        );
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} raw48 metadata did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(
            error.message(),
            "trusted QuickJS ordinary leaf is not admitted: function metadata is outside the ordinary synchronous leaf cohort",
            "{label}"
        );
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }

    let retry = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)
        .unwrap();
    assert_eq!(
        context.call(&retry, Value::Undefined, &[Value::Int(42)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
    assert!(!context.has_exception());
}
