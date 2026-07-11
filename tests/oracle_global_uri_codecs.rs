use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField, EvalOptions,
    JsBigInt, JsString, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError,
    Value, WellKnownSymbol,
};

const CODEC_NAMES: [&str; 6] = [
    "decodeURI",
    "decodeURIComponent",
    "encodeURI",
    "encodeURIComponent",
    "escape",
    "unescape",
];

const ENCODER_NAMES: [&str; 2] = ["encodeURI", "encodeURIComponent"];
const DECODER_NAMES: [&str; 2] = ["decodeURI", "decodeURIComponent"];

// Every string is rendered by UTF-16 code unit. This keeps NULs and lone
// surrogates observable without depending on the terminal's UTF-8 decoder.
const ORACLE_PROBE: &str = r#"
function renderString(value) {
    var result = "";
    for (var index = 0; index < value.length; index++) {
        var unit = value.charCodeAt(index);
        if (unit >= 32 && unit <= 126 && unit !== 92 && unit !== 124) {
            result += value.charAt(index);
        } else {
            result += "\\u" + ("000" + unit.toString(16)).slice(-4);
        }
    }
    return "s:" + result;
}
function observe(thunk) {
    try {
        return renderString(String(thunk()));
    } catch (error) {
        if (error !== null && typeof error === "object")
            return "throw:" + error.name + ":" + error.message;
        return "throw:" + String(error);
    }
}
function flags(object, key) {
    var descriptor = Object.getOwnPropertyDescriptor(object, key);
    return (descriptor.writable ? "1" : "0") +
           (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0");
}
function isConstructor(value) {
    try {
        Reflect.construct(function () {}, [], value);
        return true;
    } catch (_) {
        return false;
    }
}
function hex2(value) {
    return ("0" + value.toString(16)).slice(-2);
}
function unchangedAscii(fn) {
    var result = [];
    for (var code = 0; code < 128; code++) {
        var input = String.fromCharCode(code);
        if (fn(input) === input)
            result.push(hex2(code));
    }
    return result.join(",");
}
function preservedEscapes(fn) {
    var result = [];
    for (var code = 0; code < 128; code++) {
        var input = "%" + hex2(code).toUpperCase();
        if (fn(input) === input)
            result.push(hex2(code));
    }
    return result.join(",");
}

var names = [
    "decodeURI", "decodeURIComponent", "encodeURI",
    "encodeURIComponent", "escape", "unescape"
];
var implementedGlobalNames = [
    "parseInt", "parseFloat", "isNaN", "isFinite",
    "decodeURI", "decodeURIComponent", "encodeURI", "encodeURIComponent",
    "escape", "unescape", "Infinity", "NaN", "undefined", "Number", "Boolean"
];
print("global-order=" + Reflect.ownKeys(globalThis).filter(function (key) {
    return implementedGlobalNames.indexOf(key) >= 0;
}).map(String).join(","));
print("graph=" + names.map(function (name) {
    var fn = globalThis[name];
    return name + ":" + Reflect.ownKeys(fn).map(String).join(",") + ":" +
           flags(globalThis, name) + ":" + fn.length + ":" + fn.name + ":" +
           flags(fn, "length") + ":" + flags(fn, "name") + ":" +
           (Object.getPrototypeOf(fn) === Function.prototype) + ":" +
           isConstructor(fn);
}).join("|"));

var primitiveValues = [
    null, false, true, 0, -0, NaN, Infinity, -Infinity,
    9007199254740993n, "A B/%"
];
names.forEach(function (name) {
    var fn = globalThis[name];
    var results = [
        observe(function () { return fn(); }),
        observe(function () { return fn(undefined); })
    ];
    primitiveValues.forEach(function (value) {
        results.push(observe(function () { return fn(value); }));
    });
    results.push(observe(function () { return fn(Symbol("uri")); }));
    print(name + "-primitives=" + results.join("|"));
});

var conversionLog = "";
var extraHit = false;
var exotic = {};
Object.defineProperty(exotic, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) {
        conversionLog += hint + ",";
        return "A B/\u00e9";
    }
});
var extra = {};
Object.defineProperty(extra, Symbol.toPrimitive, {
    configurable: true,
    value: function () { extraHit = true; throw 99; }
});
var fallback = {
    toString: function () { conversionLog += "toString,"; return "A B"; },
    valueOf: function () { conversionLog += "valueOf,"; return 1; }
};
var invalid = {};
Object.defineProperty(invalid, Symbol.toPrimitive, {
    configurable: true,
    value: function () { return {}; }
});
var arbitraryThrow = {};
Object.defineProperty(arbitraryThrow, Symbol.toPrimitive, {
    configurable: true,
    value: function () { throw 71; }
});
var objectResults = names.map(function (name) {
    var fn = globalThis[name];
    conversionLog = "";
    var exoticResult = observe(function () {
        return fn.call(Symbol("this"), exotic, extra);
    });
    var exoticLog = conversionLog;
    conversionLog = "";
    var fallbackResult = observe(function () { return fn(fallback); });
    var fallbackLog = conversionLog;
    return name + ":" + exoticResult + ":" + exoticLog + ":" +
           fallbackResult + ":" + fallbackLog + ":" +
           observe(function () { return fn(invalid); }) + ":" +
           observe(function () { return fn(arbitraryThrow); });
});
print("objects=" + objectResults.join("|") + ":" + extraHit);

