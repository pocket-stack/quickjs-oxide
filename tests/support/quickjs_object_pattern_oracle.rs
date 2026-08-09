use std::ffi::OsStr;
use std::process::{Command, Output};

pub(super) fn observe_completion_strip_one_lf(
    oracle: &OsStr,
    source: &str,
    description: &str,
) -> String {
    let stdout = super::quickjs_oracle::observe_completion_output(oracle, source, description);
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
}

// Keep source in argv so parser diagnostics retain qjs's <cmdline> filename,
// line and column behavior. Callers compare the complete process result.
pub(super) fn run_cli_exact(program: &OsStr, source: &str, description: &str) -> Output {
    Command::new(program)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}
