use super::*;

#[test]
fn trusted_quickjs_ordinary_plain_calls_preserve_receiver_arity_and_argument_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_PLAIN_CALLS_CODE, &[]);
    image[33] = 5;

    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary plain-call leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 5);
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::Call(0),
            Instruction::Drop,
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::Call(1),
            Instruction::Drop,
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::Call(2),
            Instruction::Drop,
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::Call(3),
            Instruction::Drop,
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::PushI32(4),
            Instruction::Call(4),
            Instruction::Return,
        ]
    ));
    drop(snapshot);

    let callback = context
        .eval(
            r#"
                globalThis.__qjo_plain_call_log = "";
                (0, function () {
                    "use strict";
                    __qjo_plain_call_log += (this === undefined ? "u" : "x") + ":" + arguments.length;
                    var index = 0;
                    while (index < arguments.length) {
                        __qjo_plain_call_log += ":" + arguments[index];
                        index++;
                    }
                    __qjo_plain_call_log += "|";
                    return arguments.length;
                })
            "#,
        )
        .unwrap();
    assert!(matches!(callback, Value::Object(_)));
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[callback])
            .unwrap(),
        Value::Int(4)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_plain_call_log").unwrap()),
        JsString::from_static("u:0|u:1:1|u:2:1:2|u:3:1:2:3|u:4:1:2:3:4|")
    );
}

#[test]
fn trusted_quickjs_ordinary_plain_call_non_callable_exception_is_recoverable() {
    let mut image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_PLAIN_CALLS_CODE, &[]);
    image[33] = 5;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    assert_eq!(
        context.call(&function, Value::Undefined, &[Value::Int(0)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("not a function"),
        )
    );
    assert!(!context.has_exception());

    let callback = context
        .eval("(0, function () { 'use strict'; return arguments.length; })")
        .unwrap();
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[callback])
            .unwrap(),
        Value::Int(4)
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics() {
    assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_BC5.len(), 57);
    assert_eq!(
        fnv1a64(QUICKJS_ORDINARY_TAIL_CALL_BC5),
        0x8ff9_d2c1_0c7e_2228
    );
    assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5.len(), 62);
    assert_eq!(
        fnv1a64(QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5),
        0xe87d_54c0_a2a1_40ca
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let tail_call = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TAIL_CALL_BC5, 0)
        .unwrap();
    let tail_method = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5, 0)
        .unwrap();

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&tail_call).unwrap()
    else {
        panic!("trusted raw35 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 3);
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::GetArg(2),
            Instruction::TailCall(2),
        ]
    ));
    drop(snapshot);

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&tail_method).unwrap()
    else {
        panic!("trusted raw37 function did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert_eq!(snapshot.metadata.max_stack, 4);
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::GetArg(2),
            Instruction::GetArg(3),
            Instruction::TailCallMethod(2),
        ]
    ));
    drop(snapshot);

    let plain_target = context
        .eval(
            r#"
                globalThis.__qjo_tail_plain_log = "";
                (0, function (a, b) {
                    "use strict";
                    __qjo_tail_plain_log =
                        (this === undefined) + ":" + arguments.length + ":" + a + ":" + b;
                    return a * 10 + b;
                })
            "#,
        )
        .unwrap();
    assert_eq!(
        context
            .call(
                &tail_call,
                Value::Int(99),
                &[plain_target.clone(), Value::Int(4), Value::Int(2)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_tail_plain_log").unwrap()),
        JsString::from_static("true:2:4:2")
    );

    let receiver = context
        .eval("(globalThis.__qjo_tail_receiver = { base: 40 })")
        .unwrap();
    let method = context
        .eval(
            r#"
                (function (a, b) {
                    "use strict";
                    globalThis.__qjo_tail_method_log =
                        (this === globalThis.__qjo_tail_receiver) + ":" +
                        arguments.length + ":" + a + ":" + b;
                    return this.base + a + b;
                })
            "#,
        )
        .unwrap();
    assert_eq!(
        context
            .call(
                &tail_method,
                Value::Undefined,
                &[receiver, method.clone(), Value::Int(1), Value::Int(1)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_tail_method_log").unwrap()),
        JsString::from_static("true:2:1:1")
    );

    // A forged instruction after raw35 remains authenticated but unreachable:
    // the tail completion must never fall through to the underflowing Drop.
    let unreachable = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x23, 0x02, 0x00, 0x0e],
                3,
            ),
            0,
        )
        .unwrap();
    assert_eq!(
        context
            .call(
                &unreachable,
                Value::Undefined,
                &[plain_target, Value::Int(4), Value::Int(2)],
            )
            .unwrap(),
        Value::Int(42)
    );
}

