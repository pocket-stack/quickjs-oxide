use super::*;

#[test]
fn dynamic_source_builder_latches_utf16_length_failure() {
    let mut exact = DynamicSourceBuilder::with_limit(3);
    assert_eq!(exact.push_str("a😀"), Ok(()));
    assert_eq!(exact.finish(), Ok(JsString::try_from_utf8("a😀").unwrap()));

    let high = JsString::try_from_utf16([0xd801]).unwrap();
    let low = JsString::try_from_utf16([0xdc00]).unwrap();
    let mut split_pair = DynamicSourceBuilder::with_limit(2);
    assert_eq!(split_pair.push_js_string(&high), Ok(()));
    assert_eq!(split_pair.push_js_string(&low), Ok(()));
    assert_eq!(
        split_pair
            .finish()
            .unwrap()
            .utf16_units()
            .collect::<Vec<_>>(),
        vec![0xd801, 0xdc00]
    );

    let mut failed = DynamicSourceBuilder::with_limit(5);
    assert_eq!(failed.push_str("a😀"), Ok(()));

    assert_eq!(failed.push_str("abc"), Err(JsStringError::TooLong));

    assert_eq!(failed.push_str("b"), Err(JsStringError::TooLong));
    assert_eq!(failed.push_str(""), Err(JsStringError::TooLong));
    assert_eq!(failed.finish(), Err(JsStringError::TooLong));
}

#[test]
fn function_constructor_intrinsic_and_dynamic_source_match_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let global = context.global_object().unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let name = runtime.intern_property_key("name").unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    let constructor_key = runtime.intern_property_key("constructor").unwrap();
    let function_key = runtime.intern_property_key("Function").unwrap();

    assert!(runtime.is_constructor(constructor.as_object()).unwrap());
    assert_eq!(runtime.callable_realm(&constructor).unwrap(), context.realm);
    assert_eq!(
        runtime.get_prototype_of(constructor.as_object()).unwrap(),
        Some(function_prototype.clone())
    );
    assert_eq!(
        runtime.own_property_keys(constructor.as_object()).unwrap(),
        vec![length.clone(), name.clone(), prototype.clone()]
    );
    assert!(matches!(
        runtime
            .get_own_property(constructor.as_object(), &length)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(1),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));
    assert!(matches!(
        runtime
            .get_own_property(constructor.as_object(), &name)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("Function")
    ));
    assert!(matches!(
        runtime
            .get_own_property(constructor.as_object(), &prototype)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: false,
            enumerable: false,
            configurable: false,
        }) if value == function_prototype
    ));
    assert!(matches!(
        runtime.get_own_property(&global, &function_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == constructor.as_object().clone()
    ));
    assert!(matches!(
        runtime
            .get_own_property(&function_prototype, &constructor_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == constructor.as_object().clone()
    ));
    {
        let state = runtime.0.state.borrow();
        let ObjectPayload::NativeFunction { data, .. } = &state
            .heap
            .object(constructor.as_object().object_id())
            .unwrap()
            .payload
        else {
            panic!("Function was not a native constructor");
        };
        assert_eq!(
            data.target,
            NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal)
        );
        assert_eq!(
            data.target.descriptor().cproto,
            NativeCProto::ConstructorOrFunctionMagic
        );
        assert_eq!(data.min_readable_args, 1);
    }

    let to_string = runtime.intern_property_key("toString").unwrap();
    let Value::Object(to_string) = context
        .get_property(&function_prototype, &to_string)
        .unwrap()
    else {
        panic!("Function.prototype.toString was not callable");
    };
    let to_string = runtime.as_callable(&to_string).unwrap().unwrap();
    assert_eq!(
        context
            .call(
                &to_string,
                Value::Object(constructor.as_object().clone()),
                &[],
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "function Function() {\n    [native code]\n}"
        ))
    );

    let Value::Object(empty) = context.call(&constructor, Value::Null, &[]).unwrap() else {
        panic!("Function() did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&empty).unwrap(),
        Some(function_prototype)
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(empty.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("function anonymous(\n) {\n\n}"))
    );
    for (property_name, expected) in [
        ("name", Value::String(JsString::from_static("anonymous"))),
        ("length", Value::Int(0)),
        ("fileName", Value::String(JsString::from_static("<input>"))),
        ("lineNumber", Value::Int(1)),
        ("columnNumber", Value::Int(2)),
    ] {
        let key = runtime.intern_property_key(property_name).unwrap();
        assert_eq!(context.get_property(&empty, &key).unwrap(), expected);
    }

    let Value::Object(add) = context
        .call(
            &constructor,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("b")),
                Value::String(JsString::from_static("return a + b")),
            ],
        )
        .unwrap()
    else {
        panic!("Function parameters did not produce an object");
    };
    let add_callable = runtime.as_callable(&add).unwrap().unwrap();
    assert_eq!(
        context
            .call(
                &add_callable,
                Value::Undefined,
                &[Value::Int(20), Value::Int(22)],
            )
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context.call(&to_string, Value::Object(add), &[]).unwrap(),
        Value::String(JsString::from_static(
            "function anonymous(a,b\n) {\nreturn a + b\n}"
        ))
    );

    let Value::Object(duplicate) = context
        .call(
            &constructor,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("return a")),
            ],
        )
        .unwrap()
    else {
        panic!("sloppy duplicate parameters were rejected");
    };
    let duplicate = runtime.as_callable(&duplicate).unwrap().unwrap();
    assert_eq!(
        context
            .call(
                &duplicate,
                Value::Undefined,
                &[Value::Int(1), Value::Int(2)],
            )
            .unwrap(),
        Value::Int(2)
    );

    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("a")),
                Value::String(JsString::from_static("\"use strict\"; return a")),
            ],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("strict duplicate parameters did not throw an Error");
    };
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );

    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf16([0xd800]).unwrap(),)],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("lone-surrogate source did not throw an Error");
    };
    for (property_name, expected) in [
        ("name", Value::String(JsString::from_static("SyntaxError"))),
        (
            "message",
            Value::String(JsString::from_static("unexpected character")),
        ),
        ("fileName", Value::String(JsString::from_static("<input>"))),
        ("lineNumber", Value::Int(3)),
        ("columnNumber", Value::Int(1)),
    ] {
        let key = runtime.intern_property_key(property_name).unwrap();
        assert_eq!(context.get_property(&error, &key).unwrap(), expected);
    }
}

