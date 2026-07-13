use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, Context, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value, WellKnownSymbol,
};

// Pins QuickJS 2026-06-04 `js_string_constructor`, `js_string_fromCharCode`,
// `js_string_fromCodePoint`, `js_string_raw`, and `js_string_funcs`. The
// constructor-owned table is treated as one milestone because QuickJS installs
// its three lazy statics before the non-configurable `prototype` property. If
// `%String%` were published first and the statics appended later, observable
// own-key order could never be repaired.

const CASE_PRELUDE: &str = r#"
function __units(value) {
    value = String(value);
    var output = "";
    for (var index = 0; index < value.length; index++) {
        if (index) output += ",";
        var unit = value.charCodeAt(index).toString(16);
        while (unit.length < 4) unit = "0" + unit;
        output += unit;
    }
    return output;
}
function __template(raw) {
    var cooked = Object();
    cooked.raw = raw;
    return cooked;
}
function __rawObject(length, zero, one, two) {
    var raw = Object();
    raw.length = length;
    raw[0] = zero;
    raw[1] = one;
    raw[2] = two;
    return raw;
}
function __getter(getter) {
    var descriptor = Object();
    descriptor.configurable = true;
    descriptor.get = getter;
    return descriptor;
}
function __bits(object, key) {
    var descriptor = Object.getOwnPropertyDescriptor(object, key);
    return (descriptor.writable ? "1" : "0") +
           (descriptor.enumerable ? "1" : "0") +
           (descriptor.configurable ? "1" : "0");
}
function __constructor(value) {
    try { new value(); return true; }
    catch (_) { return false; }
}
"#;

const GRAPH_CASES: &[(&str, &str)] = &[
    (
        "constructor own-key table is complete and ordered",
        r#"(function(){return Object.getOwnPropertyNames(String).join(",")})()"#,
    ),
    (
        "global String occupies its pinned relative bootstrap position",
        r#"(function(){
            var selected=["Number","Boolean","String","Symbol","globalThis","BigInt"];
            var keys=Object.getOwnPropertyNames(globalThis),output=[];
            for(var index=0;index<keys.length;index++)
                if(selected.indexOf(keys[index])>=0)output.push(keys[index]);
            return output.join(",");
        })()"#,
    ),
    (
        "constructor and prototype descriptors match the function-list graph",
        r#"(function(){return [
            __bits(globalThis,"String"),__bits(String,"length"),__bits(String,"name"),
            __bits(String,"fromCharCode"),__bits(String,"fromCodePoint"),
            __bits(String,"raw"),__bits(String,"prototype"),
            __bits(String.prototype,"constructor")
        ].join("|")})()"#,
    ),
    (
        "constructor and branded empty prototype are linked",
        r#"(function(){return [
            typeof String,Object.getPrototypeOf(String)===Function.prototype,
            String.prototype.constructor===String,
            Object.getPrototypeOf(String.prototype)===Object.prototype,
            Object.prototype.toString.call(String.prototype),
            String.prototype.valueOf(),Object.isExtensible(String.prototype),
            __constructor(String)
        ].join("|")})()"#,
    ),
    (
        "static metadata and non-constructor bits are exact",
        r#"(function(){
            var names=["fromCharCode","fromCodePoint","raw"],output=[];
            for(var index=0;index<names.length;index++){
                var name=names[index],fn=String[name];
                output.push(fn.name+":"+fn.length+":"+
                    Object.getOwnPropertyNames(fn).join(",")+":"+
                    (Object.getPrototypeOf(fn)===Function.prototype)+":"+
                    __constructor(fn));
            }
            return output.join("|");
        })()"#,
    ),
    (
        "implemented prototype keys keep pinned filtered order through constructor",
        r#"(function(){
            var selected=["length","at","charCodeAt","charAt","concat","codePointAt",
                "isWellFormed","toWellFormed","indexOf","lastIndexOf","toString",
                "valueOf","constructor"];
            var keys=Object.getOwnPropertyNames(String.prototype),output=[];
            for(var index=0;index<keys.length;index++)
                if(selected.indexOf(keys[index])>=0)output.push(keys[index]);
            return output.join(",");
        })()"#,
    ),
    (
        "lazy statics materialize once and preserve identity",
        r#"(function(){
            var first=String.fromCodePoint,second=String.fromCodePoint;
            var descriptor=Object.getOwnPropertyDescriptor(String,"fromCodePoint");
            return (first===second)+"|"+(first===descriptor.value)+"|"+
                Object.getOwnPropertyNames(first).join(",");
        })()"#,
    ),
];

