use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04
// `js_regexp_Symbol_split` (`quickjs.c` 48875-48990), including
// SpeciesConstructor, abstract RegExpExec, and String.prototype.split's
// Symbol.split activation path.

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
    var output=[],index=0;
    while(index<value.length){
        output[index]=(index in value)?__show(value[index]):"<hole>";
        index++;
    }
    return value.length+"["+output.join(",")+"]";
}
"#;

const METADATA_CASES: &[(&str, &str)] = &[
    (
        "split methods expose pinned metadata descriptors and filtered key order",
        r#"(function(){
            var stringFn=String.prototype.split,
                regexpFn=RegExp.prototype[Symbol.split],
                stringKeys=Object.getOwnPropertyNames(String.prototype),
                regexpKeys=Reflect.ownKeys(RegExp.prototype),
                selected=["startsWith","search","split","substring"],
                stringOrder=[],regexpOrder=[],index=0,key;
            while(index<stringKeys.length){
                key=stringKeys[index];
                if(selected.indexOf(key)>=0)stringOrder[stringOrder.length]=key;
                index++;
            }
            index=0;
            while(index<regexpKeys.length){
                key=regexpKeys[index];
                if(key==="test"||key==="toString"||key===Symbol.match||
                    key===Symbol.search||key===Symbol.split)
                    regexpOrder[regexpOrder.length]=String(key);
                index++;
            }
            return [
                stringOrder.join(","),regexpOrder.join(","),
                __bits(String.prototype,"split"),stringFn.name,stringFn.length,
                Object.getOwnPropertyNames(stringFn).join(","),
                __bits(stringFn,"name"),__bits(stringFn,"length"),
                __isConstructor(stringFn),
                Object.prototype.hasOwnProperty.call(stringFn,"prototype"),
                __bits(RegExp.prototype,Symbol.split),regexpFn.name,regexpFn.length,
                Object.getOwnPropertyNames(regexpFn).join(","),
                __bits(regexpFn,"name"),__bits(regexpFn,"length"),
                __isConstructor(regexpFn),
                Object.prototype.hasOwnProperty.call(regexpFn,"prototype")
            ].join("|");
        })()"#,
    ),
    (
        "split AutoInit entries retain identity and can be replaced independently",
        r#"(function(){
            var stringOriginal=String.prototype.split,
                regexpOriginal=RegExp.prototype[Symbol.split],
                stable=stringOriginal===String.prototype.split&&
                    regexpOriginal===RegExp.prototype[Symbol.split],
                stringDeleted=delete String.prototype.split,
                regexpDeleted=delete RegExp.prototype[Symbol.split];
            String.prototype.split=17;
            RegExp.prototype[Symbol.split]=23;
            return [stable,stringDeleted,regexpDeleted,String.prototype.split,
                RegExp.prototype[Symbol.split],__bits(String.prototype,"split"),
                __bits(RegExp.prototype,Symbol.split),
                stringOriginal===regexpOriginal].join("|");
        })()"#,
    ),
];

const STRING_INTEGRATION_CASES: &[(&str, &str)] = &[
    (
        "String split activates the inherited RegExp Symbol.split protocol",
        r#"(function(){
            var first=/(-)/g,second=/b/;
            first.lastIndex=7;second.lastIndex=9;
            var left="a-b-c".split(first,4),right=String.prototype.split.call("abc",second);
            return [__array(left),first.lastIndex,__array(right),second.lastIndex,
                Object.getPrototypeOf(left)===Array.prototype,
                Object.getPrototypeOf(right)===Array.prototype].join("|");
        })()"#,
    ),
    (
        "String split gets Symbol.split before RegExp input conversion and passes raw limit",
        r#"(function(){
            var log="",receiver=Object(),limit=Object(),regexp=/b/,
                builtin=RegExp.prototype[Symbol.split];
            receiver.toString=function(){log+="input-string;";return "abc"};
            limit.valueOf=function(){log+="limit-number;";return 2};
            Object.defineProperty(regexp,Symbol.split,{get:function(){
                log+="split-get;";
                return function(input,rawLimit){
                    log+="split-call:"+(this===regexp)+":"+(input===receiver)+":"+
                        (rawLimit===limit)+";";
                    return builtin.call(this,input,rawLimit);
                };
            }});
            return __array(String.prototype.split.call(receiver,regexp,limit))+"|"+log;
        })()"#,
    ),
];

