use std::ffi::OsStr;
use std::fmt::Write as _;
use std::process::Command;

use quickjs_oxide::JsString;
use quickjs_oxide::regexp::{CompileErrorKind, compile, execute};

// Differential lock for pinned QuickJS 2026-06-04 forward lookahead.
//
// QuickJS compiles assertions as begin/end opcode pairs in `libregexp.c`
// `re_parse_term` (1901-1981), then executes them with typed positive and
// negative backtracking frames (2704-2925, 2967-2978). These vectors pin the
// semantics that are easy to lose when adapting that model to Rust: zero-width
// input restoration, positive capture retention, rollback on every negative or
// abandoned path, positive assertion atomicity, nesting, Annex B quantified
// assertions, scoped modifiers, and UTF-16/code-point boundaries.

#[derive(Debug)]
struct MatchCase {
    label: &'static str,
    pattern: Vec<u16>,
    flags: &'static str,
    input: Vec<u16>,
    start: usize,
}

#[derive(Clone, Copy, Debug)]
struct CompileCase {
    label: &'static str,
    pattern: &'static str,
    flags: &'static str,
    expected: &'static str,
}

const COMPILE_CASES: &[CompileCase] = &[
    CompileCase {
        label: "positive lookahead is valid in Unicode mode",
        pattern: "(?=.)",
        flags: "u",
        expected: "ok",
    },
    CompileCase {
        label: "negative lookahead is valid in Unicode mode",
        pattern: "(?!.)",
        flags: "u",
        expected: "ok",
    },
    CompileCase {
        label: "Annex B permits a quantified positive lookahead",
        pattern: "(?=.)*",
        flags: "",
        expected: "ok",
    },
    CompileCase {
        label: "Annex B permits a lazy quantified negative lookahead",
        pattern: "(?!.)??",
        flags: "",
        expected: "ok",
    },
    CompileCase {
        label: "Unicode positive lookahead star is a syntax error",
        pattern: "(?=.)*",
        flags: "u",
        expected: "SyntaxError",
    },
    CompileCase {
        label: "Unicode negative lookahead plus is a syntax error",
        pattern: "(?!.)+",
        flags: "u",
        expected: "SyntaxError",
    },
    CompileCase {
        label: "Unicode positive lookahead brace quantifier is a syntax error",
        pattern: "(?=.){1,2}",
        flags: "u",
        expected: "SyntaxError",
    },
    CompileCase {
        label: "Unicode lazy lookahead quantifier is a syntax error",
        pattern: "(?=.)??",
        flags: "u",
        expected: "SyntaxError",
    },
];

#[test]
fn regexp_lookahead_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp lookahead oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = match_cases();
    let observations = run_match_oracle(&oracle, &cases);
    assert_eq!(observations.len(), cases.len());
    for (case, observation) in cases.iter().zip(&observations) {
        assert!(
            observation == "N" || observation.starts_with("M|"),
            "lookahead oracle vector had no match completion for {}: {observation:?}",
            case.label,
        );
    }

    let observations = run_compile_oracle(&oracle, COMPILE_CASES);
    assert_eq!(observations.len(), COMPILE_CASES.len());
    for (case, observation) in COMPILE_CASES.iter().zip(&observations) {
        assert_eq!(
            observation, case.expected,
            "pinned QuickJS lookahead grammar changed for {}",
            case.label,
        );
    }
}

