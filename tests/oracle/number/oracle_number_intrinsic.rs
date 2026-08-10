use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    JsBigInt, JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError,
    Value, WellKnownSymbol,
};

// The oracle is intentionally broader than the source subset currently parsed
// by quickjs-oxide. The Rust side mirrors Object/Reflect/Array/DataView work
// through the public host API, while source evaluation is reserved for the
// primitive-reference operations whose strict/sloppy receiver behavior is the
// subject of the probe.
const ORACLE_PROBE: &str = r#"
function flags(object, key) {
    var descriptor = Object.getOwnPropertyDescriptor(object, key);
    return (descriptor.writable ? "1" : "0") +
           (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0");
}
function observe(thunk) {
    try { return String(thunk()); }
    catch (error) { return "throw:" + error.name + ":" + error.message; }
}
function signature(fn) {
    return fn.name + ":" + fn.length + ":" + Reflect.ownKeys(fn).map(String).join(",");
}
function isConstructor(fn) {
    try { Reflect.construct(function() {}, [], fn); return true; }
    catch (_) { return false; }
}
var bitStorage = new ArrayBuffer(8);
var bitView = new DataView(bitStorage);
function bits(value) {
    if (Number.isNaN(value)) return "NaN";
    bitView.setFloat64(0, value, true);
    var text = bitView.getBigUint64(0, true).toString(16);
    return ("0000000000000000" + text).slice(-16);
}

var conversionLog = "";
var numericObject = {};
Object.defineProperty(numericObject, Symbol.toPrimitive, {
    value: function(hint) { conversionLog = conversionLog + hint + ","; return "42.5"; },
    configurable: true
});
print("call=" + [
    Number(), Number(undefined), Number(null), Number(false), Number(true),
    Number(0), 1 / Number(-0), Number(NaN), Number(""), Number(" 12.5 "),
    Number("0x10"), Number(0n), Number(9007199254740993n),
    Number(numericObject), conversionLog,
    observe(function() { return Number(Symbol("number")); })
].join("|"));

conversionLog = "";
var boxedZero = new Number();
var boxedNegativeZero = new Number(-0);
var boxedBigInt = new Number(9007199254740993n);
var boxedObject = new Number(numericObject);
print("new=" + [
    typeof boxedZero,
    Object.getPrototypeOf(boxedZero) === Number.prototype,
    Object.prototype.toString.call(boxedZero), boxedZero.valueOf(),
    Reflect.ownKeys(boxedZero).length,
    1 / boxedNegativeZero.valueOf(), boxedBigInt.valueOf(), boxedObject.valueOf(),
    conversionLog,
    observe(function() { return new Number(Symbol("number")); })
].join("|"));

var boxedSeven = new Number(7);
print("coercion=" + [
    Number(boxedSeven), +boxedSeven, boxedSeven + 1,
    boxedSeven == 7, boxedSeven === 7,
    boxedSeven.valueOf(), boxedSeven.toString(),
    Object.prototype.valueOf.call(boxedSeven) === boxedSeven
].join("|"));

var constructorKeys = Reflect.ownKeys(Number);
var prototypeKeys = Reflect.ownKeys(Number.prototype);
print("ctor-keys=" + constructorKeys.map(String).join(","));
print("proto-keys=" + prototypeKeys.map(String).join(","));
print("global-order=" + Reflect.ownKeys(globalThis).filter(function(key) {
    return key === "parseInt" || key === "parseFloat" ||
           key === "Infinity" || key === "NaN" || key === "undefined" ||
           key === "Number" || key === "Boolean";
}).map(String).join(","));
print("ctor-descriptors=" + [flags(globalThis, "Number")].concat(
    constructorKeys.map(function(key) { return flags(Number, key); })
).join("|"));
print("proto-descriptors=" + prototypeKeys.map(function(key) {
    return flags(Number.prototype, key);
}).join("|"));
print("graph=" + [
    typeof Number,
    Object.getPrototypeOf(Number) === Function.prototype,
    Number.prototype.constructor === Number,
    Object.getPrototypeOf(Number.prototype) === Object.prototype,
    Object.prototype.toString.call(Number.prototype),
    Number.prototype.valueOf(),
    Object.isExtensible(Number.prototype),
    Number.parseInt === parseInt, Number.parseFloat === parseFloat
].join("|"));
var numberCallables = [
    Number,
    Number.parseInt, Number.parseFloat,
    Number.isNaN, Number.isFinite, Number.isInteger, Number.isSafeInteger,
    Number.prototype.toExponential, Number.prototype.toFixed,
    Number.prototype.toPrecision, Number.prototype.toString,
    Number.prototype.toLocaleString, Number.prototype.valueOf
];
print("signatures=" + numberCallables.map(signature).join("|"));
print("callable-graph=" + numberCallables.map(function(fn) {
    return (Object.getPrototypeOf(fn) === Function.prototype) + ":" +
           flags(fn, "length") + ":" + flags(fn, "name") + ":" +
           isConstructor(fn);
}).join("|"));

var predicateBombHit = false;
var predicateBomb = {};
Object.defineProperty(predicateBomb, Symbol.toPrimitive, {
    value: function() { predicateBombHit = true; throw new Error("predicate coerced"); },
    configurable: true
});
print("predicates=" + [
    Number.isNaN(), Number.isNaN(undefined), Number.isNaN("NaN"),
    Number.isNaN(NaN), Number.isNaN(Infinity), Number.isNaN(0),
    Number.isFinite(), Number.isFinite(null), Number.isFinite("1"),
    Number.isFinite(0), Number.isFinite(Number.MAX_VALUE), Number.isFinite(Infinity),
    Number.isInteger(), Number.isInteger("1"), Number.isInteger(NaN),
    Number.isInteger(Infinity), Number.isInteger(-0), Number.isInteger(1),
    Number.isInteger(1.5), Number.isInteger(9007199254740992),
    Number.isSafeInteger(), Number.isSafeInteger("1"), Number.isSafeInteger(NaN),
    Number.isSafeInteger(Infinity), Number.isSafeInteger(-0),
    Number.isSafeInteger(Number.MAX_SAFE_INTEGER),
    Number.isSafeInteger(Number.MIN_SAFE_INTEGER),
    Number.isSafeInteger(9007199254740992),
    Number.isNaN(1n), Number.isFinite(Symbol("predicate")),
    Number.isInteger(predicateBomb), Number.isSafeInteger(predicateBomb), predicateBombHit
    , Number.isNaN(new Number(NaN)), Number.isFinite(new Number(0)),
    Number.isInteger(new Number(1)), Number.isSafeInteger(new Number(1))
].join("|"));

print("constants=" + [
    Number.MAX_VALUE, Number.MIN_VALUE, Number.NaN,
    Number.NEGATIVE_INFINITY, Number.POSITIVE_INFINITY, Number.EPSILON,
    Number.MAX_SAFE_INTEGER, Number.MIN_SAFE_INTEGER
].map(bits).join("|"));

print("methods=" + [
    Number.prototype.valueOf.call(12.5),
    (123.5).toString(), (-255).toString(16), (10).toString(2), (35).toString(36),
    (-0).toString(), (NaN).toString(), (Infinity).toString(),
    (123.5).toLocaleString(),
    (1.25).toFixed(1), (1.005).toFixed(2), (-0).toFixed(2), (1e21).toFixed(2),
    (123).toExponential(), (123).toExponential(0),
    (1.25).toExponential(1), (0).toExponential(2),
    (123.45).toPrecision(), (123.45).toPrecision(4),
    (0.0000012345).toPrecision(3), (1234567).toPrecision(3)
].join("|"));

print("rounding=" + [
    (25).toExponential(0), (-25).toExponential(0),
    (2.5).toPrecision(1), (-2.5).toPrecision(1),
    (1.125).toFixed(2), (-1.125).toFixed(2),
    (0.5).toFixed(0), (-0.5).toFixed(0), (-1e-10).toFixed(0),
    (1 - Math.pow(2, -53)).toString(12),
    (1.3).toString(7), (1.3).toString(35)
].join("|"));
print("valid-max=" + [
    (1).toFixed(100), (1).toExponential(100), (1).toPrecision(100)
].join("|"));

print("method-edges=" + [
    (10).toString(undefined), (10).toString(10),
    (1.5).toFixed(), (NaN).toFixed(2), (Infinity).toFixed(2),
    (-Infinity).toFixed(2),
    (NaN).toExponential(101), (Infinity).toExponential(101),
    (-Infinity).toExponential(-1),
    (NaN).toPrecision(0), (Infinity).toPrecision(0),
    (-Infinity).toPrecision(101),
    Number.prototype.toLocaleString.call(1.25, Symbol("ignored"))
].join("|"));

print("ranges=" + [
    observe(function() { return (1).toString(1); }),
    observe(function() { return (1).toString(37); }),
    observe(function() { return (1).toString(NaN); }),
    observe(function() { return (1).toFixed(-1); }),
    observe(function() { return (1).toFixed(101); }),
    observe(function() { return (1).toExponential(-1); }),
    observe(function() { return (1).toExponential(101); }),
    observe(function() { return (1).toPrecision(0); }),
    observe(function() { return (1).toPrecision(101); }),
    observe(function() { return (NaN).toFixed(101); }),
    observe(function() { return (Infinity).toFixed(-1); }),
    observe(function() { return (1).toString(Symbol("radix")); }),
    observe(function() { return (1).toFixed(1n); }),
    observe(function() { return (1).toExponential(1n); }),
    observe(function() { return (1).toPrecision(1n); })
].join("|"));

var numberSpoof = Object.create(Number.prototype);
print("brands=" + [
    observe(function() { return Number.prototype.valueOf.call("1"); }),
    observe(function() { return Number.prototype.toString.call("1"); }),
    observe(function() { return Number.prototype.toLocaleString.call("1"); }),
    observe(function() { return Number.prototype.toFixed.call("1"); }),
    observe(function() { return Number.prototype.toExponential.call("1"); }),
    observe(function() { return Number.prototype.toPrecision.call("1"); }),
    observe(function() { return Number.prototype.valueOf.call(numberSpoof); })
].join("|"));

var formatLog = "";
var formatArgument = {};
Object.defineProperty(formatArgument, Symbol.toPrimitive, {
    value: function(hint) { formatLog = formatLog + hint + ","; return 2; },
    configurable: true
});
var convertedFormats = [
    Number.prototype.toString.call(10, formatArgument),
    Number.prototype.toFixed.call(1.25, formatArgument),
    Number.prototype.toExponential.call(1.25, formatArgument),
    Number.prototype.toPrecision.call(1.25, formatArgument)
].join("|");
var convertedFormatLog = formatLog;
formatLog = "";
var badThisBeforeArgument = observe(function() {
    return Number.prototype.toFixed.call("1", formatArgument);
});
var badThisLog = formatLog;
formatLog = "";
var ignoredLocaleArgument = Number.prototype.toLocaleString.call(1.25, formatArgument);
print("conversion-order=" + [
    convertedFormats, convertedFormatLog,
    badThisBeforeArgument, badThisLog,
    ignoredLocaleArgument, formatLog
].join("|"));

var objectBoxA = Object.prototype.valueOf.call(3);
var objectBoxB = Object.prototype.valueOf.call(3);
print("object-links=" + [
    Object.prototype.toString.call(3),
    Object.getPrototypeOf(objectBoxA) === Number.prototype,
    Number.prototype.valueOf.call(objectBoxA),
    objectBoxA === objectBoxB,
    Object.prototype.toLocaleString.call(3)
].join("|"));

var directStrictThis;
var directSloppyThis;
Object.defineProperty(Number.prototype, "__strictMethod", {
    configurable: true,
    value: function() { "use strict"; directStrictThis = this; return this; }
});
Object.defineProperty(Number.prototype, "__sloppyMethod", {
    configurable: true,
    value: function() { directSloppyThis = this; return this.valueOf(); }
});
var directStrictResult = (7).__strictMethod();
var directSloppyResultA = (7).__sloppyMethod();
var directSloppyThisA = directSloppyThis;
var directSloppyResultB = (7).__sloppyMethod();
var directSloppyThisB = directSloppyThis;
print("method-receivers=" + [
    directStrictResult, directStrictThis === 7, typeof directStrictThis,
    directSloppyResultA, typeof directSloppyThisA,
    Object.getPrototypeOf(directSloppyThisA) === Number.prototype,
    directSloppyThisA.valueOf(), directSloppyResultB,
    directSloppyThisA === directSloppyThisB
].join("|"));

var strictGetThis;
var sloppyGetThis;
var strictSetThis;
var strictSetValue;
var sloppySetThis;
var sloppySetValue;
Object.defineProperty(Number.prototype, "__strictGet", {
    configurable: true,
    get: function() { "use strict"; strictGetThis = this; return this; }
});
Object.defineProperty(Number.prototype, "__sloppyGet", {
    configurable: true,
    get: function() { sloppyGetThis = this; return this.valueOf(); }
});
Object.defineProperty(Number.prototype, "__strictSet", {
    configurable: true,
    set: function(value) { "use strict"; strictSetThis = this; strictSetValue = value; }
});
Object.defineProperty(Number.prototype, "__sloppySet", {
    configurable: true,
    set: function(value) { sloppySetThis = this; sloppySetValue = value; }
});
var strictGetResult = (3).__strictGet;
var sloppyGetResult = (3).__sloppyGet;
var strictSetResult = (3).__strictSet = 7;
var sloppySetResult = (3).__sloppySet = 8;
print("accessors=" + [
    strictGetResult, strictGetThis === 3,
    sloppyGetResult, typeof sloppyGetThis,
    Object.getPrototypeOf(sloppyGetThis) === Number.prototype, sloppyGetThis.valueOf(),
    strictSetResult, strictSetThis === 3, strictSetValue,
    sloppySetResult, typeof sloppySetThis,
    Object.getPrototypeOf(sloppySetThis) === Number.prototype,
    sloppySetThis.valueOf(), sloppySetValue
].join("|"));

var getterHit = false;
var deleteHit = false;
Object.defineProperty(Number.prototype, "__rw", {
    value: 1, writable: true, configurable: true
});
Object.defineProperty(Number.prototype, "__ro", {
    value: 1, writable: false, configurable: true
});
Object.defineProperty(Number.prototype, "__getterOnly", {
    configurable: true,
    get: function() { getterHit = true; return 1; }
});
Object.defineProperty(Number.prototype, "__delete", {
    configurable: true,
    get: function() { deleteHit = true; return 1; }
});
var sloppyRw = (3).__rw = 2;
var sloppyRo = (3).__ro = 2;
var sloppyGetterOnly = (3).__getterOnly = 2;
var deleted = delete (3).__delete;
print("writes=" + [
    sloppyRw, (3).__rw,
    observe(function() { "use strict"; return (3).__rw = 3; }),
    sloppyRo, (3).__ro,
    observe(function() { "use strict"; return (3).__ro = 3; }),
    sloppyGetterOnly,
    observe(function() { "use strict"; return (3).__getterOnly = 3; }),
    getterHit, deleted, deleteHit,
    Object.prototype.hasOwnProperty.call(Number.prototype, "__delete"),
    (function() { "use strict"; return delete (3).__delete; })()
].join("|"));

print("tag-before=" + [
    Object.prototype.toString.call(3),
    Object.prototype.toString.call(boxedSeven),
    Object.prototype.toString.call(Number.prototype),
    Object.prototype.hasOwnProperty.call(Number.prototype, Symbol.toStringTag)
].join("|"));
var originalNumberToStringDescriptor =
    Object.getOwnPropertyDescriptor(Number.prototype, "toString");
var tagThis;
Object.defineProperty(Number.prototype, Symbol.toStringTag, {
    configurable: true,
    get: function() { "use strict"; tagThis = this; return "CustomNumber"; }
});
var localeGetterThis;
var localeCallThis;
function localeMethod() { "use strict"; localeCallThis = this; return this; }
Object.defineProperty(Number.prototype, "toString", {
    configurable: true,
    get: function() { "use strict"; localeGetterThis = this; return localeMethod; }
});
var customTag = Object.prototype.toString.call(3);
var customLocale = Object.prototype.toLocaleString.call(3);
print("custom-object-methods=" + [
    customTag, typeof tagThis, tagThis instanceof Number, tagThis.valueOf(),
    customLocale, localeGetterThis === 3, localeCallThis === 3
].join("|"));
var boxedCustomTag = Object.prototype.toString.call(boxedSeven);
var boxedTagThis = tagThis;
var prototypeCustomTag = Object.prototype.toString.call(Number.prototype);
var prototypeTagThis = tagThis;
print("tag-getter-detail=" + [
    boxedCustomTag, boxedTagThis === boxedSeven, boxedTagThis.valueOf(),
    prototypeCustomTag, prototypeTagThis === Number.prototype,
    prototypeTagThis.valueOf()
].join("|"));
Object.defineProperty(boxedSeven, Symbol.toStringTag, {
    value: "OwnNumber", configurable: true
});
tagThis = undefined;
print("tag-own=" + [
    Object.prototype.toString.call(boxedSeven), tagThis === undefined,
    boxedSeven.valueOf()
].join("|"));
Object.defineProperty(Number.prototype, Symbol.toStringTag, {
    value: 123, configurable: true
});
print("tag-nonstring=" + [
    Object.prototype.toString.call(3),
    Object.prototype.toString.call(boxedSeven)
].join("|"));

delete boxedSeven[Symbol.toStringTag];
delete Number.prototype[Symbol.toStringTag];
Object.defineProperty(Number.prototype, "toString", originalNumberToStringDescriptor);
[
    "__strictMethod", "__sloppyMethod",
    "__strictGet", "__sloppyGet", "__strictSet", "__sloppySet",
    "__rw", "__ro", "__getterOnly", "__delete"
].forEach(function(key) { delete Number.prototype[key]; });
print("tag-restored=" + [
    Object.prototype.toString.call(3),
    Object.prototype.toString.call(boxedSeven),
    Reflect.ownKeys(Number.prototype).map(String).join(",")
].join("|"));

var capturedParseInt = Number.parseInt;
var capturedParseFloat = Number.parseFloat;
var originalGlobalParseInt = parseInt;
var originalGlobalParseFloat = parseFloat;
globalThis.parseInt = function() { return 99; };
globalThis.parseFloat = function() { return 88; };
print("alias-capture=" + [
    capturedParseInt === originalGlobalParseInt,
    Number.parseInt === parseInt,
    Number.parseInt("10", 2), parseInt("10", 2),
    capturedParseFloat === originalGlobalParseFloat,
    Number.parseFloat === parseFloat,
    Number.parseFloat("1.5tail"), parseFloat("1.5tail")
].join("|"));
"#;

#[test]
fn number_intrinsic_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Number intrinsic differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let Some(rust) = rust_observations() else {
        eprintln!("SKIP Number intrinsic differential: Number is not published yet");
        return;
    };
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "Number intrinsic behavior differed from pinned QuickJS"
    );
}

