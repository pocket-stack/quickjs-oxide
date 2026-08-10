use std::ffi::OsStr;
use std::process::Command;

pub(super) fn observe_program_property_lines(
    oracle: &OsStr,
    probe: &str,
    kind: &str,
) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", probe])
        .output()
        .unwrap_or_else(|error| panic!("run QuickJS Program-{kind} property oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS Program-{kind} property oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| {
            panic!("QuickJS Program-{kind} property output was not UTF-8: {error}")
        })
        .lines()
        .map(str::to_owned)
        .collect()
}