#[test]
fn trusted_quickjs_ordinary_tail_invocation_failures_are_recoverable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let tail_call = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TAIL_CALL_BC5, 0)
        .unwrap();
    let tail_method = context
        .read_trusted_ordinary_function(QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5, 0)
        .unwrap();

    for (label, function, arguments) in [
        (
            "plain",
            &tail_call,
            vec![Value::Int(0), Value::Int(1), Value::Int(2)],
        ),
        (
            "method",
            &tail_method,
            vec![Value::Null, Value::Int(0), Value::Int(1), Value::Int(2)],
        ),
    ] {
        assert_eq!(
            context.call(function, Value::Undefined, &arguments),
            Err(RuntimeError::Exception),
            "{label}"
        );
        assert_eq!(
            take_error_name_and_message(&runtime, &mut context),
            (
                JsString::from_static("TypeError"),
                JsString::from_static("not a function"),
            ),
            "{label}"
        );
        assert!(!context.has_exception(), "{label}");
    }

    let target = context
        .eval("(0, function (a, b) { 'use strict'; return a + b; })")
        .unwrap();
    assert_eq!(
        context
            .call(
                &tail_call,
                Value::Undefined,
                &[target.clone(), Value::Int(20), Value::Int(22)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context
            .call(
                &tail_method,
                Value::Undefined,
                &[Value::Null, target, Value::Int(20), Value::Int(22)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_tail_verification_rolls_back_heap_and_atoms() {
    const UNDERFLOW: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow";
    const DECLARED_MAXIMUM: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required";

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    let cases = [
        (
            "tail_call underflow",
            quickjs_ordinary_three_argument_with_code(&[0x23, 0x00, 0x00], 0),
            UNDERFLOW,
        ),
        (
            "tail_call_method underflow",
            quickjs_ordinary_four_argument_with_code(&[0x25, 0x00, 0x00], 0),
            UNDERFLOW,
        ),
        (
            "tail_call maximum",
            quickjs_ordinary_three_argument_with_code(&[0xcf, 0xd0, 0xd1, 0x23, 0x02, 0x00], 2),
            DECLARED_MAXIMUM,
        ),
        (
            "tail_call_method maximum",
            quickjs_ordinary_four_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0xd2, 0x25, 0x02, 0x00],
                3,
            ),
            DECLARED_MAXIMUM,
        ),
        (
            "tail_call u16 maximum operand",
            quickjs_ordinary_three_argument_with_code(&[0x23, 0xff, 0xff], 0),
            UNDERFLOW,
        ),
        (
            "tail_call_method u16 maximum operand",
            quickjs_ordinary_four_argument_with_code(&[0x25, 0xff, 0xff], 0),
            UNDERFLOW,
        ),
    ];

    for (label, image, expected_message) in cases {
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(error.message(), expected_message, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }
}

#[test]
fn trusted_quickjs_ordinary_non_tail_invocations_preserve_stack_contracts() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let mut construct_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_CONSTRUCT_CODE, &[]);
    construct_image[33] = 4;
    let construct = context
        .read_trusted_ordinary_function(&construct_image, 0)
        .unwrap();
    let constructor = context
        .eval(
            r#"
                (globalThis.__qjo_construct = function C(a, b) {
                    globalThis.__qjo_construct_log =
                        (new.target === globalThis.__qjo_new_target) + ":" +
                        (Object.getPrototypeOf(this) === new.target.prototype) + ":" +
                        a + ":" + b;
                    this.sum = a + b;
                })
            "#,
        )
        .unwrap();
    let new_target = context
        .eval("(globalThis.__qjo_new_target = function N() {})")
        .unwrap();
    let Value::Object(instance) = context
        .call(&construct, Value::Undefined, &[constructor, new_target])
        .unwrap()
    else {
        panic!("call_constructor did not return an Object");
    };
    let sum = runtime.intern_property_key("sum").unwrap();
    assert_eq!(
        context.get_property(&instance, &sum).unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_construct_log").unwrap()),
        JsString::from_static("true:true:1:2")
    );

    let mut method_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_CALL_METHOD_CODE, &[]);
    method_image[33] = 3;
    let call_method = context
        .read_trusted_ordinary_function(&method_image, 0)
        .unwrap();
    let receiver = context
        .eval("(globalThis.__qjo_method_receiver = { base: 7 })")
        .unwrap();
    let method = context
        .eval(
            "(function (value) { 'use strict'; return this === globalThis.__qjo_method_receiver ? this.base + value : -1; })",
        )
        .unwrap();
    assert_eq!(
        context
            .call(&call_method, Value::Undefined, &[receiver, method])
            .unwrap(),
        Value::Int(49)
    );

    let mut array_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_ARRAY_FROM_CODE, &[]);
    array_image[33] = 3;
    let array_from = context
        .read_trusted_ordinary_function(&array_image, 0)
        .unwrap();
    let Value::Object(first) = context.call(&array_from, Value::Undefined, &[]).unwrap() else {
        panic!("array_from did not return an Array");
    };
    let Value::Object(second) = context.call(&array_from, Value::Undefined, &[]).unwrap() else {
        panic!("array_from repeat did not return an Array");
    };
    assert_ne!(first, second);
    let defining_array_prototype = context.array_prototype().unwrap();
    for array in [&first, &second] {
        assert!(runtime.is_array_object(array).unwrap());
        assert_eq!(
            runtime.get_prototype_of(array).unwrap(),
            Some(defining_array_prototype.clone())
        );
    }
    for (key, expected) in [("0", 1), ("1", 2), ("2", 3), ("length", 3)] {
        let key = runtime.intern_property_key(key).unwrap();
        assert_eq!(
            context.get_property(&first, &key).unwrap(),
            Value::Int(expected)
        );
    }
    let mut empty_array_image =
        quickjs_ordinary_with_code_and_constants(&[0x26, 0x00, 0x00, 0x28], &[]);
    empty_array_image[33] = 1;
    let empty_array_from = context
        .read_trusted_ordinary_function(&empty_array_image, 0)
        .unwrap();
    let Value::Object(empty) = context
        .call(&empty_array_from, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("zero-element array_from did not return an Array");
    };
    assert!(runtime.is_array_object(&empty).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&empty).unwrap(),
        Some(defining_array_prototype.clone())
    );
    let length = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        context.get_property(&empty, &length).unwrap(),
        Value::Int(0)
    );

    let mut foreign_caller = runtime.new_context();
    let foreign_array_prototype = foreign_caller.array_prototype().unwrap();
    assert_ne!(defining_array_prototype, foreign_array_prototype);
    let Value::Object(cross_realm) = foreign_caller
        .call(&array_from, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("cross-realm array_from did not return an Array");
    };
    assert!(runtime.is_array_object(&cross_realm).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&cross_realm).unwrap(),
        Some(defining_array_prototype)
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&construct).unwrap()
    else {
        panic!("trusted ordinary construct leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::Construct(2),
            Instruction::Return,
        ]
    ));
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&call_method).unwrap()
    else {
        panic!("trusted ordinary call_method leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::PushI32(42),
            Instruction::CallMethod(1),
            Instruction::Return,
        ]
    ));
    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&array_from).unwrap()
    else {
        panic!("trusted ordinary array_from leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::ArrayFrom(3),
            Instruction::Return,
        ]
    ));
}

