use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DebugInfoMode,
    DescriptorField, JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime,
    RuntimeError, Value, WellKnownSymbol,
};

const ORACLE_PROBE: &str = r#"
function hex(value) {
    var out = "";
    for (var i = 0; i < value.length; i++)
        out += value.charCodeAt(i).toString(16).padStart(4, "0");
    return out;
}
function bits(descriptor) {
    return Number(descriptor.writable) + "," +
           Number(descriptor.enumerable) + "," +
           Number(descriptor.configurable);
}
function text(value) {
    return value === undefined ? "undefined" : String(value);
}
function isConstructor(value) {
    try {
        Reflect.construct(function () {}, [], value);
        return true;
    } catch (_) {
        return false;
    }
}
function firstTwoFrames(stack) {
    return String(stack).split("\n").slice(0, 2).join("|");
}
var fp = Function.prototype;
var objectPrototype = Object.prototype;
var lengthDescriptor = Object.getOwnPropertyDescriptor(Function, "length");
var nameDescriptor = Object.getOwnPropertyDescriptor(Function, "name");
var prototypeDescriptor = Object.getOwnPropertyDescriptor(Function, "prototype");
var globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "Function");
var constructorDescriptor = Object.getOwnPropertyDescriptor(fp, "constructor");
print("ctor-keys=" + Reflect.ownKeys(Function).map(String).join(","));
print("ctor-meta=" + Function.length + "|" + Function.name + "|" +
      bits(lengthDescriptor) + "|" + bits(nameDescriptor) + "|" +
      (prototypeDescriptor.value === fp) + "|" + bits(prototypeDescriptor));
print("global=" + (globalDescriptor.value === Function) + "|" + bits(globalDescriptor));
var implementedGlobals = ["Error", "EvalError", "RangeError", "ReferenceError",
                          "SyntaxError", "TypeError", "URIError", "InternalError", "Function"];
print("global-order=" + Reflect.ownKeys(globalThis).filter(function (key) {
    return implementedGlobals.indexOf(key) >= 0;
}).join(","));
print("fp-keys=" + Reflect.ownKeys(fp).map(String).join(","));
print("fp-constructor=" + (constructorDescriptor.value === Function) + "|" +
      bits(constructorDescriptor));
print("graph=" + [
    Object.getPrototypeOf(Function) === fp,
    Object.getPrototypeOf(fp) === objectPrototype,
    isConstructor(Function),
    Function instanceof Function,
    fp instanceof Function
].join(","));
print("native-source=" + hex(fp.toString.call(Function)));

var empty = Function();
print("empty=" + empty.name + "|" + empty.length + "|" + text(empty()) + "|" +
      hex(fp.toString.call(empty)) + "|" + text(empty.fileName) + "|" +
      text(empty.lineNumber) + "|" + text(empty.columnNumber) + "|" +
      (Object.getPrototypeOf(empty) === fp));

var add = Function("a", "b", "return a + b");
print("multi=" + add.name + "|" + add.length + "|" + add(20, 22) + "|" +
      hex(fp.toString.call(add)) + "|" + text(add.fileName) + "|" +
      text(add.lineNumber) + "|" + text(add.columnNumber));
var power = Function("a", "b", "return a ** b");
print("power=" + power(2, 10));

var viaNew = new Function("return 9");
print("function-new=" + viaNew() + "|" + (Object.getPrototypeOf(viaNew) === fp));
var maker = Function("return 1");
var instance = new maker();
print("dynamic-new=" + typeof instance + "|" +
      (Object.getPrototypeOf(instance) === maker.prototype));

var NewTarget = Function("");
var customPrototype = {};
NewTarget.prototype = customPrototype;
var custom = Reflect.construct(Function, ["return 5"], NewTarget);
var customResult = (Object.getPrototypeOf(custom) === customPrototype) + "|" + custom();
NewTarget.prototype = 1;
var fallback = Reflect.construct(Function, ["return 6"], NewTarget);
print("new-target=" + customResult + "|" +
      (Object.getPrototypeOf(fallback) === fp) + "|" + fallback());

