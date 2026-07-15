use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    JsBigInt, JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError,
    Value, WellKnownSymbol,
};

// The pinned probe deliberately uses Object/Reflect/Symbol to inspect QuickJS.
// quickjs-oxide does not publish those source globals yet, so the Rust side
// performs the same observations through the public host API. Source
// evaluation on the Rust side is reserved for primitive member operations,
// where strict/sloppy receiver behavior itself is under test.
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
    try { Reflect.construct(function () {}, [], fn); return true; }
    catch (_) { return false; }
}

var implementedGlobals = [
    "parseInt", "parseFloat", "isNaN", "isFinite",
    "decodeURI", "decodeURIComponent", "encodeURI", "encodeURIComponent",
    "escape", "unescape", "Infinity", "NaN", "undefined", "Number",
    "Boolean", "String", "Math", "Reflect", "Symbol", "globalThis", "BigInt"
];
print("global-order=" + Reflect.ownKeys(globalThis).filter(function (key) {
    return typeof key === "string" && implementedGlobals.indexOf(key) >= 0;
}).join(","));
print("keys=" + Reflect.ownKeys(BigInt).map(String).join(",") + "|" +
      Reflect.ownKeys(BigInt.prototype).map(String).join(","));
print("descriptors=" + [
    flags(globalThis, "BigInt"), flags(BigInt, "length"), flags(BigInt, "name"),
    flags(BigInt, "asUintN"), flags(BigInt, "asIntN"), flags(BigInt, "prototype"),
    flags(BigInt.prototype, "toString"), flags(BigInt.prototype, "valueOf"),
    flags(BigInt.prototype, "constructor"),
    flags(BigInt.prototype, Symbol.toStringTag)
].join("|"));
print("graph=" + [
    typeof BigInt,
    Object.getPrototypeOf(BigInt) === Function.prototype,
    BigInt.prototype.constructor === BigInt,
    Object.getPrototypeOf(BigInt.prototype) === Object.prototype,
    Object.prototype.toString.call(BigInt.prototype),
    Object.isExtensible(BigInt.prototype),
    isConstructor(BigInt), isConstructor(BigInt.asUintN), isConstructor(BigInt.asIntN)
].join("|"));
print("signatures=" + [BigInt, BigInt.asUintN, BigInt.asIntN,
    BigInt.prototype.toString, BigInt.prototype.valueOf].map(signature).join("|"));

var callCases = [
    function () { return BigInt(); },
    function () { return BigInt(undefined); },
    function () { return BigInt(null); },
    function () { return BigInt(false); },
    function () { return BigInt(true); },
    function () { return BigInt(0); },
    function () { return BigInt(-0); },
    function () { return BigInt(42); },
    function () { return BigInt(9007199254740992); },
    function () { return BigInt(1.5); },
    function () { return BigInt(NaN); },
    function () { return BigInt(Infinity); },
    function () { return BigInt(""); },
    function () { return BigInt("  -42  "); },
    function () { return BigInt("0xff"); },
    function () { return BigInt("+0x1"); },
    function () { return BigInt("1n"); },
    function () { return BigInt(123n); },
    function () { return BigInt(Symbol("x")); }
];
print("calls=" + callCases.map(observe).join("|"));

var constructHit = false;
var constructBomb = {};
Object.defineProperty(constructBomb, Symbol.toPrimitive, {
    configurable: true,
    value: function () { constructHit = true; throw new Error("converted"); }
});
function OtherTarget() {}
print("construct=" + [
    observe(function () { return new BigInt(constructBomb); }), constructHit,
    observe(function () { return Reflect.construct(BigInt, [constructBomb], OtherTarget); }),
    constructHit
].join("|"));

var conversionLog = "";
var exotic = {};
Object.defineProperty(exotic, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) { conversionLog += "exotic:" + hint + ","; return "42"; }
});
var fallback = {
    valueOf: function () { conversionLog += "valueOf,"; return {}; },
    toString: function () { conversionLog += "toString,"; return "43"; }
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
    observe(function () { return BigInt(exotic); }),
    observe(function () { return BigInt(fallback); }),
    observe(function () { return BigInt(invalidPrimitive); }),
    observe(function () { return BigInt(throwingPrimitive); }),
    conversionLog
].join("|"));

print("as-values=" + [
    BigInt.asUintN(0, -1n), BigInt.asIntN(0, -1n),
    BigInt.asUintN(8, 257n), BigInt.asIntN(8, 255n),
    BigInt.asIntN(8, -129n),
    BigInt.asUintN(63, -1n), BigInt.asUintN(64, -1n),
    BigInt.asUintN(64, 9223372036854775808n),
    BigInt.asUintN(128, -18446744073709551616n),
    BigInt.asUintN(65, -18446744073709551616n),
    BigInt.asIntN(65, 18446744073709551616n),
    BigInt.asUintN(1.9, 3n), BigInt.asIntN(NaN, 9n)
].join("|"));

