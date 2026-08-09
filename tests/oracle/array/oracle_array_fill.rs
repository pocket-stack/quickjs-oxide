use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, Runtime, RuntimeError,
    Value,
};

// This target pins QuickJS 2026-06-04 `Array.prototype.fill` as one complete
// generic mutation slice. The probes emphasize QuickJS's exact conversion and
// throwing-Set order, including partial mutation after an indexed write fails.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "dense fill mutates and returns the receiver",
        r#"(function(){
            var source=[0,1,2,3],result=source.fill(9,1,3);
            return (result===source)+"|"+source.length+"|"+source[0]+"|"+
                source[1]+"|"+source[2]+"|"+source[3];
        })()"#,
    ),
    (
        "holes become own properties and omitted value is undefined",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var source=Array(3);source.fill();
            return source.length+"|"+own(source,0)+"|"+own(source,1)+"|"+
                own(source,2)+"|"+(source[0]===undefined)+"|"+
                (source[1]===undefined)+"|"+(source[2]===undefined);
        })()"#,
    ),
    (
        "fill value is copied without coercion",
        r#"(function(){
            var log="",value=Object();
            value.valueOf=function(){log+="v";throw 61};
            value.toString=function(){log+="s";throw 62};
            var source=[0,0],result=source.fill(value);
            return (result===source)+"|"+(source[0]===value)+"|"+
                (source[1]===value)+"|"+log;
        })()"#,
    ),
    (
        "an inherited setter intercepts Set without creating an own property",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var log="",proto=Object(),source=Object();
            proto.__defineSetter__("1",function(value){log+="s"+value});
            Object.setPrototypeOf(source,proto);source.length=3;
            Array.prototype.fill.call(source,"x",0,3);
            return source[0]+"|"+own(source,0)+"|"+own(source,1)+"|"+
                source[2]+"|"+own(source,2)+"|"+log;
        })()"#,
    ),
];

