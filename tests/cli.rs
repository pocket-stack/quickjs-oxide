use crate::runtime_oracle::run_cli;
#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;

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
fn unsupported_grammar_is_not_rewritten_as_a_javascript_syntax_error() {
    let output = qjs().args(["-e", "import('fixture')"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "UnsupportedError at 1:1: import syntax is not implemented yet\n"
    );
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

#[test]
fn primitive_exception_dump_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP CLI dump differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (description, source) in [
        ("quoted string", "throw \"x\""),
        ("escaped string", "throw \"line\\n\\t\\\\\\\"\\0\\x7f\""),
        ("Unicode string", "throw \"é🙂中\""),
        ("short BigInt", "throw 1n"),
        ("heap BigInt", "throw 123456789012345678901234567890n"),
        ("negative zero", "throw -0"),
        ("invalid prefix update operand", "++1"),
        (
            "postfix under unary power early error has no source frame",
            "-value++ ** 2",
        ),
        (
            "strict private postfix update return marker",
            "(function named(){ 'use strict'; return named++; })()",
        ),
    ] {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), &[], source, description);
        let quickjs = run_cli(&oracle, &[], source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}
