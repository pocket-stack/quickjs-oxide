use crate::engine::heap::{AutoInitProperty, PropertySlot};
use crate::engine::object::shape::PropertyFlags;

use super::*;

#[test]
fn string_create_html_family_is_ordered_autoinit_and_has_distinct_stable_functions() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let keys = STRING_CREATE_HTML_ENTRIES.map(|(name, selector, length)| {
        (
            name,
            selector,
            length,
            runtime
                .intern_property_key(name)
                .expect("String CreateHTML key must intern"),
        )
    });
    let iterator = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
    let constructor = runtime.intern_property_key("constructor").unwrap();
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(prototype.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_indices = keys
            .each_ref()
            .map(|(_, _, _, key)| usize::try_from(shape.find(key.atom()).unwrap()).unwrap());
        let iterator_slot = usize::try_from(shape.find(iterator.atom()).unwrap()).unwrap();
        let constructor_slot = usize::try_from(shape.find(constructor.atom()).unwrap()).unwrap();
        assert_eq!(
            slot_indices[0],
            iterator_slot + 1,
            "CreateHTML must physically follow String.prototype @@iterator",
        );
        assert!(
            slot_indices.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "the thirteen CreateHTML entries did not retain QuickJS table order",
        );
        assert!(
            slot_indices[12] < constructor_slot,
            "CreateHTML was published after the constructor back-reference",
        );
        for (((name, selector, length, _), slot_index), expected_slot) in keys
            .iter()
            .zip(slot_indices)
            .zip(slot_indices[0]..=slot_indices[12])
        {
            assert_eq!(slot_index, expected_slot);
            assert_eq!(
                shape.entries()[slot_index].flags,
                PropertyFlags::data(true, false, true),
            );
            assert!(matches!(
                object.slots.get(slot_index),
                Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                    realm,
                    target: NativeFunctionId::StringPrototypeCreateHtml(target_selector),
                    name: target_name,
                    length: target_length,
                    min_readable_args,
                })) if *realm == context.realm
                    && *target_selector == *selector
                    && *target_name == *name
                    && *target_length == *length
                    && *min_readable_args == *length
            ));
        }
    }

    let length_key = runtime.intern_property_key("length").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let mut functions = Vec::with_capacity(keys.len());
    for (name, _, length, key) in &keys {
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
            Value::Int(i32::from(*length)),
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
fn string_create_html_maps_tags_and_preserves_pinned_conversion_and_argument_rules() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"[
                    "x".anchor("v"),"x".big(),"x".blink(),"x".bold(),
                    "x".fixed(),"x".fontcolor("v"),"x".fontsize("v"),
                    "x".italics(),"x".link("v"),"x".small(),"x".strike(),
                    "x".sub(),"x".sup()
                ].join("|")"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "<a name=\"v\">x</a>|<big>x</big>|<blink>x</blink>|<b>x</b>|\
             <tt>x</tt>|<font color=\"v\">x</font>|<font size=\"v\">x</font>|\
             <i>x</i>|<a href=\"v\">x</a>|<small>x</small>|<strike>x</strike>|\
             <sub>x</sub>|<sup>x</sup>",
        )),
        "CreateHTML selector-to-tag mapping drifted",
    );

    let nullish = context
        .eval(
            r#"(function(){
                var names=["anchor","fontcolor","fontsize","link"],output=[],index=0;
                while(index<names.length){
                    var name=names[index],fn=String.prototype[name],mode=0;
                    while(mode<3){
                        try{
                            if(mode===0)fn.call("x");
                            else if(mode===1)fn.call("x",undefined);
                            else fn.call("x",null);
                            output.push(name+":"+mode+":missing");
                        }catch(error){
                            output.push(name+":"+mode+":"+error.name+":"+error.message);
                        }
                        mode++;
                    }
                    index++;
                }
                return output.join("|");
            })()"#,
        )
        .unwrap();
    let expected_nullish = ["anchor", "fontcolor", "fontsize", "link"]
        .into_iter()
        .flat_map(|name| {
            (0..3)
                .map(move |mode| format!("{name}:{mode}:TypeError:null or undefined are forbidden"))
        })
        .collect::<Vec<_>>()
        .join("|");
    assert_eq!(
        nullish,
        Value::String(JsString::try_from_utf8(&expected_nullish).unwrap()),
        "CreateHTML did not apply JS_ToStringCheckObject to its attribute",
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="",receiver=Object(),attribute=Object(),extra=Object();
                    receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+",";return null};
                    attribute[Symbol.toPrimitive]=function(hint){log+="a:"+hint+",";return undefined};
                    extra[Symbol.toPrimitive]=function(hint){log+="x:"+hint+",";throw 99};
                    var attributed=String.prototype.anchor.call(receiver,attribute,extra);
                    log+="|";
                    receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+",";return "body"};
                    var plain=String.prototype.big.call(receiver,extra);
                    return attributed+"|"+plain+"|"+log;
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "<a name=\"undefined\">null</a>|<big>body</big>|r:string,a:string,|r:string,",
        )),
        "CreateHTML argument conversion count or receiver-before-attribute order drifted",
    );

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var log="",receiver=Object(),attribute=Object();
                    receiver[Symbol.toPrimitive]=function(){log+="r,";throw 71};
                    attribute[Symbol.toPrimitive]=function(){log+="a,";throw 72};
                    var first;
                    try{String.prototype.link.call(receiver,attribute)}catch(error){first=error+":"+log}
                    log="";
                    receiver[Symbol.toPrimitive]=function(){log+="r,";return "body"};
                    var second;
                    try{String.prototype.link.call(receiver,attribute)}catch(error){second=error+":"+log}
                    return first+"|"+second;
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("71:r,|72:r,a,")),
        "CreateHTML replaced or reordered a user conversion throw",
    );
}

