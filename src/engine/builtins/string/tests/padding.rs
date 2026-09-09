use super::*;

#[test]
fn string_pad_preserves_pinned_values_conversion_order_and_early_returns() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"[
                    "abc".padEnd(5),"abc".padStart(5),
                    "abc".padEnd(8,"xy"),"abc".padStart(8,"xy"),
                    "ab".padEnd(),"ab".padStart(undefined),
                    "ab".padEnd(4,undefined),"ab".padStart(4,1)
                ].join("|")"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "abc  |  abc|abcxyxyx|xyxyxabc|ab|ab|ab  |11ab",
        )),
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="",receiver=Object(),target=Object(),filler=Object(),extra=Object();
                    receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+";";return "ab"};
                    target[Symbol.toPrimitive]=function(hint){log+="t:"+hint+";";return 5.9};
                    filler[Symbol.toPrimitive]=function(hint){log+="f:"+hint+";";return "xy"};
                    extra[Symbol.toPrimitive]=function(){log+="extra;";throw "wrong"};
                    return String.prototype.padEnd.call(receiver,target,filler,extra)+"|"+log;
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("abxyx|r:string;t:number;f:string;",)),
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="",receiver=Object(),target=Object(),filler=Object();
                    receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+";";return "ab"};
                    target[Symbol.toPrimitive]=function(hint){log+="t:"+hint+";";return 2.9};
                    filler[Symbol.toPrimitive]=function(){log+="f;";throw "wrong"};
                    return String.prototype.padStart.call(receiver,target,filler)+"|"+log;
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("ab|r:string;t:number;")),
        "len >= target must return before observing the filler",
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="",receiver=Object(),target=Object(),filler=Object();
                    receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+";";return "x"};
                    target[Symbol.toPrimitive]=function(hint){log+="t:"+hint+";";return Infinity};
                    filler[Symbol.toPrimitive]=function(hint){log+="f:"+hint+";";return ""};
                    return String.prototype.padEnd.call(receiver,target,filler)+"|"+log;
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("x|r:string;t:number;f:string;")),
        "empty filler must return after conversion but before the length RangeError",
    );

    assert_eq!(
        context.eval(r#""A\ud800".padEnd(4,"\ude00x")"#).unwrap(),
        Value::String(JsString::try_from_utf16([0x41, 0xd800, 0xde00, 0x78]).unwrap()),
    );
    assert_eq!(
        context.eval(r#""A\ud800".padStart(4,"\ude00x")"#).unwrap(),
        Value::String(JsString::try_from_utf16([0xde00, 0x78, 0x41, 0xd800]).unwrap()),
    );
}

#[test]
fn string_pad_small_limit_preserves_filler_order_and_range_error_kind() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let filler = context
        .eval(
            r#"(function(){
                globalThis.padLimitLog="";
                var filler=Object();
                filler[Symbol.toPrimitive]=function(hint){padLimitLog+="f:"+hint+";";return "x"};
                return filler;
            })()"#,
        )
        .unwrap();
    let completion = runtime
        .call_string_prototype_pad_with_limit(
            context.realm,
            StringPadKind::End,
            NativeInvocation::Call {
                this_value: Value::String(JsString::from_static("a")),
            },
            &NativeArguments {
                actual_arg_count: 2,
                readable: vec![Value::Int(4), filler],
            },
            3,
        )
        .unwrap();
    let Completion::Throw(Value::Object(error)) = completion else {
        panic!("small String pad limit did not throw an Error object");
    };
    for (name, expected) in [("name", "RangeError"), ("message", "invalid string length")] {
        let Value::String(value) = context
            .get_property(&error, &runtime.intern_property_key(name).unwrap())
            .unwrap()
        else {
            panic!("small-limit pad {name} was not a String");
        };
        assert_eq!(value, JsString::from_static(expected));
    }
    assert_eq!(
        context.eval("padLimitLog").unwrap(),
        Value::String(JsString::from_static("f:string;")),
        "pad checked its output bound before converting the filler",
    );

    assert_eq!(
        runtime
            .call_string_prototype_pad_with_limit(
                context.realm,
                StringPadKind::Start,
                NativeInvocation::Call {
                    this_value: Value::String(JsString::from_static("a")),
                },
                &NativeArguments {
                    actual_arg_count: 2,
                    readable: vec![Value::Int(4), Value::String(JsString::from_static(""))],
                },
                3,
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("a"))),
        "empty filler must bypass even an otherwise invalid output length",
    );

    assert_eq!(
        runtime
            .call_string_prototype_pad_with_limit(
                context.realm,
                StringPadKind::End,
                NativeInvocation::Call {
                    this_value: Value::String(JsString::from_static("a")),
                },
                &NativeArguments {
                    actual_arg_count: 1,
                    readable: vec![Value::Int(3)],
                },
                3,
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("a  "))),
        "the length-one native ABI read a nonexistent filler argument",
    );
}