const AUTOINIT_CASES: &[(&str, &str)] = &[
    (
        "a lazy static can be deleted before first materialization",
        r#"(function(){
            var deleted=delete String.fromCharCode;
            return [deleted,"fromCharCode" in String,
                Object.prototype.hasOwnProperty.call(String,"fromCharCode"),
                typeof String.fromCharCode,Object.getOwnPropertyNames(String).join(",")].join("|");
        })()"#,
    ),
    (
        "assignment replaces a lazy static with an ordinary value",
        r#"(function(){
            String.raw=17;
            return [String.raw,__bits(String,"raw"),Object.getOwnPropertyNames(String).join(",")].join("|");
        })()"#,
    ),
    (
        "the eager global constructor itself is configurable",
        r#"(function(){
            var saved=String;
            var deleted=delete globalThis.String;
            var result=[deleted,"String" in globalThis,
                Object.prototype.hasOwnProperty.call(globalThis,"String"),typeof globalThis.String].join("|");
            globalThis.String=saved;
            return result;
        })()"#,
    ),
];

const CALL_CASES: &[(&str, &str)] = &[
    (
        "ordinary calls cover absent and primitive arguments",
        r#"(function(){
            var values=[String(),String(undefined),String(null),String(false),String(true),
                String(0),String(-0),String(1.5),String(NaN),String(Infinity),
                String(-Infinity),String(1n),String("A\ud800Z")],output=[];
            for(var index=0;index<values.length;index++)output.push(__units(values[index]));
            return output.join("|");
        })()"#,
    ),
    (
        "direct Symbol arguments use the constructor-only formatting exception",
        r#"(function(){
            var values=[String(Symbol()),String(Symbol("")),String(Symbol("x")),
                String(Symbol("\ud800")),String(Symbol.iterator)],output=[];
            for(var index=0;index<values.length;index++)output.push(__units(values[index]));
            return output.join("|");
        })()"#,
    ),
    (
        "call conversion uses string hint and ordinary fallback order",
        r#"(function(){
            var log="",exotic=Object(),fallback=Object();
            exotic[Symbol.toPrimitive]=function(hint){log+="exotic:"+hint+";";return "E"};
            fallback.toString=function(){log+="toString;";return Object()};
            fallback.valueOf=function(){log+="valueOf;";return "V"};
            var first=String(exotic),second=String(fallback);
            return __units(first)+"|"+__units(second)+"|"+log;
        })()"#,
    ),
    (
        "call ignores this and all arguments after the first",
        r#"(function(){
            var hit=false,bomb=Object();
            bomb[Symbol.toPrimitive]=function(){hit=true;throw "late"};
            return __units(String.call(bomb,"first",bomb))+"|"+hit;
        })()"#,
    ),
];

const CONSTRUCT_CASES: &[(&str, &str)] = &[
    (
        "new String creates a branded empty wrapper",
        r#"(function(){
            var value=new String();
            return [typeof value,Object.getPrototypeOf(value)===String.prototype,
                Object.prototype.toString.call(value),__units(value.valueOf()),
                Object.getOwnPropertyNames(value).join(",")].join("|");
        })()"#,
    ),
    (
        "constructed UTF-16 payload exposes virtual indices and fixed length",
        r#"(function(){
            var value=new String("A\ud800Z"),descriptor=Object.getOwnPropertyDescriptor(value,"length");
            return [__units(value.valueOf()),Object.getOwnPropertyNames(value).join(","),
                descriptor.value,__bits(value,"length"),__bits(value,"0"),
                __units(value[0]),__units(value[1]),__units(value[2])].join("|");
        })()"#,
    ),
    (
        "construction converts its argument exactly once with string hint",
        r#"(function(){
            var log="",argument=Object();
            argument[Symbol.toPrimitive]=function(hint){log+=hint+";";return "xy"};
            var value=new String(argument);
            return __units(value.valueOf())+"|"+log;
        })()"#,
    ),
];

