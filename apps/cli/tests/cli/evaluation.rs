use super::*;

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
fn print_result_exposes_the_completion_value_without_changing_eval_default() {
    let output = qjs()
        .args([
            "--print-result",
            "-e",
            "(function(a) { return a + 1; })(41)",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn qjs_keeps_quickjs_default_non_blocking_host_policy() {
    let output = qjs()
        .args([
            "-e",
            "Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 1, 0)",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "TypeError: cannot block in this thread\n    at wait (native)\n    at <eval> (<cmdline>:1:13)\n"
    );
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
fn expression_position_statement_keywords_expose_quickjs_syntax_errors() {
    for keyword in [
        "return",
        "instanceof",
        "do",
        "while",
        "break",
        "continue",
        "switch",
        "throw",
        "try",
        "with",
    ] {
        let source = format!("var x = {keyword};");
        let output = qjs().args(["-e", &source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{source:?}");
        assert!(output.stdout.is_empty(), "{source:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "SyntaxError: unexpected token in expression: '{keyword}'\n    at <cmdline>:1:9\n"
            ),
            "{source:?}",
        );
    }
}
