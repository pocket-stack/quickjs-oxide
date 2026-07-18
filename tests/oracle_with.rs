use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{JsString, Runtime, Value};

struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// These probes intentionally stay within the language surface that predates
// `with`. That keeps failures attributable to environment resolution instead
// of unrelated parser or intrinsic work.
const CASES: &[Case] = &[
    Case {
        description: "strict code rejects with during direct-eval parsing",
        source: r#"
            (function () {
                try {
                    eval("\"use strict\"; with ({}) {}");
                    return "none";
                } catch (error) {
                    return error.name;
                }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        description: "single-statement contextual let honors ASI after with",
        source: r#"
            (function () {
                if (false) { with ({}) let
                    value = 1; }
                if (false) { with ({}) let
                    {} }
                return "parsed";
            })()
        "#,
        expected: "return|string|parsed",
    },
    Case {
        description: "with applies ToObject and rejects nullish values",
        source: r#"
            (function () {
                var observations = [];
                with ("abc") { observations.push(length); }
                try { with (null) {} }
                catch (error) { observations.push(error.name); }
                try { with (undefined) {} }
                catch (error) { observations.push(error.name); }
                return observations.join("|");
            })()
        "#,
        expected: "return|string|3|TypeError|TypeError",
    },
    Case {
        description: "unscopables skips the inner binding and preserves nested fallback order",
        source: r#"
            (function () {
                var outer = { x: 10, y: 11 };
                var inner = { x: 20, y: 21 };
                inner[Symbol.unscopables] = { x: true };
                with (outer) {
                    with (inner) { return x + "|" + y; }
                }
            })()
        "#,
        expected: "return|string|10|21",
    },
    Case {
        description: "Get rechecks HasProperty after an unscopables getter deletes the selection",
        source: r#"
            (function () {
                var outer = { x: 10 };
                var inner = { x: 20 };
                Object.defineProperty(inner, Symbol.unscopables, {
                    get: function () { delete inner.x; return {}; },
                    configurable: true
                });
                with (outer) {
                    with (inner) { return x; }
                }
            })()
        "#,
        expected: "return|undefined|undefined",
    },
    Case {
        description: "simple compound logical prefix and postfix writes target the with object",
        source: r#"
            (function () {
                var object = { x: 1, a: 2, b: 0, c: 4, d: 5 };
                var old;
                with (object) {
                    x = 3;
                    a += 4;
                    b ||= 7;
                    c &&= 8;
                    old = d++;
                    ++x;
                }
                return [object.x, object.a, object.b, object.c, object.d, old].join("|");
            })()
        "#,
        expected: "return|string|4|6|7|8|6|5",
    },
    Case {
        description: "simple assignment keeps its selected base when the RHS deletes the property",
        source: r#"
            (function () {
                var object = { x: 1 };
                with (object) { x = (delete object.x, 9); }
                return object.x + "|" + ("x" in object);
            })()
        "#,
        expected: "return|string|9|true",
    },
    Case {
        description: "compound assignment keeps its selected base when the RHS deletes the property",
        source: r#"
            (function () {
                var object = { x: 1 };
                with (object) { x += (delete object.x, 4); }
                return object.x + "|" + ("x" in object);
            })()
        "#,
        expected: "return|string|5|true",
    },
    Case {
        description: "logical assignment keeps its selected base when the RHS deletes the property",
        source: r#"
            (function () {
                var object = { x: 1 };
                with (object) { x &&= (delete object.x, 7); }
                return object.x + "|" + ("x" in object);
            })()
        "#,
        expected: "return|string|7|true",
    },
    Case {
        description: "postfix update rechecks HasProperty after coercion deletes the property",
        source: r#"
            (function () {
                var object = {};
                var value = {
                    valueOf: function () { delete object.x; return 4; }
                };
                object.x = value;
                var old;
                with (object) { old = x++; }
                return old + "|" + object.x + "|" + ("x" in object);
            })()
        "#,
        expected: "return|string|4|5|true",
    },
    Case {
        description: "a strict child reports a missing selected property after its RHS deletes it",
        source: r#"
            (function () {
                var object = { x: 1 };
                var child;
                with (object) {
                    child = function () {
                        "use strict";
                        x = (delete object.x, 3);
                    };
                }
                var errorName = "none";
                try { child(); }
                catch (error) { errorName = error.name; }
                return errorName + "|" + ("x" in object) + "|" + typeof x;
            })()
        "#,
        expected: "return|string|ReferenceError|false|undefined",
    },
    Case {
        description: "failed selected writes are silent in sloppy code and throw in strict code",
        source: r#"
            (function () {
                var object = {};
                Object.defineProperty(object, "x", {
                    value: 1,
                    writable: false,
                    configurable: true
                });
                var sloppyResult, sloppyError = "none";
                try { with (object) { sloppyResult = x = 2; } }
                catch (error) { sloppyError = error.name; }
                var strictChild;
                with (object) {
                    strictChild = function () { "use strict"; return x = 3; };
                }
                var strictError = "none";
                try { strictChild(); }
                catch (error) { strictError = error.name; }
                return sloppyError + "|" + sloppyResult + "|" + object.x +
                    "|" + strictError;
            })()
        "#,
        expected: "return|string|none|2|1|TypeError",
    },
    Case {
        description: "with methods receive the object while lexical fallback receives undefined",
        source: r#"
            (function () {
                var object = {
                    tag: "with",
                    method: function () { "use strict"; return this.tag; }
                };
                function fallback() { "use strict"; return this === undefined; }
                var selected, missed;
                with (object) {
                    selected = method();
                    missed = fallback();
                }
                return selected + "|" + missed;
            })()
        "#,
        expected: "return|string|with|true",
    },
    Case {
        description: "an eval var object selected under syntactic with becomes the call receiver",
        source: r#"
            (function () {
                eval(
                    "var marker = 17; " +
                    "var readMarker = function () { " +
                    "  \"use strict\"; return this.marker; " +
                    "}"
                );
                with ({}) { return readMarker(); }
            })()
        "#,
        expected: "return|number|17",
    },
    Case {
        description: "direct eval imports with lookup without importing reference-call receiver rules",
        source: r#"
            (function () {
                var object = {
                    method: function () { "use strict"; return this === undefined; }
                };
                with (object) { return eval("method()"); }
            })()
        "#,
        expected: "return|boolean|true",
    },
    Case {
        description: "direct eval assignment resolves its imported with environment after the RHS",
        source: r#"
            (function () {
                var object = { x: 1 };
                var x = 7;
                with (object) { eval("x = (delete object.x, 2)"); }
                return object.x + "|" + ("x" in object) + "|" + x;
            })()
        "#,
        expected: "return|string|undefined|false|2",
    },
    Case {
        description: "an eval-created closure imports with lookup but still calls with undefined receiver",
        source: r#"
            (function () {
                var object = {
                    method: function () { "use strict"; return this === undefined; }
                };
                with (object) {
                    var closure = eval("(function () { return method(); })");
                    return closure();
                }
            })()
        "#,
        expected: "return|boolean|true",
    },
    Case {
        description: "delete distinguishes selected fallback and missing bindings",
        source: r#"
            (function () {
                var local = 3;
                var object = { x: 1 };
                var selected, fallback, missing;
                with (object) {
                    selected = delete x;
                    fallback = delete local;
                    missing = delete absent;
                }
                return selected + "|" + fallback + "|" + missing + "|" +
                    ("x" in object) + "|" + local;
            })()
        "#,
        expected: "return|string|true|false|true|false|3",
    },
    Case {
        description: "for-in and for-of assignment targets resolve through with on every iteration",
        source: r#"
            (function () {
                var object = { x: "" };
                var output = "";
                with (object) {
                    for (x in { a: 1, b: 2 }) output += x;
                    for (x of "cd") output += x;
                }
                return output + "|" + object.x;
            })()
        "#,
        expected: "return|string|abcd|d",
    },
    Case {
        description: "readonly fallback throws before evaluating the assignment RHS",
        source: r#"
            (function () {
                const locked = 1;
                var sideEffect = 0;
                var errorName = "none";
                try { with ({}) { locked = (sideEffect = 1, 2); } }
                catch (error) { errorName = error.name; }
                return errorName + "|" + sideEffect + "|" + locked;
            })()
        "#,
        expected: "return|string|TypeError|0|1",
    },
    Case {
        description: "readonly fallback rejects compound logical and update references eagerly",
        source: r#"
            (function () {
                const locked = 1;
                var sideEffect = 0;
                var compound = "none", logical = "none", update = "none";
                try { with ({}) { locked += (sideEffect = 1); } }
                catch (error) { compound = error.name; }
                var afterCompound = sideEffect;
                sideEffect = 0;
                try { with ({}) { locked &&= (sideEffect = 2); } }
                catch (error) { logical = error.name; }
                var afterLogical = sideEffect;
                try { with ({}) { locked++; } }
                catch (error) { update = error.name; }
                return compound + "|" + afterCompound + "|" +
                    logical + "|" + afterLogical + "|" + update;
            })()
        "#,
        expected: "return|string|TypeError|0|TypeError|0|TypeError",
    },
    Case {
        description: "a missing strict global reference stays missing across RHS creation",
        source: r#"
            (function () {
                delete globalThis.__qjo_with_missing_strict;
                with ({}) {
                    return (function () {
                        "use strict";
                        try {
                            __qjo_with_missing_strict =
                                (globalThis.__qjo_with_missing_strict = 1, 2);
                            return "none";
                        } catch (error) {
                            return error.name + "|" +
                                globalThis.__qjo_with_missing_strict;
                        }
                    })();
                }
            })()
        "#,
        expected: "return|string|ReferenceError|1",
    },
    Case {
        description: "loop closures retain the with object from each closed scope",
        source: r#"
            (function () {
                var closures = [];
                var objects = [{ x: 1 }, { x: 2 }];
                for (var index = 0; index < 2; index++) {
                    with (objects[index]) {
                        closures.push(function () { return x; });
                    }
                }
                return closures[0]() + "|" + closures[1]();
            })()
        "#,
        expected: "return|string|1|2",
    },
    Case {
        description: "a var initializer inside with assigns the object and not the function var",
        source: r#"
            (function () {
                var object = { x: 1 };
                var x;
                with (object) { var x = 2; }
                return object.x + "|" + x;
            })()
        "#,
        expected: "return|string|2|undefined",
    },
    Case {
        description: "a var initializer selects its with reference before evaluating the RHS",
        source: r#"
            (function () {
                var object = { x: 1 };
                var x;
                with (object) { var x = (delete object.x, 2); }
                return object.x + "|" + ("x" in object) + "|" + x;
            })()
        "#,
        expected: "return|string|2|true|undefined",
    },
    Case {
        description: "eval variable objects use prototype-aware HasProperty under with",
        source: r#"
            (function () {
                eval("var leak = function () { \"use strict\"; return this; }");
                with ({}) {
                    Object.setPrototypeOf(leak(), { inherited: 42 });
                    return inherited;
                }
            })()
        "#,
        expected: "return|number|42",
    },
];

#[test]
fn with_semantics_match_pinned_expectations() {
    for case in CASES {
        assert_eq!(
            rust_observation(case),
            case.expected,
            "Rust with behavior drifted for {}: {:?}",
            case.description,
            case.source,
        );
    }
}

#[test]
fn with_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP with differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        let quickjs = oracle_observation(&oracle, case);
        assert_eq!(
            quickjs, case.expected,
            "pinned with vector drifted for {}: {:?}",
            case.description, case.source,
        );
        assert_eq!(
            rust_observation(case),
            quickjs,
            "with behavior differed from pinned QuickJS for {}: {:?}",
            case.description,
            case.source,
        );
    }
}

