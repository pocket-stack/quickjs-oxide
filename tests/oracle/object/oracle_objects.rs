use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CompleteOrdinaryPropertyDescriptor, Context, DescriptorField, JsString,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, Value, WellKnownSymbol,
};

const ORACLE_PROBE: &str = r#"
function units(s) {
    var out = [];
    for (var i = 0; i < s.length; i++)
        out.push(("0000" + s.charCodeAt(i).toString(16)).slice(-4));
    return out.join(",");
}
var symbolA = Symbol("a"), symbolB = Symbol("b");
function token(key) {
    if (key === symbolA) return "@a";
    if (key === symbolB) return "@b";
    return key;
}
var ordered = {};
ordered.beta = 1;
ordered["4294967295"] = 2;
ordered["2147483648"] = 3;
ordered["01"] = 4;
ordered["4294967294"] = 5;
ordered["0"] = 6;
ordered["-0"] = 7;
ordered[symbolA] = 8;
ordered["2"] = 9;
ordered[symbolB] = 10;
print("keys=" + Reflect.ownKeys(ordered).map(token).join("|"));

var moved = { a: 1, b: 2, c: 3 };
delete moved.a;
moved.a = 4;
print("delete=" + Reflect.ownKeys(moved).join("|"));

var defaults = {};
Object.defineProperty(defaults, "x", { value: 7 });
var d = Object.getOwnPropertyDescriptor(defaults, "x");
print("defaults=" + d.value + "," + Number(d.writable) + "," +
      Number(d.enumerable) + "," + Number(d.configurable));

var frozen = {};
Object.defineProperty(frozen, "nan", { value: NaN, enumerable: true });
Object.defineProperty(frozen, "zero", { value: 0, enumerable: true });
print("frozen=" + Reflect.defineProperty(frozen, "nan", { value: NaN }) + "," +
      Reflect.defineProperty(frozen, "zero", { value: -0 }));

var parent = {};
Object.defineProperty(parent, "w", { value: 1, writable: true, configurable: true });
Object.defineProperty(parent, "r", { value: 1, writable: false, configurable: true });
var child = Object.create(parent);
print("inherited=" + Reflect.set(child, "w", 2) + "," +
      Reflect.set(child, "r", 2) + "," + child.w + "," + child.r);

var receiverTarget = {};
Object.defineProperty(receiverTarget, "x", {
    value: 1, writable: true, configurable: true
});
var receiver = {};
print("receiver=" + Reflect.set(receiverTarget, "x", 2, receiver) + "," +
      receiverTarget.x + "," + receiver.x);

var fixed = Object.create(null);
Object.preventExtensions(fixed);
var first = {}, second = {};
print("proto=" + Reflect.setPrototypeOf(fixed, null) + "," +
      Reflect.setPrototypeOf(fixed, {}) + "," +
      Reflect.setPrototypeOf(first, second) + "," +
      Reflect.setPrototypeOf(second, first));

var surrogate = {};
surrogate["\uD800"] = 1;
surrogate["\uFFFD"] = 2;
surrogate["\uD801"] = 3;
print("surrogates=" + Reflect.ownKeys(surrogate).map(units).join("|"));

var registry = Symbol.for("Symbol.iterator");
print("symbols=" + String(Symbol.keyFor(Symbol.iterator)) + "," +
      Symbol.keyFor(registry) + "," + (Symbol.iterator === registry));
"#;

