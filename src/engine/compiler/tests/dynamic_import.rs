use super::*;

#[test]
fn dynamic_import_keyword_edges_preserve_quickjs_diagnostics_and_spans() {
    for (source, message, line, column) in [
        (
            r#"im\u0070ort("module")"#,
            "'import' is a reserved identifier",
            1,
            1,
        ),
        ("typeof import;", "expecting '('", 1, 14),
        (
            "'use strict'; import(\"module\", yield);",
            "unexpected 'yield' keyword",
            1,
            32,
        ),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}: {error}");
        assert_eq!(error.message(), message, "{source:?}");
        let span = error
            .span()
            .unwrap_or_else(|| panic!("missing syntax span for {source:?}"));
        assert_eq!(
            (span.start.line, span.start.column),
            (line, column),
            "{source:?}",
        );
    }

    for source in [
        "import(\"module\", yield);",
        "function* load(){ import(\"module\", yield); }",
    ] {
        compile_unlinked_script(source).unwrap_or_else(|error| {
            panic!("valid Yield-context ImportCall {source:?} failed: {error}")
        });
    }
}

#[test]
fn dynamic_import_retains_quickjs_specifier_and_options_stack_shape() {
    for (source, expected_options) in [
        ("import(20)", None),
        ("import(20,)", None),
        ("import(20, 22)", Some(22)),
        ("import(20, 22,)", Some(22)),
    ] {
        let mut tree =
            Parser::parse(source, JsString::from_static("<dynamic-import-frontier>")).unwrap();
        assert!(
            tree.pending_unsupported.is_none(),
            "valid ImportCall retained a stale feature frontier: {source:?}"
        );

        resolve_identifiers(&mut tree).unwrap();
        let function = lower_unlinked_tree(tree, DebugInfoMode::StripDebug).unwrap();
        assert!(
            function.code().windows(3).any(|window| match window {
                [
                    Instruction::PushI32(20),
                    Instruction::Undefined,
                    Instruction::Import,
                ] => {
                    expected_options.is_none()
                }
                [
                    Instruction::PushI32(20),
                    Instruction::PushI32(actual_options),
                    Instruction::Import,
                ] => expected_options == Some(*actual_options),
                _ => false,
            }),
            "{source:?} lost its Import stack inputs: {:?}",
            function.code()
        );

        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("valid ImportCall {source:?} failed: {error}"));
    }
}
