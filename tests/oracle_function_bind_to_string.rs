use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField,
    JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value,
    WellKnownSymbol,
};

const ORACLE_PROBE: &str = r#"
function bit(value) { return value ? 1 : 0; }
function hex(value) {
    var out = "";
    for (var index = 0; index < value.length; index++) {
        var unit = value.charCodeAt(index).toString(16);
        out += ("0000" + unit).slice(-4);
    }
    return out;
}
function valueType(value) {
    return typeof value;
}
function descriptorText(key, descriptor) {
    if (Object.prototype.hasOwnProperty.call(descriptor, "value")) {
        return String(key) + ":data:" + valueType(descriptor.value) + ":" +
               bit(descriptor.writable) + "," + bit(descriptor.enumerable) + "," +
               bit(descriptor.configurable);
    }
    return String(key) + ":accessor:" + bit(descriptor.enumerable) + "," +
           bit(descriptor.configurable);
}
function show(value) {
    if (value === undefined) return "undefined";
    if (typeof value === "boolean" || typeof value === "number" ||
        typeof value === "bigint") return String(value);
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

var fp = Function.prototype;
var implementedNames = {
    length: true,
    name: true,
    caller: true,
    arguments: true,
    call: true,
    apply: true,
    bind: true,
    toString: true,
};
var implementedKeys = Reflect.ownKeys(fp).filter(function (key) {
    return key === Symbol.hasInstance ||
           (typeof key === "string" && implementedNames[key] === true);
});
print("fp-keys=" + implementedKeys.map(String).join(","));
print("fp-desc=" + implementedKeys.map(function (key) {
    return descriptorText(key, Object.getOwnPropertyDescriptor(fp, key));
}).join("|"));
var bindDescriptor = Object.getOwnPropertyDescriptor(fp, "bind");
var toStringDescriptor = Object.getOwnPropertyDescriptor(fp, "toString");
var bindLengthDescriptor = Object.getOwnPropertyDescriptor(bindDescriptor.value, "length");
var bindNameDescriptor = Object.getOwnPropertyDescriptor(bindDescriptor.value, "name");
var toStringLengthDescriptor = Object.getOwnPropertyDescriptor(toStringDescriptor.value, "length");
var toStringNameDescriptor = Object.getOwnPropertyDescriptor(toStringDescriptor.value, "name");
print("builtin-meta=" +
      "bind-length:" + bindLengthDescriptor.value + ":" + bit(bindLengthDescriptor.writable) +
      "," + bit(bindLengthDescriptor.enumerable) + "," + bit(bindLengthDescriptor.configurable) +
      "|bind-name:" + bindNameDescriptor.value + ":" + bit(bindNameDescriptor.writable) +
      "," + bit(bindNameDescriptor.enumerable) + "," + bit(bindNameDescriptor.configurable) +
      "|toString-length:" + toStringLengthDescriptor.value + ":" + bit(toStringLengthDescriptor.writable) +
      "," + bit(toStringLengthDescriptor.enumerable) + "," + bit(toStringLengthDescriptor.configurable) +
      "|toString-name:" + toStringNameDescriptor.value + ":" + bit(toStringNameDescriptor.writable) +
      "," + bit(toStringNameDescriptor.enumerable) + "," + bit(toStringNameDescriptor.configurable));

var anonymous = (0, function  ( a , b ) { return a + b; });
var named = (0, function named /*kept*/ (value) {
  return value;
});
var anonymousSource = fp.toString.call(anonymous);
var namedSource = fp.toString.call(named);
Object.defineProperty(anonymous, "name", { value: "changed anonymous" });
Object.defineProperty(named, "name", { value: "changed named" });
print("source-anonymous=" + hex(anonymousSource) + "|renamed:" +
      hex(fp.toString.call(anonymous)));
print("source-named=" + hex(namedSource) + "|renamed:" +
      hex(fp.toString.call(named)));

function nativeTarget(a) {}
var boundNativeTarget = nativeTarget.bind(null);
print("native-template=" +
      "prototype:" + hex(fp.toString.call(fp)) +
      "|call:" + hex(fp.toString.call(fp.call)) +
      "|apply:" + hex(fp.toString.call(fp.apply)) +
      "|bind:" + hex(fp.toString.call(fp.bind)) +
      "|toString:" + hex(fp.toString.call(fp.toString)) +
      "|bound:" + hex(fp.toString.call(boundNativeTarget)));
print("invalid-object=" + observe(function () { return fp.toString.call({}); }));
print("invalid-number=" + observe(function () { return fp.toString.call(1); }));

Object.defineProperty(fp.call, "name", { value: 17 });
print("native-number-name=" + hex(fp.toString.call(fp.call)));
Object.defineProperty(fp.apply, "name", { value: Symbol("native-name") });
print("native-symbol-name=" + observe(function () { return fp.toString.call(fp.apply); }));
var nativeNameSentinel = {};
Object.defineProperty(fp.bind, "name", {
    get: function () { throw nativeNameSentinel; },
});
var nativeNameIdentity = false;
try { fp.toString.call(fp.bind); } catch (error) { nativeNameIdentity = error === nativeNameSentinel; }
print("native-getter-throw=" + nativeNameIdentity);

function withLength(value, boundCount) {
    function lengthTarget(a, b, c, d, e, f) {}
    Object.defineProperty(lengthTarget, "length", { value: value });
    if (boundCount === 2) return fp.bind.call(lengthTarget, null, 1, 2).length;
    return fp.bind.call(lengthTarget, null, 1).length;
}
function withoutLength() {
    function lengthTarget() {}
    delete lengthTarget.length;
    return fp.bind.call(lengthTarget, null, 1).length;
}
print("bind-length=" +
      "int:" + withLength(5, 2) +
      "|fraction:" + withLength(5.9, 2) +
      "|infinity:" + withLength(Infinity, 1) +
      "|negative-infinity:" + withLength(-Infinity, 1) +
      "|nan:" + withLength(NaN, 1) +
      "|string:" + withLength("5", 1) +
      "|bigint:" + withLength(5n, 1) +
      "|symbol:" + withLength(Symbol("length"), 1) +
      "|absent:" + withoutLength());

function withName(value) {
    function nameTarget() {}
    Object.defineProperty(nameTarget, "name", { value: value });
    return fp.bind.call(nameTarget, null);
}
var namedBound = withName("renamed");
var numberNamedBound = withName(17);
var symbolNamedBound = withName(Symbol("name"));
var undefinedNamedBound = withName(undefined);
var boundNameDescriptor = Object.getOwnPropertyDescriptor(namedBound, "name");
print("bind-name=" +
      "string:" + namedBound.name +
      "|number:" + numberNamedBound.name +
      "|symbol:" + symbolNamedBound.name +
      "|undefined:" + undefinedNamedBound.name +
      "|desc:" + bit(boundNameDescriptor.writable) + "," +
      bit(boundNameDescriptor.enumerable) + "," + bit(boundNameDescriptor.configurable));

var getterOrder = 0;
function orderedTarget(a, b, c) {}
Object.defineProperty(orderedTarget, "length", {
    get: function () { getterOrder = getterOrder * 10 + 1; return 3; },
});
Object.defineProperty(orderedTarget, "name", {
    get: function () { getterOrder = getterOrder * 10 + 2; return "ordered"; },
});
var orderedBound = fp.bind.call(orderedTarget, null, 1);
print("bind-getter-order=" + getterOrder + "|length:" + orderedBound.length +
      "|name:" + orderedBound.name);

var lengthThrowSentinel = {};
var lengthThrowLateName = 0;
function lengthThrowTarget() {}
Object.defineProperty(lengthThrowTarget, "length", {
    get: function () { throw lengthThrowSentinel; },
});
Object.defineProperty(lengthThrowTarget, "name", {
    get: function () { lengthThrowLateName++; return "late"; },
});
var lengthThrowIdentity = false;
try { fp.bind.call(lengthThrowTarget, null); }
catch (error) { lengthThrowIdentity = error === lengthThrowSentinel; }
print("bind-length-throw=" + lengthThrowIdentity + "|late-name:" + lengthThrowLateName);

var nameThrowSentinel = {};
var nameThrowOrder = 0;
function nameThrowTarget() {}
Object.defineProperty(nameThrowTarget, "length", {
    get: function () { nameThrowOrder = nameThrowOrder * 10 + 1; return 2; },
});
Object.defineProperty(nameThrowTarget, "name", {
    get: function () { nameThrowOrder = nameThrowOrder * 10 + 2; throw nameThrowSentinel; },
});
var nameThrowIdentity = false;
try { fp.bind.call(nameThrowTarget, null); }
catch (error) { nameThrowIdentity = error === nameThrowSentinel; }
print("bind-name-throw=" + nameThrowIdentity + "|order:" + nameThrowOrder);

var inheritedLengthReads = 0;
var inheritedLengthPrototype = Object.create(fp);
Object.defineProperty(inheritedLengthPrototype, "length", {
    get: function () { inheritedLengthReads++; return 99; },
});
function inheritedLengthTarget() {}
delete inheritedLengthTarget.length;
Object.setPrototypeOf(inheritedLengthTarget, inheritedLengthPrototype);
var inheritedLengthBound = fp.bind.call(inheritedLengthTarget, null, 1);
print("bind-inherited-length=reads:" + inheritedLengthReads +
      "|length:" + inheritedLengthBound.length);

function callTarget(a, b, c) {
    "use strict";
    return a * 100 + b * 10 + c;
}
var callReceiver = {};
var simpleBound = fp.bind.call(callTarget, callReceiver, 1);
var firstBound = fp.bind.call(callTarget, callReceiver, 1);
var nestedBound = fp.bind.call(firstBound, {}, 2);
print("bound-call=simple:" + simpleBound(2, 3) +
      "|nested:" + nestedBound(3) +
      "|nested-name:" + nestedBound.name +
      "|nested-length:" + nestedBound.length);

function ConstructTarget() { return new.target; }
var ignoredBoundThis = {};
var ConstructBound = fp.bind.call(ConstructTarget, ignoredBoundThis);
var constructed = new ConstructBound();
function OtherTarget() {}
var explicitlyConstructed = Reflect.construct(ConstructBound, [], OtherTarget);
var constructArgMarker = {};
function ConstructArgTarget(value) { return value; }
var ConstructArgBound = fp.bind.call(ConstructArgTarget, null, constructArgMarker);
var constructedArg = new ConstructArgBound();
print("bound-construct=" +
      "arg:" + (constructedArg === constructArgMarker) +
      "|new-target-target:" + (constructed === ConstructTarget) +
      "|bound-own-prototype:" + Object.prototype.hasOwnProperty.call(ConstructBound, "prototype") +
      "|explicit-new-target:" + (explicitlyConstructed === OtherTarget));

var instanceMarker = {};
var instanceOther = {};
function InstanceTarget() {}
Object.defineProperty(InstanceTarget, Symbol.hasInstance, {
    value: function (candidate) { return candidate === instanceMarker; },
    configurable: true,
});
var InstanceBound = fp.bind.call(InstanceTarget, null);
var customTrue = instanceMarker instanceof InstanceBound;
var customFalse = instanceOther instanceof InstanceBound;
var instanceThrowSentinel = {};
Object.defineProperty(InstanceTarget, Symbol.hasInstance, {
    value: function () { throw instanceThrowSentinel; },
    configurable: true,
});
var instanceThrowIdentity = false;
try { instanceMarker instanceof InstanceBound; }
catch (error) { instanceThrowIdentity = error === instanceThrowSentinel; }
print("bound-has-instance=true:" + customTrue + "|false:" + customFalse +
      "|throw:" + instanceThrowIdentity);

function DeepInstanceTarget() {}
var DeepInstanceBound = DeepInstanceTarget;
for (var deepIndex = 0; deepIndex < 512; deepIndex++) {
    DeepInstanceBound = fp.bind.call(DeepInstanceBound, null);
}
print("deep-bound-has-instance=" +
      fp[Symbol.hasInstance].call(DeepInstanceBound, 1));

var targetFunctionPrototype = Object.create(fp);
function CrossRealmTarget() {}
Object.setPrototypeOf(CrossRealmTarget, targetFunctionPrototype);
var crossRealmBound = fp.bind.call(CrossRealmTarget, null);
print("cross-realm-prototype=" +
      (Object.getPrototypeOf(crossRealmBound) === fp));
"#;

#[test]
fn function_bind_and_to_string_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP Function.prototype bind/toString differential: set QJS_ORACLE to upstream qjs"
        );
        return;
    };

    assert_eq!(rust_observations(), oracle_observations(&oracle));
}

