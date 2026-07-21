use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the synchronous QuickJS 2026-06-04 FormalParameters path whose lexical
// pre-scan leaves SKIP_HAS_ASSIGNMENT clear. This includes recursive
// BindingPatterns and terminal rest BindingPatterns, but deliberately excludes
// every formal list containing a standalone `=` token. The latter creates
// QuickJS's independent Parameter Environment and is a separate ABI milestone.
const CASES: &[Case] = &[
    Case {
        description: "BindingPatterns execute across synchronous callable surfaces",
        source: r#"(function(){
            var out=[];
            function declaration([a],{b}){return a+b}
            out.push(declaration([40],{b:2}));
            out.push((function([a]){return a+2})([40]));
            out.push((([a])=>a+2)([40]));
            out.push(({base:40,method([a]){return this.base+a}}).method([2]));
            out.push(Function('[a]','return a+2')([40]));
            var assigned;
            var setter={set value([a]){assigned=a}};
            setter.value=[42];
            out.push(assigned);
            return out.join('|');
        })()"#,
        expected: "return|string|42|42|42|42|42|42",
    },
    Case {
        description: "recursive array and object patterns retain internal rest semantics",
        source: r#"(function([a,,[b,...tail]],{x:y,[String('z')]:z,...rest}){
            return a+'|'+b+'|'+tail.join(',')+'|'+y+'|'+z+'|'+rest.extra;
        })([1,0,[2,3,4]],{x:5,z:6,extra:7})"#,
        expected: "return|string|1|2|3,4|5|6|7",
    },
    Case {
        description: "a pattern parameter makes sloppy arguments unmapped",
        source: r#"(function([a]){
            arguments[0]=[9];
            a=7;
            return a+'|'+arguments[0][0];
        })([1])"#,
        expected: "return|string|7|9",
    },
    Case {
        description: "rest BindingPatterns preserve QuickJS physical-slot and length quirks",
        source: r#"(function(...[a,b]){
            return a+b+'|'+arguments.length+'|'+(function(...[x]){}).length+'|'+
                (function(x,...[y]){}).length;
        })(40,2)"#,
        expected: "return|string|42|2|1|2",
    },
    Case {
        description: "empty rest patterns retain QuickJS bytecode length publication quirks",
        source: r#"[
            (function(...[]){}).length,
            (function(...{}){}).length,
            (function(...[[]]){}).length,
            (function(...[,]){}).length,
            (function(...[a]){}).length,
            (function(...{a}){}).length,
            (function(...[]){var local}).length,
            (function(...[]){return arguments}).length,
            (function(...[]){return this}).length,
            (function(...[]){return new.target}).length,
            (function(...[]){return function(){return this}}).length,
            (function(...[]){return ()=>this}).length,
            (function named(...[]){return named}).length,
            (function named(...[]){}).length,
            (function(first,...[]){}).length
        ].join('|')"#,
        expected: "return|string|0|0|0|0|1|1|1|1|1|1|0|1|1|0|2",
    },
    Case {
        description: "an identifier rest may follow an ordinary BindingPattern",
        source: r#"(function mixed([a],...rest){
            return a+rest[0]+'|'+mixed.length+'|'+arguments.length;
        })([40],2)"#,
        expected: "return|string|42|1|2",
    },
    Case {
        description: "body var and function hoists occur after parameter destructuring",
        source: r#"(function(){
            var retained=(function([a]){var a;return a})([42]);
            var replaced=(function([a]){
                function a(){return 42}
                return typeof a+'|'+a();
            })([1]);
            return retained+';'+replaced;
        })()"#,
        expected: "return|string|42;function|42",
    },
    Case {
        description: "computed keys see root vars before body initialization",
        source: r#"(function(){
            var key='outer';
            var root=(function({[String(key)]:value}){
                var key='body';
                return value;
            })({undefined:42,outer:1});
            var lexical='outer';
            var body=(function({[lexical]:value}){
                let lexical='body';
                return value;
            })({outer:42});
            return root+'|'+body;
        })()"#,
        expected: "return|string|42|42",
    },
    Case {
        description: "computed-key closures retain the shared function-root binding",
        source: r#"(function(){
            var saved;
            function capture(fn){saved=fn;return 'undefined'}
            return (function({[capture(()=>key)]:value}){
                var key='body';
                return value+'|'+saved();
            })({undefined:42});
        })()"#,
        expected: "return|string|42|body",
    },
    Case {
        description: "direct eval observes initialized pattern BoundNames",
        source: "(function([a]){return eval('a')})([42])",
        expected: "return|number|42",
    },
    Case {
        description: "direct eval in a computed key runs before the body scope",
        source: r#"(function({[eval('"key"')]:value}){
            return value;
        })({key:42})"#,
        expected: "return|number|42",
    },
    Case {
        description: "a pattern BoundName overwrites arguments only after computed keys",
        source: r#"(function({[arguments]:arguments}){
            return arguments;
        })({undefined:1,"[object Arguments]":42})"#,
        expected: "return|number|42",
    },
    Case {
        description: "nested-arrow eval retains the missing parent body segment",
        source: r#"(function({[(()=>eval("typeof key"))()]:value}){
            var key="body";
            return value;
        })({undefined:42})"#,
        expected: "return|number|42",
    },
    Case {
        description: "an arrow pattern preserves lexical arguments",
        source: "(function(){return (([a])=>a+arguments[1])([40])})(0,2)",
        expected: "return|number|42",
    },
    Case {
        description: "compound assignment does not set QuickJS SKIP_HAS_ASSIGNMENT",
        source: r#"(function(){
            var key=0;
            return (function({[(key+=1)]:value}){return value})({1:42});
        })()"#,
        expected: "return|number|42",
    },
    Case {
        description: "parameter array destructuring performs iterator close",
        source: r#"(function(){
            var closed=0;
            var iterable={
                [Symbol.iterator](){
                    var next=0;
                    return {
                        next(){next++;return next===1?{value:42,done:false}:{done:true}},
                        return(){closed++;return {done:true}}
                    };
                }
            };
            return (function([a]){return a})(iterable)+'|'+closed;
        })()"#,
        expected: "return|string|42|1",
    },
    Case {
        description: "ordinary methods preserve HomeObject with pattern parameters",
        source: r#"(function(){
            var base={value(){return 40}};
            var object={__proto__:base,value([a]){return super.value()+a}};
            return object.value([2]);
        })()"#,
        expected: "return|number|42",
    },
];

