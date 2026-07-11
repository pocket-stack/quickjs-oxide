use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::JsBigInt;
use quickjs_oxide::value::number_to_string;

#[test]
fn bigint_to_number_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP BigInt-to-Number differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let mut inputs = [
        "0",
        "1",
        "-1",
        "9007199254740991",
        "9007199254740992",
        "9007199254740993",
        "9007199254740994",
        "9007199254740995",
        "9007199254740996",
        "9007199254740997",
        "-9007199254740991",
        "-9007199254740992",
        "-9007199254740993",
        "-9007199254740994",
        "-9007199254740995",
        "-9007199254740996",
        "-9007199254740997",
        "18446744073709551615",
        "18446744073709551616",
        "18446744073709551617",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let two_to_1023 = JsBigInt::one().shl(&JsBigInt::from(1023)).unwrap();
    let two_to_1024 = two_to_1023.shl(&JsBigInt::one()).unwrap();
    inputs.extend([
        two_to_1023.to_string(),
        two_to_1023.neg().unwrap().to_string(),
        two_to_1024.to_string(),
        two_to_1024.neg().unwrap().to_string(),
    ]);

    let rust = inputs
        .iter()
        .map(|input| {
            let value = JsBigInt::parse_js_string(input).unwrap();
            number_to_string(value.to_f64())
        })
        .collect::<Vec<_>>();
    assert_eq!(rust, oracle_results(&oracle, &inputs));
}

fn oracle_results(oracle: &OsStr, inputs: &[String]) -> Vec<String> {
    let quoted = inputs
        .iter()
        .map(|input| format!("\"{input}\""))
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        "var inputs=[{quoted}]; for (var i=0;i<inputs.length;i++) print(String(Number(BigInt(inputs[i]))));"
    );
    let output = Command::new(oracle)
        .args(["-e", &source])
        .output()
        .expect("run QuickJS BigInt-to-Number oracle");
    assert!(
        output.status.success(),
        "QuickJS BigInt-to-Number oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS BigInt-to-Number oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
