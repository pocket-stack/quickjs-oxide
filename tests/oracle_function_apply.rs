use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    JsBigInt, JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError,
    Value, WellKnownSymbol,
};

const ORACLE_HELPERS: &str = r#"
function bit(value) { return value ? 1 : 0; }
function show(value) {
    if (value === undefined) return "undefined";
    if (typeof value === "boolean" || typeof value === "number") return String(value);
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
"#;

const BASIC_PROBE: &str = r#"
const fp = Function.prototype;
const keys = Reflect.ownKeys(fp).slice(0, 6).map(String).join(",");
console.log("keys=" + keys);
const applyDescriptor = Object.getOwnPropertyDescriptor(fp, "apply");
const apply = applyDescriptor.value;
const lengthDescriptor = Object.getOwnPropertyDescriptor(apply, "length");
const nameDescriptor = Object.getOwnPropertyDescriptor(apply, "name");
console.log("apply-desc=" + bit(applyDescriptor.writable) + "," +
            bit(applyDescriptor.enumerable) + "," + bit(applyDescriptor.configurable) +
            "|length:" + lengthDescriptor.value + "|" + bit(lengthDescriptor.writable) + "," +
            bit(lengthDescriptor.enumerable) + "," + bit(lengthDescriptor.configurable) +
            "|name:" + nameDescriptor.value + "|" + bit(nameDescriptor.writable) + "," +
            bit(nameDescriptor.enumerable) + "," + bit(nameDescriptor.configurable));

function zeroTarget(a) {
    "use strict";
    return typeof this === "undefined" && typeof a === "undefined";
}
function strictThis() { "use strict"; return this; }
function forwarded(a, b) { "use strict"; return this + a * 10 + b; }
console.log("zero=" + observe(() => Reflect.apply(apply, zeroTarget, [])));
console.log("null-array=" + observe(() => Reflect.apply(apply, strictThis, [17, null])));
console.log("undefined-array=" + observe(() => Reflect.apply(apply, strictThis, [18, undefined])));
console.log("forward=" + observe(() => Reflect.apply(apply, forwarded,
            [100, { 0: 2, 1: 3, length: 2 }])));
"#;

const EARLY_ERROR_PROBE: &str = r#"
const apply = Function.prototype.apply;
const poison = {};
Object.defineProperty(poison, "length", {
    get() { throw "length throw"; },
    configurable: true,
});
console.log("noncallable-first=" + observe(() => Reflect.apply(apply, {}, [undefined, poison])));
console.log("primitive-array=" + observe(() => Reflect.apply(apply, function () {}, [undefined, 1])));
console.log("length-throw=" + observe(() => Reflect.apply(apply, function () {}, [undefined, poison])));
"#;

const TO_PRIMITIVE_PROBE: &str = r#"
const apply = Function.prototype.apply;
function sum(a, b) { return (a == null ? 0 : a) + (b == null ? 0 : b); }
function arrayLike(length) { return { 0: 10, 1: 20, length }; }

let hintSeen = "none";
const numberHint = {
    [Symbol.toPrimitive](hint) { hintSeen = hint; return 2; }
};
console.log("primitive-hint=" + observe(() => Reflect.apply(apply, sum,
            [undefined, arrayLike(numberHint)])) + "|hint:" + hintSeen);

const objectResult = { [Symbol.toPrimitive]() { return {}; } };
console.log("primitive-object=" + observe(() => Reflect.apply(apply, sum,
            [undefined, arrayLike(objectResult)])));

const noncallable = { [Symbol.toPrimitive]: 1 };
console.log("primitive-noncallable=" + observe(() => Reflect.apply(apply, sum,
            [undefined, arrayLike(noncallable)])));

const throwing = { [Symbol.toPrimitive]() { throw "primitive throw"; } };
console.log("primitive-throw=" + observe(() => Reflect.apply(apply, sum,
            [undefined, arrayLike(throwing)])));
"#;

const ORDINARY_CONVERSION_PROBE: &str = r#"
const apply = Function.prototype.apply;
function sum(a, b) { return (a == null ? 0 : a) + (b == null ? 0 : b); }
let order = 0;
const lengthObject = {
    valueOf() { order = order * 10 + 1; return lengthObject; },
    toString() { order = order * 10 + 2; return "2"; },
};
const arrayLike = { 0: 10, 1: 20, length: lengthObject };
console.log("ordinary-order=" + observe(() => Reflect.apply(apply, sum,
            [undefined, arrayLike])) + "|order:" + order);
"#;

const TO_LENGTH_PROBE: &str = r#"
const apply = Function.prototype.apply;
function sum(a, b, c) {
    return (a == null ? 0 : a) + (b == null ? 0 : b) + (c == null ? 0 : c);
}
function run(length) {
    return observe(() => Reflect.apply(apply, sum,
        [undefined, { 0: 10, 1: 20, 2: 30, length }]));
}
console.log("length-nan=" + run(NaN));
console.log("length-negative=" + run(-2));
console.log("length-fraction=" + run(2.9));
console.log("length-infinity=" + run(Infinity));
console.log("length-string=" + run("2.9"));
console.log("length-bigint=" + run(1n));
"#;

const INDEX_PROBE: &str = r#"
const apply = Function.prototype.apply;
let order = 0;
const ordered = { length: 3 };
for (const [index, digit] of [[0, 1], [1, 2], [2, 3]]) {
    Object.defineProperty(ordered, index, {
        get() { order = order * 10 + digit; return order; },
        configurable: true,
    });
}
function encode(a, b, c) { return a * 10000 + b * 100 + c; }
console.log("index-order=" + observe(() => Reflect.apply(apply, encode,
            [undefined, ordered])) + "|order:" + order);

function holeTarget(a, b, c) {
    return a * 100 + (typeof b === "undefined" ? 10 : 0) + c;
}
console.log("index-hole=" + observe(() => Reflect.apply(apply, holeTarget,
            [undefined, { 0: 1, 2: 3, length: 3 }])));

const inherited = Object.create({ 1: 2 });
inherited[0] = 1;
inherited.length = 2;
console.log("index-inherited=" + observe(() => Reflect.apply(apply,
            function (a, b) { return a * 10 + b; }, [undefined, inherited])));

const accessor = { length: 1 };
Object.defineProperty(accessor, 0, { get() { return 7; }, configurable: true });
console.log("index-accessor=" + observe(() => Reflect.apply(apply,
            function (a) { return a; }, [undefined, accessor])));

const throwing = { length: 2 };
Object.defineProperty(throwing, 0, { get() { throw "index throw"; }, configurable: true });
Object.defineProperty(throwing, 1, { get() { throw "late index read"; }, configurable: true });
console.log("index-throw=" + observe(() => Reflect.apply(apply,
            function () {}, [undefined, throwing])));
"#;

const BOUNDARY_PROBE: &str = r#"
const apply = Function.prototype.apply;
function target() { return 1; }
console.log("boundary-65534=" + observe(() => Reflect.apply(apply, target,
            [undefined, { length: 65534 }])));
console.log("boundary-65535=" + observe(() => Reflect.apply(apply, target,
            [undefined, { length: 65535 }])));
"#;

#[test]
fn function_prototype_apply_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Function.prototype.apply differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let rust = [
        rust_basic_observations(),
        rust_early_error_observations(),
        rust_to_primitive_observations(),
        rust_ordinary_conversion_observations(),
        rust_to_length_observations(),
        rust_index_observations(),
        rust_boundary_observations(),
    ]
    .concat();
    let oracle = [
        ("basic apply", BASIC_PROBE),
        ("early apply errors", EARLY_ERROR_PROBE),
        ("apply ToPrimitive", TO_PRIMITIVE_PROBE),
        ("apply ordinary conversion", ORDINARY_CONVERSION_PROBE),
        ("apply ToLength", TO_LENGTH_PROBE),
        ("apply indexed Get", INDEX_PROBE),
        ("apply argument boundary", BOUNDARY_PROBE),
    ]
    .into_iter()
    .flat_map(|(description, probe)| oracle_observations(&oracle, probe, description))
    .collect::<Vec<_>>();

    assert_eq!(rust, oracle);
}

