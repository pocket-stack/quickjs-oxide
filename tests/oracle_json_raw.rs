use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins QuickJS 2026-06-04's `js_json_rawJSON`, `js_json_isRawJSON`, and the
// `JS_CLASS_RAWJSON` branch in `js_json_to_str`. Every observer returns an
// ASCII primitive string so exact UTF-16 cases can cross the qjs process
// boundary without encoding an unpaired surrogate.
const CASES: &[Case] = &[
    Case {
        group: "global-graph",
        description: "Raw JSON methods expose the pinned JSON table and function metadata",
        source: r#"
            (function () {
                function bit(value) { return value ? 1 : 0; }
                function bits(descriptor) {
                    return "" + bit(descriptor.writable) +
                        bit(descriptor.enumerable) + bit(descriptor.configurable);
                }
                function isConstructor(value) {
                    try {
                        Reflect.construct(function () {}, [], value);
                        return true;
                    } catch (_) {
                        return false;
                    }
                }
                function method(name) {
                    var value = JSON[name];
                    var propertyDescriptor = Object.getOwnPropertyDescriptor(JSON, name);
                    var nameDescriptor = Object.getOwnPropertyDescriptor(value, "name");
                    var lengthDescriptor = Object.getOwnPropertyDescriptor(value, "length");
                    return [
                        name,
                        propertyDescriptor.value === value,
                        bits(propertyDescriptor),
                        value.name,
                        value.length,
                        bits(nameDescriptor),
                        bits(lengthDescriptor),
                        Object.getOwnPropertyNames(value).join(","),
                        Object.getPrototypeOf(value) === Function.prototype,
                        typeof value,
                        isConstructor(value)
                    ].join(":");
                }
                return [
                    Reflect.ownKeys(JSON).map(String).join(","),
                    method("isRawJSON"),
                    method("rawJSON")
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|isRawJSON,parse,rawJSON,stringify,Symbol(Symbol.toStringTag)|",
            "isRawJSON:true:101:isRawJSON:1:001:001:length,name:true:function:false|",
            "rawJSON:true:101:rawJSON:1:001:001:length,name:true:function:false",
        ),
    },
    Case {
        group: "returned-object",
        description: "rawJSON returns a null prototype frozen branded ordinary object",
        source: r#"
            (function () {
                function bit(value) { return value ? 1 : 0; }
                function bits(descriptor) {
                    return "" + bit(descriptor.writable) +
                        bit(descriptor.enumerable) + bit(descriptor.configurable);
                }
                var value = JSON.rawJSON("1");
                var descriptor = Object.getOwnPropertyDescriptor(value, "rawJSON");
                return [
                    Object.getPrototypeOf(value) === null,
                    Object.prototype.toString.call(value),
                    Object.getOwnPropertyNames(value).join(","),
                    Object.getOwnPropertySymbols(value).length,
                    Object.keys(value).join(","),
                    descriptor.value,
                    bits(descriptor),
                    Object.isExtensible(value),
                    Object.isSealed(value),
                    Object.isFrozen(value),
                    JSON.isRawJSON(value),
                    value instanceof Object
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|true|[object Object]|rawJSON|0|rawJSON|1|010|",
            "false|true|true|true|false",
        ),
    },
    Case {
        group: "tostring-order",
        description: "ToString uses string hint and propagates abrupt completion before validation",
        source: r#"
            (function () {
                var log = [];
                var exotic = {};
                exotic[Symbol.toPrimitive] = function (hint) {
                    log.push("p:" + hint);
                    return "1.20e+2";
                };
                var exoticText = JSON.rawJSON(exotic).rawJSON;

                var ordinary = {
                    toString: function () { log.push("t"); return {}; },
                    valueOf: function () { log.push("v"); return "false"; }
                };
                var ordinaryText = JSON.rawJSON(ordinary).rawJSON;

                var sentinel = {};
                var abrupt = {};
                abrupt[Symbol.toPrimitive] = function () {
                    log.push("a");
                    throw sentinel;
                };
                var abruptCaught = false;
                try { JSON.rawJSON(abrupt); }
                catch (error) { abruptCaught = error === sentinel; }

                var invalidError;
                try {
                    JSON.rawJSON({
                        toString: function () { log.push("i"); return "[]"; }
                    });
                } catch (error) {
                    invalidError = error.name + ":" + error.message;
                }

                var symbolError;
                try { JSON.rawJSON(Symbol("text")); }
                catch (error) { symbolError = error.name + ":" + error.message; }

                var missingError;
                try { JSON.rawJSON(); }
                catch (error) { missingError = error.name + ":" + error.message; }

                return [
                    exoticText,
                    ordinaryText,
                    abruptCaught,
                    invalidError,
                    symbolError,
                    missingError,
                    log.join(",")
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|1.20e+2|false|true|",
            "SyntaxError:invalid rawJSON string|",
            "TypeError:cannot convert symbol to string|",
            "SyntaxError:invalid rawJSON string|p:string,t,v,a,i",
        ),
    },
    Case {
        group: "accepted-source",
        description: "valid primitive texts retain exact spelling while non-string inputs canonicalize",
        source: r#"
            (function () {
                function units(value) {
                    var result = [];
                    for (var index = 0; index < value.length; index++) {
                        result.push(("0000" + value.charCodeAt(index).toString(16)).slice(-4));
                    }
                    return value.length + ":" + result.join(",");
                }
                var texts = [
                    "null",
                    "true",
                    "false",
                    "0",
                    "-0",
                    "1.2300",
                    "1e+9",
                    '"foo"',
                    '"\\u0061"',
                    '"a b"'
                ];
                var exact = true;
                var serialized = [];
                for (var index = 0; index < texts.length; index++) {
                    var raw = JSON.rawJSON(texts[index]);
                    exact = exact && raw.rawJSON === texts[index];
                    serialized.push(JSON.stringify(raw));
                }

                var literalLoneSurrogate = '"' + String.fromCharCode(0xd800) + '"';
                var lone = JSON.rawJSON(literalLoneSurrogate);
                return [
                    exact,
                    serialized.join(","),
                    JSON.rawJSON(-0).rawJSON,
                    JSON.rawJSON(1.1e1).rawJSON,
                    JSON.rawJSON(1.1e-1).rawJSON,
                    JSON.rawJSON(123456789012345678901234567890n).rawJSON,
                    units(lone.rawJSON),
                    units(JSON.stringify(lone))
                ].join("|");
            })()
        "#,
        expected: concat!(
            r#"return|string|true|null,true,false,0,-0,1.2300,1e+9,"foo","\u0061","a b"|"#,
            "0|11|0.11|123456789012345678901234567890|",
            "3:0022,d800,0022|3:0022,d800,0022",
        ),
    },
    Case {
        group: "rejected-source",
        description: "empty aggregate whitespace and malformed fragments use the fixed SyntaxError",
        source: r#"
            (function () {
                var invalidTexts = [
                    "",
                    " ",
                    "\t1",
                    "1\n",
                    "[]",
                    "{}",
                    "[1]",
                    '{"x":1}',
                    "01",
                    "-01",
                    "+1",
                    ".1",
                    "1.",
                    "1e",
                    "1e+",
                    "NaN",
                    "Infinity",
                    "undefined",
                    "True",
                    "FALSE",
                    "nul",
                    '"unterminated',
                    "'single'",
                    "/* comment */1",
                    String.fromCharCode(0xfeff) + "1",
                    "1" + String.fromCharCode(0x0b)
                ];
                var failures = [];
                function requireFixedSyntax(input, label) {
                    try {
                        JSON.rawJSON(input);
                        failures.push(label + ":accepted");
                    } catch (error) {
                        if (error.name !== "SyntaxError" ||
                            error.message !== "invalid rawJSON string") {
                            failures.push(label + ":" + error.name + ":" + error.message);
                        }
                    }
                }
                for (var index = 0; index < invalidTexts.length; index++) {
                    requireFixedSyntax(invalidTexts[index], "text" + index);
                }
                requireFixedSyntax(undefined, "undefined");
                requireFixedSyntax({}, "object");
                requireFixedSyntax([], "array");

                var symbolError;
                try { JSON.rawJSON(Symbol("raw")); }
                catch (error) { symbolError = error.name + ":" + error.message; }
                return failures.length + ":" + failures.join(",") + "|" + symbolError;
            })()
        "#,
        expected: "return|string|0:|TypeError:cannot convert symbol to string",
    },
    Case {
        group: "unforgeable-brand",
        description: "only direct RawJSON instances carry the trap-free internal brand",
        source: r#"
            (function () {
                var raw = JSON.rawJSON("7");
                var forged = Object.create(null);
                Object.defineProperty(forged, "rawJSON", {
                    value: "7",
                    writable: false,
                    enumerable: true,
                    configurable: false
                });
                Object.preventExtensions(forged);
                var inherited = Object.create(raw);
                var getterCalls = 0;
                var getterObject = {};
                Object.defineProperty(getterObject, "rawJSON", {
                    get: function () { getterCalls++; throw 71; },
                    enumerable: true
                });
                return [
                    JSON.isRawJSON(raw),
                    JSON.isRawJSON(forged),
                    JSON.isRawJSON(inherited),
                    JSON.isRawJSON({ rawJSON: "7" }),
                    JSON.isRawJSON(getterObject),
                    getterCalls,
                    JSON.isRawJSON(undefined),
                    JSON.isRawJSON(null),
                    JSON.isRawJSON(7),
                    JSON.isRawJSON("7"),
                    JSON.isRawJSON(Symbol("7")),
                    JSON.isRawJSON([]),
                    JSON.isRawJSON.call(123, raw),
                    JSON.isRawJSON()
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|true|false|false|false|false|0|false|false|",
            "false|false|false|false|true|false",
        ),
    },
    Case {
        group: "parse-isolation",
        description: "rawJSON invokes the internal parser without observing a patched JSON.parse",
        source: r#"
            (function () {
                var rawJSON = JSON.rawJSON;
                var calls = 0;
                var sentinel = {};
                Object.defineProperty(JSON, "parse", {
                    configurable: true,
                    get: function () { calls++; throw sentinel; }
                });
                var valid = rawJSON("42");
                var invalidError;
                try { rawJSON("[]"); }
                catch (error) { invalidError = error.name + ":" + error.message; }
                return [
                    valid.rawJSON,
                    JSON.isRawJSON(valid),
                    calls,
                    invalidError
                ].join("|");
            })()
        "#,
        expected: "return|string|42|true|0|SyntaxError:invalid rawJSON string",
    },
    Case {
        group: "stringify-splicing",
        description: "toJSON and replacer precede raw splicing at every structural position",
        source: r#"
            (function () {
                var raw42 = JSON.rawJSON("42");
                var topLog = [];
                var top = JSON.stringify(raw42, function (key, value) {
                    topLog.push(key + ":" + JSON.isRawJSON(value));
                    return value;
                });
                var replacedAway = JSON.stringify(raw42, function (key, value) {
                    return key === "" ? 7 : value;
                });

                var createLog = [];
                var created = JSON.stringify({ x: 1, y: 2 }, function (key, value) {
                    createLog.push(key + ":" + typeof value);
                    if (key === "x") return JSON.rawJSON("2.50");
                    if (key === "y") return undefined;
                    return value;
                });

                var order = [];
                var source = {
                    toJSON: function (key) {
                        order.push("toJSON:" + key);
                        return JSON.rawJSON("3");
                    }
                };
                var fromToJSON = JSON.stringify(source, function (key, value) {
                    order.push("replacer:" + key + ":" + JSON.isRawJSON(value));
                    return value;
                });

                var bigint = JSON.stringify(
                    { tooBig: 9007199254740993n },
                    function (key, value) {
                        return typeof value === "bigint" ? JSON.rawJSON(value) : value;
                    }
                );
                var pretty = JSON.stringify({
                    a: [raw42],
                    b: JSON.rawJSON('"x"')
                }, null, 2).split("\n").join("\\n");

                return [
                    top,
                    topLog.join(","),
                    replacedAway,
                    created,
                    createLog.join(","),
                    fromToJSON,
                    order.join(","),
                    bigint,
                    pretty
                ].join("|");
            })()
        "#,
        expected: concat!(
            r#"return|string|42|:true|7|{"x":2.50}|:object,x:number,y:number|3|"#,
            r#"toJSON:,replacer::true|{"tooBig":9007199254740993}|"#,
            r#"{\n  "a": [\n    42\n  ],\n  "b": "x"\n}"#,
        ),
    },
];

// Proxy is intentionally oracle-only until quickjs-oxide exposes the Proxy
// constructor. These vectors ensure the future proxy milestone does not make
// RawJSON branding transparent or trapful.
const ORACLE_ONLY_CASES: &[Case] = &[Case {
    group: "proxy-brand",
    description: "proxy wrappers are never branded and isRawJSON invokes no proxy trap",
    source: r#"
        (function () {
            var raw = JSON.rawJSON("1");
            var trapCalls = 0;
            var trapped = new Proxy(raw, {
                get: function () { trapCalls++; throw 71; },
                getPrototypeOf: function () { trapCalls++; throw 72; },
                ownKeys: function () { trapCalls++; throw 73; }
            });
            var trappedBrand = JSON.isRawJSON(trapped);
            var revocable = Proxy.revocable(raw, {});
            revocable.revoke();
            var revokedBrand = JSON.isRawJSON(revocable.proxy);
            return [
                trappedBrand,
                trapCalls,
                revokedBrand,
                JSON.stringify(new Proxy(raw, {}))
            ].join("|");
        })()
    "#,
    expected: r#"return|string|false|0|false|{"rawJSON":"1"}"#,
}];

#[test]
fn pinned_quickjs_json_raw_semantics_match_expectations() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP JSON.rawJSON oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    let mut failures = Vec::new();
    for case in CASES.iter().chain(ORACLE_ONLY_CASES) {
        let actual = oracle_observation(&oracle, case);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nexpected: {:?}\n  actual: {:?}",
                case.group, case.description, case.expected, actual,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "pinned QuickJS RawJSON vectors drifted:\n{}",
        failures.join("\n\n"),
    );
}

#[test]
fn json_raw_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP JSON.rawJSON oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        let quickjs = oracle_observation(&oracle, case);
        assert_eq!(
            rust_observation(case),
            quickjs,
            "RawJSON behavior differed from pinned QuickJS for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn json_raw_brand_and_native_errors_cross_realms() {
    // The qjs CLI has no same-runtime multi-context bridge. Exercise the
    // runtime-wide brand directly and pin native SyntaxError allocation to the
    // intrinsic method's defining realm.
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_syntax_error = eval_object(
        &mut defining,
        "SyntaxError.prototype",
        "defining SyntaxError prototype",
    );
    let caller_syntax_error = eval_object(
        &mut caller,
        "SyntaxError.prototype",
        "caller SyntaxError prototype",
    );
    let defining_json = eval_object(&mut defining, "JSON", "defining JSON object");
    let caller_json = eval_object(&mut caller, "JSON", "caller JSON object");
    let raw_json = property_callable(&runtime, &mut defining, &defining_json, "rawJSON");
    let defining_is_raw = property_callable(&runtime, &mut defining, &defining_json, "isRawJSON");
    let caller_is_raw = property_callable(&runtime, &mut caller, &caller_json, "isRawJSON");

    let Value::Object(raw) = caller
        .call(
            &raw_json,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf8("42").unwrap())],
        )
        .expect("cross-realm JSON.rawJSON call")
    else {
        panic!("cross-realm JSON.rawJSON result was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&raw).unwrap(),
        None,
        "RawJSON object did not retain its null prototype",
    );
    for (description, context, predicate) in [
        ("defining", &mut defining, &defining_is_raw),
        ("caller", &mut caller, &caller_is_raw),
    ] {
        assert_eq!(
            context
                .call(predicate, Value::Undefined, &[Value::Object(raw.clone())],)
                .unwrap_or_else(|error| panic!("{description} isRawJSON call: {error}")),
            Value::Bool(true),
            "{description} realm did not recognize the shared RawJSON brand",
        );
    }

    let forged = eval_object(
        &mut caller,
        r#"
            (function () {
                var value = Object.create(null);
                Object.defineProperty(value, "rawJSON", {
                    value: "42", writable: false, enumerable: true, configurable: false
                });
                return Object.preventExtensions(value);
            })()
        "#,
        "caller forged RawJSON object",
    );
    assert_eq!(
        caller
            .call(&defining_is_raw, Value::Undefined, &[Value::Object(forged)],)
            .expect("cross-realm forged brand check"),
        Value::Bool(false),
    );

    assert!(matches!(
        caller.call(
            &raw_json,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf8("[]").unwrap())],
        ),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "cross-realm JSON.rawJSON SyntaxError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_syntax_error),
        "JSON.rawJSON allocated SyntaxError in the caller realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_syntax_error),
        "JSON.rawJSON native SyntaxError unexpectedly used the caller realm",
    );
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
                    "throw|object|{}|{}",
                    error_string_property(&runtime, &mut context, &error, "name", case),
                    error_string_property(&runtime, &mut context, &error, "message", case),
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

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &ObjectRef,
    name: &str,
    case: &Case,
) -> String {
    let key = runtime
        .intern_property_key(name)
        .unwrap_or_else(|failure| panic!("intern Error.{name} key: {failure}"));
    let Value::String(value) = context.get_property(error, &key).unwrap_or_else(|failure| {
        panic!(
            "read Error.{name} for {} / {}: {failure}",
            case.group, case.description,
        )
    }) else {
        panic!(
            "Error.{name} was not a string for {} / {}",
            case.group, case.description,
        );
    };
    value.to_utf8_lossy()
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(function) = context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read callable {name}: {error}"))
    else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&function)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"))
    else {
        panic!("Rust {description} did not evaluate to an object");
    };
    object
}

fn take_exception_object(context: &mut Context, description: &str) -> ObjectRef {
    let Value::Object(error) = context
        .take_exception()
        .unwrap_or_else(|failure| panic!("take {description}: {failure}"))
        .unwrap_or_else(|| panic!("{description} was missing"))
    else {
        panic!("{description} was not an object");
    };
    error
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
