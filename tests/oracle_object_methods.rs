use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the synchronous, non-generator ObjectLiteral concise-method slice from
// QuickJS 2026-06-04. Accessors, async methods, generators, default/rest/
// destructuring parameters, and `super` deliberately belong to later slices.
const CASES: &[Case] = &[
    Case {
        group: "property-keys",
        description: "fixed computed string numeric and keyword keys define named methods",
        source: r#"
            (function () {
                var computedKey = "computed";
                var object = {
                    fixed() { return 1; },
                    [computedKey]() { return 2; },
                    "quoted"() { return 3; },
                    7() { return 4; },
                    if() { return 5; }
                };
                return [
                    object.fixed(),
                    object.computed(),
                    object.quoted(),
                    object[7](),
                    object.if(),
                    object.fixed.name,
                    object.computed.name,
                    object.quoted.name,
                    object[7].name,
                    object.if.name
                ].join("|");
            })()
        "#,
        expected: "return|string|1|2|3|4|5|fixed|computed|quoted|7|if",
    },
    Case {
        group: "property-keys",
        description: "computed Symbol keys use QuickJS method name formatting",
        source: r#"
            (function () {
                var described = Symbol("named");
                var missing = Symbol();
                var empty = Symbol("");
                var object = {
                    [described]() {},
                    [missing]() {},
                    [empty]() {}
                };
                return [
                    object[described].name,
                    object[missing].name,
                    object[empty].name
                ].join("|");
            })()
        "#,
        expected: "return|string|[named]||[]",
    },
    Case {
        group: "property-keys",
        description: "contextual get set and async keys remain ordinary methods before a paren",
        source: r#"
            (function () {
                var object = {
                    get() { return 20; },
                    set(value) { return value; },
                    async() { return 2; }
                };
                return [
                    object.get() + object.set(20) + object.async(),
                    object.get.name,
                    object.set.name,
                    object.async.name,
                    object.get.length,
                    object.set.length,
                    object.async.length
                ].join("|");
            })()
        "#,
        expected: "return|string|42|get|set|async|0|1|0",
    },
    Case {
        group: "call-environment",
        description: "a method owns dynamic this rather than capturing its defining receiver",
        source: r#"
            (function () {
                var object = {
                    base: 100,
                    method(delta) { return this.base + delta; }
                };
                var method = object.method;
                return object.method(-58) + "|" + method.call({ base: 40 }, 2);
            })()
        "#,
        expected: "return|string|42|42",
    },
    Case {
        group: "call-environment",
        description: "methods are not automatically strict but inherit outer strictness",
        source: r#"
            (function () {
                var sloppy = { method() { return this; } }.method;
                var strict = (function () {
                    "use strict";
                    return { method() { return this; } }.method;
                })();
                return (sloppy() === globalThis) + "|" +
                    (strict() === undefined);
            })()
        "#,
        expected: "return|string|true|true",
    },
    Case {
        group: "call-environment",
        description: "a method owns arguments and binds it to the current call",
        source: r#"
            (function (outer) {
                var outerArguments = arguments;
                return ({
                    method(left, right) {
                        return (arguments !== outerArguments) + "|" +
                            arguments.length + "|" +
                            (left + right) + "|" +
                            (arguments[0] + arguments[1]);
                    }
                }).method(20, 22);
            })(99)
        "#,
        expected: "return|string|true|2|42|42",
    },
    Case {
        group: "call-environment",
        description: "a directly called method has its own undefined new target",
        source: "({ method() { return typeof new.target; } }).method()",
        expected: "return|string|undefined",
    },
    Case {
        group: "direct-eval",
        description: "direct eval resolves the method this arguments and new target owners",
        source: r#"
            (function () {
                var method = ({
                    method(value) {
                        return eval(
                            "this.tag + arguments[0] + " +
                            "(typeof new.target === 'undefined')"
                        );
                    }
                }).method;
                return method.call({ tag: 19 }, 22);
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "scope",
        description: "the property key does not create a private function self binding",
        source: r#"
            (function () {
                var method = 40;
                return ({ method() { return method + 2; } }).method();
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "parameters",
        description: "duplicate simple method parameters are an early error in sloppy code",
        source: r#"
            (function () {
                try {
                    eval("({ method(value, value) { return value; } })");
                    return "accepted";
                } catch (error) {
                    return error.name;
                }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "parameters",
        description: "a trailing comma is accepted and does not increase method length",
        source: r#"
            (function () {
                var method = ({ method(value,) { return value + 1; } }).method;
                return method(41) + "|" + method.length;
            })()
        "#,
        expected: "return|string|42|1",
    },
    Case {
        group: "function-object",
        description: "method name length property and function descriptors match QuickJS",
        source: r#"
            (function () {
                var object = { method(left, right) {} };
                var method = object.method;
                var property = Object.getOwnPropertyDescriptor(object, "method");
                var name = Object.getOwnPropertyDescriptor(method, "name");
                var length = Object.getOwnPropertyDescriptor(method, "length");
                return [
                    method.name,
                    method.length,
                    property.value === method,
                    property.writable,
                    property.enumerable,
                    property.configurable,
                    name.value,
                    name.writable,
                    name.enumerable,
                    name.configurable,
                    length.value,
                    length.writable,
                    length.enumerable,
                    length.configurable,
                    Object.prototype.hasOwnProperty.call(method, "prototype"),
                    method.prototype === undefined,
                    Object.getPrototypeOf(method) === Function.prototype
                ].join("|");
            })()
        "#,
        expected: "return|string|method|2|true|true|true|true|method|false|false|true|2|false|false|true|false|true|true",
    },
    Case {
        group: "function-object",
        description: "methods remain callable but are not constructors or new targets",
        source: r#"
            (function () {
                var method = { method() { return 42; } }.method;
                var constructError = "none";
                var newTargetError = "none";
                try { new method(); }
                catch (error) { constructError = error.name; }
                try { Reflect.construct(function () {}, [], method); }
                catch (error) { newTargetError = error.name; }
                return method() + "|" + constructError + "|" + newTargetError;
            })()
        "#,
        expected: "return|string|42|TypeError|TypeError",
    },
    Case {
        group: "proto",
        description: "a __proto__ method is an own data property not a ProtoSetter",
        source: r#"
            (function () {
                var object = {
                    __proto__: null,
                    __proto__() { return 42; }
                };
                var descriptor = Object.getOwnPropertyDescriptor(object, "__proto__");
                return [
                    Object.getPrototypeOf(object) === null,
                    Object.hasOwn(object, "__proto__"),
                    object.__proto__(),
                    descriptor.writable,
                    descriptor.enumerable,
                    descriptor.configurable
                ].join("|");
            })()
        "#,
        expected: "return|string|true|true|42|true|true|true",
    },
];

#[test]
fn object_method_semantics_match_pinned_expectations() {
    for case in CASES {
        assert_eq!(
            rust_observation(case),
            case.expected,
            "Rust object-method behavior drifted for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn object_method_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-method differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    let quickjs = CASES
        .iter()
        .map(|case| oracle_observation(&oracle, case))
        .collect::<Vec<_>>();
    for (case, observation) in CASES.iter().zip(&quickjs) {
        assert_eq!(
            observation, case.expected,
            "pinned QuickJS object-method vector drifted for {} / {}: {:?}",
            case.group, case.description, case.source,
        );
    }
    for (case, observation) in CASES.iter().zip(&quickjs) {
        assert_eq!(
            rust_observation(case),
            *observation,
            "object-method behavior differed from pinned QuickJS for {} / {}: {:?}",
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
            "Rust rejected object-method probe {} / {} ({:?}): {error}",
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
