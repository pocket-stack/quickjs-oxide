use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins QuickJS 2026-06-04's `OP_append`, `OP_apply`, and `OP_apply_eval`
// pipeline. In particular, argument spread is not just generic IteratorStep:
// upstream performs a redundant first @@iterator Get and has a
// representation-sensitive fast-Array branch after creating the second
// iterator record. Keep those quirks explicit rather than normalizing this
// target to specification-only behavior.
const CASES: &[Case] = &[
    Case {
        group: "lowering",
        description: "empty and interleaved spreads preserve fixed tails and a trailing comma",
        source: r#"
            (function () {
                function list() {
                    return Array.prototype.join.call(arguments, ",");
                }
                return list(...[], 1, ...[2, 3], 4, ...[5],);
            })()
        "#,
        expected: "return|string|1,2,3,4,5",
    },
    Case {
        group: "method",
        description: "method receiver and callee then argument evaluation order survive spread lowering",
        source: r#"
            (function () {
                var log = [];
                function mark(label, value) { log.push(label); return value; }
                var object = { base: 10 };
                Object.defineProperty(object, "method", {
                    get: function () {
                        log.push("get");
                        return function () {
                            "use strict";
                            return (this === object) + "|" + this.base + "|" +
                                Array.prototype.join.call(arguments, ",") + "|" +
                                log.join(",");
                        };
                    }
                });
                var receiver = mark("base", object);
                return receiver.method(mark("a", 1), ...mark("spread", [2, 3]), mark("z", 4));
            })()
        "#,
        expected: "return|string|true|10|1,2,3,4|base,get,a,spread,z",
    },
    Case {
        group: "QuickJS append",
        description: "argument spread performs QuickJS's two observable iterator method Gets",
        source: r#"
            (function () {
                var gets = 0;
                var calls = 0;
                var iterable = {};
                Object.defineProperty(iterable, Symbol.iterator, {
                    get: function () {
                        gets++;
                        return function () {
                            calls++;
                            var done = false;
                            return {
                                next: function () {
                                    if (done) return { done: true };
                                    done = true;
                                    return { value: 7, done: false };
                                }
                            };
                        };
                    }
                });
                function identity(value) { return value; }
                return identity(...iterable) + "|" + gets + "|" + calls;
            })()
        "#,
        expected: "return|string|7|2|1",
    },
    Case {
        group: "QuickJS append",
        description: "fast Array copies its source after the second iterator Get selects another iterator",
        source: r#"
            (function () {
                var source = [1, 2];
                var gets = 0;
                Object.defineProperty(source, Symbol.iterator, {
                    configurable: true,
                    get: function () {
                        gets++;
                        if (gets === 1) return Array.prototype.values;
                        return function () { return [99].values(); };
                    }
                });
                function list() {
                    return Array.prototype.join.call(arguments, ",");
                }
                return list(...source) + "|" + gets;
            })()
        "#,
        expected: "return|string|1,2|2",
    },
    Case {
        group: "call",
        description: "a bare spread call supplies undefined to strict this",
        source: r#"(function () { "use strict"; return this === undefined; })(...[])"#,
        expected: "return|boolean|true",
    },
    Case {
        group: "iterator",
        description: "the iterator next method is fetched once and cached across all spread steps",
        source: r#"
            (function () {
                var gets = 0;
                var calls = 0;
                var iterable = {};
                iterable[Symbol.iterator] = function () {
                    var iterator = { index: 0 };
                    Object.defineProperty(iterator, "next", {
                        configurable: true,
                        get: function () {
                            gets++;
                            return function () {
                                calls++;
                                this.index++;
                                if (this.index === 1) {
                                    Object.defineProperty(this, "next", {
                                        configurable: true,
                                        value: function () { throw "replacement next"; }
                                    });
                                }
                                if (this.index <= 2) return { value: this.index, done: false };
                                return { done: true };
                            };
                        }
                    });
                    return iterator;
                };
                function list() {
                    return Array.prototype.join.call(arguments, ",");
                }
                return list(...iterable) + "|" + gets + "|" + calls;
            })()
        "#,
        expected: "return|string|1,2|1|3",
    },
    Case {
        group: "iterator",
        description: "a next throw closes the iterator while a close throw cannot replace it",
        source: r#"
            (function () {
                var log = "";
                var iterable = {};
                iterable[Symbol.iterator] = function () {
                    return {
                        next: function () { log += "n"; throw "next throw"; },
                        return: function () { log += "r"; throw "close throw"; }
                    };
                };
                try {
                    (function () {})(...iterable);
                } catch (error) {
                    return error + "|" + log;
                }
                return "no throw";
            })()
        "#,
        expected: "return|string|next throw|nr",
    },
    Case {
        group: "construct",
        description: "construct spread preserves arguments receiver prototype and new.target",
        source: r#"
            (function () {
                function Constructor(a, b) {
                    this.argumentsSeen = a + ":" + b;
                    this.newTargetSeen = new.target === Constructor;
                }
                var value = new Constructor(...[20], 22);
                return value.argumentsSeen + "|" + value.newTargetSeen + "|" +
                    (value instanceof Constructor);
            })()
        "#,
        expected: "return|string|20:22|true|true",
    },
    Case {
        group: "eval",
        description: "direct eval accepts an empty leading spread and ignores later arguments",
        source: r#"
            (function () {
                function run() {
                    let value = 1;
                    var result = eval(...[], "value = 42; 'direct'", ...["ignored"]);
                    return result + "|" + value;
                }
                return run();
            })()
        "#,
        expected: "return|string|direct|42",
    },
    Case {
        group: "eval",
        description: "a shadowed eval receives every spread argument with undefined strict this",
        source: r#"
            (function () {
                function run(eval) { return eval(...[1, 2], 3); }
                return run(function () {
                    "use strict";
                    return (this === undefined) + "|" +
                        Array.prototype.join.call(arguments, ",");
                });
            })()
        "#,
        expected: "return|string|true|1,2,3",
    },
    Case {
        group: "eval",
        description: "an indirect eval spread remains global rather than capturing the caller lexical",
        source: r#"
            (function () {
                globalThis.argumentSpreadIndirect = 1;
                let argumentSpreadIndirect = 2;
                (0, eval)(...["globalThis.argumentSpreadIndirect = 42"]);
                return argumentSpreadIndirect + "|" + globalThis.argumentSpreadIndirect;
            })()
        "#,
        expected: "return|string|2|42",
    },
    Case {
        group: "values",
        description: "sparse Arrays materialize undefined and String spread advances by Unicode code point",
        source: r#"
            (function () {
                function inspect() {
                    return arguments.length + "|" + typeof arguments[0] + "|" +
                        arguments[1] + "|" + arguments[2] + "|" +
                        arguments[3].length + "|" + arguments[3].charCodeAt(0) + "|" +
                        arguments[4].length + "|" + arguments[4].charCodeAt(0);
                }
                return inspect(...[, 2], ..."A\uD83D\uDCA9\uD800");
            })()
        "#,
        expected: "return|string|5|undefined|2|A|2|55357|1|55296",
    },
    Case {
        group: "error order",
        description: "a non-callable target is rejected only after its spread iterator completes",
        source: r#"
            (function () {
                var log = "";
                var iterable = {};
                iterable[Symbol.iterator] = function () {
                    var index = 0;
                    return {
                        next: function () {
                            log += "n";
                            index++;
                            return index === 1 ? { value: 1, done: false } : { done: true };
                        }
                    };
                };
                try { ({ })(...iterable); } catch (error) { return error.name + "|" + log; }
                return "no throw";
            })()
        "#,
        expected: "return|string|TypeError|nn",
    },
    Case {
        group: "error order",
        description: "an iterator acquisition throw wins before non-callability is tested",
        source: r#"
            (function () {
                var log = "";
                var iterable = {};
                Object.defineProperty(iterable, Symbol.iterator, {
                    get: function () { log += "g"; throw "spread throw"; }
                });
                try { ({ })(...iterable); } catch (error) { return error + "|" + log; }
                return "no throw";
            })()
        "#,
        expected: "return|string|spread throw|g",
    },
    Case {
        group: "error order",
        description: "a callable non-constructor is rejected after its spread iterator completes",
        source: r#"
            (function () {
                var log = "";
                var iterable = {};
                iterable[Symbol.iterator] = function () {
                    var index = 0;
                    return {
                        next: function () {
                            log += "n";
                            index++;
                            return index === 1 ? { value: 1, done: false } : { done: true };
                        }
                    };
                };
                var arrow = () => 0;
                try { new arrow(...iterable); } catch (error) { return error.name + "|" + log; }
                return "no throw";
            })()
        "#,
        expected: "return|string|TypeError|nn",
    },
];