#[test]
fn with_global_reference_sees_a_const_from_an_earlier_script() {
    let declaration = "const __qjo_with_prior_const = 1; var __qjo_with_prior_side = 0";
    let assignment = r#"
        var __qjo_with_prior_error = "none";
        try {
            with ({}) {
                __qjo_with_prior_const = (__qjo_with_prior_side = 1, 2);
            }
        } catch (error) {
            __qjo_with_prior_error = error.name;
        }
        __qjo_with_prior_error + "|" + __qjo_with_prior_side
    "#;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval(declaration).unwrap();
    assert_eq!(
        context.eval(assignment).unwrap(),
        Value::String(JsString::try_from_utf8("TypeError|0").unwrap())
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        return;
    };
    assert_eq!(
        oracle_sequence_observation(&oracle, &[declaration, assignment]),
        "return|string|TypeError|0"
    );
}

#[test]
fn with_global_reference_observes_a_lexical_declared_after_function_publication() {
    let function = r#"
        var __qjo_with_late_const_side = 0;
        var __qjo_with_late_const_function;
        with ({}) {
            __qjo_with_late_const_function = function () {
                __qjo_with_late_const = (__qjo_with_late_const_side = 1, 2);
            };
        }
    "#;
    let declaration = "const __qjo_with_late_const = 1";
    let observation = r#"
        var __qjo_with_late_const_error = "none";
        try { __qjo_with_late_const_function(); }
        catch (error) { __qjo_with_late_const_error = error.name; }
        __qjo_with_late_const_error + "|" + __qjo_with_late_const_side
    "#;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval(function).unwrap();
    context.eval(declaration).unwrap();
    assert_eq!(
        context.eval(observation).unwrap(),
        Value::String(JsString::try_from_utf8("TypeError|0").unwrap())
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        return;
    };
    assert_eq!(
        oracle_sequence_observation(&oracle, &[function, declaration, observation]),
        "return|string|TypeError|0"
    );
}

