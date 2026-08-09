use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the synchronous ObjectLiteral getter/setter slice from QuickJS
// 2026-06-04. Default/destructuring setter parameters and `super` remain
// separate frontiers because they require non-simple parameter environments
// or [[HomeObject]] semantics.
const CASES: &[Case] = &[
    Case {
        group: "pairing",
        description: "a getter and setter with the same key merge into one accessor",
        source: r#"
            (function () {
                var object = {
                    get answer() { return this.stored + 1; },
                    set answer(value) { this.stored = value - 1; }
                };
                object.answer = 42;
                var descriptor = Object.getOwnPropertyDescriptor(object, "answer");
                return [
                    object.answer,
                    object.stored,
                    typeof descriptor.get,
                    typeof descriptor.set,
                    descriptor.enumerable,
                    descriptor.configurable
                ].join("|");
            })()
        "#,
        expected: "return|string|42|41|function|function|true|true",
    },
    Case {
        group: "pairing",
        description: "repeated accessor halves replace only their matching half",
        source: r#"
            (function () {
                var object = {
                    set value(input) { this.first = input; },
                    get value() { return 1; },
                    get value() { return 42; },
                    set other(input) { this.second = input; },
                    get other() { return 42; },
                    set other(input) { this.third = input; }
                };
                var value = Object.getOwnPropertyDescriptor(object, "value");
                var other = Object.getOwnPropertyDescriptor(object, "other");
                value.set.call(object, 20);
                other.set.call(object, 22);
                return [
                    value.get.call(object),
                    object.first,
                    other.get.call(object),
                    object.second,
                    object.third
                ].join("|");
            })()
        "#,
        expected: "return|string|42|20|42||22",
    },
    Case {
        group: "pairing",
        description: "data and method definitions replace accessor descriptor kinds in source order",
        source: r#"
            (function () {
                var a = { x: 1, get x() { return 42; } };
                var b = { get x() { return 1; }, x: 42 };
                var c = { x() { return 1; }, set x(value) { this.seen = value; } };
                var d = { get x() { return 1; }, x() { return 42; } };
                var ad = Object.getOwnPropertyDescriptor(a, "x");
                var bd = Object.getOwnPropertyDescriptor(b, "x");
                var cd = Object.getOwnPropertyDescriptor(c, "x");
                var dd = Object.getOwnPropertyDescriptor(d, "x");
                c.x = 42;
                return [
                    a.x,
                    typeof ad.get,
                    ad.set === undefined,
                    b.x,
                    bd.value,
                    cd.get === undefined,
                    typeof cd.set,
                    c.seen,
                    dd.value(),
                    dd.writable
                ].join("|");
            })()
        "#,
        expected: "return|string|42|function|true|42|42|true|function|42|42|true",
    },
    Case {
        group: "property-keys",
        description: "fixed computed string numeric keyword and Symbol keys infer accessor names",
        source: r#"
            (function () {
                var described = Symbol("named");
                var missing = Symbol();
                var empty = Symbol("");
                var object = {
                    get fixed() {},
                    set "quoted"(value) {},
                    get 7() {},
                    set if(value) {},
                    get [described]() {},
                    set [described](value) {},
                    get [missing]() {},
                    set [empty](value) {}
                };
                return [
                    Object.getOwnPropertyDescriptor(object, "fixed").get.name,
                    Object.getOwnPropertyDescriptor(object, "quoted").set.name,
                    Object.getOwnPropertyDescriptor(object, "7").get.name,
                    Object.getOwnPropertyDescriptor(object, "if").set.name,
                    Object.getOwnPropertyDescriptor(object, described).get.name,
                    Object.getOwnPropertyDescriptor(object, described).set.name,
                    Object.getOwnPropertyDescriptor(object, missing).get.name,
                    Object.getOwnPropertyDescriptor(object, empty).set.name
                ].join("|");
            })()
        "#,
        expected: "return|string|get fixed|set quoted|get 7|set if|get [named]|set [named]|get |set []",
    },
    Case {
        group: "grammar",
        description: "a line terminator is allowed after contextual get and set",
        source: r#"
            (function () {
                var object = {
                    get
                    answer() { return 42; },
                    set
                    answer(value) { this.saved = value; }
                };
                var descriptor = Object.getOwnPropertyDescriptor(object, "answer");
                descriptor.set.call(object, 41);
                return [
                    descriptor.get.call(object),
                    object.saved,
                    descriptor.get.name,
                    descriptor.set.name
                ].join("|");
            })()
        "#,
        expected: "return|string|42|41|get answer|set answer",
    },
    Case {
        group: "property-keys",
        description: "computed accessor keys are converted exactly once per definition",
        source: r#"
            (function () {
                var log = "";
                var getterKey = {
                    [Symbol.toPrimitive]() { log += "g"; return "answer"; }
                };
                var setterKey = {
                    [Symbol.toPrimitive]() { log += "s"; return "answer"; }
                };
                var object = {
                    get [getterKey]() { return this.saved; },
                    set [setterKey](value) { this.saved = value; }
                };
                object.answer = 42;
                return log + "|" + object.answer;
            })()
        "#,
        expected: "return|string|gs|42",
    },
    Case {
        group: "function-object",
        description: "accessor function metadata and property descriptors match QuickJS",
        source: r#"
            (function () {
                var object = {
                    get answer() { return 42; },
                    set answer(value) {}
                };
                var property = Object.getOwnPropertyDescriptor(object, "answer");
                var getName = Object.getOwnPropertyDescriptor(property.get, "name");
                var setName = Object.getOwnPropertyDescriptor(property.set, "name");
                var getLength = Object.getOwnPropertyDescriptor(property.get, "length");
                var setLength = Object.getOwnPropertyDescriptor(property.set, "length");
                return [
                    property.get.name,
                    property.set.name,
                    property.get.length,
                    property.set.length,
                    "value" in property,
                    "writable" in property,
                    property.enumerable,
                    property.configurable,
                    getName.writable,
                    getName.enumerable,
                    getName.configurable,
                    setName.writable,
                    setName.enumerable,
                    setName.configurable,
                    getLength.writable,
                    getLength.enumerable,
                    getLength.configurable,
                    setLength.writable,
                    setLength.enumerable,
                    setLength.configurable
                ].join("|");
            })()
        "#,
        expected: "return|string|get answer|set answer|0|1|false|false|true|true|false|false|true|false|false|true|false|false|true|false|false|true",
    },
    Case {
        group: "call-environment",
        description: "accessors own dynamic this arguments and new target",
        source: r#"
            (function () {
                var descriptor = Object.getOwnPropertyDescriptor({
                    get answer() {
                        return this.base + arguments.length +
                            (typeof new.target === "undefined" ? 1 : 0);
                    },
                    set answer(value) {
                        this.saved = value + arguments.length +
                            (typeof new.target === "undefined" ? 1 : 0);
                    }
                }, "answer");
                var receiver = { base: 41 };
                var getter = descriptor.get.call(receiver);
                descriptor.set.call(receiver, 40);
                return getter + "|" + receiver.saved;
            })()
        "#,
        expected: "return|string|42|42",
    },
    Case {
        group: "call-environment",
        description: "accessors inherit strictness rather than becoming strict automatically",
        source: r#"
            (function () {
                var sloppy = Object.getOwnPropertyDescriptor({
                    get x() { return this; }
                }, "x").get;
                var strict = (function () {
                    "use strict";
                    return Object.getOwnPropertyDescriptor({
                        get x() { return this; }
                    }, "x").get;
                })();
                return (sloppy() === globalThis) + "|" +
                    (strict() === undefined);
            })()
        "#,
        expected: "return|string|true|true",
    },
    Case {
        group: "direct-eval",
        description: "direct eval resolves accessor-owned this arguments and new target",
        source: r#"
            (function () {
                var descriptor = Object.getOwnPropertyDescriptor({
                    get answer() {
                        return eval(
                            "this.base + arguments.length + " +
                            "(typeof new.target === 'undefined' ? 1 : 0)"
                        );
                    },
                    set answer(value) {
                        this.saved = eval(
                            "arguments[0] + arguments.length + " +
                            "(typeof new.target === 'undefined' ? 1 : 0)"
                        );
                    }
                }, "answer");
                var receiver = { base: 41 };
                var getter = descriptor.get.call(receiver);
                descriptor.set.call(receiver, 40);
                return getter + "|" + receiver.saved;
            })()
        "#,
        expected: "return|string|42|42",
    },
    Case {
        group: "scope",
        description: "an accessor property name does not create a private function binding",
        source: r#"
            (function () {
                var answer = 41;
                return ({ get answer() { return answer + 1; } }).answer;
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "parameters",
        description: "a simple setter accepts one parameter and a trailing comma",
        source: r#"
            (function () {
                var descriptor = Object.getOwnPropertyDescriptor({
                    set answer(value,) { this.saved = value; }
                }, "answer");
                var receiver = {};
                descriptor.set.call(receiver, 42);
                return receiver.saved + "|" + descriptor.set.length;
            })()
        "#,
        expected: "return|string|42|1",
    },
    Case {
        group: "parameters",
        description: "getter and setter arity violations are syntax errors",
        source: r#"
            (function () {
                var sources = [
                    "({ get x(value) {} })",
                    "({ get x(,) {} })",
                    "({ set x() {} })",
                    "({ set x(left, right) {} })",
                    "({ set x(...values) {} })"
                ];
                var result = [];
                for (var index = 0; index < sources.length; index++) {
                    try { eval(sources[index]); result.push("accepted"); }
                    catch (error) { result.push(error.name); }
                }
                return result.join("|");
            })()
        "#,
        expected: "return|string|SyntaxError|SyntaxError|SyntaxError|SyntaxError|SyntaxError",
    },
    Case {
        group: "function-object",
        description: "getter and setter functions are callable non-constructors without prototypes",
        source: r#"
            (function () {
                var descriptor = Object.getOwnPropertyDescriptor({
                    get answer() { return 42; },
                    set answer(value) {}
                }, "answer");
                var getError = "none";
                var setError = "none";
                try { new descriptor.get(); } catch (error) { getError = error.name; }
                try { new descriptor.set(); } catch (error) { setError = error.name; }
                return [
                    descriptor.get(),
                    getError,
                    setError,
                    Object.hasOwn(descriptor.get, "prototype"),
                    Object.hasOwn(descriptor.set, "prototype")
                ].join("|");
            })()
        "#,
        expected: "return|string|42|TypeError|TypeError|false|false",
    },
    Case {
        group: "proto",
        description: "__proto__ accessors are ordinary own properties and not ProtoSetter entries",
        source: r#"
            (function () {
                var object = {
                    __proto__: null,
                    get __proto__() { return 42; },
                    set __proto__(value) { this.saved = value; }
                };
                object.__proto__ = 41;
                var descriptor = Object.getOwnPropertyDescriptor(object, "__proto__");
                return [
                    Object.getPrototypeOf(object) === null,
                    Object.hasOwn(object, "__proto__"),
                    object.__proto__,
                    object.saved,
                    typeof descriptor.get,
                    typeof descriptor.set
                ].join("|");
            })()
        "#,
        expected: "return|string|true|true|42|41|function|function",
    },
    Case {
        group: "source",
        description: "Function.prototype.toString preserves accessor source spans",
        source: r#"
            (function () {
                var descriptor = Object.getOwnPropertyDescriptor({
                    get answer(){ return 42; },
                    set answer(value){ this.saved = value; }
                }, "answer");
                return Function.prototype.toString.call(descriptor.get) + "|" +
                    Function.prototype.toString.call(descriptor.set);
            })()
        "#,
        expected: "return|string|get answer(){ return 42; }|set answer(value){ this.saved = value; }",
    },
];

#[test]
fn object_accessor_semantics_match_pinned_expectations() {
    for case in CASES {
        assert_eq!(
            rust_observation(case),
            case.expected,
            "Rust object-accessor behavior drifted for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn object_accessor_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-accessor differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    let quickjs = CASES
        .iter()
        .map(|case| oracle_observation(&oracle, case))
        .collect::<Vec<_>>();
    for (case, observation) in CASES.iter().zip(&quickjs) {
        assert_eq!(
            observation, case.expected,
            "pinned QuickJS object-accessor vector drifted for {} / {}: {:?}",
            case.group, case.description, case.source,
        );
    }
    for (case, observation) in CASES.iter().zip(&quickjs) {
        assert_eq!(
            rust_observation(case),
            *observation,
            "object-accessor behavior differed from pinned QuickJS for {} / {}: {:?}",
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
            "Rust rejected object-accessor probe {} / {} ({:?}): {error}",
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
