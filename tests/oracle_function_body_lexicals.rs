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
    (
        "nested block lexicals shadow without changing the outer binding",
        "(function(){ let value = 1, inner; { let value = 40; const delta = 2; value += delta; inner = value; } return value + '|' + inner; })()",
    ),
    (
        "captured block lexical outlives its scope",
        "(function(){ let read; { const value = 42; read = function(){ return value; }; } return read(); })()",
    ),
    (
        "block continue closes one capture before the next scope entry",
        "(function(){ let first, second; let i = 0; while (i < 2) { let value = ++i; let read = function(){ return value; }; if (i === 1) { first = read; continue; } second = read; } return first() * 10 + second(); })()",
    ),
    (
        "labeled block break preserves its captured lexical",
        "(function(){ let read; done: { const value = 42; read = function(){ return value; }; break done; } return read(); })()",
    ),
    (
        "nested block lexical may shadow a parameter",
        "(function(value){ { let value = 42; return value; } })(1)",
    ),
    (
        "QuickJS keeps only the first sibling var declaration scope",
        "(function(){ { var value; } { var value; let value; } return value === undefined; })()",
    ),
    (
        "Function constructor nested lexical closure",
        "Function('let read; { const value = 42; read = function(){ return value; }; } return read();')()",
    ),
    (
        "nested array lexical bindings compose defaults elisions and rest",
        "(function(){const [[first]=[40],,[...[,second]]]=[undefined,0,[1,2]];return first+second;})()",
    ),
    (
        "function body object lexical binding",
        "(function(){const [{value}] = [{value:1}];return value;})()",
    ),
    (
        "nested block object lexical binding",
        "(function(){{const [{value}] = [{value:2}];return value;}})()",
    ),
    (
        "switch object lexical binding",
        "(function(){switch(0){case 0:const [{value}] = [{value:3}];return value;}})()",
    ),
    (
        "switch fallthrough shares one lexical environment",
        "(function(){ let read; switch (0) { case 0: let value = 40; read = function(){ return value; }; case 1: value++; break; } return read(); })()",
    ),
    (
        "switch lexical shadows without changing the outer binding",
        "(function(){ let value = 1, inner; switch (0) { case 0: let value = 42; inner = value; break; } return value + '|' + inner; })()",
    ),
    (
        "switch continue closes one capture before the next scope entry",
        "(function(){ let first, second; let i = 0; outer: while (i < 2) { switch (0) { case 0: let value = ++i; let read = function(){ return value; }; if (i === 1) { first = read; continue outer; } second = read; break; } } return first() * 10 + second(); })()",
    ),
];

const SCRIPT_SCOPE_VALUE_CASES: &[(&str, &str, &str)] = &[
    (
        "script nested block lexical",
        "Function.scopeLog = ''; { let value = 40; const delta = 2; Function.scopeLog += value + delta; }",
        "Function.scopeLog + '|' + typeof value",
    ),
    (
        "script nested switch lexical",
        "Function.scopeLog = ''; switch (0) { case 0: let value = 40; const delta = 2; Function.scopeLog += value + delta; break; }",
        "Function.scopeLog + '|' + typeof value",
    ),
    (
        "script captured block cell survives scope exit",
        "Function.saved = undefined; { let value = 40; Function.saved = function(){ return ++value; }; }",
        "Function.saved() * 100 + Function.saved()",
    ),
    (
        "script captured switch cell survives scope exit",
        "Function.saved = undefined; switch (0) { case 0: let value = 40; Function.saved = function(){ return ++value; }; break; }",
        "Function.saved() * 100 + Function.saved()",
    ),
];

