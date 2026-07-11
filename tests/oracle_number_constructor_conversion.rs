use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{
    AccessorValue, CallableRef, Context, DescriptorField, JsBigInt, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value, WellKnownSymbol,
};

const ORACLE_PROBE: &str = r#"
function numberText(value) {
    if (Number.isNaN(value)) return "NaN";
    if (Object.is(value, -0)) return "-0";
    return String(value);
}
function observe(thunk) {
    try { return numberText(thunk()); }
    catch (error) {
        if (error !== null && typeof error === "object")
            return "throw:" + error.name + ":" + error.message;
        return "throw:" + String(error);
    }
}

print("call=" + [
    Number(), Number(undefined), Number(null), Number(false), Number(true),
    Number(17), Number(3.5), Number(-0), Number(" 0x10 "), Number("1.25")
].map(numberText).join("|"));

print("bigint=" + [
    Number(9007199254740993n),
    Number(9007199254740995n),
    Number(18446744073709553664n),
    Number(18446744073709553665n),
    Number(1n << 1024n),
    Number(-(1n << 1024n))
].map(numberText).join("|"));

var exoticLog = "";
var exotic = {
    [Symbol.toPrimitive]: function(hint) {
        exoticLog += "exotic:" + hint + "|";
        return 41;
    }
};
var fallbackLog = "";
var fallback = {
    valueOf: function() { fallbackLog += "valueOf|"; return {}; },
    toString: function() { fallbackLog += "toString|"; return "42"; }
};
var throwing = {
    [Symbol.toPrimitive]: function() { throw 77; }
};
print("object=" + [
    numberText(Number(exotic)), exoticLog,
    numberText(Number(fallback)), fallbackLog,
    observe(function() { return Number(throwing); })
].join("|"));
print("symbol=" + observe(function() { return Number(Symbol("number")); }));
"#;

const EXPECTED_CONSTRUCTOR_OBSERVATIONS: &[&str] = &[
    "call=0|NaN|0|0|1|17|3.5|-0|16|1.25",
    "bigint=9007199254740992|9007199254740996|18446744073709552000|18446744073709556000|Infinity|-Infinity",
    "object=41|exotic:number||42|valueOf|toString||throw:77",
    "symbol=throw:TypeError:cannot convert symbol to number",
];

#[test]
fn number_call_conversion_matches_the_pinned_quickjs_probe() {
    let rust = rust_constructor_observations();
    assert_eq!(
        rust, EXPECTED_CONSTRUCTOR_OBSERVATIONS,
        "host-side Number conversion contract changed"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Number constructor differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "Number constructor conversion differed from pinned QuickJS"
    );
}

