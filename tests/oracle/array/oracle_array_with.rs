use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, Runtime, RuntimeError,
    Value,
};

// This target pins QuickJS 2026-06-04 `Array.prototype.with` as one complete
// change-by-copy slice. Source probes deliberately avoid later Array methods
// and later Object reflection APIs on the Rust side. The graph probe uses the
// host API so lazy native descriptors remain observable without expanding the
// source-level Object surface.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "dense replacement leaves the source unchanged",
        r#"(function(){
            var source=[10,20,30],result=source.with(1,99);
            return source.length+"|"+source[0]+"|"+source[1]+"|"+source[2]+"|"+
                result.length+"|"+result[0]+"|"+result[1]+"|"+result[2]+"|"+
                Array.isArray(result)+"|"+(Object.getPrototypeOf(result)===Array.prototype);
        })()"#,
    ),
    (
        "holes become dense own undefined properties",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var source=[,1,,],result=source.with(1,9);
            return source.length+"|"+own(source,0)+"|"+own(source,2)+"|"+
                result.length+"|"+own(result,0)+"|"+(result[0]===undefined)+"|"+
                own(result,1)+"|"+result[1]+"|"+own(result,2)+"|"+(result[2]===undefined);
        })()"#,
    ),
    (
        "inherited values are copied into dense own properties",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            Array.prototype[0]=7;
            var source=Array(3),result=source.with(2,9);
            var observation=result[0]+"|"+(result[1]===undefined)+"|"+result[2]+"|"+
                own(result,0)+"|"+own(result,1)+"|"+own(result,2);
            delete Array.prototype[0];
            return observation;
        })()"#,
    ),
    (
        "omitted replacement writes undefined",
        r#"(function(){
            var result=[1,2].with(0);
            return result.length+"|"+(result[0]===undefined)+"|"+result[1]+"|"+
                Object.prototype.hasOwnProperty.call(result,0);
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "replacement index is never read",
        r#"(function(){
            var log="",source=[10,20,30];
            source.__defineGetter__("1",function(){log+="1";throw 71});
            var result=source.with(1,99);
            return result[0]+"|"+result[1]+"|"+result[2]+"|"+log;
        })()"#,
    ),
    (
        "length index and source getters run in pinned order",
        r#"(function(){
            var log="",length=Object(),index=Object(),source=Object();
            length.valueOf=function(){log+="N";return 4};
            index.valueOf=function(){log+="I";return 2};
            source.__defineGetter__("length",function(){log+="L";return length});
            source.__defineGetter__("0",function(){log+="0";return "a"});
            source.__defineGetter__("1",function(){log+="1";return "b"});
            source.__defineGetter__("2",function(){log+="2";throw 72});
            source.__defineGetter__("3",function(){log+="3";return "d"});
            var result=Array.prototype.with.call(source,index,"X");
            return result[0]+result[1]+result[2]+result[3]+"|"+log;
        })()"#,
    ),
    (
        "constructor and species are not consulted",
        r#"(function(){
            var log="",speciesSource=[1,2],ctor=Object();
            ctor.__defineGetter__(Symbol.species,function(){log+="s";throw 73});
            speciesSource.constructor=ctor;
            var first=speciesSource.with(0,9);
            var constructorSource=[3,4];
            constructorSource.__defineGetter__("constructor",function(){log+="c";throw 74});
            var second=constructorSource.with(1,8);
            return first[0]+"|"+first[1]+"|"+second[0]+"|"+second[1]+"|"+log;
        })()"#,
    ),
];

