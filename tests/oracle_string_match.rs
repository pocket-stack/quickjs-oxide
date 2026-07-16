use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04 `js_string_match`'s
// Symbol.match branch (`quickjs.c` 45609-45657), abstract RegExpExec
// (`quickjs.c` 48217-48236), and RegExp.prototype[Symbol.match]
// (`quickjs.c` 48252-48333).

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
function __show(value){
    if(value===undefined)return "undefined";
    if(value===null)return "null";
    if(typeof value==="number"){
        if(value!==value)return "NaN";
        if(value===0)return 1/value===-Infinity?"-0":"+0";
    }
    return String(value);
}
function __completion(callback){
    try{return "return:"+String(callback())}
    catch(error){
        if(error!==null&&typeof error==="object")
            return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
function __array(value){
    if(value===null)return "null";
    var output=[],index=0;
    while(index<value.length){output[index]=String(value[index]);index++}
    return value.length+"["+output.join(",")+"]";
}
"#;

const METADATA_CASES: &[(&str, &str)] = &[
    (
        "match methods expose pinned metadata descriptors and filtered key order",
        r#"(function(){
            var stringFn=String.prototype.match,
                regexpFn=RegExp.prototype[Symbol.match],
                stringKeys=Object.getOwnPropertyNames(String.prototype),
                regexpKeys=Reflect.ownKeys(RegExp.prototype),
                selected=["startsWith","match","search","split"],
                stringOrder=[],regexpOrder=[],index=0,key;
            while(index<stringKeys.length){
                key=stringKeys[index];
                if(selected.indexOf(key)>=0)stringOrder[stringOrder.length]=key;
                index++;
            }
            index=0;
            while(index<regexpKeys.length){
                key=regexpKeys[index];
                if(key==="test"||key==="toString"||key===Symbol.match||key===Symbol.search)
                    regexpOrder[regexpOrder.length]=String(key);
                index++;
            }
            return [
                stringOrder.join(","),regexpOrder.join(","),
                __bits(String.prototype,"match"),stringFn.name,stringFn.length,
                Object.getOwnPropertyNames(stringFn).join(","),
                __bits(stringFn,"name"),__bits(stringFn,"length"),
                __isConstructor(stringFn),
                Object.prototype.hasOwnProperty.call(stringFn,"prototype"),
                __bits(RegExp.prototype,Symbol.match),regexpFn.name,regexpFn.length,
                Object.getOwnPropertyNames(regexpFn).join(","),
                __bits(regexpFn,"name"),__bits(regexpFn,"length"),
                __isConstructor(regexpFn),
                Object.prototype.hasOwnProperty.call(regexpFn,"prototype")
            ].join("|");
        })()"#,
    ),
    (
        "match AutoInit entries can be replaced deleted and materialized independently",
        r#"(function(){
            var original=String.prototype.match,
                regexpOriginal=RegExp.prototype[Symbol.match],
                stable=original===String.prototype.match&&
                    regexpOriginal===RegExp.prototype[Symbol.match],
                stringDeleted=delete String.prototype.match,
                regexpDeleted=delete RegExp.prototype[Symbol.match];
            String.prototype.match=17;
            RegExp.prototype[Symbol.match]=23;
            return [stable,stringDeleted,regexpDeleted,String.prototype.match,
                RegExp.prototype[Symbol.match],__bits(String.prototype,"match"),
                __bits(RegExp.prototype,Symbol.match),original===regexpOriginal].join("|");
        })()"#,
    ),
];