fn rust_observations() -> Vec<String> {
    [
        rust_intrinsic_and_source_observations(),
        rust_bind_metadata_observations(),
        rust_bound_execution_observations(),
    ]
    .concat()
}

struct Harness {
    runtime: Runtime,
    context: Context,
    function_prototype: ObjectRef,
    bind: CallableRef,
    to_string: CallableRef,
}

impl Harness {
    fn new() -> Self {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let bind = property_callable(&runtime, &mut context, &function_prototype, "bind");
        let to_string = property_callable(&runtime, &mut context, &function_prototype, "toString");
        Self {
            runtime,
            context,
            function_prototype,
            bind,
            to_string,
        }
    }

    fn function(&mut self, source: &str) -> CallableRef {
        function(&self.runtime, &mut self.context, source)
    }

    fn bind_result(
        &mut self,
        target: &CallableRef,
        this_value: Value,
        bound_arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        let mut arguments = Vec::with_capacity(bound_arguments.len() + 1);
        arguments.push(this_value);
        arguments.extend_from_slice(bound_arguments);
        self.context.call(
            &self.bind,
            Value::Object(target.as_object().clone()),
            &arguments,
        )
    }

    fn bind(
        &mut self,
        target: &CallableRef,
        this_value: Value,
        bound_arguments: &[Value],
    ) -> CallableRef {
        let Value::Object(bound) = self
            .bind_result(target, this_value, bound_arguments)
            .expect("Function.prototype.bind failed")
        else {
            panic!("Function.prototype.bind did not return an object");
        };
        self.runtime
            .as_callable(&bound)
            .unwrap()
            .expect("Function.prototype.bind result was not callable")
    }

