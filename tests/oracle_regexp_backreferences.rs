use std::ffi::OsStr;
use std::fmt::Write as _;
use std::process::Command;

use quickjs_oxide::JsString;
use quickjs_oxide::regexp::{CompileErrorKind, compile, execute};

#[derive(Debug)]
struct MatchCase {
    label: &'static str,
    pattern: Vec<u16>,
    flags: &'static str,
    input: Vec<u16>,
}

#[test]
fn regexp_backreferences_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp backreference differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = match_cases();
    let oracle_observations = run_match_oracle(&oracle, &cases);
    assert_eq!(oracle_observations.len(), cases.len());

    for (index, (case, oracle_observation)) in cases.iter().zip(&oracle_observations).enumerate() {
        let pattern = JsString::try_from_utf16(case.pattern.iter().copied()).unwrap();
        let flags = JsString::try_from_utf8(case.flags).unwrap();
        let program = compile(&pattern, &flags).unwrap_or_else(|error| {
            panic!(
                "Rust RegExp compile failed at case {index} ({}): {error}",
                case.label
            )
        });
        let result = execute(&program, &case.input, 0).unwrap_or_else(|error| {
            panic!(
                "Rust RegExp execution failed at case {index} ({}): {error}",
                case.label
            )
        });
        let rust_observation = observe_rust(result.as_ref(), &case.input);
        assert_eq!(
            rust_observation, *oracle_observation,
            "RegExp backreference mismatch at case {index}: {} (pattern {:?}, flags {:?}, input {:?})",
            case.label, case.pattern, case.flags, case.input,
        );
    }
}

#[test]
fn unicode_decimal_escape_errors_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp decimal syntax differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = [
        (r"\1", "u"),
        (r"(a)\2", "u"),
        (r"\10", "u"),
        (r"\00", "u"),
        (r"\08", "u"),
        (r"[\1]", "u"),
        ("(a)(a)(a)(a)(a)(a)(a)(a)(a)(a)\\11", "u"),
    ];
    let oracle_observations = run_compile_oracle(&oracle, &cases);
    assert_eq!(oracle_observations.len(), cases.len());

    for (index, ((pattern, flags), oracle_observation)) in
        cases.iter().zip(&oracle_observations).enumerate()
    {
        let pattern = JsString::try_from_utf8(pattern).unwrap();
        let flags = JsString::try_from_utf8(flags).unwrap();
        let rust_observation = match compile(&pattern, &flags) {
            Ok(_) => "ok",
            Err(error) if matches!(error.kind(), CompileErrorKind::Syntax) => "SyntaxError",
            Err(error) => {
                panic!("Rust RegExp compile produced the wrong error at case {index}: {error}")
            }
        };
        assert_eq!(
            rust_observation, *oracle_observation,
            "RegExp decimal syntax mismatch at case {index}: {pattern:?} / {flags:?}",
        );
    }
}

