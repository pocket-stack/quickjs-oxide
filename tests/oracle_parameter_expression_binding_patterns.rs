use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the synchronous QuickJS 2026-06-04 FormalParameters path where a
// standalone `=` token makes BindingPatterns participate in the independent
// Parameter Environment. Direct eval deliberately remains outside this oracle
// until the separate Parameter-Environment eval ABI lands.
const CASES: &[Case] = &[
    Case {
        description: "mixed defaults and BindingPatterns execute across callable surfaces",
        source: r#"(function(){
            var out=[];
            function ordinary([a]=[40],b=a+2){return b}
            out.push(ordinary());
            out.push((([a]=[40],b=a+2)=>b)());
            out.push(({method([a]=[40],b=a+2){return b}}).method());
            out.push(Function('[a]=[40]','b=a+2','return b')());
            return out.join('|');
        })()"#,
        expected: "return|string|42|42|42|42",
    },
    Case {
        description: "recursive leaf and whole-pattern defaults initialize in source order",
        source: r#"(function(){
            var calls=0;
            return (function({a:[b=40]}={a:[]},c=(calls++,b+2)){
                return b+'|'+c+'|'+calls;
            })();
        })()"#,
        expected: "return|string|40|42|1",
    },
    Case {
        description: "leaf defaults compose with a later identifier rest across callable surfaces",
        source: r#"(function(){
            function ordinary([a=1],...rest){return a+'|'+rest.join(',')}
            var arrow=([a=1],...rest)=>a+'|'+rest.join(',');
            var object={method([a=1],...rest){return a+'|'+rest.join(',')}};
            return ordinary([],2,3)+';'+arrow([],2,3)+';'+object.method([],2,3);
        })()"#,
        expected: "return|string|1|2,3;1|2,3;1|2,3",
    },
    Case {
        description: "whole-pattern defaults compose with a later identifier rest across callable surfaces",
        source: r#"(function(){
            function ordinary([a]=[1],...rest){return a+'|'+rest.join(',')}
            var arrow=([a]=[1],...rest)=>a+'|'+rest.join(',');
            var object={method([a]=[1],...rest){return a+'|'+rest.join(',')}};
            return ordinary(undefined,2,3)+';'+arrow(undefined,2,3)+';'+
                object.method(undefined,2,3);
        })()"#,
        expected: "return|string|1|2,3;1|2,3;1|2,3",
    },
    Case {
        description: "an assignment anywhere in the formal list selects the parameter-environment rest entry",
        source: r#"(function(){
            function later([a],b=1,...rest){return a+'|'+b+'|'+rest.join(',')}
            function earlier(a=0,[b],...rest){return a+'|'+b+'|'+rest.join(',')}
            function empty({},b=1,...rest){return b+'|'+rest.join(',')}
            return later([40],undefined,2,3)+';'+earlier(undefined,[40],2,3)+';'+
                empty({},undefined,2,3);
        })()"#,
        expected: "return|string|40|1|2,3;0|40|2,3;1|2,3",
    },
    Case {
        description: "a whole-pattern default resolves outside body var hoists",
        source: r#"(function(){
            var outer=42;
            return (function([a]=[outer],b=1){
                var outer=0;
                return a;
            })();
        })()"#,
        expected: "return|number|42",
    },
    Case {
        description: "a later nested default pre-scans the whole formal list",
        source: r#"(function(){
            var key='outer';
            return (function({[String(key)]:value},[x=1]){
                var key='body';
                return value;
            })({outer:42,undefined:1},[]);
        })()"#,
        expected: "return|number|42",
    },
    Case {
        description: "a pattern leaf cannot read a later parameter in its TDZ",
        source: r#"(function(){
            try { return (function([a=b],b=1){return a})([]) }
            catch(error) { return error.name }
        })()"#,
        expected: "return|string|ReferenceError",
    },
    Case {
        description: "a computed key cannot read a later parameter in its TDZ",
        source: r#"(function(){
            try { return (function({[b]:a},b=1){return a})({undefined:42}) }
            catch(error) { return error.name }
        })()"#,
        expected: "return|string|ReferenceError",
    },
    Case {
        description: "pattern BoundNames copy into the body after every initializer",
        source: "(function([a],b=(a=42)){return a})([1])",
        expected: "return|number|42",
    },
    Case {
        description: "an initializer closure retains the parameter cell after the body copy",
        source: r#"(function(){
            var read;
            return (function([a],b=(read=()=>a,0)){
                a=2;
                return a+'|'+read();
            })([1]);
        })()"#,
        expected: "return|string|2|1",
    },
    Case {
        description: "a leaf-default closure also retains the parameter cell",
        source: r#"(function(){
            var read;
            return (function([a=(read=()=>a,1)]){
                a=2;
                return a+'|'+read();
            })([]);
        })()"#,
        expected: "return|string|2|1",
    },
    Case {
        description: "parameter expressions keep sloppy arguments unmapped",
        source: r#"(function([a],b=0){
            arguments[0]=[9];
            a=7;
            return a+'|'+arguments[0][0];
        })([1])"#,
        expected: "return|string|7|9",
    },
    Case {
        description: "a zero-BoundName pattern still creates a parameter environment",
        source: r#"(function(){
            var source={};
            return (function({}=source){
                var source=null;
                return 42;
            })();
        })()"#,
        expected: "return|number|42",
    },
    Case {
        description: "function length distinguishes leaf defaults from whole-pattern defaults",
        source: r#"[
            (function([a=1],b){}).length,
            (function([a]=[1],b){}).length,
            (function(a,[b=1],c){}).length,
            (function(a,[b]=[1],c){}).length,
            (function(a,...[b=1]){}).length,
            (function(a=1,...[b]){}).length
        ].join('|')"#,
        expected: "return|string|2|0|3|1|2|0",
    },
    Case {
        description: "QuickJS accepts an unreachable default on a rest BindingPattern",
        source: r#"(function(){
            var f=function(...[a]=[99]){
                return String(a)+'|'+arguments.length;
            };
            return f.length+';'+f()+';'+f(42);
        })()"#,
        expected: "return|string|0;undefined|0;42|1",
    },
    Case {
        description: "setter defaults and getter rest BindingPatterns retain QuickJS arity quirks",
        source: r#"(function(){
            var value;
            var object={
                set item([a]=[1]){value=a},
                get read(...[a]){return a}
            };
            object.item=[42];
            var getter=Object.getOwnPropertyDescriptor(object,'read').get;
            return value+'|'+String(object.read)+'|'+getter.length;
        })()"#,
        expected: "return|string|42|undefined|1",
    },
];