    fn function_to_string_result(&mut self, value: Value) -> Result<Value, RuntimeError> {
        self.context.call(&self.to_string, value, &[])
    }

    fn function_to_string(&mut self, value: Value) -> JsString {
        let Value::String(value) = self
            .function_to_string_result(value)
            .expect("Function.prototype.toString failed")
        else {
            panic!("Function.prototype.toString did not return a string");
        };
        value
    }
}

fn rust_intrinsic_and_source_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let runtime = harness.runtime.clone();
    let implemented = [
        "length",
        "name",
        "caller",
        "arguments",
        "call",
        "apply",
        "bind",
        "toString",
        "Symbol(Symbol.hasInstance)",
    ];
    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let implemented_keys = runtime
        .own_property_keys(&harness.function_prototype)
        .unwrap()
        .into_iter()
        .filter_map(|key| {
            let name = if key == has_instance_key {
                "Symbol(Symbol.hasInstance)".to_owned()
            } else {
                runtime
                    .property_key_to_js_string(&key)
                    .unwrap()
                    .to_utf8_lossy()
            };
            implemented.contains(&name.as_str()).then_some((key, name))
        })
        .collect::<Vec<_>>();
    let key_text = implemented_keys
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let descriptor_text = implemented_keys
        .iter()
        .map(|(key, name)| {
            format_descriptor(
                &runtime,
                name,
                runtime
                    .get_own_property(&harness.function_prototype, key)
                    .unwrap()
                    .as_ref()
                    .expect("implemented Function.prototype key disappeared"),
            )
        })
        .collect::<Vec<_>>()
        .join("|");

    let bind_meta = builtin_meta(&runtime, harness.bind.as_object(), "bind");
    let to_string_meta = builtin_meta(&runtime, harness.to_string.as_object(), "toString");

    let anonymous_source = "function  ( a , b ) { return a + b; }";
    let anonymous = harness.function(&format!("(0, {anonymous_source})"));
    let named_source = "function named /*kept*/ (value) {\n  return value;\n}";
    let named = harness.function(&format!("(0, {named_source})"));
    let observed_anonymous =
        harness.function_to_string(Value::Object(anonymous.as_object().clone()));
    let observed_named = harness.function_to_string(Value::Object(named.as_object().clone()));
    define_value_only(
        &runtime,
        anonymous.as_object(),
        "name",
        Value::String(JsString::try_from_utf8("changed anonymous").unwrap()),
    );
    define_value_only(
        &runtime,
        named.as_object(),
        "name",
        Value::String(JsString::try_from_utf8("changed named").unwrap()),
    );
    let renamed_anonymous =
        harness.function_to_string(Value::Object(anonymous.as_object().clone()));
    let renamed_named = harness.function_to_string(Value::Object(named.as_object().clone()));

    let native_target = harness.function("(function nativeTarget(a){})");
    let bound_native_target = harness.bind(&native_target, Value::Null, &[]);
    let call = property_callable(
        &runtime,
        &mut harness.context,
        &harness.function_prototype,
        "call",
    );
    let apply = property_callable(
        &runtime,
        &mut harness.context,
        &harness.function_prototype,
        "apply",
    );
    let fp_template = harness.function_to_string(Value::Object(harness.function_prototype.clone()));
    let call_template = harness.function_to_string(Value::Object(call.as_object().clone()));
    let apply_template = harness.function_to_string(Value::Object(apply.as_object().clone()));
    let bind_template = harness.function_to_string(Value::Object(harness.bind.as_object().clone()));
    let to_string_template =
        harness.function_to_string(Value::Object(harness.to_string.as_object().clone()));
    let bound_template =
        harness.function_to_string(Value::Object(bound_native_target.as_object().clone()));

    let invalid_object = harness.context.new_object().unwrap();
    let invalid_object_result = harness.function_to_string_result(Value::Object(invalid_object));
    let invalid_object = observe_result(&runtime, &mut harness.context, invalid_object_result);
    let invalid_number_result = harness.function_to_string_result(Value::Int(1));
    let invalid_number = observe_result(&runtime, &mut harness.context, invalid_number_result);

    define_value_only(&runtime, call.as_object(), "name", Value::Int(17));
    let number_name = harness.function_to_string(Value::Object(call.as_object().clone()));

    let native_name_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("native-name").unwrap()))
        .unwrap();
    define_value_only(
        &runtime,
        apply.as_object(),
        "name",
        Value::Symbol(native_name_symbol),
    );
    let symbol_name_result =
        harness.function_to_string_result(Value::Object(apply.as_object().clone()));
    let symbol_name = observe_result(&runtime, &mut harness.context, symbol_name_result);

    let native_name_sentinel = harness.context.new_object().unwrap();
    define_global(
        &runtime,
        &mut harness.context,
        "nativeNameSentinel",
        Value::Object(native_name_sentinel.clone()),
    );
    let native_name_getter = harness.function("(function(){ throw nativeNameSentinel; })");
    define_getter(
        &runtime,
        harness.bind.as_object(),
        "name",
        native_name_getter,
    );
    let native_name_result =
        harness.function_to_string_result(Value::Object(harness.bind.as_object().clone()));
    let native_name_identity = thrown_identity(
        &mut harness.context,
        native_name_result,
        Value::Object(native_name_sentinel),
    );

    vec![
        format!("fp-keys={key_text}"),
        format!("fp-desc={descriptor_text}"),
        format!("builtin-meta={bind_meta}|{to_string_meta}"),
        format!(
            "source-anonymous={}|renamed:{}",
            hex(&observed_anonymous),
            hex(&renamed_anonymous)
        ),
        format!(
            "source-named={}|renamed:{}",
            hex(&observed_named),
            hex(&renamed_named)
        ),
        format!(
            "native-template=prototype:{}|call:{}|apply:{}|bind:{}|toString:{}|bound:{}",
            hex(&fp_template),
            hex(&call_template),
            hex(&apply_template),
            hex(&bind_template),
            hex(&to_string_template),
            hex(&bound_template),
        ),
        format!("invalid-object={invalid_object}"),
        format!("invalid-number={invalid_number}"),
        format!("native-number-name={}", hex(&number_name)),
        format!("native-symbol-name={symbol_name}"),
        format!("native-getter-throw={native_name_identity}"),
    ]
}