const SPECIES_ORDER_CASES: &[(&str, &str)] = &[
    (
        "RegExp split observes input constructor species flags construct limit and exec order",
        r#"(function(){
            var log="",regexp=Object(),holder=Object(),input=Object(),flags=Object(),limit=Object();
            input.toString=function(){log+="input-string;";return "x"};
            Object.defineProperty(regexp,"constructor",{get:function(){
                log+="constructor-get;";return holder;
            }});
            Object.defineProperty(holder,Symbol.species,{get:function(){
                log+="species-get:"+(this===holder)+";";return Species;
            }});
            Object.defineProperty(regexp,"flags",{get:function(){log+="flags-get;";return flags}});
            flags.toString=function(){log+="flags-string;";return "g"};
            function Species(pattern,observedFlags){
                log+="construct:"+(new.target===Species)+":"+(pattern===regexp)+":"+
                    observedFlags+":"+arguments.length+";";
                var splitter=Object();splitter.lastIndex=0;
                splitter.exec=function(text){log+="exec:"+(this===splitter)+":"+text+";";return null};
                return splitter;
            }
            limit.valueOf=function(){log+="limit-number;";return 3};
            var result=RegExp.prototype[Symbol.split].call(regexp,input,limit);
            return __array(result)+"|"+log;
        })()"#,
    ),
    (
        "primitive constructor and nonconstructor species are TypeErrors before flags",
        r#"(function(){
            function primitive(){
                var log="",regexp=Object();
                Object.defineProperty(regexp,"constructor",{get:function(){log+="constructor;";return 1}});
                Object.defineProperty(regexp,"flags",{get:function(){log+="BAD-flags;";return ""}});
                var completion=__completion(function(){
                    return RegExp.prototype[Symbol.split].call(regexp,"x");
                });
                return completion+":"+log;
            }
            function species(value){
                var log="",regexp=Object(),holder=Object();
                regexp.constructor=holder;
                Object.defineProperty(holder,Symbol.species,{get:function(){log+="species;";return value}});
                Object.defineProperty(regexp,"flags",{get:function(){log+="BAD-flags;";return ""}});
                var completion=__completion(function(){
                    return RegExp.prototype[Symbol.split].call(regexp,"x");
                });
                return completion+":"+log;
            }
            return [primitive(),species(1),species(RegExp.prototype.exec)].join("|");
        })()"#,
    ),
    (
        "nullish species uses retained RegExp and ignores poisoned global constructors",
        r#"(function(){
            var intrinsicRegExp=RegExp,intrinsicArray=Array,split=RegExp.prototype[Symbol.split],
                first=/b/,second=/b/,firstHolder=Object(),secondHolder=Object();
            firstHolder[Symbol.species]=null;secondHolder[Symbol.species]=undefined;
            first.constructor=firstHolder;second.constructor=secondHolder;
            globalThis.RegExp=function(){throw "global RegExp used"};
            globalThis.Array=function(){throw "global Array used"};
            var left=split.call(first,"abc"),right=split.call(second,"abc");
            return [__array(left),__array(right),
                Object.getPrototypeOf(left)===intrinsicArray.prototype,
                Object.getPrototypeOf(right)===intrinsicArray.prototype,
                Object.getPrototypeOf(first)===intrinsicRegExp.prototype].join("|");
        })()"#,
    ),
];

