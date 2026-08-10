use std::ffi::OsStr;
use std::process::Command;

pub(super) fn observe_cmdline_syntax_error(oracle: &OsStr, source: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {source:?}: {error}"));
    assert!(!output.status.success(), "QuickJS accepted {source:?}");
    let stderr = String::from_utf8(output.stderr).expect("QuickJS error output was not UTF-8");
    let mut lines = stderr.lines();
    let first = lines
        .find(|line| line.starts_with("SyntaxError: "))
        .unwrap_or_else(|| panic!("QuickJS emitted no SyntaxError for {source:?}: {stderr}"));
    let location = lines
        .find_map(|line| line.trim().strip_prefix("at <cmdline>:"))
        .unwrap_or_else(|| panic!("QuickJS emitted no location for {source:?}: {stderr}"));
    format!(
        "SyntaxError|{}|{location}",
        first.strip_prefix("SyntaxError: ").unwrap()
    )
}
