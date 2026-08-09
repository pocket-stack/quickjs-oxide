use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04 RegExp dotAll semantics.
// QuickJS carries `s` through `LRE_FLAG_DOTALL`: `libregexp.c` selects
// REOP_any instead of REOP_dot, scoped modifiers save and restore dotall state,
// and `quickjs.c` exposes the flag through the RegExp accessors and species
// consumers. These vectors focus on interactions not isolated by the broader
// RegExp intrinsic, modifier, matchAll, and split oracle suites.

const PRELUDE: &str = r#"
function __bit(value){return value?"1":"0"}
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    if(descriptor===undefined)return "missing";
    return __bit(descriptor.writable)+__bit(descriptor.enumerable)+
        __bit(descriptor.configurable);
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

const MATCHING_CASES: &[(&str, &str)] = &[
    (
        "dotAll includes only the four ordinary line terminators excluded by dot",
        r#"(function(){
            var values=["\n","\r","\u2028","\u2029","\v","\f","\u0085","\u180e"];
            function bits(regexp){
                var output="",index=0;
                while(index<values.length)output+=regexp.test(values[index++])?"1":"0";
                return output;
            }
            var astral="\ud83d\ude00",high="\ud83d",low="\ude00",
                legacy=/^.$/s.exec(astral),unicode=/^.$/su.exec(astral),
                unicodeHigh=/^.$/su.exec(high),unicodeLow=/^.$/su.exec(low);
            return [
                bits(/./),bits(/./s),legacy===null,
                unicode[0].length,unicode[0].codePointAt(0).toString(16),
                /^.$/s.test(high),/^.$/s.test(low),
                unicodeHigh[0].charCodeAt(0).toString(16),
                unicodeLow[0].charCodeAt(0).toString(16)
            ].join("|");
        })()"#,
    ),
    (
        "dotAll global and sticky execution retain UTF-16 low-surrogate lastIndex behavior",
        r#"(function(){
            var input="A\ud83d\ude00B";
            function run(regexp,lastIndex){
                regexp.lastIndex=lastIndex;
                var match=regexp.exec(input);
                if(match===null)return "null:"+regexp.lastIndex;
                return [match.index,match[0].length,
                    match[0].charCodeAt(0).toString(16),regexp.lastIndex].join(":");
            }
            return [
                run(/./suy,2),run(/./sy,2),run(/./sug,2),
                run(/./sg,2),run(/./suy,1)
            ].join("|");
        })()"#,
    ),
];

const SURFACE_AND_CONSTRUCTION_CASES: &[(&str, &str)] = &[
    (
        "dotAll getter metadata prototype special case and brands are exact",
        r#"(function(){
            var descriptor=Object.getOwnPropertyDescriptor(RegExp.prototype,"dotAll"),
                getter=descriptor.get;
            return [
                typeof getter,descriptor.set===undefined,
                descriptor.enumerable,descriptor.configurable,
                getter.name,getter.length,__bits(getter,"name"),__bits(getter,"length"),
                getter.call(/x/s),getter.call(/x/),
                getter.call(RegExp.prototype)===undefined,
                __completion(function(){return getter.call({})}),
                __completion(function(){return getter.call(1)})
            ].join("|");
        })()"#,
    ),
    (
        "constructor and legacy compile preserve replace and canonically order s",
        r#"(function(){
            var literal=/x/s,copied=new RegExp(literal),replaced=new RegExp(literal,"g"),
                preserved=/old/g,dropped=/old/s,canonical=/old/,
                preserveResult=preserved.compile(literal),
                dropResult=dropped.compile("x","g"),
                canonicalResult=canonical.compile("x","yusmigd");
            return [
                RegExp(literal)===literal,copied!==literal,copied.source,copied.flags,
                replaced.source,replaced.flags,
                preserveResult===preserved,preserved.source,preserved.flags,
                preserved.dotAll,preserved.lastIndex,
                dropResult===dropped,dropped.flags,dropped.dotAll,dropped.lastIndex,
                canonicalResult===canonical,canonical.flags,canonical.dotAll
            ].join("|");
        })()"#,
    ),
];

const SCOPED_MODIFIER_CASES: &[(&str, &str)] = &[(
    "nested dotAll add remove and re-add restore every enclosing state",
    r#"(function(){
        var add=/(?s:a.(?-s:b.(?s:c.d)e.)f.)/,
            remove=/(?-s:a.(?s:b.c)d.)e./s;
        return [
            add.test("a\nbXc\ndeXf\n"),
            add.test("a\nb\nc\ndeXf\n"),
            add.test("a\nbXc\nde\nf\n"),
            add.dotAll,add.flags,
            remove.test("aXb\ncdXe\n"),
            remove.test("a\nb\ncdXe\n"),
            remove.dotAll,remove.flags
        ].join("|");
    })()"#,
)];