struct Harness {
    runtime: Runtime,
    context: Context,
    function_prototype: ObjectRef,
    apply: CallableRef,
}

impl Harness {
    fn new() -> Self {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let apply_key = runtime.intern_property_key("apply").unwrap();
        let Value::Object(apply_object) = context
            .get_property(&function_prototype, &apply_key)
            .unwrap()
        else {
            panic!("Function.prototype.apply was not an object");
        };
        let apply = runtime
            .as_callable(&apply_object)
            .unwrap()
            .expect("Function.prototype.apply was not callable");
        Self {
            runtime,
            context,
            function_prototype,
            apply,
        }
    }

    fn function(&mut self, source: &str) -> CallableRef {
        function(&self.runtime, &mut self.context, source)
    }

    fn apply(&mut self, target: &CallableRef, arguments: &[Value]) -> String {
        observe_call(
            &self.runtime,
            &mut self.context,
            &self.apply,
            Value::Object(target.as_object().clone()),
            arguments,
        )
    }
}

fn rust_basic_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let runtime = harness.runtime.clone();
    let prefix = runtime
        .own_property_keys(&harness.function_prototype)
        .unwrap()
        .into_iter()
        .take(6)
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect::<Vec<_>>()
        .join(",");
    let apply_key = runtime.intern_property_key("apply").unwrap();
    let (Value::Object(apply_object), apply_w, apply_e, apply_c) =
        data_descriptor(&runtime, &harness.function_prototype, &apply_key)
    else {
        panic!("Function.prototype.apply was not a data function");
    };
    let length_key = runtime.intern_property_key("length").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let (Value::Int(length), length_w, length_e, length_c) =
        data_descriptor(&runtime, &apply_object, &length_key)
    else {
        panic!("apply.length was not an integer");
    };
    let (Value::String(name), name_w, name_e, name_c) =
        data_descriptor(&runtime, &apply_object, &name_key)
    else {
        panic!("apply.name was not a string");
    };

    let zero_target = harness.function(
        "(function zeroTarget(a){ \"use strict\"; return (typeof this === \"undefined\") && (typeof a === \"undefined\"); })",
    );
    let strict_this = harness.function("(function strictThis(){ \"use strict\"; return this; })");
    let forwarded =
        harness.function("(function forwarded(a, b){ \"use strict\"; return this + a * 10 + b; })");
    let array = array_like(&runtime, &mut harness.context, Value::Int(2));
    define_index(&runtime, &mut harness.context, &array, 0, Value::Int(2));
    define_index(&runtime, &mut harness.context, &array, 1, Value::Int(3));

    let zero = harness.apply(&zero_target, &[]);
    let null_array = harness.apply(&strict_this, &[Value::Int(17), Value::Null]);
    let undefined_array = harness.apply(&strict_this, &[Value::Int(18), Value::Undefined]);
    let forward = harness.apply(&forwarded, &[Value::Int(100), Value::Object(array)]);

    vec![
        format!("keys={prefix}"),
        format!(
            "apply-desc={},{},{}|length:{length}|{},{},{}|name:{}|{},{},{}",
            bit(apply_w),
            bit(apply_e),
            bit(apply_c),
            bit(length_w),
            bit(length_e),
            bit(length_c),
            name.to_utf8_lossy(),
            bit(name_w),
            bit(name_e),
            bit(name_c),
        ),
        format!("zero={zero}"),
        format!("null-array={null_array}"),
        format!("undefined-array={undefined_array}"),
        format!("forward={forward}"),
    ]
}

