use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, Value,
};

// This oracle is deliberately narrower than the future `%String%` intrinsic.
// It observes only the String-wrapper exotic substrate without relying on the
// wider constructor and prototype method table. Both engines obtain a genuine
// wrapper through sloppy-function `this` boxing.
const ORACLE_PROBE: &str = r#"
function flags(descriptor) {
    return (descriptor.writable ? "1" : "0") +
           (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0");
}
function units(value) {
    var result = [];
    for (var index = 0; index < value.length; index++)
        result.push(value.charCodeAt(index).toString(16).padStart(4, "0"));
    return result.join(",");
}
function keyName(key) { return typeof key === "symbol" ? String(key) : key; }
function box(value) { return (function () { return this; }).call(value); }
function dataProperty(object, key, value) {
    return Reflect.defineProperty(object, key, {
        value: value, writable: true, enumerable: true, configurable: true
    });
}

var payload = "A😀\ud800";
var wrapper = box(payload);
var prototype = Object.getPrototypeOf(wrapper);
var prototypeLength = Object.getOwnPropertyDescriptor(prototype, "length");
var wrapperLength = Object.getOwnPropertyDescriptor(wrapper, "length");
print("prototype-length=" + [
    prototypeLength.value, flags(prototypeLength),
    Object.prototype.toString.call(prototype), Object.isExtensible(prototype)
].join("|"));
print("wrapper=" + [
    Reflect.ownKeys(wrapper).map(keyName).join(","), wrapperLength.value,
    flags(wrapperLength), Object.isExtensible(wrapper)
].join("|"));

var indexRows = [];
for (var index = 0; index < 4; index++) {
    var descriptor = Object.getOwnPropertyDescriptor(wrapper, String(index));
    indexRows.push(index + ":" + units(descriptor.value) + ":" +
                   flags(descriptor) + ":" +
                   Object.prototype.hasOwnProperty.call(wrapper, String(index)));
}
indexRows.push("4:" +
               (Object.getOwnPropertyDescriptor(wrapper, "4") === undefined) + ":" +
               Object.prototype.hasOwnProperty.call(wrapper, "4"));
indexRows.push("length:" + wrapperLength.value + ":" + flags(wrapperLength) + ":" +
               Object.prototype.hasOwnProperty.call(wrapper, "length"));
print("get-has=" + indexRows.join("|"));

var tail = Symbol("tail");
dataProperty(wrapper, "foo", 1);
dataProperty(wrapper, "01", 2);
dataProperty(wrapper, "8", 3);
dataProperty(wrapper, tail, 4);
print("merged-keys=" + Reflect.ownKeys(wrapper).map(keyName).join(","));
print("merged-properties=" + ["8", "foo", "01", tail].map(function (key) {
    var descriptor = Object.getOwnPropertyDescriptor(wrapper, key);
    return keyName(key) + ":" + descriptor.value + ":" + flags(descriptor) + ":" +
           Object.prototype.hasOwnProperty.call(wrapper, key);
}).join("|"));

print("compatible=" + [
    Reflect.defineProperty(wrapper, "0", {}),
    Reflect.defineProperty(wrapper, "0", { value: "A" }),
    Reflect.defineProperty(wrapper, "0", {
        value: "A", writable: false, enumerable: true, configurable: false
    }),
    Reflect.defineProperty(wrapper, "1", { value: "\ud83d" })
].join("|"));
print("incompatible=" + [
    Reflect.defineProperty(wrapper, "0", { value: "X" }),
    Reflect.defineProperty(wrapper, "0", { writable: true }),
    Reflect.defineProperty(wrapper, "0", { enumerable: false }),
    Reflect.defineProperty(wrapper, "0", { configurable: true }),
    Reflect.defineProperty(wrapper, "0", { get: undefined })
].join("|"));

print("delete=" + [
    Reflect.deleteProperty(wrapper, "0"), Reflect.deleteProperty(wrapper, "length"),
    Reflect.deleteProperty(wrapper, "8"), Reflect.deleteProperty(wrapper, "missing"),
    Reflect.deleteProperty(wrapper, "foo"),
    Object.prototype.hasOwnProperty.call(wrapper, "0"),
    Object.prototype.hasOwnProperty.call(wrapper, "8"),
    Reflect.ownKeys(wrapper).map(keyName).join(",")
].join("|"));

var fixed = box(payload);
dataProperty(fixed, "foo", 1);
Object.preventExtensions(fixed);
print("prevent=" + [
    Object.isExtensible(fixed),
    Reflect.defineProperty(fixed, "0", {}),
    Reflect.defineProperty(fixed, "0", { value: "A" }),
    Reflect.defineProperty(fixed, "0", { value: "X" }),
    Reflect.defineProperty(fixed, "8", {
        value: 8, writable: true, enumerable: true, configurable: true
    }),
    Reflect.defineProperty(fixed, "foo", { value: 2 }), fixed.foo,
    Reflect.deleteProperty(fixed, "0"), Reflect.deleteProperty(fixed, "length"),
    Reflect.deleteProperty(fixed, "foo"),
    Reflect.ownKeys(fixed).map(keyName).join(",")
].join("|"));
print("tags=" + [
    Object.prototype.toString.call(wrapper),
    Object.prototype.toString.call(prototype)
].join("|"));
"#;

#[test]
fn string_wrapper_exotic_matches_pinned_quickjs() {
    let rust = rust_observations();
    assert_eq!(
        rust.len(),
        10,
        "the String exotic differential unexpectedly changed breadth"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String exotic differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "String wrapper exotic behavior differed from pinned QuickJS"
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let payload = JsString::try_from_utf16([0x41, 0xd83d, 0xde00, 0xd800]).unwrap();
    let wrapper = box_string(&runtime, &mut context, payload.clone());
    let prototype = runtime
        .get_prototype_of(&wrapper)
        .unwrap()
        .expect("String wrapper had no prototype");
    let length = runtime.intern_property_key("length").unwrap();
    let prototype_length = data_descriptor(&runtime, &prototype, &length);
    let wrapper_length = data_descriptor(&runtime, &wrapper, &length);
    let object_prototype = context.object_prototype().unwrap();
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");

    let mut observations = vec![format!(
        "prototype-length={}|{}|{}|{}",
        render_value(&prototype_length.0),
        flags(prototype_length.1, prototype_length.2, prototype_length.3),
        render_value(
            &context
                .call(&object_to_string, Value::Object(prototype.clone()), &[],)
                .unwrap(),
        ),
        runtime.is_extensible(&prototype).unwrap(),
    )];
    observations.push(format!(
        "wrapper={}|{}|{}|{}",
        own_key_names(&runtime, &wrapper, &[]).join(","),
        render_value(&wrapper_length.0),
        flags(wrapper_length.1, wrapper_length.2, wrapper_length.3),
        runtime.is_extensible(&wrapper).unwrap(),
    ));

    let mut index_rows = Vec::new();
    for (index, unit) in [0x41, 0xd83d, 0xde00, 0xd800].into_iter().enumerate() {
        let key = runtime.intern_property_key(&index.to_string()).unwrap();
        let (value, writable, enumerable, configurable) = data_descriptor(&runtime, &wrapper, &key);
        assert_eq!(
            value,
            Value::String(JsString::try_from_utf16([unit]).unwrap())
        );
        index_rows.push(format!(
            "{index}:{}:{}:{}",
            value_units(&value),
            flags(writable, enumerable, configurable),
            runtime.has_own_property(&wrapper, &key).unwrap(),
        ));
    }
    let four = runtime.intern_property_key("4").unwrap();
    index_rows.push(format!(
        "4:{}:{}",
        runtime.get_own_property(&wrapper, &four).unwrap().is_none(),
        runtime.has_own_property(&wrapper, &four).unwrap(),
    ));
    index_rows.push(format!(
        "length:{}:{}:{}",
        render_value(&wrapper_length.0),
        flags(wrapper_length.1, wrapper_length.2, wrapper_length.3),
        runtime.has_own_property(&wrapper, &length).unwrap(),
    ));
    observations.push(format!("get-has={}", index_rows.join("|")));

    let foo_key = runtime.intern_property_key("foo").unwrap();
    let leading_zero = runtime.intern_property_key("01").unwrap();
    let eight = runtime.intern_property_key("8").unwrap();
    let tail = PropertyKey::from(
        runtime
            .new_symbol(Some(JsString::try_from_utf8("tail").unwrap()))
            .unwrap(),
    );
    for (key, value) in [
        (&foo_key, Value::Int(1)),
        (&leading_zero, Value::Int(2)),
        (&eight, Value::Int(3)),
        (&tail, Value::Int(4)),
    ] {
        assert!(define_data(&runtime, &wrapper, key, value));
    }
    observations.push(format!(
        "merged-keys={}",
        own_key_names(&runtime, &wrapper, &[(&tail, "Symbol(tail)")]).join(",")
    ));
    observations.push(format!(
        "merged-properties={}",
        [
            (&eight, "8"),
            (&foo_key, "foo"),
            (&leading_zero, "01"),
            (&tail, "Symbol(tail)"),
        ]
        .into_iter()
        .map(|(key, name)| {
            let (value, writable, enumerable, configurable) =
                data_descriptor(&runtime, &wrapper, key);
            format!(
                "{name}:{}:{}:{}",
                render_value(&value),
                flags(writable, enumerable, configurable),
                runtime.has_own_property(&wrapper, key).unwrap(),
            )
        })
        .collect::<Vec<_>>()
        .join("|")
    ));

    let zero = runtime.intern_property_key("0").unwrap();
    let one = runtime.intern_property_key("1").unwrap();
    let compatible = [
        OrdinaryPropertyDescriptor::new(),
        OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::String(JsString::try_from_utf8("A").unwrap())),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::String(JsString::try_from_utf8("A").unwrap())),
            writable: DescriptorField::Present(false),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(false),
            ..OrdinaryPropertyDescriptor::new()
        },
    ]
    .into_iter()
    .map(|descriptor| {
        runtime
            .define_own_property(&wrapper, &zero, &descriptor)
            .unwrap()
            .to_string()
    })
    .chain(std::iter::once(
        runtime
            .define_own_property(
                &wrapper,
                &one,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(
                        JsString::try_from_utf16([0xd83d]).unwrap(),
                    )),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
            .to_string(),
    ))
    .collect::<Vec<_>>();
    observations.push(format!("compatible={}", compatible.join("|")));

    let incompatible = [
        OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::String(JsString::try_from_utf8("X").unwrap())),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            writable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            enumerable: DescriptorField::Present(false),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        },
        OrdinaryPropertyDescriptor {
            get: DescriptorField::Present(AccessorValue::Undefined),
            ..OrdinaryPropertyDescriptor::new()
        },
    ]
    .into_iter()
    .map(|descriptor| {
        runtime
            .define_own_property(&wrapper, &zero, &descriptor)
            .unwrap()
            .to_string()
    })
    .collect::<Vec<_>>();
    observations.push(format!("incompatible={}", incompatible.join("|")));

    let missing = runtime.intern_property_key("missing").unwrap();
    let deleted = [
        runtime.delete_property(&wrapper, &zero).unwrap(),
        runtime.delete_property(&wrapper, &length).unwrap(),
        runtime.delete_property(&wrapper, &eight).unwrap(),
        runtime.delete_property(&wrapper, &missing).unwrap(),
        runtime.delete_property(&wrapper, &foo_key).unwrap(),
    ];
    observations.push(format!(
        "delete={}|{}|{}|{}|{}|{}|{}|{}",
        deleted[0],
        deleted[1],
        deleted[2],
        deleted[3],
        deleted[4],
        runtime.has_own_property(&wrapper, &zero).unwrap(),
        runtime.has_own_property(&wrapper, &eight).unwrap(),
        own_key_names(&runtime, &wrapper, &[(&tail, "Symbol(tail)")]).join(","),
    ));

    let fixed = box_string(&runtime, &mut context, payload);
    let fixed_foo = runtime.intern_property_key("foo").unwrap();
    assert!(define_data(&runtime, &fixed, &fixed_foo, Value::Int(1)));
    runtime.prevent_extensions(&fixed).unwrap();
    let fixed_zero = runtime.intern_property_key("0").unwrap();
    let fixed_eight = runtime.intern_property_key("8").unwrap();
    let fixed_length = runtime.intern_property_key("length").unwrap();
    let prevent = [
        runtime.is_extensible(&fixed).unwrap().to_string(),
        runtime
            .define_own_property(&fixed, &fixed_zero, &OrdinaryPropertyDescriptor::new())
            .unwrap()
            .to_string(),
        runtime
            .define_own_property(
                &fixed,
                &fixed_zero,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(
                        JsString::try_from_utf8("A").unwrap(),
                    )),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
            .to_string(),
        runtime
            .define_own_property(
                &fixed,
                &fixed_zero,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(
                        JsString::try_from_utf8("X").unwrap(),
                    )),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
            .to_string(),
        define_data(&runtime, &fixed, &fixed_eight, Value::Int(8)).to_string(),
        runtime
            .define_own_property(
                &fixed,
                &fixed_foo,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(2)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
            .to_string(),
        render_value(&data_descriptor(&runtime, &fixed, &fixed_foo).0),
        runtime
            .delete_property(&fixed, &fixed_zero)
            .unwrap()
            .to_string(),
        runtime
            .delete_property(&fixed, &fixed_length)
            .unwrap()
            .to_string(),
        runtime
            .delete_property(&fixed, &fixed_foo)
            .unwrap()
            .to_string(),
        own_key_names(&runtime, &fixed, &[]).join(","),
    ];
    observations.push(format!("prevent={}", prevent.join("|")));
    observations.push(format!(
        "tags={}|{}",
        render_value(
            &context
                .call(&object_to_string, Value::Object(wrapper), &[])
                .unwrap(),
        ),
        render_value(
            &context
                .call(&object_to_string, Value::Object(prototype), &[])
                .unwrap(),
        ),
    ));

    observations
}

