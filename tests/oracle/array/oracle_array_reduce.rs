use super::support::*;

use std::ffi::OsStr;

use quickjs_oxide::{CallableRef, Context, JsString, Runtime, RuntimeError, Value};

// This target pins QuickJS 2026-06-04's shared `js_array_reduce` kernel:
// reduce and reduceRight, including their omitted-initial-value scan.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "reduce and reduceRight fold in opposite directions",
        r#"(function(){
            var source=[1,2,3],forward="",reverse="";
            var left=source.reduce(function(acc,value,index){
                forward+=index;return acc*10+value;
            },0);
            var right=source.reduceRight(function(acc,value,index){
                reverse+=index;return acc*10+value;
            },0);
            return left+"|"+right+"|"+forward+"|"+reverse;
        })()"#,
    ),
    (
        "omitted initial value is selected without a callback",
        r#"(function(){
            var source=[1,2,3],left="",right="";
            var forward=source.reduce(function(acc,value,index){
                left+=index;return acc+value;
            });
            var reverse=source.reduceRight(function(acc,value,index){
                right+=index;return acc+value;
            });
            return forward+"|"+reverse+"|"+left+"|"+right;
        })()"#,
    ),
    (
        "explicit undefined is an initial value even for an empty receiver",
        r#"(function(){
            var calls=0,callback=function(acc,value){
                calls++;return (acc===undefined?"undefined":acc)+value;
            };
            var empty=[].reduce(callback,undefined);
            var one=[5].reduce(callback,undefined);
            return (empty===undefined)+"|"+one+"|"+calls;
        })()"#,
    ),
    (
        "a single present element needs no callback when initial value is omitted",
        r#"(function(){
            var source=Array(4),calls=0;source[2]=7;
            var left=source.reduce(function(){calls++;return 0});
            var right=source.reduceRight(function(){calls++;return 0});
            return left+"|"+right+"|"+calls;
        })()"#,
    ),
    (
        "a present undefined element is still a valid omitted accumulator",
        r#"(function(){
            var source=Array(3),calls=0;source[1]=undefined;
            var left=source.reduce(function(){calls++;return 1});
            var right=source.reduceRight(function(){calls++;return 2});
            return (left===undefined)+"|"+(right===undefined)+"|"+calls;
        })()"#,
    ),
    (
        "each callback result becomes the next accumulator without coercion",
        r#"(function(){
            var result=[1,2,3].reduce(function(acc,value,index){
                return "("+acc+":"+value+":"+index+")";
            },"x");
            return result;
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "holes are skipped while inherited values participate",
        r#"(function(){
            var proto=Object(),source=Object(),log="";proto[1]=4;
            Object.setPrototypeOf(source,proto);source[3]=8;source.length=5;
            var left=Array.prototype.reduce.call(source,function(acc,value,index){
                log+="l"+index+":"+value+",";return acc+value;
            },1);
            var right=Array.prototype.reduceRight.call(source,function(acc,value,index){
                log+="r"+index+":"+value+",";return acc+value;
            },1);
            return left+"|"+right+"|"+log;
        })()"#,
    ),
    (
        "length is snapshotted while later HasProperty observes mutation",
        r#"(function(){
            var source=[1,2,3],log="";
            var result=source.reduce(function(acc,value,index){
                log+=index+":"+value+",";
                if(index===0){delete source[1];source[3]=4;source.length=4}
                return acc+value;
            },0);
            return result+"|"+log+"|"+source.length+"|"+source[3];
        })()"#,
    ),
    (
        "callback accumulator value index receiver and strict this are exact",
        r#"(function(){
            var source=[5],initial=Object(),log="";
            var result=source.reduce(function(acc,value,index,receiver){
                "use strict";
                log+=(this===undefined)+":"+(acc===initial)+":"+value+":"+index+":"+
                    (receiver===source);
                return receiver;
            },initial);
            return (result===source)+"|"+log;
        })()"#,
    ),
    (
        "omitted initial scan and callback follow directional Get order",
        r#"(function(){
            var source=Object(),log="";source.length=4;
            source.__defineGetter__("1",function(){log+="g1,";return 2});
            source.__defineGetter__("3",function(){log+="g3,";return 4});
            var left=Array.prototype.reduce.call(source,function(acc,value,index){
                log+="c"+index+",";return acc+value;
            });
            log+="|";
            var right=Array.prototype.reduceRight.call(source,function(acc,value,index){
                log+="c"+index+",";return acc+value;
            });
            return left+"|"+right+"|"+log;
        })()"#,
    ),
    (
        "length access precedes callback validation",
        r#"(function(){
            var source=Object(),log="";
            source.__defineGetter__("length",function(){log+="L";return 0});
            try{Array.prototype.reduce.call(source,7);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "a value getter throw precedes callback invocation",
        r#"(function(){
            var source=Object(),log="";source.length=1;
            source.__defineGetter__("0",function(){log+="G";throw 71});
            try{Array.prototype.reduce.call(source,function(){log+="C";return 0},0);return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log}
        })()"#,
    ),
    (
        "a callback throw stops before later indexed access",
        r#"(function(){
            var source=[1,2],log="";
            source.__defineGetter__("1",function(){log+="G";throw 72});
            try{source.reduce(function(){log+="C";throw 73},0);return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log}
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like receivers pass their object identity",
        r#"(function(){
            var source=Object();source[0]="a";source[2]="c";source.length=3;
            return Array.prototype.reduce.call(source,function(acc,value,index,receiver){
                return acc+(receiver===source)+":"+index+":"+value+",";
            },"");
        })()"#,
    ),
    (
        "String receivers expose UTF-16 units through a boxed receiver",
        r#"(function(){
            var source="A\uD83D\uDCA9Z",boxed=true,indices="";
            var sum=Array.prototype.reduce.call(source,function(acc,value,index,receiver){
                boxed=boxed&&typeof receiver==="object"&&receiver!==source;
                indices+=index;return acc+value.charCodeAt(0);
            },0);
            return sum+"|"+indices+"|"+boxed;
        })()"#,
    ),
    (
        "zero-length primitive receivers return an explicit initial value",
        r#"(function(){
            var calls=0,callback=function(){calls++;return 0};
            var number=Array.prototype.reduce.call(7,callback,"n");
            var boolean=Array.prototype.reduceRight.call(false,callback,"b");
            return number+"|"+boolean+"|"+calls;
        })()"#,
    ),
    (
        "MAX_SAFE_INTEGER endpoints can abort after one callback",
        r#"(function(){
            var source=Object(),log="";source.length=9007199254740991;
            source[0]="l";source[9007199254740990]="r";
            try{Array.prototype.reduce.call(source,function(acc,value,index){
                log+="L"+index+value;throw 81;
            },0)}catch(error){log+="="+error}
            try{Array.prototype.reduceRight.call(source,function(acc,value,index){
                log+="R"+index+value;throw 82;
            },0)}catch(error){log+="="+error}
            return log;
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "null receiver",
        "Array.prototype.reduce.call(null,function(){return 0},0)",
    ),
    (
        "undefined receiver",
        "Array.prototype.reduceRight.call(undefined,function(){return 0},0)",
    ),
    ("missing callback", "[1].reduce()"),
    ("undefined callback", "[1].reduceRight(undefined)"),
    ("number callback", "[].reduce(1)"),
    ("symbol callback", "[1].reduce(Symbol('callback'))"),
    ("BigInt callback", "[1].reduceRight(0n)"),
    (
        "Symbol length wins before callback validation",
        "(function(){var source=Object();source.length=Symbol('length');return Array.prototype.reduce.call(source,1)})()",
    ),
    (
        "empty reduce without initial value",
        "[].reduce(function(){})",
    ),
    (
        "empty reduceRight without initial value",
        "[].reduceRight(function(){})",
    ),
    (
        "all-hole receiver without initial value",
        "Array(3).reduce(function(){})",
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','every','some','forEach','reduce','reduceRight','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','copyWithin','values','keys','entries'];
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
print('meta='+metadata('reduce'));
print('meta='+metadata('reduceRight'));
"#;

#[test]
fn array_reduce_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.reduce oracle self-check: set QJS_ORACLE to upstream qjs");
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
    assert_eq!(oracle_graph_observations(&oracle).len(), 3);
}