const BODY_SCOPE_ERROR_CASES: &[(&str, &str)] = &[
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

const NESTED_SCOPE_ERROR_CASES: &[(&str, &str)] = &[
    (
        "nested block temporal dead zone read",
        "(function blockTdz(){\n  {\n    return value;\n    let value = 1;\n  }\n})()",
    ),
    (
        "nested block typeof temporal dead zone read",
        "(function blockTypeofTdz(){ { return typeof value; let value = 1; } })()",
    ),
    (
        "nested block direct const write",
        "(function blockReadonly(){ { const value = 1; value = 2; } })()",
    ),
    (
        "captured block const write after scope exit",
        "(function blockReadonlyOuter(){ let write; { const value = 1; write = function blockReadonlyInner(){ value = 2; }; } return write; })()()",
    ),
    (
        "block exit participates in late readonly source mapping",
        "(function blockReadonlyMarker(){\n  {\n    value = 1;\n    const value = 2;\n  }\n})()",
    ),
    (
        "block late readonly mapping crosses a lowered scope exit",
        "(function blockReadonlyAfterScope(){\n  {\n    value = 1;\n    const value = 2;\n  }\n  99;\n})()",
    ),
    (
        "block branch removal shares lowered scope label state",
        "(function blockReadonlyBranches(flag){\n  {\n    if (flag) value = 1; else value = 2;\n    const value = 3;\n    99;\n  }\n})(false)",
    ),
    (
        "switch case selector observes the shared temporal dead zone",
        "(function switchSelectorTdz(){\n  switch (0) {\n    case value:\n      let value = 0;\n      return value;\n  }\n})()",
    ),
    (
        "unselected switch declaration creates a cross-case temporal dead zone",
        "(function switchCaseTdz(){ switch (1) { case 0: let value = 0; break; case 1: return value; } })()",
    ),
    (
        "switch direct const write",
        "(function switchReadonly(){ switch (0) { case 0: const value = 1; value = 2; } })()",
    ),
    (
        "captured switch const write after scope exit",
        "(function switchReadonlyOuter(){ let write; switch (0) { case 0: const value = 1; write = function switchReadonlyInner(){ value = 2; }; break; } return write; })()()",
    ),
    (
        "switch exit participates in late readonly source mapping",
        "(function switchReadonlyMarker(){\n  switch (0) {\n    case 0:\n      value = 1;\n      const value = 2;\n  }\n})()",
    ),
    (
        "switch late readonly mapping observes the following case operation",
        "(function switchReadonlyAfterWrite(){\n  switch (0) {\n    case 0:\n      value = 1;\n      const value = 2;\n      99;\n  }\n})()",
    ),
    (
        "switch case label bounds late readonly source mapping",
        "(function switchReadonlyCaseLabel(){\n  switch (0) {\n    case 0:\n      value = 1;\n    case 1:\n      const value = 2;\n      99;\n  }\n})()",
    ),
];

struct BoundaryCase {
    description: &'static str,
    source: &'static str,
    rust_message: &'static str,
}

const BOUNDARY_CASES: &[BoundaryCase] = &[
    BoundaryCase {
        description: "function body object rest lexical binding",
        source: "(function(){ const {...rest} = {value:1}; return rest.value; })()",
        rust_message: "object rest destructuring bindings are not implemented yet",
    },
    BoundaryCase {
        description: "nested block object rest lexical binding",
        source: "(function(){ { const [{...rest}] = [{value:1}]; return rest.value; } })()",
        rust_message: "object rest destructuring bindings are not implemented yet",
    },
    BoundaryCase {
        description: "switch object rest lexical binding",
        source: "(function(){ switch (0) { case 0: const {...rest} = {value:1}; return rest.value; } })()",
        rust_message: "object rest destructuring bindings are not implemented yet",
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
    (
        "duplicate nested block lexical",
        "(function(){\n  { let value; const value = 1; }\n})",
    ),
    (
        "var before nested block lexical",
        "(function(){\n  { var value; let value; }\n})",
    ),
    (
        "var after nested block lexical",
        "(function(){\n  { let value; var value; }\n})",
    ),
    (
        "descendant var conflicts with parent block lexical",
        "(function(){\n  { let value; { var value; } }\n})",
    ),
    (
        "duplicate switch lexical across cases",
        "(function(){\n  switch (0) { case 0: let value; break; case 1: const value = 1; }\n})",
    ),
    (
        "switch var before lexical across cases",
        "(function(){\n  switch (0) { case 0: var value; break; case 1: let value; }\n})",
    ),
    (
        "switch var after lexical across cases",
        "(function(){\n  switch (0) { case 0: let value; break; case 1: var value; }\n})",
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
    for &(description, setup, observation) in SCRIPT_SCOPE_VALUE_CASES {
        let rust_source = format!("{setup}\n({observation})");
        assert_eq!(
            rust_value_observation(&rust_source, description),
            oracle_script_value_observation(&oracle, setup, observation, description),
            "script lexical value drifted for {description}: {rust_source:?}",
        );
    }
}

#[test]
fn lexical_tdz_and_readonly_cli_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical Error stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in BODY_SCOPE_ERROR_CASES
        .iter()
        .chain(NESTED_SCOPE_ERROR_CASES)
    {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), source, description);
        let quickjs = run_cli(&oracle, source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}

#[test]
fn nested_lexical_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP nested lexical StripDebug differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in NESTED_SCOPE_ERROR_CASES {
        let rust = run_cli_with_options(
            env!("CARGO_BIN_EXE_qjs").as_ref(),
            &["-s"],
            source,
            description,
        );
        let quickjs = run_cli_with_options(&oracle, &["-s"], source, description);
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

fn oracle_script_value_observation(
    oracle: &OsStr,
    setup: &str,
    observation: &str,
    description: &str,
) -> String {
    let script = format!(
        "{setup}\nvar __qjo_value = ({observation}); print(typeof __qjo_value + '|' + String(__qjo_value));"
    );
    let output = Command::new(oracle)
        .args(["-e", &script])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "pinned QuickJS rejected {description} ({script:?}): {}",
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
    run_cli_with_options(program, &[], source, description)
}

fn run_cli_with_options(
    program: &OsStr,
    options: &[&str],
    source: &str,
    description: &str,
) -> Output {
    Command::new(program)
        .args(options)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}