#[test]
fn string_create_html_escapes_only_quotes_and_preserves_raw_utf16_nul_and_ropes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let anchor_key = runtime.intern_property_key("anchor").unwrap();
    let Value::Object(anchor_object) = context.get_property(&prototype, &anchor_key).unwrap()
    else {
        panic!("String.prototype.anchor was not an object");
    };
    let anchor = runtime.as_callable(&anchor_object).unwrap().unwrap();
    let receiver =
        JsString::try_from_utf16([0x3c, 0x26, 0x3e, 0x22, 0x27, 0, 0xd83d, 0xde00, 0xd800])
            .unwrap();
    let attribute =
        JsString::try_from_utf16([0x22, 0x26, 0x3c, 0x3e, 0x27, 0, 0xd83d, 0xde00, 0xdc00])
            .unwrap();
    let Value::String(rendered) = context
        .call(
            &anchor,
            Value::String(receiver.clone()),
            &[Value::String(attribute)],
        )
        .unwrap()
    else {
        panic!("String.prototype.anchor did not return a String");
    };
    let mut expected = "<a name=\"&quot;&<>'".encode_utf16().collect::<Vec<_>>();
    expected.extend([0, 0xd83d, 0xde00, 0xdc00]);
    expected.extend("\">".encode_utf16());
    expected.extend(receiver.utf16_units());
    expected.extend("</a>".encode_utf16());
    assert_eq!(rendered, JsString::try_from_utf16(expected).unwrap());
    assert!(rendered.is_flat());
    assert!(rendered.is_wide());

    let Value::String(narrow) = context.eval(r#""x".fontcolor("\"")"#).unwrap() else {
        panic!("String.prototype.fontcolor did not return a String");
    };
    assert_eq!(
        narrow,
        JsString::from_static("<font color=\"&quot;\">x</font>"),
    );
    assert!(narrow.is_flat());
    assert!(!narrow.is_wide());

    let link_key = runtime.intern_property_key("link").unwrap();
    let Value::Object(link_object) = context.get_property(&prototype, &link_key).unwrap() else {
        panic!("String.prototype.link was not an object");
    };
    let link = runtime.as_callable(&link_object).unwrap().unwrap();
    let rope_receiver = JsString::try_from_utf8(&"R".repeat(8_193))
        .unwrap()
        .try_concat(&JsString::try_from_utf16([0xd800, u16::from(b'Z')]).unwrap())
        .unwrap();
    let rope_attribute = JsString::try_from_utf8(&"A".repeat(8_193))
        .unwrap()
        .try_concat(&JsString::try_from_utf16([0x22, 0, 0xde00]).unwrap())
        .unwrap();
    assert!(!rope_receiver.is_flat());
    assert!(!rope_attribute.is_flat());
    let Value::String(rendered) = context
        .call(
            &link,
            Value::String(rope_receiver),
            &[Value::String(rope_attribute)],
        )
        .unwrap()
    else {
        panic!("String.prototype.link did not return a rope fixture String");
    };
    assert!(rendered.is_flat());
    assert!(rendered.is_wide());
    assert_eq!(rendered.len(), 16_411);
    for (index, expected) in [
        (0, u16::from(b'<')),
        (8, u16::from(b'\"')),
        (9, u16::from(b'A')),
        (8_201, u16::from(b'A')),
        (8_202, u16::from(b'&')),
        (8_207, u16::from(b';')),
        (8_208, 0),
        (8_209, 0xde00),
        (8_210, u16::from(b'\"')),
        (8_211, u16::from(b'>')),
        (8_212, u16::from(b'R')),
        (16_404, u16::from(b'R')),
        (16_405, 0xd800),
        (16_406, u16::from(b'Z')),
        (16_407, u16::from(b'<')),
        (16_410, u16::from(b'>')),
    ] {
        assert_eq!(rendered.code_unit_at(index), Some(expected), "unit {index}");
    }
}

