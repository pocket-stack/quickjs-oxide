use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, Runtime, RuntimeError,
    Value,
};

// This target pins QuickJS 2026-06-04's shared `js_array_find` kernel. Unlike
// indexOf, every index is read and passed to the predicate, including holes.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "find and findIndex stop at the first forward match",
        r#"(function(){
            var source=[1,4,6,8],log="";
            var value=source.find(function(element,index){log+="v"+index;return element>5});
            var index=source.findIndex(function(element,position){log+="i"+position;return element>5});
            return value+"|"+index+"|"+log+"|"+source.length+"|"+source[2];
        })()"#,
    ),
    (
        "findLast and findLastIndex stop at the first reverse match",
        r#"(function(){
            var source=[1,4,6,8],log="";
            var value=source.findLast(function(element,index){log+="v"+index;return element<7});
            var index=source.findLastIndex(function(element,position){log+="i"+position;return element<7});
            return value+"|"+index+"|"+log;
        })()"#,
    ),
    (
        "not-found results use undefined and minus one",
        r#"(function(){
            var source=[1,2],never=function(){return false};
            return (source.find(never)===undefined)+"|"+source.findIndex(never)+"|"+
                (source.findLast(never)===undefined)+"|"+source.findLastIndex(never);
        })()"#,
    ),
    (
        "predicate results use ToBoolean without object coercion hooks",
        r#"(function(){
            var log="",truthy=Object();
            truthy.valueOf=function(){log+="v";throw 61};
            truthy.toString=function(){log+="s";throw 62};
            var value=[3].find(function(){return truthy});
            var index=[3].findIndex(function(){return 0});
            return value+"|"+index+"|"+log;
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "holes are read as undefined and still invoke the predicate",
        r#"(function(){
            var source=Array(3),log="";source[2]=7;
            var value=source.find(function(element,index){
                log+=index+":"+(element===undefined)+",";return index===1;
            });
            return (value===undefined)+"|"+log;
        })()"#,
    ),
    (
        "inherited values and reverse getter order remain observable",
        r#"(function(){
            var log="",proto=Object(),source=Object();proto[1]="p";
            Object.setPrototypeOf(source,proto);source.length=3;
            source.__defineGetter__("2",function(){log+="g2";return "z"});
            var value=Array.prototype.findLast.call(source,function(element,index){
                log+="c"+index+element;return element==="p";
            });
            return value+"|"+log;
        })()"#,
    ),
    (
        "length is snapshotted while later Gets observe mutation",
        r#"(function(){
            var source=[0,1,2],log="";
            var result=source.findIndex(function(element,index){
                log+=index+":"+(element===undefined)+",";
                if(index===0){delete source[1];source[3]=3;source.length=4}
                return false;
            });
            return result+"|"+log+"|"+source.length+"|"+source[3];
        })()"#,
    ),
    (
        "callback arguments thisArg and early stop use pinned identities",
        r#"(function(){
            var marker=Object(),source=[5,6],log="";
            source.__defineGetter__("1",function(){log+="late";throw 63});
            var value=source.find(function(element,index,receiver){
                "use strict";log+=(this===marker)+":"+(receiver===source)+":"+index;
                return true;
            },marker);
            return value+"|"+log;
        })()"#,
    ),
    (
        "length precedes predicate validation even for an empty receiver",
        r#"(function(){
            var log="",source=Object();
            source.__defineGetter__("length",function(){log+="L";return 0});
            try{Array.prototype.find.call(source,7);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "a Get throw precedes callback invocation",
        r#"(function(){
            var log="",source=Object();source.length=1;
            source.__defineGetter__("0",function(){log+="G";throw 71});
            try{Array.prototype.find.call(source,function(){log+="C";return true});return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log}
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like receivers pass the original object",
        r#"(function(){
            var source=Object();source[0]="a";source[1]="b";source.length=2;
            var result=Array.prototype.findIndex.call(source,function(value,index,receiver){
                return receiver===source&&value==="b"&&index===1;
            });
            return result;
        })()"#,
    ),
    (
        "String receiver callback sees UTF-16 units and the original primitive",
        r#"(function(){
            var source="A\uD83D\uDCA9Z",seen="";
            var value=Array.prototype.find.call(source,function(element,index,receiver){
                seen+=typeof receiver+":"+(receiver===source)+":"+index+",";
                return element.charCodeAt(0)===0xDCA9;
            });
            return value.charCodeAt(0)+"|"+seen;
        })()"#,
    ),
    (
        "zero-length primitive receivers validate but do not call predicates",
        r#"(function(){
            var calls=0,predicate=function(){calls++;return true};
            var number=Array.prototype.find.call(7,predicate);
            var boolean=Array.prototype.findLastIndex.call(true,predicate);
            return (number===undefined)+"|"+boolean+"|"+calls;
        })()"#,
    ),
    (
        "findLast supports one MAX_SAFE_INTEGER high-index Get",
        r#"(function(){
            var source=Object(),calls=0;source.length=9007199254740991;
            source[9007199254740990]="x";
            var value=Array.prototype.findLast.call(source,function(element,index,receiver){
                calls++;return receiver===source&&index===9007199254740990&&element==="x";
            });
            return value+"|"+calls;
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "null receiver",
        "Array.prototype.find.call(null,function(){return true})",
    ),
    (
        "undefined receiver",
        "Array.prototype.find.call(undefined,function(){return true})",
    ),
    ("missing predicate", "[].find()"),
    ("undefined predicate", "[].findIndex(undefined)"),
    ("number predicate", "[].findLast(1)"),
    ("symbol predicate", "[].findLastIndex(Symbol('predicate'))"),
    ("BigInt predicate", "[].find(0n)"),
    (
        "Symbol length wins before predicate validation",
        "(function(){var source=Object();source.length=Symbol('length');return Array.prototype.find.call(source,1)})()",
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','copyWithin','values','keys','entries'];
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
print('meta='+metadata('find'));
print('meta='+metadata('findIndex'));
print('meta='+metadata('findLast'));
print('meta='+metadata('findLastIndex'));
"#;

#[test]
fn array_find_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.find oracle self-check: set QJS_ORACLE to upstream qjs");
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
    assert_eq!(oracle_graph_observations(&oracle).len(), 5);
}

