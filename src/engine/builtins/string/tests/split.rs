use crate::engine::builtins::native::NativeCProto;
use crate::engine::heap::{AutoInitProperty, PropertySlot};
use crate::engine::object::shape::PropertyFlags;

use super::*;

#[test]
fn string_split_is_a_pinned_generic_autoinit_between_search_and_substring() {
    let descriptor = NativeFunctionId::StringPrototypeSplit.descriptor();
    assert_eq!(descriptor.cproto, NativeCProto::Generic);
    assert!(!descriptor.cproto.default_is_constructor());

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let search_key = runtime.intern_property_key("search").unwrap();
    let split_key = runtime.intern_property_key("split").unwrap();
    let substring = runtime.intern_property_key("substring").unwrap();
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(prototype.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let search = usize::try_from(shape.find(search_key.atom()).unwrap()).unwrap();
        let split = usize::try_from(shape.find(split_key.atom()).unwrap()).unwrap();
        let substring = usize::try_from(shape.find(substring.atom()).unwrap()).unwrap();
        assert_eq!(split, search + 1);
        assert_eq!(substring, split + 1);
        assert_eq!(
            shape.entries()[split].flags,
            PropertyFlags::data(true, false, true)
        );
        assert!(matches!(
            object.slots.get(split),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::StringPrototypeSplit,
                name: "split",
                length: 2,
                min_readable_args: 2,
            })) if *realm == context.realm
        ));
    }

    let Value::Object(first) = context.get_property(&prototype, &split_key).unwrap() else {
        panic!("String.prototype.split did not materialize as a function");
    };
    let Value::Object(second) = context.get_property(&prototype, &split_key).unwrap() else {
        panic!("String.prototype.split did not retain its function identity");
    };
    assert_eq!(first, second);
    assert!(runtime.as_callable(&first).unwrap().is_some());
    assert!(!runtime.is_constructor(&first).unwrap());
    assert_eq!(
        context
            .eval("String.prototype.split.name+'|'+String.prototype.split.length")
            .unwrap(),
        Value::String(JsString::from_static("split|2")),
    );
}

#[test]
fn string_split_preserves_limits_boundaries_and_utf16_code_units() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"[
                    "a--b--".split("--").join("|"),
                    "abc".split().join("|"),
                    "abc".split(undefined,0).length,
                    "".split("").length,
                    "".split("x").length,
                    "abc".split("",2).join("|"),
                    "aaaa".split("aa").join("|"),
                    "abc".split("z").join("|"),
                    "abc".split("",4294967296).length,
                    "abc".split("",-1).length,
                    "abc".split("",2.9).join("|")
                ].join(";")"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("a|b|;abc;0;0;1;a|b;||;abc;0;3;a|b",)),
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var pieces=String.fromCharCode(0x41,0xd83d,0xde00,0xd800).split("");
                    return pieces.length+"|"+pieces[0].charCodeAt(0)+"|"+
                        pieces[1].charCodeAt(0)+"|"+pieces[2].charCodeAt(0)+"|"+
                        pieces[3].charCodeAt(0)
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("4|65|55357|56832|55296")),
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var result="a,b".split(","),descriptor=Object.getOwnPropertyDescriptor(result,"0");
                    return descriptor.value+"|"+descriptor.writable+"|"+
                        descriptor.enumerable+"|"+descriptor.configurable+"|"+result.length
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("a|true|true|true|2")),
    );
}

