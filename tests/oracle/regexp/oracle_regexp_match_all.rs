use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04
// `js_regexp_Symbol_matchAll` (`quickjs.c` 48419-48482) and the RegExp String
// Iterator next operation (`quickjs.c` 48365-48417).

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
        "RegExp matchAll and iterator expose pinned metadata and key order",
        r#"(function(){
            var fn=RegExp.prototype[Symbol.matchAll],
                iterator=/a/g[Symbol.matchAll]("a"),
                prototype=Object.getPrototypeOf(iterator),
                next=prototype.next,
                regexpKeys=Reflect.ownKeys(RegExp.prototype),
                selected=[],index=0,key;
            while(index<regexpKeys.length){
                key=regexpKeys[index++];
                if(key==="test"||key==="toString"||key===Symbol.match||
                    key===Symbol.matchAll||key===Symbol.search||key===Symbol.split)
                    selected[selected.length]=String(key);
            }
            return [
                selected.join(","),
                __bits(RegExp.prototype,Symbol.matchAll),fn.name,fn.length,
                Object.getOwnPropertyNames(fn).join(","),
                __bits(fn,"name"),__bits(fn,"length"),__isConstructor(fn),
                Object.prototype.hasOwnProperty.call(fn,"prototype"),
                Reflect.ownKeys(prototype).map(String).join(","),
                __bits(prototype,"next"),next.name,next.length,
                Object.getOwnPropertyNames(next).join(","),
                __bits(next,"name"),__bits(next,"length"),__isConstructor(next),
                __bits(prototype,Symbol.toStringTag),
                prototype[Symbol.toStringTag],
                iterator[Symbol.iterator]()===iterator,
                Object.prototype.toString.call(iterator)
            ].join("|");
        })()"#,
    ),
    (
        "RegExp matchAll AutoInit identity is stable and independently replaceable",
        r#"(function(){
            var first=RegExp.prototype[Symbol.matchAll],
                second=RegExp.prototype[Symbol.matchAll],
                stable=first===second,
                deleted=delete RegExp.prototype[Symbol.matchAll];
            RegExp.prototype[Symbol.matchAll]=17;
            return [stable,deleted,RegExp.prototype[Symbol.matchAll],
                __bits(RegExp.prototype,Symbol.matchAll),
                typeof first,first.name,first.length].join("|");
        })()"#,
    ),
];

