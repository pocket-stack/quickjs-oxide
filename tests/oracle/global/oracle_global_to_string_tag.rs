use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField, JsString,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, Value, WellKnownSymbol,
};

const COMMON_GLOBALS: &[&str] = &[
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "decodeURI",
    "decodeURIComponent",
    "encodeURI",
    "encodeURIComponent",
    "escape",
    "unescape",
    "Infinity",
    "NaN",
    "undefined",
    "Number",
    "Boolean",
];

const ORACLE_PROBE: &str = r#"
function bits(descriptor) {
    return (descriptor.writable ? "1" : "0") +
           (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0");
}

var tag = Symbol.toStringTag;
var objectToString = Object.prototype.toString;
var initial = Object.getOwnPropertyDescriptor(globalThis, tag);
print("initial=" + [
    globalThis[tag], bits(initial),
    Object.prototype.hasOwnProperty.call(globalThis, tag),
    globalThis.propertyIsEnumerable(tag),
    objectToString.call(globalThis)
].join("|"));

print("readonly=" + [
    Reflect.set(globalThis, tag, "assigned-global"),
    globalThis[tag], objectToString.call(globalThis)
].join("|"));

Object.defineProperty(globalThis, tag, { value: "direct-global" });
var direct = Object.getOwnPropertyDescriptor(globalThis, tag);
print("direct-redefine=" + [
    globalThis[tag], bits(direct), globalThis.propertyIsEnumerable(tag),
    objectToString.call(globalThis)
].join("|"));
Object.defineProperty(globalThis, tag, { value: "global" });

var marker = "__quickjs_oxide_global_tag_last_string__";
Object.defineProperty(globalThis, marker, {
    value: 1, writable: true, enumerable: true, configurable: true
});
var extra = Symbol("extra-global-key");
Object.defineProperty(globalThis, extra, {
    value: 1, writable: true, enumerable: true, configurable: true
});
var keys = Reflect.ownKeys(globalThis);
var common = [
    "parseInt", "parseFloat", "isNaN", "isFinite",
    "decodeURI", "decodeURIComponent", "encodeURI", "encodeURIComponent",
    "escape", "unescape", "Infinity", "NaN", "undefined", "Number", "Boolean"
];
var selected = keys.filter(function(key) {
    return key === tag || key === extra ||
           (typeof key === "string" && common.indexOf(key) >= 0);
}).map(function(key) {
    if (key === tag) return "symbol:toStringTag";
    if (key === extra) return "symbol:extra";
    return "string:" + key;
});
var symbols = Object.getOwnPropertySymbols(globalThis);
var firstSymbol = keys.findIndex(function(key) { return typeof key === "symbol"; });
print("keys=" + [
    selected.join(","),
    keys.slice(0, firstSymbol).every(function(key) { return typeof key === "string"; }),
    keys.slice(firstSymbol).every(function(key) { return typeof key === "symbol"; }),
    symbols.length, symbols[0] === tag, symbols[1] === extra
].join("|"));
delete globalThis[extra];
delete globalThis[marker];

var deleted = delete globalThis[tag];
print("delete=" + [
    deleted,
    Object.getOwnPropertyDescriptor(globalThis, tag) === undefined,
    typeof globalThis[tag],
    Object.getOwnPropertySymbols(globalThis).indexOf(tag) < 0,
    objectToString.call(globalThis)
].join("|"));

Object.defineProperty(globalThis, tag, {
    value: "custom-global", writable: true, enumerable: true, configurable: true
});
print("redefine=" + [
    globalThis[tag],
    bits(Object.getOwnPropertyDescriptor(globalThis, tag)),
    globalThis.propertyIsEnumerable(tag),
    Object.getOwnPropertySymbols(globalThis).indexOf(tag) >= 0,
    objectToString.call(globalThis)
].join("|"));

var nonStrings = [undefined, null, false, 0, {}, Symbol("not-a-tag")];
print("non-string=" + nonStrings.map(function(value) {
    globalThis[tag] = value;
    return objectToString.call(globalThis);
}).join("|"));
globalThis[tag] = "restored-global";
print("restored=" + objectToString.call(globalThis));
"#;

const EXPECTED_OBSERVATIONS: &[&str] = &[
    "initial=global|001|true|false|[object global]",
    "readonly=false|global|[object global]",
    "direct-redefine=direct-global|001|false|[object direct-global]",
    "keys=string:parseInt,string:parseFloat,string:isNaN,string:isFinite,string:decodeURI,string:decodeURIComponent,string:encodeURI,string:encodeURIComponent,string:escape,string:unescape,string:Infinity,string:NaN,string:undefined,string:Number,string:Boolean,symbol:toStringTag,symbol:extra|true|true|2|true|true",
    "delete=true|true|undefined|true|[object Object]",
    "redefine=custom-global|111|true|true|[object custom-global]",
    "non-string=[object Object]|[object Object]|[object Object]|[object Object]|[object Object]|[object Object]",
    "restored=[object restored-global]",
];

#[test]
fn global_to_string_tag_matches_pinned_quickjs() {
    let rust = rust_observations();
    assert_eq!(
        rust, EXPECTED_OBSERVATIONS,
        "host-side global @@toStringTag contract changed"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP global @@toStringTag differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        oracle_observations(&oracle),
        EXPECTED_OBSERVATIONS,
        "the pinned QuickJS global @@toStringTag contract drifted"
    );
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "global @@toStringTag behavior differed from pinned QuickJS"
    );
}