const FLAGS_AND_ADVANCE_CASES: &[(&str, &str)] = &[(
    "split appends sticky once and uses u or v for surrogate-pair advancement",
    r#"(function(){
            function run(flags){
                var regexp=Object(),holder=Object(),sets="",seenFlags="";
                function Species(_pattern,constructedFlags){
                    seenFlags=constructedFlags;
                    var state=0,splitter=Object();
                    Object.defineProperty(splitter,"lastIndex",{
                        get:function(){return state},
                        set:function(value){if(sets!=="")sets+=",";sets+=value;state=value}
                    });
                    splitter.exec=function(){return null};
                    return splitter;
                }
                holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags=flags;
                var result=RegExp.prototype[Symbol.split].call(
                    regexp,String.fromCharCode(0xd83d,0xde00,0x58));
                return seenFlags+":"+sets+":"+result.length+":"+result[0].length;
            }
            return [run(""),run("y"),run("u"),run("uy"),run("v")].join("|");
        })()"#,
)];

const LIMIT_AND_EMPTY_CASES: &[(&str, &str)] = &[
    (
        "real RegExp split honors undefined zero Uint32 wrapping and capture truncation",
        r#"(function(){
            return [
                __array("ab12cd34".split(/\d+/,undefined)),
                __array("ab12cd34".split(/\d+/,0)),
                __array("a1b2".split(/(\d)/,3)),
                __array("a-b-c".split(/-/,4294967297)),
                __array("a-b-c".split(/-/,-1))
            ].join("|");
        })()"#,
    ),
    (
        "zero limit still constructs the species but never executes it",
        r#"(function(){
            var log="",regexp=Object(),holder=Object(),limit=Object();
            function Species(){
                log+="construct;";
                return {lastIndex:0,exec:function(){log+="BAD-exec;";return null}};
            }
            holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";
            limit.valueOf=function(){log+="limit;";return 0};
            return __array(RegExp.prototype[Symbol.split].call(regexp,"abc",limit))+"|"+log;
        })()"#,
    ),
    (
        "empty input executes once without setting lastIndex and distinguishes hit from miss",
        r#"(function(){
            function run(hit){
                var regexp=Object(),holder=Object(),splitter,calls=0;
                function Species(){
                    splitter=Object();
                    splitter.exec=function(value){calls++;return hit?{0:"",length:1}:null};
                    return splitter;
                }
                holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";
                var result=RegExp.prototype[Symbol.split].call(regexp,"");
                return __array(result)+":"+calls+":"+
                    Object.prototype.hasOwnProperty.call(splitter,"lastIndex");
            }
            return run(false)+"|"+run(true)+"|"+__array("".split(/(?:)/));
        })()"#,
    ),
];

const CAPTURE_CASES: &[(&str, &str)] = &[(
    "capture values remain raw and are read only through the active limit",
    r#"(function(){
            function run(limit){
                var log="",capture=Object(),regexp=Object(),holder=Object(),calls=0;
                function Species(){
                    var splitter=Object();splitter.lastIndex=0;
                    splitter.exec=function(){
                        calls++;
                        if(calls!==1)return null;
                        this.lastIndex=2;
                        var result=Object();
                        Object.defineProperty(result,"length",{get:function(){log+="length;";return 4}});
                        Object.defineProperty(result,"1",{get:function(){log+="one;";return capture}});
                        Object.defineProperty(result,"2",{get:function(){log+="two;";return undefined}});
                        Object.defineProperty(result,"3",{get:function(){log+="three;";return false}});
                        return result;
                    };
                    return splitter;
                }
                holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";
                var result=RegExp.prototype[Symbol.split].call(regexp,"abcd",limit);
                return [result.length,result[0],result[1]===capture,result[2]===undefined,
                    result[3]===false,result[4],log,calls].join(":");
            }
            return run(undefined)+"|"+run(2);
        })()"#,
)];

