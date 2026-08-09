use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04 `js_string_match`'s
// Symbol.search branch (`quickjs.c` 45609-45657), abstract RegExpExec
// (`quickjs.c` 48217-48236), and RegExp.prototype[Symbol.search]
// (`quickjs.c` 48817-48873).

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
"#;

const METADATA_CASES: &[(&str, &str)] = &[(
    "search methods expose pinned metadata descriptors and key order",
    r#"(function(){
            var stringFn=String.prototype.search,
                regexpFn=RegExp.prototype[Symbol.search],
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
                if(key==="test"||key==="toString"||key==="constructor"||key===Symbol.search)
                    regexpOrder[regexpOrder.length]=String(key);
                index++;
            }
            return [
                stringOrder.join(","),regexpOrder.join(","),
                __bits(String.prototype,"search"),stringFn.name,stringFn.length,
                Object.getOwnPropertyNames(stringFn).join(","),
                __bits(stringFn,"name"),__bits(stringFn,"length"),
                __isConstructor(stringFn),
                Object.prototype.hasOwnProperty.call(stringFn,"prototype"),
                __bits(RegExp.prototype,Symbol.search),regexpFn.name,regexpFn.length,
                Object.getOwnPropertyNames(regexpFn).join(","),
                __bits(regexpFn,"name"),__bits(regexpFn,"length"),
                __isConstructor(regexpFn),
                Object.prototype.hasOwnProperty.call(regexpFn,"prototype")
            ].join("|");
        })()"#,
)];

const STRING_DISPATCH_CASES: &[(&str, &str)] = &[
    (
        "String search delegates before conversion and returns the custom result unchanged",
        r#"(function(){
            var log="",receiver=Object(),regexp=Object(),result=Object();
            receiver.toString=function(){log+="BAD-receiver-string;";throw "converted"};
            Object.defineProperty(regexp,Symbol.search,{get:function(){
                log+="get;";
                return function(value){
                    log+="call:"+(this===regexp)+":"+(value===receiver)+":"+
                        arguments.length+";";
                    return result;
                };
            }});
            var actual=String.prototype.search.call(receiver,regexp);
            return (actual===result)+"|"+log;
        })()"#,
    ),
    (
        "String search rejection order noncallable hooks and primitive-pattern bypass",
        r#"(function(){
            var log="",poison=Object(),receiver=Object(),noncallable=Object(),hits=0;
            Object.defineProperty(poison,Symbol.search,{get:function(){log+="BAD-nullish;";return null}});
            var nullish=__completion(function(){return String.prototype.search.call(null,poison)});
            receiver.toString=function(){log+="BAD-noncallable-string;";return "a"};
            Object.defineProperty(noncallable,Symbol.search,{get:function(){log+="hook-get;";return 17}});
            var invalid=__completion(function(){return String.prototype.search.call(receiver,noncallable)});
            Object.defineProperty(Number.prototype,Symbol.search,{configurable:true,get:function(){
                hits++;throw "primitive hook observed";
            }});
            var fallbackReceiver=Object();
            fallbackReceiver.toString=function(){log+="fallback-string;";return "a7"};
            var fallback=String.prototype.search.call(fallbackReceiver,7);
            return [nullish,invalid,fallback,hits,log].join("|");
        })()"#,
    ),
];

