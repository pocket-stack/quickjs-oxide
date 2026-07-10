use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

const ORACLE_PROBE: &str = r#"
function frames(stack, count) {
    return String(stack).split("\n").slice(0, count).join("|");
}
function text(value) {
    return value === undefined ? "undefined" : String(value);
}

try {
    Function("(1)++");
    print("invalid=missing");
} catch (error) {
    print("invalid=" + error.name + ":" + error.message + "|" +
          text(error.fileName) + ":" + text(error.lineNumber) + ":" +
          text(error.columnNumber) + "|" + frames(error.stack, 2));
}

globalThis.__qjo_update_symbol = Symbol();
var update = Function("\nreturn __qjo_update_symbol++");
try {
    update();
    print("runtime=missing");
} catch (error) {
    print("runtime=" + error.name + ":" + error.message + "|" +
          frames(error.stack, 1));
}

try {
    Function("\"use strict\"; eval++");
    print("strict-eval=missing");
} catch (error) {
    print("strict-eval=" + error.name + ":" + error.message + "|" +
          frames(error.stack, 2));
}

try {
    Function("\"use strict\"; arguments--");
    print("strict-arguments=missing");
} catch (error) {
    print("strict-arguments=" + error.name + ":" + error.message + "|" +
          frames(error.stack, 2));
}

print("strict-caller=" + (function () {
    "use strict";
    return Function("eval", "return eval++")(9);
})());
"#;

#[test]
fn update_expressions_in_function_constructor_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP update Function constructor differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(
        rust_observations(),
        oracle_observations(&oracle),
        "Function constructor update-expression behavior differed from pinned QuickJS"
    );
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let constructor = context.function_constructor().unwrap();
    let mut output = Vec::new();

    assert_eq!(
        context.call(&constructor, Value::Undefined, &[string("(1)++")],),
        Err(RuntimeError::Exception)
    );
    let invalid = take_error(&runtime, &mut context, "invalid update SyntaxError");
    output.push(format!(
        "invalid={}|{}:{}:{}|{}",
        error_name_message(&runtime, &mut context, &invalid),
        property_text(&runtime, &mut context, &invalid, "fileName"),
        property_text(&runtime, &mut context, &invalid, "lineNumber"),
        property_text(&runtime, &mut context, &invalid, "columnNumber"),
        stack_frames(&runtime, &mut context, &invalid, 2),
    ));

    let global = context.global_object().unwrap();
    let symbol_key = runtime.intern_property_key("__qjo_update_symbol").unwrap();
    let symbol = runtime.new_symbol(None).unwrap();
    assert!(
        context
            .set_property(&global, &symbol_key, Value::Symbol(symbol))
            .unwrap()
    );
    let update = call_function_constructor(
        &mut context,
        &constructor,
        &[string("\nreturn __qjo_update_symbol++")],
    );
    let update = runtime.as_callable(&update).unwrap().unwrap();
    assert_eq!(
        context.call(&update, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let runtime_error = take_error(&runtime, &mut context, "postfix Symbol TypeError");
    output.push(format!(
        "runtime={}|{}",
        error_name_message(&runtime, &mut context, &runtime_error),
        stack_frames(&runtime, &mut context, &runtime_error, 1),
    ));

    observe_strict_body_error(
        &runtime,
        &mut context,
        &constructor,
        "strict-eval",
        "\"use strict\"; eval++",
        &mut output,
    );
    observe_strict_body_error(
        &runtime,
        &mut context,
        &constructor,
        "strict-arguments",
        "\"use strict\"; arguments--",
        &mut output,
    );

    let strict_caller = context
        .eval(
            r#"(function () {
                "use strict";
                return Function("eval", "return eval++")(9);
            })()"#,
        )
        .unwrap();
    output.push(format!("strict-caller={}", value_text(strict_caller)));

    output
}

fn observe_strict_body_error(
    runtime: &Runtime,
    context: &mut Context,
    constructor: &CallableRef,
    label: &str,
    body: &str,
    output: &mut Vec<String>,
) {
    assert_eq!(
        context.call(constructor, Value::Undefined, &[string(body)]),
        Err(RuntimeError::Exception)
    );
    let error = take_error(runtime, context, label);
    output.push(format!(
        "{label}={}|{}",
        error_name_message(runtime, context, &error),
        stack_frames(runtime, context, &error, 2),
    ));
}

fn call_function_constructor(
    context: &mut Context,
    constructor: &CallableRef,
    arguments: &[Value],
) -> ObjectRef {
    let Value::Object(function) = context
        .call(constructor, Value::Undefined, arguments)
        .unwrap()
    else {
        panic!("Function constructor result was not an object");
    };
    function
}

fn take_error(runtime: &Runtime, context: &mut Context, label: &str) -> ObjectRef {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("{label} was not an Error object");
    };
    assert!(
        runtime.is_error_object(&error).unwrap(),
        "{label} was not Error"
    );
    error
}

fn error_name_message(runtime: &Runtime, context: &mut Context, error: &ObjectRef) -> String {
    format!(
        "{}:{}",
        property_text(runtime, context, error, "name"),
        property_text(runtime, context, error, "message"),
    )
}

fn stack_frames(
    runtime: &Runtime,
    context: &mut Context,
    error: &ObjectRef,
    count: usize,
) -> String {
    let Value::String(stack) = property_value(runtime, context, error, "stack") else {
        panic!("Error.stack was not a string");
    };
    let stack = stack.to_utf8_lossy();
    let frames = stack.lines().collect::<Vec<_>>();
    assert_eq!(frames.len(), count, "Rust Error.stack grew extra frames");
    frames.join("|")
}

fn property_text(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    value_text(property_value(runtime, context, object, name))
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
    Value::String(JsString::from(value))
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "QuickJS update Function constructor oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}
