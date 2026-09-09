use super::*;

#[test]
fn number_intrinsic_graph_payload_constants_and_aliases_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let constructor = global_callable(&runtime, &mut context, "Number");
    let prototype = context.number_prototype().unwrap();

    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert_eq!(
        own_key_names(&runtime, constructor.as_object()),
        [
            "length",
            "name",
            "parseInt",
            "parseFloat",
            "isNaN",
            "isFinite",
            "isInteger",
            "isSafeInteger",
            "MAX_VALUE",
            "MIN_VALUE",
            "NaN",
            "NEGATIVE_INFINITY",
            "POSITIVE_INFINITY",
            "EPSILON",
            "MAX_SAFE_INTEGER",
            "MIN_SAFE_INTEGER",
            "prototype",
        ]
    );
    assert_eq!(
        own_key_names(&runtime, &prototype),
        [
            "toExponential",
            "toFixed",
            "toPrecision",
            "toString",
            "toLocaleString",
            "valueOf",
            "constructor",
        ]
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Number(value))
            if value.to_bits() == 0.0_f64.to_bits()
    ));
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(context.realm)
            .unwrap()
            .primitive_prototypes[PrimitiveKind::Number.index()],
        Some(prototype.object_id())
    );

    for name in ["parseInt", "parseFloat"] {
        let key = runtime.intern_property_key(name).unwrap();
        let global_value = context.get_property(&global, &key).unwrap();
        let static_value = context.get_property(constructor.as_object(), &key).unwrap();
        let (Value::Object(global_value), Value::Object(static_value)) =
            (global_value, static_value)
        else {
            panic!("Number.{name} alias was not an object");
        };
        assert_eq!(static_value, global_value, "Number.{name} identity");
    }

    for (name, expected) in [
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", f64::from_bits(1)),
        ("NaN", f64::NAN),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("POSITIVE_INFINITY", f64::INFINITY),
        ("EPSILON", f64::EPSILON),
        ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
        ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            matches!(
                runtime
                    .get_own_property(constructor.as_object(), &key)
                    .unwrap(),
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value,
                    writable: false,
                    enumerable: false,
                    configurable: false,
                }) if value.same_value(&Value::Float(expected))
            ),
            "Number.{name}"
        );
    }

    assert_eq!(
        context.call(&constructor, Value::Undefined, &[]).unwrap(),
        Value::Int(0)
    );
    let explicit_undefined = context
        .call(&constructor, Value::Undefined, &[Value::Undefined])
        .unwrap();
    assert!(matches!(explicit_undefined, Value::Float(value) if value.is_nan()));
    let Value::Object(wrapper) = context
        .construct(&constructor, &[Value::Float(-0.0)])
        .unwrap()
    else {
        panic!("new Number did not return an object");
    };
    assert_eq!(runtime.get_prototype_of(&wrapper).unwrap(), Some(prototype));
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(wrapper.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Number(value))
            if value.to_bits() == (-0.0_f64).to_bits()
    ));
}