#[test]
fn array_reduce_values_match_pinned_quickjs() {
    compare_value_cases("Array.reduce values", VALUE_CASES);
}

#[test]
fn array_reduce_holes_order_and_abrupt_completion_match_pinned_quickjs() {
    compare_value_cases("Array.reduce observable order", ORDER_CASES);
}

#[test]
fn array_reduce_generic_receivers_match_pinned_quickjs() {
    compare_value_cases("Array.reduce generic receivers", GENERIC_CASES);
}

#[test]
fn array_reduce_errors_match_pinned_quickjs() {
    compare_value_cases("Array.reduce errors", ERROR_CASES);
}

#[test]
fn array_reduce_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.reduce graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array.reduce prototype order/metadata drifted",
    );
}

#[test]
fn array_reduce_boxing_accumulators_and_errors_use_pinned_realms() {
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
    let caller_object_prototype = caller.object_prototype().unwrap();
    let reduce = property_callable(&runtime, &mut defining, &defining_array_prototype, "reduce");

    let capture = eval_callable(
        &runtime,
        &mut caller,
        "(function(acc,value,index,receiver){globalThis.reduceReceiver=receiver;return acc+value})",
        "caller receiver-capturing reduce callback",
    );
    assert_eq!(
        caller
            .call(
                &reduce,
                Value::String(JsString::try_from_utf8("a").unwrap()),
                &[
                    Value::Object(capture.as_object().clone()),
                    Value::String(JsString::try_from_utf8("").unwrap()),
                ],
            )
            .expect("cross-realm primitive Array.reduce call"),
        Value::String(JsString::try_from_utf8("a").unwrap()),
    );
    let captured = eval_object(&mut caller, "reduceReceiver", "captured reduce receiver");
    assert_eq!(
        runtime.get_prototype_of(&captured).unwrap(),
        Some(defining_string_prototype),
        "Array.reduce boxed its callback receiver in the caller realm",
    );

    let empty = eval_object(&mut caller, "[]", "caller empty Array");
    assert!(matches!(
        caller.call(&reduce, Value::Object(empty.clone()), &[Value::Int(0)]),
        Err(RuntimeError::Exception),
    ));
    let invalid_callback = take_exception_object(&mut caller, "Array.reduce callback TypeError");
    assert_eq!(
        runtime.get_prototype_of(&invalid_callback).unwrap(),
        Some(defining_type_error.clone()),
        "Array.reduce callback TypeError did not use the method defining realm",
    );

    let identity = eval_callable(
        &runtime,
        &mut caller,
        "(function(acc){return acc})",
        "caller identity reduce callback",
    );
    assert!(matches!(
        caller.call(
            &reduce,
            Value::Object(empty.clone()),
            &[Value::Object(identity.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let empty_error = take_exception_object(&mut caller, "Array.reduce empty TypeError");
    assert_eq!(
        runtime.get_prototype_of(&empty_error).unwrap(),
        Some(defining_type_error),
        "Array.reduce empty TypeError did not use the method defining realm",
    );

    let marker = eval_object(&mut caller, "Object()", "caller initial accumulator");
    assert_eq!(
        caller
            .call(
                &reduce,
                Value::Object(empty),
                &[
                    Value::Object(identity.as_object().clone()),
                    Value::Object(marker.clone()),
                ],
            )
            .expect("empty reduce with explicit accumulator"),
        Value::Object(marker),
        "Array.reduce replaced an untouched caller-realm accumulator",
    );

    let throwing = eval_callable(
        &runtime,
        &mut caller,
        "(function(){throw new TypeError('caller callback')})",
        "caller throwing reduce callback",
    );
    let one = eval_object(&mut caller, "[1]", "caller one-element Array");
    assert!(matches!(
        caller.call(
            &reduce,
            Value::Object(one.clone()),
            &[Value::Object(throwing.as_object().clone()), Value::Int(0)],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.reduce user callback error");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.reduce replaced a user callback throw with a defining-realm error",
    );

    let make_object = eval_callable(
        &runtime,
        &mut caller,
        "(function(){return Object()})",
        "caller object-producing reduce callback",
    );
    let Value::Object(produced) = caller
        .call(
            &reduce,
            Value::Object(one),
            &[
                Value::Object(make_object.as_object().clone()),
                Value::Int(0),
            ],
        )
        .expect("cross-realm callback-produced accumulator")
    else {
        panic!("Array.reduce callback did not return its object accumulator");
    };
    assert_eq!(
        runtime.get_prototype_of(&produced).unwrap(),
        Some(caller_object_prototype),
        "Array.reduce moved a callback-produced accumulator into the defining realm",
    );
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
        "reduce",
        "reduceRight",
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
    for name in ["reduce", "reduceRight"] {
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
    super::quickjs_oracle::eval_std_lines(oracle, GRAPH_ORACLE, "Array.reduce graph oracle")
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