fn match_cases() -> Vec<MatchCase> {
    let mut cases = vec![
        ascii("basic", r"(ab)\1", "", "abab"),
        ascii("ordinary mismatch", r"(ab)\1", "", "abac"),
        ascii("two references", r"(\d{2})-(\w)\1\2", "", "12-a12a"),
        ascii("forward reference", r"\1(a)", "u", "a"),
        ascii("second forward reference", r"\2(a)(b)", "u", "ab"),
        ascii("self reference", r"(a\1)", "u", "a"),
        ascii("unmatched alternative", r"(a|(b))\2c", "", "ac"),
        ascii("unmatched optional", r"(a)?\1b", "", "b"),
        ascii("empty capture", r"()\1x", "", "x"),
        ascii(
            "empty reference quantifier terminates",
            r"^(a)?\1*$",
            "",
            "",
        ),
        ascii("capture backtracking", r"^(a+)\1$", "", "aaaaaa"),
        ascii("alternative backtracking", r"^(a|ab)+\1$", "", "abab"),
        ascii("nested reset", r"(a(b)?)+\2", "", "aba"),
        ascii("stale capture reset", r"(?:^(a)|\1(a)|(ab)){2}", "", "aab"),
        ascii("last repeated capture", r"^(a|b)+\1$", "", "abb"),
        ascii("legacy ignoreCase", r"(a)\1", "i", "aA"),
        ascii("scoped add ignoreCase", r"(a)(?i:\1)", "", "aA"),
        ascii("scoped remove ignoreCase", r"(a)(?-i:\1)", "i", "aA"),
        ascii("unicode Kelvin folding", r"(k)\1", "iu", "kK"),
        ascii("legacy Kelvin does not fold", r"(k)\1", "i", "kK"),
        ascii("unicode long-s folding", r"(s)\1", "iu", "sſ"),
        ascii("legacy long-s does not fold", r"(s)\1", "i", "sſ"),
        ascii(
            "multi-character folding is forbidden",
            r"(ß)\1",
            "iu",
            "ßSS",
        ),
        ascii("Deseret code-point folding", r"(\u{10400})\1", "iu", "𐐀𐐨"),
        MatchCase {
            label: "capture end does not borrow a trailing surrogate",
            pattern: utf16(r"foo(.+)bar\1"),
            flags: "u",
            input: vec![
                u16::from(b'f'),
                u16::from(b'o'),
                u16::from(b'o'),
                0xd834,
                u16::from(b'b'),
                u16::from(b'a'),
                u16::from(b'r'),
                0xd834,
                0xdc00,
            ],
        },
        MatchCase {
            label: "lone surrogates compare independently",
            pattern: utf16(r"foo(.+)bar\1"),
            flags: "u",
            input: vec![
                u16::from(b'f'),
                u16::from(b'o'),
                u16::from(b'o'),
                0xd834,
                u16::from(b'b'),
                u16::from(b'a'),
                u16::from(b'r'),
                0xd834,
                0xd834,
            ],
        },
        MatchCase {
            label: "Annex B single octal",
            pattern: utf16(r"\1"),
            flags: "",
            input: vec![0x01],
        },
        ascii("Annex B identity eight", r"\8", "", "8"),
        MatchCase {
            label: "Annex B backspace",
            pattern: utf16(r"\10"),
            flags: "",
            input: vec![0x08],
        },
        MatchCase {
            label: "Annex B octal prefix leaves decimal suffix",
            pattern: utf16(r"\18"),
            flags: "",
            input: vec![0x01, u16::from(b'8')],
        },
        MatchCase {
            label: "Annex B maximum octal",
            pattern: utf16(r"\377"),
            flags: "",
            input: vec![0xff],
        },
        ascii("Annex B three-digit width", r"\400", "", " 0"),
        ascii("Annex B suffix after three digits", r"\1234", "", "S4"),
        MatchCase {
            label: "Annex B NUL leaves eight",
            pattern: utf16(r"\08"),
            flags: "",
            input: vec![0, u16::from(b'8')],
        },
        MatchCase {
            label: "valid reference beats Annex B fallback",
            pattern: utf16(r"(.)\1"),
            flags: "",
            input: vec![
                u16::from(b'a'),
                0x01,
                u16::from(b' '),
                u16::from(b'a'),
                u16::from(b'a'),
            ],
        },
        MatchCase {
            label: "Unicode backreference cannot split a surrogate pair",
            pattern: utf16(r"^(.+)\1$"),
            flags: "u",
            input: [
                vec![0xdc00],
                utf16("foobar"),
                vec![0xd834, 0xdc00],
                utf16("foobar"),
                vec![0xd834],
            ]
            .concat(),
        },
        MatchCase {
            label: "Annex B class octal",
            pattern: utf16(r"[\1]"),
            flags: "",
            input: vec![0x01],
        },
        ascii("Annex B class identity eight", r"[\8]", "", "8"),
        MatchCase {
            label: "Annex B class backspace",
            pattern: utf16(r"[\10]"),
            flags: "",
            input: vec![0x08],
        },
    ];

    let ten_captures = "(a)".repeat(10);
    cases.push(ascii_owned(
        "capture ten beats Annex B backspace",
        format!("{ten_captures}\\10"),
        "u",
        "a".repeat(11),
    ));
    cases.push(MatchCase {
        label: "capture eleven is absent so eleven falls back to tab",
        pattern: utf16(&format!("{ten_captures}\\11")),
        flags: "",
        input: [utf16("aaaaaaaaaa"), vec![u16::from(b'\t')]].concat(),
    });
    let eleven_captures = "(a)".repeat(11);
    cases.push(ascii_owned(
        "capture eleven beats Annex B tab",
        format!("{eleven_captures}\\11"),
        "u",
        "a".repeat(12),
    ));
    cases.push(ascii_owned(
        "forward capture ten beats Annex B backspace",
        format!("\\10{ten_captures}"),
        "u",
        "a".repeat(10),
    ));
    cases
}

fn ascii(label: &'static str, pattern: &str, flags: &'static str, input: &str) -> MatchCase {
    MatchCase {
        label,
        pattern: utf16(pattern),
        flags,
        input: utf16(input),
    }
}

fn ascii_owned(
    label: &'static str,
    pattern: String,
    flags: &'static str,
    input: String,
) -> MatchCase {
    MatchCase {
        label,
        pattern: utf16(&pattern),
        flags,
        input: utf16(&input),
    }
}

fn utf16(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn observe_rust(result: Option<&quickjs_oxide::regexp::RegExpMatch>, input: &[u16]) -> String {
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
function __qjo_observe(test) {
    var flags = test.flags;
    if (flags.indexOf("d") < 0) flags += "d";
    if (flags.indexOf("g") < 0 && flags.indexOf("y") < 0) flags += "g";
    var regexp = new RegExp(test.pattern, flags);
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
var __qjo_cases = [
"#,
    );
    for case in cases {
        writeln!(
            source,
            "{{pattern:{},flags:{:?},input:{}}},",
            js_utf16(&case.pattern),
            case.flags,
            js_utf16(&case.input),
        )
        .unwrap();
    }
    source.push_str(
        r#"];
for (var __qjo_i = 0; __qjo_i < __qjo_cases.length; __qjo_i++)
    print(__qjo_i + "|" + __qjo_observe(__qjo_cases[__qjo_i]));
"#,
    );

    run_indexed_oracle(oracle, &source, cases.len(), "RegExp backreference")
}

fn run_compile_oracle(oracle: &OsStr, cases: &[(&str, &str)]) -> Vec<String> {
    let mut source = String::from("var __qjo_cases = [\n");
    for (pattern, flags) in cases {
        writeln!(source, "[{:?},{:?}],", pattern, flags).unwrap();
    }
    source.push_str(
        r#"];
for (var __qjo_i = 0; __qjo_i < __qjo_cases.length; __qjo_i++) {
    var result = "ok";
    try {
        new RegExp(__qjo_cases[__qjo_i][0], __qjo_cases[__qjo_i][1]);
    } catch (error) {
        result = error.name;
    }
    print(__qjo_i + "|" + result);
}
"#,
    );
    run_indexed_oracle(oracle, &source, cases.len(), "RegExp decimal syntax")
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
        String::from_utf8(output.stdout).expect("QJS_ORACLE batch emitted non-UTF-8 output");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), expected, "QJS_ORACLE {label} line count");
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
