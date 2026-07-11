use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "simple let and const execution",
        "(function(){ let value; let base = 40, delta = 1; value = base + delta; value++; const answer = value; return answer; })()",
    ),
    (
        "uninitialized let becomes undefined",
        "(function(){ let value; const result = value === undefined; return result; })()",
    ),
    (
        "closure created before declaration observes initialization",
        "(function(){ const read = function(){ return later; }; let later = 42; return read(); })()",
    ),
    (
        "transitive closure relay preserves one lexical cell",
        "(function(){ let value = 40; const make = function(){ return function step(){ value += 1; return value; }; }; const step = make(); return step() + step(); })()",
    ),
    (
        "Function constructor lexical body",
        "Function('let value = 40; const delta = 2; return value + delta;')()",
    ),
    (
        "Function constructor lexical closure relay",
        "(function(){ const step = Function('let value = 40; return function(){ value += 1; return value; };')(); return step() + step(); })()",
    ),
    (
        "declaration lists initialize from left to right",
        "(function(){ let log = '', a = (log += 'a', 1), b = (log += a, 2), c = (log += b, 3); return log + '|' + a + '|' + b + '|' + c; })()",
    ),
    (
        "lexical anonymous functions receive contextual names",
        "(function(){ let first = function(){}, second = function named(){}; const third = function(){}; return first.name + '|' + second.name + '|' + third.name; })()",
    ),
    (
        "lexical delete is statically false",
        "(function(){ const deleted = delete value; let value = 1; return deleted; })()",
    ),
    (
        "short-circuited const logical assignment does not write",
        "(function(){ const value = 0; return value &&= missing; })()",
    ),
    (
        "sloppy eval and arguments lexical names remain ordinary bindings",
        "(function(){ let arguments = 20; const eval = 22; return arguments + eval; })()",
    ),
    (
        "dynamic Function does not capture caller lexical state",
        "(function(){ let callerValue = 1; return Function('return typeof callerValue')(); })()",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "direct temporal dead zone read",
        "(function lexicalTdz(){ return value; let value = 1; })()",
    ),
    (
        "typeof temporal dead zone read",
        "(function lexicalTypeofTdz(){ return typeof value; let value = 1; })()",
    ),
    (
        "direct const write",
        "(function lexicalReadonly(){ const value = 1; value = 2; })()",
    ),
    (
        "captured const write",
        "(function lexicalReadonlyOuter(){ const value = 1; return function lexicalReadonlyInner(){ value = 2; }; })()()",
    ),
    (
        "const write before declaration inherits the final dead source marker",
        "(function lexicalReadonlyBefore(){\n  value = 1;\n  const value = 2;\n})()",
    ),
    (
        "const self-write initializer inherits the declaration marker",
        "(function lexicalReadonlySelf(){\n  const value = (value = 1);\n})()",
    ),
    (
        "following return supersedes a const self-write marker",
        "(function lexicalReadonlyReturn(){\n  const value = (value = 1);\n  return 0;\n})()",
    ),
    (
        "branch join bounds a late readonly throw marker",
        "(function lexicalReadonlyBranch(flag){\n  if (flag) value = 1;\n  const value = 2;\n  return 0;\n})(true)",
    ),
    (
        "dead forward branch references are removed before late throw mapping",
        "(function lexicalReadonlyDeadBranch(){\n  value = 1;\n  if (false) 2;\n  const value = 3;\n  99;\n})()",
    ),
    (
        "return marker wins over a nested readonly assignment marker",
        "(function lexicalReadonlyReturnExpression(){\n  return 0 + (value = 2);\n  const value = 1;\n})()",
    ),
    (
        "earlier branch throw removal updates a later branch join",
        "(function lexicalReadonlyBranches(flag){\n  if (flag) value = 1; else value = 2;\n  const value = 3;\n  99;\n})(false)",
    ),
    (
        "constant branch folding releases the untaken readonly label",
        "(function lexicalReadonlyConstantBranch(){\n  if (true) value = 1; else value = 2;\n  const value = 3;\n  99;\n})()",
    ),
    (
        "unreferenced physical labels still block adjacent constant folding",
        "(function lexicalReadonlyConditionalLabel(flag){\n  if (false ? true : false) { if (flag) value = 1; } else { value = 2; }\n  const value = 9;\n  99;\n})(false)",
    ),
    (
        "conditional goto inversion preserves the inner readonly join",
        "(function lexicalReadonlyInversion(){\n  if (true) { if (false ? true : false); else value = 1; } else { value = 3; }\n  const value = 9;\n  99;\n})()",
    ),
];

struct BoundaryCase {
    description: &'static str,
    source: &'static str,
    rust_message: &'static str,
}

