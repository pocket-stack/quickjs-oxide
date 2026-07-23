use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, JsString, Runtime, RuntimeError, Value};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "direct body declaration is callable before its source position",
        "(function bodyHoist(){var first=bodyTarget();return typeof bodyTarget+'|'+first+'|'+bodyTarget.name+'|'+bodyTarget.length+'|'+(bodyTarget.prototype.constructor===bodyTarget);function bodyTarget(left,right){return 7}})()",
    ),
    (
        "each function activation creates a fresh hoisted closure",
        "(function(){function make(){return local;function local(){}}return make()!==make()})()",
    ),
    (
        "last duplicate body declaration wins in sloppy code",
        "(function(){return duplicate();function duplicate(){return 1}function duplicate(){return 2}})()",
    ),
    (
        "last duplicate body declaration wins in strict code",
        "(function(){'use strict';return duplicate();function duplicate(){return 3}function duplicate(){return 4}})()",
    ),
    (
        "var without an initializer preserves the function hoist",
        "(function(){var before=typeof mixed+'|'+mixed();var mixed;return before;function mixed(){return 5}})()",
    ),
    (
        "var initializer runs after the function hoist",
        "(function(){var before=mixed();var mixed=function(){return 9};return before+'|'+mixed();function mixed(){return 6}})()",
    ),
    (
        "body declaration replaces a same-name parameter",
        "(function(parameter){var original=parameter;function parameter(){return 7}return typeof original+'|'+original()})(3)",
    ),
    (
        "body declaration replaces the last duplicate parameter slot",
        "(function(parameter,parameter){function parameter(){return 13}return parameter()})(1,2)",
    ),
    (
        "arguments-name declaration replaces the ordinary body binding",
        "(function(){function arguments(){return 8}return typeof arguments+'|'+arguments.name+'|'+arguments()})()",
    ),
    (
        "var arguments before its declaration shares the hoisted body local",
        "(function(){var arguments;function arguments(){return 8}return arguments()})()",
    ),
    (
        "var arguments after its declaration preserves the hoisted body local",
        "(function(){function arguments(){return 8}var arguments;return arguments()})()",
    ),
    (
        "var arguments initializer overwrites the declaration hoist",
        "(function(){var arguments=function(){return 9};function arguments(){return 8}return arguments()})()",
    ),
    (
        "body declaration shadows a named-expression self binding",
        "(function(){var expression=function self(){function self(){return 'body'}return (self===expression)+'|'+self()};return expression()})()",
    ),
    (
        "declaration self reference uses the mutable body cell",
        "(function(){function mutable(){return mutable}var original=mutable;mutable=17;return (original()===17)+'|'+typeof mutable})()",
    ),
    (
        "sibling body declarations capture each other",
        "(function(){function first(){return second}function second(){return first}return first()===second&&second()===first})()",
    ),
    (
        "hoisted declaration captures a later initialized lexical",
        "(function(){function readLater(){return later}let later=42;return readLater()})()",
    ),
    (
        "Function constructor body uses the direct declaration hoist path",
        "Function('return dynamicTarget();function dynamicTarget(){return 12}')()",
    ),
    (
        "direct body generator declaration hoists and resumes through completion",
        "(function(){function* bodyGenerator(){yield 1;return 2}var iterator=bodyGenerator();return iterator.next().value+'|'+iterator.next().value+'|'+iterator.next().done})()",
    ),
    (
        "direct body async declaration hoists with async function shape",
        "(function(){var before=Object.prototype.toString.call(bodyAsync)+'|'+bodyAsync.name+'|'+bodyAsync.length+'|'+('prototype' in bodyAsync);async function bodyAsync(value){return value}return before})()",
    ),
];

