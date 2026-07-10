use std::ffi::OsStr;
use std::fmt::Write as _;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{Runtime, Value};

#[derive(Clone, Copy, Debug)]
enum NumericKind {
    Number,
    BigInt,
}

#[derive(Debug)]
struct UpdateCase {
    label: String,
    source: String,
    kind: NumericKind,
}

#[test]
fn update_numeric_matrix_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP update numeric oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = update_cases();
    assert_eq!(
        cases.len(),
        324,
        "the update numeric differential unexpectedly changed breadth"
    );
    let oracle_observations = run_oracle_batch(&oracle, &cases);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for (index, (case, oracle_observation)) in cases.iter().zip(&oracle_observations).enumerate() {
        let rust_value = context.eval(&case.source).unwrap_or_else(|error| {
            panic!(
                "Rust evaluation failed for update case {index} ({}, source {:?}): {error}",
                case.label, case.source
            )
        });
        let rust_observation = normalize_rust_numeric(&rust_value, case.kind);
        assert_eq!(
            rust_observation, *oracle_observation,
            "update numeric differential mismatch at case {index}: {} (source {:?})",
            case.label, case.source
        );
    }
}

fn update_cases() -> Vec<UpdateCase> {
    // Every input is exercised through both prefix operators. Postfix cases
    // are split into explicit result and stored-value observations so a test
    // cannot accidentally return the replacement for both positions.
    const NUMBER_INPUTS: &[(&str, &str)] = &[
        ("negative zero", "-0"),
        ("positive zero", "0"),
        ("false", "false"),
        ("true", "true"),
        ("null", "null"),
        ("undefined", "void 0"),
        ("nan", "0 / 0"),
        ("positive infinity", "1 / 0"),
        ("negative infinity", "-1 / 0"),
        ("negative i32 below", "-2147483649"),
        ("negative i32 boundary", "-2147483648"),
        ("negative i32 above", "-2147483647"),
        ("positive i32 below", "2147483646"),
        ("positive i32 boundary", "2147483647"),
        ("positive i32 above", "2147483648"),
        ("negative safe integer below", "-9007199254740992"),
        ("negative safe integer boundary", "-9007199254740991"),
        ("negative safe integer above", "-9007199254740990"),
        ("positive safe integer below", "9007199254740990"),
        ("positive safe integer boundary", "9007199254740991"),
        ("positive two to 53", "9007199254740992"),
        ("positive two to 53 next representable", "9007199254740994"),
        ("smallest positive subnormal", "5e-324"),
        ("smallest negative subnormal", "-5e-324"),
        ("largest positive subnormal", "2.225073858507201e-308"),
        ("minimum positive normal", "2.2250738585072014e-308"),
        ("positive fraction", "1.5"),
        ("negative fraction", "-1.5"),
        ("maximum finite", "1.7976931348623157e308"),
        ("negative maximum finite", "-1.7976931348623157e308"),
        ("leading-zero string", "'01'"),
        ("negative-zero string", "' -0 '"),
        ("fraction string", "'1.5'"),
        ("negative fraction string", "'-2.5'"),
        ("empty string", "''"),
        ("whitespace string", "'   '"),
        ("hexadecimal string", "'0x10'"),
        ("infinity string", "'Infinity'"),
        ("nan string", "'not-a-number'"),
    ];

    const BIGINT_INPUTS: &[(&str, &str)] = &[
        ("zero bigint", "0n"),
        ("one bigint", "1n"),
        ("negative one bigint", "-1n"),
        ("positive i32 bigint", "2147483647n"),
        ("positive i32 overflow bigint", "2147483648n"),
        ("negative i32 bigint", "-2147483648n"),
        ("negative i32 overflow bigint", "-2147483649n"),
        ("short max minus one", "9223372036854775806n"),
        ("short max", "9223372036854775807n"),
        ("heap above short max", "9223372036854775808n"),
        ("short min plus one", "-9223372036854775807n"),
        ("short min", "-9223372036854775808n"),
        ("heap below short min", "-9223372036854775809n"),
        ("wide positive heap", "123456789012345678901234567890n"),
        ("wide negative heap", "-123456789012345678901234567890n"),
    ];

    let mut cases = Vec::new();
    for (label, initial) in NUMBER_INPUTS {
        push_update_cases(&mut cases, label, initial, NumericKind::Number);
    }
    for (label, initial) in BIGINT_INPUTS {
        push_update_cases(&mut cases, label, initial, NumericKind::BigInt);
    }
    cases
}

