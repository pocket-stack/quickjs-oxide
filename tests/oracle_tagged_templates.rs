use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{DebugInfoMode, Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the first tagged-template slice against QuickJS 2026-06-04. The
// observer sources return primitive strings so object identity, descriptors,
// UTF-16 text, and abrupt-completion ordering remain directly comparable.
const CASES: &[Case] = &[
    Case {
        group: "raw-cooked",
        description: "cooked escapes and physical CRLF normalization remain distinct from raw text",
        source: concat!(
            r#"(function () {
                function units(value) {
                    if (value === void 0) return "undefined";
                    var result = "";
                    for (var index = 0; index < value.length; index++) {
                        if (index) result += ",";
                        result += ("0000" + value.charCodeAt(index).toString(16)).slice(-4);
                    }
                    return value.length + ":" + result;
                }
                function tag(strings, value) {
                    return [
                        units(strings[0]),
                        units(strings[1]),
                        units(strings.raw[0]),
                        units(strings.raw[1]),
                        value
                    ].join("|");
                }
                return tag`a\n\x42\u0043"#,
            "\r\n",
            r#"${42}z\t`;
            })()"#,
        ),
        expected: concat!(
            "return|string|",
            "5:0061,000a,0042,0043,000a|",
            "2:007a,0009|",
            "14:0061,005c,006e,005c,0078,0034,0032,005c,0075,0030,0030,0034,0033,000a|",
            "3:007a,005c,0074|42",
        ),
    },
    Case {
        group: "invalid-escape",
        description: "a tagged malformed escape produces undefined cooked entries while preserving raw source",
        source: r#"
            (function () {
                function units(value) {
                    if (value === void 0) return "undefined";
                    var result = "";
                    for (var index = 0; index < value.length; index++) {
                        if (index) result += ",";
                        result += ("0000" + value.charCodeAt(index).toString(16)).slice(-4);
                    }
                    return value.length + ":" + result;
                }
                function tag(strings, value) {
                    return [
                        units(strings[0]),
                        units(strings[1]),
                        units(strings.raw[0]),
                        units(strings.raw[1]),
                        value
                    ].join("|");
                }
                return tag`\xG0${42}\u{110000}`;
            })()
        "#,
        expected: concat!(
            "return|string|undefined|undefined|",
            "4:005c,0078,0047,0030|",
            "10:005c,0075,007b,0031,0031,0030,0030,0030,0030,007d|42",
        ),
    },
    Case {
        group: "descriptors",
        description: "template and raw arrays have QuickJS frozen descriptors and realm Array prototypes",
        source: r#"
            (function () {
                function bits(descriptor) {
                    return [
                        typeof descriptor.value,
                        descriptor.writable,
                        descriptor.enumerable,
                        descriptor.configurable
                    ].join(":");
                }
                function tag(strings) {
                    return [
                        Object.isFrozen(strings),
                        Object.isFrozen(strings.raw),
                        Object.getPrototypeOf(strings) === Array.prototype,
                        Object.getPrototypeOf(strings.raw) === Array.prototype,
                        bits(Object.getOwnPropertyDescriptor(strings, "0")),
                        bits(Object.getOwnPropertyDescriptor(strings, "length")),
                        bits(Object.getOwnPropertyDescriptor(strings, "raw")),
                        bits(Object.getOwnPropertyDescriptor(strings.raw, "0")),
                        bits(Object.getOwnPropertyDescriptor(strings.raw, "length"))
                    ].join("|");
                }
                return tag`a${1}b`;
            })()
        "#,
        expected: concat!(
            "return|string|true|true|true|true|",
            "string:false:true:false|number:false:false:false|",
            "object:false:false:false|string:false:true:false|",
            "number:false:false:false",
        ),
    },
    Case {
        group: "same-site-identity",
        description: "repeated evaluation of one site reuses its template object and raw array",
        source: r#"
            (function () {
                var firstStrings;
                var firstRaw;
                var observations = [];
                function tag(strings, value) {
                    observations.push(firstStrings === void 0
                        ? "new"
                        : String(firstStrings === strings) + ":" + String(firstRaw === strings.raw));
                    if (firstStrings === void 0) {
                        firstStrings = strings;
                        firstRaw = strings.raw;
                    }
                    return value;
                }
                function site(value) { return tag`a${value}b`; }
                return [site(1), site(2), observations.join(",")].join("|");
            })()
        "#,
        expected: "return|string|1|2|new,true:true",
    },
    Case {
        group: "different-site-identity",
        description: "textually equal but distinct sites receive distinct template objects and raw arrays",
        source: r#"
            (function () {
                function tag(strings) { return strings; }
                var first = tag`same`;
                var second = tag`same`;
                return [
                    first !== second,
                    first.raw !== second.raw,
                    first[0] === second[0],
                    first.raw[0] === second.raw[0]
                ].join("|");
            })()
        "#,
        expected: "return|string|true|true|true|true",
    },
    Case {
        group: "member-this",
        description: "dot and computed member tags receive their base object as this",
        source: r#"
            (function () {
                var receiver = {
                    base: 40,
                    tag: function (strings, value) {
                        return [
                            this === receiver,
                            this.base + value,
                            strings[0],
                            strings[1]
                        ].join(":");
                    }
                };
                return [
                    receiver.tag`a${2}b`,
                    receiver["tag"]`c${2}d`
                ].join("|");
            })()
        "#,
        expected: "return|string|true:42:a:b|true:42:c:d",
    },
    Case {
        group: "evaluation-order",
        description: "member resolution precedes substitutions and tag invocation follows every substitution",
        source: r#"
            (function () {
                var log = "";
                var receiver = {
                    get tag() {
                        log += "g";
                        return function (strings, first, second) {
                            log += "t";
                            return [
                                this === receiver,
                                first + second,
                                strings[0],
                                strings[1],
                                strings[2]
                            ].join(":");
                        };
                    }
                };
                function base() { log += "b"; return receiver; }
                function one() { log += "1"; return 10; }
                function two() { log += "2"; return 20; }
                var result = base().tag`a${one()}b${two()}c`;
                return log + "|" + result;
            })()
        "#,
        expected: "return|string|bg12t|true:30:a:b:c",
    },
    Case {
        group: "abrupt-order",
        description: "getter and substitution failures stop later work while non-callable rejection follows arguments",
        source: r#"
            (function () {
                var observations = [];
                var log = "";
                var getterThrow = {
                    get tag() { log += "g"; throw "getter"; }
                };
                try { getterThrow.tag`${(log += "x")}`; }
                catch (error) { observations.push(log + ":" + error); }

                log = "";
                var substitutionThrow = {
                    get tag() {
                        log += "g";
                        return function () { log += "t"; };
                    }
                };
                function one() { log += "1"; throw "sub"; }
                function two() { log += "2"; return 2; }
                try { substitutionThrow.tag`${one()}${two()}`; }
                catch (error) { observations.push(log + ":" + error); }

                log = "";
                var nonCallable = {
                    get tag() { log += "g"; return 0; }
                };
                try {
                    nonCallable.tag`${(log += "1", 1)}${(log += "2", 2)}`;
                } catch (error) {
                    observations.push(log + ":" + error.name);
                }
                return observations.join("|");
            })()
        "#,
        expected: "return|string|g:getter|g1:sub|g12:TypeError",
    },
    Case {
        group: "new-precedence",
        description: "new constructs the result of the tagged call with the following argument list",
        source: r#"
            (function () {
                var log = "";
                var constructor;
                function tag(strings) {
                    log += "t" + strings[0];
                    constructor = function (value) {
                        log += "c";
                        this.value = value;
                    };
                    return constructor;
                }
                var instance = new tag`x`(42);
                return [
                    log,
                    instance.value,
                    instance instanceof constructor
                ].join("|");
            })()
        "#,
        expected: "return|string|txc|42|true",
    },
    Case {
        group: "chained-tags",
        description: "a tagged result can immediately tag the following template without a receiver",
        source: r#"
            (function () {
                var log = "";
                function first(strings) {
                    log += "f:" + strings[0];
                    return function (nextStrings) {
                        "use strict";
                        log += ",s:" + nextStrings[0] + ":" + (this === void 0);
                        return 42;
                    };
                }
                var value = first`a``b`;
                return value + "|" + log;
            })()
        "#,
        expected: "return|string|42|f:a,s:b:true",
    },
    Case {
        group: "with-receiver",
        description: "an identifier tag resolved through with receives the with object as this",
        source: r#"
            (function () {
                var result;
                var receiver = {
                    base: 40,
                    tag: function (strings, value) {
                        return [
                            this === receiver,
                            this.base + value,
                            strings[0],
                            strings[1]
                        ].join(":");
                    }
                };
                with (receiver) {
                    result = tag`a${2}b`;
                }
                return result;
            })()
        "#,
        expected: "return|string|true:42:a:b",
    },
    Case {
        group: "tagged-eval",
        description: "eval used as a tag is an ordinary call and returns its non-string template argument",
        source: r#"
            (function () {
                var marker = 0;
                var result = eval`marker = 42`;
                return [
                    typeof result,
                    Object.getPrototypeOf(result) === Array.prototype,
                    Object.isFrozen(result),
                    result[0],
                    marker
                ].join("|");
            })()
        "#,
        expected: "return|string|object|true|true|marker = 42|0",
    },
    Case {
        group: "closure-site-identity",
        description: "closures instantiated from one child bytecode share its tagged-template site",
        source: r#"
            (function () {
                function tag(strings) { return strings; }
                function factory() {
                    return function () { return tag`same`; };
                }
                var firstClosure = factory();
                var secondClosure = factory();
                var first = firstClosure();
                var second = secondClosure();
                return [
                    first === second,
                    first.raw === second.raw,
                    firstClosure !== secondClosure
                ].join("|");
            })()
        "#,
        expected: "return|string|true|true|true",
    },
    Case {
        group: "dynamic-compilation-sites",
        description: "authored eval and separately constructed Function bytecode own distinct sites",
        source: r#"
            (function () {
                function tag(strings) { return strings; }
                var authored = tag`same`;
                var evalFirst = eval("tag`same`");
                var evalSecond = eval("tag`same`");
                var firstFunction = Function("tag", "return tag`same`;");
                var secondFunction = Function("tag", "return tag`same`;");
                var functionFirst = firstFunction(tag);
                var functionFirstAgain = firstFunction(tag);
                var functionSecond = secondFunction(tag);
                return [
                    authored !== evalFirst,
                    evalFirst !== evalSecond,
                    authored !== functionFirst,
                    functionFirst === functionFirstAgain,
                    functionFirst !== functionSecond
                ].join("|");
            })()
        "#,
        expected: "return|string|true|true|true|true|true",
    },
    Case {
        group: "super-member-this",
        description: "super member tags and direct eval preserve the method receiver",
        source: r#"
            (function () {
                var prototype = {
                    tag: function (strings, value) {
                        return [
                            this.value + value,
                            strings[0],
                            strings[1]
                        ].join(":");
                    }
                };
                var object = {
                    value: 40,
                    __proto__: prototype,
                    run() {
                        return [
                            super.tag`a${2}b`,
                            eval("super.tag`c${2}d`")
                        ].join("|");
                    }
                };
                return object.run();
            })()
        "#,
        expected: "return|string|42:a:b|42:c:d",
    },
    Case {
        group: "newline-continuation",
        description: "a line terminator before the template does not trigger automatic semicolon insertion",
        source: r#"
            (function () {
                var calls = 0;
                function tag(strings) {
                    calls += 1;
                    return strings[0];
                }
                var value = tag
                `line`;
                return calls + "|" + value;
            })()
        "#,
        expected: "return|string|1|line",
    },
];

