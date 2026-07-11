use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField, EvalOptions,
    JsBigInt, JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError,
    Value, WellKnownSymbol,
};

const ORACLE_PROBE: &str = r#"
function flags(object, key) {
    var descriptor = Object.getOwnPropertyDescriptor(object, key);
    return (descriptor.writable ? "1" : "0") +
           (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0");
}
function isConstructor(value) {
    try {
        Reflect.construct(function () {}, [], value);
        return true;
    } catch (_) {
        return false;
    }
}
function observe(thunk) {
    try { return String(thunk()); }
    catch (error) {
        if (error !== null && typeof error === "object")
            return "throw:" + error.name + ":" + error.message;
        return "throw:" + String(error);
    }
}

var globalNames = [
    "parseInt", "parseFloat", "isNaN", "isFinite",
    "Infinity", "NaN", "undefined", "Number", "Boolean"
];
print("global-order=" + Reflect.ownKeys(globalThis).filter(function (key) {
    return globalNames.indexOf(key) >= 0;
}).join(","));

print("graph=" + [
    Reflect.ownKeys(isNaN).map(String).join(","),
    Reflect.ownKeys(isFinite).map(String).join(","),
    flags(globalThis, "isNaN"), flags(globalThis, "isFinite"),
    isNaN.length, isNaN.name, isFinite.length, isFinite.name,
    flags(isNaN, "length"), flags(isNaN, "name"),
    flags(isFinite, "length"), flags(isFinite, "name"),
    Object.getPrototypeOf(isNaN) === Function.prototype,
    Object.getPrototypeOf(isFinite) === Function.prototype,
    isConstructor(isNaN), isConstructor(isFinite),
    isNaN === Number.isNaN, isFinite === Number.isFinite
].join("|"));

var raw = [
    undefined, null, false, true, 0, -0, NaN, Infinity, -Infinity,
    Number.MIN_VALUE, Number.MAX_VALUE, "", " ", "0", "1.5", "Infinity", "x",
    "0x10", "0b10", "0o10", "-0x10", "-0b10", "+Infinity", "-Infinity", "12x"
];
print("raw-isNaN=" + raw.map(function (value) { return isNaN(value); }).join(","));
print("raw-isFinite=" + raw.map(function (value) { return isFinite(value); }).join(","));

var extraHit = false;
var extra = {
    [Symbol.toPrimitive]: function () { extraHit = true; throw 99; }
};
print("call-shape=" + [
    isNaN.call({ ignored: true }, "x", extra),
    isFinite.call(Symbol("this"), 1, extra),
    extraHit
].join("|"));

var coercionLog = "";
var nanExotic = {
    [Symbol.toPrimitive]: function (hint) {
        coercionLog += "nan:" + hint + "|";
        return "x";
    }
};
var finiteExotic = {
    [Symbol.toPrimitive]: function (hint) {
        coercionLog += "finite:" + hint + "|";
        return "1";
    }
};
var fallback = {
    valueOf: function () { coercionLog += "valueOf|"; return {}; },
    toString: function () { coercionLog += "toString|"; return "1"; }
};
var invalid = {
    [Symbol.toPrimitive]: function () { return {}; }
};
var arbitraryThrow = {
    [Symbol.toPrimitive]: function () { throw 71; }
};
print("objects=" + [
    isNaN(nanExotic), isFinite(finiteExotic), isFinite(fallback),
    coercionLog,
    observe(function () { return isNaN(invalid); }),
    observe(function () { return isFinite(arbitraryThrow); }),
    isNaN(new Number(NaN)), isFinite(new Number(0)),
    Number.isNaN(new Number(NaN)), Number.isFinite(new Number(0))
].join("|"));

print("type-errors=" + [
    observe(function () { return isNaN(1n); }),
    observe(function () { return isFinite(1n); }),
    observe(function () { return isNaN(Symbol("nan")); }),
    observe(function () { return isFinite(Symbol("finite")); }),
    Number.isNaN(1n), Number.isFinite(Symbol("static"))
].join("|"));

var originalGlobalNaN = isNaN;
var originalGlobalFinite = isFinite;
var originalStaticNaN = Number.isNaN;
var originalStaticFinite = Number.isFinite;
globalThis.isNaN = function () { return "replacement-global"; };
Number.isFinite = function () { return "replacement-static"; };
print("mutation=" + [
    globalThis.isNaN !== originalGlobalNaN,
    Number.isNaN === originalStaticNaN,
    Number.isNaN !== globalThis.isNaN,
    Number.isFinite !== originalStaticFinite,
    globalThis.isFinite === originalGlobalFinite,
    globalThis.isFinite !== Number.isFinite
].join("|"));
"#;

const EXPECTED_OBSERVATIONS: &[&str] = &[
    "global-order=parseInt,parseFloat,isNaN,isFinite,Infinity,NaN,undefined,Number,Boolean",
    "graph=length,name|length,name|101|101|1|isNaN|1|isFinite|001|001|001|001|true|true|false|false|false|false",
    "raw-isNaN=true,false,false,false,false,false,true,false,false,false,false,false,false,false,false,false,true,false,false,false,true,true,false,false,true",
    "raw-isFinite=false,true,true,true,true,true,false,false,false,true,true,true,true,true,true,false,false,true,true,true,false,false,false,false,false",
    "call-shape=true|true|false",
    "objects=true|true|true|nan:number|finite:number|valueOf|toString||throw:TypeError:toPrimitive|throw:71|true|true|false|false",
    "type-errors=throw:TypeError:cannot convert bigint to number|throw:TypeError:cannot convert bigint to number|throw:TypeError:cannot convert symbol to number|throw:TypeError:cannot convert symbol to number|false|false",
    "mutation=true|true|true|true|true|true",
];

#[test]
fn global_numeric_predicates_match_pinned_quickjs() {
    let rust = rust_observations();
    assert_eq!(
        rust, EXPECTED_OBSERVATIONS,
        "host-side predicate contract changed"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP global numeric predicate differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "global numeric predicates differed from pinned QuickJS"
    );
}

#[test]
fn global_numeric_predicate_errors_use_the_defining_realm() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_nan = global_callable(&runtime, &mut first, "isNaN");
    let first_finite = global_callable(&runtime, &mut first, "isFinite");
    let first_function_prototype = first.function_prototype().unwrap();
    assert_eq!(
        runtime.get_prototype_of(first_nan.as_object()).unwrap(),
        Some(first_function_prototype.clone())
    );
    assert_eq!(
        runtime.get_prototype_of(first_finite.as_object()).unwrap(),
        Some(first_function_prototype)
    );

    let first_type_error = intrinsic_prototype(&runtime, &mut first, "TypeError");
    let second_type_error = intrinsic_prototype(&runtime, &mut second, "TypeError");
    assert_ne!(first_type_error, second_type_error);

    assert_eq!(
        second.construct(&first_nan, &[]),
        Err(RuntimeError::Exception)
    );
    let constructor_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&constructor_error).unwrap(),
        Some(second_type_error.clone()),
        "non-constructor rejection must use the caller realm"
    );

    let symbol = runtime.new_symbol(Some(JsString::from("foreign"))).unwrap();
    assert_eq!(
        second.call(&first_nan, Value::Undefined, &[Value::Symbol(symbol)]),
        Err(RuntimeError::Exception)
    );
    let symbol_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&symbol_error).unwrap(),
        Some(first_type_error.clone())
    );
    assert_eq!(
        error_text(&runtime, &mut second, &symbol_error, "message"),
        "cannot convert symbol to number"
    );

    let second_global = second.global_object().unwrap();
    let bad_result = second.new_object().unwrap();
    define_data(
        &runtime,
        &second_global,
        "badPredicatePrimitive",
        Value::Object(bad_result),
    );
    let bad_conversion = eval_callable(
        &runtime,
        &mut second,
        "(function() { return badPredicatePrimitive; })",
    );
    let bad_input = second.new_object().unwrap();
    define_data_key(
        &runtime,
        &bad_input,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(bad_conversion.as_object().clone()),
    );
    assert_eq!(
        second.call(&first_finite, Value::Undefined, &[Value::Object(bad_input)],),
        Err(RuntimeError::Exception)
    );
    let framework_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&framework_error).unwrap(),
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
    );
    assert_eq!(
        first.call(
            &first_nan,
            Value::Undefined,
            &[Value::Object(throwing_input)],
        ),
        Err(RuntimeError::Exception)
    );
    let user_error = take_exception_object(&mut first);
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(second_type_error)
    );
}

