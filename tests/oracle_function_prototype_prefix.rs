use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value,
    WellKnownSymbol,
};

const ORACLE_PROBE: &str = r#"
function bit(value) { return value ? 1 : 0; }
function show(value) {
    if (value === undefined) return "undefined";
    if (typeof value === "boolean") return String(value);
    if (typeof value === "number") return String(value);
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

const fp = Function.prototype;
const prefix = Reflect.ownKeys(fp).slice(0, 5).map(String).join(",");
console.log("keys=" + prefix);

const caller = Object.getOwnPropertyDescriptor(fp, "caller");
const args = Object.getOwnPropertyDescriptor(fp, "arguments");
const shared = caller.get === caller.set &&
               caller.get === args.get &&
               caller.get === args.set;
console.log("legacy-desc=caller:" + bit(caller.enumerable) + "," + bit(caller.configurable) +
            "|arguments:" + bit(args.enumerable) + "," + bit(args.configurable) +
            "|shared:" + bit(shared));

const thrower = caller.get;
const throwerLength = Object.getOwnPropertyDescriptor(thrower, "length");
const throwerName = Object.getOwnPropertyDescriptor(thrower, "name");
console.log("thrower=length:" + throwerLength.value + "|" +
            bit(throwerLength.writable) + "," + bit(throwerLength.enumerable) + "," + bit(throwerLength.configurable) +
            "|name:" + (throwerName.value === "" ? "<empty>" : throwerName.value) + "|" +
            bit(throwerName.writable) + "," + bit(throwerName.enumerable) + "," + bit(throwerName.configurable) +
            "|keys:" + Reflect.ownKeys(thrower).map(String).join(",") +
            "|extensible:" + bit(Object.isExtensible(thrower)) +
            "|frozen:" + bit(Object.isFrozen(thrower)));

console.log("legacy-sloppy=" + observe(() => caller.get.call(function sloppy() {})));
console.log("legacy-strict=" + observe(() => caller.get.call(function strict() { "use strict"; })));
console.log("legacy-set=" + observe(() => caller.set.call(function sloppy() {}, 1)));

const call = fp.call;
console.log("call-zero=" + observe(() => call.call(function one(a) { return a; })));
console.log("call-this=" + observe(() => call.call(function strictThis() { "use strict"; return this; }, 17)));
console.log("call-args=" + observe(() => call.call(function add(a, b) { return a + b; }, 99, 20, 22)));
console.log("call-throw=" + observe(() => call.call(function fail() { throw "boom"; })));
console.log("call-noncallable=" + observe(() => call.call(1)));

const hasInstanceDescriptor = Object.getOwnPropertyDescriptor(fp, Symbol.hasInstance);
console.log("has-instance-desc=" + bit(hasInstanceDescriptor.writable) + "," +
            bit(hasInstanceDescriptor.enumerable) + "," + bit(hasInstanceDescriptor.configurable));
const hasInstance = hasInstanceDescriptor.value;

const poisonTarget = {};
Object.defineProperty(poisonTarget, "prototype", {
    get() { throw "prototype read"; },
    configurable: true,
});
console.log("has-noncallable-short=" + observe(() => hasInstance.call(poisonTarget, {})));

Object.defineProperty(fp, "prototype", {
    get() { throw "prototype read"; },
    configurable: true,
});
console.log("has-primitive-short=" + observe(() => hasInstance.call(fp, 1)));
delete fp.prototype;

function InstanceTarget() {}
console.log("has-true=" + observe(() => hasInstance.call(InstanceTarget, Object.create(InstanceTarget.prototype))));
console.log("has-false=" + observe(() => hasInstance.call(InstanceTarget, {})));

function BadPrototype() {}
BadPrototype.prototype = 1;
console.log("has-bad-prototype=" + observe(() => hasInstance.call(BadPrototype, {})));
"#;

const CALL_CONFIGURABLE_PROBE: &str = r#"
const fp = Function.prototype;
const accepted = Reflect.defineProperty(fp, "call", { configurable: true });
const descriptor = Object.getOwnPropertyDescriptor(fp, "call");
const forwarded = Reflect.apply(descriptor.value, function one(a) { return a; }, [undefined, 7]);
console.log("call-configurable=accepted:" + (accepted ? 1 : 0) +
            "|kind:data|value:" + typeof descriptor.value + ":" + forwarded +
            "|desc:" + (descriptor.writable ? 1 : 0) + "," +
            (descriptor.enumerable ? 1 : 0) + "," + (descriptor.configurable ? 1 : 0));
"#;

const CALL_ENUMERABLE_PROBE: &str = r#"
const fp = Function.prototype;
const accepted = Reflect.defineProperty(fp, "call", { enumerable: true });
const descriptor = Object.getOwnPropertyDescriptor(fp, "call");
const forwarded = Reflect.apply(descriptor.value, function one(a) { return a; }, [undefined, 7]);
console.log("call-enumerable=accepted:" + (accepted ? 1 : 0) +
            "|kind:data|value:" + typeof descriptor.value + ":" + forwarded +
            "|desc:" + (descriptor.writable ? 1 : 0) + "," +
            (descriptor.enumerable ? 1 : 0) + "," + (descriptor.configurable ? 1 : 0));
"#;

const CALL_ACCESSOR_PROBE: &str = r#"
const fp = Function.prototype;
function replacementGetter() { return 73; }
const accepted = Reflect.defineProperty(fp, "call", { get: replacementGetter });
const descriptor = Object.getOwnPropertyDescriptor(fp, "call");
console.log("call-accessor=accepted:" + (accepted ? 1 : 0) +
            "|kind:accessor|get:" + typeof descriptor.get +
            "|set:" + String(descriptor.set) +
            "|desc:" + (descriptor.enumerable ? 1 : 0) + "," +
            (descriptor.configurable ? 1 : 0) + "|value:" + fp.call);
"#;

const HAS_INSTANCE_CONFIGURABLE_PROBE: &str = r#"
const fp = Function.prototype;
const accepted = Reflect.defineProperty(fp, Symbol.hasInstance, { configurable: true });
const descriptor = Object.getOwnPropertyDescriptor(fp, Symbol.hasInstance);
console.log("has-instance-configurable=accepted:" + (accepted ? 1 : 0) +
            "|kind:data|value:" + typeof descriptor.value +
            "|desc:" + (descriptor.writable ? 1 : 0) + "," +
            (descriptor.enumerable ? 1 : 0) + "," + (descriptor.configurable ? 1 : 0));
"#;

#[test]
fn function_prototype_prefix_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Function.prototype prefix differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(rust_observations(), oracle_observations(&oracle));
}