const STRING_DISPATCH_CASES: &[(&str, &str)] = &[
    (
        "String match delegates before conversion and returns the custom result unchanged",
        r#"(function(){
            var log="",receiver=Object(),regexp=Object(),result=Object();
            receiver.toString=function(){log+="BAD-receiver-string;";throw "converted"};
            Object.defineProperty(regexp,Symbol.match,{get:function(){
                log+="get;";
                return function(value){
                    log+="call:"+(this===regexp)+":"+(value===receiver)+":"+
                        arguments.length+";";
                    return result;
                };
            }});
            var actual=String.prototype.match.call(receiver,regexp);
            return (actual===result)+"|"+log;
        })()"#,
    ),
    (
        "String match rejects nullish and noncallable hooks before conversion",
        r#"(function(){
            var log="",poison=Object(),receiver=Object(),noncallable=Object();
            Object.defineProperty(poison,Symbol.match,{get:function(){log+="BAD-nullish;";return null}});
            var first=__completion(function(){return String.prototype.match.call(null,poison)}),
                second=__completion(function(){return String.prototype.match.call(undefined,poison)});
            receiver.toString=function(){log+="BAD-string;";return "a"};
            Object.defineProperty(noncallable,Symbol.match,{get:function(){log+="hook-get;";return 17}});
            var invalid=__completion(function(){return String.prototype.match.call(receiver,noncallable)});
            return [first,second,invalid,log].join("|");
        })()"#,
    ),
    (
        "primitive patterns bypass boxed prototype Symbol.match hooks",
        r#"(function(){
            var hits="",output=[];
            function poison(prototype,label){
                Object.defineProperty(prototype,Symbol.match,{configurable:true,get:function(){
                    hits+=label;throw label;
                }});
            }
            poison(String.prototype,"string;");
            poison(Number.prototype,"number;");
            poison(Boolean.prototype,"boolean;");
            poison(BigInt.prototype,"bigint;");
            output[0]=__array(String.prototype.match.call("a7b",7));
            output[1]=__array(String.prototype.match.call("atrueb",true));
            output[2]=__array(String.prototype.match.call("a9b",9n));
            return output.join("|")+"|hits:"+hits;
        })()"#,
    ),
];

const STRING_FALLBACK_CASES: &[(&str, &str)] = &[
    (
        "String match uses the retained intrinsic constructor and dynamic new protocol",
        r#"(function(){
            var log="",intrinsic=RegExp,receiver=Object(),pattern=Object(),
                source=Object(),flags=Object(),matchGets=0;
            Object.defineProperty(pattern,Symbol.match,{get:function(){
                log+="pattern-match:"+(++matchGets)+";";
                return matchGets===1?null:true;
            }});
            Object.defineProperty(pattern,"source",{get:function(){log+="source-get;";return source}});
            Object.defineProperty(pattern,"flags",{get:function(){log+="flags-get;";return flags}});
            Object.defineProperty(pattern,"constructor",{get:function(){log+="BAD-constructor;";return intrinsic}});
            receiver.toString=function(){log+="receiver-string;";return "subject"};
            source.toString=function(){log+="source-string;";return "sub"};
            flags.toString=function(){log+="flags-string;";return ""};
            Object.defineProperty(intrinsic.prototype,Symbol.match,{configurable:true,get:function(){
                log+="new-match;";
                return function(value){
                    log+="new-call:"+(Object.getPrototypeOf(this)===intrinsic.prototype)+":"+
                        (value==="subject")+":"+__show(this.lastIndex)+";";
                    return 73;
                };
            }});
            globalThis.RegExp=function(){log+="BAD-global;";throw "global constructor used"};
            var result=String.prototype.match.call(receiver,pattern);
            return result+"|"+log;
        })()"#,
    ),
    (
        "String match null and undefined hooks fall back after the receiver conversion",
        r#"(function(){
            function run(hook){
                var log="",receiver=Object(),pattern=Object();
                receiver.toString=function(){log+="receiver;";return "abc"};
                Object.defineProperty(pattern,Symbol.match,{get:function(){log+="get;";return hook}});
                pattern.toString=function(){log+="pattern;";return "b"};
                return __array(String.prototype.match.call(receiver,pattern))+":"+log;
            }
            return run(null)+"|"+run(undefined);
        })()"#,
    ),
];

