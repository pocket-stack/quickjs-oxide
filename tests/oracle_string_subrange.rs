use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, JsString, ObjectRef, OrdinaryPropertyDescriptor,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_sub_string` (quickjs.c 3964-3991),
// `js_string_substring`/`js_string_substr`/`js_string_slice` (45983-46071),
// their adjacent function-list entries (46641-46643), and the shared
// saturated conversion helpers `JS_ToInt32SatFree`/`JS_ToInt32Clamp`
// (13124-13189).
//
// These vectors intentionally preserve QuickJS's implementation contract:
// substring clamps both indices and swaps them, substr applies a string-length
// offset to a negative start and treats its second argument as a length, while
// slice applies that offset independently to both indices and never swaps.
// All string observations containing unpaired surrogates are encoded as raw
// UTF-16 code units before crossing the process stdout boundary.

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
"#;

const GRAPH_CASES: &[(&str, &str)] = &[
    (
        "prototype own-key order reaches the subrange family",
        r#"(function(){
            var selected=["length","includes","endsWith","startsWith",
                "substring","substr","slice","toString","valueOf","constructor"];
            var keys=Object.getOwnPropertyNames(String.prototype),output=[],index=0;
            while(index<keys.length){
                if(selected.indexOf(keys[index])>=0)output.push(keys[index]);
                index++;
            }
            return output.join(",");
        })()"#,
    ),
    (
        "descriptors names lengths and callable metadata are exact",
        r#"(function(){
            var names=["substring","substr","slice"],output=[],index=0;
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
        "AutoInit yields stable and mutually distinct callables",
        r#"(function(){
            var substring=String.prototype.substring;
            var substr=String.prototype.substr;
            var slice=String.prototype.slice;
            return [(substring===String.prototype.substring),
                (substr===String.prototype.substr),(slice===String.prototype.slice),
                (substring===Object.getOwnPropertyDescriptor(String.prototype,"substring").value),
                (substring===substr),(substring===slice),(substr===slice)].join("|");
        })()"#,
    ),
];

const AUTOINIT_CASES: &[(&str, &str)] = &[
    (
        "lazy substring can be deleted before materialization",
        r#"(function(){
            var deleted=delete String.prototype.substring;
            return [deleted,"substring" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"substring"),
                typeof String.prototype.substring].join("|");
        })()"#,
    ),
    (
        "lazy substr assignment becomes an ordinary replacement",
        r#"(function(){
            String.prototype.substr=17;
            return [String.prototype.substr,__bits(String.prototype,"substr"),
                Object.prototype.hasOwnProperty.call(String.prototype,"substr")].join("|");
        })()"#,
    ),
    (
        "materialized slice remains deletable",
        r#"(function(){
            var fn=String.prototype.slice,deleted=delete String.prototype.slice;
            return [typeof fn,deleted,"slice" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"slice")].join("|");
        })()"#,
    ),
];

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "substring defaults clamps and swaps its endpoints",
        r#"(function(){return [
            "012345".substring(),"012345".substring(undefined),
            "012345".substring(2),"012345".substring(2,undefined),
            "012345".substring(2,4),"012345".substring(4,2),
            "012345".substring(-2,3),"012345".substring(2,-3),
            "012345".substring(99,3),"012345".substring(3,99),
            "012345".substring(6,6),"012345".substring(9,9)
        ].join("|")})()"#,
    ),
    (
        "substr defaults negative starts and length clamping",
        r#"(function(){return [
            "012345".substr(),"012345".substr(undefined),
            "012345".substr(2),"012345".substr(2,undefined),
            "012345".substr(2,3),"012345".substr(-2),
            "012345".substr(-4,2),"012345".substr(-99,2),
            "012345".substr(2,-1),"012345".substr(2,0),
            "012345".substr(2,99),"012345".substr(99,2)
        ].join("|")})()"#,
    ),
    (
        "slice defaults negative indices and does not swap",
        r#"(function(){return [
            "012345".slice(),"012345".slice(undefined),
            "012345".slice(2),"012345".slice(2,undefined),
            "012345".slice(2,4),"012345".slice(4,2),
            "012345".slice(-2),"012345".slice(-4,-1),
            "012345".slice(-99,3),"012345".slice(2,-99),
            "012345".slice(99,3),"012345".slice(3,99)
        ].join("|")})()"#,
    ),
    (
        "fractions NaN infinities and negative zero use saturated truncation",
        r#"(function(){return [
            "012345".substring(1.9,4.9),"012345".substring(-1.9,3.9),
            "012345".substring(NaN,3),"012345".substring(Infinity,2),
            "012345".substring(-Infinity,2),"012345".substring(-0,2),
            "012345".substr(1.9,3.9),"012345".substr(-1.9,1.9),
            "012345".substr(NaN,2),"012345".substr(Infinity,2),
            "012345".substr(-Infinity,2),"012345".substr(1,Infinity),
            "012345".substr(1,-Infinity),
            "012345".slice(1.9,4.9),"012345".slice(-1.9,-0),
            "012345".slice(NaN,2),"012345".slice(Infinity,2),
            "012345".slice(-Infinity,2),"012345".slice(1,Infinity),
            "012345".slice(1,-Infinity)
        ].join("|")})()"#,
    ),
    (
        "signed 32-bit saturation boundaries clamp before subranges",
        r#"(function(){
            var points=[2147483647,2147483648,4294967297,
                -2147483648,-2147483649,-4294967297];
            var names=["substring","substr","slice"],output=[],i=0,j=0;
            while(i<names.length){
                j=0;
                while(j<points.length){
                    output.push(String.prototype[names[i]].call("012345",points[j],3));
                    output.push(String.prototype[names[i]].call("012345",1,points[j]));
                    j++;
                }
                i++;
            }
            return output.join("|");
        })()"#,
    ),
    (
        "null booleans strings and generic primitive receivers are converted",
        r#"(function(){return [
            "012345".substring(null,true),"012345".substr(false,"2.9"),
            "012345".slice("1.9","4.9"),
            String.prototype.substring.call(12345,1,4),
            String.prototype.substr.call(true,1,2),
            String.prototype.slice.call(777n,1,2),
            String.prototype.substring.call(Object(),1,7)
        ].join("|")})()"#,
    ),
    (
        "arguments after end are completely ignored",
        r#"(function(){return [
            "abcdef".substring(1,4,Symbol("extra"),1n),
            "abcdef".substr(1,3,Symbol("extra"),1n),
            "abcdef".slice(1,4,Symbol("extra"),1n)
        ].join("|")})()"#,
    ),
];

const UTF16_CASES: &[(&str, &str)] = &[
    (
        "subranges preserve lone and split surrogate code units",
        r#"(function(){
            var source="A\ud83d\ude00\ud800B\udc00Z";
            return [source.length,
                __units(source.substring(1,2)),__units(source.substring(2,4)),
                __units(source.substring(5,3)),
                __units(source.substr(1,1)),__units(source.substr(2,3)),
                __units(source.substr(-3,2)),
                __units(source.slice(1,2)),__units(source.slice(2,5)),
                __units(source.slice(-3,-1))].join("|");
        })()"#,
    ),
    (
        "subranges cross rope leaves and the 8192-code-unit boundary",
        r#"(function(){
            function grow(character,power){
                var value=character,index=0;
                while(index<power){value=value+value;index++}
                return value;
            }
            var left=grow("a",13),right=grow("b",13);
            var source=(left+"\ud83d")+("\ude00"+right)+"\ud800Z";
            var middle=source.substring(8191,8196);
            var suffix=source.substr(-4,4);
            var spanning=source.slice(8192,8192+4);
            return [source.length,middle.length,__units(middle),
                suffix.length,__units(suffix),spanning.length,__units(spanning),
                source.substring(1,source.length-1).length,
                source.substr(1,source.length-2).length,
                source.slice(1,-1).length].join("|");
        })()"#,
    ),
    (
        "large subranges retain exact first and last raw units",
        r#"(function(){
            var source="",index=0;
            while(index<9000){source+=(index%2)?"x":"y";index++}
            source="\ud800"+source+"\udc00";
            var a=source.substring(0,9001);
            var b=source.substr(1,9000);
            var c=source.slice(-9001);
            return [a.length,__units(a.substring(0,2)),__units(a.slice(-2)),
                b.length,__units(b.substring(0,2)),__units(b.slice(-2)),
                c.length,__units(c.substring(0,2)),__units(c.slice(-2))].join("|");
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "receiver start then end conversion order uses string number number hints",
        r#"(function(){
            function run(name){
                var log="",receiver=Object(),start=Object(),end=Object(),extra=Object();
                receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "abcdef"};
                start[Symbol.toPrimitive]=function(hint){log+="start:"+hint+";";return 1.9};
                end[Symbol.toPrimitive]=function(hint){log+="end:"+hint+";";return 4.9};
                extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";throw "extra"};
                return String.prototype[name].call(receiver,start,end,extra)+":"+log;
            }
            return [run("substring"),run("substr"),run("slice")].join("|");
        })()"#,
    ),
    (
        "receiver abrupt completion prevents every argument conversion",
        r#"(function(){
            function run(name){
                var log="",receiver=Object(),start=Object(),end=Object();
                receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";throw "receiver"};
                start[Symbol.toPrimitive]=function(hint){log+="start:"+hint+";";return 1};
                end[Symbol.toPrimitive]=function(hint){log+="end:"+hint+";";return 2};
                try{String.prototype[name].call(receiver,start,end)}
                catch(error){return error+":"+log}
                return "missing";
            }
            return [run("substring"),run("substr"),run("slice")].join("|");
        })()"#,
    ),
    (
        "start abrupt completion prevents end and extra conversion",
        r#"(function(){
            function run(name){
                var log="",start=Object(),end=Object(),extra=Object();
                start[Symbol.toPrimitive]=function(hint){log+="start:"+hint+";";throw "start"};
                end[Symbol.toPrimitive]=function(hint){log+="end:"+hint+";";return 2};
                extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";return 3};
                try{String.prototype[name].call("abcdef",start,end,extra)}
                catch(error){return error+":"+log}
                return "missing";
            }
            return [run("substring"),run("substr"),run("slice")].join("|");
        })()"#,
    ),
    (
        "end abrupt completion occurs after start and still ignores extra",
        r#"(function(){
            function run(name){
                var log="",start=Object(),end=Object(),extra=Object();
                start[Symbol.toPrimitive]=function(hint){log+="start:"+hint+";";return 1};
                end[Symbol.toPrimitive]=function(hint){log+="end:"+hint+";";throw "end"};
                extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";return 3};
                try{String.prototype[name].call("abcdef",start,end,extra)}
                catch(error){return error+":"+log}
                return "missing";
            }
            return [run("substring"),run("substr"),run("slice")].join("|");
        })()"#,
    ),
    (
        "undefined end skips conversion and later arguments remain unobserved",
        r#"(function(){
            function run(name){
                var log="",start=Object(),extra=Object();
                start[Symbol.toPrimitive]=function(hint){log+="start:"+hint+";";return 2};
                extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";throw "extra"};
                return String.prototype[name].call("abcdef",start,undefined,extra)+":"+log;
            }
            return [run("substring"),run("substr"),run("slice")].join("|");
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "substring rejects null receiver",
        "String.prototype.substring.call(null,0,1)",
    ),
    (
        "substr rejects undefined receiver",
        "String.prototype.substr.call(undefined,0,1)",
    ),
    (
        "slice rejects Symbol receiver",
        "String.prototype.slice.call(Symbol('receiver'),0,1)",
    ),
    (
        "substring rejects Symbol start",
        "'abc'.substring(Symbol('start'),2)",
    ),
    ("substr rejects BigInt start", "'abc'.substr(1n,2)"),
    ("slice rejects Symbol end", "'abc'.slice(0,Symbol('end'))"),
    ("substring rejects BigInt end", "'abc'.substring(0,1n)"),
    (
        "start ToPrimitive object result is rejected",
        r#"(function(){
            var start=Object();start[Symbol.toPrimitive]=function(){return Object()};
            return "abc".substr(start,1);
        })()"#,
    ),
    (
        "end ToPrimitive object result is rejected",
        r#"(function(){
            var end=Object();end[Symbol.toPrimitive]=function(){return Object()};
            return "abc".slice(0,end);
        })()"#,
    ),
    (
        "substring is not a constructor",
        "new String.prototype.substring()",
    ),
    (
        "substr is not a constructor",
        "new String.prototype.substr()",
    ),
    ("slice is not a constructor", "new String.prototype.slice()"),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "recursive start conversion throws catchably and runtime recovers",
        r#"(function(){
            var start=Object(),errorName="",errorMessage="";
            start[Symbol.toPrimitive]=function(){return "abcdef".substring(start,4)};
            try{"abcdef".substring(start,4)}
            catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage,"abcdef".substring(1,4),
                "abcdef".substr(-3,2),"abcdef".slice(2,-1)].join("|");
        })()"#,
    ),
    (
        "interleaved subrange recursion cannot bypass the family guard",
        r#"(function(){
            var position=Object(),depth=0,errorName="";
            position[Symbol.toPrimitive]=function(){
                depth++;
                if(depth%3===0)return "abcdef".substring(position,4);
                if(depth%3===1)return "abcdef".substr(position,2);
                return "abcdef".slice(position,4);
            };
            try{"abcdef".slice(position,4)}catch(error){errorName=error.name}
            return [errorName,"recovered".substring(1,5),
                "recovered".substr(-3,2),"recovered".slice(2,-1)].join("|");
        })()"#,
    ),
    (
        "recursive end conversion is catchable and leaves later calls healthy",
        r#"(function(){
            var end=Object(),errorName="";
            end[Symbol.toPrimitive]=function(){return "abcdef".substr(0,end)};
            try{"abcdef".substr(0,end)}catch(error){errorName=error.name}
            return [errorName,"abcdef".substring(0,2),"abcdef".slice(-2)].join("|");
        })()"#,
    ),
];

#[test]
fn string_subrange_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String subrange oracle self-check: set QJS_ORACLE to upstream qjs");
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
fn string_subrange_graph_and_autoinit_match_pinned_quickjs() {
    compare_cases("String subrange graph", GRAPH_CASES);
    compare_cases("String subrange AutoInit", AUTOINIT_CASES);
}

#[test]
fn string_subrange_values_and_saturated_clamping_match_pinned_quickjs() {
    compare_cases("String subrange values", VALUE_CASES);
}

#[test]
fn string_subrange_utf16_and_large_ropes_match_pinned_quickjs() {
    compare_cases("String subrange UTF-16", UTF16_CASES);
}

#[test]
fn string_subrange_conversion_order_and_abrupt_completion_match_pinned_quickjs() {
    compare_cases("String subrange conversion order", ORDER_CASES);
}

#[test]
fn string_subrange_errors_and_nonconstructors_match_pinned_quickjs() {
    compare_cases("String subrange errors", ERROR_CASES);
}

#[test]
fn string_subrange_recursion_is_catchable_and_runtime_recovers() {
    compare_cases("String subrange stack recovery", STACK_CASES);
}

#[test]
fn string_subrange_defining_realms_and_user_throw_identity_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let substring = property_callable(&runtime, &mut defining, &defining_prototype, "substring");
    let substr = property_callable(&runtime, &mut defining, &defining_prototype, "substr");
    let slice = property_callable(&runtime, &mut defining, &defining_prototype, "slice");
    assert_eq!(
        runtime.get_prototype_of(substring.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap()),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_native_type_error(
        &runtime,
        &mut caller,
        &substring,
        Value::Null,
        &[Value::Int(0), Value::Int(1)],
        &defining_type_error,
    );
    let position_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("position").unwrap()))
        .unwrap();
    assert_native_type_error(
        &runtime,
        &mut caller,
        &slice,
        Value::String(JsString::try_from_utf8("abc").unwrap()),
        &[Value::Symbol(position_symbol)],
        &defining_type_error,
    );

    let sentinel = caller.new_object().unwrap();
    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "subrangeSentinel",
        Value::Object(sentinel.clone()),
    );
    let throwing_start = caller
        .eval(
            r#"(function(){
                var value=Object();
                value[Symbol.toPrimitive]=function(){throw subrangeSentinel};
                return value;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        caller.call(
            &substr,
            Value::String(JsString::try_from_utf8("abc").unwrap()),
            &[throwing_start, Value::Int(1)],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel.clone())),
        "start conversion did not preserve the user-thrown value",
    );

    let throwing_end = caller
        .eval(
            r#"(function(){
                var value=Object();
                value[Symbol.toPrimitive]=function(){throw subrangeSentinel};
                return value;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        caller.call(
            &slice,
            Value::String(JsString::try_from_utf8("abc").unwrap()),
            &[Value::Int(0), throwing_end],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "end conversion did not preserve the user-thrown value",
    );

    assert_eq!(
        caller.construct(&substring, &[]),
        Err(RuntimeError::Exception),
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
fn string_subrange_callables_are_per_realm_distinct_and_collectable() {
    let runtime = Runtime::new();
    let retained = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_prototype = first.string_prototype().unwrap();
        let second_prototype = second.string_prototype().unwrap();
        let first_substring =
            property_callable(&runtime, &mut first, &first_prototype, "substring");
        let first_substring_again =
            property_callable(&runtime, &mut first, &first_prototype, "substring");
        let first_substr = property_callable(&runtime, &mut first, &first_prototype, "substr");
        let first_slice = property_callable(&runtime, &mut first, &first_prototype, "slice");
        let second_substring =
            property_callable(&runtime, &mut second, &second_prototype, "substring");
        let second_substr = property_callable(&runtime, &mut second, &second_prototype, "substr");
        let second_slice = property_callable(&runtime, &mut second, &second_prototype, "slice");
        assert_eq!(first_substring, first_substring_again);
        assert_ne!(first_substring, first_substr);
        assert_ne!(first_substring, first_slice);
        assert_ne!(first_substr, first_slice);
        assert_ne!(first_substring, second_substring);
        assert_ne!(first_substr, second_substr);
        assert_ne!(first_slice, second_slice);
        assert_eq!(
            runtime
                .get_prototype_of(first_substring.as_object())
                .unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        (first_substring, first_substr, first_slice)
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(retained);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn string_subrange_stack_overflow_uses_the_caller_realm_and_recovers() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let slice = property_callable(&runtime, &mut defining, &defining_prototype, "slice");
    let defining_internal_error = intrinsic_prototype(&runtime, &mut defining, "InternalError");
    let caller_internal_error = intrinsic_prototype(&runtime, &mut caller, "InternalError");
    assert_ne!(defining_internal_error, caller_internal_error);

    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "foreignSlice",
        Value::Object(slice.as_object().clone()),
    );
    let Value::Object(error) = caller
        .eval(
            r#"(function(){
                var start=Object(),localCall=Function.prototype.call;
                function invoke(){
                    return localCall.call(foreignSlice,"abcdef",start,4)
                }
                start[Symbol.toPrimitive]=function(){
                    return invoke()
                };
                try{invoke()}catch(error){return error}
                return Object();
            })()"#,
        )
        .unwrap()
    else {
        panic!("recursive cross-realm call did not return an error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_internal_error),
        "pre-dispatch stack overflow did not use the caller realm",
    );
    assert_eq!(
        caller
            .call(
                &slice,
                Value::String(JsString::try_from_utf8("recovered").unwrap()),
                &[Value::Int(2), Value::Int(7)],
            )
            .unwrap(),
        Value::String(JsString::try_from_utf8("cover").unwrap()),
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

fn assert_native_type_error(
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
