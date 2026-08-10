use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, Context, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor,
    PropertyKey, Runtime, RuntimeError, Value,
};

// Deliberate current-slice boundary: Proxy/with resolution and source
// let/const declaration instantiation stay out of this differential until the
// corresponding Rust language/runtime features exist.
const ORACLE_PROBE: &str = r#"
function ownGlobal(name) {
    return Object.prototype.hasOwnProperty.call(globalThis, name);
}

print("missing=" + delete __qjo_delete_missing + "|" +
      typeof __qjo_delete_missing);

__qjo_delete_assigned = 7;
print("assigned=" + delete __qjo_delete_assigned + "|" +
      typeof __qjo_delete_assigned + "|" + ownGlobal("__qjo_delete_assigned"));

print("intrinsics=" + delete undefined + "|" + delete NaN + "|" +
      delete Infinity + "|" + typeof undefined + "|" + typeof NaN + "|" +
      typeof Infinity);

Object.defineProperty(globalThis, "__qjo_delete_config_data", {
    value: 11, writable: true, enumerable: true, configurable: true
});
print("config-data=" + delete __qjo_delete_config_data + "|" +
      ownGlobal("__qjo_delete_config_data"));

Object.defineProperty(globalThis, "__qjo_delete_fixed_data", {
    value: 13, writable: true, enumerable: true, configurable: false
});
print("fixed-data=" + delete __qjo_delete_fixed_data + "|" +
      ownGlobal("__qjo_delete_fixed_data") + "|" + __qjo_delete_fixed_data);

var __qjo_delete_gets = 0;
var __qjo_delete_sets = 0;
function deleteGetter() { __qjo_delete_gets++; return 19; }
function deleteSetter(value) { __qjo_delete_sets++; }

Object.defineProperty(globalThis, "__qjo_delete_config_accessor", {
    get: deleteGetter, set: deleteSetter, enumerable: true, configurable: true
});
print("config-accessor=" + delete __qjo_delete_config_accessor + "|" +
      ownGlobal("__qjo_delete_config_accessor") + "|" +
      __qjo_delete_gets + "|" + __qjo_delete_sets);

Object.defineProperty(globalThis, "__qjo_delete_fixed_accessor", {
    get: deleteGetter, set: deleteSetter, enumerable: true, configurable: false
});
print("fixed-accessor=" + delete __qjo_delete_fixed_accessor + "|" +
      ownGlobal("__qjo_delete_fixed_accessor") + "|" +
      __qjo_delete_gets + "|" + __qjo_delete_sets);

var globalPrototype = Object.getPrototypeOf(globalThis);
Object.defineProperty(globalPrototype, "__qjo_delete_inherited", {
    value: 17, writable: true, enumerable: true, configurable: true
});
print("inherited=" + delete __qjo_delete_inherited + "|" +
      ownGlobal("__qjo_delete_inherited") + "|" + __qjo_delete_inherited);
delete globalPrototype.__qjo_delete_inherited;

print("scopes=" + (function(argument) {
    var local = 2;
    var closure = function() { return delete local; };
    var self = (function named() { return delete named; });
    return (delete argument) + "|" + (delete local) + "|" + closure() + "|" +
           self() + "|" + (delete arguments);
})(1));

print("references=" + (function() {
    var value = 1;
    var parenthesized = delete (value);
    var comma = delete (0, value);
    var assigned = delete (value = 7);
    var afterAssignment = value;
    var post = delete value++;
    var afterPost = value;
    var prefix = delete ++value;
    return parenthesized + "|" + comma + "|" + assigned + "|" +
           afterAssignment + "|" + post + "|" + afterPost + "|" + prefix +
           "|" + value;
})());

print("typeof-basic=" + typeof delete __qjo_delete_type_missing + "|" +
      delete typeof __qjo_delete_type_missing);

Object.defineProperty(globalThis, "__qjo_delete_typeof", {
    get: deleteGetter, set: deleteSetter, enumerable: true, configurable: true
});
print("delete-typeof=" + delete typeof __qjo_delete_typeof + "|" +
      __qjo_delete_gets + "|" + ownGlobal("__qjo_delete_typeof"));

Object.defineProperty(globalThis, "__qjo_typeof_delete", {
    get: deleteGetter, set: deleteSetter, enumerable: true, configurable: true
});
print("typeof-delete=" + typeof delete __qjo_typeof_delete + "|" +
      __qjo_delete_gets + "|" + ownGlobal("__qjo_typeof_delete"));

print("Function-local=" +
      Function("var local = 1; return delete local + '|' + local")());
print("Function-argument=" +
      Function("argument", "return delete argument + '|' + argument")(2));
