use super::*;

#[test]
fn trusted_quickjs_ordinary_apply_admits_only_canonical_typed_kinds() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (magic, expected_kind) in [(0, ApplyKind::Call), (1, ApplyKind::Construct)] {
        let code = [0xcf, 0xd0, 0xd1, 0x27, magic, 0x00, 0x28];
        let image = quickjs_ordinary_three_argument_with_code(&code, 3);
        let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
        let CallableExecution::Bytecode { bytecode, .. } =
            runtime.bytecode_for_callable(&function).unwrap()
        else {
            panic!("trusted ordinary apply leaf did not publish bytecode");
        };
        let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
        assert!(matches!(
            snapshot.code.as_ref(),
            [
                Instruction::GetArg(0),
                Instruction::GetArg(1),
                Instruction::GetArg(2),
                Instruction::Apply(actual),
                Instruction::Return,
            ] if *actual == expected_kind
        ));
    }

    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();
    for magic in [2_u16, u16::MAX] {
        let [low, high] = magic.to_le_bytes();
        let image = quickjs_ordinary_three_argument_with_code(
            &[0xcf, 0xd0, 0xd1, 0x27, low, high, 0x28],
            3,
        );
        let RuntimeError::Engine(error) = context
            .read_trusted_ordinary_function(&image, 0)
            .unwrap_err()
        else {
            panic!("noncanonical apply magic did not return an engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert_eq!(
            error.message(),
            format!(
                "trusted QuickJS ordinary leaf is not admitted: apply operand must be canonical 0 (call) or 1 (construct), found {magic}"
            )
        );
        assert!(!context.has_exception());
        assert_eq!(runtime.heap_counts(), baseline);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
    }
}