#[test]
fn trusted_quickjs_ordinary_non_tail_invocation_exceptions_are_recoverable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let mut construct_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_CONSTRUCT_CODE, &[]);
    construct_image[33] = 4;
    let construct = context
        .read_trusted_ordinary_function(&construct_image, 0)
        .unwrap();
    assert_eq!(
        context.call(
            &construct,
            Value::Undefined,
            &[Value::Int(0), Value::Int(0)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );
    assert!(!context.has_exception());

    let constructor = context
        .eval(
            "(function C(a, b) { \
                globalThis.__qjo_construct_raw_new_target = new.target; \
                this.sum = a + b; \
            })",
        )
        .unwrap();
    let Value::Object(instance) = context
        .call(
            &construct,
            Value::Undefined,
            &[constructor.clone(), Value::Int(0)],
        )
        .unwrap()
    else {
        panic!("raw nonconstructor newTarget did not construct an object");
    };
    assert_eq!(
        context.eval("__qjo_construct_raw_new_target").unwrap(),
        Value::Int(0)
    );
    let sum = runtime.intern_property_key("sum").unwrap();
    assert_eq!(
        context.get_property(&instance, &sum).unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert!(!context.has_exception());

    let mut method_image =
        quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_CALL_METHOD_CODE, &[]);
    method_image[33] = 3;
    let call_method = context
        .read_trusted_ordinary_function(&method_image, 0)
        .unwrap();
    let receiver = context.eval("({ base: 1 })").unwrap();
    assert_eq!(
        context.call(&call_method, Value::Undefined, &[receiver, Value::Int(0)],),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("not a function"),
        )
    );
    assert!(!context.has_exception());

    let new_target = context.eval("(function N() {})").unwrap();
    assert!(matches!(
        context
            .call(&construct, Value::Undefined, &[constructor, new_target],)
            .unwrap(),
        Value::Object(_)
    ));
    let receiver = context.eval("({ base: 1 })").unwrap();
    let method = context
        .eval("(function (value) { 'use strict'; return this.base + value; })")
        .unwrap();
    assert_eq!(
        context
            .call(&call_method, Value::Undefined, &[receiver, method])
            .unwrap(),
        Value::Int(43)
    );
}

