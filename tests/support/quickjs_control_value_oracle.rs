use std::ffi::OsStr;
use std::process::Command;

pub(super) fn observe_normalized_value(
    oracle: &OsStr,
    source: &str,
    description: &str,
    normalizer: &str,
) -> String {
    let script = format!("var __qjo_value = std.evalScript(scriptArgs[0]);\n{normalizer}");
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