var asLog = "";
var goodBits = {};
Object.defineProperty(goodBits, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) { asLog += "bits:" + hint + ","; return 8; }
});
var goodValue = {};
Object.defineProperty(goodValue, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) { asLog += "value:" + hint + ","; return "-1"; }
});
var untouchedValue = {};
Object.defineProperty(untouchedValue, Symbol.toPrimitive, {
    configurable: true,
    value: function () { asLog += "untouched,"; return 1n; }
});
var zeroValue = {};
Object.defineProperty(zeroValue, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) { asLog += "zero-value:" + hint + ","; return "5"; }
});
print("as-order=" + [
    observe(function () { return BigInt.asUintN(goodBits, goodValue); }),
    observe(function () { return BigInt.asUintN(-1, untouchedValue); }),
    observe(function () { return BigInt.asUintN(0, zeroValue); }),
    observe(function () { return BigInt.asUintN(1n, untouchedValue); }),
    asLog
].join("|"));
var asErrorCases = [
    function () { return BigInt.asUintN(-1, 1n); },
    function () { return BigInt.asUintN(Infinity, 1n); },
    function () { return BigInt.asUintN(9007199254740992, 1n); },
    function () { return BigInt.asUintN(8, undefined); },
    function () { return BigInt.asUintN(8, null); },
    function () { return BigInt.asUintN(8, 1); },
    function () { return BigInt.asUintN(8, Symbol("x")); },
    function () { return BigInt.asUintN(8, "bad"); }
];
print("as-errors=" + asErrorCases.map(observe).join("|"));

var boundary = 1n << 1048575n;
var overflowText = "0b1" + "0".repeat(1048576);
print("limits=" + [
    observe(function () { return BigInt(overflowText); }),
    observe(function () { return BigInt.asUintN(1048576, boundary) === (-1n << 1048575n); }),
    observe(function () { return BigInt.asUintN(1048577, boundary); }),
    observe(function () { return BigInt.asUintN(1048639, boundary); }),
    observe(function () { return BigInt.asUintN(1048640, boundary) === boundary; }),
    observe(function () { return boundary.toString(2).length; }),
    observe(function () { return boundary.toString(16).length; }),
    observe(function () { return boundary.toString(10); }),
    observe(function () { return (-1n << 1048575n).toString(2); })
].join("|"));

var radixLog = "";
var radixObject = {};
Object.defineProperty(radixObject, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) { radixLog += hint + ","; return 16; }
});
var brandBomb = {};
Object.defineProperty(brandBomb, Symbol.toPrimitive, {
    configurable: true,
    value: function () { radixLog += "brand-bomb,"; return 10; }
});
var protoCases = [
    function () { return (255n).toString(); },
    function () { return (-255n).toString(16); },
    function () { return (35n).toString(36); },
    function () { return BigInt.prototype.toString.call(255n, radixObject); },
    function () { return BigInt.prototype.valueOf.call(-7n); },
    function () { return BigInt.prototype.valueOf(); },
    function () { return BigInt.prototype.toString.call({}, brandBomb); },
    function () { return (1n).toString(1); },
    function () { return (1n).toString(37); },
    function () { return (1n).toString(1n); }
];
print("proto=" + protoCases.map(observe).join("|") + "|" + radixLog);

var wrapper = Object(123n);
var wrapper2 = Object.prototype.valueOf.call(123n);
print("objects=" + [
    typeof wrapper,
    Object.getPrototypeOf(wrapper) === BigInt.prototype,
    Reflect.ownKeys(wrapper).length,
    BigInt.prototype.valueOf.call(wrapper),
    wrapper === wrapper2,
    Object.prototype.toString.call(123n),
    Object.prototype.toString.call(wrapper),
    Object.prototype.toString.call(BigInt.prototype),
    Object.prototype.toLocaleString.call(123n)
].join("|"));
delete BigInt.prototype[Symbol.toStringTag];
print("tag-delete=" + [
    Object.prototype.toString.call(123n),
    Object.prototype.toString.call(wrapper),
    Object.prototype.toString.call(BigInt.prototype)
].join("|"));
Object.defineProperty(BigInt.prototype, Symbol.toStringTag, {
    value: "CustomBigInt", writable: false, enumerable: false, configurable: true
});
print("tag-custom=" + [
    Object.prototype.toString.call(123n),
    Object.prototype.toString.call(wrapper),
    Object.prototype.toString.call(BigInt.prototype)
].join("|"));