#[test]
fn string_create_html_small_limit_latches_too_long_but_attribute_throw_wins() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"globalThis.createHtmlLimitLog="";
                globalThis.createHtmlLimitReceiver=Object();
                createHtmlLimitReceiver[Symbol.toPrimitive]=function(hint){
                    createHtmlLimitLog+="r:"+hint+",";return "B"
                };
                globalThis.createHtmlLimitAttribute=Object();
                createHtmlLimitAttribute[Symbol.toPrimitive]=function(hint){
                    createHtmlLimitLog+="a:"+hint+",";return "Q"
                };
                globalThis.createHtmlLimitExtra=Object();
                createHtmlLimitExtra[Symbol.toPrimitive]=function(hint){
                    createHtmlLimitLog+="x:"+hint+",";throw 91
                };
                globalThis.createHtmlLimitThrow=Object();
                createHtmlLimitThrow[Symbol.toPrimitive]=function(hint){
                    createHtmlLimitLog+="t:"+hint+",";throw 72
                };"#,
        )
        .unwrap();
    let receiver = context.eval("createHtmlLimitReceiver").unwrap();
    let attribute = context.eval("createHtmlLimitAttribute").unwrap();
    let extra = context.eval("createHtmlLimitExtra").unwrap();
    let completion = runtime
        .call_string_prototype_create_html_with_limit(
            context.realm,
            StringCreateHtmlKind::Anchor,
            NativeInvocation::Call {
                this_value: receiver.clone(),
            },
            &NativeArguments {
                actual_arg_count: 2,
                readable: vec![attribute.clone(), extra],
            },
            16,
        )
        .unwrap();
    let Completion::Throw(Value::Object(error)) = completion else {
        panic!("one-below-boundary CreateHTML did not throw an Error object");
    };
    for (name, expected) in [("name", "InternalError"), ("message", "string too long")] {
        let Value::String(value) = context
            .get_property(&error, &runtime.intern_property_key(name).unwrap())
            .unwrap()
        else {
            panic!("small-limit CreateHTML {name} was not a String");
        };
        assert_eq!(value, JsString::from_static(expected));
    }
    assert_eq!(
        context.eval("createHtmlLimitLog").unwrap(),
        Value::String(JsString::from_static("r:string,a:string,")),
        "a latched prefix failure skipped the attribute or read an extra argument",
    );

    assert_eq!(
        runtime
            .call_string_prototype_create_html_with_limit(
                context.realm,
                StringCreateHtmlKind::Anchor,
                NativeInvocation::Call {
                    this_value: Value::String(JsString::from_static("B")),
                },
                &NativeArguments {
                    actual_arg_count: 1,
                    readable: vec![Value::String(JsString::from_static("Q"))],
                },
                17,
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("<a name=\"Q\">B</a>",))),
        "the exact CreateHTML output limit was rejected",
    );

    context.eval("createHtmlLimitLog=''").unwrap();
    let throwing_attribute = context.eval("createHtmlLimitThrow").unwrap();
    assert_eq!(
        runtime
            .call_string_prototype_create_html_with_limit(
                context.realm,
                StringCreateHtmlKind::Anchor,
                NativeInvocation::Call {
                    this_value: receiver,
                },
                &NativeArguments {
                    actual_arg_count: 1,
                    readable: vec![throwing_attribute],
                },
                1,
            )
            .unwrap(),
        Completion::Throw(Value::Int(72)),
        "CreateHTML's latched TooLong replaced a later user throw",
    );
    assert_eq!(
        context.eval("createHtmlLimitLog").unwrap(),
        Value::String(JsString::from_static("r:string,t:string,")),
    );
}