var duplicate = Function("a", "a", "return a");
print("sloppy-duplicate=" + duplicate.length + "|" + duplicate(1, 2));

var conversionLog = "";
var conversionCustomPrototype = {};
var conversionParameter = {
    toString: function () { conversionLog += "p"; return "a"; }
};
var conversionBody = {
    toString: function () { conversionLog += "b"; return "return a"; }
};
var ConversionNewTarget = Function.bind(null);
Object.defineProperty(ConversionNewTarget, "prototype", {
    get: function () { conversionLog += "x"; return conversionCustomPrototype; },
    configurable: true
});
var converted = Reflect.construct(
    Function, [conversionParameter, conversionBody], ConversionNewTarget);
print("conversion-success=" + conversionLog + "|" + converted(7));
conversionLog = "";
conversionParameter.toString = function () { conversionLog += "p"; return "a-"; };
try {
    Reflect.construct(Function, [conversionParameter, conversionBody], ConversionNewTarget);
    print("conversion-parse=missing");
} catch (error) {
    print("conversion-parse=" + conversionLog + "|" + error.name);
}
conversionLog = "";
conversionParameter.toString = function () { conversionLog += "t"; throw "stop"; };
try {
    Reflect.construct(Function, [conversionParameter, conversionBody], ConversionNewTarget);
    print("conversion-throw=missing");
} catch (error) {
    print("conversion-throw=" + conversionLog + "|" + error);
}

try {
    Function("a", "a", "\"use strict\"; return a");
    print("strict-error=missing");
} catch (error) {
    print("strict-error=" + error.name + ":" + error.message + "|" + text(error.fileName) + ":" +
          text(error.lineNumber) + ":" + text(error.columnNumber) + "|" +
          firstTwoFrames(error.stack));
}
try {
    Function("a-", "return 1");
    print("parameter-error=missing");
} catch (error) {
    print("parameter-error=" + error.name + ":" + error.message + "|" + text(error.fileName) + ":" +
          text(error.lineNumber) + ":" + text(error.columnNumber) + "|" +
          firstTwoFrames(error.stack));
}
try {
    Function("a", "return )");
    print("body-error=missing");
} catch (error) {
    print("body-error=" + error.name + ":" + error.message + "|" + text(error.fileName) + ":" +
          text(error.lineNumber) + ":" + text(error.columnNumber) + "|" +
          firstTwoFrames(error.stack));
}
try {
    Function("null", "return 1");
    print("formal-error=missing");
} catch (error) {
    print("formal-error=" + error.name + ":" + error.message + "|" + text(error.fileName) + ":" +
          text(error.lineNumber) + ":" + text(error.columnNumber) + "|" +
          firstTwoFrames(error.stack));
}
"#;

#[test]
fn function_constructor_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Function constructor differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (mode, oracle_flag) in [
        (DebugInfoMode::Full, None),
        (DebugInfoMode::StripSource, Some("--strip-source")),
        (DebugInfoMode::StripDebug, Some("-s")),
    ] {
        assert_eq!(
            rust_observations(mode),
            oracle_observations(&oracle, oracle_flag),
            "Function constructor debug mode {mode:?} differed from pinned QuickJS"
        );
    }
}

