use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Pins the complete synchronous AggregateError slice implemented by QuickJS
// 2026-06-04. The cases deliberately observe ordering and abrupt completion,
// not just final values: those are the places where a superficially working
// constructor most easily diverges from QuickJS and ECMA-262.
const CASES: &[Case] = &[
    Case {
        group: "intrinsic graph",
        description: "constructor and prototype graph",
        source: r#"(function(){var C=AggregateError,p=C.prototype;return [typeof C,C.name,C.length,Object.getPrototypeOf(C)===Error,Object.getPrototypeOf(p)===Error.prototype,p.constructor===C,p.name,p.message,Object.hasOwn(p,"errors")].join("|")})()"#,
        expected: "return|string|function|AggregateError|2|true|true|true|AggregateError||false",
    },
    Case {
        group: "intrinsic graph",
        description: "prototype and instance property descriptors and definition order",
        source: r#"(function(){function bits(d){return Number(d.writable)+","+Number(d.enumerable)+","+Number(d.configurable)}var e=new AggregateError([],"m",{cause:7}),p=AggregateError.prototype;return [bits(Object.getOwnPropertyDescriptor(AggregateError,"prototype")),bits(Object.getOwnPropertyDescriptor(p,"constructor")),bits(Object.getOwnPropertyDescriptor(p,"name")),bits(Object.getOwnPropertyDescriptor(p,"message")),bits(Object.getOwnPropertyDescriptor(e,"message")),bits(Object.getOwnPropertyDescriptor(e,"cause")),bits(Object.getOwnPropertyDescriptor(e,"errors")),Reflect.ownKeys(e).join(",")].join("|")})()"#,
        expected: "return|string|0,0,0|1,0,1|1,0,1|1,0,1|1,0,1|1,0,1|1,0,1|message,cause,errors,stack",
    },
    Case {
        group: "call and construct",
        description: "call and construct both create branded AggregateError instances",
        source: r#"(function(){var holder={},a=AggregateError.call(holder,[],"a"),b=new AggregateError([],"b");return [a!==holder,a instanceof AggregateError,b instanceof AggregateError,Object.getPrototypeOf(a)===AggregateError.prototype,Object.getPrototypeOf(b)===AggregateError.prototype,a.message,b.message].join("|")})()"#,
        expected: "return|string|true|true|true|true|true|a|b",
    },
    Case {
        group: "call and construct",
        description: "newTarget custom prototype and intrinsic fallback",
        source: r#"(function(){function F(){}var custom={marker:42};F.prototype=custom;var a=Reflect.construct(AggregateError,[[],"a"],F);F.prototype=1;var b=Reflect.construct(AggregateError,[[],"b"],F);return [Object.getPrototypeOf(a)===custom,a.marker,Object.getPrototypeOf(b)===AggregateError.prototype,a instanceof AggregateError,b instanceof AggregateError].join("|")})()"#,
        expected: "return|string|true|42|true|false|true",
    },
    Case {
        group: "message and cause",
        description: "undefined message is absent while explicit undefined cause is present",
        source: r#"(function(){var a=AggregateError([]),b=AggregateError([],undefined,{cause:undefined});return [Object.hasOwn(a,"message"),Object.hasOwn(a,"cause"),Object.hasOwn(a,"errors"),Object.hasOwn(b,"message"),Object.hasOwn(b,"cause"),String(b.cause),Object.hasOwn(b,"errors")].join("|")})()"#,
        expected: "return|string|false|false|true|false|true|undefined|true",
    },
    Case {
        group: "message and cause",
        description: "message and cause precede iterator acquisition",
        source: r#"(function(){var log=[],errors={};errors[Symbol.iterator]=function(){log.push("iterator-method");var iterator={};Object.defineProperty(iterator,"next",{get:function(){log.push("next-get");return function(){log.push("next-call");return {done:true}}}});return iterator};var message={toString:function(){log.push("message");return "m"}},options={};Object.defineProperty(options,"cause",{get:function(){log.push("cause");return 9}});var e=AggregateError(errors,message,options);return [log.join(","),e.message,e.cause,e.errors.length].join("|")})()"#,
        expected: "return|string|message,cause,iterator-method,next-get,next-call|m|9|0",
    },
    Case {
        group: "message and cause",
        description: "argument evaluation precedes constructor-side coercion and lookup",
        source: r#"(function(){var log=[];function arg(v,n){log.push(n);return v}var iterable={};iterable[Symbol.iterator]=function(){return [][Symbol.iterator]()};AggregateError(arg(iterable,"errors-arg"),arg({toString:function(){log.push("message-convert");return "m"}},"message-arg"),arg({get cause(){log.push("cause-get");return 1}},"options-arg"));return log.join(",")})()"#,
        expected: "return|string|errors-arg,message-arg,options-arg,message-convert,cause-get",
    },
    Case {
        group: "message and cause",
        description: "message coercion abrupt completion suppresses cause and iteration",
        source: r#"(function(){var log=[],errors={};errors[Symbol.iterator]=function(){log.push("iterator");return [][Symbol.iterator]()};var message={toString:function(){log.push("message");throw "message-error"}},options={};Object.defineProperty(options,"cause",{get:function(){log.push("cause");return 1}});try{AggregateError(errors,message,options)}catch(e){return [e,log.join(",")].join("|")}})()"#,
        expected: "return|string|message-error|message",
    },
    Case {
        group: "message and cause",
        description: "cause lookup abrupt completion suppresses iteration",
        source: r#"(function(){var log=[],errors={};errors[Symbol.iterator]=function(){log.push("iterator");return [][Symbol.iterator]()};var options={};Object.defineProperty(options,"cause",{get:function(){log.push("cause");throw "cause-error"}});try{AggregateError(errors,"m",options)}catch(e){return [e,log.join(",")].join("|")}})()"#,
        expected: "return|string|cause-error|cause",
    },
    Case {
        group: "message and cause",
        description: "ordinary and native Error constructors share cause descriptors",
        source: r#"(function(){var constructors=[Error,EvalError,RangeError,ReferenceError,SyntaxError,TypeError,URIError],out=[];for(var i=0;i<constructors.length;i++){var e=constructors[i]("m",{cause:7}),d=Object.getOwnPropertyDescriptor(e,"cause");out.push(e.name+":"+e.cause+":"+Number(d.writable)+Number(d.enumerable)+Number(d.configurable))}return out.join(",")})()"#,
        expected: "return|string|Error:7:101,EvalError:7:101,RangeError:7:101,ReferenceError:7:101,SyntaxError:7:101,TypeError:7:101,URIError:7:101",
    },
    Case {
        group: "errors array",
        description: "an Array input is copied into a genuine Array",
        source: r#"(function(){var source=[1,2],e=AggregateError(source);source[0]=9;return [Array.isArray(e.errors),Object.getPrototypeOf(e.errors)===Array.prototype,e.errors!==source,e.errors.length,e.errors[0],e.errors[1],Object.hasOwn(e.errors,"0")].join("|")})()"#,
        expected: "return|string|true|true|true|2|1|2|true",
    },
    Case {
        group: "errors array",
        description: "a custom iterable is materialized in iterator order",
        source: r#"(function(){var errors={};errors[Symbol.iterator]=function(){var i=0;return {next:function(){i++;if(i===1)return {value:3,done:false};if(i===2)return {value:4,done:false};return {done:true}}}};var e=AggregateError(errors);return [Array.isArray(e.errors),e.errors.join(","),e.errors.length].join("|")})()"#,
        expected: "return|string|true|3,4|2",
    },
    Case {
        group: "iterator completion",
        description: "normal iterator completion does not call return",
        source: r#"(function(){var log=[],errors={};errors[Symbol.iterator]=function(){var done=false;return {next:function(){log.push("next");if(done)return {done:true};done=true;return {value:1,done:false}},return:function(){log.push("return");return {done:true}}}};var e=AggregateError(errors);return [e.errors.join(","),log.join(",")].join("|")})()"#,
        expected: "return|string|1|next,next",
    },
    Case {
        group: "iterator completion",
        description: "abrupt next getter does not close an incompletely acquired iterator record",
        source: r#"(function(){var log=[],errors={};errors[Symbol.iterator]=function(){log.push("iterator");var iterator={return:function(){log.push("return");return {done:true}}};Object.defineProperty(iterator,"next",{get:function(){log.push("next-get");throw "next-get-error"}});return iterator};try{AggregateError(errors)}catch(e){return [e,log.join(",")].join("|")}})()"#,
        expected: "return|string|next-get-error|iterator,next-get",
    },
    Case {
        group: "iterator completion",
        description: "abrupt next closes while preserving the original throw",
        source: r#"(function(){var log=[],errors={};errors[Symbol.iterator]=function(){return {next:function(){log.push("next");throw "next-error"},return:function(){log.push("return");throw "close-error"}}};try{AggregateError(errors)}catch(e){return [e,log.join(",")].join("|")}})()"#,
        expected: "return|string|next-error|next,return",
    },
    Case {
        group: "iterator completion",
        description: "abrupt done lookup closes the iterator",
        source: r#"(function(){var log=[],errors={};errors[Symbol.iterator]=function(){return {next:function(){var result={value:1};Object.defineProperty(result,"done",{get:function(){log.push("done");throw "done-error"}});return result},return:function(){log.push("return");return {done:true}}}};try{AggregateError(errors)}catch(e){return [e,log.join(",")].join("|")}})()"#,
        expected: "return|string|done-error|done,return",
    },
    Case {
        group: "iterator completion",
        description: "abrupt value lookup closes the iterator",
        source: r#"(function(){var log=[],errors={};errors[Symbol.iterator]=function(){return {next:function(){var result={done:false};Object.defineProperty(result,"value",{get:function(){log.push("value");throw "value-error"}});return result},return:function(){log.push("return");return {done:true}}}};try{AggregateError(errors)}catch(e){return [e,log.join(",")].join("|")}})()"#,
        expected: "return|string|value-error|value,return",
    },
    Case {
        group: "iterator completion",
        description: "abrupt return lookup cannot replace the pending iterator throw",
        source: r#"(function(){var log=[],errors={};errors[Symbol.iterator]=function(){var iterator={next:function(){throw "body-error"}};Object.defineProperty(iterator,"return",{get:function(){log.push("return-get");throw "close-get-error"}});return iterator};try{AggregateError(errors)}catch(e){return [e,log.join(",")].join("|")}})()"#,
        expected: "return|string|body-error|return-get",
    },
    Case {
        group: "branding and stack",
        description: "Error branding is unforgeable and stack capture skips the native constructor",
        source: r#"(function(){function make(){return AggregateError([],"boom")}var e=make(),stack=String(e.stack);return [Error.isError(e),Error.isError(AggregateError.prototype),Error.isError(Object.create(AggregateError.prototype)),String(e),typeof e.stack,stack.indexOf("make")>=0,stack.indexOf("at AggregateError")>=0].join("|")})()"#,
        expected: "return|string|true|false|false|AggregateError: boom|string|true|false",
    },
];

#[test]
fn aggregate_error_matches_pinned_expectations() {
    let mut failures = Vec::new();
    for case in CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let actual = observe_rust(&runtime, &mut context, case.source, case.description);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\nactual: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "AggregateError pinned expectations failed in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn aggregate_error_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP AggregateError oracle self-check: set QJS_ORACLE to pinned upstream qjs");
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
        "pinned QuickJS AggregateError vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn aggregate_error_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP AggregateError differential: set QJS_ORACLE to pinned upstream qjs");
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
        "AggregateError semantics drifted in {} case(s):\n\n{}",
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