const ERROR_CASES: &[Case] = &[
    Case {
        description: "a pattern BoundName cannot duplicate a defaulted identifier parameter",
        source: "function invalid([value],value=1){}",
        expected: "throw|object|SyntaxError|duplicate parameter names not allowed in this context",
    },
    Case {
        description: "a defaulted leaf cannot duplicate a BoundName in the same pattern",
        source: "function invalid([value,value=1]){}",
        expected: "throw|object|SyntaxError|duplicate parameter names not allowed in this context",
    },
    Case {
        description: "setter patterns apply duplicate-BoundName early errors",
        source: "({set item([value,value=1]){}})",
        expected: "throw|object|SyntaxError|duplicate parameter names not allowed in this context",
    },
];

#[test]
fn parameter_expression_binding_pattern_vectors_exclude_direct_eval() {
    for case in CASES.iter().chain(ERROR_CASES) {
        assert!(
            !case.source.contains("eval"),
            "direct eval escaped into the parameter-expression BindingPattern oracle: {}",
            case.description,
        );
    }
}

#[test]
fn parameter_expression_binding_pattern_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP parameter-expression BindingPattern oracle self-check: set QJS_ORACLE to upstream qjs"
        );
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
fn parameter_expression_binding_pattern_rust_smoke_runs_without_an_oracle() {
    for case in CASES.iter().chain(ERROR_CASES) {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust(&runtime, &mut context, case.source, case.description),
            case.expected,
            "Rust parameter-expression BindingPattern result drifted for {}",
            case.description,
        );
    }
}

#[test]
fn parameter_expression_binding_patterns_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP parameter-expression BindingPattern differential: set QJS_ORACLE to upstream qjs"
        );
        return;
    };
    for case in CASES.iter().chain(ERROR_CASES) {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust(&runtime, &mut context, case.source, case.description),
            observe_oracle(&oracle, case.source, case.description),
            "parameter-expression BindingPattern differential drifted for {}",
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