#[test]
fn global_numeric_predicate_keeps_its_defining_realm_alive_until_collection() {
    let runtime = Runtime::new();
    let predicate = {
        let mut context = runtime.new_context();
        global_callable(&runtime, &mut context, "isNaN")
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(predicate);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn global_numeric_predicate_native_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP global numeric predicate stack differential: set QJS_ORACLE to upstream qjs"
        );
        return;
    };

    for source in ["isNaN(1n)", "isFinite(1n)", "new isNaN()", "new isFinite()"] {
        assert_eq!(
            rust_uncaught_error(source),
            oracle_uncaught_error(&oracle, source),
            "global numeric predicate stderr differed for {source:?}"
        );
    }
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let is_nan = global_callable(&runtime, &mut context, "isNaN");
    let is_finite = global_callable(&runtime, &mut context, "isFinite");
    let number = global_callable(&runtime, &mut context, "Number");
    let number_is_nan = property_callable(&runtime, &mut context, number.as_object(), "isNaN");
    let number_is_finite =
        property_callable(&runtime, &mut context, number.as_object(), "isFinite");

    let implemented_globals = [
        "parseInt",
        "parseFloat",
        "isNaN",
        "isFinite",
        "Infinity",
        "NaN",
        "undefined",
        "Number",
        "Boolean",
    ];
    let global_order = runtime
        .own_property_keys(&global)
        .unwrap()
        .iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(key)
                .unwrap()
                .to_utf8_lossy()
        })
        .filter(|name| implemented_globals.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let mut observations = vec![format!("global-order={global_order}")];

    let function_prototype = context.function_prototype().unwrap();
    observations.push(format!(
        "graph={}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        own_key_names(&runtime, is_nan.as_object()),
        own_key_names(&runtime, is_finite.as_object()),
        data_flags(&runtime, &global, "isNaN"),
        data_flags(&runtime, &global, "isFinite"),
        callable_int_property(&runtime, &mut context, &is_nan, "length"),
        callable_string_property(&runtime, &mut context, &is_nan, "name"),
        callable_int_property(&runtime, &mut context, &is_finite, "length"),
        callable_string_property(&runtime, &mut context, &is_finite, "name"),
        data_flags(&runtime, is_nan.as_object(), "length"),
        data_flags(&runtime, is_nan.as_object(), "name"),
        data_flags(&runtime, is_finite.as_object(), "length"),
        data_flags(&runtime, is_finite.as_object(), "name"),
        runtime
            .get_prototype_of(is_nan.as_object())
            .unwrap()
            .is_some_and(|prototype| prototype == function_prototype),
        runtime
            .get_prototype_of(is_finite.as_object())
            .unwrap()
            .is_some_and(|prototype| prototype == function_prototype),
        runtime.is_constructor(is_nan.as_object()).unwrap(),
        runtime.is_constructor(is_finite.as_object()).unwrap(),
        is_nan.as_object() == number_is_nan.as_object(),
        is_finite.as_object() == number_is_finite.as_object(),
    ));

    let raw = [
        Value::Undefined,
        Value::Null,
        Value::Bool(false),
        Value::Bool(true),
        Value::Int(0),
        Value::Float(-0.0),
        Value::Float(f64::NAN),
        Value::Float(f64::INFINITY),
        Value::Float(f64::NEG_INFINITY),
        Value::Float(f64::from_bits(1)),
        Value::Float(f64::MAX),
        Value::String(JsString::from("")),
        Value::String(JsString::from(" ")),
        Value::String(JsString::from("0")),
        Value::String(JsString::from("1.5")),
        Value::String(JsString::from("Infinity")),
        Value::String(JsString::from("x")),
        Value::String(JsString::from("0x10")),
        Value::String(JsString::from("0b10")),
        Value::String(JsString::from("0o10")),
        Value::String(JsString::from("-0x10")),
        Value::String(JsString::from("-0b10")),
        Value::String(JsString::from("+Infinity")),
        Value::String(JsString::from("-Infinity")),
        Value::String(JsString::from("12x")),
    ];
    observations.push(format!(
        "raw-isNaN={}",
        raw.iter()
            .cloned()
            .map(|value| call_bool(&mut context, &is_nan, value).to_string())
            .collect::<Vec<_>>()
            .join(",")
    ));
    observations.push(format!(
        "raw-isFinite={}",
        raw.iter()
            .cloned()
            .map(|value| call_bool(&mut context, &is_finite, value).to_string())
            .collect::<Vec<_>>()
            .join(",")
    ));

    define_data(&runtime, &global, "extraHit", Value::Bool(false));
    let extra = context.new_object().unwrap();
    let extra_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function() { extraHit = true; throw 99; })",
    );
    define_data_key(
        &runtime,
        &extra,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(extra_conversion.as_object().clone()),
    );
    let ignored_this = context.new_object().unwrap();
    let symbol_this = runtime.new_symbol(Some(JsString::from("this"))).unwrap();
    let nan_call = context
        .call(
            &is_nan,
            Value::Object(ignored_this),
            &[
                Value::String(JsString::from("x")),
                Value::Object(extra.clone()),
            ],
        )
        .unwrap();
    let finite_call = context
        .call(
            &is_finite,
            Value::Symbol(symbol_this),
            &[Value::Int(1), Value::Object(extra)],
        )
        .unwrap();
    observations.push(format!(
        "call-shape={}|{}|{}",
        plain_value(nan_call),
        plain_value(finite_call),
        plain_value(global_value(&runtime, &mut context, &global, "extraHit")),
    ));

    define_data(
        &runtime,
        &global,
        "coercionLog",
        Value::String(JsString::from("")),
    );
    let nan_exotic = context.new_object().unwrap();
    let nan_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { coercionLog += 'nan:' + hint + '|'; return 'x'; })",
    );
    define_data_key(
        &runtime,
        &nan_exotic,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(nan_conversion.as_object().clone()),
    );
    let finite_exotic = context.new_object().unwrap();
    let finite_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { coercionLog += 'finite:' + hint + '|'; return '1'; })",
    );
    define_data_key(
        &runtime,
        &finite_exotic,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(finite_conversion.as_object().clone()),
    );

    let fallback_result = context.new_object().unwrap();
    define_data(
        &runtime,
        &global,
        "predicateFallbackObject",
        Value::Object(fallback_result),
    );
    let fallback = context.new_object().unwrap();
    let value_of = eval_callable(
        &runtime,
        &mut context,
        "(function() { coercionLog += 'valueOf|'; return predicateFallbackObject; })",
    );
    let to_string = eval_callable(
        &runtime,
        &mut context,
        "(function() { coercionLog += 'toString|'; return '1'; })",
    );
    define_data(
        &runtime,
        &fallback,
        "valueOf",
        Value::Object(value_of.as_object().clone()),
    );
    define_data(
        &runtime,
        &fallback,
        "toString",
        Value::Object(to_string.as_object().clone()),
    );

    let invalid_result = context.new_object().unwrap();
    define_data(
        &runtime,
        &global,
        "invalidPredicatePrimitive",
        Value::Object(invalid_result),
    );
    let invalid = context.new_object().unwrap();
    let invalid_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function() { return invalidPredicatePrimitive; })",
    );
    define_data_key(
        &runtime,
        &invalid,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(invalid_conversion.as_object().clone()),
    );
    let arbitrary = context.new_object().unwrap();
    let arbitrary_conversion = eval_callable(&runtime, &mut context, "(function() { throw 71; })");
    define_data_key(
        &runtime,
        &arbitrary,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(arbitrary_conversion.as_object().clone()),
    );

    let boxed_nan = expect_object(
        context
            .construct(&number, &[Value::Float(f64::NAN)])
            .unwrap(),
        "new Number(NaN)",
    );
    let boxed_zero = expect_object(
        context.construct(&number, &[Value::Int(0)]).unwrap(),
        "new Number(0)",
    );
    let object_results = [
        context
            .call(&is_nan, Value::Undefined, &[Value::Object(nan_exotic)])
            .unwrap(),
        context
            .call(
                &is_finite,
                Value::Undefined,
                &[Value::Object(finite_exotic)],
            )
            .unwrap(),
        context
            .call(&is_finite, Value::Undefined, &[Value::Object(fallback)])
            .unwrap(),
    ];
    let coercion_log = global_value(&runtime, &mut context, &global, "coercionLog");
    let invalid_result = observe_call(&runtime, &mut context, &is_nan, Value::Object(invalid));
    let arbitrary_result =
        observe_call(&runtime, &mut context, &is_finite, Value::Object(arbitrary));
    observations.push(format!(
        "objects={}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        plain_value(object_results[0].clone()),
        plain_value(object_results[1].clone()),
        plain_value(object_results[2].clone()),
        plain_value(coercion_log),
        invalid_result,
        arbitrary_result,
        call_bool(&mut context, &is_nan, Value::Object(boxed_nan.clone())),
        call_bool(&mut context, &is_finite, Value::Object(boxed_zero.clone())),
        call_bool(&mut context, &number_is_nan, Value::Object(boxed_nan)),
        call_bool(&mut context, &number_is_finite, Value::Object(boxed_zero)),
    ));

    let nan_symbol = runtime.new_symbol(Some(JsString::from("nan"))).unwrap();
    let finite_symbol = runtime.new_symbol(Some(JsString::from("finite"))).unwrap();
    let static_symbol = runtime.new_symbol(Some(JsString::from("static"))).unwrap();
    observations.push(format!(
        "type-errors={}|{}|{}|{}|{}|{}",
        observe_call(
            &runtime,
            &mut context,
            &is_nan,
            Value::BigInt(JsBigInt::one()),
        ),
        observe_call(
            &runtime,
            &mut context,
            &is_finite,
            Value::BigInt(JsBigInt::one()),
        ),
        observe_call(&runtime, &mut context, &is_nan, Value::Symbol(nan_symbol),),
        observe_call(
            &runtime,
            &mut context,
            &is_finite,
            Value::Symbol(finite_symbol),
        ),
        call_bool(&mut context, &number_is_nan, Value::BigInt(JsBigInt::one()),),
        call_bool(
            &mut context,
            &number_is_finite,
            Value::Symbol(static_symbol),
        ),
    ));

    let replacement_global = eval_callable(
        &runtime,
        &mut context,
        "(function() { return 'replacement-global'; })",
    );
    assert!(
        context
            .set_property(
                &global,
                &runtime.intern_property_key("isNaN").unwrap(),
                Value::Object(replacement_global.as_object().clone()),
            )
            .unwrap()
    );
    let replacement_static = eval_callable(
        &runtime,
        &mut context,
        "(function() { return 'replacement-static'; })",
    );
    assert!(
        context
            .set_property(
                number.as_object(),
                &runtime.intern_property_key("isFinite").unwrap(),
                Value::Object(replacement_static.as_object().clone()),
            )
            .unwrap()
    );
    let current_global_nan = property_callable(&runtime, &mut context, &global, "isNaN");
    let current_static_nan = property_callable(&runtime, &mut context, number.as_object(), "isNaN");
    let current_global_finite = property_callable(&runtime, &mut context, &global, "isFinite");
    let current_static_finite =
        property_callable(&runtime, &mut context, number.as_object(), "isFinite");
    observations.push(format!(
        "mutation={}|{}|{}|{}|{}|{}",
        current_global_nan.as_object() != is_nan.as_object(),
        current_static_nan.as_object() == number_is_nan.as_object(),
        current_static_nan.as_object() != current_global_nan.as_object(),
        current_static_finite.as_object() != number_is_finite.as_object(),
        current_global_finite.as_object() == is_finite.as_object(),
        current_global_finite.as_object() != current_static_finite.as_object(),
    ));

    observations
}

