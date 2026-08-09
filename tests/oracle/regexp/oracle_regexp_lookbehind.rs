use std::ffi::OsStr;
use std::fmt::Write as _;
use std::process::Command;

use quickjs_oxide::JsString;
use quickjs_oxide::regexp::{CompileErrorKind, compile, execute};

// Differential lock for pinned QuickJS 2026-06-04 backward lookaround.
//
// QuickJS compiles a lookbehind through the same typed assertion frames as a
// lookahead, but reverses each alternative's term order, brackets consuming
// atoms with `prev`, swaps capture boundaries, and selects backward
// backreferences. These vectors pin the interactions that are easiest to lose
// in a Rust rewrite: variable-length reverse matching, left-to-right
// alternative priority, reverse greediness, assertion atomicity and capture
// rollback, forward and mutually recursive references, nested direction
// changes, anchors, word boundaries, and UTF-16/code-point positioning.

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
        label: "positive lookbehind is valid in legacy mode",
        pattern: "(?<=a)",
        flags: "",
        expected: "ok",
    },
    CompileCase {
        label: "negative lookbehind is valid in legacy mode",
        pattern: "(?<!a)",
        flags: "",
        expected: "ok",
    },
    CompileCase {
        label: "positive lookbehind is valid in Unicode mode",
        pattern: "(?<=.)",
        flags: "u",
        expected: "ok",
    },
    CompileCase {
        label: "negative lookbehind is valid in Unicode mode",
        pattern: "(?<!.)",
        flags: "u",
        expected: "ok",
    },
    CompileCase {
        label: "a grouped lookbehind can be quantified",
        pattern: "(?:(?<=a))*",
        flags: "",
        expected: "ok",
    },
    CompileCase {
        label: "legacy positive lookbehind star is a syntax error",
        pattern: "(?<=a)*",
        flags: "",
        expected: "SyntaxError",
    },
    CompileCase {
        label: "legacy negative lookbehind optional is a syntax error",
        pattern: "(?<!a)?",
        flags: "",
        expected: "SyntaxError",
    },
    CompileCase {
        label: "Unicode positive lookbehind plus is a syntax error",
        pattern: "(?<=a)+",
        flags: "u",
        expected: "SyntaxError",
    },
    CompileCase {
        label: "legacy negative lookbehind brace quantifier is a syntax error",
        pattern: "(?<!a){1,2}",
        flags: "",
        expected: "SyntaxError",
    },
    CompileCase {
        label: "legacy lazy lookbehind quantifier is a syntax error",
        pattern: "(?<=a)??",
        flags: "",
        expected: "SyntaxError",
    },
];

#[test]
fn regexp_lookbehind_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp lookbehind oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = match_cases();
    let observations = run_match_oracle(&oracle, &cases);
    assert_eq!(observations.len(), cases.len());
    for (case, observation) in cases.iter().zip(&observations) {
        assert!(
            observation == "N" || observation.starts_with("M|"),
            "lookbehind oracle vector had no match completion for {}: {observation:?}",
            case.label,
        );
    }

    let observations = run_compile_oracle(&oracle, COMPILE_CASES);
    assert_eq!(observations.len(), COMPILE_CASES.len());
    for (case, observation) in COMPILE_CASES.iter().zip(&observations) {
        assert_eq!(
            observation, case.expected,
            "pinned QuickJS lookbehind grammar changed for {}",
            case.label,
        );
    }
}

