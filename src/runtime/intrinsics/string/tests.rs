use super::*;

#[test]
fn string_index_selectors_use_pinned_generic_magic_cproto() {
    for selector in [StringIndexOfKind::IndexOf, StringIndexOfKind::LastIndexOf] {
        let descriptor = NativeFunctionId::StringPrototypeIndexOf(selector).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::GenericMagic);
        assert!(!descriptor.cproto.default_is_constructor());
    }
}

#[test]
fn string_index_scan_is_utf16_exact_and_inclusive() {
    let source = JsString::try_from_utf16([0x61, 0xd83d, 0xde00, 0xd800, 0x61]).unwrap();
    let crossed = JsString::try_from_utf16([0xde00, 0xd800]).unwrap();
    let empty = JsString::from_static("");
    let too_long = JsString::from_static("abcdef");

    assert_eq!(scan_string_region(&source, &crossed, 0, 3, 1), 2);
    assert_eq!(scan_string_region(&source, &crossed, 3, 0, -1), 2);
    assert_eq!(scan_string_region(&source, &empty, 4, 4, 1), 4);
    assert_eq!(scan_string_region(&source, &too_long, 0, 0, 1), -1);
}

#[test]
fn string_index_methods_preserve_pinned_positions_and_conversion_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (source, expected) in [
        (r#""aaa".indexOf("a",NaN)"#, 0),
        (r#""aaa".indexOf("a",-Infinity)"#, 0),
        (r#""aaa".indexOf("a",Infinity)"#, -1),
        (r#""aaa".indexOf("",4)"#, 3),
        (r#""aaa".lastIndexOf("a",NaN)"#, 2),
        (r#""aaa".lastIndexOf("a",-Infinity)"#, 0),
        (r#""aaa".lastIndexOf("a",Infinity)"#, 2),
        (r#""aaa".lastIndexOf("",4)"#, 3),
        (r#""abc".indexOf()"#, -1),
        (r#""abc".lastIndexOf()"#, -1),
    ] {
        assert_eq!(
            context.eval(source).unwrap(),
            Value::Int(expected),
            "{source}"
        );
    }

    let order = context
        .eval(
            r#"(function(){
                var log="";
                var receiver=Object();
                receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+",";return "ababa"};
                var search=Object();
                var descriptor=Object();
                descriptor.get=function(){log+="match,";throw "wrong"};
                Object.defineProperty(search,Symbol.match,descriptor);
                search[Symbol.toPrimitive]=function(hint){log+="s:"+hint+",";return "ba"};
                var position=Object();
                position[Symbol.toPrimitive]=function(hint){log+="p:"+hint+",";return 2};
                var forward="".indexOf.call(receiver,search,position);
                var reverse="".lastIndexOf.call(receiver,search,position);
                return forward+"|"+reverse+"|"+log;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        order,
        Value::String(JsString::from_static(
            "3|1|r:string,s:string,p:number,r:string,s:string,p:number,",
        )),
    );
}

#[test]
fn string_constructor_statics_remain_typed_autoinit_entries() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let string_key = runtime.intern_property_key("String").unwrap();
    let Value::Object(string_constructor) = context.get_property(&global, &string_key).unwrap()
    else {
        panic!("global String was not an object");
    };

    for (name, selector) in [
        ("fromCharCode", StringStaticKind::FromCharCode),
        ("fromCodePoint", StringStaticKind::FromCodePoint),
        ("raw", StringStaticKind::Raw),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        let state = runtime.0.state.borrow();
        let object = state.heap.object(string_constructor.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot_index].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::StringStatic(target_selector),
                name: target_name,
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm
                && *target_selector == selector
                && *target_name == name
        ));
    }
}

#[test]
fn string_raw_latched_overflow_preserves_pinned_observable_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(cooked) = context
        .eval(
            r#"(function(){
                globalThis.stringRawOverflowLog="";
                var cooked=Object(),raw=Object();raw.length=2;raw[0]="aa";
                raw.__defineGetter__("1",function(){stringRawOverflowLog+="g1";throw 77});
                cooked.raw=raw;return cooked;
            })()"#,
        )
        .unwrap()
    else {
        panic!("String.raw overflow fixture was not an object");
    };
    let completion = runtime
        .call_string_raw_with_limit(
            context.realm,
            &NativeArguments {
                actual_arg_count: 1,
                readable: vec![Value::Object(cooked)],
            },
            1,
        )
        .unwrap();
    assert!(matches!(completion, Completion::Throw(Value::Int(77))));
    assert_eq!(
        context.eval("stringRawOverflowLog").unwrap(),
        Value::String(JsString::from_static("g1")),
    );

    let Value::Object(cooked) = context
        .eval(
            r#"(function(){
                stringRawOverflowLog="";
                var cooked=Object(),raw=Object();raw.length=2;raw[0]="aa";raw[1]="b";
                cooked.raw=raw;return cooked;
            })()"#,
        )
        .unwrap()
    else {
        panic!("String.raw substitution-overflow fixture was not an object");
    };
    let substitution = context
        .eval(
            r#"(function(){var value=Object();value.toString=function(){stringRawOverflowLog+="s";return "x"};return value})()"#,
        )
        .unwrap();
    let error = runtime
        .call_string_raw_with_limit(
            context.realm,
            &NativeArguments {
                actual_arg_count: 2,
                readable: vec![Value::Object(cooked), substitution],
            },
            1,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::Engine(ref error)
            if error.kind() == ErrorKind::JsInternal && error.message() == "string too long"
    ));
    assert_eq!(
        context.eval("stringRawOverflowLog").unwrap(),
        Value::String(JsString::from_static("")),
        "a checked substitution was converted after the raw append had failed",
    );
}

