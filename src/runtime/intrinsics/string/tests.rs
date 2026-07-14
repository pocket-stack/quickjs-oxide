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
fn string_subrange_selectors_use_pinned_generic_cproto() {
    for selector in [
        StringSubrangeKind::Substring,
        StringSubrangeKind::Substr,
        StringSubrangeKind::Slice,
    ] {
        let descriptor = NativeFunctionId::StringPrototypeSubrange(selector).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::Generic);
        assert!(!descriptor.cproto.default_is_constructor());
    }

    let descriptor = NativeFunctionId::StringPrototypeRepeat.descriptor();
    assert_eq!(descriptor.cproto, NativeCProto::Generic);
    assert!(!descriptor.cproto.default_is_constructor());

    assert_eq!(string_to_int32_clamp(f64::NAN, 6, 6), 0);
    assert_eq!(string_to_int32_clamp(f64::NEG_INFINITY, 6, 6), 0);
    assert_eq!(string_to_int32_clamp(-2.9, 6, 6), 4);
    assert_eq!(string_to_int32_clamp(f64::INFINITY, 6, 6), 6);
}

#[test]
fn string_pad_selectors_use_pinned_generic_magic_cproto() {
    for selector in [StringPadKind::End, StringPadKind::Start] {
        let descriptor = NativeFunctionId::StringPrototypePad(selector).descriptor();
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
fn string_subrange_family_publishes_generic_autoinit_entries_and_identities() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let entries = [
        ("substring", StringSubrangeKind::Substring),
        ("substr", StringSubrangeKind::Substr),
        ("slice", StringSubrangeKind::Slice),
    ];
    let keys = entries.map(|(name, selector)| {
        (
            name,
            selector,
            runtime
                .intern_property_key(name)
                .expect("String subrange-family key must intern"),
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
                target: NativeFunctionId::StringPrototypeSubrange(target_selector),
                name: target_name,
                length: 2,
                min_readable_args: 2,
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
fn string_repeat_publishes_one_generic_autoinit_entry() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let key = runtime.intern_property_key("repeat").unwrap();
    let state = runtime.0.state.borrow();
    let object = state.heap.object(prototype.object_id()).unwrap();
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
            target: NativeFunctionId::StringPrototypeRepeat,
            name,
            length: 1,
            min_readable_args: 1,
        })) if *realm == context.realm && *name == "repeat"
    ));
    drop(state);

    let Value::Object(first) = context.get_property(&prototype, &key).unwrap() else {
        panic!("repeat did not materialize as a function object");
    };
    let Value::Object(second) = context.get_property(&prototype, &key).unwrap() else {
        panic!("repeat did not remain a function object");
    };
    assert_eq!(first, second);
    assert!(runtime.as_callable(&first).unwrap().is_some());
    assert!(!runtime.is_constructor(&first).unwrap());
}

#[test]
fn string_pad_family_publishes_pinned_autoinit_entries_and_identities() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let entries = [
        ("padEnd", StringPadKind::End),
        ("padStart", StringPadKind::Start),
    ];
    let keys = entries.map(|(name, selector)| {
        (
            name,
            selector,
            runtime
                .intern_property_key(name)
                .expect("String pad-family key must intern"),
        )
    });
    let state = runtime.0.state.borrow();
    let object = state.heap.object(prototype.object_id()).unwrap();
    let shape = state.heap.shape(object.shape).unwrap();
    let slot_indices = keys
        .each_ref()
        .map(|(_, _, key)| usize::try_from(shape.find(key.atom()).unwrap()).unwrap());
    assert!(
        slot_indices[0] < slot_indices[1],
        "padEnd must precede padStart"
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
                target: NativeFunctionId::StringPrototypePad(target_selector),
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
}

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

    crate::value::fail_next_repeat_reservation_for_test();
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

    crate::value::fail_next_pad_reservation_for_test();
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