#[test]
fn regexp_lookbehind_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp lookbehind differential: set QJS_ORACLE to upstream qjs");
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
        "RegExp backward lookaround drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn match_cases() -> Vec<MatchCase> {
    vec![
        ascii(
            "positive fixed-length lookbehind succeeds without consuming its body",
            r"(?<=abc)def",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "positive fixed-length lookbehind rejects a mismatched body",
            r"(?<=abd)def",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "negative lookbehind succeeds where its body cannot match",
            r"(?<!abc)\w{3}",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "negative lookbehind rejects a matching predecessor",
            r"(?<!abc)def",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "unbounded variable-length lookbehind reaches the available prefix",
            r"(?<=\w*)[^abc]{3}",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "a greedy reverse loop captures the complete suffix",
            r"(?<=(b+))c",
            "",
            "abbbbbbc",
            0,
        ),
        ascii(
            "a lazy reverse loop stops at the nearest predecessor",
            r"(?<=(b+?))c",
            "",
            "abbbc",
            0,
        ),
        ascii(
            "reverse execution makes the right capture greedy first",
            r"(?<=([ab]+)([bc]+))$",
            "",
            "abc",
            0,
        ),
        ascii(
            "a repeated capture retains the leftmost reverse iteration",
            r"(?<=(\w){3})def",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "lookbehind alternatives retain source-order priority",
            r".*(?<=(..|...|....))(.*)",
            "",
            "xabcd",
            0,
        ),
        ascii(
            "a later lookbehind alternative runs after the first one fails",
            r".*(?<=(xx|...|....))(.*)",
            "",
            "xabcd",
            0,
        ),
        ascii(
            "a successful lookbehind is atomic under outer failure",
            r"(?<=([abc]+)).\1",
            "",
            "abcdbc",
            0,
        ),
        ascii(
            "a positive lookbehind retains captures for the outer expression",
            r"(?<=(c))def\1?",
            "",
            "abcdefc",
            0,
        ),
        ascii(
            "a successful negative lookbehind rolls its body capture back",
            r"(?<!(^|[ab]))\w{2}",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "outer backtracking rolls an abandoned positive capture back",
            r"(?:(?<=(a))bX|b)\1?$",
            "",
            "ab",
            0,
        ),
        ascii(
            "a numeric forward reference executes after its capture in reverse order",
            r"(?<=\1(\w))d",
            "i",
            "abcCd",
            0,
        ),
        ascii(
            "a variable capture satisfies a numeric forward reference",
            r"(?<=\1(\w+))c",
            "",
            "ababc",
            0,
        ),
        ascii(
            "an outer capture can be compared by a backward backreference",
            r"(.)(?<=(\1\1))",
            "",
            "abb",
            0,
        ),
        ascii(
            "mutually recursive captures preserve QuickJS undefined-reference semantics",
            r"(?<=a(.\2)b(\1)).{4}",
            "",
            "aabcacbc",
            0,
        ),
        ascii(
            "a forward reference and a later backreference can share reverse state",
            r"(?<=a(\2)b(..\1))b",
            "",
            "aacbacb",
            0,
        ),
        ascii(
            "a forward self-reference is empty before its capture is established",
            r"(?<=(?:\1b)(aa)).",
            "",
            "aabaax",
            0,
        ),
        ascii(
            "a backward body can switch to a nested forward lookahead",
            r"(?<=ab(?=c)\wd)\w\w",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "nested lookahead and lookbehind retain their successful capture",
            r"(?<=a(?=([bc]{2}(?<!a{2}))d)\w{3})\w\w",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "a failing nested negative lookbehind rejects the outer assertion",
            r"(?<=a(?=([bc]{2}(?<!a*))d)\w{3})\w\w",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "a forward lookahead can contain a backward lookbehind",
            r"(?=(?<=ab)c)c",
            "",
            "abc",
            0,
        ),
        ascii(
            "start-of-input anchors are evaluated at the reversed position",
            r"(?<=^abc)def",
            "",
            "abcdef",
            0,
        ),
        ascii(
            "multiline start anchors work inside lookbehind",
            r"(?<=^[a-c]{3})def",
            "m",
            "xyz\nabcdef",
            0,
        ),
        ascii(
            "a variable reverse loop can extend to a start anchor",
            r"^foo(?<=^fo+)$",
            "",
            "foo",
            0,
        ),
        ascii(
            "end-of-input remains zero width after a lookbehind",
            r"^foo(?<=foo)$",
            "",
            "foo",
            0,
        ),
        ascii(
            "a word boundary can be tested inside lookbehind",
            r"(?<=\b)[d-f]{3}",
            "",
            "abc def",
            0,
        ),
        ascii(
            "a non-word boundary can be tested inside lookbehind",
            r"(?<=\B)\w{3}",
            "",
            "ab cdef",
            0,
        ),
        ascii(
            "nested backward assertions share the correct word-boundary position",
            r"(?<=\B)(?<=c(?<=\w))\w{3}",
            "",
            "ab cdef",
            0,
        ),
        ascii(
            "ignoreCase applies while matching backward",
            r"(?<=abc)d",
            "i",
            "ABCd",
            0,
        ),
        ascii(
            "Unicode lookbehind consumes one astral code point backward",
            r"(?<=😀)x",
            "u",
            "😀x",
            0,
        ),
        ascii(
            "legacy reverse dot captures only the low surrogate",
            r"(?<=(.))x",
            "",
            "😀x",
            0,
        ),
        ascii(
            "Unicode reverse dot captures the complete astral code point",
            r"(?<=(.))x",
            "u",
            "😀x",
            0,
        ),
        raw(
            "Unicode reverse dot treats a lone high surrogate as one code point",
            r"(?<=(.))x",
            "u",
            vec![0xd83d, u16::from(b'x')],
            0,
        ),
        ascii(
            "Unicode ignoreCase folds an astral literal while moving backward",
            r"(?<=𐐀)x",
            "iu",
            "𐐨x",
            0,
        ),
        ascii(
            "sticky lookbehind succeeds at the exact requested position",
            r"(?<=a)b",
            "y",
            "ab",
            1,
        ),
        ascii(
            "sticky lookbehind does not search past a failing requested position",
            r"(?<=a)b",
            "y",
            "ab",
            0,
        ),
        ascii(
            "Unicode sticky start inside a surrogate pair normalizes backward",
            r"(?<=x)😀",
            "uy",
            "x😀",
            2,
        ),
    ]
}