fn push_update_cases(cases: &mut Vec<UpdateCase>, label: &str, initial: &str, kind: NumericKind) {
    for (operation, expression) in [
        ("prefix increment", "++value"),
        ("prefix decrement", "--value"),
    ] {
        cases.push(UpdateCase {
            label: format!("{label} {operation} result"),
            source: format!("(function(){{ var value = {initial}; return {expression}; }})()"),
            kind,
        });
    }

    for (operation, expression) in [
        ("postfix increment", "value++"),
        ("postfix decrement", "value--"),
    ] {
        cases.push(UpdateCase {
            label: format!("{label} {operation} old"),
            source: format!(
                "(function(){{ var value = {initial}; var old = {expression}; return old; }})()"
            ),
            kind,
        });
        cases.push(UpdateCase {
            label: format!("{label} {operation} new"),
            source: format!(
                "(function(){{ var value = {initial}; {expression}; return value; }})()"
            ),
            kind,
        });
    }
}

fn run_oracle_batch(oracle: &OsStr, cases: &[UpdateCase]) -> Vec<String> {
    let mut script = String::from(
        r#"
function __qjo_hex32(value) {
    return ("00000000" + value.toString(16)).slice(-8);
}
function __qjo_observe_numeric(value) {
    if (typeof value === "bigint") return "bigint|" + String(value);
    if (typeof value !== "number") return typeof value + "|" + String(value);
    var buffer = new ArrayBuffer(8);
    var view = new DataView(buffer);
    view.setFloat64(0, value, false);
    var bits = __qjo_hex32(view.getUint32(0, false)) +
               __qjo_hex32(view.getUint32(4, false));
    return "number|" + bits + "|" + String(value);
}
var __qjo_update_cases = [
"#,
    );
    for case in cases {
        writeln!(script, "function(){{ return {}; }},", case.source)
            .expect("writing to a String cannot fail");
    }
    script.push_str(
        r#"];
for (var __qjo_index = 0; __qjo_index < __qjo_update_cases.length; __qjo_index++) {
    print(__qjo_index + "|" + __qjo_observe_numeric(__qjo_update_cases[__qjo_index]()));
}
"#,
    );

    let output = Command::new(oracle)
        .arg("-e")
        .arg(script)
        .output()
        .unwrap_or_else(|error| panic!("could not execute QJS_ORACLE update batch: {error}"));
    assert!(
        output.status.success(),
        "QJS_ORACLE update batch failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout =
        String::from_utf8(output.stdout).expect("QJS_ORACLE update batch emitted non-UTF-8 output");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        cases.len(),
        "QJS_ORACLE update batch emitted the wrong number of observations"
    );
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            line.strip_prefix(&format!("{index}|"))
                .unwrap_or_else(|| {
                    panic!("QJS_ORACLE update observation {index} was malformed: {line:?}")
                })
                .to_owned()
        })
        .collect()
}

fn normalize_rust_numeric(value: &Value, expected_kind: NumericKind) -> String {
    match (expected_kind, value) {
        (NumericKind::Number, Value::Int(value)) => normalize_rust_number(f64::from(*value)),
        (NumericKind::Number, Value::Float(value)) => normalize_rust_number(*value),
        (NumericKind::BigInt, Value::BigInt(value)) => format!("bigint|{value}"),
        (NumericKind::Number, other) => {
            panic!("Number update unexpectedly produced a non-Number value: {other:?}")
        }
        (NumericKind::BigInt, other) => {
            panic!("BigInt update unexpectedly produced a non-BigInt value: {other:?}")
        }
    }
}

fn normalize_rust_number(value: f64) -> String {
    format!(
        "number|{:016x}|{}",
        value.to_bits(),
        number_to_string(value)
    )
}
