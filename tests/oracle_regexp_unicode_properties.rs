use std::ffi::OsStr;
use std::fmt::Write as _;
use std::process::Command;

use quickjs_oxide::JsString;
use quickjs_oxide::regexp::{CompileErrorKind, compile, execute};

// Differential lock for pinned QuickJS 2026-06-04 Unicode property escapes.
//
// QuickJS parses \p/\P in `libregexp.c` `parse_unicode_property`
// (869-986), dispatches them only for Unicode patterns (1122-1154), and
// resolves General_Category, Script/Script_Extensions, and binary properties
// through `libunicode.c` (1274-1394, 1595-1621, 1625 onward). These vectors
// pin representative canonical and short aliases, exact parser diagnostics,
// class placement, Annex B non-Unicode identity behavior, the Unicode
// ignoreCase ordering for \P, scoped modifiers, and UTF-16/code-point edges.

#[derive(Debug)]
struct MatchCase {
    label: &'static str,
    pattern: Vec<u16>,
    flags: &'static str,
    input: Vec<u16>,
    expected: &'static str,
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
        label: "Unicode property escape requires an opening brace",
        pattern: r"\p",
        flags: "u",
        expected: r"SyntaxError|expecting '{' after \p",
    },
    CompileCase {
        label: "empty lone property name is unknown",
        pattern: r"\p{}",
        flags: "u",
        expected: "SyntaxError|unknown unicode property name",
    },
    CompileCase {
        label: "property aliases are case sensitive",
        pattern: r"\p{letter}",
        flags: "u",
        expected: "SyntaxError|unknown unicode property name",
    },
    CompileCase {
        label: "loose hyphen spelling loses to closing-brace validation",
        pattern: r"\p{General-Category=Letter}",
        flags: "u",
        expected: "SyntaxError|expecting '}'",
    },
    CompileCase {
        label: "empty General_Category value has its family-specific error",
        pattern: r"\p{General_Category=}",
        flags: "u",
        expected: "SyntaxError|unknown unicode general category",
    },
    CompileCase {
        label: "unknown General_Category value has its family-specific error",
        pattern: r"\p{General_Category=Nope}",
        flags: "u",
        expected: "SyntaxError|unknown unicode general category",
    },
    CompileCase {
        label: "empty Script value has its family-specific error",
        pattern: r"\p{Script=}",
        flags: "u",
        expected: "SyntaxError|unknown unicode script",
    },
    CompileCase {
        label: "unknown Script value has its family-specific error",
        pattern: r"\p{Script=Nope}",
        flags: "u",
        expected: "SyntaxError|unknown unicode script",
    },
    CompileCase {
        label: "binary property cannot use an equals value",
        pattern: r"\p{ASCII=Yes}",
        flags: "u",
        expected: "SyntaxError|unknown unicode property name",
    },
    CompileCase {
        label: "empty equals value falls back to lone binary property lookup",
        pattern: r"\p{ASCII=}",
        flags: "u",
        expected: "ok",
    },
    CompileCase {
        label: "empty equals value falls back to lone General_Category lookup",
        pattern: r"\p{Letter=}",
        flags: "u",
        expected: "ok",
    },
    CompileCase {
        label: "unknown property family is a property-name error",
        pattern: r"\p{Unknown=Latin}",
        flags: "u",
        expected: "SyntaxError|unknown unicode property name",
    },
    CompileCase {
        label: "empty equals value does not rescue an unknown lone property",
        pattern: r"\p{Unknown=}",
        flags: "u",
        expected: "SyntaxError|unknown unicode property name",
    },
    CompileCase {
        label: "Unicode mode does not admit a property of strings",
        pattern: r"\p{RGI_Emoji}",
        flags: "u",
        expected: "SyntaxError|unknown unicode property name",
    },
    CompileCase {
        label: "Unicode mode does not admit inverted properties of strings",
        pattern: r"\P{RGI_Emoji}",
        flags: "u",
        expected: "SyntaxError|unknown unicode property name",
    },
    CompileCase {
        label: "unterminated property escape reports the missing brace",
        pattern: r"\p{ASCII",
        flags: "u",
        expected: "SyntaxError|expecting '}'",
    },
    CompileCase {
        label: "invalid property-name character wins before alias lookup",
        pattern: r"\p{Script=Nope!}",
        flags: "u",
        expected: "SyntaxError|expecting '}'",
    },
    CompileCase {
        label: "overlong property value has its bounded-buffer diagnostic",
        pattern: r"\p{Script=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA}",
        flags: "u",
        expected: "SyntaxError|unknown unicode property value",
    },
    CompileCase {
        label: "property set cannot be the left endpoint of a class range",
        pattern: r"[\p{ASCII}-z]",
        flags: "u",
        expected: "SyntaxError|invalid class range",
    },
    CompileCase {
        label: "property set cannot be the right endpoint of a class range",
        pattern: r"[a-\p{ASCII}]",
        flags: "u",
        expected: "SyntaxError|invalid class range",
    },
    CompileCase {
        label: "two property sets cannot form a class range",
        pattern: r"[\p{ASCII}-\P{ASCII}]",
        flags: "u",
        expected: "SyntaxError|invalid class range",
    },
    CompileCase {
        label: "set left endpoint wins before an unknown right property is resolved",
        pattern: r"[\p{ASCII}-\p{Unknown}]",
        flags: "u",
        expected: "SyntaxError|invalid class range",
    },
    CompileCase {
        label: "set left endpoint wins before a malformed hexadecimal right endpoint",
        pattern: r"[\p{ASCII}-\xZZ]",
        flags: "u",
        expected: "SyntaxError|invalid class range",
    },
    CompileCase {
        label: "set left endpoint wins even when the pattern ends after the hyphen",
        pattern: r"[\p{ASCII}-",
        flags: "u",
        expected: "SyntaxError|invalid class range",
    },
    CompileCase {
        label: "character left endpoint resolves an unknown right property first",
        pattern: r"[a-\p{Unknown}]",
        flags: "u",
        expected: "SyntaxError|unknown unicode property name",
    },
    CompileCase {
        label: "non-Unicode lowercase property spelling is an identity escape",
        pattern: r"\p{ASCII}",
        flags: "",
        expected: "ok",
    },
    CompileCase {
        label: "non-Unicode uppercase property spelling is an identity escape",
        pattern: r"\P{ASCII}",
        flags: "",
        expected: "ok",
    },
    CompileCase {
        label: "non-Unicode property letter is an identity escape in a class",
        pattern: r"[\p]",
        flags: "",
        expected: "ok",
    },
];

