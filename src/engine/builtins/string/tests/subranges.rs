use super::*;

#[test]
fn string_subrange_preserves_pinned_clamps_utf16_and_rope_copying() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (source, expected) in [
        (r#""abcdef".substring(4,1)"#, "bcd"),
        (r#""abcdef".substring(-Infinity,Infinity)"#, "abcdef"),
        (r#""abcdef".substring(NaN,2.9)"#, "ab"),
        (r#""abcdef".substring(2147483648,1)"#, "bcdef"),
        (r#""abcdef".substr(-2,1)"#, "e"),
        (r#""abcdef".substr(-99,2)"#, "ab"),
        (r#""abcdef".substr(2,-1)"#, ""),
        (r#""abcdef".substr(2,Infinity)"#, "cdef"),
        (r#""abcdef".substr()"#, "abcdef"),
        (r#""abcdef".slice(-3,-1)"#, "de"),
        (r#""abcdef".slice(4,1)"#, ""),
        (r#""abcdef".slice(-Infinity,Infinity)"#, "abcdef"),
        (r#""abcdef".slice()"#, "abcdef"),
        (r#""abcdef".slice(NaN,2.9)"#, "ab"),
    ] {
        assert_eq!(
            context.eval(source).unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap()),
            "{source}"
        );
    }
    assert_eq!(
        context
            .eval(r#""A\ud83d\ude00\ud800Z".substring(1,2)"#)
            .unwrap(),
        Value::String(JsString::try_from_utf16([0xd83d]).unwrap())
    );
    assert_eq!(
        context
            .eval(r#""A\ud83d\ude00\ud800Z".slice(2,4)"#)
            .unwrap(),
        Value::String(JsString::try_from_utf16([0xde00, 0xd800]).unwrap())
    );

    let left =
        JsString::try_from_utf16(std::iter::repeat_n(u16::from(b'a'), 4_999).chain([0xd83d]))
            .unwrap();
    let right = JsString::try_from_utf16(
        [0xde00]
            .into_iter()
            .chain(std::iter::repeat_n(u16::from(b'b'), 5_000)),
    )
    .unwrap();
    let rope = left.try_concat(&right).unwrap();
    assert!(!rope.is_flat());
    let completion = runtime
        .call_string_prototype_subrange(
            context.realm,
            StringSubrangeKind::Slice,
            NativeInvocation::Call {
                this_value: Value::String(rope),
            },
            &NativeArguments {
                actual_arg_count: 2,
                readable: vec![Value::Int(4_999), Value::Int(5_002)],
            },
        )
        .unwrap();
    assert_eq!(
        completion,
        Completion::Return(Value::String(
            JsString::try_from_utf16([0xd83d, 0xde00, u16::from(b'b')]).unwrap()
        ))
    );
}

#[test]
fn string_subrange_preserves_conversion_order_throws_and_defining_realm() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();

    for (method, expected) in [("substring", "bcd"), ("substr", "e"), ("slice", "")] {
        let value = first
            .eval(&format!(
                r#"(function(){{
                    var log="",receiver=Object(),start=Object(),end=Object();
                    receiver[Symbol.toPrimitive]=function(hint){{log+="r:"+hint+",";return "abcdef"}};
                    start[Symbol.toPrimitive]=function(hint){{log+="s:"+hint+",";return 4}};
                    end[Symbol.toPrimitive]=function(hint){{log+="e:"+hint+",";return 1}};
                    return "".{method}.call(receiver,start,end)+"|"+log;
                }})()"#,
            ))
            .unwrap();
        assert_eq!(
            value,
            Value::String(
                JsString::try_from_utf8(&format!("{expected}|r:string,s:number,e:number,"))
                    .unwrap()
            ),
            "{method} conversion order drifted"
        );
    }

    let short_circuit = first
        .eval(
            r#"(function(){
                var log="",receiver=Object(),start=Object(),end=Object();
                receiver[Symbol.toPrimitive]=function(){log+="r,";return "abc"};
                start[Symbol.toPrimitive]=function(){log+="s,";throw 71};
                end[Symbol.toPrimitive]=function(){log+="e,";throw 72};
                try{"".slice.call(receiver,start,end)}catch(error){return error+"|"+log}
            })()"#,
        )
        .unwrap();
    assert_eq!(
        short_circuit,
        Value::String(JsString::from_static("71|r,s,"))
    );

    let first_string = first.string_prototype().unwrap();
    let slice_key = runtime.intern_property_key("slice").unwrap();
    let Value::Object(slice_object) = first.get_property(&first_string, &slice_key).unwrap() else {
        panic!("String.prototype.slice was not an object");
    };
    let slice = runtime.as_callable(&slice_object).unwrap().unwrap();
    let Value::Object(first_type_error_prototype) = first.eval("TypeError.prototype").unwrap()
    else {
        panic!("first realm TypeError.prototype was not an object");
    };
    let mut second = runtime.new_context();
    assert_eq!(
        second.call(
            &slice,
            Value::String(JsString::from_static("abc")),
            &[Value::Symbol(runtime.new_symbol(None).unwrap())],
        ),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(error)) = second.take_exception().unwrap() else {
        panic!("cross-realm slice conversion did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(first_type_error_prototype)
    );
}