var priorityLog = "";
var loneSurrogateResult = {};
Object.defineProperty(loneSurrogateResult, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) {
        priorityLog += "encode:" + hint + ",";
        return "\ud800";
    }
});
var malformedResult = {};
Object.defineProperty(malformedResult, Symbol.toPrimitive, {
    configurable: true,
    value: function (hint) {
        priorityLog += "decode:" + hint + ",";
        return "%";
    }
});
var thrownBeforeCodec = {};
Object.defineProperty(thrownBeforeCodec, Symbol.toPrimitive, {
    configurable: true,
    value: function () {
        priorityLog += "throw,";
        throw 73;
    }
});
print("error-priority=" + [
    observe(function () { return encodeURI(loneSurrogateResult); }),
    observe(function () { return decodeURIComponent(malformedResult); }),
    observe(function () { return encodeURIComponent(thrownBeforeCodec); }),
    priorityLog
].join("|"));

print("encodeURI-ascii=" + unchangedAscii(encodeURI));
print("encodeURIComponent-ascii=" + unchangedAscii(encodeURIComponent));
print("escape-ascii=" + unchangedAscii(escape));
print("decodeURI-preserved=" + preservedEscapes(decodeURI));
print("decodeURIComponent-preserved=" + preservedEscapes(decodeURIComponent));

var unicodeInputs = [
    "\u00e9", "\u4e2d", "\ud83d\ude00", "A \u00e9/\u4e2d?\ud83d\ude00#",
    "\u0000\u007f\u0080\u07ff\u0800\uffff\ud7ff\ue000\udbff\udfff"
];
["encodeURI", "encodeURIComponent"].forEach(function (name) {
    var fn = globalThis[name];
    print(name + "-unicode=" + unicodeInputs.map(function (input) {
        return observe(function () { return fn(input); });
    }).join("|"));
});
var surrogateInputs = [
    "\ud800", "\udfff", "A\ud800B", "\ud800\ud800", "\udc00\ud800"
];
["encodeURI", "encodeURIComponent"].forEach(function (name) {
    var fn = globalThis[name];
    print(name + "-surrogates=" + surrogateInputs.map(function (input) {
        return observe(function () { return fn(input); });
    }).join("|"));
});

var validEncodedInputs = [
    "%00%7F%C2%80%DF%BF%E0%A0%80%EF%BF%BF%F0%90%80%80%F4%8F%BF%BF",
    "%C3%A9%E4%B8%AD%F0%9F%98%80",
    "%23%24%26%2B%2C%2F%3A%3B%3D%3F%40",
    "%2f%3f%23%41",
    "plain-\u00e9-\ud83d\ude00"
];
["decodeURI", "decodeURIComponent"].forEach(function (name) {
    var fn = globalThis[name];
    print(name + "-valid=" + validEncodedInputs.map(function (input) {
        return observe(function () { return fn(input); });
    }).join("|"));
});
var malformedEncodedInputs = [
    "%", "%0", "%GG", "%C2", "%C2%20", "%80", "%C0%80", "%E0%80%80",
    "%ED%A0%80", "%F0%80%80%80", "%F4%90%80%80", "%F5%80%80%80",
    "%E2%28%A1", "%F0%9F%98", "%F0%9F%98%GG", "%41%"
];
["decodeURI", "decodeURIComponent"].forEach(function (name) {
    var fn = globalThis[name];
    print(name + "-malformed=" + malformedEncodedInputs.map(function (input) {
        return observe(function () { return fn(input); });
    }).join("|"));
});

var legacyInputs = [
    "", "AZaz09@*_+-./", " !#$%&\"'(),:;<=>?[\\]^`{|}~",
    "\u00e9", "\u0100", "\u4e2d", "\ud83d\ude00", "\ud800", "\udfff",
    "A\u0000\u00ff\u0100\uffff"
];
print("escape-cases=" + legacyInputs.map(function (input) {
    return observe(function () { return escape(input); });
}).join("|"));
var unescapeInputs = [
    "", "%41", "%4a%4A", "%u0041", "%u00e9", "%u4E2d",
    "%uD83D%uDE00", "%uD800", "%uDFFF", "%C3%A9", "%00%7f%FF",
    "%", "%0", "%GG", "%u", "%u0", "%u000", "%uGGGG", "%U0041",
    "%%41", "%2520", "%u{41}", "%u0041tail", "%41%u0042"
];
print("unescape-cases=" + unescapeInputs.map(function (input) {
    return observe(function () { return unescape(input); });
}).join("|"));
print("legacy-roundtrip=" + legacyInputs.map(function (input) {
    return observe(function () { return unescape(escape(input)); });
}).join("|"));
"#;