const BOUND_CASES: &[(&str, &str)] = &[
    (
        "start uses saturating Int64 conversion and negative-length offset",
        r#"(function(){
            function run(start){var a=[0,1,2,3];a.fill(9,start);return ""+a[0]+a[1]+a[2]+a[3]}
            return run(undefined)+"|"+run(0/0)+"|"+run(-0)+"|"+run(1.9)+"|"+
                run(-1.9)+"|"+run(-9)+"|"+run(1/0)+"|"+run(-1/0)+"|"+run("2");
        })()"#,
    ),
    (
        "end uses saturating Int64 conversion and explicit undefined means length",
        r#"(function(){
            function run(end){var a=[0,1,2,3];a.fill(9,1,end);return ""+a[0]+a[1]+a[2]+a[3]}
            return run(undefined)+"|"+run(0/0)+"|"+run(2.9)+"|"+run(-1.9)+"|"+
                run(-9)+"|"+run(1/0)+"|"+run(-1/0)+"|"+run("3");
        })()"#,
    ),
    (
        "zero length still converts explicit start and end",
        r#"(function(){
            var log="",source=Object(),start=Object(),end=Object();
            source.__defineGetter__("length",function(){log+="L";return 0});
            start.valueOf=function(){log+="S";return 1};
            end.valueOf=function(){log+="E";return 2};
            var result=Array.prototype.fill.call(source,7,start,end);
            return (result===source)+"|"+log;
        })()"#,
    ),
    (
        "MAX_SAFE_INTEGER length supports a narrow high-index range",
        r#"(function(){
            var source=Object();source.length=9007199254740991;
            var result=Array.prototype.fill.call(source,7,9007199254740990);
            return (result===source)+"|"+source[9007199254740990]+"|"+
                Object.prototype.hasOwnProperty.call(source,"9007199254740990");
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "length then start then end then ascending throwing Set order",
        r#"(function(){
            var log="",source=Object(),length=Object(),start=Object(),end=Object();
            source.__defineGetter__("length",function(){log+="L";return length});
            length.valueOf=function(){log+="N";return 3};
            start.valueOf=function(){log+="S";return 0};
            end.valueOf=function(){log+="E";return 3};
            source.__defineSetter__("0",function(value){log+="0"+value});
            source.__defineSetter__("1",function(value){log+="1"+value});
            source.__defineSetter__("2",function(value){log+="2"+value});
            var result=Array.prototype.fill.call(source,"x",start,end);
            return (result===source)+"|"+log;
        })()"#,
    ),
    (
        "a failed Set preserves earlier mutation and skips later indices",
        r#"(function(){
            var source=[0,1,2],descriptor=Object();
            descriptor.value=1;descriptor.writable=false;descriptor.configurable=true;
            Object.defineProperty(source,"1",descriptor);
            try{source.fill(9);return "missing"}
            catch(error){return source[0]+"|"+source[1]+"|"+source[2]+"|"+
                error.name+"|"+error.message}
        })()"#,
    ),
    (
        "a start throw prevents end conversion and indexed writes",
        r#"(function(){
            var log="",source=Object(),start=Object(),end=Object();source.length=2;
            start.valueOf=function(){log+="S";throw 71};
            end.valueOf=function(){log+="E";return 2};
            source.__defineSetter__("0",function(){log+="0"});
            try{Array.prototype.fill.call(source,9,start,end);return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log}
        })()"#,
    ),
    (
        "an end throw follows start conversion but precedes indexed writes",
        r#"(function(){
            var log="",source=Object(),start=Object(),end=Object();source.length=2;
            start.valueOf=function(){log+="S";return 0};
            end.valueOf=function(){log+="E";throw 72};
            source.__defineSetter__("0",function(){log+="0"});
            try{Array.prototype.fill.call(source,9,start,end);return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log}
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like receiver is mutated in place",
        r#"(function(){
            var source=Object();source[0]="a";source.length=3;
            var result=Array.prototype.fill.call(source,"x",1);
            return (result===source)+"|"+source[0]+"|"+source[1]+"|"+source[2]+"|"+source.length;
        })()"#,
    ),
    (
        "String indexed properties reject the first attempted Set",
        r#"(function(){
            try{Array.prototype.fill.call("abc","x",1,2);return "missing"}
            catch(error){return error.name+"|"+error.message}
        })()"#,
    ),
    (
        "primitive receivers with zero length return their wrappers",
        r#"(function(){
            var number=Array.prototype.fill.call(7,"x");
            var boolean=Array.prototype.fill.call(true,"x");
            return Object.prototype.toString.call(number)+"|"+
                Object.prototype.toString.call(boolean)+"|"+typeof number+"|"+typeof boolean;
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("null receiver", "Array.prototype.fill.call(null,1)"),
    (
        "undefined receiver",
        "Array.prototype.fill.call(undefined,1)",
    ),
    ("Symbol start", "[0].fill(1,Symbol('start'))"),
    ("Symbol end", "[0].fill(1,0,Symbol('end'))"),
    ("BigInt start", "[0].fill(1,0n)"),
    ("BigInt end", "[0].fill(1,0,1n)"),
    (
        "Symbol length",
        "(function(){var source=Object();source.length=Symbol('length');return Array.prototype.fill.call(source,1)})()",
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','fill','indexOf','lastIndexOf','includes','values','keys','entries'];
var own=Reflect.ownKeys(Array.prototype),names=[];
for(var i=0;i<own.length;i++)
  if(implemented.indexOf(own[i])>=0)names[names.length]=own[i];
function bits(descriptor) {
  return 'D'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
var descriptor=Object.getOwnPropertyDescriptor(Array.prototype,'fill'),fn=descriptor.value;
var constructable;
try { Reflect.construct(function(){},[],fn); constructable=true; }
catch(error) { constructable=false; }
print('keys='+names.join(','));
print('meta=fill:'+fn.name+':'+fn.length+':'+bits(descriptor)+':'+
      bits(Object.getOwnPropertyDescriptor(fn,'name'))+':'+
      bits(Object.getOwnPropertyDescriptor(fn,'length'))+':'+
      (typeof fn==='function')+':'+(Object.getPrototypeOf(fn)===Function.prototype)+':'+constructable);
"#;

#[test]
fn array_fill_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.fill oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("bounds", BOUND_CASES),
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
    assert_eq!(oracle_graph_observations(&oracle).len(), 2);
}

#[test]
fn array_fill_values_match_pinned_quickjs() {
    compare_value_cases("Array.fill values", VALUE_CASES);
}

#[test]
fn array_fill_bounds_match_pinned_quickjs() {
    compare_value_cases("Array.fill bounds", BOUND_CASES);
}

#[test]
fn array_fill_observable_order_and_partial_mutation_match_pinned_quickjs() {
    compare_value_cases("Array.fill observable order", ORDER_CASES);
}

#[test]
fn array_fill_generic_receivers_match_pinned_quickjs() {
    compare_value_cases("Array.fill generic receivers", GENERIC_CASES);
}

#[test]
fn array_fill_errors_match_pinned_quickjs() {
    compare_value_cases("Array.fill errors", ERROR_CASES);
}

#[test]
fn array_fill_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.fill graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array.fill prototype order/metadata drifted",
    );
}