#[test]
fn regexp_unicode_property_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp Unicode property oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = match_cases();
    let observations = run_match_oracle(&oracle, &cases);
    assert_eq!(observations.len(), cases.len());
    for (case, observation) in cases.iter().zip(&observations) {
        assert_eq!(
            observation, case.expected,
            "pinned QuickJS Unicode property matching changed for {}",
            case.label,
        );
    }

    let observations = run_compile_oracle(&oracle, COMPILE_CASES);
    assert_eq!(observations.len(), COMPILE_CASES.len());
    for (case, observation) in COMPILE_CASES.iter().zip(&observations) {
        assert_eq!(
            observation, case.expected,
            "pinned QuickJS Unicode property grammar changed for {}",
            case.label,
        );
    }
}

#[test]
fn regexp_unicode_properties_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp Unicode property differential: set QJS_ORACLE to upstream qjs");
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
                "match case {index}: {}\npattern: {:?}\nflags: {:?}\ninput: {:?}\noxide: {actual:?}\noracle: {expected:?}",
                case.label, case.pattern, case.flags, case.input,
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
        "RegExp Unicode properties drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn match_cases() -> Vec<MatchCase> {
    vec![
        ascii(
            "General_Category canonical name and value",
            r"^\p{General_Category=Uppercase_Letter}$",
            "u",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "General_Category short property and value aliases",
            r"^\p{gc=Lu}$",
            "u",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "lone General_Category canonical value",
            r"^\p{Letter}$",
            "u",
            "Ω",
            "M|0|1|0,1:03a9",
        ),
        ascii(
            "lone General_Category aggregate alias",
            r"^\p{L}$",
            "u",
            "Ω",
            "M|0|1|0,1:03a9",
        ),
        ascii(
            "General_Category complement",
            r"^\P{Number}$",
            "u",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "Script canonical property and value",
            r"^\p{Script=Latin}$",
            "u",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "Script short property and value aliases",
            r"^\p{sc=Latn}$",
            "u",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "Script canonical Greek value",
            r"^\p{Script=Greek}$",
            "u",
            "Ω",
            "M|0|1|0,1:03a9",
        ),
        ascii(
            "Script excludes a Common code point with Hiragana extensions",
            r"^\p{Script=Hiragana}$",
            "u",
            "ー",
            "N",
        ),
        ascii(
            "Script_Extensions includes a Common code point",
            r"^\p{Script_Extensions=Hiragana}$",
            "u",
            "ー",
            "M|0|1|0,1:30fc",
        ),
        ascii(
            "Script_Extensions short aliases",
            r"^\p{scx=Hira}$",
            "u",
            "ー",
            "M|0|1|0,1:30fc",
        ),
        ascii(
            "special binary ASCII property",
            r"^\p{ASCII}$",
            "u",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "empty equals value resolves the lone binary ASCII property",
            r"^\p{ASCII=}$",
            "u",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "empty equals value resolves the lone Letter category",
            r"^\p{Letter=}$",
            "u",
            "Ω",
            "M|0|1|0,1:03a9",
        ),
        ascii(
            "binary ASCII_Hex_Digit short alias",
            r"^\p{AHex}$",
            "u",
            "F",
            "M|0|1|0,1:0046",
        ),
        ascii(
            "binary Emoji_Presentation canonical name",
            r"^\p{Emoji_Presentation}$",
            "u",
            "😀",
            "M|0|1|0,2:d83dde00",
        ),
        ascii(
            "binary Emoji_Presentation short alias",
            r"^\p{EPres}$",
            "u",
            "😀",
            "M|0|1|0,2:d83dde00",
        ),
        ascii(
            "binary ID_Start canonical name",
            r"^\p{ID_Start}$",
            "u",
            "Ω",
            "M|0|1|0,1:03a9",
        ),
        ascii(
            "binary ID_Start short alias",
            r"^\p{IDS}$",
            "u",
            "Ω",
            "M|0|1|0,1:03a9",
        ),
        ascii(
            "special binary Any consumes one astral code point",
            r"^\p{Any}$",
            "u",
            "😀",
            "M|0|1|0,2:d83dde00",
        ),
        ascii(
            "derived binary Assigned property",
            r"^\p{Assigned}$",
            "u",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "property sets union inside a character class",
            r"^[\p{ASCII}\p{Script=Greek}]+$",
            "u",
            "AΩ",
            "M|0|1|0,2:004103a9",
        ),
        ascii(
            "inverted property set inside a character class",
            r"^[\P{ASCII}]+$",
            "u",
            "Ω",
            "M|0|1|0,1:03a9",
        ),
        ascii(
            "non-Unicode lowercase property syntax remains literal text",
            r"^\p{ASCII}$",
            "",
            "p{ASCII}",
            "M|0|1|0,8:0070007b00410053004300490049007d",
        ),
        ascii(
            "non-Unicode uppercase property syntax remains literal text",
            r"^\P{ASCII}$",
            "",
            "P{ASCII}",
            "M|0|1|0,8:0050007b00410053004300490049007d",
        ),
        ascii(
            "non-Unicode property letters are identity escapes in a class",
            r"^[\p\P]+$",
            "",
            "pP",
            "M|0|1|0,2:00700050",
        ),
        ascii(
            "Unicode ignoreCase closes a positive Lowercase_Letter set",
            r"^\p{Lowercase_Letter}$",
            "iu",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "Unicode ignoreCase folds after Lowercase_Letter inversion for lowercase input",
            r"^\P{Lowercase_Letter}$",
            "iu",
            "a",
            "M|0|1|0,1:0061",
        ),
        ascii(
            "Unicode ignoreCase folds after Lowercase_Letter inversion for uppercase input",
            r"^\P{Lowercase_Letter}$",
            "iu",
            "A",
            "M|0|1|0,1:0041",
        ),
        ascii(
            "scoped ignoreCase closes a Unicode property set",
            r"^(?i:\p{Uppercase_Letter})$",
            "u",
            "a",
            "M|0|1|0,1:0061",
        ),
        ascii(
            "scoped removal of global ignoreCase restores the narrow property set",
            r"^(?-i:\p{Uppercase_Letter})$",
            "iu",
            "a",
            "N",
        ),
        ascii(
            "scoped ignoreCase preserves Unicode complement ordering",
            r"^(?i:\P{Lowercase_Letter})$",
            "u",
            "a",
            "M|0|1|0,1:0061",
        ),
        ascii(
            "astral Script value consumes one code point and two UTF-16 units",
            r"^\p{Script=Deseret}$",
            "u",
            "𐐀",
            "M|0|1|0,2:d801dc00",
        ),
        MatchCase {
            label: "Surrogate category matches a lone high surrogate",
            pattern: utf16(r"^\p{General_Category=Surrogate}$"),
            flags: "u",
            input: vec![0xd800],
            expected: "M|0|1|0,1:d800",
        },
        ascii(
            "Surrogate category does not split an astral pair",
            r"^\p{General_Category=Surrogate}$",
            "u",
            "😀",
            "N",
        ),
        MatchCase {
            label: "Any includes a lone surrogate code point",
            pattern: utf16(r"^\p{Any}$"),
            flags: "u",
            input: vec![0xd800],
            expected: "M|0|1|0,1:d800",
        },
        MatchCase {
            label: "ASCII excludes a lone surrogate code point",
            pattern: utf16(r"^\p{ASCII}$"),
            flags: "u",
            input: vec![0xd800],
            expected: "N",
        },
    ]
}

