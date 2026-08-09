use std::ffi::OsStr;
use std::fmt::Write as _;

use quickjs_oxide::JsString;
use quickjs_oxide::regexp::{CompileErrorKind, UnsupportedFeature, compile, execute};

// Differential lock for the deliberately narrow first `v`-flag slice.
//
// Pinned QuickJS 2026-06-04 initializes `unicode_sets` in `lre_compile`,
// recognizes d/D/s/S/w/W in `get_class_atom`, and builds their ranges through
// `cr_init_char_range`. Its separate `re_parse_nested_class` paths implement
// set operations, nested sets, strings, and properties of strings; those paths
// remain typed Unsupported in quickjs-oxide until their own parity milestones.

#[derive(Debug)]
struct MatchCase {
    label: &'static str,
    pattern: Vec<u16>,
    flags: &'static str,
    input: Vec<u16>,
    expected: &'static str,
}

#[test]
fn regexp_v_character_class_escape_slice_matches_expected_semantics() {
    for case in match_cases() {
        assert_eq!(
            observe_oxide(&case),
            case.expected,
            "unexpected v-slice result for {}",
            case.label,
        );
    }
}

#[test]
fn regexp_v_character_class_escape_slice_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp v CharacterClassEscape differential: set QJS_ORACLE");
        return;
    };

    let cases = match_cases();
    let oracle_observations = observe_quickjs(&oracle, &cases);
    for (case, oracle_observation) in cases.iter().zip(oracle_observations) {
        let oxide_observation = observe_oxide(case);
        assert_eq!(
            oracle_observation, case.expected,
            "pinned QuickJS vector changed for {}",
            case.label,
        );
        assert_eq!(
            oxide_observation, oracle_observation,
            "quickjs-oxide differs from pinned QuickJS for {}",
            case.label,
        );
    }
}

#[test]
fn regexp_v_non_slice_and_malformed_syntax_remain_distinct() {
    for pattern in [
        "",
        "^$",
        "a",
        ".",
        r"(\d)",
        r"\d|\w",
        r"\b",
        r"\p{ASCII}",
        r"[\p{ASCII}]",
        r"[\q{ab}]",
        r"[[\d]]",
        r"[\d&&\w]",
        r"[\d--\w]",
        r"[a]",
        r"[a-z]",
    ] {
        let error = compile_ascii(pattern, "v").unwrap_err();
        assert_eq!(
            error.kind(),
            &CompileErrorKind::Unsupported(UnsupportedFeature::UnicodeSetOperation),
            "{pattern}",
        );
    }

    for pattern in [
        "\\", r"[\d", "[\\", r"*\d", r"\d{2,1}", r"\d{1", r"\d{a}", r"\d{1,a}", r"\d++", r"[\d-]",
        r"[\d-\w]",
    ] {
        let error = compile_ascii(pattern, "v").unwrap_err();
        assert_eq!(error.kind(), &CompileErrorKind::Syntax, "{pattern}");
    }
}

