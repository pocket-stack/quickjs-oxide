use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{
    AccessorValue, CallableRef, Context, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value, WellKnownSymbol,
};

const ORACLE_HELPERS: &str = r#"
function show(value) {
    if (value === undefined) return "undefined";
    if (typeof value === "number" || typeof value === "boolean") return String(value);
    if (typeof value === "bigint") return String(value) + "n";
    if (typeof value === "string") return "string:" + value;
    return "unexpected";
}
function observe(thunk) {
    try {
        return show(thunk());
    } catch (error) {
        if (typeof error === "string") return "throw-string:" + error;
        return "throw:" + error.name + "|" + error.message;
    }
}
function bit(value) { return value ? 1 : 0; }
"#;

const NUMERIC_PROBE: &str = r#"
let numericHint = "none";
const numericObject = {
    [Symbol.toPrimitive](hint) { numericHint = hint; return 6; }
};
const leftNumericSymbol = Symbol("left");
const rightNumericSymbol = Symbol("right");
console.log("unary=" + observe(() => +numericObject) + "," + observe(() => -numericObject));
console.log("binary=" + observe(() => numericObject - 2) + "," +
            observe(() => numericObject * 3) + "," +
            observe(() => numericObject / 2) + "," +
            observe(() => numericObject % 4));
let rightValueCalls = 0;
const rightValue = {
    [Symbol.toPrimitive]() { rightValueCalls++; return 1; }
};
console.log("symbol-left-right-value=" +
            observe(() => leftNumericSymbol - rightValue) + "," +
            observe(() => leftNumericSymbol * rightValue) + "," +
            observe(() => leftNumericSymbol / rightValue) + "," +
            observe(() => leftNumericSymbol % rightValue) +
            "|calls:" + rightValueCalls);
let rightThrowCalls = 0;
const rightThrow = {
    [Symbol.toPrimitive]() { rightThrowCalls++; throw "right"; }
};
console.log("symbol-left-right-throw=" +
            observe(() => leftNumericSymbol - rightThrow) + "," +
            observe(() => leftNumericSymbol * rightThrow) + "," +
            observe(() => leftNumericSymbol / rightThrow) + "," +
            observe(() => leftNumericSymbol % rightThrow) +
            "|calls:" + rightThrowCalls);
console.log("bigint-symbol-add=" + observe(() => 1n + rightNumericSymbol) + "," +
            observe(() => leftNumericSymbol + 1n));
console.log("bigint-symbol-sub=" + observe(() => 1n - rightNumericSymbol) + "," +
            observe(() => leftNumericSymbol - 1n));
console.log("relational=" + observe(() => numericObject < 7) + "," +
            observe(() => numericObject <= 6) + "," +
            observe(() => numericObject > 5) + "," +
            observe(() => numericObject >= 6));
console.log("numeric-hint=" + numericHint);
console.log("bigint-string-fraction=" + observe(() => 1n < "1.5") + "," +
            observe(() => "1.5" > 1n));
console.log("bigint-string-large=" +
            observe(() => 9007199254740992n < "9007199254740993") + "," +
            observe(() => "9007199254740993" > 9007199254740992n));
console.log("bigint-invalid-right=" + observe(() => 1n < "invalid") + "," +
            observe(() => 1n <= "invalid") + "," +
            observe(() => 1n > "invalid") + "," +
            observe(() => 1n >= "invalid"));
console.log("bigint-invalid-left=" + observe(() => "invalid" < 1n) + "," +
            observe(() => "invalid" <= 1n) + "," +
            observe(() => "invalid" > 1n) + "," +
            observe(() => "invalid" >= 1n));
"#;

const ADD_PROBE: &str = r#"
let addHint = "none";
const stringObject = {
    [Symbol.toPrimitive](hint) { addHint = hint; return "x"; }
};
console.log("add-string=" + observe(() => stringObject + 2) + "|hint:" + addHint);
const bigintObject = { [Symbol.toPrimitive]() { return 7n; } };
console.log("add-mixed=" + observe(() => bigintObject + 1));
console.log("function-plus=" + observe(() => +(function () {})));
"#;

