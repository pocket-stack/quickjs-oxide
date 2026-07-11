use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{EvalOptions, Runtime, Value};

const EXPRESSIONS: &[&str] = &[
    "parseInt()",
    "parseInt(undefined)",
    "parseInt(null)",
    "parseInt(true)",
    "parseInt(false)",
    "parseInt('0x10')",
    "parseInt('0x10', 10)",
    "parseInt('10', 2)",
    "parseInt('10', 4294967298)",
    "parseInt('10102', 2)",
    "parseInt('zZ!', 36)",
    "parseInt('1e3', 10)",
    "1 / parseInt('-0')",
    "parseInt('300000000000000031025361333325263798273', 10)",
    "parseFloat()",
    "parseFloat(null)",
    "parseFloat(true)",
    "parseFloat('0x10')",
    "parseFloat('1.25tail')",
    "parseFloat('1e')",
    "parseFloat('.5e+2')",
    "1 / parseFloat('-1e-9999')",
    "parseFloat('1.7976931348623159e308')",
    "parseFloat('1.0000000000000001110223024625156540423631668090820313')",
];

#[test]
fn global_number_parsers_match_pinned_quickjs_through_source_execution() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP global numeric parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let expression = format!("'' + {}", EXPRESSIONS.join(" + '|' + "));
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::String(rust) = context.eval(&expression).unwrap() else {
        panic!("Rust numeric parser probe did not return a string");
    };
    assert_eq!(
        rust.to_utf8_lossy(),
        oracle_observation(&oracle, &expression)
    );
}

fn oracle_observation(oracle: &OsStr, expression: &str) -> String {
    let source = format!("print({expression})");
    let output = Command::new(oracle)
        .args(["-e", &source])
        .output()
        .expect("run QuickJS global numeric parser oracle");
    assert!(
        output.status.success(),
        "QuickJS global numeric parser oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout)
        .expect("QuickJS global numeric parser oracle emitted non-UTF-8 output");
    output
        .strip_suffix('\n')
        .map(str::to_owned)
        .unwrap_or(output)
}

#[test]
fn parser_function_names_are_stable_in_native_error_stacks() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP numeric parser stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let source = "parseInt('10', 1n)";
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert!(
        context
            .eval_with_options(source, &EvalOptions::new("<cmdline>"))
            .is_err()
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("parseInt BigInt radix did not throw an Error object");
    };
    let name_key = runtime.intern_property_key("name").unwrap();
    let Value::String(name) = context.get_property(&error, &name_key).unwrap() else {
        panic!("parseInt BigInt radix Error.name was not a string");
    };
    let message_key = runtime.intern_property_key("message").unwrap();
    let Value::String(message) = context.get_property(&error, &message_key).unwrap() else {
        panic!("parseInt BigInt radix Error.message was not a string");
    };
    let stack_key = runtime.intern_property_key("stack").unwrap();
    let Value::String(stack) = context.get_property(&error, &stack_key).unwrap() else {
        panic!("parseInt BigInt radix Error.stack was not a string");
    };
    let rust = format!(
        "{}: {}\n{}",
        name.to_utf8_lossy(),
        message.to_utf8_lossy(),
        stack.to_utf8_lossy()
    );
    let output = Command::new(oracle)
        .args(["-e", source])
        .output()
        .expect("run QuickJS numeric parser stack oracle");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(rust, String::from_utf8(output.stderr).unwrap());
}