const FROM_CHAR_CODE_CASES: &[(&str, &str)] = &[
    (
        "fromCharCode applies ToInt32 and retains exact low UTF-16 units",
        r#"(function(){return __units(String.fromCharCode())+"|"+
            __units(String.fromCharCode(65,0xd800,0x1f600,-1,65536,65.9,NaN,Infinity,-Infinity))})()"#,
    ),
    (
        "fromCharCode converts arguments left to right and stops on abrupt completion",
        r#"(function(){
            var log="";
            function item(name,value,throws){
                var object=Object();
                object[Symbol.toPrimitive]=function(hint){
                    log+=name+":"+hint+";";if(throws)throw name;return value;
                };
                return object;
            }
            var result;
            try{result=String.fromCharCode(item("a",65,false),item("b",66,false),
                item("c",67,true),item("d",68,false))}
            catch(error){result="throw:"+error}
            return result+"|"+log;
        })()"#,
    ),
    (
        "fromCharCode ignores receiver and converts every actual argument",
        r#"(function(){return __units(String.fromCharCode.call(null,65,66,67))})()"#,
    ),
];

const FROM_CODE_POINT_CASES: &[(&str, &str)] = &[
    (
        "fromCodePoint emits BMP surrogate and astral inputs exactly",
        r#"(function(){return __units(String.fromCodePoint())+"|"+
            __units(String.fromCodePoint(65,0xd800,0x1f600,0x10ffff))})()"#,
    ),
    (
        "fromCodePoint converts arguments left to right",
        r#"(function(){
            var log="";
            function item(name,value){
                var object=Object();
                object[Symbol.toPrimitive]=function(hint){log+=name+":"+hint+";";return value};
                return object;
            }
            var result=String.fromCodePoint(item("a",65),item("b",0x1f600));
            return __units(result)+"|"+log;
        })()"#,
    ),
    (
        "fromCodePoint ignores receiver",
        r#"(function(){return __units(String.fromCodePoint.call(17,65,66,67))})()"#,
    ),
];

