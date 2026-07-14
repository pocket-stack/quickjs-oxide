use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "own integer and string key order excludes symbols",
        "(function(){var o={b:1,2:1,a:1,1:1};o[Symbol.iterator]=1;var s='';for(var k in o)s+=k+',';return s})()",
    ),
    (
        "nullish values enumerate nothing",
        "(function(){var n=0;for(var a in null)n++;for(var b in undefined)n++;return n})()",
    ),
    (
        "String primitive exposes UTF-16 indices",
        "(function(){var s='';for(var k in 'A\\uD83D\\uDCA9')s+=k+',';return s})()",
    ),
    (
        "non-string primitives box without own keys",
        "(function(){var n=0;for(var a in 1)n++;for(var b in true)n++;for(var c in 1n)n++;for(var d in Symbol.iterator)n++;return n})()",
    ),
    (
        "prototype duplicates and nonenumerable own keys are shadowed",
        "(function(){var p={x:1,y:1},o=Object.create(p);Object.defineProperty(o,'x',{value:2,enumerable:false,configurable:true});o.a=1;var s='';for(var k in o)s+=k+',';return s})()",
    ),
    (
        "deleted pending keys are skipped and additions miss the snapshot",
        "(function(){var o={a:1,b:2},s='';for(var k in o){s+=k;if(k==='a'){delete o.b;o.c=3}}return s})()",
    ),
    (
        "enumerability is captured but presence is live",
        "(function(){var o={a:1,b:2},s='';for(var k in o){s+=k;if(k==='a')Object.defineProperty(o,'b',{enumerable:false})}return s})()",
    ),
    (
        "prototype is snapshotted only when reached",
        "(function(){var p={p1:1,p2:2},o=Object.create(p),s='';o.a=1;for(var k in o){s+=k+',';if(k==='a'){delete p.p2;p.p3=3}}return s})()",
    ),
    (
        "prototype links are read live between levels",
        "(function(){var p1={x:1},p2={y:1},o=Object.create(p1),s='';o.a=1;for(var k in o){s+=k;if(k==='a')Object.setPrototypeOf(o,p2)}return s})()",
    ),
    (
        "deleted snapshotted own key still shadows its prototype duplicate",
        "(function(){var p={x:1},o=Object.create(p),s='';Object.defineProperty(o,'x',{value:2,enumerable:false,configurable:true});o.a=1;for(var k in o){s+=k;if(k==='a')delete o.x}return s})()",
    ),
    (
        "ordinary accessors are never invoked by enumeration",
        "(function(){var n=0,o={};Object.defineProperty(o,'x',{enumerable:true,get:function(){n++;return 1}});var s='';for(var k in o)s+=k;return s+'|'+n})()",
    ),
    (
        "fast Array deletion exposes an inherited same-name index",
        "(function(){var p=[];p[1]='proto';var a=[0,1],s='';Object.setPrototypeOf(a,p);for(var k in a){s+=k+',';if(k==='0')delete a[1]}return s})()",
    ),
    (
        "fast Array additions shadow prototype keys without joining the iteration",
        "(function(){var p={foo:'proto'},a=[0],s='';Object.setPrototypeOf(a,p);for(var k in a){s+=k+',';if(k==='0')a.foo='own'}return s})()",
    ),
    (
        "fast Array non-enumerable additions still shadow prototype keys",
        "(function(){var p={foo:'proto'},a=[0],s='';Object.setPrototypeOf(a,p);for(var k in a){s+=k+',';if(k==='0')Object.defineProperty(a,'foo',{value:'own'})}return s})()",
    ),
    (
        "defineProperty turns an Array slow and preserves its initial shadow set",
        "(function(){var p=[];p[1]='proto';var a=[0,1],s='';Object.defineProperty(a,'0',{writable:false});Object.setPrototypeOf(a,p);for(var k in a){s+=k+',';if(k==='0')delete a[1]}return s})()",
    ),
    (
        "sparse growth keeps an Array slow after the sparse key is deleted",
        "(function(){var p=[];p[1]='proto';var a=[0,1],s='';a[3]=3;delete a[3];Object.setPrototypeOf(a,p);for(var k in a){s+=k+',';if(k==='0')delete a[1]}return s})()",
    ),
    (
        "sloppy var initializer executes before the right operand",
        "(function(){var log='',k;for(var k=(log+='i',7) in (log+='r',{a:1}))log+=k;return log+'|'+k})()",
    ),
    (
        "for-in right operand accepts a comma Expression",
        "(function(){var k;for(k in (0,{x:1})){}return k})()",
    ),
    (
        "fixed and computed member targets are evaluated per key",
        "(function(){var o={value:''},name='value',s='';for(o[name] in {a:1,b:1})s+=o.value;return s+'|'+o.value})()",
    ),
    (
        "let head creates fresh captured cells",
        "(function(){var a=[];for(let k in {x:1,y:1})a.push(function(){return k});return a[0]()+'|'+a[1]()})()",
    ),
    (
        "labelled continue and break retain then drop the enumeration object",
        "(function(){var s='';outer:for(var k in {a:1,b:1,c:1}){if(k==='a')continue outer;s+=k;if(k==='b')break outer}return s})()",
    ),
    (
        "finally runs on continue and break",
        "(function(){var s='';for(var k in {a:1,b:1}){try{if(k==='a')continue;break}finally{s+=k}}return s})()",
    ),
    (
        "nested for-in loops retain independent records",
        "(function(){var s='';for(var a in {x:1,y:1})for(var b in {1:1,2:1})s+=a+b;return s})()",
    ),
];

const SYNTAX_CASES: &[(&str, &str)] = &[
    (
        "strict var initializer is rejected",
        "(function(){'use strict';for(var k=1 in {a:1}){}})()",
    ),
    ("lexical initializer is rejected", "for(let k=1 in {a:1}){}"),
    (
        "ordinary assignment initializer is rejected",
        "var k;for(k=1 in {a:1}){}",
    ),
];

#[test]
fn for_in_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP for-in differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle(&oracle, source, description),
            "for-in drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn for_in_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP for-in parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in SYNTAX_CASES {
        let rust = Command::new(env!("CARGO_BIN_EXE_qjs"))
            .args(["-e", source])
            .output()
            .unwrap_or_else(|error| panic!("could not run Rust CLI for {description}: {error}"));
        let quickjs = Command::new(&oracle)
            .args(["-e", source])
            .output()
            .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}

#[test]
fn for_in_rust_smoke_runs_without_an_oracle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::String(value) = context
        .eval("(function(){var s='';for(var k in {b:1,a:1})s+=k;return s})()")
        .expect("execute for-in smoke")
    else {
        panic!("for-in smoke did not return a String");
    };
    assert_eq!(value.to_utf8_lossy(), "ba");
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
