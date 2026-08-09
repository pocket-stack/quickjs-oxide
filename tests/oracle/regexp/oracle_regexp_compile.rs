use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04
// `js_regexp_compile` (`quickjs.c` 47584-47626). This Annex B method is
// deliberately tested against the pinned implementation rather than a later
// specification interpretation: QuickJS accepts every genuine RegExp class
// instance, including derived and cross-realm objects.

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

const METADATA_CASES: &[(&str, &str)] = &[
    (
        "RegExp compile exposes pinned metadata descriptor and source-table order",
        r#"(function(){
            var fn=RegExp.prototype.compile,
                keys=Reflect.ownKeys(RegExp.prototype),
                selected=[],index=0,key;
            while(index<keys.length){
                key=keys[index++];
                if(key==="exec"||key==="compile"||key==="test"||key==="toString"||
                    key==="constructor"||key===Symbol.match||key===Symbol.search||
                    key===Symbol.split)
                    selected[selected.length]=String(key);
            }
            return [
                selected.join(","),
                __bits(RegExp.prototype,"compile"),fn.name,fn.length,
                Object.getOwnPropertyNames(fn).join(","),
                __bits(fn,"name"),__bits(fn,"length"),
                __isConstructor(fn),
                Object.prototype.hasOwnProperty.call(fn,"prototype")
            ].join("|");
        })()"#,
    ),
    (
        "RegExp compile AutoInit identity is stable and independently replaceable",
        r#"(function(){
            var first=RegExp.prototype.compile,
                second=RegExp.prototype.compile,
                stable=first===second,
                deleted=delete RegExp.prototype.compile;
            RegExp.prototype.compile=17;
            return [stable,deleted,RegExp.prototype.compile,
                __bits(RegExp.prototype,"compile"),
                typeof first,first.name,first.length].join("|");
        })()"#,
    ),
];

const BRAND_AND_COPY_CASES: &[(&str, &str)] = &[
    (
        "RegExp compile validates the receiver before touching the arguments",
        r#"(function(){
            var log="",pattern=Object(),compile=RegExp.prototype.compile;
            pattern.toString=function(){log+="BAD";throw "converted"};
            var receivers=[undefined,null,23,true,"text",Object(),[]],output=[],index=0;
            while(index<receivers.length){
                output[output.length]=__completion(function(receiver){
                    return function(){return compile.call(receiver,pattern)};
                }(receivers[index++]));
            }
            return output.join(",")+"|"+log;
        })()"#,
    ),
    (
        "RegExp compile copies genuine same and distinct patterns without public property reads",
        r#"(function(){
            var log="",subject=/old/g,pattern=/new/imy;
            subject.lastIndex=23;pattern.lastIndex=45;
            Object.defineProperty(pattern,"source",{get:function(){log+="BAD-source;";throw 1}});
            Object.defineProperty(pattern,"flags",{get:function(){log+="BAD-flags;";throw 2}});
            Object.defineProperty(pattern,Symbol.match,{get:function(){
                log+="BAD-match;";throw 3;
            }});
            var result=subject.compile(pattern),
                distinct=[result===subject,subject.source,subject.flags,
                    subject.lastIndex,pattern.lastIndex,log].join(":");
            subject.lastIndex=17;
            result=subject.compile(subject);
            return distinct+"|"+
                [result===subject,subject.source,subject.flags,subject.lastIndex,log].join(":");
        })()"#,
    ),
    (
        "RegExp compile rejects defined flags for genuine patterns before coercion",
        r#"(function(){
            var log="",subject=/old/g,pattern=/new/i,flags=Object();
            subject.lastIndex=29;
            flags.toString=function(){log+="BAD-flags;";throw "converted"};
            var result=__completion(function(){return subject.compile(pattern,flags)});
            return [result,log,subject.source,subject.flags,subject.lastIndex,
                pattern.source,pattern.flags].join("|");
        })()"#,
    ),
    (
        "RegExp compile accepts a genuine derived instance like pinned QuickJS",
        r#"(function(){
            function Derived(){}
            Derived.prototype=Object.create(RegExp.prototype);
            Object.defineProperty(Derived.prototype,"constructor",{
                value:Derived,writable:true,configurable:true
            });
            var regexp=Reflect.construct(RegExp,["before","g"],Derived);
            regexp.lastIndex=7;
            var result=regexp.compile("after","i");
            return [result===regexp,regexp instanceof Derived,
                Object.getPrototypeOf(regexp)===Derived.prototype,
                regexp.source,regexp.flags,regexp.lastIndex,regexp.test("AFTER")].join("|");
        })()"#,
    ),
];

