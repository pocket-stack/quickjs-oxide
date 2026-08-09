use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, Runtime,
    RuntimeError, Value,
};

// This target pins the non-allocating modes of QuickJS 2026-06-04's shared
// `js_array_every` kernel: every, some, and forEach.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "every returns true or short-circuits false",
        r#"(function(){
            var log="",source=[2,4,5,8];
            var first=source.every(function(value,index){log+="a"+index;return value%2===0});
            var second=[2,4,8].every(function(value,index){log+="b"+index;return value%2===0});
            return first+"|"+second+"|"+log;
        })()"#,
    ),
    (
        "some returns true or exhausts to false",
        r#"(function(){
            var log="",source=[1,3,4,6];
            var first=source.some(function(value,index){log+="a"+index;return value%2===0});
            var second=[1,3].some(function(value,index){log+="b"+index;return value%2===0});
            return first+"|"+second+"|"+log;
        })()"#,
    ),
    (
        "forEach visits all present elements and returns undefined",
        r#"(function(){
            var log="",source=[3,4,5];
            var result=source.forEach(function(value,index){log+=index+":"+value+",";return true});
            return (result===undefined)+"|"+log;
        })()"#,
    ),
    (
        "empty receivers use vacuous results without callbacks",
        r#"(function(){
            var calls=0,callback=function(){calls++;return true};
            return [].every(callback)+"|"+[].some(callback)+"|"+
                ([].forEach(callback)===undefined)+"|"+calls;
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "holes are skipped while inherited values are visited",
        r#"(function(){
            var log="",proto=Object(),source=Object();proto[1]="p";
            Object.setPrototypeOf(source,proto);source[2]="z";source.length=4;
            source.forEach(function(value,index){log+=index+":"+value+","});
            return log;
        })()"#,
    ),
    (
        "length is snapshotted and later HasProperty observes mutation",
        r#"(function(){
            var source=[0,1,2],log="";
            source.forEach(function(value,index){
                log+=index+":"+value+",";
                if(index===0){delete source[1];source[3]=3;source.length=4}
            });
            return log+"|"+source.length+"|"+source[3];
        })()"#,
    ),
    (
        "callback value index boxed receiver and strict thisArg are exact",
        r#"(function(){
            var marker=Object(),source="ab",log="";
            var result=Array.prototype.every.call(source,function(value,index,receiver){
                "use strict";
                log+=(this===marker)+":"+value+":"+index+":"+
                    (typeof receiver)+":"+Object.prototype.toString.call(receiver)+":"+
                    (receiver===source)+",";
                return true;
            },marker);
            return result+"|"+log;
        })()"#,
    ),
    (
        "every and some short-circuit before a later getter",
        r#"(function(){
            var first=[0,1],second=[1,0],log="";
            first.__defineGetter__("1",function(){log+="E";throw 61});
            second.__defineGetter__("1",function(){log+="S";throw 62});
            var every=first.every(function(value,index){log+="e"+index;return false});
            var some=second.some(function(value,index){log+="s"+index;return true});
            return every+"|"+some+"|"+log;
        })()"#,
    ),
    (
        "callback result uses ToBoolean without invoking object hooks",
        r#"(function(){
            var log="",truthy=Object();
            truthy.valueOf=function(){log+="v";throw 63};
            truthy.toString=function(){log+="s";throw 64};
            var every=[1].every(function(){return truthy});
            var some=[1].some(function(){return truthy});
            var count=0;[1,2].forEach(function(){count++;return truthy});
            return every+"|"+some+"|"+count+"|"+log;
        })()"#,
    ),
    (
        "length precedes callback validation even for an empty receiver",
        r#"(function(){
            var log="",source=Object();
            source.__defineGetter__("length",function(){log+="L";return 0});
            try{Array.prototype.every.call(source,7);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "a getter throw precedes callback invocation",
        r#"(function(){
            var log="",source=Object();source.length=1;
            source.__defineGetter__("0",function(){log+="G";throw 71});
            try{Array.prototype.some.call(source,function(){log+="C";return true});return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log}
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like receivers pass their object identity",
        r#"(function(){
            var source=Object();source[0]="a";source[2]="c";source.length=3;
            var log="",result=Array.prototype.forEach.call(source,function(value,index,receiver){
                log+=(receiver===source)+":"+index+":"+value+",";
            });
            return (result===undefined)+"|"+log;
        })()"#,
    ),
    (
        "String receiver iterates UTF-16 units through a boxed callback receiver",
        r#"(function(){
            var source="A\uD83D\uDCA9Z",sum=0,boxed=true;
            var result=Array.prototype.every.call(source,function(value,index,receiver){
                sum+=value.charCodeAt(0);boxed=boxed&&typeof receiver==="object";
                return index<3;
            });
            return result+"|"+sum+"|"+boxed;
        })()"#,
    ),
    (
        "zero-length primitive receivers still validate callbacks",
        r#"(function(){
            var calls=0,callback=function(){calls++;return true};
            var number=Array.prototype.every.call(7,callback);
            var boolean=Array.prototype.some.call(false,callback);
            return number+"|"+boolean+"|"+calls;
        })()"#,
    ),
    (
        "MAX_SAFE_INTEGER length can short-circuit at index zero",
        r#"(function(){
            var source=Object(),calls=0;source.length=9007199254740991;source[0]="x";
            var every=Array.prototype.every.call(source,function(value,index,receiver){
                calls++;return !(receiver===source&&index===0&&value==="x");
            });
            var some=Array.prototype.some.call(source,function(value,index,receiver){
                calls++;return receiver===source&&index===0&&value==="x";
            });
            return every+"|"+some+"|"+calls;
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "null receiver",
        "Array.prototype.every.call(null,function(){return true})",
    ),
    (
        "undefined receiver",
        "Array.prototype.some.call(undefined,function(){return true})",
    ),
    ("missing callback", "[].forEach()"),
    ("undefined callback", "[].every(undefined)"),
    ("number callback", "[].some(1)"),
    ("symbol callback", "[].forEach(Symbol('callback'))"),
    ("BigInt callback", "[].every(0n)"),
    (
        "Symbol length wins before callback validation",
        "(function(){var source=Object();source.length=Symbol('length');return Array.prototype.some.call(source,1)})()",
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','every','some','forEach','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','copyWithin','values','keys','entries'];
var own=Reflect.ownKeys(Array.prototype),names=[];
for(var i=0;i<own.length;i++)
  if(implemented.indexOf(own[i])>=0)names[names.length]=own[i];
function bits(descriptor) {
  return 'D'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
function metadata(name) {
  var descriptor=Object.getOwnPropertyDescriptor(Array.prototype,name),fn=descriptor.value;
  var constructable;
  try { Reflect.construct(function(){},[],fn); constructable=true; }
  catch(error) { constructable=false; }
  return name+':'+fn.name+':'+fn.length+':'+bits(descriptor)+':'+
    bits(Object.getOwnPropertyDescriptor(fn,'name'))+':'+
    bits(Object.getOwnPropertyDescriptor(fn,'length'))+':'+
    (typeof fn==='function')+':'+(Object.getPrototypeOf(fn)===Function.prototype)+':'+constructable;
}
print('keys='+names.join(','));
print('meta='+metadata('every'));
print('meta='+metadata('some'));
print('meta='+metadata('forEach'));
"#;

#[test]
fn array_iteration_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array iteration oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("order", ORDER_CASES),
        ("generic", GENERIC_CASES),
        ("errors", ERROR_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector did not produce a completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 4);
}

#[test]
fn array_iteration_values_match_pinned_quickjs() {
    compare_value_cases("Array every/some/forEach values", VALUE_CASES);
}

#[test]
fn array_iteration_holes_order_and_abrupt_completion_match_pinned_quickjs() {
    compare_value_cases("Array every/some/forEach observable order", ORDER_CASES);
}

#[test]
fn array_iteration_generic_receivers_match_pinned_quickjs() {
    compare_value_cases("Array every/some/forEach generic receivers", GENERIC_CASES);
}

#[test]
fn array_iteration_errors_match_pinned_quickjs() {
    compare_value_cases("Array every/some/forEach errors", ERROR_CASES);
}

#[test]
fn array_iteration_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array iteration graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array iteration prototype order/metadata drifted",
    );
}

