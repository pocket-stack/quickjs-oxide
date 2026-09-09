use super::*;

#[test]
fn string_repeat_preserves_pinned_values_order_and_errors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"[
                    "ab".repeat(),"ab".repeat(undefined),"ab".repeat(null),
                    "ab".repeat(false),"ab".repeat(true),"ab".repeat(2.9),
                    "ab".repeat(NaN),"ab".repeat(-0),"".repeat(2147483647),
                    "A\ud83d\ude00\ud800".repeat(2)
                ].join("|")"#,
            )
            .unwrap(),
        Value::String(
            JsString::try_from_utf16([
                0x7c, 0x7c, 0x7c, 0x7c, 0x61, 0x62, 0x7c, 0x61, 0x62, 0x61, 0x62, 0x7c, 0x7c, 0x7c,
                0x7c, 0x41, 0xd83d, 0xde00, 0xd800, 0x41, 0xd83d, 0xde00, 0xd800,
            ])
            .unwrap()
        ),
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="",receiver=Object(),count=Object(),extra=Object();
                    receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+";";return "ab"};
                    count[Symbol.toPrimitive]=function(hint){log+="c:"+hint+";";return 2.9};
                    extra[Symbol.toPrimitive]=function(){log+="extra;";throw "wrong"};
                    return String.prototype.repeat.call(receiver,count,extra)+"|"+log;
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("abab|r:string;c:number;")),
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var values=[-1,-Infinity,Infinity,2147483648],output=[],index=0;
                    while(index<values.length){
                        try{"a".repeat(values[index]);output.push("return")}
                        catch(error){output.push(error.name+":"+error.message)}
                        index++;
                    }
                    try{"ab".repeat(536870912)}
                    catch(error){output.push(error.name+":"+error.message)}
                    try{new String.prototype.repeat()}
                    catch(error){output.push(error.name+":"+error.message)}
                    return output.join("|");
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "RangeError:invalid repeat count|RangeError:invalid repeat count|\
             RangeError:invalid repeat count|RangeError:invalid repeat count|\
             RangeError:invalid string length|TypeError:repeat is not a constructor",
        )),
    );
}

#[test]
fn string_repeat_reservation_oom_is_catchable_in_defining_realm_and_recovers() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let prototype = defining.string_prototype().unwrap();
    let repeat_key = runtime.intern_property_key("repeat").unwrap();
    let Value::Object(repeat_object) = defining.get_property(&prototype, &repeat_key).unwrap()
    else {
        panic!("String.prototype.repeat was not an object");
    };
    let repeat = runtime.as_callable(&repeat_object).unwrap().unwrap();
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
            r#"globalThis.repeatReservationLog="";
                globalThis.repeatReservationReceiver=Object();
                repeatReservationReceiver[Symbol.toPrimitive]=function(hint){
                    repeatReservationLog+="receiver:"+hint+";";return "xy"
                };
                globalThis.repeatReservationCount=Object();
                repeatReservationCount[Symbol.toPrimitive]=function(hint){
                    repeatReservationLog+="count:"+hint+";";return 2
                };"#,
        )
        .unwrap();
    let receiver = caller.eval("repeatReservationReceiver").unwrap();
    let count = caller.eval("repeatReservationCount").unwrap();

    crate::engine::value::fail_next_repeat_reservation_for_test();
    assert_eq!(
        caller.call(&repeat, receiver, &[count]),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("repeat reservation failure did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_internal_error),
        "repeat reservation OOM did not use the native function's defining realm",
    );
    for (name, expected) in [("name", "InternalError"), ("message", "out of memory")] {
        let Value::String(value) = caller
            .get_property(&error, &runtime.intern_property_key(name).unwrap())
            .unwrap()
        else {
            panic!("repeat reservation OOM {name} was not a String");
        };
        assert_eq!(value, JsString::from_static(expected));
    }
    assert_eq!(
        caller.eval("repeatReservationLog").unwrap(),
        Value::String(JsString::from_static("receiver:string;count:number;")),
        "repeat buffer reservation happened before its observable conversions",
    );
    assert_eq!(
        caller
            .call(
                &repeat,
                Value::String(JsString::from_static("xy")),
                &[Value::Int(2)],
            )
            .unwrap(),
        Value::String(JsString::from_static("xyxy")),
        "runtime did not recover after repeat reservation OOM",
    );
}
