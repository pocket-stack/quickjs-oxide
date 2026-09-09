use super::*;

#[test]
fn raw_module_and_script_source_length_guard_matches_quickjs_signed_debug_limit() {
    // Both explicitly sized compiler entry points call this shared guard before
    // constructing a SourceText carrier.
    assert_eq!(validate_source_length(i32::MAX as usize), Ok(()));

    let error = validate_source_length(i32::MAX as usize + 1).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(
        error.message(),
        "source is too large for QuickJS debug metadata"
    );
}

#[test]
fn raw_module_wtf8_surrogate_preserves_debug_source_and_definition_column() {
    let raw = b"/*\xed\xa0\x80*/export function f(){return '\xed\xa0\x80';}";
    let module = compile_unlinked_module_bytes_with_name_and_attribute_checker(
        raw,
        JsString::from_static("raw-module.mjs"),
        DebugInfoMode::Full,
        None,
    )
    .unwrap();
    let child = module
        .function()
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("exported function declaration child");

    assert_eq!(
        child.debug().unwrap().pc2line.as_ref().unwrap().definition,
        crate::source::LineColumn::new(0, 12)
    );
    assert_eq!(
        child.debug().unwrap().source.as_deref(),
        Some(b"function f(){return '\xed\xa0\x80';}".as_slice())
    );
}

#[test]
fn raw_module_accepts_malformed_bytes_inside_comments() {
    let module = compile_unlinked_module_bytes_with_name_and_attribute_checker(
        b"/*\x80X*/export const answer = 42;",
        JsString::from_static("raw-comment.mjs"),
        DebugInfoMode::StripDebug,
        None,
    )
    .unwrap();

    assert_eq!(module.exports().len(), 1);
    assert_eq!(module.exports()[0].export_name.to_utf8_lossy(), "answer");
    assert!(
        module
            .function()
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PushI32(42)))
    );
}

#[test]
fn raw_module_malformed_token_reports_authored_byte_span() {
    let raw = b"export const bad = \x80;";
    let ModuleCompileFailure::Engine(error) =
        compile_unlinked_module_bytes_with_name_and_attribute_checker(
            raw,
            JsString::from_static("raw-token.mjs"),
            DebugInfoMode::Full,
            None,
        )
        .unwrap_err()
    else {
        panic!("malformed raw Module token did not produce an engine diagnostic");
    };
    let span = error.span().expect("malformed token source span");
    let raw_offset = raw.iter().position(|byte| *byte == 0x80).unwrap();

    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "unexpected character");
    assert_eq!(span.start.byte_offset, raw_offset);
    assert_eq!(span.end.byte_offset, raw_offset + 1);
    assert_eq!((span.start.line, span.start.column), (1, 20));
}

#[test]
fn raw_script_debug_uses_authored_bytes_for_columns_and_function_source() {
    fn compile(raw: &[u8]) -> crate::engine::code::function::UnlinkedFunction {
        let source = SourceText::try_from_raw_bytes(raw).unwrap();
        compile_unlinked_script_source_with_filename(&source, "raw.js", DebugInfoMode::Full)
            .unwrap()
    }

    for (raw, expected_column) in [
        (b"/*\xf0\x9f\x98\x80*/function f(){}".as_slice(), 5),
        (b"/*\xed\xa0\xbd\xed\xb8\x80*/function f(){}", 6),
    ] {
        let root = compile(raw);
        let child = root
            .constants()
            .iter()
            .find_map(|constant| constant.as_child())
            .expect("function declaration child");
        assert_eq!(
            child.debug().unwrap().pc2line.as_ref().unwrap().definition,
            crate::source::LineColumn::new(0, expected_column),
            "{raw:02x?}"
        );
    }

    let raw =
        b"function f(){\xef\xbb\xbf/*\x80X*/return '\0\xed\xa0\xbd\xed\xb8\x80\xed\xa0\x80';}";
    let root = compile(raw);
    let child = root
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("function declaration child");
    assert_eq!(
        child.debug().unwrap().source.as_deref(),
        Some(raw.as_slice())
    );
}