const STRING_FALLBACK_CASES: &[(&str, &str)] = &[(
    "String search uses the intrinsic constructor and pinned fallback conversion order",
    r#"(function(){
            var log="",intrinsic=RegExp,receiver=Object(),pattern=Object(),
                source=Object(),flags=Object();
            Object.defineProperty(pattern,Symbol.search,{get:function(){log+="pattern-search;";return null}});
            Object.defineProperty(pattern,Symbol.match,{get:function(){log+="pattern-match;";return true}});
            Object.defineProperty(pattern,"source",{get:function(){log+="source-get;";return source}});
            Object.defineProperty(pattern,"flags",{get:function(){log+="flags-get;";return flags}});
            Object.defineProperty(pattern,"constructor",{get:function(){log+="BAD-constructor;";return intrinsic}});
            receiver.toString=function(){log+="receiver-string;";return "subject"};
            source.toString=function(){log+="source-string;";return "sub"};
            flags.toString=function(){log+="flags-string;";return ""};
            Object.defineProperty(intrinsic.prototype,Symbol.search,{configurable:true,get:function(){
                log+="new-search;";
                return function(value){
                    log+="new-call:"+(Object.getPrototypeOf(this)===intrinsic.prototype)+":"+
                        (value==="subject")+":"+__show(this.lastIndex)+";";
                    return 73;
                };
            }});
            globalThis.RegExp=function(){log+="BAD-global;";throw "global constructor used"};
            var result=String.prototype.search.call(receiver,pattern);
            return result+"|"+log;
        })()"#,
)];

const LAST_INDEX_CASES: &[(&str, &str)] = &[
    (
        "RegExp Symbol.search converts then restores negative zero before reading raw index",
        r#"(function(){
            var log="",state=-0,index=Object(),input=Object(),receiver=Object(),result=Object();
            input.toString=function(){log+="input-string;";return "abc"};
            Object.defineProperty(receiver,"lastIndex",{
                get:function(){log+="get:"+__show(state)+";";return state},
                set:function(value){log+="set:"+__show(value)+";";state=value}
            });
            Object.defineProperty(receiver,"exec",{get:function(){
                log+="exec-get;";
                return function(value){log+="exec-call:"+(this===receiver)+":"+value+";";return result};
            }});
            Object.defineProperty(result,"index",{get:function(){log+="index-get;";return index}});
            var actual=RegExp.prototype[Symbol.search].call(receiver,input);
            return [(actual===index),__show(state),log].join("|");
        })()"#,
    ),
    (
        "RegExp Symbol.search uses SameValue for positive zero and NaN",
        r#"(function(){
            function run(initial,writeNaN){
                var log="",state=initial,receiver=Object();
                Object.defineProperty(receiver,"lastIndex",{
                    get:function(){log+="get:"+__show(state)+";";return state},
                    set:function(value){log+="set:"+__show(value)+";";state=value}
                });
                receiver.exec=function(){
                    log+="exec;";
                    if(writeNaN)this.lastIndex=0/0;
                    return null;
                };
                var result=RegExp.prototype[Symbol.search].call(receiver,"x");
                return result+":"+__show(state)+":"+log;
            }
            return run(0,false)+"|"+run(0/0,true);
        })()"#,
    ),
];

