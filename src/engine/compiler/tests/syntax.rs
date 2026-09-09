use super::*;

#[test]
fn strict_and_escaped_reserved_binding_names_are_rejected_late() {
    for source in [
        "(function(implements) { 'use strict'; return implements; })(1)",
        "'use strict'; (function(let) { return let; })(1)",
        "(function() { 'use strict'; var eval = 1; return eval; })()",
        "(function() { 'use strict'; return impl\\u0065ments; })()",
        "(function(\\u0069f) { return \\u0069f; })(1)",
    ] {
        assert!(
            compile_unlinked_script(source).is_err(),
            "accepted {source:?}"
        );
    }
    assert_eq!(
        evaluate_in_context("(function(implements) { return implements; })(1)"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context("(function(impl\\u0065ments) { return impl\\u0065ments; })(1)"),
        Value::Int(1)
    );
}

#[test]
fn always_reserved_words_use_quickjs_syntax_diagnostics() {
    for (source, message) in [
        ("enum;", "unsupported keyword: enum"),
        ("'use strict'; enum;", "unsupported keyword: enum"),
        ("export;", "unsupported keyword: export"),
        ("'use strict'; export;", "unsupported keyword: export"),
        ("extends;", "unsupported keyword: extends"),
        ("'use strict'; extends;", "unsupported keyword: extends"),
        ("import;", "expecting '('"),
        ("'use strict'; import;", "expecting '('"),
        ("(enum);", "unexpected token in expression: 'enum'"),
        ("void export;", "unexpected token in expression: 'export'"),
        ("1 + extends;", "unexpected token in expression: 'extends'"),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}: {error}");
        assert_eq!(error.message(), message, "{source:?}");
    }

    for (source, message) in [
        ("var enum;", "variable name expected"),
        ("'use strict'; let export;", "variable name expected"),
        ("const extends = 1;", "variable name expected"),
        ("function f(import){}", "missing formal parameter"),
        ("try{}catch(enum){}", "identifier expected"),
        ("let { enum: export } = {};", "invalid destructuring target"),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}: {error}");
        assert_eq!(error.message(), message, "{source:?}");
    }

    for (source, word) in [
        (r"var en\u0075m;", "enum"),
        (r"exp\u006frt;", "export"),
        (r"var ext\u0065nds;", "extends"),
        (r"imp\u006frt;", "import"),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}: {error}");
        assert_eq!(
            error.message(),
            format!("'{word}' is a reserved identifier"),
            "{source:?}"
        );
    }
}

#[test]
fn statement_and_binary_keywords_in_expression_position_are_syntax_errors() {
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
        let error = compile_unlinked_script(&source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}: {error}");
        assert_eq!(
            error.message(),
            format!("unexpected token in expression: '{keyword}'"),
            "{source:?}",
        );
        let span = error
            .span()
            .expect("keyword SyntaxError lost its source span");
        assert_eq!(span.start.byte_offset, 8, "{source:?}");
        assert_eq!(span.end.byte_offset, 8 + keyword.len(), "{source:?}");
    }
}

#[test]
fn statement_keywords_in_primary_expression_use_quickjs_syntax_diagnostics() {
    for (source, keyword, column) in [
        ("({[if (0) 0;]})", "if", 4),
        ("[for (x of [1]) x]", "for", 2),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}: {error}");
        assert_eq!(
            error.message(),
            format!("unexpected token in expression: '{keyword}'"),
            "{source:?}"
        );
        let span = error
            .span()
            .unwrap_or_else(|| panic!("missing syntax span for {source:?}"));
        assert_eq!(
            (span.start.line, span.start.column),
            (1, column),
            "{source:?}"
        );
    }
}