#[test]
fn lazy_function_prototype_defines_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP Function.prototype lazy define differential: set QJS_ORACLE to upstream qjs"
        );
        return;
    };

    assert_eq!(
        rust_lazy_define_observations(),
        [
            ("call configurable", CALL_CONFIGURABLE_PROBE),
            ("call enumerable", CALL_ENUMERABLE_PROBE),
            ("call accessor", CALL_ACCESSOR_PROBE),
            (
                "@@hasInstance configurable rejection",
                HAS_INSTANCE_CONFIGURABLE_PROBE,
            ),
        ]
        .into_iter()
        .map(|(description, probe)| oracle_observation(&oracle, probe, description))
        .collect::<Vec<_>>()
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();

    let prefix = runtime
        .own_property_keys(&function_prototype)
        .unwrap()
        .into_iter()
        .take(5)
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect::<Vec<_>>()
        .join(",");

    let caller_key = runtime.intern_property_key("caller").unwrap();
    let arguments_key = runtime.intern_property_key("arguments").unwrap();
    let (caller_get, caller_set, caller_enumerable, caller_configurable) =
        accessor_descriptor(&runtime, &function_prototype, &caller_key);
    let (arguments_get, arguments_set, arguments_enumerable, arguments_configurable) =
        accessor_descriptor(&runtime, &function_prototype, &arguments_key);
    let shared = caller_get.as_object() == caller_set.as_object()
        && caller_get.as_object() == arguments_get.as_object()
        && caller_get.as_object() == arguments_set.as_object();

    let length_key = runtime.intern_property_key("length").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let (thrower_length, length_writable, length_enumerable, length_configurable) =
        data_descriptor(&runtime, caller_get.as_object(), &length_key);
    let (thrower_name, name_writable, name_enumerable, name_configurable) =
        data_descriptor(&runtime, caller_get.as_object(), &name_key);
    let thrower_keys = key_names(&runtime, caller_get.as_object()).join(",");
    let thrower_extensible = runtime.is_extensible(caller_get.as_object()).unwrap();
    let thrower_frozen = !thrower_extensible
        && runtime
            .own_property_keys(caller_get.as_object())
            .unwrap()
            .into_iter()
            .all(|key| {
                match runtime
                    .get_own_property(caller_get.as_object(), &key)
                    .unwrap()
                    .unwrap()
                {
                    CompleteOrdinaryPropertyDescriptor::Data {
                        writable,
                        configurable,
                        ..
                    } => !writable && !configurable,
                    CompleteOrdinaryPropertyDescriptor::Accessor { configurable, .. } => {
                        !configurable
                    }
                }
            });
    let Value::Int(thrower_length) = thrower_length else {
        panic!("%ThrowTypeError%.length was not an integer");
    };
    let Value::String(thrower_name) = thrower_name else {
        panic!("%ThrowTypeError%.name was not a string");
    };
    let thrower_name = if thrower_name.is_empty() {
        "<empty>".to_owned()
    } else {
        thrower_name.to_utf8_lossy()
    };

    let sloppy = function(&runtime, &mut context, "(function sloppy(){})");
    let strict = function(
        &runtime,
        &mut context,
        "(function strict(){ \"use strict\"; })",
    );
    let legacy_sloppy = observe_call(
        &runtime,
        &mut context,
        &caller_get,
        Value::Object(sloppy.as_object().clone()),
        &[],
    );
    let legacy_strict = observe_call(
        &runtime,
        &mut context,
        &caller_get,
        Value::Object(strict.as_object().clone()),
        &[],
    );
    let legacy_set = observe_call(
        &runtime,
        &mut context,
        &caller_set,
        Value::Object(sloppy.as_object().clone()),
        &[Value::Int(1)],
    );

    let call_key = runtime.intern_property_key("call").unwrap();
    let Value::Object(call_object) = context
        .get_property(&function_prototype, &call_key)
        .unwrap()
    else {
        panic!("Function.prototype.call was not an object");
    };
    let call = runtime
        .as_callable(&call_object)
        .unwrap()
        .expect("Function.prototype.call was not callable");
    let one = function(&runtime, &mut context, "(function one(a){ return a; })");
    let strict_this = function(
        &runtime,
        &mut context,
        "(function strictThis(){ \"use strict\"; return this; })",
    );
    let add = function(
        &runtime,
        &mut context,
        "(function add(a, b){ return a + b; })",
    );
    let fail = function(
        &runtime,
        &mut context,
        "(function fail(){ throw \"boom\"; })",
    );
    let call_zero = observe_call(
        &runtime,
        &mut context,
        &call,
        Value::Object(one.as_object().clone()),
        &[],
    );
    let call_this = observe_call(
        &runtime,
        &mut context,
        &call,
        Value::Object(strict_this.as_object().clone()),
        &[Value::Int(17)],
    );
    let call_args = observe_call(
        &runtime,
        &mut context,
        &call,
        Value::Object(add.as_object().clone()),
        &[Value::Int(99), Value::Int(20), Value::Int(22)],
    );
    let call_throw = observe_call(
        &runtime,
        &mut context,
        &call,
        Value::Object(fail.as_object().clone()),
        &[],
    );
    let call_noncallable = observe_call(&runtime, &mut context, &call, Value::Int(1), &[]);

    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let (
        Value::Object(has_instance_object),
        has_instance_writable,
        has_instance_enumerable,
        has_instance_configurable,
    ) = data_descriptor(&runtime, &function_prototype, &has_instance_key)
    else {
        panic!("Function.prototype[@@hasInstance] was not an object");
    };
    let has_instance = runtime
        .as_callable(&has_instance_object)
        .unwrap()
        .expect("Function.prototype[@@hasInstance] was not callable");

    let prototype_getter = function(
        &runtime,
        &mut context,
        "(function prototypeGetter(){ throw \"prototype read\"; })",
    );
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let poison_target = context.new_object().unwrap();
    assert!(
        context
            .define_own_property(
                &poison_target,
                &prototype_key,
                &getter_descriptor(prototype_getter.clone()),
            )
            .unwrap()
    );
    let candidate = context.new_object().unwrap();
    let has_noncallable_short = observe_call(
        &runtime,
        &mut context,
        &has_instance,
        Value::Object(poison_target),
        &[Value::Object(candidate)],
    );

    assert!(
        context
            .define_own_property(
                &function_prototype,
                &prototype_key,
                &getter_descriptor(prototype_getter),
            )
            .unwrap()
    );
    let has_primitive_short = observe_call(
        &runtime,
        &mut context,
        &has_instance,
        Value::Object(function_prototype.clone()),
        &[Value::Int(1)],
    );
    assert!(
        runtime
            .delete_property(&function_prototype, &prototype_key)
            .unwrap()
    );

    let instance_target = function(&runtime, &mut context, "(function InstanceTarget(){})");
    let Value::Object(instance_prototype) = context
        .get_property(instance_target.as_object(), &prototype_key)
        .unwrap()
    else {
        panic!("InstanceTarget.prototype was not an object");
    };
    let positive_candidate = context
        .new_object_with_prototype(Some(&instance_prototype))
        .unwrap();
    let negative_candidate = context.new_object().unwrap();
    let has_true = observe_call(
        &runtime,
        &mut context,
        &has_instance,
        Value::Object(instance_target.as_object().clone()),
        &[Value::Object(positive_candidate)],
    );
    let has_false = observe_call(
        &runtime,
        &mut context,
        &has_instance,
        Value::Object(instance_target.as_object().clone()),
        &[Value::Object(negative_candidate)],
    );

    let bad_prototype = function(&runtime, &mut context, "(function BadPrototype(){})");
    assert!(
        context
            .set_property(bad_prototype.as_object(), &prototype_key, Value::Int(1))
            .unwrap()
    );
    let bad_candidate = context.new_object().unwrap();
    let has_bad_prototype = observe_call(
        &runtime,
        &mut context,
        &has_instance,
        Value::Object(bad_prototype.as_object().clone()),
        &[Value::Object(bad_candidate)],
    );

    vec![
        format!("keys={prefix}"),
        format!(
            "legacy-desc=caller:{},{}|arguments:{},{}|shared:{}",
            bit(caller_enumerable),
            bit(caller_configurable),
            bit(arguments_enumerable),
            bit(arguments_configurable),
            bit(shared),
        ),
        format!(
            "thrower=length:{thrower_length}|{},{},{}|name:{thrower_name}|{},{},{}|keys:{thrower_keys}|extensible:{}|frozen:{}",
            bit(length_writable),
            bit(length_enumerable),
            bit(length_configurable),
            bit(name_writable),
            bit(name_enumerable),
            bit(name_configurable),
            bit(thrower_extensible),
            bit(thrower_frozen),
        ),
        format!("legacy-sloppy={legacy_sloppy}"),
        format!("legacy-strict={legacy_strict}"),
        format!("legacy-set={legacy_set}"),
        format!("call-zero={call_zero}"),
        format!("call-this={call_this}"),
        format!("call-args={call_args}"),
        format!("call-throw={call_throw}"),
        format!("call-noncallable={call_noncallable}"),
        format!(
            "has-instance-desc={},{},{}",
            bit(has_instance_writable),
            bit(has_instance_enumerable),
            bit(has_instance_configurable),
        ),
        format!("has-noncallable-short={has_noncallable_short}"),
        format!("has-primitive-short={has_primitive_short}"),
        format!("has-true={has_true}"),
        format!("has-false={has_false}"),
        format!("has-bad-prototype={has_bad_prototype}"),
    ]
}

