use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04 RegExp match indices.
//
// These vectors keep `d` observable all the way through the consumers that
// either preserve the `indices` graph (`exec`, non-global match, and matchAll)
// or intentionally discard it (global match, split, and replacement).  They
// also pin QuickJS's UTF-16 `lastIndex` behavior when execution starts on the
// low surrogate of a pair.

const PRELUDE: &str = r#"
function __bit(value){return value?"1":"0"}
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    if(descriptor===undefined)return "missing";
    return __bit(descriptor.writable)+__bit(descriptor.enumerable)+
        __bit(descriptor.configurable);
}
function __units(value){
    var output=[],index=0;
    while(index<value.length){
        output[index]=value.charCodeAt(index).toString(16);
        index++;
    }
    return value.length+"["+output.join(",")+"]";
}
"#;

const RESULT_SHAPE_CASES: &[(&str, &str)] = &[
    (
        "indices and pair arrays expose own data descriptors including unmatched undefined",
        r#"(function(){
            var match=/(a)(b)?/d.exec("za"),indices=match.indices,
                pair=indices[0],unmatched=Object.getOwnPropertyDescriptor(indices,"2");
            return [
                Object.getOwnPropertyNames(indices).join(","),
                Object.getOwnPropertyNames(pair).join(","),
                __bits(match,"indices"),__bits(indices,"0"),__bits(indices,"1"),
                __bits(indices,"2"),__bits(indices,"groups"),
                Object.prototype.hasOwnProperty.call(indices,"2"),
                unmatched.value===undefined,
                __bits(pair,"0"),__bits(pair,"1"),__bits(pair,"length"),
                pair[0],pair[1],indices[1][0],indices[1][1],
                Object.getPrototypeOf(indices)===Array.prototype,
                Object.getPrototypeOf(pair)===Array.prototype
            ].join("|");
        })()"#,
    ),
    (
        "duplicate named indices alias the first participating capture pair",
        r#"(function(){
            var regexp=/(?<x>a)|(?<x>b)/d,
                left=regexp.exec("..a"),right=regexp.exec("..b"),
                leftGroups=left.indices.groups,rightGroups=right.indices.groups;
            return [
                Object.getPrototypeOf(leftGroups)===null,
                Object.getPrototypeOf(rightGroups)===null,
                leftGroups.x===left.indices[1],leftGroups.x===left.indices[2],
                rightGroups.x===right.indices[1],rightGroups.x===right.indices[2],
                Object.prototype.hasOwnProperty.call(left.indices,"2"),
                left.indices[2]===undefined,
                Object.prototype.hasOwnProperty.call(right.indices,"1"),
                right.indices[1]===undefined,
                __bits(leftGroups,"x"),__bits(rightGroups,"x"),
                leftGroups.x.join(","),rightGroups.x.join(",")
            ].join("|");
        })()"#,
    ),
];

const UTF16_LAST_INDEX_CASES: &[(&str, &str)] = &[(
    "Unicode scanning and sticky execution distinguish a low-surrogate lastIndex",
    r#"(function(){
        var input="A\ud83d\ude00B";
        function run(regexp,lastIndex){
            regexp.lastIndex=lastIndex;
            var match=regexp.exec(input);
            return [match.index,__units(match[0]),match.indices[0].join(","),
                regexp.lastIndex].join(":");
        }
        return [
            run(/./dgu,2),
            run(/./duy,2),
            run(/./dg,2),
            run(/./du,2),
            run(/./duy,1)
        ].join("|");
    })()"#,
)];

const MATCH_CASES: &[(&str, &str)] = &[(
    "non-global match preserves indices while global match returns only strings",
    r#"(function(){
        var single="aba".match(/(a)/d),regexp=/a/dg;
        regexp.lastIndex=2;
        var global="aba".match(regexp);
        return [
            single[0],single[1],single.index,single.input,
            single.indices[0].join(","),single.indices[1].join(","),
            Object.prototype.hasOwnProperty.call(single,"indices"),
            __bits(single,"indices"),
            global.join(","),Object.getOwnPropertyNames(global).join(","),
            Object.prototype.hasOwnProperty.call(global,"indices"),
            Object.prototype.hasOwnProperty.call(global,"index"),
            Object.prototype.hasOwnProperty.call(global,"input"),
            Object.prototype.hasOwnProperty.call(global,"groups"),
            regexp.lastIndex
        ].join("|");
    })()"#,
)];

const SPECIES_FLAGS_CASES: &[(&str, &str)] = &[(
    "matchAll preserves d and g while split preserves d and appends sticky",
    r#"(function(){
        var matching=/a/dg,matchHolder=Object(),matchFlags="",
            matchPattern=false,matchArguments=0;
        function MatchSpecies(pattern,flags){
            matchFlags=flags;
            matchPattern=pattern===matching;
            matchArguments=arguments.length;
            return /a/dg;
        }
        matchHolder[Symbol.species]=MatchSpecies;
        matching.constructor=matchHolder;
        matching.lastIndex=1;
        var iterator=RegExp.prototype[Symbol.matchAll].call(matching,"ba"),
            match=iterator.next().value;

        var separator=/,/d,splitHolder=Object(),splitFlags="",
            splitPattern=false,splitArguments=0;
        function SplitSpecies(pattern,flags){
            splitFlags=flags;
            splitPattern=pattern===separator;
            splitArguments=arguments.length;
            return /,/dy;
        }
        splitHolder[Symbol.species]=SplitSpecies;
        separator.constructor=splitHolder;
        separator.lastIndex=7;
        var parts=RegExp.prototype[Symbol.split].call(separator,"a,b");

        return [
            matchFlags,matchPattern,matchArguments,matching.lastIndex,
            match.index,match.indices[0].join(","),
            splitFlags,splitPattern,splitArguments,separator.lastIndex,
            parts.join(",")
        ].join("|");
    })()"#,
)];

