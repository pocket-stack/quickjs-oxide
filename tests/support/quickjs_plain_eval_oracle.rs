use std::ffi::OsStr;
use std::process::Command;

pub(super) fn eval_plain_lines(oracle: &OsStr, source: &str) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not execute QJS_ORACLE: {error}"));
    assert!(
        output.status.success(),
        "QJS_ORACLE failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QJS_ORACLE emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
