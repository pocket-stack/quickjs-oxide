use super::*;

#[test]
fn string_trim_preserves_whitespace_sides_utf16_rope_identity_and_argument_ignorance() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#""\u0009\u000a\u000b\u000c\u000d\u0020\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff".trim()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("")),
        "the pinned ECMAScript whitespace set did not trim to empty",
    );
    assert_eq!(
        context
            .eval(
                r#"[
                    " \tvalue \n".trim(),
                    " \tvalue \n".trimEnd(),
                    " \tvalue \n".trimRight(),
                    " \tvalue \n".trimStart(),
                    " \tvalue \n".trimLeft(),
                    "\u180ex\u180e".trim()
                ].join("|")"#,
            )
            .unwrap(),
        Value::String(
            JsString::try_from_utf16([
                0x76, 0x61, 0x6c, 0x75, 0x65, 0x7c, 0x20, 0x09, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x7c,
                0x20, 0x09, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x7c, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x20,
                0x0a, 0x7c, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x20, 0x0a, 0x7c, 0x180e, 0x78, 0x180e,
            ])
            .unwrap()
        ),
        "one-sided trims, aliases, or non-whitespace U+180E drifted",
    );
    assert_eq!(
        context
            .eval(r#""\u3000\ud83d\ude00\ud800\u00a0".trim()"#)
            .unwrap(),
        Value::String(JsString::try_from_utf16([0xd83d, 0xde00, 0xd800]).unwrap()),
        "trim decoded or repaired raw UTF-16 code units",
    );

    for (method, expected) in [
        ("trim", "x|r:string;"),
        ("trimEnd", "  x|r:string;"),
        ("trimRight", "  x|r:string;"),
        ("trimStart", "x  |r:string;"),
        ("trimLeft", "x  |r:string;"),
    ] {
        assert_eq!(
            context
                .eval(&format!(
                    r#"(function(){{
                        var log="",receiver=Object(),extra=Object();
                        receiver[Symbol.toPrimitive]=function(hint){{log+="r:"+hint+";";return "  x  "}};
                        extra[Symbol.toPrimitive]=function(){{log+="extra;";throw "wrong"}};
                        return String.prototype.{method}.call(receiver,extra)+"|"+log;
                    }})()"#,
                ))
                .unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap()),
            "{method} read an ignored argument or converted its receiver incorrectly",
        );
    }

    let unchanged = JsString::try_from_utf16([0xd800, 0x20, 0x61, 0xdc00]).unwrap();
    let Completion::Return(Value::String(identity)) = runtime
        .call_string_prototype_trim(
            context.realm,
            StringTrimKind::Both,
            NativeInvocation::Call {
                this_value: Value::String(unchanged.clone()),
            },
        )
        .unwrap()
    else {
        panic!("identity trim did not return a String");
    };
    assert!(
        identity.same_representation(&unchanged),
        "a full-range flat trim did not reuse the original String",
    );

    let left = JsString::try_from_utf16(
        [0x3000]
            .into_iter()
            .chain(std::iter::repeat_n(u16::from(b'a'), 4_999))
            .chain([0xd83d]),
    )
    .unwrap();
    let right = JsString::try_from_utf16(
        [0xde00]
            .into_iter()
            .chain(std::iter::repeat_n(u16::from(b'b'), 4_999))
            .chain([0xfeff]),
    )
    .unwrap();
    let rope = left.try_concat(&right).unwrap();
    assert!(!rope.is_flat());
    let Completion::Return(Value::String(trimmed)) = runtime
        .call_string_prototype_trim(
            context.realm,
            StringTrimKind::Both,
            NativeInvocation::Call {
                this_value: Value::String(rope),
            },
        )
        .unwrap()
    else {
        panic!("rope trim did not return a String");
    };
    assert!(trimmed.is_flat());
    assert_eq!(trimmed.len(), 10_000);
    assert_eq!(trimmed.code_unit_at(0), Some(u16::from(b'a')));
    assert_eq!(trimmed.code_unit_at(4_998), Some(u16::from(b'a')));
    assert_eq!(trimmed.code_unit_at(4_999), Some(0xd83d));
    assert_eq!(trimmed.code_unit_at(5_000), Some(0xde00));
    assert_eq!(trimmed.code_unit_at(5_001), Some(u16::from(b'b')));
    assert_eq!(trimmed.code_unit_at(9_999), Some(u16::from(b'b')));
}

