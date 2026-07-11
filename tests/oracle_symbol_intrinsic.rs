use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    JsBigInt, JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError,
    SymbolRef, Value, WellKnownSymbol,
};

// The pinned probe uses Object and Reflect to inspect QuickJS. quickjs-oxide
// intentionally exposes the same graph through its public host API while the
// Object constructor is still outside the source slice. Source evaluation on
// the Rust side is reserved for primitive receiver get/set/delete behavior.
const ORACLE_PROBE: &str = r#"
function flags(object, key) {
    var descriptor = Object.getOwnPropertyDescriptor(object, key);
    return (descriptor.writable ? "1" : "0") +
           (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0");
}
function accessorFlags(object, key) {
    var descriptor = Object.getOwnPropertyDescriptor(object, key);
    return (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0") + ":" +
           (descriptor.get ? descriptor.get.name + "/" + descriptor.get.length : "-") + ":" +
           (descriptor.set ? descriptor.set.name + "/" + descriptor.set.length : "-");
}
function keyName(key) { return typeof key === "symbol" ? String(key) : key; }
function signature(fn) {
    return fn.name + ":" + fn.length + ":" + Reflect.ownKeys(fn).map(keyName).join(",");
}
function isConstructor(fn) {
    try { Reflect.construct(function () {}, [], fn); return true; }
    catch (_) { return false; }
}
function units(value) {
    if (value === undefined) return "undefined";
    var result = [];
    for (var index = 0; index < value.length; index++)
        result.push(value.charCodeAt(index).toString(16).padStart(4, "0"));
    return result.join(",");
}
function symbolRecord(symbol) {
    return units(symbol.description) + "/" + units(String(symbol));
}
function render(value) {
    if (typeof value === "symbol") return "symbol:" + symbolRecord(value);
    if (value !== null && typeof value === "object")
        return Object.prototype.toString.call(value);
    return String(value);
}
function observe(thunk) {
    try { return render(thunk()); }
    catch (error) {
        if (error !== null && typeof error === "object")
            return "throw:" + error.name + ":" + error.message;
        return "throw:" + String(error);
    }
}
function observeSymbol(thunk) {
    try { return "symbol:" + symbolRecord(thunk()); }
    catch (error) {
        if (error !== null && typeof error === "object")
            return "throw:" + error.name + ":" + error.message;
        return "throw:" + String(error);
    }
}

var implementedGlobals = [
    "parseInt", "parseFloat", "isNaN", "isFinite",
    "decodeURI", "decodeURIComponent", "encodeURI", "encodeURIComponent",
    "escape", "unescape", "Infinity", "NaN", "undefined", "Number",
    "Boolean", "Symbol", "globalThis", "BigInt"
];
print("global-order=" + Reflect.ownKeys(globalThis).filter(function (key) {
    return typeof key === "string" && implementedGlobals.indexOf(key) >= 0;
}).join(","));
print("keys=" + Reflect.ownKeys(Symbol).map(keyName).join(",") + "|" +
      Reflect.ownKeys(Symbol.prototype).map(keyName).join(","));
var wellKnownNames = [
    "toPrimitive", "iterator", "match", "matchAll", "replace", "search", "split",
    "toStringTag", "isConcatSpreadable", "hasInstance", "species", "unscopables",
    "asyncIterator"
];
print("descriptors=" + [
    flags(globalThis, "Symbol"), flags(Symbol, "length"), flags(Symbol, "name"),
    flags(Symbol, "for"), flags(Symbol, "keyFor"),
    wellKnownNames.map(function (name) { return flags(Symbol, name); }).join(","),
    flags(Symbol, "prototype"), flags(Symbol.prototype, "toString"),
    flags(Symbol.prototype, "valueOf"), accessorFlags(Symbol.prototype, "description"),
    flags(Symbol.prototype, "constructor"), flags(Symbol.prototype, Symbol.toPrimitive),
    flags(Symbol.prototype, Symbol.toStringTag)
].join("|"));
print("graph=" + [
    typeof Symbol,
    Object.getPrototypeOf(Symbol) === Function.prototype,
    Symbol.prototype.constructor === Symbol,
    Object.getPrototypeOf(Symbol.prototype) === Object.prototype,
    Object.prototype.toString.call(Symbol.prototype),
    Object.isExtensible(Symbol.prototype),
    isConstructor(Symbol), isConstructor(Symbol.for), isConstructor(Symbol.keyFor)
].join("|"));
var descriptionGetter = Object.getOwnPropertyDescriptor(Symbol.prototype, "description").get;
print("signatures=" + [
    Symbol, Symbol.for, Symbol.keyFor, Symbol.prototype.toString,
    Symbol.prototype.valueOf, Symbol.prototype[Symbol.toPrimitive], descriptionGetter
].map(signature).join("|"));

var callArguments = [
    [], [undefined], [""], [null], [false], [0], [-0], [1.5], [1n],
    ["a\0b"], ["\ud800"], ["\udc00"], ["😀"]
];
var callResults = callArguments.map(function (arguments_) {
    return symbolRecord(Symbol.apply(undefined, arguments_));
});
callResults.push(Symbol("same") !== Symbol("same"));
callResults.push(Symbol() !== Symbol(""));
print("calls=" + callResults.join("|"));

var constructHit = false;
var constructBomb = {};
Object.defineProperty(constructBomb, Symbol.toPrimitive, {
    configurable: true,
    value: function () { constructHit = true; throw new Error("converted"); }
});
function OtherTarget() {}
print("construct=" + [
    observe(function () { return new Symbol(constructBomb); }), constructHit,
    observe(function () { return Reflect.construct(Symbol, [constructBomb], OtherTarget); }),
    constructHit
].join("|"));

var conversionLog = "";
var exotic = {};
Object.defineProperty(exotic, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) { conversionLog += "exotic:" + hint + ","; return "E"; }
});
var fallback = {
    toString: function () { conversionLog += "toString,"; return {}; },
    valueOf: function () { conversionLog += "valueOf,"; return "V"; }
};
var invalidPrimitive = {};
Object.defineProperty(invalidPrimitive, Symbol.toPrimitive, {
    configurable: true,
    value: function () { conversionLog += "invalid,"; return {}; }
});
var throwingPrimitive = {};
Object.defineProperty(throwingPrimitive, Symbol.toPrimitive, {
    configurable: true,
    value: function () { conversionLog += "throw,"; throw new RangeError("sentinel"); }
});
print("conversion=" + [
    observeSymbol(function () { return Symbol(exotic); }),
    observeSymbol(function () { return Symbol(fallback); }),
    observeSymbol(function () { return Symbol(invalidPrimitive); }),
    observeSymbol(function () { return Symbol(throwingPrimitive); }),
    observeSymbol(function () { return Symbol(Symbol("source")); }),
    conversionLog
].join("|"));

function registryRecord(value) {
    var symbol = Symbol.for(value);
    return symbolRecord(symbol) + "/" + units(Symbol.keyFor(symbol));
}
print("registry=" + [
    Symbol.for() === Symbol.for(undefined), registryRecord(undefined),
    registryRecord(null), registryRecord(false), registryRecord(-0), registryRecord(1n),
    registryRecord(""), registryRecord("a\0b"), registryRecord("\ud800"),
    registryRecord("😀")
].join("|"));
var registryLog = "";
var registryExotic = {};
Object.defineProperty(registryExotic, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) { registryLog += "exotic:" + hint + ","; return "K"; }
});
var registryFallback = {
    toString: function () { registryLog += "toString,"; return {}; },
    valueOf: function () { registryLog += "valueOf,"; return "L"; }
};
print("registry-conversion=" + [
    Symbol.for(registryExotic) === Symbol.for("K"),
    Symbol.for(registryFallback) === Symbol.for("L"),
    observe(function () { return Symbol.for(Symbol("x")); }),
    observe(function () { return Symbol.keyFor(Symbol("fresh")); }),
    observe(function () { return Symbol.keyFor(Symbol.iterator); }),
    observe(function () { return Symbol.keyFor(Object(Symbol.for("wrapped"))); }),
    observe(function () { return Symbol.keyFor("x"); }),
    observe(function () { return Symbol.keyFor(); }), registryLog
].join("|"));