const CONSTRUCTION_CASES: &[(&str, &str)] = &[
    (
        "matchAll converts input then resolves species flags construction and lastIndex",
        r#"(function(){
            var log="",receiver=Object(),holder=Object(),input=Object(),
                flags=Object(),length=Object(),matcher;
            input.toString=function(){log+="input-string;";return "subject"};
            Object.defineProperty(receiver,"constructor",{get:function(){
                log+="constructor-get;";return holder;
            }});
            Object.defineProperty(holder,Symbol.species,{get:function(){
                log+="species-get:"+(this===holder)+";";return Species;
            }});
            Object.defineProperty(receiver,"flags",{get:function(){
                log+="flags-get;";return flags;
            }});
            flags.toString=function(){log+="flags-string;";return "g"};
            Object.defineProperty(receiver,"lastIndex",{get:function(){
                log+="last-get;";return length;
            }});
            length.valueOf=function(){log+="last-number;";return 2.9};
            function Species(pattern,observedFlags){
                log+="construct:"+(new.target===Species)+":"+(pattern===receiver)+":"+
                    observedFlags+":"+arguments.length+";";
                matcher=Object();
                Object.defineProperty(matcher,"lastIndex",{
                    get:function(){return 0},
                    set:function(value){log+="matcher-last-set:"+value+";"}
                });
                matcher.exec=function(){return null};
                return matcher;
            }
            var iterator=RegExp.prototype[Symbol.matchAll].call(receiver,input);
            return log+"|"+Object.prototype.toString.call(iterator)+
                "|"+(iterator[Symbol.iterator]()===iterator);
        })()"#,
    ),
    (
        "matchAll abrupt completions stop at exact construction boundaries",
        r#"(function(){
            function primitive(){
                var log="",input=Object();
                input.toString=function(){log+="BAD-input;";return "x"};
                return __completion(function(){
                    return RegExp.prototype[Symbol.matchAll].call(1,input);
                })+":"+log;
            }
            function badSpecies(){
                var log="",receiver=Object(),holder=Object(),input=Object();
                input.toString=function(){log+="input;";return "x"};
                Object.defineProperty(receiver,"constructor",{get:function(){
                    log+="constructor;";return holder;
                }});
                Object.defineProperty(holder,Symbol.species,{get:function(){
                    log+="species;";return 1;
                }});
                Object.defineProperty(receiver,"flags",{get:function(){
                    log+="BAD-flags;";return "g";
                }});
                return __completion(function(){
                    return RegExp.prototype[Symbol.matchAll].call(receiver,input);
                })+":"+log;
            }
            function badFlags(){
                var log="",receiver=Object(),holder=Object(),flags=Object();
                receiver.constructor=holder;holder[Symbol.species]=function(){
                    log+="BAD-construct;";return Object();
                };
                Object.defineProperty(receiver,"flags",{get:function(){
                    log+="flags-get;";return flags;
                }});
                flags.toString=function(){log+="flags-string;";throw 73};
                return __completion(function(){
                    return RegExp.prototype[Symbol.matchAll].call(receiver,"x");
                })+":"+log;
            }
            function badConstruct(){
                var log="",receiver=Object(),holder=Object();
                receiver.constructor=holder;receiver.flags="g";
                holder[Symbol.species]=function(){log+="construct;";throw 74};
                Object.defineProperty(receiver,"lastIndex",{get:function(){
                    log+="BAD-last;";return 0;
                }});
                return __completion(function(){
                    return RegExp.prototype[Symbol.matchAll].call(receiver,"x");
                })+":"+log;
            }
            return [primitive(),badSpecies(),badFlags(),badConstruct()].join("|");
        })()"#,
    ),
    (
        "nullish species uses the retained RegExp constructor and ignores global replacement",
        r#"(function(){
            var intrinsicRegExp=RegExp,intrinsicIteratorPrototype=
                    Object.getPrototypeOf(/x/g[Symbol.matchAll]("")),
                method=RegExp.prototype[Symbol.matchAll],
                first=/a/g,second=/b/g,firstHolder=Object(),secondHolder=Object();
            firstHolder[Symbol.species]=null;
            secondHolder[Symbol.species]=undefined;
            first.constructor=firstHolder;second.constructor=secondHolder;
            globalThis.RegExp=function(){throw "global RegExp used"};
            var left=method.call(first,"a"),right=method.call(second,"b");
            return [
                __step(left),__step(left),__step(right),__step(right),
                Object.getPrototypeOf(left)===intrinsicIteratorPrototype,
                Object.getPrototypeOf(right)===intrinsicIteratorPrototype,
                Object.getPrototypeOf(first)===intrinsicRegExp.prototype
            ].join("|");
        })()"#,
    ),
    (
        "source lastIndex is ToLength copied once without mutating the source",
        r#"(function(){
            function run(initial){
                var receiver=Object(),holder=Object(),copied="missing",sets=0;
                receiver.constructor=holder;receiver.flags="g";receiver.lastIndex=initial;
                holder[Symbol.species]=function(){
                    var matcher=Object();
                    Object.defineProperty(matcher,"lastIndex",{set:function(value){
                        sets++;copied=__show(value);
                    }});
                    matcher.exec=function(){return null};
                    return matcher;
                };
                var iterator=RegExp.prototype[Symbol.matchAll].call(receiver,"x");
                return copied+":"+sets+":"+__show(receiver.lastIndex)+":"+
                    Object.prototype.toString.call(iterator);
            }
            var number=Object();number.valueOf=function(){return 3.8};
            return [run(-4),run(number),run(Infinity)].join("|");
        })()"#,
    ),
];

