use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04 `js_string_split`
// (`quickjs.c` 45894-45980) and its prototype-table entry (46640).
//
// The JavaScript vectors intentionally stay inside quickjs-oxide's implemented
// grammar. In particular they do not use RegExp literals or constructors,
// object-literal methods/accessors, classes, arrows, destructuring, modules, or
// async syntax. RegExp integration belongs to RegExp.prototype[Symbol.split];
// this gate covers the complete generic String method, including arbitrary
// @@split delegation.

const PRELUDE: &str = r#"
function __bit(value){return value?"1":"0"}
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    if(descriptor===undefined)return "missing";
    return __bit(descriptor.writable)+__bit(descriptor.enumerable)+
        __bit(descriptor.configurable);
}
function __isConstructor(value){
    try{Reflect.construct(function(){},[],value);return true}
    catch(_error){return false}
}
function __units(value){
    var output="",index=0,unit;
    while(index<value.length){
        unit=value.charCodeAt(index).toString(16);
        while(unit.length<4)unit="0"+unit;
        if(index!==0)output+=",";
        output+=unit;
        index++;
    }
    return value.length+":"+output;
}
function __arrayUnits(value){
    var output=[],index=0;
    while(index<value.length){output[index]=__units(value[index]);index++}
    return value.length+"["+output.join(";")+"]";
}
function __completion(callback){
    try{return "return:"+String(callback())}
    catch(error){
        if(error!==null&&typeof error==="object")
            return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
"#;

const GRAPH_CASES: &[(&str, &str)] = &[
    (
        "split occupies the pinned filtered prototype key slot",
        r#"(function(){
            var selected=["startsWith","split","substring","substr","slice","repeat"],
                keys=Object.getOwnPropertyNames(String.prototype),output=[],index=0;
            while(index<keys.length){
                if(selected.indexOf(keys[index])>=0)output[output.length]=keys[index];
                index++;
            }
            return output.join(",");
        })()"#,
    ),
    (
        "split exposes exact descriptor name length function graph and own keys",
        r#"(function(){
            var fn=String.prototype.split;
            return [
                __bits(String.prototype,"split"),fn.name,fn.length,
                Object.getOwnPropertyNames(fn).join(","),
                __bits(fn,"name"),__bits(fn,"length"),
                Object.getPrototypeOf(fn)===Function.prototype,
                typeof fn,__isConstructor(fn),
                fn===Object.getOwnPropertyDescriptor(String.prototype,"split").value,
                fn===String.prototype.split
            ].join("|");
        })()"#,
    ),
    (
        "split is distinct from adjacent methods and stable after materialization",
        r#"(function(){
            var split=String.prototype.split;
            return [split===String.prototype.split,split===String.prototype.startsWith,
                split===String.prototype.substring,split===String.prototype.slice,
                split===String.prototype.repeat].join("|");
        })()"#,
    ),
];

const AUTOINIT_CASES: &[(&str, &str)] = &[
    (
        "split can be deleted before materialization",
        r#"(function(){
            var deleted=delete String.prototype.split;
            return [deleted,"split" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"split"),
                typeof String.prototype.split].join("|");
        })()"#,
    ),
    (
        "split assignment before materialization creates an ordinary replacement",
        r#"(function(){
            String.prototype.split=17;
            return [String.prototype.split,__bits(String.prototype,"split"),
                Object.prototype.hasOwnProperty.call(String.prototype,"split")].join("|");
        })()"#,
    ),
    (
        "materialized split remains configurable",
        r#"(function(){
            var fn=String.prototype.split,deleted=delete String.prototype.split;
            return [typeof fn,deleted,"split" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"split")].join("|");
        })()"#,
    ),
];