const EQUALITY_PROBE: &str = r#"
const symbolValue = Symbol("s");
function box(value) {
    return { [Symbol.toPrimitive]() { return value; } };
}
function equality(name, object, primitive) {
    console.log("eq-" + name + "=" +
        bit(object == primitive) + "," + bit(primitive == object) + "," +
        bit(object != primitive) + "," + bit(primitive != object));
}
equality("number", box(7), 7);
equality("string", box("x"), "x");
equality("bigint", box(7n), 7n);
equality("symbol", box(symbolValue), symbolValue);
equality("boolean", box(1), true);
"#;

const ORDER_AND_ERROR_PROBE: &str = r#"
let order = 0;
const left = { [Symbol.toPrimitive]() { order = order * 10 + 1; return 1; } };
const right = { [Symbol.toPrimitive]() { order = order * 10 + 2; return 2; } };
console.log("order-add=" + observe(() => left + right) + "|order:" + order);
order = 0;
console.log("order-relational=" + observe(() => left < right) + "|order:" + order);

order = 0;
const ordinary = {
    valueOf() { order = order * 10 + 1; return ordinary; },
    toString() { order = order * 10 + 2; return "5"; },
};
console.log("ordinary-number=" + observe(() => +ordinary) + "|order:" + order);
order = 0;
console.log("ordinary-add=" + observe(() => ordinary + 1) + "|order:" + order);

const sentinel = {};
const getterThrow = {};
Object.defineProperty(getterThrow, Symbol.toPrimitive, {
    get() { throw sentinel; },
    configurable: true,
});
let getterSame = false;
try { +getterThrow; } catch (error) { getterSame = error === sentinel; }
console.log("getter-throw=" + (getterSame ? "same" : "changed"));

const methodThrow = { [Symbol.toPrimitive]() { throw sentinel; } };
let methodSame = false;
try { +methodThrow; } catch (error) { methodSame = error === sentinel; }
console.log("method-throw=" + (methodSame ? "same" : "changed"));

const objectReturn = { [Symbol.toPrimitive]() { return {}; } };
console.log("object-return=" + observe(() => +objectReturn));
const noncallable = { [Symbol.toPrimitive]: 1 };
console.log("noncallable=" + observe(() => +noncallable));