const NON_GLOBAL_CASES: &[(&str, &str)] = &[
    (
        "RegExp Symbol.match converts input and flags before one abstract exec and returns raw result",
        r#"(function(){
            var log="",input=Object(),flags=Object(),receiver=Object(),result=Object();
            input.toString=function(){log+="input-string;";return "abc"};
            Object.defineProperty(receiver,"flags",{get:function(){log+="flags-get;";return flags}});
            flags.toString=function(){log+="flags-string;";return "i"};
            Object.defineProperty(receiver,"exec",{get:function(){
                log+="exec-get;";
                return function(value){
                    log+="exec-call:"+(this===receiver)+":"+(value==="abc")+":"+
                        arguments.length+";";
                    return result;
                };
            }});
            var actual=RegExp.prototype[Symbol.match].call(receiver,input);
            return (actual===result)+"|"+log+"|has-lastIndex:"+
                Object.prototype.hasOwnProperty.call(receiver,"lastIndex");
        })()"#,
    ),
    (
        "RegExp Symbol.match validates receiver before input and enforces abstract exec results",
        r#"(function(){
            var hits=0,input=Object();
            input.toString=function(){hits++;throw "BAD-input"};
            var primitive=__completion(function(){
                return RegExp.prototype[Symbol.match].call(1,input);
            });
            var ordinary=Object();ordinary.flags=undefined;ordinary.exec=null;
            var nonregexp=__completion(function(){
                return RegExp.prototype[Symbol.match].call(ordinary,"x");
            });
            var invalid=Object();invalid.flags="";invalid.exec=function(){return 7};
            var invalidResult=__completion(function(){
                return RegExp.prototype[Symbol.match].call(invalid,"x");
            });
            var miss=Object();miss.flags="";miss.exec=function(){return null};
            return [primitive,hits,nonregexp,invalidResult,
                RegExp.prototype[Symbol.match].call(miss,"x")].join("|");
        })()"#,
    ),
    (
        "RegExp Symbol.match builtin fallback accepts branded regexps and preserves non-global lastIndex",
        r#"(function(){
            var branded=/b/;branded.lastIndex=2;branded.exec=null;
            var actual=RegExp.prototype[Symbol.match].call(branded,"abc");
            return [__array(actual),actual.index,actual.input,branded.lastIndex].join("|");
        })()"#,
    ),
];

const GLOBAL_CASES: &[(&str, &str)] = &[
    (
        "global match resets lastIndex and collects stringified zero properties in a fresh Array",
        r#"(function(){
            var log="",state=9,calls=0,receiver=Object(),first=Object(),second=Object(),text=Object();
            Object.defineProperty(receiver,"flags",{get:function(){log+="flags-get;";return "g"}});
            Object.defineProperty(receiver,"lastIndex",{
                get:function(){log+="last-get:"+__show(state)+";";return state},
                set:function(value){log+="last-set:"+__show(value)+";";state=value}
            });
            text.toString=function(){log+="zero-string;";return "A"};
            Object.defineProperty(first,"0",{get:function(){log+="zero-get-1;";return text}});
            Object.defineProperty(second,"0",{get:function(){log+="zero-get-2;";return 7}});
            Object.defineProperty(receiver,"exec",{get:function(){
                log+="exec-get;";
                return function(value){
                    calls++;log+="exec-call:"+calls+":"+value+";";
                    if(calls===1)return first;
                    if(calls===2)return second;
                    return null;
                };
            }});
            var actual=RegExp.prototype[Symbol.match].call(receiver,"subject"),
                zero=Object.getOwnPropertyDescriptor(actual,"0"),
                one=Object.getOwnPropertyDescriptor(actual,"1");
            return [__array(actual),state,log,Object.getPrototypeOf(actual)===Array.prototype,
                __bits(actual,"0"),__bits(actual,"1"),zero.value,one.value].join("|");
        })()"#,
    ),
    (
        "global no-match returns null after the unconditional zero write",
        r#"(function(){
            var log="",state=-0,receiver=Object();receiver.flags="g";
            Object.defineProperty(receiver,"lastIndex",{
                get:function(){log+="BAD-get;";return state},
                set:function(value){log+="set:"+__show(value)+";";state=value}
            });
            receiver.exec=function(){log+="exec;";return null};
            var actual=RegExp.prototype[Symbol.match].call(receiver,"x");
            return [actual,__show(state),log].join("|");
        })()"#,
    ),
    (
        "native global regexps collect all matches and finish with the builtin lastIndex reset",
        r#"(function(){
            var hit=/a/g;hit.lastIndex=3;
            var miss=/z/g;miss.lastIndex=2;
            var nonGlobal=/a/;nonGlobal.lastIndex=2;
            return [__array(RegExp.prototype[Symbol.match].call(hit,"baac")),hit.lastIndex,
                RegExp.prototype[Symbol.match].call(miss,"abc"),miss.lastIndex,
                __array(RegExp.prototype[Symbol.match].call(nonGlobal,"ba")),
                nonGlobal.lastIndex].join("|");
        })()"#,
    ),
];