fn rust_bind_metadata_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let runtime = harness.runtime.clone();

    let length_cases = [
        ("int", Value::Int(5), 2),
        ("fraction", Value::Float(5.9), 2),
        ("infinity", Value::Float(f64::INFINITY), 1),
        ("negative-infinity", Value::Float(f64::NEG_INFINITY), 1),
        ("nan", Value::Float(f64::NAN), 1),
        (
            "string",
            Value::String(JsString::try_from_utf8("5").unwrap()),
            1,
        ),
        (
            "bigint",
            Value::BigInt(quickjs_oxide::JsBigInt::from(5_i32)),
            1,
        ),
        (
            "symbol",
            Value::Symbol(
                runtime
                    .new_symbol(Some(JsString::try_from_utf8("length").unwrap()))
                    .unwrap(),
            ),
            1,
        ),
    ];
    let mut length_parts = Vec::new();
    for (label, length, count) in length_cases {
        let target = harness.function("(function lengthTarget(a,b,c,d,e,f){})");
        define_value_only(&runtime, target.as_object(), "length", length);
        let arguments = vec![Value::Int(1); count];
        let bound = harness.bind(&target, Value::Null, &arguments);
        length_parts.push(format!(
            "{label}:{}",
            show_value(get_property(
                &runtime,
                &mut harness.context,
                bound.as_object(),
                "length"
            ))
        ));
    }
    let absent_target = harness.function("(function lengthTarget(){})");
    delete_property(&runtime, absent_target.as_object(), "length");
    let absent_bound = harness.bind(&absent_target, Value::Null, &[Value::Int(1)]);
    length_parts.push(format!(
        "absent:{}",
        show_value(get_property(
            &runtime,
            &mut harness.context,
            absent_bound.as_object(),
            "length"
        ))
    ));

    let name_cases = [
        (
            "string",
            Value::String(JsString::try_from_utf8("renamed").unwrap()),
        ),
        ("number", Value::Int(17)),
        (
            "symbol",
            Value::Symbol(
                runtime
                    .new_symbol(Some(JsString::try_from_utf8("name").unwrap()))
                    .unwrap(),
            ),
        ),
        ("undefined", Value::Undefined),
    ];
    let mut name_parts = Vec::new();
    let mut first_named_bound = None;
    for (label, name) in name_cases {
        let target = harness.function("(function nameTarget(){})");
        define_value_only(&runtime, target.as_object(), "name", name);
        let bound = harness.bind(&target, Value::Null, &[]);
        let Value::String(name) =
            get_property(&runtime, &mut harness.context, bound.as_object(), "name")
        else {
            panic!("bound name was not a string");
        };
        name_parts.push(format!("{label}:{}", name.to_utf8_lossy()));
        if label == "string" {
            first_named_bound = Some(bound);
        }
    }
    let name_descriptor = data_descriptor(
        &runtime,
        first_named_bound
            .as_ref()
            .expect("missing named bound")
            .as_object(),
        "name",
    );
    let CompleteOrdinaryPropertyDescriptor::Data {
        writable,
        enumerable,
        configurable,
        ..
    } = name_descriptor
    else {
        panic!("bound name was not a data property");
    };
    name_parts.push(format!(
        "desc:{},{},{}",
        bit(writable),
        bit(enumerable),
        bit(configurable)
    ));

    define_global(&runtime, &mut harness.context, "getterOrder", Value::Int(0));
    let ordered_target = harness.function("(function orderedTarget(a,b,c){})");
    let length_getter =
        harness.function("(function(){ getterOrder = getterOrder * 10 + 1; return 3; })");
    let name_getter =
        harness.function("(function(){ getterOrder = getterOrder * 10 + 2; return \"ordered\"; })");
    define_getter(
        &runtime,
        ordered_target.as_object(),
        "length",
        length_getter,
    );
    define_getter(&runtime, ordered_target.as_object(), "name", name_getter);
    let ordered_bound = harness.bind(&ordered_target, Value::Null, &[Value::Int(1)]);
    let ordered_length = get_property(
        &runtime,
        &mut harness.context,
        ordered_bound.as_object(),
        "length",
    );
    let ordered_name = get_property(
        &runtime,
        &mut harness.context,
        ordered_bound.as_object(),
        "name",
    );
    let getter_order = global_value(&runtime, &mut harness.context, "getterOrder");

    let length_throw_sentinel = harness.context.new_object().unwrap();
    define_global(
        &runtime,
        &mut harness.context,
        "lengthThrowSentinel",
        Value::Object(length_throw_sentinel.clone()),
    );
    define_global(
        &runtime,
        &mut harness.context,
        "lengthThrowLateName",
        Value::Int(0),
    );
    let length_throw_target = harness.function("(function lengthThrowTarget(){})");
    let length_throw_getter = harness.function("(function(){ throw lengthThrowSentinel; })");
    let late_name_getter = harness.function(
        "(function(){ lengthThrowLateName = lengthThrowLateName + 1; return \"late\"; })",
    );
    define_getter(
        &runtime,
        length_throw_target.as_object(),
        "length",
        length_throw_getter,
    );
    define_getter(
        &runtime,
        length_throw_target.as_object(),
        "name",
        late_name_getter,
    );
    let length_throw_result = harness.bind_result(&length_throw_target, Value::Null, &[]);
    let length_throw_identity = thrown_identity(
        &mut harness.context,
        length_throw_result,
        Value::Object(length_throw_sentinel),
    );
    let late_name = global_value(&runtime, &mut harness.context, "lengthThrowLateName");

    let name_throw_sentinel = harness.context.new_object().unwrap();
    define_global(
        &runtime,
        &mut harness.context,
        "nameThrowSentinel",
        Value::Object(name_throw_sentinel.clone()),
    );
    define_global(
        &runtime,
        &mut harness.context,
        "nameThrowOrder",
        Value::Int(0),
    );
    let name_throw_target = harness.function("(function nameThrowTarget(){})");
    let ordered_length_getter =
        harness.function("(function(){ nameThrowOrder = nameThrowOrder * 10 + 1; return 2; })");
    let throwing_name_getter = harness.function(
        "(function(){ nameThrowOrder = nameThrowOrder * 10 + 2; throw nameThrowSentinel; })",
    );
    define_getter(
        &runtime,
        name_throw_target.as_object(),
        "length",
        ordered_length_getter,
    );
    define_getter(
        &runtime,
        name_throw_target.as_object(),
        "name",
        throwing_name_getter,
    );
    let name_throw_result = harness.bind_result(&name_throw_target, Value::Null, &[]);
    let name_throw_identity = thrown_identity(
        &mut harness.context,
        name_throw_result,
        Value::Object(name_throw_sentinel),
    );
    let name_throw_order = global_value(&runtime, &mut harness.context, "nameThrowOrder");

    define_global(
        &runtime,
        &mut harness.context,
        "inheritedLengthReads",
        Value::Int(0),
    );
    let inherited_length_getter = harness
        .function("(function(){ inheritedLengthReads = inheritedLengthReads + 1; return 99; })");
    let inherited_prototype = harness
        .context
        .new_object_with_prototype(Some(&harness.function_prototype))
        .unwrap();
    define_getter(
        &runtime,
        &inherited_prototype,
        "length",
        inherited_length_getter,
    );
    let inherited_target = harness.function("(function inheritedLengthTarget(){})");
    delete_property(&runtime, inherited_target.as_object(), "length");
    assert!(
        runtime
            .set_prototype_of(inherited_target.as_object(), Some(&inherited_prototype))
            .unwrap()
    );
    let inherited_bound = harness.bind(&inherited_target, Value::Null, &[Value::Int(1)]);
    let inherited_reads = global_value(&runtime, &mut harness.context, "inheritedLengthReads");
    let inherited_bound_length = get_property(
        &runtime,
        &mut harness.context,
        inherited_bound.as_object(),
        "length",
    );

    vec![
        format!("bind-length={}", length_parts.join("|")),
        format!("bind-name={}", name_parts.join("|")),
        format!(
            "bind-getter-order={}|length:{}|name:{}",
            show_value(getter_order),
            show_value(ordered_length),
            plain_string(ordered_name),
        ),
        format!(
            "bind-length-throw={length_throw_identity}|late-name:{}",
            show_value(late_name)
        ),
        format!(
            "bind-name-throw={name_throw_identity}|order:{}",
            show_value(name_throw_order)
        ),
        format!(
            "bind-inherited-length=reads:{}|length:{}",
            show_value(inherited_reads),
            show_value(inherited_bound_length)
        ),
    ]
}

