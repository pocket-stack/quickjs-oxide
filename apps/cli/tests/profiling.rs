use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qjs"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn profiling_inactive_keeps_normal_output() {
    let output = run(&["-e", "print(42)"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());
}

#[cfg(not(feature = "profiling"))]
#[test]
fn profiling_flags_explain_missing_build_feature() {
    for args in [
        vec!["-d", "-e", "print(42)"],
        vec!["-Tq"],
        vec!["--profile-json", "-q"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("does not include profiling"));
    }
}

#[cfg(feature = "profiling")]
#[test]
fn profiling_reports_preserve_exception_and_cover_teardown() {
    let output = run(&[
        "-dT",
        "--profile-json",
        "-e",
        "print(42); throw new Error('probe')",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"42\n");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("probe"));
    assert!(error.contains("\"phase\":\"error-before-context-drop\""));
    assert!(error.contains("\"finished\":true"));
    assert!(error.contains("\"complete_within_scope\":true"));
    assert!(error.contains("\"kind\":\"F\""));
}

#[cfg(feature = "profiling")]
#[test]
fn profiling_options_validate_scope_and_bounds() {
    for args in [
        vec!["-q", "--profile-events", "1"],
        vec!["-d", "--profile-iterations", "2", "-e", "42"],
        vec!["-qd", "--profile-iterations", "0"],
        vec!["-qT", "--profile-events", "1000001"],
    ] {
        assert_eq!(run(&args).status.code(), Some(2));
    }
}

#[cfg(feature = "profiling")]
#[test]
fn profiling_lifecycle_and_overflow_are_labelled() {
    let output = run(&[
        "-qdT",
        "--profile-json",
        "--profile-iterations",
        "2",
        "--profile-events",
        "0",
    ]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let report = String::from_utf8(output.stderr).unwrap();
    assert_eq!(report.lines().count(), 3);
    assert!(report.contains("\"timer\":\"monotonic-wall\""));
    assert!(report.contains("\"iterations\":2"));
    assert!(report.contains("\"complete_within_scope\":false"));
    assert!(report.contains("\"events\":[]"));
}