const SPECIES_CASES: &[(&str, &str)] = &[(
    "matchAll preserves s while split preserves s and appends sticky",
    r#"(function(){
        var matching=/./sg,matchHolder=Object(),matchFlags="",
            matchPattern=false,matchArguments=0;
        function MatchSpecies(pattern,flags){
            matchFlags=flags;
            matchPattern=pattern===matching;
            matchArguments=arguments.length;
            return new RegExp(pattern,flags);
        }
        matchHolder[Symbol.species]=MatchSpecies;
        matching.constructor=matchHolder;
        matching.lastIndex=0;
        var iterator=RegExp.prototype[Symbol.matchAll].call(matching,"\n"),
            match=iterator.next().value,done=iterator.next();

        var separator=/./s,splitHolder=Object(),splitFlags="",
            splitPattern=false,splitArguments=0;
        function SplitSpecies(pattern,flags){
            splitFlags=flags;
            splitPattern=pattern===separator;
            splitArguments=arguments.length;
            return new RegExp(pattern,flags);
        }
        splitHolder[Symbol.species]=SplitSpecies;
        separator.constructor=splitHolder;
        separator.lastIndex=7;
        var parts=RegExp.prototype[Symbol.split].call(separator,"\n");

        return [
            matchFlags,matchPattern,matchArguments,matching.lastIndex,
            match.index,match[0].charCodeAt(0),done.done,
            splitFlags,splitPattern,splitArguments,separator.lastIndex,
            parts.length,parts[0],parts[1]
        ].join("|");
    })()"#,
)];

#[test]
fn regexp_dotall_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp dotAll oracle self-check: set QJS_ORACLE to upstream qjs");
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
fn regexp_dotall_matching_and_utf16_state_match_pinned_quickjs() {
    compare_cases("RegExp dotAll matching and UTF-16 state", MATCHING_CASES);
}

#[test]
fn regexp_dotall_surface_and_construction_match_pinned_quickjs() {
    compare_cases(
        "RegExp dotAll surface and construction",
        SURFACE_AND_CONSTRUCTION_CASES,
    );
}

#[test]
fn regexp_dotall_scoped_modifiers_match_pinned_quickjs() {
    compare_cases("RegExp dotAll scoped modifiers", SCOPED_MODIFIER_CASES);
}

#[test]
fn regexp_dotall_species_flags_match_pinned_quickjs() {
    compare_cases("RegExp dotAll species flags", SPECIES_CASES);
}

#[test]
fn regexp_dotall_getter_cross_realm_brand_and_errors_use_exact_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let defining_getter = eval_callable(
        &runtime,
        &mut defining,
        "Object.getOwnPropertyDescriptor(RegExp.prototype,'dotAll').get",
        "defining dotAll getter",
    );
    let caller_getter = eval_callable(
        &runtime,
        &mut caller,
        "Object.getOwnPropertyDescriptor(RegExp.prototype,'dotAll').get",
        "caller dotAll getter",
    );
    let defining_prototype = eval_object(
        &mut defining,
        "RegExp.prototype",
        "defining RegExp prototype",
    );
    let caller_prototype = eval_object(&mut caller, "RegExp.prototype", "caller RegExp prototype");
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
    let caller_dotall = eval_object(&mut caller, "/./s", "caller dotAll RegExp");
    let defining_plain = eval_object(&mut defining, "/./", "defining plain RegExp");

    assert_eq!(
        caller
            .call(&defining_getter, Value::Object(caller_dotall), &[])
            .unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        defining
            .call(&caller_getter, Value::Object(defining_plain), &[])
            .unwrap(),
        Value::Bool(false),
    );
    assert_eq!(
        defining
            .call(
                &defining_getter,
                Value::Object(defining_prototype.clone()),
                &[],
            )
            .unwrap(),
        Value::Undefined,
    );
    assert_eq!(
        caller
            .call(&caller_getter, Value::Object(caller_prototype.clone()), &[],)
            .unwrap(),
        Value::Undefined,
    );

    assert_eq!(
        caller.call(&defining_getter, Value::Object(caller_prototype), &[],),
        Err(RuntimeError::Exception),
    );
    let defining_error = take_exception_object(&mut caller, "defining dotAll brand TypeError");
    assert_eq!(
        runtime.get_prototype_of(&defining_error).unwrap(),
        Some(defining_type_error),
    );

    assert_eq!(
        defining.call(&caller_getter, Value::Object(defining_prototype), &[],),
        Err(RuntimeError::Exception),
    );
    let caller_error = take_exception_object(&mut defining, "caller dotAll brand TypeError");
    assert_eq!(
        runtime.get_prototype_of(&caller_error).unwrap(),
        Some(caller_type_error),
    );
}

fn case_groups() -> [(&'static str, &'static [(&'static str, &'static str)]); 4] {
    [
        ("matching", MATCHING_CASES),
        ("surface and construction", SURFACE_AND_CONSTRUCTION_CASES),
        ("scoped modifiers", SCOPED_MODIFIER_CASES),
        ("species", SPECIES_CASES),
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
