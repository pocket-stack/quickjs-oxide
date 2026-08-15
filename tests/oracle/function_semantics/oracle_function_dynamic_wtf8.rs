//! UTF-16 source-boundary differential for the four Dynamic Function kinds.

use crate::quickjs_argv_completion_oracle::observe_completion_argv_trim_end as observe_oracle;
use crate::runtime_completion_oracle::observe_eval_completion as observe_oxide;
use quickjs_oxide::Runtime;

struct DynamicKind {
    label: &'static str,
    constructor: &'static str,
    parameter_column: u32,
}

struct Case {
    description: String,
    source: String,
    expected: String,
}

const KINDS: &[DynamicKind] = &[
    DynamicKind {
        label: "Function",
        constructor: "Function",
        parameter_column: 21,
    },
    DynamicKind {
        label: "GeneratorFunction",
        constructor: "(function*(){}).constructor",
        parameter_column: 22,
    },
    DynamicKind {
        label: "AsyncFunction",
        constructor: "(async function(){}).constructor",
        parameter_column: 27,
    },
    DynamicKind {
        label: "AsyncGeneratorFunction",
        constructor: "(async function*(){}).constructor",
        parameter_column: 28,
    },
];

// Keep every transport source ASCII-only. The JavaScript program creates the
// UTF-16 code units after parsing, then crosses the same StringBuffer ->
// indirect-eval boundary as QuickJS's js_function_constructor.
const PRELUDE: &str = r#"
(function () {
    function unitHex(value) {
        value = String(value);
        var output = "";
        for (var index = 0; index < value.length; index++)
            output += ("0000" + value.charCodeAt(index).toString(16)).slice(-4);
        return output;
    }
    function diagnostic(error) {
        var frames = String(error.stack).split("\n").slice(0, 2).join("/");
        return [error.name, error.message, error.fileName,
                error.lineNumber, error.columnNumber, frames].join("|");
    }
    var high = String.fromCharCode(0xd801);
    var low = String.fromCharCode(0xdc00);
    var pair = high + low;
    var privateUse = String.fromCharCode(0xe001);
    var Ctor =
"#;

const BODY_PREFIX: &str = r#";
    try {
        return (function () {
"#;

const POSTLUDE: &str = r#"
        })();
    } catch (error) {
        if (error !== null && typeof error === "object")
            return "unexpected|" + diagnostic(error);
        return "unexpected|" + typeof error + "|" + String(error);
    }
})()
"#;

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for kind in KINDS {
        cases.push(case(
            kind,
            "lone high surrogate parameter diagnostic",
            r#"
                try {
                    Ctor(high, "");
                    return "no-throw";
                } catch (error) {
                    return diagnostic(error);
                }
            "#,
            &format!(
                "SyntaxError|unexpected character|<input>|1|{}|    at <input>:1:{}/    at {} (native)",
                kind.parameter_column, kind.parameter_column, kind.label,
            ),
        ));
        cases.push(case(
            kind,
            "lone low surrogate body diagnostic",
            r#"
                try {
                    Ctor("var " + low + "=1");
                    return "no-throw";
                } catch (error) {
                    return diagnostic(error);
                }
            "#,
            &format!(
                "SyntaxError|unexpected character|<input>|3|5|    at <input>:3:5/    at {} (native)",
                kind.label,
            ),
        ));
        cases.push(case(
            kind,
            "parameter and body comments preserve lone surrogates",
            r#"
                var functionValue = Ctor(
                    "value/*" + high + "*/",
                    "/*" + low + "*/ return value"
                );
                var rendered = Function.prototype.toString.call(functionValue);
                var highIndex = rendered.indexOf(high);
                var lowIndex = rendered.indexOf(low);
                return functionValue.name + "|" +
                    unitHex(rendered.slice(highIndex, highIndex + 1)) + "|" +
                    unitHex(rendered.slice(lowIndex, lowIndex + 1));
            "#,
            "anonymous|d801|dc00",
        ));
        cases.push(case(
            kind,
            "valid surrogate pair is an identifier in parameters and body",
            r#"
                var functionValue = Ctor(pair, "return " + pair);
                var rendered = Function.prototype.toString.call(functionValue);
                return functionValue.name + "|" + functionValue.length + "|" +
                    (rendered.indexOf(pair) >= 0);
            "#,
            "anonymous|1|true",
        ));
        cases.push(case(
            kind,
            "body ToString runs before a lone-surrogate parameter is parsed",
            r#"
                var log = "";
                var parameter = {
                    toString: function () { log += "p"; return high; }
                };
                var body = {
                    toString: function () { log += "b"; return ""; }
                };
                try {
                    Ctor(parameter, body);
                    return "no-throw";
                } catch (error) {
                    return log + "|" + diagnostic(error);
                }
            "#,
            &format!(
                "pb|SyntaxError|unexpected character|<input>|1|{}|    at <input>:1:{}/    at {} (native)",
                kind.parameter_column, kind.parameter_column, kind.label,
            ),
        ));
    }

    let normal = &KINDS[0];
    cases.push(case(
        normal,
        "body string literal returns a lone high surrogate",
        r#"
            var functionValue = Ctor("return '" + high + "'");
            return unitHex(functionValue());
        "#,
        "d801",
    ));
    cases.push(case(
        normal,
        "parameter default returns a lone low surrogate",
        r#"
            var functionValue = Ctor("value='" + low + "'", "return value");
            return unitHex(functionValue());
        "#,
        "dc00",
    ));
    cases.push(case(
        normal,
        "private-use text stays distinct from a lone-surrogate carrier",
        r#"
            var functionValue = Ctor("return '" + privateUse + high + "'");
            var rendered = Function.prototype.toString.call(functionValue);
            return unitHex(functionValue()) + "|" +
                (rendered.indexOf(privateUse + high) >= 0);
        "#,
        "e001d801|true",
    ));
    cases
}

