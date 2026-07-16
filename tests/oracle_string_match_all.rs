use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04 `js_string_match`'s
// Symbol.matchAll path and `check_regexp_g_flag` (`quickjs.c` 45583-45657).

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
function __step(iterator){
    var result=iterator.next(),value=result.value;
    return String(result.done)+":"+
        (value===undefined?"undefined":__show(value[0])+":"+
            __show(value.index));
}
"#;

const METADATA_CASES: &[(&str, &str)] = &[
    (
        "String matchAll exposes pinned metadata and prototype key order",
        r#"(function(){
            var fn=String.prototype.matchAll,
                keys=Object.getOwnPropertyNames(String.prototype),
                selected=["startsWith","match","matchAll","search","split","substring"],
                order=[],index=0,key;
            while(index<keys.length){
                key=keys[index++];
                if(selected.indexOf(key)>=0)order[order.length]=key;
            }
            return [
                order.join(","),__bits(String.prototype,"matchAll"),
                fn.name,fn.length,Object.getOwnPropertyNames(fn).join(","),
                __bits(fn,"name"),__bits(fn,"length"),__isConstructor(fn),
                Object.prototype.hasOwnProperty.call(fn,"prototype")
            ].join("|");
        })()"#,
    ),
    (
        "String matchAll AutoInit identity is stable and independently replaceable",
        r#"(function(){
            var first=String.prototype.matchAll,
                second=String.prototype.matchAll,
                stable=first===second,
                deleted=delete String.prototype.matchAll;
            String.prototype.matchAll=17;
            return [stable,deleted,String.prototype.matchAll,
                __bits(String.prototype,"matchAll"),
                typeof first,first.name,first.length].join("|");
        })()"#,
    ),
];

const DISPATCH_CASES: &[(&str, &str)] = &[
    (
        "String matchAll gets the hook then checks regexp identity flags and delegates raw this",
        r#"(function(){
            var log="",receiver=Object(),regexp=Object(),flags=Object(),result=Object();
            receiver.toString=function(){log+="BAD-receiver-string;";throw "converted"};
            Object.defineProperty(regexp,Symbol.matchAll,{get:function(){
                log+="matchAll-get;";
                return function(value){
                    log+="matchAll-call:"+(this===regexp)+":"+(value===receiver)+":"+
                        arguments.length+";";
                    return result;
                };
            }});
            Object.defineProperty(regexp,Symbol.match,{get:function(){
                log+="match-get;";return true;
            }});
            Object.defineProperty(regexp,"flags",{get:function(){
                log+="flags-get;";return flags;
            }});
            flags.toString=function(){log+="flags-string;";return "ig"};
            var actual=String.prototype.matchAll.call(receiver,regexp);
            return (actual===result)+"|"+log;
        })()"#,
    ),
    (
        "Symbol.match false bypasses the g check while true enforces it before hook call",
        r#"(function(){
            function run(matchValue,flags){
                var log="",regexp=Object(),result=Object();
                Object.defineProperty(regexp,Symbol.matchAll,{get:function(){
                    log+="all-get;";
                    return function(){log+="all-call;";return result};
                }});
                Object.defineProperty(regexp,Symbol.match,{get:function(){
                    log+="match-get;";return matchValue;
                }});
                Object.defineProperty(regexp,"flags",{get:function(){
                    log+="flags-get;";return flags;
                }});
                var completion=__completion(function(){
                    return String.prototype.matchAll.call("x",regexp)===result;
                });
                return completion+":"+log;
            }
            var real=/a/;real[Symbol.match]=false;
            real[Symbol.matchAll]=function(){return 81};
            return [
                run(false,""),run(true,""),run(true,"g"),
                String.prototype.matchAll.call("a",real)
            ].join("|");
        })()"#,
    ),
    (
        "nullish this is rejected before pattern hooks and noncallable hooks after the g check",
        r#"(function(){
            var log="",poison=Object(),noncallable=Object();
            Object.defineProperty(poison,Symbol.matchAll,{get:function(){
                log+="BAD-all;";throw 70;
            }});
            var nullResult=__completion(function(){
                    return String.prototype.matchAll.call(null,poison);
                }),
                undefinedResult=__completion(function(){
                    return String.prototype.matchAll.call(undefined,poison);
                });
            Object.defineProperty(noncallable,Symbol.matchAll,{get:function(){
                log+="all-get;";return 1;
            }});
            Object.defineProperty(noncallable,Symbol.match,{get:function(){
                log+="match-get;";return true;
            }});
            Object.defineProperty(noncallable,"flags",{get:function(){
                log+="flags-get;";return "g";
            }});
            var invalid=__completion(function(){
                return String.prototype.matchAll.call("x",noncallable);
            });
            return [nullResult,undefinedResult,invalid,log].join("|");
        })()"#,
    ),
    (
        "missing null and throwing flags fail before delegation or receiver conversion",
        r#"(function(){
            function run(flags,throws){
                var log="",receiver=Object(),regexp=Object();
                receiver.toString=function(){log+="BAD-receiver;";return "x"};
                Object.defineProperty(regexp,Symbol.matchAll,{get:function(){
                    log+="all-get;";
                    return function(){log+="BAD-call;";return 1};
                }});
                Object.defineProperty(regexp,Symbol.match,{get:function(){
                    log+="match-get;";return true;
                }});
                Object.defineProperty(regexp,"flags",{get:function(){
                    log+="flags-get;";
                    if(throws)throw 72;
                    return flags;
                }});
                return __completion(function(){
                    return String.prototype.matchAll.call(receiver,regexp);
                })+":"+log;
            }
            var poison=Object();
            poison.toString=function(){throw 73};
            return [run(undefined,false),run(null,false),run(poison,false),
                run("g",true)].join("|");
        })()"#,
    ),
];