const LAST_INDEX_AND_EXEC_CASES: &[(&str, &str)] = &[
    (
        "empty matches advance and a retreating lastIndex is consumed exactly",
        r#"(function(){
            function empty(){
                var regexp=Object(),holder=Object(),sets="";
                function Species(){
                    var state=0,splitter=Object();
                    Object.defineProperty(splitter,"lastIndex",{
                        get:function(){return state},
                        set:function(value){if(sets!=="")sets+=",";sets+=value;state=value}
                    });
                    splitter.exec=function(){return {length:1}};
                    return splitter;
                }
                holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";
                return __array(RegExp.prototype[Symbol.split].call(regexp,"ab"))+":"+sets;
            }
            function retreat(){
                var regexp=Object(),holder=Object(),sets="",calls=0;
                function Species(){
                    var state=0,splitter=Object();
                    Object.defineProperty(splitter,"lastIndex",{
                        get:function(){return state},
                        set:function(value){if(sets!=="")sets+=",";sets+=value;state=value}
                    });
                    splitter.exec=function(){
                        calls++;
                        if(calls===1){state=2;return {length:1}}
                        if(calls===2){state=1;return {length:1}}
                        return null;
                    };
                    return splitter;
                }
                holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";
                return __array(RegExp.prototype[Symbol.split].call(regexp,"abcd"))+":"+sets+":"+calls;
            }
            return empty()+"|"+retreat();
        })()"#,
    ),
    (
        "lastIndex is clamped to input length after a successful abstract exec",
        r#"(function(){
            var regexp=Object(),holder=Object(),calls=0;
            function Species(){
                var splitter=Object();splitter.lastIndex=0;
                splitter.exec=function(){calls++;this.lastIndex=99;return {length:1}};
                return splitter;
            }
            holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";
            return __array(RegExp.prototype[Symbol.split].call(regexp,"abcd"))+"|"+calls;
        })()"#,
    ),
    (
        "abstract exec accepts branded fallback and rejects missing or primitive results",
        r#"(function(){
            function run(kind){
                var regexp=Object(),holder=Object();
                function Species(){
                    if(kind===0){var branded=/b/y;branded.exec=null;return branded}
                    if(kind===1)return {lastIndex:0,exec:null};
                    return {lastIndex:0,exec:function(){return 7}};
                }
                holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";
                return kind===0?__array(RegExp.prototype[Symbol.split].call(regexp,"abc")):
                    __completion(function(){return RegExp.prototype[Symbol.split].call(regexp,"abc")});
            }
            return [run(0),run(1),run(2)].join("|");
        })()"#,
    ),
];