fn case(kind: &DynamicKind, detail: &str, body: &str, expected: &str) -> Case {
    let mut source = String::with_capacity(
        PRELUDE.len() + kind.constructor.len() + BODY_PREFIX.len() + body.len() + POSTLUDE.len(),
    );
    source.push_str(PRELUDE);
    source.push_str(kind.constructor);
    source.push_str(BODY_PREFIX);
    source.push_str(body);
    source.push_str(POSTLUDE);
    assert!(source.is_ascii());
    Case {
        description: format!("{} / {detail}", kind.label),
        source,
        expected: format!("return|string|{expected}"),
    }
}

#[test]
fn dynamic_function_wtf8_source_matches_expected_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut failures = Vec::new();
    for case in cases() {
        let actual = observe_oxide(&runtime, &mut context, &case.source, &case.description);
        if actual != case.expected {
            failures.push(format!(
                "{}\nactual: {:?}\nexpected: {:?}",
                case.description, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "dynamic Function WTF-8 expectations failed in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn dynamic_function_wtf8_source_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP dynamic Function WTF-8 differential: set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut failures = Vec::new();
    for case in cases() {
        let oxide = observe_oxide(&runtime, &mut context, &case.source, &case.description);
        let quickjs = observe_oracle(&oracle, &case.source, &case.description);
        if quickjs != case.expected || oxide != quickjs {
            failures.push(format!(
                "{}\noxide: {:?}\nquickjs: {:?}\nexpected: {:?}",
                case.description, oxide, quickjs, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "dynamic Function WTF-8 behavior differed from pinned QuickJS in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn dynamic_function_wtf8_transport_sources_remain_ascii() {
    assert!(PRELUDE.is_ascii());
    assert!(BODY_PREFIX.is_ascii());
    assert!(POSTLUDE.is_ascii());
    for case in cases() {
        assert!(case.source.is_ascii(), "{}", case.description);
    }
}