var seenWellKnown = [];
print("well-known=" + wellKnownNames.map(function (name) {
    var value = Symbol[name];
    var unique = seenWellKnown.indexOf(value) < 0;
    seenWellKnown.push(value);
    return name + ":" + units(value.description) + ":" +
           (Symbol.keyFor(value) === undefined) + ":" +
           (value === Symbol[name]) + ":" + unique;
}).join("|"));

var primitive = Symbol("x");
var noDescription = Symbol();
var wrapper = Object(primitive);
var spoof = Object.create(Symbol.prototype);
var toPrimitive = Symbol.prototype[Symbol.toPrimitive];
var hostileHint = {};
Object.defineProperty(hostileHint, "x", { get: function () { throw 91; } });
print("proto=" + [
    Symbol.prototype.toString.call(primitive), Symbol.prototype.toString.call(wrapper),
    Symbol.prototype.toString.call(noDescription),
    Symbol.prototype.valueOf.call(primitive) === primitive,
    Symbol.prototype.valueOf.call(wrapper) === primitive,
    toPrimitive.call(primitive, "default") === primitive,
    toPrimitive.call(primitive, "string") === primitive,
    toPrimitive.call(primitive, "number") === primitive,
    toPrimitive.call(primitive, hostileHint) === primitive,
    units(descriptionGetter.call(primitive)), units(descriptionGetter.call(noDescription)),
    observe(function () { return Symbol.prototype.valueOf.call(Symbol.prototype); }),
    observe(function () { return Symbol.prototype.toString.call(spoof); }),
    observe(function () { return descriptionGetter.call(null); }),
    Symbol.prototype.toString.call(primitive, hostileHint)
].join("|"));

var wrapper2 = Object.prototype.valueOf.call(primitive);
print("objects=" + [
    typeof wrapper, Object.getPrototypeOf(wrapper) === Symbol.prototype,
    Reflect.ownKeys(wrapper).length, wrapper.valueOf() === primitive,
    wrapper === wrapper2,
    Object.getPrototypeOf(wrapper2) === Symbol.prototype,
    Object.prototype.toString.call(primitive), Object.prototype.toString.call(wrapper),
    Object.prototype.toString.call(Symbol.prototype),
    Object.prototype.toLocaleString.call(primitive)
].join("|"));
delete Symbol.prototype[Symbol.toStringTag];
print("tag-delete=" + [
    Object.prototype.toString.call(primitive), Object.prototype.toString.call(wrapper),
    Object.prototype.toString.call(Symbol.prototype)
].join("|"));
Object.defineProperty(Symbol.prototype, Symbol.toStringTag, {
    value: "CustomSymbol", writable: false, enumerable: false, configurable: true
});
print("tag-custom=" + [
    Object.prototype.toString.call(primitive), Object.prototype.toString.call(wrapper),
    Object.prototype.toString.call(Symbol.prototype)
].join("|"));
Object.defineProperty(Symbol.prototype, Symbol.toStringTag, { value: 1 });
print("tag-non-string=" + [
    Object.prototype.toString.call(primitive), Object.prototype.toString.call(wrapper),
    Object.prototype.toString.call(Symbol.prototype)
].join("|"));

var strictGetThis, sloppyGetThis, strictSetThis, sloppySetThis;
Object.defineProperty(Symbol.prototype, "__strictGet", {
    configurable: true,
    get: function () { "use strict"; strictGetThis = this; return this; }
});
Object.defineProperty(Symbol.prototype, "__sloppyGet", {
    configurable: true,
    get: function () { sloppyGetThis = this; return this.valueOf(); }
});
Object.defineProperty(Symbol.prototype, "__strictSet", {
    configurable: true,
    set: function () { "use strict"; strictSetThis = this; }
});
Object.defineProperty(Symbol.prototype, "__sloppySet", {
    configurable: true,
    set: function () { sloppySetThis = this; }
});
var strictGetResult = Symbol("get").__strictGet;
var sloppyGetResult = Symbol("get").__sloppyGet;
var strictSetResult = Symbol("strict-set").__strictSet = 7;
var sloppySetResult = Symbol("sloppy-set").__sloppySet = 8;
print("accessors=" + [
    typeof strictGetResult, units(strictGetResult.description),
    typeof strictGetThis, strictGetThis === strictGetResult,
    typeof sloppyGetResult, units(sloppyGetResult.description), typeof sloppyGetThis,
    Object.getPrototypeOf(sloppyGetThis) === Symbol.prototype,
    units(sloppyGetThis.valueOf().description),
    strictSetResult, typeof strictSetThis, units(strictSetThis.description),
    sloppySetResult, typeof sloppySetThis,
    Object.getPrototypeOf(sloppySetThis) === Symbol.prototype,
    units(sloppySetThis.valueOf().description)
].join("|"));

Object.defineProperty(Symbol.prototype, "__rw", {
    value: 1, writable: true, configurable: true
});
Object.defineProperty(Symbol.prototype, "__ro", {
    value: 1, writable: false, configurable: true
});
var deleteHit = false;
Object.defineProperty(Symbol.prototype, "__delete", {
    configurable: true,
    get: function () { deleteHit = true; return 1; }
});
print("writes=" + [
    (Symbol("x").__rw = 2), Symbol("x").__rw,
    observe(function () { "use strict"; return Symbol("x").__rw = 3; }),
    (Symbol("x").__ro = 2), Symbol("x").__ro,
    observe(function () { "use strict"; return Symbol("x").__ro = 3; }),
    delete Symbol("x").__delete, deleteHit,
    Object.prototype.hasOwnProperty.call(Symbol.prototype, "__delete"),
    (function () { "use strict"; return delete Symbol("x").__delete; })()
].join("|"));
"#;

