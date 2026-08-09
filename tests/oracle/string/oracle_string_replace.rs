use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04 `js_string_replace`
// (`quickjs.c` 45781-45892), including the shared GetSubstitution helper
// (`quickjs.c` 45661-45779), String.prototype.replace, and
// String.prototype.replaceAll.

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
function __units(value){
    var string=String(value),output=[],index=0;
    while(index<string.length){
        output[index]=string.charCodeAt(index).toString(16);
        index++;
    }
    return string.length+"["+output.join(",")+"]";
}
"#;

const METADATA_CASES: &[(&str, &str)] = &[
    (
        "String replace methods expose pinned metadata descriptors and table order",
        r#"(function(){
            var replace=String.prototype.replace,
                replaceAll=String.prototype.replaceAll,
                keys=Object.getOwnPropertyNames(String.prototype),
                selected=["slice","repeat","replace","replaceAll","padEnd","padStart","trim"],
                order=[],index=0,key;
            while(index<keys.length){
                key=keys[index++];
                if(selected.indexOf(key)>=0)order[order.length]=key;
            }
            return [
                order.join(","),
                __bits(String.prototype,"replace"),replace.name,replace.length,
                Object.getOwnPropertyNames(replace).join(","),
                __bits(replace,"name"),__bits(replace,"length"),
                __isConstructor(replace),
                Object.prototype.hasOwnProperty.call(replace,"prototype"),
                __bits(String.prototype,"replaceAll"),replaceAll.name,replaceAll.length,
                Object.getOwnPropertyNames(replaceAll).join(","),
                __bits(replaceAll,"name"),__bits(replaceAll,"length"),
                __isConstructor(replaceAll),
                Object.prototype.hasOwnProperty.call(replaceAll,"prototype"),
                replace===replaceAll
            ].join("|");
        })()"#,
    ),
    (
        "String replace AutoInit entries retain identity and mutate independently",
        r#"(function(){
            var replace=String.prototype.replace,
                replaceAll=String.prototype.replaceAll,
                stable=replace===String.prototype.replace&&
                    replaceAll===String.prototype.replaceAll,
                deleteReplace=delete String.prototype.replace,
                deleteReplaceAll=delete String.prototype.replaceAll;
            String.prototype.replace=17;
            String.prototype.replaceAll=23;
            return [stable,deleteReplace,deleteReplaceAll,
                String.prototype.replace,String.prototype.replaceAll,
                __bits(String.prototype,"replace"),
                __bits(String.prototype,"replaceAll"),
                replace===replaceAll].join("|");
        })()"#,
    ),
];

const PROTOCOL_CASES: &[(&str, &str)] = &[
    (
        "replace and replaceAll intercept object protocols before any string conversion",
        r#"(function(){
            function run(all){
                var log="",receiver=Object(),search=Object(),replacement=Object(),result=Object();
                receiver.toString=function(){log+="BAD-receiver;";throw "receiver"};
                replacement.toString=function(){log+="BAD-replacement;";throw "replacement"};
                if(all){
                    Object.defineProperty(search,Symbol.match,{get:function(){
                        log+="match-get;";return false;
                    }});
                }
                Object.defineProperty(search,Symbol.replace,{get:function(){
                    log+="replace-get;";
                    return function(input,value){
                        log+="replace-call:"+(this===search)+":"+(input===receiver)+":"+
                            (value===replacement)+":"+arguments.length+";";
                        return result;
                    };
                }});
                var actual=all?
                    String.prototype.replaceAll.call(receiver,search,replacement):
                    String.prototype.replace.call(receiver,search,replacement);
                return (actual===result)+":"+log;
            }
            return run(false)+"|"+run(true);
        })()"#,
    ),
    (
        "replaceAll validates IsRegExp and global flags before reading Symbol.replace",
        r#"(function(){
            function run(matchValue,flagsValue){
                var log="",search=Object(),receiver=Object(),replacement=Object();
                Object.defineProperty(search,Symbol.match,{get:function(){
                    log+="match-get;";return matchValue;
                }});
                Object.defineProperty(search,"flags",{get:function(){
                    log+="flags-get;";return flagsValue;
                }});
                Object.defineProperty(search,Symbol.replace,{get:function(){
                    log+="replace-get;";
                    return function(input,value){
                        log+="replace-call:"+(input===receiver)+":"+
                            (value===replacement)+";";
                        return "hook";
                    };
                }});
                if(flagsValue!==null&&flagsValue!==undefined&&typeof flagsValue==="object")
                    flagsValue.toString=function(){log+="flags-string;";return this.value};
                var completion=__completion(function(){
                    return String.prototype.replaceAll.call(receiver,search,replacement);
                });
                return completion+":"+log;
            }
            var globalFlags=Object();globalFlags.value="ug",
                localFlags=Object();localFlags.value="u";
            return [
                run(true,globalFlags),
                run(true,localFlags),
                run(true,null),
                run(false,localFlags)
            ].join("|");
        })()"#,
    ),
    (
        "primitive search values bypass boxed hooks and invalid hooks precede receiver conversion",
        r#"(function(){
            var hits="",log="",receiver=Object(),invalid=Object(),poison=Object();
            Object.defineProperty(Number.prototype,Symbol.replace,{configurable:true,get:function(){
                hits+="number;";throw "number hook";
            }});
            Object.defineProperty(String.prototype,Symbol.replace,{configurable:true,get:function(){
                hits+="string;";throw "string hook";
            }});
            receiver.toString=function(){log+="BAD-receiver;";return "a1b"};
            Object.defineProperty(invalid,Symbol.replace,{get:function(){
                log+="invalid-get;";return 17;
            }});
            Object.defineProperty(poison,Symbol.replace,{get:function(){
                log+="BAD-nullish;";return function(){};
            }});
            return [
                String.prototype.replace.call("a1b1",1,"X"),
                String.prototype.replaceAll.call("a,b,c",",","X"),
                __completion(function(){
                    return String.prototype.replace.call(receiver,invalid,"X");
                }),
                __completion(function(){
                    return String.prototype.replace.call(null,poison,"X");
                }),
                hits,log
            ].join("|");
        })()"#,
    ),
];

