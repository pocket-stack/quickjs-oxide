//! B12 (S2 C5/C6): malformed string-escape wording and unterminated-string
//! columns must match the pinned QuickJS byte-for-byte at the command line,
//! including the message text and the line:column location.

use super::quickjs_syntax_diagnostic_oracle::observe_cmdline_syntax_error as oracle_observation;
use quickjs_oxide::engine::api::{Context, Runtime, RuntimeError, Value};

/// (description, source, `SyntaxError|<message>|<line>:<column>`)
///
/// Locations are asserted against pinned QuickJS `qjs -e` output, so each
/// expectation is a copy of the pinned engine's emitted frame.
const CASES: &[(&str, &str, &str)] = &[
    // --- C6: unterminated strings point at the opening quote ---
    (
        "newline-terminated single-quoted string",
        "var s = 'abc\n",
        "SyntaxError|unexpected end of string|1:9",
    ),
    (
        "crlf-terminated single-quoted string",
        "var s = 'abc\r\n",
        "SyntaxError|unexpected end of string|1:9",
    ),
    (
        "newline-terminated string on the second line",
        "var t = 1;\nvar s = 'abc\n",
        "SyntaxError|unexpected end of string|2:9",
    ),
    (
        "eof-terminated string already matched and keeps the quote",
        "var s = 'abc",
        "SyntaxError|unexpected end of string|1:9",
    ),
    (
        "bare backslash before a newline",
        "var s = '\\\n",
        "SyntaxError|unexpected end of string|1:9",
    ),
    (
        "bare backslash at end of file",
        "var s = '\\",
        "SyntaxError|unexpected end of string|1:9",
    ),
    // --- C5: every malformed \x / \u escape shares one pinned message ---
    (
        "short fixed unicode escape",
        "var s = '\\u00'",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "non-hex fixed unicode escape",
        "var s = '\\uz'",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "unicode escape exhausted at end of file",
        "var s = '\\u'",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "non-hex hex escape",
        "var s = '\\xZZ'",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "hex escape exhausted at end of file",
        "var s = '\\x'",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "empty braced unicode escape",
        "var s = '\\u{}'",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "non-hex braced unicode escape",
        "var s = '\\u{zz}'",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "unterminated braced unicode escape",
        "var s = '\\u{41'",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "braced unicode escape beyond U+10FFFF",
        "var s = '\\u{110000}'",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "double-quoted malformed escape behaves identically",
        "var s = \"\\u00\"",
        "SyntaxError|malformed escape sequence in string literal|1:10",
    ),
    (
        "strict mode \\8 uses the generic message",
        "\"use strict\"; var s = \"a\\8b\"",
        "SyntaxError|malformed escape sequence in string literal|1:25",
    ),
    (
        "strict mode \\9 uses the generic message",
        "\"use strict\"; var s = \"a\\9b\"",
        "SyntaxError|malformed escape sequence in string literal|1:25",
    ),
    // --- unaffected surfaces that must not regress ---
    (
        "strict legacy octal keeps its dedicated message",
        "\"use strict\"; var s = \"a\\07b\"",
        "SyntaxError|octal escape sequences are not allowed in strict mode|1:25",
    ),
];

#[test]
fn string_escape_diagnostics_match_pinned_quickjs() {
    for &(description, source, expected) in CASES {
        assert_eq!(rust_observation(source), expected, "Rust: {description}");
    }

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP string escape diagnostic differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source, expected) in CASES {
        assert_eq!(
            oracle_observation(&oracle, source),
            expected,
            "QuickJS: {description}"
        );
    }
}

fn rust_observation(source: &str) -> String {
    let runtime =
        Runtime::new_with_host_services(quickjs_oxide_host::SystemHostServices::default());
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval(source),
        Err(RuntimeError::Exception),
        "Rust accepted {source:?}"
    );
    let Value::Object(error) = context
        .take_exception()
        .unwrap()
        .unwrap_or_else(|| panic!("Rust produced no exception for {source:?}"))
    else {
        panic!("Rust exception was not an Error object for {source:?}");
    };
    let read = |context: &mut Context, name: &str| {
        let key = runtime.intern_property_key(name).unwrap();
        context.get_property(&error, &key).unwrap()
    };
    let Value::String(name) = read(&mut context, "name") else {
        panic!("Rust Error.name was not a string for {source:?}");
    };
    let Value::String(message) = read(&mut context, "message") else {
        panic!("Rust Error.message was not a string for {source:?}");
    };
    let Value::Int(line) = read(&mut context, "lineNumber") else {
        panic!("Rust Error.lineNumber was not an integer for {source:?}");
    };
    let Value::Int(column) = read(&mut context, "columnNumber") else {
        panic!("Rust Error.columnNumber was not an integer for {source:?}");
    };
    format!(
        "{}|{}|{line}:{column}",
        name.to_utf8_lossy(),
        message.to_utf8_lossy()
    )
}