#[test]
fn raw_script_rejects_malformed_bytes_in_regexp_before_value_conversion() {
    for raw in [b"/\x80/;".as_slice(), b"/a\\\x80/;", b"/a/\x80;"] {
        let source = SourceText::try_from_raw_bytes(raw).unwrap();
        let error = compile_unlinked_script_source_with_filename(
            &source,
            "raw-regexp.js",
            DebugInfoMode::Full,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{raw:02x?}");
        assert_eq!(error.message(), "invalid UTF-8 sequence", "{raw:02x?}");
        assert!(source.invalid_byte_at(error.span().unwrap().start.byte_offset));
    }
}

#[test]
fn raw_script_malformed_diagnostics_match_quickjs_context_and_location() {
    for (raw, expected_message, expected_offset) in [
        (b"void \x80;".as_slice(), "unexpected character", 5_usize),
        (b"void '\x80';", "invalid UTF-8 sequence", 5),
        (b"void '\\\x80';", "invalid UTF-8 sequence", 5),
        (
            b"void '\\x\x80';",
            "malformed escape sequence in string literal",
            6,
        ),
        (
            b"void '\\u\x80';",
            "malformed escape sequence in string literal",
            6,
        ),
        (
            b"void '\\u{\x80}';",
            "malformed escape sequence in string literal",
            6,
        ),
        (b"void `\x80`;", "invalid UTF-8 sequence", 6),
        (b"void `\\u\x80`;", "invalid UTF-8 sequence", 8),
        (b"void /\x80/;", "invalid UTF-8 sequence", 6),
        (b"var a\\u\x80;", "expecting ';'", 5),
    ] {
        let source = SourceText::try_from_raw_bytes(raw).unwrap();
        let error = compile_unlinked_script_source_with_filename(
            &source,
            "raw-diagnostic.js",
            DebugInfoMode::Full,
        )
        .unwrap_err();
        let location = error.span().unwrap().start;
        assert_eq!(error.kind(), ErrorKind::Syntax, "{raw:02x?}");
        assert_eq!(error.message(), expected_message, "{raw:02x?}");
        assert_eq!(location.byte_offset, expected_offset, "{raw:02x?}");
        assert_eq!(
            (location.line, location.column),
            (1, expected_offset as u32 + 1)
        );
    }
}

#[test]
fn raw_script_nul_token_uses_quickjs_empty_spelling() {
    let source = SourceText::try_from_raw_bytes(b"void \0;").unwrap();
    let error =
        compile_unlinked_script_source_with_filename(&source, "raw-nul.js", DebugInfoMode::Full)
            .unwrap_err();
    let location = error.span().unwrap().start;

    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "unexpected token in expression: ''");
    assert_eq!(location.byte_offset, 5);
    assert_eq!((location.line, location.column), (1, 6));
}

#[test]
fn raw_script_error_columns_scan_authored_comment_bytes() {
    for (raw, expected_column) in [
        (b"/*\x80*/@".as_slice(), 5_u32),
        (b"/*\xff*/@", 6),
        (b"/*\xc0\x80*/@", 6),
        (b"/*\xe2\x82*/@", 6),
        (b"/*\xf4\x90\x80\x80*/@", 6),
        (b"/*\xf8\x88\x80\x80\x80*/@", 6),
        (b"/*\xed\xa0\xbd*/@", 6),
        (b"/*\xed\xa0\xbd\xed\xb8\x80*/@", 7),
        (b"/*\xf0\x9f\x98\x80*/@", 6),
    ] {
        let source = SourceText::try_from_raw_bytes(raw).unwrap();
        let error = compile_unlinked_script_source_with_filename(
            &source,
            "raw-column.js",
            DebugInfoMode::Full,
        )
        .unwrap_err();
        let location = error.span().unwrap().start;
        assert_eq!(error.kind(), ErrorKind::Syntax, "{raw:02x?}");
        assert_eq!(location.byte_offset, raw.len() - 1, "{raw:02x?}");
        assert_eq!((location.line, location.column), (1, expected_column));
    }
}
