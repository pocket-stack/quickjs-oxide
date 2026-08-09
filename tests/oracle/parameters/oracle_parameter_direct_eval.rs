use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins QuickJS 2026-06-04's separate parameter lexical environment and the
// hidden sloppy-direct-eval variable objects conventionally named `<arg_var>`
// and `<var>` in upstream bytecode dumps. These probes intentionally exercise
// authored identifier lookup as well as eval declaration instantiation: either
// half in isolation misses observable ordering and closure-capture behavior.
const CASES: &[Case] = &[
    Case {
        group: "parameter target",
        description: "a parameter eval var is visible to later parameters and authored body lookup",
        source: r#"(function(a=eval("var x=1"),b=x,c=()=>x){return [b,x,c()].join("|")})()"#,
        expected: "return|string|1|1|1",
    },
    Case {
        group: "parameter target",
        description: "a parameter eval function declaration is hoisted into the argument variable object",
        source: r#"(function(a=eval("var r=x();function x(){return 42};r"),b=x()){return [a,b,x()].join("|")})()"#,
        expected: "return|string|42|42|42",
    },
    Case {
        group: "parameter target",
        description: "a sloppy eval block function receives its Annex B argument-variable alias",
        source: r#"(function(a=eval("{function x(){return 42}}"),b=typeof x,c=x()){return [b,c,x()].join("|")})()"#,
        expected: "return|string|function|42|42",
    },
    Case {
        group: "parameter target",
        description: "an eval var cannot redeclare an earlier parameter lexical cell",
        source: r#"(function(){var touch=0;try{(function(a=eval("touch=1;var a")){})()}catch(e){return e.name+"|"+touch}return "none"})()"#,
        expected: "return|string|SyntaxError|0",
    },
    Case {
        group: "parameter target",
        description: "an eval var cannot redeclare a later parameter lexical cell",
        source: r#"(function(){var touch=0;try{(function(a=eval("touch=1;var b"),b=1){})()}catch(e){return e.name+"|"+touch}return "none"})()"#,
        expected: "return|string|SyntaxError|0",
    },
    Case {
        group: "parameter target",
        description: "an eval function cannot redeclare a later parameter lexical cell",
        source: r#"(function(){var touch=0;try{(function(a=eval("touch=1;function b(){}"),b=1){})()}catch(e){return e.name+"|"+touch}return "none"})()"#,
        expected: "return|string|SyntaxError|0",
    },
    Case {
        group: "parameter target",
        description: "an eval assignment mutates the parameter cell while the body keeps its copied argument",
        source: r#"(function(a=1,b=eval("a=7"),c=a,d=()=>a){return [c,d(),a].join("|")})()"#,
        expected: "return|string|7|7|1",
    },
    Case {
        group: "parameter target",
        description: "a default anywhere moves a preceding plain formal into the parameter lexical environment",
        source: r#"(function(a,b=eval("a=7"),c=a,d=()=>a){return [c,d(),a].join("|")})(1)"#,
        expected: "return|string|7|7|1",
    },
    Case {
        group: "parameter target",
        description: "a BindingPattern copies its final parameter cell value into the body environment",
        source: r#"(function({x:a=1}={},b=eval("a=7"),c=a,d=()=>a){return [c,d(),a].join("|")})()"#,
        expected: "return|string|7|7|7",
    },
    Case {
        group: "parameter target",
        description: "an eval assignment to a later parameter observes its temporal dead zone",
        source: r#"(function(){try{return (function(a=eval("b=7"),b=1){return "none"})()}catch(e){return e.name}})()"#,
        expected: "return|string|ReferenceError",
    },
    Case {
        group: "parameter target",
        description: "a closure created by parameter eval captures the parameter cell rather than the body copy",
        source: r#"(function(a=1,b=eval("var read=()=>a")){a=2;return [read(),a].join("|")})()"#,
        expected: "return|string|1|2",
    },
    Case {
        group: "parameter target",
        description: "eval dynamic lookup precedes the private function name but authored lookup keeps the private name",
        source: r#"(function(){return (function f(a=eval("var f=7"),b=eval("f"),c=()=>f){return [b,typeof c(),typeof f].join("|")})()})()"#,
        expected: "return|string|7|function|function",
    },
    Case {
        group: "parameter target",
        description: "an eval lexical declaration remains local to the eval invocation",
        source: r#"(function(a=eval("let x=1;x"),b=typeof x){return [a,b,typeof x].join("|")})()"#,
        expected: "return|string|1|undefined|undefined",
    },
    Case {
        group: "environment split",
        description: "body eval creates a body variable that shadows the argument eval variable",
        source: r#"(function(a=eval("var x=1"),b=()=>x){eval("var x=2");return [x,b()].join("|")})()"#,
        expected: "return|string|2|1",
    },
    Case {
        group: "environment split",
        description: "an authored body var shadows an argument eval variable",
        source: r#"(function(a=eval("var x=1"),b=()=>x){var x=2;return [x,b()].join("|")})()"#,
        expected: "return|string|2|1",
    },
    Case {
        group: "environment split",
        description: "an authored body function shadows an argument eval variable",
        source: r#"(function(a=eval("var x=1"),b=()=>x){function x(){return 2}return [x(),b()].join("|")})()"#,
        expected: "return|string|2|1",
    },
    Case {
        group: "environment split",
        description: "nested parameter eval reuses the argument variable object",
        source: r#"(function(a=eval("var x=1;eval('x=2;var y=3')"),b=()=>x+y){return [x,y,b()].join("|")})()"#,
        expected: "return|string|2|3|5",
    },
    Case {
        group: "environment split",
        description: "nested body eval targets the body variable object before the argument variable object",
        source: r#"(function(a=eval("var x=1"),b=()=>x){eval("eval('var x=2')");return [x,b()].join("|")})()"#,
        expected: "return|string|2|1",
    },
    Case {
        group: "environment split",
        description: "body eval updates exact body bindings without overwriting the argument variable object",
        source: r#"(function(a=1,b=eval("var x=9"),c=()=>x){eval("a=2;var x=3");return [a,x,c()].join("|")})()"#,
        expected: "return|string|2|3|9",
    },
    Case {
        group: "environment split",
        description: "a function created by body eval captures the body binding",
        source: r#"(function(a=eval("var x=9"),b=()=>x){var x=2;eval("var f=()=>x");x=3;return [f(),b()].join("|")})()"#,
        expected: "return|string|3|9",
    },
    Case {
        group: "dynamic object",
        description: "deleting an argument eval var removes authored and later-parameter visibility",
        source: r#"(function(a=eval("var x=1;delete x"),b=typeof x){return [a,b,typeof x].join("|")})()"#,
        expected: "return|string|true|undefined|undefined",
    },
    Case {
        group: "dynamic object",
        description: "an eval function closure survives deletion of its dynamic binding",
        source: r#"(function(){var saved;return (function(a=eval("function x(){return 42};saved=x;delete x"),b=typeof x){return [b,saved()].join("|")})()})()"#,
        expected: "return|string|undefined|42",
    },
    Case {
        group: "dynamic object",
        description: "a repeated object-backed var declaration resets the property to undefined",
        source: r#"(function(a=eval("var x=1"),before=x,reset=eval("var x"),after=typeof x){return [before,typeof reset,after,typeof x].join("|")})()"#,
        expected: "return|string|1|undefined|undefined|undefined",
    },
    Case {
        group: "dynamic object",
        description: "closures relay the argument variable object after its creating frame exits",
        source: r#"(function(){var pair=(function(a=eval("var x=1"),get=()=>x,set=v=>x=v){return [get,set]})();var before=pair[0]();pair[1](2);return [before,pair[0]()].join("|")})()"#,
        expected: "return|string|1|2",
    },
    Case {
        group: "arguments",
        description: "the parameter and body share one unmapped arguments object",
        source: r#"(function(a=1,b=(eval("0"),arguments[0]=9,a),c=arguments[0]){return [a,b,c,arguments[0]].join("|")})(undefined)"#,
        expected: "return|string|1|1|9|9",
    },
    Case {
        group: "arguments",
        description: "a BindingPattern initializer reads the synthetic parameter arguments cell",
        source: r#"(function({x=arguments[0].answer}={},a=eval("0")){return x})({x:undefined,answer:42})"#,
        expected: "return|number|42",
    },
    Case {
        group: "arguments",
        description: "a BindingPattern initializer closure captures the synthetic parameter arguments cell",
        source: r#"(function({x=()=>arguments[0].answer}={},a=eval("0")){return x()})({x:undefined,answer:42})"#,
        expected: "return|number|42",
    },
    Case {
        group: "arguments",
        description: "an eval var cannot redeclare the synthetic parameter arguments cell",
        source: r#"(function(){try{(function(a=eval("var arguments")){})()}catch(e){return e.name}return "none"})()"#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "arguments",
        description: "an arrow parameter eval may create arguments in the argument variable object",
        source: r#"(function(){return ((a=eval("var arguments=7"),b=arguments)=>[b,arguments].join("|"))()})()"#,
        expected: "return|string|7|7",
    },
    Case {
        group: "arguments",
        description: "inherited strict parameter eval does not receive QuickJS's implicit arguments binding",
        source: r#"(function(){"use strict";return (function(a=eval("typeof arguments"),b=eval("try{arguments;'none'}catch(e){e.name}")){return [a,b].join("|")})()})()"#,
        expected: "return|string|undefined|ReferenceError",
    },
    Case {
        group: "arguments",
        description: "a named arguments parameter remains in parameter scope while body lookup sees the implicit object",
        source: r#"(function(arguments=1,a=eval("0"),b=arguments){return [b,typeof arguments,arguments===b].join("|")})()"#,
        expected: "return|string|1|object|false",
    },
    Case {
        group: "arguments",
        description: "an eval var cannot redeclare a named arguments parameter",
        source: r#"(function(){try{(function(arguments=1,a=eval("var arguments")){})()}catch(e){return e.name}return "none"})()"#,
        expected: "return|string|SyntaxError",
    },
    Case {
        group: "arguments",
        description: "an authored body var named arguments precedes the body eval variable object",
        source: r#"(function(a=eval("0")){var arguments;eval("var arguments=7");return [typeof arguments,String(arguments)].join("|")})()"#,
        expected: "return|string|number|7",
    },
    Case {
        group: "arguments",
        description: "an authored body function named arguments precedes the body eval variable object",
        source: r#"(function(a=eval("0")){function arguments(){return 3}eval("var arguments=7");return [typeof arguments,String(arguments)].join("|")})()"#,
        expected: "return|string|number|7",
    },
    Case {
        group: "arguments",
        description: "a descendant arrow eval does not synthesize arguments in the outer parameter environment",
        source: r#"(function(a=(()=>eval("typeof arguments"))()){return a})()"#,
        expected: "return|string|undefined",
    },
    Case {
        group: "arguments",
        description: "an authored arguments capture is appended to the caller closure suffix seen by eval",
        source: r#"(function(a=(()=>[eval("typeof arguments"),arguments.length].join("|"))()){return a})(undefined)"#,
        expected: "return|string|object|1",
    },
    Case {
        group: "arguments",
        description: "a descendant arrow arguments capture is appended to the caller closure suffix seen by eval",
        source: r#"(function(a=(()=>{var g=()=>arguments;return eval("typeof arguments")})()){return a})(undefined)"#,
        expected: "return|string|object",
    },
    Case {
        group: "entry ordering",
        description: "a script pseudo-binding prologue precedes an earlier global function hoist",
        source: r#"function f(){};var a=(p=eval("0"))=>p;a()"#,
        expected: "return|number|0",
    },
    Case {
        group: "scope switch",
        description: "a computed parameter key without any default remains in the function variable scope",
        source: r#"(function({[eval("var x=7")]:a}){var x;return [typeof x,x].join("|")})({7:0})"#,
        expected: "return|string|number|7",
    },
    Case {
        group: "scope switch",
        description: "any default switches a computed parameter eval to the separate argument variable object",
        source: r#"(function({[eval("var x=7")]:a},b=0,c=x){var x;return [c,typeof x].join("|")})({7:0})"#,
        expected: "return|string|7|undefined",
    },
    Case {
        group: "strict eval",
        description: "a strict eval source mutates imported parameters but does not leak its var declaration",
        source: r#"(function(a=1,b=eval("'use strict';a=2;var x=3;typeof x"),c=a){return [b,c,a,typeof x].join("|")})()"#,
        expected: "return|string|number|2|1|undefined",
    },
    Case {
        group: "strict eval",
        description: "direct eval inherits strictness from the containing function bytecode",
        source: r#"(function(){"use strict";return (function(a=1,b=eval("a=2;var x=3;typeof x"),c=a){return [b,c,a,typeof x].join("|")})()})()"#,
        expected: "return|string|number|2|1|undefined",
    },
];