#[test]
fn sloppy_string_boxing_uses_the_bytecode_functions_defining_realm() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_boxer = eval_callable(&runtime, &mut first, "(function () { return this; })");
    let second_boxer = eval_callable(&runtime, &mut second, "(function () { return this; })");
    let payload = JsString::try_from_utf16([0x41, 0xd800]).unwrap();

    let first_wrapper = expect_object(
        second
            .call(&first_boxer, Value::String(payload.clone()), &[])
            .unwrap(),
        "foreign call to first-realm sloppy boxer",
    );
    let first_wrapper_again = expect_object(
        first
            .call(&first_boxer, Value::String(payload.clone()), &[])
            .unwrap(),
        "local call to first-realm sloppy boxer",
    );
    let second_wrapper = expect_object(
        first
            .call(&second_boxer, Value::String(payload), &[])
            .unwrap(),
        "foreign call to second-realm sloppy boxer",
    );
    let first_prototype = runtime.get_prototype_of(&first_wrapper).unwrap().unwrap();
    let first_prototype_again = runtime
        .get_prototype_of(&first_wrapper_again)
        .unwrap()
        .unwrap();
    let second_prototype = runtime.get_prototype_of(&second_wrapper).unwrap().unwrap();

    assert_eq!(first_prototype, first_prototype_again);
    assert_ne!(first_prototype, second_prototype);
    let length = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        data_descriptor(&runtime, &first_prototype, &length),
        (Value::Int(0), false, false, true)
    );
    assert_eq!(
        data_descriptor(&runtime, &second_prototype, &length),
        (Value::Int(0), false, false, true)
    );
}