fn global_callable(runtime: &Runtime, context: &mut Context, name: &str) -> CallableRef {
    let global = context.global_object().unwrap();
    property_callable(runtime, context, &global, name)
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

fn intrinsic_prototype(runtime: &Runtime, context: &mut Context, name: &str) -> ObjectRef {
    let constructor = global_callable(runtime, context, name);
    let prototype = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(prototype) = context
        .get_property(constructor.as_object(), &prototype)
        .unwrap()
    else {
        panic!("{name}.prototype was not an object");
    };
    prototype
}

fn callable_int_property(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    name: &str,
) -> i32 {
    let Value::Int(value) = context
        .get_property(
            callable.as_object(),
            &runtime.intern_property_key(name).unwrap(),
        )
        .unwrap()
    else {
        panic!("callable {name} was not an Int");
    };
    value
}

fn callable_string_property(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(
            callable.as_object(),
            &runtime.intern_property_key(name).unwrap(),
        )
        .unwrap()
    else {
        panic!("callable {name} was not a String");
    };
    value.to_utf8_lossy()
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> String {
    runtime
        .own_property_keys(object)
        .unwrap()
        .iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn data_flags(runtime: &Runtime, object: &ObjectRef, name: &str) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        writable,
        enumerable,
        configurable,
        ..
    }) = runtime.get_own_property(object, &key).unwrap()
    else {
        panic!("{name} was not an own data property");
    };
    format!(
        "{}{}{}",
        u8::from(writable),
        u8::from(enumerable),
        u8::from(configurable)
    )
}