#[test]
fn string_split_preserves_delegation_conversion_order_and_abrupt_completion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="",receiver=Object(),separator=Object(),limit=Object(),descriptor=Object();
                    receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "a,b"};
                    descriptor.get=function(){log+="get;";return null};
                    Object.defineProperty(separator,Symbol.split,descriptor);
                    separator[Symbol.toPrimitive]=function(hint){log+="separator:"+hint+";";return ","};
                    limit[Symbol.toPrimitive]=function(hint){log+="limit:"+hint+";";return 2};
                    return String.prototype.split.call(receiver,separator,limit).join("|")+"|"+log
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "a|b|get;receiver:string;limit:number;separator:string;",
        )),
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="",receiver=Object(),separator=Object(),limit=Object(),descriptor=Object();
                    receiver[Symbol.toPrimitive]=function(){log+="wrong-receiver;";throw 1};
                    limit[Symbol.toPrimitive]=function(){log+="wrong-limit;";throw 2};
                    descriptor.get=function(){
                        log+="get;";
                        return function(originalReceiver,originalLimit){
                            log+="call;";
                            return (this===separator)+"|"+(originalReceiver===receiver)+"|"+
                                (originalLimit===limit)+"|"+log
                        }
                    };
                    Object.defineProperty(separator,Symbol.split,descriptor);
                    return String.prototype.split.call(receiver,separator,limit)
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("true|true|true|get;call;")),
    );

    for (source, expected) in [
        (
            r#"(function(){
                var log="",separator=Object(),descriptor=Object();
                descriptor.get=function(){log+="get;";return function(){throw 91}};
                Object.defineProperty(separator,Symbol.split,descriptor);
                try{String.prototype.split.call(null,separator)}
                catch(error){return error.name+"|"+log}
            })()"#,
            "TypeError|",
        ),
        (
            r#"(function(){
                var separator=Object();separator[Symbol.split]=1;
                try{"x".split(separator)}catch(error){return error.name+":"+error.message}
            })()"#,
            "TypeError:not a function",
        ),
        (
            r#"(function(){
                var log="",receiver=Object(),separator=Object(),limit=Object(),descriptor=Object();
                descriptor.get=function(){log+="get;";return undefined};
                Object.defineProperty(separator,Symbol.split,descriptor);
                receiver[Symbol.toPrimitive]=function(){log+="receiver;";return "a,b"};
                limit[Symbol.toPrimitive]=function(){log+="limit;";throw 72};
                separator[Symbol.toPrimitive]=function(){log+="separator;";throw 73};
                try{String.prototype.split.call(receiver,separator,limit)}
                catch(error){return error+"|"+log}
            })()"#,
            "72|get;receiver;limit;",
        ),
    ] {
        assert_eq!(
            context.eval(source).unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap()),
            "{source}",
        );
    }
}

#[test]
fn string_split_result_and_native_errors_use_the_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let prototype = defining.string_prototype().unwrap();
    let split_key = runtime.intern_property_key("split").unwrap();
    let Value::Object(split_object) = defining.get_property(&prototype, &split_key).unwrap() else {
        panic!("String.prototype.split was not an object");
    };
    let split = runtime.as_callable(&split_object).unwrap().unwrap();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let Value::Object(defining_type_error) = defining.eval("TypeError.prototype").unwrap() else {
        panic!("defining TypeError.prototype was not an object");
    };

    let mut caller = runtime.new_context();
    let result = caller
        .call(
            &split,
            Value::String(JsString::from_static("a,b")),
            &[Value::String(JsString::from_static(",")), Value::Undefined],
        )
        .unwrap();
    let Value::Object(result) = result else {
        panic!("cross-realm String split result was not an Array object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype),
    );

    assert_eq!(
        caller.call(&split, Value::Null, &[]),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("cross-realm String split null receiver did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error),
    );
}

#[test]
fn string_split_output_index_overflow_is_checked_before_array_mutation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = runtime.new_array(context.realm).unwrap();
    let mut length = u32::MAX;

    assert!(matches!(
        runtime.define_string_split_element(
            context.realm,
            &result,
            &mut length,
            Value::String(JsString::from_static("unreachable")),
        ),
        Err(RuntimeError::Invariant(
            "String split output index exceeded Uint32"
        )),
    ));
    assert_eq!(length, u32::MAX);
    let length_key = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        context.get_property(&result, &length_key).unwrap(),
        Value::Int(0),
    );
}
