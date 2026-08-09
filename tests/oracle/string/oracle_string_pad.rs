use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, JsString, ObjectRef, OrdinaryPropertyDescriptor,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_string_pad` (quickjs.c 46073-46142), the
// `padEnd`/`padStart` GenericMagic entries and selectors (46647-46648),
// `JS_ToInt32SatFree`/`JS_ToInt32Sat` (13124-13172), and the 30-bit String
// length cap (212). QuickJS uses magic 1 for padEnd and 0 for padStart.
//
// The exact ordering matters: convert the receiver, saturate the target length,
// return immediately when the source is already long enough, then convert the
// optional filler. An empty converted filler also returns before the length-cap
// check. Padding repeats and truncates raw UTF-16 code units, so it may retain
// lone surrogates or split an astral pair.

const CASE_PRELUDE: &str = r#"
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    return (descriptor.writable?"1":"0")+
           (descriptor.enumerable?"1":"0")+
           (descriptor.configurable?"1":"0");
}
function __isConstructor(value){
    try{new value();return true}catch(_){return false}
}
function __units(value){
    value=String(value);
    var output="",index=0;
    while(index<value.length){
        var unit=value.charCodeAt(index).toString(16);
        while(unit.length<4)unit="0"+unit;
        if(index)output+=",";
        output+=unit;
        index++;
    }
    return output;
}
function __capture(callback){
    try{return "return:"+callback()}
    catch(error){
        if(error!==null&&typeof error==="object")return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
"#;

const GRAPH_CASES: &[(&str, &str)] = &[
    (
        "prototype own-key order places the pad pair after repeat",
        r#"(function(){
            var selected=["length","includes","endsWith","startsWith",
                "substring","substr","slice","repeat","padEnd","padStart",
                "toString","valueOf","constructor"];
            var keys=Object.getOwnPropertyNames(String.prototype),output=[],index=0;
            while(index<keys.length){
                if(selected.indexOf(keys[index])>=0)output.push(keys[index]);
                index++;
            }
            return output.join(",");
        })()"#,
    ),
    (
        "GenericMagic descriptors names lengths and metadata are exact",
        r#"(function(){
            var names=["padEnd","padStart"],output=[],index=0;
            while(index<names.length){
                var name=names[index],fn=String.prototype[name];
                output.push(name+":"+__bits(String.prototype,name)+":"+fn.name+":"+fn.length+
                    ":"+Object.getOwnPropertyNames(fn).join(",")+":"+
                    __bits(fn,"length")+":"+__bits(fn,"name")+":"+
                    (Object.getPrototypeOf(fn)===Function.prototype)+":"+
                    (typeof fn)+":"+__isConstructor(fn));
                index++;
            }
            return output.join("|");
        })()"#,
    ),
    (
        "AutoInit yields stable mutually distinct pad identities",
        r#"(function(){
            var end=String.prototype.padEnd,start=String.prototype.padStart;
            return [(end===String.prototype.padEnd),(start===String.prototype.padStart),
                (end===Object.getOwnPropertyDescriptor(String.prototype,"padEnd").value),
                (start===Object.getOwnPropertyDescriptor(String.prototype,"padStart").value),
                (end===start),(end===String.prototype.repeat),
                (start===String.prototype.repeat)].join("|");
        })()"#,
    ),
];

const AUTOINIT_CASES: &[(&str, &str)] = &[
    (
        "lazy padEnd can be deleted before materialization",
        r#"(function(){
            var deleted=delete String.prototype.padEnd;
            return [deleted,"padEnd" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"padEnd"),
                typeof String.prototype.padEnd].join("|");
        })()"#,
    ),
    (
        "lazy padStart assignment becomes an ordinary replacement",
        r#"(function(){
            String.prototype.padStart=17;
            return [String.prototype.padStart,__bits(String.prototype,"padStart"),
                Object.prototype.hasOwnProperty.call(String.prototype,"padStart")].join("|");
        })()"#,
    ),
    (
        "materialized padStart remains deletable",
        r#"(function(){
            var fn=String.prototype.padStart,deleted=delete String.prototype.padStart;
            return [typeof fn,deleted,"padStart" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"padStart")].join("|");
        })()"#,
    ),
];

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "default undefined and primitive targets use saturated truncation",
        r#"(function(){return [
            "abc".padEnd(),"abc".padStart(),
            "abc".padEnd(undefined),"abc".padStart(undefined),
            "abc".padEnd(null),"abc".padStart(false),
            "abc".padEnd(NaN),"abc".padStart(-0),
            "abc".padEnd(3.9),"abc".padStart(3.9),
            "abc".padEnd(4.9),"abc".padStart("5.9"),"done"
        ].join("|")})()"#,
    ),
    (
        "default empty one-unit and multi-unit fillers are exact",
        r#"(function(){return [
            "abc".padEnd(6),"abc".padStart(6),
            "abc".padEnd(7,undefined),"abc".padStart(7,undefined),
            "abc".padEnd(7,""),"abc".padStart(7,""),
            "abc".padEnd(7,"x"),"abc".padStart(7,"x"),
            "abc".padEnd(8,"xy"),"abc".padStart(8,"xy"),
            "abc".padEnd(8,7n),"abc".padStart(8,true),
            "abc".padEnd(8,null),"done"
        ].join("|")})()"#,
    ),
    (
        "fractions negative values and infinities preserve Int32Sat behavior",
        r#"(function(){return [
            "x".padEnd(-Infinity,"_"),"x".padStart(-2147483649,"_"),
            "x".padEnd(-1.9,"_"),"x".padStart(-0.9,"_"),
            "x".padEnd(1.9,"_"),"x".padStart(2.9,"_"),
            "x".padEnd(4.9,"ab"),"x".padStart(4.9,"ab"),
            "x".padEnd(Infinity,""),"x".padStart(Number.MAX_VALUE,""),
            "x".padEnd(2147483648,""),"x".padStart(4294967297,"")
        ].join("|")})()"#,
    ),
    (
        "signed 32-bit boundaries determine whether filler conversion occurs",
        r#"(function(){
            var points=[2147483647,2147483648,4294967297,Infinity,
                -2147483648,-2147483649,-4294967297,-Infinity],output=[],index=0;
            while(index<points.length){
                var hits=0,filler=Object();
                filler[Symbol.toPrimitive]=function(hint){hits++;return ""};
                output.push("x".padEnd(points[index],filler)+":"+hits);
                hits=0;
                output.push("x".padStart(points[index],filler)+":"+hits);
                index++;
            }
            return output.join("|");
        })()"#,
    ),
    (
        "generic primitive and object receivers are converted",
        r#"(function(){return [
            String.prototype.padEnd.call(123,5,"x"),
            String.prototype.padStart.call(true,6,"_"),
            String.prototype.padEnd.call(7n,4,"0"),
            String.prototype.padStart.call(Object(),17,"."),
            String.prototype.padEnd.call(new String("xy"),5,"z")
        ].join("|")})()"#,
    ),
    (
        "arguments after filler are completely ignored",
        r#"(function(){
            var extra=Object(),hits=0;
            extra[Symbol.toPrimitive]=function(){hits++;throw "extra"};
            return [String.prototype.padEnd.call("x",4,"_",extra,Symbol("later"),1n),
                String.prototype.padStart.call("x",4,"_",extra),hits].join("|");
        })()"#,
    ),
];

const UTF16_CASES: &[(&str, &str)] = &[
    (
        "astral fillers are truncated at raw UTF-16 code-unit boundaries",
        r#"(function(){
            var astral="\ud83d\ude00",end="X".padEnd(4,astral),start="X".padStart(4,astral);
            var source="A\ud800B\udc00",lone="\udc00Q\ud800";
            var loneEnd=source.padEnd(11,lone),loneStart=source.padStart(11,lone);
            return [end.length,__units(end),start.length,__units(start),
                __units("X".padEnd(2,astral)),__units("X".padStart(2,astral)),
                source.length,loneEnd.length,__units(loneEnd),
                loneStart.length,__units(loneStart)].join("|");
        })()"#,
    ),
    (
        "rope source and filler cross 8192-code-unit and truncation boundaries",
        r#"(function(){
            function grow(character,power){
                var value=character,index=0;
                while(index<power){value=value+value;index++}
                return value;
            }
            var source=(grow("a",13)+"\udc00")+"Z";
            var filler=grow("q",13)+"\ud83d\ude00";
            var target=source.length+8193;
            var end=source.padEnd(target,filler),start=source.padStart(target,filler);
            var join=source.length;
            return [source.length,filler.length,end.length,start.length,
                __units(end.slice(join-3,join+4)),__units(end.slice(-5)),
                __units(start.slice(8189,8197)),__units(start.slice(8190,8195)),
                __units(start.slice(8191,8196+source.length).slice(-5))].join("|");
        })()"#,
    ),
    (
        "large padding retains exact source and filler boundaries",
        r#"(function(){
            var source="",index=0;
            while(index<9000){source+=(index%2)?"x":"y";index++}
            source="\ud800"+source+"\udc00";
            var end=source.padEnd(source.length+9,"Q\ud83d\ude00");
            var start=source.padStart(source.length+9,"Q\ud83d\ude00");
            return [end.length,start.length,__units(end.slice(0,3)),
                __units(end.slice(source.length-2,source.length+5)),
                __units(end.slice(-4)),__units(start.slice(0,5)),
                __units(start.slice(7,12)),__units(start.slice(-3))].join("|");
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "receiver target then filler conversion uses string number string hints",
        r#"(function(){
            function run(name){
                var log="",receiver=Object(),target=Object(),filler=Object(),extra=Object();
                receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "x"};
                target[Symbol.toPrimitive]=function(hint){log+="target:"+hint+";";return 5.9};
                filler[Symbol.toPrimitive]=function(hint){log+="filler:"+hint+";";return "ab"};
                extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";throw "extra"};
                return String.prototype[name].call(receiver,target,filler,extra)+":"+log;
            }
            return [run("padEnd"),run("padStart")].join("|");
        })()"#,
    ),
    (
        "ordinary ToPrimitive fallback follows the two conversion hints",
        r#"(function(){
            var log="",receiver=Object(),target=Object(),filler=Object();
            receiver.toString=function(){log+="receiver:toString;";return "x"};
            receiver.valueOf=function(){log+="receiver:valueOf;";throw "wrong receiver order"};
            target.valueOf=function(){log+="target:valueOf;";return 4};
            target.toString=function(){log+="target:toString;";throw "wrong target order"};
            filler.toString=function(){log+="filler:toString;";return "_"};
            filler.valueOf=function(){log+="filler:valueOf;";throw "wrong filler order"};
            return String.prototype.padEnd.call(receiver,target,filler)+"|"+log;
        })()"#,
    ),
    (
        "receiver abrupt completion prevents target filler and extra conversion",
        r#"(function(){
            var log="",receiver=Object(),target=Object(),filler=Object(),extra=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";throw "receiver"};
            target[Symbol.toPrimitive]=function(hint){log+="target:"+hint+";";return 4};
            filler[Symbol.toPrimitive]=function(hint){log+="filler:"+hint+";";return "_"};
            extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";return "_"};
            try{String.prototype.padStart.call(receiver,target,filler,extra)}
            catch(error){return error+"|"+log}
            return "missing";
        })()"#,
    ),
    (
        "target abrupt completion follows receiver and prevents filler conversion",
        r#"(function(){
            var log="",receiver=Object(),target=Object(),filler=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "x"};
            target[Symbol.toPrimitive]=function(hint){log+="target:"+hint+";";throw "target"};
            filler[Symbol.toPrimitive]=function(hint){log+="filler:"+hint+";";return "_"};
            try{String.prototype.padEnd.call(receiver,target,filler)}
            catch(error){return error+"|"+log}
            return "missing";
        })()"#,
    ),
    (
        "len equal to target returns before filler conversion",
        r#"(function(){
            function run(name){
                var log="",target=Object(),filler=Object();
                target[Symbol.toPrimitive]=function(hint){log+="target:"+hint+";";return 6.9};
                filler[Symbol.toPrimitive]=function(hint){log+="filler:"+hint+";";throw "filler"};
                return String.prototype[name].call("abcdef",target,filler)+":"+log;
            }
            return [run("padEnd"),run("padStart")].join("|");
        })()"#,
    ),
    (
        "filler abrupt completion happens only after padding is required",
        r#"(function(){
            function run(name){
                var log="",target=Object(),filler=Object();
                target[Symbol.toPrimitive]=function(hint){log+="target:"+hint+";";return 4};
                filler[Symbol.toPrimitive]=function(hint){log+="filler:"+hint+";";throw "filler"};
                try{String.prototype[name].call("x",target,filler)}
                catch(error){return error+":"+log}
                return "missing";
            }
            return [run("padEnd"),run("padStart")].join("|");
        })()"#,
    ),
    (
        "empty filler returns after conversion and before cap validation",
        r#"(function(){
            function run(name){
                var log="",target=Object(),filler=Object(),extra=Object();
                target[Symbol.toPrimitive]=function(hint){log+="target:"+hint+";";return 2147483647};
                filler[Symbol.toPrimitive]=function(hint){log+="filler:"+hint+";";return ""};
                extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";throw "extra"};
                return String.prototype[name].call("x",target,filler,extra)+":"+log;
            }
            return [run("padEnd"),run("padStart")].join("|");
        })()"#,
    ),
    (
        "length-cap rejection occurs after nonempty filler conversion",
        r#"(function(){
            function run(name){
                var log="",target=Object(),filler=Object();
                target[Symbol.toPrimitive]=function(hint){log+="target:"+hint+";";return 2147483647};
                filler[Symbol.toPrimitive]=function(hint){log+="filler:"+hint+";";return "_"};
                try{String.prototype[name].call("x",target,filler)}
                catch(error){return error.name+":"+error.message+"|"+log}
                return "missing";
            }
            return [run("padEnd"),run("padStart")].join("|");
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "padEnd rejects null receiver before target conversion",
        "String.prototype.padEnd.call(null,4,'_')",
    ),
    (
        "padStart rejects undefined receiver",
        "String.prototype.padStart.call(undefined,4,'_')",
    ),
    (
        "padEnd rejects Symbol receiver",
        "String.prototype.padEnd.call(Symbol('receiver'),4,'_')",
    ),
    (
        "padStart rejects Symbol target",
        "'x'.padStart(Symbol('target'),'_')",
    ),
    ("padEnd rejects BigInt target", "'x'.padEnd(4n,'_')"),
    (
        "target ToPrimitive object result is rejected",
        r#"(function(){
            var target=Object();target[Symbol.toPrimitive]=function(){return Object()};
            return "x".padEnd(target,"_");
        })()"#,
    ),
    (
        "padEnd rejects Symbol filler when padding is required",
        "'x'.padEnd(4,Symbol('filler'))",
    ),
    (
        "padStart rejects Symbol filler when padding is required",
        "'x'.padStart(4,Symbol('filler'))",
    ),
    (
        "filler ToPrimitive object result is rejected",
        r#"(function(){
            var filler=Object();filler[Symbol.toPrimitive]=function(){return Object()};
            return "x".padStart(4,filler);
        })()"#,
    ),
    (
        "padEnd target above 30-bit cap has exact RangeError",
        "'x'.padEnd(1073741824,'_')",
    ),
    (
        "padEnd is not a constructor",
        "new String.prototype.padEnd()",
    ),
    (
        "padStart is not a constructor",
        "new String.prototype.padStart()",
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "recursive receiver conversion throws catchably and runtime recovers",
        r#"(function(){
            var receiver=Object(),errorName="",errorMessage="";
            receiver[Symbol.toPrimitive]=function(){
                return String.prototype.padEnd.call(receiver,2,"_")
            };
            try{String.prototype.padEnd.call(receiver,2,"_")}
            catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage,"x".padEnd(4,"_"),"x".padStart(4,"_")].join("|");
        })()"#,
    ),
    (
        "recursive target and filler conversions remain catchable",
        r#"(function(){
            var target=Object(),filler=Object(),targetError="",fillerError="";
            target[Symbol.toPrimitive]=function(){return "x".padStart(target,"_")};
            try{"x".padStart(target,"_")}catch(error){targetError=error.name}
            filler[Symbol.toPrimitive]=function(){return "x".padEnd(4,filler)};
            try{"x".padEnd(4,filler)}catch(error){fillerError=error.name}
            return [targetError,fillerError,"ok".padEnd(4,"!"),"ok".padStart(4,"!")].join("|");
        })()"#,
    ),
    (
        "pad variants repeat includes and subrange share one recursion guard",
        r#"(function(){
            var value=Object(),depth=0,errorName="",errorMessage="";
            value[Symbol.toPrimitive]=function(){
                depth++;
                if(depth%5===0)return "x".padEnd(value,"_");
                if(depth%5===1)return "x".padStart(value,"_");
                if(depth%5===2)return "x".repeat(value);
                if(depth%5===3)return "abcdef".slice(value,4);
                return "abcdef".includes("a",value);
            };
            try{"x".padEnd(value,"_")}
            catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage,"x".padEnd(3,"_"),"x".padStart(3,"_"),
                "ok".repeat(2),"abcdef".slice(1,3),"abcdef".includes("bc")].join("|");
        })()"#,
    ),
];

#[test]
fn string_pad_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String pad oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("AutoInit", AUTOINIT_CASES),
        ("values", VALUE_CASES),
        ("UTF-16", UTF16_CASES),
        ("order", ORDER_CASES),
        ("stack", STACK_CASES),
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
fn string_pad_graph_and_autoinit_match_pinned_quickjs() {
    compare_cases("String pad graph", GRAPH_CASES);
    compare_cases("String pad AutoInit", AUTOINIT_CASES);
}

#[test]
fn string_pad_values_int32_saturation_and_length_cap_match_pinned_quickjs() {
    compare_cases("String pad values", VALUE_CASES);
}

#[test]
fn string_pad_utf16_astral_truncation_and_ropes_match_pinned_quickjs() {
    compare_cases("String pad UTF-16", UTF16_CASES);
}

#[test]
fn string_pad_conversion_order_and_early_returns_match_pinned_quickjs() {
    compare_cases("String pad conversion order", ORDER_CASES);
}

#[test]
fn string_pad_errors_length_cap_and_nonconstructors_match_pinned_quickjs() {
    compare_cases("String pad errors", ERROR_CASES);
}

#[test]
fn string_pad_recursion_is_catchable_and_shared_family_recovers() {
    compare_cases("String pad stack recovery", STACK_CASES);
}

#[test]
fn string_pad_defining_realms_and_user_throw_identity_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let pad_end = property_callable(&runtime, &mut defining, &defining_prototype, "padEnd");
    let pad_start = property_callable(&runtime, &mut defining, &defining_prototype, "padStart");
    assert_eq!(
        runtime.get_prototype_of(pad_end.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap()),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let defining_range_error = intrinsic_prototype(&runtime, &mut defining, "RangeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_native_error(
        &runtime,
        &mut caller,
        &pad_end,
        Value::Null,
        &[
            Value::Int(4),
            Value::String(JsString::try_from_utf8("_").unwrap()),
        ],
        &defining_type_error,
    );
    let target_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("target").unwrap()))
        .unwrap();
    assert_native_error(
        &runtime,
        &mut caller,
        &pad_start,
        Value::String(JsString::try_from_utf8("x").unwrap()),
        &[Value::Symbol(target_symbol)],
        &defining_type_error,
    );
    let filler_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("filler").unwrap()))
        .unwrap();
    assert_native_error(
        &runtime,
        &mut caller,
        &pad_end,
        Value::String(JsString::try_from_utf8("x").unwrap()),
        &[Value::Int(4), Value::Symbol(filler_symbol)],
        &defining_type_error,
    );
    assert_native_error(
        &runtime,
        &mut caller,
        &pad_start,
        Value::String(JsString::try_from_utf8("x").unwrap()),
        &[
            Value::Int(i32::MAX),
            Value::String(JsString::try_from_utf8("_").unwrap()),
        ],
        &defining_range_error,
    );

    let sentinel = caller.new_object().unwrap();
    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "padSentinel",
        Value::Object(sentinel.clone()),
    );
    let throwing_target = caller
        .eval(
            r#"(function(){
                var value=Object();
                value[Symbol.toPrimitive]=function(){throw padSentinel};
                return value;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        caller.call(
            &pad_end,
            Value::String(JsString::try_from_utf8("x").unwrap()),
            &[
                throwing_target,
                Value::String(JsString::try_from_utf8("_").unwrap()),
            ],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel.clone())),
        "target conversion did not preserve the user-thrown value",
    );

    let throwing_filler = caller
        .eval(
            r#"(function(){
                var value=Object();
                value[Symbol.toPrimitive]=function(){throw padSentinel};
                return value;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        caller.call(
            &pad_start,
            Value::String(JsString::try_from_utf8("x").unwrap()),
            &[Value::Int(4), throwing_filler],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "filler conversion did not preserve the user-thrown value",
    );

    assert_eq!(
        caller.construct(&pad_end, &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut caller))
            .unwrap(),
        Some(caller_type_error),
        "non-constructor rejection did not use the caller realm",
    );
}

#[test]
fn string_pad_callables_are_per_realm_distinct_and_collectable() {
    let runtime = Runtime::new();
    let retained = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_prototype = first.string_prototype().unwrap();
        let second_prototype = second.string_prototype().unwrap();
        let first_end = property_callable(&runtime, &mut first, &first_prototype, "padEnd");
        let first_end_again = property_callable(&runtime, &mut first, &first_prototype, "padEnd");
        let first_start = property_callable(&runtime, &mut first, &first_prototype, "padStart");
        let first_repeat = property_callable(&runtime, &mut first, &first_prototype, "repeat");
        let second_end = property_callable(&runtime, &mut second, &second_prototype, "padEnd");
        let second_start = property_callable(&runtime, &mut second, &second_prototype, "padStart");
        assert_eq!(first_end, first_end_again);
        assert_ne!(first_end, first_start);
        assert_ne!(first_end, first_repeat);
        assert_ne!(first_start, first_repeat);
        assert_ne!(first_end, second_end);
        assert_ne!(first_start, second_start);
        assert_eq!(
            runtime.get_prototype_of(first_end.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        (first_end, first_start)
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(retained);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn string_pad_stack_overflow_uses_the_caller_realm_and_recovers() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let pad_end = property_callable(&runtime, &mut defining, &defining_prototype, "padEnd");
    let defining_internal_error = intrinsic_prototype(&runtime, &mut defining, "InternalError");
    let caller_internal_error = intrinsic_prototype(&runtime, &mut caller, "InternalError");
    assert_ne!(defining_internal_error, caller_internal_error);

    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "foreignPadEnd",
        Value::Object(pad_end.as_object().clone()),
    );
    let Value::Object(error) = caller
        .eval(
            r#"(function(){
                var target=Object(),localCall=Function.prototype.call;
                function invoke(){return localCall.call(foreignPadEnd,"x",target,"_")}
                target[Symbol.toPrimitive]=function(){return invoke()};
                try{invoke()}catch(error){return error}
                return Object();
            })()"#,
        )
        .unwrap()
    else {
        panic!("recursive cross-realm padEnd did not return an error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_internal_error),
        "pre-dispatch stack overflow did not use the caller realm",
    );
    assert_eq!(
        caller
            .call(
                &pad_end,
                Value::String(JsString::try_from_utf8("x").unwrap()),
                &[
                    Value::Int(4),
                    Value::String(JsString::try_from_utf8("_").unwrap()),
                ],
            )
            .unwrap(),
        Value::String(JsString::try_from_utf8("x___").unwrap()),
    );
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

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("pending exception was not an object");
    };
    error
}

fn assert_native_error(
    runtime: &Runtime,
    context: &mut Context,
    method: &CallableRef,
    this_value: Value,
    arguments: &[Value],
    expected_prototype: &ObjectRef,
) {
    assert_eq!(
        context.call(method, this_value, arguments),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(context))
            .unwrap()
            .as_ref(),
        Some(expected_prototype),
    );
}

fn define_data(runtime: &Runtime, object: &ObjectRef, name: &str, value: Value) {
    assert!(
        runtime
            .define_own_property(
                object,
                &runtime.intern_property_key(name).unwrap(),
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