#[test]
fn recursive_string_conversion_family_is_guarded_on_libtest_stack_and_recovers() {
    std::thread::Builder::new()
        .name("string-conversion-stack-proof".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context
                .eval(
                    r#"function stringSubrangeRecurse(kind,depth){
                var start=Object();
                start[Symbol.toPrimitive]=function(){
                    if(depth!==0)stringSubrangeRecurse((kind+1)%3,depth-1);
                    return 0;
                };
                if(kind===0)return "x".substring(start);
                if(kind===1)return "x".substr(start);
                return "x".slice(start);
            }"#,
                )
                .unwrap();

            for kind in 0..3 {
                assert_eq!(
                    context
                        .eval(&format!("stringSubrangeRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::from_static("x")),
                    "the proven-safe four-frame subrange chain was rejected for kind {kind}"
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                        try{{stringSubrangeRecurse({kind},4);return "missing"}}
                        catch(error){{return error.name+":"+error.message}}
                    }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "the fifth subrange family frame was not rejected for kind {kind}"
                );
            }

            context
                .eval(
                    r#"function mixedStringSearchRecurse(kind,depth){
                        if(kind===0){
                            var search=Object(),descriptor=Object();
                            descriptor.get=function(){
                                if(depth!==0)mixedStringSearchRecurse(1,depth-1);
                                return false;
                            };
                            Object.defineProperty(search,Symbol.match,descriptor);
                            search[Symbol.toPrimitive]=function(){return "x"};
                            return "x".includes(search);
                        }
                        var start=Object();
                        start[Symbol.toPrimitive]=function(){
                            if(depth!==0)mixedStringSearchRecurse(0,depth-1);
                            return 0;
                        };
                        return "x".slice(start);
                    }"#,
                )
                .unwrap();
            assert_eq!(
                context.eval("mixedStringSearchRecurse(0,3)").unwrap(),
                Value::Bool(true),
                "the proven-safe four-frame includes/subrange chain was rejected"
            );
            assert_eq!(
                context.eval("mixedStringSearchRecurse(1,3)").unwrap(),
                Value::String(JsString::from_static("x")),
                "the reverse four-frame includes/subrange chain was rejected"
            );
            assert_eq!(
                context
                    .eval(
                        r#"(function(){
                            try{mixedStringSearchRecurse(0,4);return "missing"}
                            catch(error){return error.name+":"+error.message}
                        })()"#,
                    )
                    .unwrap(),
                Value::String(JsString::from_static("InternalError:stack overflow")),
                "alternating includes/subrange calls bypassed the shared fifth-frame guard"
            );

            context
                .eval(
                    r#"function mixedStringRepeatRecurse(kind,depth){
                        if(kind===0){
                            var count=Object();
                            count[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringRepeatRecurse(1,depth-1);
                                return 1;
                            };
                            return "x".repeat(count);
                        }
                        var start=Object();
                        start[Symbol.toPrimitive]=function(){
                            if(depth!==0)mixedStringRepeatRecurse(0,depth-1);
                            return 0;
                        };
                        return "x".slice(start);
                    }"#,
                )
                .unwrap();
            for kind in 0..2 {
                assert_eq!(
                    context
                        .eval(&format!("mixedStringRepeatRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::from_static("x")),
                    "the proven-safe repeat/subrange chain was rejected for kind {kind}"
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringRepeatRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "alternating repeat/subrange calls bypassed the shared fifth-frame guard"
                );
            }

            context
                .eval(
                    r#"function mixedStringPadRecurse(kind,depth){
                        if(kind===0){
                            var target=Object();
                            target[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringPadRecurse(1,depth-1);
                                return 2;
                            };
                            return "x".padEnd(target);
                        }
                        if(kind===1){
                            var count=Object();
                            count[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringPadRecurse(2,depth-1);
                                return 1;
                            };
                            return "x".repeat(count);
                        }
                        if(kind===2){
                            var start=Object();
                            start[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringPadRecurse(3,depth-1);
                                return 0;
                            };
                            return "x".slice(start);
                        }
                        var filler=Object();
                        filler[Symbol.toPrimitive]=function(){
                            if(depth!==0)mixedStringPadRecurse(0,depth-1);
                            return "_";
                        };
                        return "x".padStart(2,filler);
                    }"#,
                )
                .unwrap();
            for (kind, expected) in [(0, "x "), (1, "x"), (2, "x"), (3, "_x")] {
                assert_eq!(
                    context
                        .eval(&format!("mixedStringPadRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::try_from_utf8(expected).unwrap()),
                    "the proven-safe pad/repeat/slice chain was rejected for kind {kind}"
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringPadRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "alternating pad/repeat/slice calls bypassed the shared fifth-frame guard"
                );
            }
            assert_eq!(
                context
                    .eval(
                        r#""abc".includes("b")+"|"+"abc".slice(1)+"|"+
                           "ab".repeat(2)+"|"+"a".padEnd(3,"x")+"|"+"a".padStart(3,"x")"#,
                    )
                    .unwrap(),
                Value::String(JsString::from_static("true|bc|abab|axx|xxa")),
                "the runtime did not recover after mixed String-family overflow"
            );
        })
        .expect("2 MiB String conversion stack-proof thread did not start")
        .join()
        .expect("2 MiB String conversion stack-proof thread panicked");
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
