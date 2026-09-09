use crate::engine::builtins::native::NativeCProto;
use crate::engine::heap::{AutoInitProperty, PropertySlot, RawValue};
use crate::engine::object::shape::PropertyFlags;

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
fn string_trim_selectors_use_pinned_generic_magic_cproto() {
    for selector in [
        StringTrimKind::Both,
        StringTrimKind::End,
        StringTrimKind::Start,
    ] {
        let descriptor = NativeFunctionId::StringPrototypeTrim(selector).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::GenericMagic);
        assert!(!descriptor.cproto.default_is_constructor());
    }
}

#[test]
fn string_create_html_selectors_use_pinned_generic_magic_cproto() {
    for (_, selector, _) in STRING_CREATE_HTML_ENTRIES {
        let descriptor = NativeFunctionId::StringPrototypeCreateHtml(selector).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::GenericMagic);
        assert!(!descriptor.cproto.default_is_constructor());
    }
}

#[test]
fn string_case_selectors_use_pinned_generic_magic_cproto() {
    for selector in [StringCaseKind::Lower, StringCaseKind::Upper] {
        let descriptor = NativeFunctionId::StringPrototypeCase(selector).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::GenericMagic);
        assert!(!descriptor.cproto.default_is_constructor());
    }
}

#[test]
fn string_unicode_intrinsics_use_pinned_generic_cproto_and_append_order() {
    for target in [
        NativeFunctionId::StringPrototypeNormalize,
        NativeFunctionId::StringPrototypeLocaleCompare,
    ] {
        let descriptor = target.descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::Generic);
        assert!(!descriptor.cproto.default_is_constructor());
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let sup = runtime.intern_property_key("sup").unwrap();
    let constructor = runtime.intern_property_key("constructor").unwrap();
    let normalize = runtime.intern_property_key("normalize").unwrap();
    let locale_compare = runtime.intern_property_key("localeCompare").unwrap();
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(prototype.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let sup = usize::try_from(shape.find(sup.atom()).unwrap()).unwrap();
        let constructor = usize::try_from(shape.find(constructor.atom()).unwrap()).unwrap();
        let normalize = usize::try_from(shape.find(normalize.atom()).unwrap()).unwrap();
        let locale_compare = usize::try_from(shape.find(locale_compare.atom()).unwrap()).unwrap();
        assert_eq!(constructor, sup + 1);
        assert_eq!(normalize, constructor + 1);
        assert_eq!(locale_compare, normalize + 1);
        assert_eq!(
            shape.entries()[normalize].flags,
            PropertyFlags::data(true, false, true),
        );
        assert_eq!(
            shape.entries()[locale_compare].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(normalize),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::StringPrototypeNormalize,
                name: "normalize",
                length: 0,
                min_readable_args: 0,
            })) if *realm == context.realm
        ));
        assert!(matches!(
            object.slots.get(locale_compare),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::StringPrototypeLocaleCompare,
                name: "localeCompare",
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm
        ));
    }

    let Value::Object(function) = context.get_property(&prototype, &normalize).unwrap() else {
        panic!("String.prototype.normalize did not materialize as a function");
    };
    assert!(runtime.as_callable(&function).unwrap().is_some());
    assert!(!runtime.is_constructor(&function).unwrap());
    assert_eq!(
        context
            .eval("String.prototype.normalize.name+'|'+String.prototype.normalize.length")
            .unwrap(),
        Value::String(JsString::from_static("normalize|0")),
    );
    let Value::Object(function) = context.get_property(&prototype, &locale_compare).unwrap() else {
        panic!("String.prototype.localeCompare did not materialize as a function");
    };
    assert!(runtime.as_callable(&function).unwrap().is_some());
    assert!(!runtime.is_constructor(&function).unwrap());
    assert_eq!(
        context
            .eval("String.prototype.localeCompare.name+'|'+String.prototype.localeCompare.length")
            .unwrap(),
        Value::String(JsString::from_static("localeCompare|1")),
    );
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
fn string_trim_family_preserves_alias_materialization_order_and_independence() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let entries = [
        ("trim", StringTrimKind::Both),
        ("trimEnd", StringTrimKind::End),
        ("trimRight", StringTrimKind::End),
        ("trimStart", StringTrimKind::Start),
        ("trimLeft", StringTrimKind::Start),
    ];
    let keys = entries.map(|(name, selector)| {
        (
            name,
            selector,
            runtime
                .intern_property_key(name)
                .expect("String trim-family key must intern"),
        )
    });
    let (trim_end_id, trim_start_id) = {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(prototype.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_indices = keys
            .each_ref()
            .map(|(_, _, key)| usize::try_from(shape.find(key.atom()).unwrap()).unwrap());
        assert!(
            slot_indices.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "the five trim-family entries did not retain QuickJS table order",
        );
        for slot_index in slot_indices {
            assert_eq!(
                shape.entries()[slot_index].flags,
                PropertyFlags::data(true, false, true),
            );
        }
        assert!(matches!(
            object.slots.get(slot_indices[0]),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::StringPrototypeTrim(StringTrimKind::Both),
                name: "trim",
                length: 0,
                min_readable_args: 0,
            })) if *realm == context.realm
        ));

        let Some(PropertySlot::Data(RawValue::Object(trim_end_id))) =
            object.slots.get(slot_indices[1])
        else {
            panic!("trimEnd was not eagerly materialized for trimRight");
        };
        assert!(matches!(
            object.slots.get(slot_indices[2]),
            Some(PropertySlot::Data(RawValue::Object(alias_id))) if alias_id == trim_end_id
        ));
        let Some(PropertySlot::Data(RawValue::Object(trim_start_id))) =
            object.slots.get(slot_indices[3])
        else {
            panic!("trimStart was not eagerly materialized for trimLeft");
        };
        assert!(matches!(
            object.slots.get(slot_indices[4]),
            Some(PropertySlot::Data(RawValue::Object(alias_id))) if alias_id == trim_start_id
        ));
        for (id, selector) in [
            (*trim_end_id, StringTrimKind::End),
            (*trim_start_id, StringTrimKind::Start),
        ] {
            assert!(matches!(
                &state.heap.object(id).unwrap().payload,
                ObjectPayload::NativeFunction { data, .. }
                    if data.target == NativeFunctionId::StringPrototypeTrim(selector)
                        && data.realm == Some(context.realm)
                        && data.min_readable_args == 0
            ));
        }
        (*trim_end_id, *trim_start_id)
    };

    let mut functions = Vec::new();
    for (name, _, key) in &keys {
        let Value::Object(first) = context.get_property(&prototype, key).unwrap() else {
            panic!("{name} did not resolve to a function object");
        };
        let Value::Object(second) = context.get_property(&prototype, key).unwrap() else {
            panic!("{name} did not remain a function object");
        };
        assert_eq!(first, second, "{name} identity was unstable");
        assert!(runtime.as_callable(&first).unwrap().is_some());
        assert!(!runtime.is_constructor(&first).unwrap());
        functions.push(first);
    }
    assert_ne!(functions[0], functions[1]);
    assert_ne!(functions[0], functions[3]);
    assert_ne!(functions[1], functions[3]);
    assert_eq!(functions[1], functions[2]);
    assert_eq!(functions[3], functions[4]);
    assert_eq!(functions[1].object_id(), trim_end_id);
    assert_eq!(functions[3].object_id(), trim_start_id);

    let length = runtime.intern_property_key("length").unwrap();
    let name = runtime.intern_property_key("name").unwrap();
    for (function, expected_name) in
        functions
            .iter()
            .zip(["trim", "trimEnd", "trimEnd", "trimStart", "trimStart"])
    {
        assert_eq!(
            context.get_property(function, &length).unwrap(),
            Value::Int(0),
        );
        assert_eq!(
            context.get_property(function, &name).unwrap(),
            Value::String(JsString::try_from_utf8(expected_name).unwrap()),
        );
    }

    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var p=String.prototype,end=p.trimEnd,start=p.trimStart,rows=[];
                    p.trimRight=91;
                    rows.push(p.trimEnd===end,p.trimRight===91);
                    delete p.trimEnd;
                    rows.push(!("trimEnd" in p),p.trimRight===91);
                    p.trimStart=92;
                    rows.push(p.trimLeft===start,p.trimStart===92);
                    delete p.trimLeft;
                    rows.push(!("trimLeft" in p),p.trimStart===92);
                    return rows.join("|");
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "true|true|true|true|true|true|true|true",
        )),
        "overwriting or deleting one alias property changed its peer",
    );
}