const ERROR_CASES: &[Case] = &[
    Case {
        description: "a BoundName cannot duplicate another formal parameter",
        source: "function invalid([value],value){}",
        expected: "throw|object|SyntaxError|duplicate argument names not allowed in this context",
    },
    Case {
        description: "duplicate BoundNames inside a pattern are rejected",
        source: "function invalid([value,value]){}",
        expected: "throw|object|SyntaxError|duplicate parameter names not allowed in this context",
    },
    Case {
        description: "a strict directive is forbidden with non-simple formals",
        source: "function invalid([value]){'use strict'}",
        expected: "throw|object|SyntaxError|\"use strict\" not allowed in function with default or destructuring parameter",
    },
    Case {
        description: "a rest BindingPattern rejects a trailing comma",
        source: "(...[value],)=>0",
        expected: "throw|object|SyntaxError|expecting ')'",
    },
];

#[test]
fn parameter_binding_pattern_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP parameter-BindingPattern oracle self-check: set QJS_ORACLE to upstream qjs"
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
fn parameter_binding_pattern_rust_smoke_runs_without_an_oracle() {
    for case in CASES.iter().chain(ERROR_CASES) {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust(&runtime, &mut context, case.source, case.description),
            case.expected,
            "Rust parameter-BindingPattern result drifted for {}",
            case.description,
        );
    }
}

#[test]
fn parameter_binding_patterns_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP parameter-BindingPattern differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for case in CASES.iter().chain(ERROR_CASES) {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust(&runtime, &mut context, case.source, case.description),
            observe_oracle(&oracle, case.source, case.description),
            "parameter-BindingPattern differential drifted for {}",
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
