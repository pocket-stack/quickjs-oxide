use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins lexical SuperProperty inheritance through arrows nested in synchronous
// ObjectLiteral methods against QuickJS 2026-06-04. Direct eval, classes,
// parameter initializers, async methods, and generators remain separate slices.
const CASES: &[Case] = &[
    Case {
        group: "escaped-live",
        description: "an escaped arrow keeps lexical this while reading the HomeObject's live prototype",
        source: r#"
            (function () {
                var getterThis;
                var first = { get value() { getterThis = this; return -1; } };
                var second = { get value() { getterThis = this; return this.base + 2; } };
                var home = {
                    __proto__: first,
                    make() { return () => super.value; }
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
        group: "nested",
        description: "nested arrows inherit lexical this and the enclosing method's HomeObject",
        source: r#"
            (function () {
                var getterThis;
                var proto = {
                    get value() { getterThis = this; return this.base + 2; }
                };
                var home = {
                    __proto__: proto,
                    make() { return () => () => super.value; }
                };
                var receiver = { base: 40 };
                return [home.make.call(receiver)()(), getterThis === receiver].join("|");
            })()
        "#,
        expected: "return|string|42|true",
    },
    Case {
        group: "accessor",
        description: "an escaped arrow from an accessor inherits its lexical this and HomeObject",
        source: r#"
            (function () {
                var getterThis;
                var proto = {
                    get value() { getterThis = this; return this.base + 2; }
                };
                var home = {
                    __proto__: proto,
                    get make() { return () => super.value; }
                };
                var receiver = { base: 40 };
                var getter = Object.getOwnPropertyDescriptor(home, "make").get;
                var arrow = getter.call(receiver);
                return [arrow(), getterThis === receiver].join("|");
            })()
        "#,
        expected: "return|string|42|true",
    },
    Case {
        group: "mixed-entry",
        description: "arguments, new.target, and a function hoist coexist with lexical arrow super",
        source: r#"
            (function () {
                var proto = {
                    get value() { return this.base + 2; }
                };
                var home = {
                    __proto__: proto,
                    make(marker) {
                        function hoisted() { return 1; }
                        return () => [
                            super.value,
                            this.base,
                            arguments[0],
                            new.target === undefined,
                            hoisted()
                        ].join("|");
                    }
                };
                var receiver = { base: 40 };
                return home.make.call(receiver, "arg")();
            })()
        "#,
        expected: "return|string|42|40|arg|true|1",
    },
    Case {
        group: "call-receiver",
        description: "an arrow super call reads its getter from the base and calls with lexical this",
        source: r#"
            (function () {
                var getterReceiver;
                var proto = {
                    get callable() {
                        getterReceiver = this;
                        return function () { return this.base + 2; };
                    }
                };
                var home = {
                    __proto__: proto,
                    make() { return () => super.callable(); }
                };
                var receiver = { base: 40 };
                var result = home.make.call(receiver)();
                return [result, getterReceiver === proto].join("|");
            })()
        "#,
        expected: "return|string|42|true",
    },
    Case {
        group: "write",
        description: "computed super assignment through an arrow writes to lexical this",
        source: r#"
            (function () {
                var proto = {
                    set value(input) { this.saved = input + 1; }
                };
                var home = {
                    __proto__: proto,
                    make() { return (key, input) => super[key] = input; }
                };
                var receiver = {};
                var result = home.make.call(receiver)("value", 41);
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
        description: "postfix and prefix super updates in an arrow preserve values and lexical receiver",
        source: r#"
            (function () {
                var log = "";
                var proto = {
                    get value() { log += "g"; return this.count; },
                    set value(input) { log += "s"; this.count = input; }
                };
                var home = {
                    __proto__: proto,
                    make() {
                        return () => {
                            var post = super.value++;
                            var pre = ++super.value;
                            return [post, pre, this.count, log].join("|");
                        };
                    }
                };
                var receiver = { count: 40 };
                return home.make.call(receiver)();
            })()
        "#,
        expected: "return|string|40|42|42|gsgs",
    },
    Case {
        group: "strictness",
        description: "an arrow super write uses inherited or local strictness independently",
        source: r#"
            (function () {
                var home = {
                    sloppy() {
                        return () => {
                            Object.freeze(this);
                            super.value = 1;
                            return 42;
                        };
                    },
                    outerStrict() {
                        "use strict";
                        return () => {
                            Object.freeze(this);
                            super.value = 1;
                        };
                    },
                    localStrict() {
                        return () => {
                            "use strict";
                            Object.freeze(this);
                            super.value = 1;
                        };
                    }
                };
                var sloppyReceiver = {};
                var outerReceiver = {};
                var localReceiver = {};
                var outerError;
                var localError;
                var result = home.sloppy.call(sloppyReceiver)();
                try { home.outerStrict.call(outerReceiver)(); }
                catch (error) { outerError = error.name; }
                try { home.localStrict.call(localReceiver)(); }
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
        expected: "return|string|42|TypeError|TypeError|false|false|false",
    },
    Case {
        group: "delete",
        description: "delete super in an arrow throws before computed-key coercion",
        source: r#"
            (function () {
                var log = [];
                var key = {
                    toString() { log.push("coerce"); throw "key coercion"; }
                };
                var home = {
                    make() {
                        return () => delete super[(log.push("expr"), key)];
                    }
                };
                try { home.make()(); }
                catch (error) { return [error.name, log.join(",")].join("|"); }
            })()
        "#,
        expected: "return|string|ReferenceError|expr",
    },
    Case {
        group: "grammar",
        description: "a normal function cuts off an enclosing method's super binding",
        source: "({ method() { return function () { return super.value; }; } })",
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "grammar",
        description: "a global arrow has no super binding to inherit",
        source: "() => super.value",
        expected: "throw|object|SyntaxError",
    },
];

#[test]
fn pinned_quickjs_object_super_arrow_semantics_match_expectations() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-super-arrow oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        assert_eq!(
            oracle_observation(&oracle, case),
            case.expected,
            "pinned QuickJS object-super-arrow vector drifted for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn object_super_arrow_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-super-arrow oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        let quickjs = oracle_observation(&oracle, case);
        assert_eq!(
            rust_observation(case),
            quickjs,
            "object-super-arrow behavior differed from pinned QuickJS for {} / {}: {:?}",
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
