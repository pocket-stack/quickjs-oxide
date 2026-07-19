use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the direct SuperProperty slice for synchronous ObjectLiteral methods
// and accessors against QuickJS 2026-06-04 before the Rust implementation is
// opened. Nested Arrow/eval inheritance, parameter initializers, classes,
// async methods, and generators deliberately belong to later slices.
const CASES: &[Case] = &[
    Case {
        group: "lookup",
        description: "dot and computed reads start at the HomeObject prototype",
        source: r#"
            (function () {
                var grand = { inherited: 20 };
                var proto = Object.create(grand);
                proto.direct = 22;
                var object = {
                    __proto__: proto,
                    direct: 100,
                    inherited: 100,
                    read() { return super.direct + super["inherited"]; }
                };
                return object.read();
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "receiver",
        description: "super getters calls and setters use the current call receiver",
        source: r#"
            (function () {
                var proto = {
                    method(addend) { return this.base + addend; },
                    get current() { return this.base + 1; },
                    set current(value) { this.saved = value + 1; }
                };
                var home = {
                    __proto__: proto,
                    method() { return super.method(2); },
                    get read() { return super.current; },
                    set write(value) { super.current = value; }
                };
                var receiver = { base: 40 };
                var descriptor = Object.getOwnPropertyDescriptor(home, "read");
                var setter = Object.getOwnPropertyDescriptor(home, "write").set;
                var methodResult = home.method.call(receiver);
                var getterResult = descriptor.get.call(receiver);
                setter.call(receiver, 41);
                return [
                    methodResult,
                    getterResult,
                    receiver.saved,
                    Object.hasOwn(home, "saved")
                ].join("|");
            })()
        "#,
        expected: "return|string|42|41|42|false",
    },
    Case {
        group: "receiver",
        description: "QuickJS super-call getter rewrite reads with the frozen base before calling with this",
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
                    call() { return super.callable(); }
                };
                var receiver = { base: 40 };
                return [home.call.call(receiver), getterReceiver === proto].join("|");
            })()
        "#,
        expected: "return|string|42|true",
    },
    Case {
        group: "home-object",
        description: "an extracted method retains its HomeObject and observes its current prototype",
        source: r#"
            (function () {
                var first = { value: 1 };
                var second = { value: 42 };
                var home = {
                    __proto__: first,
                    method() { return super.value; }
                };
                var method = home.method;
                Object.setPrototypeOf(home, second);
                return method.call({ value: 100 });
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "assignment",
        description: "simple assignment returns the RHS and sends the current receiver to super Set",
        source: r#"
            (function () {
                var proto = {
                    set value(input) { this.saved = input + 1; }
                };
                var home = {
                    __proto__: proto,
                    write(input) { return super.value = input; }
                };
                var receiver = {};
                var result = home.write.call(receiver, 41);
                return [result, receiver.saved, Object.hasOwn(home, "saved")].join("|");
            })()
        "#,
        expected: "return|string|41|42|false",
    },
    Case {
        group: "assignment",
        description: "compound assignment performs receiver-sensitive Get then Set once each",
        source: r#"
            (function () {
                var log = "";
                var proto = {
                    get value() { log += "g"; return this.count; },
                    set value(input) { log += "s"; this.count = input; }
                };
                var home = {
                    __proto__: proto,
                    count: 40,
                    bump() { return super.value += 2; }
                };
                var result = home.bump();
                return [result, home.count, log].join("|");
            })()
        "#,
        expected: "return|string|42|42|gs",
    },
    Case {
        group: "assignment",
        description: "logical assignments short circuit RHS and super Set independently",
        source: r#"
            (function () {
                var log = "";
                var rhsCalls = 0;
                var proto = {
                    get value() { log += "g"; return this.slot; },
                    set value(input) { log += "s"; this.slot = input; }
                };
                var home = {
                    __proto__: proto,
                    and(rhs) { return super.value &&= rhs(); },
                    or(rhs) { return super.value ||= rhs(); },
                    nil(rhs) { return super.value ??= rhs(); }
                };
                function rhs() { rhsCalls += 1; return 42; }
                var a = { slot: 0 };
                var b = { slot: 0 };
                var c = { slot: null };
                var andResult = home.and.call(a, rhs);
                var orResult = home.or.call(b, rhs);
                var nilResult = home.nil.call(c, rhs);
                return [
                    andResult,
                    orResult,
                    nilResult,
                    a.slot,
                    b.slot,
                    c.slot,
                    rhsCalls,
                    log
                ].join("|");
            })()
        "#,
        expected: "return|string|0|42|42|0|42|42|2|ggsgs",
    },
    Case {
        group: "assignment",
        description: "JS_PROP_THROW_STRICT distinguishes rejected sloppy and strict super writes",
        source: r#"
            (function () {
                var home = {
                    sloppy() {
                        Object.freeze(this);
                        super.value = 1;
                        return 42;
                    },
                    strict() {
                        "use strict";
                        Object.freeze(this);
                        super.value = 1;
                    }
                };
                var sloppyReceiver = {};
                var strictReceiver = {};
                var strictError;
                var result = home.sloppy.call(sloppyReceiver);
                try { home.strict.call(strictReceiver); }
                catch (error) { strictError = error.name; }
                return [
                    result,
                    strictError,
                    Object.hasOwn(sloppyReceiver, "value"),
                    Object.hasOwn(strictReceiver, "value")
                ].join("|");
            })()
        "#,
        expected: "return|string|42|TypeError|false|false",
    },
    Case {
        group: "update",
        description: "postfix and prefix updates preserve their distinct result values",
        source: r#"
            (function () {
                var log = "";
                var proto = {
                    get value() { log += "g"; return this.count; },
                    set value(input) { log += "s"; this.count = input; }
                };
                var home = {
                    __proto__: proto,
                    count: 40,
                    update() {
                        var post = super.value++;
                        var pre = ++super.value;
                        return [post, pre, this.count, log].join("|");
                    }
                };
                return home.update();
            })()
        "#,
        expected: "return|string|40|42|42|gsgs",
    },
    Case {
        group: "null-base",
        description: "null HomeObject prototypes throw after assignment RHS evaluation",
        source: r#"
            (function () {
                var rhsCalls = 0;
                var home = {
                    __proto__: null,
                    read() { return super.value; },
                    write() { return super.value = (rhsCalls += 1, 42); }
                };
                var readError;
                var writeError;
                try { home.read(); } catch (error) { readError = error.name; }
                try { home.write(); } catch (error) { writeError = error.name; }
                return [
                    readError,
                    writeError,
                    rhsCalls,
                    Object.hasOwn(home, "value")
                ].join("|");
            })()
        "#,
        expected: "return|string|TypeError|TypeError|1|false",
    },
    Case {
        group: "null-base",
        description: "computed super read coerces its key before diagnosing a null base",
        source: r#"
            (function () {
                var key = { toString() { throw "key coercion"; } };
                var home = { __proto__: null, read() { return super[key]; } };
                try { home.read(); } catch (error) { return error; }
            })()
        "#,
        expected: "return|string|key coercion",
    },
    Case {
        group: "null-base",
        description: "null super assignment rejects the base before deferred key coercion",
        source: r#"
            (function () {
                var log = [];
                var key = { toString() { log.push("key"); return "value"; } };
                var home = {
                    __proto__: null,
                    write() { return super[key] = (log.push("rhs"), 42); }
                };
                var errorName;
                try { home.write(); } catch (error) { errorName = error.name; }
                return [errorName, log.join(",")].join("|");
            })()
        "#,
        expected: "return|string|TypeError|rhs",
    },
    Case {
        group: "computed-order",
        description: "SuperBase is fixed before deferred computed-key conversion",
        source: r#"
            (function () {
                var log = [];
                var first = {
                    get value() { log.push("get:first"); return this.stored; },
                    set value(input) { log.push("set:first"); this.stored = input; }
                };
                var second = {
                    get value() { log.push("get:second"); return -1; },
                    set value(input) { log.push("set:second"); this.stored = -1; }
                };
                var home = {
                    __proto__: first,
                    stored: 40,
                    read() { return super[key]; },
                    compound() { return super[key] += 1; },
                    prefix() { return ++super[key]; }
                };
                var key = {
                    toString() {
                        log.push("key");
                        Object.setPrototypeOf(home, second);
                        return "value";
                    }
                };
                Object.setPrototypeOf(home, first);
                var read = home.read();
                Object.setPrototypeOf(home, first);
                var compound = home.compound();
                Object.setPrototypeOf(home, first);
                var prefix = home.prefix();
                return [read, compound, prefix, home.stored, log.join(",")].join("|");
            })()
        "#,
        expected: "return|string|40|41|42|42|key,get:first,key,get:first,set:first,key,get:first,set:first",
    },
    Case {
        group: "computed-order",
        description: "computed method and accessor keys run in source order and retain HomeObject",
        source: r#"
            (function () {
                var log = [];
                function key(name) { log.push("key:" + name); return name; }
                var proto = {
                    method() { return this.base + 2; },
                    get value() { return this.base + 1; },
                    set value(input) { this.saved = input + 1; }
                };
                var home = {
                    __proto__: (log.push("proto"), proto),
                    [key("method")]() {
                        log.push("method-body");
                        return super.method();
                    },
                    get [key("read")]() {
                        log.push("get-body");
                        return super.value;
                    },
                    set [key("write")](input) {
                        log.push("set-body");
                        super.value = input;
                    }
                };
                var receiver = { base: 40 };
                var methodResult = home.method.call(receiver);
                var getter = Object.getOwnPropertyDescriptor(home, "read").get;
                var setter = Object.getOwnPropertyDescriptor(home, "write").set;
                var getterResult = getter.call(receiver);
                setter.call(receiver, 41);
                return [
                    methodResult,
                    getterResult,
                    receiver.saved,
                    log.join(",")
                ].join("|");
            })()
        "#,
        expected: "return|string|42|41|42|proto,key:method,key:read,key:write,method-body,get-body,set-body",
    },
    Case {
        group: "reference",
        description: "delete super property is a runtime ReferenceError",
        source: r#"
            (function () {
                var home = { remove() { return delete super.value; } };
                try { home.remove(); } catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|ReferenceError",
    },
    Case {
        group: "reference",
        description: "for-of assignment rotates the yielded value behind the super Reference",
        source: r#"
            (function () {
                var proto = { set value(input) { this.total += input; } };
                var home = {
                    __proto__: proto,
                    total: 0,
                    run() {
                        for (super.value of [20, 22]) {}
                        return this.total;
                    }
                };
                return home.run();
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "grammar",
        description: "direct super calls are forbidden in ObjectLiteral methods",
        source: "({ method() { super(); } })",
        expected: "throw|object|SyntaxError",
    },
];

#[test]
fn object_super_semantics_match_pinned_expectations() {
    for case in CASES {
        assert_eq!(
            rust_observation(case),
            case.expected,
            "Rust object-super behavior drifted for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn object_super_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-super oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    let quickjs = CASES
        .iter()
        .map(|case| oracle_observation(&oracle, case))
        .collect::<Vec<_>>();
    for (case, observation) in CASES.iter().zip(&quickjs) {
        assert_eq!(
            observation, case.expected,
            "pinned QuickJS object-super vector drifted for {} / {}: {:?}",
            case.group, case.description, case.source,
        );
    }
    for (case, observation) in CASES.iter().zip(&quickjs) {
        assert_eq!(
            rust_observation(case),
            *observation,
            "object-super behavior differed from pinned QuickJS for {} / {}: {:?}",
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