fn rust_bound_execution_observations() -> Vec<String> {
    let mut harness = Harness::new();
    let runtime = harness.runtime.clone();

    let call_target = harness
        .function("(function callTarget(a,b,c){ \"use strict\"; return a * 100 + b * 10 + c; })");
    let receiver = harness.context.new_object().unwrap();
    define_data(&runtime, &receiver, "base", Value::Int(4), true, true, true);
    let ignored_receiver = harness.context.new_object().unwrap();
    define_data(
        &runtime,
        &ignored_receiver,
        "base",
        Value::Int(9),
        true,
        true,
        true,
    );
    let simple_bound = harness.bind(
        &call_target,
        Value::Object(receiver.clone()),
        &[Value::Int(1)],
    );
    let simple = harness
        .context
        .call(
            &simple_bound,
            Value::Undefined,
            &[Value::Int(2), Value::Int(3)],
        )
        .unwrap();
    let first_bound = harness.bind(&call_target, Value::Object(receiver), &[Value::Int(1)]);
    let nested_bound = harness.bind(
        &first_bound,
        Value::Object(ignored_receiver),
        &[Value::Int(2)],
    );
    let nested = harness
        .context
        .call(&nested_bound, Value::Undefined, &[Value::Int(3)])
        .unwrap();
    let nested_name = get_property(
        &runtime,
        &mut harness.context,
        nested_bound.as_object(),
        "name",
    );
    let nested_length = get_property(
        &runtime,
        &mut harness.context,
        nested_bound.as_object(),
        "length",
    );

    let construct_target = harness.function("(function ConstructTarget(){ return new.target; })");
    let ignored_bound_this = harness.context.new_object().unwrap();
    let construct_bound = harness.bind(&construct_target, Value::Object(ignored_bound_this), &[]);
    let constructed = harness.context.construct(&construct_bound, &[]).unwrap();
    let new_target_target = constructed == Value::Object(construct_target.as_object().clone());
    let bound_own_prototype = runtime
        .has_own_property(
            construct_bound.as_object(),
            &runtime.intern_property_key("prototype").unwrap(),
        )
        .unwrap();

    let other_target = harness.function("(function OtherTarget(){})");
    let explicitly_constructed = harness
        .context
        .construct_with_new_target(&construct_bound, &other_target, &[])
        .unwrap();
    let explicit_new_target = explicitly_constructed == Value::Object(other_target.into_object());

    let construct_arg_marker = harness.context.new_object().unwrap();
    let construct_arg_target =
        harness.function("(function ConstructArgTarget(value){ return value; })");
    let construct_arg_bound = harness.bind(
        &construct_arg_target,
        Value::Null,
        &[Value::Object(construct_arg_marker.clone())],
    );
    let constructed_arg = harness
        .context
        .construct(&construct_arg_bound, &[])
        .unwrap();
    let construct_arg = constructed_arg == Value::Object(construct_arg_marker);

    let instance_marker = harness.context.new_object().unwrap();
    let instance_other = harness.context.new_object().unwrap();
    define_global(
        &runtime,
        &mut harness.context,
        "instanceMarker",
        Value::Object(instance_marker.clone()),
    );
    let instance_target = harness.function("(function InstanceTarget(){})");
    let custom_has_instance =
        harness.function("(function(candidate){ return candidate === instanceMarker; })");
    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    define_data_key(
        &runtime,
        instance_target.as_object(),
        &has_instance_key,
        Value::Object(custom_has_instance.as_object().clone()),
        true,
        false,
        true,
    );
    let instance_bound = harness.bind(&instance_target, Value::Null, &[]);
    let has_instance = property_callable_key(
        &runtime,
        &mut harness.context,
        &harness.function_prototype,
        &has_instance_key,
    );
    let custom_true = harness
        .context
        .call(
            &has_instance,
            Value::Object(instance_bound.as_object().clone()),
            &[Value::Object(instance_marker.clone())],
        )
        .unwrap();
    let custom_false = harness
        .context
        .call(
            &has_instance,
            Value::Object(instance_bound.as_object().clone()),
            &[Value::Object(instance_other)],
        )
        .unwrap();
    let instance_throw_sentinel = harness.context.new_object().unwrap();
    define_global(
        &runtime,
        &mut harness.context,
        "instanceThrowSentinel",
        Value::Object(instance_throw_sentinel.clone()),
    );
    let throwing_has_instance = harness.function("(function(){ throw instanceThrowSentinel; })");
    define_value_only_key(
        &runtime,
        instance_target.as_object(),
        &has_instance_key,
        Value::Object(throwing_has_instance.as_object().clone()),
    );
    let instance_throw_result = harness.context.call(
        &has_instance,
        Value::Object(instance_bound.as_object().clone()),
        &[Value::Object(instance_marker)],
    );
    let instance_throw_identity = thrown_identity(
        &mut harness.context,
        instance_throw_result,
        Value::Object(instance_throw_sentinel),
    );

    let mut deep_instance_bound = harness.function("(function DeepInstanceTarget(){})");
    for _ in 0..512 {
        deep_instance_bound = harness.bind(&deep_instance_bound, Value::Null, &[]);
    }
    let deep_instance_result = harness
        .context
        .call(
            &has_instance,
            Value::Object(deep_instance_bound.into_object()),
            &[Value::Int(1)],
        )
        .unwrap();

    let cross_runtime = Runtime::new();
    let mut target_realm = cross_runtime.new_context();
    let mut bind_realm = cross_runtime.new_context();
    let cross_target = function(
        &cross_runtime,
        &mut target_realm,
        "(function CrossRealmTarget(){})",
    );
    let target_function_prototype = cross_runtime
        .get_prototype_of(cross_target.as_object())
        .unwrap()
        .expect("cross-realm target function had no prototype");
    let bind_realm_function_prototype = bind_realm.function_prototype().unwrap();
    assert_ne!(target_function_prototype, bind_realm_function_prototype);
    let cross_bind = property_callable(
        &cross_runtime,
        &mut bind_realm,
        &bind_realm_function_prototype,
        "bind",
    );
    let Value::Object(cross_bound) = bind_realm
        .call(
            &cross_bind,
            Value::Object(cross_target.as_object().clone()),
            &[Value::Null],
        )
        .unwrap()
    else {
        panic!("cross-realm bind did not return an object");
    };
    let cross_realm_prototype = cross_runtime.get_prototype_of(&cross_bound).unwrap()
        == Some(bind_realm_function_prototype);

    vec![
        format!(
            "bound-call=simple:{}|nested:{}|nested-name:{}|nested-length:{}",
            show_value(simple),
            show_value(nested),
            plain_string(nested_name),
            show_value(nested_length),
        ),
        format!(
            "bound-construct=arg:{construct_arg}|new-target-target:{new_target_target}|bound-own-prototype:{bound_own_prototype}|explicit-new-target:{explicit_new_target}",
        ),
        format!(
            "bound-has-instance=true:{}|false:{}|throw:{instance_throw_identity}",
            plain_bool(custom_true),
            plain_bool(custom_false),
        ),
        format!(
            "deep-bound-has-instance={}",
            plain_bool(deep_instance_result)
        ),
        format!("cross-realm-prototype={cross_realm_prototype}"),
    ]
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    property_callable_key(runtime, context, object, &key)
}