const EXPECTED_OBSERVATIONS: &[&str] = &[
    r#"global-order=parseInt,parseFloat,isNaN,isFinite,decodeURI,decodeURIComponent,encodeURI,encodeURIComponent,escape,unescape,Infinity,NaN,undefined,Number,Boolean"#,
    r#"graph=decodeURI:length,name:101:1:decodeURI:001:001:true:false|decodeURIComponent:length,name:101:1:decodeURIComponent:001:001:true:false|encodeURI:length,name:101:1:encodeURI:001:001:true:false|encodeURIComponent:length,name:101:1:encodeURIComponent:001:001:true:false|escape:length,name:101:1:escape:001:001:true:false|unescape:length,name:101:1:unescape:001:001:true:false"#,
    r#"decodeURI-primitives=s:undefined|s:undefined|s:null|s:false|s:true|s:0|s:0|s:NaN|s:Infinity|s:-Infinity|s:9007199254740993|throw:URIError:expecting hex digit|throw:TypeError:cannot convert symbol to string"#,
    r#"decodeURIComponent-primitives=s:undefined|s:undefined|s:null|s:false|s:true|s:0|s:0|s:NaN|s:Infinity|s:-Infinity|s:9007199254740993|throw:URIError:expecting hex digit|throw:TypeError:cannot convert symbol to string"#,
    r#"encodeURI-primitives=s:undefined|s:undefined|s:null|s:false|s:true|s:0|s:0|s:NaN|s:Infinity|s:-Infinity|s:9007199254740993|s:A%20B/%25|throw:TypeError:cannot convert symbol to string"#,
    r#"encodeURIComponent-primitives=s:undefined|s:undefined|s:null|s:false|s:true|s:0|s:0|s:NaN|s:Infinity|s:-Infinity|s:9007199254740993|s:A%20B%2F%25|throw:TypeError:cannot convert symbol to string"#,
    r#"escape-primitives=s:undefined|s:undefined|s:null|s:false|s:true|s:0|s:0|s:NaN|s:Infinity|s:-Infinity|s:9007199254740993|s:A%20B/%25|throw:TypeError:cannot convert symbol to string"#,
    r#"unescape-primitives=s:undefined|s:undefined|s:null|s:false|s:true|s:0|s:0|s:NaN|s:Infinity|s:-Infinity|s:9007199254740993|s:A B/%|throw:TypeError:cannot convert symbol to string"#,
    r#"objects=decodeURI:s:A B/\u00e9:string,:s:A B:toString,:throw:TypeError:toPrimitive:throw:71|decodeURIComponent:s:A B/\u00e9:string,:s:A B:toString,:throw:TypeError:toPrimitive:throw:71|encodeURI:s:A%20B/%C3%A9:string,:s:A%20B:toString,:throw:TypeError:toPrimitive:throw:71|encodeURIComponent:s:A%20B%2F%C3%A9:string,:s:A%20B:toString,:throw:TypeError:toPrimitive:throw:71|escape:s:A%20B/%E9:string,:s:A%20B:toString,:throw:TypeError:toPrimitive:throw:71|unescape:s:A B/\u00e9:string,:s:A B:toString,:throw:TypeError:toPrimitive:throw:71:false"#,
    r#"error-priority=throw:URIError:expecting surrogate pair|throw:URIError:expecting hex digit|throw:73|encode:string,decode:string,throw,"#,
    r#"encodeURI-ascii=21,23,24,26,27,28,29,2a,2b,2c,2d,2e,2f,30,31,32,33,34,35,36,37,38,39,3a,3b,3d,3f,40,41,42,43,44,45,46,47,48,49,4a,4b,4c,4d,4e,4f,50,51,52,53,54,55,56,57,58,59,5a,5f,61,62,63,64,65,66,67,68,69,6a,6b,6c,6d,6e,6f,70,71,72,73,74,75,76,77,78,79,7a,7e"#,
    r#"encodeURIComponent-ascii=21,27,28,29,2a,2d,2e,30,31,32,33,34,35,36,37,38,39,41,42,43,44,45,46,47,48,49,4a,4b,4c,4d,4e,4f,50,51,52,53,54,55,56,57,58,59,5a,5f,61,62,63,64,65,66,67,68,69,6a,6b,6c,6d,6e,6f,70,71,72,73,74,75,76,77,78,79,7a,7e"#,
    r#"escape-ascii=2a,2b,2d,2e,2f,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,4a,4b,4c,4d,4e,4f,50,51,52,53,54,55,56,57,58,59,5a,5f,61,62,63,64,65,66,67,68,69,6a,6b,6c,6d,6e,6f,70,71,72,73,74,75,76,77,78,79,7a"#,
    r#"decodeURI-preserved=23,24,26,2b,2c,2f,3a,3b,3d,3f,40"#,
    r#"decodeURIComponent-preserved="#,
    r#"encodeURI-unicode=s:%C3%A9|s:%E4%B8%AD|s:%F0%9F%98%80|s:A%20%C3%A9/%E4%B8%AD?%F0%9F%98%80#|s:%00%7F%C2%80%DF%BF%E0%A0%80%EF%BF%BF%ED%9F%BF%EE%80%80%F4%8F%BF%BF"#,
    r#"encodeURIComponent-unicode=s:%C3%A9|s:%E4%B8%AD|s:%F0%9F%98%80|s:A%20%C3%A9%2F%E4%B8%AD%3F%F0%9F%98%80%23|s:%00%7F%C2%80%DF%BF%E0%A0%80%EF%BF%BF%ED%9F%BF%EE%80%80%F4%8F%BF%BF"#,
    r#"encodeURI-surrogates=throw:URIError:expecting surrogate pair|throw:URIError:invalid character|throw:URIError:expecting surrogate pair|throw:URIError:expecting surrogate pair|throw:URIError:invalid character"#,
    r#"encodeURIComponent-surrogates=throw:URIError:expecting surrogate pair|throw:URIError:invalid character|throw:URIError:expecting surrogate pair|throw:URIError:expecting surrogate pair|throw:URIError:invalid character"#,
    r#"decodeURI-valid=s:\u0000\u007f\u0080\u07ff\u0800\uffff\ud800\udc00\udbff\udfff|s:\u00e9\u4e2d\ud83d\ude00|s:%23%24%26%2B%2C%2F%3A%3B%3D%3F%40|s:%2f%3f%23A|s:plain-\u00e9-\ud83d\ude00"#,
    r#"decodeURIComponent-valid=s:\u0000\u007f\u0080\u07ff\u0800\uffff\ud800\udc00\udbff\udfff|s:\u00e9\u4e2d\ud83d\ude00|s:#$&+,/:;=?@|s:/?#A|s:plain-\u00e9-\ud83d\ude00"#,
    r#"decodeURI-malformed=throw:URIError:expecting hex digit|throw:URIError:expecting hex digit|throw:URIError:expecting hex digit|throw:URIError:expecting %|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:expecting %|throw:URIError:expecting hex digit|throw:URIError:expecting hex digit"#,
    r#"decodeURIComponent-malformed=throw:URIError:expecting hex digit|throw:URIError:expecting hex digit|throw:URIError:expecting hex digit|throw:URIError:expecting %|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:malformed UTF-8|throw:URIError:expecting %|throw:URIError:expecting hex digit|throw:URIError:expecting hex digit"#,
    r#"escape-cases=s:|s:AZaz09@*_+-./|s:%20%21%23%24%25%26%22%27%28%29%2C%3A%3B%3C%3D%3E%3F%5B%5C%5D%5E%60%7B%7C%7D%7E|s:%E9|s:%u0100|s:%u4E2D|s:%uD83D%uDE00|s:%uD800|s:%uDFFF|s:A%00%FF%u0100%uFFFF"#,
    r#"unescape-cases=s:|s:A|s:JJ|s:A|s:\u00e9|s:\u4e2d|s:\ud83d\ude00|s:\ud800|s:\udfff|s:\u00c3\u00a9|s:\u0000\u007f\u00ff|s:%|s:%0|s:%GG|s:%u|s:%u0|s:%u000|s:%uGGGG|s:%U0041|s:%A|s:%20|s:%u{41}|s:Atail|s:AB"#,
    r#"legacy-roundtrip=s:|s:AZaz09@*_+-./|s: !#$%&"'(),:;<=>?[\u005c]^`{\u007c}~|s:\u00e9|s:\u0100|s:\u4e2d|s:\ud83d\ude00|s:\ud800|s:\udfff|s:A\u0000\u00ff\u0100\uffff"#,
];