const INDEX_CASES: &[(&str, &str)] = &[
    (
        "index conversion matrix",
        r#"(function(){
            var source=[10,20,30];
            return source.with(undefined,9)[0]+"|"+source.with(0/0,9)[0]+"|"+
                source.with(-0,9)[0]+"|"+source.with(1.9,9)[1]+"|"+
                source.with(-1.9,9)[2]+"|"+source.with(-3,9)[0]+"|"+
                source.with("1",9)[1];
        })()"#,
    ),
    (
        "zero length still converts the index before rejecting it",
        r#"(function(){
            var log="",index=Object();index.valueOf=function(){log+="I";return 0};
            try{Array(0).with(index,9);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "length above the pinned dense limit fails before element access",
        r#"(function(){
            var log="",source=Object();source.length=2147483648;
            source.__defineGetter__("0",function(){log+="0";throw 75});
            try{Array.prototype.with.call(source,1,9);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like receiver returns a base Array",
        r#"(function(){
            var source=Object();source[0]="a";source[2]="c";source.length=3;
            var result=Array.prototype.with.call(source,1,"b");
            return result.length+"|"+result[0]+"|"+result[1]+"|"+result[2]+"|"+
                Array.isArray(result)+"|"+(Object.getPrototypeOf(result)===Array.prototype);
        })()"#,
    ),
    (
        "String receiver copies UTF-16 code units",
        r#"(function(){
            var source="A\uD83D\uDCA9Z",result=Array.prototype.with.call(source,1,"X");
            return result.length+"|"+result[0]+"|"+result[1]+"|"+
                result[2].charCodeAt(0)+"|"+result[3]+"|"+Array.isArray(result);
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("empty receiver index zero", "[].with(0,1)"),
    ("positive out of range index", "[1].with(1,2)"),
    ("negative out of range index", "[1].with(-2,2)"),
    ("positive infinity index", "[1].with(1/0,2)"),
    ("negative infinity index", "[1].with(-1/0,2)"),
    ("Symbol index", "[1].with(Symbol('index'),2)"),
    ("BigInt index", "[1].with(0n,2)"),
    ("null receiver", "Array.prototype.with.call(null,0,2)"),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','indexOf','lastIndexOf','includes','values','keys','entries'];
var own=Reflect.ownKeys(Array.prototype),names=[];
for(var i=0;i<own.length;i++)
  if(implemented.indexOf(own[i])>=0)names[names.length]=own[i];
function bits(descriptor) {
  return 'D'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
var descriptor=Object.getOwnPropertyDescriptor(Array.prototype,'with'),fn=descriptor.value;
var constructable;
try { Reflect.construct(function(){},[],fn); constructable=true; }
catch(error) { constructable=false; }
print('keys='+names.join(','));
print('meta=with:'+fn.name+':'+fn.length+':'+bits(descriptor)+':'+
      bits(Object.getOwnPropertyDescriptor(fn,'name'))+':'+
      bits(Object.getOwnPropertyDescriptor(fn,'length'))+':'+
      (typeof fn==='function')+':'+(Object.getPrototypeOf(fn)===Function.prototype)+':'+constructable);
"#;

#[test]
fn array_with_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.with oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("order", ORDER_CASES),
        ("index", INDEX_CASES),
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
fn array_with_values_match_pinned_quickjs() {
    compare_value_cases("Array.with values", VALUE_CASES);
}

#[test]
fn array_with_observable_order_matches_pinned_quickjs() {
    compare_value_cases("Array.with observable order", ORDER_CASES);
}

#[test]
fn array_with_index_conversion_and_errors_match_pinned_quickjs() {
    compare_value_cases("Array.with index conversion", INDEX_CASES);
    compare_value_cases("Array.with errors", ERROR_CASES);
}

#[test]
fn array_with_generic_receivers_match_pinned_quickjs() {
    compare_value_cases("Array.with generic receivers", GENERIC_CASES);
}

#[test]
fn array_with_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.with graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array.with prototype order/metadata drifted",
    );
}

#[test]
fn array_with_result_and_native_errors_use_the_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let caller_array_prototype = caller.array_prototype().unwrap();
    let defining_range_error = eval_object(
        &mut defining,
        "RangeError.prototype",
        "RangeError prototype",
    );
    let caller_type_error = eval_object(&mut caller, "TypeError.prototype", "TypeError prototype");
    let method = property_callable(&runtime, &mut defining, &defining_array_prototype, "with");

    let receiver = eval_object(&mut caller, "[10,20]", "caller Array receiver");
    let Value::Object(result) = caller
        .call(
            &method,
            Value::Object(receiver.clone()),
            &[Value::Int(1), Value::Int(99)],
        )
        .expect("cross-realm Array.with call")
    else {
        panic!("cross-realm Array.with result was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype.clone()),
        "Array.with result did not use the native defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(caller_array_prototype),
    );
    assert_eq!(int_property(&runtime, &mut caller, &result, "0"), 10);
    assert_eq!(int_property(&runtime, &mut caller, &result, "1"), 99);

    assert!(matches!(
        caller.call(
            &method,
            Value::Object(receiver),
            &[Value::Int(2), Value::Int(0)],
        ),
        Err(RuntimeError::Exception),
    ));
    let range_error = take_exception_object(&mut caller, "cross-realm Array.with RangeError");
    assert_eq!(
        runtime.get_prototype_of(&range_error).unwrap(),
        Some(defining_range_error),
        "Array.with RangeError did not use the native defining realm",
    );

    let throwing_receiver = eval_object(
        &mut caller,
        r#"(function(){
            var source=Object();source.length=2;
            source.__defineGetter__("0",function(){throw new TypeError("caller getter")});
            return source;
        })()"#,
        "caller throwing receiver",
    );
    assert!(matches!(
        caller.call(
            &method,
            Value::Object(throwing_receiver),
            &[Value::Int(1), Value::Int(0)],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.with user getter error");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.with replaced a user getter throw with a defining-realm error",
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
                "with",
            )
        ),
    ]
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS Array.with graph oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS Array.with graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array.with graph output was not UTF-8")
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
        .expect("Array.with name descriptor");
    let length_descriptor = runtime
        .get_own_property(function, &runtime.intern_property_key("length").unwrap())
        .unwrap()
        .expect("Array.with length descriptor");
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