var strictGetThis, sloppyGetThis, strictSetThis, sloppySetThis;
Object.defineProperty(BigInt.prototype, "__strictGet", {
    configurable: true,
    get: function () { "use strict"; strictGetThis = this; return this; }
});
Object.defineProperty(BigInt.prototype, "__sloppyGet", {
    configurable: true,
    get: function () { sloppyGetThis = this; return this.valueOf(); }
});
Object.defineProperty(BigInt.prototype, "__strictSet", {
    configurable: true,
    set: function () { "use strict"; strictSetThis = this; }
});
Object.defineProperty(BigInt.prototype, "__sloppySet", {
    configurable: true,
    set: function () { sloppySetThis = this; }
});
var strictGetResult = (1n).__strictGet;
var sloppyGetResult = (1n).__sloppyGet;
var strictSetResult = (1n).__strictSet = 7;
var sloppySetResult = (1n).__sloppySet = 8;
print("accessors=" + [
    strictGetResult, typeof strictGetThis, strictGetThis === 1n,
    sloppyGetResult, typeof sloppyGetThis,
    Object.getPrototypeOf(sloppyGetThis) === BigInt.prototype,
    sloppyGetThis.valueOf(),
    strictSetResult, typeof strictSetThis, strictSetThis === 1n,
    sloppySetResult, typeof sloppySetThis,
    Object.getPrototypeOf(sloppySetThis) === BigInt.prototype,
    sloppySetThis.valueOf()
].join("|"));

Object.defineProperty(BigInt.prototype, "__rw", {
    value: 1, writable: true, configurable: true
});
Object.defineProperty(BigInt.prototype, "__ro", {
    value: 1, writable: false, configurable: true
});
var deleteHit = false;
Object.defineProperty(BigInt.prototype, "__delete", {
    configurable: true,
    get: function () { deleteHit = true; return 1; }
});
print("writes=" + [
    ((1n).__rw = 2), (1n).__rw,
    observe(function () { "use strict"; return (1n).__rw = 3; }),
    ((1n).__ro = 2), (1n).__ro,
    observe(function () { "use strict"; return (1n).__ro = 3; }),
    delete (1n).__delete, deleteHit,
    Object.prototype.hasOwnProperty.call(BigInt.prototype, "__delete"),
    (function () { "use strict"; return delete (1n).__delete; })()
].join("|"));
"#;