const FALLBACK_CASES: &[(&str, &str)] = &[
    (
        "String matchAll fallback converts this then uses retained RegExp and dynamic protocol",
        r#"(function(){
            var log="",intrinsic=RegExp,receiver=Object(),pattern=Object(),
                result=Object(),gets=0;
            receiver.toString=function(){log+="receiver-string;";return "subject"};
            Object.defineProperty(pattern,Symbol.matchAll,{get:function(){
                log+="pattern-all:"+(++gets)+";";
                return null;
            }});
            Object.defineProperty(pattern,Symbol.match,{get:function(){
                log+="pattern-match;";return false;
            }});
            pattern.toString=function(){log+="pattern-string;";return "sub"};
            Object.defineProperty(intrinsic.prototype,Symbol.matchAll,{
                configurable:true,get:function(){
                    log+="new-all;";
                    return function(value){
                        log+="new-call:"+
                            (Object.getPrototypeOf(this)===intrinsic.prototype)+":"+
                            (value==="subject")+":"+this.flags+":"+arguments.length+";";
                        return result;
                    };
                }
            });
            globalThis.RegExp=function(){log+="BAD-global;";throw "global constructor used"};
            var actual=String.prototype.matchAll.call(receiver,pattern);
            return (actual===result)+"|"+log;
        })()"#,
    ),
    (
        "primitive patterns bypass boxed Symbol.matchAll hooks and become global regexps",
        r#"(function(){
            var hits="",output=[];
            function poison(prototype,label){
                Object.defineProperty(prototype,Symbol.matchAll,{configurable:true,get:function(){
                    hits+=label;throw label;
                }});
            }
            poison(String.prototype,"string;");
            poison(Number.prototype,"number;");
            poison(Boolean.prototype,"boolean;");
            poison(BigInt.prototype,"bigint;");
            var stringIterator=String.prototype.matchAll.call("aba","a"),
                numberIterator=String.prototype.matchAll.call("a7b",7),
                booleanIterator=String.prototype.matchAll.call("atrueb",true),
                bigintIterator=String.prototype.matchAll.call("a9b",9n);
            output[0]=__step(stringIterator)+","+__step(stringIterator)+","+
                __step(stringIterator);
            output[1]=__step(numberIterator)+","+__step(numberIterator);
            output[2]=__step(booleanIterator)+","+__step(booleanIterator);
            output[3]=__step(bigintIterator)+","+__step(bigintIterator);
            return output.join("|")+"|hits:"+hits;
        })()"#,
    ),
    (
        "native String matchAll preserves a global source lastIndex and clones its start",
        r#"(function(){
            var regexp=/a/g;regexp.lastIndex=1;
            var iterator=String.prototype.matchAll.call("baac",regexp);
            return [
                __step(iterator),__step(iterator),__step(iterator),
                regexp.lastIndex,Object.getPrototypeOf(iterator)===
                    Object.getPrototypeOf(/x/g[Symbol.matchAll](""))
            ].join("|");
        })()"#,
    ),
    (
        "null RegExp hook falls back through a fresh global clone starting at zero",
        r#"(function(){
            var regexp=/a/g;regexp.lastIndex=2;
            Object.defineProperty(regexp,Symbol.matchAll,{value:null});
            var iterator=String.prototype.matchAll.call("baac",regexp);
            return [
                __step(iterator),__step(iterator),__step(iterator),
                regexp.lastIndex
            ].join("|");
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "non-global RegExp rejection precedes hook call and receiver conversion",
        r#"(function(){
            var log="",receiver=Object(),regexp=/a/;
            receiver.toString=function(){log+="BAD-receiver;";return "a"};
            Object.defineProperty(regexp,Symbol.matchAll,{get:function(){
                log+="all-get;";
                return function(){log+="BAD-call;";return 1};
            }});
            Object.defineProperty(regexp,"flags",{get:function(){
                log+="flags-get;";return "i";
            }});
            return __completion(function(){
                return String.prototype.matchAll.call(receiver,regexp);
            })+"|"+log;
        })()"#,
    ),
    (
        "Symbol.matchAll and Symbol.match getter errors preserve exact short-circuit order",
        r#"(function(){
            function allError(){
                var log="",regexp=Object();
                Object.defineProperty(regexp,Symbol.matchAll,{get:function(){
                    log+="all;";throw 71;
                }});
                Object.defineProperty(regexp,Symbol.match,{get:function(){
                    log+="BAD-match;";return true;
                }});
                return __completion(function(){
                    return String.prototype.matchAll.call("x",regexp);
                })+":"+log;
            }
            function matchError(){
                var log="",regexp=Object();
                Object.defineProperty(regexp,Symbol.matchAll,{get:function(){
                    log+="all;";return function(){log+="BAD-call;";return 1};
                }});
                Object.defineProperty(regexp,Symbol.match,{get:function(){
                    log+="match;";throw 72;
                }});
                Object.defineProperty(regexp,"flags",{get:function(){
                    log+="BAD-flags;";return "g";
                }});
                return __completion(function(){
                    return String.prototype.matchAll.call("x",regexp);
                })+":"+log;
            }
            return allError()+"|"+matchError();
        })()"#,
    ),
    (
        "fallback input and pattern conversion errors occur before iterator creation",
        r#"(function(){
            function inputError(){
                var log="",receiver=Object(),pattern=Object();
                receiver.toString=function(){log+="input;";throw 73};
                pattern[Symbol.matchAll]=null;
                pattern[Symbol.match]=false;
                pattern.toString=function(){log+="BAD-pattern;";return "x"};
                return __completion(function(){
                    return String.prototype.matchAll.call(receiver,pattern);
                })+":"+log;
            }
            function patternError(){
                var log="",receiver=Object(),pattern=Object();
                receiver.toString=function(){log+="input;";return "x"};
                pattern[Symbol.matchAll]=null;
                pattern[Symbol.match]=false;
                pattern.toString=function(){log+="pattern;";throw 74};
                return __completion(function(){
                    return String.prototype.matchAll.call(receiver,pattern);
                })+":"+log;
            }
            return inputError()+"|"+patternError();
        })()"#,
    ),
];

