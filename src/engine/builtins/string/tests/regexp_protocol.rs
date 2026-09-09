use crate::engine::builtins::native::{NativeCProto, RegExpNativeKind};
use crate::engine::heap::{AutoInitProperty, PropertySlot};
use crate::engine::object::shape::PropertyFlags;

use super::*;

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
