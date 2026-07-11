use std::process::Command;

use quickjs_oxide::number::{to_exponential, to_fixed, to_precision, to_string_radix};

#[test]
fn number_formatting_kernel_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Number formatting differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let mut bits = vec![
        0_u64,
        1_u64 << 63,
        1,
        2,
        3,
        (1_u64 << 63) | 1,
        (1_u64 << 52) - 2,
        (1_u64 << 52) - 1,
        1_u64 << 52,
        (1_u64 << 52) + 1,
        0x3fb9_9999_9999_999a, // 0.1
        0x3fef_ffff_ffff_ffff, // previous binary64 before 1
        0x3ff0_0000_0000_0001, // next binary64 after 1
        0x3ff0_147a_e147_ae14, // 1.005
        0x4004_0000_0000_0000, // 2.5
        0x7fef_ffff_ffff_fffe,
        0x7fef_ffff_ffff_ffff,
        0xffef_ffff_ffff_ffff,
        f64::INFINITY.to_bits(),
        f64::NEG_INFINITY.to_bits(),
        f64::NAN.to_bits(),
    ];
    let mut state = 0x1319_8a2e_0370_7344_u64;
    for _ in 0..64 {
        state = state
            .wrapping_mul(2_862_933_555_777_941_757)
            .wrapping_add(3_037_000_493);
        bits.push(state);
    }

    let radices = (2_u32..=36).collect::<Vec<_>>();
    let fixed_digits = [0_i32, 1, 2, 20, 100];
    let exponential_digits = [None, Some(0), Some(2), Some(20), Some(100)];
    let precisions = [None, Some(1), Some(2), Some(20), Some(100)];
    let mut expected = Vec::new();
    for raw in &bits {
        let value = f64::from_bits(*raw);
        for radix in &radices {
            expected.push(to_string_radix(value, *radix).unwrap());
        }
        for digits in fixed_digits {
            expected.push(to_fixed(value, digits).unwrap());
        }
        for digits in exponential_digits {
            expected.push(to_exponential(value, digits).unwrap());
        }
        for precision in precisions {
            expected.push(to_precision(value, precision).unwrap());
        }
    }

    let bit_literals = bits
        .iter()
        .map(|bits| format!("0x{bits:016x}n"))
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        r#"
var storage = new ArrayBuffer(8);
var view = new DataView(storage);
var bits = [{bit_literals}];
var radices = [];
for (var radix = 2; radix <= 36; radix++) radices.push(radix);
var fixedDigits = [0,1,2,20,100];
var exponentialDigits = [-1,0,2,20,100];
var precisions = [-1,1,2,20,100];
for (var i = 0; i < bits.length; i++) {{
    view.setBigUint64(0, bits[i], true);
    var value = view.getFloat64(0, true);
    for (var j = 0; j < radices.length; j++) print(value.toString(radices[j]));
    for (var j = 0; j < fixedDigits.length; j++) print(value.toFixed(fixedDigits[j]));
    for (var j = 0; j < exponentialDigits.length; j++)
        print(exponentialDigits[j] < 0 ? value.toExponential() : value.toExponential(exponentialDigits[j]));
    for (var j = 0; j < precisions.length; j++)
        print(precisions[j] < 0 ? value.toPrecision() : value.toPrecision(precisions[j]));
}}
"#
    );
    let output = Command::new(oracle)
        .args(["-e", &source])
        .output()
        .expect("run QuickJS Number formatting oracle");
    assert!(
        output.status.success(),
        "QuickJS Number formatting oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual = String::from_utf8(output.stdout)
        .expect("QuickJS Number formatting oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(expected, actual);
}