const ABRUPT_CASES: &[(&str, &str)] = &[
    (
        "prefix abrupt completions stop at each input species flags construct and limit boundary",
        r#"(function(){
            function run(stage){
                var log="",sentinel=Object(),regexp=Object(),holder=Object(),
                    input=Object(),flags=Object(),limit=Object(),same=false;
                input.toString=function(){log+="input;";if(stage===0)throw sentinel;return "x"};
                Object.defineProperty(regexp,"constructor",{get:function(){
                    log+="constructor;";if(stage===1)throw sentinel;return holder;
                }});
                Object.defineProperty(holder,Symbol.species,{get:function(){
                    log+="species;";if(stage===2)throw sentinel;return Species;
                }});
                Object.defineProperty(regexp,"flags",{get:function(){
                    log+="flags-get;";if(stage===3)throw sentinel;return flags;
                }});
                flags.toString=function(){log+="flags-string;";if(stage===4)throw sentinel;return ""};
                function Species(){
                    log+="construct;";if(stage===5)throw sentinel;
                    return {lastIndex:0,exec:function(){log+="exec;";return null}};
                }
                limit.valueOf=function(){log+="limit;";if(stage===6)throw sentinel;return 2};
                try{RegExp.prototype[Symbol.split].call(regexp,input,limit)}
                catch(error){same=error===sentinel}
                return same+":"+log;
            }
            var output=[],index=0;
            while(index<7){output[index]=run(index);index++}
            return output.join("|");
        })()"#,
    ),
    (
        "loop abrupt completions stop at setter exec end length and capture boundaries",
        r#"(function(){
            function run(stage){
                var log="",sentinel=Object(),regexp=Object(),holder=Object(),same=false;
                function Species(){
                    var state=0,splitter=Object(),result=Object(),end=Object(),length=Object();
                    end.valueOf=function(){log+="end-value;";if(stage===4)throw sentinel;return 1};
                    length.valueOf=function(){log+="length-value;";if(stage===6)throw sentinel;return 2};
                    Object.defineProperty(splitter,"lastIndex",{
                        get:function(){log+="last-get;";if(stage===3)throw sentinel;
                            return stage===4?end:state},
                        set:function(value){log+="last-set;";if(stage===0)throw sentinel;state=value}
                    });
                    Object.defineProperty(result,"length",{get:function(){
                        log+="length-get;";if(stage===5)throw sentinel;return stage===6?length:2;
                    }});
                    Object.defineProperty(result,"1",{get:function(){
                        log+="capture-get;";if(stage===7)throw sentinel;return "capture";
                    }});
                    Object.defineProperty(splitter,"exec",{get:function(){
                        log+="exec-get;";if(stage===1)throw sentinel;
                        return function(){log+="exec-call;";if(stage===2)throw sentinel;
                            state=1;return result};
                    }});
                    return splitter;
                }
                holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";
                try{RegExp.prototype[Symbol.split].call(regexp,"x")}
                catch(error){same=error===sentinel}
                return same+":"+log;
            }
            var output=[],index=0;
            while(index<8){output[index]=run(index);index++}
            return output.join("|");
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[(
    "RegExp Symbol.split rejects primitive receivers before input conversion",
    r#"(function(){
            var input=Object(),hits=0;
            input.toString=function(){hits++;return "x"};
            var completion=__completion(function(){
                return RegExp.prototype[Symbol.split].call(1,input);
            });
            return completion+"|"+hits;
        })()"#,
)];

const RECURSION_CASES: &[(&str, &str)] = &[(
    "recursive String and RegExp split protocol calls are catchable and recover",
    r#"(function(){
            var separator=Object(),stringError="",regexp=Object(),holder=Object(),regexpError="";
            separator[Symbol.split]=function(value,limit){
                return String.prototype.split.call(value,separator,limit);
            };
            try{"x".split(separator)}catch(error){stringError=error.name+":"+error.message}
            function Species(){return regexp}
            holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";regexp.lastIndex=0;
            regexp.exec=function(){return RegExp.prototype[Symbol.split].call(regexp,"x")};
            try{RegExp.prototype[Symbol.split].call(regexp,"x")}
            catch(error){regexpError=error.name+":"+error.message}
            return [stringError,regexpError,__array("a-b".split(/-/)),
                __array((function(){
                    var recovery=Object(),recoveryHolder=Object();
                    recoveryHolder[Symbol.species]=function(){
                        return {lastIndex:0,exec:function(){return null}};
                    };
                    recovery.constructor=recoveryHolder;recovery.flags="";
                    return RegExp.prototype[Symbol.split].call(recovery,"x");
                })())].join("|");
        })()"#,
)];

#[test]
fn regexp_split_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String/RegExp split oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("metadata", METADATA_CASES),
        ("String integration", STRING_INTEGRATION_CASES),
        ("species order", SPECIES_ORDER_CASES),
        ("flags and advance", FLAGS_AND_ADVANCE_CASES),
        ("limit and empty", LIMIT_AND_EMPTY_CASES),
        ("captures", CAPTURE_CASES),
        ("lastIndex and exec", LAST_INDEX_AND_EXEC_CASES),
        ("abrupt completion", ABRUPT_CASES),
        ("errors", ERROR_CASES),
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
fn regexp_split_metadata_autoinit_and_string_activation_match_pinned_quickjs() {
    compare_cases("RegExp split metadata", METADATA_CASES);
    compare_cases(
        "String to RegExp split integration",
        STRING_INTEGRATION_CASES,
    );
}

