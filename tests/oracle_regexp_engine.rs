use std::ffi::OsStr;
use std::fmt::Write as _;
use std::process::Command;

use quickjs_oxide::JsString;
use quickjs_oxide::regexp::{compile, execute};

#[derive(Debug)]
struct Case {
    label: &'static str,
    pattern: Vec<u16>,
    flags: &'static str,
    input: Vec<u16>,
    start: usize,
}

#[test]
fn regexp_compiler_and_executor_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp engine differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = cases();
    let oracle_observations = run_oracle_batch(&oracle, &cases);
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
        let result = execute(&program, &case.input, case.start).unwrap_or_else(|error| {
            panic!(
                "Rust RegExp execution failed at case {index} ({}): {error}",
                case.label
            )
        });
        let rust_observation = observe_rust(result.as_ref(), &case.input);
        assert_eq!(
            rust_observation, *oracle_observation,
            "RegExp differential mismatch at case {index}: {} (pattern {:?}, flags {:?}, input {:?}, start {})",
            case.label, case.pattern, case.flags, case.input, case.start
        );
    }
}

fn cases() -> Vec<Case> {
    let mut cases = vec![
        ascii("empty pattern at explicit start", "", "", "abc", 1),
        ascii("literal leftmost search", "ab", "", "zzababa", 0),
        ascii("literal no match", "ab", "", "zzac", 0),
        ascii("sticky match at start", "ab", "y", "zab", 1),
        ascii("sticky miss does not scan", "ab", "y", "zab", 0),
        ascii("dot skips line terminator", ".", "", "\nq", 0),
        ascii("dotAll consumes line terminator", ".", "s", "\nq", 0),
        ascii("multiline anchors", "^b$", "m", "a\nb\r\nc", 0),
        ascii(
            "single-line anchors reject interior",
            "^b$",
            "",
            "a\nb\n",
            0,
        ),
        ascii("left alternative wins", "a|ab", "", "ab", 0),
        ascii("capturing and optional group", "(ab|a)(b)?", "", "ab", 0),
        ascii("noncapturing group repetition", "(?:ab)+", "", "zzababx", 0),
        ascii("greedy star", "a*", "", "aaab", 0),
        ascii("lazy star", "a*?", "", "aaab", 0),
        ascii("lazy plus constrained by suffix", "a+?b", "", "aaab", 0),
        ascii("optional atom present", "colou?r", "", "colour", 0),
        ascii("optional atom absent", "colou?r", "", "color", 0),
        ascii("class ranges and union", "[a-cx]+", "", "zzxbc!", 0),
        ascii("inverted class", "[^0-9]+", "", "12ab3", 0),
        ascii("digit shorthands", "\\d+\\D", "", "a12x", 0),
        ascii("word shorthands", "\\w+\\W", "", "__9!", 0),
        ascii("word boundaries", "\\bcat\\b", "", "-cat!", 0),
        ascii("inverted word boundary", "\\Bcat", "", "scat", 0),
        ascii(
            "hex unicode and newline escapes",
            "\\x41\\u0042\\n",
            "",
            "xAB\n",
            0,
        ),
        ascii(
            "control character escapes",
            "\\f\\t\\r\\v",
            "",
            "\u{c}\t\r\u{b}",
            0,
        ),
        ascii("end anchor finds terminal empty match", "$", "", "abc", 0),
        ascii("search starts at requested UTF-16 index", "a", "", "ba", 1),
        ascii("alternation rolls back captures", "(a)|(b)", "", "b", 0),
        ascii("repetition keeps final capture", "(a)+", "", "aaa", 0),
        ascii(
            "repetition resets nested optional capture",
            "(a(b)?)+",
            "",
            "aba",
            0,
        ),
        ascii("nullable optional capture rollback", "(a?)?", "", "", 0),
        ascii("nullable finite capture rollback", "(a|){0,2}", "", "a", 0),
        ascii("NUL escape inside class", r"[\0]", "", "\0", 0),
        // These four cases intentionally lock the pattern-side
        // canonicalization required by QuickJS, not just input-side folding.
        ascii("lower literal ignoreCase", "a", "i", "A", 0),
        ascii("upper literal unicode ignoreCase", "A", "iu", "a", 0),
        ascii("lower class ignoreCase", "[a]", "i", "A", 0),
        ascii("upper range unicode ignoreCase", "[A-Z]", "iu", "q", 0),
    ];

    cases.push(Case {
        label: "non-ASCII whitespace shorthand",
        pattern: utf16("\\s+\\S"),
        flags: "",
        input: vec![0x00a0, 0x3000, u16::from(b'x')],
        start: 0,
    });
    cases.push(Case {
        label: "unicode dot consumes one surrogate pair",
        pattern: utf16("(.)"),
        flags: "u",
        input: vec![0xd83d, 0xde00, u16::from(b'x')],
        start: 0,
    });
    cases.push(Case {
        label: "non-unicode dot consumes one code unit",
        pattern: utf16("(.)"),
        flags: "",
        input: vec![0xd83d, 0xde00, u16::from(b'x')],
        start: 0,
    });
    cases.push(Case {
        label: "unicode astral literal searches by UTF-16 index",
        pattern: vec![0xd83d, 0xde00],
        flags: "u",
        input: vec![u16::from(b'x'), 0xd83d, 0xde00, u16::from(b'y')],
        start: 0,
    });
    cases.push(Case {
        label: "legacy lone surrogate literal",
        pattern: vec![0xd800],
        flags: "",
        input: vec![u16::from(b'x'), 0xd800, u16::from(b'y')],
        start: 0,
    });
    cases.push(Case {
        label: "unicode ignoreCase word complement excludes Kelvin sign",
        pattern: utf16(r"[\W]"),
        flags: "iu",
        input: vec![0x212a],
        start: 0,
    });
    cases.push(Case {
        label: "unicode ignoreCase double word complement includes long s",
        pattern: utf16(r"[^\W]"),
        flags: "iu",
        input: vec![0x017f],
        start: 0,
    });
    cases
}