fn rust_observations(mode: DebugInfoMode) -> Vec<String> {
    let runtime = Runtime::new();
    runtime.set_debug_info_mode(mode);
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let global = context.global_object().unwrap();
    let mut output = Vec::new();

    let length_descriptor = own_data(&runtime, constructor.as_object(), "length");
    let name_descriptor = own_data(&runtime, constructor.as_object(), "name");
    let prototype_descriptor = own_data(&runtime, constructor.as_object(), "prototype");
    let global_descriptor = own_data(&runtime, &global, "Function");
    let fp_constructor_descriptor = own_data(&runtime, &function_prototype, "constructor");

    output.push(format!(
        "ctor-keys={}",
        own_key_names(&runtime, constructor.as_object())
    ));
    output.push(format!(
        "ctor-meta={}|{}|{}|{}|{}|{}",
        value_text(length_descriptor.0.clone()),
        value_text(name_descriptor.0.clone()),
        descriptor_bits(&length_descriptor),
        descriptor_bits(&name_descriptor),
        matches!(&prototype_descriptor.0, Value::Object(value) if value == &function_prototype),
        descriptor_bits(&prototype_descriptor),
    ));
    output.push(format!(
        "global={}|{}",
        matches!(&global_descriptor.0, Value::Object(value) if value == constructor.as_object()),
        descriptor_bits(&global_descriptor),
    ));
    let implemented_globals = [
        "Error",
        "EvalError",
        "RangeError",
        "ReferenceError",
        "SyntaxError",
        "TypeError",
        "URIError",
        "InternalError",
        "Function",
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
    output.push(format!("global-order={global_order}"));
    output.push(format!(
        "fp-keys={}",
        own_key_names(&runtime, &function_prototype)
    ));
    output.push(format!(
        "fp-constructor={}|{}",
        matches!(&fp_constructor_descriptor.0, Value::Object(value) if value == constructor.as_object()),
        descriptor_bits(&fp_constructor_descriptor),
    ));

    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let has_instance = property_callable_by_key(
        &runtime,
        &mut context,
        &function_prototype,
        &has_instance_key,
    );
    let function_instanceof_function = expect_bool(
        context
            .call(
                &has_instance,
                Value::Object(constructor.as_object().clone()),
                &[Value::Object(constructor.as_object().clone())],
            )
            .unwrap(),
    );
    let prototype_instanceof_function = expect_bool(
        context
            .call(
                &has_instance,
                Value::Object(constructor.as_object().clone()),
                &[Value::Object(function_prototype.clone())],
            )
            .unwrap(),
    );
    output.push(format!(
        "graph={},{},{},{},{}",
        runtime.get_prototype_of(constructor.as_object()).unwrap()
            == Some(function_prototype.clone()),
        runtime.get_prototype_of(&function_prototype).unwrap() == Some(object_prototype),
        runtime.is_constructor(constructor.as_object()).unwrap(),
        function_instanceof_function,
        prototype_instanceof_function,
    ));

    let to_string = property_callable(&runtime, &mut context, &function_prototype, "toString");
    output.push(format!(
        "native-source={}",
        function_source_hex(&mut context, &to_string, constructor.as_object())
    ));

    let empty = expect_object(
        context.call(&constructor, Value::Null, &[]).unwrap(),
        "Function()",
    );
    let empty_callable = runtime.as_callable(&empty).unwrap().unwrap();
    let empty_return = context
        .call(&empty_callable, Value::Undefined, &[])
        .unwrap();
    output.push(format!(
        "empty={}|{}|{}|{}|{}|{}|{}|{}",
        property_text(&runtime, &mut context, &empty, "name"),
        property_text(&runtime, &mut context, &empty, "length"),
        value_text(empty_return),
        function_source_hex(&mut context, &to_string, &empty),
        property_text(&runtime, &mut context, &empty, "fileName"),
        property_text(&runtime, &mut context, &empty, "lineNumber"),
        property_text(&runtime, &mut context, &empty, "columnNumber"),
        runtime.get_prototype_of(&empty).unwrap() == Some(function_prototype.clone()),
    ));

    let add = call_function_constructor(
        &mut context,
        &constructor,
        &[string("a"), string("b"), string("return a + b")],
    );
    let add_callable = runtime.as_callable(&add).unwrap().unwrap();
    let add_result = context
        .call(
            &add_callable,
            Value::Undefined,
            &[Value::Int(20), Value::Int(22)],
        )
        .unwrap();
    output.push(format!(
        "multi={}|{}|{}|{}|{}|{}|{}",
        property_text(&runtime, &mut context, &add, "name"),
        property_text(&runtime, &mut context, &add, "length"),
        value_text(add_result),
        function_source_hex(&mut context, &to_string, &add),
        property_text(&runtime, &mut context, &add, "fileName"),
        property_text(&runtime, &mut context, &add, "lineNumber"),
        property_text(&runtime, &mut context, &add, "columnNumber"),
    ));

    let power = call_function_constructor(
        &mut context,
        &constructor,
        &[string("a"), string("b"), string("return a ** b")],
    );
    let power_callable = runtime.as_callable(&power).unwrap().unwrap();
    let power_result = context
        .call(
            &power_callable,
            Value::Undefined,
            &[Value::Int(2), Value::Int(10)],
        )
        .unwrap();
    output.push(format!("power={}", value_text(power_result)));

    let via_new = expect_object(
        context
            .construct(&constructor, &[string("return 9")])
            .unwrap(),
        "new Function",
    );
    let via_new_callable = runtime.as_callable(&via_new).unwrap().unwrap();
    let via_new_result = context
        .call(&via_new_callable, Value::Undefined, &[])
        .unwrap();
    output.push(format!(
        "function-new={}|{}",
        value_text(via_new_result),
        runtime.get_prototype_of(&via_new).unwrap() == Some(function_prototype.clone()),
    ));

    let maker = call_function_constructor(&mut context, &constructor, &[string("return 1")]);
    let maker_callable = runtime.as_callable(&maker).unwrap().unwrap();
    let maker_prototype = expect_object(
        property_value(&runtime, &mut context, &maker, "prototype"),
        "dynamic function prototype",
    );
    let instance = expect_object(
        context.construct(&maker_callable, &[]).unwrap(),
        "dynamic function instance",
    );
    output.push(format!(
        "dynamic-new=object|{}",
        runtime.get_prototype_of(&instance).unwrap() == Some(maker_prototype),
    ));

    let new_target_object = call_function_constructor(&mut context, &constructor, &[string("")]);
    let new_target = runtime.as_callable(&new_target_object).unwrap().unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let custom_prototype = context.new_object().unwrap();
    define_value(
        &mut context,
        new_target.as_object(),
        &prototype_key,
        Value::Object(custom_prototype.clone()),
    );
    let custom = expect_object(
        context
            .construct_with_new_target(&constructor, &new_target, &[string("return 5")])
            .unwrap(),
        "custom newTarget Function",
    );
    let custom_callable = runtime.as_callable(&custom).unwrap().unwrap();
    let custom_value = context
        .call(&custom_callable, Value::Undefined, &[])
        .unwrap();
    define_value(
        &mut context,
        new_target.as_object(),
        &prototype_key,
        Value::Int(1),
    );
    let fallback = expect_object(
        context
            .construct_with_new_target(&constructor, &new_target, &[string("return 6")])
            .unwrap(),
        "fallback newTarget Function",
    );
    let fallback_callable = runtime.as_callable(&fallback).unwrap().unwrap();
    let fallback_value = context
        .call(&fallback_callable, Value::Undefined, &[])
        .unwrap();
    output.push(format!(
        "new-target={}|{}|{}|{}",
        runtime.get_prototype_of(&custom).unwrap() == Some(custom_prototype),
        value_text(custom_value),
        runtime.get_prototype_of(&fallback).unwrap() == Some(function_prototype.clone()),
        value_text(fallback_value),
    ));

    let duplicate = call_function_constructor(
        &mut context,
        &constructor,
        &[string("a"), string("a"), string("return a")],
    );
    let duplicate_callable = runtime.as_callable(&duplicate).unwrap().unwrap();
    let duplicate_result = context
        .call(
            &duplicate_callable,
            Value::Undefined,
            &[Value::Int(1), Value::Int(2)],
        )
        .unwrap();
    output.push(format!(
        "sloppy-duplicate={}|{}",
        property_text(&runtime, &mut context, &duplicate, "length"),
        value_text(duplicate_result),
    ));

    output.extend(conversion_order_observations(
        &runtime,
        &mut context,
        &constructor,
        &function_prototype,
    ));

    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[string("a"), string("a"), string("\"use strict\"; return a"),],
        ),
        Err(RuntimeError::Exception)
    );
    let strict_error = expect_object(
        context.take_exception().unwrap().unwrap(),
        "strict duplicate SyntaxError",
    );
    let stack = property_value(&runtime, &mut context, &strict_error, "stack");
    let Value::String(stack) = stack else {
        panic!("strict duplicate SyntaxError stack was not a string");
    };
    let strict_stack = stack.to_utf8_lossy();
    let strict_frames = strict_stack.lines().collect::<Vec<_>>();
    assert_eq!(
        strict_frames.len(),
        2,
        "Rust strict stack grew extra frames"
    );
    let strict_frames = strict_frames.join("|");
    output.push(format!(
        "strict-error={}:{}|{}:{}:{}|{}",
        property_text(&runtime, &mut context, &strict_error, "name"),
        property_text(&runtime, &mut context, &strict_error, "message"),
        property_text(&runtime, &mut context, &strict_error, "fileName"),
        property_text(&runtime, &mut context, &strict_error, "lineNumber"),
        property_text(&runtime, &mut context, &strict_error, "columnNumber"),
        strict_frames,
    ));

    assert_eq!(
        context.call(
            &constructor,
            Value::Undefined,
            &[string("a-"), string("return 1")],
        ),
        Err(RuntimeError::Exception)
    );
    let parameter_error = expect_object(
        context.take_exception().unwrap().unwrap(),
        "malformed parameter SyntaxError",
    );
    let parameter_stack = property_value(&runtime, &mut context, &parameter_error, "stack");
    let Value::String(parameter_stack) = parameter_stack else {
        panic!("malformed parameter SyntaxError stack was not a string");
    };
    let parameter_stack = parameter_stack.to_utf8_lossy();
    let parameter_frames = parameter_stack.lines().collect::<Vec<_>>();
    assert_eq!(
        parameter_frames.len(),
        2,
        "Rust parameter stack grew extra frames"
    );
    let parameter_frames = parameter_frames.join("|");
    output.push(format!(
        "parameter-error={}:{}|{}:{}:{}|{}",
        property_text(&runtime, &mut context, &parameter_error, "name"),
        property_text(&runtime, &mut context, &parameter_error, "message"),
        property_text(&runtime, &mut context, &parameter_error, "fileName"),
        property_text(&runtime, &mut context, &parameter_error, "lineNumber"),
        property_text(&runtime, &mut context, &parameter_error, "columnNumber"),
        parameter_frames,
    ));

    observe_constructor_syntax_error(
        &runtime,
        &mut context,
        &constructor,
        "body-error",
        &[string("a"), string("return )")],
        &mut output,
    );
    observe_constructor_syntax_error(
        &runtime,
        &mut context,
        &constructor,
        "formal-error",
        &[string("null"), string("return 1")],
        &mut output,
    );

    output
}