const RAW_CASES: &[(&str, &str)] = &[
    (
        "raw interleaves raw chunks and present substitutions",
        r#"(function(){return [
            __units(String.raw(__template(["a","b","c"]),1,2)),
            __units(String.raw(__template(["a","b","c"]),1)),
            __units(String.raw(__template(["a","b","c"]))),
            __units(String.raw(__template([]))),__units(String.raw(__template("A\ud800Z"),"-","+"))
        ].join("|")})()"#,
    ),
    (
        "raw observes cooked raw length index and substitution order",
        r#"(function(){
            var log="",cooked=Object(),raw=Object(),length=Object(),substitution=Object();
            Object.defineProperty(cooked,"raw",__getter(function(){log+="get-raw;";return raw}));
            Object.defineProperty(raw,"length",__getter(function(){log+="get-length;";return length}));
            length[Symbol.toPrimitive]=function(hint){log+="length:"+hint+";";return 2};
            Object.defineProperty(raw,"0",__getter(function(){log+="get-0;";return "A"}));
            Object.defineProperty(raw,"1",__getter(function(){log+="get-1;";return "B"}));
            substitution.toString=function(){log+="sub-toString;";return Object()};
            substitution.valueOf=function(){log+="sub-valueOf;";return "S"};
            var result=String.raw(cooked,substitution);
            return __units(result)+"|"+log;
        })()"#,
    ),
    (
        "raw converts each chunk before its following substitution",
        r#"(function(){
            var log="",raw=Object(),first=Object(),second=Object(),sub=Object();raw.length=2;
            first.toString=function(){log+="first;";return "A"};
            second.toString=function(){log+="second;";return "B"};
            sub.toString=function(){log+="sub;";return "S"};
            raw[0]=first;raw[1]=second;
            var result=String.raw(__template(raw),sub);
            return __units(result)+"|"+log;
        })()"#,
    ),
    (
        "raw ToLength clamps negative and NaN lengths to zero",
        r#"(function(){return [
            __units(String.raw(__template(__rawObject(-1,"wrong")))),
            __units(String.raw(__template(__rawObject(NaN,"wrong")))),
            __units(String.raw(__template(__rawObject(2.9,"a","b","wrong"))))
        ].join("|")})()"#,
    ),
    (
        "raw ignores this",
        r#"(function(){return __units(String.raw.call(Symbol("this"),__template(["x","y"]),"-"))})()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "ordinary ToString rejects a Symbol wrapper",
        r#"String(Object(Symbol("wrapped")))"#,
    ),
    (
        "new String does not use the direct Symbol exception",
        r#"new String(Symbol("x"))"#,
    ),
    (
        "String preserves an arbitrary conversion throw",
        r#"(function(){var value=Object();value[Symbol.toPrimitive]=function(){throw "sentinel"};return String(value)})()"#,
    ),
    (
        "String rejects an object ToPrimitive result",
        r#"(function(){var value=Object();value[Symbol.toPrimitive]=function(){return Object()};return String(value)})()"#,
    ),
    ("fromCharCode rejects BigInt", "String.fromCharCode(1n)"),
    (
        "fromCharCode rejects Symbol",
        "String.fromCharCode(Symbol('x'))",
    ),
    (
        "fromCodePoint rejects a negative",
        "String.fromCodePoint(-1)",
    ),
    (
        "fromCodePoint rejects an out-of-range value",
        "String.fromCodePoint(0x110000)",
    ),
    ("fromCodePoint rejects NaN", "String.fromCodePoint(NaN)"),
    (
        "fromCodePoint rejects infinity",
        "String.fromCodePoint(Infinity)",
    ),
    (
        "fromCodePoint rejects a fraction",
        "String.fromCodePoint(1.5)",
    ),
    ("fromCodePoint rejects BigInt", "String.fromCodePoint(1n)"),
    ("raw rejects a missing template", "String.raw()"),
    (
        "raw rejects a null raw value",
        "String.raw(__template(null))",
    ),
    (
        "raw rejects a Symbol chunk",
        "String.raw(__template([Symbol('chunk')]))",
    ),
    (
        "raw preserves a primitive raw getter throw",
        r#"(function(){var value=Object();Object.defineProperty(value,"raw",__getter(function(){throw 91}));return String.raw(value)})()"#,
    ),
    (
        "raw stops after an abrupt chunk conversion",
        r#"(function(){
            var log="",raw=Object(),first=Object(),second=Object();raw.length=2;
            first.toString=function(){log+="first;";throw "chunk"};
            second.toString=function(){log+="second;";return "late"};raw[0]=first;raw[1]=second;
            try{String.raw(__template(raw))}catch(error){return log+error}
            return "missing";
        })()"#,
    ),
    (
        "fromCharCode is not a constructor",
        "new String.fromCharCode()",
    ),
    (
        "fromCodePoint is not a constructor",
        "new String.fromCodePoint()",
    ),
    ("raw is not a constructor", "new String.raw()"),
];

// These vectors document the internal-method surfaces which the Rust heap does
// not publish yet. They are self-checked against the pinned oracle so they can
// move into the ordinary differential as Proxy/RegExp/TypedArray land.
const EXOTIC_ORACLE_ONLY_CASES: &[(&str, &str)] = &[
    (
        "Proxy raw access exposes every ordered internal Get",
        r#"(function(){
            var log="",rawTarget={length:2,0:"a",1:"b"};
            var raw=new Proxy(rawTarget,{get:function(target,key,receiver){
                log+="raw:"+String(key)+";";return Reflect.get(target,key,receiver)}});
            var cooked=new Proxy({raw:raw},{get:function(target,key,receiver){
                log+="cooked:"+String(key)+";";return Reflect.get(target,key,receiver)}});
            return String.raw(cooked,"-")+"|"+log;
        })()"#,
    ),
    (
        "Proxy newTarget prototype is read only after argument conversion",
        r#"(function(){
            var log="",argument=Object();
            argument[Symbol.toPrimitive]=function(hint){log+="argument:"+hint+";";return "x"};
            var target=new Proxy(function(){},{get:function(object,key,receiver){
                log+="target:"+String(key)+";";return Reflect.get(object,key,receiver)}});
            var value=Reflect.construct(String,[argument],target);
            return value.valueOf()+"|"+log;
        })()"#,
    ),
    (
        "RegExp and TypedArray values use ordinary conversion and array-like paths",
        r#"(function(){return String(/a/g)+"|"+String.raw({raw:new Uint16Array([65,66])},"-")})()"#,
    ),
];

