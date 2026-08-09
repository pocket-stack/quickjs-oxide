use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the synchronous BindingIdentifier-default subset of QuickJS 2026-06-04
// FormalParameters. Direct eval deliberately remains outside this oracle until
// the separate Parameter-Environment eval ABI lands. BindingPatterns,
// async/generators, and class forms also remain later milestones.
const CASES: &[Case] = &[
    Case {
        description: "ordinary defaults distinguish undefined from supplied values",
        source: r#"(function(){
            function f(a=40,b=a+2){return a+'|'+b}
            return f(undefined)+';'+f(41,1);
        })()"#,
        expected: "return|string|40|42;41|1",
    },
    Case {
        description: "a supplied non-undefined value skips its initializer",
        source: r#"(function(){
            var calls=0;
            function f(a=(calls++,42)){return a}
            var value=f(7);
            return calls+'|'+value;
        })()"#,
        expected: "return|string|0|7",
    },
    Case {
        description: "arrow defaults resolve prior parameter bindings",
        source: "((a=40,b=a+2)=>b)()",
        expected: "return|number|42",
    },
    Case {
        description: "object method defaults retain dynamic this",
        source: "({base:40,method(a=this.base+2){return a}}).method()",
        expected: "return|number|42",
    },
    Case {
        description: "Function constructor reuses ordinary default parsing",
        source: "Function('a=40','b=a+2','return b')()",
        expected: "return|number|42",
    },
    Case {
        description: "a later parameter remains in TDZ during an earlier initializer",
        source: r#"(function(){
            try {(function(a=b,b=1){})()}
            catch(error) {return error.name}
            return 'none';
        })()"#,
        expected: "return|string|ReferenceError",
    },
    Case {
        description: "a parameter remains in its own TDZ during self initialization",
        source: r#"(function(){
            try {(function(a=a){})()}
            catch(error) {return error.name}
            return 'none';
        })()"#,
        expected: "return|string|ReferenceError",
    },
    Case {
        description: "default parameters make sloppy arguments unmapped",
        source: r#"(function(a=1){
            arguments[0]=9;
            var before=a;
            a=7;
            return before+'|'+a+'|'+arguments[0];
        })(2)"#,
        expected: "return|string|2|7|9",
    },
    Case {
        description: "function length stops before the first default",
        source: r#"(function(){
            var ordinary=function(a,b=1,c){};
            var arrow=(a=1,b)=>0;
            var method=({method(a,b=1){}}).method;
            var dynamic=Function('a','b=1','c','');
            return ordinary.length+'|'+arrow.length+'|'+method.length+'|'+dynamic.length;
        })()"#,
        expected: "return|string|1|0|1|1",
    },
    Case {
        description: "body hoists replace the raw argument but not the initializer closure cell",
        source: r#"(function(){
            var read;
            function f(a=1,b=(read=()=>a,0)){
                function a(){return 42}
                return typeof a+'|'+a()+'|'+read();
            }
            return f();
        })()"#,
        expected: "return|string|function|42|1",
    },
    Case {
        description: "anonymous function defaults receive the parameter name",
        source: "(function(a=function(){}){return a.name})()",
        expected: "return|string|a",
    },
    Case {
        description: "anonymous arrow defaults receive the parameter name",
        source: "((a=()=>0)=>a.name)()",
        expected: "return|string|a",
    },
    Case {
        description: "private function names stay outside same-named body bindings",
        source: r#"(function(){
            var read=(function f(a=f){var f;return typeof a+'|'+(a===undefined)+'|'+typeof f})();
            var closure=(function f(a=()=>f){var f=1;return typeof a()+'|'+(a()===f)})();
            var write=(function f(a=(f=1)){var f;return typeof f+'|'+(f===1)})();
            var strict;
            try {
                (function(){'use strict';return (function f(a=(f=1)){var f})()})()
            } catch(error) { strict=error.name+'|'+error.message }
            return read+';'+closure+';'+write+';'+strict;
        })()"#,
        expected: "return|string|function|false|undefined;function|false;undefined|false;TypeError|'f' is read-only",
    },
    Case {
        description: "an identifier default composes with an identifier rest parameter",
        source: "(function(a=40,...rest){return a+rest[0]+rest.length})(undefined,1)",
        expected: "return|number|42",
    },
    Case {
        description: "QuickJS separates the body raw argument from the initializer closure cell",
        source: r#"(function(){
            var read;
            function f(a=1,b=(read=()=>a,0)){
                a=2;
                return a+'|'+read();
            }
            return f();
        })()"#,
        expected: "return|string|2|1",
    },
];

#[test]
fn identifier_default_parameter_vectors_exclude_direct_eval() {
    for case in CASES {
        assert!(
            !case.source.contains("eval"),
            "direct eval escaped into the identifier-default oracle: {}",
            case.description,
        );
    }
}

#[test]
fn identifier_default_parameter_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP identifier-default-parameter oracle self-check: set QJS_ORACLE to upstream qjs"
        );
        return;
    };
    for case in CASES {
        assert_eq!(
            observe_oracle(&oracle, case.source, case.description),
            case.expected,
            "pinned QuickJS vector drifted for {}",
            case.description,
        );
    }
}

#[test]
fn identifier_default_parameter_rust_smoke_runs_without_an_oracle() {
    for case in CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust(&runtime, &mut context, case.source, case.description),
            case.expected,
            "Rust identifier-default-parameter result drifted for {}",
            case.description,
        );
    }
}

#[test]
fn identifier_default_parameter_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP identifier-default-parameter differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for case in CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust(&runtime, &mut context, case.source, case.description),
            observe_oracle(&oracle, case.source, case.description),
            "identifier-default-parameter differential drifted for {}",
            case.description,
        );
    }
}

fn observe_rust(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_text(value),
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
                    primitive_text(value),
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
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"))
        .trim_end()
        .to_owned()
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
            if runtime.as_callable(object).unwrap().is_some() {
                "function"
            } else {
                "object"
            }
        }
        Value::Symbol(_) => "symbol",
    }
}

fn primitive_text(value: Value) -> String {
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