#[test]
fn boolean_intrinsic_graph_payload_and_brand_methods_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = global_callable(&runtime, &mut context, "Boolean");
    let prototype = context.boolean_prototype().unwrap();

    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert_eq!(
        own_key_names(&runtime, constructor.as_object()),
        ["length", "name", "prototype"]
    );
    assert_eq!(
        own_key_names(&runtime, &prototype),
        ["toString", "valueOf", "constructor"]
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Boolean(false))
    ));
    let bigint_prototype = context.bigint_prototype().unwrap();
    let symbol_prototype = context.symbol_prototype().unwrap();
    {
        let state = runtime.0.state.borrow();
        let slots = state
            .heap
            .context(context.realm)
            .unwrap()
            .primitive_prototypes;
        assert_eq!(
            slots[PrimitiveKind::Boolean.index()],
            Some(prototype.object_id())
        );
        assert_eq!(
            slots[PrimitiveKind::BigInt.index()],
            Some(bigint_prototype.object_id())
        );
        assert_eq!(
            slots[PrimitiveKind::Symbol.index()],
            Some(symbol_prototype.object_id())
        );
        assert!(
            slots[PrimitiveKind::String.index()].is_some(),
            "String exotic-core prototype slot was not initialized"
        );
    }

    assert_eq!(
        context.call(&constructor, Value::Undefined, &[]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .call(&constructor, Value::Undefined, &[Value::Int(1)])
            .unwrap(),
        Value::Bool(true)
    );
    let wrapper = context
        .construct(&constructor, &[Value::Bool(false)])
        .unwrap();
    let Value::Object(wrapper) = wrapper else {
        panic!("new Boolean did not return an object");
    };
    assert_eq!(runtime.own_property_keys(&wrapper).unwrap(), []);
    assert_eq!(
        runtime.get_prototype_of(&wrapper).unwrap(),
        Some(prototype.clone())
    );
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
    let to_string = property_callable(&runtime, &mut context, &prototype, "toString");
    assert_eq!(
        context
            .call(&value_of, Value::Object(wrapper.clone()), &[])
            .unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(wrapper.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("false"))
    );
    assert_eq!(
        context.call(&value_of, Value::Bool(true), &[]).unwrap(),
        Value::Bool(true)
    );
    let spoof = runtime.new_object(Some(&prototype)).unwrap();
    assert!(matches!(
        context.call(&value_of, Value::Object(spoof), &[]),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("not a boolean")
    );
    assert_eq!(
        context
            .eval("true.toString() + '|' + false.valueOf()")
            .unwrap(),
        Value::String(JsString::from_static("true|false"))
    );
    assert_eq!(context.eval("+new Boolean(false)").unwrap(), Value::Int(0));
}

#[test]
fn bigint_intrinsic_graph_conversion_truncation_and_wrappers_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = global_callable(&runtime, &mut context, "BigInt");
    let prototype = context.bigint_prototype().unwrap();

    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(prototype.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Ordinary
    ));

    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let value_of_key = runtime.intern_property_key("valueOf").unwrap();
    let constructor_key = runtime.intern_property_key("constructor").unwrap();
    let tag_key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert_eq!(
        runtime.own_property_keys(&prototype).unwrap(),
        [
            to_string_key,
            value_of_key,
            constructor_key,
            tag_key.clone(),
        ]
    );
    assert!(matches!(
        runtime.get_own_property(&prototype, &tag_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("BigInt")
    ));
    assert_eq!(
        own_key_names(&runtime, constructor.as_object()),
        ["length", "name", "asUintN", "asIntN", "prototype"]
    );

    assert_eq!(
        context
            .call(&constructor, Value::Undefined, &[Value::Int(42)])
            .unwrap(),
        Value::BigInt(JsBigInt::from(42))
    );
    assert_eq!(
        context
            .call(
                &constructor,
                Value::Undefined,
                &[Value::String(JsString::from_static("0x10000000000000000"))],
            )
            .unwrap(),
        Value::BigInt(JsBigInt::parse_js_string("0x10000000000000000").unwrap())
    );
    assert!(matches!(
        context.construct(&constructor, &[Value::Int(1)]),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("BigInt is not a constructor")
    );

    let as_uint_n = property_callable(&runtime, &mut context, constructor.as_object(), "asUintN");
    let as_int_n = property_callable(&runtime, &mut context, constructor.as_object(), "asIntN");
    assert_eq!(
        context
            .call(
                &as_uint_n,
                Value::Undefined,
                &[Value::Int(64), Value::BigInt(JsBigInt::from(-1))],
            )
            .unwrap(),
        Value::BigInt(JsBigInt::from(-1))
    );
    assert_eq!(
        context
            .call(
                &as_int_n,
                Value::Undefined,
                &[Value::Int(8), Value::BigInt(JsBigInt::from(255))],
            )
            .unwrap(),
        Value::BigInt(JsBigInt::from(-1))
    );

    let to_string = property_callable(&runtime, &mut context, &prototype, "toString");
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
    assert_eq!(
        context
            .call(
                &to_string,
                Value::BigInt(JsBigInt::from(255)),
                &[Value::Int(16)],
            )
            .unwrap(),
        Value::String(JsString::from_static("ff"))
    );
    assert!(matches!(
        context.call(&value_of, Value::Object(prototype.clone()), &[]),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("not a BigInt")
    );

    let object_prototype = context.object_prototype().unwrap();
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    let Value::Object(wrapper) = context
        .call(&object_value_of, Value::BigInt(JsBigInt::from(123)), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box a BigInt primitive");
    };
    assert_eq!(runtime.get_prototype_of(&wrapper).unwrap(), Some(prototype));
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(wrapper.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::BigInt(value))
            if value == &JsBigInt::from(123)
    ));
    assert_eq!(
        context
            .call(&value_of, Value::Object(wrapper), &[])
            .unwrap(),
        Value::BigInt(JsBigInt::from(123))
    );
}

