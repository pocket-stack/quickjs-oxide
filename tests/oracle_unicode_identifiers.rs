use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::lexer::{Lexer, TokenKind};
use quickjs_oxide::{Runtime, RuntimeError, Value};

#[test]
fn unicode_identifier_execution_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Unicode identifier differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let newest_unicode_17_start = char::from_u32(0x33479).unwrap();
    let cases = vec![
        (
            "direct BMP identifier",
            "(function(){ var π = 40; return π + 2; })()".to_owned(),
        ),
        (
            "direct and escaped astral identity",
            r"(function(){ var 𐐀 = 42; return \u{10400}; })()".to_owned(),
        ),
        (
            "arbitrarily zero-padded astral escape",
            r"(function(){ var \u{0000000000010400} = 42; return 𐐀; })()".to_owned(),
        ),
        (
            "combining-mark continuation",
            "(function(){ var a\u{0300} = 42; return a\\u0300; })()".to_owned(),
        ),
        (
            "join-control continuations",
            "(function(){ var a\u{200c}\u{200d} = 42; return a\\u200c\\u200d; })()".to_owned(),
        ),
        (
            "identifier spelling is not normalized",
            "(function(){ var é = 4; var e\u{0301} = 2; return é * 10 + e\u{0301}; })()".to_owned(),
        ),
        (
            "ID Start differs from XID Start",
            "(function(){ var ͺ = 42; return ͺ; })()".to_owned(),
        ),
        (
            "Unicode 17 upper identifier addition",
            format!(
                "(function(){{ var {newest_unicode_17_start} = 42; return {newest_unicode_17_start}; }})()"
            ),
        ),
        (
            "Unicode parameter and local resolution",
            "(function(变量){ var résumé = 1; return 变量 + résumé; })(41)".to_owned(),
        ),
        (
            "escaped identifier closure capture",
            r"(function(){ var π = 40; return (function(){ return \u03c0 + 2; })(); })()"
                .to_owned(),
        ),
        (
            "Unicode named function self binding",
            "(function 名字(n){ return n ? 名字(n - 1) : 42; })(2)".to_owned(),
        ),
        (
            "astral continue-only digit",
            "(function(){ var a𝟎 = 42; return a\\u{1d7ce}; })()".to_owned(),
        ),
        (
            "direct and escaped Unicode global atom relay",
            r"(function(){ 𐐀全局 = 40; return (function(){ return \u{10400}全局 + 2; })(); })()"
                .to_owned(),
        ),
    ];

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for (description, source) in cases {
        assert_eq!(
            context.eval(&source).unwrap_or_else(|error| {
                panic!("Rust rejected {description:?} ({source:?}): {error}")
            }),
            Value::Int(42),
            "Rust result for {description:?}"
        );
        assert_eq!(
            oracle_number_observation(&oracle, &source, description),
            "number|42",
            "QuickJS result for {description:?}"
        );
    }
}

#[test]
fn unicode_17_identifier_classification_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Unicode table differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let rust = rust_classification_observation();
    let source = r#"
var idStart = /^\p{ID_Start}$/u;
var idContinue = /^\p{ID_Continue}$/u;
var startCount = 0, continueCount = 0, hash = 2166136261;
var planeStart = 0, planeContinue = 0, planeHash = 2166136261, planes = [];
for (var codePoint = 0; codePoint <= 0x10ffff; codePoint++) {
    var text = String.fromCodePoint(codePoint);
    var start = codePoint === 0x24 || codePoint === 0x5f || idStart.test(text);
    var continuation = codePoint === 0x24 || idContinue.test(text);
    if (start) startCount++;
    if (continuation) continueCount++;
    if (start) planeStart++;
    if (continuation) planeContinue++;
    var marker = (start ? 1 : 0) | (continuation ? 2 : 0);
    hash = Math.imul((hash ^ marker) >>> 0, 16777619) >>> 0;
    planeHash = Math.imul((planeHash ^ marker) >>> 0, 16777619) >>> 0;
    if ((codePoint & 0xffff) === 0xffff) {
        planes.push(planeStart + ":" + planeContinue + ":" + planeHash);
        planeStart = 0;
        planeContinue = 0;
        planeHash = 2166136261;
    }
}

print(startCount + "|" + continueCount + "|" + hash + "|" + planes.join(";"));
"#;
    let output = Command::new(&oracle)
        .args(["--std", "-e", source])
        .output()
        .expect("run QuickJS Unicode classification oracle");
    assert!(
        output.status.success(),
        "QuickJS Unicode classification failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let quickjs = String::from_utf8(output.stdout)
        .expect("QuickJS Unicode classification emitted non-UTF-8")
        .trim()
        .to_owned();

    assert!(rust.starts_with("145918|149241|1136856707|"));
    assert_eq!(rust, quickjs);
}