#[test]
fn number_native_error_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Number native stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (description, source) in [
        (
            "brand failure through Function.prototype.call",
            "Number.prototype.toFixed.call(Function, 0)",
        ),
        ("toFixed range", "(1).toFixed(-1)"),
        ("toExponential range", "(1).toExponential(101)"),
        ("toPrecision range", "(1).toPrecision(0)"),
        ("toString radix range", "(1).toString(1)"),
        ("BigInt digits conversion", "(1).toFixed(1n)"),
        ("predicate is not constructable", "new Number.isNaN()"),
        ("parse alias is not constructable", "new Number.parseInt()"),
        (
            "formatter is not constructable",
            "new Number.prototype.toFixed()",
        ),
        (
            "valueOf is not constructable",
            "new Number.prototype.valueOf()",
        ),
    ] {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), source, description);
        let quickjs = run_cli(&oracle, source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}

fn rust_observations() -> Option<Vec<String>> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let number_key = runtime.intern_property_key("Number").unwrap();
    let Value::Object(number_object) = context.get_property(&global, &number_key).unwrap() else {
        return None;
    };
    let Some(number) = runtime.as_callable(&number_object).unwrap() else {
        panic!("global Number was not callable");
    };
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(number_prototype) = context
        .get_property(&number_object, &prototype_key)
        .unwrap()
    else {
        panic!("Number.prototype was not an object");
    };
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let object_to_locale_string =
        property_callable(&runtime, &mut context, &object_prototype, "toLocaleString");
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    let parse_int = property_callable(&runtime, &mut context, &global, "parseInt");
    let parse_float = property_callable(&runtime, &mut context, &global, "parseFloat");
    let number_parse_int = property_callable(&runtime, &mut context, &number_object, "parseInt");
    let number_parse_float =
        property_callable(&runtime, &mut context, &number_object, "parseFloat");
    let is_nan = property_callable(&runtime, &mut context, &number_object, "isNaN");
    let is_finite = property_callable(&runtime, &mut context, &number_object, "isFinite");
    let is_integer = property_callable(&runtime, &mut context, &number_object, "isInteger");
    let is_safe_integer =
        property_callable(&runtime, &mut context, &number_object, "isSafeInteger");
    let to_exponential =
        property_callable(&runtime, &mut context, &number_prototype, "toExponential");
    let to_fixed = property_callable(&runtime, &mut context, &number_prototype, "toFixed");
    let to_precision = property_callable(&runtime, &mut context, &number_prototype, "toPrecision");
    let to_string = property_callable(&runtime, &mut context, &number_prototype, "toString");
    let to_locale_string =
        property_callable(&runtime, &mut context, &number_prototype, "toLocaleString");
    let value_of = property_callable(&runtime, &mut context, &number_prototype, "valueOf");

    let conversion_log = define_global(
        &runtime,
        &global,
        "conversionLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let numeric_object = context.new_object().unwrap();
    let numeric_conversion = eval_callable(
        &runtime,
        &mut context,
        r#"(function(hint) { conversionLog = conversionLog + hint + ","; return "42.5"; })"#,
    );
    define_data_key(
        &runtime,
        &numeric_object,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(numeric_conversion.as_object().clone()),
        true,
        false,
        true,
    );
    let big_rounded = JsBigInt::parse_js_string("9007199254740993").unwrap();
    let number_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("number").unwrap()))
        .unwrap();
    let negative_zero = call_one(&mut context, &number, Value::Float(-0.0));
    let call_values = [
        context.call(&number, Value::Undefined, &[]).unwrap(),
        call_one(&mut context, &number, Value::Undefined),
        call_one(&mut context, &number, Value::Null),
        call_one(&mut context, &number, Value::Bool(false)),
        call_one(&mut context, &number, Value::Bool(true)),
        call_one(&mut context, &number, Value::Int(0)),
        reciprocal(negative_zero),
        call_one(&mut context, &number, Value::Float(f64::NAN)),
        call_one(
            &mut context,
            &number,
            Value::String(JsString::try_from_utf8("").unwrap()),
        ),
        call_one(
            &mut context,
            &number,
            Value::String(JsString::try_from_utf8(" 12.5 ").unwrap()),
        ),
        call_one(
            &mut context,
            &number,
            Value::String(JsString::try_from_utf8("0x10").unwrap()),
        ),
        call_one(&mut context, &number, Value::BigInt(JsBigInt::zero())),
        call_one(&mut context, &number, Value::BigInt(big_rounded.clone())),
        call_one(&mut context, &number, Value::Object(numeric_object.clone())),
        context.get_property(&global, &conversion_log).unwrap(),
    ];
    let mut observations = vec![format!(
        "call={}|{}",
        join_values(&call_values),
        observe_call_args(
            &runtime,
            &mut context,
            &number,
            Value::Undefined,
            &[Value::Symbol(number_symbol.clone())],
        )
    )];

    assert!(
        context
            .set_property(
                &global,
                &conversion_log,
                Value::String(JsString::try_from_utf8("").unwrap()),
            )
            .unwrap()
    );
    let boxed_zero = expect_object(context.construct(&number, &[]).unwrap(), "new Number()");
    let boxed_negative_zero = expect_object(
        context.construct(&number, &[Value::Float(-0.0)]).unwrap(),
        "new Number(-0)",
    );
    let boxed_bigint = expect_object(
        context
            .construct(&number, &[Value::BigInt(big_rounded)])
            .unwrap(),
        "new Number(bigint)",
    );
    let boxed_object = expect_object(
        context
            .construct(&number, &[Value::Object(numeric_object)])
            .unwrap(),
        "new Number(object)",
    );
    let boxed_negative_value = context
        .call(&value_of, Value::Object(boxed_negative_zero), &[])
        .unwrap();
    let new_values = [
        runtime
            .get_prototype_of(&boxed_zero)
            .unwrap()
            .is_some_and(|prototype| prototype == number_prototype)
            .to_string(),
        plain_value(
            context
                .call(&object_to_string, Value::Object(boxed_zero.clone()), &[])
                .unwrap(),
        ),
        plain_value(
            context
                .call(&value_of, Value::Object(boxed_zero.clone()), &[])
                .unwrap(),
        ),
        runtime
            .own_property_keys(&boxed_zero)
            .unwrap()
            .len()
            .to_string(),
        plain_value(reciprocal(boxed_negative_value)),
        plain_value(
            context
                .call(&value_of, Value::Object(boxed_bigint), &[])
                .unwrap(),
        ),
        plain_value(
            context
                .call(&value_of, Value::Object(boxed_object), &[])
                .unwrap(),
        ),
        plain_value(context.get_property(&global, &conversion_log).unwrap()),
        observe_construct_args(
            &runtime,
            &mut context,
            &number,
            &[Value::Symbol(number_symbol)],
        ),
    ];
    observations.push(format!("new=object|{}", new_values.join("|")));

    let boxed_seven = expect_object(
        context.construct(&number, &[Value::Int(7)]).unwrap(),
        "new Number(7)",
    );
    define_global(
        &runtime,
        &global,
        "boxedSeven",
        Value::Object(boxed_seven.clone()),
    );
    let coercion_sources = [
        "Number(boxedSeven)",
        "+boxedSeven",
        "boxedSeven + 1",
        "boxedSeven == 7",
        "boxedSeven === 7",
        "boxedSeven.valueOf()",
        "boxedSeven.toString()",
    ];
    let mut coercion = coercion_sources
        .into_iter()
        .map(|source| plain_value(context.eval(source).unwrap()))
        .collect::<Vec<_>>();
    coercion.push(
        matches!(
            context
                .call(&object_value_of, Value::Object(boxed_seven.clone()), &[])
                .unwrap(),
            Value::Object(object) if object == boxed_seven
        )
        .to_string(),
    );
    observations.push(format!("coercion={}", coercion.join("|")));

    let constructor_keys = runtime.own_property_keys(&number_object).unwrap();
    let prototype_keys = runtime.own_property_keys(&number_prototype).unwrap();
    observations.push(format!(
        "ctor-keys={}",
        key_names(&runtime, &constructor_keys).join(",")
    ));
    observations.push(format!(
        "proto-keys={}",
        key_names(&runtime, &prototype_keys).join(",")
    ));
    let supported_globals = key_names(&runtime, &runtime.own_property_keys(&global).unwrap())
        .into_iter()
        .filter(|name| {
            matches!(
                name.as_str(),
                "parseInt" | "parseFloat" | "Infinity" | "NaN" | "undefined" | "Number" | "Boolean"
            )
        })
        .collect::<Vec<_>>();
    observations.push(format!("global-order={}", supported_globals.join(",")));
    let mut constructor_descriptors = vec![data_flags(&runtime, &global, "Number")];
    constructor_descriptors.extend(
        constructor_keys
            .iter()
            .map(|key| data_flags_key(&runtime, &number_object, key)),
    );
    observations.push(format!(
        "ctor-descriptors={}",
        constructor_descriptors.join("|")
    ));
    observations.push(format!(
        "proto-descriptors={}",
        prototype_keys
            .iter()
            .map(|key| data_flags_key(&runtime, &number_prototype, key))
            .collect::<Vec<_>>()
            .join("|")
    ));
    let number_prototype_value = context
        .call(&value_of, Value::Object(number_prototype.clone()), &[])
        .unwrap();
    observations.push(format!(
        "graph=function|{}|{}|{}|{}|{}|{}|{}|{}",
        runtime
            .get_prototype_of(&number_object)
            .unwrap()
            .is_some_and(|prototype| prototype == function_prototype),
        matches!(
            context
                .get_property(
                    &number_prototype,
                    &runtime.intern_property_key("constructor").unwrap(),
                )
                .unwrap(),
            Value::Object(object) if object == number_object
        ),
        runtime
            .get_prototype_of(&number_prototype)
            .unwrap()
            .is_some_and(|prototype| prototype == object_prototype),
        plain_value(
            context
                .call(
                    &object_to_string,
                    Value::Object(number_prototype.clone()),
                    &[],
                )
                .unwrap(),
        ),
        plain_value(number_prototype_value),
        runtime.is_extensible(&number_prototype).unwrap(),
        number_parse_int.as_object() == parse_int.as_object(),
        number_parse_float.as_object() == parse_float.as_object(),
    ));
    let number_callables = [
        &number,
        &number_parse_int,
        &number_parse_float,
        &is_nan,
        &is_finite,
        &is_integer,
        &is_safe_integer,
        &to_exponential,
        &to_fixed,
        &to_precision,
        &to_string,
        &to_locale_string,
        &value_of,
    ];
    observations.push(format!(
        "signatures={}",
        number_callables
            .iter()
            .copied()
            .map(|callable| function_signature(&runtime, callable))
            .collect::<Vec<_>>()
            .join("|")
    ));
    observations.push(format!(
        "callable-graph={}",
        number_callables
            .iter()
            .copied()
            .map(|callable| {
                format!(
                    "{}:{}:{}:{}",
                    runtime
                        .get_prototype_of(callable.as_object())
                        .unwrap()
                        .is_some_and(|prototype| prototype == function_prototype),
                    data_flags(&runtime, callable.as_object(), "length"),
                    data_flags(&runtime, callable.as_object(), "name"),
                    runtime.is_constructor(callable.as_object()).unwrap(),
                )
            })
            .collect::<Vec<_>>()
            .join("|")
    ));

    let predicate_bomb_hit =
        define_global(&runtime, &global, "predicateBombHit", Value::Bool(false));
    let predicate_bomb = context.new_object().unwrap();
    let predicate_conversion = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { predicateBombHit = true; throw Error("predicate coerced"); })"#,
    );
    define_data_key(
        &runtime,
        &predicate_bomb,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(predicate_conversion.as_object().clone()),
        true,
        false,
        true,
    );
    let predicate_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("predicate").unwrap()))
        .unwrap();
    let max_value = global_property(&runtime, &mut context, &number_object, "MAX_VALUE");
    let max_safe = global_property(&runtime, &mut context, &number_object, "MAX_SAFE_INTEGER");
    let min_safe = global_property(&runtime, &mut context, &number_object, "MIN_SAFE_INTEGER");
    let boxed_predicate_nan = expect_object(
        context
            .construct(&number, &[Value::Float(f64::NAN)])
            .unwrap(),
        "predicate Number(NaN) wrapper",
    );
    let boxed_predicate_zero = expect_object(
        context.construct(&number, &[Value::Int(0)]).unwrap(),
        "predicate Number(0) wrapper",
    );
    let boxed_predicate_one_a = expect_object(
        context.construct(&number, &[Value::Int(1)]).unwrap(),
        "predicate Number(1) wrapper",
    );
    let boxed_predicate_one_b = expect_object(
        context.construct(&number, &[Value::Int(1)]).unwrap(),
        "second predicate Number(1) wrapper",
    );
    let predicate_values = [
        context.call(&is_nan, Value::Undefined, &[]).unwrap(),
        call_one(&mut context, &is_nan, Value::Undefined),
        call_one(
            &mut context,
            &is_nan,
            Value::String(JsString::try_from_utf8("NaN").unwrap()),
        ),
        call_one(&mut context, &is_nan, Value::Float(f64::NAN)),
        call_one(&mut context, &is_nan, Value::Float(f64::INFINITY)),
        call_one(&mut context, &is_nan, Value::Int(0)),
        context.call(&is_finite, Value::Undefined, &[]).unwrap(),
        call_one(&mut context, &is_finite, Value::Null),
        call_one(
            &mut context,
            &is_finite,
            Value::String(JsString::try_from_utf8("1").unwrap()),
        ),
        call_one(&mut context, &is_finite, Value::Int(0)),
        call_one(&mut context, &is_finite, max_value),
        call_one(&mut context, &is_finite, Value::Float(f64::INFINITY)),
        context.call(&is_integer, Value::Undefined, &[]).unwrap(),
        call_one(
            &mut context,
            &is_integer,
            Value::String(JsString::try_from_utf8("1").unwrap()),
        ),
        call_one(&mut context, &is_integer, Value::Float(f64::NAN)),
        call_one(&mut context, &is_integer, Value::Float(f64::INFINITY)),
        call_one(&mut context, &is_integer, Value::Float(-0.0)),
        call_one(&mut context, &is_integer, Value::Int(1)),
        call_one(&mut context, &is_integer, Value::Float(1.5)),
        call_one(
            &mut context,
            &is_integer,
            Value::Float(9_007_199_254_740_992.0),
        ),
        context
            .call(&is_safe_integer, Value::Undefined, &[])
            .unwrap(),
        call_one(
            &mut context,
            &is_safe_integer,
            Value::String(JsString::try_from_utf8("1").unwrap()),
        ),
        call_one(&mut context, &is_safe_integer, Value::Float(f64::NAN)),
        call_one(&mut context, &is_safe_integer, Value::Float(f64::INFINITY)),
        call_one(&mut context, &is_safe_integer, Value::Float(-0.0)),
        call_one(&mut context, &is_safe_integer, max_safe),
        call_one(&mut context, &is_safe_integer, min_safe),
        call_one(
            &mut context,
            &is_safe_integer,
            Value::Float(9_007_199_254_740_992.0),
        ),
        call_one(&mut context, &is_nan, Value::BigInt(JsBigInt::one())),
        call_one(&mut context, &is_finite, Value::Symbol(predicate_symbol)),
        call_one(
            &mut context,
            &is_integer,
            Value::Object(predicate_bomb.clone()),
        ),
        call_one(
            &mut context,
            &is_safe_integer,
            Value::Object(predicate_bomb),
        ),
        context.get_property(&global, &predicate_bomb_hit).unwrap(),
        call_one(&mut context, &is_nan, Value::Object(boxed_predicate_nan)),
        call_one(
            &mut context,
            &is_finite,
            Value::Object(boxed_predicate_zero),
        ),
        call_one(
            &mut context,
            &is_integer,
            Value::Object(boxed_predicate_one_a),
        ),
        call_one(
            &mut context,
            &is_safe_integer,
            Value::Object(boxed_predicate_one_b),
        ),
    ];
    observations.push(format!("predicates={}", join_values(&predicate_values)));

    let constant_names = [
        "MAX_VALUE",
        "MIN_VALUE",
        "NaN",
        "NEGATIVE_INFINITY",
        "POSITIVE_INFINITY",
        "EPSILON",
        "MAX_SAFE_INTEGER",
        "MIN_SAFE_INTEGER",
    ];
    observations.push(format!(
        "constants={}",
        constant_names
            .into_iter()
            .map(|name| {
                number_bits(global_property(
                    &runtime,
                    &mut context,
                    &number_object,
                    name,
                ))
            })
            .collect::<Vec<_>>()
            .join("|")
    ));

    let method_values = [
        call_with(&mut context, &value_of, Value::Float(12.5), &[]),
        call_with(&mut context, &to_string, Value::Float(123.5), &[]),
        call_with(
            &mut context,
            &to_string,
            Value::Int(-255),
            &[Value::Int(16)],
        ),
        call_with(&mut context, &to_string, Value::Int(10), &[Value::Int(2)]),
        call_with(&mut context, &to_string, Value::Int(35), &[Value::Int(36)]),
        call_with(&mut context, &to_string, Value::Float(-0.0), &[]),
        call_with(&mut context, &to_string, Value::Float(f64::NAN), &[]),
        call_with(&mut context, &to_string, Value::Float(f64::INFINITY), &[]),
        call_with(&mut context, &to_locale_string, Value::Float(123.5), &[]),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(1.25),
            &[Value::Int(1)],
        ),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(1.005),
            &[Value::Int(2)],
        ),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(-0.0),
            &[Value::Int(2)],
        ),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(1e21),
            &[Value::Int(2)],
        ),
        call_with(&mut context, &to_exponential, Value::Int(123), &[]),
        call_with(
            &mut context,
            &to_exponential,
            Value::Int(123),
            &[Value::Int(0)],
        ),
        call_with(
            &mut context,
            &to_exponential,
            Value::Float(1.25),
            &[Value::Int(1)],
        ),
        call_with(
            &mut context,
            &to_exponential,
            Value::Int(0),
            &[Value::Int(2)],
        ),
        call_with(&mut context, &to_precision, Value::Float(123.45), &[]),
        call_with(
            &mut context,
            &to_precision,
            Value::Float(123.45),
            &[Value::Int(4)],
        ),
        call_with(
            &mut context,
            &to_precision,
            Value::Float(0.000_001_234_5),
            &[Value::Int(3)],
        ),
        call_with(
            &mut context,
            &to_precision,
            Value::Int(1_234_567),
            &[Value::Int(3)],
        ),
    ];
    observations.push(format!("methods={}", join_values(&method_values)));

    let rounding_values = [
        call_with(
            &mut context,
            &to_exponential,
            Value::Int(25),
            &[Value::Int(0)],
        ),
        call_with(
            &mut context,
            &to_exponential,
            Value::Int(-25),
            &[Value::Int(0)],
        ),
        call_with(
            &mut context,
            &to_precision,
            Value::Float(2.5),
            &[Value::Int(1)],
        ),
        call_with(
            &mut context,
            &to_precision,
            Value::Float(-2.5),
            &[Value::Int(1)],
        ),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(1.125),
            &[Value::Int(2)],
        ),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(-1.125),
            &[Value::Int(2)],
        ),
        call_with(&mut context, &to_fixed, Value::Float(0.5), &[Value::Int(0)]),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(-0.5),
            &[Value::Int(0)],
        ),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(-1e-10),
            &[Value::Int(0)],
        ),
        call_with(
            &mut context,
            &to_string,
            Value::Float(f64::from_bits(1.0_f64.to_bits() - 1)),
            &[Value::Int(12)],
        ),
        call_with(
            &mut context,
            &to_string,
            Value::Float(1.3),
            &[Value::Int(7)],
        ),
        call_with(
            &mut context,
            &to_string,
            Value::Float(1.3),
            &[Value::Int(35)],
        ),
    ];
    observations.push(format!("rounding={}", join_values(&rounding_values)));
    let valid_max = [
        call_with(&mut context, &to_fixed, Value::Int(1), &[Value::Int(100)]),
        call_with(
            &mut context,
            &to_exponential,
            Value::Int(1),
            &[Value::Int(100)],
        ),
        call_with(
            &mut context,
            &to_precision,
            Value::Int(1),
            &[Value::Int(100)],
        ),
    ];
    observations.push(format!("valid-max={}", join_values(&valid_max)));

    let ignored_locale_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("ignored").unwrap()))
        .unwrap();
    let method_edges = [
        call_with(
            &mut context,
            &to_string,
            Value::Int(10),
            &[Value::Undefined],
        ),
        call_with(&mut context, &to_string, Value::Int(10), &[Value::Int(10)]),
        call_with(&mut context, &to_fixed, Value::Float(1.5), &[]),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(f64::NAN),
            &[Value::Int(2)],
        ),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(f64::INFINITY),
            &[Value::Int(2)],
        ),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(f64::NEG_INFINITY),
            &[Value::Int(2)],
        ),
        call_with(
            &mut context,
            &to_exponential,
            Value::Float(f64::NAN),
            &[Value::Int(101)],
        ),
        call_with(
            &mut context,
            &to_exponential,
            Value::Float(f64::INFINITY),
            &[Value::Int(101)],
        ),
        call_with(
            &mut context,
            &to_exponential,
            Value::Float(f64::NEG_INFINITY),
            &[Value::Int(-1)],
        ),
        call_with(
            &mut context,
            &to_precision,
            Value::Float(f64::NAN),
            &[Value::Int(0)],
        ),
        call_with(
            &mut context,
            &to_precision,
            Value::Float(f64::INFINITY),
            &[Value::Int(0)],
        ),
        call_with(
            &mut context,
            &to_precision,
            Value::Float(f64::NEG_INFINITY),
            &[Value::Int(101)],
        ),
        call_with(
            &mut context,
            &to_locale_string,
            Value::Float(1.25),
            &[Value::Symbol(ignored_locale_symbol)],
        ),
    ];
    observations.push(format!("method-edges={}", join_values(&method_edges)));

    let radix_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("radix").unwrap()))
        .unwrap();
    let range_values = [
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::Int(1),
            &[Value::Int(1)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::Int(1),
            &[Value::Int(37)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::Int(1),
            &[Value::Float(f64::NAN)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_fixed,
            Value::Int(1),
            &[Value::Int(-1)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_fixed,
            Value::Int(1),
            &[Value::Int(101)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_exponential,
            Value::Int(1),
            &[Value::Int(-1)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_exponential,
            Value::Int(1),
            &[Value::Int(101)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_precision,
            Value::Int(1),
            &[Value::Int(0)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_precision,
            Value::Int(1),
            &[Value::Int(101)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_fixed,
            Value::Float(f64::NAN),
            &[Value::Int(101)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_fixed,
            Value::Float(f64::INFINITY),
            &[Value::Int(-1)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::Int(1),
            &[Value::Symbol(radix_symbol)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_fixed,
            Value::Int(1),
            &[Value::BigInt(JsBigInt::one())],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_exponential,
            Value::Int(1),
            &[Value::BigInt(JsBigInt::one())],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_precision,
            Value::Int(1),
            &[Value::BigInt(JsBigInt::one())],
        ),
    ];
    observations.push(format!("ranges={}", range_values.join("|")));

    let brand_receiver = Value::String(JsString::try_from_utf8("1").unwrap());
    let number_spoof = context
        .new_object_with_prototype(Some(&number_prototype))
        .unwrap();
    let brand_values = [
        observe_call_args(
            &runtime,
            &mut context,
            &value_of,
            brand_receiver.clone(),
            &[],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            brand_receiver.clone(),
            &[],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_locale_string,
            brand_receiver.clone(),
            &[],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_fixed,
            brand_receiver.clone(),
            &[],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_exponential,
            brand_receiver.clone(),
            &[],
        ),
        observe_call_args(&runtime, &mut context, &to_precision, brand_receiver, &[]),
        observe_call_args(
            &runtime,
            &mut context,
            &value_of,
            Value::Object(number_spoof),
            &[],
        ),
    ];
    observations.push(format!("brands={}", brand_values.join("|")));

    let format_log = define_global(
        &runtime,
        &global,
        "formatLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let format_argument = context.new_object().unwrap();
    let format_conversion = eval_callable(
        &runtime,
        &mut context,
        r#"(function(hint) { formatLog = formatLog + hint + ","; return 2; })"#,
    );
    define_data_key(
        &runtime,
        &format_argument,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(format_conversion.as_object().clone()),
        true,
        false,
        true,
    );
    let converted_formats = [
        call_with(
            &mut context,
            &to_string,
            Value::Int(10),
            &[Value::Object(format_argument.clone())],
        ),
        call_with(
            &mut context,
            &to_fixed,
            Value::Float(1.25),
            &[Value::Object(format_argument.clone())],
        ),
        call_with(
            &mut context,
            &to_exponential,
            Value::Float(1.25),
            &[Value::Object(format_argument.clone())],
        ),
        call_with(
            &mut context,
            &to_precision,
            Value::Float(1.25),
            &[Value::Object(format_argument.clone())],
        ),
    ];
    let converted_format_log = context.get_property(&global, &format_log).unwrap();
    assert!(
        context
            .set_property(
                &global,
                &format_log,
                Value::String(JsString::try_from_utf8("").unwrap()),
            )
            .unwrap()
    );
    let bad_this_before_argument = observe_call_args(
        &runtime,
        &mut context,
        &to_fixed,
        Value::String(JsString::try_from_utf8("1").unwrap()),
        &[Value::Object(format_argument.clone())],
    );
    let bad_this_log = context.get_property(&global, &format_log).unwrap();
    assert!(
        context
            .set_property(
                &global,
                &format_log,
                Value::String(JsString::try_from_utf8("").unwrap()),
            )
            .unwrap()
    );
    let ignored_locale_argument = call_with(
        &mut context,
        &to_locale_string,
        Value::Float(1.25),
        &[Value::Object(format_argument)],
    );
    observations.push(format!(
        "conversion-order={}|{}|{}|{}|{}|{}",
        join_values(&converted_formats),
        plain_value(converted_format_log),
        bad_this_before_argument,
        plain_value(bad_this_log),
        plain_value(ignored_locale_argument),
        plain_value(context.get_property(&global, &format_log).unwrap()),
    ));

    let object_box_a = expect_object(
        context.call(&object_value_of, Value::Int(3), &[]).unwrap(),
        "Object.prototype.valueOf.call(3)",
    );
    let object_box_b = expect_object(
        context.call(&object_value_of, Value::Int(3), &[]).unwrap(),
        "second Object.prototype.valueOf.call(3)",
    );
    let object_links = [
        plain_value(context.call(&object_to_string, Value::Int(3), &[]).unwrap()),
        runtime
            .get_prototype_of(&object_box_a)
            .unwrap()
            .is_some_and(|prototype| prototype == number_prototype)
            .to_string(),
        plain_value(
            context
                .call(&value_of, Value::Object(object_box_a.clone()), &[])
                .unwrap(),
        ),
        (object_box_a == object_box_b).to_string(),
        plain_value(
            context
                .call(&object_to_locale_string, Value::Int(3), &[])
                .unwrap(),
        ),
    ];
    observations.push(format!("object-links={}", object_links.join("|")));

    define_global(&runtime, &global, "directStrictThis", Value::Undefined);
    define_global(&runtime, &global, "directSloppyThis", Value::Undefined);
    let direct_strict_method = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; directStrictThis = this; return this; })"#,
    );
    let direct_sloppy_method = eval_callable(
        &runtime,
        &mut context,
        "(function() { directSloppyThis = this; return this.valueOf(); })",
    );
    define_data(
        &runtime,
        &number_prototype,
        "__strictMethod",
        Value::Object(direct_strict_method.as_object().clone()),
        true,
        false,
        true,
    );
    define_data(
        &runtime,
        &number_prototype,
        "__sloppyMethod",
        Value::Object(direct_sloppy_method.as_object().clone()),
        true,
        false,
        true,
    );
    let direct_strict_result = context.eval("(7).__strictMethod()").unwrap();
    let direct_strict_this = global_value(&runtime, &mut context, &global, "directStrictThis");
    let direct_sloppy_result_a = context.eval("(7).__sloppyMethod()").unwrap();
    let direct_sloppy_this_a = global_value(&runtime, &mut context, &global, "directSloppyThis");
    let direct_sloppy_result_b = context.eval("(7).__sloppyMethod()").unwrap();
    let direct_sloppy_this_b = global_value(&runtime, &mut context, &global, "directSloppyThis");
    let method_receivers = [
        plain_value(direct_strict_result),
        same_number(&direct_strict_this, 7.0).to_string(),
        "number".to_owned(),
        plain_value(direct_sloppy_result_a),
        "object".to_owned(),
        object_has_prototype(&runtime, &direct_sloppy_this_a, &number_prototype).to_string(),
        plain_value(
            context
                .call(&value_of, direct_sloppy_this_a.clone(), &[])
                .unwrap(),
        ),
        plain_value(direct_sloppy_result_b),
        (direct_sloppy_this_a == direct_sloppy_this_b).to_string(),
    ];
    observations.push(format!("method-receivers={}", method_receivers.join("|")));

    for name in [
        "strictGetThis",
        "sloppyGetThis",
        "strictSetThis",
        "strictSetValue",
        "sloppySetThis",
        "sloppySetValue",
    ] {
        define_global(&runtime, &global, name, Value::Undefined);
    }
    let strict_get = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; strictGetThis = this; return this; })"#,
    );
    let sloppy_get = eval_callable(
        &runtime,
        &mut context,
        "(function() { sloppyGetThis = this; return this.valueOf(); })",
    );
    let strict_set = eval_callable(
        &runtime,
        &mut context,
        r#"(function(value) { "use strict"; strictSetThis = this; strictSetValue = value; })"#,
    );
    let sloppy_set = eval_callable(
        &runtime,
        &mut context,
        "(function(value) { sloppySetThis = this; sloppySetValue = value; })",
    );
    define_accessor(
        &runtime,
        &number_prototype,
        "__strictGet",
        Some(strict_get),
        None,
    );
    define_accessor(
        &runtime,
        &number_prototype,
        "__sloppyGet",
        Some(sloppy_get),
        None,
    );
    define_accessor(
        &runtime,
        &number_prototype,
        "__strictSet",
        None,
        Some(strict_set),
    );
    define_accessor(
        &runtime,
        &number_prototype,
        "__sloppySet",
        None,
        Some(sloppy_set),
    );
    let strict_get_result = context.eval("(3).__strictGet").unwrap();
    let sloppy_get_result = context.eval("(3).__sloppyGet").unwrap();
    let strict_set_result = context.eval("(3).__strictSet = 7").unwrap();
    let sloppy_set_result = context.eval("(3).__sloppySet = 8").unwrap();
    let strict_get_this = global_value(&runtime, &mut context, &global, "strictGetThis");
    let sloppy_get_this = global_value(&runtime, &mut context, &global, "sloppyGetThis");
    let strict_set_this = global_value(&runtime, &mut context, &global, "strictSetThis");
    let sloppy_set_this = global_value(&runtime, &mut context, &global, "sloppySetThis");
    let accessor_values = [
        plain_value(strict_get_result),
        same_number(&strict_get_this, 3.0).to_string(),
        plain_value(sloppy_get_result),
        "object".to_owned(),
        object_has_prototype(&runtime, &sloppy_get_this, &number_prototype).to_string(),
        plain_value(context.call(&value_of, sloppy_get_this, &[]).unwrap()),
        plain_value(strict_set_result),
        same_number(&strict_set_this, 3.0).to_string(),
        plain_value(global_value(
            &runtime,
            &mut context,
            &global,
            "strictSetValue",
        )),
        plain_value(sloppy_set_result),
        "object".to_owned(),
        object_has_prototype(&runtime, &sloppy_set_this, &number_prototype).to_string(),
        plain_value(context.call(&value_of, sloppy_set_this, &[]).unwrap()),
        plain_value(global_value(
            &runtime,
            &mut context,
            &global,
            "sloppySetValue",
        )),
    ];
    observations.push(format!("accessors={}", accessor_values.join("|")));

    define_global(&runtime, &global, "getterHit", Value::Bool(false));
    define_global(&runtime, &global, "deleteHit", Value::Bool(false));
    define_data(
        &runtime,
        &number_prototype,
        "__rw",
        Value::Int(1),
        true,
        false,
        true,
    );
    define_data(
        &runtime,
        &number_prototype,
        "__ro",
        Value::Int(1),
        false,
        false,
        true,
    );
    let getter_only = eval_callable(
        &runtime,
        &mut context,
        "(function() { getterHit = true; return 1; })",
    );
    let delete_getter = eval_callable(
        &runtime,
        &mut context,
        "(function() { deleteHit = true; return 1; })",
    );
    define_accessor(
        &runtime,
        &number_prototype,
        "__getterOnly",
        Some(getter_only),
        None,
    );
    define_accessor(
        &runtime,
        &number_prototype,
        "__delete",
        Some(delete_getter),
        None,
    );
    let sloppy_rw = context.eval("(3).__rw = 2").unwrap();
    let rw_after = context.eval("(3).__rw").unwrap();
    let strict_rw = observe_eval(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; return (3).__rw = 3; })()"#,
    );
    let sloppy_ro = context.eval("(3).__ro = 2").unwrap();
    let ro_after = context.eval("(3).__ro").unwrap();
    let strict_ro = observe_eval(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; return (3).__ro = 3; })()"#,
    );
    let sloppy_getter_only = context.eval("(3).__getterOnly = 2").unwrap();
    let strict_getter_only = observe_eval(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; return (3).__getterOnly = 3; })()"#,
    );
    let deleted = context.eval("delete (3).__delete").unwrap();
    let delete_key = runtime.intern_property_key("__delete").unwrap();
    let writes = [
        plain_value(sloppy_rw),
        plain_value(rw_after),
        strict_rw,
        plain_value(sloppy_ro),
        plain_value(ro_after),
        strict_ro,
        plain_value(sloppy_getter_only),
        strict_getter_only,
        plain_value(global_value(&runtime, &mut context, &global, "getterHit")),
        plain_value(deleted),
        plain_value(global_value(&runtime, &mut context, &global, "deleteHit")),
        runtime
            .has_own_property(&number_prototype, &delete_key)
            .unwrap()
            .to_string(),
        observe_eval(
            &runtime,
            &mut context,
            r#"(function() { "use strict"; return delete (3).__delete; })()"#,
        ),
    ];
    observations.push(format!("writes={}", writes.join("|")));

    let to_string_tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    let tag_before = [
        plain_value(context.call(&object_to_string, Value::Int(3), &[]).unwrap()),
        plain_value(
            context
                .call(&object_to_string, Value::Object(boxed_seven.clone()), &[])
                .unwrap(),
        ),
        plain_value(
            context
                .call(
                    &object_to_string,
                    Value::Object(number_prototype.clone()),
                    &[],
                )
                .unwrap(),
        ),
        runtime
            .has_own_property(&number_prototype, &to_string_tag)
            .unwrap()
            .to_string(),
    ];
    observations.push(format!("tag-before={}", tag_before.join("|")));

    let tag_this_key = define_global(&runtime, &global, "tagThis", Value::Undefined);
    define_global(&runtime, &global, "localeGetterThis", Value::Undefined);
    define_global(&runtime, &global, "localeCallThis", Value::Undefined);
    let tag_getter = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; tagThis = this; return "CustomNumber"; })"#,
    );
    define_accessor_key(
        &runtime,
        &number_prototype,
        &to_string_tag,
        Some(tag_getter),
        None,
    );
    let locale_method = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; localeCallThis = this; return this; })"#,
    );
    define_global(
        &runtime,
        &global,
        "localeMethod",
        Value::Object(locale_method.as_object().clone()),
    );
    let locale_getter = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { "use strict"; localeGetterThis = this; return localeMethod; })"#,
    );
    define_accessor(
        &runtime,
        &number_prototype,
        "toString",
        Some(locale_getter),
        None,
    );
    let custom_tag = context.call(&object_to_string, Value::Int(3), &[]).unwrap();
    let tag_this = global_value(&runtime, &mut context, &global, "tagThis");
    let custom_locale = context
        .call(&object_to_locale_string, Value::Int(3), &[])
        .unwrap();
    let custom = [
        plain_value(custom_tag),
        "object".to_owned(),
        object_has_prototype(&runtime, &tag_this, &number_prototype).to_string(),
        plain_value(context.call(&value_of, tag_this, &[]).unwrap()),
        plain_value(custom_locale),
        same_number(
            &global_value(&runtime, &mut context, &global, "localeGetterThis"),
            3.0,
        )
        .to_string(),
        same_number(
            &global_value(&runtime, &mut context, &global, "localeCallThis"),
            3.0,
        )
        .to_string(),
    ];
    observations.push(format!("custom-object-methods={}", custom.join("|")));

    let boxed_custom_tag = context
        .call(&object_to_string, Value::Object(boxed_seven.clone()), &[])
        .unwrap();
    let boxed_tag_this = global_value(&runtime, &mut context, &global, "tagThis");
    let prototype_custom_tag = context
        .call(
            &object_to_string,
            Value::Object(number_prototype.clone()),
            &[],
        )
        .unwrap();
    let prototype_tag_this = global_value(&runtime, &mut context, &global, "tagThis");
    let tag_getter_detail = [
        plain_value(boxed_custom_tag),
        matches!(&boxed_tag_this, Value::Object(object) if *object == boxed_seven).to_string(),
        plain_value(context.call(&value_of, boxed_tag_this, &[]).unwrap()),
        plain_value(prototype_custom_tag),
        matches!(&prototype_tag_this, Value::Object(object) if *object == number_prototype)
            .to_string(),
        plain_value(context.call(&value_of, prototype_tag_this, &[]).unwrap()),
    ];
    observations.push(format!("tag-getter-detail={}", tag_getter_detail.join("|")));

    define_data_key(
        &runtime,
        &boxed_seven,
        &to_string_tag,
        Value::String(JsString::try_from_utf8("OwnNumber").unwrap()),
        false,
        false,
        true,
    );
    assert!(
        context
            .set_property(&global, &tag_this_key, Value::Undefined)
            .unwrap()
    );
    let own_tag = context
        .call(&object_to_string, Value::Object(boxed_seven.clone()), &[])
        .unwrap();
    let tag_own = [
        plain_value(own_tag),
        matches!(
            global_value(&runtime, &mut context, &global, "tagThis"),
            Value::Undefined
        )
        .to_string(),
        plain_value(
            context
                .call(&value_of, Value::Object(boxed_seven.clone()), &[])
                .unwrap(),
        ),
    ];
    observations.push(format!("tag-own={}", tag_own.join("|")));

    define_data_key(
        &runtime,
        &number_prototype,
        &to_string_tag,
        Value::Int(123),
        false,
        false,
        true,
    );
    let tag_nonstring = [
        plain_value(context.call(&object_to_string, Value::Int(3), &[]).unwrap()),
        plain_value(
            context
                .call(&object_to_string, Value::Object(boxed_seven.clone()), &[])
                .unwrap(),
        ),
    ];
    observations.push(format!("tag-nonstring={}", tag_nonstring.join("|")));

    assert!(
        runtime
            .delete_property(&boxed_seven, &to_string_tag)
            .unwrap()
    );
    assert!(
        runtime
            .delete_property(&number_prototype, &to_string_tag)
            .unwrap()
    );
    define_data(
        &runtime,
        &number_prototype,
        "toString",
        Value::Object(to_string.as_object().clone()),
        true,
        false,
        true,
    );
    for name in [
        "__strictMethod",
        "__sloppyMethod",
        "__strictGet",
        "__sloppyGet",
        "__strictSet",
        "__sloppySet",
        "__rw",
        "__ro",
        "__getterOnly",
        "__delete",
    ] {
        assert!(
            runtime
                .delete_property(
                    &number_prototype,
                    &runtime.intern_property_key(name).unwrap(),
                )
                .unwrap()
        );
    }
    let tag_restored = [
        plain_value(context.call(&object_to_string, Value::Int(3), &[]).unwrap()),
        plain_value(
            context
                .call(&object_to_string, Value::Object(boxed_seven), &[])
                .unwrap(),
        ),
        key_names(
            &runtime,
            &runtime.own_property_keys(&number_prototype).unwrap(),
        )
        .join(","),
    ];
    observations.push(format!("tag-restored={}", tag_restored.join("|")));

    let replacement_parse_int =
        eval_callable(&runtime, &mut context, "(function() { return 99; })");
    let replacement_parse_float =
        eval_callable(&runtime, &mut context, "(function() { return 88; })");
    assert!(
        context
            .set_property(
                &global,
                &runtime.intern_property_key("parseInt").unwrap(),
                Value::Object(replacement_parse_int.as_object().clone()),
            )
            .unwrap()
    );
    assert!(
        context
            .set_property(
                &global,
                &runtime.intern_property_key("parseFloat").unwrap(),
                Value::Object(replacement_parse_float.as_object().clone()),
            )
            .unwrap()
    );
    let current_parse_int = property_callable(&runtime, &mut context, &global, "parseInt");
    let current_parse_float = property_callable(&runtime, &mut context, &global, "parseFloat");
    let alias_values = [
        (number_parse_int.as_object() == parse_int.as_object()).to_string(),
        (number_parse_int.as_object() == current_parse_int.as_object()).to_string(),
        plain_value(
            context
                .call(
                    &number_parse_int,
                    Value::Undefined,
                    &[
                        Value::String(JsString::try_from_utf8("10").unwrap()),
                        Value::Int(2),
                    ],
                )
                .unwrap(),
        ),
        plain_value(
            context
                .call(
                    &current_parse_int,
                    Value::Undefined,
                    &[
                        Value::String(JsString::try_from_utf8("10").unwrap()),
                        Value::Int(2),
                    ],
                )
                .unwrap(),
        ),
        (number_parse_float.as_object() == parse_float.as_object()).to_string(),
        (number_parse_float.as_object() == current_parse_float.as_object()).to_string(),
        plain_value(
            context
                .call(
                    &number_parse_float,
                    Value::Undefined,
                    &[Value::String(JsString::try_from_utf8("1.5tail").unwrap())],
                )
                .unwrap(),
        ),
        plain_value(
            context
                .call(
                    &current_parse_float,
                    Value::Undefined,
                    &[Value::String(JsString::try_from_utf8("1.5tail").unwrap())],
                )
                .unwrap(),
        ),
    ];
    observations.push(format!("alias-capture={}", alias_values.join("|")));

    Some(observations)
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
    let key = runtime.intern_property_key(name).unwrap();
    define_data_key(
        runtime,
        object,
        &key,
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
    let key = runtime.intern_property_key(name).unwrap();
    define_accessor_key(runtime, object, &key, get, set);
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
    let Value::Object(object) = context.eval(source).unwrap() else {
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
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(value) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn global_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> Value {
    context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
}

fn global_value(runtime: &Runtime, context: &mut Context, global: &ObjectRef, name: &str) -> Value {
    global_property(runtime, context, global, name)
}

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(object) = value else {
        panic!("{description} did not produce an object");
    };
    object
}

fn key_names(runtime: &Runtime, keys: &[PropertyKey]) -> Vec<String> {
    keys.iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect()
}

fn data_flags(runtime: &Runtime, object: &ObjectRef, name: &str) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    data_flags_key(runtime, object, &key)
}

