use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, JsString, Runtime, RuntimeError, Value};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "sloppy block lexical and Annex outer use distinct closures",
        "(function(){var inside;{function block(){return block}inside=block}return (inside!==block)+'|'+(inside()===inside)+'|'+(block()===inside)})()",
    ),
    (
        "strict block function stays lexical and mutable",
        "(function(){'use strict';var inside;{function block(){return 3}inside=block;block=4}return typeof block+'|'+inside()})()",
    ),
    (
        "skipped sloppy block still declares an undefined Annex outer",
        "(function(){if(false){function skipped(){return 1}}return typeof skipped+'|'+delete skipped})()",
    ),
    (
        "authored Annex closure does not copy a mutated block lexical",
        "(function(){var inside;{block=8;function block(){}inside=block}return inside+'|'+typeof block+'|'+block.name})()",
    ),
    (
        "sloppy duplicate keeps the last block child and first Annex child",
        "(function(){var inside;{function duplicate(){return 1}function duplicate(){return 2}inside=duplicate}return inside()+'|'+duplicate()+'|'+(inside===duplicate)})()",
    ),
    (
        "duplicate child recursion resolves through the last lexical slot",
        "(function(){var inside;{function duplicate(){return duplicate}function duplicate(){return 2}inside=duplicate}return (duplicate()===inside)+'|'+inside()})()",
    ),
    (
        "existing outer var is updated at the declaration source position",
        "(function(){var target=7,get=function(){return target},before,after;{before=get();function target(){return 4}after=get()}return before+'|'+typeof after+'|'+after()})()",
    ),
    (
        "simple parameter suppresses the sloppy Annex outer update",
        "(function(parameter){var inside;{function parameter(){return 5}inside=parameter}return parameter+'|'+inside()})(1)",
    ),
    (
        "arguments name suppresses the sloppy Annex outer update",
        "(function(){var inside;{function arguments(){return 6}inside=arguments}return inside()})()",
    ),
    (
        "explicit arguments parameter remains unchanged beside its block lexical",
        "(function(arguments){var inside;{function arguments(){return 6}inside=arguments}return arguments+'|'+inside()})(7)",
    ),
    (
        "named expression self is shadowed by the synthetic Annex root var",
        "(function(){var original=function self(){{function self(){return 7}}return self};var replacement=original();return (replacement!==original)+'|'+replacement()})()",
    ),
    (
        "block function captures a later lexical initialized before its call",
        "(function(){{function laterRead(){return later}let later=8;return laterRead()}})()",
    ),
    (
        "captured block functions and cells are fresh on loop reentry",
        "(function(){var first,second,i=0;while(i<2){let value=i;function fresh(){return value}if(i===0)first=fresh;else second=fresh;i++}return (first!==second)+'|'+first()+'|'+second()})()",
    ),
    (
        "switch enters one declaration scope after the discriminant and before case tests",
        "(function(){var trace=typeof selected,inside;switch(0){case (trace+='|'+typeof selected,0):function selected(){return 9}inside=selected;break;case 1:function selected(){return 10}}return trace+'|'+inside()+'|'+selected()})()",
    ),
    (
        "normal Function constructor block follows the same declaration path",
        "Function(\"var inside;{function dynamicBlock(){return 11}inside=dynamicBlock}return inside()+'|'+dynamicBlock()+'|'+(inside===dynamicBlock)\")()",
    ),
    (
        "prior enclosing lexical suppresses the sloppy Annex outer update",
        "(function(){let shadow=12;{function shadow(){return 13}}return shadow})()",
    ),
    (
        "strict block generator declaration resumes through completion",
        "(function(){'use strict';var saved;{function* blockGenerator(){yield 4}saved=blockGenerator}var iterator=saved();return iterator.next().value+'|'+iterator.next().done})()",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "entry-created block function observes a later lexical TDZ",
        "(function(){{return readBeforeInitialization();function readBeforeInitialization(){return later}let later=1}})()",
    ),
    (
        "Program Annex update observes a later global lexical TDZ",
        "{function laterGlobalLexical(){}}let laterGlobalLexical;",
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "block lexical closure fault",
        "(function blockOuter(){\n  {\n    function blockInner(){\n      missingBlockLexical;\n    }\n    return blockInner();\n  }\n})()",
    ),
    (
        "Annex outer closure fault",
        "(function annexOuter(){\n  {\n    function annexInner(){\n      missingAnnexOuter;\n    }\n  }\n  return annexInner();\n})()",
    ),
    (
        "Function constructor block declaration fault",
        "Function(\"{\\n  function dynamicBlockInner(){\\n    missingDynamicBlock;\\n  }\\n  return dynamicBlockInner();\\n}\")()",
    ),
];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    (
        "lexical before malformed block function keeps redefinition priority",
        "(function(){\n  { let conflict; function conflict( }\n})",
    ),
    (
        "strict duplicate block function keeps redefinition priority",
        "(function(){\n  'use strict';\n  { function duplicate(){} function duplicate( }\n})",
    ),
    (
        "block function before lexical",
        "(function(){\n  { function conflict(){} let conflict; }\n})",
    ),
    (
        "var before block function",
        "(function(){\n  { var conflict; function conflict(){} }\n})",
    ),
    (
        "block function before var",
        "(function(){\n  { function conflict(){} var conflict; }\n})",
    ),
    (
        "switch cases share one lexical declaration scope",
        "(function(value){\n  switch(value){ case 0: let conflict; break; case 1: function conflict(){} }\n})",
    ),
    (
        "child directive makes its block function eval name strict",
        "(function(){\n  { function eval(){ 'use strict'; } }\n})",
    ),
];

