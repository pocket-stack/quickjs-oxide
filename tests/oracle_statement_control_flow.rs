use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const ORACLE_NORMALIZER: &str = r#"
var __qjo_type = typeof __qjo_value;
if (__qjo_type === "number") {
    if (__qjo_value !== __qjo_value) {
        print("number|NaN");
    } else if (__qjo_value === 0 && 1 / __qjo_value === -Infinity) {
        print("number|-0");
    } else if (__qjo_value === Infinity) {
        print("number|Infinity");
    } else if (__qjo_value === -Infinity) {
        print("number|-Infinity");
    } else {
        print("number|" + String(__qjo_value));
    }
} else if (__qjo_type === "string") {
    var __qjo_units = "";
    for (var __qjo_index = 0; __qjo_index < __qjo_value.length; __qjo_index++) {
        var __qjo_hex = __qjo_value.charCodeAt(__qjo_index).toString(16);
        if (__qjo_index !== 0) __qjo_units += ",";
        __qjo_units += ("0000" + __qjo_hex).slice(-4);
    }
    print("string|" + __qjo_value.length + "|" + __qjo_units);
} else if (__qjo_value === null) {
    print("object|null");
} else {
    print(__qjo_type + "|" + String(__qjo_value));
}
"#;

const VALUE_CASES: &[(&str, &str)] = &[
    ("empty script", ""),
    ("empty statement", ";"),
    ("empty statement preserves completion", "1; ;"),
    ("empty block preserves completion", "1; {}"),
    ("empty nested block preserves completion", "1; {{;}}"),
    ("block expression updates completion", "1; { 2; }"),
    ("nested block keeps its last expression", "{ 1; { 2; {} } }"),
    ("deep empty blocks preserve completion", "7; {{{}}}"),
    ("false if resets completion", "1; if (false) 2;"),
    ("taken empty if resets completion", "1; if (true) {}"),
    ("taken block updates completion", "1; if (true) { 2; }"),
    ("false branch selects else", "if (false) 1; else 2;"),
    ("true branch skips else", "if (true) 1; else 2;"),
    (
        "dangling else binds to nearest if",
        "if (true) if (false) 1; else 2;",
    ),
    (
        "outer else follows completed nested if",
        "if (false) if (true) 1; else 2; else 3;",
    ),
    ("taken empty statement", "if (true) ; else 2;"),
    ("empty else statement", "if (false) 1; else ;"),
    ("comma condition", "if ((0, 1)) 8; else 9;"),
    (
        "nested if resets a previous branch completion",
        "if (true) { 1; if (false) 2; }",
    ),
    (
        "function if return false",
        "(function(x){ if (x) return 1; else return 2; })(0)",
    ),
    (
        "function if return true",
        "(function(x){ if (x) return 1; else return 2; })(1)",
    ),
    (
        "nested branch return",
        "(function(){ if (true) { if (true) { return 42; } return 1; } return 0; })()",
    ),
    (
        "dead branch var is function scoped",
        "(function(){ if (false) { var x = 1; } return typeof x; })()",
    ),
    (
        "block var remains function scoped",
        "(function(){ { var x = 4; } return x; })()",
    ),
    (
        "selected var initializer",
        "(function(flag){ if (flag) { var x = 3; } return typeof x + '|' + x; })(true)",
    ),
    (
        "dead throw is not executed",
        "(function(){ if (false) throw 1; return 42; })()",
    ),
    (
        "return restricted production in a branch",
        "(function(){ if (true) return\n42; return 7; })()",
    ),
    (
        "function expression statements stay discarded",
        "(function(flag){ if (flag) 1; else 2; })()",
    ),
    (
        "condition and selected branch order",
        "(function(){ var log=''; var c=function(){log=log+'c';return true;}; var y=function(){log=log+'y';}; var n=function(){log=log+'n';}; if(c()) y(); else n(); return log; })()",
    ),
    (
        "condition and false branch order",
        "(function(){ var log=''; var c=function(){log=log+'c';return false;}; var y=function(){log=log+'y';}; var n=function(){log=log+'n';}; if(c()) y(); else n(); return log; })()",
    ),
    (
        "unselected branch has no effects",
        "(function(){ var x=0; if (true) x=1; else missing(); return x; })()",
    ),
    (
        "object condition does not coerce",
        "(function(){ var log=''; var o=function(){}; o.valueOf=function(){log=log+'v';return 0;}; if(o) log=log+'t'; return log; })()",
    ),
    (
        "nested condition side effect order",
        "(function(){ var log=''; if((log=log+'a',true)) if((log=log+'b',false)) log=log+'c'; else log=log+'d'; return log; })()",
    ),
    (
        "block string is not a directive",
        "{ 'use strict'; '\\1'; }",
    ),
    (
        "block before string prevents directive prologue",
        "{}; 'use strict'; '\\1';",
    ),
    (
        "function block string is not a directive",
        "(function(){ { 'use strict'; } return '\\1'.charCodeAt(0); })()",
    ),
    (
        "block strict spelling leaves legacy octal sloppy",
        "{ 'use strict'; 010; }",
    ),
    (
        "if strict spelling leaves legacy octal sloppy",
        "if (true) 'use strict'; 010;",
    ),
    ("LF participates in if ASI", "if (false)\n1;\nelse\n2;"),
    ("CR participates in if ASI", "if (false)\r1;\relse\r2;"),
    (
        "CRLF participates in if ASI",
        "if (false)\r\n1;\r\nelse\r\n2;",
    ),
    (
        "line separator participates in if ASI",
        "if (false)\u{2028}1;\u{2028}else\u{2028}2;",
    ),
    (
        "paragraph separator participates in if ASI",
        "if (false)\u{2029}1;\u{2029}else\u{2029}2;",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "missing condition left parenthesis wins over later lex error",
        "if true 1; \"unterminated",
    ),
    (
        "missing condition right parenthesis wins over later lex error",
        "if (true 0; \"unterminated",
    ),
    ("missing consequent", "if (true)"),
    ("else cannot be a consequent", "if (true) else 1;"),
    ("orphan else", "else 1;"),
    ("unterminated then block", "if (true) {"),
    ("unterminated nonempty block", "if (true) { 1;"),
    ("stray right brace", "if (true) {} }"),
    ("stray right brace after root expression", "1 }"),
    ("stray right brace after if expression", "if (true) 1 }"),
    (
        "stray right brace after nested if expression",
        "if (false) if (true) 1 }",
    ),
    ("missing else branch", "if (true) {} else"),
    (
        "return remains an early error in a dead branch",
        "if (false) return 1;",
    ),
    (
        "escaped reserved binding remains an early error in a dead branch",
        "(function(){ if (false) { var \\u0069f=1; } })()",
    ),
    (
        "raw malformed escape wins before later lex error",
        "if (true) \\u{}; \"unterminated",
    ),
    (
        "dead return wins before a later lex error",
        "if (false) { return 1; \"unterminated",
    ),
    (
        "dead branch still scans its reached lexical error",
        "if (false) { \"unterminated",
    ),
    (
        "strict program directive reaches a block legacy escape",
        "\"use strict\"; { \"\\1\"; }",
    ),
    (
        "strict function directive reaches a block legacy escape",
        "(function(){ \"use strict\"; { return \"\\1\"; } })()",
    ),
    (
        "LF throw restriction reports the following token",
        "if (true) throw\n1;",
    ),
    (
        "CR throw restriction uses QuickJS debug coordinates",
        "if (true) throw\r1;",
    ),
    (
        "line separator throw restriction uses QuickJS debug coordinates",
        "if (true) throw\u{2028}1;",
    ),
];

