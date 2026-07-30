use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins QuickJS 2026-06-04's optional-chain lowering before the Test262 feature
// is admitted globally. In addition to the ordinary successful paths, this
// table deliberately retains two upstream quirks:
//
// - an optional call of an identifier in a function containing `with` fails
//   QuickJS's bytecode stack verification with InternalError;
// - `delete value?.#private` is accepted, performs the private brand/read
//   check for non-nullish values, returns true, and leaves the element intact.
//
// Those cases are semantic target evidence, not assumptions derived from the
// ECMAScript specification. Every expected observation below was produced by
// the repository's pinned QuickJS 2026-06-04 oracle.
const CASES: &[Case] = &[
    Case {
        group: "basic member",
        description: "fixed and computed members return values while nullish bases skip the key",
        source: r#"
            (function () {
                function text(value) {
                    return value === void 0 ? "undefined" : String(value);
                }
                var conversions = 0;
                var key = {
                    toString: function () {
                        conversions++;
                        return "answer";
                    }
                };
                var object = { answer: 42 };
                return [
                    object?.answer,
                    object?.[key],
                    text(null?.answer),
                    text(undefined?.[key]),
                    conversions
                ].join("|");
            })()
        "#,
        expected: "return|string|42|42|undefined|undefined|1",
    },
    Case {
        group: "basic call",
        description: "bare member computed and chained optional calls select the intended boundary",
        source: r#"
            (function () {
                function text(value) {
                    return value === void 0 ? "undefined" : String(value);
                }
                function plain(value) {
                    return (this === globalThis) + ":" + value;
                }
                function factory() {
                    return { answer: 42 };
                }
                var object = {
                    base: 40,
                    method: function (value) {
                        return this.base + value;
                    }
                };
                var nil = null;
                return [
                    plain?.(42),
                    object?.method(2),
                    object.method?.(2),
                    object?.["method"]?.(2),
                    factory?.()?.answer,
                    text(nil?.method(2)),
                    text(nil?.(1)),
                    text(nil?.()?.answer)
                ].join("|");
            })()
        "#,
        expected: "return|string|true:42|42|42|42|42|undefined|undefined|undefined",
    },
    Case {
        group: "basic call",
        description: "an optional call still rejects a present non-callable value",
        source: r#"({ value: 1 }).value?.()"#,
        expected: "throw|object|TypeError",
    },
    Case {
        group: "basic call",
        description: "an optional base does not make a present method call optional",
        source: r#"({ method: 1 })?.method()"#,
        expected: "throw|object|TypeError",
    },
    Case {
        group: "short circuit",
        description: "nullish member and call boundaries skip keys arguments and the remaining chain",
        source: r#"
            (function () {
                var log = "";
                function base(value) {
                    log += "b";
                    return value;
                }
                function key() {
                    log += "k";
                    return "method";
                }
                function argument() {
                    log += "a";
                    return 2;
                }
                var first = base(null)?.[key()](argument());
                var second = base({ method: null })?.[key()]?.(argument());
                var third = base({
                    method: function (value) {
                        log += "c";
                        return value + 40;
                    }
                })?.[key()]?.(argument());
                return [String(first), String(second), third, log].join("|");
            })()
        "#,
        expected: "return|string|undefined|undefined|42|bbkbkac",
    },
    Case {
        group: "short circuit",
        description: "the base is evaluated once and only traversed getters execute",
        source: r#"
            (function () {
                var baseCalls = 0;
                var gets = 0;
                function base() {
                    baseCalls++;
                    return {
                        get child() {
                            gets++;
                            return { answer: 42 };
                        }
                    };
                }
                var answer = base()?.child.answer;
                var skipped = (function () {
                    baseCalls++;
                    return null;
                })()?.child.answer;
                return [answer, String(skipped), baseCalls, gets].join("|");
            })()
        "#,
        expected: "return|string|42|undefined|2|1",
    },
    Case {
        group: "chain extent",
        description: "one optional boundary covers its tail while grouping and a nonoptional null tail throw",
        source: r#"
            (function () {
                var first = null?.child.answer.value;
                var second = ({ child: null })?.child?.answer.value;
                var third = ({ child: { answer: { value: 42 } } })?.child.answer?.value;
                var groupedError;
                var tailError;
                try {
                    (null?.child).answer;
                } catch (error) {
                    groupedError = error.name;
                }
                try {
                    ({ child: null })?.child.answer;
                } catch (error) {
                    tailError = error.name;
                }
                return [
                    String(first),
                    String(second),
                    third,
                    groupedError,
                    tailError
                ].join("|");
            })()
        "#,
        expected: "return|string|undefined|undefined|42|TypeError|TypeError",
    },
    Case {
        group: "receiver",
        description: "dot computed optional-base and grouped calls retain the member receiver",
        source: r#"
            (function () {
                var object = {
                    base: 40,
                    method: function (value) {
                        return (this === object) + ":" + (this.base + value);
                    }
                };
                var conversions = 0;
                var key = {
                    toString: function () {
                        conversions++;
                        return "method";
                    }
                };
                return [
                    object.method?.(2),
                    object?.method(2),
                    object?.[key]?.(2),
                    (object?.method)(2),
                    conversions
                ].join("|");
            })()
        "#,
        expected: "return|string|true:42|true:42|true:42|true:42|1",
    },
    Case {
        group: "receiver",
        description: "nullish grouped fixed spread and multi-edge calls evaluate arguments then throw",
        source: r#"
            (function () {
                var log = "";
                var errors = [];
                try {
                    (null?.method)((log += "f", 1));
                } catch (error) {
                    errors.push(error.name);
                }
                try {
                    (null?.method)(...(log += "s", [1]));
                } catch (error) {
                    errors.push(error.name);
                }
                try {
                    (({ child: null })?.child?.method)((log += "m", 1));
                } catch (error) {
                    errors.push(error.name);
                }
                return errors.join(",") + "|" + log;
            })()
        "#,
        expected: "return|string|TypeError,TypeError,TypeError|fsm",
    },
    Case {
        group: "QuickJS with quirk",
        description: "an optional call of a with-resolved identifier hits QuickJS stack verification",
        source: r#"
            (function () {
                var scope = {
                    base: 40,
                    method: function (value) {
                        return (this === scope) + ":" + (this.base + value);
                    }
                };
                var result;
                with (scope) {
                    result = method?.(2);
                }
                return result;
            })()
        "#,
        expected: "throw|object|InternalError",
    },
    Case {
        group: "receiver",
        description: "QuickJS reads optional super getters with the prototype and calls with the live receiver",
        source: r#"
            (function () {
                var getterReceivers = [];
                var receiver;
                var proto = {
                    get method() {
                        getterReceivers.push(this === proto ? "proto" : "other");
                        return function (value) {
                            return (this.base + value) + ":" + (this === receiver);
                        };
                    }
                };
                var home = {
                    __proto__: proto,
                    dot(value) {
                        return super.method?.(value);
                    },
                    computed(value) {
                        return super["method"]?.(value);
                    },
                    missing(side) {
                        return super.absent?.(side());
                    }
                };
                receiver = { base: 40 };
                var sideCalls = 0;
                var missing = home.missing.call(receiver, function () {
                    sideCalls++;
                    return 1;
                });
                return [
                    home.dot.call(receiver, 2),
                    home.computed.call(receiver, 2),
                    String(missing),
                    sideCalls,
                    getterReceivers.join(",")
                ].join("|");
            })()
        "#,
        expected: "return|string|42:true|42:true|undefined|0|proto,proto",
    },
    Case {
        group: "eval",
        description: "eval optional call is indirect and cannot see the caller lexical environment",
        source: r#"
            (function () {
                var __qjo_optional_local = 41;
                var direct = eval("__qjo_optional_local + 1");
                var indirect = eval?.("typeof __qjo_optional_local");
                return [direct, indirect, __qjo_optional_local].join("|");
            })()
        "#,
        expected: "return|string|42|undefined|41",
    },
    Case {
        group: "eval",
        description: "a shadowed eval remains an ordinary optional call and skips a nullish argument",
        source: r#"
            (function () {
                var calls = 0;
                var eval = function (source) {
                    "use strict";
                    calls++;
                    return (this === void 0) + ":" + source;
                };
                var first = eval?.("shadow");
                eval = null;
                var second = eval?.((calls++, "skipped"));
                return [first, String(second), calls].join("|");
            })()
        "#,
        expected: "return|string|true:shadow|undefined|1",
    },
    Case {
        group: "delete",
        description: "delete short circuits nullish keys and preserves sloppy and strict property rules",
        source: r#"
            (function () {
                var log = "";
                var object = { answer: 42 };
                var fixed = {};
                Object.defineProperty(fixed, "answer", {
                    value: 42,
                    configurable: false
                });
                var skipped = delete null?.[log += "key"];
                var removed = delete object?.answer;
                var rejected = delete fixed?.answer;
                var strictDelete = Function(
                    "value",
                    "\"use strict\"; return delete value?.answer;"
                );
                var strictError;
                try {
                    strictDelete(fixed);
                } catch (error) {
                    strictError = error.name;
                }
                return [
                    skipped,
                    removed,
                    Object.hasOwn(object, "answer"),
                    rejected,
                    strictDelete(null),
                    strictError,
                    log
                ].join("|");
            })()
        "#,
        expected: "return|string|true|true|false|false|true|TypeError|",
    },
    Case {
        group: "grouped boundary",
        description: "grouping ends the optional chain so its result can be constructed",
        source: r#"
            (function () {
                function Constructor(value) {
                    this.answer = value;
                }
                var holder = { Constructor: Constructor };
                return new (holder?.Constructor)(42).answer;
            })()
        "#,
        expected: "return|number|42",
    },
    Case {
        group: "grouped boundary",
        description: "grouping ends the optional chain while retaining a member tag receiver",
        source: r#"
            (function () {
                var holder = {
                    tag: function (strings, value) {
                        return [
                            this === holder,
                            strings[0],
                            value,
                            strings[1]
                        ].join(":");
                    }
                };
                return (holder?.tag)`a${42}b`;
            })()
        "#,
        expected: "return|string|true:a:42:b",
    },
    Case {
        group: "special base",
        description: "new.target can be the base of an optional member chain",
        source: r#"
            (function () {
                function Probe() {
                    if (new.target === void 0) {
                        return String(new.target?.name);
                    }
                    this.seen = new.target?.name;
                }
                return [Probe(), new Probe().seen].join("|");
            })()
        "#,
        expected: "return|string|undefined|Probe",
    },
    Case {
        group: "async",
        description: "a nullish optional call skips an await argument while a callable path suspends",
        source: r#"
            (function () {
                var log = "";
                async function probe(callback) {
                    var value = callback?.(await (log += "a", 40));
                    log += "r";
                    return value;
                }
                var skipped = probe(null);
                var afterSkipped = log;
                log = "";
                var called = probe(function (value) {
                    log += "c";
                    return value + 2;
                });
                var afterCalled = log;
                return [
                    afterSkipped,
                    skipped instanceof Promise,
                    afterCalled,
                    called instanceof Promise
                ].join("|");
            })()
        "#,
        expected: "return|string|r|true|a|true",
    },
    Case {
        group: "generator",
        description: "a nullish optional call skips a yield argument while a callable path resumes it",
        source: r#"
            (function () {
                function* probe(callback) {
                    return callback?.(yield 40);
                }
                var skipped = probe(null).next();
                var iterator = probe(function (value) {
                    return value + 2;
                });
                var first = iterator.next();
                var second = iterator.next(40);
                return [
                    skipped.done,
                    String(skipped.value),
                    first.done,
                    first.value,
                    second.done,
                    second.value
                ].join("|");
            })()
        "#,
        expected: "return|string|true|undefined|false|40|true|42",
    },
    Case {
        group: "private element",
        description: "optional private fields and methods short circuit before brand checks on nullish bases",
        source: r#"
            (function () {
                class Box {
                    #value = 40;
                    #add(value) {
                        return this.#value + value;
                    }
                    static read(value) {
                        return value?.#value;
                    }
                    static call(value) {
                        return value?.#add?.(2);
                    }
                }
                var box = new Box();
                var brandError;
                try {
                    Box.read({});
                } catch (error) {
                    brandError = error.name;
                }
                return [
                    Box.read(box),
                    String(Box.read(null)),
                    String(Box.read(void 0)),
                    Box.call(box),
                    String(Box.call(null)),
                    brandError
                ].join("|");
            })()
        "#,
        expected: "return|string|40|undefined|undefined|42|undefined|TypeError",
    },
    Case {
        group: "private element",
        description: "a private field after an earlier optional link remains inside the same chain",
        source: r#"
            (function () {
                class Box {
                    #value = 42;
                    read(value) {
                        return value?.holder.#value;
                    }
                }
                var box = new Box();
                var nullHolderError;
                var wrongBrandError;
                try {
                    box.read({ holder: null });
                } catch (error) {
                    nullHolderError = error.name;
                }
                try {
                    box.read({ holder: {} });
                } catch (error) {
                    wrongBrandError = error.name;
                }
                return [
                    box.read({ holder: box }),
                    String(box.read(null)),
                    nullHolderError,
                    wrongBrandError
                ].join("|");
            })()
        "#,
        expected: "return|string|42|undefined|TypeError|TypeError",
    },
    Case {
        group: "QuickJS private quirk",
        description: "grouping an optional private method loses the receiver before fixed or optional call",
        source: r#"
            (function () {
                class Box {
                    #receiver() {
                        return this === void 0 ? "undefined" : "instance";
                    }
                    test() {
                        return [
                            this?.#receiver(),
                            (this?.#receiver)(),
                            (this?.#receiver)?.()
                        ].join("|");
                    }
                }
                return new Box().test();
            })()
        "#,
        expected: "return|string|instance|undefined|undefined",
    },
    Case {
        group: "QuickJS private quirk",
        description: "delete of an optional private access returns true without deleting the private field",
        source: r#"
            (function () {
                class Box {
                    #value = 42;
                    test(value) {
                        var result = delete value?.#value;
                        var after;
                        try {
                            after = value?.#value;
                        } catch (error) {
                            after = error.name;
                        }
                        return result + ":" + after;
                    }
                }
                var box = new Box();
                var wrongBrand;
                try {
                    box.test({});
                } catch (error) {
                    wrongBrand = error.name;
                }
                return [
                    box.test(box),
                    box.test(null),
                    wrongBrand
                ].join("|");
            })()
        "#,
        expected: "return|string|true:42|true:undefined|TypeError",
    },
    Case {
        group: "early error assignment",
        description: "an optional member cannot be a simple assignment target",
        source: r#"var object = {}; object?.answer = 42;"#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error assignment",
        description: "an optional member cannot be a compound assignment target",
        source: r#"var object = { answer: 1 }; object?.answer += 1;"#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error assignment",
        description: "an optional member cannot be a logical assignment target",
        source: r#"var object = { answer: 0 }; object?.answer ||= 1;"#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error destructuring",
        description: "an optional member cannot be an array destructuring assignment target",
        source: r#"
            var object = {};
            var values = [42];
            [object?.answer] = values;
        "#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error iteration",
        description: "an optional member cannot be a for-in assignment target",
        source: r#"
            var object = {};
            var source = { answer: 42 };
            for (object?.answer in source) {}
        "#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error iteration",
        description: "an optional member cannot be a for-of assignment target",
        source: r#"
            var object = {};
            var source = [42];
            for (object?.answer of source) {}
        "#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error update",
        description: "an optional member cannot be a postfix update target",
        source: r#"var object = {}; object?.answer++;"#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error update",
        description: "an optional member cannot be a prefix update target",
        source: r#"var object = {}; ++object?.answer;"#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error new",
        description: "new cannot directly construct an optional member chain",
        source: r#"
            var holder = { Constructor: function () {} };
            new holder?.Constructor();
        "#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error new",
        description: "new cannot directly construct an optional call chain",
        source: r#"
            var Constructor = function () {};
            new Constructor?.();
        "#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error template",
        description: "a tagged template cannot directly follow an optional member chain",
        source: r#"
            var holder = { tag: function () {} };
            holder?.tag`value`;
        "#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error template",
        description: "an optional tag call cannot use tagged-template syntax",
        source: r#"
            var tag = function () {};
            tag?.`value`;
        "#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error super",
        description: "super itself cannot be the base of an optional member chain",
        source: r#"
            class Base {}
            class Derived extends Base {
                method() {
                    return super?.value;
                }
            }
        "#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error private",
        description: "delete of an ordinary private access remains a syntax error",
        source: r#"
            var optional = null?.value;
            class Box {
                #value;
                remove(value) {
                    return delete value.#value;
                }
            }
        "#,
        expected: "throw|object|SyntaxError",
    },
    Case {
        group: "early error private",
        description: "an optional private name must still be declared by an enclosing class",
        source: r#"var value = {}; value?.#missing;"#,
        expected: "throw|object|SyntaxError",
    },
];