#[test]
fn regexp_split_species_order_defaults_and_errors_match_pinned_quickjs() {
    compare_cases("RegExp split SpeciesConstructor", SPECIES_ORDER_CASES);
    compare_cases("RegExp split native errors", ERROR_CASES);
}

#[test]
fn regexp_split_flags_limits_empty_and_captures_match_pinned_quickjs() {
    compare_cases("RegExp split flags and advance", FLAGS_AND_ADVANCE_CASES);
    compare_cases("RegExp split limits and empty input", LIMIT_AND_EMPTY_CASES);
    compare_cases("RegExp split captures", CAPTURE_CASES);
}

#[test]
fn regexp_split_last_index_abstract_exec_and_abrupt_boundaries_match_pinned_quickjs() {
    compare_cases(
        "RegExp split lastIndex and abstract exec",
        LAST_INDEX_AND_EXEC_CASES,
    );
    compare_cases("RegExp split abrupt completion", ABRUPT_CASES);
}

#[test]
fn regexp_split_recursion_is_catchable_and_recovers_like_pinned_quickjs() {
    compare_cases("String/RegExp split recursion", RECURSION_CASES);
}

#[test]
fn regexp_split_intrinsics_use_defining_realms_and_foreign_species() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let regexp_split = eval_callable(
        &runtime,
        &mut defining,
        "RegExp.prototype[Symbol.split]",
        "defining RegExp Symbol.split",
    );
    let defining_array_prototype = defining.array_prototype().unwrap();
    let caller_array_prototype = caller.array_prototype().unwrap();
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

    let foreign_regexp = eval_object(
        &mut caller,
        "(function(){var r=/b/;r.exec=null;return r})()",
        "caller branded RegExp",
    );
    let Value::Object(foreign_result) = caller
        .call(
            &regexp_split,
            Value::Object(foreign_regexp),
            &[string_value("abc")],
        )
        .expect("cross-realm branded RegExp split")
    else {
        panic!("cross-realm branded RegExp split did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&foreign_result).unwrap(),
        Some(defining_array_prototype.clone()),
        "RegExp split result did not use the method defining realm Array",
    );
    assert_ne!(
        runtime.get_prototype_of(&foreign_result).unwrap(),
        Some(caller_array_prototype),
    );
    assert_eq!(
        string_property(&runtime, &mut caller, &foreign_result, "0"),
        "a"
    );
    assert_eq!(
        string_property(&runtime, &mut caller, &foreign_result, "1"),
        "c"
    );

    let foreign_species_receiver = eval_object(
        &mut caller,
        r#"(function(){
            var holder=Object(),regexp=Object();
            holder[Symbol.species]=function(_pattern,flags){
                globalThis.splitSpeciesFlags=flags;
                var splitter=Object(),calls=0;globalThis.splitterPrototype=Object.getPrototypeOf(splitter);
                splitter.lastIndex=0;
                splitter.exec=function(){
                    calls++;globalThis.splitExecCalls=calls;
                    if(calls===2){this.lastIndex=2;return {length:1}}
                    return null;
                };
                return splitter;
            };
            regexp.constructor=holder;regexp.flags="";return regexp;
        })()"#,
        "caller custom RegExp species receiver",
    );
    let Value::Object(species_result) = caller
        .call(
            &regexp_split,
            Value::Object(foreign_species_receiver),
            &[string_value("abc")],
        )
        .expect("cross-realm custom species RegExp split")
    else {
        panic!("cross-realm custom species RegExp split did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&species_result).unwrap(),
        Some(defining_array_prototype),
        "foreign species changed the defining realm of the split result Array",
    );
    assert_eq!(
        string_property(&runtime, &mut caller, &species_result, "0"),
        "a"
    );
    assert_eq!(
        string_property(&runtime, &mut caller, &species_result, "1"),
        "c"
    );
    assert_eq!(caller.eval("splitSpeciesFlags").unwrap(), string_value("y"));
    assert_eq!(caller.eval("splitExecCalls").unwrap(), Value::Int(3));
    assert_eq!(
        caller.eval("splitterPrototype===Object.prototype").unwrap(),
        Value::Bool(true),
        "custom splitter did not retain the species constructor realm",
    );

    assert_eq!(
        caller.call(&regexp_split, Value::Int(1), &[string_value("x")]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "defining-realm split TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "native RegExp Symbol.split TypeError used the caller realm",
    );

    let throwing_receiver = eval_object(
        &mut caller,
        r#"(function(){
            var regexp=Object();
            Object.defineProperty(regexp,"constructor",{get:function(){
                throw new TypeError("caller constructor");
            }});
            return regexp;
        })()"#,
        "caller throwing constructor receiver",
    );
    assert_eq!(
        caller.call(
            &regexp_split,
            Value::Object(throwing_receiver),
            &[string_value("x")],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller split constructor TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "RegExp Symbol.split replaced a caller-realm user exception",
    );
}

