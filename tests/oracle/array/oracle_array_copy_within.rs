use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, Runtime, RuntimeError,
    Value,
};

// This target pins QuickJS 2026-06-04 `Array.prototype.copyWithin`, including
// overlap direction and the hole-to-throwing-Delete path in `JS_CopySubArray`.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "non-overlapping copy runs forward and returns the receiver",
        r#"(function(){
            var source=[0,1,2,3,4],result=source.copyWithin(0,3);
            return (result===source)+"|"+source.length+"|"+source[0]+"|"+
                source[1]+"|"+source[2]+"|"+source[3]+"|"+source[4];
        })()"#,
    ),
    (
        "overlapping copy runs backward",
        r#"(function(){
            var source=[0,1,2,3,4],result=source.copyWithin(1,0,4);
            return (result===source)+"|"+source[0]+"|"+source[1]+"|"+
                source[2]+"|"+source[3]+"|"+source[4];
        })()"#,
    ),
    (
        "a source hole deletes the corresponding target property",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var source=[0,1,2,3];delete source[2];source.copyWithin(0,2,4);
            return source.length+"|"+own(source,0)+"|"+(source[0]===undefined)+"|"+
                own(source,1)+"|"+source[1]+"|"+own(source,2)+"|"+source[3];
        })()"#,
    ),
    (
        "an inherited source value is read and written as an own target",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var proto=Object(),source=Object();proto[1]="p";
            Object.setPrototypeOf(source,proto);source[0]="z";source.length=2;
            var result=Array.prototype.copyWithin.call(source,0,1,2);
            return (result===source)+"|"+source[0]+"|"+own(source,0)+"|"+
                source[1]+"|"+own(source,1);
        })()"#,
    ),
];

