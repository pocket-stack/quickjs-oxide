use super::*;

const STRING_CREATE_HTML_ENTRIES: [(&str, StringCreateHtmlKind, u8); 13] = [
    ("anchor", StringCreateHtmlKind::Anchor, 1),
    ("big", StringCreateHtmlKind::Big, 0),
    ("blink", StringCreateHtmlKind::Blink, 0),
    ("bold", StringCreateHtmlKind::Bold, 0),
    ("fixed", StringCreateHtmlKind::Fixed, 0),
    ("fontcolor", StringCreateHtmlKind::FontColor, 1),
    ("fontsize", StringCreateHtmlKind::FontSize, 1),
    ("italics", StringCreateHtmlKind::Italics, 0),
    ("link", StringCreateHtmlKind::Link, 1),
    ("small", StringCreateHtmlKind::Small, 0),
    ("strike", StringCreateHtmlKind::Strike, 0),
    ("sub", StringCreateHtmlKind::Sub, 0),
    ("sup", StringCreateHtmlKind::Sup, 0),
];

const STRING_CASE_ENTRIES: [(&str, StringCaseKind); 4] = [
    ("toLowerCase", StringCaseKind::Lower),
    ("toUpperCase", StringCaseKind::Upper),
    ("toLocaleLowerCase", StringCaseKind::Lower),
    ("toLocaleUpperCase", StringCaseKind::Upper),
];

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
fn match_match_all_search_and_split_entries_preserve_pinned_cproto_and_order() {
    let string_match_descriptor = NativeFunctionId::StringPrototypeMatch.descriptor();
    assert_eq!(string_match_descriptor.cproto, NativeCProto::GenericMagic);
    assert!(!string_match_descriptor.cproto.default_is_constructor());
    let string_match_all_descriptor = NativeFunctionId::StringPrototypeMatchAll.descriptor();
    assert_eq!(
        string_match_all_descriptor.cproto,
        NativeCProto::GenericMagic
    );
    assert!(!string_match_all_descriptor.cproto.default_is_constructor());
    let string_descriptor = NativeFunctionId::StringPrototypeSearch.descriptor();
    assert_eq!(string_descriptor.cproto, NativeCProto::GenericMagic);
    assert!(!string_descriptor.cproto.default_is_constructor());
    let regexp_match_descriptor = NativeFunctionId::RegExp(RegExpNativeKind::Match).descriptor();
    assert_eq!(regexp_match_descriptor.cproto, NativeCProto::Generic);
    assert!(!regexp_match_descriptor.cproto.default_is_constructor());
    let regexp_match_all_descriptor =
        NativeFunctionId::RegExp(RegExpNativeKind::MatchAll).descriptor();
    assert_eq!(regexp_match_all_descriptor.cproto, NativeCProto::Generic);
    assert!(!regexp_match_all_descriptor.cproto.default_is_constructor());
    let regexp_descriptor = NativeFunctionId::RegExp(RegExpNativeKind::Search).descriptor();
    assert_eq!(regexp_descriptor.cproto, NativeCProto::Generic);
    assert!(!regexp_descriptor.cproto.default_is_constructor());
    let regexp_split_descriptor = NativeFunctionId::RegExp(RegExpNativeKind::Split).descriptor();
    assert_eq!(regexp_split_descriptor.cproto, NativeCProto::Generic);
    assert!(!regexp_split_descriptor.cproto.default_is_constructor());

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let string_prototype = context.string_prototype().unwrap();
    let Value::Object(regexp_prototype) = context.eval("RegExp.prototype").unwrap() else {
        panic!("RegExp.prototype was not an object");
    };
    let starts_with = runtime.intern_property_key("startsWith").unwrap();
    let string_match = runtime.intern_property_key("match").unwrap();
    let string_match_all = runtime.intern_property_key("matchAll").unwrap();
    let string_search = runtime.intern_property_key("search").unwrap();
    let split = runtime.intern_property_key("split").unwrap();
    let symbol_match = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Match));
    let symbol_match_all = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::MatchAll));
    let symbol_search = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Search));
    let symbol_split = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Split));
    {
        let state = runtime.0.state.borrow();
        let string_object = state.heap.object(string_prototype.object_id()).unwrap();
        let string_shape = state.heap.shape(string_object.shape).unwrap();
        let starts_with = usize::try_from(string_shape.find(starts_with.atom()).unwrap()).unwrap();
        let match_position =
            usize::try_from(string_shape.find(string_match.atom()).unwrap()).unwrap();
        let match_all =
            usize::try_from(string_shape.find(string_match_all.atom()).unwrap()).unwrap();
        let search = usize::try_from(string_shape.find(string_search.atom()).unwrap()).unwrap();
        let split = usize::try_from(string_shape.find(split.atom()).unwrap()).unwrap();
        assert_eq!(match_position, starts_with + 1);
        assert_eq!(match_all, match_position + 1);
        assert_eq!(search, match_all + 1);
        assert_eq!(split, search + 1);
        assert_eq!(
            string_shape.entries()[match_position].flags,
            PropertyFlags::data(true, false, true)
        );
        assert!(matches!(
            string_object.slots.get(match_position),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::StringPrototypeMatch,
                name: "match",
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm
        ));
        assert_eq!(
            string_shape.entries()[match_all].flags,
            PropertyFlags::data(true, false, true)
        );
        assert!(matches!(
            string_object.slots.get(match_all),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::StringPrototypeMatchAll,
                name: "matchAll",
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm
        ));
        assert_eq!(
            string_shape.entries()[search].flags,
            PropertyFlags::data(true, false, true)
        );
        assert!(matches!(
            string_object.slots.get(search),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::StringPrototypeSearch,
                name: "search",
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm
        ));

        let regexp_object = state.heap.object(regexp_prototype.object_id()).unwrap();
        let regexp_shape = state.heap.shape(regexp_object.shape).unwrap();
        let match_position =
            usize::try_from(regexp_shape.find(symbol_match.atom()).unwrap()).unwrap();
        let match_all =
            usize::try_from(regexp_shape.find(symbol_match_all.atom()).unwrap()).unwrap();
        let search = usize::try_from(regexp_shape.find(symbol_search.atom()).unwrap()).unwrap();
        let split = usize::try_from(regexp_shape.find(symbol_split.atom()).unwrap()).unwrap();
        assert_eq!(match_all, match_position + 1);
        assert_eq!(search, match_all + 1);
        assert_eq!(split, search + 1);
        assert_eq!(
            regexp_shape.entries()[match_position].flags,
            PropertyFlags::data(true, false, true)
        );
        assert!(matches!(
            regexp_object.slots.get(match_position),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::RegExp(RegExpNativeKind::Match),
                name: "[Symbol.match]",
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm
        ));
        assert_eq!(
            regexp_shape.entries()[match_all].flags,
            PropertyFlags::data(true, false, true)
        );
        assert!(matches!(
            regexp_object.slots.get(match_all),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::RegExp(RegExpNativeKind::MatchAll),
                name: "[Symbol.matchAll]",
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm
        ));
        assert_eq!(
            regexp_shape.entries()[search].flags,
            PropertyFlags::data(true, false, true)
        );
        assert!(matches!(
            regexp_object.slots.get(search),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::RegExp(RegExpNativeKind::Search),
                name: "[Symbol.search]",
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm
        ));
        assert_eq!(
            regexp_shape.entries()[split].flags,
            PropertyFlags::data(true, false, true)
        );
        assert!(matches!(
            regexp_object.slots.get(split),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::RegExp(RegExpNativeKind::Split),
                name: "[Symbol.split]",
                length: 2,
                min_readable_args: 2,
            })) if *realm == context.realm
        ));
    }

    let Value::Object(string_match_first) = context
        .get_property(&string_prototype, &string_match)
        .unwrap()
    else {
        panic!("String.prototype.match did not materialize as a function");
    };
    let Value::Object(string_match_second) = context
        .get_property(&string_prototype, &string_match)
        .unwrap()
    else {
        panic!("String.prototype.match did not retain its function identity");
    };
    assert_eq!(string_match_first, string_match_second);
    assert!(runtime.as_callable(&string_match_first).unwrap().is_some());
    assert!(!runtime.is_constructor(&string_match_first).unwrap());

    let Value::Object(string_match_all_first) = context
        .get_property(&string_prototype, &string_match_all)
        .unwrap()
    else {
        panic!("String.prototype.matchAll did not materialize as a function");
    };
    let Value::Object(string_match_all_second) = context
        .get_property(&string_prototype, &string_match_all)
        .unwrap()
    else {
        panic!("String.prototype.matchAll did not retain its function identity");
    };
    assert_eq!(string_match_all_first, string_match_all_second);
    assert!(
        runtime
            .as_callable(&string_match_all_first)
            .unwrap()
            .is_some()
    );
    assert!(!runtime.is_constructor(&string_match_all_first).unwrap());

    let Value::Object(string_first) = context
        .get_property(&string_prototype, &string_search)
        .unwrap()
    else {
        panic!("String.prototype.search did not materialize as a function");
    };
    let Value::Object(string_second) = context
        .get_property(&string_prototype, &string_search)
        .unwrap()
    else {
        panic!("String.prototype.search did not retain its function identity");
    };
    assert_eq!(string_first, string_second);
    assert!(runtime.as_callable(&string_first).unwrap().is_some());
    assert!(!runtime.is_constructor(&string_first).unwrap());

    let Value::Object(regexp_match_first) = context
        .get_property(&regexp_prototype, &symbol_match)
        .unwrap()
    else {
        panic!("RegExp.prototype[Symbol.match] did not materialize as a function");
    };
    let Value::Object(regexp_match_second) = context
        .get_property(&regexp_prototype, &symbol_match)
        .unwrap()
    else {
        panic!("RegExp.prototype[Symbol.match] did not retain its function identity");
    };
    assert_eq!(regexp_match_first, regexp_match_second);
    assert!(runtime.as_callable(&regexp_match_first).unwrap().is_some());
    assert!(!runtime.is_constructor(&regexp_match_first).unwrap());

    let Value::Object(regexp_match_all_first) = context
        .get_property(&regexp_prototype, &symbol_match_all)
        .unwrap()
    else {
        panic!("RegExp.prototype[Symbol.matchAll] did not materialize as a function");
    };
    let Value::Object(regexp_match_all_second) = context
        .get_property(&regexp_prototype, &symbol_match_all)
        .unwrap()
    else {
        panic!("RegExp.prototype[Symbol.matchAll] did not retain its function identity");
    };
    assert_eq!(regexp_match_all_first, regexp_match_all_second);
    assert!(
        runtime
            .as_callable(&regexp_match_all_first)
            .unwrap()
            .is_some()
    );
    assert!(!runtime.is_constructor(&regexp_match_all_first).unwrap());

    let Value::Object(regexp_first) = context
        .get_property(&regexp_prototype, &symbol_search)
        .unwrap()
    else {
        panic!("RegExp.prototype[Symbol.search] did not materialize as a function");
    };
    let Value::Object(regexp_second) = context
        .get_property(&regexp_prototype, &symbol_search)
        .unwrap()
    else {
        panic!("RegExp.prototype[Symbol.search] did not retain its function identity");
    };
    assert_eq!(regexp_first, regexp_second);
    assert!(runtime.as_callable(&regexp_first).unwrap().is_some());
    assert!(!runtime.is_constructor(&regexp_first).unwrap());
    let Value::Object(regexp_split_first) = context
        .get_property(&regexp_prototype, &symbol_split)
        .unwrap()
    else {
        panic!("RegExp.prototype[Symbol.split] did not materialize as a function");
    };
    let Value::Object(regexp_split_second) = context
        .get_property(&regexp_prototype, &symbol_split)
        .unwrap()
    else {
        panic!("RegExp.prototype[Symbol.split] did not retain its function identity");
    };
    assert_eq!(regexp_split_first, regexp_split_second);
    assert!(runtime.as_callable(&regexp_split_first).unwrap().is_some());
    assert!(!runtime.is_constructor(&regexp_split_first).unwrap());
    assert_ne!(string_match_first, string_match_all_first);
    assert_ne!(string_match_all_first, string_first);
    assert_ne!(string_match_first, string_first);
    assert_ne!(regexp_match_first, regexp_match_all_first);
    assert_ne!(regexp_match_all_first, regexp_first);
    assert_ne!(regexp_match_first, regexp_first);
    assert_ne!(regexp_split_first, regexp_first);
    assert_ne!(regexp_split_first, regexp_match_first);
    assert_ne!(string_match_first, regexp_match_first);
    assert_ne!(string_first, regexp_first);
    assert_eq!(
        context
            .eval(
                "String.prototype.match.name+'|'+String.prototype.match.length+'|'+\
                 String.prototype.matchAll.name+'|'+String.prototype.matchAll.length+'|'+\
                 String.prototype.search.name+'|'+String.prototype.search.length+'|'+\
                 RegExp.prototype[Symbol.match].name+'|'+\
                 RegExp.prototype[Symbol.match].length+'|'+\
                 RegExp.prototype[Symbol.matchAll].name+'|'+\
                 RegExp.prototype[Symbol.matchAll].length+'|'+\
                 RegExp.prototype[Symbol.search].name+'|'+\
                 RegExp.prototype[Symbol.search].length+'|'+\
                 RegExp.prototype[Symbol.split].name+'|'+\
                 RegExp.prototype[Symbol.split].length",
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "match|1|matchAll|1|search|1|[Symbol.match]|1|[Symbol.matchAll]|1|\
             [Symbol.search]|1|[Symbol.split]|2",
        )),
    );
}