const BOUNDARY_CASES: &[BoundaryCase] = &[
    BoundaryCase {
        description: "global lexical declaration",
        source: "let globalValue = 1; globalValue;",
        rust_message: "top-level lexical declarations and global bindings are not implemented yet",
    },
    BoundaryCase {
        description: "nested block lexical declaration",
        source: "(function(){ { const nested = 1; nested; } })()",
        rust_message: "lexical declarations in nested blocks and switch bodies are not implemented yet",
    },
    BoundaryCase {
        description: "switch-body lexical declaration",
        source: "(function(){ switch (0) { case 0: let nested = 1; nested; } })()",
        rust_message: "lexical declarations in nested blocks and switch bodies are not implemented yet",
    },
    BoundaryCase {
        description: "for-head lexical declaration",
        source: "(function(){ for (let index = 0; index < 1; index++) {} })()",
        rust_message: "lexical declarations in for heads are not implemented yet",
    },
    BoundaryCase {
        description: "lexical destructuring binding",
        source: "(function(){ const [value] = [1]; return value; })()",
        rust_message: "lexical destructuring bindings are not implemented yet",
    },
];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    (
        "duplicate body lexical",
        "(function(){\n  let value;\n  let value;\n})",
    ),
    (
        "var before body lexical",
        "(function(){\n  var value;\n  let value;\n})",
    ),
    (
        "var after body lexical",
        "(function(){\n  let value;\n  var value;\n})",
    ),
    (
        "parameter conflicts with body lexical",
        "(function(value){\n  let value;\n})",
    ),
    (
        "const requires initializer",
        "(function(){\n  const value;\n})",
    ),
    (
        "duplicate name in one declaration list",
        "(function(){\n  let value, value;\n})",
    ),
    ("let cannot bind itself", "(function(){\n  let let = 1;\n})"),
    (
        "strict eval lexical name",
        "(function(){\n  'use strict';\n  let eval = 1;\n})",
    ),
    (
        "single statement lexical declaration",
        "(function(){\n  if (true) let value = 1;\n})",
    ),
];

#[test]
fn ordinary_function_body_lexical_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP function-body lexical differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        assert_eq!(
            rust_value_observation(source, description),
            oracle_value_observation(&oracle, source, description),
            "function-body lexical value drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn lexical_tdz_and_readonly_cli_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical Error stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), source, description);
        let quickjs = run_cli(&oracle, source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}

#[test]
fn lexical_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in SYNTAX_ERROR_CASES {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), source, description);
        let quickjs = run_cli(&oracle, source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}

#[test]
fn unsupported_lexical_boundaries_remain_explicit_while_quickjs_accepts_them() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical boundary differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for case in BOUNDARY_CASES {
        let output = run_cli(&oracle, case.source, case.description);
        assert!(
            output.status.success(),
            "pinned QuickJS rejected {} ({:?}): {}",
            case.description,
            case.source,
            String::from_utf8_lossy(&output.stderr),
        );
        assert_eq!(
            rust_error_observation(case.source, case.description),
            format!("SyntaxError|{}", case.rust_message),
            "Rust boundary drifted for {} ({:?})",
            case.description,
            case.source,
        );
    }
}

fn rust_value_observation(source: &str, description: &str) -> String {
    let runtime = Runtime::new();
    let value = runtime
        .new_context()
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"));
    normalize_rust_value(value)
}

fn normalize_rust_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined|undefined".to_owned(),
        Value::Null => "object|null".to_owned(),
        Value::Bool(value) => format!("boolean|{value}"),
        Value::Int(value) => format!("number|{value}"),
        Value::Float(value) => format!("number|{value}"),
        Value::BigInt(value) => format!("bigint|{value}"),
        Value::String(value) => format!("string|{}", value.to_utf8_lossy()),
        Value::Object(_) => "object|<object>".to_owned(),
        Value::Symbol(_) => "symbol|<symbol>".to_owned(),
    }
}

fn oracle_value_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = format!(
        "var __qjo_value = ({source}); print(typeof __qjo_value + '|' + String(__qjo_value));"
    );
    let output = Command::new(oracle)
        .args(["-e", &script])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "pinned QuickJS rejected {description} ({source:?}): {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| {
            panic!("QuickJS emitted non-UTF-8 output for {description}: {error}")
        })
        .trim_end()
        .to_owned()
}

fn rust_error_observation(source: &str, description: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval_with_filename(source, "<cmdline>"),
        Err(RuntimeError::Exception),
        "Rust unexpectedly accepted {description}: {source:?}",
    );
    let Value::Object(error) = context
        .take_exception()
        .expect("take Rust exception")
        .expect("Rust exception is present")
    else {
        panic!("Rust did not materialize an Error object for {description}");
    };
    let name = error_string_property(&runtime, &mut context, &error, "name", description);
    let message = error_string_property(&runtime, &mut context, &error, "message", description);
    format!("{name}|{message}")
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

fn run_cli(program: &OsStr, source: &str, description: &str) -> Output {
    Command::new(program)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}