const DELEGATION_CASES: &[(&str, &str)] = &[
    (
        "@@split getter and call receive the raw receiver limit and separator this",
        r#"(function(){
            var log="",receiver=Object(),limit=Object(),separator=Object(),result=Object();
            receiver.toString=function(){log+="receiver-tostring;";throw "receiver-converted"};
            limit.valueOf=function(){log+="limit-valueof;";throw "limit-converted"};
            separator.toString=function(){log+="separator-tostring;";throw "separator-converted"};
            Object.defineProperty(separator,Symbol.split,{configurable:true,get:function(){
                log+="split-get;";
                return function(source,rawLimit){
                    log+="split-call:"+(this===separator)+":"+(source===receiver)+":"+
                        (rawLimit===limit)+":"+arguments.length+";";
                    return result;
                };
            }});
            var actual=String.prototype.split.call(receiver,separator,limit);
            return log+"result:"+(actual===result);
        })()"#,
    ),
    (
        "@@split observes unboxed primitive receivers",
        r#"(function(){
            var separator=Object(),output=[];
            separator[Symbol.split]=function(value,limit){
                return [typeof value,String(value),typeof limit,String(limit),
                    this===separator,arguments.length].join(":");
            };
            output[0]=String.prototype.split.call("xy",separator,1);
            output[1]=String.prototype.split.call(7,separator,true);
            output[2]=String.prototype.split.call(false,separator,7n);
            output[3]=String.prototype.split.call(9n,separator,"limit");
            output[4]=String.prototype.split.call(Symbol.iterator,separator,null);
            return output.join("|");
        })()"#,
    ),
    (
        "primitive separators never consult boxed prototype @@split getters",
        r#"(function(){
            var hits="",output=[];
            function poison(prototype,label){
                Object.defineProperty(prototype,Symbol.split,{configurable:true,get:function(){
                    hits+=label;throw label;
                }});
            }
            poison(String.prototype,"string;");
            poison(Number.prototype,"number;");
            poison(Boolean.prototype,"boolean;");
            poison(BigInt.prototype,"bigint;");
            poison(Symbol.prototype,"symbol;");
            output[0]=__arrayUnits("a,b".split(","));
            output[1]=__arrayUnits("a1b1".split(1));
            output[2]=__arrayUnits("atrueb".split(true));
            output[3]=__arrayUnits("a7b7".split(7n));
            output[4]=__completion(function(){return "abc".split(Symbol.iterator)});
            return output.join("|")+"|hits:"+hits;
        })()"#,
    ),
    (
        "null and undefined @@split values fall back after one getter",
        r#"(function(){
            function run(splitter){
                var log="",separator=Object();
                Object.defineProperty(separator,Symbol.split,{get:function(){log+="get;";return splitter}});
                separator.toString=function(){log+="string;";return ","};
                return __arrayUnits("a,b".split(separator,3))+":"+log;
            }
            return run(null)+"|"+run(undefined);
        })()"#,
    ),
    (
        "noncallable @@split fails before any ordinary conversion",
        r#"(function(){
            var log="",receiver=Object(),limit=Object(),separator=Object();
            receiver.toString=function(){log+="receiver;";return "a,b"};
            limit.valueOf=function(){log+="limit;";return 2};
            separator.toString=function(){log+="separator;";return ","};
            Object.defineProperty(separator,Symbol.split,{get:function(){log+="get;";return 17}});
            var completion=__completion(function(){
                return String.prototype.split.call(receiver,separator,limit);
            });
            return completion+"|"+log;
        })()"#,
    ),
    (
        "@@split getter abrupt completion preserves identity and prevents conversion",
        r#"(function(){
            var log="",sentinel=Object(),receiver=Object(),limit=Object(),separator=Object(),same=false;
            receiver.toString=function(){log+="receiver;";return "x"};
            limit.valueOf=function(){log+="limit;";return 1};
            Object.defineProperty(separator,Symbol.split,{get:function(){log+="get;";throw sentinel}});
            try{String.prototype.split.call(receiver,separator,limit)}
            catch(error){same=error===sentinel}
            return same+"|"+log;
        })()"#,
    ),
    (
        "nullish receiver rejection precedes @@split lookup",
        r#"(function(){
            var hits=0,separator=Object(),first,second;
            Object.defineProperty(separator,Symbol.split,{get:function(){hits++;return function(){return 1}}});
            first=__completion(function(){return String.prototype.split.call(null,separator,1)});
            second=__completion(function(){return String.prototype.split.call(undefined,separator,1)});
            return first+"|"+second+"|hits:"+hits;
        })()"#,
    ),
];

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "undefined null and primitive separators retain pinned special cases",
        r#"(function(){
            return [
                __arrayUnits("abc".split()),
                __arrayUnits("abc".split(undefined)),
                __arrayUnits("undefined".split(undefined)),
                __arrayUnits("abc".split(null)),
                __arrayUnits("atrueb".split(true)),
                __arrayUnits("a17b17".split(17)),
                __arrayUnits(String.prototype.split.call(12321,2))
            ].join("|");
        })()"#,
    ),
    (
        "empty source and separator cases are exact",
        r#"(function(){
            return [
                __arrayUnits("".split()),__arrayUnits("".split(undefined)),
                __arrayUnits("".split("")),__arrayUnits("".split("x")),
                __arrayUnits("abc".split("")),__arrayUnits("abc".split("x")),
                __arrayUnits("abc".split("abc")),__arrayUnits("abc".split("abcd")),
                __arrayUnits("aaaa".split("aa")),__arrayUnits("ababa".split("aba"))
            ].join("|");
        })()"#,
    ),
    (
        "ordinary separators copy exact tails and repeated matches",
        r#"(function(){
            return [
                __arrayUnits("a,b,c".split(",")),
                __arrayUnits(",a,,b,".split(",")),
                __arrayUnits("one<>two<>three".split("<>")),
                __arrayUnits("abababa".split("ba")),
                __arrayUnits("xx--yy--".split("--"))
            ].join("|");
        })()"#,
    ),
    (
        "generic receivers and separator ToString results are exact",
        r#"(function(){
            var receiver=Object(),separator=Object();
            receiver.toString=function(){return "left::right::tail"};
            separator.valueOf=function(){return Object()};
            separator.toString=function(){return "::"};
            return [__arrayUnits(String.prototype.split.call(receiver,separator)),
                __arrayUnits(String.prototype.split.call(true,"r")),
                __arrayUnits(String.prototype.split.call(9009,0)),
                __arrayUnits(String.prototype.split.call(5n,""))].join("|");
        })()"#,
    ),
];

