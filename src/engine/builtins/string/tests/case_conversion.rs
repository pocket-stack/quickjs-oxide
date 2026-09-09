use crate::engine::heap::{AutoInitProperty, PropertySlot};
use crate::engine::object::shape::PropertyFlags;

use super::*;

#[test]
fn string_case_family_is_ordered_autoinit_and_has_distinct_stable_functions() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let keys = STRING_CASE_ENTRIES.map(|(name, selector)| {
        (
            name,
            selector,
            runtime
                .intern_property_key(name)
                .expect("String case key must intern"),
        )
    });
    let value_of = runtime.intern_property_key("valueOf").unwrap();
    let iterator = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(prototype.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_indices = keys
            .each_ref()
            .map(|(_, _, key)| usize::try_from(shape.find(key.atom()).unwrap()).unwrap());
        let value_of_slot = usize::try_from(shape.find(value_of.atom()).unwrap()).unwrap();
        let iterator_slot = usize::try_from(shape.find(iterator.atom()).unwrap()).unwrap();
        assert_eq!(
            slot_indices[0],
            value_of_slot + 1,
            "case conversion methods must physically follow String.prototype.valueOf",
        );
        assert!(
            slot_indices.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "the four case methods did not retain QuickJS table order",
        );
        assert_eq!(
            slot_indices[3] + 1,
            iterator_slot,
            "case conversion methods must physically precede @@iterator",
        );
        for ((name, selector, _), slot_index) in keys.iter().zip(slot_indices) {
            assert_eq!(
                shape.entries()[slot_index].flags,
                PropertyFlags::data(true, false, true),
            );
            assert!(matches!(
                object.slots.get(slot_index),
                Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                    realm,
                    target: NativeFunctionId::StringPrototypeCase(target_selector),
                    name: target_name,
                    length: 0,
                    min_readable_args: 0,
                })) if *realm == context.realm
                    && *target_selector == *selector
                    && *target_name == *name
            ));
        }
    }

    let length_key = runtime.intern_property_key("length").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let mut functions = Vec::with_capacity(keys.len());
    for (name, _, key) in &keys {
        let Value::Object(first) = context.get_property(&prototype, key).unwrap() else {
            panic!("{name} did not materialize as a function object");
        };
        let Value::Object(second) = context.get_property(&prototype, key).unwrap() else {
            panic!("{name} did not remain a function object");
        };
        assert_eq!(first, second, "{name} AutoInit identity was unstable");
        assert!(runtime.as_callable(&first).unwrap().is_some());
        assert!(!runtime.is_constructor(&first).unwrap());
        assert_eq!(
            context.get_property(&first, &length_key).unwrap(),
            Value::Int(0)
        );
        assert_eq!(
            context.get_property(&first, &name_key).unwrap(),
            Value::String(JsString::try_from_utf8(name).unwrap()),
        );
        functions.push(first);
    }
    for left in 0..functions.len() {
        for right in left + 1..functions.len() {
            assert_ne!(
                functions[left], functions[right],
                "{} and {} unexpectedly shared a callable",
                keys[left].0, keys[right].0,
            );
        }
    }
}

#[test]
fn string_case_methods_coerce_only_the_receiver_and_ignore_every_argument() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="",receiver=Object(),ignored=Object();
                    receiver[Symbol.toPrimitive]=function(hint){
                        log+="r:"+hint+",";return "AbΣ"
                    };
                    ignored[Symbol.toPrimitive]=function(hint){
                        log+="a:"+hint+",";throw 91
                    };
                    var values=[
                        String.prototype.toLowerCase.call(receiver,ignored,ignored),
                        String.prototype.toUpperCase.call(receiver,ignored,ignored),
                        String.prototype.toLocaleLowerCase.call(receiver,ignored,ignored),
                        String.prototype.toLocaleUpperCase.call(receiver,ignored,ignored)
                    ];
                    return values.join("|")+";"+log;
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "abς|ABΣ|abς|ABΣ;r:string,r:string,r:string,r:string,",
        )),
        "case conversion touched locale/extra arguments or reordered receiver coercion",
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var receiver=Object();
                    receiver[Symbol.toPrimitive]=function(){throw 72};
                    try{String.prototype.toLocaleLowerCase.call(receiver,null)}
                    catch(error){return error}
                    return "missing";
                })()"#,
            )
            .unwrap(),
        Value::Int(72),
        "case conversion replaced the receiver's user throw",
    );
}

#[test]
fn string_case_expansion_limit_uses_internal_error_and_accepts_exact_boundary() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Completion::Throw(Value::Object(error)) = runtime
        .call_string_prototype_case_with_limit(
            context.realm,
            StringCaseKind::Upper,
            NativeInvocation::Call {
                this_value: Value::String(JsString::try_from_utf8("ß").unwrap()),
            },
            1,
        )
        .unwrap()
    else {
        panic!("one-below-boundary uppercase conversion did not throw an Error object");
    };
    for (name, expected) in [("name", "InternalError"), ("message", "string too long")] {
        let Value::String(value) = context
            .get_property(&error, &runtime.intern_property_key(name).unwrap())
            .unwrap()
        else {
            panic!("small-limit case conversion {name} was not a String");
        };
        assert_eq!(value, JsString::from_static(expected));
    }
    assert_eq!(
        runtime
            .call_string_prototype_case_with_limit(
                context.realm,
                StringCaseKind::Upper,
                NativeInvocation::Call {
                    this_value: Value::String(JsString::try_from_utf8("ß").unwrap()),
                },
                2,
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("SS"))),
        "the exact uppercase expansion boundary was rejected",
    );
}

#[test]
fn string_case_oom_and_type_errors_use_the_defining_realm_and_recover() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let prototype = defining.string_prototype().unwrap();
    let key = runtime.intern_property_key("toLowerCase").unwrap();
    let Value::Object(function_object) = defining.get_property(&prototype, &key).unwrap() else {
        panic!("String.prototype.toLowerCase was not an object");
    };
    let function = runtime.as_callable(&function_object).unwrap().unwrap();
    let Value::Object(defining_internal_error) = defining.eval("InternalError.prototype").unwrap()
    else {
        panic!("defining InternalError.prototype was not an object");
    };
    let Value::Object(defining_type_error) = defining.eval("TypeError.prototype").unwrap() else {
        panic!("defining TypeError.prototype was not an object");
    };

    crate::source::unicode::case::fail_next_case_reservation_for_test();
    assert_eq!(
        caller.call(&function, Value::String(JsString::from_static("A")), &[],),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("case reservation failure did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_internal_error),
        "case reservation OOM did not use the function's defining realm",
    );
    let Value::String(message) = caller
        .get_property(&error, &runtime.intern_property_key("message").unwrap())
        .unwrap()
    else {
        panic!("case reservation OOM message was not a String");
    };
    assert_eq!(message, JsString::from_static("out of memory"));
    assert_eq!(
        caller
            .call(&function, Value::String(JsString::from_static("A")), &[],)
            .unwrap(),
        Value::String(JsString::from_static("a")),
        "runtime did not recover after case reservation OOM",
    );

    assert_eq!(
        caller.call(&function, Value::Null, &[]),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("cross-realm null receiver did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error),
        "case receiver TypeError did not use the function's defining realm",
    );
}