print("Function-missing=" + Function("return delete __qjo_function_missing")());
"#;

#[test]
fn identifier_delete_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP identifier-delete differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(
        rust_observations(),
        oracle_observations(&oracle),
        "identifier delete behavior differed from pinned QuickJS"
    );
}

#[test]
fn identifier_delete_errors_and_function_stack_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP identifier-delete Error differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (description, source) in [
        ("strict direct identifier", "\"use strict\";\ndelete victim"),
        (
            "strict parenthesized identifier",
            "\"use strict\";\ndelete (victim)",
        ),
        (
            "strict Function constructor body",
            r#"Function("\"use strict\"; delete victim")"#,
        ),
    ] {
        assert_eq!(
            rust_uncaught_error(source),
            oracle_uncaught_error(&oracle, source, description),
            "identifier delete Error drifted for {description}: {source:?}"
        );
    }
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let mut output = Vec::new();

    output.push(format!(
        "missing={}|{}",
        eval_text(&mut context, "delete __qjo_delete_missing"),
        eval_text(&mut context, "typeof __qjo_delete_missing"),
    ));

    let assigned = context
        .eval("__qjo_delete_assigned = 7; delete __qjo_delete_assigned")
        .unwrap();
    output.push(format!(
        "assigned={}|{}|{}",
        value_text(assigned),
        eval_text(&mut context, "typeof __qjo_delete_assigned"),
        has_own(&runtime, &global, "__qjo_delete_assigned"),
    ));

    output.push(format!(
        "intrinsics={}|{}|{}|{}|{}|{}",
        eval_text(&mut context, "delete undefined"),
        eval_text(&mut context, "delete NaN"),
        eval_text(&mut context, "delete Infinity"),
        eval_text(&mut context, "typeof undefined"),
        eval_text(&mut context, "typeof NaN"),
        eval_text(&mut context, "typeof Infinity"),
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_delete_config_data",
        Value::Int(11),
        true,
    );
    output.push(format!(
        "config-data={}|{}",
        eval_text(&mut context, "delete __qjo_delete_config_data"),
        has_own(&runtime, &global, "__qjo_delete_config_data"),
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_delete_fixed_data",
        Value::Int(13),
        false,
    );
    output.push(format!(
        "fixed-data={}|{}|{}",
        eval_text(&mut context, "delete __qjo_delete_fixed_data"),
        has_own(&runtime, &global, "__qjo_delete_fixed_data"),
        eval_text(&mut context, "__qjo_delete_fixed_data"),
    ));

    define_global_data(
        &runtime,
        &mut context,
        "__qjo_delete_gets",
        Value::Int(0),
        true,
    );
    define_global_data(
        &runtime,
        &mut context,
        "__qjo_delete_sets",
        Value::Int(0),
        true,
    );

    define_global_accessor(&runtime, &mut context, "__qjo_delete_config_accessor", true);
    output.push(format!(
        "config-accessor={}|{}|{}|{}",
        eval_text(&mut context, "delete __qjo_delete_config_accessor"),
        has_own(&runtime, &global, "__qjo_delete_config_accessor"),
        global_text(&runtime, &mut context, "__qjo_delete_gets"),
        global_text(&runtime, &mut context, "__qjo_delete_sets"),
    ));

    define_global_accessor(&runtime, &mut context, "__qjo_delete_fixed_accessor", false);
    output.push(format!(
        "fixed-accessor={}|{}|{}|{}",
        eval_text(&mut context, "delete __qjo_delete_fixed_accessor"),
        has_own(&runtime, &global, "__qjo_delete_fixed_accessor"),
        global_text(&runtime, &mut context, "__qjo_delete_gets"),
        global_text(&runtime, &mut context, "__qjo_delete_sets"),
    ));

    let global_prototype = runtime.get_prototype_of(&global).unwrap().unwrap();
    let inherited_key = runtime
        .intern_property_key("__qjo_delete_inherited")
        .unwrap();
    define_data(
        &mut context,
        &global_prototype,
        &inherited_key,
        Value::Int(17),
        true,
    );
    output.push(format!(
        "inherited={}|{}|{}",
        eval_text(&mut context, "delete __qjo_delete_inherited"),
        has_own(&runtime, &global, "__qjo_delete_inherited"),
        eval_text(&mut context, "__qjo_delete_inherited"),
    ));
    assert!(
        runtime
            .delete_property(&global_prototype, &inherited_key)
            .unwrap()
    );

    output.push(format!(
        "scopes={}",
        eval_text(
            &mut context,
            "(function(argument){ var local = 2; var closure = function(){ return delete local; }; var self = (function named(){ return delete named; }); return (delete argument) + '|' + (delete local) + '|' + closure() + '|' + self() + '|' + (delete arguments); })(1)",
        )
    ));

    output.push(format!(
        "references={}",
        eval_text(
            &mut context,
            "(function(){ var value = 1; var parenthesized = delete (value); var comma = delete (0, value); var assigned = delete (value = 7); var afterAssignment = value; var post = delete value++; var afterPost = value; var prefix = delete ++value; return parenthesized + '|' + comma + '|' + assigned + '|' + afterAssignment + '|' + post + '|' + afterPost + '|' + prefix + '|' + value; })()",
        )
    ));

    output.push(format!(
        "typeof-basic={}|{}",
        eval_text(&mut context, "typeof delete __qjo_delete_type_missing"),
        eval_text(&mut context, "delete typeof __qjo_delete_type_missing"),
    ));

    define_global_accessor(&runtime, &mut context, "__qjo_delete_typeof", true);
    output.push(format!(
        "delete-typeof={}|{}|{}",
        eval_text(&mut context, "delete typeof __qjo_delete_typeof"),
        global_text(&runtime, &mut context, "__qjo_delete_gets"),
        has_own(&runtime, &global, "__qjo_delete_typeof"),
    ));

    define_global_accessor(&runtime, &mut context, "__qjo_typeof_delete", true);
    output.push(format!(
        "typeof-delete={}|{}|{}",
        eval_text(&mut context, "typeof delete __qjo_typeof_delete"),
        global_text(&runtime, &mut context, "__qjo_delete_gets"),
        has_own(&runtime, &global, "__qjo_typeof_delete"),
    ));

    output.push(format!(
        "Function-local={}",
        eval_text(
            &mut context,
            r#"Function("var local = 1; return delete local + '|' + local")()"#,
        )
    ));
    output.push(format!(
        "Function-argument={}",
        eval_text(
            &mut context,
            r#"Function("argument", "return delete argument + '|' + argument")(2)"#,
        )
    ));
    output.push(format!(
        "Function-missing={}",
        eval_text(
            &mut context,
            r#"Function("return delete __qjo_function_missing")()"#,
        )
    ));

    output
}

fn define_global_data(
    runtime: &Runtime,
    context: &mut Context,
    name: &str,
    value: Value,
    configurable: bool,
) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    define_data(context, &global, &key, value, configurable);
}