fn rust_early_error_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let poison_getter = harness.function("(function(){ throw \"length throw\"; })");
    let poison = harness.context.new_object().unwrap();
    let length_key = harness.runtime.intern_property_key("length").unwrap();
    assert!(
        harness
            .context
            .define_own_property(&poison, &length_key, &getter_descriptor(poison_getter))
            .unwrap()
    );
    let noncallable = harness.context.new_object().unwrap();
    let noncallable_first = observe_call(
        &harness.runtime,
        &mut harness.context,
        &harness.apply,
        Value::Object(noncallable),
        &[Value::Undefined, Value::Object(poison.clone())],
    );
    let target = harness.function("(function(){})");
    let primitive_array = harness.apply(&target, &[Value::Undefined, Value::Int(1)]);
    let length_throw = harness.apply(&target, &[Value::Undefined, Value::Object(poison)]);
    vec![
        format!("noncallable-first={noncallable_first}"),
        format!("primitive-array={primitive_array}"),
        format!("length-throw={length_throw}"),
    ]
}

fn rust_to_primitive_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let sum = harness
        .function("(function sum(a, b){ return (a == null ? 0 : a) + (b == null ? 0 : b); })");
    let to_primitive = PropertyKey::from(
        harness
            .runtime
            .well_known_symbol(WellKnownSymbol::ToPrimitive),
    );

    define_global(
        &harness.runtime,
        &mut harness.context,
        "hintSeen",
        Value::String(JsString::try_from_utf8("none").unwrap()),
    );
    let hint = harness.function("(function(hint){ hintSeen = hint; return 2; })");
    let hint_length = harness.context.new_object().unwrap();
    define_data(
        &harness.runtime,
        &mut harness.context,
        &hint_length,
        &to_primitive,
        Value::Object(hint.as_object().clone()),
    );
    let hint_array = populated_two(
        &harness.runtime,
        &mut harness.context,
        Value::Object(hint_length),
    );
    let hint_result = harness.apply(&sum, &[Value::Undefined, Value::Object(hint_array)]);
    let Value::String(hint_seen) = global_value(&harness.runtime, &mut harness.context, "hintSeen")
    else {
        panic!("number hint marker was not a string");
    };
    let hint_seen = hint_seen.to_utf8_lossy();

    let primitive_result = harness.context.new_object().unwrap();
    define_global(
        &harness.runtime,
        &mut harness.context,
        "primitiveResult",
        Value::Object(primitive_result),
    );
    let object_return = harness.function("(function(){ return primitiveResult; })");
    let object_length = harness.context.new_object().unwrap();
    define_data(
        &harness.runtime,
        &mut harness.context,
        &object_length,
        &to_primitive,
        Value::Object(object_return.as_object().clone()),
    );
    let object_array = populated_two(
        &harness.runtime,
        &mut harness.context,
        Value::Object(object_length),
    );
    let object_result = harness.apply(&sum, &[Value::Undefined, Value::Object(object_array)]);

    let noncallable_length = harness.context.new_object().unwrap();
    define_data(
        &harness.runtime,
        &mut harness.context,
        &noncallable_length,
        &to_primitive,
        Value::Int(1),
    );
    let noncallable_array = populated_two(
        &harness.runtime,
        &mut harness.context,
        Value::Object(noncallable_length),
    );
    let noncallable_result =
        harness.apply(&sum, &[Value::Undefined, Value::Object(noncallable_array)]);

    let throwing = harness.function("(function(){ throw \"primitive throw\"; })");
    let throwing_length = harness.context.new_object().unwrap();
    define_data(
        &harness.runtime,
        &mut harness.context,
        &throwing_length,
        &to_primitive,
        Value::Object(throwing.as_object().clone()),
    );
    let throwing_array = populated_two(
        &harness.runtime,
        &mut harness.context,
        Value::Object(throwing_length),
    );
    let throwing_result = harness.apply(&sum, &[Value::Undefined, Value::Object(throwing_array)]);

    vec![
        format!("primitive-hint={hint_result}|hint:{hint_seen}"),
        format!("primitive-object={object_result}"),
        format!("primitive-noncallable={noncallable_result}"),
        format!("primitive-throw={throwing_result}"),
    ]
}