const stackObject = {
    [Symbol.toPrimitive]: function convert() { throw new Error("coerce"); }
};
function frameNames(stack) {
    return stack.trim().split("\n").slice(0, 3).map(line => {
        const match = /^\s*at ([^(]+?)(?: \(|$)/.exec(line);
        return match ? match[1].trim() : "?";
    }).join(",");
}
try {
    (function outer() { return +stackObject; })();
} catch (error) {
    console.log("stack=" + frameNames(error.stack));
}
"#;

#[test]
fn vm_object_coercion_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP VM object-coercion differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let rust = [
        rust_numeric_observations(),
        rust_add_observations(),
        rust_equality_observations(),
        rust_order_and_error_observations(),
    ]
    .concat();
    let oracle = [
        ("numeric object coercion", NUMERIC_PROBE),
        ("addition object coercion", ADD_PROBE),
        ("abstract equality object coercion", EQUALITY_PROBE),
        ("coercion order and errors", ORDER_AND_ERROR_PROBE),
    ]
    .into_iter()
    .flat_map(|(description, probe)| oracle_observations(&oracle, probe, description))
    .collect::<Vec<_>>();

    assert_eq!(rust, oracle);
}

struct Harness {
    runtime: Runtime,
    context: Context,
    to_primitive: PropertyKey,
}

impl Harness {
    fn new() -> Self {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let to_primitive =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
        Self {
            runtime,
            context,
            to_primitive,
        }
    }

    fn function(&mut self, source: &str) -> CallableRef {
        function(&self.runtime, &mut self.context, source)
    }

    fn object_with_exotic(&mut self, method: Value) -> ObjectRef {
        let object = self.context.new_object().unwrap();
        define_data(
            &self.runtime,
            &mut self.context,
            &object,
            &self.to_primitive,
            method,
        );
        object
    }

    fn bind(&mut self, name: &str, value: Value) {
        define_global(&self.runtime, &mut self.context, name, value);
    }

    fn observe(&mut self, source: &str) -> String {
        observe_eval(&self.runtime, &mut self.context, source)
    }
}

fn rust_numeric_observations() -> Vec<String> {
    let mut harness = Harness::new();
    harness.bind("numericHint", Value::String(JsString::from("none")));
    let method = harness.function("(function(hint){ numericHint = hint; return 6; })");
    let object = harness.object_with_exotic(Value::Object(method.as_object().clone()));
    harness.bind("numericObject", Value::Object(object));
    let left_symbol = harness
        .runtime
        .new_symbol(Some(JsString::from("left")))
        .unwrap();
    let right_symbol = harness
        .runtime
        .new_symbol(Some(JsString::from("right")))
        .unwrap();
    harness.bind("leftNumericSymbol", Value::Symbol(left_symbol));
    harness.bind("rightNumericSymbol", Value::Symbol(right_symbol));

    let unary_plus = harness.observe("+numericObject");
    let unary_neg = harness.observe("-numericObject");
    let sub = harness.observe("numericObject - 2");
    let mul = harness.observe("numericObject * 3");
    let div = harness.observe("numericObject / 2");
    let rem = harness.observe("numericObject % 4");

    harness.bind("rightValueCalls", Value::Int(0));
    let right_value_method =
        harness.function("(function(){ rightValueCalls = rightValueCalls + 1; return 1; })");
    let right_value =
        harness.object_with_exotic(Value::Object(right_value_method.as_object().clone()));
    harness.bind("rightValue", Value::Object(right_value));
    let symbol_left_right_value = ["-", "*", "/", "%"]
        .map(|operator| harness.observe(&format!("leftNumericSymbol {operator} rightValue")))
        .join(",");
    let right_value_calls =
        integer_global(&harness.runtime, &mut harness.context, "rightValueCalls");

    harness.bind("rightThrowCalls", Value::Int(0));
    let right_throw_method =
        harness.function("(function(){ rightThrowCalls = rightThrowCalls + 1; throw \"right\"; })");
    let right_throw =
        harness.object_with_exotic(Value::Object(right_throw_method.as_object().clone()));
    harness.bind("rightThrow", Value::Object(right_throw));
    let symbol_left_right_throw = ["-", "*", "/", "%"]
        .map(|operator| harness.observe(&format!("leftNumericSymbol {operator} rightThrow")))
        .join(",");
    let right_throw_calls =
        integer_global(&harness.runtime, &mut harness.context, "rightThrowCalls");

    let bigint_symbol_add = harness.observe("1n + rightNumericSymbol");
    let symbol_bigint_add = harness.observe("leftNumericSymbol + 1n");
    let bigint_symbol_sub = harness.observe("1n - rightNumericSymbol");
    let symbol_bigint_sub = harness.observe("leftNumericSymbol - 1n");
    let lt = harness.observe("numericObject < 7");
    let lte = harness.observe("numericObject <= 6");
    let gt = harness.observe("numericObject > 5");
    let gte = harness.observe("numericObject >= 6");
    let Value::String(hint) = global_value(&harness.runtime, &mut harness.context, "numericHint")
    else {
        panic!("numeric hint was not a string");
    };
    let bigint_fraction = harness.observe("1n < \"1.5\"");
    let fraction_bigint = harness.observe("\"1.5\" > 1n");
    let bigint_large = harness.observe("9007199254740992n < \"9007199254740993\"");
    let large_bigint = harness.observe("\"9007199254740993\" > 9007199254740992n");
    let invalid_right = ["<", "<=", ">", ">="]
        .map(|operator| harness.observe(&format!("1n {operator} \"invalid\"")))
        .join(",");
    let invalid_left = ["<", "<=", ">", ">="]
        .map(|operator| harness.observe(&format!("\"invalid\" {operator} 1n")))
        .join(",");

    vec![
        format!("unary={unary_plus},{unary_neg}"),
        format!("binary={sub},{mul},{div},{rem}"),
        format!("symbol-left-right-value={symbol_left_right_value}|calls:{right_value_calls}"),
        format!("symbol-left-right-throw={symbol_left_right_throw}|calls:{right_throw_calls}"),
        format!("bigint-symbol-add={bigint_symbol_add},{symbol_bigint_add}"),
        format!("bigint-symbol-sub={bigint_symbol_sub},{symbol_bigint_sub}"),
        format!("relational={lt},{lte},{gt},{gte}"),
        format!("numeric-hint={}", hint.to_utf8_lossy()),
        format!("bigint-string-fraction={bigint_fraction},{fraction_bigint}"),
        format!("bigint-string-large={bigint_large},{large_bigint}"),
        format!("bigint-invalid-right={invalid_right}"),
        format!("bigint-invalid-left={invalid_left}"),
    ]
}

fn rust_add_observations() -> Vec<String> {
    let mut harness = Harness::new();
    harness.bind("addHint", Value::String(JsString::from("none")));
    let string_method = harness.function("(function(hint){ addHint = hint; return \"x\"; })");
    let string_object =
        harness.object_with_exotic(Value::Object(string_method.as_object().clone()));
    harness.bind("stringObject", Value::Object(string_object));
    let add_string = harness.observe("stringObject + 2");
    let Value::String(hint) = global_value(&harness.runtime, &mut harness.context, "addHint")
    else {
        panic!("addition hint was not a string");
    };

    let bigint_method = harness.function("(function(){ return 7n; })");
    let bigint_object =
        harness.object_with_exotic(Value::Object(bigint_method.as_object().clone()));
    harness.bind("bigintObject", Value::Object(bigint_object));
    let mixed = harness.observe("bigintObject + 1");
    let function_plus = harness.observe("+(function(){})");

    vec![
        format!("add-string={add_string}|hint:{}", hint.to_utf8_lossy()),
        format!("add-mixed={mixed}"),
        format!("function-plus={function_plus}"),
    ]
}

fn rust_equality_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let symbol = harness
        .runtime
        .new_symbol(Some(JsString::from("s")))
        .unwrap();
    harness.bind("symbolValue", Value::Symbol(symbol));

    let cases = [
        ("number", "7", "(function(){ return 7; })"),
        ("string", "\"x\"", "(function(){ return \"x\"; })"),
        ("bigint", "7n", "(function(){ return 7n; })"),
        (
            "symbol",
            "symbolValue",
            "(function(){ return symbolValue; })",
        ),
        ("boolean", "true", "(function(){ return 1; })"),
    ];

    cases
        .into_iter()
        .map(|(name, primitive, method_source)| {
            let method = harness.function(method_source);
            let object = harness.object_with_exotic(Value::Object(method.as_object().clone()));
            let object_name = format!("equalityObject{name}");
            harness.bind(&object_name, Value::Object(object));
            let eq_left = bool_bit(&harness.observe(&format!("{object_name} == {primitive}")));
            let eq_right = bool_bit(&harness.observe(&format!("{primitive} == {object_name}")));
            let neq_left = bool_bit(&harness.observe(&format!("{object_name} != {primitive}")));
            let neq_right = bool_bit(&harness.observe(&format!("{primitive} != {object_name}")));
            format!("eq-{name}={eq_left},{eq_right},{neq_left},{neq_right}")
        })
        .collect()
}