#[test]
fn function_constructor_html_comment_boundaries_match_quickjs_wrapper() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"[
                    typeof Function("-->"),
                    (function () {
                        try {
                            Function("-->", "");
                            return "missing";
                        } catch (error) {
                            return error.name;
                        }
                    })(),
                    typeof Function("\n-->", ""),
                    typeof Function("<!--"),
                    typeof Function("<!--", "")
                ].join("|")"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "function|SyntaxError|function|function|function"
        ))
    );
}

#[test]
fn function_constructor_uses_defining_realm_and_new_target_prototype() {
    let runtime = Runtime::new();
    let first = runtime.new_context();
    let mut second = runtime.new_context();
    let constructor = first.function_constructor().unwrap();
    let first_function_prototype = first.function_prototype().unwrap();
    let second_function_prototype = second.function_prototype().unwrap();
    let first_object_prototype = first.object_prototype().unwrap();
    let marker = runtime.intern_property_key("dynamicRealmMarker").unwrap();
    runtime
        .define_function_data_property(
            &first.global_object().unwrap(),
            "dynamicRealmMarker",
            Value::Int(11),
            true,
            true,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            &second.global_object().unwrap(),
            "dynamicRealmMarker",
            Value::Int(22),
            true,
            true,
        )
        .unwrap();
    assert_eq!(
        second
            .get_property(&first.global_object().unwrap(), &marker)
            .unwrap(),
        Value::Int(11)
    );

    let Value::Object(dynamic) = second
        .call(
            &constructor,
            Value::Object(second.global_object().unwrap()),
            &[Value::String(JsString::from_static(
                "return dynamicRealmMarker",
            ))],
        )
        .unwrap()
    else {
        panic!("cross-realm Function call did not return an object");
    };
    let dynamic_callable = runtime.as_callable(&dynamic).unwrap().unwrap();
    assert_eq!(
        runtime.callable_realm(&dynamic_callable).unwrap(),
        first.realm
    );
    assert_eq!(
        runtime.get_prototype_of(&dynamic).unwrap(),
        Some(first_function_prototype.clone())
    );
    assert_eq!(
        second
            .call(&dynamic_callable, Value::Undefined, &[])
            .unwrap(),
        Value::Int(11)
    );

    let Value::Object(new_target) = second.eval("(function NewTarget(){})").unwrap() else {
        panic!("newTarget source did not return a function");
    };
    let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let custom_prototype = second.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Object(custom_prototype.clone())),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(customized) = second
        .construct_with_new_target(
            &constructor,
            &new_target,
            &[Value::String(JsString::from_static(
                "return dynamicRealmMarker",
            ))],
        )
        .unwrap()
    else {
        panic!("custom newTarget did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&customized).unwrap(),
        Some(custom_prototype)
    );
    let customized_callable = runtime.as_callable(&customized).unwrap().unwrap();
    assert_eq!(
        second
            .call(&customized_callable, Value::Undefined, &[])
            .unwrap(),
        Value::Int(11)
    );
    let Value::Object(instance_prototype) =
        second.get_property(&customized, &prototype_key).unwrap()
    else {
        panic!("dynamic function prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance_prototype).unwrap(),
        Some(first_object_prototype)
    );

    assert!(
        runtime
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(fallback) = second
        .construct_with_new_target(
            &constructor,
            &new_target,
            &[Value::String(JsString::from_static("return 3"))],
        )
        .unwrap()
    else {
        panic!("fallback newTarget did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback).unwrap(),
        Some(second_function_prototype)
    );
    assert_eq!(runtime.callable_realm(&constructor).unwrap(), first.realm);
}

#[test]
fn function_constructor_samples_strip_mode_and_keeps_parse_locations() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let Value::Object(to_string) = context
        .get_property(&function_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Function.prototype.toString was not an object");
    };
    let to_string = runtime.as_callable(&to_string).unwrap().unwrap();
    let keys = ["fileName", "lineNumber", "columnNumber"]
        .map(|name| runtime.intern_property_key(name).unwrap());

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let Value::Object(source_stripped) = context
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static("return 1"))],
        )
        .unwrap()
    else {
        panic!("strip-source Function did not return an object");
    };
    for (key, expected) in keys.iter().zip([
        Value::String(JsString::from_static("<input>")),
        Value::Int(1),
        Value::Int(2),
    ]) {
        assert_eq!(
            context.get_property(&source_stripped, key).unwrap(),
            expected
        );
    }
    assert_eq!(
        context
            .call(&to_string, Value::Object(source_stripped.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function anonymous() {\n    [native code]\n}"
        ))
    );
    let name = runtime.intern_property_key("name").unwrap();
    assert!(
        runtime
            .define_own_property(
                &source_stripped,
                &name,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(JsString::from_static(
                        "renamed"
                    ))),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(source_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function renamed() {\n    [native code]\n}"
        ))
    );

    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let Value::Object(debug_stripped) = context
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static("return 2"))],
        )
        .unwrap()
    else {
        panic!("strip-debug Function did not return an object");
    };
    for key in &keys {
        assert_eq!(
            context.get_property(&debug_stripped, key).unwrap(),
            Value::Undefined
        );
    }
    assert_eq!(
        context
            .call(&to_string, Value::Object(debug_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function anonymous() {\n    [native code]\n}"
        ))
    );

    let message = runtime.intern_property_key("message").unwrap();
    for (description, body, expected_name, expected_message) in [
        (
            "local TDZ",
            "for(let value=value;false;){}",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "direct-eval local TDZ",
            "eval('');for(let value=value;false;){}",
            "ReferenceError",
            "value is not initialized",
        ),
        (
            "direct-eval environment TDZ",
            "let value=eval('value')",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "nested direct-eval environment TDZ",
            "let value=eval(\"eval('');value\")",
            "ReferenceError",
            "value is not initialized",
        ),
        (
            "closure TDZ read",
            "let read=()=>value;return read();let value",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "closure TDZ write",
            "let write=()=>value=1;return write();let value",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "parent direct-eval child closure TDZ",
            "eval('');let read=()=>value;return read();let value",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "child direct-eval closure TDZ",
            "let read=()=>{eval('');return value};return read();let value",
            "ReferenceError",
            "value is not initialized",
        ),
        (
            "derived this TDZ with descendant eval",
            "class A extends Object{constructor(){let f=()=>eval('');this.x;super()}}new A",
            "ReferenceError",
            "lexical variable is not initialized",
        ),
        (
            "closure readonly write",
            "let write;{const value=1;write=()=>value=2}return write()",
            "TypeError",
            "'value' is read-only",
        ),
    ] {
        let Value::Object(function) = context
            .call(
                &constructor,
                Value::Undefined,
                &[Value::String(JsString::from_static(body))],
            )
            .unwrap_or_else(|error| panic!("strip-debug {description} failed to compile: {error}"))
        else {
            panic!("strip-debug {description} did not return an object");
        };
        let function = runtime.as_callable(&function).unwrap().unwrap();
        assert_eq!(
            context.call(&function, Value::Undefined, &[]),
            Err(RuntimeError::Exception),
            "{description}"
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("strip-debug {description} did not throw an Error");
        };
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from_static(expected_name)),
            "{description}"
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from_static(expected_message)),
            "{description}"
        );
    }

    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[
                Value::String(JsString::from_static("a-")),
                Value::String(JsString::from_static("return 1")),
            ],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("malformed Function did not throw an Error");
    };
    for (name, expected) in [
        ("fileName", Value::String(JsString::from_static("<input>"))),
        ("lineNumber", Value::Int(1)),
        ("columnNumber", Value::Int(22)),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(context.get_property(&error, &key).unwrap(), expected);
    }
    let stack = runtime.intern_property_key("stack").unwrap();
    let Value::String(stack) = context.get_property(&error, &stack).unwrap() else {
        panic!("Function syntax error had no stack");
    };
    assert_eq!(
        stack,
        JsString::from_static("    at <input>:1:22\n    at Function (native)\n")
    );
}

