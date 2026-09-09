use crate::engine::heap::native::{
    ArrayFindKind, ArrayFlattenKind, ArrayIterationKind, ArrayJoinKind, ArrayPopKind,
    ArrayPushKind, ArrayReduceKind, ArraySearchKind, ArraySliceKind, BigIntAsNKind,
    DateGetFieldKind, DateNativeKind, DateSetFieldKind, DateStringMethod,
    GlobalNumberPredicateKind, GlobalUriCodecKind, MathBinaryKind, MathMinMaxKind, MathUnaryKind,
    NativeCProto, NumberFormatKind, NumberParseKind, NumberPredicateKind, ObjectAccessorKind,
    ObjectExtensibilityKind, ObjectIntegrityKind, ObjectKeysKind, ObjectOwnPropertyKeysKind,
    RegExpFlagKind, StringCharAtKind, StringPadKind, StringReplaceKind, StringStaticKind,
    StringSubrangeKind, StringTrimKind, StringWellFormedKind, SymbolRegistryKind,
};

use super::*;

#[test]
fn numeric_and_uri_native_selectors_use_pinned_cproto() {
    let targets = [
        NativeFunctionId::GlobalEval,
        NativeFunctionId::GlobalNumberParse(NumberParseKind::ParseInt),
        NativeFunctionId::GlobalNumberParse(NumberParseKind::ParseFloat),
        NativeFunctionId::GlobalNumberPredicate(GlobalNumberPredicateKind::IsNaN),
        NativeFunctionId::GlobalNumberPredicate(GlobalNumberPredicateKind::IsFinite),
        NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::Escape),
        NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::Unescape),
        NativeFunctionId::NumberPredicate(NumberPredicateKind::IsNaN),
        NativeFunctionId::NumberPredicate(NumberPredicateKind::IsFinite),
        NativeFunctionId::NumberPredicate(NumberPredicateKind::IsInteger),
        NativeFunctionId::NumberPredicate(NumberPredicateKind::IsSafeInteger),
        NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::Exponential),
        NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::Fixed),
        NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::Precision),
        NativeFunctionId::NumberPrototypeFormat(NumberFormatKind::LocaleString),
        NativeFunctionId::SymbolRegistry(SymbolRegistryKind::For),
        NativeFunctionId::SymbolRegistry(SymbolRegistryKind::KeyFor),
        NativeFunctionId::StringPrototypeCharCodeAt,
        NativeFunctionId::StringPrototypeConcat,
        NativeFunctionId::StringPrototypeCodePointAt,
        NativeFunctionId::StringPrototypeWellFormed(StringWellFormedKind::IsWellFormed),
        NativeFunctionId::StringPrototypeWellFormed(StringWellFormedKind::ToWellFormed),
        NativeFunctionId::StringPrototypeSplit,
        NativeFunctionId::StringPrototypeSubrange(StringSubrangeKind::Substring),
        NativeFunctionId::StringPrototypeSubrange(StringSubrangeKind::Substr),
        NativeFunctionId::StringPrototypeSubrange(StringSubrangeKind::Slice),
        NativeFunctionId::StringPrototypeRepeat,
        NativeFunctionId::StringPrototypeNormalize,
        NativeFunctionId::StringPrototypeLocaleCompare,
        NativeFunctionId::MathHypot,
        NativeFunctionId::MathRandom,
        NativeFunctionId::MathImul,
        NativeFunctionId::MathClz32,
        NativeFunctionId::MathSumPrecise,
    ];

    for target in targets {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
    #[cfg(feature = "test262-host")]
    {
        let target = NativeFunctionId::StringCodePointRange;
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
    for target in [
        NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::DecodeUri),
        NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::DecodeUriComponent),
        NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::EncodeUri),
        NativeFunctionId::GlobalUriCodec(GlobalUriCodecKind::EncodeUriComponent),
        NativeFunctionId::BigIntAsN(BigIntAsNKind::AsUintN),
        NativeFunctionId::BigIntAsN(BigIntAsNKind::AsIntN),
        NativeFunctionId::StringPrototypeCharAt(StringCharAtKind::At),
        NativeFunctionId::StringPrototypeCharAt(StringCharAtKind::CharAt),
        NativeFunctionId::StringPrototypePad(StringPadKind::End),
        NativeFunctionId::StringPrototypePad(StringPadKind::Start),
        NativeFunctionId::StringPrototypeReplace(StringReplaceKind::Replace),
        NativeFunctionId::StringPrototypeReplace(StringReplaceKind::ReplaceAll),
        NativeFunctionId::StringPrototypeMatch,
        NativeFunctionId::StringPrototypeMatchAll,
        NativeFunctionId::StringPrototypeSearch,
        NativeFunctionId::StringPrototypeTrim(StringTrimKind::Both),
        NativeFunctionId::StringPrototypeTrim(StringTrimKind::End),
        NativeFunctionId::StringPrototypeTrim(StringTrimKind::Start),
        NativeFunctionId::MathMinMax(MathMinMaxKind::Min),
        NativeFunctionId::MathMinMax(MathMinMaxKind::Max),
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
    assert_eq!(
        NativeFunctionId::SymbolPrototypeDescription
            .descriptor()
            .cproto,
        NativeCProto::Getter
    );
    assert_eq!(
        NativeFunctionId::StringIteratorNext.descriptor().cproto,
        NativeCProto::IteratorNext
    );
    assert_eq!(
        NativeFunctionId::RegExpStringIteratorNext
            .descriptor()
            .cproto,
        NativeCProto::IteratorNext
    );
    assert!(!NativeCProto::IteratorNext.default_is_constructor());
    assert_eq!(
        NativeFunctionId::MathUnary(MathUnaryKind::Round)
            .descriptor()
            .cproto,
        NativeCProto::UnaryF64
    );
    assert_eq!(
        NativeFunctionId::MathBinary(MathBinaryKind::Pow)
            .descriptor()
            .cproto,
        NativeCProto::BinaryF64
    );
    assert!(!NativeCProto::UnaryF64.default_is_constructor());
    assert!(!NativeCProto::BinaryF64.default_is_constructor());
}