#[test]
fn bigint_intrinsic_matches_pinned_quickjs() {
    let rust = rust_observations();
    assert_eq!(
        rust.len(),
        18,
        "the BigInt differential unexpectedly changed breadth"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP BigInt intrinsic differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "BigInt intrinsic behavior differed from pinned QuickJS"
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let bigint_prototype = context.bigint_prototype().unwrap();
    let bigint = property_callable(&runtime, &mut context, &global, "BigInt");
    let as_uint_n = property_callable(&runtime, &mut context, bigint.as_object(), "asUintN");
    let as_int_n = property_callable(&runtime, &mut context, bigint.as_object(), "asIntN");
    let to_string = property_callable(&runtime, &mut context, &bigint_prototype, "toString");
    let value_of = property_callable(&runtime, &mut context, &bigint_prototype, "valueOf");
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
        "String",
        "Math",
        "Reflect",
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
        own_key_names(&runtime, bigint.as_object()).join(","),
        own_key_names(&runtime, &bigint_prototype).join(",")
    ));
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    observations.push(format!(
        "descriptors={}",
        [
            data_flags(&runtime, &global, "BigInt"),
            data_flags(&runtime, bigint.as_object(), "length"),
            data_flags(&runtime, bigint.as_object(), "name"),
            data_flags(&runtime, bigint.as_object(), "asUintN"),
            data_flags(&runtime, bigint.as_object(), "asIntN"),
            data_flags(&runtime, bigint.as_object(), "prototype"),
            data_flags(&runtime, &bigint_prototype, "toString"),
            data_flags(&runtime, &bigint_prototype, "valueOf"),
            data_flags(&runtime, &bigint_prototype, "constructor"),
            data_flags_key(&runtime, &bigint_prototype, &tag),
        ]
        .join("|")
    ));
    observations.push(format!(
        "graph=function|{}|{}|{}|{}|{}|{}|{}|{}",
        runtime
            .get_prototype_of(bigint.as_object())
            .unwrap()
            .is_some_and(|prototype| prototype == function_prototype),
        matches!(
            context
                .get_property(
                    &bigint_prototype,
                    &runtime.intern_property_key("constructor").unwrap(),
                )
                .unwrap(),
            Value::Object(object) if object == *bigint.as_object()
        ),
        runtime
            .get_prototype_of(&bigint_prototype)
            .unwrap()
            .is_some_and(|prototype| prototype == object_prototype),
        plain_value(
            context
                .call(
                    &object_to_string,
                    Value::Object(bigint_prototype.clone()),
                    &[],
                )
                .unwrap()
        ),
        runtime.is_extensible(&bigint_prototype).unwrap(),
        runtime.is_constructor(bigint.as_object()).unwrap(),
        runtime.is_constructor(as_uint_n.as_object()).unwrap(),
        runtime.is_constructor(as_int_n.as_object()).unwrap(),
    ));
    observations.push(format!(
        "signatures={}",
        [&bigint, &as_uint_n, &as_int_n, &to_string, &value_of]
            .into_iter()
            .map(|callable| function_signature(&runtime, &mut context, callable))
            .collect::<Vec<_>>()
            .join("|")
    ));

    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("x").unwrap()))
        .unwrap();
    let call_arguments = vec![
        vec![],
        vec![Value::Undefined],
        vec![Value::Null],
        vec![Value::Bool(false)],
        vec![Value::Bool(true)],
        vec![Value::Int(0)],
        vec![Value::Float(-0.0)],
        vec![Value::Int(42)],
        vec![Value::Float(9_007_199_254_740_992.0)],
        vec![Value::Float(1.5)],
        vec![Value::Float(f64::NAN)],
        vec![Value::Float(f64::INFINITY)],
        vec![Value::String(JsString::try_from_utf8("").unwrap())],
        vec![Value::String(JsString::try_from_utf8("  -42  ").unwrap())],
        vec![Value::String(JsString::try_from_utf8("0xff").unwrap())],
        vec![Value::String(JsString::try_from_utf8("+0x1").unwrap())],
        vec![Value::String(JsString::try_from_utf8("1n").unwrap())],
        vec![Value::BigInt(JsBigInt::from(123))],
        vec![Value::Symbol(symbol)],
    ];
    observations.push(format!(
        "calls={}",
        call_arguments
            .iter()
            .map(|arguments| {
                observe_call_args(&runtime, &mut context, &bigint, Value::Undefined, arguments)
            })
            .collect::<Vec<_>>()
            .join("|")
    ));

    let construct_hit = define_global(&runtime, &global, "constructHit", Value::Bool(false));
    let construct_bomb = context.new_object().unwrap();
    let construct_conversion = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { constructHit = true; throw new Error("converted"); })"#,
    );
    define_data_key(
        &runtime,
        &construct_bomb,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(construct_conversion.as_object().clone()),
        true,
        false,
        true,
    );
    let other_target = eval_callable(&runtime, &mut context, "(function OtherTarget() {})");
    let first_construct = observe_construct_args(
        &runtime,
        &mut context,
        &bigint,
        &[Value::Object(construct_bomb.clone())],
    );
    let first_hit = plain_value(context.get_property(&global, &construct_hit).unwrap());
    let second_construct = observe_construct_with_new_target(
        &runtime,
        &mut context,
        &bigint,
        &other_target,
        &[Value::Object(construct_bomb)],
    );
    let second_hit = plain_value(context.get_property(&global, &construct_hit).unwrap());
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
        r#"(function(hint) { conversionLog += "exotic:" + hint + ","; return "42"; })"#,
    );
    let fallback = context.new_object().unwrap();
    let fallback_result = context.new_object().unwrap();
    define_global(
        &runtime,
        &global,
        "fallbackResult",
        Value::Object(fallback_result),
    );
    let fallback_value_of = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { conversionLog += "valueOf,"; return fallbackResult; })"#,
    );
    let fallback_to_string = eval_callable(
        &runtime,
        &mut context,
        r#"(function() { conversionLog += "toString,"; return "43"; })"#,
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
    define_data(
        &runtime,
        &fallback,
        "toString",
        Value::Object(fallback_to_string.as_object().clone()),
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
        r#"(function() { conversionLog += "invalid,"; return invalidPrimitiveResult; })"#,
    );
    let throwing = conversion_object(
        &runtime,
        &mut context,
        r#"(function() { conversionLog += "throw,"; throw new RangeError("sentinel"); })"#,
    );
    let converted = [&exotic, &fallback, &invalid, &throwing]
        .into_iter()
        .map(|object| {
            observe_call_args(
                &runtime,
                &mut context,
                &bigint,
                Value::Undefined,
                &[Value::Object(object.clone())],
            )
        })
        .collect::<Vec<_>>();
    let log = plain_value(context.get_property(&global, &conversion_log).unwrap());
    observations.push(format!("conversion={}|{log}", converted.join("|")));

    let two_63 = parse_bigint("9223372036854775808");
    let two_64 = parse_bigint("18446744073709551616");
    let minus_two_64 = parse_bigint("-18446744073709551616");
    let as_values = [
        call_two(
            &mut context,
            &as_uint_n,
            Value::Int(0),
            Value::BigInt(JsBigInt::from(-1)),
        ),
        call_two(
            &mut context,
            &as_int_n,
            Value::Int(0),
            Value::BigInt(JsBigInt::from(-1)),
        ),
        call_two(
            &mut context,
            &as_uint_n,
            Value::Int(8),
            Value::BigInt(JsBigInt::from(257)),
        ),
        call_two(
            &mut context,
            &as_int_n,
            Value::Int(8),
            Value::BigInt(JsBigInt::from(255)),
        ),
        call_two(
            &mut context,
            &as_int_n,
            Value::Int(8),
            Value::BigInt(JsBigInt::from(-129)),
        ),
        call_two(
            &mut context,
            &as_uint_n,
            Value::Int(63),
            Value::BigInt(JsBigInt::from(-1)),
        ),
        call_two(
            &mut context,
            &as_uint_n,
            Value::Int(64),
            Value::BigInt(JsBigInt::from(-1)),
        ),
        call_two(
            &mut context,
            &as_uint_n,
            Value::Int(64),
            Value::BigInt(two_63),
        ),
        call_two(
            &mut context,
            &as_uint_n,
            Value::Int(128),
            Value::BigInt(minus_two_64.clone()),
        ),
        call_two(
            &mut context,
            &as_uint_n,
            Value::Int(65),
            Value::BigInt(minus_two_64),
        ),
        call_two(
            &mut context,
            &as_int_n,
            Value::Int(65),
            Value::BigInt(two_64),
        ),
        call_two(
            &mut context,
            &as_uint_n,
            Value::Float(1.9),
            Value::BigInt(JsBigInt::from(3)),
        ),
        call_two(
            &mut context,
            &as_int_n,
            Value::Float(f64::NAN),
            Value::BigInt(JsBigInt::from(9)),
        ),
    ];
    observations.push(format!("as-values={}", join_values(&as_values)));

    let as_log = define_global(
        &runtime,
        &global,
        "asLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let good_bits = conversion_object(
        &runtime,
        &mut context,
        r#"(function(hint) { asLog += "bits:" + hint + ","; return 8; })"#,
    );
    let good_value = conversion_object(
        &runtime,
        &mut context,
        r#"(function(hint) { asLog += "value:" + hint + ","; return "-1"; })"#,
    );
    let untouched = conversion_object(
        &runtime,
        &mut context,
        r#"(function() { asLog += "untouched,"; return 1n; })"#,
    );
    let zero_value = conversion_object(
        &runtime,
        &mut context,
        r#"(function(hint) { asLog += "zero-value:" + hint + ","; return "5"; })"#,
    );
    let as_order = [
        observe_call_args(
            &runtime,
            &mut context,
            &as_uint_n,
            Value::Undefined,
            &[Value::Object(good_bits), Value::Object(good_value)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &as_uint_n,
            Value::Undefined,
            &[Value::Int(-1), Value::Object(untouched.clone())],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &as_uint_n,
            Value::Undefined,
            &[Value::Int(0), Value::Object(zero_value)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &as_uint_n,
            Value::Undefined,
            &[Value::BigInt(JsBigInt::one()), Value::Object(untouched)],
        ),
    ];
    let as_log = plain_value(context.get_property(&global, &as_log).unwrap());
    observations.push(format!("as-order={}|{as_log}", as_order.join("|")));

    let as_error_arguments = [
        vec![Value::Int(-1), Value::BigInt(JsBigInt::one())],
        vec![Value::Float(f64::INFINITY), Value::BigInt(JsBigInt::one())],
        vec![
            Value::Float(9_007_199_254_740_992.0),
            Value::BigInt(JsBigInt::one()),
        ],
        vec![Value::Int(8), Value::Undefined],
        vec![Value::Int(8), Value::Null],
        vec![Value::Int(8), Value::Int(1)],
        vec![
            Value::Int(8),
            Value::Symbol(
                runtime
                    .new_symbol(Some(JsString::try_from_utf8("x").unwrap()))
                    .unwrap(),
            ),
        ],
        vec![
            Value::Int(8),
            Value::String(JsString::try_from_utf8("bad").unwrap()),
        ],
    ];
    observations.push(format!(
        "as-errors={}",
        as_error_arguments
            .iter()
            .map(|arguments| {
                observe_call_args(
                    &runtime,
                    &mut context,
                    &as_uint_n,
                    Value::Undefined,
                    arguments,
                )
            })
            .collect::<Vec<_>>()
            .join("|")
    ));

    let boundary = JsBigInt::one()
        .shl(&JsBigInt::from(1_048_575))
        .expect("construct extended QuickJS boundary BigInt");
    let negative_boundary = JsBigInt::from(-1)
        .shl(&JsBigInt::from(1_048_575))
        .expect("construct negative extended QuickJS boundary BigInt");
    let overflow_text = format!("0b1{}", "0".repeat(1_048_576));
    let limits = [
        observe_call_args(
            &runtime,
            &mut context,
            &bigint,
            Value::Undefined,
            &[Value::String(
                JsString::try_from_utf8(overflow_text.as_str()).unwrap(),
            )],
        ),
        plain_value(Value::Bool(matches!(
            context.call(
                &as_uint_n,
                Value::Undefined,
                &[Value::Int(1_048_576), Value::BigInt(boundary.clone())],
            ),
            Ok(Value::BigInt(value)) if value == negative_boundary
        ))),
        observe_call_args(
            &runtime,
            &mut context,
            &as_uint_n,
            Value::Undefined,
            &[Value::Int(1_048_577), Value::BigInt(boundary.clone())],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &as_uint_n,
            Value::Undefined,
            &[Value::Int(1_048_639), Value::BigInt(boundary.clone())],
        ),
        plain_value(Value::Bool(matches!(
            context.call(
                &as_uint_n,
                Value::Undefined,
                &[
                    Value::Int(1_048_640),
                    Value::BigInt(boundary.clone()),
                ],
            ),
            Ok(Value::BigInt(value)) if value == boundary
        ))),
        observe_bigint_radix_length(&runtime, &mut context, &to_string, &boundary, 2),
        observe_bigint_radix_length(&runtime, &mut context, &to_string, &boundary, 16),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::BigInt(boundary),
            &[Value::Int(10)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::BigInt(negative_boundary),
            &[Value::Int(2)],
        ),
    ];
    observations.push(format!("limits={}", limits.join("|")));

    let radix_log = define_global(
        &runtime,
        &global,
        "radixLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let radix_object = conversion_object(
        &runtime,
        &mut context,
        r#"(function(hint) { radixLog += hint + ","; return 16; })"#,
    );
    let brand_bomb = conversion_object(
        &runtime,
        &mut context,
        r#"(function() { radixLog += "brand-bomb,"; return 10; })"#,
    );
    let ordinary = context.new_object().unwrap();
    let proto = [
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::BigInt(JsBigInt::from(255)),
            &[],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::BigInt(JsBigInt::from(-255)),
            &[Value::Int(16)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::BigInt(JsBigInt::from(35)),
            &[Value::Int(36)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::BigInt(JsBigInt::from(255)),
            &[Value::Object(radix_object)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &value_of,
            Value::BigInt(JsBigInt::from(-7)),
            &[],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &value_of,
            Value::Object(bigint_prototype.clone()),
            &[],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::Object(ordinary),
            &[Value::Object(brand_bomb)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::BigInt(JsBigInt::one()),
            &[Value::Int(1)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::BigInt(JsBigInt::one()),
            &[Value::Int(37)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            &to_string,
            Value::BigInt(JsBigInt::one()),
            &[Value::BigInt(JsBigInt::one())],
        ),
    ];
    let radix_log = plain_value(context.get_property(&global, &radix_log).unwrap());
    observations.push(format!("proto={}|{radix_log}", proto.join("|")));

    let wrapper = expect_object(
        context
            .call(&object_value_of, Value::BigInt(JsBigInt::from(123)), &[])
            .unwrap(),
        "Object.prototype.valueOf.call(123n)",
    );
    let wrapper2 = expect_object(
        context
            .call(&object_value_of, Value::BigInt(JsBigInt::from(123)), &[])
            .unwrap(),
        "second Object.prototype.valueOf.call(123n)",
    );
    let object_values = [
        "object".to_owned(),
        runtime
            .get_prototype_of(&wrapper)
            .unwrap()
            .is_some_and(|prototype| prototype == bigint_prototype)
            .to_string(),
        runtime
            .own_property_keys(&wrapper)
            .unwrap()
            .len()
            .to_string(),
        observe_call_args(
            &runtime,
            &mut context,
            &value_of,
            Value::Object(wrapper.clone()),
            &[],
        ),
        (wrapper == wrapper2).to_string(),
        plain_value(
            context
                .call(&object_to_string, Value::BigInt(JsBigInt::from(123)), &[])
                .unwrap(),
        ),
        plain_value(
            context
                .call(&object_to_string, Value::Object(wrapper.clone()), &[])
                .unwrap(),
        ),
        plain_value(
            context
                .call(
                    &object_to_string,
                    Value::Object(bigint_prototype.clone()),
                    &[],
                )
                .unwrap(),
        ),
        plain_value(
            context
                .call(
                    &object_to_locale_string,
                    Value::BigInt(JsBigInt::from(123)),
                    &[],
                )
                .unwrap(),
        ),
    ];
    observations.push(format!("objects={}", object_values.join("|")));

    assert!(runtime.delete_property(&bigint_prototype, &tag).unwrap());
    observations.push(format!(
        "tag-delete={}",
        object_tags(&mut context, &object_to_string, &bigint_prototype, &wrapper,).join("|")
    ));
    define_data_key(
        &runtime,
        &bigint_prototype,
        &tag,
        Value::String(JsString::try_from_utf8("CustomBigInt").unwrap()),
        false,
        false,
        true,
    );
    observations.push(format!(
        "tag-custom={}",
        object_tags(&mut context, &object_to_string, &bigint_prototype, &wrapper,).join("|")
    ));

    install_primitive_accessors(&runtime, &mut context, &global, &bigint_prototype);
    let strict_get_result = context.eval("(1n).__strictGet").unwrap();
    let strict_get_this = global_value(&runtime, &mut context, &global, "strictGetThis");
    let sloppy_get_result = context.eval("(1n).__sloppyGet").unwrap();
    let sloppy_get_this = global_value(&runtime, &mut context, &global, "sloppyGetThis");
    let sloppy_get_unboxed = context
        .call(&value_of, sloppy_get_this.clone(), &[])
        .unwrap();
    let strict_set_result = context.eval("(1n).__strictSet = 7").unwrap();
    let strict_set_this = global_value(&runtime, &mut context, &global, "strictSetThis");
    let sloppy_set_result = context.eval("(1n).__sloppySet = 8").unwrap();
    let sloppy_set_this = global_value(&runtime, &mut context, &global, "sloppySetThis");
    let sloppy_set_unboxed = context
        .call(&value_of, sloppy_set_this.clone(), &[])
        .unwrap();
    let accessor_values = [
        strict_get_result,
        Value::String(JsString::try_from_utf8("bigint").unwrap()),
        Value::Bool(matches!(
            strict_get_this,
            Value::BigInt(value) if value == JsBigInt::one()
        )),
        sloppy_get_result,
        Value::String(JsString::try_from_utf8("object").unwrap()),
        Value::Bool(object_has_prototype(
            &runtime,
            &sloppy_get_this,
            &bigint_prototype,
        )),
        sloppy_get_unboxed,
        strict_set_result,
        Value::String(JsString::try_from_utf8("bigint").unwrap()),
        Value::Bool(matches!(
            strict_set_this,
            Value::BigInt(value) if value == JsBigInt::one()
        )),
        sloppy_set_result,
        Value::String(JsString::try_from_utf8("object").unwrap()),
        Value::Bool(object_has_prototype(
            &runtime,
            &sloppy_set_this,
            &bigint_prototype,
        )),
        sloppy_set_unboxed,
    ];
    observations.push(format!("accessors={}", join_values(&accessor_values)));

    define_data(
        &runtime,
        &bigint_prototype,
        "__rw",
        Value::Int(1),
        true,
        false,
        true,
    );
    define_data(
        &runtime,
        &bigint_prototype,
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
        &bigint_prototype,
        "__delete",
        Some(delete_getter),
        None,
    );
    let delete_key = runtime.intern_property_key("__delete").unwrap();
    let writes = [
        plain_value(context.eval("(1n).__rw = 2").unwrap()),
        plain_value(context.eval("(1n).__rw").unwrap()),
        observe_eval(
            &runtime,
            &mut context,
            r#"(function() { "use strict"; return (1n).__rw = 3; })()"#,
        ),
        plain_value(context.eval("(1n).__ro = 2").unwrap()),
        plain_value(context.eval("(1n).__ro").unwrap()),
        observe_eval(
            &runtime,
            &mut context,
            r#"(function() { "use strict"; return (1n).__ro = 3; })()"#,
        ),
        plain_value(context.eval("delete (1n).__delete").unwrap()),
        plain_value(context.get_property(&global, &delete_hit).unwrap()),
        runtime
            .has_own_property(&bigint_prototype, &delete_key)
            .unwrap()
            .to_string(),
        plain_value(
            context
                .eval(r#"(function() { "use strict"; return delete (1n).__delete; })()"#)
                .unwrap(),
        ),
    ];
    observations.push(format!("writes={}", writes.join("|")));

    observations
}

#[test]
fn bigint_cross_realm_routes_boxing_lookups_and_native_errors_to_the_defining_realm() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_global = first.global_object().unwrap();
    let second_global = second.global_object().unwrap();
    let first_prototype = first.bigint_prototype().unwrap();
    let second_prototype = second.bigint_prototype().unwrap();
    assert_ne!(first_prototype, second_prototype);

    let first_bigint = property_callable(&runtime, &mut first, &first_global, "BigInt");
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

    let first_wrapper = expect_object(
        second
            .call(
                &first_object_value_of,
                Value::BigInt(JsBigInt::from(7)),
                &[],
            )
            .unwrap(),
        "foreign Object.prototype.valueOf BigInt boxing",
    );
    let second_wrapper = expect_object(
        first
            .call(
                &second_object_value_of,
                Value::BigInt(JsBigInt::from(9)),
                &[],
            )
            .unwrap(),
        "second-realm Object.prototype.valueOf BigInt boxing",
    );
    assert_eq!(
        runtime.get_prototype_of(&first_wrapper).unwrap(),
        Some(first_prototype.clone()),
        "primitive boxing must use the native method's defining realm"
    );
    assert_eq!(
        runtime.get_prototype_of(&second_wrapper).unwrap(),
        Some(second_prototype.clone())
    );
    for (method, wrapper, expected) in [
        (&first_value_of, &first_wrapper, 7),
        (&second_value_of, &first_wrapper, 7),
        (&first_value_of, &second_wrapper, 9),
        (&second_value_of, &second_wrapper, 9),
    ] {
        assert_eq!(
            second
                .call(method, Value::Object(wrapper.clone()), &[])
                .unwrap(),
            Value::BigInt(JsBigInt::from(expected)),
            "BigInt wrapper branding must be realm-independent"
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
        "(function() { return (1n).__realmMarker; })",
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
        Value::String(JsString::try_from_utf8("FirstBigInt").unwrap()),
        false,
        false,
        true,
    );
    define_data_key(
        &runtime,
        &second_prototype,
        &tag,
        Value::String(JsString::try_from_utf8("SecondBigInt").unwrap()),
        false,
        false,
        true,
    );
    assert_eq!(
        second
            .call(&first_object_to_string, Value::BigInt(JsBigInt::one()), &[],)
            .unwrap(),
        Value::String(JsString::try_from_utf8("[object FirstBigInt]").unwrap()),
        "Object.prototype.toString must box a primitive in its defining realm"
    );

    let first_type_error = intrinsic_prototype(&runtime, &mut first, "TypeError");
    let second_type_error = intrinsic_prototype(&runtime, &mut second, "TypeError");
    let first_range_error = intrinsic_prototype(&runtime, &mut first, "RangeError");
    assert_ne!(first_type_error, second_type_error);
    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("bigint").unwrap()))
        .unwrap();
    assert_eq!(
        second.call(&first_bigint, Value::Undefined, &[Value::Symbol(symbol)],),
        Err(RuntimeError::Exception)
    );
    let conversion_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&conversion_error).unwrap(),
        Some(first_type_error.clone()),
        "BigInt conversion errors must use the constructor's defining realm"
    );

    assert_eq!(
        second.call(&first_bigint, Value::Undefined, &[Value::Float(1.5)],),
        Err(RuntimeError::Exception)
    );
    let range_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&range_error).unwrap(),
        Some(first_range_error)
    );

    let spoof = second.new_object().unwrap();
    assert_eq!(
        second.call(&first_value_of, Value::Object(spoof), &[]),
        Err(RuntimeError::Exception)
    );
    let brand_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&brand_error).unwrap(),
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
            &first_bigint,
            Value::Undefined,
            &[Value::Object(throwing_input)],
        ),
        Err(RuntimeError::Exception)
    );
    let user_error = take_exception_object(&mut first);
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(second_type_error),
        "an explicit user throw must retain the conversion function's realm"
    );

    drop(second_global);
}