fn rust_lazy_define_observations() -> Vec<String> {
    vec![
        rust_call_data_define(
            "call-configurable",
            OrdinaryPropertyDescriptor {
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        ),
        rust_call_data_define(
            "call-enumerable",
            OrdinaryPropertyDescriptor {
                enumerable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        ),
        rust_call_accessor_define(),
        rust_has_instance_configurable_rejection(),
    ]
}

fn rust_call_data_define(label: &str, descriptor: OrdinaryPropertyDescriptor) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let call_key = runtime.intern_property_key("call").unwrap();

    // This is deliberately the first operation which touches the lazy `call`
    // slot in this fresh realm.
    let accepted = runtime
        .define_own_property(&function_prototype, &call_key, &descriptor)
        .unwrap();
    let (Value::Object(call_object), writable, enumerable, configurable) =
        data_descriptor(&runtime, &function_prototype, &call_key)
    else {
        panic!("Function.prototype.call was not a data function");
    };
    let call = runtime
        .as_callable(&call_object)
        .unwrap()
        .expect("Function.prototype.call was not callable");
    let one = function(&runtime, &mut context, "(function one(a){ return a; })");
    let forwarded = observe_call(
        &runtime,
        &mut context,
        &call,
        Value::Object(one.as_object().clone()),
        &[Value::Undefined, Value::Int(7)],
    );

    format!(
        "{label}=accepted:{}|kind:data|value:function:{forwarded}|desc:{},{},{}",
        bit(accepted),
        bit(writable),
        bit(enumerable),
        bit(configurable),
    )
}

fn rust_call_accessor_define() -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let call_key = runtime.intern_property_key("call").unwrap();
    let getter = function(
        &runtime,
        &mut context,
        "(function replacementGetter(){ return 73; })",
    );

    // Creating the getter does not read `Function.prototype.call`; the define
    // remains the first operation on that lazy slot in this fresh realm.
    let accepted = runtime
        .define_own_property(
            &function_prototype,
            &call_key,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(getter)),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
        .unwrap();
    let Some(CompleteOrdinaryPropertyDescriptor::Accessor {
        get: Some(get),
        set: None,
        enumerable,
        configurable,
    }) = runtime
        .get_own_property(&function_prototype, &call_key)
        .unwrap()
    else {
        panic!("Function.prototype.call was not replaced by the requested accessor");
    };
    assert!(runtime.as_callable(get.as_object()).unwrap().is_some());
    let Value::Int(observed) = context
        .get_property(&function_prototype, &call_key)
        .unwrap()
    else {
        panic!("replacement call getter did not return its observable integer");
    };

    format!(
        "call-accessor=accepted:{}|kind:accessor|get:function|set:undefined|desc:{},{}|value:{observed}",
        bit(accepted),
        bit(enumerable),
        bit(configurable),
    )
}