fn ascii(
    label: &'static str,
    pattern: &str,
    flags: &'static str,
    input: &str,
    start: usize,
) -> MatchCase {
    raw(label, pattern, flags, utf16(input), start)
}

fn raw(
    label: &'static str,
    pattern: &str,
    flags: &'static str,
    input: Vec<u16>,
    start: usize,
) -> MatchCase {
    MatchCase {
        label,
        pattern: utf16(pattern),
        flags,
        input,
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
function __qjo_observe_lookbehind(test) {
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
var __qjo_lookbehind_cases = [
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
for (var __qjo_i = 0; __qjo_i < __qjo_lookbehind_cases.length; __qjo_i++)
    print(__qjo_i + "|" + __qjo_observe_lookbehind(__qjo_lookbehind_cases[__qjo_i]));
"#,
    );
    run_indexed_oracle(oracle, &source, cases.len(), "RegExp lookbehind matching")
}

fn run_compile_oracle(oracle: &OsStr, cases: &[CompileCase]) -> Vec<String> {
    let mut source = String::from("var __qjo_lookbehind_compile_cases = [\n");
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
for (var __qjo_i = 0; __qjo_i < __qjo_lookbehind_compile_cases.length; __qjo_i++) {
    var result = "ok";
    try {
        new RegExp(__qjo_lookbehind_compile_cases[__qjo_i].pattern,
                   __qjo_lookbehind_compile_cases[__qjo_i].flags);
    } catch (error) {
        result = error.name;
    }
    print(__qjo_i + "|" + result);
}
"#,
    );
    run_indexed_oracle(oracle, &source, cases.len(), "RegExp lookbehind grammar")
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
        String::from_utf8(output.stdout).expect("QJS_ORACLE lookbehind batch emitted non-UTF-8");
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
