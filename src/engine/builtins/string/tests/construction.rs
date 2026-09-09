use crate::engine::builtins::ErrorKind;
use crate::engine::heap::{AutoInitProperty, PropertySlot};
use crate::engine::object::shape::PropertyFlags;

use super::*;

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