fn define_data(runtime: &Runtime, object: &ObjectRef, name: &str, value: Value) {
    let key = runtime.intern_property_key(name).unwrap();
    define_data_key(runtime, object, &key, value);
}

fn define_data_key(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey, value: Value) {
    assert!(
        runtime
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap(),
        "host data-property definition was rejected"
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

fn global_value(runtime: &Runtime, context: &mut Context, global: &ObjectRef, name: &str) -> Value {
    context
        .get_property(global, &runtime.intern_property_key(name).unwrap())
        .unwrap()
}

fn call_bool(context: &mut Context, callable: &CallableRef, argument: Value) -> bool {
    let Value::Bool(value) = context
        .call(callable, Value::Undefined, &[argument])
        .unwrap()
    else {
        panic!("numeric predicate did not return a Boolean");
    };
    value
}

fn observe_call(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    argument: Value,
) -> String {
    match context.call(callable, Value::Undefined, &[argument]) {
        Ok(value) => plain_value(value),
        Err(RuntimeError::Exception) => {
            let exception = context.take_exception().unwrap().unwrap();
            if let Value::Object(error) = exception {
                format!(
                    "throw:{}:{}",
                    error_text(runtime, context, &error, "name"),
                    error_text(runtime, context, &error, "message")
                )
            } else {
                format!("throw:{}", plain_value(exception))
            }
        }
        Err(error) => panic!("numeric predicate returned engine error: {error}"),
    }
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("operation did not throw an object");
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

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(object) = value else {
        panic!("{description} did not produce an object");
    };
    object
}

fn plain_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) if value.is_nan() => "NaN".to_owned(),
        Value::Float(value) if value == f64::INFINITY => "Infinity".to_owned(),
        Value::Float(value) if value == f64::NEG_INFINITY => "-Infinity".to_owned(),
        Value::Float(value) if value == 0.0 && value.is_sign_negative() => "-0".to_owned(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Symbol(_) => "Symbol".to_owned(),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

fn rust_uncaught_error(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval_with_options(source, &EvalOptions::new("<cmdline>")),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("uncaught predicate error was not an Error object");
    };
    let name = error_text(&runtime, &mut context, &error, "name");
    let message = error_text(&runtime, &mut context, &error, "message");
    let stack = error_text(&runtime, &mut context, &error, "stack");
    format!("{name}: {message}\n{stack}")
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS global numeric predicate oracle");
    assert!(
        output.status.success(),
        "QuickJS global numeric predicate oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS global numeric predicate oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn oracle_uncaught_error(oracle: &OsStr, source: &str) -> String {
    let output = Command::new(oracle)
        .args(["-e", source])
        .output()
        .expect("run QuickJS global numeric predicate stack oracle");
    assert_eq!(output.status.code(), Some(1));
    String::from_utf8(output.stderr).expect("QuickJS predicate stderr was not UTF-8")
}