#[test]
fn trusted_quickjs_ordinary_apply_preserves_object_list_call_and_construct_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let call = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x00, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let construct = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();

    let receiver = context
        .eval("(globalThis.__qjo_apply_receiver = { marker: 7 })")
        .unwrap();
    let target = context
        .eval(
            r#"
                (function (a, b) {
                    "use strict";
                    return (this === globalThis.__qjo_apply_receiver) + ":" +
                        arguments.length + ":" + a + ":" + b + ":" +
                        (new.target === undefined);
                })
            "#,
        )
        .unwrap();
    let list = context.eval("[11, 22]").unwrap();
    assert_eq!(
        expect_string_value(
            context
                .call(&call, Value::Undefined, &[target, receiver, list])
                .unwrap()
        ),
        JsString::from_static("true:2:11:22:true")
    );

    let constructor = context
        .eval(
            r#"
                (globalThis.__qjo_apply_constructor = function C(a, b) {
                    globalThis.__qjo_apply_construct_log =
                        (new.target === globalThis.__qjo_apply_new_target) + ":" + a + ":" + b;
                    this.sum = a + b;
                })
            "#,
        )
        .unwrap();
    let new_target = context
        .eval("(globalThis.__qjo_apply_new_target = function N() {})")
        .unwrap();
    let list = context.eval("[20, 22]").unwrap();
    let Value::Object(instance) = context
        .call(
            &construct,
            Value::Undefined,
            &[constructor, new_target.clone(), list],
        )
        .unwrap()
    else {
        panic!("apply construct did not return an object");
    };
    assert_eq!(
        expect_string_value(context.eval("__qjo_apply_construct_log").unwrap()),
        JsString::from_static("true:20:22")
    );
    let Value::Object(new_target) = new_target else {
        panic!("newTarget was not an object");
    };
    let prototype = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(prototype) = context.get_property(&new_target, &prototype).unwrap() else {
        panic!("newTarget prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(prototype)
    );
    let sum = runtime.intern_property_key("sum").unwrap();
    assert_eq!(
        context.get_property(&instance, &sum).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn trusted_quickjs_ordinary_apply_nullish_lists_use_raw_receiver_for_both_kinds() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let receiver = context
        .eval("(globalThis.__qjo_nullish_receiver = { marker: 42 })")
        .unwrap();
    let target = context
        .eval(
            r#"
                (globalThis.__qjo_nullish_target = ({
                    probe() {
                        "use strict";
                        return (this === globalThis.__qjo_nullish_receiver) + ":" +
                            arguments.length + ":" + (new.target === undefined);
                    }
                }).probe)
            "#,
        )
        .unwrap();
    let Value::Object(target_object) = &target else {
        panic!("nullish apply target was not an object");
    };
    assert!(!runtime.is_constructor(target_object).unwrap());

    for magic in [0_u8, 1] {
        let wrapper = context
            .read_trusted_ordinary_function(
                &quickjs_ordinary_three_argument_with_code(
                    &[0xcf, 0xd0, 0xd1, 0x27, magic, 0x00, 0x28],
                    3,
                ),
                0,
            )
            .unwrap();
        for argument_array in [Value::Null, Value::Undefined] {
            assert_eq!(
                expect_string_value(
                    context
                        .call(
                            &wrapper,
                            Value::Undefined,
                            &[target.clone(), receiver.clone(), argument_array],
                        )
                        .unwrap()
                ),
                JsString::from_static("true:0:true"),
                "magic {magic}"
            );
            assert!(!context.has_exception());
        }
    }
}

#[test]
fn trusted_quickjs_ordinary_apply_preserves_error_order_realm_and_pending_identity() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let call = defining
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x00, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let construct = defining
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let defining_type_error_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(defining.realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Type.index()]
    .unwrap();

    let mut caller = runtime.new_context();
    let poison = caller
        .eval(
            r#"
                globalThis.__qjo_apply_length_reads = 0;
                globalThis.__qjo_apply_sentinel = {};
                ({ get length() {
                    __qjo_apply_length_reads++;
                    throw __qjo_apply_sentinel;
                } })
            "#,
        )
        .unwrap();
    assert_eq!(
        caller.call(
            &call,
            Value::Undefined,
            &[Value::Int(0), Value::Undefined, poison.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(not_callable) = caller.take_exception().unwrap().unwrap() else {
        panic!("non-callable apply did not throw an object");
    };
    assert_eq!(
        runtime
            .get_prototype_of(&not_callable)
            .unwrap()
            .unwrap()
            .object_id(),
        defining_type_error_prototype
    );
    assert_eq!(
        caller.eval("__qjo_apply_length_reads").unwrap(),
        Value::Int(0)
    );
    assert!(!caller.has_exception());

    let target = caller
        .eval("(globalThis.__qjo_apply_plain = ({ plain() {} }).plain)")
        .unwrap();
    assert_eq!(
        caller.call(
            &construct,
            Value::Undefined,
            &[target, Value::Int(17), poison.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    let thrown = caller.take_exception().unwrap().unwrap();
    let sentinel = caller.eval("__qjo_apply_sentinel").unwrap();
    assert_eq!(thrown, sentinel);
    assert_eq!(
        caller.eval("__qjo_apply_length_reads").unwrap(),
        Value::Int(1)
    );
    assert!(!caller.has_exception());

    let constructor = caller.eval("(function C() {})").unwrap();
    assert_eq!(
        caller.call(
            &construct,
            Value::Undefined,
            &[constructor, Value::Null, poison],
        ),
        Err(RuntimeError::Exception)
    );
    let thrown = caller.take_exception().unwrap().unwrap();
    assert_eq!(thrown, sentinel);
    assert_eq!(
        caller.eval("__qjo_apply_length_reads").unwrap(),
        Value::Int(2)
    );
    assert!(!caller.has_exception());

    let receiver = caller.eval("({})").unwrap();
    let target = caller
        .eval("({ retry() { 'use strict'; return this; } }).retry")
        .unwrap();
    assert_eq!(
        caller
            .call(
                &construct,
                Value::Undefined,
                &[target, receiver.clone(), Value::Null],
            )
            .unwrap(),
        receiver
    );
    assert!(!caller.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_apply_raw_prototype_get_preserves_order_and_receiver() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let wrapper = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let constructor = context
        .eval(
            "globalThis.__qjo_raw_get_body_count = 0; \
             (function RawGetTarget() { \
                 __qjo_raw_get_body_count++; \
                 globalThis.__qjo_raw_get_new_target = new.target; \
             })",
        )
        .unwrap();
    let empty = context.eval("[]").unwrap();

    let Value::Object(undefined_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), Value::Undefined, empty.clone()],
        )
        .unwrap()
    else {
        panic!("undefined raw newTarget did not construct an object");
    };
    assert_eq!(
        context.eval("__qjo_raw_get_new_target").unwrap(),
        Value::Undefined
    );
    assert_eq!(
        runtime.get_prototype_of(&undefined_instance).unwrap(),
        Some(context.object_prototype().unwrap())
    );

    context.eval("__qjo_raw_get_body_count = 0").unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), Value::Null, empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );
    assert_eq!(
        context.eval("__qjo_raw_get_body_count").unwrap(),
        Value::Int(0)
    );

    context
        .eval(
            r#"
                globalThis.__qjo_number_receiver = undefined;
                globalThis.__qjo_number_custom_prototype = {};
                Object.defineProperty(Number.prototype, "prototype", {
                    configurable: true,
                    get: function () {
                        "use strict";
                        globalThis.__qjo_number_receiver = this;
                        return globalThis.__qjo_number_custom_prototype;
                    }
                });
            "#,
        )
        .unwrap();
    let Value::Object(number_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), Value::Int(17), empty.clone()],
        )
        .unwrap()
    else {
        panic!("primitive prototype getter did not construct an object");
    };
    let Value::Object(number_prototype) = context.eval("__qjo_number_custom_prototype").unwrap()
    else {
        panic!("primitive getter prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&number_instance).unwrap(),
        Some(number_prototype)
    );
    assert_eq!(
        context.eval("__qjo_number_receiver").unwrap(),
        Value::Int(17)
    );
    context.eval("delete Number.prototype.prototype").unwrap();

    context
        .eval(
            r#"
                globalThis.__qjo_raw_get_sentinel = {};
                globalThis.__qjo_throwing_new_target = {};
                Object.defineProperty(__qjo_throwing_new_target, "prototype", {
                    get: function () { throw __qjo_raw_get_sentinel; }
                });
                __qjo_raw_get_body_count = 0;
            "#,
        )
        .unwrap();
    let throwing_new_target = context.eval("__qjo_throwing_new_target").unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), throwing_new_target, empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap().unwrap(),
        context.eval("__qjo_raw_get_sentinel").unwrap()
    );
    assert_eq!(
        context.eval("__qjo_raw_get_body_count").unwrap(),
        Value::Int(0)
    );

    let non_callable_new_target = context.eval("({ prototype: 1 })").unwrap();
    let Value::Object(fallback_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), non_callable_new_target, empty.clone()],
        )
        .unwrap()
    else {
        panic!("noncallable raw newTarget did not use the caller realm fallback");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback_instance).unwrap(),
        Some(context.object_prototype().unwrap())
    );

    let revoking_new_target = context
        .eval(
            r#"
                globalThis.__qjo_realm_revoke_record = undefined;
                var realmRevokeHandler = {
                    get: function (target, key, receiver) {
                        if (key === "prototype") {
                            __qjo_realm_revoke_record.revoke();
                            return 1;
                        }
                        return Reflect.get(target, key, receiver);
                    }
                };
                __qjo_realm_revoke_record =
                    Proxy.revocable(function RealmRevokeTarget() {}, realmRevokeHandler);
                __qjo_realm_revoke_record.proxy
            "#,
        )
        .unwrap();
    context.eval("__qjo_raw_get_body_count = 0").unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[constructor, revoking_new_target, empty],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("revoked proxy"),
        )
    );
    assert_eq!(
        context.eval("__qjo_raw_get_body_count").unwrap(),
        Value::Int(0)
    );
    assert!(!context.has_exception());
}

