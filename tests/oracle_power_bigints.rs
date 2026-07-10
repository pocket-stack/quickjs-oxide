use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::process::Command;

use quickjs_oxide::{Runtime, Value};

#[derive(Debug)]
struct PowerCase {
    base: String,
    exponent: String,
    source: String,
}

#[test]
fn bigint_power_matrix_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP BigInt power oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = power_cases();
    assert_eq!(
        cases.len(),
        725,
        "the BigInt power differential unexpectedly changed breadth"
    );
    let oracle_observations = run_oracle_batch(&oracle, &cases);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for (index, (case, oracle_observation)) in cases.iter().zip(&oracle_observations).enumerate() {
        let rust_value = context.eval(&case.source).unwrap_or_else(|error| {
            panic!(
                "Rust evaluation failed for BigInt power case {index} ({} ** {}, source {:?}): {error}",
                case.base, case.exponent, case.source
            )
        });
        let rust_observation = normalize_rust_bigint(&rust_value);
        assert_eq!(
            rust_observation, *oracle_observation,
            "BigInt power differential mismatch at case {index}: {} ** {} (source {:?})",
            case.base, case.exponent, case.source
        );
    }
}

fn power_cases() -> Vec<PowerCase> {
    // Values through i64::MIN/MAX exercise QuickJS's immediate BigInt path.
    // The neighboring and wider values exercise its heap representation. The
    // matrix deliberately retains both signs and several power-of-two edges.
    const BASES: &[&str] = &[
        "0",
        "1",
        "-1",
        "2",
        "-2",
        "3",
        "-3",
        "4",
        "-4",
        "5",
        "-5",
        "10",
        "-10",
        "63",
        "-63",
        "64",
        "-64",
        "127",
        "-127",
        "128",
        "-128",
        "255",
        "-255",
        "256",
        "-256",
        "2147483647",
        "-2147483648",
        "4294967295",
        "-4294967296",
        "9007199254740991",
        "-9007199254740991",
        "9223372036854775807",
        "-9223372036854775808",
        "9223372036854775808",
        "-9223372036854775809",
        "18446744073709551615",
        "-18446744073709551616",
        "123456789012345678901234567890",
        "-123456789012345678901234567890",
        "170141183460469231731687303715884105727",
        "-170141183460469231731687303715884105728",
    ];
    const EXPONENTS: &[&str] = &[
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "15", "16", "17", "31", "32", "33",
    ];

    let mut cases = Vec::new();
    let mut seen = HashSet::new();
    for base in BASES {
        for exponent in EXPONENTS {
            push_case(&mut cases, &mut seen, base, exponent);
        }
    }

    // Keep these results comfortably below the allocation boundary while
    // forcing exact decimal conversion of thousands of bits. Heap exponents
    // are meaningful only for the 0/1/-1 shortcuts here; other such cases are
    // allocation errors and belong to the focused error differential.
    for (base, exponent) in [
        ("2", "63"),
        ("2", "64"),
        ("2", "65"),
        ("2", "127"),
        ("2", "128"),
        ("2", "255"),
        ("2", "256"),
        ("2", "1024"),
        ("2", "4096"),
        ("2", "16384"),
        ("-2", "63"),
        ("-2", "64"),
        ("-2", "127"),
        ("-2", "128"),
        ("-2", "1023"),
        ("-2", "1024"),
        ("3", "256"),
        ("3", "512"),
        ("3", "1024"),
        ("10", "128"),
        ("10", "1024"),
        ("10", "4096"),
        ("9223372036854775808", "64"),
        ("-9223372036854775809", "63"),
        ("0", "9223372036854775808"),
        ("1", "9223372036854775808"),
        ("-1", "9223372036854775808"),
        ("-1", "9223372036854775809"),
    ] {
        push_case(&mut cases, &mut seen, base, exponent);
    }

    cases
}

fn push_case(
    cases: &mut Vec<PowerCase>,
    seen: &mut HashSet<(String, String)>,
    base: &str,
    exponent: &str,
) {
    if !seen.insert((base.to_owned(), exponent.to_owned())) {
        return;
    }
    cases.push(PowerCase {
        base: base.to_owned(),
        exponent: exponent.to_owned(),
        source: format!("({base}n) ** ({exponent}n)"),
    });
}

fn run_oracle_batch(oracle: &OsStr, cases: &[PowerCase]) -> Vec<String> {
    let mut script = String::from("var __qjo_power_cases = [\n");
    for case in cases {
        writeln!(script, "[{}n, {}n],", case.base, case.exponent)
            .expect("writing to a String cannot fail");
    }
    script.push_str(
        r#"];
for (var __qjo_index = 0; __qjo_index < __qjo_power_cases.length; __qjo_index++) {
    var __qjo_case = __qjo_power_cases[__qjo_index];
    print(__qjo_index + "|" + String(__qjo_case[0] ** __qjo_case[1]));
}
"#,
    );

    let output = Command::new(oracle)
        .arg("-e")
        .arg(script)
        .output()
        .unwrap_or_else(|error| panic!("could not execute QJS_ORACLE BigInt power batch: {error}"));
    assert!(
        output.status.success(),
        "QJS_ORACLE BigInt power batch failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout)
        .expect("QJS_ORACLE BigInt power batch emitted non-UTF-8 output");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        cases.len(),
        "QJS_ORACLE BigInt power batch emitted the wrong number of observations"
    );
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            line.strip_prefix(&format!("{index}|"))
                .unwrap_or_else(|| {
                    panic!("QJS_ORACLE BigInt power observation {index} was malformed: {line:?}")
                })
                .to_owned()
        })
        .collect()
}

fn normalize_rust_bigint(value: &Value) -> String {
    match value {
        Value::BigInt(value) => value.to_string(),
        other => panic!("BigInt power unexpectedly produced a non-BigInt value: {other:?}"),
    }
}
