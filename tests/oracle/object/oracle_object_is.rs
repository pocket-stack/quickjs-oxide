use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, PropertyKey, Runtime,
    RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_object_is`. The upstream builtin is a direct
// `js_same_value` call: it never boxes or coerces either argument and it has no
// object-family-specific behavior to defer to later Proxy/TypedArray slices.

const SAME_VALUE_CASES: &[(&str, &str)] = &[
    (
        "missing arguments nullish booleans and type boundaries",
        r#"(function(){
            return [
                Object.is(),Object.is(undefined),Object.is(undefined,undefined),
                Object.is(null,null),Object.is(false,false),Object.is(true,true),
                Object.is(null,undefined),Object.is(false,0),Object.is(true,1),
                Object.is("1",1),Object.is(1,1n),Object.is(1,1,2),Object.is(1,2,1)
            ].join(":");
        })()"#,
    ),
    (
        "SameValue distinguishes signed zero and equates every NaN",
        r#"(function(){
            var positive=0,negative=-0;
            return [
                Object.is(positive,positive),Object.is(negative,negative),
                Object.is(positive,negative),Object.is(-negative,positive),
                Object.is(NaN,NaN),Object.is(0/0,Number("not-a-number")),
                Object.is(Infinity,1/0),Object.is(-Infinity,-1/0),
                Object.is(Infinity,-Infinity),Object.is(1,1.0),Object.is(1,1.0000000000000002)
            ].join(":");
        })()"#,
    ),
    (
        "strings BigInts and Symbols use primitive value or identity semantics",
        r#"(function(){
            var prefix="ox",shared=Symbol("shared"),other=Symbol("shared");
            var registeredA=Symbol.for("quickjs-oxide-object-is");
            var registeredB=Symbol.for("quickjs-oxide-object-is");
            var large=900719925474099312345678901234567890n;
            return [
                Object.is("oxide",prefix+"ide"),Object.is("oxide","Oxide"),
                Object.is("",""+""),Object.is(0n,-0n),Object.is(7n,3n+4n),
                Object.is(large,900719925474099312345678901234567890n),
                Object.is(large,large+1n),Object.is(shared,shared),Object.is(shared,other),
                Object.is(registeredA,registeredB),Object.is(Symbol.iterator,Symbol.iterator)
            ].join(":");
        })()"#,
    ),
    (
        "ordinary exotic callable and boxed object values compare only by identity",
        r#"(function(){
            var object=Object(),other=Object(),array=[],fn=function(){},error=new Error("x");
            var boxedNumber=Object(1),boxedString=Object("x"),boxedBigInt=Object(1n);
            return [
                Object.is(object,object),Object.is(object,other),Object.is(array,array),
                Object.is([],[]),Object.is(fn,fn),Object.is(fn,function(){}),
                Object.is(error,error),Object.is(new Error("x"),new Error("x")),
                Object.is(boxedNumber,boxedNumber),Object.is(boxedNumber,Object(1)),
                Object.is(boxedNumber,1),Object.is(boxedString,"x"),Object.is(boxedBigInt,1n)
            ].join(":");
        })()"#,
    ),
];

const NO_COERCION_CASES: &[(&str, &str)] = &[
    (
        "comparison does not read or call primitive conversion hooks",
        r#"(function(){
            var log="",left=Object(),right=Object();
            left[Symbol.toPrimitive]=function(){log+="p";throw "primitive"};
            left.valueOf=function(){log+="v";throw "valueOf"};
            left.toString=function(){log+="s";throw "toString"};
            right[Symbol.toPrimitive]=left[Symbol.toPrimitive];
            return [
                Object.is(left,left),Object.is(left,right),Object.is(left,1),
                Object.is(1,left),Object.is.call(right,left,left),log
            ].join(":");
        })()"#,
    ),
    (
        "comparison does not inspect wrappers getters or object structure",
        r#"(function(){
            var calls=0,primitiveSymbol=Symbol("x"),number=Object(3),string=Object("ab"),symbol=Object(primitiveSymbol);
            number.__defineGetter__("valueOf",function(){calls++;throw 1});
            string.__defineGetter__("toString",function(){calls++;throw 2});
            symbol.__defineGetter__(Symbol.toPrimitive,function(){calls++;throw 3});
            var sameNumber=number,sameString=string,sameSymbol=symbol;
            return [
                Object.is(number,sameNumber),Object.is(string,sameString),
                Object.is(symbol,sameSymbol),Object.is(number,3),Object.is(string,"ab"),
                Object.is(symbol,primitiveSymbol),calls
            ].join(":");
        })()"#,
    ),
    (
        "this value is ignored and frozen aliases preserve identity",
        r#"(function(){
            var receiver=Object(),value=Object(),alias=value;
            Object.preventExtensions(value);
            return [
                Object.is.call(receiver,value,alias),Object.is.call(null,+0,-0),
                Object.is.call(undefined,NaN,NaN),Object.is.apply(receiver,[value,value]),
                Object.is.apply(receiver,[value,Object()])
            ].join(":");
        })()"#,
    ),
];

const GRAPH_ORACLE: &str = r#"
function bit(value){return value?1:0}
function bits(descriptor){return 'D:'+bit(descriptor.writable)+bit(descriptor.enumerable)+bit(descriptor.configurable)}
function isConstructor(value){try{Reflect.construct(function(){},[],value);return true}catch(_){return false}}
function meta(name){
  var descriptor=Object.getOwnPropertyDescriptor(Object,name),value=descriptor.value;
  return value.name+':'+value.length+':' +(Object.getPrototypeOf(value)===Function.prototype)+':' +
    (typeof value==='function')+':'+isConstructor(value)+':' +Object.getOwnPropertyNames(value).join(',')+':'+bits(descriptor);
}
var selected=['length','name','prototype','create','getPrototypeOf','setPrototypeOf','defineProperty',
  'defineProperties','getOwnPropertyNames','getOwnPropertySymbols','groupBy','keys','values','entries',
  'isExtensible','preventExtensions','getOwnPropertyDescriptor','getOwnPropertyDescriptors','is'];
print('prefix='+Reflect.ownKeys(Object).filter(function(key){return selected.indexOf(key)>=0}).join(','));
print('is='+meta('is'));
print('identity='+(Object.is===Object.is));
var fn=Object.is;
print('is-props='+bits(Object.getOwnPropertyDescriptor(fn,'length'))+':' +bits(Object.getOwnPropertyDescriptor(fn,'name')));
"#;

const FRESH_DELETE_ORACLE: &str = r#"
var deleted=delete Object.is;
print([deleted,'is' in Object,Object.prototype.hasOwnProperty.call(Object,'is'),typeof Object.is].join('|'));
"#;

#[test]
fn object_is_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.is oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("SameValue", SAME_VALUE_CASES),
        ("no coercion", NO_COERCION_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector had no completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 4);
}

#[test]
fn object_is_same_value_edges_match_pinned_quickjs() {
    compare_cases("Object.is SameValue edges", SAME_VALUE_CASES);
}

#[test]
fn object_is_never_coerces_matches_pinned_quickjs() {
    compare_cases("Object.is no coercion", NO_COERCION_CASES);
}

#[test]
fn object_is_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.is graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle)
    );
}

#[test]
fn object_is_autoinit_can_be_deleted_before_materialization() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.is AutoInit delete: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_lines(&oracle, FRESH_DELETE_ORACLE, "Object.is fresh delete");

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = global_callable(&runtime, &mut context, "Object");
    let key = runtime.intern_property_key("is").unwrap();
    let deleted = runtime.delete_property(object.as_object(), &key).unwrap();
    let Value::Bool(in_object) = context.eval("'is' in Object").unwrap() else {
        panic!("Object.is membership check was not Boolean");
    };
    let values = [
        deleted.to_string(),
        in_object.to_string(),
        runtime
            .has_own_property(object.as_object(), &key)
            .unwrap()
            .to_string(),
        value_type(
            &runtime,
            &context.get_property(object.as_object(), &key).unwrap(),
        )
        .to_owned(),
    ];
    assert_eq!(vec![values.join("|")], expected);
}

#[test]
fn object_is_cross_realm_identity_and_constructor_error_realm_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object = global_callable(&runtime, &mut defining, "Object");
    let object_is = property_callable(&runtime, &mut defining, defining_object.as_object(), "is");
    let source = caller.new_object().unwrap();
    let other = caller.new_object().unwrap();

    assert_eq!(
        caller
            .call(
                &object_is,
                Value::Object(other.clone()),
                &[Value::Object(source.clone()), Value::Object(source.clone())],
            )
            .unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        caller
            .call(
                &object_is,
                Value::Undefined,
                &[Value::Object(source), Value::Object(other)],
            )
            .unwrap(),
        Value::Bool(false),
    );
    assert_eq!(
        caller
            .call(
                &object_is,
                Value::Undefined,
                &[Value::Int(0), Value::Float(-0.0)],
            )
            .unwrap(),
        Value::Bool(false),
    );
    assert_eq!(
        caller
            .call(
                &object_is,
                Value::Undefined,
                &[Value::Float(f64::NAN), Value::Float(f64::NAN)],
            )
            .unwrap(),
        Value::Bool(true),
    );

    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_eq!(
        caller.construct(&object_is, &[]),
        Err(RuntimeError::Exception)
    );
    let error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error),
        "non-constructor rejection must use the caller realm",
    );
}

#[test]
fn object_is_methods_are_per_realm_and_retain_then_release_their_realm() {
    let runtime = Runtime::new();
    let object_is = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_object = global_callable(&runtime, &mut first, "Object");
        let second_object = global_callable(&runtime, &mut second, "Object");
        let first_is = property_callable(&runtime, &mut first, first_object.as_object(), "is");
        let first_is_again =
            property_callable(&runtime, &mut first, first_object.as_object(), "is");
        let second_is = property_callable(&runtime, &mut second, second_object.as_object(), "is");
        assert_eq!(first_is, first_is_again);
        assert_ne!(first_is, second_is);
        assert_eq!(
            runtime.get_prototype_of(first_is.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        drop(second_is);
        first_is
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(object_is);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

fn compare_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle(&oracle, source, description),
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
            primitive_value_text(value)
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    string_property(runtime, context, &error, "name"),
                    string_property(runtime, context, &error, "message"),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value)
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
    let wrapper = r#"
try {
  var value=std.evalScript(scriptArgs[0]);
  print('return|'+typeof value+'|'+String(value));
} catch(error) {
  if(error!==null&&typeof error==='object')print('throw|object|'+error.name+'|'+error.message);
  else print('throw|'+typeof error+'|'+String(error));
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
    let function_prototype = context.function_prototype().unwrap();
    let object = global_callable(&runtime, &mut context, "Object");
    let selected = [
        "length",
        "name",
        "prototype",
        "create",
        "getPrototypeOf",
        "setPrototypeOf",
        "defineProperty",
        "defineProperties",
        "getOwnPropertyNames",
        "getOwnPropertySymbols",
        "groupBy",
        "keys",
        "values",
        "entries",
        "isExtensible",
        "preventExtensions",
        "getOwnPropertyDescriptor",
        "getOwnPropertyDescriptors",
        "is",
    ];
    let prefix = own_key_names(&runtime, object.as_object())
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let key = runtime.intern_property_key("is").unwrap();
    let descriptor = data_descriptor(&runtime, object.as_object(), &key);
    let Value::Object(function) = descriptor.0 else {
        panic!("Object.is was not an object");
    };
    let Value::Object(function_again) = context.get_property(object.as_object(), &key).unwrap()
    else {
        panic!("Object.is was not stable after materialization");
    };
    let callable = runtime.as_callable(&function).unwrap();
    let mut output = vec![
        format!("prefix={prefix}"),
        format!(
            "is={}:{}:{}:{}:{}:{}:{}",
            string_property(&runtime, &mut context, &function, "name"),
            int_property(&runtime, &mut context, &function, "length"),
            runtime.get_prototype_of(&function).unwrap().as_ref() == Some(&function_prototype),
            callable.is_some(),
            runtime.is_constructor(&function).unwrap(),
            own_key_names(&runtime, &function).join(","),
            data_bits(descriptor.1, descriptor.2, descriptor.3),
        ),
        format!("identity={}", function == function_again),
    ];
    let length = data_descriptor(
        &runtime,
        &function,
        &runtime.intern_property_key("length").unwrap(),
    );
    let function_name = data_descriptor(
        &runtime,
        &function,
        &runtime.intern_property_key("name").unwrap(),
    );
    output.push(format!(
        "is-props={}:{}",
        data_bits(length.1, length.2, length.3),
        data_bits(function_name.1, function_name.2, function_name.3),
    ));
    output
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    oracle_lines(oracle, GRAPH_ORACLE, "Object.is graph")
}

fn oracle_lines(oracle: &OsStr, source: &str, description: &str) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS {description} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS {description} output was not UTF-8: {error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}

fn global_callable(runtime: &Runtime, context: &mut Context, name: &str) -> CallableRef {
    let global = context.global_object().unwrap();
    property_callable(runtime, context, &global, name)
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    owner: &ObjectRef,
    name: &str,
) -> CallableRef {
    let Value::Object(object) = context
        .get_property(owner, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn intrinsic_prototype(
    runtime: &Runtime,
    context: &mut Context,
    constructor_name: &str,
) -> ObjectRef {
    let constructor = global_callable(runtime, context, constructor_name);
    let Value::Object(prototype) = context
        .get_property(
            constructor.as_object(),
            &runtime.intern_property_key("prototype").unwrap(),
        )
        .unwrap()
    else {
        panic!("{constructor_name}.prototype was not an object");
    };
    prototype
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("pending exception was not an object");
    };
    error
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect()
}

fn data_descriptor(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
) -> (Value, bool, bool, bool) {
    let CompleteOrdinaryPropertyDescriptor::Data {
        value,
        writable,
        enumerable,
        configurable,
    } = runtime
        .get_own_property(object, key)
        .unwrap()
        .expect("missing data descriptor")
    else {
        panic!("property was not a data descriptor");
    };
    (value, writable, enumerable, configurable)
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not a String property");
    };
    value.to_utf8_lossy()
}

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let Value::Int(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an Int property");
    };
    value
}

fn data_bits(writable: bool, enumerable: bool, configurable: bool) -> String {
    format!(
        "D:{}{}{}",
        Number(writable),
        Number(enumerable),
        Number(configurable)
    )
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
