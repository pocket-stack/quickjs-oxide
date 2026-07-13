use super::*;

#[test]
fn string_index_selectors_use_pinned_generic_magic_cproto() {
    for selector in [StringIndexOfKind::IndexOf, StringIndexOfKind::LastIndexOf] {
        let descriptor = NativeFunctionId::StringPrototypeIndexOf(selector).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::GenericMagic);
        assert!(!descriptor.cproto.default_is_constructor());
    }
    for selector in [
        StringIncludesKind::Includes,
        StringIncludesKind::EndsWith,
        StringIncludesKind::StartsWith,
    ] {
        let descriptor = NativeFunctionId::StringPrototypeIncludes(selector).descriptor();
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
fn string_includes_family_publishes_typed_autoinit_entries_and_identities() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let entries = [
        ("includes", StringIncludesKind::Includes),
        ("endsWith", StringIncludesKind::EndsWith),
        ("startsWith", StringIncludesKind::StartsWith),
    ];
    let keys = entries.map(|(name, selector)| {
        (
            name,
            selector,
            runtime
                .intern_property_key(name)
                .expect("String includes-family key must intern"),
        )
    });
    let state = runtime.0.state.borrow();
    let object = state.heap.object(prototype.object_id()).unwrap();
    let shape = state.heap.shape(object.shape).unwrap();
    for (name, selector, key) in &keys {
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot_index].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::StringPrototypeIncludes(target_selector),
                name: target_name,
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm
                && *target_selector == *selector
                && *target_name == *name
        ));
    }
    drop(state);

    let identities = keys.map(|(name, _, key)| {
        let Value::Object(first) = context.get_property(&prototype, &key).unwrap() else {
            panic!("{name} did not materialize as a function object");
        };
        let Value::Object(second) = context.get_property(&prototype, &key).unwrap() else {
            panic!("{name} did not remain a function object");
        };
        assert_eq!(first, second, "{name} AutoInit identity was unstable");
        assert!(runtime.as_callable(&first).unwrap().is_some());
        assert!(!runtime.is_constructor(&first).unwrap());
        first
    });
    assert_ne!(identities[0], identities[1]);
    assert_ne!(identities[0], identities[2]);
    assert_ne!(identities[1], identities[2]);
}

#[test]
fn string_includes_preserves_pinned_values_utf16_and_shared_magic_kernel() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for (source, expected) in [
        (r#""abc".includes("b")"#, true),
        (r#""abc".includes("z")"#, false),
        (r#""abc".includes("",Infinity)"#, true),
        (r#""abc".includes("c",Infinity)"#, false),
        (r#""abc".includes("b",1.9)"#, true),
        (r#""abc".includes("a",-Infinity)"#, true),
        (r#""undefined".includes()"#, true),
        (r#""A\ud83d\ude00\ud800Z".includes("\ude00\ud800")"#, true),
    ] {
        assert_eq!(
            context.eval(source).unwrap(),
            Value::Bool(expected),
            "{source}"
        );
    }
    assert_eq!(
        context
            .eval("typeof String.prototype.endsWith+'|'+typeof String.prototype.startsWith")
            .unwrap(),
        Value::String(JsString::from_static("function|function")),
    );

    for (selector, search, position, expected) in [
        (StringIncludesKind::StartsWith, "ab", None, true),
        (StringIncludesKind::StartsWith, "bc", Some(1), true),
        (StringIncludesKind::EndsWith, "bc", None, true),
        (StringIncludesKind::EndsWith, "ab", Some(2), true),
    ] {
        let mut readable = vec![Value::String(JsString::from_static(search))];
        if let Some(position) = position {
            readable.push(Value::Int(position));
        }
        assert_eq!(
            runtime
                .call_string_prototype_includes(
                    context.realm,
                    selector,
                    NativeInvocation::Call {
                        this_value: Value::String(JsString::from_static("abc")),
                    },
                    &NativeArguments {
                        actual_arg_count: readable.len(),
                        readable,
                    },
                )
                .unwrap(),
            Completion::Return(Value::Bool(expected)),
        );
    }
}

#[test]
fn string_includes_preserves_is_regexp_and_conversion_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let order = context
        .eval(
            r#"(function(){
                var log="",receiver=Object(),search=Object(),position=Object(),descriptor=Object();
                receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+",";return "ababa"};
                descriptor.get=function(){log+="match,";return false};
                Object.defineProperty(search,Symbol.match,descriptor);
                search[Symbol.toPrimitive]=function(hint){log+="s:"+hint+",";return "ba"};
                position[Symbol.toPrimitive]=function(hint){log+="p:"+hint+",";return 2};
                return "".includes.call(receiver,search,position)+"|"+log;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        order,
        Value::String(JsString::from_static(
            "true|r:string,match,s:string,p:number,",
        )),
    );

    let short_circuits = context
        .eval(
            r#"(function(){
                var log="",marker=Object(),search=Object(),position=Object(),descriptor=Object();
                marker[Symbol.toPrimitive]=function(){log+="marker,";throw "wrong"};
                descriptor.get=function(){log+="match,";return marker};
                Object.defineProperty(search,Symbol.match,descriptor);
                search.toString=function(){log+="search,";return "b"};
                position.valueOf=function(){log+="position,";return 0};
                var first;
                try{"abc".includes(search,position)}catch(error){first=error.name+":"+error.message+":"+log}
                log="";
                position.valueOf=function(){log+="position,";throw 91};
                var second;
                try{"a".includes("long",position)}catch(error){second=error+":"+log}
                return first+"|"+second;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        short_circuits,
        Value::String(JsString::from_static(
            "TypeError:regexp not supported:match,|91:position,",
        )),
    );

    let primitive_search = context
        .eval(
            r#"(function(){
                var log="",descriptor=Object();
                descriptor.configurable=true;
                descriptor.get=function(){log+="match,";throw "wrong"};
                Object.defineProperty(String.prototype,Symbol.match,descriptor);
                var result="abc".includes("b");
                delete String.prototype[Symbol.match];
                return result+"|"+log;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        primitive_search,
        Value::String(JsString::from_static("true|")),
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

#[test]
fn recursive_string_includes_family_match_getter_is_guarded_and_runtime_recovers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"function stringIncludesFamilyRecurse(kind,depth){
                var search=Object(),descriptor=Object();
                descriptor.get=function(){
                    if(depth!==0)stringIncludesFamilyRecurse(kind,depth-1);
                    return false;
                };
                Object.defineProperty(search,Symbol.match,descriptor);
                search.toString=function(){return "x"};
                if(kind===0)return "x".includes(search);
                if(kind===1)return "x".endsWith(search);
                return "x".startsWith(search);
            }"#,
        )
        .unwrap();

    for (method, kind) in [("includes", 0), ("endsWith", 1), ("startsWith", 2)] {
        assert_eq!(
            context
                .eval(&format!("stringIncludesFamilyRecurse({kind},3)"))
                .unwrap(),
            Value::Bool(true),
            "the proven-safe four-frame {method} chain was rejected",
        );
        assert_eq!(
            context
                .eval(&format!(
                    r#"(function(){{
                        try{{stringIncludesFamilyRecurse({kind},4);return "missing"}}
                        catch(error){{return error.name+":"+error.message}}
                    }})()"#,
                ))
                .unwrap(),
            Value::String(JsString::from_static("InternalError:stack overflow")),
        );
    }
    assert_eq!(context.eval("1+1").unwrap(), Value::Int(2));
}