#[test]
fn replace_entries_preserve_pinned_cproto_autoinit_and_table_order() {
    for selector in [StringReplaceKind::Replace, StringReplaceKind::ReplaceAll] {
        let descriptor = NativeFunctionId::StringPrototypeReplace(selector).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::GenericMagic);
        assert!(!descriptor.cproto.default_is_constructor());
    }
    let regexp_descriptor = NativeFunctionId::RegExp(RegExpNativeKind::Replace).descriptor();
    assert_eq!(regexp_descriptor.cproto, NativeCProto::Generic);
    assert!(!regexp_descriptor.cproto.default_is_constructor());

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let string_prototype = context.string_prototype().unwrap();
    let Value::Object(regexp_prototype) = context.eval("RegExp.prototype").unwrap() else {
        panic!("RegExp.prototype was not an object");
    };
    let repeat_key = runtime.intern_property_key("repeat").unwrap();
    let replace_key = runtime.intern_property_key("replace").unwrap();
    let replace_all_key = runtime.intern_property_key("replaceAll").unwrap();
    let pad_end_key = runtime.intern_property_key("padEnd").unwrap();
    let symbol_replace = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Replace));
    let symbol_match = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Match));

    {
        let state = runtime.0.state.borrow();
        let string_object = state.heap.object(string_prototype.object_id()).unwrap();
        let string_shape = state.heap.shape(string_object.shape).unwrap();
        let repeat = usize::try_from(string_shape.find(repeat_key.atom()).unwrap()).unwrap();
        let replace = usize::try_from(string_shape.find(replace_key.atom()).unwrap()).unwrap();
        let replace_all =
            usize::try_from(string_shape.find(replace_all_key.atom()).unwrap()).unwrap();
        let pad_end = usize::try_from(string_shape.find(pad_end_key.atom()).unwrap()).unwrap();
        assert_eq!(replace, repeat + 1);
        assert_eq!(replace_all, replace + 1);
        assert_eq!(pad_end, replace_all + 1);
        for (position, target, name) in [
            (
                replace,
                NativeFunctionId::StringPrototypeReplace(StringReplaceKind::Replace),
                "replace",
            ),
            (
                replace_all,
                NativeFunctionId::StringPrototypeReplace(StringReplaceKind::ReplaceAll),
                "replaceAll",
            ),
        ] {
            assert_eq!(
                string_shape.entries()[position].flags,
                PropertyFlags::data(true, false, true)
            );
            assert!(matches!(
                string_object.slots.get(position),
                Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                    realm,
                    target: actual_target,
                    name: actual_name,
                    length: 2,
                    min_readable_args: 2,
                })) if *realm == context.realm
                    && *actual_target == target
                    && *actual_name == name
            ));
        }

        let regexp_object = state.heap.object(regexp_prototype.object_id()).unwrap();
        let regexp_shape = state.heap.shape(regexp_object.shape).unwrap();
        let replace = usize::try_from(regexp_shape.find(symbol_replace.atom()).unwrap()).unwrap();
        let match_position =
            usize::try_from(regexp_shape.find(symbol_match.atom()).unwrap()).unwrap();
        assert_eq!(match_position, replace + 1);
        assert_eq!(
            regexp_shape.entries()[replace].flags,
            PropertyFlags::data(true, false, true)
        );
        assert!(matches!(
            regexp_object.slots.get(replace),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::RegExp(RegExpNativeKind::Replace),
                name: "[Symbol.replace]",
                length: 2,
                min_readable_args: 2,
            })) if *realm == context.realm
        ));
    }

    for (object, key) in [
        (&string_prototype, replace_key),
        (&string_prototype, replace_all_key),
        (&regexp_prototype, symbol_replace),
    ] {
        let Value::Object(first) = context.get_property(object, &key).unwrap() else {
            panic!("replace AutoInit entry did not materialize as a function");
        };
        let Value::Object(second) = context.get_property(object, &key).unwrap() else {
            panic!("replace AutoInit entry did not retain a function");
        };
        assert_eq!(first, second);
        assert!(runtime.as_callable(&first).unwrap().is_some());
        assert!(!runtime.is_constructor(&first).unwrap());
    }
}

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
                ObjectPayload::NativeFunction { data }
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

    crate::unicode_case::fail_next_case_reservation_for_test();
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

    crate::value::fail_next_create_html_reservation_for_test();
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
    crate::value::fail_next_create_html_reservation_for_test();
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
fn string_trim_preserves_whitespace_sides_utf16_rope_identity_and_argument_ignorance() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#""\u0009\u000a\u000b\u000c\u000d\u0020\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff".trim()"#,
            )
            .unwrap(),
        Value::String(JsString::from_static("")),
        "the pinned ECMAScript whitespace set did not trim to empty",
    );
    assert_eq!(
        context
            .eval(
                r#"[
                    " \tvalue \n".trim(),
                    " \tvalue \n".trimEnd(),
                    " \tvalue \n".trimRight(),
                    " \tvalue \n".trimStart(),
                    " \tvalue \n".trimLeft(),
                    "\u180ex\u180e".trim()
                ].join("|")"#,
            )
            .unwrap(),
        Value::String(
            JsString::try_from_utf16([
                0x76, 0x61, 0x6c, 0x75, 0x65, 0x7c, 0x20, 0x09, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x7c,
                0x20, 0x09, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x7c, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x20,
                0x0a, 0x7c, 0x76, 0x61, 0x6c, 0x75, 0x65, 0x20, 0x0a, 0x7c, 0x180e, 0x78, 0x180e,
            ])
            .unwrap()
        ),
        "one-sided trims, aliases, or non-whitespace U+180E drifted",
    );
    assert_eq!(
        context
            .eval(r#""\u3000\ud83d\ude00\ud800\u00a0".trim()"#)
            .unwrap(),
        Value::String(JsString::try_from_utf16([0xd83d, 0xde00, 0xd800]).unwrap()),
        "trim decoded or repaired raw UTF-16 code units",
    );

    for (method, expected) in [
        ("trim", "x|r:string;"),
        ("trimEnd", "  x|r:string;"),
        ("trimRight", "  x|r:string;"),
        ("trimStart", "x  |r:string;"),
        ("trimLeft", "x  |r:string;"),
    ] {
        assert_eq!(
            context
                .eval(&format!(
                    r#"(function(){{
                        var log="",receiver=Object(),extra=Object();
                        receiver[Symbol.toPrimitive]=function(hint){{log+="r:"+hint+";";return "  x  "}};
                        extra[Symbol.toPrimitive]=function(){{log+="extra;";throw "wrong"}};
                        return String.prototype.{method}.call(receiver,extra)+"|"+log;
                    }})()"#,
                ))
                .unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap()),
            "{method} read an ignored argument or converted its receiver incorrectly",
        );
    }

    let unchanged = JsString::try_from_utf16([0xd800, 0x20, 0x61, 0xdc00]).unwrap();
    let Completion::Return(Value::String(identity)) = runtime
        .call_string_prototype_trim(
            context.realm,
            StringTrimKind::Both,
            NativeInvocation::Call {
                this_value: Value::String(unchanged.clone()),
            },
        )
        .unwrap()
    else {
        panic!("identity trim did not return a String");
    };
    assert!(
        identity.same_representation(&unchanged),
        "a full-range flat trim did not reuse the original String",
    );

    let left = JsString::try_from_utf16(
        [0x3000]
            .into_iter()
            .chain(std::iter::repeat_n(u16::from(b'a'), 4_999))
            .chain([0xd83d]),
    )
    .unwrap();
    let right = JsString::try_from_utf16(
        [0xde00]
            .into_iter()
            .chain(std::iter::repeat_n(u16::from(b'b'), 4_999))
            .chain([0xfeff]),
    )
    .unwrap();
    let rope = left.try_concat(&right).unwrap();
    assert!(!rope.is_flat());
    let Completion::Return(Value::String(trimmed)) = runtime
        .call_string_prototype_trim(
            context.realm,
            StringTrimKind::Both,
            NativeInvocation::Call {
                this_value: Value::String(rope),
            },
        )
        .unwrap()
    else {
        panic!("rope trim did not return a String");
    };
    assert!(trimmed.is_flat());
    assert_eq!(trimmed.len(), 10_000);
    assert_eq!(trimmed.code_unit_at(0), Some(u16::from(b'a')));
    assert_eq!(trimmed.code_unit_at(4_998), Some(u16::from(b'a')));
    assert_eq!(trimmed.code_unit_at(4_999), Some(0xd83d));
    assert_eq!(trimmed.code_unit_at(5_000), Some(0xde00));
    assert_eq!(trimmed.code_unit_at(5_001), Some(u16::from(b'b')));
    assert_eq!(trimmed.code_unit_at(9_999), Some(u16::from(b'b')));
}