const EMPTY_ADVANCE_CASES: &[(&str, &str)] = &[
    (
        "empty global matches advance one UTF-16 unit or one Unicode code point including v",
        r#"(function(){
            function run(flags,input,current){
                var state=99,calls=0,receiver=Object();receiver.flags=flags;
                Object.defineProperty(receiver,"lastIndex",{
                    get:function(){return state},set:function(value){state=value}
                });
                receiver.exec=function(){
                    calls++;
                    if(calls===1){state=current;return {0:""}}
                    return null;
                };
                var actual=RegExp.prototype[Symbol.match].call(receiver,input);
                return flags+":"+__show(current)+":"+__show(state)+":"+__array(actual);
            }
            var astral="\ud83d\ude00",lone="\ud800";
            return [run("g",astral,0),run("gu",astral,0),run("gv",astral,0),
                run("gu",astral,1),run("gu",astral,2),run("gu",lone,0),
                run("g","abc",-3),run("g","abc",1.9),
                run("g","abc",Infinity)].join("|");
        })()"#,
    ),
    (
        "empty match reads and converts lastIndex only after defining the result element",
        r#"(function(){
            var log="",state=8,calls=0,index=Object(),receiver=Object();receiver.flags="gu";
            Object.defineProperty(receiver,"lastIndex",{
                get:function(){log+="last-get;";return index},
                set:function(value){log+="last-set:"+__show(value)+";";state=value}
            });
            index.valueOf=function(){log+="index-value;";return 0};
            receiver.exec=function(){calls++;log+="exec:"+calls+";";return calls===1?{0:""}:null};
            var actual=RegExp.prototype[Symbol.match].call(receiver,"\ud83d\ude00");
            return [__array(actual),state,log].join("|");
        })()"#,
    ),
];