#[test]
fn regexp_lookahead_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp lookahead differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = match_cases();
    let oracle_observations = run_match_oracle(&oracle, &cases);
    assert_eq!(oracle_observations.len(), cases.len());

    let mut failures = Vec::new();
    for (index, (case, expected)) in cases.iter().zip(&oracle_observations).enumerate() {
        let actual = observe_rust_match(case);
        if actual != *expected {
            failures.push(format!(
                "match case {index}: {}\npattern: {:?}\nflags: {:?}\ninput: {:?}\nstart: {}\noxide: {actual:?}\noracle: {expected:?}",
                case.label, case.pattern, case.flags, case.input, case.start,
            ));
        }
    }

    let oracle_observations = run_compile_oracle(&oracle, COMPILE_CASES);
    for (index, (case, expected)) in COMPILE_CASES.iter().zip(&oracle_observations).enumerate() {
        let actual = observe_rust_compile(case);
        if actual != *expected {
            failures.push(format!(
                "compile case {index}: {}\npattern: {:?}\nflags: {:?}\noxide: {actual:?}\noracle: {expected:?}",
                case.label, case.pattern, case.flags,
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "RegExp forward lookahead drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn match_cases() -> Vec<MatchCase> {
    vec![
        ascii(
            "positive assertion succeeds without consuming twice",
            r"^a(?=b)b$",
            "",
            "ab",
            0,
        ),
        ascii(
            "positive assertion failure rejects the branch",
            r"^a(?=b)c$",
            "",
            "ac",
            0,
        ),
        ascii(
            "negative assertion succeeds after its body fails",
            r"^a(?!b)c$",
            "",
            "ac",
            0,
        ),
        ascii(
            "negative assertion fails after its body succeeds",
            r"^a(?!b)b$",
            "",
            "ab",
            0,
        ),
        ascii(
            "positive assertion retains captures for an outer backreference",
            r"^(?=(a+))\1$",
            "",
            "aaa",
            0,
        ),
        ascii(
            "positive body failure rolls its capture back before an outer alternative",
            r"^(?:(?=(a))b|a)\1?$",
            "",
            "a",
            0,
        ),
        ascii(
            "outer backtracking rolls a committed positive capture back",
            r"^(?:(?=(a))abX|a)\1?$",
            "",
            "a",
            0,
        ),
        ascii(
            "negative body failure rolls its temporary capture back",
            r"^(?!(a)b)a\1?$",
            "",
            "a",
            0,
        ),
        ascii(
            "negative body success rolls its capture back before an outer alternative",
            r"^(?:(?!(a))b|a)\1?$",
            "",
            "a",
            0,
        ),
        ascii(
            "positive assertion is atomic after choosing its first alternative",
            r"^(?=(a|aa))\1b$",
            "",
            "aab",
            0,
        ),
        ascii(
            "positive assertion succeeds when its chosen alternative is sufficient",
            r"^(?=(aa|a))\1b$",
            "",
            "aab",
            0,
        ),
        ascii(
            "negative assertion exhausts internal alternatives before succeeding",
            r"^(?!(a|aa)c)aab$",
            "",
            "aab",
            0,
        ),
        ascii(
            "negative assertion fails when a later internal alternative succeeds",
            r"^(?!(a|aa)b)aab$",
            "",
            "aab",
            0,
        ),
        ascii(
            "nested positive assertions retain the innermost capture",
            r"^(?=(?=(a))\1)\1$",
            "",
            "a",
            0,
        ),
        ascii(
            "positive assertion can contain a successful negative assertion",
            r"^(?=(?!b)(a))\1$",
            "",
            "a",
            0,
        ),
        ascii(
            "Annex B quantified positive assertion remains zero width",
            r".(?=Z)+",
            "",
            "a bZ cZZ",
            0,
        ),
        ascii(
            "Annex B quantified negative assertion may choose zero repetitions",
            r".(?!Z)*",
            "",
            "aZ",
            0,
        ),
        ascii(
            "zero-count optional assertion resets its nested capture",
            r"(?:(?=(abc)))?a",
            "",
            "abc",
            0,
        ),
        ascii(
            "one exact assertion repetition retains its nested capture",
            r"(?:(?=(abc))){1,1}a",
            "",
            "abc",
            0,
        ),
        ascii(
            "empty Annex B lookahead star terminates",
            r"^(?=)*$",
            "",
            "",
            0,
        ),
        ascii(
            "Unicode quantified wrapper stops after a zero-advance assertion",
            r"^(?:(?=(a)))*a$",
            "u",
            "a",
            0,
        ),
        ascii(
            "scoped ignoreCase applies to an enclosing lookahead",
            r"^(?i:(?=a)a)$",
            "",
            "A",
            0,
        ),
        ascii(
            "scoped ignoreCase is restored after the lookahead group",
            r"^(?i:(?=(a)))a$",
            "",
            "A",
            0,
        ),
        ascii(
            "capture text leaves an ignoreCase scope intact",
            r"^(?=(?i:(a)))\1$",
            "",
            "A",
            0,
        ),
        ascii(
            "scoped removal of global ignoreCase applies inside lookahead",
            r"^(?=(?-i:a))a$",
            "i",
            "A",
            0,
        ),
        ascii(
            "Unicode lookahead dot captures one astral code point",
            r"^(?=(.)).$",
            "u",
            "😀",
            0,
        ),
        ascii(
            "legacy lookahead dot captures one astral UTF-16 code unit",
            r"^(?=(.))..$",
            "",
            "😀",
            0,
        ),
        ascii(
            "Unicode ignoreCase folds an astral capture and backreference",
            r"^(?=(𐐀))\1$",
            "iu",
            "𐐨",
            0,
        ),
        ascii(
            "Unicode search start inside a surrogate pair normalizes before lookahead",
            r"(?=😀)😀",
            "u",
            "x😀",
            2,
        ),
        ascii("empty positive assertion succeeds", r"^(?=)a$", "", "a", 0),
        ascii("empty negative assertion fails", r"^(?!)a$", "", "a", 0),
    ]
}

fn ascii(
    label: &'static str,
    pattern: &str,
    flags: &'static str,
    input: &str,
    start: usize,
) -> MatchCase {
    MatchCase {
        label,
        pattern: utf16(pattern),
        flags,
        input: utf16(input),
        start,
    }
}

fn utf16(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn observe_rust_match(case: &MatchCase) -> String {
    let pattern = JsString::try_from_utf16(case.pattern.iter().copied()).unwrap();
    let flags = JsString::try_from_utf8(case.flags).unwrap();
    let program = match compile(&pattern, &flags) {
        Ok(program) => program,
        Err(error) => {
            return format!(
                "CompileError|{:?}|{}|{}",
                error.kind(),
                error.position(),
                error.message(),
            );
        }
    };
    match execute(&program, &case.input, case.start) {
        Ok(result) => observe_rust_result(result.as_ref(), &case.input),
        Err(error) => format!("ExecError|{error}"),
    }
}

fn observe_rust_compile(case: &CompileCase) -> String {
    let pattern = JsString::try_from_utf8(case.pattern).unwrap();
    let flags = JsString::try_from_utf8(case.flags).unwrap();
    match compile(&pattern, &flags) {
        Ok(_) => "ok".to_owned(),
        Err(error) if matches!(error.kind(), CompileErrorKind::Syntax) => "SyntaxError".to_owned(),
        Err(error) => format!("CompileError|{:?}", error.kind()),
    }
}

fn observe_rust_result(
    result: Option<&quickjs_oxide::regexp::RegExpMatch>,
    input: &[u16],
) -> String {
    let Some(result) = result else {
        return "N".to_owned();
    };
    let complete = result
        .capture(0)
        .expect("a successful match has capture zero");
    let mut output = format!("M|{}|{}", complete.start, result.captures().len());
    for capture in result.captures() {
        match capture {
            None => output.push_str("|U"),
            Some(range) => {
                write!(output, "|{},{}:", range.start, range.end).unwrap();
                for unit in &input[range.clone()] {
                    write!(output, "{unit:04x}").unwrap();
                }
            }
        }
    }
    output
}

fn run_match_oracle(oracle: &OsStr, cases: &[MatchCase]) -> Vec<String> {
    let mut source = String::from(
        r#"
function __qjo_hex_utf16(value) {
    var result = "";
    for (var i = 0; i < value.length; i++)
        result += ("0000" + value.charCodeAt(i).toString(16)).slice(-4);
    return result;
}
function __qjo_observe_lookahead(test) {
    var flags = test.flags;
    if (flags.indexOf("d") < 0) flags += "d";
    if (flags.indexOf("g") < 0 && flags.indexOf("y") < 0) flags += "g";
    var regexp = new RegExp(test.pattern, flags);
    regexp.lastIndex = test.start;
    var match = regexp.exec(test.input);
    if (match === null) return "N";
    var fields = ["M", String(match.index), String(match.length)];
    for (var i = 0; i < match.length; i++) {
        if (match[i] === undefined) {
            fields.push("U");
        } else {
            var range = match.indices[i];
            fields.push(String(range[0]) + "," + String(range[1]) + ":" +
                        __qjo_hex_utf16(match[i]));
        }
    }
    return fields.join("|");
}
var __qjo_lookahead_cases = [
"#,
    );
    for case in cases {
        writeln!(
            source,
            "{{pattern:{},flags:{:?},input:{},start:{}}},",
            js_utf16(&case.pattern),
            case.flags,
            js_utf16(&case.input),
            case.start,
        )
        .unwrap();
    }
    source.push_str(
        r#"];
for (var __qjo_i = 0; __qjo_i < __qjo_lookahead_cases.length; __qjo_i++)
    print(__qjo_i + "|" + __qjo_observe_lookahead(__qjo_lookahead_cases[__qjo_i]));
"#,
    );
    run_indexed_oracle(oracle, &source, cases.len(), "RegExp lookahead matching")
}

fn run_compile_oracle(oracle: &OsStr, cases: &[CompileCase]) -> Vec<String> {
    let mut source = String::from("var __qjo_lookahead_compile_cases = [\n");
    for case in cases {
        writeln!(
            source,
            "{{pattern:{},flags:{:?}}},",
            js_utf16(&utf16(case.pattern)),
            case.flags,
        )
        .unwrap();
    }
    source.push_str(
        r#"];
for (var __qjo_i = 0; __qjo_i < __qjo_lookahead_compile_cases.length; __qjo_i++) {
    var result = "ok";
    try {
        new RegExp(__qjo_lookahead_compile_cases[__qjo_i].pattern,
                   __qjo_lookahead_compile_cases[__qjo_i].flags);
    } catch (error) {
        result = error.name;
    }
    print(__qjo_i + "|" + result);
}
"#,
    );
    run_indexed_oracle(oracle, &source, cases.len(), "RegExp lookahead grammar")
}

fn run_indexed_oracle(oracle: &OsStr, source: &str, expected: usize, label: &str) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not execute QJS_ORACLE {label} batch: {error}"));
    assert!(
        output.status.success(),
        "QJS_ORACLE {label} batch failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout =
        String::from_utf8(output.stdout).expect("QJS_ORACLE lookahead batch emitted non-UTF-8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        expected,
        "QJS_ORACLE {label} emitted the wrong line count",
    );
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let prefix = format!("{index}|");
            line.strip_prefix(&prefix)
                .unwrap_or_else(|| panic!("QJS_ORACLE {label} index mismatch: {line:?}"))
                .to_owned()
        })
        .collect()
}

fn js_utf16(units: &[u16]) -> String {
    let mut source = String::from("\"");
    for unit in units {
        write!(source, "\\u{unit:04x}").unwrap();
    }
    source.push('"');
    source
}