const CONVERSION_CASES: &[(&str, &str)] = &[
    (
        "RegExp compile converts pattern before flags then replaces matcher state",
        r#"(function(){
            var log="",subject=/old/g,pattern=Object(),flags=Object();
            subject.lastIndex=31;
            pattern.toString=function(){log+="pattern;";return "n(e)w"};
            flags.toString=function(){log+="flags;";return "im"};
            var result=subject.compile(pattern,flags);
            return [result===subject,log,subject.source,subject.flags,subject.lastIndex,
                subject.test("NEW"),subject.exec("new")[1]].join("|");
        })()"#,
    ),
    (
        "RegExp compile maps undefined pattern and flags to empty strings",
        r#"(function(){
            var implicit=/old/g,explicit=/old/i,a,b;
            implicit.lastIndex=9;explicit.lastIndex=11;
            a=implicit.compile();
            b=explicit.compile(undefined,undefined);
            return [a===implicit,implicit.source,implicit.flags,implicit.lastIndex,
                implicit.test(""),b===explicit,explicit.source,explicit.flags,
                explicit.lastIndex,explicit.test("")].join("|");
        })()"#,
    ),
    (
        "RegExp compile preserves canonical flags and exact source spelling",
        r#"(function(){
            var regexp=/old/;
            regexp.compile("a/b\nc\r","ydgimsu");
            return [regexp.source,regexp.flags,regexp.hasIndices,regexp.global,
                regexp.ignoreCase,regexp.multiline,regexp.dotAll,regexp.unicode,
                regexp.sticky].join("|");
        })()"#,
    ),
];