#[test]
fn trusted_quickjs_ordinary_non_tail_invocation_verification_rejects_before_publication() {
    const UNDERFLOW: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow";
    const DECLARED_MAXIMUM: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required";
    const CASES: [(&str, &[u8], u8, &str); 6] = [
        (
            "construct underflow",
            &[0x21, 0x00, 0x00, 0x28],
            0,
            UNDERFLOW,
        ),
        ("method underflow", &[0x24, 0x00, 0x00, 0x28], 0, UNDERFLOW),
        ("array underflow", &[0x26, 0x01, 0x00, 0x28], 0, UNDERFLOW),
        (
            "construct declared maximum",
            &[0x06, 0x06, 0x21, 0x00, 0x00, 0x28],
            1,
            DECLARED_MAXIMUM,
        ),
        (
            "method declared maximum",
            &[0x06, 0x06, 0x24, 0x00, 0x00, 0x28],
            1,
            DECLARED_MAXIMUM,
        ),
        (
            "array declared maximum",
            &[0xb4, 0x26, 0x01, 0x00, 0x28],
            0,
            DECLARED_MAXIMUM,
        ),
    ];

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    for (label, code, max_stack, expected_message) in CASES {
        let mut image = quickjs_ordinary_with_code_and_constants(code, &[]);
        image[33] = max_stack;
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(error.message(), expected_message, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }
}

#[test]
fn trusted_quickjs_ordinary_branch_targets_can_land_on_call_method_after_expansion() {
    let code = [
        0xcf, 0xd0, 0xea, 0x05, 0xb4, 0xb5, 0xb6, 0x1d, 0x24, 0x00, 0x00, 0x28,
    ];
    let mut image = quickjs_ordinary_with_code_and_constants(&code, &[]);
    image[33] = 2;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let receiver = context.eval("({ base: 42 })").unwrap();
    let method = context
        .eval("(function () { 'use strict'; return this.base; })")
        .unwrap();
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[receiver, method])
            .unwrap(),
        Value::Int(42)
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("trusted ordinary branch-to-method leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::Goto(8),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::Perm3,
            Instruction::Swap,
            Instruction::CallMethod(0),
            Instruction::Return,
        ]
    ));
}

#[test]
fn trusted_quickjs_ordinary_plain_call_verification_rejections_do_not_publish() {
    const CASES: [(&str, &[u8], u8, &str); 2] = [
        (
            "call operand underflow",
            &[0x22, 0x04, 0x00, 0x28],
            0,
            "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow",
        ),
        (
            "declared maximum stack",
            &[0x06, 0xb4, 0xb5, 0xb6, 0xb7, 0x22, 0x04, 0x00, 0x28],
            4,
            "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required",
        ),
    ];

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    for (label, code, max_stack, expected_message) in CASES {
        let mut image = quickjs_ordinary_with_code_and_constants(code, &[]);
        image[33] = max_stack;
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("{label} did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{label}");
        assert_eq!(error.message(), expected_message, "{label}");
        assert!(!context.has_exception(), "{label}");
        assert_eq!(runtime.heap_counts(), baseline, "{label}");
        assert_eq!(runtime.test_atom_count(), baseline_atoms, "{label}");
    }
}