fn rust_ordinary_conversion_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let sum = harness
        .function("(function sum(a, b){ return (a == null ? 0 : a) + (b == null ? 0 : b); })");
    define_global(
        &harness.runtime,
        &mut harness.context,
        "order",
        Value::Int(0),
    );
    let length_object = harness.context.new_object().unwrap();
    define_global(
        &harness.runtime,
        &mut harness.context,
        "lengthObject",
        Value::Object(length_object.clone()),
    );
    let value_of = harness.function("(function(){ order = order * 10 + 1; return lengthObject; })");
    let to_string = harness.function("(function(){ order = order * 10 + 2; return \"2\"; })");
    let value_of_key = harness.runtime.intern_property_key("valueOf").unwrap();
    let to_string_key = harness.runtime.intern_property_key("toString").unwrap();
    define_data(
        &harness.runtime,
        &mut harness.context,
        &length_object,
        &value_of_key,
        Value::Object(value_of.as_object().clone()),
    );
    define_data(
        &harness.runtime,
        &mut harness.context,
        &length_object,
        &to_string_key,
        Value::Object(to_string.as_object().clone()),
    );
    let array = populated_two(
        &harness.runtime,
        &mut harness.context,
        Value::Object(length_object),
    );
    let result = harness.apply(&sum, &[Value::Undefined, Value::Object(array)]);
    let Value::Int(order) = global_value(&harness.runtime, &mut harness.context, "order") else {
        panic!("ordinary conversion order was not an integer");
    };
    vec![format!("ordinary-order={result}|order:{order}")]
}