fn rust_order_and_error_observations() -> Vec<String> {
    let mut harness = Harness::new();
    harness.bind("order", Value::Int(0));
    let left_method = harness.function("(function(){ order = order * 10 + 1; return 1; })");
    let right_method = harness.function("(function(){ order = order * 10 + 2; return 2; })");
    let left = harness.object_with_exotic(Value::Object(left_method.as_object().clone()));
    let right = harness.object_with_exotic(Value::Object(right_method.as_object().clone()));
    harness.bind("left", Value::Object(left));
    harness.bind("right", Value::Object(right));
    let add = harness.observe("left + right");
    let add_order = integer_global(&harness.runtime, &mut harness.context, "order");
    set_global(
        &harness.runtime,
        &mut harness.context,
        "order",
        Value::Int(0),
    );
    let relational = harness.observe("left < right");
    let relational_order = integer_global(&harness.runtime, &mut harness.context, "order");

    set_global(
        &harness.runtime,
        &mut harness.context,
        "order",
        Value::Int(0),
    );
    let ordinary = harness.context.new_object().unwrap();
    harness.bind("ordinary", Value::Object(ordinary.clone()));
    let value_of = harness.function("(function(){ order = order * 10 + 1; return ordinary; })");
    let to_string = harness.function("(function(){ order = order * 10 + 2; return \"5\"; })");
    let value_of_key = harness.runtime.intern_property_key("valueOf").unwrap();
    let to_string_key = harness.runtime.intern_property_key("toString").unwrap();
    define_data(
        &harness.runtime,
        &mut harness.context,
        &ordinary,
        &value_of_key,
        Value::Object(value_of.as_object().clone()),
    );
    define_data(
        &harness.runtime,
        &mut harness.context,
        &ordinary,
        &to_string_key,
        Value::Object(to_string.as_object().clone()),
    );
    let ordinary_number = harness.observe("+ordinary");
    let ordinary_number_order = integer_global(&harness.runtime, &mut harness.context, "order");
    set_global(
        &harness.runtime,
        &mut harness.context,
        "order",
        Value::Int(0),
    );
    let ordinary_add = harness.observe("ordinary + 1");
    let ordinary_add_order = integer_global(&harness.runtime, &mut harness.context, "order");

    let sentinel = harness.context.new_object().unwrap();
    harness.bind("sentinel", Value::Object(sentinel.clone()));
    let getter = harness.function("(function(){ throw sentinel; })");
    let getter_throw = harness.context.new_object().unwrap();
    assert!(
        harness
            .context
            .define_own_property(
                &getter_throw,
                &harness.to_primitive,
                &getter_descriptor(getter),
            )
            .unwrap()
    );
    harness.bind("getterThrow", Value::Object(getter_throw));
    let getter_same = eval_thrown_identity(
        &harness.runtime,
        &mut harness.context,
        "+getterThrow",
        &sentinel,
    );

    let method = harness.function("(function(){ throw sentinel; })");
    let method_throw = harness.object_with_exotic(Value::Object(method.as_object().clone()));
    harness.bind("methodThrow", Value::Object(method_throw));
    let method_same = eval_thrown_identity(
        &harness.runtime,
        &mut harness.context,
        "+methodThrow",
        &sentinel,
    );

    let returned_object = harness.context.new_object().unwrap();
    harness.bind("returnedObject", Value::Object(returned_object));
    let object_method = harness.function("(function(){ return returnedObject; })");
    let object_return =
        harness.object_with_exotic(Value::Object(object_method.as_object().clone()));
    harness.bind("objectReturn", Value::Object(object_return));
    let object_return = harness.observe("+objectReturn");

    let noncallable = harness.object_with_exotic(Value::Int(1));
    harness.bind("noncallable", Value::Object(noncallable));
    let noncallable = harness.observe("+noncallable");

    let convert = harness.function("(function convert(){ throw new Error(\"coerce\"); })");
    let stack_object = harness.object_with_exotic(Value::Object(convert.as_object().clone()));
    harness.bind("stackObject", Value::Object(stack_object));
    let stack = error_stack_frame_names(
        &harness.runtime,
        &mut harness.context,
        "(function outer(){ return +stackObject; })()",
    );

    vec![
        format!("order-add={add}|order:{add_order}"),
        format!("order-relational={relational}|order:{relational_order}"),
        format!("ordinary-number={ordinary_number}|order:{ordinary_number_order}"),
        format!("ordinary-add={ordinary_add}|order:{ordinary_add_order}"),
        format!(
            "getter-throw={}",
            if getter_same { "same" } else { "changed" }
        ),
        format!(
            "method-throw={}",
            if method_same { "same" } else { "changed" }
        ),
        format!("object-return={object_return}"),
        format!("noncallable={noncallable}"),
        format!("stack={stack}"),
    ]
}