#[test]
fn mixed_string_and_regexp_split_recursion_guard_is_catchable_and_recovers() {
    std::thread::Builder::new()
        .name("string-regexp-split-stack-proof".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            context
                .eval(
                    r#"function mixedSplitRecurse(kind,depth){
                        if(kind===0){
                            var separator=Object();
                            separator[Symbol.split]=function(){
                                if(depth!==0)return mixedSplitRecurse(1,depth-1);
                                return ["leaf"];
                            };
                            return "x".split(separator);
                        }
                        var regexp=Object(),holder=Object();
                        function Species(){
                            var splitter=Object();splitter.lastIndex=0;
                            splitter.exec=function(){
                                if(depth!==0)mixedSplitRecurse(0,depth-1);
                                return null;
                            };
                            return splitter;
                        }
                        holder[Symbol.species]=Species;regexp.constructor=holder;regexp.flags="";
                        return RegExp.prototype[Symbol.split].call(regexp,"x");
                    }"#,
                )
                .unwrap();

            for (entry, kind, safe_depth, overflow_depth) in [
                ("String.prototype.split", 0, 2, 3),
                ("RegExp @@split", 1, 3, 4),
            ] {
                assert_eq!(
                    context
                        .eval(&format!("mixedSplitRecurse({kind},{safe_depth}).length"))
                        .unwrap(),
                    Value::Int(1),
                    "the proven-safe mixed {entry} chain was rejected",
                );
                assert_eq!(
                    context
                        .eval(&format!(
                            r#"(function(){{
                                try{{mixedSplitRecurse({kind},{overflow_depth});return "missing"}}
                                catch(error){{return error.name+":"+error.message}}
                            }})()"#,
                        ))
                        .unwrap(),
                    string_value("InternalError:stack overflow"),
                    "the first unsafe mixed split frame was not rejected from {entry}",
                );
            }
            assert_eq!(
                context
                    .eval(
                        r#"__splitRecovery="a-b".split(/-/);
                           __splitRecovery.join("|")+"|"+
                           (function(){
                               var recovery=Object(),holder=Object();
                               holder[Symbol.species]=function(){
                                   return {lastIndex:0,exec:function(){return null}};
                               };
                               recovery.constructor=holder;recovery.flags="";
                               return RegExp.prototype[Symbol.split].call(recovery,"x").join("|");
                           })()"#,
                    )
                    .unwrap(),
                string_value("a|b|x"),
                "the runtime did not recover after mixed split overflow",
            );
        })
        .expect("2 MiB String/RegExp split stack-proof thread did not start")
        .join()
        .expect("2 MiB String/RegExp split stack-proof thread panicked");
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
        Value::Float(value) => quickjs_oxide::value::number_to_string(value),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}