#[test]
fn escaped_new_target_pseudo_keyword_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP escaped new.target differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for source in [
        r"(function(){ return new.\u0074arget; })()",
        r"(function(){ return new.t\u0061rget; })()",
    ] {
        assert_eq!(
            rust_error_observation(source),
            "SyntaxError|expecting target"
        );
        assert_eq!(
            oracle_error_observation(&oracle, source),
            "SyntaxError|expecting target"
        );
    }
}

#[test]
fn identifier_diagnostic_priority_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP identifier diagnostic differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = [
        (
            r"(function(){ var \u0069f\u{}=14; })()",
            "SyntaxError|'if' is a reserved identifier|1:18",
        ),
        (
            r"(function(){ var if\u{}=14; })()",
            "SyntaxError|'if' is a reserved identifier|1:18",
        ),
        (
            r"(function(){ var if\x61=1; })()",
            "SyntaxError|variable name expected|1:18",
        ),
        (
            r"(function(){ var \u{}=1; })()",
            "SyntaxError|variable name expected|1:18",
        ),
        (
            r"(function(){ var a\u{}=1; })()",
            "SyntaxError|expecting ';'|1:19",
        ),
        (
            r"(function(){ var a\u{2d}=1; })()",
            "SyntaxError|expecting ';'|1:19",
        ),
        ("1π", "SyntaxError|invalid number literal|1:1"),
        (r"1\u0300", "SyntaxError|expecting ';'|1:2"),
        (
            r"(function(){ return new.\u0074arget; })()",
            "SyntaxError|expecting target|1:25",
        ),
        (
            r"(function(){ return new.t\u0061rget; })()",
            "SyntaxError|expecting target|1:25",
        ),
        (
            r"\u{}",
            "SyntaxError|unexpected token in expression: '\\'|1:1",
        ),
        (r"return \u{}", "SyntaxError|return not in a function|1:1"),
        ("a😀", "SyntaxError|unexpected character|1:2"),
        (
            r"(function(){ var \uD800=1; })()",
            "SyntaxError|variable name expected|1:18",
        ),
        (
            r"(function(){ var a\uD800=1; })()",
            "SyntaxError|expecting ';'|1:19",
        ),
        (
            r"(function(){ var 𐐀\u{}=1; })()",
            "SyntaxError|expecting ';'|1:19",
        ),
        (
            r"#\u{}",
            "SyntaxError|invalid first character of private name|1:1",
        ),
        (
            "(function(){ var \\u0069f=14; \"unterminated })()",
            "SyntaxError|'if' is a reserved identifier|1:18",
        ),
        (
            "(function(eval){ \"use strict\"; \"x\"; \"unterminated })()",
            "SyntaxError|unexpected end of string|1:37",
        ),
        (
            "(function(eval){ \"use strict\"; \"ok\"; \"\\1\"; })()",
            "SyntaxError|octal escape sequences are not allowed in strict mode|1:39",
        ),
        (
            "(function(){ return new.\\u0074arget; \"unterminated })()",
            "SyntaxError|expecting target|1:25",
        ),
        (
            "(function(a b \"unterminated){})",
            "SyntaxError|expecting ','|1:13",
        ),
        (
            "(function(){ return (1 2 \"unterminated); })()",
            "SyntaxError|expecting ')'|1:24",
        ),
        (
            "(function(eval){ \"use strict\"; return 1; \"unterminated })()",
            "SyntaxError|invalid argument name in strict code|1:18",
        ),
        (
            "(function(eval){\"use strict\";})()",
            "SyntaxError|invalid argument name in strict code|1:17",
        ),
        (
            "(function(eval){\"use strict\";\\u{};})()",
            "SyntaxError|invalid argument name in strict code|1:17",
        ),
        (
            "(function(implements){\"use strict\";})()",
            "SyntaxError|invalid argument name in strict code|1:23",
        ),
        (
            "(function impl\\u0065ments(){\"use strict\";})()",
            "SyntaxError|invalid function name in strict code|1:29",
        ),
        (
            "(function(){\"use strict\";var eval=1;})()",
            "SyntaxError|invalid variable name in strict mode|1:34",
        ),
        (
            "(function(){\"use strict\";var arguments=1;})()",
            "SyntaxError|invalid variable name in strict mode|1:39",
        ),
        (
            "(function(){\"use strict\";var eval \"unterminated})()",
            "SyntaxError|unexpected end of string|1:35",
        ),
        (
            "(function(){\"use strict\";var arguments \"unterminated})()",
            "SyntaxError|unexpected end of string|1:40",
        ),
        (
            "(function(){ throw\n'unterminated })()",
            "SyntaxError|unexpected end of string|2:1",
        ),
        (
            "(function(){ throw\n\\u{}; })()",
            "SyntaxError|line terminator not allowed after throw|2:1",
        ),
    ];

    for (source, expected) in cases {
        assert_eq!(
            rust_diagnostic_observation(source),
            expected,
            "Rust: {source}"
        );
        assert_eq!(
            oracle_diagnostic_observation(&oracle, source),
            expected,
            "QuickJS: {source}"
        );
    }
}

