use super::*;

#[test]
fn version_names_the_pinned_compatibility_target() {
    let output = qjs().arg("--version").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("QuickJS 2026-06-04"));
}

#[test]
fn strip_flags_match_quickjs_debug_stack_behavior_and_last_option_wins() {
    let source = "1n + 1";
    let located = "TypeError: cannot convert bigint to number\n    at <eval> (<cmdline>:1:4)\n";
    let stripped = "TypeError: cannot convert bigint to number\n    at <eval>\n";
    for (arguments, expected) in [
        (vec!["--strip-source", "-e", source], located),
        (vec!["-s", "-e", source], stripped),
        (vec!["-s", "--strip-source", "-e", source], located),
        (vec!["--strip-source", "-s", "-e", source], stripped),
        (vec!["-e", source, "-s"], stripped),
        (vec!["-se", source], stripped),
        (vec!["-e1n + 1", "--strip-source"], located),
    ] {
        let output = qjs().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
    }

    for arguments in [vec!["-sq"], vec!["-qs"], vec!["-q", "-s"]] {
        let output = qjs().args(arguments).output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