const REPLACE_CASES: &[(&str, &str)] = &[(
    "replacement callbacks neither receive nor read match indices",
    r#"(function(){
        var nativeSeen="";
        var nativeResult="za".replace(/(?<x>a)/d,
            function(match,capture,index,input,groups){
                nativeSeen=[
                    arguments.length,match,capture,index,input,groups.x,
                    Object.getPrototypeOf(groups)===null,
                    arguments[5]===undefined
                ].join(":");
                return groups.x.toUpperCase();
            });

        var reads=0,calls=0,result=Object(),receiver=Object(),genericSeen="";
        result.length=1;
        result[0]="a";
        result.index=0;
        result.groups=undefined;
        Object.defineProperty(result,"indices",{get:function(){
            reads++;
            throw 73;
        }});
        receiver.flags="";
        receiver.exec=function(){return result};
        var genericResult=RegExp.prototype[Symbol.replace].call(
            receiver,"a",function(match,index,input){
                calls++;
                genericSeen=[arguments.length,match,index,input,
                    arguments[3]===undefined].join(":");
                return "X";
            });
        return [nativeResult,nativeSeen,genericResult,genericSeen,calls,reads].join("|");
    })()"#,
)];

#[test]
fn regexp_match_indices_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp match-indices oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for (group, cases) in case_groups() {
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
fn regexp_match_indices_result_shapes_match_pinned_quickjs() {
    compare_cases("RegExp match-indices result shapes", RESULT_SHAPE_CASES);
}

#[test]
fn regexp_match_indices_utf16_last_index_matches_pinned_quickjs() {
    compare_cases(
        "RegExp match-indices UTF-16 lastIndex",
        UTF16_LAST_INDEX_CASES,
    );
}

#[test]
fn regexp_match_indices_match_consumers_match_pinned_quickjs() {
    compare_cases("RegExp match-indices match consumers", MATCH_CASES);
}

#[test]
fn regexp_match_indices_species_flags_match_pinned_quickjs() {
    compare_cases("RegExp match-indices species flags", SPECIES_FLAGS_CASES);
}

#[test]
fn regexp_match_indices_replace_callbacks_match_pinned_quickjs() {
    compare_cases("RegExp match-indices replacement", REPLACE_CASES);
}

#[test]
fn regexp_match_indices_nested_arrays_use_the_exec_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype =
        eval_object(&mut defining, "Array.prototype", "defining Array prototype");
    let caller_array_prototype =
        eval_object(&mut caller, "Array.prototype", "caller Array prototype");
    let exec = eval_callable(
        &runtime,
        &mut defining,
        "RegExp.prototype.exec",
        "defining RegExp exec",
    );
    let regexp = eval_object(
        &mut caller,
        "new RegExp('(?<x>a)(b)?','d')",
        "caller match-indices RegExp",
    );
    let result = expect_object(
        caller
            .call(
                &exec,
                Value::Object(regexp),
                &[Value::String(JsString::try_from_utf8("za").unwrap())],
            )
            .expect("cross-realm match-indices exec"),
        "cross-realm match-indices result",
    );
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype.clone()),
    );
    assert_ne!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(caller_array_prototype.clone()),
    );

    let indices = object_property(&runtime, &mut caller, &result, "indices");
    assert_eq!(
        runtime.get_prototype_of(&indices).unwrap(),
        Some(defining_array_prototype.clone()),
    );
    let whole_pair = object_property(&runtime, &mut caller, &indices, "0");
    let named_pair = object_property(&runtime, &mut caller, &indices, "1");
    for pair in [&whole_pair, &named_pair] {
        assert_eq!(
            runtime.get_prototype_of(pair).unwrap(),
            Some(defining_array_prototype.clone()),
        );
        assert_ne!(
            runtime.get_prototype_of(pair).unwrap(),
            Some(caller_array_prototype.clone()),
        );
    }
    assert_eq!(
        value_property(&runtime, &mut caller, &indices, "2"),
        Value::Undefined,
    );

    let groups = object_property(&runtime, &mut caller, &indices, "groups");
    assert_eq!(runtime.get_prototype_of(&groups).unwrap(), None);
    assert_eq!(
        value_property(&runtime, &mut caller, &groups, "x"),
        Value::Object(named_pair),
        "indices.groups did not alias the participating pair",
    );
}

fn case_groups() -> [(&'static str, &'static [(&'static str, &'static str)]); 5] {
    [
        ("result shapes", RESULT_SHAPE_CASES),
        ("UTF-16 lastIndex", UTF16_LAST_INDEX_CASES),
        ("match consumers", MATCH_CASES),
        ("species flags", SPECIES_FLAGS_CASES),
        ("replacement", REPLACE_CASES),
    ]
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
    let callable = eval_object(context, source, description);
    runtime
        .as_callable(&callable)
        .unwrap()
        .unwrap_or_else(|| panic!("{description} was not callable"))
}

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    expect_object(
        context
            .eval(source)
            .unwrap_or_else(|error| panic!("evaluate {description}: {error}")),
        description,
    )
}

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(value) = value else {
        panic!("{description} was not an object");
    };
    value
}

fn value_property(
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

fn object_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> ObjectRef {
    expect_object(value_property(runtime, context, object, name), name)
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = value_property(runtime, context, object, name) else {
        panic!("{name} was not a string");
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