#[test]
fn symbol_intrinsic_matches_pinned_quickjs() {
    let rust = rust_observations();
    assert_eq!(
        rust.len(),
        18,
        "the Symbol differential unexpectedly changed breadth"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Symbol intrinsic differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "Symbol intrinsic behavior differed from pinned QuickJS"
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let symbol_prototype = context.symbol_prototype().unwrap();
    let symbol = property_callable(&runtime, &mut context, &global, "Symbol");
    let symbol_for = property_callable(&runtime, &mut context, symbol.as_object(), "for");
    let key_for = property_callable(&runtime, &mut context, symbol.as_object(), "keyFor");
    let to_string = property_callable(&runtime, &mut context, &symbol_prototype, "toString");
    let value_of = property_callable(&runtime, &mut context, &symbol_prototype, "valueOf");
    let to_primitive_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    let to_primitive = property_callable_key(
        &runtime,
        &mut context,
        &symbol_prototype,
        &to_primitive_key,
        "Symbol.prototype[Symbol.toPrimitive]",
    );
    let description_getter = accessor_getter(
        &runtime,
        &symbol_prototype,
        &runtime.intern_property_key("description").unwrap(),
    );
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let object_to_locale_string =
        property_callable(&runtime, &mut context, &object_prototype, "toLocaleString");
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");

    let implemented_globals = [
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
        "Symbol",
        "globalThis",
        "BigInt",
    ];
    let global_order = own_key_names(&runtime, &global)
        .into_iter()
        .filter(|name| implemented_globals.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let mut observations = vec![format!("global-order={global_order}")];
    observations.push(format!(
        "keys={}|{}",
        own_key_names(&runtime, symbol.as_object()).join(","),
        own_key_names(&runtime, &symbol_prototype).join(",")
    ));

    let well_known = well_known_entries();
    observations.push(format!(
        "descriptors={}",
        [
            data_flags(&runtime, &global, "Symbol"),
            data_flags(&runtime, symbol.as_object(), "length"),
            data_flags(&runtime, symbol.as_object(), "name"),
            data_flags(&runtime, symbol.as_object(), "for"),
            data_flags(&runtime, symbol.as_object(), "keyFor"),
            well_known
                .iter()
                .map(|(name, _)| data_flags(&runtime, symbol.as_object(), name))
                .collect::<Vec<_>>()
                .join(","),
            data_flags(&runtime, symbol.as_object(), "prototype"),
            data_flags(&runtime, &symbol_prototype, "toString"),
            data_flags(&runtime, &symbol_prototype, "valueOf"),
            accessor_flags(
                &runtime,
                &mut context,
                &symbol_prototype,
                &runtime.intern_property_key("description").unwrap(),
            ),
            data_flags(&runtime, &symbol_prototype, "constructor"),
            data_flags_key(&runtime, &symbol_prototype, &to_primitive_key),
            data_flags_key(
                &runtime,
                &symbol_prototype,
                &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag)),
            ),
        ]
        .join("|")
    ));
    observations.push(format!(
        "graph=function|{}|{}|{}|{}|{}|{}|{}|{}",
        runtime.get_prototype_of(symbol.as_object()).unwrap() == Some(function_prototype),
        matches!(
            context
                .get_property(
                    &symbol_prototype,
                    &runtime.intern_property_key("constructor").unwrap(),
                )
                .unwrap(),
            Value::Object(object) if object == *symbol.as_object()
        ),
        runtime.get_prototype_of(&symbol_prototype).unwrap() == Some(object_prototype.clone()),
        render_value(
            &runtime,
            context
                .call(
                    &object_to_string,
                    Value::Object(symbol_prototype.clone()),
                    &[],
                )
                .unwrap(),
        ),
        runtime.is_extensible(&symbol_prototype).unwrap(),
        runtime.is_constructor(symbol.as_object()).unwrap(),
        runtime.is_constructor(symbol_for.as_object()).unwrap(),
        runtime.is_constructor(key_for.as_object()).unwrap(),
    ));
    observations.push(format!(
        "signatures={}",
        [
            &symbol,
            &symbol_for,
            &key_for,
            &to_string,
            &value_of,
            &to_primitive,
            &description_getter,
        ]
        .into_iter()
        .map(|callable| function_signature(&runtime, &mut context, callable))
        .collect::<Vec<_>>()
        .join("|")
    ));

    let call_arguments = vec![
        vec![],
        vec![Value::Undefined],
        vec![Value::String(JsString::try_from_utf8("").unwrap())],
        vec![Value::Null],
        vec![Value::Bool(false)],
        vec![Value::Int(0)],
        vec![Value::Float(-0.0)],
        vec![Value::Float(1.5)],
        vec![Value::BigInt(JsBigInt::from(1))],
        vec![Value::String(
            JsString::try_from_utf16([0x61, 0x00, 0x62]).unwrap(),
        )],
        vec![Value::String(JsString::try_from_utf16([0xd800]).unwrap())],
        vec![Value::String(JsString::try_from_utf16([0xdc00]).unwrap())],
        vec![Value::String(
            JsString::try_from_utf16([0xd83d, 0xde00]).unwrap(),
        )],
    ];
    let mut calls = call_arguments
        .iter()
        .map(|arguments| {
            let Value::Symbol(value) = context.call(&symbol, Value::Undefined, arguments).unwrap()
            else {
                panic!("Symbol call did not return a Symbol primitive");
            };
            symbol_record_via_to_string(&runtime, &mut context, &to_string, &value)
        })
        .collect::<Vec<_>>();
    let first_same = call_symbol(
        &mut context,
        &symbol,
        &[Value::String(JsString::try_from_utf8("same").unwrap())],
    );
    let second_same = call_symbol(
        &mut context,
        &symbol,
        &[Value::String(JsString::try_from_utf8("same").unwrap())],
    );
    calls.push((first_same != second_same).to_string());
    calls.push(
        (call_symbol(&mut context, &symbol, &[])
            != call_symbol(
                &mut context,
                &symbol,
                &[Value::String(JsString::try_from_utf8("").unwrap())],
            ))
        .to_string(),
    );
    observations.push(format!("calls={}", calls.join("|")));

    let construct_hit = define_global(&runtime, &global, "constructHit", Value::Bool(false));
    let construct_bomb = conversion_object(
        &runtime,
        &mut context,
        "(function() { constructHit = true; throw new Error('converted'); })",
    );
    let other_target = eval_callable(&runtime, &mut context, "(function OtherTarget() {})");
    let first_construct = observe_construct(
        &runtime,
        &mut context,
        &symbol,
        &[Value::Object(construct_bomb.clone())],
    );
    let first_hit = render_value(
        &runtime,
        context.get_property(&global, &construct_hit).unwrap(),
    );
    let second_construct = observe_construct_with_new_target(
        &runtime,
        &mut context,
        &symbol,
        &other_target,
        &[Value::Object(construct_bomb)],
    );
    let second_hit = render_value(
        &runtime,
        context.get_property(&global, &construct_hit).unwrap(),
    );
    observations.push(format!(
        "construct={first_construct}|{first_hit}|{second_construct}|{second_hit}"
    ));

    let conversion_log = define_global(
        &runtime,
        &global,
        "conversionLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let exotic = conversion_object(
        &runtime,
        &mut context,
        r#"(function(hint) { conversionLog += "exotic:" + hint + ","; return "E"; })"#,
    );
    let fallback = context.new_object().unwrap();
    let fallback_result = context.new_object().unwrap();
    define_global(
        &runtime,
        &global,
        "fallbackResult",
        Value::Object(fallback_result),
    );
    let fallback_to_string = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { conversionLog += "toString,"; return fallbackResult; })"#,
    );
    let fallback_value_of = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { conversionLog += "valueOf,"; return "V"; })"#,
    );
    define_data(
        &runtime,
        &fallback,
        "toString",
        Value::Object(fallback_to_string.as_object().clone()),
        true,
        false,
        true,
    );
    define_data(
        &runtime,
        &fallback,
        "valueOf",
        Value::Object(fallback_value_of.as_object().clone()),
        true,
        false,
        true,
    );
    let invalid_result = context.new_object().unwrap();
    define_global(
        &runtime,
        &global,
        "invalidPrimitiveResult",
        Value::Object(invalid_result),
    );
    let invalid = conversion_object(
        &runtime,
        &mut context,
        "(function() { conversionLog += 'invalid,'; return invalidPrimitiveResult; })",
    );
    let throwing = conversion_object(
        &runtime,
        &mut context,
        "(function() { conversionLog += 'throw,'; throw new RangeError('sentinel'); })",
    );
    let source_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("source").unwrap()))
        .unwrap();
    let converted = [
        Value::Object(exotic),
        Value::Object(fallback),
        Value::Object(invalid),
        Value::Object(throwing),
        Value::Symbol(source_symbol),
    ]
    .into_iter()
    .map(|argument| {
        observe_symbol_call(
            &runtime,
            &mut context,
            &symbol,
            &to_string,
            Value::Undefined,
            &[argument],
        )
    })
    .collect::<Vec<_>>();
    let conversion_log = render_value(
        &runtime,
        context.get_property(&global, &conversion_log).unwrap(),
    );
    observations.push(format!(
        "conversion={}|{conversion_log}",
        converted.join("|")
    ));

    let registry_inputs = [
        Value::Undefined,
        Value::Null,
        Value::Bool(false),
        Value::Float(-0.0),
        Value::BigInt(JsBigInt::from(1)),
        Value::String(JsString::try_from_utf8("").unwrap()),
        Value::String(JsString::try_from_utf16([0x61, 0x00, 0x62]).unwrap()),
        Value::String(JsString::try_from_utf16([0xd800]).unwrap()),
        Value::String(JsString::try_from_utf16([0xd83d, 0xde00]).unwrap()),
    ];
    let missing = call_symbol(&mut context, &symbol_for, &[]);
    let explicit_undefined = call_symbol(&mut context, &symbol_for, &[Value::Undefined]);
    let mut registry = vec![(missing == explicit_undefined).to_string()];
    registry.extend(registry_inputs.into_iter().map(|input| {
        let registered = call_symbol(&mut context, &symbol_for, &[input]);
        let key = context
            .call(
                &key_for,
                Value::Undefined,
                &[Value::Symbol(registered.clone())],
            )
            .unwrap();
        let Value::String(key) = key else {
            panic!("Symbol.keyFor(registry symbol) did not return a string");
        };
        format!(
            "{}/{}",
            symbol_record_via_to_string(&runtime, &mut context, &to_string, &registered),
            utf16_units(&key)
        )
    }));
    observations.push(format!("registry={}", registry.join("|")));

    let registry_log = define_global(
        &runtime,
        &global,
        "registryLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let registry_exotic = conversion_object(
        &runtime,
        &mut context,
        r#"(function(hint) { registryLog += "exotic:" + hint + ","; return "K"; })"#,
    );
    let registry_fallback = context.new_object().unwrap();
    let registry_fallback_result = context.new_object().unwrap();
    define_global(
        &runtime,
        &global,
        "registryFallbackResult",
        Value::Object(registry_fallback_result),
    );
    let registry_to_string = eval_callable(
        &runtime,
        &mut context,
        "(function() { registryLog += 'toString,'; return registryFallbackResult; })",
    );
    let registry_value_of = eval_callable(
        &runtime,
        &mut context,
        "(function() { registryLog += 'valueOf,'; return 'L'; })",
    );
    define_data(
        &runtime,
        &registry_fallback,
        "toString",
        Value::Object(registry_to_string.as_object().clone()),
        true,
        false,
        true,
    );
    define_data(
        &runtime,
        &registry_fallback,
        "valueOf",
        Value::Object(registry_value_of.as_object().clone()),
        true,
        false,
        true,
    );
    let exotic_registered =
        call_symbol(&mut context, &symbol_for, &[Value::Object(registry_exotic)]);
    let k_registered = call_symbol(
        &mut context,
        &symbol_for,
        &[Value::String(JsString::try_from_utf8("K").unwrap())],
    );
    let fallback_registered = call_symbol(
        &mut context,
        &symbol_for,
        &[Value::Object(registry_fallback)],
    );
    let l_registered = call_symbol(
        &mut context,
        &symbol_for,
        &[Value::String(JsString::try_from_utf8("L").unwrap())],
    );
    let wrapped_registered = call_symbol(
        &mut context,
        &symbol_for,
        &[Value::String(JsString::try_from_utf8("wrapped").unwrap())],
    );
    let wrapped = expect_object(
        context
            .call(&object_value_of, Value::Symbol(wrapped_registered), &[])
            .unwrap(),
        "registered Symbol wrapper",
    );
    let registry_conversion = [
        (exotic_registered == k_registered).to_string(),
        (fallback_registered == l_registered).to_string(),
        observe_call(
            &runtime,
            &mut context,
            &symbol_for,
            Value::Undefined,
            &[Value::Symbol(
                runtime
                    .new_symbol(Some(JsString::try_from_utf8("x").unwrap()))
                    .unwrap(),
            )],
        ),
        observe_call(
            &runtime,
            &mut context,
            &key_for,
            Value::Undefined,
            &[Value::Symbol(
                runtime
                    .new_symbol(Some(JsString::try_from_utf8("fresh").unwrap()))
                    .unwrap(),
            )],
        ),
        observe_call(
            &runtime,
            &mut context,
            &key_for,
            Value::Undefined,
            &[Value::Symbol(
                runtime.well_known_symbol(WellKnownSymbol::Iterator),
            )],
        ),
        observe_call(
            &runtime,
            &mut context,
            &key_for,
            Value::Undefined,
            &[Value::Object(wrapped)],
        ),
        observe_call(
            &runtime,
            &mut context,
            &key_for,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf8("x").unwrap())],
        ),
        observe_call(&runtime, &mut context, &key_for, Value::Undefined, &[]),
        render_value(
            &runtime,
            context.get_property(&global, &registry_log).unwrap(),
        ),
    ];
    observations.push(format!(
        "registry-conversion={}",
        registry_conversion.join("|")
    ));

    let mut seen = Vec::<SymbolRef>::new();
    let well_known_observations = well_known
        .iter()
        .map(|(name, kind)| {
            let Value::Symbol(value) = context
                .get_property(
                    symbol.as_object(),
                    &runtime.intern_property_key(name).unwrap(),
                )
                .unwrap()
            else {
                panic!("Symbol.{name} was not a Symbol");
            };
            let stable = value == runtime.well_known_symbol(*kind);
            let unique = !seen.contains(&value);
            seen.push(value.clone());
            format!(
                "{name}:{}:{}:{stable}:{unique}",
                utf16_units(&runtime.symbol_description(&value).unwrap().unwrap()),
                runtime.symbol_key_for(&value).unwrap().is_none(),
            )
        })
        .collect::<Vec<_>>();
    observations.push(format!("well-known={}", well_known_observations.join("|")));

    let primitive = runtime
        .new_symbol(Some(JsString::try_from_utf8("x").unwrap()))
        .unwrap();
    let no_description = runtime.new_symbol(None).unwrap();
    let wrapper = expect_object(
        context
            .call(&object_value_of, Value::Symbol(primitive.clone()), &[])
            .unwrap(),
        "Symbol wrapper",
    );
    let spoof = runtime.new_object(Some(&symbol_prototype)).unwrap();
    let hostile_hint = context.new_object().unwrap();
    let hostile_getter = eval_callable(&runtime, &mut context, "(function() { throw 91; })");
    define_accessor_key(
        &runtime,
        &hostile_hint,
        &runtime.intern_property_key("x").unwrap(),
        Some(hostile_getter),
        None,
    );
    let proto = [
        render_value(
            &runtime,
            context
                .call(&to_string, Value::Symbol(primitive.clone()), &[])
                .unwrap(),
        ),
        render_value(
            &runtime,
            context
                .call(&to_string, Value::Object(wrapper.clone()), &[])
                .unwrap(),
        ),
        render_value(
            &runtime,
            context
                .call(&to_string, Value::Symbol(no_description.clone()), &[])
                .unwrap(),
        ),
        matches!(
            context.call(&value_of, Value::Symbol(primitive.clone()), &[]),
            Ok(Value::Symbol(value)) if value == primitive
        )
        .to_string(),
        matches!(
            context.call(&value_of, Value::Object(wrapper.clone()), &[]),
            Ok(Value::Symbol(value)) if value == primitive
        )
        .to_string(),
        matches!(
            context.call(
                &to_primitive,
                Value::Symbol(primitive.clone()),
                &[Value::String(JsString::try_from_utf8("default").unwrap())],
            ),
            Ok(Value::Symbol(value)) if value == primitive
        )
        .to_string(),
        matches!(
            context.call(
                &to_primitive,
                Value::Symbol(primitive.clone()),
                &[Value::String(JsString::try_from_utf8("string").unwrap())],
            ),
            Ok(Value::Symbol(value)) if value == primitive
        )
        .to_string(),
        matches!(
            context.call(
                &to_primitive,
                Value::Symbol(primitive.clone()),
                &[Value::String(JsString::try_from_utf8("number").unwrap())],
            ),
            Ok(Value::Symbol(value)) if value == primitive
        )
        .to_string(),
        matches!(
            context.call(
                &to_primitive,
                Value::Symbol(primitive.clone()),
                &[Value::Object(hostile_hint.clone())],
            ),
            Ok(Value::Symbol(value)) if value == primitive
        )
        .to_string(),
        utf16_units(&expect_string(
            context
                .call(&description_getter, Value::Symbol(primitive.clone()), &[])
                .unwrap(),
            "Symbol description",
        )),
        match context.call(
            &description_getter,
            Value::Symbol(no_description.clone()),
            &[],
        ) {
            Ok(Value::Undefined) => "undefined".to_owned(),
            Ok(value) => panic!("description getter returned {value:?}"),
            Err(error) => panic!("description getter failed: {error}"),
        },
        observe_call(
            &runtime,
            &mut context,
            &value_of,
            Value::Object(symbol_prototype.clone()),
            &[],
        ),
        observe_call(
            &runtime,
            &mut context,
            &to_string,
            Value::Object(spoof),
            &[],
        ),
        observe_call(
            &runtime,
            &mut context,
            &description_getter,
            Value::Null,
            &[],
        ),
        render_value(
            &runtime,
            context
                .call(
                    &to_string,
                    Value::Symbol(primitive.clone()),
                    &[Value::Object(hostile_hint)],
                )
                .unwrap(),
        ),
    ];
    observations.push(format!("proto={}", proto.join("|")));

    let wrapper2 = expect_object(
        context
            .call(&object_value_of, Value::Symbol(primitive.clone()), &[])
            .unwrap(),
        "second Symbol wrapper",
    );
    let objects = [
        "object".to_owned(),
        (runtime.get_prototype_of(&wrapper).unwrap() == Some(symbol_prototype.clone())).to_string(),
        runtime
            .own_property_keys(&wrapper)
            .unwrap()
            .len()
            .to_string(),
        matches!(
            context.call(&value_of, Value::Object(wrapper.clone()), &[]),
            Ok(Value::Symbol(value)) if value == primitive
        )
        .to_string(),
        (wrapper == wrapper2).to_string(),
        (runtime.get_prototype_of(&wrapper2).unwrap() == Some(symbol_prototype.clone()))
            .to_string(),
        render_value(
            &runtime,
            context
                .call(&object_to_string, Value::Symbol(primitive.clone()), &[])
                .unwrap(),
        ),
        render_value(
            &runtime,
            context
                .call(&object_to_string, Value::Object(wrapper.clone()), &[])
                .unwrap(),
        ),
        render_value(
            &runtime,
            context
                .call(
                    &object_to_string,
                    Value::Object(symbol_prototype.clone()),
                    &[],
                )
                .unwrap(),
        ),
        render_value(
            &runtime,
            context
                .call(
                    &object_to_locale_string,
                    Value::Symbol(primitive.clone()),
                    &[],
                )
                .unwrap(),
        ),
    ];
    observations.push(format!("objects={}", objects.join("|")));

    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert!(runtime.delete_property(&symbol_prototype, &tag).unwrap());
    observations.push(format!(
        "tag-delete={}",
        symbol_tags(
            &runtime,
            &mut context,
            &object_to_string,
            &symbol_prototype,
            &wrapper,
            &primitive,
        )
        .join("|")
    ));
    define_data_key(
        &runtime,
        &symbol_prototype,
        &tag,
        Value::String(JsString::try_from_utf8("CustomSymbol").unwrap()),
        false,
        false,
        true,
    );
    observations.push(format!(
        "tag-custom={}",
        symbol_tags(
            &runtime,
            &mut context,
            &object_to_string,
            &symbol_prototype,
            &wrapper,
            &primitive,
        )
        .join("|")
    ));
    define_data_key(
        &runtime,
        &symbol_prototype,
        &tag,
        Value::Int(1),
        false,
        false,
        false,
    );
    observations.push(format!(
        "tag-non-string={}",
        symbol_tags(
            &runtime,
            &mut context,
            &object_to_string,
            &symbol_prototype,
            &wrapper,
            &primitive,
        )
        .join("|")
    ));

    install_primitive_accessors(&runtime, &mut context, &global, &symbol_prototype);
    let strict_get_result = context.eval("Symbol('get').__strictGet").unwrap();
    let strict_get_this = global_value(&runtime, &mut context, &global, "strictGetThis");
    let sloppy_get_result = context.eval("Symbol('get').__sloppyGet").unwrap();
    let sloppy_get_this = global_value(&runtime, &mut context, &global, "sloppyGetThis");
    let strict_set_result = context
        .eval("Symbol('strict-set').__strictSet = 7")
        .unwrap();
    let strict_set_this = global_value(&runtime, &mut context, &global, "strictSetThis");
    let sloppy_set_result = context
        .eval("Symbol('sloppy-set').__sloppySet = 8")
        .unwrap();
    let sloppy_set_this = global_value(&runtime, &mut context, &global, "sloppySetThis");
    let sloppy_get_unboxed = expect_symbol(
        context
            .call(&value_of, sloppy_get_this.clone(), &[])
            .unwrap(),
        "sloppy getter wrapper",
    );
    let sloppy_set_unboxed = expect_symbol(
        context
            .call(&value_of, sloppy_set_this.clone(), &[])
            .unwrap(),
        "sloppy setter wrapper",
    );
    let strict_get_symbol = expect_symbol(strict_get_result.clone(), "strict getter result");
    let sloppy_get_symbol = expect_symbol(sloppy_get_result, "sloppy getter result");
    let strict_set_symbol = expect_symbol(strict_set_this.clone(), "strict setter receiver");
    let accessors = [
        "symbol".to_owned(),
        symbol_description_units(&runtime, &strict_get_symbol),
        "symbol".to_owned(),
        (strict_get_this == strict_get_result).to_string(),
        "symbol".to_owned(),
        symbol_description_units(&runtime, &sloppy_get_symbol),
        "object".to_owned(),
        object_has_prototype(&runtime, &sloppy_get_this, &symbol_prototype).to_string(),
        symbol_description_units(&runtime, &sloppy_get_unboxed),
        render_value(&runtime, strict_set_result),
        "symbol".to_owned(),
        symbol_description_units(&runtime, &strict_set_symbol),
        render_value(&runtime, sloppy_set_result),
        "object".to_owned(),
        object_has_prototype(&runtime, &sloppy_set_this, &symbol_prototype).to_string(),
        symbol_description_units(&runtime, &sloppy_set_unboxed),
    ];
    observations.push(format!("accessors={}", accessors.join("|")));

    define_data(
        &runtime,
        &symbol_prototype,
        "__rw",
        Value::Int(1),
        true,
        false,
        true,
    );
    define_data(
        &runtime,
        &symbol_prototype,
        "__ro",
        Value::Int(1),
        false,
        false,
        true,
    );
    let delete_hit = define_global(&runtime, &global, "deleteHit", Value::Bool(false));
    let delete_getter = eval_callable(
        &runtime,
        &mut context,
        "(function() { deleteHit = true; return 1; })",
    );
    define_accessor(
        &runtime,
        &symbol_prototype,
        "__delete",
        Some(delete_getter),
        None,
    );
    let delete_key = runtime.intern_property_key("__delete").unwrap();
    let writes = [
        render_value(&runtime, context.eval("Symbol('x').__rw = 2").unwrap()),
        render_value(&runtime, context.eval("Symbol('x').__rw").unwrap()),
        observe_eval(
            &runtime,
            &mut context,
            r#"(function() { "use strict"; return Symbol("x").__rw = 3; })()"#,
        ),
        render_value(&runtime, context.eval("Symbol('x').__ro = 2").unwrap()),
        render_value(&runtime, context.eval("Symbol('x').__ro").unwrap()),
        observe_eval(
            &runtime,
            &mut context,
            r#"(function() { "use strict"; return Symbol("x").__ro = 3; })()"#,
        ),
        render_value(
            &runtime,
            context.eval("delete Symbol('x').__delete").unwrap(),
        ),
        render_value(
            &runtime,
            context.get_property(&global, &delete_hit).unwrap(),
        ),
        runtime
            .has_own_property(&symbol_prototype, &delete_key)
            .unwrap()
            .to_string(),
        render_value(
            &runtime,
            context
                .eval(r#"(function() { "use strict"; return delete Symbol("x").__delete; })()"#)
                .unwrap(),
        ),
    ];
    observations.push(format!("writes={}", writes.join("|")));

    observations
}

