use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, JsString, ObjectRef, OrdinaryPropertyDescriptor,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_string_trim` (quickjs.c 46189-46213), its
// GenericMagic table entries and alias entries (46649-46653),
// `JS_ToStringCheckObject` (13670-13676), `js_sub_string` (3964-3990), and
// `lre_is_space`/`char_range_s` (libunicode.h 140-166; libunicode.c
// 1885-1917). The magic bitmask uses one for the leading end and two for the
// trailing end. `trimRight` copies the `trimEnd` function value and `trimLeft`
// copies the `trimStart` function value, so aliases share canonical callable
// identity and names initially while their writable properties mutate
// independently afterward.
//
// Raw UTF-16 observations encode code units as hexadecimal before crossing the
// process boundary, so lone surrogates never depend on terminal UTF-8 handling.

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
        "prototype own-key order places all five trim properties after padding",
        r#"(function(){
            var selected=["repeat","padEnd","padStart","trim","trimEnd",
                "trimRight","trimStart","trimLeft","toString","valueOf","constructor"];
            var keys=Object.getOwnPropertyNames(String.prototype),output=[],index=0;
            while(index<keys.length){
                if(selected.indexOf(keys[index])>=0)output.push(keys[index]);
                index++;
            }
            return output.join(",");
        })()"#,
    ),
    (
        "five descriptors and canonical callable metadata are exact",
        r#"(function(){
            var names=["trim","trimEnd","trimRight","trimStart","trimLeft"];
            var output=[],index=0;
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
        "canonical methods are stable and aliases initially share their callable",
        r#"(function(){
            var trim=String.prototype.trim,end=String.prototype.trimEnd;
            var right=String.prototype.trimRight,start=String.prototype.trimStart;
            var left=String.prototype.trimLeft;
            return [trim===String.prototype.trim,
                end===String.prototype.trimEnd,right===String.prototype.trimRight,
                start===String.prototype.trimStart,left===String.prototype.trimLeft,
                end===right,start===left,trim===end,trim===start,end===start,
                trim===Object.getOwnPropertyDescriptor(String.prototype,"trim").value,
                right===Object.getOwnPropertyDescriptor(String.prototype,"trimRight").value,
                left===Object.getOwnPropertyDescriptor(String.prototype,"trimLeft").value,
                right.name,left.name].join("|");
        })()"#,
    ),
];

const PROPERTY_CASES: &[(&str, &str)] = &[
    (
        "lazy trim can be deleted before materialization",
        r#"(function(){
            var deleted=delete String.prototype.trim;
            return [deleted,"trim" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"trim"),
                typeof String.prototype.trim].join("|");
        })()"#,
    ),
    (
        "lazy trim assignment becomes an ordinary replacement",
        r#"(function(){
            String.prototype.trim=17;
            return [String.prototype.trim,__bits(String.prototype,"trim"),
                Object.prototype.hasOwnProperty.call(String.prototype,"trim")].join("|");
        })()"#,
    ),
    (
        "trimEnd and trimRight properties mutate independently in both directions",
        r#"(function(){
            var originalEnd=String.prototype.trimEnd;
            var originalRight=String.prototype.trimRight;
            String.prototype.trimEnd=17;
            var rightSurvived=String.prototype.trimRight===originalRight;
            String.prototype.trimRight=23;
            var endStayed=String.prototype.trimEnd===17;
            var deleted=delete String.prototype.trimEnd;
            return [originalEnd===originalRight,rightSurvived,endStayed,deleted,
                Object.prototype.hasOwnProperty.call(String.prototype,"trimEnd"),
                String.prototype.trimRight,__bits(String.prototype,"trimRight")].join("|");
        })()"#,
    ),
    (
        "trimStart and trimLeft properties mutate independently in both directions",
        r#"(function(){
            var originalStart=String.prototype.trimStart;
            var originalLeft=String.prototype.trimLeft;
            String.prototype.trimLeft=29;
            var startSurvived=String.prototype.trimStart===originalStart;
            String.prototype.trimStart=31;
            var leftStayed=String.prototype.trimLeft===29;
            var deleted=delete String.prototype.trimLeft;
            return [originalStart===originalLeft,startSurvived,leftStayed,deleted,
                Object.prototype.hasOwnProperty.call(String.prototype,"trimLeft"),
                String.prototype.trimStart,__bits(String.prototype,"trimStart")].join("|");
        })()"#,
    ),
];

const WHITESPACE_CASES: &[(&str, &str)] = &[
    (
        "all twenty-five pinned whitespace code units trim at either end",
        r#"(function(){
            var spaces=["\u0009","\u000a","\u000b","\u000c","\u000d","\u0020",
                "\u00a0","\u1680","\u2000","\u2001","\u2002","\u2003","\u2004",
                "\u2005","\u2006","\u2007","\u2008","\u2009","\u200a","\u2028",
                "\u2029","\u202f","\u205f","\u3000","\ufeff"];
            var output=[spaces.length],index=0;
            while(index<spaces.length){
                var unit=spaces[index],sample=unit+"X"+unit;
                output.push(unit.charCodeAt(0).toString(16)+":"+
                    (sample.trim()==="X")+":"+
                    (sample.trimStart()==="X"+unit)+":"+
                    (sample.trimEnd()===unit+"X"));
                index++;
            }
            return output.join("|");
        })()"#,
    ),
    (
        "Unicode and format near-misses are not treated as whitespace",
        r#"(function(){
            var values=["\u0085","\u180e","\u200b","\u200c","\u200d","\u2060",
                "\u0000","\u001c","\ufff9"];
            var output=[values.length],index=0;
            while(index<values.length){
                var unit=values[index],sample=unit+"X"+unit;
                output.push(unit.charCodeAt(0).toString(16)+":"+
                    __units(sample.trim())+":"+__units(sample.trimStart())+":"+
                    __units(sample.trimEnd()));
                index++;
            }
            return output.join("|");
        })()"#,
    ),
    (
        "trim variants remove only their selected outer runs",
        r#"(function(){
            var source="\u0020\u0009A\u00a0B\u000a\u3000";
            return [__units(source),__units(source.trim()),
                __units(source.trimStart()),__units(source.trimEnd()),
                __units(source.trimLeft()),__units(source.trimRight())].join("|");
        })()"#,
    ),
    (
        "empty no-space and all-space inputs retain exact values",
        r#"(function(){
            var all="\u0009\u0020\u00a0\u1680\u2000\u200a\u2028\u2029\u202f\u205f\u3000\ufeff";
            return ["".trim().length,"plain".trim(),"plain".trimStart(),
                "plain".trimEnd(),all.trim().length,all.trimStart().length,
                all.trimEnd().length,all.trimLeft().length,all.trimRight().length].join("|");
        })()"#,
    ),
    (
        "internal whitespace is preserved after both outer scans",
        r#"(function(){
            var value="\u0009A\u0020\u00a0\u2028B\u000d";
            return [__units(value.trim()),__units(value.trimStart()),
                __units(value.trimEnd())].join("|");
        })()"#,
    ),
];

const UTF16_CASES: &[(&str, &str)] = &[
    (
        "lone surrogates and astral pairs remain exact raw UTF-16 units",
        r#"(function(){
            var value="\u0020\ud800\ud83d\ude00\udc00\u3000";
            var guarded="\u0020\ud800\u0020X\u3000\udc00\u0020";
            return [value.length,__units(value.trim()),__units(value.trimStart()),
                __units(value.trimEnd()),__units(guarded.trim()),
                __units(guarded.trimStart()),__units(guarded.trimEnd())].join("|");
        })()"#,
    ),
    (
        "rope scans cross 8192-code-unit boundaries at both ends",
        r#"(function(){
            function grow(character,power){
                var value=character,index=0;
                while(index<power){value=value+value;index++}
                return value;
            }
            var left=grow("\u0020",13),right=grow("\u3000",13);
            var middle="\ud800A\ud83d\ude00B\udc00";
            var source=(left+middle)+right;
            var both=source.trim(),start=source.trimStart(),end=source.trimEnd();
            return [left.length,middle.length,right.length,source.length,
                both.length,__units(both),start.length,
                __units(start.slice(0,middle.length+2)),__units(start.slice(-3)),
                end.length,__units(end.slice(0,3)),__units(end.slice(-middle.length))].join("|");
        })()"#,
    ),
    (
        "alternating rope leaves stop on the first interior non-space unit",
        r#"(function(){
            function grow(character,power){
                var value=character,index=0;
                while(index<power){value=value+value;index++}
                return value;
            }
            var a=grow("\u00a0",12),b=grow("\ufeff",12);
            var source=((a+b)+"X\ud800\u0020Y\udc00")+(b+a);
            var result=source.trim();
            return [source.length,result.length,__units(result),
                __units(source.trimStart().slice(0,7)),
                __units(source.trimEnd().slice(-7))].join("|");
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "generic receivers use a string hint and every extra argument is ignored",
        r#"(function(){
            function run(name){
                var log="",hits=0,receiver=Object(),extra=Object();
                receiver[Symbol.toPrimitive]=function(hint){
                    log+="receiver:"+hint+";";return "\u0020X\u000a"
                };
                extra[Symbol.toPrimitive]=function(hint){hits++;throw "extra:"+hint};
                var result=String.prototype[name].call(receiver,extra,extra,Symbol("later"),1n);
                return name+":"+__units(result)+":"+log+":"+hits;
            }
            return [run("trim"),run("trimEnd"),run("trimRight"),
                run("trimStart"),run("trimLeft")].join("|");
        })()"#,
    ),
    (
        "ordinary ToPrimitive fallback calls toString before valueOf",
        r#"(function(){
            var log="",receiver=Object(),extra=Object();
            receiver.toString=function(){log+="toString;";return "\u0020X\u0020"};
            receiver.valueOf=function(){log+="valueOf;";throw "wrong order"};
            extra[Symbol.toPrimitive]=function(){log+="extra;";throw "extra"};
            return String.prototype.trim.call(receiver,extra)+"|"+log;
        })()"#,
    ),
    (
        "receiver abrupt completion preserves identity and prevents extra conversion",
        r#"(function(){
            function run(name){
                var sentinel=Object(),receiver=Object(),extra=Object(),hits=0;
                receiver[Symbol.toPrimitive]=function(hint){hits++;throw sentinel};
                extra[Symbol.toPrimitive]=function(hint){hits+=100;throw extra};
                try{String.prototype[name].call(receiver,extra)}
                catch(error){return name+":"+(error===sentinel)+":"+hits}
                return "missing";
            }
            return [run("trim"),run("trimEnd"),run("trimStart")].join("|");
        })()"#,
    ),
    (
        "a Symbol.toPrimitive Symbol result fails after the single receiver conversion",
        r#"(function(){
            var log="",receiver=Object(),extra=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return Symbol("result")};
            extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";throw "extra"};
            try{String.prototype.trimEnd.call(receiver,extra)}
            catch(error){return error.name+":"+error.message+"|"+log}
            return "missing";
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "trim rejects a null receiver with the exact TypeError",
        "String.prototype.trim.call(null)",
    ),
    (
        "trimEnd rejects an undefined receiver with the exact TypeError",
        "String.prototype.trimEnd.call(undefined)",
    ),
    (
        "trimStart rejects a Symbol receiver with the exact TypeError",
        "String.prototype.trimStart.call(Symbol('receiver'))",
    ),
    (
        "trimRight alias rejects a null receiver",
        "String.prototype.trimRight.call(null)",
    ),
    (
        "trimLeft alias rejects an undefined receiver",
        "String.prototype.trimLeft.call(undefined)",
    ),
    (
        "receiver ToPrimitive returning an object is rejected",
        r#"(function(){
            var receiver=Object();receiver[Symbol.toPrimitive]=function(){return Object()};
            return String.prototype.trim.call(receiver);
        })()"#,
    ),
    ("trim is not a constructor", "new String.prototype.trim()"),
    (
        "trimEnd is not a constructor",
        "new String.prototype.trimEnd()",
    ),
    (
        "trimRight alias is not a constructor",
        "new String.prototype.trimRight()",
    ),
    (
        "trimStart is not a constructor",
        "new String.prototype.trimStart()",
    ),
    (
        "trimLeft alias is not a constructor",
        "new String.prototype.trimLeft()",
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "recursive receiver conversion throws catchably and runtime recovers",
        r#"(function(){
            var receiver=Object(),errorName="",errorMessage="";
            receiver[Symbol.toPrimitive]=function(){
                return String.prototype.trim.call(receiver)
            };
            try{String.prototype.trim.call(receiver)}
            catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage," x ".trim()," x ".trimStart(),
                " x ".trimEnd()].join("|");
        })()"#,
    ),
    (
        "trim aliases and existing String methods share one recursion guard",
        r#"(function(){
            var value=Object(),depth=0,errorName="",errorMessage="";
            value[Symbol.toPrimitive]=function(){
                depth++;
                if(depth%8===0)return String.prototype.trim.call(value);
                if(depth%8===1)return String.prototype.trimEnd.call(value);
                if(depth%8===2)return String.prototype.trimRight.call(value);
                if(depth%8===3)return String.prototype.trimStart.call(value);
                if(depth%8===4)return "x".padEnd(value,"_");
                if(depth%8===5)return "x".repeat(value);
                if(depth%8===6)return "abcdef".slice(value,4);
                return "abcdef".includes("a",value);
            };
            try{String.prototype.trimLeft.call(value)}
            catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage," x ".trim()," x ".trimRight(),
                " x ".trimLeft(),"x".padEnd(3,"_"),"ok".repeat(2),
                "abcdef".slice(1,3),"abcdef".includes("bc")].join("|");
        })()"#,
    ),
];

#[test]
fn string_trim_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String trim oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("properties", PROPERTY_CASES),
        ("whitespace", WHITESPACE_CASES),
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
fn string_trim_graph_descriptors_aliases_and_properties_match_pinned_quickjs() {
    compare_cases("String trim graph", GRAPH_CASES);
    compare_cases("String trim property behavior", PROPERTY_CASES);
}

#[test]
fn string_trim_exact_whitespace_set_and_nonspace_boundaries_match_pinned_quickjs() {
    compare_cases("String trim whitespace", WHITESPACE_CASES);
}

#[test]
fn string_trim_utf16_lone_surrogates_astral_and_ropes_match_pinned_quickjs() {
    compare_cases("String trim UTF-16", UTF16_CASES);
}

#[test]
fn string_trim_receiver_conversion_order_and_abrupt_identity_match_pinned_quickjs() {
    compare_cases("String trim conversion order", ORDER_CASES);
}

#[test]
fn string_trim_errors_and_nonconstructors_match_pinned_quickjs() {
    compare_cases("String trim errors", ERROR_CASES);
}

#[test]
fn string_trim_recursion_is_catchable_shared_and_recovers() {
    compare_cases("String trim stack recovery", STACK_CASES);
}

#[test]
fn string_trim_defining_realms_user_throw_identity_and_caller_construct_error_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let trim = property_callable(&runtime, &mut defining, &defining_prototype, "trim");
    let trim_end = property_callable(&runtime, &mut defining, &defining_prototype, "trimEnd");
    let trim_right = property_callable(&runtime, &mut defining, &defining_prototype, "trimRight");
    let trim_start = property_callable(&runtime, &mut defining, &defining_prototype, "trimStart");
    assert_eq!(trim_end, trim_right);
    assert_eq!(
        runtime.get_prototype_of(trim.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap()),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_native_error(
        &runtime,
        &mut caller,
        &trim,
        Value::Null,
        &[],
        &defining_type_error,
    );
    let receiver_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("receiver").unwrap()))
        .unwrap();
    assert_native_error(
        &runtime,
        &mut caller,
        &trim_start,
        Value::Symbol(receiver_symbol),
        &[],
        &defining_type_error,
    );

    let sentinel = caller.new_object().unwrap();
    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "trimSentinel",
        Value::Object(sentinel.clone()),
    );
    let throwing_receiver = caller
        .eval(
            r#"(function(){
                var value=Object();
                value[Symbol.toPrimitive]=function(){throw trimSentinel};
                return value;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        caller.call(&trim_end, throwing_receiver, &[]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "receiver conversion did not preserve the user-thrown value",
    );

    assert_eq!(
        caller.construct(&trim_right, &[]),
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
fn string_trim_callables_aliases_are_per_realm_distinct_and_collectable() {
    let runtime = Runtime::new();
    let retained = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_prototype = first.string_prototype().unwrap();
        let second_prototype = second.string_prototype().unwrap();
        let first_trim = property_callable(&runtime, &mut first, &first_prototype, "trim");
        let first_trim_again = property_callable(&runtime, &mut first, &first_prototype, "trim");
        let first_end = property_callable(&runtime, &mut first, &first_prototype, "trimEnd");
        let first_right = property_callable(&runtime, &mut first, &first_prototype, "trimRight");
        let first_start = property_callable(&runtime, &mut first, &first_prototype, "trimStart");
        let first_left = property_callable(&runtime, &mut first, &first_prototype, "trimLeft");
        let second_trim = property_callable(&runtime, &mut second, &second_prototype, "trim");
        let second_end = property_callable(&runtime, &mut second, &second_prototype, "trimEnd");
        let second_right = property_callable(&runtime, &mut second, &second_prototype, "trimRight");
        let second_start = property_callable(&runtime, &mut second, &second_prototype, "trimStart");
        let second_left = property_callable(&runtime, &mut second, &second_prototype, "trimLeft");
        assert_eq!(first_trim, first_trim_again);
        assert_eq!(first_end, first_right);
        assert_eq!(first_start, first_left);
        assert_eq!(second_end, second_right);
        assert_eq!(second_start, second_left);
        assert_ne!(first_trim, first_end);
        assert_ne!(first_trim, first_start);
        assert_ne!(first_end, first_start);
        assert_ne!(first_trim, second_trim);
        assert_ne!(first_end, second_end);
        assert_ne!(first_start, second_start);
        assert_eq!(
            runtime.get_prototype_of(first_end.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        (first_trim, first_end, first_start)
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(retained);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn string_trim_stack_overflow_uses_the_caller_realm_and_recovers() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let trim = property_callable(&runtime, &mut defining, &defining_prototype, "trim");
    let defining_internal_error = intrinsic_prototype(&runtime, &mut defining, "InternalError");
    let caller_internal_error = intrinsic_prototype(&runtime, &mut caller, "InternalError");
    assert_ne!(defining_internal_error, caller_internal_error);

    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "foreignTrim",
        Value::Object(trim.as_object().clone()),
    );
    let Value::Object(error) = caller
        .eval(
            r#"(function(){
                var receiver=Object(),localCall=Function.prototype.call;
                function invoke(){return localCall.call(foreignTrim,receiver)}
                receiver[Symbol.toPrimitive]=function(){return invoke()};
                try{invoke()}catch(error){return error}
                return Object();
            })()"#,
        )
        .unwrap()
    else {
        panic!("recursive cross-realm trim did not return an error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_internal_error),
        "pre-dispatch stack overflow did not use the caller realm",
    );
    assert_eq!(
        caller
            .call(
                &trim,
                Value::String(JsString::try_from_utf8("  x  ").unwrap()),
                &[],
            )
            .unwrap(),
        Value::String(JsString::try_from_utf8("x").unwrap()),
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