fn ascii(
    label: &'static str,
    pattern: &str,
    flags: &'static str,
    input: &str,
    expected: &'static str,
) -> MatchCase {
    MatchCase {
        label,
        pattern: utf16(pattern),
        flags,
        input: utf16(input),
        expected,
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
    match execute(&program, &case.input, 0) {
        Ok(result) => observe_rust_result(result.as_ref(), &case.input),
        Err(error) => format!("ExecError|{error}"),
    }
}

fn observe_rust_compile(case: &CompileCase) -> String {
    let pattern = JsString::try_from_utf8(case.pattern).unwrap();
    let flags = JsString::try_from_utf8(case.flags).unwrap();
    match compile(&pattern, &flags) {
        Ok(_) => "ok".to_owned(),
        Err(error) if matches!(error.kind(), CompileErrorKind::Syntax) => {
            format!("SyntaxError|{}", error.message())
        }
        Err(error) => format!("CompileError|{:?}|{}", error.kind(), error.message()),
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
function __qjo_observe_unicode_property(test) {
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
var __qjo_unicode_property_cases = [
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
for (var __qjo_i = 0; __qjo_i < __qjo_unicode_property_cases.length; __qjo_i++)
    print(__qjo_i + "|" +
          __qjo_observe_unicode_property(__qjo_unicode_property_cases[__qjo_i]));
"#,
    );
    run_indexed_oracle(
        oracle,
        &source,
        cases.len(),
        "RegExp Unicode property matching",
    )
}

fn run_compile_oracle(oracle: &OsStr, cases: &[CompileCase]) -> Vec<String> {
    let mut source = String::from("var __qjo_unicode_property_compile_cases = [\n");
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
for (var __qjo_i = 0; __qjo_i < __qjo_unicode_property_compile_cases.length; __qjo_i++) {
    var result = "ok";
    try {
        new RegExp(__qjo_unicode_property_compile_cases[__qjo_i].pattern,
                   __qjo_unicode_property_compile_cases[__qjo_i].flags);
    } catch (error) {
        result = error.name + "|" + error.message;
    }
    print(__qjo_i + "|" + result);
}
"#,
    );
    run_indexed_oracle(
        oracle,
        &source,
        cases.len(),
        "RegExp Unicode property grammar",
    )
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

    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QJS_ORACLE {label} emitted non-UTF-8 output: {error}"));
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