fn rust_to_length_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let sum = harness.function(
        "(function sum(a, b, c){ return (a == null ? 0 : a) + (b == null ? 0 : b) + (c == null ? 0 : c); })",
    );
    let cases = [
        ("length-nan", Value::Float(f64::NAN)),
        ("length-negative", Value::Int(-2)),
        ("length-fraction", Value::Float(2.9)),
        ("length-infinity", Value::Float(f64::INFINITY)),
        (
            "length-string",
            Value::String(JsString::try_from_utf8("2.9").unwrap()),
        ),
        ("length-bigint", Value::BigInt(JsBigInt::one())),
    ];
    cases
        .into_iter()
        .map(|(name, length)| {
            let array = populated_three(&harness.runtime, &mut harness.context, length);
            let result = harness.apply(&sum, &[Value::Undefined, Value::Object(array)]);
            format!("{name}={result}")
        })
        .collect()
}

fn rust_index_observations() -> Vec<String> {
    let mut harness = Harness::new();
    define_global(
        &harness.runtime,
        &mut harness.context,
        "order",
        Value::Int(0),
    );
    let ordered = array_like(&harness.runtime, &mut harness.context, Value::Int(3));
    for (index, digit) in [(0, 1), (1, 2), (2, 3)] {
        let getter = harness.function(&format!(
            "(function(){{ order = order * 10 + {digit}; return order; }})"
        ));
        define_index_getter(
            &harness.runtime,
            &mut harness.context,
            &ordered,
            index,
            getter,
        );
    }
    let encode = harness.function("(function encode(a, b, c){ return a * 10000 + b * 100 + c; })");
    let order_result = harness.apply(&encode, &[Value::Undefined, Value::Object(ordered)]);
    let Value::Int(order) = global_value(&harness.runtime, &mut harness.context, "order") else {
        panic!("index access order was not an integer");
    };

    let hole = array_like(&harness.runtime, &mut harness.context, Value::Int(3));
    define_index(
        &harness.runtime,
        &mut harness.context,
        &hole,
        0,
        Value::Int(1),
    );
    define_index(
        &harness.runtime,
        &mut harness.context,
        &hole,
        2,
        Value::Int(3),
    );
    let hole_target = harness.function(
        "(function holeTarget(a, b, c){ return a * 100 + (typeof b === \"undefined\" ? 10 : 0) + c; })",
    );
    let hole_result = harness.apply(&hole_target, &[Value::Undefined, Value::Object(hole)]);

    let inherited_prototype = harness.context.new_object().unwrap();
    let inherited_one = harness.runtime.intern_property_key("1").unwrap();
    define_data(
        &harness.runtime,
        &mut harness.context,
        &inherited_prototype,
        &inherited_one,
        Value::Int(2),
    );
    let inherited = harness
        .context
        .new_object_with_prototype(Some(&inherited_prototype))
        .unwrap();
    let length = harness.runtime.intern_property_key("length").unwrap();
    define_data(
        &harness.runtime,
        &mut harness.context,
        &inherited,
        &length,
        Value::Int(2),
    );
    define_index(
        &harness.runtime,
        &mut harness.context,
        &inherited,
        0,
        Value::Int(1),
    );
    let inherited_target = harness.function("(function(a, b){ return a * 10 + b; })");
    let inherited_result = harness.apply(
        &inherited_target,
        &[Value::Undefined, Value::Object(inherited)],
    );

    let accessor = array_like(&harness.runtime, &mut harness.context, Value::Int(1));
    let accessor_getter = harness.function("(function(){ return 7; })");
    define_index_getter(
        &harness.runtime,
        &mut harness.context,
        &accessor,
        0,
        accessor_getter,
    );
    let identity = harness.function("(function(a){ return a; })");
    let accessor_result = harness.apply(&identity, &[Value::Undefined, Value::Object(accessor)]);

    let throwing = array_like(&harness.runtime, &mut harness.context, Value::Int(2));
    let early_throw = harness.function("(function(){ throw \"index throw\"; })");
    let late_throw = harness.function("(function(){ throw \"late index read\"; })");
    define_index_getter(
        &harness.runtime,
        &mut harness.context,
        &throwing,
        0,
        early_throw,
    );
    define_index_getter(
        &harness.runtime,
        &mut harness.context,
        &throwing,
        1,
        late_throw,
    );
    let no_op = harness.function("(function(){})");
    let throw_result = harness.apply(&no_op, &[Value::Undefined, Value::Object(throwing)]);

    vec![
        format!("index-order={order_result}|order:{order}"),
        format!("index-hole={hole_result}"),
        format!("index-inherited={inherited_result}"),
        format!("index-accessor={accessor_result}"),
        format!("index-throw={throw_result}"),
    ]
}