const ERROR_OBSERVATION_CASES: &[(&str, &str)] = &[(
    "hoisted declaration observes the later lexical temporal dead zone",
    "(function bodyDeclarationTdz(){return readLater();function readLater(){return later}let later=42})()",
)];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "hoisted body declaration fault",
        "(function bodyDeclarationOuter(){\n  return bodyDeclarationInner();\n  function bodyDeclarationInner(){\n    missingBodyDeclaration;\n  }\n})()",
    ),
    (
        "last duplicate body declaration owns the stack location",
        "(function duplicateBodyDeclaration(){\n  function duplicateTarget(){ missingFirstBodyDeclaration; }\n  function duplicateTarget(){ missingSecondBodyDeclaration; }\n  return duplicateTarget();\n})()",
    ),
    (
        "Function constructor body declaration owns its dynamic stack",
        "Function(\"function bodyInner(){\\n  missingDynamicBody;\\n}\\nreturn bodyInner()\")()",
    ),
];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    (
        "body lexical before function declaration",
        "(function(){\n  let conflict;\n  function conflict(){}\n})",
    ),
    (
        "body function declaration before lexical",
        "(function(){\n  function conflict(){}\n  let conflict;\n})",
    ),
    (
        "strict body function named eval",
        "(function(){\n  'use strict';\n  function eval(){}\n})",
    ),
    (
        "strict body function named arguments",
        "(function(){\n  'use strict';\n  function arguments(){}\n})",
    ),
    (
        "declared function directive makes its eval name strict",
        "(function(){\n  function eval(){ 'use strict'; }\n})",
    ),
    (
        "declared function directive rejects duplicate parameters",
        "(function(){\n  function child(a,a){ 'use strict'; }\n})",
    ),
];

#[test]
fn direct_function_body_declaration_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP function-body declaration differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle_sequence(&oracle, &[source], description),
            "function-body declaration drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn direct_function_body_declaration_tdz_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP function-body declaration TDZ differential: set QJS_ORACLE to upstream qjs"
        );
        return;
    };

    for &(description, source) in ERROR_OBSERVATION_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle_sequence(&oracle, &[source], description),
            "function-body declaration error drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn failed_body_lexical_initializer_keeps_escaped_hoist_tdz_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP function-body failed-initializer differential: set QJS_ORACLE to upstream qjs"
        );
        return;
    };
    let description = "failed body lexical initializer preserves its escaped hoist cell";
    let sources = [
        "(function(){Function.savedBodyRead=readLater;let later=missingBodyInitializer;function readLater(){return later}})()",
        "Function.savedBodyRead()",
    ];
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let rust = sources
        .iter()
        .map(|source| observe_rust_eval(&runtime, &mut context, source, description))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        rust,
        observe_oracle_sequence(&oracle, &sources, description),
        "function-body declaration failed-initializer state drifted",
    );
}