const BOUND_CASES: &[(&str, &str)] = &[
    (
        "target uses saturating Int64 conversion and negative-length offset",
        r#"(function(){
            function run(target){var a=[0,1,2,3];a.copyWithin(target,2,4);return ""+a[0]+a[1]+a[2]+a[3]}
            return run(undefined)+"|"+run(0/0)+"|"+run(-0)+"|"+run(1.9)+"|"+
                run(-1.9)+"|"+run(-9)+"|"+run(1/0)+"|"+run(-1/0)+"|"+run("1");
        })()"#,
    ),
    (
        "start and end use the pinned clamp and explicit undefined end means length",
        r#"(function(){
            function startRun(start){var a=[0,1,2,3];a.copyWithin(0,start);return ""+a[0]+a[1]+a[2]+a[3]}
            function endRun(end){var a=[0,1,2,3];a.copyWithin(0,2,end);return ""+a[0]+a[1]+a[2]+a[3]}
            return startRun(undefined)+"|"+startRun(1.9)+"|"+startRun(-1.9)+"|"+
                startRun(1/0)+"|"+startRun(-1/0)+"|"+endRun(undefined)+"|"+
                endRun(0/0)+"|"+endRun(3.9)+"|"+endRun(-1.9)+"|"+endRun(1/0);
        })()"#,
    ),
    (
        "zero length still converts target start and explicit end",
        r#"(function(){
            var log="",source=Object(),target=Object(),start=Object(),end=Object();
            source.__defineGetter__("length",function(){log+="L";return 0});
            target.valueOf=function(){log+="T";return 0};
            start.valueOf=function(){log+="S";return 0};
            end.valueOf=function(){log+="E";return 0};
            var result=Array.prototype.copyWithin.call(source,target,start,end);
            return (result===source)+"|"+log;
        })()"#,
    ),
    (
        "MAX_SAFE_INTEGER length supports a narrow high-index copy",
        r#"(function(){
            var source=Object();source.length=9007199254740991;
            source[9007199254740990]="x";
            var result=Array.prototype.copyWithin.call(
                source,9007199254740989,9007199254740990);
            return (result===source)+"|"+source[9007199254740989]+"|"+
                Object.prototype.hasOwnProperty.call(source,"9007199254740989");
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "length target start end and backward indexed operations use pinned order",
        r#"(function(){
            var log="",source=Object(),length=Object(),target=Object(),start=Object(),end=Object();
            source.__defineGetter__("length",function(){log+="L";return length});
            length.valueOf=function(){log+="N";return 3};
            target.valueOf=function(){log+="T";return 1};
            start.valueOf=function(){log+="S";return 0};
            end.valueOf=function(){log+="E";return 2};
            source.__defineGetter__("0",function(){log+="g0";return "a"});
            source.__defineGetter__("1",function(){log+="g1";return "b"});
            source.__defineSetter__("1",function(value){log+="s1"+value});
            source.__defineSetter__("2",function(value){log+="s2"+value});
            var result=Array.prototype.copyWithin.call(source,target,start,end);
            return (result===source)+"|"+log;
        })()"#,
    ),
    (
        "a backward Set failure preserves the earlier high-index write",
        r#"(function(){
            var source=[0,1,2],descriptor=Object();
            descriptor.value=1;descriptor.writable=false;descriptor.configurable=true;
            Object.defineProperty(source,"1",descriptor);
            try{source.copyWithin(1,0,2);return "missing"}
            catch(error){return source[0]+"|"+source[1]+"|"+source[2]+"|"+
                error.name+"|"+error.message}
        })()"#,
    ),
    (
        "a Delete failure preserves an earlier forward write",
        r#"(function(){
            var source=[0,1,9,3],descriptor=Object();delete source[3];
            descriptor.value=1;descriptor.writable=true;descriptor.configurable=false;
            Object.defineProperty(source,"1",descriptor);
            try{source.copyWithin(0,2,4);return "missing"}
            catch(error){return source[0]+"|"+source[1]+"|"+
                error.name+"|"+error.message}
        })()"#,
    ),
    (
        "a later source getter throw preserves the first backward write",
        r#"(function(){
            var source=Object();source.length=3;source[2]=2;
            source.__defineGetter__("0",function(){throw 73});
            source.__defineGetter__("1",function(){return 7});
            try{Array.prototype.copyWithin.call(source,1,0,2);return "missing"}
            catch(error){return source[2]+"|"+typeof error+"|"+error}
        })()"#,
    ),
    (
        "target and start throws short-circuit later conversions",
        r#"(function(){
            function run(which){
                var log="",source=Object(),target=Object(),start=Object(),end=Object();source.length=1;
                target.valueOf=function(){log+="T";if(which===0)throw 81;return 0};
                start.valueOf=function(){log+="S";if(which===1)throw 82;return 0};
                end.valueOf=function(){log+="E";return 1};
                try{Array.prototype.copyWithin.call(source,target,start,end);return "missing"}
                catch(error){return typeof error+":"+error+":"+log}
            }
            return run(0)+"|"+run(1);
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like receiver is mutated in place",
        r#"(function(){
            var source=Object();source[0]="a";source[1]="b";source[2]="c";source.length=3;
            var result=Array.prototype.copyWithin.call(source,0,1);
            return (result===source)+"|"+source[0]+"|"+source[1]+"|"+source[2]+"|"+source.length;
        })()"#,
    ),
    (
        "String indexed targets reject the first attempted Set",
        r#"(function(){
            try{Array.prototype.copyWithin.call("abc",0,1,2);return "missing"}
            catch(error){return error.name+"|"+error.message}
        })()"#,
    ),
    (
        "primitive receivers with zero length return their wrappers",
        r#"(function(){
            var number=Array.prototype.copyWithin.call(7,0,0);
            var boolean=Array.prototype.copyWithin.call(true,0,0);
            return Object.prototype.toString.call(number)+"|"+
                Object.prototype.toString.call(boolean)+"|"+typeof number+"|"+typeof boolean;
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("null receiver", "Array.prototype.copyWithin.call(null,0,0)"),
    (
        "undefined receiver",
        "Array.prototype.copyWithin.call(undefined,0,0)",
    ),
    ("Symbol target", "[0].copyWithin(Symbol('target'),0)"),
    ("Symbol start", "[0].copyWithin(0,Symbol('start'))"),
    ("Symbol end", "[0].copyWithin(0,0,Symbol('end'))"),
    ("BigInt target", "[0].copyWithin(0n,0)"),
    ("BigInt start", "[0].copyWithin(0,0n)"),
    ("BigInt end", "[0].copyWithin(0,0,1n)"),
    (
        "Symbol length",
        "(function(){var source=Object();source.length=Symbol('length');return Array.prototype.copyWithin.call(source,0,0)})()",
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','fill','indexOf','lastIndexOf','includes','copyWithin','values','keys','entries'];
var own=Reflect.ownKeys(Array.prototype),names=[];
for(var i=0;i<own.length;i++)
  if(implemented.indexOf(own[i])>=0)names[names.length]=own[i];
function bits(descriptor) {
  return 'D'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
var descriptor=Object.getOwnPropertyDescriptor(Array.prototype,'copyWithin'),fn=descriptor.value;
var constructable;
try { Reflect.construct(function(){},[],fn); constructable=true; }
catch(error) { constructable=false; }
print('keys='+names.join(','));
print('meta=copyWithin:'+fn.name+':'+fn.length+':'+bits(descriptor)+':'+
      bits(Object.getOwnPropertyDescriptor(fn,'name'))+':'+
      bits(Object.getOwnPropertyDescriptor(fn,'length'))+':'+
      (typeof fn==='function')+':'+(Object.getPrototypeOf(fn)===Function.prototype)+':'+constructable);
"#;

#[test]
fn array_copy_within_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.copyWithin oracle self-check: set QJS_ORACLE to upstream qjs");
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
fn array_copy_within_values_and_holes_match_pinned_quickjs() {
    compare_value_cases("Array.copyWithin values", VALUE_CASES);
}

#[test]
fn array_copy_within_bounds_match_pinned_quickjs() {
    compare_value_cases("Array.copyWithin bounds", BOUND_CASES);
}

#[test]
fn array_copy_within_order_and_partial_mutation_match_pinned_quickjs() {
    compare_value_cases("Array.copyWithin observable order", ORDER_CASES);
}

#[test]
fn array_copy_within_generic_receivers_match_pinned_quickjs() {
    compare_value_cases("Array.copyWithin generic receivers", GENERIC_CASES);
}

#[test]
fn array_copy_within_errors_match_pinned_quickjs() {
    compare_value_cases("Array.copyWithin errors", ERROR_CASES);
}

#[test]
fn array_copy_within_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.copyWithin graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array.copyWithin prototype order/metadata drifted",
    );
}

#[test]
fn array_copy_within_boxing_native_errors_and_user_throws_use_pinned_realms() {
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
    let method = property_callable(
        &runtime,
        &mut defining,
        &defining_array_prototype,
        "copyWithin",
    );

    let receiver = eval_object(&mut caller, "[10,20,30]", "caller Array receiver");
    let Value::Object(result) = caller
        .call(
            &method,
            Value::Object(receiver.clone()),
            &[Value::Int(0), Value::Int(1)],
        )
        .expect("cross-realm Array.copyWithin call")
    else {
        panic!("cross-realm Array.copyWithin result was not an object");
    };
    assert_eq!(
        result, receiver,
        "Array.copyWithin did not return its receiver"
    );
    assert_eq!(int_property(&runtime, &mut caller, &result, "0"), 20);
    assert_eq!(int_property(&runtime, &mut caller, &result, "1"), 30);

    let Value::Object(wrapper) = caller
        .call(&method, Value::Bool(true), &[Value::Int(0), Value::Int(0)])
        .expect("cross-realm primitive Array.copyWithin call")
    else {
        panic!("Array.copyWithin primitive result was not boxed");
    };
    assert_eq!(
        runtime.get_prototype_of(&wrapper).unwrap(),
        Some(defining_boolean_prototype),
        "Array.copyWithin boxed a primitive in the caller realm",
    );

    let readonly = eval_object(
        &mut caller,
        r#"(function(){
            var source=[0,1],descriptor=Object();
            descriptor.value=0;descriptor.writable=false;
            Object.defineProperty(source,"0",descriptor);return source;
        })()"#,
        "caller read-only copyWithin receiver",
    );
    assert!(matches!(
        caller.call(
            &method,
            Value::Object(readonly),
            &[Value::Int(0), Value::Int(1), Value::Int(2)],
        ),
        Err(RuntimeError::Exception),
    ));
    let set_error = take_exception_object(&mut caller, "Array.copyWithin Set TypeError");
    assert_eq!(
        runtime.get_prototype_of(&set_error).unwrap(),
        Some(defining_type_error.clone()),
        "Array.copyWithin Set TypeError did not use the method defining realm",
    );

    let undeletable = eval_object(
        &mut caller,
        r#"(function(){
            var source=[0,1],descriptor=Object();delete source[1];
            descriptor.value=0;descriptor.configurable=false;
            Object.defineProperty(source,"0",descriptor);return source;
        })()"#,
        "caller undeletable copyWithin receiver",
    );
    assert!(matches!(
        caller.call(
            &method,
            Value::Object(undeletable),
            &[Value::Int(0), Value::Int(1), Value::Int(2)],
        ),
        Err(RuntimeError::Exception),
    ));
    let delete_error = take_exception_object(&mut caller, "Array.copyWithin Delete TypeError");
    assert_eq!(
        runtime.get_prototype_of(&delete_error).unwrap(),
        Some(defining_type_error),
        "Array.copyWithin Delete TypeError did not use the method defining realm",
    );

    let throwing_receiver = eval_object(
        &mut caller,
        r#"(function(){
            var source=Object();source.length=1;
            source.__defineGetter__("0",function(){throw new TypeError("caller getter")});
            return source;
        })()"#,
        "caller throwing copyWithin receiver",
    );
    assert!(matches!(
        caller.call(
            &method,
            Value::Object(throwing_receiver),
            &[Value::Int(0), Value::Int(0), Value::Int(1)],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.copyWithin user getter error");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.copyWithin replaced a user getter throw with a defining-realm error",
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
    vec![
        format!("keys={}", names.join(",")),
        format!(
            "meta={}",
            method_metadata(
                &runtime,
                &mut context,
                &array_prototype,
                &function_prototype,
                "copyWithin",
            )
        ),
    ]
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS Array.copyWithin graph oracle: {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS Array.copyWithin graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array.copyWithin graph output was not UTF-8")
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
        .expect("Array.copyWithin name descriptor");
    let length_descriptor = runtime
        .get_own_property(function, &runtime.intern_property_key("length").unwrap())
        .unwrap()
        .expect("Array.copyWithin length descriptor");
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