#[test]
fn number_construct_converts_before_new_target_prototype_and_uses_its_realm() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_global = first.global_object().unwrap();
    let second_global = second.global_object().unwrap();
    let first_number = global_callable(&runtime, &mut first, "Number");
    let second_number = global_callable(&runtime, &mut second, "Number");
    let first_prototype = constructor_prototype(&runtime, &mut first, &first_number);
    let second_prototype = constructor_prototype(&runtime, &mut second, &second_number);
    let first_value_of = property_callable(&runtime, &mut first, &first_prototype, "valueOf");
    let second_value_of = property_callable(&runtime, &mut second, &second_prototype, "valueOf");

    let default_wrapper = expect_object(
        first
            .construct(&first_number, &[Value::Float(-0.0)])
            .unwrap(),
        "default Number construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&default_wrapper).unwrap(),
        Some(first_prototype.clone())
    );
    assert!(
        runtime
            .own_property_keys(&default_wrapper)
            .unwrap()
            .is_empty()
    );
    assert_number(
        first
            .call(&first_value_of, Value::Object(default_wrapper), &[])
            .unwrap(),
        -0.0,
    );

    define_global(
        &runtime,
        &first_global,
        "orderLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let custom_prototype = second.new_object().unwrap();
    define_global(
        &runtime,
        &first_global,
        "customNumberPrototype",
        Value::Object(custom_prototype.clone()),
    );

    let conversion = eval_callable(
        &runtime,
        &mut first,
        "(function(hint) { orderLog += 'convert:' + hint + '|'; return 7; })",
    );
    let argument = first.new_object().unwrap();
    define_data_key(
        &runtime,
        &argument,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(conversion.as_object().clone()),
    );

    let custom_target = bound_constructor(&runtime, &mut second);
    let prototype_getter = eval_callable(
        &runtime,
        &mut first,
        "(function() { orderLog += 'prototype|'; return customNumberPrototype; })",
    );
    define_getter(
        &runtime,
        custom_target.as_object(),
        "prototype",
        prototype_getter,
    );
    let custom_wrapper = expect_object(
        first
            .construct_with_new_target(
                &first_number,
                &custom_target,
                &[Value::Object(argument.clone())],
            )
            .unwrap(),
        "custom-newTarget Number construction",
    );
    assert_eq!(
        string_global(&runtime, &mut first, &first_global, "orderLog"),
        "convert:number|prototype|"
    );
    assert_eq!(
        runtime.get_prototype_of(&custom_wrapper).unwrap(),
        Some(custom_prototype)
    );
    assert_number(
        first
            .call(&second_value_of, Value::Object(custom_wrapper.clone()), &[])
            .unwrap(),
        7.0,
    );

    // A non-object prototype falls back to the callable newTarget's realm,
    // not the Number constructor's defining or caller realm.
    let fallback_target = bound_constructor(&runtime, &mut second);
    define_data(
        &runtime,
        fallback_target.as_object(),
        "prototype",
        Value::Int(1),
    );
    let fallback_wrapper = expect_object(
        first
            .construct_with_new_target(&first_number, &fallback_target, &[Value::Int(9)])
            .unwrap(),
        "fallback-newTarget Number construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&fallback_wrapper).unwrap(),
        Some(second_prototype.clone())
    );
    assert_number(
        first
            .call(
                &first_value_of,
                Value::Object(fallback_wrapper.clone()),
                &[],
            )
            .unwrap(),
        9.0,
    );

    // Using the other realm's Number itself as newTarget selects its explicit
    // prototype. Number branding is class-based and works with either realm's
    // valueOf method.
    let cross_wrapper = expect_object(
        first
            .construct_with_new_target(&first_number, &second_number, &[Value::Int(11)])
            .unwrap(),
        "cross-realm Number construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&cross_wrapper).unwrap(),
        Some(second_prototype)
    );
    assert_number(
        first
            .call(&first_value_of, Value::Object(cross_wrapper.clone()), &[])
            .unwrap(),
        11.0,
    );
    assert_number(
        second
            .call(&second_value_of, Value::Object(cross_wrapper), &[])
            .unwrap(),
        11.0,
    );

    // Keep the second global live until all realm-dependent assertions above
    // have completed; this also makes an accidental caller-realm fallback
    // distinguishable from the newTarget-realm result.
    drop(second_global);
}