const UTF16_CASES: &[(&str, &str)] = &[
    (
        "empty separator splits UTF-16 code units including astral pairs",
        r#"(function(){
            var astral="A\ud83d\ude00B",lone="\ud800X\udc00",mixed="\ud83d\ude00\ud800\udc00";
            return [__units(astral),__arrayUnits(astral.split("")),
                __units(lone),__arrayUnits(lone.split("")),
                __units(mixed),__arrayUnits(mixed.split(""))].join("|");
        })()"#,
    ),
    (
        "wide separators match raw code units without scalar normalization",
        r#"(function(){
            var source="A\ud83d\ude00B\ud83d\ude00C\ud800D\ud800E";
            return [
                __arrayUnits(source.split("\ud83d\ude00")),
                __arrayUnits(source.split("\ud83d")),
                __arrayUnits(source.split("\ude00")),
                __arrayUnits(source.split("\ud800")),
                __arrayUnits(source.split("\ud800D"))
            ].join("|");
        })()"#,
    ),
    (
        "UTF-16 limits can end between surrogate halves",
        r#"(function(){
            var source="\ud83d\ude00\ud83d\ude00";
            return [__arrayUnits(source.split("",1)),__arrayUnits(source.split("",2)),
                __arrayUnits(source.split("",3)),__arrayUnits(source.split("",4)),
                __arrayUnits(source.split("",5))].join("|");
        })()"#,
    ),
];