#[test]
fn symbol_cross_realm_routes_registry_boxing_lookup_and_errors() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_global = first.global_object().unwrap();
    let second_global = second.global_object().unwrap();
    let first_symbol = property_callable(&runtime, &mut first, &first_global, "Symbol");
    let second_symbol = property_callable(&runtime, &mut second, &second_global, "Symbol");
    let first_for = property_callable(&runtime, &mut first, first_symbol.as_object(), "for");
    let second_for = property_callable(&runtime, &mut second, second_symbol.as_object(), "for");
    let first_key_for = property_callable(&runtime, &mut first, first_symbol.as_object(), "keyFor");
    let first_prototype = first.symbol_prototype().unwrap();
    let second_prototype = second.symbol_prototype().unwrap();
    assert_ne!(first_prototype, second_prototype);

    for kind in WellKnownSymbol::ALL {
        let name = well_known_entries()
            .into_iter()
            .find_map(|(name, candidate)| (candidate == kind).then_some(name))
            .unwrap();
        let first_value = first
            .get_property(
                first_symbol.as_object(),
                &runtime.intern_property_key(name).unwrap(),
            )
            .unwrap();
        let second_value = second
            .get_property(
                second_symbol.as_object(),
                &runtime.intern_property_key(name).unwrap(),
            )
            .unwrap();
        assert_eq!(
            first_value, second_value,
            "well-known symbol split by realm"
        );
    }

    let registry_key = Value::String(JsString::try_from_utf8("cross-realm").unwrap());
    let first_registered = first
        .call(
            &first_for,
            Value::Undefined,
            std::slice::from_ref(&registry_key),
        )
        .unwrap();
    let second_registered = second
        .call(
            &second_for,
            Value::Undefined,
            std::slice::from_ref(&registry_key),
        )
        .unwrap();
    assert_eq!(first_registered, second_registered);
    assert_ne!(
        first
            .call(
                &first_symbol,
                Value::Undefined,
                &[Value::String(JsString::try_from_utf8("fresh").unwrap())],
            )
            .unwrap(),
        second
            .call(
                &second_symbol,
                Value::Undefined,
                &[Value::String(JsString::try_from_utf8("fresh").unwrap())],
            )
            .unwrap(),
    );

    let first_value_of = property_callable(&runtime, &mut first, &first_prototype, "valueOf");
    let second_value_of = property_callable(&runtime, &mut second, &second_prototype, "valueOf");
    let first_object_prototype = first.object_prototype().unwrap();
    let second_object_prototype = second.object_prototype().unwrap();
    let first_object_value_of =
        property_callable(&runtime, &mut first, &first_object_prototype, "valueOf");
    let second_object_value_of =
        property_callable(&runtime, &mut second, &second_object_prototype, "valueOf");
    let first_object_to_string =
        property_callable(&runtime, &mut first, &first_object_prototype, "toString");
    let seven = runtime
        .new_symbol(Some(JsString::try_from_utf8("seven").unwrap()))
        .unwrap();
    let nine = runtime
        .new_symbol(Some(JsString::try_from_utf8("nine").unwrap()))
        .unwrap();
    let first_wrapper = expect_object(
        second
            .call(&first_object_value_of, Value::Symbol(seven.clone()), &[])
            .unwrap(),
        "first-realm Symbol wrapper",
    );
    let second_wrapper = expect_object(
        first
            .call(&second_object_value_of, Value::Symbol(nine.clone()), &[])
            .unwrap(),
        "second-realm Symbol wrapper",
    );
    assert_eq!(
        runtime.get_prototype_of(&first_wrapper).unwrap(),
        Some(first_prototype.clone())
    );
    assert_eq!(
        runtime.get_prototype_of(&second_wrapper).unwrap(),
        Some(second_prototype.clone())
    );
    for (method, wrapper, expected) in [
        (&first_value_of, &first_wrapper, &seven),
        (&second_value_of, &first_wrapper, &seven),
        (&first_value_of, &second_wrapper, &nine),
        (&second_value_of, &second_wrapper, &nine),
    ] {
        assert_eq!(
            second
                .call(method, Value::Object(wrapper.clone()), &[])
                .unwrap(),
            Value::Symbol(expected.clone()),
            "Symbol wrapper branding must be realm-independent"
        );
    }

    define_data(
        &runtime,
        &first_prototype,
        "__realmMarker",
        Value::String(JsString::try_from_utf8("first").unwrap()),
        true,
        false,
        true,
    );
    define_data(
        &runtime,
        &second_prototype,
        "__realmMarker",
        Value::String(JsString::try_from_utf8("second").unwrap()),
        true,
        false,
        true,
    );
    let first_reader = eval_callable(
        &runtime,
        &mut first,
        "(function() { return Symbol('reader').__realmMarker; })",
    );
    assert_eq!(
        second.call(&first_reader, Value::Undefined, &[]).unwrap(),
        Value::String(JsString::try_from_utf8("first").unwrap()),
        "primitive member lookup must use the bytecode function's realm"
    );

    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    define_data_key(
        &runtime,
        &first_prototype,
        &tag,
        Value::String(JsString::try_from_utf8("FirstSymbol").unwrap()),
        false,
        false,
        true,
    );
    define_data_key(
        &runtime,
        &second_prototype,
        &tag,
        Value::String(JsString::try_from_utf8("SecondSymbol").unwrap()),
        false,
        false,
        true,
    );
    assert_eq!(
        second
            .call(&first_object_to_string, Value::Symbol(seven.clone()), &[],)
            .unwrap(),
        Value::String(JsString::try_from_utf8("[object FirstSymbol]").unwrap()),
        "Object.prototype.toString must box in the method's defining realm"
    );

    let first_type_error = intrinsic_prototype(&runtime, &mut first, "TypeError");
    let second_type_error = intrinsic_prototype(&runtime, &mut second, "TypeError");
    assert_ne!(first_type_error, second_type_error);
    let symbol_argument = runtime
        .new_symbol(Some(JsString::try_from_utf8("argument").unwrap()))
        .unwrap();
    assert_eq!(
        second.call(
            &first_symbol,
            Value::Undefined,
            &[Value::Symbol(symbol_argument)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut second))
            .unwrap(),
        Some(first_type_error.clone()),
        "Symbol conversion TypeError must use the constructor's defining realm"
    );
    assert_eq!(
        second.call(&first_key_for, Value::Undefined, &[Value::Int(1)]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut second))
            .unwrap(),
        Some(first_type_error.clone())
    );
    let spoof = second.new_object().unwrap();
    assert_eq!(
        second.call(&first_value_of, Value::Object(spoof), &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut second))
            .unwrap(),
        Some(first_type_error.clone())
    );
    assert_eq!(
        second.construct(&first_symbol, &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut second))
            .unwrap(),
        Some(first_type_error)
    );

    let user_throw = eval_callable(
        &runtime,
        &mut second,
        "(function() { throw new TypeError('foreign conversion'); })",
    );
    let throwing_input = second.new_object().unwrap();
    define_data_key(
        &runtime,
        &throwing_input,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(user_throw.as_object().clone()),
        true,
        false,
        true,
    );
    assert_eq!(
        first.call(
            &first_symbol,
            Value::Undefined,
            &[Value::Object(throwing_input)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut first))
            .unwrap(),
        Some(second_type_error),
        "an explicit user throw must retain the conversion function's realm"
    );

    assert_eq!(
        first
            .call(
                &first_key_for,
                Value::Undefined,
                std::slice::from_ref(&first_registered),
            )
            .unwrap(),
        registry_key
    );
    drop(second_global);
}