#[test]
fn array_iteration_boxing_native_errors_and_user_throws_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let defining_string_prototype = defining.string_prototype().unwrap();
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
    let every = property_callable(&runtime, &mut defining, &defining_array_prototype, "every");

    let capture = eval_callable(
        &runtime,
        &mut caller,
        "(function(value,index,receiver){globalThis.iterationReceiver=receiver;return true})",
        "caller receiver-capturing callback",
    );
    assert_eq!(
        caller
            .call(
                &every,
                Value::String(JsString::try_from_utf8("a").unwrap()),
                &[Value::Object(capture.as_object().clone())],
            )
            .expect("cross-realm primitive Array.every call"),
        Value::Bool(true),
    );
    let captured = eval_object(
        &mut caller,
        "iterationReceiver",
        "captured iteration receiver",
    );
    assert_eq!(
        runtime.get_prototype_of(&captured).unwrap(),
        Some(defining_string_prototype),
        "Array iteration boxed its callback receiver in the caller realm",
    );

    let empty = eval_object(&mut caller, "[]", "caller empty Array");
    assert!(matches!(
        caller.call(&every, Value::Object(empty), &[Value::Int(0)]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.every callback TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.every native TypeError did not use the method defining realm",
    );

    let throwing = eval_callable(
        &runtime,
        &mut caller,
        "(function(){throw new TypeError('caller callback')})",
        "caller throwing iteration callback",
    );
    let one = eval_object(&mut caller, "[1]", "caller one-element Array");
    assert!(matches!(
        caller.call(
            &every,
            Value::Object(one),
            &[Value::Object(throwing.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.every user callback error");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.every replaced a user callback throw with a defining-realm error",
    );
}

fn compare_value_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group} differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in cases {
        let expected = observe_oracle(&oracle, source, description);
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            expected,
            "{group} drifted for {description}: {source:?}",
        );
    }
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
                    error_string_property(runtime, context, &error, "name", description),
                    error_string_property(runtime, context, &error, "message", description),
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

fn rust_graph_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array_prototype = context.array_prototype().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let implemented = [
        "at",
        "with",
        "every",
        "some",
        "forEach",
        "fill",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "indexOf",
        "lastIndexOf",
        "includes",
        "copyWithin",
        "values",
        "keys",
        "entries",
    ];
    let names = runtime
        .own_property_keys(&array_prototype)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .filter(|name| implemented.contains(&name.as_str()))
        .collect::<Vec<_>>();
    let mut observations = vec![format!("keys={}", names.join(","))];
    for name in ["every", "some", "forEach"] {
        observations.push(format!(
            "meta={}",
            method_metadata(
                &runtime,
                &mut context,
                &array_prototype,
                &function_prototype,
                name,
            )
        ));
    }
    observations
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS Array iteration graph oracle: {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS Array iteration graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array iteration graph output was not UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn method_metadata(
    runtime: &Runtime,
    context: &mut Context,
    owner: &ObjectRef,
    function_prototype: &ObjectRef,
    name: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let descriptor = runtime
        .get_own_property(owner, &key)
        .unwrap()
        .unwrap_or_else(|| panic!("missing Array.prototype.{name}"));
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(function),
        writable,
        enumerable,
        configurable,
    } = &descriptor
    else {
        panic!("Array.prototype.{name} was not a function data property");
    };
    let callable = runtime
        .as_callable(function)
        .unwrap()
        .unwrap_or_else(|| panic!("Array.prototype.{name} was not callable"));
    let function_name = context
        .get_property(function, &runtime.intern_property_key("name").unwrap())
        .unwrap();
    let function_length = context
        .get_property(function, &runtime.intern_property_key("length").unwrap())
        .unwrap();
    let name_descriptor = runtime
        .get_own_property(function, &runtime.intern_property_key("name").unwrap())
        .unwrap()
        .unwrap_or_else(|| panic!("Array.{name} name descriptor was missing"));
    let length_descriptor = runtime
        .get_own_property(function, &runtime.intern_property_key("length").unwrap())
        .unwrap()
        .unwrap_or_else(|| panic!("Array.{name} length descriptor was missing"));
    format!(
        "{name}:{}:{}:D{}{}{}:{}:{}:{}:{}:{}",
        primitive_value_text(function_name),
        primitive_value_text(function_length),
        Number(*writable),
        Number(*enumerable),
        Number(*configurable),
        data_descriptor_bits(&name_descriptor),
        data_descriptor_bits(&length_descriptor),
        true,
        runtime.get_prototype_of(function).unwrap().as_ref() == Some(function_prototype),
        runtime.is_constructor(callable.as_object()).unwrap(),
    )
}

fn data_descriptor_bits(descriptor: &CompleteOrdinaryPropertyDescriptor) -> String {
    let CompleteOrdinaryPropertyDescriptor::Data {
        writable,
        enumerable,
        configurable,
        ..
    } = descriptor
    else {
        panic!("expected a data descriptor");
    };
    format!(
        "D{}{}{}",
        Number(*writable),
        Number(*enumerable),
        Number(*configurable),
    )
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(function) = context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read callable {name}: {error}"))
    else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&function)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
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
        .unwrap_or_else(|| panic!("Rust {description} was not callable"))
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

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &ObjectRef,
    name: &str,
    description: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
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

struct Number(bool);

impl std::fmt::Display for Number {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0 { "1" } else { "0" })
    }
}