const CUSTOM_NEW_TARGET_ORACLE: &str = r#"
var log="",custom=Object(),argument=Object();
var target=(function ForeignString(){}).bind(null);
argument[Symbol.toPrimitive]=function(hint){log+="argument:"+hint+";";return "xy"};
Object.defineProperty(target,"prototype",{configurable:true,get:function(){
    log+="prototype;";return custom;
}});
var value=Reflect.construct(String,[argument],target);
print((Object.getPrototypeOf(value)===custom)+"|"+
      String.prototype.valueOf.call(value)+"|"+
      Object.getOwnPropertyNames(value).join(",")+"|"+log);
var fallbackTarget=(function ForeignFallback(){}).bind(null);
fallbackTarget.prototype=1;
var fallback=Reflect.construct(String,["z"],fallbackTarget);
print((Object.getPrototypeOf(fallback)===String.prototype)+"|"+
      fallback.valueOf()+"|"+Object.getOwnPropertyNames(fallback).join(","));
"#;

#[test]
fn string_intrinsic_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String intrinsic oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("AutoInit", AUTOINIT_CASES),
        ("call", CALL_CASES),
        ("construct", CONSTRUCT_CASES),
        ("fromCharCode", FROM_CHAR_CODE_CASES),
        ("fromCodePoint", FROM_CODE_POINT_CASES),
        ("raw", RAW_CASES),
        ("errors", ERROR_CASES),
        ("exotic boundary", EXOTIC_ORACLE_ONLY_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector had no completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(
        oracle_lines(
            &oracle,
            CUSTOM_NEW_TARGET_ORACLE,
            "String custom new.target"
        ),
        [
            "true|xy|0,1,length|argument:string;prototype;",
            "true|z|0,length",
        ],
    );
}

#[test]
fn string_intrinsic_graph_matches_pinned_quickjs() {
    compare_cases("String intrinsic graph", GRAPH_CASES);
}

#[test]
fn string_intrinsic_autoinit_matches_pinned_quickjs() {
    compare_cases("String intrinsic AutoInit", AUTOINIT_CASES);
}

#[test]
fn string_call_and_construct_match_pinned_quickjs() {
    compare_cases("String calls", CALL_CASES);
    compare_cases("String construction", CONSTRUCT_CASES);
}

#[test]
fn string_static_values_and_conversion_order_match_pinned_quickjs() {
    compare_cases("String.fromCharCode", FROM_CHAR_CODE_CASES);
    compare_cases("String.fromCodePoint", FROM_CODE_POINT_CASES);
    compare_cases("String.raw", RAW_CASES);
}

#[test]
fn string_intrinsic_errors_match_pinned_quickjs() {
    compare_cases("String intrinsic errors", ERROR_CASES);
}

#[test]
fn string_custom_new_target_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String custom new.target: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_custom_new_target_observations(),
        oracle_lines(
            &oracle,
            CUSTOM_NEW_TARGET_ORACLE,
            "String custom new.target"
        ),
    );
}

#[test]
fn string_cross_realm_results_errors_and_user_throws_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_global = defining.global_object().unwrap();
    let string = property_callable(&runtime, &mut defining, &defining_global, "String");
    let from_code_point =
        property_callable(&runtime, &mut defining, string.as_object(), "fromCodePoint");
    let raw = property_callable(&runtime, &mut defining, string.as_object(), "raw");

    let Value::Object(wrapper) = caller
        .construct(
            &string,
            &[Value::String(JsString::try_from_utf8("foreign").unwrap())],
        )
        .unwrap()
    else {
        panic!("foreign new String did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&wrapper).unwrap(),
        Some(defining.string_prototype().unwrap()),
        "ordinary construction did not use the constructor's defining realm",
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let defining_range_error = intrinsic_prototype(&runtime, &mut defining, "RangeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("construct").unwrap()))
        .unwrap();
    assert_eq!(
        caller.construct(&string, &[Value::Symbol(symbol)]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut caller))
            .unwrap(),
        Some(defining_type_error.clone()),
    );
    assert_eq!(
        caller.call(&from_code_point, Value::Undefined, &[Value::Int(-1)]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut caller))
            .unwrap(),
        Some(defining_range_error),
    );
    assert_eq!(
        caller.call(&raw, Value::Undefined, &[Value::Null]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut caller))
            .unwrap(),
        Some(defining_type_error),
    );

    let sentinel = caller.new_object().unwrap();
    let sentinel_key = runtime.intern_property_key("stringSentinel").unwrap();
    assert!(
        caller
            .set_property(
                &caller.global_object().unwrap(),
                &sentinel_key,
                Value::Object(sentinel.clone()),
            )
            .unwrap()
    );
    let throwing = eval_object(
        &mut caller,
        r#"(function(){var value=Object();value[Symbol.toPrimitive]=function(){throw stringSentinel};return value})()"#,
    );
    assert_eq!(
        defining.call(&string, Value::Undefined, &[Value::Object(throwing)]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        defining.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "an explicit user throw did not retain identity",
    );

    assert_eq!(caller.construct(&raw, &[]), Err(RuntimeError::Exception));
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut caller))
            .unwrap(),
        Some(caller_type_error),
        "non-constructor rejection did not use the caller realm",
    );
}