#[test]
fn reserved_property_names_and_import_calls_remain_distinct() {
    for source in [
        "import('module')",
        "import /* trivia */ ('module')",
        "import\n('module')",
        "import('module',)",
        "import('module', {})",
        "import('module', {},)",
        "import(('mod' + 'ule')).then",
        "import('module')?.foo",
        "import('module')?.[0]",
        "import('module')?.()",
        "import('module')`tag`",
        "new (import('module'))",
        "new C(import('module'))",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("valid ImportCall {source:?} failed: {error}"));
    }

    for (source, message) in [
        ("import binding from 'module';", "expecting '('"),
        ("import { binding } from 'module';", "expecting '('"),
        ("import.meta", "import.meta only valid in module code"),
        (
            "import /* trivia */ . meta",
            "import.meta only valid in module code",
        ),
        ("import.foo", "meta expected"),
        (r"import.\u006deta", "meta expected"),
        ("import()", "unexpected token in expression: ')'"),
        ("import(,)", "unexpected token in expression: ','"),
        ("import(...source)", "unexpected token in expression: '...'"),
        ("import(source extra)", "expecting ')'"),
        ("import(source, {}, extra)", "expecting ')'"),
        ("new import(source)", "invalid use of 'import()'"),
        ("new import()", "invalid use of 'import()'"),
        ("new/*gap*/import('module')", "invalid use of 'import()'"),
        ("new\nimport('module')", "invalid use of 'import()'"),
        (
            "new import /* gap */ ('module')",
            "invalid use of 'import()'",
        ),
        (
            "(() => new/*gap*/import('module'))",
            "invalid use of 'import()'",
        ),
        ("import(source) = 1", "invalid assignment left-hand side"),
        ("import(source)++", "invalid increment/decrement operand"),
        ("++import(source)", "invalid increment/decrement operand"),
        ("import?.(source)", "expecting '('"),
        ("import`source`", "expecting '('"),
        ("import('module')?.`tag`", "expecting field name"),
        (
            "import('module')?.foo`tag`",
            "template literal cannot appear in an optional chain",
        ),
        (
            "new (import('module'))?.foo",
            "new keyword cannot be used with an optional chain",
        ),
        ("import('module'); enum;", "unsupported keyword: enum"),
        (
            "import('module'); class C { method(){ return this.#missing; } }",
            "undefined private field '#missing'",
        ),
        (
            "import('module'); class C { method(object){ return #missing in object; } }",
            "undefined private field '#missing'",
        ),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}: {error}");
        assert_eq!(error.message(), message, "{source:?}");
    }

    let detached_error =
        compile_script("import('module'); class C { method(){ return this.#missing; } }")
            .unwrap_err();
    assert_eq!(detached_error.kind(), ErrorKind::Syntax);
    assert_eq!(
        detached_error.message(),
        "undefined private field '#missing'"
    );

    for source in [
        "({enum:20,export:22}).enum+({enum:20,export:22}).export",
        "({extends(){return 20},import(){return 22}}).extends()+({extends(){return 20},import(){return 22}}).import()",
        r"({en\u0075m:42}).en\u0075m",
        r"({exp\u006frt(){return 42}}).exp\u006frt()",
        "class C extends Object {}; 42",
        "class C { extends(){return 42} import(){return 42} }; new C().extends()",
        "({enum:42})?.enum",
    ] {
        assert_eq!(evaluate_in_context(source), Value::Int(42), "{source:?}");
    }
}

