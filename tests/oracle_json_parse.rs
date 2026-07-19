use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins QuickJS 2026-06-04's `js_json_parse`, `json_parse_value`, and
// `internalize_json_property` paths. Every observer returns a primitive string
// and deliberately avoids JSON.stringify so this target remains usable while
// parse and stringify are implemented as separate milestones.
const CASES: &[Case] = &[
    Case {
        group: "global-graph",
        description: "JSON and JSON.parse expose the pinned lazy-global shape and metadata",
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
                var parseDescriptor = Object.getOwnPropertyDescriptor(JSON, "parse");
                var nameDescriptor = Object.getOwnPropertyDescriptor(JSON.parse, "name");
                var lengthDescriptor = Object.getOwnPropertyDescriptor(JSON.parse, "length");
                var tagDescriptor = Object.getOwnPropertyDescriptor(JSON, Symbol.toStringTag);
                return [
                    globalDescriptor.value === JSON,
                    bits(globalDescriptor),
                    Object.getPrototypeOf(JSON) === Object.prototype,
                    Object.isExtensible(JSON),
                    Object.prototype.toString.call(JSON),
                    Reflect.ownKeys(JSON).map(String).join(","),
                    parseDescriptor.value === JSON.parse,
                    bits(parseDescriptor),
                    JSON.parse.name,
                    JSON.parse.length,
                    bits(nameDescriptor),
                    bits(lengthDescriptor),
                    Object.getOwnPropertyNames(JSON.parse).join(","),
                    Object.getPrototypeOf(JSON.parse) === Function.prototype,
                    typeof JSON.parse,
                    isConstructor(JSON.parse),
                    tagDescriptor.value,
                    bits(tagDescriptor)
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|true|101|true|true|[object JSON]|",
            "isRawJSON,parse,rawJSON,stringify,Symbol(Symbol.toStringTag)|",
            "true|101|parse|2|001|001|length,name|true|function|false|JSON|001",
        ),
    },
    Case {
        group: "strict-grammar",
        description: "only JSON whitespace tokens strings and number grammar are accepted",
        source: r#"
            (function () {
                function status(text) {
                    try {
                        JSON.parse(text);
                        return "ok";
                    } catch (error) {
                        return error.name;
                    }
                }
                var invalid = [
                    "",
                    " ",
                    "[1,]",
                    '{"a":1,}',
                    "01",
                    "-01",
                    "+1",
                    ".1",
                    "1.",
                    "1e",
                    "1e+",
                    "0x10",
                    "NaN",
                    "Infinity",
                    "undefined",
                    "'x'",
                    "{a:1}",
                    "/* comment */1",
                    "[1 2]",
                    '{"a" 1}',
                    '{"a":}',
                    "true false",
                    String.fromCharCode(0xfeff) + "1",
                    String.fromCharCode(0x0b) + "1",
                    String.fromCharCode(0x0c) + "1",
                    '"' + String.fromCharCode(1) + '"',
                    '"\\u005"',
                    '"\\x41"'
                ];
                var results = [];
                for (var index = 0; index < invalid.length; index++) {
                    results.push(status(invalid[index]));
                }
                return [
                    JSON.parse("\t\r\n 42 ") === 42,
                    JSON.parse('"\\f"').charCodeAt(0),
                    JSON.parse('"' + String.fromCharCode(0x2028) + '"').charCodeAt(0),
                    results.join(",")
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|true|12|8232|",
            "SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,",
            "SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,",
            "SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,",
            "SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,SyntaxError,",
            "SyntaxError,SyntaxError,SyntaxError,SyntaxError",
        ),
    },
    Case {
        group: "primitives-utf16",
        description: "primitive coercions escapes surrogate pairs and lone surrogates preserve UTF-16 units",
        source: r#"
            (function () {
                function units(value) {
                    var result = [];
                    for (var index = 0; index < value.length; index++) {
                        result.push(("0000" + value.charCodeAt(index).toString(16)).slice(-4));
                    }
                    return value.length + ":" + result.join(",");
                }
                var missing;
                try { JSON.parse(); } catch (error) { missing = error.name; }
                var symbolError;
                try { JSON.parse(Symbol("text")); } catch (error) { symbolError = error.name; }
                return [
                    JSON.parse(null) === null,
                    JSON.parse(false) === false,
                    JSON.parse(true) === true,
                    Object.is(JSON.parse(0), 0),
                    JSON.parse(3.14),
                    JSON.parse('""').length,
                    units(JSON.parse('"A\\nB\\tC"')),
                    units(JSON.parse('"\\u0000\\/\\\\\\\""')),
                    units(JSON.parse('"\\ud83d\\ude00"')),
                    units(JSON.parse('"\\ud800"')),
                    units(JSON.parse('"\\udc00"')),
                    units(JSON.parse('"é"')),
                    units(Object.keys(JSON.parse('{"\\ud800":1}'))[0]),
                    missing,
                    symbolError
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|true|true|true|true|3.14|0|",
            "5:0041,000a,0042,0009,0043|4:0000,002f,005c,0022|",
            "2:d83d,de00|1:d800|1:dc00|1:00e9|1:d800|SyntaxError|TypeError",
        ),
    },
    Case {
        group: "objects-and-keys",
        description: "duplicate keys overwrite in place and __proto__ is an ordinary own data property",
        source: r#"
            (function () {
                var value = JSON.parse(
                    '{"2":"two","01":"leading","4294967295":"not-index",' +
                    '"1":"one","4294967294":"max-index","x":1,"x":2,' +
                    '"__proto__":{"polluted":true},"__proto__":7,"a":3}'
                );
                var descriptor = Object.getOwnPropertyDescriptor(value, "__proto__");
                var array = JSON.parse('[1,2]');
                var lengthDescriptor = Object.getOwnPropertyDescriptor(array, "length");
                return [
                    Object.keys(value).join(","),
                    value.x,
                    value.__proto__,
                    Object.getPrototypeOf(value) === Object.prototype,
                    Object.prototype.hasOwnProperty.call(value, "__proto__"),
                    descriptor.value,
                    descriptor.writable,
                    descriptor.enumerable,
                    descriptor.configurable,
                    typeof Object.prototype.polluted,
                    Object.keys(array).join(","),
                    array.length,
                    lengthDescriptor.writable,
                    lengthDescriptor.enumerable,
                    lengthDescriptor.configurable
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|1,2,4294967294,01,4294967295,x,__proto__,a|",
            "2|7|true|true|7|true|true|true|undefined|0,1|2|true|false|false",
        ),
    },
    Case {
        group: "number-edges",
        description: "JSON number lexemes round through the pinned binary64 conversion",
        source: r#"
            (function () {
                function observation(text) {
                    var value = JSON.parse(text);
                    return text + "=" + String(value) + ":" +
                        Object.is(value, -0) + ":" + Number.isFinite(value);
                }
                return [
                    observation("-0"),
                    observation("0"),
                    observation("9007199254740991"),
                    observation("9007199254740992"),
                    observation("9007199254740993"),
                    observation("1.7976931348623157e308"),
                    observation("1.7976931348623159e308"),
                    observation("1e400"),
                    observation("-1e400"),
                    observation("5e-324"),
                    observation("4e-324"),
                    observation("1e-324"),
                    observation("2.2250738585072014e-308"),
                    observation("2.225073858507201e-308")
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|-0=0:true:true|0=0:false:true|",
            "9007199254740991=9007199254740991:false:true|",
            "9007199254740992=9007199254740992:false:true|",
            "9007199254740993=9007199254740992:false:true|",
            "1.7976931348623157e308=1.7976931348623157e+308:false:true|",
            "1.7976931348623159e308=Infinity:false:false|",
            "1e400=Infinity:false:false|-1e400=-Infinity:false:false|",
            "5e-324=5e-324:false:true|4e-324=5e-324:false:true|",
            "1e-324=0:false:true|",
            "2.2250738585072014e-308=2.2250738585072014e-308:false:true|",
            "2.225073858507201e-308=2.225073858507201e-308:false:true",
        ),
    },
    Case {
        group: "tostring-order",
        description: "text ToString completes before parsing and reviver classification preserves abrupt values",
        source: r#"
            (function () {
                var log = [];
                var text = {};
                text[Symbol.toPrimitive] = function (hint) {
                    log.push("primitive:" + hint + ":" + (this === text));
                    return '{"answer":42}';
                };
                var first = JSON.parse(text, function (key, value) {
                    log.push("reviver:" + key);
                    return value;
                });

                var fallback = {
                    toString: function () { log.push("toString"); return {}; },
                    valueOf: function () { log.push("valueOf"); return "false"; }
                };
                var second = JSON.parse(fallback);

                var sentinel = { marker: "sentinel" };
                var poison = {};
                Object.defineProperty(poison, Symbol.toPrimitive, {
                    get: function () { log.push("poison-get"); throw sentinel; }
                });
                var poisonResult;
                try {
                    JSON.parse(poison, function () { log.push("poison-reviver"); });
                } catch (error) {
                    poisonResult = error === sentinel;
                }

                var invalidCalls = 0;
                var invalidName;
                try {
                    JSON.parse("{", function () { invalidCalls++; });
                } catch (error) {
                    invalidName = error.name;
                }

                var fakeReviver = {};
                Object.defineProperty(fakeReviver, "call", {
                    get: function () { log.push("fake-call-get"); throw sentinel; }
                });
                var ignored = JSON.parse("1", fakeReviver);

                var symbolName;
                try { JSON.parse(Symbol()); } catch (error) { symbolName = error.name; }
                return [
                    first.answer,
                    second,
                    poisonResult,
                    invalidName,
                    invalidCalls,
                    ignored,
                    symbolName,
                    JSON.parse(new String("true")),
                    JSON.parse(1n),
                    log.join(",")
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|42|false|true|SyntaxError|0|1|TypeError|true|1|",
            "primitive:string:true,reviver:answer,reviver:,toString,valueOf,poison-get",
        ),
    },
    Case {
        group: "reviver-postorder",
        description: "reviver walks integer keys first and nested children before their parents with exact receivers",
        source: r#"
            (function () {
                var calls = [];
                var contexts = [];
                var wrapperOkay = false;
                var value = JSON.parse(
                    '{"p1":0,"p2":{"a":[1,{"b":2}]},"p1":3,"2":4,"1":5}',
                    function (key, current, context) {
                        var owner;
                        if (key === "") owner = "wrapper";
                        else if (Array.isArray(this)) owner = "array";
                        else if (Object.prototype.hasOwnProperty.call(this, "b")) owner = "nested";
                        else if (Object.prototype.hasOwnProperty.call(this, "a")) owner = "p2";
                        else owner = "root";
                        var source = Object.prototype.hasOwnProperty.call(context, "source")
                            ? context.source
                            : "-";
                        calls.push(key + ":" + owner + ":" + typeof current + ":" + source +
                            ":" + arguments.length + ":" +
                            (Object.getPrototypeOf(context) === Object.prototype));
                        contexts.push(context);
                        if (key === "") {
                            wrapperOkay = this[""] === current &&
                                Object.getPrototypeOf(this) === Object.prototype;
                        }
                        return current;
                    }
                );
                var uniqueContexts = true;
                for (var left = 0; left < contexts.length; left++) {
                    for (var right = left + 1; right < contexts.length; right++) {
                        if (contexts[left] === contexts[right]) uniqueContexts = false;
                    }
                }
                return [
                    calls.join(","),
                    wrapperOkay,
                    uniqueContexts,
                    value.p1,
                    value.p2.a[1].b
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|1:root:number:5:3:true,2:root:number:4:3:true,",
            "p1:root:number:-:3:true,0:array:number:1:3:true,",
            "b:nested:number:2:3:true,1:array:object:-:3:true,",
            "a:p2:object:-:3:true,p2:root:object:-:3:true,",
            ":wrapper:object:-:3:true|true|true|3|2",
        ),
    },
    Case {
        group: "reviver-mutation",
        description: "deletion redefinition forward mutation and captured array length follow QuickJS internalization",
        source: r#"
            (function () {
                var calls = [];
                var value = JSON.parse(
                    '{"a":1,"b":2,"arr":[3,4],"tail":5}',
                    function (key, current, context) {
                        var source = Object.prototype.hasOwnProperty.call(context, "source")
                            ? context.source
                            : "-";
                        calls.push(key + ":" + String(current) + ":" + source);
                        if (key === "a") {
                            delete this.b;
                            this.added = 9;
                            return undefined;
                        }
                        if (key === "b") return 20;
                        if (key === "0" && Array.isArray(this)) {
                            delete this[1];
                            this[2] = 6;
                            return undefined;
                        }
                        if (key === "1" && Array.isArray(this)) return 40;
                        return current;
                    }
                );
                return [
                    calls.join(","),
                    Object.keys(value).join(","),
                    Object.prototype.hasOwnProperty.call(value, "a"),
                    value.b,
                    value.added,
                    value.arr.length,
                    Object.keys(value.arr).join(","),
                    0 in value.arr,
                    value.arr[1],
                    value.arr[2]
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|a:1:1,b:undefined:-,0:3:3,1:undefined:-,",
            "arr:,40,6:-,tail:5:5,:[object Object]:-|",
            "arr,tail,added,b|false|20|9|3|1,2|false|40|6",
        ),
    },
    Case {
        group: "reviver-prototype-and-descriptors",
        description: "reviver Get sees prototypes while failed create and delete on non-configurable properties stay silent",
        source: r#"
            (function () {
                Object.prototype.__qjoJsonInherited = 30;
                Array.prototype[1] = 40;
                try {
                    var objectValue = JSON.parse(
                        '{"a":1,"__qjoJsonInherited":2}',
                        function (key, current, context) {
                            if (key === "a") delete this.__qjoJsonInherited;
                            return current;
                        }
                    );
                    var arrayValue = JSON.parse('[1,2]', function (key, current) {
                        if (key === "0") delete this[1];
                        return current;
                    });
                    var failedCreate = JSON.parse('{"a":1,"b":2}', function (key, current) {
                        if (key === "a") {
                            Object.defineProperty(this, "b", { configurable: false });
                        }
                        if (key === "b") return 22;
                        return current;
                    });
                    var failedDelete = JSON.parse('{"a":1,"b":2}', function (key, current) {
                        if (key === "a") {
                            Object.defineProperty(this, "b", { configurable: false });
                        }
                        if (key === "b") return undefined;
                        return current;
                    });
                    return [
                        objectValue.__qjoJsonInherited,
                        Object.prototype.hasOwnProperty.call(objectValue, "__qjoJsonInherited"),
                        arrayValue[1],
                        Object.prototype.hasOwnProperty.call(arrayValue, "1"),
                        failedCreate.b,
                        Object.getOwnPropertyDescriptor(failedCreate, "b").configurable,
                        failedDelete.b,
                        Object.prototype.hasOwnProperty.call(failedDelete, "b")
                    ].join("|");
                } finally {
                    delete Object.prototype.__qjoJsonInherited;
                    delete Array.prototype[1];
                }
            })()
        "#,
        expected: "return|string|30|true|40|true|2|false|2|true",
    },
    Case {
        group: "reviver-abrupt",
        description: "reviver and forward getter abrupt completions preserve identity and stop later callbacks",
        source: r#"
            (function () {
                var first = { kind: "first" };
                var firstCalls = [];
                var firstCaught;
                try {
                    JSON.parse('{"a":1,"b":2}', function (key, current) {
                        firstCalls.push(key);
                        if (key === "a") throw first;
                        return current;
                    });
                } catch (error) {
                    firstCaught = error === first;
                }

                var second = { kind: "second" };
                var secondCalls = [];
                var secondCaught;
                try {
                    JSON.parse('{"a":1,"b":2}', function (key, current) {
                        secondCalls.push(key);
                        if (key === "a") {
                            Object.defineProperty(this, "b", {
                                configurable: true,
                                enumerable: true,
                                get: function () { throw second; }
                            });
                        }
                        return current;
                    });
                } catch (error) {
                    secondCaught = error === second;
                }
                return [
                    firstCaught,
                    firstCalls.join(","),
                    secondCaught,
                    secondCalls.join(",")
                ].join("|");
            })()
        "#,
        expected: "return|string|true|a|true|a",
    },
    Case {
        group: "context-source",
        description: "the third reviver argument preserves exact primitive lexemes and empty object contexts",
        source: r#"
            (function () {
                var calls = [];
                var allDescriptors = true;
                var value = JSON.parse(
                    ' {"s":"\\u0041","n":1.2300e+2,"t":true,"f":false,"z":null,' +
                    '"a":[-0,5e-324],"o":{"x":42}} ',
                    function (key, current, context) {
                        var descriptor = Object.getOwnPropertyDescriptor(context, "source");
                        var source = descriptor === undefined ? "-" : descriptor.value;
                        if (descriptor !== undefined) {
                            allDescriptors = allDescriptors && descriptor.writable &&
                                descriptor.enumerable && descriptor.configurable;
                        }
                        calls.push(key + ":" + source + ":" +
                            Object.keys(context).join(",") + ":" +
                            (Object.getPrototypeOf(context) === Object.prototype));
                        return current;
                    }
                );
                var rootSource;
                JSON.parse("  42 \n", function (key, current, context) {
                    if (key === "") rootSource = context.source;
                    return current;
                });
                return [
                    calls.join("|"),
                    allDescriptors,
                    rootSource,
                    value.s,
                    value.n,
                    Object.is(value.a[0], -0),
                    value.a[1]
                ].join("~");
            })()
        "#,
        expected: concat!(
            "return|string|s:\"\\u0041\":source:true|n:1.2300e+2:source:true|",
            "t:true:source:true|f:false:source:true|z:null:source:true|",
            "0:-0:source:true|1:5e-324:source:true|a:-::true|",
            "x:42:source:true|o:-::true|:-::true~true~42~A~123~true~5e-324",
        ),
    },
    Case {
        group: "context-source-duplicate-hash",
        description: "the ninth object parse record switches duplicate source lookup from first to latest",
        source: r#"
            (function () {
                function xObservation(text) {
                    var seen = "unvisited";
                    var value = JSON.parse(text, function (key, current, context) {
                        if (key === "x") {
                            seen = Object.prototype.hasOwnProperty.call(context, "source")
                                ? context.source
                                : "-";
                        }
                        return current;
                    });
                    return seen + ":" + value.x + ":" + Object.keys(value).join(",");
                }
                return [
                    xObservation('{"x":2,"a":0,"b":0,"c":0,"d":0,"e":0,"f":0,"x":2.0}'),
                    xObservation('{"x":2,"a":0,"b":0,"c":0,"d":0,"e":0,"f":0,"g":0,"x":2.0}'),
                    xObservation('{"x":1,"a":0,"b":0,"c":0,"d":0,"e":0,"f":0,"x":2}'),
                    xObservation('{"x":1,"a":0,"b":0,"c":0,"d":0,"e":0,"f":0,"g":0,"x":2}')
                ].join("|");
            })()
        "#,
        expected: concat!(
            "return|string|2:2:x,a,b,c,d,e,f|2.0:2:x,a,b,c,d,e,f,g|",
            "-:2:x,a,b,c,d,e,f|2:2:x,a,b,c,d,e,f,g",
        ),
    },
];

#[test]
fn pinned_quickjs_json_parse_semantics_match_expectations() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP JSON.parse oracle: set QJS_ORACLE to pinned upstream qjs");
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
        "pinned QuickJS JSON.parse vectors drifted:\n{}",
        failures.join("\n\n"),
    );
}

#[test]
fn json_parse_semantics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP JSON.parse oracle: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in CASES {
        let quickjs = oracle_observation(&oracle, case);
        assert_eq!(
            rust_observation(case),
            quickjs,
            "JSON.parse behavior differed from pinned QuickJS for {} / {}: {:?}",
            case.group,
            case.description,
            case.source,
        );
    }
}

#[test]
fn json_parse_allocations_and_native_errors_use_the_method_defining_realm() {
    // The qjs CLI has no same-runtime multi-context bridge, so this regression
    // pins the C path directly: QuickJS stores a realm on every intrinsic C
    // function (`JS_NewCFunction3`) and switches to it before `js_json_parse`
    // allocates the parse graph, reviver contexts, wrapper, or SyntaxError.
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object_prototype = defining.object_prototype().unwrap();
    let defining_array_prototype = defining.array_prototype().unwrap();
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
    let json = eval_object(&mut defining, "JSON", "defining JSON object");
    let parse = property_callable(&runtime, &mut defining, &json, "parse");
    let reviver_object = eval_object(
        &mut caller,
        r#"
            (function (key, value, context) {
                if (key === "answer") {
                    globalThis.__qjoJsonParseContext = context;
                    globalThis.__qjoJsonParseAnswerHolder = this;
                }
                if (key === "") globalThis.__qjoJsonParseRootHolder = this;
                return value;
            })
        "#,
        "caller reviver",
    );

    let Value::Object(result) = caller
        .call(
            &parse,
            Value::Undefined,
            &[
                Value::String(JsString::try_from_utf8(r#"{"answer":42,"array":[1]}"#).unwrap()),
                Value::Object(reviver_object),
            ],
        )
        .expect("cross-realm JSON.parse call")
    else {
        panic!("cross-realm JSON.parse result was not an object");
    };
    let array = object_property(&runtime, &mut caller, &result, "array");
    let context = eval_object(
        &mut caller,
        "globalThis.__qjoJsonParseContext",
        "reviver context",
    );
    let answer_holder = eval_object(
        &mut caller,
        "globalThis.__qjoJsonParseAnswerHolder",
        "answer holder",
    );
    let root_holder = eval_object(
        &mut caller,
        "globalThis.__qjoJsonParseRootHolder",
        "root holder",
    );

    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_object_prototype.clone()),
        "JSON.parse allocated its object graph in the caller realm",
    );
    assert_eq!(
        runtime.get_prototype_of(&array).unwrap(),
        Some(defining_array_prototype),
        "JSON.parse allocated its arrays in the caller realm",
    );
    assert_eq!(
        runtime.get_prototype_of(&context).unwrap(),
        Some(defining_object_prototype.clone()),
        "JSON.parse allocated reviver contexts in the callback realm",
    );
    assert_eq!(
        answer_holder, result,
        "reviver received the wrong child holder"
    );
    assert_ne!(
        root_holder, result,
        "JSON.parse exposed the result as its root holder"
    );
    assert_eq!(
        runtime.get_prototype_of(&root_holder).unwrap(),
        Some(defining_object_prototype),
        "JSON.parse allocated its root holder in the callback realm",
    );

    assert!(matches!(
        caller.call(
            &parse,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf8("{").unwrap())],
        ),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "cross-realm JSON.parse SyntaxError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_syntax_error),
        "JSON.parse allocated SyntaxError in the caller realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_syntax_error),
        "JSON.parse native SyntaxError unexpectedly used the caller realm",
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
    context: &mut quickjs_oxide::Context,
    error: &quickjs_oxide::ObjectRef,
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

fn object_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> ObjectRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(value) = context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read object property {name}: {error}"))
    else {
        panic!("{name} was not an object property");
    };
    value
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