const ABRUPT_CASES: &[(&str, &str)] = &[
    (
        "RegExp Symbol.search stops at initial lastIndex and exec abrupt boundaries",
        r#"(function(){
            function make(){
                var receiver=Object();receiver.state=5;receiver.log="";
                Object.defineProperty(receiver,"lastIndex",{
                    get:function(){this.log+="get;";return this.state},
                    set:function(value){this.log+="set:"+value+";";this.state=value}
                });
                return receiver;
            }
            function capture(receiver,sentinel){
                try{RegExp.prototype[Symbol.search].call(receiver,"x");return "return"}
                catch(error){
                    var kind=error===sentinel?"same":error.name+":"+error.message;
                    return kind+":"+receiver.state+":"+receiver.log;
                }
            }
            var initialGet=Object(),initialGetBoom=Object();
            initialGet.state=5;initialGet.log="";
            Object.defineProperty(initialGet,"lastIndex",{
                get:function(){this.log+="get;";throw initialGetBoom},
                set:function(){this.log+="BAD-set;"}
            });
            initialGet.exec=function(){this.log+="BAD-exec;";return null};

            var initialSet=Object(),initialSetBoom=Object();
            initialSet.state=5;initialSet.log="";
            Object.defineProperty(initialSet,"lastIndex",{
                get:function(){this.log+="get;";return this.state},
                set:function(value){this.log+="set:"+value+";";throw initialSetBoom}
            });
            initialSet.exec=function(){this.log+="BAD-exec;";return null};

            var first=make(),firstBoom=Object();
            Object.defineProperty(first,"exec",{get:function(){this.log+="exec-get;";throw firstBoom}});
            var second=make(),secondBoom=Object();
            second.exec=function(){this.log+="exec-call;";this.state=9;throw secondBoom};
            var third=make();
            third.exec=function(){this.log+="exec-call;";this.state=9;return 1};
            return [capture(initialGet,initialGetBoom),capture(initialSet,initialSetBoom),
                capture(first,firstBoom),capture(second,secondBoom),capture(third,null)].join("|");
        })()"#,
    ),
    (
        "RegExp Symbol.search post-exec failures bracket restoration and index access",
        r#"(function(){
            function capture(receiver,sentinel){
                try{RegExp.prototype[Symbol.search].call(receiver,"x");return "return"}
                catch(error){
                    return (error===sentinel)+":"+receiver.state+":"+receiver.log;
                }
            }
            var current=Object(),currentBoom=Object(),currentGets=0;
            current.state=5;current.log="";
            Object.defineProperty(current,"lastIndex",{
                get:function(){this.log+="get:"+(++currentGets)+";";if(currentGets===2)throw currentBoom;return this.state},
                set:function(value){this.log+="set:"+value+";";this.state=value}
            });
            current.exec=function(){this.log+="exec;";return null};

            var restore=Object(),restoreBoom=Object(),restoreSets=0,restoreResult=Object();
            restore.state=5;restore.log="";
            Object.defineProperty(restore,"lastIndex",{
                get:function(){this.log+="get;";return this.state},
                set:function(value){this.log+="set:"+value+";";if(++restoreSets===2)throw restoreBoom;this.state=value}
            });
            Object.defineProperty(restoreResult,"index",{get:function(){restore.log+="BAD-index;";return 1}});
            restore.exec=function(){this.log+="exec;";return restoreResult};

            var index=Object(),indexBoom=Object(),indexResult=Object();
            index.state=5;index.log="";
            Object.defineProperty(index,"lastIndex",{
                get:function(){this.log+="get;";return this.state},
                set:function(value){this.log+="set:"+value+";";this.state=value}
            });
            Object.defineProperty(indexResult,"index",{get:function(){index.log+="index;";throw indexBoom}});
            index.exec=function(){this.log+="exec;";this.state=8;return indexResult};

            return [capture(current,currentBoom),capture(restore,restoreBoom),
                capture(index,indexBoom)].join("|");
        })()"#,
    ),
];

const ABSTRACT_EXEC_CASES: &[(&str, &str)] = &[(
    "RegExp Symbol.search honors builtin fallback branding null and raw index results",
    r#"(function(){
            var branded=/b/g;branded.lastIndex=7;branded.exec=null;
            var brandedResult=RegExp.prototype[Symbol.search].call(branded,"abc");

            var ordinary=Object();ordinary.exec=null;
            var ordinaryCompletion=__completion(function(){
                return RegExp.prototype[Symbol.search].call(ordinary,"abc");
            });

            var miss=Object();miss.exec=function(){return null};
            var missResult=RegExp.prototype[Symbol.search].call(miss,"abc");

            var raw=Object();raw.exec=function(){var result=Object();result.index="raw";return result};
            var rawResult=RegExp.prototype[Symbol.search].call(raw,"abc");

            return [brandedResult,branded.lastIndex,ordinaryCompletion,
                Object.prototype.hasOwnProperty.call(ordinary,"lastIndex"),__show(ordinary.lastIndex),
                missResult,Object.prototype.hasOwnProperty.call(miss,"lastIndex"),__show(miss.lastIndex),
                rawResult,Object.prototype.hasOwnProperty.call(raw,"lastIndex"),__show(raw.lastIndex)
            ].join("|");
        })()"#,
)];