#[test]
fn statement_control_flow_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP statement-control-flow differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let value = context
            .eval(source)
            .unwrap_or_else(|error| panic!("Rust rejected {description:?} ({source:?}): {error}"));
        assert_eq!(
            normalize_rust_value(&value),
            oracle_value_observation(&oracle, source, description),
            "value mismatch for {description:?} ({source:?})"
        );
    }
}

#[test]
fn statement_control_flow_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP statement diagnostic differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        assert_eq!(
            rust_error_observation(source),
            oracle_error_observation(&oracle, source),
            "diagnostic mismatch for {description:?} ({source:?})"
        );
    }
}

fn oracle_value_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = format!("var __qjo_value = std.evalScript(scriptArgs[0]);\n{ORACLE_NORMALIZER}");
    let output = Command::new(oracle)
        .args(["--std", "-e", &script, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description:?}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS rejected {description:?} ({source:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS value output was not UTF-8")
        .trim_end()
        .to_owned()
}

fn rust_error_observation(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    take_rust_error(&runtime, &mut context)
}

fn take_rust_error(runtime: &Runtime, context: &mut Context) -> String {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Rust parser did not materialize an Error object");
    };
    let read = |context: &mut Context, name: &str| {
        let key = runtime.intern_property_key(name).unwrap();
        context.get_property(&error, &key).unwrap()
    };
    let Value::String(name) = read(context, "name") else {
        panic!("Rust Error.name was not a string");
    };
    let Value::String(message) = read(context, "message") else {
        panic!("Rust Error.message was not a string");
    };
    let Value::Int(line) = read(context, "lineNumber") else {
        panic!("Rust Error.lineNumber was not an integer");
    };
    let Value::Int(column) = read(context, "columnNumber") else {
        panic!("Rust Error.columnNumber was not an integer");
    };
    format!(
        "{}|{}|{line}:{column}",
        name.to_utf8_lossy(),
        message.to_utf8_lossy()
    )
}

fn oracle_error_observation(oracle: &OsStr, source: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {source:?}: {error}"));
    assert!(!output.status.success(), "QuickJS accepted {source:?}");
    let stderr = String::from_utf8(output.stderr).expect("QuickJS error output was not UTF-8");
    let mut lines = stderr.lines();
    let first = lines
        .find(|line| line.starts_with("SyntaxError: "))
        .unwrap_or_else(|| panic!("QuickJS emitted no SyntaxError for {source:?}: {stderr}"));
    let location = lines
        .find_map(|line| line.trim().strip_prefix("at <cmdline>:"))
        .unwrap_or_else(|| panic!("QuickJS emitted no location for {source:?}: {stderr}"));
    format!(
        "SyntaxError|{}|{location}",
        first.strip_prefix("SyntaxError: ").unwrap()
    )
}

fn normalize_rust_value(value: &Value) -> String {
    match value {
        Value::Undefined => "undefined|undefined".to_owned(),
        Value::Null => "object|null".to_owned(),
        Value::Bool(value) => format!("boolean|{value}"),
        Value::Int(value) => normalize_number(f64::from(*value)),
        Value::Float(value) => normalize_number(*value),
        Value::BigInt(value) => format!("bigint|{value}"),
        Value::String(value) => {
            let units = value
                .utf16_units()
                .map(|unit| format!("{unit:04x}"))
                .collect::<Vec<_>>()
                .join(",");
            format!("string|{}|{units}", value.len())
        }
        Value::Symbol(_) => "symbol|<identity>".to_owned(),
        Value::Object(_) => "object|<identity>".to_owned(),
    }
}

#[allow(clippy::float_cmp)]
fn normalize_number(value: f64) -> String {
    if value.is_nan() {
        "number|NaN".to_owned()
    } else if value == 0.0 && value.is_sign_negative() {
        "number|-0".to_owned()
    } else if value == f64::INFINITY {
        "number|Infinity".to_owned()
    } else if value == f64::NEG_INFINITY {
        "number|-Infinity".to_owned()
    } else {
        format!("number|{}", number_to_string(value))
    }
}