#[test]
fn string_create_html_reservation_oom_uses_defining_realm_is_latched_and_recovers() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let prototype = defining.string_prototype().unwrap();
    let anchor_key = runtime.intern_property_key("anchor").unwrap();
    let Value::Object(anchor_object) = defining.get_property(&prototype, &anchor_key).unwrap()
    else {
        panic!("String.prototype.anchor was not an object");
    };
    let anchor = runtime.as_callable(&anchor_object).unwrap().unwrap();
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
            r#"globalThis.createHtmlReservationLog="";
                globalThis.createHtmlReservationReceiver=Object();
                createHtmlReservationReceiver[Symbol.toPrimitive]=function(hint){
                    createHtmlReservationLog+="r:"+hint+",";return "B"
                };
                globalThis.createHtmlReservationAttribute=Object();
                createHtmlReservationAttribute[Symbol.toPrimitive]=function(hint){
                    createHtmlReservationLog+="a:"+hint+",";return "Q"
                };
                globalThis.createHtmlReservationThrow=Object();
                createHtmlReservationThrow[Symbol.toPrimitive]=function(hint){
                    createHtmlReservationLog+="t:"+hint+",";throw 73
                };"#,
        )
        .unwrap();
    let receiver = caller.eval("createHtmlReservationReceiver").unwrap();
    let attribute = caller.eval("createHtmlReservationAttribute").unwrap();

    crate::engine::value::fail_next_create_html_reservation_for_test();
    assert_eq!(
        caller.call(&anchor, receiver.clone(), std::slice::from_ref(&attribute),),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(error)) = caller.take_exception().unwrap() else {
        panic!("CreateHTML reservation failure did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_internal_error),
        "CreateHTML reservation OOM did not use the function's defining realm",
    );
    for (name, expected) in [("name", "InternalError"), ("message", "out of memory")] {
        let Value::String(value) = caller
            .get_property(&error, &runtime.intern_property_key(name).unwrap())
            .unwrap()
        else {
            panic!("CreateHTML reservation OOM {name} was not a String");
        };
        assert_eq!(value, JsString::from_static(expected));
    }
    assert_eq!(
        caller.eval("createHtmlReservationLog").unwrap(),
        Value::String(JsString::from_static("r:string,a:string,")),
        "CreateHTML initial OOM did not remain latched through attribute conversion",
    );
    assert_eq!(
        caller
            .call(&anchor, receiver.clone(), std::slice::from_ref(&attribute),)
            .unwrap(),
        Value::String(JsString::from_static("<a name=\"Q\">B</a>")),
        "runtime did not recover after CreateHTML reservation OOM",
    );

    caller.eval("createHtmlReservationLog=''").unwrap();
    let throwing_attribute = caller.eval("createHtmlReservationThrow").unwrap();
    crate::engine::value::fail_next_create_html_reservation_for_test();
    assert_eq!(
        caller.call(&anchor, receiver.clone(), &[throwing_attribute]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Int(73)),
        "a latched CreateHTML OOM replaced the later user throw",
    );
    assert_eq!(
        caller.eval("createHtmlReservationLog").unwrap(),
        Value::String(JsString::from_static("r:string,t:string,")),
    );
    assert_eq!(
        caller.call(&anchor, receiver, &[attribute]).unwrap(),
        Value::String(JsString::from_static("<a name=\"Q\">B</a>")),
        "the CreateHTML OOM hook was not consumed exactly once before a user throw",
    );
}

#[test]
fn saved_create_html_keeps_its_realm_alive_and_cross_realm_throws_stay_exact() {
    let runtime = Runtime::new();
    let (defining_realm, anchor) = {
        let mut defining = runtime.new_context();
        let prototype = defining.string_prototype().unwrap();
        let key = runtime.intern_property_key("anchor").unwrap();
        let Value::Object(object) = defining.get_property(&prototype, &key).unwrap() else {
            panic!("String.prototype.anchor was not an object");
        };
        let callable = runtime.as_callable(&object).unwrap().unwrap();
        (defining.realm, callable)
    };
    runtime.run_gc().unwrap();
    let defining_type_error = runtime
        .0
        .state
        .borrow()
        .heap
        .context(defining_realm)
        .expect("saved CreateHTML did not retain its defining realm")
        .native_error_prototypes[NativeErrorKind::Type.index()]
    .expect("defining realm had no TypeError prototype");

    let mut caller = runtime.new_context();
    let Value::Object(caller_error_prototype) = caller.eval("Error.prototype").unwrap() else {
        panic!("caller Error.prototype was not an object");
    };
    let user_receiver = caller
        .eval(
            r#"(function(){
                var receiver=Object();
                receiver[Symbol.toPrimitive]=function(){throw new Error("user")};
                return receiver;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        caller.call(
            &anchor,
            user_receiver,
            &[Value::String(JsString::from_static("name"))],
        ),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(user_error)) = caller.take_exception().unwrap() else {
        panic!("CreateHTML receiver did not preserve the user's Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_error_prototype.clone()),
    );
    drop(user_error);

    assert_eq!(
        caller.call(
            &anchor,
            Value::String(JsString::from_static("body")),
            &[Value::Null],
        ),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(type_error)) = caller.take_exception().unwrap() else {
        panic!("cross-realm CreateHTML null attribute did not throw an Error object");
    };
    let type_error_prototype = runtime
        .get_prototype_of(&type_error)
        .unwrap()
        .expect("CreateHTML TypeError had no prototype");
    assert_eq!(
        type_error_prototype.object_id(),
        defining_type_error,
        "CreateHTML conversion TypeError did not use the function's defining realm",
    );
    drop(type_error_prototype);
    drop(type_error);
    drop(anchor);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .is_err(),
        "dropping the saved CreateHTML did not release its defining realm",
    );
}