const LIMIT_CASES: &[(&str, &str)] = &[
    (
        "ToUint32 boundary values control result length",
        r#"(function(){
            var limits=[undefined,0,-0,NaN,Infinity,-Infinity,-1,-1.9,1.9,
                2,3,4294967295,4294967296,4294967297,8589934593],output=[],index=0;
            while(index<limits.length){
                output[index]=String(limits[index])+":"+__arrayUnits("a,b,c".split(",",limits[index]));
                index++;
            }
            return output.join("|");
        })()"#,
    ),
    (
        "zero limit still converts the separator after receiver and limit",
        r#"(function(){
            var log="",receiver=Object(),limit=Object(),separator=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "a,b"};
            limit[Symbol.toPrimitive]=function(hint){log+="limit:"+hint+";";return 0};
            Object.defineProperty(separator,Symbol.split,{get:function(){log+="split-get;";return undefined}});
            separator[Symbol.toPrimitive]=function(hint){log+="separator:"+hint+";";return ","};
            return __arrayUnits(String.prototype.split.call(receiver,separator,limit))+"|"+log;
        })()"#,
    ),
    (
        "undefined separator obeys converted limit without matching its text",
        r#"(function(){
            return [__arrayUnits("--undefined--".split(undefined,0)),
                __arrayUnits("--undefined--".split(undefined,1)),
                __arrayUnits("--undefined--".split(undefined,2)),
                __arrayUnits("--undefined--".split("undefined",1)),
                __arrayUnits("--undefined--".split("undefined",2))].join("|");
        })()"#,
    ),
    (
        "arguments after limit are ignored without conversion",
        r#"(function(){
            var hits=0,extra=Object();
            extra[Symbol.toPrimitive]=function(){hits++;throw "extra"};
            return __arrayUnits(String.prototype.split.call("a,b,c",",",2,extra,Symbol.iterator,7n))+
                "|hits:"+hits;
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "ordinary path observes @@split then receiver limit separator conversion",
        r#"(function(){
            var log="",receiver=Object(),limit=Object(),separator=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "a,b,c"};
            limit[Symbol.toPrimitive]=function(hint){log+="limit:"+hint+";";return 2};
            Object.defineProperty(separator,Symbol.split,{get:function(){log+="split-get;";return undefined}});
            separator[Symbol.toPrimitive]=function(hint){log+="separator:"+hint+";";return ","};
            var result=String.prototype.split.call(receiver,separator,limit);
            return __arrayUnits(result)+"|"+log;
        })()"#,
    ),
    (
        "receiver abrupt completion follows @@split lookup and stops later conversion",
        r#"(function(){
            var log="",sentinel=Object(),receiver=Object(),limit=Object(),separator=Object(),same=false;
            Object.defineProperty(separator,Symbol.split,{get:function(){log+="split-get;";return undefined}});
            receiver[Symbol.toPrimitive]=function(){log+="receiver;";throw sentinel};
            limit.valueOf=function(){log+="limit;";return 1};
            separator.toString=function(){log+="separator;";return ","};
            try{String.prototype.split.call(receiver,separator,limit)}catch(error){same=error===sentinel}
            return same+"|"+log;
        })()"#,
    ),
    (
        "limit abrupt completion follows receiver and stops separator ToString",
        r#"(function(){
            var log="",sentinel=Object(),receiver=Object(),limit=Object(),separator=Object(),same=false;
            Object.defineProperty(separator,Symbol.split,{get:function(){log+="split-get;";return undefined}});
            receiver.toString=function(){log+="receiver;";return "a,b"};
            limit.valueOf=function(){log+="limit;";throw sentinel};
            separator.toString=function(){log+="separator;";return ","};
            try{String.prototype.split.call(receiver,separator,limit)}catch(error){same=error===sentinel}
            return same+"|"+log;
        })()"#,
    ),
    (
        "separator abrupt completion follows completed limit conversion",
        r#"(function(){
            var log="",sentinel=Object(),receiver=Object(),limit=Object(),separator=Object(),same=false;
            Object.defineProperty(separator,Symbol.split,{get:function(){log+="split-get;";return undefined}});
            receiver.toString=function(){log+="receiver;";return "a,b"};
            limit.valueOf=function(){log+="limit;";return 1};
            separator.toString=function(){log+="separator;";throw sentinel};
            try{String.prototype.split.call(receiver,separator,limit)}catch(error){same=error===sentinel}
            return same+"|"+log;
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "null receiver is rejected",
        "String.prototype.split.call(null,',',1)",
    ),
    (
        "undefined receiver is rejected",
        "String.prototype.split.call(undefined,',',1)",
    ),
    (
        "Symbol separator cannot take the ordinary ToString path",
        "'abc'.split(Symbol.iterator)",
    ),
    (
        "BigInt limit cannot be converted with ToUint32",
        "'a,b'.split(',',1n)",
    ),
    (
        "Symbol limit cannot be converted with ToUint32",
        "'a,b'.split(',',Symbol.iterator)",
    ),
    (
        "object-valued limit primitive conversion is rejected",
        r#"(function(){
            var limit=Object();limit[Symbol.toPrimitive]=function(){return Object()};
            return "a,b".split(",",limit);
        })()"#,
    ),
    (
        "object-valued separator primitive conversion is rejected",
        r#"(function(){
            var separator=Object();separator[Symbol.toPrimitive]=function(){return Object()};
            return "a,b".split(separator,2);
        })()"#,
    ),
    (
        "noncallable @@split is rejected",
        r#"(function(){var separator=Object();separator[Symbol.split]=17;return "a".split(separator)})()"#,
    ),
    ("split is not a constructor", "new String.prototype.split()"),
];