const ABRUPT_CASES: &[(&str, &str)] = &[
    (
        "input flags and initial lastIndex abrupt completions stop at exact boundaries",
        r#"(function(){
            function capture(receiver,input,sentinel){
                try{RegExp.prototype[Symbol.match].call(receiver,input);return "return"}
                catch(error){return (error===sentinel)+":"+receiver.log+":"+__show(receiver.state)}
            }
            var inputBoom=Object(),input=Object(),inputReceiver=Object();
            inputReceiver.log="";inputReceiver.state=3;
            input.toString=function(){inputReceiver.log+="input;";throw inputBoom};
            Object.defineProperty(inputReceiver,"flags",{get:function(){this.log+="BAD-flags;";return "g"}});

            var flagsBoom=Object(),flagsReceiver=Object();flagsReceiver.log="";flagsReceiver.state=3;
            Object.defineProperty(flagsReceiver,"flags",{get:function(){this.log+="flags;";throw flagsBoom}});

            var convertBoom=Object(),flags=Object(),convertReceiver=Object();
            convertReceiver.log="";convertReceiver.state=3;
            Object.defineProperty(convertReceiver,"flags",{get:function(){this.log+="flags;";return flags}});
            flags.toString=function(){convertReceiver.log+="flags-string;";throw convertBoom};

            var setBoom=Object(),setReceiver=Object();setReceiver.log="";setReceiver.state=3;setReceiver.flags="g";
            Object.defineProperty(setReceiver,"lastIndex",{
                get:function(){this.log+="BAD-get;";return this.state},
                set:function(value){this.log+="set:"+value+";";throw setBoom}
            });
            setReceiver.exec=function(){this.log+="BAD-exec;";return null};
            return [capture(inputReceiver,input,inputBoom),capture(flagsReceiver,"x",flagsBoom),
                capture(convertReceiver,"x",convertBoom),capture(setReceiver,"x",setBoom)].join("|");
        })()"#,
    ),
    (
        "exec result zero and empty lastIndex abrupt completions preserve identity and effects",
        r#"(function(){
            function capture(receiver,sentinel){
                try{RegExp.prototype[Symbol.match].call(receiver,"x");return "return"}
                catch(error){return (error===sentinel)+":"+receiver.log+":"+__show(receiver.state)}
            }
            function base(){
                var receiver=Object();receiver.flags="g";receiver.log="";receiver.state=7;
                Object.defineProperty(receiver,"lastIndex",{
                    get:function(){this.log+="get;";return this.state},
                    set:function(value){this.log+="set:"+value+";";this.state=value},
                    configurable:true
                });
                return receiver;
            }
            var execBoom=Object(),execReceiver=base();
            Object.defineProperty(execReceiver,"exec",{get:function(){this.log+="exec-get;";throw execBoom}});

            var zeroBoom=Object(),zeroReceiver=base(),zeroResult=Object();
            Object.defineProperty(zeroResult,"0",{get:function(){zeroReceiver.log+="zero-get;";throw zeroBoom}});
            zeroReceiver.exec=function(){this.log+="exec;";return zeroResult};

            var stringBoom=Object(),stringReceiver=base(),stringResult=Object(),zero=Object();
            zero.toString=function(){stringReceiver.log+="zero-string;";throw stringBoom};
            Object.defineProperty(stringResult,"0",{get:function(){stringReceiver.log+="zero-get;";return zero}});
            stringReceiver.exec=function(){this.log+="exec;";return stringResult};

            var getBoom=Object(),getReceiver=base(),getCalls=0;
            Object.defineProperty(getReceiver,"lastIndex",{
                get:function(){this.log+="get;";throw getBoom},
                set:function(value){this.log+="set:"+value+";";this.state=value}
            });
            getReceiver.exec=function(){getCalls++;this.log+="exec;";return getCalls===1?{0:""}:null};

            var convertBoom=Object(),convertReceiver=base(),convertCalls=0,index=Object();
            index.valueOf=function(){convertReceiver.log+="index-value;";throw convertBoom};
            Object.defineProperty(convertReceiver,"lastIndex",{
                get:function(){this.log+="get;";return index},
                set:function(value){this.log+="set:"+value+";";this.state=value}
            });
            convertReceiver.exec=function(){convertCalls++;this.log+="exec;";return convertCalls===1?{0:""}:null};

            var setBoom=Object(),setReceiver=base(),setCalls=0,setCount=0;
            Object.defineProperty(setReceiver,"lastIndex",{
                get:function(){this.log+="get;";return 0},
                set:function(value){this.log+="set:"+value+";";this.state=value;if(++setCount===2)throw setBoom}
            });
            setReceiver.exec=function(){setCalls++;this.log+="exec;";return setCalls===1?{0:""}:null};

            return [capture(execReceiver,execBoom),capture(zeroReceiver,zeroBoom),
                capture(stringReceiver,stringBoom),capture(getReceiver,getBoom),
                capture(convertReceiver,convertBoom),capture(setReceiver,setBoom)].join("|");
        })()"#,
    ),
];