#[test]
fn string_trim_throws_in_defining_realm_preserves_user_throw_and_recovers_from_oom() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let prototype = defining.string_prototype().unwrap();
    let trim_key = runtime.intern_property_key("trim").unwrap();
    let Value::Object(trim_object) = defining.get_property(&prototype, &trim_key).unwrap() else {
        panic!("String.prototype.trim was not an object");
    };
    let trim = runtime.as_callable(&trim_object).unwrap().unwrap();
    let Value::Object(defining_type_error) = defining.eval("TypeError.prototype").unwrap() else {
        panic!("defining TypeError.prototype was not an object");
    };
    let Value::Object(caller_type_error) = caller.eval("TypeError.prototype").unwrap() else {
        panic!("caller TypeError.prototype was not an object");
    };
    let Value::Object(defining_internal_error) = defining.eval("InternalError.prototype").unwrap()
    else {
        panic!("defining InternalError.prototype was not an object");
    };
    let Value::Object(caller_internal_error) = caller.eval("InternalError.prototype").unwrap()
    else {
        panic!("caller InternalError.prototype was not an object");
    };
    assert_ne!(defining_type_error, caller_type_error);
    assert_ne!(defining_internal_error, caller_internal_error);

    assert_eq!(
        caller.call(&trim, Value::Symbol(runtime.new_symbol(None).unwrap()), &[],),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(type_error)) = caller.take_exception().unwrap() else {
        panic!("cross-realm trim conversion did not throw an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&type_error).unwrap(),
        Some(defining_type_error),
        "trim receiver TypeError did not use the function's defining realm",
    );

    caller
        .eval(
            r#"globalThis.trimThrowReceiver=Object();
                trimThrowReceiver[Symbol.toPrimitive]=function(hint){throw 73};
                globalThis.trimReservationLog="";
                globalThis.trimReservationReceiver=Object();
                trimReservationReceiver[Symbol.toPrimitive]=function(hint){
                    trimReservationLog+="receiver:"+hint+";";return "  xy  "
                };"#,
        )
        .unwrap();
    let throwing_receiver = caller.eval("trimThrowReceiver").unwrap();
    assert_eq!(
        caller.call(&trim, throwing_receiver, &[Value::Int(91)]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Int(73)),
        "trim replaced a user receiver-conversion throw",
    );

    crate::value::fail_next_trim_reservation_for_test();
    assert_eq!(
        caller
            .call(
                &trim,
                Value::String(JsString::from_static("identity")),
                &[Value::Int(1)],
            )
            .unwrap(),
        Value::String(JsString::from_static("identity")),
        "the full-range trim fast path failed while the OOM hook was armed",
    );
    assert_eq!(
        caller
            .call(
                &trim,
                Value::String(JsString::from_static("   ")),
                &[Value::Int(2)],
            )
            .unwrap(),
        Value::String(JsString::from_static("")),
        "the empty trim fast path failed while the OOM hook was armed",
    );
    let reservation_receiver = caller.eval("trimReservationReceiver").unwrap();
    assert_eq!(
        caller.call(&trim, reservation_receiver, &[Value::Int(3)]),
        Err(RuntimeError::Exception),
    );
    let Some(Value::Object(oom)) = caller.take_exception().unwrap() else {
        panic!("trim reservation failure did not publish an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&oom).unwrap(),
        Some(defining_internal_error),
        "trim reservation OOM did not use the function's defining realm",
    );
    for (name, expected) in [("name", "InternalError"), ("message", "out of memory")] {
        let Value::String(value) = caller
            .get_property(&oom, &runtime.intern_property_key(name).unwrap())
            .unwrap()
        else {
            panic!("trim reservation OOM {name} was not a String");
        };
        assert_eq!(value, JsString::from_static(expected));
    }
    assert_eq!(
        caller.eval("trimReservationLog").unwrap(),
        Value::String(JsString::from_static("receiver:string;")),
        "trim allocated before its observable receiver conversion",
    );
    assert_eq!(
        caller
            .call(
                &trim,
                Value::String(JsString::from_static("  xy  ")),
                &[Value::Int(4)],
            )
            .unwrap(),
        Value::String(JsString::from_static("xy")),
        "runtime did not recover after trim reservation OOM",
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

            context
                .eval(
                    r#"function mixedStringTrimRecurse(kind,depth){
                        if(kind===0){
                            var receiver=Object();
                            receiver[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringTrimRecurse(1,depth-1);
                                return " x ";
                            };
                            return String.prototype.trim.call(receiver);
                        }
                        if(kind===1){
                            var target=Object();
                            target[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringTrimRecurse(2,depth-1);
                                return 1;
                            };
                            return "x".padEnd(target);
                        }
                        if(kind===2){
                            var count=Object();
                            count[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringTrimRecurse(3,depth-1);
                                return 1;
                            };
                            return "x".repeat(count);
                        }
                        if(kind===3){
                            var start=Object();
                            start[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringTrimRecurse(4,depth-1);
                                return 0;
                            };
                            return "x".slice(start);
                        }
                        var search=Object(),descriptor=Object();
                        descriptor.get=function(){
                            if(depth!==0)mixedStringTrimRecurse(0,depth-1);
                            return false;
                        };
                        Object.defineProperty(search,Symbol.match,descriptor);
                        search[Symbol.toPrimitive]=function(){return "x"};
                        return "x".includes(search)?"x":"wrong";
                    }"#,
                )
                .unwrap();
            for kind in 0..5 {
                assert_eq!(
                    context
                        .eval(&format!("mixedStringTrimRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::from_static("x")),
                    "the proven-safe trim/shared-String chain was rejected for kind {kind}",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringTrimRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "trim alternation bypassed the shared fifth-frame guard for kind {kind}",
                );
            }

            context
                .eval(
                    r#"function mixedStringCreateHtmlRecurse(kind,depth){
                        if(kind===0){
                            var receiver=Object();
                            receiver[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(1,depth-1);
                                return "x";
                            };
                            receiver.anchor=String.prototype.anchor;
                            return receiver.anchor("n");
                        }
                        if(kind===1){
                            var attribute=Object();
                            attribute[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(2,depth-1);
                                return "u";
                            };
                            return "x".link(attribute);
                        }
                        if(kind===2){
                            var receiver=Object();
                            receiver[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(3,depth-1);
                                return "x";
                            };
                            receiver.big=String.prototype.big;
                            return receiver.big();
                        }
                        if(kind===3){
                            var receiver=Object();
                            receiver[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(4,depth-1);
                                return " x ";
                            };
                            receiver.trim=String.prototype.trim;
                            return receiver.trim();
                        }
                        if(kind===4){
                            var target=Object();
                            target[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(5,depth-1);
                                return 1;
                            };
                            return "x".padEnd(target);
                        }
                        if(kind===5){
                            var count=Object();
                            count[Symbol.toPrimitive]=function(){
                                if(depth!==0)mixedStringCreateHtmlRecurse(6,depth-1);
                                return 1;
                            };
                            return "x".repeat(count);
                        }
                        var search=Object(),descriptor=Object();
                        descriptor.get=function(){
                            if(depth!==0)mixedStringCreateHtmlRecurse(0,depth-1);
                            return false;
                        };
                        Object.defineProperty(search,Symbol.match,descriptor);
                        search[Symbol.toPrimitive]=function(){return "x"};
                        return "x".includes(search)?"x":"wrong";
                    }"#,
                )
                .unwrap();
            for (kind, expected) in [
                (0, "<a name=\"n\">x</a>"),
                (1, "<a href=\"u\">x</a>"),
                (2, "<big>x</big>"),
                (3, "x"),
                (4, "x"),
                (5, "x"),
                (6, "x"),
            ] {
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{return mixedStringCreateHtmlRecurse({kind},3)}}
                                catch(error){{return "ERROR:"+error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::try_from_utf8(expected).unwrap()),
                    "the proven-safe CreateHTML/shared-String chain was rejected for kind {kind}",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringCreateHtmlRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "CreateHTML alternation bypassed the shared fifth-frame guard for kind {kind}",
                );
            }

            context
                .eval(
                    r#"function mixedStringCaseRecurse(kind,depth){
                        var receiver=Object();
                        receiver[Symbol.toPrimitive]=function(){
                            if(depth!==0)mixedStringCaseRecurse((kind+1)%2,depth-1);
                            return kind===0?"A":" x ";
                        };
                        if(kind===0){
                            receiver.toLowerCase=String.prototype.toLowerCase;
                            return receiver.toLowerCase();
                        }
                        receiver.trim=String.prototype.trim;
                        return receiver.trim();
                    }"#,
                )
                .unwrap();
            for (kind, expected) in [(0, "a"), (1, "x")] {
                assert_eq!(
                    context
                        .eval(&format!("mixedStringCaseRecurse({kind},3)"))
                        .unwrap(),
                    Value::String(JsString::from_static(expected)),
                    "the proven-safe case/trim chain was rejected for kind {kind}",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedStringCaseRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "case/trim alternation bypassed the shared fifth-frame guard for kind {kind}",
                );
            }
            assert_eq!(
                context
                    .eval(
                        r#""abc".includes("b")+"|"+"abc".slice(1)+"|"+
                           "ab".repeat(2)+"|"+"a".padEnd(3,"x")+"|"+"a".padStart(3,"x")+"|"+
                           " z ".trim()+"|"+"z".bold()+"|"+"AbΣ".toLocaleLowerCase()"#,
                    )
                    .unwrap(),
                Value::String(JsString::from_static("true|bc|abab|axx|xxa|z|<b>z</b>|abς",)),
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

#[test]
fn mixed_string_and_regexp_search_recursion_is_guarded_and_runtime_recovers() {
    std::thread::Builder::new()
        .name("string-regexp-search-stack-proof".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context
                .eval(
                    r#"function mixedSearchRecurse(kind,depth){
                        if(kind===0){
                            var pattern=Object();
                            pattern[Symbol.search]=function(){
                                if(depth!==0)return mixedSearchRecurse(1,depth-1);
                                return 0;
                            };
                            return "x".search(pattern);
                        }
                        var regexp=Object();
                        regexp.lastIndex=0;
                        regexp.exec=function(){
                            if(depth!==0)mixedSearchRecurse(0,depth-1);
                            return {index:0};
                        };
                        return RegExp.prototype[Symbol.search].call(regexp,"x");
                    }"#,
                )
                .unwrap();

            for (entry, kind) in [("String.prototype.search", 0), ("RegExp @@search", 1)] {
                assert_eq!(
                    context
                        .eval(&format!("mixedSearchRecurse({kind},3)"))
                        .unwrap(),
                    Value::Int(0),
                    "the proven-safe four-frame {entry} chain was rejected",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedSearchRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    Value::String(JsString::from_static("InternalError:stack overflow")),
                    "the fifth mixed search frame was not rejected from {entry}",
                );
            }
            assert_eq!(
                context
                    .eval(
                        r#""abc".search("b")+"|"+
                           RegExp.prototype[Symbol.search].call({
                               lastIndex:0,exec:function(){return null}
                           },"x")"#,
                    )
                    .unwrap(),
                Value::String(JsString::from_static("1|-1")),
                "the runtime did not recover after mixed search overflow",
            );
        })
        .expect("2 MiB String/RegExp search stack-proof thread did not start")
        .join()
        .expect("2 MiB String/RegExp search stack-proof thread panicked");
}