fn function(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("function probe did not return an object: {source}");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("function probe was not callable: {source}"))
}

fn define_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    define_data(runtime, context, &global, &key, value);
}

fn set_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    assert!(context.set_property(&global, &key, value).unwrap());
}

fn global_value(runtime: &Runtime, context: &mut Context, name: &str) -> Value {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    context.get_property(&global, &key).unwrap()
}

fn integer_global(runtime: &Runtime, context: &mut Context, name: &str) -> i32 {
    let Value::Int(value) = global_value(runtime, context, name) else {
        panic!("global marker {name} was not an integer");
    };
    value
}

fn define_data(
    _runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
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
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn getter_descriptor(getter: CallableRef) -> OrdinaryPropertyDescriptor {
    OrdinaryPropertyDescriptor {
        get: DescriptorField::Present(AccessorValue::Callable(getter)),
        set: DescriptorField::Present(AccessorValue::Undefined),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    }
}

fn observe_eval(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    match context.eval(source) {
        Ok(value) => show_value(value),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap()
                .expect("exception completion had no value");
            show_exception(runtime, context, exception)
        }
        Err(error) => panic!("eval probe failed with an engine error for {source:?}: {error}"),
    }
}

fn show_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => number_to_string(value),
        Value::BigInt(value) => format!("{value}n"),
        Value::String(value) => format!("string:{}", value.to_utf8_lossy()),
        value => panic!("unexpected probe return value: {value:?}"),
    }
}