#[test]
fn number_construct_preserves_conversion_and_prototype_getter_throws_in_order() {
    let runtime = Runtime::new();
    let mut constructor_context = runtime.new_context();
    let mut target_context = runtime.new_context();
    let global = constructor_context.global_object().unwrap();
    let number = global_callable(&runtime, &mut constructor_context, "Number");
    define_global(
        &runtime,
        &global,
        "orderLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );

    let conversion_throw = eval_callable(
        &runtime,
        &mut constructor_context,
        "(function() { orderLog += 'convert-throw|'; throw 71; })",
    );
    let throwing_argument = constructor_context.new_object().unwrap();
    define_data_key(
        &runtime,
        &throwing_argument,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(conversion_throw.as_object().clone()),
    );
    let target = bound_constructor(&runtime, &mut target_context);
    let getter = eval_callable(
        &runtime,
        &mut constructor_context,
        "(function() { orderLog += 'prototype-should-not-run|'; return null; })",
    );
    define_getter(&runtime, target.as_object(), "prototype", getter);
    assert_eq!(
        constructor_context.construct_with_new_target(
            &number,
            &target,
            &[Value::Object(throwing_argument)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        constructor_context.take_exception().unwrap(),
        Some(Value::Int(71))
    );
    assert_eq!(
        string_global(&runtime, &mut constructor_context, &global, "orderLog"),
        "convert-throw|"
    );

    constructor_context.eval("orderLog = ''").unwrap();
    let conversion = eval_callable(
        &runtime,
        &mut constructor_context,
        "(function(hint) { orderLog += 'convert:' + hint + '|'; return 3; })",
    );
    let argument = constructor_context.new_object().unwrap();
    define_data_key(
        &runtime,
        &argument,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(conversion.as_object().clone()),
    );
    let getter_throw_target = bound_constructor(&runtime, &mut target_context);
    let throwing_getter = eval_callable(
        &runtime,
        &mut constructor_context,
        "(function() { orderLog += 'prototype-throw|'; throw 72; })",
    );
    define_getter(
        &runtime,
        getter_throw_target.as_object(),
        "prototype",
        throwing_getter,
    );
    assert_eq!(
        constructor_context.construct_with_new_target(
            &number,
            &getter_throw_target,
            &[Value::Object(argument)],
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        constructor_context.take_exception().unwrap(),
        Some(Value::Int(72))
    );
    assert_eq!(
        string_global(&runtime, &mut constructor_context, &global, "orderLog"),
        "convert:number|prototype-throw|"
    );
}

#[test]
fn number_constructor_and_brand_errors_use_the_defining_realm() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_number = global_callable(&runtime, &mut first, "Number");
    let first_prototype = constructor_prototype(&runtime, &mut first, &first_number);
    let first_value_of = property_callable(&runtime, &mut first, &first_prototype, "valueOf");
    let first_type_error = intrinsic_prototype(&runtime, &mut first, "TypeError");
    let second_type_error = intrinsic_prototype(&runtime, &mut second, "TypeError");
    assert_ne!(first_type_error, second_type_error);

    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("number").unwrap()))
        .unwrap();
    assert_eq!(
        second.call(&first_number, Value::Undefined, &[Value::Symbol(symbol)],),
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

    // A framework error produced after a foreign @@toPrimitive returns an
    // object still belongs to the Number constructor's defining realm.
    let second_global = second.global_object().unwrap();
    let bad_primitive_result = second.new_object().unwrap();
    define_global(
        &runtime,
        &second_global,
        "badPrimitiveResult",
        Value::Object(bad_primitive_result),
    );
    let bad_conversion = eval_callable(
        &runtime,
        &mut second,
        "(function() { return badPrimitiveResult; })",
    );
    let bad_input = second.new_object().unwrap();
    define_data_key(
        &runtime,
        &bad_input,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(bad_conversion.as_object().clone()),
    );
    assert_eq!(
        second.call(&first_number, Value::Undefined, &[Value::Object(bad_input)],),
        Err(RuntimeError::Exception)
    );
    let framework_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&framework_error).unwrap(),
        Some(first_type_error.clone())
    );

    // An Error explicitly thrown by user conversion code keeps that code's
    // realm and is propagated without wrapping.
    let user_throw = eval_callable(
        &runtime,
        &mut second,
        "(function() { throw new TypeError('user conversion'); })",
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
            &first_number,
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

    // A branded method creates its own TypeError in the method's defining
    // realm, regardless of the caller or receiver realm.
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
    assert_eq!(
        error_text(&runtime, &mut second, &brand_error, "message"),
        "not a number"
    );
}

#[test]
fn number_wrapper_keeps_its_realm_graph_alive_until_collection() {
    let runtime = Runtime::new();
    let wrapper = {
        let mut context = runtime.new_context();
        let number = global_callable(&runtime, &mut context, "Number");
        expect_object(
            context.construct(&number, &[Value::Float(-0.0)]).unwrap(),
            "new Number(-0)",
        )
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

fn rust_constructor_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let number = global_callable(&runtime, &mut context, "Number");

    let call_values = [
        context.call(&number, Value::Undefined, &[]).unwrap(),
        context
            .call(&number, Value::Undefined, &[Value::Undefined])
            .unwrap(),
        context
            .call(&number, Value::Undefined, &[Value::Null])
            .unwrap(),
        context
            .call(&number, Value::Undefined, &[Value::Bool(false)])
            .unwrap(),
        context
            .call(&number, Value::Undefined, &[Value::Bool(true)])
            .unwrap(),
        context
            .call(&number, Value::Undefined, &[Value::Int(17)])
            .unwrap(),
        context
            .call(&number, Value::Undefined, &[Value::Float(3.5)])
            .unwrap(),
        context
            .call(&number, Value::Undefined, &[Value::Float(-0.0)])
            .unwrap(),
        context
            .call(
                &number,
                Value::Undefined,
                &[Value::String(JsString::try_from_utf8(" 0x10 ").unwrap())],
            )
            .unwrap(),
        context
            .call(
                &number,
                Value::Undefined,
                &[Value::String(JsString::try_from_utf8("1.25").unwrap())],
            )
            .unwrap(),
    ];

    let short_down = JsBigInt::parse_js_string("9007199254740993").unwrap();
    let short_up = JsBigInt::parse_js_string("9007199254740995").unwrap();
    let heap_down = JsBigInt::parse_js_string("18446744073709553664").unwrap();
    let heap_up = JsBigInt::parse_js_string("18446744073709553665").unwrap();
    let two_to_1024 = JsBigInt::parse_radix(&format!("1{}", "0".repeat(1024)), 2).unwrap();
    let negative_two_to_1024 = two_to_1024.neg().unwrap();
    let bigint_values = [
        short_down,
        short_up,
        heap_down,
        heap_up,
        two_to_1024,
        negative_two_to_1024,
    ]
    .map(|value| {
        context
            .call(&number, Value::Undefined, &[Value::BigInt(value)])
            .unwrap()
    });

    define_global(
        &runtime,
        &global,
        "exoticLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let exotic = context.new_object().unwrap();
    let exotic_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { exoticLog += 'exotic:' + hint + '|'; return 41; })",
    );
    define_data_key(
        &runtime,
        &exotic,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(exotic_conversion.as_object().clone()),
    );
    let exotic_value = context
        .call(&number, Value::Undefined, &[Value::Object(exotic)])
        .unwrap();

    define_global(
        &runtime,
        &global,
        "fallbackLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let fallback = context.new_object().unwrap();
    let fallback_result = context.new_object().unwrap();
    define_global(
        &runtime,
        &global,
        "fallbackObject",
        Value::Object(fallback_result),
    );
    let value_of = eval_callable(
        &runtime,
        &mut context,
        "(function() { fallbackLog += 'valueOf|'; return fallbackObject; })",
    );
    let to_string = eval_callable(
        &runtime,
        &mut context,
        "(function() { fallbackLog += 'toString|'; return '42'; })",
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
    let fallback_value = context
        .call(&number, Value::Undefined, &[Value::Object(fallback)])
        .unwrap();

    let throwing = context.new_object().unwrap();
    let thrower = eval_callable(&runtime, &mut context, "(function() { throw 77; })");
    define_data_key(
        &runtime,
        &throwing,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(thrower.as_object().clone()),
    );
    let thrown = match context.call(&number, Value::Undefined, &[Value::Object(throwing)]) {
        Err(RuntimeError::Exception) => context.take_exception().unwrap().unwrap(),
        result => panic!("throwing Number conversion unexpectedly returned {result:?}"),
    };

    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("number").unwrap()))
        .unwrap();
    let symbol_error = match context.call(&number, Value::Undefined, &[Value::Symbol(symbol)]) {
        Err(RuntimeError::Exception) => take_exception_object(&mut context),
        result => panic!("Number(Symbol) unexpectedly returned {result:?}"),
    };

    vec![
        format!("call={}", join_number_values(&call_values)),
        format!("bigint={}", join_number_values(&bigint_values)),
        format!(
            "object={}|{}|{}|{}|throw:{}",
            number_value_text(exotic_value),
            string_global(&runtime, &mut context, &global, "exoticLog"),
            number_value_text(fallback_value),
            string_global(&runtime, &mut context, &global, "fallbackLog"),
            value_text(thrown),
        ),
        format!(
            "symbol=throw:{}:{}",
            error_text(&runtime, &mut context, &symbol_error, "name"),
            error_text(&runtime, &mut context, &symbol_error, "message"),
        ),
    ]
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

fn constructor_prototype(
    runtime: &Runtime,
    context: &mut Context,
    constructor: &CallableRef,
) -> ObjectRef {
    let key = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(prototype) = context.get_property(constructor.as_object(), &key).unwrap()
    else {
        panic!("constructor prototype was not an object");
    };
    prototype
}

fn intrinsic_prototype(runtime: &Runtime, context: &mut Context, name: &str) -> ObjectRef {
    let constructor = global_callable(runtime, context, name);
    constructor_prototype(runtime, context, &constructor)
}

fn bound_constructor(runtime: &Runtime, context: &mut Context) -> CallableRef {
    let function = eval_callable(runtime, context, "(function Target() {}).bind(null)");
    assert!(
        runtime.is_constructor(function.as_object()).unwrap(),
        "bound target must remain constructible"
    );
    function
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

fn define_global(runtime: &Runtime, global: &ObjectRef, name: &str, value: Value) {
    define_data(runtime, global, name, value);
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

fn define_getter(runtime: &Runtime, object: &ObjectRef, name: &str, getter: CallableRef) {
    let key = runtime.intern_property_key(name).unwrap();
    assert!(
        runtime
            .define_own_property(
                object,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap(),
        "host accessor-property definition was rejected"
    );
}

fn string_global(
    runtime: &Runtime,
    context: &mut Context,
    global: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(global, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("global {name} was not a string");
    };
    value.to_utf8_lossy()
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

fn assert_number(value: Value, expected: f64) {
    let actual = value
        .as_number()
        .unwrap_or_else(|| panic!("expected Number, got {value:?}"));
    assert_eq!(actual.to_bits(), expected.to_bits());
}

fn join_number_values<const N: usize>(values: &[Value; N]) -> String {
    values
        .iter()
        .cloned()
        .map(number_value_text)
        .collect::<Vec<_>>()
        .join("|")
}

fn number_value_text(value: Value) -> String {
    let number = value
        .as_number()
        .unwrap_or_else(|| panic!("expected Number observation, got {value:?}"));
    if number.is_nan() {
        "NaN".to_owned()
    } else if number == 0.0 && number.is_sign_negative() {
        "-0".to_owned()
    } else {
        number_to_string(number)
    }
}

fn value_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => number_to_string(value),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Symbol(_) => "Symbol".to_owned(),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS Number constructor oracle");
    assert!(
        output.status.success(),
        "QuickJS Number constructor oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Number constructor oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