fn rust_boundary_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let target = harness.function("(function target(){ return 1; })");
    let accepted = array_like(&harness.runtime, &mut harness.context, Value::Int(65_534));
    let rejected = array_like(&harness.runtime, &mut harness.context, Value::Int(65_535));
    let accepted = harness.apply(&target, &[Value::Undefined, Value::Object(accepted)]);
    let rejected = harness.apply(&target, &[Value::Undefined, Value::Object(rejected)]);
    vec![
        format!("boundary-65534={accepted}"),
        format!("boundary-65535={rejected}"),
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

fn array_like(runtime: &Runtime, context: &mut Context, length: Value) -> ObjectRef {
    let object = context.new_object().unwrap();
    let length_key = runtime.intern_property_key("length").unwrap();
    define_data(runtime, context, &object, &length_key, length);
    object
}

fn populated_two(runtime: &Runtime, context: &mut Context, length: Value) -> ObjectRef {
    let object = array_like(runtime, context, length);
    define_index(runtime, context, &object, 0, Value::Int(10));
    define_index(runtime, context, &object, 1, Value::Int(20));
    object
}

fn populated_three(runtime: &Runtime, context: &mut Context, length: Value) -> ObjectRef {
    let object = populated_two(runtime, context, length);
    define_index(runtime, context, &object, 2, Value::Int(30));
    object
}

fn define_index(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    index: usize,
    value: Value,
) {
    let key = runtime.intern_property_key(&index.to_string()).unwrap();
    define_data(runtime, context, object, &key, value);
}

fn define_index_getter(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    index: usize,
    getter: CallableRef,
) {
    let key = runtime.intern_property_key(&index.to_string()).unwrap();
    assert!(
        context
            .define_own_property(object, &key, &getter_descriptor(getter))
            .unwrap()
    );
}

fn define_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    define_data(runtime, context, &global, &key, value);
}

fn global_value(runtime: &Runtime, context: &mut Context, name: &str) -> Value {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    context.get_property(&global, &key).unwrap()
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

fn data_descriptor(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
) -> (Value, bool, bool, bool) {
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value,
        writable,
        enumerable,
        configurable,
    }) = runtime.get_own_property(object, key).unwrap()
    else {
        panic!("expected a complete data descriptor");
    };
    (value, writable, enumerable, configurable)
}

fn observe_call(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    this_value: Value,
    arguments: &[Value],
) -> String {
    match context.call(callable, this_value, arguments) {
        Ok(value) => show_value(value),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap()
                .expect("exception completion had no value");
            show_exception(runtime, context, exception)
        }
        Err(error) => panic!("call probe failed with an engine error: {error}"),
    }
}

fn show_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
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

const fn bit(value: bool) -> u8 {
    value as u8
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