#[test]
fn trusted_quickjs_ordinary_apply_construct_preserves_raw_new_target_across_dispatch() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let wrapper = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let empty = context.eval("[]").unwrap();

    let constructor = context
        .eval(
            r#"
                (globalThis.__qjo_raw_constructor = function C() {
                    globalThis.__qjo_raw_base_new_target = new.target;
                    this.marker = 42;
                })
            "#,
        )
        .unwrap();
    let Value::Object(instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor.clone(), Value::Int(17), empty.clone()],
        )
        .unwrap()
    else {
        panic!("raw primitive newTarget did not produce a base instance");
    };
    assert_eq!(
        context.eval("__qjo_raw_base_new_target").unwrap(),
        Value::Int(17)
    );
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(context.object_prototype().unwrap())
    );

    let new_target = context
        .eval(
            r#"
                globalThis.__qjo_raw_custom_prototype = { custom: true };
                (globalThis.__qjo_raw_new_target_object = {
                    prototype: globalThis.__qjo_raw_custom_prototype
                })
            "#,
        )
        .unwrap();
    let Value::Object(instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[constructor, new_target.clone(), empty.clone()],
        )
        .unwrap()
    else {
        panic!("raw object newTarget did not produce a base instance");
    };
    let Value::Object(expected_prototype) = context.eval("__qjo_raw_custom_prototype").unwrap()
    else {
        panic!("custom newTarget prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(expected_prototype)
    );
    assert_eq!(
        context.eval("__qjo_raw_base_new_target").unwrap(),
        new_target
    );

    let function_prototype = context.function_prototype().unwrap();
    let native = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ConstructorProbe,
            0,
        )
        .unwrap();
    runtime
        .set_constructor_bit(native.as_object(), true)
        .unwrap();
    assert_eq!(
        expect_string_value(
            context
                .call(
                    &wrapper,
                    Value::Undefined,
                    &[
                        Value::Object(native.as_object().clone()),
                        Value::Int(17),
                        empty.clone(),
                    ],
                )
                .unwrap()
        ),
        JsString::from_static("0|0|0|false")
    );

    let bound = context
        .eval(
            r#"
                globalThis.__qjo_bound_target = function BoundTarget() {
                    globalThis.__qjo_bound_rewrite =
                        new.target === globalThis.__qjo_bound_target;
                };
                (globalThis.__qjo_bound = __qjo_bound_target.bind(null))
            "#,
        )
        .unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[bound.clone(), bound, empty.clone()],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        context.eval("__qjo_bound_rewrite").unwrap(),
        Value::Bool(true)
    );

    let nested_bound = context
        .eval(
            r#"
                globalThis.__qjo_nested_bound_target = function (a, b, c) {
                    globalThis.__qjo_nested_bound_log =
                        new.target + ":" + a + ":" + b + ":" + c;
                };
                globalThis.__qjo_nested_bound_inner =
                    __qjo_nested_bound_target.bind(null, 1);
                __qjo_nested_bound_inner.bind(null, 2)
            "#,
        )
        .unwrap();
    let third_argument = context.eval("[3]").unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[nested_bound, Value::Int(17), third_argument],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        expect_string_value(context.eval("__qjo_nested_bound_log").unwrap()),
        JsString::from_static("17:1:2:3")
    );

    let proxy = context
        .eval(
            r#"
                (globalThis.__qjo_raw_proxy = new Proxy(function ProxyTarget() {}, {
                    construct(target, args, newTarget) {
                        globalThis.__qjo_proxy_new_target = newTarget;
                        return { proxy: true };
                    }
                }))
            "#,
        )
        .unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[proxy, Value::Int(17), empty.clone()],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        context.eval("__qjo_proxy_new_target").unwrap(),
        Value::Int(17)
    );

    let proxy_without_trap = context
        .eval(
            r#"
                globalThis.__qjo_proxy_without_trap_target = function () {
                    globalThis.__qjo_proxy_without_trap_new_target = new.target;
                };
                new Proxy(__qjo_proxy_without_trap_target, {})
            "#,
        )
        .unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[proxy_without_trap, Value::Int(17), empty.clone()],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        context.eval("__qjo_proxy_without_trap_new_target").unwrap(),
        Value::Int(17)
    );

    let primitive_proxy_result = context
        .eval("new Proxy(function () {}, { construct() { return 1; } })")
        .unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[primitive_proxy_result, Value::Int(17), empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );

    let revoked_target = context
        .eval(
            "globalThis.__qjo_revoked_target = \
                Proxy.revocable(function () {}, {}); \
             __qjo_revoked_target.revoke(); \
             __qjo_revoked_target.proxy",
        )
        .unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[revoked_target, Value::Int(17), empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );

    let revoked_new_target = context
        .eval(
            "globalThis.__qjo_revoked_new_target = \
                Proxy.revocable(function () {}, {}); \
             __qjo_revoked_new_target.revoke(); \
             __qjo_revoked_new_target.proxy",
        )
        .unwrap();
    let body_probe = context
        .eval(
            "globalThis.__qjo_revoked_body_count = 0; \
             (function () { __qjo_revoked_body_count++; })",
        )
        .unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[body_probe, revoked_new_target, empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context).0,
        JsString::from_static("TypeError")
    );
    assert_eq!(
        context.eval("__qjo_revoked_body_count").unwrap(),
        Value::Int(0)
    );

    let outer_proxy = context
        .eval(
            r#"
                globalThis.__qjo_outer_construct_gets = 0;
                globalThis.__qjo_inner_construct_gets = 0;
                var innerHandler = {};
                Object.defineProperty(innerHandler, "construct", {
                    get() {
                        __qjo_inner_construct_gets++;
                        return undefined;
                    }
                });
                globalThis.__qjo_inner_record =
                    Proxy.revocable(function InnerTarget() {}, innerHandler);
                var outerHandler = {};
                Object.defineProperty(outerHandler, "construct", {
                    get() {
                        __qjo_outer_construct_gets++;
                        return undefined;
                    }
                });
                new Proxy(__qjo_inner_record.proxy, outerHandler)
            "#,
        )
        .unwrap();
    let Value::Object(inner_proxy) = context.eval("__qjo_inner_record.proxy").unwrap() else {
        panic!("inner Proxy was not an object");
    };
    runtime.set_constructor_bit(&inner_proxy, false).unwrap();
    context.eval("__qjo_inner_record.revoke()").unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[outer_proxy, Value::Int(17), empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("not a constructor"),
        )
    );
    assert_eq!(
        context.eval("__qjo_outer_construct_gets").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("__qjo_inner_construct_gets").unwrap(),
        Value::Int(0)
    );

    let non_callable_trap = context
        .eval(
            "globalThis.__qjo_trap_order_target = \
                function TrapOrderTarget() {}; \
             new Proxy(__qjo_trap_order_target, { construct: 1 })",
        )
        .unwrap();
    let Value::Object(trap_order_target) = context.eval("__qjo_trap_order_target").unwrap() else {
        panic!("trap-order target was not an object");
    };
    runtime
        .set_constructor_bit(&trap_order_target, false)
        .unwrap();
    assert_eq!(
        context.call(
            &wrapper,
            Value::Undefined,
            &[non_callable_trap, Value::Int(17), empty.clone()],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_name_and_message(&runtime, &mut context),
        (
            JsString::from_static("TypeError"),
            JsString::from_static("TrapOrderTarget is not a constructor"),
        )
    );

    let derived = context
        .eval(
            r#"
                class RawBase {
                    constructor() {
                        globalThis.__qjo_raw_super_new_target = new.target;
                        this.base = true;
                    }
                }
                class RawDerived extends RawBase {
                    constructor() {
                        super();
                        globalThis.__qjo_raw_derived_new_target = new.target;
                    }
                }
                RawDerived
            "#,
        )
        .unwrap();
    assert!(matches!(
        context
            .call(
                &wrapper,
                Value::Undefined,
                &[derived, Value::Int(17), empty.clone()],
            )
            .unwrap(),
        Value::Object(_)
    ));
    assert_eq!(
        context.eval("__qjo_raw_super_new_target").unwrap(),
        Value::Int(17)
    );
    assert_eq!(
        context.eval("__qjo_raw_derived_new_target").unwrap(),
        Value::Int(17)
    );

    let spread_derived = context
        .eval(
            r#"
                class RawSpreadBase {
                    constructor(a, b) {
                        globalThis.__qjo_raw_spread_super =
                            new.target + ":" + a + ":" + b;
                        this.sum = a + b;
                    }
                }
                class RawSpreadDerived extends RawSpreadBase {
                    constructor(...args) { super(...args); }
                }
                RawSpreadDerived
            "#,
        )
        .unwrap();
    let spread_arguments = context.eval("[20, 22]").unwrap();
    let Value::Object(spread_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[spread_derived, Value::Int(17), spread_arguments],
        )
        .unwrap()
    else {
        panic!("ApplySuper with raw primitive newTarget did not return an object");
    };
    let sum = runtime.intern_property_key("sum").unwrap();
    assert_eq!(
        context.get_property(&spread_instance, &sum).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        expect_string_value(context.eval("__qjo_raw_spread_super").unwrap()),
        JsString::from_static("17:20:22")
    );

    let array = context.eval("Array").unwrap();
    let list = context.eval("[1, 2]").unwrap();
    let Value::Object(array) = context
        .call(&wrapper, Value::Undefined, &[array, Value::Int(17), list])
        .unwrap()
    else {
        panic!("Array with raw primitive newTarget did not return an object");
    };
    assert!(runtime.is_array_object(&array).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&array).unwrap(),
        Some(context.array_prototype().unwrap())
    );
    assert!(!context.has_exception());
}