const UNSUPPORTED_BOUNDARY_CASES: &[(&str, &str)] = &[(
    "async block function declaration",
    "(function(){{async function asyncBlock(){return 3}}return typeof asyncBlock})()",
)];

#[test]
fn block_function_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP block-function differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle_sequence(&oracle, &[source], description),
            "block-function value drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn block_function_errors_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP block-function error differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle_sequence(&oracle, &[source], description),
            "block-function error drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn escaped_block_lexical_after_failed_initializer_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP block-function failed-initializer differential: set QJS_ORACLE to upstream qjs"
        );
        return;
    };
    let description = "failed block lexical initializer preserves the escaped function cell";
    let sources = [
        "(function(){{function escapedBlock(){return later}Function.savedBlock=escapedBlock;let later=missingBlockInitializer}})()",
        "Function.savedBlock()",
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
        "escaped block lexical state drifted",
    );
}

#[test]
fn program_annex_then_lexical_state_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP block-function global-state differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let description = "Annex global var precedes a later global lexical";
    let sources = [
        "{function orderedGlobalCollision(){}}let orderedGlobalCollision;",
        "typeof globalThis.orderedGlobalCollision",
        "orderedGlobalCollision",
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
        "Program Annex/global-lexical state drifted",
    );
}

#[test]
fn block_function_full_and_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP block-function stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in STACK_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn block_function_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP block-function parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in SYNTAX_ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn unsupported_block_function_boundaries_remain_explicit() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP block-function boundaries: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in UNSUPPORTED_BOUNDARY_CASES {
        let quickjs = observe_oracle_sequence(&oracle, &[source], description);
        assert!(
            quickjs.starts_with("return|"),
            "pinned QuickJS unexpectedly rejected {description}: {quickjs}"
        );
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let rust = observe_rust_eval(&runtime, &mut context, source, description);
        assert!(
            rust.starts_with("throw|object|SyntaxError|"),
            "unsupported {description} was not rejected explicitly: {rust}"
        );
    }
}

#[test]
fn block_function_cross_realm_regression() {
    // Pinned with the QuickJS C API compile-in-A/execute-in-B path. The Rust
    // API exposes the same operation directly, so this remains unconditional.
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    defining.eval("globalThis.realmTag='A'").unwrap();
    caller.eval("globalThis.realmTag='B'").unwrap();

    let bytecode = defining
        .compile(
            "{function crossRealmBlock(){return realmTag+'|'+(this===globalThis)}\
             Function.savedCrossRealmBlock=crossRealmBlock}\
             crossRealmBlock",
        )
        .unwrap();
    let Value::Object(outer) = caller.execute(&bytecode).unwrap() else {
        panic!("cross-realm block Program did not return its Annex function");
    };
    let Value::Object(inner) = caller.eval("Function.savedCrossRealmBlock").unwrap() else {
        panic!("cross-realm block did not save its lexical function");
    };
    assert_ne!(outer, inner);

    let Value::Object(prototype_a) = defining.eval("Function.prototype").unwrap() else {
        panic!("defining Function.prototype was not an object");
    };
    let Value::Object(prototype_b) = caller.eval("Function.prototype").unwrap() else {
        panic!("caller Function.prototype was not an object");
    };
    for function in [&outer, &inner] {
        assert_eq!(
            runtime.get_prototype_of(function).unwrap(),
            Some(prototype_a.clone())
        );
        assert_ne!(
            runtime.get_prototype_of(function).unwrap(),
            Some(prototype_b.clone())
        );
        let callable = runtime.as_callable(function).unwrap().unwrap();
        assert_eq!(
            caller.call(&callable, Value::Undefined, &[]).unwrap(),
            Value::String(JsString::try_from_utf8("B|false").unwrap())
        );
    }
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