#[test]
fn pinned_quickjs_uri_codec_contract_is_stable() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP pinned URI codec contract: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        oracle_observations(&oracle),
        EXPECTED_OBSERVATIONS,
        "the QuickJS URI codec pin drifted"
    );
}

#[test]
fn global_uri_codecs_match_pinned_quickjs() {
    let rust = rust_observations();
    assert_eq!(
        rust, EXPECTED_OBSERVATIONS,
        "host-side URI codec contract changed"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP URI codec differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "global URI codecs differed from pinned QuickJS"
    );
}

#[test]
fn global_uri_codec_errors_use_the_defining_realm() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_encode = global_callable(&runtime, &mut first, "encodeURI");
    let first_decode = global_callable(&runtime, &mut first, "decodeURIComponent");
    let first_function_prototype = first.function_prototype().unwrap();
    assert_eq!(
        runtime.get_prototype_of(first_encode.as_object()).unwrap(),
        Some(first_function_prototype.clone())
    );
    assert_eq!(
        runtime.get_prototype_of(first_decode.as_object()).unwrap(),
        Some(first_function_prototype)
    );

    let first_type_error = intrinsic_prototype(&runtime, &mut first, "TypeError");
    let second_type_error = intrinsic_prototype(&runtime, &mut second, "TypeError");
    let first_uri_error = intrinsic_prototype(&runtime, &mut first, "URIError");
    let second_uri_error = intrinsic_prototype(&runtime, &mut second, "URIError");
    assert_ne!(first_type_error, second_type_error);
    assert_ne!(first_uri_error, second_uri_error);

    let foreign_symbol = runtime.new_symbol(Some(JsString::from("foreign"))).unwrap();
    assert_eq!(
        second.call(
            &first_encode,
            Value::Undefined,
            &[Value::Symbol(foreign_symbol)],
        ),
        Err(RuntimeError::Exception)
    );
    let symbol_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&symbol_error).unwrap(),
        Some(first_type_error)
    );

    assert_eq!(
        second.call(
            &first_decode,
            Value::Undefined,
            &[Value::String(JsString::from("%"))],
        ),
        Err(RuntimeError::Exception)
    );
    let malformed_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&malformed_error).unwrap(),
        Some(first_uri_error)
    );

    assert_eq!(
        second.construct(&first_encode, &[]),
        Err(RuntimeError::Exception)
    );
    let constructor_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&constructor_error).unwrap(),
        Some(second_type_error),
        "the caller realm rejects construction before entering the native"
    );

    let foreign_throw = eval_callable(
        &runtime,
        &mut second,
        "(function() { throw new URIError('foreign conversion'); })",
    );
    let foreign_object = second.new_object().unwrap();
    define_data_key(
        &runtime,
        &foreign_object,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(foreign_throw.as_object().clone()),
    );
    assert_eq!(
        first.call(
            &first_encode,
            Value::Undefined,
            &[Value::Object(foreign_object)],
        ),
        Err(RuntimeError::Exception)
    );
    let user_error = take_exception_object(&mut first);
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(second_uri_error),
        "user-thrown conversion errors keep their originating realm"
    );
}