const CONVERSION_CASES: &[(&str, &str)] = &[
    (
        "string fallback converts receiver search and noncallable replacement before searching",
        r#"(function(){
            function run(all,needle){
                var log="",receiver=Object(),search=Object(),replacement=Object();
                receiver.toString=function(){log+="receiver;";return "abcabc"};
                search.toString=function(){log+="search;";return needle};
                replacement.toString=function(){log+="replacement;";return "X"};
                var result=all?
                    String.prototype.replaceAll.call(receiver,search,replacement):
                    String.prototype.replace.call(receiver,search,replacement);
                return result+":"+log;
            }
            return [run(false,"z"),run(true,"b"),run(true,"z")].join("|");
        })()"#,
    ),
    (
        "functional replacements skip upfront ToString and receive exact arguments",
        r#"(function(){
            function run(all,needle){
                var log="",receiver=Object(),search=Object(),calls=0;
                receiver.toString=function(){log+="receiver;";return "aba"};
                search.toString=function(){log+="search;";return needle};
                function replacement(match,position,input){
                    "use strict";
                    calls++;
                    log+="call:"+(this===undefined)+":"+match+":"+position+":"+
                        input+":"+arguments.length+";";
                    var result=Object();
                    result.toString=function(){log+="result-string;";return "X"};
                    return result;
                }
                replacement.toString=function(){log+="BAD-function-string;";throw "function"};
                var result=all?
                    String.prototype.replaceAll.call(receiver,search,replacement):
                    String.prototype.replace.call(receiver,search,replacement);
                return result+":"+calls+":"+log;
            }
            return [run(false,"a"),run(true,"a"),run(true,"z")].join("|");
        })()"#,
    ),
    (
        "fallback abrupt completions stop at receiver search and replacement boundaries",
        r#"(function(){
            function run(stage){
                var log="",sentinel=Object(),receiver=Object(),search=Object(),replacement=Object(),
                    same=false;
                receiver.toString=function(){log+="receiver;";if(stage===0)throw sentinel;return "abc"};
                search.toString=function(){log+="search;";if(stage===1)throw sentinel;return "z"};
                replacement.toString=function(){log+="replacement;";if(stage===2)throw sentinel;return "X"};
                try{String.prototype.replace.call(receiver,search,replacement)}
                catch(error){same=error===sentinel}
                return same+":"+log;
            }
            return [run(0),run(1),run(2)].join("|");
        })()"#,
    ),
];

const UTF16_AND_SUBSTITUTION_CASES: &[(&str, &str)] = &[
    (
        "string replacement expands every substitution token without captures",
        r#"(function(){
            var template="$$|$&|$`|$'|$0|$1|$01|$99|$<x>|$<";
            return [
                "abc".replace("b",template),
                "abc".replaceAll("b",template),
                "aaa".replace("a","X"),
                "aaa".replaceAll("a","X")
            ].join("|");
        })()"#,
    ),
    (
        "replaceAll empty search walks every UTF-16 code-unit boundary",
        r#"(function(){
            var input=String.fromCharCode(0xd83d,0xde00,0xd800),
                positions=[],matches=[],inputs=[],calls=0;
            var result=input.replaceAll("",function(match,position,source){
                calls++;positions[positions.length]=position;
                matches[matches.length]=match.length;
                inputs[inputs.length]=source===input;
                return "|";
            });
            return [__units(input),__units(result),calls,positions.join(","),
                matches.join(","),inputs.join(",")].join("|");
        })()"#,
    ),
    (
        "empty replacement tokens use code-unit positions including both string ends",
        r#"(function(){
            var input=String.fromCharCode(0xd83d,0xde00);
            return [
                __units(input.replaceAll("","[$`][$&][$']")),
                __units("".replaceAll("","X")),
                __units("".replace("","$`-$&-$'"))
            ].join("|");
        })()"#,
    ),
];