const FAILURE_ATOMICITY_CASES: &[(&str, &str)] = &[
    (
        "RegExp compile conversion failures leave source matcher and lastIndex unchanged",
        r#"(function(){
            var log="",subject=/stable/gi,pattern=Object(),flags=Object(),first,second;
            subject.lastIndex=19;
            pattern.toString=function(){log+="pattern-throw;";throw "pattern-boom"};
            flags.toString=function(){log+="BAD-flags;";return "m"};
            first=__completion(function(){return subject.compile(pattern,flags)});
            var afterPattern=[subject.source,subject.flags,subject.lastIndex,
                subject.test("STABLE")].join(":");
            subject.lastIndex=19;
            pattern.toString=function(){log+="pattern-ok;";return "changed"};
            flags.toString=function(){log+="flags-throw;";throw "flags-boom"};
            second=__completion(function(){return subject.compile(pattern,flags)});
            return [first,afterPattern,second,subject.source,subject.flags,
                subject.lastIndex,subject.test("STABLE"),log].join("|");
        })()"#,
    ),
    (
        "RegExp compile syntax and flag failures are atomic",
        r#"(function(){
            var subject=/stable/gi,output=[];
            function attempt(pattern,flags){
                subject.lastIndex=27;
                output[output.length]=__completion(function(){
                    return subject.compile(pattern,flags);
                });
                output[output.length]=
                    [subject.source,subject.flags,subject.lastIndex,
                        subject.test("STABLE")].join(":");
            }
            attempt("?","");
            attempt("changed","gg");
            attempt("changed","uv");
            attempt("\\2","u");
            return output.join("|");
        })()"#,
    ),
    (
        "RegExp compile rejects Symbol conversion without mutating the target",
        r#"(function(){
            var subject=/stable/g,output=[];
            subject.lastIndex=13;
            output[output.length]=__completion(function(){
                return subject.compile(Symbol("pattern"));
            });
            output[output.length]=
                [subject.source,subject.flags,subject.lastIndex].join(":");
            output[output.length]=__completion(function(){
                return subject.compile("changed",Symbol("flags"));
            });
            output[output.length]=
                [subject.source,subject.flags,subject.lastIndex].join(":");
            return output.join("|");
        })()"#,
    ),
    (
        "RegExp compile reentrant conversions retain inner effects until an outer commit",
        r#"(function(){
            var success=/start/g,patternFailure=/start/g,flagsFailure=/start/g,log="";
            success.lastIndex=5;
            var result=success.compile({toString:function(){
                success.compile("inner","i");log+="success-inner;";return "outer";
            }},"m");
            patternFailure.lastIndex=7;
            var patternCompletion=__completion(function(){
                return patternFailure.compile({toString:function(){
                    patternFailure.compile("inner","i");
                    log+="pattern-inner;";return "[";
                }});
            });
            flagsFailure.lastIndex=9;
            var flags=Object();
            flags.toString=function(){
                flagsFailure.compile("flag-inner","y");
                log+="flags-inner;";throw "flags-boom";
            };
            var flagsCompletion=__completion(function(){
                return flagsFailure.compile("outer",flags);
            });
            return [result===success,success.source,success.flags,success.lastIndex,
                patternCompletion,patternFailure.source,patternFailure.flags,
                patternFailure.lastIndex,flagsCompletion,flagsFailure.source,
                flagsFailure.flags,flagsFailure.lastIndex,log].join("|");
        })()"#,
    ),
];

const MUTATION_ORDER_CASES: &[(&str, &str)] = &[
    (
        "RegExp compile replaces internal state before readonly lastIndex throws",
        r#"(function(){
            var subject=/initial/;
            Object.defineProperty(subject,"lastIndex",{value:45,writable:false});
            var completion=__completion(function(){return subject.compile(/updated/gi)});
            return [completion,subject.source,subject.flags,subject.lastIndex,
                new RegExp(subject).test("UPDATED")].join("|");
        })()"#,
    ),
    (
        "RegExp compile resets only the receiver lastIndex and returns that receiver",
        r#"(function(){
            var subject=/subject/g,pattern=/pattern/y;
            subject.lastIndex=8;pattern.lastIndex=6;
            var result=RegExp.prototype.compile.call(subject,pattern);
            return [result===subject,result===pattern,subject.source,subject.flags,
                subject.lastIndex,pattern.source,pattern.flags,pattern.lastIndex].join("|");
        })()"#,
    ),
];

const RECURSION_CASES: &[(&str, &str)] = &[(
    "RegExp compile conversion recursion overflows catchably and the runtime recovers",
    r#"(function(){
        function recurse(depth){
            var regexp=/a/;
            if(depth===0)return "done";
            regexp.compile({toString:function(){recurse(depth-1);return "a"}});
            return "done";
        }
        var finite=recurse(8),completion;
        try{recurse(Infinity)}
        catch(error){completion=error.name+":"+error.message}
        return finite+"|"+completion+"|"+(/a/.compile("b").test("b"));
    })()"#,
)];

#[test]
fn regexp_compile_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp compile oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("metadata", METADATA_CASES),
        ("brand and copy", BRAND_AND_COPY_CASES),
        ("conversion", CONVERSION_CASES),
        ("failure atomicity", FAILURE_ATOMICITY_CASES),
        ("mutation order", MUTATION_ORDER_CASES),
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
fn regexp_compile_metadata_and_brand_copy_match_pinned_quickjs() {
    compare_cases("RegExp compile metadata", METADATA_CASES);
    compare_cases("RegExp compile brand and copy", BRAND_AND_COPY_CASES);
}