const RECURSION_CASES: &[(&str, &str)] = &[(
    "recursive String and RegExp match protocol calls are catchable and recover",
    r#"(function(){
            var pattern=Object(),stringError="",regexp=Object(),regexpError="";
            pattern[Symbol.match]=function(value){return String.prototype.match.call(value,pattern)};
            try{"x".match(pattern)}catch(error){stringError=error.name+":"+error.message}
            regexp.flags="";
            regexp.exec=function(){return RegExp.prototype[Symbol.match].call(regexp,"x")};
            try{RegExp.prototype[Symbol.match].call(regexp,"x")}
            catch(error){regexpError=error.name+":"+error.message}
            return [stringError,regexpError,__array("abc".match("b")),
                RegExp.prototype[Symbol.match].call({flags:"",exec:function(){return null}},"x")].join("|");
        })()"#,
)];

#[test]
fn string_match_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String/RegExp match oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("metadata", METADATA_CASES),
        ("String dispatch", STRING_DISPATCH_CASES),
        ("String fallback", STRING_FALLBACK_CASES),
        ("non-global", NON_GLOBAL_CASES),
        ("global", GLOBAL_CASES),
        ("empty advance", EMPTY_ADVANCE_CASES),
        ("abrupt completion", ABRUPT_CASES),
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
}

#[test]
fn match_metadata_autoinit_and_property_order_match_pinned_quickjs() {
    compare_cases("match metadata", METADATA_CASES);
}

#[test]
fn string_match_protocol_dispatch_matches_pinned_quickjs() {
    compare_cases("String match dispatch", STRING_DISPATCH_CASES);
}

#[test]
fn string_match_intrinsic_fallback_matches_pinned_quickjs() {
    compare_cases("String match fallback", STRING_FALLBACK_CASES);
}

#[test]
fn regexp_symbol_match_non_global_and_abstract_exec_match_pinned_quickjs() {
    compare_cases("RegExp Symbol.match non-global", NON_GLOBAL_CASES);
}

#[test]
fn regexp_symbol_match_global_collection_matches_pinned_quickjs() {
    compare_cases("RegExp Symbol.match global", GLOBAL_CASES);
}

#[test]
fn regexp_symbol_match_empty_unicode_advance_matches_pinned_quickjs() {
    compare_cases("RegExp Symbol.match empty advance", EMPTY_ADVANCE_CASES);
}

#[test]
fn regexp_symbol_match_abrupt_completion_matches_pinned_quickjs() {
    compare_cases("RegExp Symbol.match abrupt completion", ABRUPT_CASES);
}

#[test]
fn match_protocol_recursion_is_catchable_and_recovers_like_pinned_quickjs() {
    compare_cases("String/RegExp match recursion", RECURSION_CASES);
}