#[test]
fn string_constructor_and_statics_are_per_realm_and_collectable() {
    let runtime = Runtime::new();
    let retained = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_global = first.global_object().unwrap();
        let second_global = second.global_object().unwrap();
        let first_string = property_callable(&runtime, &mut first, &first_global, "String");
        let first_string_again = property_callable(&runtime, &mut first, &first_global, "String");
        let second_string = property_callable(&runtime, &mut second, &second_global, "String");
        assert_eq!(first_string, first_string_again);
        assert_ne!(first_string, second_string);

        let first_raw = property_callable(&runtime, &mut first, first_string.as_object(), "raw");
        let first_raw_again =
            property_callable(&runtime, &mut first, first_string.as_object(), "raw");
        let second_raw = property_callable(&runtime, &mut second, second_string.as_object(), "raw");
        assert_eq!(first_raw, first_raw_again);
        assert_ne!(first_raw, second_raw);
        assert_eq!(
            runtime.get_prototype_of(first_raw.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        first_raw
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(retained);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn string_wrapper_retains_then_releases_its_realm_graph() {
    let runtime = Runtime::new();
    let wrapper = {
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        let string = property_callable(&runtime, &mut context, &global, "String");
        let Value::Object(wrapper) = context
            .construct(
                &string,
                &[Value::String(JsString::try_from_utf8("retained").unwrap())],
            )
            .unwrap()
        else {
            panic!("new String did not return a wrapper");
        };
        wrapper
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn string_intrinsic_records_current_proxy_regexp_and_typed_array_boundaries() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval("typeof Proxy+'|'+typeof RegExp+'|'+typeof Uint16Array+'|'+typeof String.prototype.includes")
            .unwrap(),
        Value::String(JsString::try_from_utf8("undefined|undefined|undefined|undefined").unwrap()),
        "move the oracle-only vectors into the differential as these surfaces are published",
    );
    // Module namespace and mapped-arguments raw objects remain corresponding
    // language/object-model boundaries.
}

fn compare_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source = format!("{CASE_PRELUDE}\n{source}");
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, &source, description),
            observe_oracle_source(&oracle, &source, description),
            "{group} drifted for {description}",
        );
    }
}

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value),
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    string_property(runtime, context, &error, "name"),
                    string_property(runtime, context, &error, "message"),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value),
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description}: {error}"),
    }
}

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
    let source = format!("{CASE_PRELUDE}\n{source}");
    observe_oracle_source(oracle, &source, description)
}