fn data_flags_key(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey) -> String {
    let descriptor = runtime
        .get_own_property(object, key)
        .unwrap()
        .unwrap_or_else(|| panic!("property descriptor was absent"));
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

fn function_signature(runtime: &Runtime, callable: &CallableRef) -> String {
    let name_key = runtime.intern_property_key("name").unwrap();
    let length_key = runtime.intern_property_key("length").unwrap();
    let name = match runtime
        .get_own_property(callable.as_object(), &name_key)
        .unwrap()
        .expect("native function name descriptor was absent")
    {
        CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        } => value.to_utf8_lossy(),
        _ => panic!("native function name was not string data"),
    };
    let length = match runtime
        .get_own_property(callable.as_object(), &length_key)
        .unwrap()
        .expect("native function length descriptor was absent")
    {
        CompleteOrdinaryPropertyDescriptor::Data { value, .. } => plain_value(value),
        _ => panic!("native function length was not data"),
    };
    format!(
        "{name}:{length}:{}",
        key_names(
            runtime,
            &runtime.own_property_keys(callable.as_object()).unwrap()
        )
        .join(",")
    )
}

fn object_has_prototype(runtime: &Runtime, value: &Value, prototype: &ObjectRef) -> bool {
    let Value::Object(object) = value else {
        return false;
    };
    runtime
        .get_prototype_of(object)
        .unwrap()
        .is_some_and(|actual| actual == *prototype)
}