#[test]
fn string_trim_throws_in_defining_realm_preserves_user_throw_and_recovers_from_oom() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let prototype = defining.string_prototype().unwrap();
    let trim_key = runtime.intern_property_key("trim").unwrap();
    let Value::Object(trim_object) = defining.get_property(&prototype, &trim_key).unwrap() else {
        panic!("String.prototype.trim was not an object");
    };
    let trim = runtime.as_callable(&trim_object).unwrap().unwrap();
    let Value::Object(defining_type_error) = defining.eval("TypeError.prototype").unwrap() else {
        panic!("defining TypeError.prototype was not an object");
    };
    let Value::Object(caller_type_error) = caller.eval("TypeError.prototype").unwrap() else {
        panic!("caller TypeError.prototype was not an object");
    };
    let Value::Object(defining_internal_error) = defining.eval("InternalError.prototype").unwrap()
    else {
        panic!("defining InternalError.prototype was not an object");
    };
    let Value::Object(caller_internal_error) = caller.eval("InternalError.prototype").unwrap()
    else {
        panic!("caller InternalError.prototype was not an object");
    };
    assert_ne!(defining_type_error, caller_type_error);
    assert_ne!(defining_internal_error, caller_internal_error);

    assert_eq!(
        caller.call(&trim, Value::Symbol(runtime.new_symbol(None).unwrap()), &[],),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(type_error)) = caller.take_exception().unwrap() else {
        panic!("cross-realm trim conversion did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&type_error).unwrap(),
        Some(defining_type_error),
        "trim receiver TypeError did not use the function's defining realm",
    );

    caller
        .eval(
            r#"globalThis.trimThrowReceiver=Object();
                trimThrowReceiver[Symbol.toPrimitive]=function(hint){throw 73};
                globalThis.trimReservationLog="";
                globalThis.trimReservationReceiver=Object();
                trimReservationReceiver[Symbol.toPrimitive]=function(hint){
                    trimReservationLog+="receiver:"+hint+";";return "  xy  "
                };"#,
        )
        .unwrap();
    let throwing_receiver = caller.eval("trimThrowReceiver").unwrap();
    assert_eq!(
        caller.call(&trim, throwing_receiver, &[Value::Int(91)]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Int(73)),
        "trim replaced a user receiver-conversion throw",
    );

    crate::engine::value::fail_next_trim_reservation_for_test();
    assert_eq!(
        caller
            .call(
                &trim,
                Value::String(JsString::from_static("identity")),
                &[Value::Int(1)],
            )
            .unwrap(),
        Value::String(JsString::from_static("identity")),
        "the full-range trim fast path failed while the OOM hook was armed",
    );
    assert_eq!(
        caller
            .call(
                &trim,
                Value::String(JsString::from_static("   ")),
                &[Value::Int(2)],
            )
            .unwrap(),
        Value::String(JsString::from_static("")),
        "the empty trim fast path failed while the OOM hook was armed",
    );
    let reservation_receiver = caller.eval("trimReservationReceiver").unwrap();
    assert_eq!(
        caller.call(&trim, reservation_receiver, &[Value::Int(3)]),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(oom)) = caller.take_exception().unwrap() else {
        panic!("trim reservation failure did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&oom).unwrap(),
        Some(defining_internal_error),
        "trim reservation OOM did not use the function's defining realm",
    );
    for (name, expected) in [("name", "InternalError"), ("message", "out of memory")] {
        let Value::String(value) = caller
            .get_property(&oom, &runtime.intern_property_key(name).unwrap())
            .unwrap()
        else {
            panic!("trim reservation OOM {name} was not a String");
        };
        assert_eq!(value, JsString::from_static(expected));
    }
    assert_eq!(
        caller.eval("trimReservationLog").unwrap(),
        Value::String(JsString::from_static("receiver:string;")),
        "trim allocated before its observable receiver conversion",
    );
    assert_eq!(
        caller
            .call(
                &trim,
                Value::String(JsString::from_static("  xy  ")),
                &[Value::Int(4)],
            )
            .unwrap(),
        Value::String(JsString::from_static("xy")),
        "runtime did not recover after trim reservation OOM",
    );
}