#[test]
fn tagged_template_rust_smoke_matches_pinned_expectation() {
    let case = &CASES[0];
    assert_eq!(
        rust_observation(case),
        case.expected,
        "Rust tagged-template smoke drifted for {} / {}: {:?}",
        case.group,
        case.description,
        case.source,
    );
}

#[test]
fn tagged_template_site_identity_survives_gc_in_strip_debug_mode() {
    let runtime = Runtime::new();
    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let mut context = runtime.new_context();
    context
        .eval(
            r#"
                function __qjoTaggedGcTag(strings) { return strings; }
                function __qjoTaggedGcSite() { return __qjoTaggedGcTag`alive`; }
            "#,
        )
        .expect("compile StripDebug tagged-template GC fixture");

    let Value::Object(first) = context
        .eval("__qjoTaggedGcSite()")
        .expect("evaluate tagged-template site before GC")
    else {
        panic!("tagged-template site did not return its template object before GC");
    };
    let raw_key = runtime
        .intern_property_key("raw")
        .expect("template raw property key");
    let Value::Object(first_raw) = context
        .get_property(&first, &raw_key)
        .expect("read template raw array before GC")
    else {
        panic!("template raw property was not an object before GC");
    };

    runtime.run_gc().expect("collect tagged-template fixture");

    let Value::Object(second) = context
        .eval("__qjoTaggedGcSite()")
        .expect("evaluate tagged-template site after GC")
    else {
        panic!("tagged-template site did not return its template object after GC");
    };
    let Value::Object(second_raw) = context
        .get_property(&second, &raw_key)
        .expect("read template raw array after GC")
    else {
        panic!("template raw property was not an object after GC");
    };

    assert_eq!(first, second, "template object identity changed across GC");
    assert_eq!(
        first_raw, second_raw,
        "template raw array identity changed across GC"
    );
}

#[test]
fn pinned_quickjs_tagged_template_semantics_match_expectations() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP tagged-template oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        assert_eq!(
            oracle_observation(&oracle, case),
            case.expected,
            "pinned QuickJS tagged-template vector drifted for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn tagged_template_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP tagged-template oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        let quickjs = oracle_observation(&oracle, case);
        assert_eq!(
            rust_observation(case),
            quickjs,
            "tagged-template behavior differed from pinned QuickJS for {} / {}: {:?}",
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