const RECURSION_CASES: &[(&str, &str)] = &[(
    "String replace protocol recursion overflows catchably and the runtime recovers",
    r#"(function(){
        function recurse(depth,all){
            var search=Object();
            if(all)search[Symbol.match]=false;
            search[Symbol.replace]=function(){
                if(depth!==0)return recurse(depth-1,!all);
                return "done";
            };
            return all?"x".replaceAll(search,"y"):"x".replace(search,"y");
        }
        var finite=recurse(8,false),completion;
        try{recurse(Infinity,false)}
        catch(error){completion=error.name+":"+error.message}
        return finite+"|"+completion+"|"+"abc".replace("b","X")+"|"+
            "aaa".replaceAll("a","X");
    })()"#,
)];

#[test]
fn string_replace_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String replace oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("metadata", METADATA_CASES),
        ("protocol", PROTOCOL_CASES),
        ("conversion", CONVERSION_CASES),
        ("UTF-16 and substitution", UTF16_AND_SUBSTITUTION_CASES),
        ("recursion", RECURSION_CASES),
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
fn string_replace_metadata_matches_pinned_quickjs() {
    compare_cases("String replace metadata", METADATA_CASES);
}

#[test]
fn string_replace_protocol_dispatch_matches_pinned_quickjs() {
    compare_cases("String replace protocol", PROTOCOL_CASES);
}

#[test]
fn string_replace_conversion_order_matches_pinned_quickjs() {
    compare_cases("String replace conversion", CONVERSION_CASES);
}

#[test]
fn string_replace_utf16_and_substitution_match_pinned_quickjs() {
    compare_cases(
        "String replace UTF-16 and substitution",
        UTF16_AND_SUBSTITUTION_CASES,
    );
}

#[test]
fn string_replace_intrinsics_use_their_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let Some(replace) = eval_optional_callable(
        &runtime,
        &mut defining,
        "String.prototype.replace",
        "defining String replace",
    ) else {
        eprintln!("SKIP String replace defining-realm lock: intrinsic is not published yet");
        return;
    };
    let replace_all = eval_optional_callable(
        &runtime,
        &mut defining,
        "String.prototype.replaceAll",
        "defining String replaceAll",
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

    let noncallable = eval_object(
        &mut caller,
        "(function(){var value=Object();value[Symbol.replace]=17;return value})()",
        "caller noncallable replace hook",
    );
    assert_eq!(
        caller.call(
            &replace,
            string_value("abc"),
            &[Value::Object(noncallable), string_value("X")],
        ),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "defining String replace TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error.clone()),
        "String replace native TypeError used the caller realm",
    );

    let throwing = eval_object(
        &mut caller,
        r#"(function(){
            var value=Object();
            Object.defineProperty(value,Symbol.replace,{get:function(){
                throw new TypeError("caller");
            }});
            return value;
        })()"#,
        "caller throwing replace hook",
    );
    assert_eq!(
        caller.call(
            &replace,
            string_value("abc"),
            &[Value::Object(throwing), string_value("X")],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller replace getter TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "String replace replaced a caller-realm user exception",
    );

    if let Some(replace_all) = replace_all {
        let foreign_non_global = eval_object(
            &mut caller,
            "/a/",
            "caller non-global RegExp for replaceAll",
        );
        assert_eq!(
            caller.call(
                &replace_all,
                string_value("a"),
                &[Value::Object(foreign_non_global), string_value("X")],
            ),
            Err(RuntimeError::Exception),
        );
        let global_error =
            take_exception_object(&mut caller, "defining String replaceAll global TypeError");
        assert_eq!(
            runtime.get_prototype_of(&global_error).unwrap(),
            Some(defining_type_error),
            "String replaceAll global validation used the caller realm",
        );
    }
}

#[test]
fn string_replace_recursion_matches_pinned_quickjs() {
    std::thread::Builder::new()
        .name("string-replace-oracle-stack".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| compare_cases("String replace recursion", RECURSION_CASES))
        .unwrap()
        .join()
        .unwrap();
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

fn eval_optional_callable(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> Option<CallableRef> {
    let value = context
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"));
    let Value::Object(object) = value else {
        return None;
    };
    runtime
        .as_callable(&object)
        .unwrap_or_else(|error| panic!("inspect {description}: {error}"))
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