#[test]
fn construct_only_proxy_and_new_target_do_not_require_call_capability() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let target = context.new_object().unwrap();
    runtime.set_constructor_bit(&target, true).unwrap();
    let global = context.global_object().unwrap();
    let target_key = runtime
        .intern_property_key("__qjo_construct_only_target")
        .unwrap();
    assert!(
        context
            .set_property(&global, &target_key, Value::Object(target.clone()))
            .unwrap()
    );

    let Value::Object(proxy) = context
        .eval(
            r#"
                globalThis.__qjo_construct_only_log = "";
                globalThis.__qjo_construct_only_proxy =
                    new Proxy(__qjo_construct_only_target, {
                        construct(target, args, newTarget) {
                            __qjo_construct_only_seen_new_target = newTarget;
                            __qjo_construct_only_log =
                                (target === __qjo_construct_only_target) + ":" +
                                args[0] + ":" + newTarget;
                            return { trapped: true };
                        }
                    });
                __qjo_construct_only_proxy
            "#,
        )
        .unwrap()
    else {
        panic!("construct-only Proxy was not an object");
    };
    assert!(runtime.is_constructor(&proxy).unwrap());
    assert!(runtime.as_callable(&proxy).unwrap().is_none());

    let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
    assert!(matches!(
        VmHost::construct(
            &mut host,
            Value::Object(proxy.clone()),
            Value::Int(17),
            vec![Value::Int(42)],
        )
        .unwrap(),
        Completion::Return(Value::Object(_))
    ));
    assert_eq!(
        expect_string_value(context.eval("__qjo_construct_only_log").unwrap()),
        JsString::from_static("true:42:17")
    );
    assert_eq!(
        context
            .eval("__qjo_construct_only_seen_new_target")
            .unwrap(),
        Value::Int(17)
    );

    let new_target = context.new_object().unwrap();
    runtime.set_constructor_bit(&new_target, true).unwrap();
    let prototype = context.new_object().unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    assert!(
        context
            .set_property(
                &new_target,
                &prototype_key,
                Value::Object(prototype.clone()),
            )
            .unwrap()
    );
    let new_target_key = runtime
        .intern_property_key("__qjo_construct_only_new_target")
        .unwrap();
    assert!(
        context
            .set_property(&global, &new_target_key, Value::Object(new_target.clone()),)
            .unwrap()
    );
    assert_eq!(
        expect_string_value(
            context
                .eval(
                    r#"
                        (function () {
                            function Base() {}
                            var instance = Reflect.construct(
                                Base,
                                [],
                                __qjo_construct_only_new_target
                            );
                            var trapped = Reflect.construct(
                                __qjo_construct_only_proxy,
                                [9],
                                __qjo_construct_only_new_target
                            );
                            var source = [1];
                            source.constructor = __qjo_construct_only_proxy;
                            return (
                                Object.getPrototypeOf(instance) ===
                                __qjo_construct_only_new_target.prototype
                            ) + ":" + trapped.trapped + ":" +
                                Array.isArray(source.slice());
                        })()
                    "#,
                )
                .unwrap()
        ),
        JsString::from_static("true:true:true")
    );
    assert_eq!(
        context
            .eval("__qjo_construct_only_seen_new_target")
            .unwrap(),
        Value::Object(new_target)
    );

    assert_eq!(
        expect_string_value(
            context
                .eval(
                    r#"
                        (function () {
                            var missing = new Proxy(__qjo_construct_only_target, {});
                            var record = Proxy.revocable(
                                __qjo_construct_only_target,
                                { construct() { return {}; } }
                            );
                            record.revoke();
                            var results = [];
                            for (var candidate of [missing, record.proxy]) {
                                try { Reflect.construct(candidate, []); }
                                catch (error) { results.push(error.message); }
                            }
                            return results.join("|");
                        })()
                    "#,
                )
                .unwrap()
        ),
        JsString::from_static("not a function|revoked proxy")
    );

    assert_eq!(
        expect_string_value(
            context
                .eval(
                    r#"
                        (function () {
                            var log = [];
                            var innerHandler = {};
                            Object.defineProperty(innerHandler, "apply", {
                                get() { log.push("inner"); return undefined; }
                            });
                            var inner = new Proxy({}, innerHandler);
                            var outerHandler = {};
                            Object.defineProperty(outerHandler, "apply", {
                                get() { log.push("outer"); return undefined; }
                            });
                            var outer = new Proxy(inner, outerHandler);
                            var candidate = new Proxy(
                                __qjo_construct_only_target,
                                { construct: outer }
                            );
                            try { Reflect.construct(candidate, []); }
                            catch (error) {
                                return log.join("|") + "|" +
                                    error.name + ":" + error.message;
                            }
                            return "missing";
                        })()
                    "#,
                )
                .unwrap()
        ),
        JsString::from_static("outer|TypeError:not a function")
    );
}