#[test]
fn ordinary_object_core_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(rust_observations(), oracle_observations(&oracle));
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut output = Vec::new();

    let ordered = runtime.new_object(None).unwrap();
    let symbol_a = runtime
        .new_symbol(Some(JsString::try_from_utf8("a").unwrap()))
        .unwrap();
    let symbol_b = runtime
        .new_symbol(Some(JsString::try_from_utf8("b").unwrap()))
        .unwrap();
    let symbol_key_a = PropertyKey::from(&symbol_a);
    let symbol_key_b = PropertyKey::from(&symbol_b);
    for (key, value) in [
        (runtime.intern_property_key("beta").unwrap(), 1),
        (runtime.intern_property_key("4294967295").unwrap(), 2),
        (runtime.intern_property_key("2147483648").unwrap(), 3),
        (runtime.intern_property_key("01").unwrap(), 4),
        (runtime.intern_property_key("4294967294").unwrap(), 5),
        (runtime.intern_property_key("0").unwrap(), 6),
        (runtime.intern_property_key("-0").unwrap(), 7),
    ] {
        assert!(
            context
                .set_property(&ordered, &key, Value::Int(value))
                .unwrap()
        );
    }
    assert!(
        context
            .set_property(&ordered, &symbol_key_a, Value::Int(8))
            .unwrap()
    );
    set(&mut context, &runtime, &ordered, "2", Value::Int(9));
    assert!(
        context
            .set_property(&ordered, &symbol_key_b, Value::Int(10))
            .unwrap()
    );
    let keys = runtime
        .own_property_keys(&ordered)
        .unwrap()
        .into_iter()
        .map(|key| {
            if key == symbol_key_a {
                "@a".to_owned()
            } else if key == symbol_key_b {
                "@b".to_owned()
            } else {
                runtime
                    .property_key_to_js_string(&key)
                    .unwrap()
                    .to_utf8_lossy()
            }
        })
        .collect::<Vec<_>>()
        .join("|");
    output.push(format!("keys={keys}"));

    let moved = runtime.new_object(None).unwrap();
    for name in ["a", "b", "c"] {
        set(&mut context, &runtime, &moved, name, Value::Int(1));
    }
    let a = runtime.intern_property_key("a").unwrap();
    assert!(runtime.delete_property(&moved, &a).unwrap());
    assert!(context.set_property(&moved, &a, Value::Int(4)).unwrap());
    output.push(format!(
        "delete={}",
        string_keys(&runtime, &moved).join("|")
    ));

    let defaults = runtime.new_object(None).unwrap();
    let x = runtime.intern_property_key("x").unwrap();
    assert!(
        runtime
            .define_own_property(
                &defaults,
                &x,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(7)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Int(value),
        writable,
        enumerable,
        configurable,
    } = runtime.get_own_property(&defaults, &x).unwrap().unwrap()
    else {
        panic!("unexpected default property descriptor");
    };
    output.push(format!(
        "defaults={value},{},{},{}",
        Number(writable),
        Number(enumerable),
        Number(configurable)
    ));

    let frozen = runtime.new_object(None).unwrap();
    let nan = runtime.intern_property_key("nan").unwrap();
    let zero = runtime.intern_property_key("zero").unwrap();
    define_data(
        &runtime,
        &frozen,
        &nan,
        Value::Float(f64::NAN),
        false,
        true,
        false,
    );
    define_data(&runtime, &frozen, &zero, Value::Int(0), false, true, false);
    let same_nan = redefine_value(&runtime, &frozen, &nan, Value::Float(f64::NAN));
    let minus_zero = redefine_value(&runtime, &frozen, &zero, Value::Float(-0.0));
    output.push(format!("frozen={same_nan},{minus_zero}"));

    let parent = runtime.new_object(None).unwrap();
    let w = runtime.intern_property_key("w").unwrap();
    let r = runtime.intern_property_key("r").unwrap();
    define_data(&runtime, &parent, &w, Value::Int(1), true, false, true);
    define_data(&runtime, &parent, &r, Value::Int(1), false, false, true);
    let child = runtime.new_object(Some(&parent)).unwrap();
    let write = context.set_property(&child, &w, Value::Int(2)).unwrap();
    let read_only = context.set_property(&child, &r, Value::Int(2)).unwrap();
    output.push(format!(
        "inherited={write},{read_only},{},{}",
        int_value(context.get_property(&child, &w).unwrap()),
        int_value(context.get_property(&child, &r).unwrap())
    ));

    let receiver_target = runtime.new_object(None).unwrap();
    let receiver = runtime.new_object(None).unwrap();
    let receiver_x = runtime.intern_property_key("x").unwrap();
    define_data(
        &runtime,
        &receiver_target,
        &receiver_x,
        Value::Int(1),
        true,
        false,
        true,
    );
    let receiver_result = context
        .set_property_with_receiver(
            &receiver_target,
            &receiver_x,
            Value::Int(2),
            Value::Object(receiver.clone()),
        )
        .unwrap();
    output.push(format!(
        "receiver={receiver_result},{},{}",
        int_value(context.get_property(&receiver_target, &receiver_x).unwrap()),
        int_value(context.get_property(&receiver, &receiver_x).unwrap())
    ));

    let fixed = runtime.new_object(None).unwrap();
    runtime.prevent_extensions(&fixed).unwrap();
    let first = runtime.new_object(None).unwrap();
    let second = runtime.new_object(None).unwrap();
    output.push(format!(
        "proto={},{},{},{}",
        runtime.set_prototype_of(&fixed, None).unwrap(),
        runtime.set_prototype_of(&fixed, Some(&parent)).unwrap(),
        runtime.set_prototype_of(&first, Some(&second)).unwrap(),
        runtime.set_prototype_of(&second, Some(&first)).unwrap()
    ));

    let surrogate = runtime.new_object(None).unwrap();
    for (unit, value) in [(0xd800, 1), (0xfffd, 2), (0xd801, 3)] {
        let key = runtime
            .intern_property_key_js_string(&JsString::try_from_utf16([unit]).unwrap())
            .unwrap();
        assert!(
            context
                .set_property(&surrogate, &key, Value::Int(value))
                .unwrap()
        );
    }
    output.push(format!(
        "surrogates={}",
        runtime
            .own_property_keys(&surrogate)
            .unwrap()
            .iter()
            .map(|key| units(&runtime.property_key_to_js_string(key).unwrap()))
            .collect::<Vec<_>>()
            .join("|")
    ));

    let name = JsString::try_from_utf8("Symbol.iterator").unwrap();
    let well_known = runtime.well_known_symbol(WellKnownSymbol::Iterator);
    let registry = runtime.symbol_for(&name).unwrap();
    output.push(format!(
        "symbols={},{},{}",
        runtime
            .symbol_key_for(&well_known)
            .unwrap()
            .map_or_else(|| "undefined".to_owned(), |value| value.to_utf8_lossy()),
        runtime
            .symbol_key_for(&registry)
            .unwrap()
            .unwrap()
            .to_utf8_lossy(),
        well_known == registry
    ));
    output
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .unwrap_or_else(|error| panic!("could not run object oracle: {error}"));
    assert!(
        output.status.success(),
        "object oracle failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("object oracle emitted UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn set(
    context: &mut Context,
    runtime: &Runtime,
    object: &quickjs_oxide::ObjectRef,
    name: &str,
    value: Value,
) {
    let key = runtime.intern_property_key(name).unwrap();
    assert!(context.set_property(object, &key, value).unwrap());
}

fn define_data(
    runtime: &Runtime,
    object: &quickjs_oxide::ObjectRef,
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
            .unwrap()
    );
}

fn redefine_value(
    runtime: &Runtime,
    object: &quickjs_oxide::ObjectRef,
    key: &PropertyKey,
    value: Value,
) -> bool {
    runtime
        .define_own_property(
            object,
            key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
        .unwrap()
}

fn string_keys(runtime: &Runtime, object: &quickjs_oxide::ObjectRef) -> Vec<String> {
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
        .collect()
}

fn units(value: &JsString) -> String {
    value
        .utf16_units()
        .map(|unit| format!("{unit:04x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn int_value(value: Value) -> i32 {
    let Value::Int(value) = value else {
        panic!("expected integer property value");
    };
    value
}

struct Number(bool);

impl std::fmt::Display for Number {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0 { "1" } else { "0" })
    }
}