#[test]
fn match_intrinsics_use_defining_realms_and_accept_foreign_regexp_brands() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let string_match = eval_callable(
        &runtime,
        &mut defining,
        "String.prototype.match",
        "defining String match",
    );
    let regexp_match = eval_callable(
        &runtime,
        &mut defining,
        "RegExp.prototype[Symbol.match]",
        "defining RegExp Symbol.match",
    );
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
    assert_ne!(defining_type_error, caller_type_error);

    defining
        .eval("RegExp.prototype[Symbol.match]=function(){return 41}")
        .unwrap();
    caller
        .eval("RegExp.prototype[Symbol.match]=function(){return 99}")
        .unwrap();
    assert_eq!(
        caller
            .call(
                &string_match,
                string_value("subject"),
                &[string_value("needle")],
            )
            .unwrap(),
        Value::Int(41),
        "String match fallback did not use its defining-realm RegExp intrinsic",
    );

    let global_receiver = eval_object(
        &mut caller,
        r#"(function(){
            var count=0,receiver=Object();receiver.flags="g";receiver.lastIndex=8;
            receiver.exec=function(){count++;return count===1?{0:"x"}:null};
            return receiver;
        })()"#,
        "caller global RegExp-like receiver",
    );
    let Value::Object(global_result) = caller
        .call(
            &regexp_match,
            Value::Object(global_receiver),
            &[string_value("x")],
        )
        .unwrap()
    else {
        panic!("global RegExp Symbol.match did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&global_result).unwrap(),
        Some(defining.array_prototype().unwrap()),
        "global match result did not use the method defining realm Array",
    );
    assert_eq!(
        string_property(&runtime, &mut caller, &global_result, "0"),
        "x",
    );

    let foreign_regexp = eval_object(
        &mut caller,
        "(function(){var r=/b/;r.exec=null;return r})()",
        "caller RegExp with builtin exec fallback",
    );
    let Value::Object(foreign_result) = caller
        .call(
            &regexp_match,
            Value::Object(foreign_regexp),
            &[string_value("abc")],
        )
        .unwrap()
    else {
        panic!("foreign branded RegExp did not return its match object");
    };
    assert_eq!(
        runtime.get_prototype_of(&foreign_result).unwrap(),
        Some(defining.array_prototype().unwrap()),
        "foreign branded RegExp match result used the caller Array realm",
    );
    assert_eq!(
        string_property(&runtime, &mut caller, &foreign_result, "0"),
        "b",
    );

    assert_eq!(
        caller.call(&regexp_match, Value::Int(1), &[string_value("x")]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "defining-realm match TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "native RegExp Symbol.match TypeError used the caller realm",
    );

    let throwing_receiver = eval_object(
        &mut caller,
        r#"(function(){
            var receiver=Object();
            Object.defineProperty(receiver,"flags",{get:function(){throw new TypeError("caller flags")}});
            return receiver;
        })()"#,
        "caller throwing flags receiver",
    );
    assert_eq!(
        caller.call(
            &regexp_match,
            Value::Object(throwing_receiver),
            &[string_value("x")],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller flags TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "RegExp Symbol.match replaced a caller-realm user exception",
    );
}

#[test]
fn mixed_string_and_regexp_match_recursion_guard_is_catchable_and_recovers() {
    std::thread::Builder::new()
        .name("string-regexp-match-stack-proof".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context
                .eval(
                    r#"function mixedMatchRecurse(kind,depth){
                        if(kind===0){
                            var pattern=Object();
                            pattern[Symbol.match]=function(){
                                if(depth!==0)return mixedMatchRecurse(1,depth-1);
                                return null;
                            };
                            return "x".match(pattern);
                        }
                        var regexp=Object();regexp.flags="";
                        regexp.exec=function(){
                            if(depth!==0)mixedMatchRecurse(0,depth-1);
                            return null;
                        };
                        return RegExp.prototype[Symbol.match].call(regexp,"x");
                    }"#,
                )
                .unwrap();

            for (entry, kind) in [("String.prototype.match", 0), ("RegExp @@match", 1)] {
                assert_eq!(
                    context
                        .eval(&format!("mixedMatchRecurse({kind},3)"))
                        .unwrap(),
                    Value::Null,
                    "the proven-safe four-frame {entry} chain was rejected",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedMatchRecurse({kind},4);return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    string_value("InternalError:stack overflow"),
                    "the fifth mixed match frame was not rejected from {entry}",
                );
            }
            assert_eq!(
                context
                    .eval(
                        r#"__arrayForRecovery="abc".match("b");
                           __arrayForRecovery[0]+"|"+
                           RegExp.prototype[Symbol.match].call({
                               flags:"",exec:function(){return null}
                           },"x")"#,
                    )
                    .unwrap(),
                string_value("b|null"),
                "the runtime did not recover after mixed match overflow",
            );
        })
        .expect("2 MiB String/RegExp match stack-proof thread did not start")
        .join()
        .expect("2 MiB String/RegExp match stack-proof thread panicked");
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

fn eval_callable(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> CallableRef {
    let object = eval_object(context, source, description);
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("{description} was not callable"))
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

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::String(value) = context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read string property {name}: {error}"))
    else {
        panic!("{name} was not a string");
    };
    value.to_utf8_lossy()
}

fn string_value(value: &str) -> Value {
    Value::String(JsString::try_from_utf8(value).unwrap())
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
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Symbol(_) => "<symbol>".to_owned(),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}