const RECURSION_CASES: &[(&str, &str)] = &[
    (
        "recursive receiver conversion is catchable and the runtime recovers",
        r#"(function(){
            var receiver=Object(),name="";
            receiver.toString=function(){return String.prototype.split.call(receiver,",",1)};
            try{String.prototype.split.call(receiver,",",1)}catch(error){name=error.name}
            return name+"|"+__arrayUnits("a,b".split(","));
        })()"#,
    ),
    (
        "recursive limit conversion is catchable and the runtime recovers",
        r#"(function(){
            var limit=Object(),name="";
            limit.valueOf=function(){return "a,b".split(",",limit)};
            try{"a,b".split(",",limit)}catch(error){name=error.name}
            return name+"|"+__arrayUnits("x-y".split("-",1));
        })()"#,
    ),
    (
        "recursive @@split delegation is catchable and the runtime recovers",
        r#"(function(){
            var separator=Object(),name="";
            separator[Symbol.split]=function(source,limit){return source.split(separator,limit)};
            try{"a,b".split(separator,2)}catch(error){name=error.name}
            return name+"|"+__arrayUnits("x-y".split("-"));
        })()"#,
    ),
];

#[test]
fn string_split_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String split oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("AutoInit", AUTOINIT_CASES),
        ("delegation", DELEGATION_CASES),
        ("values", VALUE_CASES),
        ("UTF-16", UTF16_CASES),
        ("limits", LIMIT_CASES),
        ("order", ORDER_CASES),
        ("recursion", RECURSION_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|"),
                "{group} oracle vector unexpectedly threw for {description}: {observation:?}",
            );
        }
    }
    for &(description, source) in ERROR_CASES {
        let observation = observe_oracle(&oracle, source, description);
        assert!(
            observation.starts_with("throw|"),
            "error oracle vector unexpectedly returned for {description}: {observation:?}",
        );
    }
}

#[test]
fn string_split_graph_and_autoinit_match_pinned_quickjs() {
    compare_cases("String split graph", GRAPH_CASES);
    compare_cases("String split AutoInit", AUTOINIT_CASES);
}

#[test]
fn string_split_symbol_delegation_matches_pinned_quickjs() {
    compare_cases("String split @@split delegation", DELEGATION_CASES);
}

#[test]
fn string_split_values_utf16_and_lone_surrogates_match_pinned_quickjs() {
    compare_cases("String split values", VALUE_CASES);
    compare_cases("String split UTF-16", UTF16_CASES);
}

#[test]
fn string_split_limits_and_conversion_order_match_pinned_quickjs() {
    compare_cases("String split limits", LIMIT_CASES);
    compare_cases("String split conversion order", ORDER_CASES);
}

#[test]
fn string_split_errors_and_nonconstructor_match_pinned_quickjs() {
    compare_cases("String split errors", ERROR_CASES);
}

#[test]
fn string_split_recursion_is_catchable_and_runtime_recovers() {
    compare_cases("String split recursion", RECURSION_CASES);
}