fn rust_has_instance_configurable_rejection() -> String {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));

    // Public integration APIs can observe the pre-materialization rejection
    // and the eventual descriptor, but not the internal AutoInit slot kind.
    // A runtime unit should additionally assert the rejected define leaves the
    // slot as AutoInit until this descriptor read materializes it.
    let accepted = runtime
        .define_own_property(
            &function_prototype,
            &has_instance_key,
            &OrdinaryPropertyDescriptor {
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
        .unwrap();
    let (Value::Object(value), writable, enumerable, configurable) =
        data_descriptor(&runtime, &function_prototype, &has_instance_key)
    else {
        panic!("Function.prototype[@@hasInstance] was not a data function");
    };
    assert!(runtime.as_callable(&value).unwrap().is_some());

    format!(
        "has-instance-configurable=accepted:{}|kind:data|value:function|desc:{},{},{}",
        bit(accepted),
        bit(writable),
        bit(enumerable),
        bit(configurable),
    )
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

fn accessor_descriptor(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
) -> (CallableRef, CallableRef, bool, bool) {
    let Some(CompleteOrdinaryPropertyDescriptor::Accessor {
        get: Some(get),
        set: Some(set),
        enumerable,
        configurable,
    }) = runtime.get_own_property(object, key).unwrap()
    else {
        panic!("expected a complete accessor descriptor");
    };
    (get, set, enumerable, configurable)
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

fn getter_descriptor(getter: CallableRef) -> OrdinaryPropertyDescriptor {
    OrdinaryPropertyDescriptor {
        get: DescriptorField::Present(AccessorValue::Callable(getter)),
        set: DescriptorField::Present(AccessorValue::Undefined),
        enumerable: DescriptorField::Present(false),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    }
}

fn key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect()
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

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS Function.prototype prefix oracle");
    assert!(
        output.status.success(),
        "QuickJS Function.prototype prefix oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Function.prototype prefix oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn oracle_observation(oracle: &OsStr, probe: &str, description: &str) -> String {
    let output = Command::new(oracle)
        .args(["-e", probe])
        .output()
        .unwrap_or_else(|error| panic!("run QuickJS {description} oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS {description} oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS {description} emitted non-UTF-8 output: {error}"));
    let mut lines = stdout.lines();
    let observation = lines
        .next()
        .unwrap_or_else(|| panic!("QuickJS {description} emitted no observation"));
    assert!(
        lines.next().is_none(),
        "QuickJS {description} emitted multiple observations: {stdout:?}"
    );
    observation.to_owned()
}
