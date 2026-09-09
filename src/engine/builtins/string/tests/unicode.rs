use super::*;

#[test]
fn string_normalize_matches_quickjs_forms_and_coercion_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (source, expected) in [
        (
            r#"'\u1E9B\u0323'.normalize('NFC')"#,
            [0x1e9b, 0x0323].as_slice(),
        ),
        (
            r#"'\u1E9B\u0323'.normalize('NFD')"#,
            [0x017f, 0x0323, 0x0307].as_slice(),
        ),
        (r#"'\u1E9B\u0323'.normalize('NFKC')"#, [0x1e69].as_slice()),
        (
            r#"'\u1E9B\u0323'.normalize('NFKD')"#,
            [0x0073, 0x0323, 0x0307].as_slice(),
        ),
        (r#"'A\u030a'.normalize()"#, [0x00c5].as_slice()),
        (
            r#"String.fromCharCode(0xd800,0x0301).normalize('NFC')"#,
            [0xd800, 0x0301].as_slice(),
        ),
    ] {
        let Value::String(actual) = context.eval(source).unwrap() else {
            panic!("{source} did not return a String");
        };
        assert_eq!(
            actual.utf16_units().collect::<Vec<_>>(),
            expected,
            "{source}"
        );
    }

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="";
                    var receiver={toString:function(){log+="this,";return "A\u030a"}};
                    var form={toString:function(){log+="form,";return "NFC"}};
                    var value=String.prototype.normalize.call(receiver,form);
                    return value+"|"+log;
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("Å|this,form,")),
    );
    assert_eq!(
        context
            .eval(
                r#"(function(){
                    try { return "x".normalize("bad"); }
                    catch (error) { return error.name+"|"+error.message; }
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("RangeError|bad normalization form",)),
    );
}

#[test]
fn string_locale_compare_matches_quickjs_normalized_code_point_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (source, expected) in [
        (r#""a".localeCompare("c")"#, -2),
        (r#""c".localeCompare("a")"#, 2),
        (r#""a".localeCompare("aa")"#, -1),
        (r#""aa".localeCompare("a")"#, 1),
        (r#""é".localeCompare("e\u0301")"#, 0),
        (r#""ﬁ".localeCompare("fi")"#, 64_155),
        (
            r#"String.fromCharCode(0xd800).localeCompare(String.fromCharCode(0xe000))"#,
            -2_048,
        ),
        (
            r#"String.fromCharCode(0xd800,0xdc00).localeCompare(String.fromCharCode(0xe000))"#,
            8_192,
        ),
        (r#""undefined".localeCompare()"#, 0),
        (r#"String.prototype.localeCompare.call(123,124)"#, -1),
    ] {
        assert_eq!(
            context.eval(source).unwrap(),
            Value::Int(expected),
            "{source}"
        );
    }
}

#[test]
fn string_locale_compare_coerces_only_receiver_then_that() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="";
                    var receiver={};
                    receiver[Symbol.toPrimitive]=function(hint){
                        log+="receiver:"+hint+",";
                        return "e\u0301";
                    };
                    var that={};
                    that[Symbol.toPrimitive]=function(hint){
                        log+="that:"+hint+",";
                        return "é";
                    };
                    var locales=new Proxy({}, {
                        get:function(){throw new Error("locales was observed")}
                    });
                    var options=new Proxy({}, {
                        get:function(){throw new Error("options was observed")}
                    });
                    var result=String.prototype.localeCompare.call(
                        receiver,that,locales,options
                    );
                    return result+"|"+log;
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("0|receiver:string,that:string,")),
    );
    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var hits=0;
                    var that={toString:function(){hits++;return "x"}};
                    try { String.prototype.localeCompare.call(null,that); }
                    catch (error) { return error.name+"|"+hits; }
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("TypeError|0")),
    );
    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="";
                    var receiver={toString:function(){log+="receiver,";return "x"}};
                    var that={toString:function(){log+="that,";throw "stop"}};
                    try { String.prototype.localeCompare.call(receiver,that); }
                    catch (error) { return error+"|"+log; }
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("stop|receiver,that,")),
    );
}