fn conversion_order_observations(
    runtime: &Runtime,
    context: &mut Context,
    constructor: &CallableRef,
    function_prototype: &ObjectRef,
) -> Vec<String> {
    let global = context.global_object().unwrap();
    let order_key = runtime.intern_property_key("conversionLog").unwrap();
    let custom_key = runtime
        .intern_property_key("conversionCustomPrototype")
        .unwrap();
    let custom_prototype = context.new_object().unwrap();
    define_writable_value(
        context,
        &global,
        &order_key,
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    define_writable_value(
        context,
        &global,
        &custom_key,
        Value::Object(custom_prototype.clone()),
    );

    let (
        parameter_to_string,
        body_to_string,
        bad_parameter_to_string,
        throwing_to_string,
        prototype_getter,
    ) = {
        let mut eval_callable = |source: &str| {
            let function = expect_object(context.eval(source).unwrap(), "conversion helper");
            runtime.as_callable(&function).unwrap().unwrap()
        };
        (
            eval_callable("(function(){ conversionLog = conversionLog + \"p\"; return \"a\"; })"),
            eval_callable(
                "(function(){ conversionLog = conversionLog + \"b\"; return \"return a\"; })",
            ),
            eval_callable("(function(){ conversionLog = conversionLog + \"p\"; return \"a-\"; })"),
            eval_callable("(function(){ conversionLog = conversionLog + \"t\"; throw \"stop\"; })"),
            eval_callable(
                "(function(){ conversionLog = conversionLog + \"x\"; return conversionCustomPrototype; })",
            ),
        )
    };
    let parameter = context.new_object().unwrap();
    let body = context.new_object().unwrap();
    let to_string = runtime.intern_property_key("toString").unwrap();
    define_writable_value(
        context,
        &parameter,
        &to_string,
        Value::Object(parameter_to_string.as_object().clone()),
    );
    define_writable_value(
        context,
        &body,
        &to_string,
        Value::Object(body_to_string.as_object().clone()),
    );

    let bind = property_callable(runtime, context, function_prototype, "bind");
    let new_target = expect_object(
        context
            .call(
                &bind,
                Value::Object(constructor.as_object().clone()),
                &[Value::Undefined],
            )
            .unwrap(),
        "conversion newTarget",
    );
    let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert!(
        context
            .define_own_property(
                new_target.as_object(),
                &prototype,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(prototype_getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let converted = expect_object(
        context
            .construct_with_new_target(
                constructor,
                &new_target,
                &[
                    Value::Object(parameter.clone()),
                    Value::Object(body.clone()),
                ],
            )
            .unwrap(),
        "converted Function",
    );
    assert_eq!(
        runtime.get_prototype_of(&converted).unwrap(),
        Some(custom_prototype)
    );
    let converted = runtime.as_callable(&converted).unwrap().unwrap();
    let converted_value = context
        .call(&converted, Value::Undefined, &[Value::Int(7)])
        .unwrap();
    let mut output = vec![format!(
        "conversion-success={}|{}",
        property_text(runtime, context, &global, "conversionLog"),
        value_text(converted_value),
    )];

    define_value(
        context,
        &global,
        &order_key,
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    define_value(
        context,
        &parameter,
        &to_string,
        Value::Object(bad_parameter_to_string.as_object().clone()),
    );
    assert_eq!(
        context.construct_with_new_target(
            constructor,
            &new_target,
            &[
                Value::Object(parameter.clone()),
                Value::Object(body.clone())
            ],
        ),
        Err(RuntimeError::Exception)
    );
    let parse_error = expect_object(
        context.take_exception().unwrap().unwrap(),
        "conversion parse error",
    );
    output.push(format!(
        "conversion-parse={}|{}",
        property_text(runtime, context, &global, "conversionLog"),
        property_text(runtime, context, &parse_error, "name"),
    ));

    define_value(
        context,
        &global,
        &order_key,
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    define_value(
        context,
        &parameter,
        &to_string,
        Value::Object(throwing_to_string.as_object().clone()),
    );
    assert_eq!(
        context.construct_with_new_target(
            constructor,
            &new_target,
            &[Value::Object(parameter), Value::Object(body)],
        ),
        Err(RuntimeError::Exception)
    );
    output.push(format!(
        "conversion-throw={}|{}",
        property_text(runtime, context, &global, "conversionLog"),
        value_text(context.take_exception().unwrap().unwrap()),
    ));
    output
}

fn observe_constructor_syntax_error(
    runtime: &Runtime,
    context: &mut Context,
    constructor: &CallableRef,
    label: &str,
    arguments: &[Value],
    output: &mut Vec<String>,
) {
    assert_eq!(
        context.call(constructor, Value::Undefined, arguments),
        Err(RuntimeError::Exception)
    );
    let error = expect_object(
        context.take_exception().unwrap().unwrap(),
        "Function SyntaxError",
    );
    let Value::String(stack) = property_value(runtime, context, &error, "stack") else {
        panic!("Function SyntaxError stack was not a string");
    };
    let stack = stack.to_utf8_lossy();
    let frames = stack.lines().collect::<Vec<_>>();
    assert_eq!(frames.len(), 2, "Rust Function stack grew extra frames");
    let frames = frames.join("|");
    output.push(format!(
        "{label}={}:{}|{}:{}:{}|{}",
        property_text(runtime, context, &error, "name"),
        property_text(runtime, context, &error, "message"),
        property_text(runtime, context, &error, "fileName"),
        property_text(runtime, context, &error, "lineNumber"),
        property_text(runtime, context, &error, "columnNumber"),
        frames,
    ));
}

fn call_function_constructor(
    context: &mut Context,
    constructor: &CallableRef,
    arguments: &[Value],
) -> ObjectRef {
    expect_object(
        context
            .call(constructor, Value::Undefined, arguments)
            .unwrap(),
        "Function result",
    )
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    property_callable_by_key(runtime, context, object, &key)
}

fn property_callable_by_key(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    key: &PropertyKey,
) -> CallableRef {
    let Value::Object(value) = context.get_property(object, key).unwrap() else {
        panic!("callable property was not an object");
    };
    runtime.as_callable(&value).unwrap().unwrap()
}

fn property_value(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> Value {
    let key = runtime.intern_property_key(name).unwrap();
    context.get_property(object, &key).unwrap()
}

fn property_text(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    value_text(property_value(runtime, context, object, name))
}

fn own_data(runtime: &Runtime, object: &ObjectRef, name: &str) -> (Value, bool, bool, bool) {
    let key = runtime.intern_property_key(name).unwrap();
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        value,
        writable,
        enumerable,
        configurable,
    }) = runtime.get_own_property(object, &key).unwrap()
    else {
        panic!("{name} was not an own data property");
    };
    (value, writable, enumerable, configurable)
}

fn descriptor_bits(descriptor: &(Value, bool, bool, bool)) -> String {
    format!(
        "{},{},{}",
        bit(descriptor.1),
        bit(descriptor.2),
        bit(descriptor.3)
    )
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> String {
    let has_instance = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    runtime
        .own_property_keys(object)
        .unwrap()
        .iter()
        .map(|key| {
            if key == &has_instance {
                "Symbol(Symbol.hasInstance)".to_owned()
            } else {
                runtime
                    .property_key_to_js_string(key)
                    .unwrap()
                    .to_utf8_lossy()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn function_source_hex(
    context: &mut Context,
    to_string: &CallableRef,
    function: &ObjectRef,
) -> String {
    let Value::String(source) = context
        .call(to_string, Value::Object(function.clone()), &[])
        .unwrap()
    else {
        panic!("Function.prototype.toString did not return a string");
    };
    source
        .utf16_units()
        .map(|unit| format!("{unit:04x}"))
        .collect()
}

fn define_value(context: &mut Context, object: &ObjectRef, key: &PropertyKey, value: Value) {
    assert!(
        context
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

fn define_writable_value(
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
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn expect_object(value: Value, label: &str) -> ObjectRef {
    let Value::Object(value) = value else {
        panic!("{label} was not an object");
    };
    value
}

fn expect_bool(value: Value) -> bool {
    let Value::Bool(value) = value else {
        panic!("expected a boolean observation");
    };
    value
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
        Value::Object(_) | Value::Symbol(_) => panic!("unexpected observation value"),
    }
}

fn string(value: &str) -> Value {
    Value::String(JsString::try_from_utf8(value).unwrap())
}

const fn bit(value: bool) -> u8 {
    value as u8
}

fn oracle_observations(oracle: &OsStr, flag: Option<&str>) -> Vec<String> {
    let mut command = Command::new(oracle);
    if let Some(flag) = flag {
        command.arg(flag);
    }
    let output = command.args(["-e", ORACLE_PROBE]).output().unwrap();
    assert!(
        output.status.success(),
        "QuickJS Function constructor oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}