fn rust_classification_observation() -> String {
    let mut start_count = 0_u32;
    let mut continue_count = 0_u32;
    let mut hash = 2_166_136_261_u32;
    let mut plane_start = 0_u32;
    let mut plane_continue = 0_u32;
    let mut plane_hash = 2_166_136_261_u32;
    let mut planes = Vec::with_capacity(17);
    for code_point in 0..=0x10ffff {
        let start = char::from_u32(code_point).is_some_and(lexer_accepts_start);
        let continuation = char::from_u32(code_point).is_some_and(lexer_accepts_continue);
        start_count += u32::from(start);
        continue_count += u32::from(continuation);
        plane_start += u32::from(start);
        plane_continue += u32::from(continuation);
        let marker = u32::from(start) | (u32::from(continuation) << 1);
        hash = (hash ^ marker).wrapping_mul(16_777_619);
        plane_hash = (plane_hash ^ marker).wrapping_mul(16_777_619);
        if code_point & 0xffff == 0xffff {
            planes.push(format!("{plane_start}:{plane_continue}:{plane_hash}"));
            plane_start = 0;
            plane_continue = 0;
            plane_hash = 2_166_136_261;
        }
    }
    format!("{start_count}|{continue_count}|{hash}|{}", planes.join(";"))
}

fn lexer_accepts_start(ch: char) -> bool {
    let source = ch.to_string();
    matches!(
        Lexer::new(&source).next_token(),
        Ok(token)
            if matches!(token.kind, TokenKind::Identifier(ref identifier) if identifier.raw == source)
    )
}

fn lexer_accepts_continue(ch: char) -> bool {
    let mut source = String::from("A");
    source.push(ch);
    matches!(
        Lexer::new(&source).next_token(),
        Ok(token)
            if matches!(token.kind, TokenKind::Identifier(ref identifier) if identifier.raw == source)
    )
}

fn oracle_number_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = format!("var result = {source}; print(typeof result + '|' + String(result));");
    let output = Command::new(oracle)
        .args(["--std", "-e", &script])
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS for {description:?} ({source:?}): {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS rejected {description:?} ({source:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Unicode identifier output was not UTF-8")
        .trim()
        .to_owned()
}

fn rust_error_observation(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Rust parser did not materialize an Error object for {source:?}");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    let Value::String(name) = context.get_property(&error, &name).unwrap() else {
        panic!("Rust parser Error.name was not a string for {source:?}");
    };
    let Value::String(message) = context.get_property(&error, &message).unwrap() else {
        panic!("Rust parser Error.message was not a string for {source:?}");
    };
    format!("{}|{}", name.to_utf8_lossy(), message.to_utf8_lossy())
}

fn oracle_error_observation(oracle: &OsStr, source: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {source:?}: {error}"));
    assert!(!output.status.success(), "QuickJS accepted {source:?}");
    let stderr = String::from_utf8(output.stderr).expect("QuickJS error output was not UTF-8");
    let line = stderr
        .lines()
        .find(|line| line.starts_with("SyntaxError: "))
        .unwrap_or_else(|| panic!("QuickJS emitted no SyntaxError for {source:?}: {stderr}"));
    format!(
        "SyntaxError|{}",
        line.strip_prefix("SyntaxError: ").unwrap()
    )
}

fn rust_diagnostic_observation(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Rust parser did not materialize an Error object for {source:?}");
    };
    let read = |context: &mut quickjs_oxide::Context, name: &str| {
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

fn oracle_diagnostic_observation(oracle: &OsStr, source: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {source:?}: {error}"));
    assert!(!output.status.success(), "QuickJS accepted {source:?}");
    let stderr = String::from_utf8(output.stderr).expect("QuickJS error output was not UTF-8");
    let mut lines = stderr.lines();
    let first = lines
        .find(|line| line.starts_with("SyntaxError: "))
        .unwrap_or_else(|| panic!("QuickJS emitted no SyntaxError for {source:?}: {stderr}"));
    let location = lines
        .find_map(|line| line.trim().strip_prefix("at <cmdline>:"))
        .unwrap_or_else(|| panic!("QuickJS emitted no location for {source:?}: {stderr}"));
    format!(
        "SyntaxError|{}|{location}",
        first.strip_prefix("SyntaxError: ").unwrap()
    )
}