#[test]
fn recursive_string_constructor_family_is_guarded_and_runtime_recovers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"function stringConstructorRecurse(depth){
                    var value=Object();
                    value[Symbol.toPrimitive]=function(){
                        if(depth!==0)stringConstructorRecurse(depth-1);
                        return "x";
                    };
                    return String(value);
                }
                function stringFromCharCodeRecurse(depth){
                    var value=Object();
                    value[Symbol.toPrimitive]=function(){
                        if(depth!==0)stringFromCharCodeRecurse(depth-1);
                        return 65;
                    };
                    return String.fromCharCode(value);
                }
                function stringFromCodePointRecurse(depth){
                    var value=Object();
                    value[Symbol.toPrimitive]=function(){
                        if(depth!==0)stringFromCodePointRecurse(depth-1);
                        return 65;
                    };
                    return String.fromCodePoint(value);
                }
                function stringRawRecurse(depth){
                    var cooked=Object(),raw=Object();raw.length=1;raw[0]="x";
                    cooked.__defineGetter__("raw",function(){
                        if(depth!==0)stringRawRecurse(depth-1);
                        return raw;
                    });
                    return String.raw(cooked);
                }"#,
        )
        .unwrap();

    for (call, expected) in [
        ("stringConstructorRecurse(8)", "x"),
        ("stringFromCharCodeRecurse(8)", "A"),
        ("stringFromCodePointRecurse(8)", "A"),
        ("stringRawRecurse(8)", "x"),
    ] {
        assert_eq!(
            context.eval(call).unwrap(),
            Value::String(JsString::from_static(expected)),
            "safe String-family recursion drifted for {call}",
        );
    }
    for call in [
        "stringConstructorRecurse(9)",
        "stringFromCharCodeRecurse(9)",
        "stringFromCodePointRecurse(9)",
        "stringRawRecurse(9)",
    ] {
        let value = context
            .eval(&format!(
                r#"(function(){{
                    try{{{call};return "missing"}}
                    catch(error){{return error.name+":"+error.message}}
                }})()"#,
            ))
            .unwrap();
        assert_eq!(
            value,
            Value::String(JsString::from_static("InternalError:stack overflow")),
            "String-family recursion guard drifted for {call}",
        );
    }
    assert_eq!(context.eval("1+1").unwrap(), Value::Int(2));
}