fn rust_observation(case: &Case) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context.eval(case.source).unwrap_or_else(|error| {
        panic!(
            "Rust rejected with probe {} ({:?}): {error}",
            case.description, case.source,
        )
    });
    format!(
        "return|{}|{}",
        value_type(&runtime, &value),
        primitive_value_text(value),
    )
}

fn oracle_observation(oracle: &OsStr, case: &Case) -> String {
    let wrapper = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print('return|' + typeof value + '|' + String(value));
} catch (error) {
  if (error !== null && typeof error === 'object')
    print('throw|object|' + error.name + '|' + error.message);
  else
    print('throw|' + typeof error + '|' + String(error));
}
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, case.source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {}: {error}", case.description,));
    assert!(
        output.status.success(),
        "QuickJS observer failed for {}: {}",
        case.description,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| {
            panic!(
                "QuickJS output was not UTF-8 for {}: {error}",
                case.description,
            )
        })
        .trim_end()
        .to_owned()
}

fn oracle_sequence_observation(oracle: &OsStr, scripts: &[&str]) -> String {
    assert!(
        !scripts.is_empty(),
        "QuickJS sequence needs an observation script"
    );
    let wrapper = r#"
for (var index = 0; index + 1 < scriptArgs.length; index++)
  std.evalScript(scriptArgs[index]);
try {
  var value = std.evalScript(scriptArgs[scriptArgs.length - 1]);
  print('return|' + typeof value + '|' + String(value));
} catch (error) {
  if (error !== null && typeof error === 'object')
    print('throw|object|' + error.name + '|' + error.message);
  else
    print('throw|' + typeof error + '|' + String(error));
}
"#;
    let mut command = Command::new(oracle);
    command.args(["--std", "-e", wrapper]).args(scripts);
    let output = command
        .output()
        .expect("could not run QuickJS global-reference sequence");
    assert!(
        output.status.success(),
        "QuickJS global-reference observer failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS global-reference output was not UTF-8")
        .trim_end()
        .to_owned()
}

fn value_type(runtime: &Runtime, value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Int(_) | Value::Float(_) => "number",
        Value::BigInt(_) => "bigint",
        Value::String(_) => "string",
        Value::Object(object) => {
            if runtime.as_callable(object).unwrap().is_some() {
                "function"
            } else {
                "object"
            }
        }
        Value::Symbol(_) => "symbol",
    }
}

fn primitive_value_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => quickjs_oxide::value::number_to_string(value),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}