#[test]
fn string_match_all_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String matchAll oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("metadata", METADATA_CASES),
        ("dispatch", DISPATCH_CASES),
        ("fallback", FALLBACK_CASES),
        ("errors", ERROR_CASES),
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
fn string_match_all_metadata_matches_pinned_quickjs() {
    compare_cases("String matchAll metadata", METADATA_CASES);
}

#[test]
fn string_match_all_g_check_and_delegation_match_pinned_quickjs() {
    compare_cases("String matchAll dispatch", DISPATCH_CASES);
}

#[test]
fn string_match_all_intrinsic_fallback_matches_pinned_quickjs() {
    compare_cases("String matchAll fallback", FALLBACK_CASES);
}

#[test]
fn string_match_all_abrupt_completion_order_matches_pinned_quickjs() {
    compare_cases("String matchAll errors", ERROR_CASES);
}

#[test]
fn string_match_all_delegation_and_fallback_preserve_cross_realm_ownership() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let method = eval_callable(
        &runtime,
        &mut defining,
        "String.prototype.matchAll",
        "defining String matchAll",
    );
    let defining_iterator_prototype = eval_object(
        &mut defining,
        "Object.getPrototypeOf(/x/g[Symbol.matchAll](\"\"))",
        "defining RegExp String Iterator prototype",
    );
    let defining_object_prototype = eval_object(
        &mut defining,
        "Object.prototype",
        "defining Object prototype",
    );
    let defining_array_prototype =
        eval_object(&mut defining, "Array.prototype", "defining Array prototype");
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let caller_iterator_prototype = eval_object(
        &mut caller,
        "Object.getPrototypeOf(/x/g[Symbol.matchAll](\"\"))",
        "caller RegExp String Iterator prototype",
    );
    let caller_object_prototype =
        eval_object(&mut caller, "Object.prototype", "caller Object prototype");
    let caller_array_prototype =
        eval_object(&mut caller, "Array.prototype", "caller Array prototype");
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );

    let Value::Object(fallback_iterator) = caller
        .call(&method, string_value("ba"), &[string_value("a")])
        .expect("cross-realm primitive String matchAll fallback")
    else {
        panic!("String matchAll primitive fallback did not return an iterator");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback_iterator).unwrap(),
        Some(defining_iterator_prototype),
        "String matchAll fallback did not use its defining RegExp intrinsic",
    );
    let fallback_next = callable_property(&runtime, &mut caller, &fallback_iterator, "next");
    let Value::Object(fallback_step) = caller
        .call(
            &fallback_next,
            Value::Object(fallback_iterator.clone()),
            &[],
        )
        .expect("fallback iterator next")
    else {
        panic!("fallback iterator next did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback_step).unwrap(),
        Some(defining_object_prototype),
        "fallback iterator result wrapper did not use the defining realm",
    );
    let Value::Object(fallback_match) =
        object_property(&runtime, &mut caller, &fallback_step, "value")
    else {
        panic!("fallback iterator did not yield a match object");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback_match).unwrap(),
        Some(defining_array_prototype),
        "fallback RegExp match result did not use the defining realm Array",
    );

    let foreign_regexp = eval_object(&mut caller, "/a/g", "caller global RegExp");
    let Value::Object(delegated_iterator) = caller
        .call(
            &method,
            string_value("ba"),
            &[Value::Object(foreign_regexp)],
        )
        .expect("cross-realm delegated String matchAll")
    else {
        panic!("delegated String matchAll did not return an iterator");
    };
    assert_eq!(
        runtime.get_prototype_of(&delegated_iterator).unwrap(),
        Some(caller_iterator_prototype),
        "String matchAll did not return the foreign RegExp hook result unchanged",
    );
    let delegated_next = callable_property(&runtime, &mut caller, &delegated_iterator, "next");
    let Value::Object(delegated_step) = caller
        .call(
            &delegated_next,
            Value::Object(delegated_iterator.clone()),
            &[],
        )
        .expect("delegated iterator next")
    else {
        panic!("delegated iterator next did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&delegated_step).unwrap(),
        Some(caller_object_prototype),
        "delegated iterator result wrapper moved into the String method realm",
    );
    let Value::Object(delegated_match) =
        object_property(&runtime, &mut caller, &delegated_step, "value")
    else {
        panic!("delegated iterator did not yield a match object");
    };
    assert_eq!(
        runtime.get_prototype_of(&delegated_match).unwrap(),
        Some(caller_array_prototype),
        "delegated foreign RegExp match result moved into the String method realm",
    );

    let nonglobal = eval_object(&mut caller, "/a/", "caller non-global RegExp");
    assert_eq!(
        caller.call(&method, string_value("a"), &[Value::Object(nonglobal)],),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "String matchAll g-check TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "String matchAll g-check TypeError did not use the method realm",
    );

    let throwing_pattern = eval_object(
        &mut caller,
        r#"(function(){
            var pattern=Object();
            pattern[Symbol.matchAll]=function(){return null};
            pattern[Symbol.match]=true;
            Object.defineProperty(pattern,"flags",{get:function(){
                throw new TypeError("caller flags");
            }});
            return pattern;
        })()"#,
        "caller throwing matchAll pattern",
    );
    assert_eq!(
        caller.call(
            &method,
            string_value("a"),
            &[Value::Object(throwing_pattern)],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller flags TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "String matchAll replaced a caller-realm flags exception",
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

fn callable_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let Value::Object(value) = object_property(runtime, context, object, name) else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn object_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> Value {
    let key = runtime.intern_property_key(name).unwrap();
    context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read property {name}: {error}"))
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
    let Value::String(value) = object_property(runtime, context, object, name) else {
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
        Value::Float(value) => {
            if value.is_nan() {
                "NaN".to_owned()
            } else if value == 0.0 && value.is_sign_negative() {
                "-0".to_owned()
            } else {
                value.to_string()
            }
        }
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Symbol(_) => "Symbol()".to_owned(),
    }
}