#[test]
fn object_to_string_cross_realm_uses_receiver_tag_and_method_defining_realm() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_global = first.global_object().unwrap();
    let second_global = second.global_object().unwrap();
    let first_object_prototype = first.object_prototype().unwrap();
    let second_object_prototype = second.object_prototype().unwrap();
    let first_to_string =
        property_callable(&runtime, &mut first, &first_object_prototype, "toString");
    let second_to_string =
        property_callable(&runtime, &mut second, &second_object_prototype, "toString");
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));

    define_data_key(
        &mut first,
        &first_global,
        &tag,
        Value::String(JsString::try_from_utf8("first-global").unwrap()),
        false,
        false,
        true,
    );
    define_data_key(
        &mut second,
        &second_global,
        &tag,
        Value::String(JsString::try_from_utf8("second-global").unwrap()),
        false,
        false,
        true,
    );
    assert_eq!(
        second
            .call(&first_to_string, Value::Object(second_global.clone()), &[],)
            .unwrap(),
        Value::String(JsString::try_from_utf8("[object second-global]").unwrap()),
        "a foreign native method must read the receiver global's own tag"
    );
    assert_eq!(
        first
            .call(&second_to_string, Value::Object(first_global.clone()), &[],)
            .unwrap(),
        Value::String(JsString::try_from_utf8("[object first-global]").unwrap())
    );

    let first_number_prototype = first.number_prototype().unwrap();
    let second_number_prototype = second.number_prototype().unwrap();
    define_data_key(
        &mut first,
        &first_number_prototype,
        &tag,
        Value::String(JsString::try_from_utf8("FirstNumber").unwrap()),
        false,
        false,
        true,
    );
    define_data_key(
        &mut second,
        &second_number_prototype,
        &tag,
        Value::String(JsString::try_from_utf8("SecondNumber").unwrap()),
        false,
        false,
        true,
    );
    assert_eq!(
        second.call(&first_to_string, Value::Int(1), &[]).unwrap(),
        Value::String(JsString::try_from_utf8("[object FirstNumber]").unwrap()),
        "primitive boxing must use Object.prototype.toString's defining realm"
    );
    assert_eq!(
        first.call(&second_to_string, Value::Int(1), &[]).unwrap(),
        Value::String(JsString::try_from_utf8("[object SecondNumber]").unwrap())
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));

    let initial = runtime.get_own_property(&global, &tag).unwrap().unwrap();
    let mut observations = vec![format!(
        "initial={}|{}|{}|{}|{}",
        plain_value(context.get_property(&global, &tag).unwrap()),
        descriptor_bits(&initial),
        runtime.has_own_property(&global, &tag).unwrap(),
        descriptor_enumerable(&initial),
        call_to_string(&mut context, &object_to_string, &global),
    )];

    let assigned = context
        .set_property(
            &global,
            &tag,
            Value::String(JsString::try_from_utf8("assigned-global").unwrap()),
        )
        .unwrap();
    observations.push(format!(
        "readonly={assigned}|{}|{}",
        plain_value(context.get_property(&global, &tag).unwrap()),
        call_to_string(&mut context, &object_to_string, &global),
    ));

    define_value_key(
        &mut context,
        &global,
        &tag,
        Value::String(JsString::try_from_utf8("direct-global").unwrap()),
    );
    let direct = runtime.get_own_property(&global, &tag).unwrap().unwrap();
    observations.push(format!(
        "direct-redefine={}|{}|{}|{}",
        plain_value(context.get_property(&global, &tag).unwrap()),
        descriptor_bits(&direct),
        descriptor_enumerable(&direct),
        call_to_string(&mut context, &object_to_string, &global),
    ));
    define_value_key(
        &mut context,
        &global,
        &tag,
        Value::String(JsString::try_from_utf8("global").unwrap()),
    );

    let marker = runtime
        .intern_property_key("__quickjs_oxide_global_tag_last_string__")
        .unwrap();
    define_data_key(
        &mut context,
        &global,
        &marker,
        Value::Int(1),
        true,
        true,
        true,
    );
    let extra = PropertyKey::from(
        runtime
            .new_symbol(Some(JsString::try_from_utf8("extra-global-key").unwrap()))
            .unwrap(),
    );
    define_data_key(
        &mut context,
        &global,
        &extra,
        Value::Int(1),
        true,
        true,
        true,
    );
    let keys = runtime.own_property_keys(&global).unwrap();
    let selected = keys
        .iter()
        .filter_map(|key| {
            if key == &tag {
                return Some("symbol:toStringTag".to_owned());
            }
            if key == &extra {
                return Some("symbol:extra".to_owned());
            }
            let name = runtime
                .property_key_to_js_string(key)
                .unwrap()
                .to_utf8_lossy();
            COMMON_GLOBALS
                .contains(&name.as_str())
                .then(|| format!("string:{name}"))
        })
        .collect::<Vec<_>>()
        .join(",");
    let marker_index = keys
        .iter()
        .position(|key| key == &marker)
        .expect("the last inserted string key must be observable");
    let tag_index = keys
        .iter()
        .position(|key| key == &tag)
        .expect("the global tag key must be observable");
    let symbols = &keys[tag_index..];
    let strings_precede_symbols = marker_index.checked_add(1) == Some(tag_index);
    let symbols_are_exact_tail = symbols == [tag.clone(), extra.clone()];
    observations.push(format!(
        "keys={selected}|{strings_precede_symbols}|{symbols_are_exact_tail}|{}|{}|{}",
        symbols.len(),
        symbols.first() == Some(&tag),
        symbols.get(1) == Some(&extra),
    ));
    assert!(runtime.delete_property(&global, &extra).unwrap());
    assert!(runtime.delete_property(&global, &marker).unwrap());

    let deleted = runtime.delete_property(&global, &tag).unwrap();
    observations.push(format!(
        "delete={deleted}|{}|{}|{}|{}",
        runtime.get_own_property(&global, &tag).unwrap().is_none(),
        plain_value(context.get_property(&global, &tag).unwrap()),
        !runtime.own_property_keys(&global).unwrap().contains(&tag),
        call_to_string(&mut context, &object_to_string, &global),
    ));

    define_data_key(
        &mut context,
        &global,
        &tag,
        Value::String(JsString::try_from_utf8("custom-global").unwrap()),
        true,
        true,
        true,
    );
    let redefined = runtime.get_own_property(&global, &tag).unwrap().unwrap();
    observations.push(format!(
        "redefine={}|{}|{}|{}|{}",
        plain_value(context.get_property(&global, &tag).unwrap()),
        descriptor_bits(&redefined),
        descriptor_enumerable(&redefined),
        runtime.own_property_keys(&global).unwrap().contains(&tag),
        call_to_string(&mut context, &object_to_string, &global),
    ));

    let object_value = context.new_object().unwrap();
    let symbol_value = runtime
        .new_symbol(Some(JsString::try_from_utf8("not-a-tag").unwrap()))
        .unwrap();
    let non_strings = [
        Value::Undefined,
        Value::Null,
        Value::Bool(false),
        Value::Int(0),
        Value::Object(object_value),
        Value::Symbol(symbol_value),
    ];
    let non_string_tags = non_strings
        .into_iter()
        .map(|value| {
            define_data_key(&mut context, &global, &tag, value, true, true, true);
            call_to_string(&mut context, &object_to_string, &global)
        })
        .collect::<Vec<_>>()
        .join("|");
    observations.push(format!("non-string={non_string_tags}"));

    define_data_key(
        &mut context,
        &global,
        &tag,
        Value::String(JsString::try_from_utf8("restored-global").unwrap()),
        true,
        true,
        true,
    );
    observations.push(format!(
        "restored={}",
        call_to_string(&mut context, &object_to_string, &global)
    ));
    observations
}