#[test]
fn string_split_cross_realm_results_errors_and_user_throws_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_string_prototype = defining.string_prototype().unwrap();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let caller_array_prototype = caller.array_prototype().unwrap();
    let split = property_callable(&runtime, &mut defining, &defining_string_prototype, "split");
    assert_eq!(
        runtime.get_prototype_of(split.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap()),
    );

    let Value::Object(result) = caller
        .call(
            &split,
            Value::String(JsString::try_from_utf8("a,b,c").unwrap()),
            &[
                Value::String(JsString::try_from_utf8(",").unwrap()),
                Value::Int(2),
            ],
        )
        .expect("cross-realm String.split call")
    else {
        panic!("cross-realm String.split result was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype),
        "String.split result did not use the method defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(caller_array_prototype),
    );
    assert_eq!(int_property(&runtime, &mut caller, &result, "length"), 2);
    assert_eq!(string_property(&runtime, &mut caller, &result, "0"), "a",);
    assert_eq!(string_property(&runtime, &mut caller, &result, "1"), "b",);

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_eq!(
        caller.call(&split, Value::Null, &[]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(
                &mut caller,
                "null receiver TypeError"
            ))
            .unwrap(),
        Some(defining_type_error.clone()),
        "String.split native TypeError did not use its defining realm",
    );

    let noncallable_separator = eval_object(
        &mut caller,
        r#"(function(){var separator=Object();separator[Symbol.split]=17;return separator})()"#,
        "caller noncallable @@split",
    );
    assert_eq!(
        caller.call(
            &split,
            Value::String(JsString::try_from_utf8("a,b").unwrap()),
            &[Value::Object(noncallable_separator)],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(
                &mut caller,
                "noncallable @@split TypeError",
            ))
            .unwrap(),
        Some(defining_type_error),
        "noncallable @@split TypeError did not use split's defining realm",
    );

    let separator = eval_object(
        &mut caller,
        r#"(function(){
            var separator=Object();
            Object.defineProperty(separator,Symbol.split,{get:function(){
                throw new TypeError("caller splitter getter");
            }});
            return separator;
        })()"#,
        "caller throwing @@split getter",
    );
    assert_eq!(
        caller.call(
            &split,
            Value::String(JsString::try_from_utf8("a,b").unwrap()),
            &[Value::Object(separator)],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(
                &mut caller,
                "caller @@split getter error"
            ))
            .unwrap(),
        Some(caller_type_error),
        "String.split replaced a user throw with a defining-realm error",
    );
}

#[test]
fn detached_string_split_and_result_retain_then_release_their_defining_realm() {
    let runtime = Runtime::new();
    let split = {
        let mut defining = runtime.new_context();
        let prototype = defining.string_prototype().unwrap();
        property_callable(&runtime, &mut defining, &prototype, "split")
    };
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        1,
        "detached split must retain its defining realm",
    );

    let result = {
        let mut caller = runtime.new_context();
        let Value::Object(result) = caller
            .call(
                &split,
                Value::String(JsString::try_from_utf8("x-y").unwrap()),
                &[Value::String(JsString::try_from_utf8("-").unwrap())],
            )
            .expect("detached split call")
        else {
            panic!("detached split result was not an object");
        };
        assert_eq!(int_property(&runtime, &mut caller, &result, "length"), 2);
        result
    };
    drop(split);
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        1,
        "split result must retain its defining Array realm",
    );
    drop(result);
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().live,
        0,
        "split callable, result, and defining realm must be collectable",
    );
}

fn compare_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let actual = observe_rust_eval(&runtime, &mut context, source, description);
        let expected = observe_oracle(&oracle, source, description);
        if actual != expected {
            failures.push(format!(
                "{description}\nsource: {source:?}\noxide: {actual:?}\noracle: {expected:?}",
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{group} drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn observed_source(source: &str) -> String {
    format!("{PRELUDE}\n{source}")
}

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    let source = observed_source(source);
    match context.eval(&source) {
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
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
    let source = observed_source(source);
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
        .args(["--std", "-e", wrapper, &source])
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

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("evaluate {description}: {error}"))
    else {
        panic!("{description} was not an object");
    };
    object
}

fn take_exception_object(context: &mut Context, description: &str) -> ObjectRef {
    let Some(Value::Object(error)) = context
        .take_exception()
        .unwrap_or_else(|failure| panic!("take {description}: {failure}"))
    else {
        panic!("{description} was not an object");
    };
    error
}

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let Value::Int(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an Int property");
    };
    value
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
        Value::Symbol(_) => "symbol",
        Value::Object(object) => {
            if runtime.as_callable(object).unwrap().is_some() {
                "function"
            } else {
                "object"
            }
        }
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
        Value::Symbol(_) => "<symbol>".to_owned(),
        Value::Object(_) => "<object>".to_owned(),
    }
}