#[test]
fn function_constructor_orders_source_conversion_parse_and_prototype_get() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let global = context.global_object().unwrap();
    runtime
        .define_function_data_property(
            &global,
            "functionOrder",
            Value::String(JsString::from_static("")),
            true,
            true,
        )
        .unwrap();
    let custom_prototype = context.new_object().unwrap();
    runtime
        .define_function_data_property(
            &global,
            "functionCustomPrototype",
            Value::Object(custom_prototype.clone()),
            true,
            true,
        )
        .unwrap();

    let (
        parameter_to_string,
        body_to_string,
        bad_parameter_to_string,
        throwing_to_string,
        prototype_getter,
    ) = {
        let mut eval_callable = |source: &str| {
            let Value::Object(function) = context.eval(source).unwrap() else {
                panic!("conversion helper was not a function");
            };
            runtime.as_callable(&function).unwrap().unwrap()
        };
        (
            eval_callable("(function(){ functionOrder = functionOrder + \"p\"; return \"a\"; })"),
            eval_callable(
                "(function(){ functionOrder = functionOrder + \"b\"; return \"return a\"; })",
            ),
            eval_callable("(function(){ functionOrder = functionOrder + \"p\"; return \"a-\"; })"),
            eval_callable("(function(){ functionOrder = functionOrder + \"t\"; throw \"stop\"; })"),
            eval_callable(
                "(function(){ functionOrder = functionOrder + \"x\"; return functionCustomPrototype; })",
            ),
        )
    };

    let to_string = runtime.intern_property_key("toString").unwrap();
    let parameter = context.new_object().unwrap();
    let body = context.new_object().unwrap();
    runtime
        .define_function_data_property(
            &parameter,
            "toString",
            Value::Object(parameter_to_string.as_object().clone()),
            true,
            true,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            &body,
            "toString",
            Value::Object(body_to_string.as_object().clone()),
            true,
            true,
        )
        .unwrap();
    assert!(runtime.has_own_property(&parameter, &to_string).unwrap());

    let function_prototype = context.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind) = context
        .get_property(&function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind).unwrap().unwrap();
    let Value::Object(new_target) = context
        .call(
            &bind,
            Value::Object(constructor.as_object().clone()),
            &[Value::Undefined],
        )
        .unwrap()
    else {
        panic!("bound Function was not an object");
    };
    let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert!(
        runtime
            .define_own_property(
                new_target.as_object(),
                &prototype,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(prototype_getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    let Value::Object(function) = context
        .construct_with_new_target(
            &constructor,
            &new_target,
            &[
                Value::Object(parameter.clone()),
                Value::Object(body.clone()),
            ],
        )
        .unwrap()
    else {
        panic!("converted Function source did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&function).unwrap(),
        Some(custom_prototype)
    );
    let order = runtime.intern_property_key("functionOrder").unwrap();
    assert_eq!(
        context.get_property(&global, &order).unwrap(),
        Value::String(JsString::from_static("pbx"))
    );

    runtime
        .define_function_data_property(
            &global,
            "functionOrder",
            Value::String(JsString::from_static("")),
            true,
            true,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            &parameter,
            "toString",
            Value::Object(bad_parameter_to_string.as_object().clone()),
            true,
            true,
        )
        .unwrap();
    assert_eq!(
        context.construct_with_new_target(
            &constructor,
            &new_target,
            &[
                Value::Object(parameter.clone()),
                Value::Object(body.clone())
            ],
        ),
        Err(RuntimeError::Exception)
    );
    drop(context.take_exception().unwrap());
    assert_eq!(
        context.get_property(&global, &order).unwrap(),
        Value::String(JsString::from_static("pb"))
    );

    runtime
        .define_function_data_property(
            &global,
            "functionOrder",
            Value::String(JsString::from_static("")),
            true,
            true,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            &parameter,
            "toString",
            Value::Object(throwing_to_string.as_object().clone()),
            true,
            true,
        )
        .unwrap();
    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[Value::Object(parameter), Value::Object(body)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static("stop")))
    );
    assert_eq!(
        context.get_property(&global, &order).unwrap(),
        Value::String(JsString::from_static("t"))
    );
}

#[test]
fn function_constructor_typed_realm_root_and_cycles_are_collectable() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let realm = context.realm;
    let constructor = context.function_constructor().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let global = context.global_object().unwrap();
    let function_key = runtime.intern_property_key("Function").unwrap();
    let constructor_key = runtime.intern_property_key("constructor").unwrap();

    assert!(runtime.delete_property(&global, &function_key).unwrap());
    assert!(
        runtime
            .delete_property(&function_prototype, &constructor_key)
            .unwrap()
    );
    drop(constructor);
    let rooted = context.function_constructor().unwrap();
    assert!(runtime.is_constructor(rooted.as_object()).unwrap());
    assert!(matches!(
        runtime.bytecode_for_callable(&rooted).unwrap(),
        CallableExecution::Native {
            target: NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal),
            realm: target_realm,
            min_readable_args: 1,
        } if target_realm == realm
    ));

    drop(rooted);
    drop(function_key);
    drop(constructor_key);
    drop(global);
    drop(function_prototype);
    drop(context);
    runtime.run_gc().unwrap();
    assert!(runtime.0.state.borrow().heap.context(realm).is_err());
    let counts = runtime.heap_counts();
    assert_eq!(counts.context_nodes, 0);
    assert_eq!(counts.object_nodes, 0);
    assert_eq!(counts.shape_nodes, 0);
    assert_eq!(counts.function_bytecode_nodes, 0);
}