fn property_callable(
    runtime: &Runtime,
    context: &mut quickjs_oxide::Context,
    object: &quickjs_oxide::ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(function) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&function)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn define_data_key(
    context: &mut quickjs_oxide::Context,
    object: &quickjs_oxide::ObjectRef,
    key: &PropertyKey,
    value: Value,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) {
    assert!(
        context
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(writable),
                    enumerable: DescriptorField::Present(enumerable),
                    configurable: DescriptorField::Present(configurable),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn define_value_key(
    context: &mut quickjs_oxide::Context,
    object: &quickjs_oxide::ObjectRef,
    key: &PropertyKey,
    value: Value,
) {
    assert!(
        context
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn call_to_string(
    context: &mut quickjs_oxide::Context,
    method: &CallableRef,
    object: &quickjs_oxide::ObjectRef,
) -> String {
    plain_value(
        context
            .call(method, Value::Object(object.clone()), &[])
            .unwrap(),
    )
}

fn descriptor_bits(descriptor: &CompleteOrdinaryPropertyDescriptor) -> String {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data {
            writable,
            enumerable,
            configurable,
            ..
        } => format!(
            "{}{}{}",
            u8::from(*writable),
            u8::from(*enumerable),
            u8::from(*configurable)
        ),
        CompleteOrdinaryPropertyDescriptor::Accessor { .. } => "accessor".to_owned(),
    }
}

fn descriptor_enumerable(descriptor: &CompleteOrdinaryPropertyDescriptor) -> bool {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data { enumerable, .. }
        | CompleteOrdinaryPropertyDescriptor::Accessor { enumerable, .. } => *enumerable,
    }
}

fn plain_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::BigInt(value) => value.to_string(),
        Value::Symbol(_) => "symbol".to_owned(),
        Value::Object(_) => "[object]".to_owned(),
    }
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .unwrap_or_else(|error| panic!("could not execute QJS_ORACLE: {error}"));
    assert!(
        output.status.success(),
        "QJS_ORACLE failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QJS_ORACLE emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