const ITERATION_CASES: &[(&str, &str)] = &[
    (
        "non-global iterators finish after one match while global iterators run to null",
        r#"(function(){
            function run(flags){
                var receiver=Object(),holder=Object(),calls=0;
                receiver.constructor=holder;receiver.flags=flags;receiver.lastIndex=0;
                holder[Symbol.species]=function(){
                    return {lastIndex:0,exec:function(){
                        calls++;
                        if(calls<=2)return {0:"m"+calls,index:calls};
                        return null;
                    }};
                };
                var iterator=RegExp.prototype[Symbol.matchAll].call(receiver,"subject");
                return [__step(iterator),__step(iterator),__step(iterator),
                    __step(iterator),calls].join(",");
            }
            return run("")+"|"+run("g");
        })()"#,
    ),
    (
        "real RegExp iterators copy lastIndex and preserve the source regexp",
        r#"(function(){
            var global=/a/g,plain=/a/;
            global.lastIndex=1;plain.lastIndex=2;
            var globalIterator=RegExp.prototype[Symbol.matchAll].call(global,"baac"),
                plainIterator=RegExp.prototype[Symbol.matchAll].call(plain,"baac");
            return [
                __step(globalIterator),__step(globalIterator),__step(globalIterator),
                global.lastIndex,
                __step(plainIterator),__step(plainIterator),plain.lastIndex
            ].join("|");
        })()"#,
    ),
    (
        "empty global matches advance by code unit or Unicode code point from matcher lastIndex",
        r#"(function(){
            function run(flags){
                var receiver=Object(),holder=Object(),state=0,log="",calls=0;
                receiver.constructor=holder;receiver.flags=flags;receiver.lastIndex=0;
                holder[Symbol.species]=function(){
                    var matcher=Object();
                    Object.defineProperty(matcher,"lastIndex",{
                        get:function(){log+="get:"+state+";";return state},
                        set:function(value){log+="set:"+value+";";state=value}
                    });
                    matcher.exec=function(){
                        log+="exec:"+state+";";calls++;
                        return calls===1?{0:"",index:state}:null;
                    };
                    return matcher;
                };
                var iterator=RegExp.prototype[Symbol.matchAll].call(
                    receiver,String.fromCharCode(0xd83d,0xde00,0x58));
                return [__step(iterator),__step(iterator),__step(iterator),log].join(",");
            }
            return [run(""),run("g"),run("gu"),run("gv")].join("|");
        })()"#,
    ),
    (
        "iterator next result objects preserve match identity and stable done state",
        r#"(function(){
            var receiver=Object(),holder=Object(),match=Object(),calls=0;
            match[0]="x";match.index=4;
            receiver.constructor=holder;receiver.flags="g";receiver.lastIndex=0;
            holder[Symbol.species]=function(){
                return {lastIndex:0,exec:function(){calls++;return calls===1?match:null}};
            };
            var iterator=RegExp.prototype[Symbol.matchAll].call(receiver,"subject"),
                first=iterator.next(),second=iterator.next(),third=iterator.next();
            return [
                first.value===match,first.done,
                second.value,second.done,
                third.value,third.done,calls,
                Object.getPrototypeOf(first)===Object.prototype,
                Object.getPrototypeOf(second)===Object.prototype
            ].join("|");
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "iterator next rejects incompatible receivers without running user code",
        r#"(function(){
            var iterator=/a/g[Symbol.matchAll]("a"),
                next=Object.getPrototypeOf(iterator).next;
            return [
                __completion(function(){return next.call(undefined)}),
                __completion(function(){return next.call(null)}),
                __completion(function(){return next.call(1)}),
                __completion(function(){return next.call(Object())}),
                __step(iterator),__step(iterator)
            ].join("|");
        })()"#,
    ),
    (
        "exec throws and primitive results leave the iterator retryable",
        r#"(function(){
            var receiver=Object(),holder=Object(),calls=0;
            receiver.constructor=holder;receiver.flags="g";receiver.lastIndex=0;
            holder[Symbol.species]=function(){
                return {lastIndex:0,exec:function(){
                    calls++;
                    if(calls===1)throw 71;
                    if(calls===2)return 1;
                    return null;
                }};
            };
            var iterator=RegExp.prototype[Symbol.matchAll].call(receiver,"x");
            return [
                __completion(function(){return iterator.next()}),
                __completion(function(){return iterator.next()}),
                __step(iterator),__step(iterator),calls
            ].join("|");
        })()"#,
    ),
    (
        "match zero and empty-match lastIndex errors leave the iterator retryable",
        r#"(function(){
            function zeroGetter(){
                var receiver=Object(),holder=Object(),calls=0,match=Object();
                receiver.constructor=holder;receiver.flags="g";receiver.lastIndex=0;
                Object.defineProperty(match,"0",{get:function(){throw 72}});
                holder[Symbol.species]=function(){
                    return {lastIndex:0,exec:function(){
                        calls++;return calls===1?match:null;
                    }};
                };
                var iterator=RegExp.prototype[Symbol.matchAll].call(receiver,"x");
                return __completion(function(){return iterator.next()})+":"+
                    __step(iterator)+":"+__step(iterator)+":"+calls;
            }
            function lastIndexGetter(){
                var receiver=Object(),holder=Object(),calls=0,state=0;
                receiver.constructor=holder;receiver.flags="g";receiver.lastIndex=0;
                holder[Symbol.species]=function(){
                    var matcher=Object();
                    Object.defineProperty(matcher,"lastIndex",{
                        get:function(){throw 73},
                        set:function(value){state=value}
                    });
                    matcher.exec=function(){
                        calls++;return calls===1?{0:"",index:state}:null;
                    };
                    return matcher;
                };
                var iterator=RegExp.prototype[Symbol.matchAll].call(receiver,"x");
                return __completion(function(){return iterator.next()})+":"+
                    __step(iterator)+":"+__step(iterator)+":"+calls;
            }
            return zeroGetter()+"|"+lastIndexGetter();
        })()"#,
    ),
];

