use std::ffi::OsStr;
use std::process::Command;

const COMPLETION_OBSERVER: &str = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print('return|' + typeof value + '|' + String(value));
} catch (error) {
  if (error !== null && typeof error === 'object')
    print('throw|object|' + error.name + '|' + error.message);
  else
    print('throw|' + typeof error + '|' + String(error));
}
"#;

const SEQUENCE_COMPLETION_OBSERVER: &str = r#"
(function () {
for (var index = 0; index < scriptArgs.length; index++) {
  try {
    var value = std.evalScript(scriptArgs[index]);
    print('return|' + typeof value + '|' + String(value));
  } catch (error) {
    if (error !== null && typeof error === 'object')
      print('throw|object|' + error.name + '|' + error.message);
    else
      print('throw|' + typeof error + '|' + String(error));
  }
}
})();
"#;

// Each integration target path-includes this module and may need only one
// output-normalization variant.
#[allow(dead_code)]
pub(super) fn observe_completion_argv_trim_end(
    oracle: &OsStr,
    source: &str,
    description: &str,
) -> String {
    observe_completion_argv_output(oracle, source, description)
        .trim_end()
        .to_owned()
}

#[allow(dead_code)]
pub(super) fn observe_completion_argv_strip_one_lf(
    oracle: &OsStr,
    source: &str,
    description: &str,
) -> String {
    let stdout = observe_completion_argv_output(oracle, source, description);
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
}

#[allow(dead_code)]
pub(super) fn observe_completion_argv_sequence_strip_one_lf(
    oracle: &OsStr,
    sources: &[&str],
    description: &str,
) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", SEQUENCE_COMPLETION_OBSERVER])
        .args(sources)
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS sequence failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"));
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
}

fn observe_completion_argv_output(oracle: &OsStr, source: &str, description: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", COMPLETION_OBSERVER, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS observer failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"))
}
