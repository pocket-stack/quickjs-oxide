use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "actual argc is distinct from padded formal slots",
        "(function(a,b){return arguments.length+'|'+arguments[0]+'|'+arguments[1]+'|'+(1 in arguments)+'|'+(2 in arguments)})(1)",
    ),
    (
        "an explicit undefined argument still creates an index",
        "(function(a){return arguments.length+'|'+(0 in arguments)+'|'+arguments[0]})(undefined)",
    ),
    (
        "extra actual arguments are indexed without exposing padded formals",
        "(function(a){arguments[1]=3;return arguments.length+'|'+a+'|'+arguments[1]})(1,2)",
    ),
    (
        "sloppy simple parameters alias in both directions",
        "(function(a){a=2;var first=arguments[0];arguments[0]=3;return first+'|'+a})(1)",
    ),
    (
        "strict arguments are unmapped",
        "(function(a){'use strict';a=2;var first=arguments[0];arguments[0]=3;return first+'|'+a})(1)",
    ),
    (
        "duplicate parameters retain one mapped cell per actual index",
        "(function(a,a){arguments[0]=4;var first=a;arguments[1]=5;return first+'|'+a+'|'+arguments[0]})(1,2)",
    ),
    (
        "var arguments preserves then may overwrite the implicit object",
        "(function(a){var before=arguments[0];var arguments;var same=arguments[0];arguments=7;return before+'|'+same+'|'+arguments+'|'+a})(1)",
    ),
    (
        "the implicit binding precedes a sloppy named-expression self binding",
        "(function arguments(a){return (arguments===arguments.callee)+'|'+arguments[0]})(6)",
    ),
    (
        "an explicit arguments parameter suppresses the implicit object",
        "(function(arguments){return typeof arguments+'|'+arguments})(7)",
    ),
    (
        "a body lexical arguments binding suppresses the implicit object",
        "(function(){let arguments=9;return typeof arguments+'|'+arguments})()",
    ),
    (
        "a direct body declaration overwrites the initialized arguments local",
        "(function(){var beforeType=typeof arguments;function arguments(){return 8}return beforeType+'|'+typeof arguments+'|'+arguments()})()",
    ),
    (
        "QuickJS leaves an Annex B block function disconnected from implicit arguments",
        "(function(){var before=typeof arguments;{function arguments(){return 8}}var call;try{arguments()}catch(e){call=e.name+':'+e.message}return before+'|'+typeof arguments+'|'+call})()",
    ),
    (
        "mapped parameter cells survive the creating frame",
        "(function(){var args,set,get;(function(a){args=arguments;set=function(v){a=v};get=function(){return a}})(1);set(7);var first=args[0];args[0]=8;return first+'|'+get()})()",
    ),
    (
        "nested ordinary functions each receive their own arguments object",
        "(function(a){return (function(b){return arguments[0]})(2)+'|'+arguments[0]})(1)",
    ),
    (
        "deleting an index severs its parameter mapping",
        "(function(a){delete arguments[0];a=2;return (0 in arguments)+'|'+arguments[0]+'|'+a})(1)",
    ),
    (
        "descriptor changes preserve mapping until writable becomes false",
        "(function(a){Object.defineProperty(arguments,'0',{value:4,enumerable:false});a=5;var first=arguments[0];Object.defineProperty(arguments,'0',{writable:false});a=6;return first+'|'+arguments[0]+'|'+a+'|'+Object.keys(arguments)})(1)",
    ),
    (
        "an accessor descriptor severs a mapped index",
        "(function(a){Object.defineProperty(arguments,'0',{get:function(){return 9},set:function(v){}});a=4;return arguments[0]+'|'+a})(1)",
    ),
    (
        "seal preserves mapping while freeze severs it",
        "(function(a,b){Object.seal(arguments);a=3;var first=arguments[0];Object.freeze(arguments);b=4;return first+'|'+arguments[0]+'|'+arguments[1]+'|'+b})(1,2)",
    ),
    (
        "preventExtensions preserves writes to an existing mapped index",
        "(function(a){Object.preventExtensions(arguments);a=2;arguments[0]=3;return a+'|'+arguments[0]+'|'+Object.isExtensible(arguments)})(1)",
    ),
    (
        "a rejected nonconfigurable redefinition preserves the mapping",
        "(function(a){Object.defineProperty(arguments,'0',{configurable:false});var threw;try{Object.defineProperty(arguments,'0',{configurable:true})}catch(e){threw=e.name}a=4;return threw+'|'+arguments[0]})(1)",
    ),
    (
        "delete followed by an inherited setter never reconnects the mapping",
        "(function(a){var seen=0,p={};Object.defineProperty(p,'0',{set:function(v){seen=v},configurable:true});Object.setPrototypeOf(arguments,p);delete arguments[0];arguments[0]=5;a=6;return seen+'|'+a+'|'+(0 in arguments)+'|'+arguments[0]})(1)",
    ),
    (
        "mapped callee is a configurable data property",
        "(function(){var d=Object.getOwnPropertyDescriptor(arguments,'callee');return typeof d.value+'|'+d.writable+'|'+d.enumerable+'|'+d.configurable+'|'+(d.value===arguments.callee)})()",
    ),
    (
        "strict callee uses the realm poison accessor",
        "(function(){'use strict';var d=Object.getOwnPropertyDescriptor(arguments,'callee'),e;try{arguments.callee}catch(x){e=x.name+':'+x.message}return typeof d.get+'|'+(d.get===d.set)+'|'+d.enumerable+'|'+d.configurable+'|'+e})()",
    ),
    (
        "brand prototype keys and symbols match the Arguments class",
        "(function(){return Object.getOwnPropertyNames(arguments)+'|'+Object.getOwnPropertySymbols(arguments).length+'|'+Object.prototype.toString.call(arguments)+'|'+Array.isArray(arguments)+'|'+(Object.getPrototypeOf(arguments)===Object.prototype)})(1,2)",
    ),
    (
        "iterator identity is the realm-cached original Array values function",
        "(function(){var original=Array.prototype.values;Array.prototype.values=function(){};return (function(){return (arguments[Symbol.iterator]===original)+'|'+Array.from(arguments)})(7,8)})()",
    ),
    (
        "length is an independent writable configurable data property",
        "(function(a){arguments.length=9;var d=Object.getOwnPropertyDescriptor(arguments,'length');delete arguments.length;return d.value+'|'+d.writable+'|'+d.enumerable+'|'+d.configurable+'|'+('length' in arguments)+'|'+a})(1)",
    ),
    (
        "fast Arguments deletion reveals an inherited same-name index",
        "(function(a,b){var p={};p[1]='proto';Object.setPrototypeOf(arguments,p);var s='';for(var k in arguments){s+=k;if(k==='0')delete arguments[1]}return s})(1,2)",
    ),
    (
        "defineProperty makes fast Arguments slow without breaking mapping",
        "(function(a,b){Object.defineProperty(arguments,'0',{});var p={};p[1]='proto';Object.setPrototypeOf(arguments,p);var s='';for(var k in arguments){s+=k;if(k==='0')delete arguments[1]}a=7;return s+'|'+arguments[0]})(1,2)",
    ),
    (
        "new own properties shadow prototypes but miss a fast snapshot",
        "(function(a){var p={x:1};Object.setPrototypeOf(arguments,p);var s='';for(var k in arguments){s+=k;if(k==='0')arguments.x=2}return s})(1)",
    ),
    (
        "Function constructor bodies receive their own arguments object",
        "Function('a','return arguments.length+\"|\"+arguments[0]')(11)",
    ),
    (
        "ordinary construction preserves actual argc and callee identity",
        "(function(){var F=function(a){this.out=arguments.length+'|'+arguments[0]+'|'+(arguments.callee===F)};return new F(7).out})()",
    ),
    (
        "bound calls expose bound and call-time actuals to the target",
        "(function(){var target=function(a){return arguments.length+'|'+arguments[0]+'|'+arguments[1]+'|'+(arguments.callee===target)};return target.bind(null,3)(4)})()",
    ),
    (
        "Function apply supplies the target's exact actual argument vector",
        "(function(){var target=function(a){return arguments.length+'|'+arguments[0]+'|'+arguments[1]+'|'+(arguments.callee===target)};return target.apply(null,[5,6])})()",
    ),
];

#[test]
fn arguments_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP arguments differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle(&oracle, source, description),
            "arguments drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn arguments_rust_smoke_runs_without_an_oracle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context
        .eval("(function(a){a=41;arguments[0]++;return a})(1)")
        .expect("execute arguments smoke");
    assert_eq!(value, Value::Int(42));
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
            primitive_value_text(value)
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    error_string_property(runtime, context, &error, "name", description),
                    error_string_property(runtime, context, &error, "message", description),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value)
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
    let wrapper = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print('return|' + typeof value + '|' + String(value));
} catch (error) {
  if (error !== null && typeof error === 'object')
    print('throw|object|' + error.name + '|' + error.message);
  else
    print('throw|' + typeof error + '|' + String(error));
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
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"));
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
}

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &quickjs_oxide::ObjectRef,
    name: &str,
    description: &str,
) -> String {
    let key = runtime
        .intern_property_key(name)
        .expect("Error property key");
    let Value::String(value) = context
        .get_property(error, &key)
        .unwrap_or_else(|failure| panic!("read Error.{name} for {description}: {failure}"))
    else {
        panic!("Error.{name} was not a string for {description}");
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
            if runtime
                .as_callable(object)
                .expect("inspect callable")
                .is_some()
            {
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
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}