#[test]
fn bigint_wrapper_keeps_its_realm_graph_alive_until_collection() {
    let runtime = Runtime::new();
    let wrapper = {
        let mut context = runtime.new_context();
        let object_prototype = context.object_prototype().unwrap();
        let object_value_of =
            property_callable(&runtime, &mut context, &object_prototype, "valueOf");
        expect_object(
            context
                .call(&object_value_of, Value::BigInt(JsBigInt::from(123)), &[])
                .unwrap(),
            "Object.prototype.valueOf BigInt wrapper",
        )
    };

    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        1,
        "the live wrapper must retain its prototype and defining context graph"
    );
    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().live,
        0,
        "BigInt contexts, prototypes, native functions, and wrappers must be collectable"
    );
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
    assert!(
        runtime
            .define_own_property(
                object,
                &key,
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
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(value) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
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

fn global_value(runtime: &Runtime, context: &mut Context, global: &ObjectRef, name: &str) -> Value {
    context
        .get_property(global, &runtime.intern_property_key(name).unwrap())
        .unwrap()
}

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(object) = value else {
        panic!("{description} did not produce an object");
    };
    object
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
    let to_string_tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    runtime
        .own_property_keys(object)
        .unwrap()
        .iter()
        .map(|key| {
            if key == &to_string_tag {
                "Symbol(Symbol.toStringTag)".to_owned()
            } else {
                runtime
                    .property_key_to_js_string(key)
                    .unwrap()
                    .to_utf8_lossy()
            }
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

fn function_signature(runtime: &Runtime, context: &mut Context, callable: &CallableRef) -> String {
    let name = plain_value(
        context
            .get_property(
                callable.as_object(),
                &runtime.intern_property_key("name").unwrap(),
            )
            .unwrap(),
    );
    let length = plain_value(
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

fn call_two(context: &mut Context, callable: &CallableRef, first: Value, second: Value) -> Value {
    context
        .call(callable, Value::Undefined, &[first, second])
        .unwrap()
}

fn parse_bigint(text: &str) -> JsBigInt {
    JsBigInt::parse_js_string(text).unwrap()
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

fn object_tags(
    context: &mut Context,
    object_to_string: &CallableRef,
    prototype: &ObjectRef,
    wrapper: &ObjectRef,
) -> Vec<String> {
    [
        Value::BigInt(JsBigInt::from(123)),
        Value::Object(wrapper.clone()),
        Value::Object(prototype.clone()),
    ]
    .into_iter()
    .map(|value| plain_value(context.call(object_to_string, value, &[]).unwrap()))
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

fn observe_construct_with_new_target(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    new_target: &CallableRef,
    arguments: &[Value],
) -> String {
    match context.construct_with_new_target(callable, new_target, arguments) {
        Ok(value) => plain_value(value),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("Rust construct failed outside JavaScript completion: {error}"),
    }
}

fn observe_bigint_radix_length(
    runtime: &Runtime,
    context: &mut Context,
    to_string: &CallableRef,
    value: &JsBigInt,
    radix: i32,
) -> String {
    match context.call(
        to_string,
        Value::BigInt(value.clone()),
        &[Value::Int(radix)],
    ) {
        Ok(Value::String(value)) => value.len().to_string(),
        Ok(value) => panic!("BigInt.prototype.toString returned {value:?}"),
        Err(RuntimeError::Exception) => take_error(runtime, context),
        Err(error) => panic!("BigInt radix call failed outside JavaScript completion: {error}"),
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
        panic!("BigInt operation did not throw an Error object");
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
        Value::Symbol(_) => "Symbol(bigint)".to_owned(),
        value => value
            .to_js_string()
            .expect("ordinary BigInt observation must stringify")
            .to_utf8_lossy(),
    }
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS BigInt intrinsic oracle");
    assert!(
        output.status.success(),
        "QuickJS BigInt intrinsic oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS BigInt intrinsic oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
