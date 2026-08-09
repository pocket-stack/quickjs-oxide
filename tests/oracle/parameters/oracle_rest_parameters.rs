use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the synchronous identifier-rest subset of QuickJS 2026-06-04
// FormalParameters. Rest BindingPatterns, defaults, destructuring, async,
// generators, and classes remain outside this milestone.
const CASES: &[Case] = &[
    Case {
        description: "ordinary functions collect only actual trailing arguments",
        source: r#"(function(left,...rest){
            return left+'|'+rest.length+'|'+rest[0]+'|'+rest[1]+'|'+
                Array.isArray(rest)+'|'+(Object.getPrototypeOf(rest)===Array.prototype);
        })(40,1,2)"#,
        expected: "return|string|40|2|1|2|true|true",
    },
    Case {
        description: "missing fixed arguments do not create rest padding",
        source: r#"(function(left,right,...rest){
            return left+'|'+String(right)+'|'+rest.length+'|'+(0 in rest);
        })(42)"#,
        expected: "return|string|42|undefined|0|false",
    },
    Case {
        description: "function length stops before an identifier rest parameter",
        source: r#"(function(){
            var ordinary=function(left,...rest){};
            var arrow=(left,right,...rest)=>0;
            var method=({method(left,...rest){}}).method;
            var dynamic=Function('left','...rest','');
            return ordinary.length+'|'+arrow.length+'|'+method.length+'|'+dynamic.length;
        })()"#,
        expected: "return|string|1|2|1|1",
    },
    Case {
        description: "rest makes sloppy arguments unmapped in both directions",
        source: r#"(function(left,...rest){
            arguments[0]=7;arguments[1]=8;
            var before=left+'|'+rest[0];
            left=9;rest[0]=10;
            return before+'|'+left+'|'+rest[0]+'|'+arguments[0]+'|'+arguments[1];
        })(1,2)"#,
        expected: "return|string|1|2|9|10|7|8",
    },
    Case {
        description: "arguments snapshots raw values before rest overwrites its slot",
        source: r#"(function(...rest){
            return arguments.length+'|'+arguments[0]+'|'+rest.length+'|'+rest[0]+'|'+
                (arguments[0]===rest);
        })(42)"#,
        expected: "return|string|1|42|1|42|false",
    },
    Case {
        description: "body var reuse retains rest while function hoists replace it afterward",
        source: r#"(function(){
            var retained=(function(...rest){var rest;return Array.isArray(rest)+'|'+rest.length})(1,2);
            var hoisted=(function(...rest){function rest(){return 42}return typeof rest+'|'+rest()})(1);
            return retained+';'+hoisted;
        })()"#,
        expected: "return|string|true|2;function|42",
    },
    Case {
        description: "closures capture the initialized rest binding",
        source: r#"(function(...rest){
            return function(){return rest[0]+rest[1]};
        })(40,2)()"#,
        expected: "return|number|42",
    },
    Case {
        description: "arrows collect their own rest while preserving lexical this",
        source: r#"(function(){
            var arrow=(left,...rest)=>this.base+left+rest[0];
            return arrow.call({base:100},1,1);
        }).call({base:40})"#,
        expected: "return|number|42",
    },
    Case {
        description: "object methods combine dynamic this with their own rest",
        source: r#"({base:40,method(left,...rest){
            return this.base+left+rest[0];
        }}).method(1,1)"#,
        expected: "return|number|42",
    },
    Case {
        description: "call and apply preserve identifier rest indexing",
        source: r#"(function(){
            function collect(left,...rest){return left+'|'+rest.join(',')}
            return collect.call(null,40,1,2)+';'+collect.apply(null,[40,1,2]);
        })()"#,
        expected: "return|string|40|1,2;40|1,2",
    },
    Case {
        description: "direct eval observes the initialized rest binding",
        source: r#"(function(...rest){return eval('rest[0]+rest[1]')})(40,2)"#,
        expected: "return|number|42",
    },
    Case {
        description: "Function constructor reuses ordinary rest parsing",
        source: "Function('left','...rest','return left+rest[0]+rest[1]')(40,1,1)",
        expected: "return|number|42",
    },
];

const ERROR_CASES: &[Case] = &[
    Case {
        description: "ordinary rest must be last",
        source: "function invalid(...rest,next){}",
        expected: "throw|object|SyntaxError|expecting ')'",
    },
    Case {
        description: "arrow rest rejects a trailing comma",
        source: "(...rest,)=>0",
        expected: "throw|object|SyntaxError|expecting ')'",
    },
    Case {
        description: "method rest rejects an initializer",
        source: "({method(...rest=[]) {}})",
        expected: "throw|object|SyntaxError|expecting ')'",
    },
    Case {
        description: "non simple ordinary parameters reject duplicates",
        source: "function invalid(value,value,...rest){}",
        expected: "throw|object|SyntaxError|duplicate argument names not allowed in this context",
    },
    Case {
        description: "non simple arrow use strict wins over duplicate",
        source: "(value,...value)=>{'use strict'}",
        expected: "throw|object|SyntaxError|\"use strict\" not allowed in function with default or destructuring parameter",
    },
    Case {
        description: "non simple method use strict wins over duplicate",
        source: "({method(value,...value){'use strict'}})",
        expected: "throw|object|SyntaxError|\"use strict\" not allowed in function with default or destructuring parameter",
    },
    Case {
        description: "getters cannot declare rest",
        source: "({get value(...rest){}})",
        expected: "throw|object|SyntaxError|invalid number of arguments for getter or setter",
    },
    Case {
        description: "setters cannot declare rest",
        source: "({set value(...rest){}})",
        expected: "throw|object|SyntaxError|invalid number of arguments for getter or setter",
    },
];

#[test]
fn identifier_rest_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP identifier-rest oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for case in CASES.iter().chain(ERROR_CASES) {
        assert_eq!(
            observe_oracle(&oracle, case.source, case.description),
            case.expected,
            "pinned QuickJS vector drifted for {}",
            case.description,
        );
    }
}

#[test]
fn identifier_rest_rust_smoke_runs_without_an_oracle() {
    for case in CASES.iter().chain(ERROR_CASES) {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust(&runtime, &mut context, case.source, case.description),
            case.expected,
            "Rust identifier-rest result drifted for {}",
            case.description,
        );
    }
}

#[test]
fn identifier_rest_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP identifier-rest differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for case in CASES.iter().chain(ERROR_CASES) {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust(&runtime, &mut context, case.source, case.description),
            observe_oracle(&oracle, case.source, case.description),
            "identifier-rest differential drifted for {}",
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
