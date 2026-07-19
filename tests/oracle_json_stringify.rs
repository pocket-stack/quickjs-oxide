use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, ObjectRef, Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins QuickJS 2026-06-04's `JS_JSONStringify`, `js_json_to_str`, and
// `JS_ToQuotedString` paths. Each observer returns an ASCII primitive string,
// including the UTF-16 vectors, so the qjs process boundary never has to
// encode an unpaired surrogate.
const CASES: &[Case] = &[
    Case {
        group: "global-graph",
        description: "JSON.stringify exposes the pinned lazy-global shape and metadata",
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
                var globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "JSON");
                var stringifyDescriptor = Object.getOwnPropertyDescriptor(JSON, "stringify");
                var nameDescriptor = Object.getOwnPropertyDescriptor(JSON.stringify, "name");
                var lengthDescriptor = Object.getOwnPropertyDescriptor(JSON.stringify, "length");
                var tagDescriptor = Object.getOwnPropertyDescriptor(JSON, Symbol.toStringTag);
                return [
                    globalDescriptor.value === JSON,
                    bits(globalDescriptor),
                    Object.getPrototypeOf(JSON) === Object.prototype,
                    Object.isExtensible(JSON),
                    Object.prototype.toString.call(JSON),
                    Reflect.ownKeys(JSON).map(String).join(","),
                    stringifyDescriptor.value === JSON.stringify,
                    bits(stringifyDescriptor),
                    JSON.stringify.name,
                    JSON.stringify.length,
                    bits(nameDescriptor),
                    bits(lengthDescriptor),
                    Object.getOwnPropertyNames(JSON.stringify).join(","),
                    Object.getPrototypeOf(JSON.stringify) === Function.prototype,
                    typeof JSON.stringify,
                    isConstructor(JSON.stringify),
                    tagDescriptor.value,
                    bits(tagDescriptor)
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|true|101|true|true|[object JSON]|",
            "isRawJSON,parse,rawJSON,stringify,Symbol(Symbol.toStringTag)|",
            "true|101|stringify|3|001|001|length,name|true|function|false|JSON|001",
        ),
    },
    Case {
        group: "root-primitives",
        description: "root values wrappers and unrepresentable values follow the JSON type table",
        source: r#"
            (function () {
                function observation(value) {
                    var result = JSON.stringify(value);
                    return typeof result + ":" + String(result);
                }
                var fn = function () {};
                return [
                    observation(undefined),
                    observation(null),
                    observation(true),
                    observation(false),
                    observation("A\nB"),
                    observation(-0),
                    observation(1.25),
                    observation(NaN),
                    observation(Infinity),
                    observation(-Infinity),
                    observation(fn),
                    observation(Symbol("x")),
                    observation(new Boolean(false)),
                    observation(new Number(3)),
                    observation(new String("xy")),
                    JSON.stringify({ u: undefined, f: fn, s: Symbol("x"), n: null }),
                    JSON.stringify([undefined, fn, Symbol("x"), null])
                ].join("|");
            })()
        "#,
        expected: concat!(
            r#"return|string|undefined:undefined|string:null|string:true|string:false|"#,
            r#"string:"A\nB"|string:0|string:1.25|string:null|string:null|string:null|"#,
            r#"undefined:undefined|undefined:undefined|string:false|string:3|string:"xy"|"#,
            r#"{"n":null}|[null,null,null,null]"#,
        ),
    },
    Case {
        group: "arrays-objects-and-keys",
        description: "arrays use length while objects use enumerable own string keys in pinned order",
        source: r#"
            (function () {
                var proto = { inherited: "ignored" };
                var object = Object.create(proto);
                object["10"] = "ten";
                object["2"] = "two";
                object.b = 1;
                object.a = 2;
                Object.defineProperty(object, "hidden", { value: 3, enumerable: false });
                Object.defineProperty(object, "__proto__", {
                    value: 4, writable: true, enumerable: true, configurable: true
                });
                object[Symbol("symbol-key")] = 5;

                var array = [];
                array[2] = "x";
                array.extra = "ignored";
                Object.defineProperty(array, "hidden", { value: 6, enumerable: false });
                return [
                    JSON.stringify(object),
                    JSON.stringify(array),
                    JSON.stringify([, 1, ,]),
                    JSON.stringify(Object.create(null)),
                    JSON.stringify({ value: Symbol("ignored"), keep: 1 })
                ].join("|");
            })()
        "#,
        expected: concat!(
            r#"return|string|{"2":"two","10":"ten","b":1,"a":2,"#,
            r#""__proto__":4}|[null,null,"x"]|[null,1,null]|{}|{"keep":1}"#,
        ),
    },
    Case {
        group: "quoted-utf16",
        description: "string quoting escapes controls and lone surrogates while preserving scalar pairs",
        source: r#"
            (function () {
                function units(value) {
                    var result = [];
                    for (var index = 0; index < value.length; index++) {
                        result.push(("0000" + value.charCodeAt(index).toString(16)).slice(-4));
                    }
                    return value.length + ":" + result.join(",");
                }
                var controls = "\"\\/\b\f\n\r\t" + String.fromCharCode(0, 1, 31);
                var pair = String.fromCharCode(0xd83d, 0xde00);
                var high = String.fromCharCode(0xd800);
                var low = String.fromCharCode(0xdc00);
                var separators = String.fromCharCode(0x2028, 0x2029);
                var keyed = {};
                keyed[high] = low;
                return [
                    units(JSON.stringify(controls)),
                    units(JSON.stringify(pair)),
                    units(JSON.stringify(high)),
                    units(JSON.stringify(low)),
                    units(JSON.stringify(separators)),
                    units(JSON.stringify(keyed))
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|35:0022,005c,0022,005c,005c,002f,005c,0062,005c,0066,",
            "005c,006e,005c,0072,005c,0074,005c,0075,0030,0030,0030,0030,005c,",
            "0075,0030,0030,0030,0031,005c,0075,0030,0030,0031,0066,0022|",
            "4:0022,d83d,de00,0022|8:0022,005c,0075,0064,0038,0030,0030,0022|",
            "8:0022,005c,0075,0064,0063,0030,0030,0022|4:0022,2028,2029,0022|",
            "19:007b,0022,005c,0075,0064,0038,0030,0030,0022,003a,0022,005c,0075,",
            "0064,0063,0030,0030,0022,007d",
        ),
    },
    Case {
        group: "number-formatting",
        description: "finite binary64 values use QuickJS number formatting and non-finite values become null",
        source: r#"
            (function () {
                return [
                    JSON.stringify(-0),
                    JSON.stringify(0),
                    JSON.stringify(0.000001),
                    JSON.stringify(0.0000001),
                    JSON.stringify(100000000000000000000),
                    JSON.stringify(1000000000000000000000),
                    JSON.stringify(9007199254740991),
                    JSON.stringify(9007199254740993),
                    JSON.stringify(1.7976931348623157e308),
                    JSON.stringify(5e-324),
                    JSON.stringify(NaN),
                    JSON.stringify(Infinity),
                    JSON.stringify(-Infinity),
                    JSON.stringify([NaN, Infinity, -Infinity, -0])
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|0|0|0.000001|1e-7|100000000000000000000|1e+21|",
            "9007199254740991|9007199254740992|1.7976931348623157e+308|5e-324|",
            "null|null|null|[null,null,null,0]",
        ),
    },
    Case {
        group: "tojson-and-replacer",
        description: "toJSON precedes the replacer and callbacks receive exact keys values and holders",
        source: r#"
            (function () {
                var log = [];
                var replacement = { y: 2 };
                var nested = { x: 1 };
                Object.defineProperty(nested, "toJSON", {
                    value: function (key) {
                        log.push("toJSON:nested:" + key + ":" + (this === nested));
                        return replacement;
                    }
                });
                var value = { a: nested, b: 3 };
                Object.defineProperty(value, "toJSON", {
                    value: function (key) {
                        log.push("toJSON:root:" + key + ":" + (this === value));
                        return this;
                    }
                });
                var rootHolder;
                function replacer(key, current) {
                    var owner = key === "" ? "wrapper" :
                        this === value ? "root" :
                        this === replacement ? "replacement" : "other";
                    log.push("replacer:" + key + ":" + owner + ":" + typeof current);
                    if (key === "") rootHolder = this;
                    if (key === "b") return undefined;
                    return current;
                }
                var result = JSON.stringify(value, replacer);
                return [
                    result,
                    log.join(","),
                    rootHolder[""] === value,
                    Object.getPrototypeOf(rootHolder) === Object.prototype,
                    Object.keys(rootHolder).join(",")
                ].join("|");
            })()
        "#,
        expected: concat!(
            r#"return|string|{"a":{"y":2}}|toJSON:root::true,"#,
            "replacer::wrapper:object,toJSON:nested:a:true,replacer:a:root:object,",
            "replacer:y:replacement:number,replacer:b:root:number|true|true|",
        ),
    },
    Case {
        group: "replacer-array",
        description: "replacer arrays snapshot length use Get coerce wrappers and deduplicate first occurrence",
        source: r#"
            (function () {
                var log = [];
                var boxed = new String("a");
                boxed.toString = function () { log.push("boxed"); return "a"; };
                var keys = ["b", "unused", 2, "b", , new Number(3), {}, Symbol("s"), boxed];
                Object.defineProperty(keys, "0", {
                    configurable: true,
                    enumerable: true,
                    get: function () {
                        log.push("get0");
                        keys[1] = "c";
                        keys.push("late");
                        return "b";
                    }
                });
                Object.defineProperty(keys, "2", {
                    configurable: true,
                    enumerable: true,
                    get: function () { log.push("get2"); return 2; }
                });
                Array.prototype[4] = "a";
                try {
                    var value = {
                        "2": "two", "3": "three", a: "A", b: "B", c: "C", late: "L"
                    };
                    return [
                        JSON.stringify(value, keys),
                        JSON.stringify([value], ["b"]),
                        log.join(","),
                        keys.length
                    ].join("|");
                } finally {
                    delete Array.prototype[4];
                }
            })()
        "#,
        expected: concat!(
            r#"return|string|{"b":"B","c":"C","2":"two","#,
            r#""a":"A","3":"three"}|[{"b":"B"}]|"#,
            "get0,get2,boxed|10",
        ),
    },
    Case {
        group: "space-and-order",
        description: "gap coercion follows replacer normalization and clamps or truncates to ten UTF-16 units",
        source: r#"
            (function () {
                function visible(value) {
                    var result = "";
                    for (var index = 0; index < value.length; index++) {
                        var unit = value.charCodeAt(index);
                        result += unit === 10 ? "\\n" : String.fromCharCode(unit);
                    }
                    return result;
                }
                function units(value) {
                    var result = [];
                    for (var index = 0; index < value.length; index++) {
                        result.push(("0000" + value.charCodeAt(index).toString(16)).slice(-4));
                    }
                    return value.length + ":" + result.join(",");
                }
                var log = [];
                var propertyList = ["a", "b"];
                Object.defineProperty(propertyList, "0", {
                    configurable: true,
                    get: function () { log.push("replacer-get"); return "a"; }
                });
                var space = new Number(2);
                space.valueOf = function () { log.push("space-valueOf"); return 4; };
                var value = { a: 1, b: { c: 2 } };
                var surrogateGap = "123456789" + String.fromCharCode(0xd800) + "ignored";
                return [
                    visible(JSON.stringify(value, propertyList, space)),
                    visible(JSON.stringify(value, null, 99)),
                    visible(JSON.stringify(value, null, "abcdefghijk")),
                    visible(JSON.stringify(value, null, -1)),
                    units(JSON.stringify({ a: 1 }, null, surrogateGap)),
                    log.join(",")
                ].join("|");
            })()
        "#,
        expected: concat!(
            r#"return|string|{\n    "a": 1,\n    "b": {}\n}|"#,
            r#"{\n          "a": 1,\n          "b": {\n                    "c": 2\n          }\n}|"#,
            r#"{\nabcdefghij"a": 1,\nabcdefghij"b": {\nabcdefghijabcdefghij"c": 2\nabcdefghij}\n}|"#,
            r#"{"a":1,"b":{"c":2}}|"#,
            "20:007b,000a,0031,0032,0033,0034,0035,0036,0037,0038,0039,d800,",
            "0022,0061,0022,003a,0020,0031,000a,007d|replacer-get,space-valueOf",
        ),
    },
    Case {
        group: "cycles-and-shared-objects",
        description: "only ancestor cycles throw while shared acyclic objects serialize at each path",
        source: r#"
            (function () {
                function thrown(value, replacer) {
                    try {
                        JSON.stringify(value, replacer);
                        return "none";
                    } catch (error) {
                        return error.name + ":" + error.message;
                    }
                }
                var shared = { x: 1 };
                var objectCycle = {};
                objectCycle.self = objectCycle;
                var arrayCycle = [];
                arrayCycle[0] = arrayCycle;
                var rescued = {};
                rescued.self = rescued;
                Object.defineProperty(rescued, "toJSON", {
                    value: function () { return 7; }
                });
                return [
                    JSON.stringify({ a: shared, b: shared }),
                    JSON.stringify([shared, shared]),
                    thrown(objectCycle),
                    thrown(arrayCycle),
                    JSON.stringify(rescued),
                    JSON.stringify(objectCycle, function (key, value) {
                        return key === "self" ? undefined : value;
                    })
                ].join("|");
            })()
        "#,
        expected: concat!(
            r#"return|string|{"a":{"x":1},"b":{"x":1}}|"#,
            r#"[{"x":1},{"x":1}]|TypeError:circular reference|"#,
            "TypeError:circular reference|7|{}",
        ),
    },
    Case {
        group: "deep-iterative-traversal",
        description: "deep acyclic arrays use the engine traversal stack rather than a fixed 256-level cutoff",
        source: r#"
            (function () {
                function serializedLength(depth) {
                    var value = 0;
                    for (var index = 0; index < depth; index++) value = [value];
                    return JSON.stringify(value).length;
                }
                return serializedLength(257) + "|" + serializedLength(4096);
            })()
        "#,
        expected: "return|string|515|8193",
    },
    Case {
        group: "bigint",
        description: "BigInt throws after transformation but toJSON and replacers can rescue it",
        source: r#"
            (function () {
                function thrown(value) {
                    try {
                        JSON.stringify(value);
                        return "none";
                    } catch (error) {
                        return error.name + ":" + error.message;
                    }
                }
                var direct = thrown(1n);
                var member = thrown({ x: 1n });
                var element = thrown([1n]);
                var log = [];
                BigInt.prototype.toJSON = function (key) {
                    log.push(key + ":" + typeof this + ":" + this.valueOf());
                    return String(this.valueOf()) + "n";
                };
                try {
                    return [
                        direct,
                        member,
                        element,
                        JSON.stringify(2n),
                        JSON.stringify({ x: 3n }),
                        JSON.stringify([4n]),
                        JSON.stringify({ x: 5n }, function (key, value) {
                            return typeof value === "bigint" ? Number(value) : value;
                        }),
                        log.join(",")
                    ].join("|");
                } finally {
                    delete BigInt.prototype.toJSON;
                }
            })()
        "#,
        expected: concat!(
            "return|string|TypeError:Do not know how to serialize a BigInt|",
            "TypeError:Do not know how to serialize a BigInt|",
            "TypeError:Do not know how to serialize a BigInt|",
            r#""2n"|{"x":"3n"}|["4n"]|{"x":"5n"}|"#,
            ":object:2,x:object:3,0:object:4,x:object:5",
        ),
    },
    Case {
        group: "mutation-snapshots",
        description: "object keys and array length snapshot while later Gets observe deletion mutation and prototypes",
        source: r#"
            (function () {
                var object = {};
                Object.defineProperty(object, "a", {
                    configurable: true,
                    enumerable: true,
                    get: function () {
                        delete object.b;
                        object.c = 30;
                        Object.defineProperty(object, "c", { enumerable: false });
                        object.d = 4;
                        return 1;
                    }
                });
                object.b = 2;
                object.c = 3;
                var array = [0, 1, 2];
                Object.defineProperty(array, "0", {
                    configurable: true,
                    enumerable: true,
                    get: function () {
                        delete array[1];
                        array[2] = 20;
                        array.push(3);
                        return 0;
                    }
                });
                Array.prototype[1] = 11;
                try {
                    return [
                        JSON.stringify(object),
                        Object.keys(object).join(","),
                        JSON.stringify(array),
                        array.length,
                        Object.keys(array).join(",")
                    ].join("|");
                } finally {
                    delete Array.prototype[1];
                }
            })()
        "#,
        expected: r#"return|string|{"a":1,"c":30}|a,d|[0,11,20]|4|0,2,3"#,
    },
    Case {
        group: "abrupt-order",
        description: "property-list and gap work precede traversal and abrupt values preserve identity",
        source: r#"
            (function () {
                var sentinel = { marker: "sentinel" };
                var log = [];
                var propertyList = ["a"];
                Object.defineProperty(propertyList, "0", {
                    configurable: true,
                    get: function () { log.push("property-list"); return "a"; }
                });
                var gap = new String(" ");
                gap.toString = function () { log.push("gap"); throw sentinel; };
                var value = { a: 1 };
                Object.defineProperty(value, "toJSON", {
                    get: function () { log.push("toJSON-get"); return undefined; }
                });
                var gapCaught = false;
                try { JSON.stringify(value, propertyList, gap); }
                catch (error) { gapCaught = error === sentinel; }

                var getterCalls = [];
                var getterValue = {};
                Object.defineProperty(getterValue, "a", {
                    enumerable: true,
                    get: function () { getterCalls.push("a"); throw sentinel; }
                });
                getterValue.b = 2;
                var getterCaught = false;
                try { JSON.stringify(getterValue); }
                catch (error) { getterCaught = error === sentinel; }

                var replacerCalls = [];
                var replacerCaught = false;
                try {
                    JSON.stringify({ a: 1, b: 2 }, function (key, current) {
                        replacerCalls.push(key);
                        if (key === "a") throw sentinel;
                        return current;
                    });
                } catch (error) {
                    replacerCaught = error === sentinel;
                }
                return [
                    gapCaught,
                    log.join(","),
                    getterCaught,
                    getterCalls.join(","),
                    replacerCaught,
                    replacerCalls.join(",")
                ].join("|");
            })()
        "#,
        expected: "return|string|true|property-list,gap|true|a|true|,a",
    },
];

#[test]
fn pinned_quickjs_json_stringify_semantics_match_expectations() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP JSON.stringify oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    let mut failures = Vec::new();
    for case in CASES {
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
        "pinned QuickJS JSON.stringify vectors drifted:\n{}",
        failures.join("\n\n"),
    );
}

#[test]
fn json_stringify_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP JSON.stringify oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        let quickjs = oracle_observation(&oracle, case);
        assert_eq!(
            rust_observation(case),
            quickjs,
            "JSON.stringify behavior differed from pinned QuickJS for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn json_stringify_wrapper_and_native_errors_use_the_method_defining_realm() {
    // The qjs CLI has no same-runtime multi-context bridge. This regression
    // pins the C path directly: intrinsic C functions carry their defining
    // realm, which supplies the root wrapper and native TypeError objects.
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object_prototype = defining.object_prototype().unwrap();
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );
    let json = eval_object(&mut defining, "JSON", "defining JSON object");
    let stringify = property_callable(&runtime, &mut defining, &json, "stringify");
    let input = eval_object(&mut caller, "({ answer: 42 })", "caller input");
    let replacer = eval_object(
        &mut caller,
        r#"
            (function (key, value) {
                if (key === "") globalThis.__qjoJsonStringifyRootHolder = this;
                return value;
            })
        "#,
        "caller replacer",
    );

    let Value::String(result) = caller
        .call(
            &stringify,
            Value::Undefined,
            &[
                Value::Object(input.clone()),
                Value::Object(replacer),
                Value::Undefined,
            ],
        )
        .expect("cross-realm JSON.stringify call")
    else {
        panic!("cross-realm JSON.stringify result was not a string");
    };
    assert_eq!(result.to_utf8_lossy(), r#"{"answer":42}"#);
    let root_holder = eval_object(
        &mut caller,
        "globalThis.__qjoJsonStringifyRootHolder",
        "stringify root holder",
    );
    assert_eq!(
        runtime.get_prototype_of(&root_holder).unwrap(),
        Some(defining_object_prototype),
        "JSON.stringify allocated its root holder in the callback realm",
    );
    assert_ne!(
        root_holder, input,
        "JSON.stringify exposed the input object as its root holder",
    );

    let cycle = eval_object(
        &mut caller,
        "(function () { var value = {}; value.self = value; return value; })()",
        "caller cyclic input",
    );
    assert!(matches!(
        caller.call(&stringify, Value::Undefined, &[Value::Object(cycle)],),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "cross-realm JSON.stringify TypeError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error),
        "JSON.stringify allocated TypeError in the caller realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error),
        "JSON.stringify native TypeError unexpectedly used the caller realm",
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