#[test]
fn date_native_selectors_preserve_pinned_descriptors_and_magic() {
    let constructor = NativeFunctionId::Date(DateNativeKind::Constructor);
    assert_eq!(
        constructor.descriptor().cproto,
        NativeCProto::ConstructorOrFunction
    );
    assert!(constructor.descriptor().cproto.default_is_constructor());
    assert_eq!(DateNativeKind::Constructor.unique_name(), Some("Date"));
    assert_eq!(DateNativeKind::Constructor.length(), 7);

    for kind in [
        DateNativeKind::Now,
        DateNativeKind::Parse,
        DateNativeKind::Utc,
        DateNativeKind::TimeValue,
        DateNativeKind::ToPrimitive,
        DateNativeKind::TimezoneOffset,
        DateNativeKind::SetTime,
        DateNativeKind::SetYear,
        DateNativeKind::ToJson,
    ] {
        let target = NativeFunctionId::Date(kind);
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
    assert_eq!(DateNativeKind::Now.unique_name(), Some("now"));
    assert_eq!(DateNativeKind::Now.length(), 0);
    assert_eq!(DateNativeKind::Parse.unique_name(), Some("parse"));
    assert_eq!(DateNativeKind::Parse.length(), 1);
    assert_eq!(DateNativeKind::Utc.unique_name(), Some("UTC"));
    assert_eq!(DateNativeKind::Utc.length(), 7);
    assert_eq!(DateNativeKind::TimeValue.unique_name(), None);
    assert_eq!(DateNativeKind::TimeValue.length(), 0);
    assert_eq!(
        DateNativeKind::ToPrimitive.unique_name(),
        Some("[Symbol.toPrimitive]")
    );
    assert_eq!(DateNativeKind::ToPrimitive.length(), 1);
    assert_eq!(
        DateNativeKind::TimezoneOffset.unique_name(),
        Some("getTimezoneOffset")
    );
    assert_eq!(DateNativeKind::TimezoneOffset.length(), 0);
    assert_eq!(DateNativeKind::SetTime.unique_name(), Some("setTime"));
    assert_eq!(DateNativeKind::SetTime.length(), 1);
    assert_eq!(DateNativeKind::SetYear.unique_name(), Some("setYear"));
    assert_eq!(DateNativeKind::SetYear.length(), 1);
    assert_eq!(DateNativeKind::ToJson.unique_name(), Some("toJSON"));
    assert_eq!(DateNativeKind::ToJson.length(), 1);

    let string_metadata = [
        (DateStringMethod::String, "toString", 0x13, true),
        (DateStringMethod::UtcString, "toUTCString", 0x03, false),
        (DateStringMethod::IsoString, "toISOString", 0x23, false),
        (DateStringMethod::DateString, "toDateString", 0x11, true),
        (DateStringMethod::TimeString, "toTimeString", 0x12, true),
        (DateStringMethod::LocaleString, "toLocaleString", 0x33, true),
        (
            DateStringMethod::LocaleDateString,
            "toLocaleDateString",
            0x31,
            true,
        ),
        (
            DateStringMethod::LocaleTimeString,
            "toLocaleTimeString",
            0x32,
            true,
        ),
    ];
    assert_eq!(
        string_metadata.map(|(kind, ..)| kind),
        DateStringMethod::ALL
    );
    for (kind, name, magic, uses_local_time) in string_metadata {
        let native = DateNativeKind::String(kind);
        assert_eq!(native.unique_name(), Some(name));
        assert_eq!(native.length(), 0);
        assert_eq!(kind.quickjs_magic(), magic);
        assert_eq!(kind.uses_local_time(), uses_local_time);
        assert_eq!(
            NativeFunctionId::Date(native).descriptor().cproto,
            NativeCProto::GenericMagic
        );
    }

    let get_metadata = [
        (DateGetFieldKind::Year, "getYear", 0x101),
        (DateGetFieldKind::FullYear, "getFullYear", 0x01),
        (DateGetFieldKind::UtcFullYear, "getUTCFullYear", 0x00),
        (DateGetFieldKind::Month, "getMonth", 0x11),
        (DateGetFieldKind::UtcMonth, "getUTCMonth", 0x10),
        (DateGetFieldKind::Date, "getDate", 0x21),
        (DateGetFieldKind::UtcDate, "getUTCDate", 0x20),
        (DateGetFieldKind::Hours, "getHours", 0x31),
        (DateGetFieldKind::UtcHours, "getUTCHours", 0x30),
        (DateGetFieldKind::Minutes, "getMinutes", 0x41),
        (DateGetFieldKind::UtcMinutes, "getUTCMinutes", 0x40),
        (DateGetFieldKind::Seconds, "getSeconds", 0x51),
        (DateGetFieldKind::UtcSeconds, "getUTCSeconds", 0x50),
        (DateGetFieldKind::Milliseconds, "getMilliseconds", 0x61),
        (
            DateGetFieldKind::UtcMilliseconds,
            "getUTCMilliseconds",
            0x60,
        ),
        (DateGetFieldKind::Day, "getDay", 0x71),
        (DateGetFieldKind::UtcDay, "getUTCDay", 0x70),
    ];
    assert_eq!(get_metadata.map(|(kind, ..)| kind), DateGetFieldKind::ALL);
    for (kind, name, magic) in get_metadata {
        let native = DateNativeKind::GetField(kind);
        assert_eq!(native.unique_name(), Some(name));
        assert_eq!(native.length(), 0);
        assert_eq!(kind.quickjs_magic(), magic);
        assert_eq!(
            NativeFunctionId::Date(native).descriptor().cproto,
            NativeCProto::GenericMagic
        );
    }

    let set_metadata = [
        (DateSetFieldKind::Milliseconds, "setMilliseconds", 1, 0x671),
        (
            DateSetFieldKind::UtcMilliseconds,
            "setUTCMilliseconds",
            1,
            0x670,
        ),
        (DateSetFieldKind::Seconds, "setSeconds", 2, 0x571),
        (DateSetFieldKind::UtcSeconds, "setUTCSeconds", 2, 0x570),
        (DateSetFieldKind::Minutes, "setMinutes", 3, 0x471),
        (DateSetFieldKind::UtcMinutes, "setUTCMinutes", 3, 0x470),
        (DateSetFieldKind::Hours, "setHours", 4, 0x371),
        (DateSetFieldKind::UtcHours, "setUTCHours", 4, 0x370),
        (DateSetFieldKind::Date, "setDate", 1, 0x231),
        (DateSetFieldKind::UtcDate, "setUTCDate", 1, 0x230),
        (DateSetFieldKind::Month, "setMonth", 2, 0x131),
        (DateSetFieldKind::UtcMonth, "setUTCMonth", 2, 0x130),
        (DateSetFieldKind::FullYear, "setFullYear", 3, 0x031),
        (DateSetFieldKind::UtcFullYear, "setUTCFullYear", 3, 0x030),
    ];
    assert_eq!(set_metadata.map(|(kind, ..)| kind), DateSetFieldKind::ALL);
    for (kind, name, length, magic) in set_metadata {
        let native = DateNativeKind::SetField(kind);
        assert_eq!(native.unique_name(), Some(name));
        assert_eq!(native.length(), length);
        assert_eq!(kind.length(), length);
        assert_eq!(kind.quickjs_magic(), magic);
        assert_eq!(
            NativeFunctionId::Date(native).descriptor().cproto,
            NativeCProto::GenericMagic
        );
    }
}

#[test]
fn date_native_payload_owns_only_its_shape_and_defining_realm_edges() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    let realm = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    let function_shape = heap
        .allocate_shape(Shape::new(Some(prototype), []).unwrap())
        .unwrap();
    let target = NativeFunctionId::Date(DateNativeKind::Constructor);
    let function = heap
        .allocate_object(ObjectData::bound_native_function(
            function_shape,
            Vec::new(),
            target,
            realm,
            1,
        ))
        .unwrap();

    let function_data = heap.object(function).unwrap();
    assert!(function_data.is_constructor);
    assert!(matches!(
        function_data.payload,
        ObjectPayload::NativeFunction {
            data: NativeFunctionData {
                target: stored_target,
                realm: Some(stored_realm),
                min_readable_args: 1,
            },
            ..
        } if stored_target == target && stored_realm == realm
    ));
    assert_eq!(
        object_edges(function_data),
        vec![RawId::Shape(function_shape), RawId::Context(realm)]
    );
    assert_eq!(heap.context_strong_count(realm), Ok(2));

    heap.release_context(realm).unwrap();
    assert_eq!(heap.context_strong_count(realm), Ok(1));
    let cleanup = heap.release_object(function).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_contexts, 1);
    heap.release_shape(function_shape).unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn string_static_native_selectors_use_pinned_cproto() {
    for target in [
        NativeFunctionId::StringStatic(StringStaticKind::FromCharCode),
        NativeFunctionId::StringStatic(StringStaticKind::FromCodePoint),
        NativeFunctionId::StringStatic(StringStaticKind::Raw),
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn array_search_native_selectors_use_pinned_cproto() {
    for target in [
        NativeFunctionId::ArrayPrototypeAt,
        NativeFunctionId::ArrayPrototypeSearch(ArraySearchKind::Includes),
        NativeFunctionId::ArrayPrototypeSearch(ArraySearchKind::IndexOf),
        NativeFunctionId::ArrayPrototypeSearch(ArraySearchKind::LastIndexOf),
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn array_with_native_selector_uses_pinned_cproto() {
    let target = NativeFunctionId::ArrayPrototypeWith;
    assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
    assert!(!target.descriptor().cproto.default_is_constructor());
}

#[test]
fn array_concat_native_selector_uses_pinned_cproto() {
    let target = NativeFunctionId::ArrayPrototypeConcat;
    assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
    assert!(!target.descriptor().cproto.default_is_constructor());
}

#[test]
fn array_stringification_native_selectors_use_pinned_cproto() {
    for kind in [ArrayJoinKind::Join, ArrayJoinKind::ToLocaleString] {
        let target = NativeFunctionId::ArrayPrototypeJoin(kind);
        assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
    let target = NativeFunctionId::ArrayPrototypeToString;
    assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
    assert!(!target.descriptor().cproto.default_is_constructor());
}

#[test]
fn array_pop_push_native_selectors_use_pinned_cproto() {
    for target in [
        NativeFunctionId::ArrayPrototypePop(ArrayPopKind::Pop),
        NativeFunctionId::ArrayPrototypePop(ArrayPopKind::Shift),
        NativeFunctionId::ArrayPrototypePush(ArrayPushKind::Push),
        NativeFunctionId::ArrayPrototypePush(ArrayPushKind::Unshift),
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn array_reverse_native_targets_use_pinned_cproto() {
    for target in [
        NativeFunctionId::ArrayPrototypeReverse,
        NativeFunctionId::ArrayPrototypeToReversed,
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn array_sort_native_targets_use_pinned_cproto() {
    for target in [
        NativeFunctionId::ArrayPrototypeSort,
        NativeFunctionId::ArrayPrototypeToSorted,
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn array_slice_native_targets_use_pinned_cproto() {
    for kind in [ArraySliceKind::Slice, ArraySliceKind::Splice] {
        let target = NativeFunctionId::ArrayPrototypeSlice(kind);
        assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
    let target = NativeFunctionId::ArrayPrototypeToSpliced;
    assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
    assert!(!target.descriptor().cproto.default_is_constructor());
}

#[test]
fn array_fill_native_selector_uses_pinned_cproto() {
    let target = NativeFunctionId::ArrayPrototypeFill;
    assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
    assert!(!target.descriptor().cproto.default_is_constructor());
}

#[test]
fn array_find_native_selectors_use_pinned_cproto() {
    for kind in [
        ArrayFindKind::Find,
        ArrayFindKind::FindIndex,
        ArrayFindKind::FindLast,
        ArrayFindKind::FindLastIndex,
    ] {
        let target = NativeFunctionId::ArrayPrototypeFind(kind);
        assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn array_iteration_native_selectors_use_pinned_cproto() {
    for kind in [
        ArrayIterationKind::Every,
        ArrayIterationKind::Some,
        ArrayIterationKind::ForEach,
        ArrayIterationKind::Map,
        ArrayIterationKind::Filter,
    ] {
        let target = NativeFunctionId::ArrayPrototypeIteration(kind);
        assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn array_reduce_native_selectors_use_pinned_cproto() {
    for kind in [ArrayReduceKind::Reduce, ArrayReduceKind::ReduceRight] {
        let target = NativeFunctionId::ArrayPrototypeReduce(kind);
        assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn array_copy_within_native_selector_uses_pinned_cproto() {
    let target = NativeFunctionId::ArrayPrototypeCopyWithin;
    assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
    assert!(!target.descriptor().cproto.default_is_constructor());
}

#[test]
fn array_flatten_native_selectors_use_pinned_cproto() {
    for kind in [ArrayFlattenKind::FlatMap, ArrayFlattenKind::Flat] {
        let target = NativeFunctionId::ArrayPrototypeFlatten(kind);
        assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn object_native_selectors_use_pinned_cproto() {
    for target in [
        NativeFunctionId::ObjectCreate,
        NativeFunctionId::ObjectSetPrototypeOf,
        NativeFunctionId::ObjectDefineProperties,
        NativeFunctionId::ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind::Names),
        NativeFunctionId::ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind::Symbols),
        NativeFunctionId::ObjectGetOwnPropertyDescriptors,
        NativeFunctionId::ObjectIs,
        NativeFunctionId::ObjectAssign,
        NativeFunctionId::ObjectFromEntries,
        NativeFunctionId::ObjectHasOwn,
        NativeFunctionId::ObjectPrototypeHasOwnProperty,
        NativeFunctionId::ObjectPrototypeIsPrototypeOf,
        NativeFunctionId::ObjectPrototypePropertyIsEnumerable,
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }

    for target in [
        NativeFunctionId::ObjectGetPrototypeOf,
        NativeFunctionId::ObjectDefineProperty,
        NativeFunctionId::ObjectGroupBy,
        NativeFunctionId::ObjectKeys(ObjectKeysKind::Keys),
        NativeFunctionId::ObjectKeys(ObjectKeysKind::Values),
        NativeFunctionId::ObjectKeys(ObjectKeysKind::Entries),
        NativeFunctionId::ObjectExtensibility(ObjectExtensibilityKind::IsExtensible),
        NativeFunctionId::ObjectExtensibility(ObjectExtensibilityKind::PreventExtensions),
        NativeFunctionId::ObjectGetOwnPropertyDescriptor,
        NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::Seal),
        NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::Freeze),
        NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::IsSealed),
        NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::IsFrozen),
        NativeFunctionId::ObjectPrototypeDefineAccessor(ObjectAccessorKind::Getter),
        NativeFunctionId::ObjectPrototypeDefineAccessor(ObjectAccessorKind::Setter),
        NativeFunctionId::ObjectPrototypeLookupAccessor(ObjectAccessorKind::Getter),
        NativeFunctionId::ObjectPrototypeLookupAccessor(ObjectAccessorKind::Setter),
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::GenericMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }

    assert_eq!(
        NativeFunctionId::ObjectConstructor.descriptor().cproto,
        NativeCProto::ConstructorOrFunction
    );
    assert!(
        NativeFunctionId::ObjectConstructor
            .descriptor()
            .cproto
            .default_is_constructor()
    );
    assert_eq!(
        NativeFunctionId::ObjectPrototypeProtoGetter
            .descriptor()
            .cproto,
        NativeCProto::Getter
    );
    assert_eq!(
        NativeFunctionId::ObjectPrototypeProtoSetter
            .descriptor()
            .cproto,
        NativeCProto::Setter
    );
}

#[test]
fn regexp_native_selectors_use_pinned_cproto() {
    let constructor = NativeFunctionId::RegExp(RegExpNativeKind::Constructor);
    assert_eq!(
        constructor.descriptor().cproto,
        NativeCProto::ConstructorOrFunction
    );
    assert!(constructor.descriptor().cproto.default_is_constructor());

    for target in [
        NativeFunctionId::RegExp(RegExpNativeKind::Escape),
        NativeFunctionId::RegExp(RegExpNativeKind::Exec),
        NativeFunctionId::RegExp(RegExpNativeKind::Compile),
        NativeFunctionId::RegExp(RegExpNativeKind::Test),
        NativeFunctionId::RegExp(RegExpNativeKind::ToString),
        NativeFunctionId::RegExp(RegExpNativeKind::Replace),
        NativeFunctionId::RegExp(RegExpNativeKind::Match),
        NativeFunctionId::RegExp(RegExpNativeKind::MatchAll),
        NativeFunctionId::RegExp(RegExpNativeKind::Search),
        NativeFunctionId::RegExp(RegExpNativeKind::Split),
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
    for target in [
        NativeFunctionId::RegExp(RegExpNativeKind::Species),
        NativeFunctionId::RegExp(RegExpNativeKind::Source),
        NativeFunctionId::RegExp(RegExpNativeKind::Flags),
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Getter);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
    for flag in [
        RegExpFlagKind::HasIndices,
        RegExpFlagKind::Global,
        RegExpFlagKind::IgnoreCase,
        RegExpFlagKind::Multiline,
        RegExpFlagKind::DotAll,
        RegExpFlagKind::Unicode,
        RegExpFlagKind::UnicodeSets,
        RegExpFlagKind::Sticky,
    ] {
        let target = NativeFunctionId::RegExp(RegExpNativeKind::Flag(flag));
        assert_eq!(target.descriptor().cproto, NativeCProto::GetterMagic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}
