use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the synchronous, simple-parameter ArrowFunction slice from QuickJS
// 2026-06-04. Default/rest/destructuring parameters and async arrows belong to
// later slices; the lexical environment cases here must already be correct
// before the simpler syntax is treated as an ordinary function shorthand.
const CASES: &[Case] = &[
    Case {
        group: "lookahead",
        description: "a single identifier parameter needs no parentheses",
        source: "(value => value + 1)(41)",
        expected: "return|number|42",
    },
    Case {
        group: "lookahead",
        description: "empty and multiple parenthesized parameters are recognized",
        source: r#"
            (function () {
                return (() => 42)() + "|" +
                    ((left, right) => left + right)(20, 22);
            })()
        "#,
        expected: "return|string|42|42",
    },
    Case {
        group: "lookahead",
        description: "a parenthesized expression is not stolen by arrow lookahead",
        source: "(1 + 2) * 14",
        expected: "return|number|42",
    },
    Case {
        group: "lookahead",
        description: "a line terminator after an identifier parameter rejects the arrow",
        source: r#"
            (function () {
                try { eval("value\n=> value"); return "accepted"; }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "lookahead",
        description: "a line terminator after a closing parameter paren rejects the arrow",
        source: r#"
            (function () {
                try { eval("(value)\n=> value"); return "accepted"; }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "lookahead",
        description: "a line terminator inside a block comment rejects the arrow",
        source: r#"
            (function () {
                try {
                    eval("value /* line\n break */ => value");
                    return "accepted";
                } catch (error) {
                    return error.name;
                }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "lookahead",
        description: "a same-line block comment before the arrow is permitted",
        source: "(value /* no LineTerminator */ => value)(42)",
        expected: "return|number|42",
    },
    Case {
        group: "lookahead",
        description: "an always-reserved word cannot be an arrow parameter",
        source: r#"
            (function () {
                try { eval("enum => 1"); return "accepted"; }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "lookahead",
        description: "a statement keyword cannot be an arrow parameter",
        source: r#"
            (function () {
                try { eval("switch => 1"); return "accepted"; }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "lookahead",
        description: "a strict reserved word cannot be an arrow parameter",
        source: r#"
            (function () {
                "use strict";
                try { eval("package => 1"); return "accepted"; }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "body",
        description: "an expression body consumes a conditional AssignmentExpression",
        source: "(value => value ? 0 : 42)(false)",
        expected: "return|number|42",
    },
    Case {
        group: "body",
        description: "a block body has its own statements and explicit return",
        source: r#"
            (() => {
                var answer = 40;
                answer += 2;
                return answer;
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "body",
        description: "a parenthesized object literal remains an expression body",
        source: "(() => ({ answer: 42 }))().answer",
        expected: "return|number|42",
    },
    Case {
        group: "body",
        description: "an empty block body returns undefined",
        source: "typeof (() => {})()",
        expected: "return|string|undefined",
    },
    Case {
        group: "strictness",
        description: "duplicate simple parameters are rejected in sloppy code",
        source: r#"
            (function () {
                try { eval("(value, value) => value"); return "accepted"; }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "strictness",
        description: "duplicate simple parameters are rejected before a strict body",
        source: r#"
            (function () {
                try {
                    eval("(value, value) => { 'use strict'; return value; }");
                    return "accepted";
                } catch (error) {
                    return error.name;
                }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "strictness",
        description: "a block directive makes undeclared assignment strict",
        source: r#"
            (function () {
                delete globalThis.__qjo_arrow_strict_write;
                return (() => {
                    "use strict";
                    try {
                        __qjo_arrow_strict_write = 1;
                        return "accepted";
                    } catch (error) {
                        return error.name;
                    }
                })();
            })()
        "#,
        expected: "return|string|ReferenceError",
    },
    Case {
        group: "strictness",
        description: "an arrow inherits undefined this from a strict outer call",
        source: r#"
            (function () {
                "use strict";
                return (() => this)() === undefined;
            })()
        "#,
        expected: "return|boolean|true",
    },
    Case {
        group: "metadata",
        description: "length counts simple parameters and has the standard descriptor flags",
        source: r#"
            (function () {
                var zero = () => 0;
                var one = value => value;
                var two = (left, right) => left + right;
                var descriptor = Object.getOwnPropertyDescriptor(two, "length");
                return [
                    zero.length,
                    one.length,
                    two.length,
                    descriptor.writable,
                    descriptor.enumerable,
                    descriptor.configurable
                ].join("|");
            })()
        "#,
        expected: "return|string|0|1|2|false|false|true",
    },
    Case {
        group: "metadata",
        description: "anonymous arrows gain names from assignment and property contexts",
        source: r#"
            (function () {
                var assigned = value => value;
                var object = { property: value => value };
                var computed = { ["computed"]: value => value };
                var target;
                target = value => value;
                return [
                    (value => value).name,
                    assigned.name,
                    object.property.name,
                    computed.computed.name,
                    target.name
                ].join("|");
            })()
        "#,
        expected: "return|string||assigned|property|computed|target",
    },
    Case {
        group: "metadata",
        description: "toString preserves exact expression-body arrow source",
        source: r#"
            (function () {
                var arrow = ( /*a*/ value /*b*/ ) => /*c*/ value + 1;
                return Function.prototype.toString.call(arrow);
            })()
        "#,
        expected: "return|string|( /*a*/ value /*b*/ ) => /*c*/ value + 1",
    },
    Case {
        group: "metadata",
        description: "toString preserves exact block-body arrow source",
        source: r#"
            (function () {
                var arrow = value => { /*body*/ return value; };
                return Function.prototype.toString.call(arrow);
            })()
        "#,
        expected: "return|string|value => { /*body*/ return value; }",
    },
    Case {
        group: "lexical-bindings",
        description: "a Program arrow captures the script global this",
        source: "(() => this)() === globalThis",
        expected: "return|boolean|true",
    },
    Case {
        group: "lexical-bindings",
        description: "call and bind receivers do not replace lexical this",
        source: r#"
            (function () {
                var arrow = () => this.tag;
                return arrow.call({ tag: "call" }) + "|" +
                    arrow.bind({ tag: "bind" })();
            }).call({ tag: "outer" })
        "#,
        expected: "return|string|outer|outer",
    },
    Case {
        group: "lexical-bindings",
        description: "an arrow resolves arguments from the nearest ordinary function",
        source: r#"
            (function (first, second) {
                return (() => arguments[0] + arguments[1])();
            })(20, 22)
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "lexical-bindings",
        description: "an arrow observes the outer call and construct new target",
        source: r#"
            (function () {
                function Target() {
                    this.matches = (() => new.target === Target)();
                    return this.matches;
                }
                var receiver = {};
                var call = Target.call(receiver);
                var constructed = new Target();
                return call + "|" + receiver.matches + "|" +
                    constructed.matches;
            })()
        "#,
        expected: "return|string|false|false|true",
    },
    Case {
        group: "lexical-bindings",
        description: "multiple arrow layers relay locals this and arguments",
        source: r#"
            (function (left, right) {
                var nested = () => () =>
                    left + right + this.offset + arguments.length;
                return nested().call({ offset: 1000 });
            }).call({ offset: 10 }, 10, 20)
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "with",
        description: "an escaped arrow retains with lookup and honors unscopables",
        source: r#"
            (function () {
                var x = 40;
                var arrow;
                var environment = { x: 500, y: 2 };
                environment[Symbol.unscopables] = { x: true };
                with (environment) {
                    arrow = () => x + y;
                }
                return arrow.call({ x: 0, y: 0 });
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "direct-eval",
        description: "direct eval in an arrow sees its parameter and outer lexical bindings",
        source: r#"
            (function (value) {
                return (add =>
                    eval("value + add + arguments[0] + this.offset"))(20);
            }).call({ offset: 2 }, 10)
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "direct-eval",
        description: "strict direct eval preserves typeof this as an authenticated pseudo read",
        source: r#"
            (function () {
                "use strict";
                return eval("typeof this") + "|" +
                    (eval("this") === undefined);
            })()
        "#,
        expected: "return|string|undefined|true",
    },
    Case {
        group: "direct-eval",
        description: "direct eval in an arrow observes the outer new target",
        source: r#"
            (function () {
                function Target() {
                    this.matches = (() => eval("new.target === Target"))();
                    return this.matches;
                }
                var receiver = {};
                var call = Target.call(receiver);
                var constructed = new Target();
                return call + "|" + receiver.matches + "|" +
                    constructed.matches;
            })()
        "#,
        expected: "return|string|false|false|true",
    },
    Case {
        group: "direct-eval",
        description: "an eval-created arrow and nested eval retain with and unscopables",
        source: r#"
            (function () {
                var x = 500;
                var environment = { x: 1000 };
                environment[Symbol.unscopables] = { x: true };
                var factory;
                with (environment) {
                    factory = eval('value => eval("x + value")');
                }
                return factory(10);
            })()
        "#,
        expected: "return|number|510",
    },
    Case {
        group: "direct-eval",
        description: "an arrow returned by eval retains an eval var binding",
        source: r#"
            (function () {
                var arrow = eval("var captured = 40; value => captured + value");
                return arrow(2);
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "function-object",
        description: "arrows and bound arrows have no prototype and are not constructors",
        source: r#"
            (function () {
                var arrow = () => 42;
                var constructError = "none";
                var boundConstructError = "none";
                var reflectError = "none";
                try { new arrow(); }
                catch (error) { constructError = error.name; }
                var bound = arrow.bind({ ignored: true });
                try { new bound(); }
                catch (error) { boundConstructError = error.name; }
                try { Reflect.construct(function () {}, [], arrow); }
                catch (error) { reflectError = error.name; }
                return [
                    Object.prototype.hasOwnProperty.call(arrow, "prototype"),
                    arrow.prototype === undefined,
                    Object.getPrototypeOf(arrow) === Function.prototype,
                    constructError,
                    boundConstructError,
                    reflectError
                ].join("|");
            })()
        "#,
        expected: "return|string|false|true|true|TypeError|TypeError|TypeError",
    },
];

#[test]
fn arrow_function_semantics_match_pinned_expectations() {
    for case in CASES {
        assert_eq!(
            rust_observation(case),
            case.expected,
            "Rust ArrowFunction behavior drifted for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn arrow_function_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP ArrowFunction differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    // Check every pinned QuickJS result before entering the intentionally red
    // Rust comparison. This makes an oracle drift distinguishable from the
    // expected parser/runtime gap while ArrowFunction is being implemented.
    let quickjs = CASES
        .iter()
        .map(|case| oracle_observation(&oracle, case))
        .collect::<Vec<_>>();
    for (case, observation) in CASES.iter().zip(&quickjs) {
        assert_eq!(
            observation, case.expected,
            "pinned QuickJS ArrowFunction vector drifted for {} / {}: {:?}",
            case.group, case.description, case.source,
        );
    }
    for (case, observation) in CASES.iter().zip(&quickjs) {
        assert_eq!(
            rust_observation(case),
            *observation,
            "ArrowFunction behavior differed from pinned QuickJS for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

fn rust_observation(case: &Case) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context.eval(case.source).unwrap_or_else(|error| {
        panic!(
            "Rust rejected ArrowFunction probe {} / {} ({:?}): {error}",
            case.group, case.description, case.source,
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

fn value_type(runtime: &Runtime, value: &Value) -> &'static str {
    match value {
        Value::Undefined => "undefined",
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Int(_) | Value::Float(_) => "number",
        Value::BigInt(_) => "bigint",
        Value::String(_) => "string",
        Value::Object(object) => {
            if runtime
                .as_callable(object)
                .expect("inspect callable")
                .is_some()
            {
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
