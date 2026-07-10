use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{Runtime, Value};

#[derive(Debug)]
struct PowerCase {
    base: f64,
    exponent: f64,
    source: String,
}

#[test]
fn number_power_matrix_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Number power oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = power_cases();
    assert_eq!(
        cases.len(),
        1_421,
        "the Number power differential unexpectedly changed breadth"
    );
    let oracle_observations = run_oracle_batch(&oracle, &cases);

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for (index, (case, oracle_observation)) in cases.iter().zip(&oracle_observations).enumerate() {
        let rust_value = context.eval(&case.source).unwrap_or_else(|error| {
            panic!(
                "Rust evaluation failed for power case {index} ({:?} ** {:?}, source {:?}): {error}",
                case.base, case.exponent, case.source
            )
        });
        let rust_observation = normalize_rust_number(&rust_value);
        assert_eq!(
            rust_observation, *oracle_observation,
            "Number power differential mismatch at case {index}: {:?} ** {:?} (source {:?})",
            case.base, case.exponent, case.source
        );
    }
}

fn power_cases() -> Vec<PowerCase> {
    let fixed_bases = [
        f64::NAN,
        f64::NEG_INFINITY,
        f64::INFINITY,
        -0.0,
        0.0,
        -1.0,
        1.0,
        f64::from_bits((-1.0f64).to_bits() - 1),
        f64::from_bits((-1.0f64).to_bits() + 1),
        f64::from_bits(1.0f64.to_bits() - 1),
        f64::from_bits(1.0f64.to_bits() + 1),
        -10.0,
        -2.0,
        -1.5,
        -0.5,
        0.5,
        1.5,
        2.0,
        10.0,
        -2_147_483_648.0,
        2_147_483_647.0,
        -9_007_199_254_740_991.0,
        9_007_199_254_740_991.0,
        -f64::MIN_POSITIVE,
        f64::MIN_POSITIVE,
        -f64::from_bits(1),
        f64::from_bits(1),
        -f64::from_bits(0x000f_ffff_ffff_ffff),
        f64::from_bits(0x000f_ffff_ffff_ffff),
        -f64::MAX,
        f64::MAX,
    ];
    let fixed_exponents = [
        f64::NAN,
        f64::NEG_INFINITY,
        f64::INFINITY,
        -0.0,
        0.0,
        -2_147_483_648.0,
        -1075.0,
        -1074.0,
        -1024.0,
        -1023.0,
        -53.0,
        -52.0,
        -31.0,
        -3.0,
        -2.0,
        -1.5,
        -1.0,
        -0.5,
        -(1.0 / 3.0),
        1.0 / 3.0,
        0.5,
        1.0,
        1.5,
        2.0,
        3.0,
        31.0,
        52.0,
        53.0,
        1023.0,
        1024.0,
        1074.0,
        1075.0,
        2_147_483_647.0,
    ];

    let mut cases = Vec::new();
    let mut seen = HashSet::new();
    for base in fixed_bases {
        for exponent in fixed_exponents {
            push_case(&mut cases, &mut seen, base, exponent);
        }
    }

    // These pairs concentrate around overflow, underflow, subnormal, and
    // near-one rounding boundaries where a one-ULP difference matters.
    for (base, exponent) in [
        (2.0, 1023.0),
        (2.0, 1024.0),
        (2.0, -1074.0),
        (2.0, -1075.0),
        (0.5, 1074.0),
        (0.5, 1075.0),
        (10.0, 307.0),
        (10.0, 308.0),
        (10.0, 309.0),
        (f64::MAX, 1.0),
        (f64::MAX, 1.000_000_000_000_000_2),
        (f64::MIN_POSITIVE, 0.5),
        (f64::MIN_POSITIVE, 2.0),
        (f64::from_bits(1), 0.5),
        (f64::from_bits(1), 2.0),
        (
            f64::from_bits(1.0f64.to_bits() + 1),
            4_503_599_627_370_496.0,
        ),
        (
            f64::from_bits(1.0f64.to_bits() + 1),
            9_007_199_254_740_991.0,
        ),
        (
            f64::from_bits(1.0f64.to_bits() - 1),
            4_503_599_627_370_496.0,
        ),
        (
            f64::from_bits(1.0f64.to_bits() - 1),
            9_007_199_254_740_991.0,
        ),
        (-2.0, 1_073.0),
        (-2.0, 1_074.0),
        (-2.0, -1_073.0),
        (-2.0, -1_074.0),
        (-2.0, -1_075.0),
        (-0.0, f64::from_bits(1.0f64.to_bits() + 1)),
        (-0.0, -f64::from_bits(1.0f64.to_bits() + 1)),
        (f64::NEG_INFINITY, f64::from_bits(1.0f64.to_bits() + 1)),
        (f64::NEG_INFINITY, -f64::from_bits(1.0f64.to_bits() + 1)),
    ] {
        push_case(&mut cases, &mut seen, base, exponent);
    }

    // SplitMix64 gives stable, platform-independent bit patterns. Every base
    // is forced to be finite, while the exponent schedule alternates integer,
    // fractional, small, and wide values to avoid a matrix made only of
    // overflow and underflow results.
    let mut state = 0x4d59_5df4_d0f3_3173_u64;
    for index in 0..384_u64 {
        let mut base_bits = splitmix64(&mut state);
        if base_bits & 0x7ff0_0000_0000_0000 == 0x7ff0_0000_0000_0000 {
            base_bits ^= 0x0010_0000_0000_0000;
        }
        let base = f64::from_bits(base_bits);
        let sample = splitmix64(&mut state);
        let exponent = match index % 4 {
            0 => (sample % 65) as f64 - 32.0,
            1 => ((sample % 257) as f64 - 128.0) / 8.0,
            2 => ((sample % 4097) as f64 - 2048.0) / 64.0,
            _ => (sample % 2151) as f64 - 1075.0,
        };
        push_case(&mut cases, &mut seen, base, exponent);
    }

    cases
}