#[test]
fn parser_driven_lexing_preserves_quickjs_error_priority_and_locations() {
    let cases = [
        (
            r"(function(){ var \u0069f\u{}=14; })()",
            "'if' is a reserved identifier",
            1,
            18,
        ),
        (
            r"(function(){ var if\u{}=14; })()",
            "'if' is a reserved identifier",
            1,
            18,
        ),
        (
            r"(function(){ var if\x61=1; })()",
            "variable name expected",
            1,
            18,
        ),
        (
            r"(function(){ var \u{}=1; })()",
            "variable name expected",
            1,
            18,
        ),
        (r"(function(){ var a\u{}=1; })()", "expecting ';'", 1, 19),
        (
            "(function(){ var 'unterminated })()",
            "unexpected end of string",
            1,
            18,
        ),
        (
            "(function(a 'unterminated){})",
            "unexpected end of string",
            1,
            13,
        ),
        (
            "(function(){ return (1 'unterminated); })()",
            "unexpected end of string",
            1,
            24,
        ),
        (
            "(function(eval){ \"use strict\"; \"x\"; \"unterminated })()",
            "unexpected end of string",
            1,
            37,
        ),
        (
            "(function(){ \"use strict\"; (function(eval){ \"x\"; \"unterminated })() })()",
            "unexpected end of string",
            1,
            50,
        ),
    ];

    for (source, message, line, column) in cases {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
        let span = error
            .span()
            .unwrap_or_else(|| panic!("missing span for {source}"));
        assert_eq!(
            (span.start.line, span.start.column),
            (line, column),
            "{source}"
        );
    }

    let reached_lex_error =
        compile_unlinked_script("(function(){ throw\n'unterminated })()").unwrap_err();
    assert_eq!(reached_lex_error.message(), "unexpected end of string");
    let reached_span = reached_lex_error.span().unwrap();
    assert_eq!((reached_span.start.line, reached_span.start.column), (2, 1));

    let raw_token_error = compile_unlinked_script("(function(){ throw\n\\u{}; })()").unwrap_err();
    assert_eq!(
        raw_token_error.message(),
        "line terminator not allowed after throw"
    );
    let raw_span = raw_token_error.span().unwrap();
    assert_eq!((raw_span.start.line, raw_span.start.column), (2, 1));
}

#[test]
fn primary_expression_slashes_are_rescanned_as_complete_regexp_tokens() {
    // Pinned QuickJS makes this decision in its primary-expression parser:
    // `/` and `/=` are ordinary punctuators until the grammar requires an
    // operand, at which point it rewinds and scans the complete literal.
    for source in [
        "/start/g;",
        "/=prefix/m;",
        "Function.value = /rhs/gi;",
        "(function(){ return /ret/m; })",
        "Function(/argument/s);",
        "true ? /consequent/u : 0;",
        "false ? 0 : /alternate/y;",
        "false || /logical/d;",
        "1 / /denominator/u;",
    ] {
        compile_unlinked_script(source)
            .unwrap_or_else(|error| panic!("RegExp literal {source:?} failed: {error}"));
    }
    let script = compile_unlinked_script("/start/g;").unwrap();
    assert!(matches!(script.code()[0], Instruction::RegExp(0)));

    let invalid_pattern = compile_unlinked_script("/(/").unwrap_err();
    assert_eq!(invalid_pattern.kind(), ErrorKind::Syntax);
    assert_eq!(invalid_pattern.message(), "expecting ')'");
    let span = invalid_pattern.span().expect("literal SyntaxError span");
    assert_eq!((span.start.line, span.start.column), (1, 1));
    assert_eq!((span.start.byte_offset, span.end.byte_offset), (0, 3));

    let invalid_flags = compile_unlinked_script("/a/gg").unwrap_err();
    assert_eq!(invalid_flags.kind(), ErrorKind::Syntax);
    assert_eq!(invalid_flags.message(), "invalid regular expression flags");

    for (source, expected) in [
        ("/a", "unexpected end of regexp"),
        ("/a\n/", "unexpected line terminator in regexp"),
        ("/a\\\n/", "unexpected line terminator in regexp"),
    ] {
        let error = compile_unlinked_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source:?}");
        assert_eq!(error.message(), expected, "{source:?}");
    }

    compile_unlinked_script("/(?=a)/").expect("forward lookahead literal should compile");
    compile_unlinked_script("/(?<=a)/").expect("backward lookaround literal should compile");
    compile_unlinked_script("1 / /denominator/v;")
        .expect("Unicode Sets RegExp literal should compile after slash rescanning");

    // The same slash tokens remain operators when the expression parser
    // has already produced their left operand.
    assert_eq!(evaluate("84 / 2"), Value::Int(42));
    assert_eq!(
        evaluate_in_context("(function(){ var value=84; value /= 2; return value; })()"),
        Value::Int(42)
    );
}
