use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, JsString, ObjectRef, OrdinaryPropertyDescriptor,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_string_repeat` (quickjs.c 46144-46187),
// its String prototype table entry (46644), `JS_ToInt64SatFree` (13191-13234),
// `JS_ToStringCheckObject` (13670-13676), and the 30-bit String length cap
// (212). QuickJS converts the receiver first, converts and validates the count
// second, returns the converted source for an empty String or count one, and
// otherwise builds one flat String using exact UTF-16 code-unit repetition.
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
        "prototype own-key order places repeat after slice",
        r#"(function(){
            var selected=["length","includes","endsWith","startsWith",
                "substring","substr","slice","repeat","toString","valueOf","constructor"];
            var keys=Object.getOwnPropertyNames(String.prototype),output=[],index=0;
            while(index<keys.length){
                if(selected.indexOf(keys[index])>=0)output.push(keys[index]);
                index++;
            }
            return output.join(",");
        })()"#,
    ),
    (
        "descriptor name length and callable metadata are exact",
        r#"(function(){
            var fn=String.prototype.repeat;
            return [__bits(String.prototype,"repeat"),fn.name,fn.length,
                Object.getOwnPropertyNames(fn).join(","),__bits(fn,"length"),
                __bits(fn,"name"),Object.getPrototypeOf(fn)===Function.prototype,
                typeof fn,__isConstructor(fn)].join("|");
        })()"#,
    ),
    (
        "AutoInit materializes one stable repeat identity",
        r#"(function(){
            var first=String.prototype.repeat,again=String.prototype.repeat;
            var descriptor=Object.getOwnPropertyDescriptor(String.prototype,"repeat");
            return [first===again,first===descriptor.value,
                first===String.prototype.slice,first===String.prototype.substring].join("|");
        })()"#,
    ),
];

const AUTOINIT_CASES: &[(&str, &str)] = &[
    (
        "lazy repeat can be deleted before materialization",
        r#"(function(){
            var deleted=delete String.prototype.repeat;
            return [deleted,"repeat" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"repeat"),
                typeof String.prototype.repeat].join("|");
        })()"#,
    ),
    (
        "lazy repeat assignment becomes an ordinary replacement",
        r#"(function(){
            String.prototype.repeat=17;
            return [String.prototype.repeat,__bits(String.prototype,"repeat"),
                Object.prototype.hasOwnProperty.call(String.prototype,"repeat")].join("|");
        })()"#,
    ),
    (
        "materialized repeat remains deletable",
        r#"(function(){
            var fn=String.prototype.repeat,deleted=delete String.prototype.repeat;
            return [typeof fn,deleted,"repeat" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"repeat")].join("|");
        })()"#,
    ),
];

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "defaults primitives fractions and negative zero use saturated truncation",
        r#"(function(){return [
            "ab".repeat(),"ab".repeat(undefined),"ab".repeat(null),
            "ab".repeat(false),"ab".repeat(true),"ab".repeat(NaN),
            "ab".repeat(0),"ab".repeat(-0),"ab".repeat(-0.9),
            "ab".repeat(1),"ab".repeat(2.9),"ab".repeat("3.9")
        ].join("|")})()"#,
    ),
    (
        "generic primitive and object receivers are converted",
        r#"(function(){return [
            String.prototype.repeat.call(123,2),
            String.prototype.repeat.call(true,2),
            String.prototype.repeat.call(7n,3),
            String.prototype.repeat.call(Object(),2),
            String.prototype.repeat.call(new String("xy"),2)
        ].join("|")})()"#,
    ),
    (
        "arguments after count are completely ignored",
        r#"(function(){
            var extra=Object(),hits=0;
            extra[Symbol.toPrimitive]=function(){hits++;throw "extra"};
            var value=String.prototype.repeat.call("xy",2,extra,Symbol("later"),1n);
            return value+"|"+hits;
        })()"#,
    ),
    (
        "valid empty fast path retains the complete signed count boundary",
        r#"(function(){return [
            "".repeat(2147483647).length,
            "".repeat(2147483647.9).length,
            "".repeat(0).length,"".repeat(1).length
        ].join("|")})()"#,
    ),
    (
        "Int64 saturation and result length failures remain distinct",
        r#"(function(){return [
            __capture(function(){return "".repeat(2147483648).length}),
            __capture(function(){return "x".repeat(9223372036854775808).length}),
            __capture(function(){return "x".repeat(-9223372036854775808).length}),
            __capture(function(){return "x".repeat(Number.MAX_VALUE).length}),
            __capture(function(){return "x".repeat(-Number.MAX_VALUE).length}),
            __capture(function(){return "x".repeat(2147483647).length}),
            __capture(function(){return "ab".repeat(536870912).length}),
            __capture(function(){return "\ud83d\ude00".repeat(536870912).length})
        ].join("|")})()"#,
    ),
];

const UTF16_CASES: &[(&str, &str)] = &[
    (
        "repeat preserves astral and lone surrogate code units",
        r#"(function(){
            var source="A\ud83d\ude00\ud800B\udc00Z",result=source.repeat(2);
            return [source.length,result.length,__units(result)].join("|");
        })()"#,
    ),
    (
        "repeat linearizes ropes crossing the 8192-code-unit boundary",
        r#"(function(){
            function grow(character,power){
                var value=character,index=0;
                while(index<power){value=value+value;index++}
                return value;
            }
            var left=grow("a",13),right=grow("b",13);
            var source=(left+"\ud83d")+("\ude00"+right)+"\ud800Z";
            var result=source.repeat(2),join=source.length;
            return [source.length,result.length,
                __units(result.slice(8189,8197)),
                __units(result.slice(join-3,join+4)),
                __units(result.slice(-5)),result.charCodeAt(join),
                result.charCodeAt(join+8192)].join("|");
        })()"#,
    ),
    (
        "large repeated output retains exact boundaries without decoding",
        r#"(function(){
            var source="",index=0;
            while(index<9000){source+=(index%2)?"x":"y";index++}
            source="\ud800"+source+"\udc00";
            var result=source.repeat(3),chunk=source.length;
            return [result.length,__units(result.slice(0,3)),
                __units(result.slice(chunk-2,chunk+2)),
                __units(result.slice(chunk*2-2,chunk*2+2)),
                __units(result.slice(-3))].join("|");
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "receiver then count conversion uses string and number hints",
        r#"(function(){
            var log="",receiver=Object(),count=Object(),extra=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "ab"};
            count[Symbol.toPrimitive]=function(hint){log+="count:"+hint+";";return 2.9};
            extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";throw "extra"};
            return String.prototype.repeat.call(receiver,count,extra)+"|"+log;
        })()"#,
    ),
    (
        "ordinary ToPrimitive fallback orders differ by conversion hint",
        r#"(function(){
            var log="",receiver=Object(),count=Object();
            receiver.toString=function(){log+="receiver:toString;";return "xy"};
            receiver.valueOf=function(){log+="receiver:valueOf;";throw "wrong receiver order"};
            count.valueOf=function(){log+="count:valueOf;";return 2};
            count.toString=function(){log+="count:toString;";throw "wrong count order"};
            return String.prototype.repeat.call(receiver,count)+"|"+log;
        })()"#,
    ),
    (
        "receiver abrupt completion prevents count and extra conversion",
        r#"(function(){
            var log="",receiver=Object(),count=Object(),extra=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";throw "receiver"};
            count[Symbol.toPrimitive]=function(hint){log+="count:"+hint+";";return 2};
            extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";return 3};
            try{String.prototype.repeat.call(receiver,count,extra)}
            catch(error){return error+"|"+log}
            return "missing";
        })()"#,
    ),
    (
        "count abrupt completion occurs after receiver and ignores extra",
        r#"(function(){
            var log="",receiver=Object(),count=Object(),extra=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "ab"};
            count[Symbol.toPrimitive]=function(hint){log+="count:"+hint+";";throw "count"};
            extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";return 3};
            try{String.prototype.repeat.call(receiver,count,extra)}
            catch(error){return error+"|"+log}
            return "missing";
        })()"#,
    ),
    (
        "empty and count-one fast paths still perform count conversion",
        r#"(function(){
            var log="",zero=Object(),one=Object();
            zero[Symbol.toPrimitive]=function(hint){log+="zero:"+hint+";";return 2147483647};
            one[Symbol.toPrimitive]=function(hint){log+="one:"+hint+";";return 1};
            return ["".repeat(zero).length,"abc".repeat(one),log].join("|");
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "repeat rejects null receiver before count",
        "String.prototype.repeat.call(null,2147483648)",
    ),
    (
        "repeat rejects undefined receiver",
        "String.prototype.repeat.call(undefined,1)",
    ),
    (
        "repeat rejects Symbol receiver",
        "String.prototype.repeat.call(Symbol('receiver'),1)",
    ),
    (
        "repeat rejects Symbol count",
        "'abc'.repeat(Symbol('count'))",
    ),
    ("repeat rejects BigInt count", "'abc'.repeat(1n)"),
    (
        "count ToPrimitive object result is rejected",
        r#"(function(){
            var count=Object();count[Symbol.toPrimitive]=function(){return Object()};
            return "abc".repeat(count);
        })()"#,
    ),
    ("negative count has exact RangeError", "'abc'.repeat(-1)"),
    (
        "negative infinity has exact RangeError",
        "'abc'.repeat(-Infinity)",
    ),
    (
        "positive infinity has exact RangeError",
        "'abc'.repeat(Infinity)",
    ),
    (
        "count above Int32 max has exact RangeError",
        "''.repeat(2147483648)",
    ),
    (
        "result above String cap has distinct RangeError",
        "'ab'.repeat(536870912)",
    ),
    (
        "repeat is not a constructor",
        "new String.prototype.repeat()",
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "recursive receiver conversion throws catchably and runtime recovers",
        r#"(function(){
            var receiver=Object(),errorName="",errorMessage="";
            receiver[Symbol.toPrimitive]=function(){
                return String.prototype.repeat.call(receiver,1)
            };
            try{String.prototype.repeat.call(receiver,1)}
            catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage,"ab".repeat(2),"abcdef".slice(1,4)].join("|");
        })()"#,
    ),
    (
        "recursive count conversion is catchable and leaves later calls healthy",
        r#"(function(){
            var count=Object(),errorName="";
            count[Symbol.toPrimitive]=function(){return "x".repeat(count)};
            try{"x".repeat(count)}catch(error){errorName=error.name}
            return [errorName,"xy".repeat(2),"abcdef".includes("cd")].join("|");
        })()"#,
    ),
    (
        "repeat includes and subrange recursion share one guarded family",
        r#"(function(){
            var value=Object(),depth=0,errorName="",errorMessage="";
            value[Symbol.toPrimitive]=function(){
                depth++;
                if(depth%3===0)return "x".repeat(value);
                if(depth%3===1)return "abcdef".slice(value,4);
                return "abcdef".includes("a",value);
            };
            try{"x".repeat(value)}
            catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage,"ok".repeat(2),
                "abcdef".slice(1,3),"abcdef".includes("bc")].join("|");
        })()"#,
    ),
];

#[test]
fn string_repeat_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String repeat oracle self-check: set QJS_ORACLE to upstream qjs");
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
fn string_repeat_graph_and_autoinit_match_pinned_quickjs() {
    compare_cases("String repeat graph", GRAPH_CASES);
    compare_cases("String repeat AutoInit", AUTOINIT_CASES);
}

#[test]
fn string_repeat_values_and_int64_saturation_match_pinned_quickjs() {
    compare_cases("String repeat values", VALUE_CASES);
}

#[test]
fn string_repeat_utf16_and_large_ropes_match_pinned_quickjs() {
    compare_cases("String repeat UTF-16", UTF16_CASES);
}

#[test]
fn string_repeat_conversion_order_and_abrupt_completion_match_pinned_quickjs() {
    compare_cases("String repeat conversion order", ORDER_CASES);
}

#[test]
fn string_repeat_errors_and_nonconstructor_match_pinned_quickjs() {
    compare_cases("String repeat errors", ERROR_CASES);
}

#[test]
fn string_repeat_recursion_is_catchable_and_shared_family_recovers() {
    compare_cases("String repeat stack recovery", STACK_CASES);
}

#[test]
fn string_repeat_defining_realms_and_user_throw_identity_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let repeat = property_callable(&runtime, &mut defining, &defining_prototype, "repeat");
    assert_eq!(
        runtime.get_prototype_of(repeat.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap()),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let defining_range_error = intrinsic_prototype(&runtime, &mut defining, "RangeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_native_error(
        &runtime,
        &mut caller,
        &repeat,
        Value::Null,
        &[Value::Int(1)],
        &defining_type_error,
    );
    let count_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("count").unwrap()))
        .unwrap();
    assert_native_error(
        &runtime,
        &mut caller,
        &repeat,
        Value::String(JsString::try_from_utf8("abc").unwrap()),
        &[Value::Symbol(count_symbol)],
        &defining_type_error,
    );
    assert_native_error(
        &runtime,
        &mut caller,
        &repeat,
        Value::String(JsString::try_from_utf8("abc").unwrap()),
        &[Value::Int(-1)],
        &defining_range_error,
    );

    let sentinel = caller.new_object().unwrap();
    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "repeatSentinel",
        Value::Object(sentinel.clone()),
    );
    let throwing_count = caller
        .eval(
            r#"(function(){
                var value=Object();
                value[Symbol.toPrimitive]=function(){throw repeatSentinel};
                return value;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        caller.call(
            &repeat,
            Value::String(JsString::try_from_utf8("abc").unwrap()),
            &[throwing_count],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "count conversion did not preserve the user-thrown value",
    );

    assert_eq!(caller.construct(&repeat, &[]), Err(RuntimeError::Exception));
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut caller))
            .unwrap(),
        Some(caller_type_error),
        "non-constructor rejection did not use the caller realm",
    );
}

#[test]
fn string_repeat_callables_are_per_realm_distinct_and_collectable() {
    let runtime = Runtime::new();
    let retained = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_prototype = first.string_prototype().unwrap();
        let second_prototype = second.string_prototype().unwrap();
        let first_repeat = property_callable(&runtime, &mut first, &first_prototype, "repeat");
        let first_repeat_again =
            property_callable(&runtime, &mut first, &first_prototype, "repeat");
        let first_slice = property_callable(&runtime, &mut first, &first_prototype, "slice");
        let second_repeat = property_callable(&runtime, &mut second, &second_prototype, "repeat");
        assert_eq!(first_repeat, first_repeat_again);
        assert_ne!(first_repeat, first_slice);
        assert_ne!(first_repeat, second_repeat);
        assert_eq!(
            runtime.get_prototype_of(first_repeat.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        first_repeat
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(retained);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn string_repeat_stack_overflow_uses_the_caller_realm_and_recovers() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let repeat = property_callable(&runtime, &mut defining, &defining_prototype, "repeat");
    let defining_internal_error = intrinsic_prototype(&runtime, &mut defining, "InternalError");
    let caller_internal_error = intrinsic_prototype(&runtime, &mut caller, "InternalError");
    assert_ne!(defining_internal_error, caller_internal_error);

    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "foreignRepeat",
        Value::Object(repeat.as_object().clone()),
    );
    let Value::Object(error) = caller
        .eval(
            r#"(function(){
                var count=Object(),localCall=Function.prototype.call;
                function invoke(){return localCall.call(foreignRepeat,"x",count)}
                count[Symbol.toPrimitive]=function(){return invoke()};
                try{invoke()}catch(error){return error}
                return Object();
            })()"#,
        )
        .unwrap()
    else {
        panic!("recursive cross-realm repeat did not return an error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_internal_error),
        "pre-dispatch stack overflow did not use the caller realm",
    );
    assert_eq!(
        caller
            .call(
                &repeat,
                Value::String(JsString::try_from_utf8("recovered").unwrap()),
                &[Value::Int(2)],
            )
            .unwrap(),
        Value::String(JsString::try_from_utf8("recoveredrecovered").unwrap()),
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