#[test]
fn regexp_match_all_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp matchAll oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("metadata", METADATA_CASES),
        ("construction", CONSTRUCTION_CASES),
        ("iteration", ITERATION_CASES),
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
fn regexp_match_all_metadata_matches_pinned_quickjs() {
    compare_cases("RegExp matchAll metadata", METADATA_CASES);
}

#[test]
fn regexp_match_all_species_order_and_last_index_match_pinned_quickjs() {
    compare_cases("RegExp matchAll construction", CONSTRUCTION_CASES);
}

#[test]
fn regexp_match_all_iterator_progress_matches_pinned_quickjs() {
    compare_cases("RegExp matchAll iteration", ITERATION_CASES);
}

#[test]
fn regexp_match_all_iterator_errors_match_pinned_quickjs() {
    compare_cases("RegExp matchAll errors", ERROR_CASES);
}

#[test]
fn regexp_match_all_iterator_and_match_results_use_their_defining_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let method = eval_callable(
        &runtime,
        &mut defining,
        "RegExp.prototype[Symbol.matchAll]",
        "defining RegExp Symbol.matchAll",
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
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let caller_array_prototype =
        eval_object(&mut caller, "Array.prototype", "caller Array prototype");
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );

    let foreign_regexp = eval_object(
        &mut caller,
        "(function(){var value=/a/g;value.lastIndex=1;return value})()",
        "caller RegExp",
    );
    let Value::Object(iterator) = caller
        .call(
            &method,
            Value::Object(foreign_regexp),
            &[string_value("ba")],
        )
        .expect("cross-realm RegExp matchAll")
    else {
        panic!("cross-realm RegExp matchAll did not return an iterator");
    };
    assert_eq!(
        runtime.get_prototype_of(&iterator).unwrap(),
        Some(defining_iterator_prototype),
        "RegExp String Iterator did not use the method defining realm",
    );

    let next = callable_property(&runtime, &mut caller, &iterator, "next");
    let Value::Object(iteration_result) = caller
        .call(&next, Value::Object(iterator.clone()), &[])
        .expect("cross-realm RegExp String Iterator next")
    else {
        panic!("RegExp String Iterator next did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&iteration_result).unwrap(),
        Some(defining_object_prototype),
        "iterator result wrapper did not use the iterator method realm",
    );
    let Value::Object(match_result) =
        object_property(&runtime, &mut caller, &iteration_result, "value")
    else {
        panic!("RegExp String Iterator did not yield a match object");
    };
    assert_eq!(
        runtime.get_prototype_of(&match_result).unwrap(),
        Some(caller_array_prototype),
        "species-created foreign RegExp did not retain its match-result Array realm",
    );

    assert_eq!(
        caller.call(&method, Value::Int(1), &[string_value("x")]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "RegExp matchAll receiver TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "native RegExp matchAll TypeError did not use the method realm",
    );

    let throwing_input = eval_object(
        &mut caller,
        r#"(function(){
            var input=Object();
            input.toString=function(){throw new TypeError("caller input")};
            return input;
        })()"#,
        "caller throwing matchAll input",
    );
    let receiver = eval_object(&mut caller, "/a/g", "caller RegExp for throwing input");
    assert_eq!(
        caller.call(
            &method,
            Value::Object(receiver),
            &[Value::Object(throwing_input)],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller input TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "RegExp matchAll replaced a caller-realm user exception",
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