fn push_case(cases: &mut Vec<PowerCase>, seen: &mut HashSet<(u64, u64)>, base: f64, exponent: f64) {
    if !seen.insert((base.to_bits(), exponent.to_bits())) {
        return;
    }
    let source = format!(
        "({}) ** ({})",
        number_literal(base),
        number_literal(exponent)
    );
    cases.push(PowerCase {
        base,
        exponent,
        source,
    });
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[allow(clippy::float_cmp)]
fn number_literal(value: f64) -> String {
    if value.is_nan() {
        "(0 / 0)".to_owned()
    } else if value == f64::INFINITY {
        "(1 / 0)".to_owned()
    } else if value == f64::NEG_INFINITY {
        "(-1 / 0)".to_owned()
    } else if value == 0.0 && value.is_sign_negative() {
        "-0".to_owned()
    } else {
        // Rust's independent shortest-roundtrip formatter supplies the test
        // inputs; the crate formatter remains only on the observed-result
        // side, where it is compared directly with QuickJS `String(number)`.
        format!("{value:?}")
    }
}

fn run_oracle_batch(oracle: &OsStr, cases: &[PowerCase]) -> Vec<String> {
    let mut script = String::from(
        r#"
function __qjo_observe_number(value) {
    if (value !== value) return "NaN";
    if (value === 0 && 1 / value === -Infinity) return "-0";
    if (value === Infinity) return "Infinity";
    if (value === -Infinity) return "-Infinity";
    return String(value);
}
var __qjo_cases = [
"#,
    );
    for case in cases {
        writeln!(
            script,
            "[{}, {}],",
            number_literal(case.base),
            number_literal(case.exponent)
        )
        .expect("writing to a String cannot fail");
    }
    script.push_str(
        r#"];
for (var __qjo_index = 0; __qjo_index < __qjo_cases.length; __qjo_index++) {
    var __qjo_case = __qjo_cases[__qjo_index];
    print(__qjo_index + "|" + __qjo_observe_number(__qjo_case[0] ** __qjo_case[1]));
}
"#,
    );

    let output = Command::new(oracle)
        .arg("-e")
        .arg(script)
        .output()
        .unwrap_or_else(|error| panic!("could not execute QJS_ORACLE power batch: {error}"));
    assert!(
        output.status.success(),
        "QJS_ORACLE power batch failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout =
        String::from_utf8(output.stdout).expect("QJS_ORACLE power batch emitted non-UTF-8 output");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        cases.len(),
        "QJS_ORACLE power batch emitted the wrong number of observations"
    );
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            line.strip_prefix(&format!("{index}|"))
                .unwrap_or_else(|| {
                    panic!("QJS_ORACLE power observation {index} was malformed: {line:?}")
                })
                .to_owned()
        })
        .collect()
}

fn normalize_rust_number(value: &Value) -> String {
    match value {
        Value::Int(value) => number_to_string(f64::from(*value)),
        Value::Float(value) if value.is_nan() => "NaN".to_owned(),
        Value::Float(value) if value.is_infinite() && value.is_sign_positive() => {
            "Infinity".to_owned()
        }
        Value::Float(value) if value.is_infinite() && value.is_sign_negative() => {
            "-Infinity".to_owned()
        }
        Value::Float(value) if *value == 0.0 && value.is_sign_negative() => "-0".to_owned(),
        Value::Float(value) => number_to_string(*value),
        other => panic!("Number power unexpectedly produced a non-Number value: {other:?}"),
    }
}