#[test]
fn dynamic_function_keeps_its_defining_realm_alive() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let defining_realm = defining.realm;
    let constructor = defining.function_constructor().unwrap();
    let Value::Object(function_object) = defining
        .call(
            &constructor,
            Value::Undefined,
            &[Value::String(JsString::from_static("return 9"))],
        )
        .unwrap()
    else {
        panic!("Function did not return a bytecode function");
    };
    let function = runtime.as_callable(&function_object).unwrap().unwrap();
    drop(function_object);
    let mut caller = runtime.new_context();

    drop(constructor);
    drop(defining);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .is_ok()
    );
    assert_eq!(
        caller.call(&function, Value::Undefined, &[]).unwrap(),
        Value::Int(9)
    );

    drop(function);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .is_err()
    );
    assert_eq!(runtime.heap_counts().context_nodes, 1);
}

#[test]
fn function_constructor_failure_paths_release_temporary_graphs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let live_counts = || {
        let counts = runtime.heap_counts();
        (
            counts.object_nodes,
            counts.shape_nodes,
            counts.var_ref_nodes,
            counts.context_nodes,
            counts.function_bytecode_nodes,
        )
    };

    let parse_baseline = live_counts();
    let parse_atom_baseline = runtime.test_atom_count();
    for _ in 0..3 {
        assert_eq!(
            context.call(
                &constructor,
                Value::Undefined,
                &[
                    Value::String(JsString::from_static("a-")),
                    Value::String(JsString::from_static("return 1")),
                ],
            ),
            Err(RuntimeError::Exception)
        );
        drop(context.take_exception().unwrap());
        runtime.run_gc().unwrap();
        assert_eq!(live_counts(), parse_baseline);
        assert_eq!(runtime.test_atom_count(), parse_atom_baseline);
    }

    let function_prototype = context.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind) = context
        .get_property(&function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind).unwrap().unwrap();
    let Value::Object(new_target) = context
        .call(
            &bind,
            Value::Object(constructor.as_object().clone()),
            &[Value::Undefined],
        )
        .unwrap()
    else {
        panic!("bound Function was not an object");
    };
    let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
    let Value::Object(getter) = context
        .eval("(function(){ throw \"prototype\"; })")
        .unwrap()
    else {
        panic!("prototype getter was not an object");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert!(
        runtime
            .define_own_property(
                new_target.as_object(),
                &prototype,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    runtime.run_gc().unwrap();
    let getter_baseline = live_counts();
    let getter_atom_baseline = runtime.test_atom_count();
    for _ in 0..3 {
        assert_eq!(
            context.construct_with_new_target(
                &constructor,
                &new_target,
                &[Value::String(JsString::from_static("return 1"))],
            ),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            context.take_exception().unwrap(),
            Some(Value::String(JsString::from_static("prototype")))
        );
        runtime.run_gc().unwrap();
        assert_eq!(live_counts(), getter_baseline);
        assert_eq!(runtime.test_atom_count(), getter_atom_baseline);
    }
}