fn property_callable_key(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    key: &PropertyKey,
) -> CallableRef {
    let Value::Object(value) = context.get_property(object, key).unwrap() else {
        panic!("callable property was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .expect("callable property was not callable")
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

fn get_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> Value {
    let key = runtime.intern_property_key(name).unwrap();
    context.get_property(object, &key).unwrap()
}

fn global_value(runtime: &Runtime, context: &mut Context, name: &str) -> Value {
    let global = context.global_object().unwrap();
    get_property(runtime, context, &global, name)
}

fn define_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    define_data(runtime, &global, name, value, true, true, true);
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
            .unwrap()
    );
}

fn define_value_only(runtime: &Runtime, object: &ObjectRef, name: &str, value: Value) {
    let key = runtime.intern_property_key(name).unwrap();
    define_value_only_key(runtime, object, &key, value);
}

fn define_value_only_key(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey, value: Value) {
    assert!(
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
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn delete_property(runtime: &Runtime, object: &ObjectRef, name: &str) {
    let key = runtime.intern_property_key(name).unwrap();
    assert!(runtime.delete_property(object, &key).unwrap());
}

fn data_descriptor(
    runtime: &Runtime,
    object: &ObjectRef,
    name: &str,
) -> CompleteOrdinaryPropertyDescriptor {
    let key = runtime.intern_property_key(name).unwrap();
    runtime
        .get_own_property(object, &key)
        .unwrap()
        .expect("data descriptor property was absent")
}

fn format_descriptor(
    runtime: &Runtime,
    name: &str,
    descriptor: &CompleteOrdinaryPropertyDescriptor,
) -> String {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => format!(
            "{name}:data:{}:{},{},{}",
            value_type(runtime, value),
            bit(*writable),
            bit(*enumerable),
            bit(*configurable)
        ),
        CompleteOrdinaryPropertyDescriptor::Accessor {
            enumerable,
            configurable,
            ..
        } => format!(
            "{name}:accessor:{},{}",
            bit(*enumerable),
            bit(*configurable)
        ),
    }
}

fn builtin_meta(runtime: &Runtime, object: &ObjectRef, label: &str) -> String {
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: length,
        writable: length_writable,
        enumerable: length_enumerable,
        configurable: length_configurable,
    } = data_descriptor(runtime, object, "length")
    else {
        panic!("builtin length was not data");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::String(name),
        writable: name_writable,
        enumerable: name_enumerable,
        configurable: name_configurable,
    } = data_descriptor(runtime, object, "name")
    else {
        panic!("builtin name was not string data");
    };
    format!(
        "{label}-length:{}:{},{},{}|{label}-name:{}:{},{},{}",
        show_value(length),
        bit(length_writable),
        bit(length_enumerable),
        bit(length_configurable),
        name.to_utf8_lossy(),
        bit(name_writable),
        bit(name_enumerable),
        bit(name_configurable),
    )
}

fn observe_result(
    runtime: &Runtime,
    context: &mut Context,
    result: Result<Value, RuntimeError>,
) -> String {
    match result {
        Ok(value) => show_value(value),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap()
                .expect("exception completion had no value");
            show_exception(runtime, context, exception)
        }
        Err(error) => panic!("probe failed with an engine error: {error}"),
    }
}

