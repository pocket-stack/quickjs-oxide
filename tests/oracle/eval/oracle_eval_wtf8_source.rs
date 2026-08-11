use crate::runtime_observation::plain_value_type as value_type;
use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    body: &'static str,
    expected: &'static str,
}

// The outer programs stay ASCII-only. Each probe creates lone UTF-16
// surrogates as JavaScript Strings and passes those Strings to direct or
// indirect eval, which is the JS_ToCStringLen2(..., cesu8 = false) WTF-8
// boundary in pinned QuickJS 2026-06-04: valid pairs use standard UTF-8 while
// lone surrogates retain their code units. Observations are encoded as UTF-16
// unit hex before they cross Rust's `str`, argv, or QuickJS stdout boundaries.
const CASES: &[Case] = &[
    Case {
        group: "string literal",
        description: "sloppy direct eval preserves a raw lone high surrogate",
        body: r#"return unitHex(eval(quote + high + quote));"#,
        expected: "d800",
    },
    Case {
        group: "string literal",
        description: "strict direct eval preserves a raw lone low surrogate",
        body: r#""use strict"; return unitHex(eval(quote + low + quote));"#,
        expected: "dfff",
    },
    Case {
        group: "string literal",
        description: "indirect eval preserves both kinds of lone surrogate independently",
        body: r#"
            var first = (0, eval)(quote + high + quote);
            var second = (0, eval)(quote + low + quote);
            return unitHex(first) + "|" + unitHex(second);
        "#,
        expected: "d800|dfff",
    },
    Case {
        group: "template literal",
        description: "direct eval preserves a lone high surrogate in template text",
        body: r#"return unitHex(eval("`A" + high + "B`"));"#,
        expected: "0041d8000042",
    },
    Case {
        group: "template literal",
        description: "indirect eval preserves a lone low surrogate in template text",
        body: r#"return unitHex((0, eval)("`C" + low + "D`"));"#,
        expected: "0043dfff0044",
    },
    Case {
        group: "carrier collision",
        description: "a real private-use code point stays distinct from the surrogate carrier",
        body: r#"return unitHex(eval(quote + privateUse + high + quote));"#,
        expected: "e000d800",
    },
    Case {
        group: "regexp literal",
        description: "direct eval preserves and matches a raw lone high surrogate atom",
        body: r#"
            var pattern = eval(slash + high + slash);
            return unitHex(pattern.source) + "|" + pattern.test(high) + "|" + pattern.test(privateUse);
        "#,
        expected: "d800|true|false",
    },
    Case {
        group: "regexp literal",
        description: "indirect eval preserves a raw lone low surrogate in a Unicode class",
        body: r#"
            var pattern = (0, eval)(slash + "[" + low + "]" + slash + "u");
            return unitHex(pattern.source) + "|" + pattern.test(low) + "|" + pattern.test(high);
        "#,
        expected: "005bdfff005d|true|false",
    },
    Case {
        group: "regexp literal",
        description: "a backslash followed by a raw lone high surrogate remains an identity escape",
        body: r#"
            var pattern = eval(slash + backslash + high + slash);
            return unitHex(pattern.source) + "|" + pattern.test(high);
        "#,
        expected: "005cd800|true",
    },
    Case {
        group: "comments",
        description: "a direct-eval line comment accepts a lone high surrogate",
        body: r#"return String(eval("var marker = 42; //" + high + "\nmarker"));"#,
        expected: "42",
    },
    Case {
        group: "comments",
        description: "an indirect-eval block comment accepts a lone low surrogate",
        body: r#"return String((0, eval)("var __qjoWtf8Marker = 42; /*" + low + "*/ __qjoWtf8Marker"));"#,
        expected: "42",
    },
    Case {
        group: "early error",
        description: "a raw lone high surrogate in an identifier reports its second-line location",
        body: r#"
            try {
                eval("0;\nvar a" + high + " = 1");
                return "no-throw";
            } catch (error) {
                return [error.name, error.fileName, error.lineNumber, error.columnNumber].join("|");
            }
        "#,
        expected: "SyntaxError|<input>|2|6",
    },
    Case {
        group: "early error",
        description: "a raw lone low surrogate in an indirect-eval identifier is a SyntaxError",
        body: r#"
            try {
                (0, eval)("var a" + low + " = 1");
                return "no-throw";
            } catch (error) {
                return [error.name, error.fileName, error.lineNumber, error.columnNumber].join("|");
            }
        "#,
        expected: "SyntaxError|<input>|1|6",
    },
    Case {
        group: "early error",
        description: "a lone surrogate cannot become part of a RegExp group name",
        body: r#"
            try {
                eval(slash + "(?<a" + high + ">.)" + slash + "u");
                return "no-throw";
            } catch (error) {
                return error.name;
            }
        "#,
        expected: "SyntaxError",
    },
    Case {
        group: "debug source",
        description: "Function toString reconstructs the original private-use and surrogate units",
        body: r#"
            var source = "(function preserved(){return " + quote + privateUse + high + quote + "})";
            var functionValue = eval(source);
            var rendered = Function.prototype.toString.call(functionValue);
            var firstQuote = rendered.indexOf(quote);
            var lastQuote = rendered.lastIndexOf(quote);
            return String(rendered === source.slice(1, -1)) + "|" +
                unitHex(rendered.slice(firstQuote + 1, lastQuote));
        "#,
        expected: "true|e000d800",
    },
];