fn define_data(
    context: &mut Context,
    object: &ObjectRef,
    key: &PropertyKey,
    value: Value,
    configurable: bool,
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
                    configurable: DescriptorField::Present(configurable),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn define_global_accessor(
    runtime: &Runtime,
    context: &mut Context,
    name: &str,
    configurable: bool,
) {
    let getter = function(
        runtime,
        context,
        "(function(){ __qjo_delete_gets++; return 19; })",
    );
    let setter = function(
        runtime,
        context,
        "(function(value){ __qjo_delete_sets++; })",
    );
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(configurable),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn function(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("function probe did not produce an object: {source}");
    };
    runtime.as_callable(&object).unwrap().unwrap()
}

fn has_own(runtime: &Runtime, object: &ObjectRef, name: &str) -> bool {
    let key = runtime.intern_property_key(name).unwrap();
    runtime.has_own_property(object, &key).unwrap()
}

fn global_text(runtime: &Runtime, context: &mut Context, name: &str) -> String {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    value_text(context.get_property(&global, &key).unwrap())
}

fn eval_text(context: &mut Context, source: &str) -> String {
    value_text(context.eval(source).unwrap())
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

fn rust_uncaught_error(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval_with_filename(source, "<cmdline>"),
        Err(RuntimeError::Exception),
        "Rust source unexpectedly completed: {source:?}",
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Rust identifier-delete exception was not an object: {source:?}");
    };
    assert!(runtime.is_error_object(&error).unwrap());
    format!(
        "{}: {}\n{}",
        error_property_text(&runtime, &mut context, &error, "name"),
        error_property_text(&runtime, &mut context, &error, "message"),
        error_property_text(&runtime, &mut context, &error, "stack"),
    )
}

fn error_property_text(
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

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS identifier-delete oracle");
    assert!(
        output.status.success(),
        "QuickJS identifier-delete oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS identifier-delete oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn oracle_uncaught_error(oracle: &OsStr, source: &str, description: &str) -> String {
    let output = Command::new(oracle)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        !output.status.success(),
        "QuickJS unexpectedly completed {description}: {source:?}",
    );
    String::from_utf8(output.stderr)
        .unwrap_or_else(|error| panic!("QuickJS emitted non-UTF-8 stderr: {error}"))
}
