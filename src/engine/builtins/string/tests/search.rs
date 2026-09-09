use crate::engine::heap::{AutoInitProperty, PropertySlot};
use crate::engine::object::shape::PropertyFlags;

use super::*;

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
