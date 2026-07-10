use std::process::Command;

fn qjs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_qjs"))
}

#[test]
fn eval_executes_the_rust_compiler_and_vm() {
    let output = qjs().args(["-e", "(6 + 1) * 6"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn eval_executes_source_level_functions_and_formats_native_errors() {
    let function = qjs()
        .args(["-e", "(function(a, b) { return a + b; })(20, 22)"])
        .output()
        .unwrap();
    assert!(function.status.success());

    let error = qjs().args(["-e", "1n + 1"]).output().unwrap();
    assert_eq!(error.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(error.stderr).unwrap(),
        "TypeError: cannot convert bigint to number\n    at <eval> (<cmdline>:1:4)\n"
    );
}

#[test]
fn unparenthesized_power_unary_error_omits_a_source_frame_like_quickjs() {
    for source in ["-2 ** 2", "-value++ ** 2"] {
        let output = qjs().args(["-e", source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{source}");
        assert!(output.stdout.is_empty(), "{source}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "SyntaxError: unparenthesized unary expression can't appear on the left-hand side of '**'\n\n",
            "{source}"
        );
    }

    let dynamic = qjs()
        .args(["-e", "Function(\"return -2 ** 2\")"])
        .output()
        .unwrap();
    assert_eq!(dynamic.status.code(), Some(1));
    assert!(dynamic.stdout.is_empty());
    assert_eq!(
        String::from_utf8(dynamic.stderr).unwrap(),
        "SyntaxError: unparenthesized unary expression can't appear on the left-hand side of '**'\n    at Function (native)\n    at <eval> (<cmdline>:1:9)\n"
    );
}

#[test]
fn eval_executes_the_dynamic_function_constructor_path() {
    for source in [
        "throw Function(\"a\", \"return a + 1\")(41)",
        "throw new Function(\"return 42\")()",
    ] {
        let output = qjs().args(["-e", source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap(), "42\n");
    }
}

#[test]
fn exception_output_quotes_strings_and_marks_bigints() {
    for (source, expected) in [
        ("throw \"x\"", "\"x\"\n"),
        (
            "throw 123456789012345678901234567890n",
            "123456789012345678901234567890n\n",
        ),
        ("throw -0", "-0\n"),
    ] {
        let output = qjs().args(["-e", source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{source}");
        assert!(output.stdout.is_empty(), "{source}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn unsupported_source_fails_instead_of_falling_back_to_an_external_engine() {
    let output = qjs().args(["-e", "answer"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("'answer' is not defined"));
}

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
