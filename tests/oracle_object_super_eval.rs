use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins direct-eval inheritance of ObjectLiteral HomeObject state against
// QuickJS 2026-06-04. Classes, parameter initializers, async methods, and
// generators remain separate slices.
const CASES: &[Case] = &[
    Case {
        group: "method-read",
        description: "dot and computed direct eval use the live HomeObject prototype and method receiver",
        source: r#"
            (function () {
                var getterThis;
                var first = { get value() { getterThis = this; return -1; } };
                var second = {
                    get value() { getterThis = this; return this.base + 2; }
                };
                var home = {
                    __proto__: first,
                    read(key) {
                        var dot = eval("super.value");
                        var computed = eval("super[key]");
                        return [dot, computed, getterThis === this].join("|");
                    }
                };
                var receiver = { base: 40 };
                Object.setPrototypeOf(home, second);
                return home.read.call(receiver, "value");
            })()
        "#,
        expected: "return|string|42|42|true",
    },
    Case {
        group: "strictness",
        description: "caller and eval-source strictness govern rejected direct-eval super writes",
        source: r#"
            (function () {
                var home = {
                    sloppy() {
                        Object.freeze(this);
                        return eval("super.value = 1");
                    },
                    outerStrict() {
                        "use strict";
                        Object.freeze(this);
                        return eval("super.value = 1");
                    },
                    localStrict() {
                        Object.freeze(this);
                        return eval("'use strict'; super.value = 1");
                    }
                };
                var sloppyReceiver = {};
                var outerReceiver = {};
                var localReceiver = {};
                var outerError;
                var localError;
                var result = home.sloppy.call(sloppyReceiver);
                try { home.outerStrict.call(outerReceiver); }
                catch (error) { outerError = error.name; }
                try { home.localStrict.call(localReceiver); }
                catch (error) { localError = error.name; }
                return [
                    result,
                    outerError,
                    localError,
                    Object.hasOwn(sloppyReceiver, "value"),
                    Object.hasOwn(outerReceiver, "value"),
                    Object.hasOwn(localReceiver, "value")
                ].join("|");
            })()
        "#,
        expected: "return|string|1|TypeError|TypeError|false|false|false",
    },
    Case {
        group: "call-receiver",
        description: "direct-eval super call reads from the base and invokes with the method receiver",
        source: r#"
            (function () {
                var getterThis;
                var callThis;
                var proto = {
                    get callable() {
                        getterThis = this;
                        return function () {
                            callThis = this;
                            return this.base + 2;
                        };
                    }
                };
                var home = {
                    __proto__: proto,
                    invoke() { return eval("super.callable()"); }
                };
                var receiver = { base: 40 };
                var result = home.invoke.call(receiver);
                return [
                    result,
                    getterThis === proto,
                    callThis === receiver
                ].join("|");
            })()
        "#,
        expected: "return|string|42|true|true",
    },
    Case {
        group: "write",
        description: "computed direct-eval super assignment writes through the current receiver",
        source: r#"
            (function () {
                var proto = {
                    set value(input) { this.saved = input + 1; }
                };
                var home = {
                    __proto__: proto,
                    write(key, input) { return eval("super[key] = input"); }
                };
                var receiver = {};
                var result = home.write.call(receiver, "value", 41);
                return [
                    result,
                    receiver.saved,
                    Object.hasOwn(home, "saved"),
                    Object.hasOwn(proto, "saved")
                ].join("|");
            })()
        "#,
        expected: "return|string|41|42|false|false",
    },
    Case {
        group: "update",
        description: "direct-eval postfix and prefix super updates preserve values receiver and ordering",
        source: r#"
            (function () {
                var log = "";
                var proto = {
                    get value() { log += "g"; return this.count; },
                    set value(input) { log += "s"; this.count = input; }
                };
                var home = {
                    __proto__: proto,
                    update() {
                        return eval(
                            "var post = super.value++;" +
                            "var pre = ++super.value;" +
                            "[post, pre, this.count, log].join('|')"
                        );
                    }
                };
                return home.update.call({ count: 40 });
            })()
        "#,
        expected: "return|string|40|42|42|gsgs",
    },
    Case {
        group: "delete",
        description: "direct-eval delete super throws before computed-key coercion",
        source: r#"
            (function () {
                var log = [];
                var key = {
                    toString() { log.push("coerce"); throw "key coercion"; }
                };
                var home = {
                    remove() {
                        return eval(
                            "delete super[(log.push('expr'), key)]"
                        );
                    }
                };
                try { home.remove(); }
                catch (error) { return [error.name, log.join(",")].join("|"); }
            })()
        "#,
        expected: "return|string|ReferenceError|expr",
    },
    Case {
        group: "accessors",
        description: "getter and setter direct eval inherit their own HomeObject this and arguments",
        source: r#"
            (function () {
                var proto = {
                    get value() { return this.base + 2; },
                    set value(input) { this.saved = input + 2; }
                };
                var home = {
                    __proto__: proto,
                    get answer() { return eval("super.value"); },
                    set answer(input) {
                        eval("super.value = arguments[0]");
                    }
                };
                var descriptor = Object.getOwnPropertyDescriptor(home, "answer");
                var receiver = { base: 40 };
                var value = descriptor.get.call(receiver);
                descriptor.set.call(receiver, 40);
                return [value, receiver.saved].join("|");
            })()
        "#,
        expected: "return|string|42|42",
    },
    Case {
        group: "authored-arrow",
        description: "an escaped authored arrow carries method direct-eval super across a live prototype change",
        source: r#"
            (function () {
                var getterThis;
                var first = { value: -1 };
                var second = {
                    get value() { getterThis = this; return this.base + 2; }
                };
                var home = {
                    __proto__: first,
                    make() { return () => eval("super.value"); }
                };
                var receiver = { base: 40 };
                var arrow = home.make.call(receiver);
                Object.setPrototypeOf(home, second);
                return [arrow(), getterThis === receiver].join("|");
            })()
        "#,
        expected: "return|string|42|true",
    },
    Case {
        group: "eval-created-arrow",
        description: "an escaped arrow created by eval retains imported this and HomeObject cells",
        source: r#"
            (function () {
                var getterThis;
                var first = { value: -1 };
                var second = {
                    get value() { getterThis = this; return this.base + 2; }
                };
                var home = {
                    __proto__: first,
                    make() { return eval("() => super.value"); }
                };
                var receiver = { base: 40 };
                var arrow = home.make.call(receiver);
                Object.setPrototypeOf(home, second);
                return [arrow(), getterThis === receiver].join("|");
            })()
        "#,
        expected: "return|string|42|true",
    },
    Case {
        group: "nested-eval",
        description: "method and arrow nested direct eval recursively preserve super capability",
        source: r#"
            (function () {
                var proto = {
                    get value() { return this.base + 2; }
                };
                var home = {
                    __proto__: proto,
                    method() { return eval("eval('super.value')"); },
                    arrow() {
                        return (() => eval("eval('super.value')"))();
                    }
                };
                var receiver = { base: 40 };
                return [
                    home.method.call(receiver),
                    home.arrow.call(receiver)
                ].join("|");
            })()
        "#,
        expected: "return|string|42|42",
    },
    Case {
        group: "first-slot-wins",
        description: "eval-visible pseudo relays are reused by authored super in the same escaped arrow",
        source: r#"
            (function () {
                var getterThis = [];
                var proto = {
                    get value() {
                        getterThis.push(this);
                        return this.base + 2;
                    }
                };
                var home = {
                    __proto__: proto,
                    make() {
                        return () => [
                            super.value,
                            eval("super.value"),
                            super.value
                        ].join("|");
                    }
                };
                var receiver = { base: 40 };
                var arrow = home.make.call(receiver);
                return [
                    arrow(),
                    getterThis.length,
                    getterThis[0] === receiver,
                    getterThis[1] === receiver,
                    getterThis[2] === receiver
                ].join("|");
            })()
        "#,
        expected: "return|string|42|42|42|3|true|true|true",
    },
    Case {
        group: "ordinary-cutoff",
        description: "ordinary functions cut off authored and eval-created method super capability",
        source: r#"
            (function () {
                var proto = { value: 42 };
                var home = {
                    __proto__: proto,
                    authored() {
                        super.value;
                        function inner() { return eval("super.value"); }
                        return inner();
                    },
                    arrow() {
                        super.value;
                        function inner() {
                            return (() => eval("super.value"))();
                        }
                        return inner();
                    },
                    created() {
                        super.value;
                        return eval("(function () { return super.value; })");
                    }
                };
                var authoredError;
                var arrowError;
                var createdError;
                try { home.authored(); }
                catch (error) { authoredError = error.name; }
                try { home.arrow(); }
                catch (error) { arrowError = error.name; }
                try { home.created(); }
                catch (error) { createdError = error.name; }
                return [authoredError, arrowError, createdError].join("|");
            })()
        "#,
        expected: "return|string|SyntaxError|SyntaxError|SyntaxError",
    },
    Case {
        group: "global-cutoff",
        description: "global direct eval and a global arrow have no HomeObject capability",
        source: r#"
            var directError;
            var arrowError;
            try { eval("super.value"); }
            catch (error) { directError = error.name; }
            try { (() => eval("super.value"))(); }
            catch (error) { arrowError = error.name; }
            [directError, arrowError].join("|");
        "#,
        expected: "return|string|SyntaxError|SyntaxError",
    },
    Case {
        group: "indirect-cutoff",
        description: "indirect eval inside a method does not inherit its HomeObject capability",
        source: r#"
            (function () {
                var home = {
                    method() { return (0, eval)("super.value"); }
                };
                try { home.method(); }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "alias-cutoff",
        description: "an eval alias called inside a method remains indirect and has no HomeObject",
        source: r#"
            (function () {
                var alias = eval;
                var home = {
                    method() { return alias("super.value"); }
                };
                try { home.method(); }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "super-call",
        description: "object-method and global direct eval reject super call before arguments execute",
        source: r#"
            var log = [];
            var methodError;
            var arrowError;
            var nestedError;
            var globalError;
            var nestedSource = "super(log.push('nested'))";
            var home = {
                method() {
                    try { eval("super(log.push('method'))"); }
                    catch (error) { methodError = error.name; }
                    try { (() => eval("super(log.push('arrow'))"))(); }
                    catch (error) { arrowError = error.name; }
                    try { eval("eval(nestedSource)"); }
                    catch (error) { nestedError = error.name; }
                }
            };
            home.method();
            try { eval("super(log.push('global'))"); }
            catch (error) { globalError = error.name; }
            [methodError, arrowError, nestedError, globalError, log.join(",")].join("|");
        "#,
        expected: "return|string|SyntaxError|SyntaxError|SyntaxError|SyntaxError|",
    },
];

#[test]
fn object_super_eval_semantics_match_pinned_expectations() {
    for case in CASES {
        assert_eq!(
            rust_observation(case),
            case.expected,
            "Rust object-super-eval behavior drifted for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn pinned_quickjs_object_super_eval_semantics_match_expectations() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-super-eval oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        assert_eq!(
            oracle_observation(&oracle, case),
            case.expected,
            "pinned QuickJS object-super-eval vector drifted for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn object_super_eval_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-super-eval oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        let quickjs = oracle_observation(&oracle, case);
        assert_eq!(
            rust_observation(case),
            quickjs,
            "object-super-eval behavior differed from pinned QuickJS for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

fn rust_observation(case: &Case) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    match context.eval(case.source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(&runtime, &value),
            primitive_value_text(value),
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| {
                    panic!(
                        "take Rust exception for {} / {}: {error}",
                        case.group, case.description,
                    )
                })
                .unwrap_or_else(|| {
                    panic!(
                        "Rust exception was missing for {} / {}",
                        case.group, case.description,
                    )
                });
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}",
                    error_name(&runtime, &mut context, &error, case),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(&runtime, &value),
                    primitive_value_text(value),
                ),
            }
        }
        Err(error) => panic!(
            "Rust engine failure for {} / {} ({:?}): {error}",
            case.group, case.description, case.source,
        ),
    }
}

fn error_name(
    runtime: &Runtime,
    context: &mut quickjs_oxide::Context,
    error: &quickjs_oxide::ObjectRef,
    case: &Case,
) -> String {
    let key = runtime
        .intern_property_key("name")
        .expect("Error name property key");
    let Value::String(value) = context.get_property(error, &key).unwrap_or_else(|failure| {
        panic!(
            "read Error.name for {} / {}: {failure}",
            case.group, case.description,
        )
    }) else {
        panic!(
            "Error.name was not a string for {} / {}",
            case.group, case.description,
        );
    };
    value.to_utf8_lossy()
}

fn oracle_observation(oracle: &OsStr, case: &Case) -> String {
    let wrapper = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print('return|' + typeof value + '|' + String(value));
} catch (error) {
  if (error !== null && typeof error === 'object')
    print('throw|object|' + error.name);
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