#[test]
fn optional_chaining_case_table_is_reviewed() {
    assert_eq!(CASES.len(), 38, "update the reviewed oracle case count");
    for (index, case) in CASES.iter().enumerate() {
        assert!(
            case.source.contains("?."),
            "case lacks an optional chain: {}",
            case.description,
        );
        assert!(
            CASES[..index]
                .iter()
                .all(|earlier| earlier.description != case.description),
            "duplicate case description: {}",
            case.description,
        );
    }
}

#[test]
fn optional_chaining_matches_pinned_expectations_without_an_oracle() {
    let mut failures = Vec::new();
    for case in CASES {
        let actual = rust_observation(case);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\noxide: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "optional-chaining pinned expectations failed in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn optional_chaining_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP optional-chaining oracle self-check: set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };

    let mut failures = Vec::new();
    for case in CASES {
        let actual = oracle_observation(&oracle, case);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\nactual: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "pinned QuickJS optional-chaining vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn optional_chaining_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP optional-chaining differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    let mut failures = Vec::new();
    for case in CASES {
        let oxide = rust_observation(case);
        let quickjs = oracle_observation(&oracle, case);
        if oxide != quickjs {
            failures.push(format!(
                "{} / {}\nsource: {:?}\noxide: {:?}\nquickjs: {:?}",
                case.group, case.description, case.source, oxide, quickjs,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "optional-chaining semantics drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn rust_observation(case: &Case) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    match context.eval(case.source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(&runtime, &value),
            primitive_text(value),
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
                    primitive_text(value),
                ),
            }
        }
        Err(error) => format!("engine|{error}"),
    }
}

fn oracle_observation(oracle: &OsStr, case: &Case) -> String {
    let wrapper = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print("return|" + typeof value + "|" + String(value));
} catch (error) {
  if (error !== null && typeof error === "object")
    print("throw|object|" + error.name);
  else
    print("throw|" + typeof error + "|" + String(error));
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

fn error_name(
    runtime: &Runtime,
    context: &mut Context,
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