fn match_cases() -> Vec<MatchCase> {
    vec![
        ascii("digit positive", r"^\d+$", "v", "0123456789", "M:0:10"),
        ascii("digit stays ASCII-only", r"^\d+$", "v", "\u{0661}", "N"),
        ascii("digit complement rejects digits", r"^\D+$", "v", "7", "N"),
        units(
            "digit complement consumes one astral code point",
            r"^\D$",
            "v",
            &[0xd83d, 0xde00],
            "M:0:2",
        ),
        ascii(
            "whitespace positive",
            r"^\s+$",
            "v",
            "\t\n\u{00a0}\u{2028}\u{feff}",
            "M:0:5",
        ),
        ascii("whitespace excludes NEL", r"^\s$", "v", "\u{0085}", "N"),
        ascii(
            "non-whitespace includes NEL",
            r"^\S$",
            "v",
            "\u{0085}",
            "M:0:1",
        ),
        ascii("non-whitespace rejects NBSP", r"^\S$", "v", "\u{00a0}", "N"),
        ascii("word positive", r"^\w+$", "v", "Az_09", "M:0:5"),
        ascii("word stays ASCII without i", r"^\w$", "v", "\u{212a}", "N"),
        ascii("word complement positive", r"^\W+$", "v", "-!", "M:0:2"),
        ascii("word complement rejects underscore", r"^\W$", "v", "_", "N"),
        ascii(
            "simple class unions admitted escapes",
            r"^[\d\s\w]+$",
            "v",
            "A9_ \t",
            "M:0:5",
        ),
        ascii(
            "simple class union rejects punctuation",
            r"^[\d\s\w]+$",
            "v",
            "!",
            "N",
        ),
        ascii(
            "outer inversion composes with D",
            r"^[^\D]+$",
            "v",
            "42",
            "M:0:2",
        ),
        ascii(
            "outer inversion rejects a D member",
            r"^[^\D]+$",
            "v",
            "x",
            "N",
        ),
        ascii(
            "legacy i does not Unicode-fold Kelvin",
            r"^\w$",
            "i",
            "\u{212a}",
            "N",
        ),
        ascii("u plus i folds Kelvin", r"^\w$", "iu", "\u{212a}", "M:0:1"),
        ascii("v plus i folds Kelvin", r"^\w$", "iv", "\u{212a}", "M:0:1"),
        ascii(
            "v plus i folds long s",
            r"^[\w]$",
            "iv",
            "\u{017f}",
            "M:0:1",
        ),
        ascii(
            "v plus i complements after folding",
            r"^[\W]$",
            "iv",
            "\u{212a}",
            "N",
        ),
        units(
            "legacy D sees an astral pair as two code units",
            r"^\D$",
            "",
            &[0xd83d, 0xde00],
            "N",
        ),
        units(
            "u D consumes one astral code point",
            r"^\D$",
            "u",
            &[0xd83d, 0xde00],
            "M:0:2",
        ),
        units(
            "v D preserves lone surrogates",
            r"^\D$",
            "v",
            &[0xdc00],
            "M:0:1",
        ),
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
        pattern: pattern.encode_utf16().collect(),
        flags,
        input: input.encode_utf16().collect(),
        expected,
    }
}

fn units(
    label: &'static str,
    pattern: &str,
    flags: &'static str,
    input: &[u16],
    expected: &'static str,
) -> MatchCase {
    MatchCase {
        label,
        pattern: pattern.encode_utf16().collect(),
        flags,
        input: input.to_vec(),
        expected,
    }
}

fn compile_ascii(
    pattern: &str,
    flags: &str,
) -> Result<quickjs_oxide::regexp::CompiledRegExp, quickjs_oxide::regexp::CompileError> {
    compile(
        &JsString::try_from_utf8(pattern).unwrap(),
        &JsString::try_from_utf8(flags).unwrap(),
    )
}

fn observe_oxide(case: &MatchCase) -> String {
    let pattern = JsString::try_from_utf16(case.pattern.iter().copied()).unwrap();
    let flags = JsString::try_from_utf8(case.flags).unwrap();
    let compiled = compile(&pattern, &flags)
        .unwrap_or_else(|error| panic!("compile failed for {}: {error}", case.label));
    match execute(&compiled, &case.input, 0)
        .unwrap_or_else(|error| panic!("execute failed for {}: {error}", case.label))
    {
        Some(found) => {
            let capture = found.capture(0).expect("complete match capture");
            format!("M:{}:{}", capture.start, capture.end - capture.start)
        }
        None => "N".to_owned(),
    }
}

fn observe_quickjs(oracle: &OsStr, cases: &[MatchCase]) -> Vec<String> {
    let mut source = String::new();
    for (index, case) in cases.iter().enumerate() {
        writeln!(
            source,
            "try{{var r=new RegExp({},{}),m=r.exec({});print('{}|'+(m===null?'N':'M:'+m.index+':'+m[0].length))}}catch(e){{print('{}|throw:'+e.name+':'+e.message)}}",
            js_utf16(&case.pattern),
            js_utf16(&case.flags.encode_utf16().collect::<Vec<_>>()),
            js_utf16(&case.input),
            index,
            index,
        )
        .unwrap();
    }

    super::quickjs_indexed_oracle::eval_indexed_plain_lines(
        oracle,
        &source,
        cases.len(),
        "RegExp v character class escapes",
    )
}

fn js_utf16(units: &[u16]) -> String {
    let values = units
        .iter()
        .map(|unit| format!("0x{unit:04x}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("String.fromCharCode({values})")
}