#[test]
fn rooted_string_wrapper_preserves_payload_and_final_release_collects_the_graph() {
    let runtime = Runtime::new();
    let wrapper = {
        let mut context = runtime.new_context();
        box_string(
            &runtime,
            &mut context,
            JsString::try_from_utf16([0x41, 0xd83d, 0xde00, 0xd800]).unwrap(),
        )
    };

    runtime.run_gc().unwrap();
    let index = runtime.intern_property_key("3").unwrap();
    assert_eq!(
        data_descriptor(&runtime, &wrapper, &index),
        (
            Value::String(JsString::try_from_utf16([0xd800]).unwrap()),
            false,
            true,
            false,
        ),
        "the rooted wrapper must preserve its exact UTF-16 payload across GC"
    );
    assert_eq!(
        own_key_names(&runtime, &wrapper, &[]),
        ["0", "1", "2", "3", "length"]
    );

    // The String prototype now publishes the seventeen-key partial surface;
    // its saved-method realm edge is tested by the UTF-16-prefix oracle. This
    // test isolates wrapper/payload survival and final cleanup.
    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

fn box_string(runtime: &Runtime, context: &mut Context, value: JsString) -> ObjectRef {
    let boxer = eval_callable(runtime, context, "(function () { return this; })");
    expect_object(
        context.call(&boxer, Value::String(value), &[]).unwrap(),
        "sloppy String this boxing",
    )
}

fn eval_callable(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("source did not produce a function: {source:?}");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("source was not callable: {source:?}"))
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let Value::Object(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(object) = value else {
        panic!("{description} did not produce an object");
    };
    object
}

fn define_data(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey, value: Value) -> bool {
    runtime
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
}

fn data_descriptor(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
) -> (Value, bool, bool, bool) {
    let CompleteOrdinaryPropertyDescriptor::Data {
        value,
        writable,
        enumerable,
        configurable,
    } = runtime
        .get_own_property(object, key)
        .unwrap()
        .expect("data property was absent")
    else {
        panic!("property was not data");
    };
    (value, writable, enumerable, configurable)
}

fn own_key_names(
    runtime: &Runtime,
    object: &ObjectRef,
    symbols: &[(&PropertyKey, &str)],
) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .iter()
        .map(|key| {
            symbols
                .iter()
                .find_map(|(symbol, name)| (key == *symbol).then(|| (*name).to_owned()))
                .unwrap_or_else(|| {
                    runtime
                        .property_key_to_js_string(key)
                        .unwrap()
                        .to_utf8_lossy()
                })
        })
        .collect()
}

fn flags(writable: bool, enumerable: bool, configurable: bool) -> String {
    format!(
        "{}{}{}",
        u8::from(writable),
        u8::from(enumerable),
        u8::from(configurable)
    )
}

fn value_units(value: &Value) -> String {
    let Value::String(value) = value else {
        panic!("String exotic index was not a string");
    };
    value
        .utf16_units()
        .map(|unit| format!("{unit:04x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn render_value(value: &Value) -> String {
    value
        .to_js_string()
        .expect("String exotic observation must stringify")
        .to_utf8_lossy()
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS String exotic oracle");
    assert!(
        output.status.success(),
        "QuickJS String exotic oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS String exotic oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