fn observe_oracle_source(oracle: &OsStr, source: &str, description: &str) -> String {
    let wrapper = r#"
try {
  var value=std.evalScript(scriptArgs[0]);
  print('return|'+typeof value+'|'+String(value));
} catch(error) {
  if(error!==null&&typeof error==='object')print('throw|object|'+error.name+'|'+error.message);
  else print('throw|'+typeof error+'|'+String(error));
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

fn oracle_lines(oracle: &OsStr, source: &str, description: &str) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS {description} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS {description} output was not UTF-8: {error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}

fn rust_custom_new_target_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_global = defining.global_object().unwrap();
    let caller_global = caller.global_object().unwrap();
    let string = property_callable(&runtime, &mut defining, &defining_global, "String");
    let defining_string_prototype = defining.string_prototype().unwrap();
    let value_of = property_callable(
        &runtime,
        &mut defining,
        &defining_string_prototype,
        "valueOf",
    );
    let target = eval_callable(
        &runtime,
        &mut caller,
        "(function ForeignString(){}).bind(null)",
    );
    let custom = caller.new_object().unwrap();
    define_data(
        &runtime,
        &caller_global,
        "stringCustomPrototype",
        Value::Object(custom.clone()),
    );
    define_data(
        &runtime,
        &caller_global,
        "stringLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let getter = eval_callable(
        &runtime,
        &mut caller,
        "(function(){stringLog=stringLog+'prototype;';return stringCustomPrototype})",
    );
    define_accessor(&runtime, target.as_object(), "prototype", Some(getter));
    let argument = caller.new_object().unwrap();
    let conversion = eval_callable(
        &runtime,
        &mut caller,
        "(function(hint){stringLog=stringLog+'argument:'+hint+';';return 'xy'})",
    );
    define_data_key(
        &runtime,
        &argument,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(conversion.as_object().clone()),
    );
    let Value::Object(value) = caller
        .construct_with_new_target(&string, &target, &[Value::Object(argument)])
        .unwrap()
    else {
        panic!("custom new.target String did not return an object");
    };
    let Value::String(payload) = caller
        .call(&value_of, Value::Object(value.clone()), &[])
        .unwrap()
    else {
        panic!("custom new.target String wrapper was not branded");
    };
    let Value::String(log) = caller
        .get_property(
            &caller_global,
            &runtime.intern_property_key("stringLog").unwrap(),
        )
        .unwrap()
    else {
        panic!("custom new.target log was not a String");
    };
    let first = format!(
        "{}|{}|{}|{}",
        runtime.get_prototype_of(&value).unwrap() == Some(custom),
        payload.to_utf8_lossy(),
        own_key_names(&runtime, &value).join(","),
        log.to_utf8_lossy(),
    );

    let fallback_target = eval_callable(
        &runtime,
        &mut caller,
        "(function ForeignFallback(){}).bind(null)",
    );
    assert!(
        caller
            .set_property(
                fallback_target.as_object(),
                &runtime.intern_property_key("prototype").unwrap(),
                Value::Int(1),
            )
            .unwrap()
    );
    let Value::Object(fallback) = caller
        .construct_with_new_target(
            &string,
            &fallback_target,
            &[Value::String(JsString::try_from_utf8("z").unwrap())],
        )
        .unwrap()
    else {
        panic!("fallback new.target String did not return an object");
    };
    let Value::String(fallback_payload) = caller
        .call(&value_of, Value::Object(fallback.clone()), &[])
        .unwrap()
    else {
        panic!("fallback String wrapper was not branded");
    };
    let second = format!(
        "{}|{}|{}",
        runtime.get_prototype_of(&fallback).unwrap() == Some(caller.string_prototype().unwrap()),
        fallback_payload.to_utf8_lossy(),
        own_key_names(&runtime, &fallback).join(","),
    );
    vec![first, second]
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    owner: &ObjectRef,
    name: &str,
) -> CallableRef {
    let Value::Object(object) = context
        .get_property(owner, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn intrinsic_prototype(
    runtime: &Runtime,
    context: &mut Context,
    constructor_name: &str,
) -> ObjectRef {
    let global = context.global_object().unwrap();
    let constructor = property_callable(runtime, context, &global, constructor_name);
    let Value::Object(prototype) = context
        .get_property(
            constructor.as_object(),
            &runtime.intern_property_key("prototype").unwrap(),
        )
        .unwrap()
    else {
        panic!("{constructor_name}.prototype was not an object");
    };
    prototype
}

fn eval_callable(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("{source:?} was not callable"))
}

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    object
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("pending exception was not an object");
    };
    error
}

fn define_data(runtime: &Runtime, object: &ObjectRef, name: &str, value: Value) {
    define_data_key(
        runtime,
        object,
        &runtime.intern_property_key(name).unwrap(),
        value,
    );
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
            .unwrap()
    );
}

fn define_accessor(runtime: &Runtime, object: &ObjectRef, name: &str, getter: Option<CallableRef>) {
    assert!(
        runtime
            .define_own_property(
                object,
                &runtime.intern_property_key(name).unwrap(),
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(match getter {
                        Some(getter) => AccessorValue::Callable(getter),
                        None => AccessorValue::Undefined,
                    }),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect()
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not a String property");
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
            if runtime.as_callable(object).unwrap().is_some() {
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