fn same_number(value: &Value, expected: f64) -> bool {
    value
        .as_number()
        .is_some_and(|actual| actual.to_bits() == expected.to_bits())
}

fn call_one(context: &mut Context, callable: &CallableRef, argument: Value) -> Value {
    context
        .call(callable, Value::Undefined, &[argument])
        .unwrap()
}

fn call_with(
    context: &mut Context,
    callable: &CallableRef,
    this_value: Value,
    arguments: &[Value],
) -> Value {
    context.call(callable, this_value, arguments).unwrap()
}

fn reciprocal(value: Value) -> Value {
    let number = value
        .as_number()
        .unwrap_or_else(|| panic!("reciprocal operand was not a Number: {value:?}"));
    Value::number(1.0 / number)
}

fn observe_eval(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    match context.eval(source) {
        Ok(value) => plain_value(value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => {
            panic!("Rust source failed outside JavaScript completion: {source:?}: {error}")
        }
    }
}

fn observe_call_args(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    this_value: Value,
    arguments: &[Value],
) -> String {
    match context.call(callable, this_value, arguments) {
        Ok(value) => plain_value(value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("Rust call failed outside JavaScript completion: {error}"),
    }
}

fn observe_construct_args(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    arguments: &[Value],
) -> String {
    match context.construct(callable, arguments) {
        Ok(value) => plain_value(value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("Rust construct failed outside JavaScript completion: {error}"),
    }
}

fn take_error(runtime: &Runtime, context: &mut Context) -> String {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Number operation did not throw an Error object");
    };
    let name = error_text(runtime, context, &error, "name");
    let message = error_text(runtime, context, &error, "message");
    format!("throw:{name}:{message}")
}

fn error_text(runtime: &Runtime, context: &mut Context, error: &ObjectRef, name: &str) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::String(value) = context.get_property(error, &key).unwrap() else {
        panic!("Error.{name} was not a string");
    };
    value.to_utf8_lossy()
}

fn number_bits(value: Value) -> String {
    let number = value
        .as_number()
        .unwrap_or_else(|| panic!("Number constant was not numeric: {value:?}"));
    if number.is_nan() {
        "NaN".to_owned()
    } else {
        format!("{:016x}", number.to_bits())
    }
}

fn join_values(values: &[Value]) -> String {
    values
        .iter()
        .cloned()
        .map(plain_value)
        .collect::<Vec<_>>()
        .join("|")
}

fn plain_value(value: Value) -> String {
    match value {
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Symbol(_) => "Symbol(number)".to_owned(),
        value => value
            .to_js_string()
            .expect("ordinary Number observation must stringify")
            .to_utf8_lossy(),
    }
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS Number intrinsic oracle");
    assert!(
        output.status.success(),
        "QuickJS Number intrinsic oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Number intrinsic oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn run_cli(program: &OsStr, source: &str, description: &str) -> Output {
    Command::new(program)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}