fn show_exception(runtime: &Runtime, context: &mut Context, exception: Value) -> String {
    match exception {
        Value::String(value) => format!("throw-string:{}", value.to_utf8_lossy()),
        Value::Object(error) if runtime.is_error_object(&error).unwrap() => {
            let name = runtime.intern_property_key("name").unwrap();
            let message = runtime.intern_property_key("message").unwrap();
            let Value::String(name) = context.get_property(&error, &name).unwrap() else {
                panic!("error name was not a string");
            };
            let Value::String(message) = context.get_property(&error, &message).unwrap() else {
                panic!("error message was not a string");
            };
            format!("throw:{}|{}", name.to_utf8_lossy(), message.to_utf8_lossy())
        }
        value => panic!("unexpected thrown probe value: {value:?}"),
    }
}

fn bool_bit(value: &str) -> u8 {
    match value {
        "true" => 1,
        "false" => 0,
        value => panic!("expected boolean observation, got {value:?}"),
    }
}

fn eval_thrown_identity(
    _runtime: &Runtime,
    context: &mut Context,
    source: &str,
    expected: &ObjectRef,
) -> bool {
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(object)) if object == *expected
    )
}

fn error_stack_frame_names(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("stack coercion did not throw an object");
    };
    assert!(runtime.is_error_object(&error).unwrap());
    let stack_key = runtime.intern_property_key("stack").unwrap();
    let Value::String(stack) = context.get_property(&error, &stack_key).unwrap() else {
        panic!("coercion Error.stack was not a string");
    };
    stack
        .to_utf8_lossy()
        .lines()
        .take(3)
        .map(|line| {
            let line = line.trim().strip_prefix("at ").unwrap_or(line.trim());
            line.split_once(" (")
                .map_or(line, |(name, _)| name)
                .trim()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn oracle_observations(oracle: &OsStr, probe: &str, description: &str) -> Vec<String> {
    let source = format!("{ORACLE_HELPERS}\n{probe}");
    let output = Command::new(oracle)
        .args(["-e", &source])
        .output()
        .unwrap_or_else(|error| panic!("run QuickJS {description} oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS {description} oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS {description} emitted non-UTF-8 output: {error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}