#[test]
fn string_locale_compare_errors_preserve_quickjs_realm_boundaries() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let prototype = defining.string_prototype().unwrap();
    let key = runtime.intern_property_key("localeCompare").unwrap();
    let Value::Object(function_object) = defining.get_property(&prototype, &key).unwrap() else {
        panic!("String.prototype.localeCompare was not an object");
    };
    let function = runtime.as_callable(&function_object).unwrap().unwrap();
    let Value::Object(defining_type_error) = defining.eval("TypeError.prototype").unwrap() else {
        panic!("defining TypeError.prototype was not an object");
    };
    let Value::Object(defining_internal_error) = defining.eval("InternalError.prototype").unwrap()
    else {
        panic!("defining InternalError.prototype was not an object");
    };
    let Value::Object(caller_type_error) = caller.eval("TypeError.prototype").unwrap() else {
        panic!("caller TypeError.prototype was not an object");
    };

    for (this_value, arguments, label) in [
        (
            Value::Null,
            vec![Value::String(JsString::from_static("x"))],
            "null receiver",
        ),
        (
            Value::String(JsString::from_static("x")),
            vec![Value::Symbol(runtime.new_symbol(None).unwrap())],
            "Symbol that",
        ),
    ] {
        assert_eq!(
            caller.call(&function, this_value, &arguments),
            Err(RuntimeError::Exception),
            "{label} did not throw",
        );
        let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
            panic!("{label} did not publish an Error object");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(defining_type_error.clone()),
            "{label} used the caller realm",
        );
    }

    crate::source::unicode::normalize::fail_next_normalize_reservation_for_test();
    assert_eq!(
        caller.call(
            &function,
            Value::String(JsString::from_static("a")),
            &[Value::String(JsString::from_static("b"))],
        ),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("localeCompare normalization OOM did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_internal_error),
        "localeCompare normalization OOM did not use the defining realm",
    );

    assert_eq!(
        caller.construct(&function, &[]),
        Err(RuntimeError::Exception),
        "localeCompare unexpectedly constructed",
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("localeCompare constructor rejection did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error),
        "pre-dispatch constructor rejection did not use the caller realm",
    );
}

#[test]
fn string_locale_compare_and_normalize_share_native_stack_budget() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var compare=String.prototype.localeCompare;
                    var normalize=String.prototype.normalize;
                    var depth=0;
                    var receiver={toString:function(){
                        depth++;
                        if(depth%2)return normalize.call(receiver);
                        return compare.call(receiver,"x");
                    }};
                    try { compare.call(receiver,"x"); }
                    catch (error) { return error.name+"|"+error.message; }
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("InternalError|stack overflow")),
    );
    assert_eq!(
        context.eval(r#""a".localeCompare("c")"#).unwrap(),
        Value::Int(-2)
    );
    assert_eq!(
        context.eval(r#""e\u0301".normalize()"#).unwrap(),
        Value::String(JsString::from_static("é")),
    );
}

#[test]
fn string_normalize_limit_and_oom_use_internal_error_and_recover() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let arguments = NativeArguments {
        actual_arg_count: 1,
        readable: vec![Value::String(JsString::from_static("NFD"))],
    };
    let Completion::Throw(Value::Object(error)) = runtime
        .call_string_prototype_normalize_with_limit(
            defining.realm,
            NativeInvocation::Call {
                this_value: Value::String(JsString::try_from_utf8("ý").unwrap()),
            },
            &arguments,
            1,
        )
        .unwrap()
    else {
        panic!("one-below-boundary normalization did not throw an Error object");
    };
    for (name, expected) in [("name", "InternalError"), ("message", "string too long")] {
        let Value::String(value) = defining
            .get_property(&error, &runtime.intern_property_key(name).unwrap())
            .unwrap()
        else {
            panic!("small-limit normalization {name} was not a String");
        };
        assert_eq!(value, JsString::from_static(expected));
    }
    assert_eq!(
        runtime
            .call_string_prototype_normalize_with_limit(
                defining.realm,
                NativeInvocation::Call {
                    this_value: Value::String(JsString::try_from_utf8("ý").unwrap()),
                },
                &arguments,
                2,
            )
            .unwrap(),
        Completion::Return(Value::String(
            JsString::try_from_utf16([u16::from(b'y'), 0x0301]).unwrap(),
        )),
        "the exact normalization expansion boundary was rejected",
    );

    let prototype = defining.string_prototype().unwrap();
    let key = runtime.intern_property_key("normalize").unwrap();
    let Value::Object(function_object) = defining.get_property(&prototype, &key).unwrap() else {
        panic!("String.prototype.normalize was not an object");
    };
    let function = runtime.as_callable(&function_object).unwrap().unwrap();
    let Value::Object(defining_internal_error) = defining.eval("InternalError.prototype").unwrap()
    else {
        panic!("defining InternalError.prototype was not an object");
    };

    crate::source::unicode::normalize::fail_next_normalize_reservation_for_test();
    assert_eq!(
        caller.call(
            &function,
            Value::String(JsString::try_from_utf8("e\u{301}").unwrap()),
            &[],
        ),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("normalization reservation failure did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_internal_error),
        "normalization reservation OOM did not use the function's defining realm",
    );
    let Value::String(message) = caller
        .get_property(&error, &runtime.intern_property_key("message").unwrap())
        .unwrap()
    else {
        panic!("normalization reservation OOM message was not a String");
    };
    assert_eq!(message, JsString::from_static("out of memory"));
    assert_eq!(
        caller
            .call(
                &function,
                Value::String(JsString::try_from_utf8("e\u{301}").unwrap()),
                &[],
            )
            .unwrap(),
        Value::String(JsString::try_from_utf8("é").unwrap()),
        "runtime did not recover after normalization reservation OOM",
    );
}

#[test]
fn string_normalize_errors_preserve_quickjs_realm_boundaries() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let prototype = defining.string_prototype().unwrap();
    let key = runtime.intern_property_key("normalize").unwrap();
    let Value::Object(function_object) = defining.get_property(&prototype, &key).unwrap() else {
        panic!("String.prototype.normalize was not an object");
    };
    let function = runtime.as_callable(&function_object).unwrap().unwrap();
    let Value::Object(defining_range_error) = defining.eval("RangeError.prototype").unwrap() else {
        panic!("defining RangeError.prototype was not an object");
    };
    let Value::Object(defining_type_error) = defining.eval("TypeError.prototype").unwrap() else {
        panic!("defining TypeError.prototype was not an object");
    };
    let Value::Object(caller_type_error) = caller.eval("TypeError.prototype").unwrap() else {
        panic!("caller TypeError.prototype was not an object");
    };

    for (this_value, arguments, expected_prototype, label) in [
        (
            Value::String(JsString::from_static("x")),
            vec![Value::String(JsString::from_static("bad"))],
            defining_range_error,
            "invalid form",
        ),
        (
            Value::Null,
            Vec::new(),
            defining_type_error.clone(),
            "null receiver",
        ),
        (
            Value::String(JsString::from_static("x")),
            vec![Value::Symbol(runtime.new_symbol(None).unwrap())],
            defining_type_error,
            "Symbol form",
        ),
    ] {
        assert_eq!(
            caller.call(&function, this_value, &arguments),
            Err(RuntimeError::Exception),
            "{label} did not throw",
        );
        let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
            panic!("{label} did not publish an Error object");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(expected_prototype),
            "{label} used the caller realm",
        );
    }

    assert_eq!(
        caller.construct(&function, &[]),
        Err(RuntimeError::Exception),
        "normalize unexpectedly constructed",
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("normalize constructor rejection did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error),
        "pre-dispatch constructor rejection did not use the caller realm",
    );
}
