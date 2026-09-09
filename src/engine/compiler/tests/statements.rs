use super::*;

#[test]
fn script_roots_enable_annex_b_html_comments_without_stealing_decrement() {
    assert_eq!(evaluate_in_context("40 + 2 <!-- ignored"), Value::Int(42));
    assert_eq!(
        evaluate_in_context("   --> ignored on the first line\n42"),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context("'use strict';\n<!-- ignored\n42"),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context("var value = [23]\n-->[0];\nvalue[0]"),
        Value::Int(23)
    );
    assert_eq!(
        evaluate_in_context("var count = 0; 0/*\n*/--> ignored\ncount += 1; count"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context("var count = 1; count-->0"),
        Value::Bool(true)
    );

    for source in ["; --> ignored", "/*\n*/ arbitrary -->"] {
        assert_eq!(
            compile_unlinked_script(source).unwrap_err().kind(),
            ErrorKind::Syntax,
            "{source:?}"
        );
    }
}

#[test]
fn direct_and_indirect_eval_roots_enable_annex_b_html_comments() {
    assert_eq!(
        evaluate_in_context(
            r#"(function () {
                let local = 1;
                eval("<!-- direct eval\nlocal = 42");
                let indirect = (0, eval)("<!-- indirect eval\n40 + 2");
                return local + "|" + indirect;
            })()"#,
        ),
        Value::String(JsString::from_static("42|42"))
    );
    assert_eq!(
        evaluate_in_context(
            r#"(function () {
                "use strict";
                let local = 1;
                eval("--> strict direct eval\nlocal = 42");
                return local;
            })()"#,
        ),
        Value::Int(42)
    );
}

#[test]
fn debugger_statement_is_an_asi_noop_that_preserves_eval_completion() {
    for (source, expected) in [
        ("40; debugger;", 40),
        ("40; debugger", 40),
        ("40; { debugger }", 40),
        ("debugger\n42", 42),
        ("debugger\r42", 42),
        ("debugger\u{2028}42", 42),
        ("debugger\u{2029}42", 42),
        ("while (false) debugger; 42", 42),
        ("if (true) debugger\n42", 42),
        ("label: debugger\n42", 42),
        (r#""use strict"; 40; debugger"#, 40),
        ("\"use strict\"\ndebugger\n42", 42),
    ] {
        assert_eq!(
            evaluate_in_context(source),
            Value::Int(expected),
            "{source:?}"
        );
    }

    assert_eq!(
        evaluate_in_context("(function () { debugger; return 42; })()"),
        Value::Int(42)
    );
}

#[test]
fn debugger_remains_reserved_outside_statement_grammar() {
    for (source, expected_message) in [
        ("(debugger);", "unexpected token in expression: 'debugger'"),
        ("debugger + 1;", "expecting ';'"),
        ("var debugger = 1;", "variable name expected"),
        ("let debugger = 1;", "expecting ';'"),
        (r"deb\u0075gger;", "'debugger' is a reserved identifier"),
        (r"deb\u0075gger: 42;", "'debugger' is a reserved identifier"),
        (
            r"var deb\u0075gger = 1;",
            "'debugger' is a reserved identifier",
        ),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}");
        assert_eq!(error.message(), expected_message, "{source:?}");
    }

    assert_eq!(
        evaluate_in_context(
            r"({ debugger: 20 }).deb\u0075gger
                + ({ deb\u0075gger() { return 22; } }).debugger()"
        ),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context(r"class C { deb\u0075gger() { return 42; } } new C().debugger()"),
        Value::Int(42)
    );
}
