use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, Value};

#[test]
fn source_member_reads_and_intrinsic_method_calls_match_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP member-read differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = [
        ("fixed global field", "Function.name"),
        ("computed global field", "Function['name']"),
        (
            "fixed and computed identity",
            "Function.prototype === Function['prototype']",
        ),
        (
            "chained constructor identity",
            "Function['prototype'].constructor === Function",
        ),
        ("chained member type", "typeof Function.prototype.toString"),
        ("fixed method receiver", "Function().toString()"),
        ("computed method receiver", "Function()['toString']()"),
        (
            "new member constructor head",
            "new Function.prototype.constructor('return 42')()",
        ),
        (
            "new parenthesized member constructor head",
            "new (Function.prototype.constructor)('return 43')()",
        ),
        ("string own length", "'abc'.length"),
        ("string own indexed property", "'abc'[1]"),
    ];

    for (description, source) in cases {
        assert_eq!(
            rust_observation(source),
            oracle_observation(&oracle, source, description),
            "QuickJS member-read behavior drifted for {description}: {source:?}",
        );
    }
}

fn rust_observation(source: &str) -> String {
    let runtime = Runtime::new();
    let value = runtime
        .new_context()
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust source failed for {source:?}: {error}"));
    match value {
        Value::Undefined => "undefined:undefined\n".to_owned(),
        Value::Null => "object:null\n".to_owned(),
        Value::Bool(value) => format!("boolean:{value}\n"),
        Value::Int(value) => format!("number:{value}\n"),
        Value::Float(value) => format!("number:{value}\n"),
        Value::BigInt(value) => format!("bigint:{value}\n"),
        Value::String(value) => format!("string:{}\n", value.to_utf8_lossy()),
        Value::Symbol(_) => "symbol:<symbol>\n".to_owned(),
        Value::Object(_) => "object:<object>\n".to_owned(),
    }
}

fn oracle_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let wrapper = format!("const value = ({source}); print(typeof value + ':' + String(value));");
    let output = Command::new(oracle)
        .args(["-e", &wrapper])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS failed {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS emitted non-UTF-8 stdout: {error}"))
}