#[test]
fn array_find_values_match_pinned_quickjs() {
    compare_value_cases("Array.find values", VALUE_CASES);
}

#[test]
fn array_find_holes_order_and_abrupt_completion_match_pinned_quickjs() {
    compare_value_cases("Array.find observable order", ORDER_CASES);
}

#[test]
fn array_find_generic_receivers_match_pinned_quickjs() {
    compare_value_cases("Array.find generic receivers", GENERIC_CASES);
}

#[test]
fn array_find_errors_match_pinned_quickjs() {
    compare_value_cases("Array.find errors", ERROR_CASES);
}

#[test]
fn array_find_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.find graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array.find prototype order/metadata drifted",
    );
}

#[test]
fn array_find_native_errors_and_user_throws_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let caller_object_prototype =
        eval_object(&mut caller, "Object.prototype", "caller Object prototype");
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );
    let find = property_callable(&runtime, &mut defining, &defining_array_prototype, "find");
    let find_index = property_callable(
        &runtime,
        &mut defining,
        &defining_array_prototype,
        "findIndex",
    );

    let receiver = eval_object(&mut caller, "[Object()]", "caller object element Array");
    let always = eval_callable(
        &runtime,
        &mut caller,
        "(function(){return true})",
        "caller true predicate",
    );
    let Value::Object(found) = caller
        .call(
            &find,
            Value::Object(receiver),
            &[Value::Object(always.as_object().clone())],
        )
        .expect("cross-realm Array.find call")
    else {
        panic!("cross-realm Array.find result was not the caller object element");
    };
    assert_eq!(
        runtime.get_prototype_of(&found).unwrap(),
        Some(caller_object_prototype),
        "Array.find replaced a caller-realm element",
    );

    let indexed_receiver = eval_object(&mut caller, "[10,20]", "caller indexed Array");
    let twenty = eval_callable(
        &runtime,
        &mut caller,
        "(function(value){return value===20})",
        "caller value predicate",
    );
    assert_eq!(
        caller
            .call(
                &find_index,
                Value::Object(indexed_receiver),
                &[Value::Object(twenty.as_object().clone())],
            )
            .expect("cross-realm Array.findIndex call"),
        Value::Int(1),
    );

    let empty = eval_object(&mut caller, "[]", "caller empty Array");
    assert!(matches!(
        caller.call(&find, Value::Object(empty), &[Value::Int(0)]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.find predicate TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.find native TypeError did not use the method defining realm",
    );

    let throwing = eval_callable(
        &runtime,
        &mut caller,
        "(function(){throw new TypeError('caller predicate')})",
        "caller throwing predicate",
    );
    let one = eval_object(&mut caller, "[1]", "caller one-element Array");
    assert!(matches!(
        caller.call(
            &find,
            Value::Object(one),
            &[Value::Object(throwing.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.find user predicate error");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.find replaced a user predicate throw with a defining-realm error",
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
    for name in ["find", "findIndex", "findLast", "findLastIndex"] {
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
        .unwrap_or_else(|error| panic!("could not run QuickJS Array.find graph oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS Array.find graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array.find graph output was not UTF-8")
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