#[test]
fn function_body_declaration_full_and_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP function-body declaration stack differential: set QJS_ORACLE to upstream qjs"
        );
        return;
    };

    for &(description, source) in STACK_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn function_body_declaration_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP function-body declaration parser differential: set QJS_ORACLE to upstream qjs"
        );
        return;
    };

    for &(description, source) in SYNTAX_ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn function_body_declaration_cross_realm_regression() {
    // These expectations are pinned from the QuickJS C API's compile-in-A,
    // execute-in-B path; the Rust API exposes the same two-context operation
    // directly, so keep it in the unconditional product regression suite.
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    defining.eval("globalThis.realmTag='A'").unwrap();
    caller.eval("globalThis.realmTag='B'").unwrap();

    let bytecode = defining
        .compile(
            "(function crossRealmOuter(value,selectBoom){\
                 function crossRealmInner(){\
                   return value+'|'+realmTag+'|'+\
                     (this===globalThis)\
                 }\
                 function crossRealmBoom(){missingBodyRealm}\
                 if(selectBoom)return crossRealmBoom;\
                 return crossRealmInner\
             })",
        )
        .unwrap();
    let Value::Object(outer) = caller.execute(&bytecode).unwrap() else {
        panic!("cross-realm Program did not return its outer function");
    };

    let Value::Object(function_prototype_a) = defining.eval("Function.prototype").unwrap() else {
        panic!("defining Function.prototype was not an object");
    };
    let Value::Object(function_prototype_b) = caller.eval("Function.prototype").unwrap() else {
        panic!("caller Function.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&outer).unwrap(),
        Some(function_prototype_a.clone())
    );
    assert_ne!(
        runtime.get_prototype_of(&outer).unwrap(),
        Some(function_prototype_b.clone())
    );

    let outer = runtime.as_callable(&outer).unwrap().unwrap();
    let Value::Object(inner) = caller
        .call(
            &outer,
            Value::Undefined,
            &[
                Value::String(JsString::try_from_utf8("argB").unwrap()),
                Value::Bool(false),
            ],
        )
        .unwrap()
    else {
        panic!("outer function did not return its inner declaration");
    };
    assert_eq!(
        runtime.get_prototype_of(&inner).unwrap(),
        Some(function_prototype_a.clone())
    );
    assert_ne!(
        runtime.get_prototype_of(&inner).unwrap(),
        Some(function_prototype_b)
    );
    let inner = runtime.as_callable(&inner).unwrap().unwrap();
    let inner_result = caller
        .call(&inner, Value::Undefined, &[])
        .unwrap_or_else(|failure| {
            let exception = caller
                .take_exception()
                .expect("take cross-realm inner exception")
                .expect("cross-realm inner exception was missing");
            let detail = match exception {
                Value::Object(error) => format!(
                    "{}: {}",
                    error_string_property(
                        &runtime,
                        &mut caller,
                        &error,
                        "name",
                        "cross-realm inner call",
                    ),
                    error_string_property(
                        &runtime,
                        &mut caller,
                        &error,
                        "message",
                        "cross-realm inner call",
                    ),
                ),
                value => primitive_value_text(value),
            };
            panic!("cross-realm inner call failed with {failure}: {detail}",);
        });
    assert_eq!(
        inner_result,
        Value::String(JsString::try_from_utf8("argB|B|false").unwrap())
    );

    let Value::Object(boom) = caller
        .call(
            &outer,
            Value::Undefined,
            &[
                Value::String(JsString::try_from_utf8("argB").unwrap()),
                Value::Bool(true),
            ],
        )
        .unwrap()
    else {
        panic!("outer function did not return its failing declaration");
    };
    let boom = runtime.as_callable(&boom).unwrap().unwrap();
    assert_eq!(
        caller.call(&boom, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller
        .take_exception()
        .unwrap()
        .expect("cross-realm body declaration exception")
    else {
        panic!("cross-realm body declaration did not throw an object");
    };
    let Value::Object(reference_error_prototype_a) =
        defining.eval("ReferenceError.prototype").unwrap()
    else {
        panic!("defining ReferenceError.prototype was not an object");
    };
    let Value::Object(reference_error_prototype_b) =
        caller.eval("ReferenceError.prototype").unwrap()
    else {
        panic!("caller ReferenceError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(reference_error_prototype_a)
    );
    assert_ne!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(reference_error_prototype_b)
    );
}

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value)
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    error_string_property(runtime, context, &error, "name", description),
                    error_string_property(runtime, context, &error, "message", description),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value)
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn observe_oracle_sequence(oracle: &OsStr, sources: &[&str], description: &str) -> String {
    let wrapper = r#"
(function () {
for (var index = 0; index < scriptArgs.length; index++) {
  try {
    var value = std.evalScript(scriptArgs[index]);
    print('return|' + typeof value + '|' + String(value));
  } catch (error) {
    if (error !== null && typeof error === 'object')
      print('throw|object|' + error.name + '|' + error.message);
    else
      print('throw|' + typeof error + '|' + String(error));
  }
}
})();
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper])
        .args(sources)
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS sequence failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"));
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
}

fn compare_cli(oracle: &OsStr, options: &[&str], source: &str, description: &str) {
    let rust = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        options,
        source,
        description,
    );
    let quickjs = run_cli(oracle, options, source, description);
    assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
    assert_eq!(rust.stdout, quickjs.stdout, "{description}");
    assert_eq!(rust.stderr, quickjs.stderr, "{description}");
}

fn run_cli(program: &OsStr, options: &[&str], source: &str, description: &str) -> Output {
    Command::new(program)
        .args(options)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &quickjs_oxide::ObjectRef,
    name: &str,
    description: &str,
) -> String {
    let key = runtime
        .intern_property_key(name)
        .expect("Error property key");
    let Value::String(value) = context
        .get_property(error, &key)
        .unwrap_or_else(|failure| panic!("read Error.{name} for {description}: {failure}"))
    else {
        panic!("Error.{name} was not a string for {description}");
    };
    value.to_utf8_lossy()
}

fn value_type(runtime: &Runtime, value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Int(_) | Value::Float(_) => "number",
        Value::BigInt(_) => "bigint",
        Value::String(_) => "string",
        Value::Object(object) => {
            if runtime.as_callable(object).unwrap().is_some() {
                "function"
            } else {
                "object"
            }
        }
        Value::Symbol(_) => "symbol",
    }
}

fn primitive_value_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}