#[test]
fn string_pad_reservation_oom_uses_defining_realm_and_runtime_recovers() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let prototype = defining.string_prototype().unwrap();
    let pad_end_key = runtime.intern_property_key("padEnd").unwrap();
    let Value::Object(pad_end_object) = defining.get_property(&prototype, &pad_end_key).unwrap()
    else {
        panic!("String.prototype.padEnd was not an object");
    };
    let pad_end = runtime.as_callable(&pad_end_object).unwrap().unwrap();
    let Value::Object(defining_internal_error) = defining.eval("InternalError.prototype").unwrap()
    else {
        panic!("defining InternalError.prototype was not an object");
    };
    let Value::Object(caller_internal_error) = caller.eval("InternalError.prototype").unwrap()
    else {
        panic!("caller InternalError.prototype was not an object");
    };
    assert_ne!(defining_internal_error, caller_internal_error);

    caller
        .eval(
            r#"globalThis.padReservationLog="";
                globalThis.padReservationReceiver=Object();
                padReservationReceiver[Symbol.toPrimitive]=function(hint){
                    padReservationLog+="receiver:"+hint+";";return "xy"
                };
                globalThis.padReservationTarget=Object();
                padReservationTarget[Symbol.toPrimitive]=function(hint){
                    padReservationLog+="target:"+hint+";";return 4
                };
                globalThis.padReservationFiller=Object();
                padReservationFiller[Symbol.toPrimitive]=function(hint){
                    padReservationLog+="filler:"+hint+";";return "z"
                };"#,
        )
        .unwrap();
    let receiver = caller.eval("padReservationReceiver").unwrap();
    let target = caller.eval("padReservationTarget").unwrap();
    let filler = caller.eval("padReservationFiller").unwrap();

    crate::engine::value::fail_next_pad_reservation_for_test();
    assert_eq!(
        caller.call(&pad_end, receiver, &[target, filler]),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("pad reservation failure did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_internal_error),
        "pad reservation OOM did not use the native function's defining realm",
    );
    for (name, expected) in [("name", "InternalError"), ("message", "out of memory")] {
        let Value::String(value) = caller
            .get_property(&error, &runtime.intern_property_key(name).unwrap())
            .unwrap()
        else {
            panic!("pad reservation OOM {name} was not a String");
        };
        assert_eq!(value, JsString::from_static(expected));
    }
    assert_eq!(
        caller.eval("padReservationLog").unwrap(),
        Value::String(JsString::from_static(
            "receiver:string;target:number;filler:string;",
        )),
        "pad result reservation happened before its observable conversions",
    );
    assert_eq!(
        caller
            .call(
                &pad_end,
                Value::String(JsString::from_static("xy")),
                &[Value::Int(5), Value::String(JsString::from_static("_"))],
            )
            .unwrap(),
        Value::String(JsString::from_static("xy___")),
        "runtime did not recover after pad reservation OOM",
    );
}