#[test]
fn parameter_direct_eval_oracle_inventory_is_stable() {
    assert_eq!(CASES.len(), 42, "update the reviewed oracle case count");
    for (index, case) in CASES.iter().enumerate() {
        assert!(
            case.source.contains("eval"),
            "case lacks eval: {}",
            case.description
        );
        assert!(
            CASES[..index]
                .iter()
                .all(|earlier| earlier.description != case.description),
            "duplicate case description: {}",
            case.description,
        );
    }
}

#[test]
fn parameter_direct_eval_matches_pinned_expectations() {
    let mut failures = Vec::new();
    for case in CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let actual = observe_rust(&runtime, &mut context, case.source, case.description);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\noxide: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "parameter-direct-eval pinned expectations failed in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn parameter_direct_eval_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP parameter-direct-eval oracle self-check: set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };
    let mut failures = Vec::new();
    for case in CASES {
        let actual = observe_oracle(&oracle, case.source, case.description);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\nactual: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "pinned QuickJS parameter-direct-eval vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn parameter_direct_eval_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP parameter-direct-eval differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for case in CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let oxide = observe_rust(&runtime, &mut context, case.source, case.description);
        let quickjs = observe_oracle(&oracle, case.source, case.description);
        if oxide != quickjs {
            failures.push(format!(
                "{} / {}\nsource: {:?}\noxide: {:?}\nquickjs: {:?}",
                case.group, case.description, case.source, oxide, quickjs,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "parameter-direct-eval semantics drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
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
        Err(error) => format!("engine|{error}"),
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