#[test]
fn string_search_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String/RegExp search oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("metadata", METADATA_CASES),
        ("String dispatch", STRING_DISPATCH_CASES),
        ("String fallback", STRING_FALLBACK_CASES),
        ("lastIndex", LAST_INDEX_CASES),
        ("abrupt completion", ABRUPT_CASES),
        ("abstract RegExpExec", ABSTRACT_EXEC_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector had no completion for {description}: {observation:?}",
            );
        }
    }
}

#[test]
fn search_metadata_and_property_order_match_pinned_quickjs() {
    compare_cases("search metadata", METADATA_CASES);
}

#[test]
fn string_search_protocol_dispatch_matches_pinned_quickjs() {
    compare_cases("String search dispatch", STRING_DISPATCH_CASES);
}

#[test]
fn string_search_intrinsic_fallback_matches_pinned_quickjs() {
    compare_cases("String search fallback", STRING_FALLBACK_CASES);
}

#[test]
fn regexp_symbol_search_last_index_and_same_value_match_pinned_quickjs() {
    compare_cases("RegExp Symbol.search lastIndex", LAST_INDEX_CASES);
}

#[test]
fn regexp_symbol_search_abrupt_completion_matches_pinned_quickjs() {
    compare_cases("RegExp Symbol.search abrupt completion", ABRUPT_CASES);
}

#[test]
fn regexp_symbol_search_abstract_exec_matches_pinned_quickjs() {
    compare_cases("RegExp Symbol.search abstract exec", ABSTRACT_EXEC_CASES);
}

#[test]
fn search_intrinsics_use_defining_realms_and_accept_foreign_regexp_brands() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let string_search = eval_callable(
        &runtime,
        &mut defining,
        "String.prototype.search",
        "defining String search",
    );
    let regexp_search = eval_callable(
        &runtime,
        &mut defining,
        "RegExp.prototype[Symbol.search]",
        "defining RegExp Symbol.search",
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
        .eval("RegExp.prototype[Symbol.search]=function(){return 41}")
        .unwrap();
    caller
        .eval("RegExp.prototype[Symbol.search]=function(){return 99}")
        .unwrap();
    assert_eq!(
        caller
            .call(
                &string_search,
                string_value("subject"),
                &[string_value("needle")],
            )
            .unwrap(),
        Value::Int(41),
        "String search fallback did not use its defining-realm RegExp intrinsic",
    );

    let foreign_regexp = eval_object(
        &mut caller,
        "(function(){var r=/b/g;r.lastIndex=7;r.exec=null;return r})()",
        "caller RegExp with builtin exec fallback",
    );
    assert_eq!(
        caller
            .call(
                &regexp_search,
                Value::Object(foreign_regexp.clone()),
                &[string_value("abc")],
            )
            .unwrap(),
        Value::Int(1),
        "foreign branded RegExp did not pass defining-realm builtin fallback",
    );
    let last_index_key = runtime.intern_property_key("lastIndex").unwrap();
    assert_eq!(
        caller
            .get_property(&foreign_regexp, &last_index_key)
            .unwrap(),
        Value::Int(7),
        "foreign RegExp lastIndex was not restored",
    );

    assert_eq!(
        caller.call(&regexp_search, Value::Int(1), &[string_value("x")]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "defining-realm search TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "native RegExp Symbol.search TypeError used the caller realm",
    );

    let throwing_receiver = eval_object(
        &mut caller,
        r#"(function(){
            var receiver=Object();receiver.lastIndex=0;
            receiver.exec=function(){throw new TypeError("caller exec")};
            return receiver;
        })()"#,
        "caller throwing custom exec receiver",
    );
    assert_eq!(
        caller.call(
            &regexp_search,
            Value::Object(throwing_receiver),
            &[string_value("x")],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller custom exec TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "RegExp Symbol.search replaced a caller-realm user exception",
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