fn ascii(
    label: &'static str,
    pattern: &str,
    flags: &'static str,
    input: &str,
    start: usize,
) -> Case {
    Case {
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

fn run_oracle_batch(oracle: &OsStr, cases: &[Case]) -> Vec<String> {
    let mut source = String::from(
        r#"
function __qjo_hex_utf16(value) {
    var result = "";
    for (var i = 0; i < value.length; i++)
        result += ("0000" + value.charCodeAt(i).toString(16)).slice(-4);
    return result;
}
function __qjo_regexp_observation(test) {
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
var __qjo_regexp_cases = [
"#,
    );
    for case in cases {
        writeln!(
            source,
            "{{pattern:{},flags:{:?},input:{},start:{}}},",
            js_utf16(&case.pattern),
            case.flags,
            js_utf16(&case.input),
            case.start
        )
        .unwrap();
    }
    source.push_str(
        r#"];
for (var __qjo_i = 0; __qjo_i < __qjo_regexp_cases.length; __qjo_i++)
    print(__qjo_i + "|" + __qjo_regexp_observation(__qjo_regexp_cases[__qjo_i]));
"#,
    );

    let output = Command::new(oracle)
        .args(["-e", &source])
        .output()
        .unwrap_or_else(|error| panic!("could not execute QJS_ORACLE RegExp batch: {error}"));
    assert!(
        output.status.success(),
        "QJS_ORACLE RegExp batch failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout =
        String::from_utf8(output.stdout).expect("QJS_ORACLE RegExp batch emitted non-UTF-8 output");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        cases.len(),
        "QJS_ORACLE RegExp batch emitted the wrong number of observations"
    );
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            line.strip_prefix(&format!("{index}|"))
                .unwrap_or_else(|| panic!("malformed QJS_ORACLE observation: {line:?}"))
                .to_owned()
        })
        .collect()
}

fn js_utf16(units: &[u16]) -> String {
    let values = units
        .iter()
        .map(|unit| format!("0x{unit:04x}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("String.fromCharCode({values})")
}