#[test]
fn trusted_quickjs_ordinary_apply_raw_calling_realm_functions_fall_back_to_the_caller() {
    let runtime = Runtime::new();
    let mut caller = runtime.new_context();
    let wrapper = caller
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let array = caller.eval("Array").unwrap();
    let empty = caller.eval("[]").unwrap();
    let caller_array_prototype = caller.array_prototype().unwrap();

    let mut foreign = runtime.new_context();
    let promise_resolve = foreign
        .eval("new Promise(function (resolve) { globalThis.resolve = resolve; }); resolve")
        .unwrap();
    let proxy_revoke = foreign.eval("Proxy.revocable({}, {}).revoke").unwrap();

    for (label, raw_new_target) in [
        ("Promise resolving function", promise_resolve),
        ("Proxy revoke function", proxy_revoke),
    ] {
        let Value::Object(array_instance) = caller
            .call(
                &wrapper,
                Value::Undefined,
                &[array.clone(), raw_new_target, empty.clone()],
            )
            .unwrap()
        else {
            panic!("{label} raw newTarget did not construct an Array");
        };
        assert_eq!(
            runtime.get_prototype_of(&array_instance).unwrap(),
            Some(caller_array_prototype.clone()),
            "{label}"
        );
    }

    foreign
        .eval(
            r#"
                globalThis.__qjo_calling_realm_caught = undefined;
                globalThis.__qjo_calling_realm_promise = new Promise(function (resolve) {
                    globalThis.__qjo_calling_realm_resolve = resolve;
                });
                __qjo_calling_realm_promise.catch(function (error) {
                    globalThis.__qjo_calling_realm_caught = error;
                });
            "#,
        )
        .unwrap();
    let promise = foreign.eval("__qjo_calling_realm_promise").unwrap();
    let resolve = foreign.eval("__qjo_calling_realm_resolve").unwrap();
    let promise_list = foreign.eval("[__qjo_calling_realm_promise]").unwrap();
    let Value::Object(resolve_object) = resolve.clone() else {
        panic!("Promise resolving function was not an object");
    };
    runtime.set_constructor_bit(&resolve_object, true).unwrap();
    assert_eq!(
        caller
            .call(
                &wrapper,
                Value::Undefined,
                &[resolve, Value::Int(17), promise_list],
            )
            .unwrap(),
        Value::Undefined
    );
    runtime.set_constructor_bit(&resolve_object, false).unwrap();
    assert_eq!(
        runtime.execute_pending_job().unwrap().context(),
        Some(caller.realm)
    );
    let Value::Object(caught) = foreign.eval("__qjo_calling_realm_caught").unwrap() else {
        panic!("Promise self-resolution did not capture an error");
    };
    let Value::Object(caller_type_error_prototype) = caller.eval("TypeError.prototype").unwrap()
    else {
        panic!("caller TypeError prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&caught).unwrap(),
        Some(caller_type_error_prototype)
    );
    drop(promise);

    foreign
        .eval(
            "globalThis.__qjo_revoke_record = Proxy.revocable({}, {}); \
             globalThis.__qjo_revoke_proxy = __qjo_revoke_record.proxy;",
        )
        .unwrap();
    let revoke = foreign.eval("__qjo_revoke_record.revoke").unwrap();
    let Value::Object(revoke_object) = revoke.clone() else {
        panic!("Proxy revoke function was not an object");
    };
    runtime.set_constructor_bit(&revoke_object, true).unwrap();
    assert_eq!(
        caller
            .call(&wrapper, Value::Undefined, &[revoke, Value::Int(17), empty],)
            .unwrap(),
        Value::Undefined
    );
    runtime.set_constructor_bit(&revoke_object, false).unwrap();
    assert_eq!(
        foreign.eval("__qjo_revoke_proxy.marker"),
        Err(RuntimeError::Exception)
    );
    assert!(matches!(
        foreign.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
}

#[test]
fn trusted_quickjs_ordinary_apply_raw_native_constructors_use_class_fallbacks() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let wrapper = context
        .read_trusted_ordinary_function(
            &quickjs_ordinary_three_argument_with_code(
                &[0xcf, 0xd0, 0xd1, 0x27, 0x01, 0x00, 0x28],
                3,
            ),
            0,
        )
        .unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();

    let direct_arguments = crate::engine::vm::call::NativeArguments {
        actual_arg_count: 0,
        readable: Vec::new(),
    };
    let Completion::Return(Value::Object(direct_array)) = runtime
        .call_array_constructor(
            context.realm,
            crate::engine::vm::call::NativeInvocation::Construct {
                new_target: Value::Undefined,
            },
            &direct_arguments,
        )
        .unwrap()
    else {
        panic!("Array undefined newTarget required an active function");
    };
    assert_eq!(
        runtime.get_prototype_of(&direct_array).unwrap(),
        Some(context.array_prototype().unwrap())
    );

    let array_constructor = context.eval("Array").unwrap();
    let empty_arguments = context.eval("[]").unwrap();
    let Value::Object(raw_undefined_array) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[array_constructor, Value::Undefined, empty_arguments],
        )
        .unwrap()
    else {
        panic!("raw Array undefined newTarget did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&raw_undefined_array).unwrap(),
        Some(context.array_prototype().unwrap())
    );

    let function_prototype = context.function_prototype().unwrap();
    let custom_array = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ArrayConstructor,
            1,
        )
        .unwrap();
    runtime
        .set_constructor_bit(custom_array.as_object(), true)
        .unwrap();
    let custom_prototype = context.new_object().unwrap();
    assert!(
        context
            .define_own_property(
                custom_array.as_object(),
                &prototype_key,
                &data_descriptor(Value::Object(custom_prototype.clone()), true, false, true,),
            )
            .unwrap()
    );
    let custom_empty_arguments = context.eval("[]").unwrap();
    let Value::Object(custom_array_instance) = context
        .call(
            &wrapper,
            Value::Undefined,
            &[
                Value::Object(custom_array.as_object().clone()),
                Value::Undefined,
                custom_empty_arguments,
            ],
        )
        .unwrap()
    else {
        panic!("custom native Array did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&custom_array_instance).unwrap(),
        Some(context.array_prototype().unwrap())
    );
    assert_ne!(
        runtime.get_prototype_of(&custom_array_instance).unwrap(),
        Some(custom_prototype)
    );

    let cases = [
        ("Array", "Array", "[]"),
        ("Object", "Object", "[]"),
        ("Function", "Function", "['return 1']"),
        (
            "GeneratorFunction",
            "(function* () {}).constructor",
            "['yield 1']",
        ),
        (
            "AsyncFunction",
            "(async function () {}).constructor",
            "['return 1']",
        ),
        (
            "AsyncGeneratorFunction",
            "(async function* () {}).constructor",
            "['yield 1']",
        ),
        ("Boolean", "Boolean", "[true]"),
        ("Number", "Number", "[1]"),
        ("String", "String", "['x']"),
        ("Error", "Error", "['x']"),
        ("EvalError", "EvalError", "['x']"),
        ("RangeError", "RangeError", "['x']"),
        ("ReferenceError", "ReferenceError", "['x']"),
        ("SyntaxError", "SyntaxError", "['x']"),
        ("TypeError", "TypeError", "['x']"),
        ("URIError", "URIError", "['x']"),
        ("InternalError", "InternalError", "['x']"),
        ("AggregateError", "AggregateError", "[[], 'x']"),
        ("Date", "Date", "[0]"),
        ("RegExp", "RegExp", "['a']"),
        ("Map", "Map", "[]"),
        ("Set", "Set", "[]"),
        ("WeakMap", "WeakMap", "[]"),
        ("WeakSet", "WeakSet", "[]"),
        ("Promise", "Promise", "[function (resolve) { resolve(1); }]"),
        ("ArrayBuffer", "ArrayBuffer", "[8]"),
        ("SharedArrayBuffer", "SharedArrayBuffer", "[8]"),
        ("DataView", "DataView", "[new ArrayBuffer(8)]"),
        ("Uint8ClampedArray", "Uint8ClampedArray", "[2]"),
        ("Int8Array", "Int8Array", "[2]"),
        ("Uint8Array", "Uint8Array", "[2]"),
        ("Int16Array", "Int16Array", "[2]"),
        ("Uint16Array", "Uint16Array", "[2]"),
        ("Int32Array", "Int32Array", "[2]"),
        ("Uint32Array", "Uint32Array", "[2]"),
        ("BigInt64Array", "BigInt64Array", "[2]"),
        ("BigUint64Array", "BigUint64Array", "[2]"),
        ("Float16Array", "Float16Array", "[2]"),
        ("Float32Array", "Float32Array", "[2]"),
        ("Float64Array", "Float64Array", "[2]"),
        ("WeakRef", "WeakRef", "[{}]"),
        (
            "FinalizationRegistry",
            "FinalizationRegistry",
            "[function () {}]",
        ),
    ];

    for (label, constructor_source, arguments_source) in cases {
        let constructor = context.eval(constructor_source).unwrap();
        let Value::Object(constructor_object) = constructor.clone() else {
            panic!("{label} constructor was not an object");
        };
        let Value::Object(expected_prototype) = context
            .get_property(&constructor_object, &prototype_key)
            .unwrap()
        else {
            panic!("{label}.prototype was not an object");
        };
        let arguments = context.eval(arguments_source).unwrap();
        let Value::Object(instance) = context
            .call(
                &wrapper,
                Value::Undefined,
                &[constructor, Value::Int(17), arguments],
            )
            .unwrap()
        else {
            panic!("{label} with raw primitive newTarget did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&instance).unwrap(),
            Some(expected_prototype),
            "{label}"
        );
        assert!(!context.has_exception(), "{label}");
    }
}

#[test]
fn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {
    const UNDERFLOW: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: bytecode stack underflow";
    const DECLARED_MAXIMUM: &str = "trusted QuickJS ordinary leaf is not admitted by typed verification: InternalError: declared maximum stack is smaller than required";
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline = runtime.heap_counts();
    let baseline_atoms = runtime.test_atom_count();

    for (label, code, max_stack, expected) in [
        (
            "apply underflow",
            &[0x27, 0x00, 0x00, 0x28][..],
            0,
            UNDERFLOW,
        ),
        (
            "apply declared maximum",
            &[0xcf, 0xd0, 0xd1, 0x27, 0x00, 0x00, 0x28][..],
            2,
            DECLARED_MAXIMUM,
        ),
    ] {
        let image = quickjs_ordinary_three_argument_with_code(code, max_stack);
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

    // The taken label8 jump skips raw rot3l. rot3l expands to Perm3, Swap,
    // so the source target at OP_apply becomes IR instruction 9.
    let code = [
        0xcf, 0xd0, 0xd1, 0xea, 0x05, 0xb4, 0xb5, 0xb6, 0x1d, 0x27, 0x00, 0x00, 0x28,
    ];
    let image = quickjs_ordinary_three_argument_with_code(&code, 3);
    let function = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let receiver = context
        .eval("(globalThis.__qjo_branch_receiver = {})")
        .unwrap();
    let target = context
        .eval("(function () { 'use strict'; return this === globalThis.__qjo_branch_receiver; })")
        .unwrap();
    let list = context.eval("[]").unwrap();
    assert_eq!(
        context
            .call(&function, Value::Undefined, &[target, receiver, list])
            .unwrap(),
        Value::Bool(true)
    );

    let CallableExecution::Bytecode { bytecode, .. } =
        runtime.bytecode_for_callable(&function).unwrap()
    else {
        panic!("branch-to-apply leaf did not publish bytecode");
    };
    let snapshot = runtime.snapshot_function_bytecode(&bytecode).unwrap();
    assert!(matches!(
        snapshot.code.as_ref(),
        [
            Instruction::GetArg(0),
            Instruction::GetArg(1),
            Instruction::GetArg(2),
            Instruction::Goto(9),
            Instruction::PushI32(1),
            Instruction::PushI32(2),
            Instruction::PushI32(3),
            Instruction::Perm3,
            Instruction::Swap,
            Instruction::Apply(ApplyKind::Call),
            Instruction::Return,
        ]
    ));
}