fn thrown_identity(
    context: &mut Context,
    result: Result<Value, RuntimeError>,
    expected: Value,
) -> bool {
    assert_eq!(result, Err(RuntimeError::Exception));
    context.take_exception().unwrap() == Some(expected)
}

fn show_exception(runtime: &Runtime, context: &mut Context, exception: Value) -> String {
    match exception {
        Value::String(value) => format!("throw-string:{}", value.to_utf8_lossy()),
        Value::Object(error) if runtime.is_error_object(&error).unwrap() => {
            let Value::String(name) = get_property(runtime, context, &error, "name") else {
                panic!("error name was not a string");
            };
            let Value::String(message) = get_property(runtime, context, &error, "message") else {
                panic!("error message was not a string");
            };
            format!("throw:{}|{}", name.to_utf8_lossy(), message.to_utf8_lossy())
        }
        value => panic!("unexpected thrown probe value: {value:?}"),
    }
}

fn show_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) if value.is_nan() => "NaN".to_owned(),
        Value::Float(value) if value == f64::INFINITY => "Infinity".to_owned(),
        Value::Float(value) if value == f64::NEG_INFINITY => "-Infinity".to_owned(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => format!("string:{}", value.to_utf8_lossy()),
        value => panic!("unexpected probe value: {value:?}"),
    }
}

fn plain_string(value: Value) -> String {
    let Value::String(value) = value else {
        panic!("expected a string, got {value:?}");
    };
    value.to_utf8_lossy()
}

fn plain_bool(value: Value) -> bool {
    let Value::Bool(value) = value else {
        panic!("expected a boolean, got {value:?}");
    };
    value
}

fn value_type(runtime: &Runtime, value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Object(object) if runtime.as_callable(object).unwrap().is_some() => "function",
        Value::Null | Value::Object(_) => "object",
        Value::Bool(_) => "boolean",
        Value::Int(_) | Value::Float(_) => "number",
        Value::BigInt(_) => "bigint",
        Value::String(_) => "string",
        Value::Symbol(_) => "symbol",
    }
}

fn hex(value: &JsString) -> String {
    value
        .utf16_units()
        .map(|unit| format!("{unit:04x}"))
        .collect()
}

const fn bit(value: bool) -> u8 {
    value as u8
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS Function.prototype bind/toString oracle");
    assert!(
        output.status.success(),
        "QuickJS Function.prototype bind/toString oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Function.prototype bind/toString oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