// These vectors allocate the largest temporary argument Arrays accepted by
// QuickJS. Keep them out of the ordinary expectation/differential loops: the
// immutable-shape representation currently makes growing a 65K dense Array
// quadratic. The shared `build_arg_list` limit remains a fast mandatory oracle
// in `oracle_function_apply`; this end-to-end stress test stays available for
// manual runs until dense bulk growth lands.
const BOUNDARY_CASES: &[Case] = &[
    Case {
        group: "argument limit",
        description: "the QuickJS runtime accepts exactly 65534 spread arguments",
        source: r#"
            (function () {
                return Boolean(...Array(65534).fill(0));
            })()
        "#,
        expected: "return|boolean|false",
    },
    Case {
        group: "argument limit",
        description: "the QuickJS runtime rejects 65535 spread arguments",
        source: r#"
            (function () {
                try { return Boolean(...Array(65535).fill(0)); }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|RangeError",
    },
    Case {
        group: "error order",
        description: "the 65535 argument RangeError precedes constructability validation",
        source: r#"
            (function () {
                var arrow = () => 0;
                try { return new arrow(...Array(65535).fill(0)); }
                catch (error) { return error.name; }
            })()
        "#,
        expected: "return|string|RangeError",
    },
];

#[test]
fn argument_spread_oracle_inventory_is_stable() {
    assert_eq!(CASES.len(), 15, "update the reviewed ordinary case count");
    assert_eq!(
        BOUNDARY_CASES.len(),
        3,
        "update the reviewed boundary case count"
    );
    let cases = CASES.iter().chain(BOUNDARY_CASES).collect::<Vec<_>>();
    for (index, case) in cases.iter().enumerate() {
        assert!(
            case.source.contains("..."),
            "argument-spread case lacks an ellipsis: {}",
            case.description,
        );
        assert!(
            cases[..index]
                .iter()
                .all(|earlier| earlier.description != case.description),
            "duplicate case description: {}",
            case.description,
        );
    }
}

#[test]
fn argument_spread_matches_pinned_expectations() {
    let mut failures = Vec::new();
    for case in CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let actual = observe_rust(&runtime, &mut context, case.source, case.description);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\noxide: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "argument-spread pinned expectations failed in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn argument_spread_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP argument-spread oracle self-check: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for case in CASES.iter().chain(BOUNDARY_CASES) {
        let actual = observe_oracle(&oracle, case.source, case.description);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\nactual: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "pinned QuickJS argument-spread vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn argument_spread_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP argument-spread differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for case in CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let oxide = observe_rust(&runtime, &mut context, case.source, case.description);
        let quickjs = observe_oracle(&oracle, case.source, case.description);
        if oxide != quickjs {
            failures.push(format!(
                "{} / {}\nsource: {:?}\noxide: {:?}\nquickjs: {:?}",
                case.group, case.description, case.source, oxide, quickjs,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "argument-spread semantics drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
#[ignore = "65K temporary Array growth is quadratic; run manually after dense bulk growth lands"]
fn argument_spread_runtime_argument_limit_matches_pinned_quickjs() {
    let oracle = std::env::var_os("QJS_ORACLE");
    let mut failures = Vec::new();
    for case in BOUNDARY_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let oxide = observe_rust(&runtime, &mut context, case.source, case.description);
        if oxide != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\noxide: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, oxide, case.expected,
            ));
        }
        if let Some(oracle) = oracle.as_deref() {
            let quickjs = observe_oracle(oracle, case.source, case.description);
            if oxide != quickjs {
                failures.push(format!(
                    "{} / {}\nsource: {:?}\noxide: {:?}\nquickjs: {:?}",
                    case.group, case.description, case.source, oxide, quickjs,
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "argument-spread runtime limits drifted in {} observation(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn observe_rust(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_text(value),
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    error_string_property(runtime, context, &error, "name", description),
                    error_string_property(runtime, context, &error, "message", description),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_text(value),
                ),
            }
        }
        Err(error) => format!("engine|{error}"),
    }
}

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
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
        .args(["--std", "-e", wrapper, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS observer failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"))
        .trim_end()
        .to_owned()
}

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &quickjs_oxide::ObjectRef,
    name: &str,
    description: &str,
) -> String {
    let key = runtime
        .intern_property_key(name)
        .expect("Error property key");
    let Value::String(value) = context
        .get_property(error, &key)
        .unwrap_or_else(|failure| panic!("read Error.{name} for {description}: {failure}"))
    else {
        panic!("Error.{name} was not a string for {description}");
    };
    value.to_utf8_lossy()
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

fn primitive_text(value: Value) -> String {
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