const PRELUDE: &str = r#"
(function () {
    function unitHex(value) {
        value = String(value);
        var output = "";
        for (var index = 0; index < value.length; index++)
            output += ("0000" + value.charCodeAt(index).toString(16)).slice(-4);
        return output;
    }
    var high = String.fromCharCode(0xd800);
    var low = String.fromCharCode(0xdfff);
    var privateUse = String.fromCharCode(0xe000);
    var quote = String.fromCharCode(0x27);
    var slash = String.fromCharCode(0x2f);
    var backslash = String.fromCharCode(0x5c);
    try {
        return (function () {
"#;

const POSTLUDE: &str = r#"
        })();
    } catch (error) {
        if (error !== null && typeof error === "object")
            return "throw|object|" + unitHex(error.name) + "|" + unitHex(error.message);
        return "throw|" + typeof error + "|" + unitHex(error);
    }
})()
"#;

#[test]
fn eval_wtf8_source_matches_expected_semantics() {
    let mut failures = Vec::new();
    for case in CASES {
        let actual = oxide_observation(case);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nactual: {:?}\nexpected: {:?}",
                case.group, case.description, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "eval WTF-8 expectations failed in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn eval_wtf8_source_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP eval WTF-8 oracle self-check: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    let mut failures = Vec::new();
    for case in CASES {
        let actual = quickjs_observation(&oracle, case);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nactual: {:?}\nexpected: {:?}",
                case.group, case.description, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "pinned QuickJS eval WTF-8 vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn eval_wtf8_source_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP eval WTF-8 differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    let mut failures = Vec::new();
    for case in CASES {
        let oxide = oxide_observation(case);
        let quickjs = quickjs_observation(&oracle, case);
        if oxide != quickjs {
            failures.push(format!(
                "{} / {}\noxide: {:?}\nquickjs: {:?}",
                case.group, case.description, oxide, quickjs,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "eval WTF-8 behavior differed from pinned QuickJS in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn source_for(case: &Case) -> String {
    let mut source = String::with_capacity(PRELUDE.len() + case.body.len() + POSTLUDE.len());
    source.push_str(PRELUDE);
    source.push_str(case.body);
    source.push_str(POSTLUDE);
    assert!(
        source.is_ascii(),
        "oracle transport source was not ASCII for {} / {}",
        case.group,
        case.description,
    );
    source
}

fn oxide_observation(case: &Case) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = source_for(case);
    match context.eval(&source) {
        Ok(Value::String(value)) => value.to_utf8_lossy(),
        Ok(value) => format!("unexpected-return|{}", value_type(&value)),
        Err(RuntimeError::Engine(error)) => {
            format!("engine|{:?}|{}", error.kind(), error.message(),)
        }
        Err(RuntimeError::Exception) => "unexpected-runtime-exception".to_owned(),
        Err(error) => format!("runtime|{error}"),
    }
}

fn quickjs_observation(oracle: &OsStr, case: &Case) -> String {
    let source = source_for(case);
    let wrapper = r#"
try {
    var value = std.evalScript(scriptArgs[0]);
    print(String(value));
} catch (error) {
    if (error !== null && typeof error === "object")
        print("oracle-throw|" + error.name + "|" + error.message);
    else
        print("oracle-throw|" + typeof error + "|" + String(error));
}
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, &source])
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "could not run QuickJS for {} / {}: {error}",
                case.group, case.description,
            )
        });
    assert!(
        output.status.success(),
        "QuickJS observer failed for {} / {}: {}",
        case.group,
        case.description,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| {
            panic!(
                "QuickJS output was not UTF-8 for {} / {}: {error}",
                case.group, case.description,
            )
        })
        .trim_end()
        .to_owned()
}

#[test]
fn eval_wtf8_transport_sources_remain_ascii() {
    assert!(PRELUDE.is_ascii());
    assert!(POSTLUDE.is_ascii());
    for case in CASES {
        assert!(case.body.is_ascii());
        assert!(source_for(case).is_ascii());
    }
}