#[test]
fn global_uri_codec_keeps_its_defining_realm_alive_until_collection() {
    let runtime = Runtime::new();
    let codec = {
        let mut context = runtime.new_context();
        global_callable(&runtime, &mut context, "encodeURIComponent")
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(codec);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn global_uri_codec_native_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP URI codec stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = [
        r#"encodeURI("\ud800")"#,
        r#"encodeURIComponent("\udfff")"#,
        r#"decodeURI("%")"#,
        r#"decodeURIComponent("%C2")"#,
        "new encodeURI()",
        "new decodeURIComponent()",
    ];
    for source in cases {
        assert_eq!(
            rust_uncaught_error(source),
            oracle_uncaught_error(&oracle, source),
            "URI codec stderr differed for {source:?}"
        );
    }

    // The current source slice does not publish Symbol yet. Injecting the
    // primitive through the host still exercises the native ToString fault;
    // the eval-site columns necessarily differ, so compare through the named
    // native frame and leave the exact call-site checks to the cases above.
    for name in ["escape", "unescape"] {
        let rust_source = format!("{name}(__qjo_uri_symbol)");
        let oracle_source = format!(r#"{name}(Symbol("uri"))"#);
        assert_eq!(
            native_stack_prefix(&rust_uncaught_error_with_symbol(&rust_source), name,),
            native_stack_prefix(&oracle_uncaught_error(&oracle, &oracle_source), name),
            "{name} ToString native stack differed"
        );
    }
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let codecs = CODEC_NAMES
        .iter()
        .map(|name| (*name, global_callable(&runtime, &mut context, name)))
        .collect::<Vec<_>>();

    let implemented_global_names = [
        "parseInt",
        "parseFloat",
        "isNaN",
        "isFinite",
        "decodeURI",
        "decodeURIComponent",
        "encodeURI",
        "encodeURIComponent",
        "escape",
        "unescape",
        "Infinity",
        "NaN",
        "undefined",
        "Number",
        "Boolean",
    ];
    let global_order = runtime
        .own_property_keys(&global)
        .unwrap()
        .iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(key)
                .unwrap()
                .to_utf8_lossy()
        })
        .filter(|name| implemented_global_names.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let mut observations = vec![format!("global-order={global_order}")];

    let graph = codecs
        .iter()
        .map(|(name, callable)| {
            format!(
                "{name}:{}:{}:{}:{}:{}:{}:{}:{}",
                own_key_names(&runtime, callable.as_object()),
                data_flags(&runtime, &global, name),
                callable_int_property(&runtime, &mut context, callable, "length"),
                callable_string_property(&runtime, &mut context, callable, "name"),
                data_flags(&runtime, callable.as_object(), "length"),
                data_flags(&runtime, callable.as_object(), "name"),
                runtime
                    .get_prototype_of(callable.as_object())
                    .unwrap()
                    .is_some_and(|prototype| prototype == function_prototype),
                runtime.is_constructor(callable.as_object()).unwrap(),
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    observations.push(format!("graph={graph}"));

    for (name, callable) in &codecs {
        let mut results = vec![
            observe_call_args(&runtime, &mut context, callable, Value::Undefined, &[]),
            observe_call_args(
                &runtime,
                &mut context,
                callable,
                Value::Undefined,
                &[Value::Undefined],
            ),
        ];
        let primitive_values = [
            Value::Null,
            Value::Bool(false),
            Value::Bool(true),
            Value::Int(0),
            Value::Float(-0.0),
            Value::Float(f64::NAN),
            Value::Float(f64::INFINITY),
            Value::Float(f64::NEG_INFINITY),
            Value::BigInt(JsBigInt::from(9_007_199_254_740_993_u64)),
            Value::String(JsString::from("A B/%")),
        ];
        results.extend(primitive_values.into_iter().map(|value| {
            observe_call_args(&runtime, &mut context, callable, Value::Undefined, &[value])
        }));
        let symbol = runtime.new_symbol(Some(JsString::from("uri"))).unwrap();
        results.push(observe_call_args(
            &runtime,
            &mut context,
            callable,
            Value::Undefined,
            &[Value::Symbol(symbol)],
        ));
        observations.push(format!("{name}-primitives={}", results.join("|")));
    }

    define_data(
        &runtime,
        &global,
        "conversionLog",
        Value::String(JsString::from("")),
    );
    define_data(&runtime, &global, "extraHit", Value::Bool(false));
    let exotic = context.new_object().unwrap();
    let exotic_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { conversionLog += hint + ','; return 'A B/\u{e9}'; })",
    );
    define_data_key(
        &runtime,
        &exotic,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(exotic_conversion.as_object().clone()),
    );
    let extra = context.new_object().unwrap();
    let extra_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function() { extraHit = true; throw 99; })",
    );
    define_data_key(
        &runtime,
        &extra,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(extra_conversion.as_object().clone()),
    );
    let fallback = context.new_object().unwrap();
    let fallback_to_string = eval_callable(
        &runtime,
        &mut context,
        "(function() { conversionLog += 'toString,'; return 'A B'; })",
    );
    let fallback_value_of = eval_callable(
        &runtime,
        &mut context,
        "(function() { conversionLog += 'valueOf,'; return 1; })",
    );
    define_data(
        &runtime,
        &fallback,
        "toString",
        Value::Object(fallback_to_string.as_object().clone()),
    );
    define_data(
        &runtime,
        &fallback,
        "valueOf",
        Value::Object(fallback_value_of.as_object().clone()),
    );
    let invalid_result = context.new_object().unwrap();
    define_data(
        &runtime,
        &global,
        "invalidUriPrimitive",
        Value::Object(invalid_result),
    );
    let invalid = context.new_object().unwrap();
    let invalid_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function() { return invalidUriPrimitive; })",
    );
    define_data_key(
        &runtime,
        &invalid,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(invalid_conversion.as_object().clone()),
    );
    let arbitrary_throw = context.new_object().unwrap();
    let arbitrary_conversion = eval_callable(&runtime, &mut context, "(function() { throw 71; })");
    define_data_key(
        &runtime,
        &arbitrary_throw,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(arbitrary_conversion.as_object().clone()),
    );
    let this_symbol = runtime.new_symbol(Some(JsString::from("this"))).unwrap();
    let mut object_results = Vec::new();
    for (name, callable) in &codecs {
        set_global_string(&runtime, &mut context, &global, "conversionLog", "");
        let exotic_result = observe_call_args(
            &runtime,
            &mut context,
            callable,
            Value::Symbol(this_symbol.clone()),
            &[Value::Object(exotic.clone()), Value::Object(extra.clone())],
        );
        let exotic_log = global_string(&runtime, &mut context, &global, "conversionLog");
        set_global_string(&runtime, &mut context, &global, "conversionLog", "");
        let fallback_result = observe_call_args(
            &runtime,
            &mut context,
            callable,
            Value::Undefined,
            &[Value::Object(fallback.clone())],
        );
        let fallback_log = global_string(&runtime, &mut context, &global, "conversionLog");
        let invalid_result = observe_call_args(
            &runtime,
            &mut context,
            callable,
            Value::Undefined,
            &[Value::Object(invalid.clone())],
        );
        let arbitrary_result = observe_call_args(
            &runtime,
            &mut context,
            callable,
            Value::Undefined,
            &[Value::Object(arbitrary_throw.clone())],
        );
        object_results.push(format!(
            "{name}:{exotic_result}:{exotic_log}:{fallback_result}:{fallback_log}:{invalid_result}:{arbitrary_result}"
        ));
    }
    observations.push(format!(
        "objects={}:{}",
        object_results.join("|"),
        global_plain_value(&runtime, &mut context, &global, "extraHit")
    ));

    define_data(
        &runtime,
        &global,
        "priorityLog",
        Value::String(JsString::from("")),
    );
    define_data(
        &runtime,
        &global,
        "priorityLoneSurrogate",
        Value::String(JsString::from_utf16([0xd800])),
    );
    let lone_surrogate_result = context.new_object().unwrap();
    let lone_surrogate_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { priorityLog += 'encode:' + hint + ','; return priorityLoneSurrogate; })",
    );
    define_data_key(
        &runtime,
        &lone_surrogate_result,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(lone_surrogate_conversion.as_object().clone()),
    );
    let malformed_result = context.new_object().unwrap();
    let malformed_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function(hint) { priorityLog += 'decode:' + hint + ','; return '%'; })",
    );
    define_data_key(
        &runtime,
        &malformed_result,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(malformed_conversion.as_object().clone()),
    );
    let thrown_before_codec = context.new_object().unwrap();
    let thrown_conversion = eval_callable(
        &runtime,
        &mut context,
        "(function() { priorityLog += 'throw,'; throw 73; })",
    );
    define_data_key(
        &runtime,
        &thrown_before_codec,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(thrown_conversion.as_object().clone()),
    );
    let priority_results = [
        observe_call_args(
            &runtime,
            &mut context,
            codec(&codecs, "encodeURI"),
            Value::Undefined,
            &[Value::Object(lone_surrogate_result)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            codec(&codecs, "decodeURIComponent"),
            Value::Undefined,
            &[Value::Object(malformed_result)],
        ),
        observe_call_args(
            &runtime,
            &mut context,
            codec(&codecs, "encodeURIComponent"),
            Value::Undefined,
            &[Value::Object(thrown_before_codec)],
        ),
        global_string(&runtime, &mut context, &global, "priorityLog"),
    ];
    observations.push(format!("error-priority={}", priority_results.join("|")));

    for name in ["encodeURI", "encodeURIComponent", "escape"] {
        let callable = codec(&codecs, name);
        observations.push(format!(
            "{name}-ascii={}",
            unchanged_ascii(&mut context, callable)
        ));
    }
    for name in DECODER_NAMES {
        let callable = codec(&codecs, name);
        observations.push(format!(
            "{name}-preserved={}",
            preserved_escapes(&mut context, callable)
        ));
    }

    let unicode_inputs = vec![
        JsString::from_utf16([0x00e9]),
        JsString::from_utf16([0x4e2d]),
        JsString::from_utf16([0xd83d, 0xde00]),
        JsString::from_utf16([
            0x0041, 0x0020, 0x00e9, 0x002f, 0x4e2d, 0x003f, 0xd83d, 0xde00, 0x0023,
        ]),
        JsString::from_utf16([
            0x0000, 0x007f, 0x0080, 0x07ff, 0x0800, 0xffff, 0xd7ff, 0xe000, 0xdbff, 0xdfff,
        ]),
    ];
    for name in ENCODER_NAMES {
        observations.push(format!(
            "{name}-unicode={}",
            observe_string_inputs(
                &runtime,
                &mut context,
                codec(&codecs, name),
                &unicode_inputs
            )
        ));
    }
    let surrogate_inputs = vec![
        JsString::from_utf16([0xd800]),
        JsString::from_utf16([0xdfff]),
        JsString::from_utf16([0x0041, 0xd800, 0x0042]),
        JsString::from_utf16([0xd800, 0xd800]),
        JsString::from_utf16([0xdc00, 0xd800]),
    ];
    for name in ENCODER_NAMES {
        observations.push(format!(
            "{name}-surrogates={}",
            observe_string_inputs(
                &runtime,
                &mut context,
                codec(&codecs, name),
                &surrogate_inputs,
            )
        ));
    }

    let valid_encoded_inputs = strings(&[
        "%00%7F%C2%80%DF%BF%E0%A0%80%EF%BF%BF%F0%90%80%80%F4%8F%BF%BF",
        "%C3%A9%E4%B8%AD%F0%9F%98%80",
        "%23%24%26%2B%2C%2F%3A%3B%3D%3F%40",
        "%2f%3f%23%41",
        "plain-é-😀",
    ]);
    for name in DECODER_NAMES {
        observations.push(format!(
            "{name}-valid={}",
            observe_string_inputs(
                &runtime,
                &mut context,
                codec(&codecs, name),
                &valid_encoded_inputs,
            )
        ));
    }
    let malformed_encoded_inputs = strings(&[
        "%",
        "%0",
        "%GG",
        "%C2",
        "%C2%20",
        "%80",
        "%C0%80",
        "%E0%80%80",
        "%ED%A0%80",
        "%F0%80%80%80",
        "%F4%90%80%80",
        "%F5%80%80%80",
        "%E2%28%A1",
        "%F0%9F%98",
        "%F0%9F%98%GG",
        "%41%",
    ]);
    for name in DECODER_NAMES {
        observations.push(format!(
            "{name}-malformed={}",
            observe_string_inputs(
                &runtime,
                &mut context,
                codec(&codecs, name),
                &malformed_encoded_inputs,
            )
        ));
    }

    let legacy_inputs = vec![
        JsString::from(""),
        JsString::from("AZaz09@*_+-./"),
        JsString::from(" !#$%&\"'(),:;<=>?[\\]^`{|}~"),
        JsString::from_utf16([0x00e9]),
        JsString::from_utf16([0x0100]),
        JsString::from_utf16([0x4e2d]),
        JsString::from_utf16([0xd83d, 0xde00]),
        JsString::from_utf16([0xd800]),
        JsString::from_utf16([0xdfff]),
        JsString::from_utf16([0x0041, 0x0000, 0x00ff, 0x0100, 0xffff]),
    ];
    let escape_callable = codec(&codecs, "escape");
    let unescape_callable = codec(&codecs, "unescape");
    observations.push(format!(
        "escape-cases={}",
        observe_string_inputs(&runtime, &mut context, escape_callable, &legacy_inputs)
    ));
    let unescape_inputs = strings(&[
        "",
        "%41",
        "%4a%4A",
        "%u0041",
        "%u00e9",
        "%u4E2d",
        "%uD83D%uDE00",
        "%uD800",
        "%uDFFF",
        "%C3%A9",
        "%00%7f%FF",
        "%",
        "%0",
        "%GG",
        "%u",
        "%u0",
        "%u000",
        "%uGGGG",
        "%U0041",
        "%%41",
        "%2520",
        "%u{41}",
        "%u0041tail",
        "%41%u0042",
    ]);
    observations.push(format!(
        "unescape-cases={}",
        observe_string_inputs(&runtime, &mut context, unescape_callable, &unescape_inputs,)
    ));
    let roundtrip = legacy_inputs
        .iter()
        .map(|input| {
            let encoded = context
                .call(
                    escape_callable,
                    Value::Undefined,
                    &[Value::String(input.clone())],
                )
                .unwrap();
            observe_call_args(
                &runtime,
                &mut context,
                unescape_callable,
                Value::Undefined,
                &[encoded],
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    observations.push(format!("legacy-roundtrip={roundtrip}"));

    observations
}

fn codec<'a>(codecs: &'a [(&str, CallableRef)], name: &str) -> &'a CallableRef {
    &codecs
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .unwrap_or_else(|| panic!("missing codec {name}"))
        .1
}

fn strings(values: &[&str]) -> Vec<JsString> {
    values.iter().map(|value| JsString::from(*value)).collect()
}

fn unchanged_ascii(context: &mut Context, callable: &CallableRef) -> String {
    (0_u16..128)
        .filter(|code| {
            let input = JsString::from_utf16([*code]);
            let Value::String(output) = context
                .call(callable, Value::Undefined, &[Value::String(input.clone())])
                .unwrap()
            else {
                panic!("URI encoder did not return a string")
            };
            output == input
        })
        .map(|code| format!("{code:02x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn preserved_escapes(context: &mut Context, callable: &CallableRef) -> String {
    (0_u16..128)
        .filter(|code| {
            let input = JsString::from(format!("%{code:02X}").as_str());
            let Value::String(output) = context
                .call(callable, Value::Undefined, &[Value::String(input.clone())])
                .unwrap()
            else {
                panic!("URI decoder did not return a string")
            };
            output == input
        })
        .map(|code| format!("{code:02x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn observe_string_inputs(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    inputs: &[JsString],
) -> String {
    inputs
        .iter()
        .map(|input| {
            observe_call_args(
                runtime,
                context,
                callable,
                Value::Undefined,
                &[Value::String(input.clone())],
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn render_string(value: &JsString) -> String {
    let mut rendered = String::from("s:");
    for unit in value.utf16_units() {
        if (0x20..=0x7e).contains(&unit) && unit != 0x5c && unit != 0x7c {
            rendered.push(char::from_u32(u32::from(unit)).unwrap());
        } else {
            rendered.push_str(&format!("\\u{unit:04x}"));
        }
    }
    rendered
}

fn observe_call_args(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    this_value: Value,
    arguments: &[Value],
) -> String {
    match context.call(callable, this_value, arguments) {
        Ok(Value::String(value)) => render_string(&value),
        Ok(value) => panic!("URI codec returned a non-string value: {value:?}"),
        Err(RuntimeError::Exception) => {
            let exception = context.take_exception().unwrap().unwrap();
            match exception {
                Value::Object(error) => format!(
                    "throw:{}:{}",
                    error_text(runtime, context, &error, "name"),
                    error_text(runtime, context, &error, "message")
                ),
                Value::String(value) => format!("throw:{}", render_string(&value)),
                value => format!("throw:{}", plain_value(value)),
            }
        }
        Err(error) => panic!("URI codec returned engine error: {error}"),
    }
}

fn global_callable(runtime: &Runtime, context: &mut Context, name: &str) -> CallableRef {
    let global = context.global_object().unwrap();
    property_callable(runtime, context, &global, name)
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(value) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn intrinsic_prototype(runtime: &Runtime, context: &mut Context, name: &str) -> ObjectRef {
    let constructor = global_callable(runtime, context, name);
    let prototype = runtime.intern_property_key("prototype").unwrap();
    let Value::Object(prototype) = context
        .get_property(constructor.as_object(), &prototype)
        .unwrap()
    else {
        panic!("{name}.prototype was not an object");
    };
    prototype
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> String {
    runtime
        .own_property_keys(object)
        .unwrap()
        .iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn data_flags(runtime: &Runtime, object: &ObjectRef, name: &str) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let Some(CompleteOrdinaryPropertyDescriptor::Data {
        writable,
        enumerable,
        configurable,
        ..
    }) = runtime.get_own_property(object, &key).unwrap()
    else {
        panic!("{name} was not an own data property");
    };
    format!(
        "{}{}{}",
        u8::from(writable),
        u8::from(enumerable),
        u8::from(configurable)
    )
}

fn callable_int_property(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    name: &str,
) -> i32 {
    let Value::Int(value) = context
        .get_property(
            callable.as_object(),
            &runtime.intern_property_key(name).unwrap(),
        )
        .unwrap()
    else {
        panic!("callable {name} was not an Int");
    };
    value
}

fn callable_string_property(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(
            callable.as_object(),
            &runtime.intern_property_key(name).unwrap(),
        )
        .unwrap()
    else {
        panic!("callable {name} was not a String");
    };
    value.to_utf8_lossy()
}

fn define_data(runtime: &Runtime, object: &ObjectRef, name: &str, value: Value) {
    let key = runtime.intern_property_key(name).unwrap();
    define_data_key(runtime, object, &key, value);
}

fn define_data_key(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey, value: Value) {
    assert!(
        runtime
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap(),
        "host data-property definition was rejected"
    );
}

fn eval_callable(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("callable source did not produce an object: {source:?}");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("source did not produce a callable: {source:?}"))
}

fn set_global_string(
    runtime: &Runtime,
    context: &mut Context,
    global: &ObjectRef,
    name: &str,
    value: &str,
) {
    assert!(
        context
            .set_property(
                global,
                &runtime.intern_property_key(name).unwrap(),
                Value::String(JsString::from(value)),
            )
            .unwrap()
    );
}

fn global_string(
    runtime: &Runtime,
    context: &mut Context,
    global: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(global, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("global {name} was not a string");
    };
    value.to_utf8_lossy()
}

fn global_plain_value(
    runtime: &Runtime,
    context: &mut Context,
    global: &ObjectRef,
    name: &str,
) -> String {
    plain_value(
        context
            .get_property(global, &runtime.intern_property_key(name).unwrap())
            .unwrap(),
    )
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("operation did not throw an object");
    };
    error
}

fn error_text(runtime: &Runtime, context: &mut Context, error: &ObjectRef, name: &str) -> String {
    let Value::String(value) = context
        .get_property(error, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("Error.{name} was not a string");
    };
    value.to_utf8_lossy()
}

fn plain_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) if value.is_nan() => "NaN".to_owned(),
        Value::Float(value) if value == f64::INFINITY => "Infinity".to_owned(),
        Value::Float(value) if value == f64::NEG_INFINITY => "-Infinity".to_owned(),
        Value::Float(value) if value == 0.0 && value.is_sign_negative() => "-0".to_owned(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => render_string(&value),
        Value::Symbol(_) => "Symbol".to_owned(),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

fn rust_uncaught_error(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval_with_options(source, &EvalOptions::new("<cmdline>")),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("uncaught URI codec error was not an Error object");
    };
    let name = error_text(&runtime, &mut context, &error, "name");
    let message = error_text(&runtime, &mut context, &error, "message");
    let stack = error_text(&runtime, &mut context, &error, "stack");
    format!("{name}: {message}\n{stack}")
}

fn rust_uncaught_error_with_symbol(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    define_data(
        &runtime,
        &global,
        "__qjo_uri_symbol",
        Value::Symbol(runtime.new_symbol(Some(JsString::from("uri"))).unwrap()),
    );
    assert_eq!(
        context.eval_with_options(source, &EvalOptions::new("<cmdline>")),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("uncaught URI codec ToString error was not an Error object");
    };
    let name = error_text(&runtime, &mut context, &error, "name");
    let message = error_text(&runtime, &mut context, &error, "message");
    let stack = error_text(&runtime, &mut context, &error, "stack");
    format!("{name}: {message}\n{stack}")
}

fn native_stack_prefix(stderr: &str, function_name: &str) -> String {
    let marker = format!("    at {function_name} (native)");
    let mut prefix = Vec::new();
    for line in stderr.lines() {
        prefix.push(line);
        if line == marker {
            return format!("{}\n", prefix.join("\n"));
        }
    }
    panic!("no {function_name} native frame in {stderr:?}");
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["-e", ORACLE_PROBE])
        .output()
        .expect("run QuickJS URI codec oracle");
    assert!(
        output.status.success(),
        "QuickJS URI codec oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS URI codec oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn oracle_uncaught_error(oracle: &OsStr, source: &str) -> String {
    let output = Command::new(oracle)
        .args(["-e", source])
        .output()
        .expect("run QuickJS URI codec stack oracle");
    assert_eq!(output.status.code(), Some(1));
    String::from_utf8(output.stderr).expect("QuickJS URI codec stderr was not UTF-8")
}