#[test]
fn symbol_primitives_do_not_retain_realms_but_wrappers_do_and_atoms_survive() {
    let runtime = Runtime::new();
    let primitive = {
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        let constructor = property_callable(&runtime, &mut context, &global, "Symbol");
        call_symbol(
            &mut context,
            &constructor,
            &[Value::String(
                JsString::try_from_utf16([0x61, 0xd800, 0x62]).unwrap(),
            )],
        )
    };
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        0,
        "a realm-neutral Symbol primitive must not retain its creating context"
    );
    assert_eq!(
        runtime.symbol_description(&primitive).unwrap(),
        Some(JsString::try_from_utf16([0x61, 0xd800, 0x62]).unwrap()),
        "the symbol atom must outlive its creating context while rooted"
    );
    drop(primitive);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);

    let wrapper = {
        let mut context = runtime.new_context();
        let object_prototype = context.object_prototype().unwrap();
        let object_value_of =
            property_callable(&runtime, &mut context, &object_prototype, "valueOf");
        let symbol = runtime
            .new_symbol(Some(JsString::try_from_utf8("wrapper-only").unwrap()))
            .unwrap();
        expect_object(
            context
                .call(&object_value_of, Value::Symbol(symbol), &[])
                .unwrap(),
            "Object.prototype.valueOf Symbol wrapper",
        )
    };
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        1,
        "a live Symbol wrapper must retain its prototype and defining realm graph"
    );
    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().live,
        0,
        "Symbol contexts, prototypes, native functions, wrappers, and atoms must collect"
    );
}

