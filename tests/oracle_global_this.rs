use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value,
    WellKnownSymbol,
};

const IMPLEMENTED_GLOBALS: &[&str] = &[
    "Error",
    "EvalError",
    "RangeError",
    "ReferenceError",
    "SyntaxError",
    "TypeError",
    "URIError",
    "InternalError",
    "Array",
    "Object",
    "Function",
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
    "eval",
    "Number",
    "Boolean",
    "String",
    "Math",
    "Reflect",
    "Symbol",
    "globalThis",
    "BigInt",
    "Date",
    "RegExp",
    "JSON",
    "Map",
    "Set",
];

const ORACLE_PROBE: &str = r#"
function bits(descriptor) {
    return (descriptor.writable ? "1" : "0") +
           (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0");
}

var root = globalThis;
var reader = (function() { return globalThis; });
var strictWriter = (function(value) { "use strict"; globalThis = value; });
var implemented = [
    "Error", "EvalError", "RangeError", "ReferenceError", "SyntaxError",
    "TypeError", "URIError", "InternalError", "Array", "Object", "Function", "parseInt",
    "parseFloat", "isNaN", "isFinite", "decodeURI", "decodeURIComponent",
    "encodeURI", "encodeURIComponent", "escape", "unescape", "Infinity",
    "NaN", "undefined", "eval", "Number", "Boolean", "String", "Math", "Reflect", "Symbol", "globalThis", "BigInt", "Date", "RegExp", "JSON", "Map", "Set"
];
var keys = Reflect.ownKeys(root);
var firstSymbol = keys.findIndex(function(key) { return typeof key === "symbol"; });

print("initial=" + [
    globalThis === root,
    bits(Object.getOwnPropertyDescriptor(root, "globalThis")),
    root.globalThis === root,
    globalThis === this
].join("|"));
print("keys=" + [
    keys.filter(function(key) {
        return typeof key === "string" && implemented.indexOf(key) >= 0;
    }).join(","),
    keys.slice(0, firstSymbol).every(function(key) { return typeof key === "string"; }) &&
        keys.slice(firstSymbol).every(function(key) { return typeof key === "symbol"; }),
    keys[keys.length - 1] === Symbol.toStringTag
].join("|"));

strictWriter(17);
print("assignment=" + [
    root.globalThis,
    bits(Object.getOwnPropertyDescriptor(root, "globalThis"))
].join("|"));
root.globalThis = root;

var deleted = delete root.globalThis;
print("delete=" + [
    deleted,
    Object.getOwnPropertyDescriptor(root, "globalThis") === undefined,
    typeof globalThis
].join("|"));

var strictMissing;
try {
    strictWriter(19);
    strictMissing = "no-throw";
} catch (error) {
    strictMissing = error.name + ":" + error.message + ":" +
                    (error instanceof ReferenceError);
}
print("strict-missing=" + [
    strictMissing,
    Object.getOwnPropertyDescriptor(root, "globalThis") === undefined
].join("|"));

globalThis = root;
print("sloppy=" + [
    globalThis === root,
    bits(Object.getOwnPropertyDescriptor(root, "globalThis"))
].join("|"));

Object.defineProperty(root, "globalThis", {
    value: root, writable: true, enumerable: false, configurable: true
});
print("direct=" + [
    globalThis === root,
    bits(Object.getOwnPropertyDescriptor(root, "globalThis"))
].join("|"));

Object.defineProperty(root, "globalThis", {
    get: function() { "use strict"; return this; },
    set: function(value) {
        "use strict";
        this.globalThisSetterValue = value;
        this.globalThisSetterReceiver = this;
    },
    enumerable: false,
    configurable: true
});
var accessor = Object.getOwnPropertyDescriptor(root, "globalThis");
strictWriter(47);
print("accessor=" + [
    reader() === root,
    typeof accessor.get,
    typeof accessor.set,
    root.globalThisSetterValue,
    root.globalThisSetterReceiver === root,
    accessor.enumerable,
    accessor.configurable
].join("|"));

delete root.globalThis;
var missing;
try {
    reader();
    missing = "no-throw";
} catch (error) {
    missing = error.name + ":" + error.message + ":" +
              (error instanceof ReferenceError);
}
print("captured-delete=" + [
    Object.getOwnPropertyDescriptor(root, "globalThis") === undefined,
    typeof globalThis,
    missing
].join("|"));

Object.defineProperty(root, "globalThis", {
    value: root, writable: true, enumerable: false, configurable: true
});
print("reconnect=" + [
    reader() === root,
    bits(Object.getOwnPropertyDescriptor(root, "globalThis"))
].join("|"));
"#;

const EXPECTED_OBSERVATIONS: &[&str] = &[
    "initial=true|101|true|true",
    "keys=Error,EvalError,RangeError,ReferenceError,SyntaxError,TypeError,URIError,InternalError,Array,Object,Function,parseInt,parseFloat,isNaN,isFinite,decodeURI,decodeURIComponent,encodeURI,encodeURIComponent,escape,unescape,Infinity,NaN,undefined,eval,Number,Boolean,String,Math,Reflect,Symbol,globalThis,BigInt,Date,RegExp,JSON,Map,Set|true|true",
    "assignment=17|101",
    "delete=true|true|undefined",
    "strict-missing=ReferenceError:'globalThis' is not defined:true|true",
    "sloppy=true|111",
    "direct=true|101",
    "accessor=true|function|function|47|true|false|true",
    "captured-delete=true|undefined|ReferenceError:'globalThis' is not defined:true",
    "reconnect=true|101",
];

#[test]
fn global_this_matches_pinned_quickjs() {
    let rust = rust_observations();
    assert_eq!(
        rust, EXPECTED_OBSERVATIONS,
        "Rust globalThis state-machine observations changed"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP globalThis differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let oracle = oracle_observations(&oracle);
    assert_eq!(
        oracle, EXPECTED_OBSERVATIONS,
        "the pinned QuickJS globalThis contract drifted"
    );
    assert_eq!(
        rust, oracle,
        "globalThis behavior differed from pinned QuickJS"
    );
}

#[test]
fn captured_global_this_tracks_the_defining_realm_across_property_transitions() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_global = defining.global_object().unwrap();
    let caller_global = caller.global_object().unwrap();
    let key = runtime.intern_property_key("globalThis").unwrap();

    assert_ne!(defining_global, caller_global);
    assert_eq!(
        defining.eval("globalThis").unwrap(),
        Value::Object(defining_global.clone())
    );
    assert_eq!(
        caller.eval("globalThis").unwrap(),
        Value::Object(caller_global.clone())
    );

    let reader = function(
        &runtime,
        &mut defining,
        "(function() { return globalThis; })",
    );
    assert_eq!(
        caller.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Object(defining_global.clone()),
        "a foreign call must resolve globalThis in the bytecode's defining realm"
    );

    let getter = function(
        &runtime,
        &mut defining,
        "(function() { \"use strict\"; return this; })",
    );
    assert!(
        defining
            .define_own_property(
                &defining_global,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        caller.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Object(defining_global.clone()),
        "the captured global VarRef must fall through to the replacement accessor"
    );

    assert!(runtime.delete_property(&defining_global, &key).unwrap());
    assert!(matches!(
        caller.call(&reader, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = caller.take_exception().unwrap().unwrap() else {
        panic!("missing captured globalThis did not throw an Error object");
    };
    let defining_reference_error = reference_error_prototype(&runtime, &mut defining);
    let caller_reference_error = reference_error_prototype(&runtime, &mut caller);
    assert_ne!(defining_reference_error, caller_reference_error);
    assert_eq!(
        runtime.get_prototype_of(&exception).unwrap(),
        Some(defining_reference_error),
        "the missing-name ReferenceError must be allocated in the reader's defining realm"
    );

    define_global_this(&mut defining, &defining_global, &key);
    assert_eq!(
        caller.call(&reader, Value::Undefined, &[]).unwrap(),
        Value::Object(defining_global),
        "the captured global VarRef must reconnect to a later data property"
    );
    assert_eq!(
        caller.eval("globalThis").unwrap(),
        Value::Object(caller_global),
        "mutating the defining realm must not replace the caller realm's self reference"
    );
}

#[test]
fn global_this_var_ref_cycle_is_collectable_after_context_drop() {
    let runtime = Runtime::new();
    {
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        let reader = function(
            &runtime,
            &mut context,
            "(function() { return globalThis; })",
        );
        assert_eq!(
            context.call(&reader, Value::Undefined, &[]).unwrap(),
            Value::Object(global)
        );
        let counts = runtime.heap_counts();
        assert_eq!(counts.context_nodes, 1);
        assert!(
            counts.var_ref_nodes > 0,
            "globalThis must use a global VarRef"
        );
    }

    assert_eq!(runtime.heap_counts().context_nodes, 1);
    runtime.run_gc().unwrap();
    let counts = runtime.heap_counts();
    assert_eq!(counts.context_nodes, 0);
    assert_eq!(counts.object_nodes, 0);
    assert_eq!(counts.shape_nodes, 0);
    assert_eq!(counts.var_ref_nodes, 0);
    assert_eq!(counts.function_bytecode_nodes, 0);
    assert_eq!(counts.live, 0);
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("globalThis").unwrap();
    let tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    let reader = function(
        &runtime,
        &mut context,
        "(function() { return globalThis; })",
    );
    let strict_writer = function(
        &runtime,
        &mut context,
        "(function(value) { \"use strict\"; globalThis = value; })",
    );

    let initial = runtime.get_own_property(&global, &key).unwrap().unwrap();
    let mut observations = vec![format!(
        "initial={}|{}|{}|{}",
        matches!(context.eval("globalThis").unwrap(), Value::Object(value) if value == global),
        descriptor_bits(&initial),
        matches!(context.get_property(&global, &key).unwrap(), Value::Object(value) if value == global),
        matches!(
            context.eval("globalThis === this").unwrap(),
            Value::Bool(true)
        ),
    )];

    let keys = runtime.own_property_keys(&global).unwrap();
    let implemented_keys = IMPLEMENTED_GLOBALS
        .iter()
        .map(|name| (*name, runtime.intern_property_key(name).unwrap()))
        .collect::<Vec<_>>();
    let implemented = keys
        .iter()
        .filter_map(|candidate| {
            implemented_keys
                .iter()
                .find_map(|(name, key)| (key == candidate).then_some(*name))
        })
        .collect::<Vec<_>>()
        .join(",");
    let expected_keys = implemented_keys
        .iter()
        .map(|(_, key)| key.clone())
        .chain(std::iter::once(tag.clone()))
        .collect::<Vec<_>>();
    observations.push(format!(
        "keys={implemented}|{}|{}",
        keys == expected_keys,
        keys.last() == Some(&tag),
    ));

    assert_eq!(
        context
            .call(&strict_writer, Value::Undefined, &[Value::Int(17)])
            .unwrap(),
        Value::Undefined
    );
    observations.push(format!(
        "assignment={}|{}",
        value_text(context.get_property(&global, &key).unwrap()),
        descriptor_bits(&runtime.get_own_property(&global, &key).unwrap().unwrap()),
    ));
    assert_eq!(
        context.eval("globalThis = this").unwrap(),
        Value::Object(global.clone())
    );

    let deleted = context.eval("delete globalThis").unwrap();
    observations.push(format!(
        "delete={}|{}|{}",
        value_text(deleted),
        runtime.get_own_property(&global, &key).unwrap().is_none(),
        value_text(context.eval("typeof globalThis").unwrap()),
    ));

    assert!(matches!(
        context.call(&strict_writer, Value::Undefined, &[Value::Int(19)]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("strict missing globalThis write did not throw an Error object");
    };
    let error_name = error_string_property(&runtime, &mut context, &exception, "name");
    let error_message = error_string_property(&runtime, &mut context, &exception, "message");
    let is_reference_error = runtime.get_prototype_of(&exception).unwrap()
        == Some(reference_error_prototype(&runtime, &mut context));
    observations.push(format!(
        "strict-missing={error_name}:{error_message}:{is_reference_error}|{}",
        runtime.get_own_property(&global, &key).unwrap().is_none(),
    ));

    assert_eq!(
        context.eval("globalThis = this").unwrap(),
        Value::Object(global.clone())
    );
    observations.push(format!(
        "sloppy={}|{}",
        matches!(context.eval("globalThis").unwrap(), Value::Object(value) if value == global),
        descriptor_bits(&runtime.get_own_property(&global, &key).unwrap().unwrap()),
    ));

    define_global_this(&mut context, &global, &key);
    observations.push(format!(
        "direct={}|{}",
        matches!(context.eval("globalThis").unwrap(), Value::Object(value) if value == global),
        descriptor_bits(&runtime.get_own_property(&global, &key).unwrap().unwrap()),
    ));

    let getter = function(
        &runtime,
        &mut context,
        "(function() { \"use strict\"; return this; })",
    );
    let setter = function(
        &runtime,
        &mut context,
        "(function(value) { \"use strict\"; this.globalThisSetterValue = value; this.globalThisSetterReceiver = this; })",
    );
    assert!(
        context
            .define_own_property(
                &global,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let accessor = runtime.get_own_property(&global, &key).unwrap().unwrap();
    let CompleteOrdinaryPropertyDescriptor::Accessor {
        get,
        set,
        enumerable,
        configurable,
    } = accessor
    else {
        panic!("globalThis did not become an accessor property");
    };
    assert_eq!(
        context
            .call(&strict_writer, Value::Undefined, &[Value::Int(47)])
            .unwrap(),
        Value::Undefined
    );
    let setter_value = global_value(&runtime, &mut context, &global, "globalThisSetterValue");
    let setter_receiver = global_value(&runtime, &mut context, &global, "globalThisSetterReceiver");
    observations.push(format!(
        "accessor={}|{}|{}|{}|{}|{enumerable}|{configurable}",
        matches!(context.call(&reader, Value::Undefined, &[]).unwrap(), Value::Object(value) if value == global),
        if get.is_some() { "function" } else { "undefined" },
        if set.is_some() { "function" } else { "undefined" },
        value_text(setter_value),
        matches!(setter_receiver, Value::Object(value) if value == global),
    ));

    assert!(runtime.delete_property(&global, &key).unwrap());
    assert!(matches!(
        context.call(&reader, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("captured missing globalThis did not throw an Error object");
    };
    let error_name = error_string_property(&runtime, &mut context, &exception, "name");
    let error_message = error_string_property(&runtime, &mut context, &exception, "message");
    let is_reference_error = runtime.get_prototype_of(&exception).unwrap()
        == Some(reference_error_prototype(&runtime, &mut context));
    observations.push(format!(
        "captured-delete={}|{}|{error_name}:{error_message}:{is_reference_error}",
        runtime.get_own_property(&global, &key).unwrap().is_none(),
        value_text(context.eval("typeof globalThis").unwrap()),
    ));

    define_global_this(&mut context, &global, &key);
    observations.push(format!(
        "reconnect={}|{}",
        matches!(context.call(&reader, Value::Undefined, &[]).unwrap(), Value::Object(value) if value == global),
        descriptor_bits(&runtime.get_own_property(&global, &key).unwrap().unwrap()),
    ));
    observations
}

fn define_global_this(context: &mut Context, global: &ObjectRef, key: &PropertyKey) {
    assert!(
        context
            .define_own_property(
                global,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Object(global.clone())),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn function(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("function source did not produce an object: {source}");
    };
    runtime.as_callable(&object).unwrap().unwrap()
}

fn descriptor_bits(descriptor: &CompleteOrdinaryPropertyDescriptor) -> String {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data {
            writable,
            enumerable,
            configurable,
            ..
        } => format!(
            "{}{}{}",
            u8::from(*writable),
            u8::from(*enumerable),
            u8::from(*configurable)
        ),
        CompleteOrdinaryPropertyDescriptor::Accessor { .. } => "accessor".to_owned(),
    }
}

fn reference_error_prototype(runtime: &Runtime, context: &mut Context) -> ObjectRef {
    let global = context.global_object().unwrap();
    let constructor_key = runtime.intern_property_key("ReferenceError").unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(constructor) = context.get_property(&global, &constructor_key).unwrap()
    else {
        panic!("ReferenceError was not an object");
    };
    let Value::Object(prototype) = context.get_property(&constructor, &prototype_key).unwrap()
    else {
        panic!("ReferenceError.prototype was not an object");
    };
    prototype
}

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &ObjectRef,
    name: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::String(value) = context.get_property(error, &key).unwrap() else {
        panic!("Error.{name} was not a string");
    };
    value.to_utf8_lossy()
}

fn global_value(runtime: &Runtime, context: &mut Context, global: &ObjectRef, name: &str) -> Value {
    let key = runtime.intern_property_key(name).unwrap();
    context.get_property(global, &key).unwrap()
}

fn value_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) | Value::Symbol(_) => panic!("unexpected scalar observation value"),
    }
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .unwrap_or_else(|error| panic!("could not execute QJS_ORACLE: {error}"));
    assert!(
        output.status.success(),
        "QJS_ORACLE failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QJS_ORACLE emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
