use std::ffi::OsStr;
use std::process::Command;

const STRING_VALUE_OBSERVER: &str = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print(String(value));
} catch (error) {
  print("UNEXPECTED THROW: " + error.name + ": " + error.message);
}
"#;

pub(super) fn observe_string_value(oracle: &OsStr, source: &str, description: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", STRING_VALUE_OBSERVER, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}