fn well_known_entries() -> [(&'static str, WellKnownSymbol); 13] {
    [
        ("toPrimitive", WellKnownSymbol::ToPrimitive),
        ("iterator", WellKnownSymbol::Iterator),
        ("match", WellKnownSymbol::Match),
        ("matchAll", WellKnownSymbol::MatchAll),
        ("replace", WellKnownSymbol::Replace),
        ("search", WellKnownSymbol::Search),
        ("split", WellKnownSymbol::Split),
        ("toStringTag", WellKnownSymbol::ToStringTag),
        ("isConcatSpreadable", WellKnownSymbol::IsConcatSpreadable),
        ("hasInstance", WellKnownSymbol::HasInstance),
        ("species", WellKnownSymbol::Species),
        ("unscopables", WellKnownSymbol::Unscopables),
        ("asyncIterator", WellKnownSymbol::AsyncIterator),
    ]
}

fn define_global(runtime: &Runtime, global: &ObjectRef, name: &str, value: Value) -> PropertyKey {
    let key = runtime.intern_property_key(name).unwrap();
    define_data_key(runtime, global, &key, value, true, true, true);
    key
}

fn define_data(
    runtime: &Runtime,
    object: &ObjectRef,
    name: &str,
    value: Value,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) {
    define_data_key(
        runtime,
        object,
        &runtime.intern_property_key(name).unwrap(),
        value,
        writable,
        enumerable,
        configurable,
    );
}

fn define_data_key(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
    value: Value,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) {
    assert!(
        runtime
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
            .unwrap(),
        "host data-property definition was rejected"
    );
}

fn define_accessor(
    runtime: &Runtime,
    object: &ObjectRef,
    name: &str,
    get: Option<CallableRef>,
    set: Option<CallableRef>,
) {
    define_accessor_key(
        runtime,
        object,
        &runtime.intern_property_key(name).unwrap(),
        get,
        set,
    );
}

fn define_accessor_key(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
    get: Option<CallableRef>,
    set: Option<CallableRef>,
) {
    assert!(
        runtime
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(
                        get.map_or(AccessorValue::Undefined, AccessorValue::Callable),
                    ),
                    set: DescriptorField::Present(
                        set.map_or(AccessorValue::Undefined, AccessorValue::Callable),
                    ),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap(),
        "host accessor-property definition was rejected"
    );
}

fn eval_callable(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let value = match context.eval(source) {
        Ok(value) => value,
        Err(RuntimeError::Exception) => {
            panic!(
                "callable source threw: {source:?}: {}",
                take_error(runtime, context)
            )
        }
        Err(error) => panic!("callable source failed: {source:?}: {error}"),
    };
    let Value::Object(object) = value else {
        panic!("callable source did not produce an object: {source:?}");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("source did not produce a callable: {source:?}"))
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    property_callable_key(
        runtime,
        context,
        object,
        &runtime.intern_property_key(name).unwrap(),
        name,
    )
}

fn property_callable_key(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    key: &PropertyKey,
    name: &str,
) -> CallableRef {
    let Value::Object(value) = context.get_property(object, key).unwrap() else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn accessor_getter(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey) -> CallableRef {
    let descriptor = runtime
        .get_own_property(object, key)
        .unwrap()
        .expect("accessor property was absent");
    let CompleteOrdinaryPropertyDescriptor::Accessor { get: Some(get), .. } = descriptor else {
        panic!("property was not a getter accessor");
    };
    get
}

fn conversion_object(runtime: &Runtime, context: &mut Context, source: &str) -> ObjectRef {
    let object = context.new_object().unwrap();
    let conversion = eval_callable(runtime, context, source);
    define_data_key(
        runtime,
        &object,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(conversion.as_object().clone()),
        true,
        false,
        true,
    );
    object
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .iter()
        .map(|key| {
            well_known_entries()
                .into_iter()
                .find_map(|(_, symbol)| {
                    let symbol_key = PropertyKey::from(runtime.well_known_symbol(symbol));
                    (key == &symbol_key).then(|| format!("Symbol({})", symbol.description()))
                })
                .unwrap_or_else(|| {
                    runtime
                        .property_key_to_js_string(key)
                        .unwrap()
                        .to_utf8_lossy()
                })
        })
        .collect()
}

fn data_flags(runtime: &Runtime, object: &ObjectRef, name: &str) -> String {
    data_flags_key(runtime, object, &runtime.intern_property_key(name).unwrap())
}

fn data_flags_key(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey) -> String {
    let descriptor = runtime
        .get_own_property(object, key)
        .unwrap()
        .expect("property descriptor was absent");
    let CompleteOrdinaryPropertyDescriptor::Data {
        writable,
        enumerable,
        configurable,
        ..
    } = descriptor
    else {
        panic!("property descriptor was not data");
    };
    format!(
        "{}{}{}",
        u8::from(writable),
        u8::from(enumerable),
        u8::from(configurable)
    )
}

fn accessor_flags(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    key: &PropertyKey,
) -> String {
    let descriptor = runtime
        .get_own_property(object, key)
        .unwrap()
        .expect("accessor descriptor was absent");
    let CompleteOrdinaryPropertyDescriptor::Accessor {
        get,
        set,
        enumerable,
        configurable,
    } = descriptor
    else {
        panic!("property descriptor was not an accessor");
    };
    let callable = |value: Option<CallableRef>, context: &mut Context| {
        value.map_or_else(
            || "-".to_owned(),
            |callable| {
                let name = render_value(
                    runtime,
                    context
                        .get_property(
                            callable.as_object(),
                            &runtime.intern_property_key("name").unwrap(),
                        )
                        .unwrap(),
                );
                let length = render_value(
                    runtime,
                    context
                        .get_property(
                            callable.as_object(),
                            &runtime.intern_property_key("length").unwrap(),
                        )
                        .unwrap(),
                );
                format!("{name}/{length}")
            },
        )
    };
    format!(
        "{}{}:{}:{}",
        u8::from(enumerable),
        u8::from(configurable),
        callable(get, context),
        callable(set, context),
    )
}

fn function_signature(runtime: &Runtime, context: &mut Context, callable: &CallableRef) -> String {
    let name = render_value(
        runtime,
        context
            .get_property(
                callable.as_object(),
                &runtime.intern_property_key("name").unwrap(),
            )
            .unwrap(),
    );
    let length = render_value(
        runtime,
        context
            .get_property(
                callable.as_object(),
                &runtime.intern_property_key("length").unwrap(),
            )
            .unwrap(),
    );
    format!(
        "{name}:{length}:{}",
        own_key_names(runtime, callable.as_object()).join(",")
    )
}

fn call_symbol(context: &mut Context, callable: &CallableRef, arguments: &[Value]) -> SymbolRef {
    let Value::Symbol(value) = context.call(callable, Value::Undefined, arguments).unwrap() else {
        panic!("Symbol-producing call did not return a Symbol");
    };
    value
}

fn observe_symbol_call(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    to_string: &CallableRef,
    this_value: Value,
    arguments: &[Value],
) -> String {
    match context.call(callable, this_value, arguments) {
        Ok(Value::Symbol(value)) => format!(
            "symbol:{}",
            symbol_record_via_to_string(runtime, context, to_string, &value)
        ),
        Ok(value) => panic!("Symbol observation returned {value:?}"),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("Symbol call failed outside JavaScript completion: {error}"),
    }
}

fn observe_call(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    this_value: Value,
    arguments: &[Value],
) -> String {
    match context.call(callable, this_value, arguments) {
        Ok(value) => render_value(runtime, value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("Rust call failed outside JavaScript completion: {error}"),
    }
}

fn observe_construct(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    arguments: &[Value],
) -> String {
    match context.construct(callable, arguments) {
        Ok(value) => render_value(runtime, value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("Rust construct failed outside JavaScript completion: {error}"),
    }
}

fn observe_construct_with_new_target(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    new_target: &CallableRef,
    arguments: &[Value],
) -> String {
    match context.construct_with_new_target(callable, new_target, arguments) {
        Ok(value) => render_value(runtime, value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("Rust construct failed outside JavaScript completion: {error}"),
    }
}

fn observe_eval(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    match context.eval(source) {
        Ok(value) => render_value(runtime, value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => {
            panic!("Rust source failed outside JavaScript completion: {source:?}: {error}")
        }
    }
}

fn symbol_record(runtime: &Runtime, symbol: &SymbolRef) -> String {
    let description = runtime.symbol_description(symbol).unwrap();
    let description_units = description
        .as_ref()
        .map_or_else(|| "undefined".to_owned(), utf16_units);
    let text = JsString::try_from_utf8("Symbol(")
        .unwrap()
        .try_concat(&description.unwrap_or_else(|| JsString::try_from_utf8("").unwrap()))
        .unwrap()
        .try_concat(&JsString::try_from_utf8(")").unwrap())
        .unwrap();
    format!("{description_units}/{}", utf16_units(&text))
}

fn symbol_record_via_to_string(
    runtime: &Runtime,
    context: &mut Context,
    to_string: &CallableRef,
    symbol: &SymbolRef,
) -> String {
    let description_units = symbol_description_units(runtime, symbol);
    let value = context
        .call(to_string, Value::Symbol(symbol.clone()), &[])
        .expect("Symbol.prototype.toString must accept a Symbol primitive");
    let Value::String(text) = value else {
        panic!("Symbol.prototype.toString did not return a string");
    };
    format!("{description_units}/{}", utf16_units(&text))
}

fn symbol_description_units(runtime: &Runtime, symbol: &SymbolRef) -> String {
    runtime
        .symbol_description(symbol)
        .unwrap()
        .as_ref()
        .map_or_else(|| "undefined".to_owned(), utf16_units)
}

fn utf16_units(value: &JsString) -> String {
    value
        .utf16_units()
        .map(|unit| format!("{unit:04x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn symbol_tags(
    runtime: &Runtime,
    context: &mut Context,
    object_to_string: &CallableRef,
    prototype: &ObjectRef,
    wrapper: &ObjectRef,
    primitive: &SymbolRef,
) -> Vec<String> {
    [
        Value::Symbol(primitive.clone()),
        Value::Object(wrapper.clone()),
        Value::Object(prototype.clone()),
    ]
    .into_iter()
    .map(|value| render_value(runtime, context.call(object_to_string, value, &[]).unwrap()))
    .collect()
}

fn install_primitive_accessors(
    runtime: &Runtime,
    context: &mut Context,
    global: &ObjectRef,
    prototype: &ObjectRef,
) {
    for name in [
        "strictGetThis",
        "sloppyGetThis",
        "strictSetThis",
        "sloppySetThis",
    ] {
        define_global(runtime, global, name, Value::Undefined);
    }
    let strict_get = eval_callable(
        runtime,
        context,
        r#"(function() { "use strict"; strictGetThis = this; return this; })"#,
    );
    let sloppy_get = eval_callable(
        runtime,
        context,
        "(function() { sloppyGetThis = this; return this.valueOf(); })",
    );
    let strict_set = eval_callable(
        runtime,
        context,
        r#"(function() { "use strict"; strictSetThis = this; })"#,
    );
    let sloppy_set = eval_callable(runtime, context, "(function() { sloppySetThis = this; })");
    define_accessor(runtime, prototype, "__strictGet", Some(strict_get), None);
    define_accessor(runtime, prototype, "__sloppyGet", Some(sloppy_get), None);
    define_accessor(runtime, prototype, "__strictSet", None, Some(strict_set));
    define_accessor(runtime, prototype, "__sloppySet", None, Some(sloppy_set));
}

fn global_value(runtime: &Runtime, context: &mut Context, global: &ObjectRef, name: &str) -> Value {
    context
        .get_property(global, &runtime.intern_property_key(name).unwrap())
        .unwrap()
}

fn object_has_prototype(runtime: &Runtime, value: &Value, prototype: &ObjectRef) -> bool {
    let Value::Object(object) = value else {
        return false;
    };
    runtime.get_prototype_of(object).unwrap() == Some(prototype.clone())
}

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(object) = value else {
        panic!("{description} did not produce an object");
    };
    object
}

fn expect_symbol(value: Value, description: &str) -> SymbolRef {
    let Value::Symbol(symbol) = value else {
        panic!("{description} did not produce a Symbol");
    };
    symbol
}

fn expect_string(value: Value, description: &str) -> JsString {
    let Value::String(value) = value else {
        panic!("{description} did not produce a string");
    };
    value
}

fn render_value(runtime: &Runtime, value: Value) -> String {
    match value {
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Symbol(value) => format!("symbol:{}", symbol_record(runtime, &value)),
        value => value
            .to_js_string()
            .expect("ordinary Symbol observation must stringify")
            .to_utf8_lossy(),
    }
}

fn take_error(runtime: &Runtime, context: &mut Context) -> String {
    let error = take_exception_object(context);
    let name = error_text(runtime, context, &error, "name");
    let message = error_text(runtime, context, &error, "message");
    format!("throw:{name}:{message}")
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Symbol operation did not throw an Error object");
    };
    error
}

fn error_text(runtime: &Runtime, context: &mut Context, error: &ObjectRef, name: &str) -> String {
    let Value::String(value) = context
        .get_property(error, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("Error.{name} was not a string");
    };
    value.to_utf8_lossy()
}

fn intrinsic_prototype(runtime: &Runtime, context: &mut Context, name: &str) -> ObjectRef {
    let global = context.global_object().unwrap();
    let constructor = property_callable(runtime, context, &global, name);
    let Value::Object(prototype) = context
        .get_property(
            constructor.as_object(),
            &runtime.intern_property_key("prototype").unwrap(),
        )
        .unwrap()
    else {
        panic!("{name}.prototype was not an object");
    };
    prototype
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS Symbol intrinsic oracle");
    assert!(
        output.status.success(),
        "QuickJS Symbol intrinsic oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Symbol intrinsic oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