#[test]
fn regexp_compile_conversion_and_failure_atomicity_match_pinned_quickjs() {
    compare_cases("RegExp compile conversion", CONVERSION_CASES);
    compare_cases("RegExp compile failure atomicity", FAILURE_ATOMICITY_CASES);
}

#[test]
fn regexp_compile_mutation_order_matches_pinned_quickjs() {
    compare_cases("RegExp compile mutation order", MUTATION_ORDER_CASES);
}

#[test]
fn regexp_compile_intrinsic_uses_defining_realm_and_accepts_foreign_brands() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let compile = eval_callable(
        &runtime,
        &mut defining,
        "RegExp.prototype.compile",
        "defining RegExp compile",
    );
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let defining_syntax_error = eval_object(
        &mut defining,
        "SyntaxError.prototype",
        "defining SyntaxError prototype",
    );
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );
    assert_ne!(defining_type_error, caller_type_error);

    let foreign_regexp = eval_object(
        &mut caller,
        "(function(){var r=/old/g;r.lastIndex=7;return r})()",
        "caller RegExp",
    );
    let result = caller
        .call(
            &compile,
            Value::Object(foreign_regexp.clone()),
            &[string_value("new")],
        )
        .expect("defining-realm compile on a caller RegExp");
    assert_eq!(
        result,
        Value::Object(foreign_regexp.clone()),
        "RegExp compile did not return the foreign receiver",
    );
    assert_eq!(
        string_property(&runtime, &mut caller, &foreign_regexp, "source"),
        "new",
    );
    assert_eq!(
        number_property(&runtime, &mut caller, &foreign_regexp, "lastIndex"),
        0.0,
    );

    let ordinary = eval_object(&mut caller, "({})", "caller ordinary object");
    assert_eq!(
        caller.call(&compile, Value::Object(ordinary), &[]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "defining-realm compile TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "native RegExp compile TypeError used the caller realm",
    );

    let stable = eval_object(
        &mut caller,
        "(function(){var r=/stable/g;r.lastIndex=9;return r})()",
        "caller stable RegExp",
    );
    let throwing_pattern = eval_object(
        &mut caller,
        r#"({toString:function(){throw new TypeError("caller pattern")}})"#,
        "caller throwing pattern",
    );
    assert_eq!(
        caller.call(
            &compile,
            Value::Object(stable.clone()),
            &[Value::Object(throwing_pattern)],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller pattern TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "RegExp compile replaced a caller-realm conversion exception",
    );
    assert_eq!(
        string_property(&runtime, &mut caller, &stable, "source"),
        "stable",
    );
    assert_eq!(
        number_property(&runtime, &mut caller, &stable, "lastIndex"),
        9.0,
    );

    assert_eq!(
        caller.call(
            &compile,
            Value::Object(stable.clone()),
            &[string_value("[")],
        ),
        Err(RuntimeError::Exception),
    );
    let syntax_error = take_exception_object(&mut caller, "defining-realm compile SyntaxError");
    assert_eq!(
        runtime.get_prototype_of(&syntax_error).unwrap(),
        Some(defining_syntax_error),
        "RegExp compile SyntaxError used the caller realm",
    );
    assert_eq!(
        string_property(&runtime, &mut caller, &stable, "source"),
        "stable",
        "failed compilation mutated the foreign RegExp",
    );
}

#[test]
fn regexp_compile_recursion_is_catchable_and_recovers_like_pinned_quickjs() {
    std::thread::Builder::new()
        .name("regexp-compile-stack-proof".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| compare_cases("RegExp compile recursion", RECURSION_CASES))
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

fn number_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> f64 {
    let key = runtime.intern_property_key(name).unwrap();
    context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read number property {name}: {error}"))
        .as_number()
        .unwrap_or_else(|| panic!("{name} was not a number"))
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