#[test]
fn array_fill_boxing_native_errors_and_user_throws_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let defining_boolean_prototype = eval_object(
        &mut defining,
        "Boolean.prototype",
        "defining Boolean prototype",
    );
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
    let method = property_callable(&runtime, &mut defining, &defining_array_prototype, "fill");

    let receiver = eval_object(&mut caller, "[10,20]", "caller Array receiver");
    let Value::Object(result) = caller
        .call(
            &method,
            Value::Object(receiver.clone()),
            &[Value::Int(99), Value::Int(1)],
        )
        .expect("cross-realm Array.fill call")
    else {
        panic!("cross-realm Array.fill result was not an object");
    };
    assert_eq!(result, receiver, "Array.fill did not return its receiver");
    assert_eq!(int_property(&runtime, &mut caller, &result, "0"), 10);
    assert_eq!(int_property(&runtime, &mut caller, &result, "1"), 99);

    let Value::Object(wrapper) = caller
        .call(&method, Value::Bool(true), &[])
        .expect("cross-realm primitive Array.fill call")
    else {
        panic!("Array.fill primitive result was not boxed");
    };
    assert_eq!(
        runtime.get_prototype_of(&wrapper).unwrap(),
        Some(defining_boolean_prototype),
        "Array.fill boxed a primitive in the caller realm",
    );

    let readonly = eval_object(
        &mut caller,
        r#"(function(){
            var source=[1],descriptor=Object();
            descriptor.value=1;descriptor.writable=false;
            Object.defineProperty(source,"0",descriptor);return source;
        })()"#,
        "caller read-only receiver",
    );
    assert!(matches!(
        caller.call(&method, Value::Object(readonly), &[Value::Int(9)],),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.fill native TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.fill native TypeError did not use the method defining realm",
    );

    let throwing_receiver = eval_object(
        &mut caller,
        r#"(function(){
            var source=Object();source.length=1;
            source.__defineSetter__("0",function(){throw new TypeError("caller setter")});
            return source;
        })()"#,
        "caller throwing receiver",
    );
    assert!(matches!(
        caller.call(&method, Value::Object(throwing_receiver), &[Value::Int(9)],),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.fill user setter error");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.fill replaced a user setter throw with a defining-realm error",
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
        "fill",
        "indexOf",
        "lastIndexOf",
        "includes",
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
    vec![
        format!("keys={}", names.join(",")),
        format!(
            "meta={}",
            method_metadata(
                &runtime,
                &mut context,
                &array_prototype,
                &function_prototype,
                "fill",
            )
        ),
    ]
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS Array.fill graph oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS Array.fill graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array.fill graph output was not UTF-8")
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
        .expect("Array.fill name descriptor");
    let length_descriptor = runtime
        .get_own_property(function, &runtime.intern_property_key("length").unwrap())
        .unwrap()
        .expect("Array.fill length descriptor");
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

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Int(value) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an Int property");
    };
    value
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