#[test]
fn global_numeric_parsers_match_quickjs_graph_conversion_order_and_results() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let parse_int = global_callable(&runtime, &mut context, "parseInt");
    let parse_float = global_callable(&runtime, &mut context, "parseFloat");

    for (name, callable, length) in [("parseInt", &parse_int, 2), ("parseFloat", &parse_float, 1)] {
        assert_eq!(
            own_key_names(&runtime, callable.as_object()),
            ["length", "name"]
        );
        assert_eq!(
            runtime.get_prototype_of(callable.as_object()).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert!(!runtime.is_constructor(callable.as_object()).unwrap());
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "length"),
            Value::Int(length)
        );
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "name"),
            Value::String(JsString::try_from_utf8(name).unwrap())
        );
        for property in ["length", "name"] {
            let property = runtime.intern_property_key(property).unwrap();
            assert!(matches!(
                runtime
                    .get_own_property(callable.as_object(), &property)
                    .unwrap(),
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    ..
                })
            ));
        }
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime.get_own_property(&global, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: true,
                enumerable: false,
                configurable: true,
                ..
            })
        ));
    }

    assert_eq!(
        context
            .call(
                &parse_int,
                Value::Bool(true),
                &[Value::String(JsString::from_static("0x10"))],
            )
            .unwrap(),
        Value::Int(16)
    );
    assert_eq!(
        context
            .call(
                &parse_int,
                Value::Undefined,
                &[
                    Value::String(JsString::from_static("10")),
                    Value::Float(4_294_967_298.0),
                ],
            )
            .unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context
            .call(
                &parse_float,
                Value::Undefined,
                &[Value::String(JsString::from_static(
                    "1.0000000000000001110223024625156540423631668090820313",
                ))],
            )
            .unwrap(),
        Value::Int(1)
    );
    let Value::Float(negative_zero) = context
        .call(
            &parse_int,
            Value::Undefined,
            &[Value::String(JsString::from_static("-0"))],
        )
        .unwrap()
    else {
        panic!("parseInt('-0') did not preserve the float tag");
    };
    assert_eq!(negative_zero.to_bits(), (-0.0_f64).to_bits());

    let log_key = runtime.intern_property_key("parseLog").unwrap();
    assert!(
        runtime
            .define_own_property(
                &global,
                &log_key,
                &data_descriptor(Value::String(JsString::from_static("")), true, true, true),
            )
            .unwrap()
    );
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    let input = context.new_object().unwrap();
    let input_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { parseLog = parseLog + 'input:' + hint + '|'; return '10'; })",
    );
    assert!(
        runtime
            .define_own_property(
                &input,
                &to_primitive,
                &data_descriptor(
                    Value::Object(input_conversion.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    let radix = context.new_object().unwrap();
    let radix_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { parseLog = parseLog + 'radix:' + hint + '|'; return 2; })",
    );
    assert!(
        runtime
            .define_own_property(
                &radix,
                &to_primitive,
                &data_descriptor(
                    Value::Object(radix_conversion.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(
                &parse_int,
                Value::Undefined,
                &[Value::Object(input.clone()), Value::Object(radix)],
            )
            .unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        context.get_property(&global, &log_key).unwrap(),
        Value::String(JsString::from_static("input:string|radix:number|"))
    );

    assert!(
        runtime
            .define_own_property(
                &global,
                &log_key,
                &data_descriptor(Value::String(JsString::from_static("")), true, true, true),
            )
            .unwrap()
    );
    let throwing_input = context.new_object().unwrap();
    let input_throw = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { parseLog = parseLog + 'input-throw:' + hint + '|'; throw 'input boom'; })",
    );
    assert!(
        runtime
            .define_own_property(
                &throwing_input,
                &to_primitive,
                &data_descriptor(
                    Value::Object(input_throw.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    let late_radix = context.new_object().unwrap();
    let late_radix_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function() { parseLog = parseLog + 'late-radix|'; return 2; })",
    );
    assert!(
        runtime
            .define_own_property(
                &late_radix,
                &to_primitive,
                &data_descriptor(
                    Value::Object(late_radix_conversion.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(
            &parse_int,
            Value::Undefined,
            &[Value::Object(throwing_input), Value::Object(late_radix),],
        ),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static("input boom")))
    );
    assert_eq!(
        context.get_property(&global, &log_key).unwrap(),
        Value::String(JsString::from_static("input-throw:string|"))
    );

    let symbol = runtime
        .new_symbol(Some(JsString::from_static("parse")))
        .unwrap();
    assert!(
        runtime
            .define_own_property(
                &global,
                &log_key,
                &data_descriptor(Value::String(JsString::from_static("")), true, true, true),
            )
            .unwrap()
    );
    let symbol_radix = context.new_object().unwrap();
    let symbol_radix_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function() { parseLog = parseLog + 'symbol-radix|'; return 2; })",
    );
    assert!(
        runtime
            .define_own_property(
                &symbol_radix,
                &to_primitive,
                &data_descriptor(
                    Value::Object(symbol_radix_conversion.as_object().clone()),
                    true,
                    false,
                    true,
                ),
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(
            &parse_int,
            Value::Undefined,
            &[Value::Symbol(symbol.clone()), Value::Object(symbol_radix),],
        ),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("cannot convert symbol to string")
    );
    assert_eq!(
        context.get_property(&global, &log_key).unwrap(),
        Value::String(JsString::from_static(""))
    );
    assert!(matches!(
        context.call(&parse_float, Value::Undefined, &[Value::Symbol(symbol)],),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("cannot convert symbol to string")
    );

    let type_error = global_callable(&runtime, &mut context, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(defining_type_error_prototype) = context
        .get_property(type_error.as_object(), &prototype_key)
        .unwrap()
    else {
        panic!("defining TypeError.prototype was not an object");
    };
    let mut caller = runtime.new_context();
    assert_eq!(
        caller.call(
            &parse_int,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("10")),
                Value::BigInt(JsBigInt::one()),
            ],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm parseInt did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error_prototype)
    );
}

#[test]
fn global_numeric_predicates_match_quickjs_graph_and_coercion_split() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    for name in ["isNaN", "isFinite"] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(runtime.is_auto_init_own_property(&global, &key).unwrap());
    }
    let is_nan = global_callable(&runtime, &mut context, "isNaN");
    let is_finite = global_callable(&runtime, &mut context, "isFinite");
    let number = global_callable(&runtime, &mut context, "Number");
    let number_is_nan = property_callable(&runtime, &mut context, number.as_object(), "isNaN");
    let number_is_finite =
        property_callable(&runtime, &mut context, number.as_object(), "isFinite");
    assert_ne!(is_nan.as_object(), number_is_nan.as_object());
    assert_ne!(is_finite.as_object(), number_is_finite.as_object());

    for (name, callable) in [("isNaN", &is_nan), ("isFinite", &is_finite)] {
        assert_eq!(
            own_key_names(&runtime, callable.as_object()),
            ["length", "name"]
        );
        assert_eq!(
            runtime.get_prototype_of(callable.as_object()).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert!(!runtime.is_constructor(callable.as_object()).unwrap());
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "length"),
            Value::Int(1)
        );
        assert_eq!(
            own_data_value(&runtime, callable.as_object(), "name"),
            Value::String(JsString::try_from_utf8(name).unwrap())
        );
        let key = runtime.intern_property_key(name).unwrap();
        assert!(matches!(
            runtime.get_own_property(&global, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: true,
                enumerable: false,
                configurable: true,
                ..
            })
        ));
    }

    for (input, nan, finite) in [
        (Value::Undefined, true, false),
        (Value::Null, false, true),
        (Value::Bool(false), false, true),
        (Value::String(JsString::from_static("")), false, true),
        (Value::String(JsString::from_static("number")), true, false),
        (Value::Float(f64::NAN), true, false),
        (Value::Float(f64::INFINITY), false, false),
        (Value::Float(f64::NEG_INFINITY), false, false),
        (Value::Float(f64::from_bits(1)), false, true),
    ] {
        assert_eq!(
            context
                .call(
                    &is_nan,
                    Value::String(JsString::from_static("ignored")),
                    std::slice::from_ref(&input)
                )
                .unwrap(),
            Value::Bool(nan)
        );
        assert_eq!(
            context
                .call(&is_finite, Value::Null, &[input, Value::Int(99)])
                .unwrap(),
            Value::Bool(finite)
        );
    }
    assert_eq!(
        context.call(&is_nan, Value::Undefined, &[]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context.call(&is_finite, Value::Undefined, &[]).unwrap(),
        Value::Bool(false)
    );
    for callable in [&is_nan, &is_finite] {
        assert_eq!(
            context.call(
                callable,
                Value::Undefined,
                &[Value::BigInt(JsBigInt::one())],
            ),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from_static("cannot convert bigint to number")
        );
    }
    assert_eq!(
        context.eval("isNaN('x') + '|' + isFinite('1')").unwrap(),
        Value::String(JsString::from_static("true|true"))
    );
}
