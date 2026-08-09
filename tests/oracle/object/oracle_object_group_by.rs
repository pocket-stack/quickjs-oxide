use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, PropertyKey, Runtime,
    RuntimeError, Value,
};

// Pins the QuickJS 2026-06-04 `Object.groupBy` table entry and the shared
// `js_object_groupBy(..., is_map = 0)` iterator kernel.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "basic grouping uses a null-prototype object and genuine Arrays",
        r#"(function(){
            var result=Object.groupBy([1,2,3,4],function(value,index){
                return value%2 ? "odd" : "even";
            });
            return (Object.getPrototypeOf(result)===null)+"|"+
                Object.getOwnPropertyNames(result).join(",")+"|"+
                result.odd.join(",")+"|"+result.even.join(",")+"|"+
                Array.isArray(result.odd)+"|"+
                (Object.getPrototypeOf(result.odd)===Array.prototype);
        })()"#,
    ),
    (
        "property-key conversion supports symbols objects and proto spelling",
        r#"(function(){
            var symbol=Symbol("bucket"),key=Object(),log="";
            key.toString=function(){log+="k";return "__proto__"};
            var result=Object.groupBy([1,2,3],function(value){
                return value===2 ? symbol : key;
            });
            var symbols=Object.getOwnPropertySymbols(result);
            return log+"|"+Object.getOwnPropertyNames(result).join(",")+"|"+
                (symbols.length===1&&symbols[0]===symbol)+"|"+
                result.__proto__.join(",")+"|"+result[symbol].join(",");
        })()"#,
    ),
    (
        "string iteration groups Unicode code points rather than UTF-16 halves",
        r#"(function(){
            var result=Object.groupBy("a𝄞a",function(value){return value});
            return Object.getOwnPropertyNames(result).join("|")+"|"+
                result.a.length+"|"+result["𝄞"].length+"|"+result["𝄞"][0];
        })()"#,
    ),
    (
        "numeric property keys retain ordinary own-key ordering",
        r#"(function(){
            var result=Object.groupBy([0,1,2,3],function(value){
                return value===0 ? 2 : value===1 ? 1 : value===2 ? -0 : "01";
            });
            return Object.getOwnPropertyNames(result).join("|")+"|"+
                result[0]+"|"+result[1]+"|"+result[2]+"|"+result["01"];
        })()"#,
    ),
    (
        "callback receives the defining global object value and monotonic index",
        r#"(function(){
            var log="";
            var result=Object.groupBy([4,5],function(value,index){
                "use strict";
                log+=(this===globalThis)+":"+value+":"+index+";";
                return "all";
            });
            return log+"|"+result.all.join(",");
        })()"#,
    ),
    (
        "group properties are writable enumerable and configurable",
        r#"(function(){
            var result=Object.groupBy([1],function(){return "x"});
            var enumerable=Object.prototype.propertyIsEnumerable.call(result,"x");
            result.x=9;
            var writable=result.x===9,deleted=delete result.x;
            return enumerable+"|"+writable+"|"+deleted+"|"+("x" in result);
        })()"#,
    ),
    (
        "ordinary recursive callback reentry completes below the host-safe ceiling",
        r#"(function(){
            var count=0;
            function group(depth){
                Object.groupBy([depth],function(){
                    count++;
                    if(depth)group(depth-1);
                    return "x";
                });
            }
            group(4);
            return count;
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "callback validation precedes access to the iterable",
        r#"(function(){
            var log="",iterable=Object();
            iterable.__defineGetter__(Symbol.iterator,function(){log+="iterator;";throw "touched"});
            try{Object.groupBy(iterable,0)}catch(error){
                return log+error.name+":"+error.message;
            }
            return "missing";
        })()"#,
    ),
    (
        "iterator next is cached once and each step precedes callback and key conversion",
        r#"(function(){
            var log="",count=0,iterable=Object(),iterator=Object(),key=Object();
            key.toString=function(){log+="key;";return "g"};
            iterable.__defineGetter__(Symbol.iterator,function(){
                log+="iterator-get;";
                return function(){log+="iterator-call;";return iterator};
            });
            iterator.__defineGetter__("next",function(){
                log+="next-get;";
                return function(){
                    log+="next-call;";
                    if(count++){
                        var finished=Object();finished.done=true;return finished;
                    }
                    var result=Object();
                    result.__defineGetter__("done",function(){log+="done;";return false});
                    result.__defineGetter__("value",function(){log+="value;";return 7});
                    return result;
                };
            });
            var grouped=Object.groupBy(iterable,function(value,index){
                log+="callback:"+value+":"+index+";";
                iterator.next=function(){throw "changed"};
                return key;
            });
            return log+"|"+grouped.g.join(",");
        })()"#,
    ),
    (
        "callback and key conversion failures close while preserving the original throw",
        r#"(function(){
            function run(mode){
                var log="",iterable=Object(),iterator=Object();
                iterable[Symbol.iterator]=function(){log+="iterator;";return iterator};
                iterator.next=function(){
                    log+="next;";
                    var result=Object();result.done=false;result.value=1;return result;
                };
                iterator.return=function(){log+="return;";throw "close"};
                try{
                    Object.groupBy(iterable,function(){
                        log+="callback;";
                        if(mode===0)throw "callback";
                        var key=Object();
                        key.toString=function(){log+="key;";throw "key"};
                        return key;
                    });
                }catch(error){return log+"throw:"+error}
                return "missing";
            }
            return run(0)+"|"+run(1);
        })()"#,
    ),
    (
        "next done and value failures do not close the iterator",
        r#"(function(){
            function run(mode){
                var log="",iterable=Object(),iterator=Object(),result=Object();
                iterable[Symbol.iterator]=function(){log+="iterator;";return iterator};
                iterator.next=function(){
                    log+="next;";
                    if(mode===0)throw "next";
                    return result;
                };
                iterator.return=function(){log+="return;"};
                result.__defineGetter__("done",function(){
                    log+="done;";if(mode===1)throw "done";return false;
                });
                result.__defineGetter__("value",function(){
                    log+="value;";throw "value";
                });
                try{Object.groupBy(iterable,function(){return "x"})}
                catch(error){return log+"throw:"+error}
                return "missing";
            }
            return run(0)+"|"+run(1)+"|"+run(2);
        })()"#,
    ),
    (
        "a throwing next getter does not close the iterator",
        r#"(function(){
            var log="",iterable=Object(),iterator=Object();
            iterable[Symbol.iterator]=function(){return iterator};
            iterator.__defineGetter__("next",function(){log+="next-get;";throw "next-get"});
            iterator.return=function(){log+="return;"};
            try{Object.groupBy(iterable,function(){return "x"})}
            catch(error){return log+"throw:"+error}
            return "missing";
        })()"#,
    ),
    (
        "group append uses Array push Set semantics",
        r#"(function(){
            var source=[9],log="";
            Array.prototype.__defineSetter__("0",function(value){log+="set0:"+value});
            try{
                var result=Object.groupBy(source,function(){return "x"});
                return result.x.length+"|"+result.x.hasOwnProperty("0")+"|"+
                    result.x[0]+"|"+log;
            }finally{delete Array.prototype[0]}
        })()"#,
    ),
    (
        "an internal Array push failure does not close the iterator",
        r#"(function(){
            var log="",iterable=Object(),iterator=Object();
            iterable[Symbol.iterator]=function(){return iterator};
            iterator.next=function(){
                log+="next;";
                var result=Object();result.done=false;result.value=9;return result;
            };
            iterator.return=function(){log+="return;"};
            try{
                Object.groupBy(iterable,function(){
                    log+="callback;";
                    var descriptor=Object();
                    descriptor.value=1;descriptor.writable=false;descriptor.configurable=true;
                    Object.defineProperty(Array.prototype,"0",descriptor);
                    return "x";
                });
            }catch(error){return log+"throw:"+error.name+":"+error.message}
            finally{delete Array.prototype[0]}
            return "missing";
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("missing callback", "Object.groupBy([])"),
    (
        "undefined iterable",
        "Object.groupBy(undefined,function(){})",
    ),
    ("null iterable", "Object.groupBy(null,function(){})"),
    ("non-iterable primitive", "Object.groupBy(1,function(){})"),
    (
        "non-callable iterator method",
        r#"(function(){var value=Object();value[Symbol.iterator]=1;return Object.groupBy(value,function(){})})()"#,
    ),
    (
        "iterator method returns a primitive",
        r#"(function(){var value=Object();value[Symbol.iterator]=function(){return 1};return Object.groupBy(value,function(){})})()"#,
    ),
    (
        "next is not callable",
        r#"(function(){
            var value=Object();
            value[Symbol.iterator]=function(){var iterator=Object();iterator.next=1;return iterator};
            return Object.groupBy(value,function(){});
        })()"#,
    ),
    (
        "next returns a primitive",
        r#"(function(){
            var value=Object();
            value[Symbol.iterator]=function(){
                var iterator=Object();iterator.next=function(){return 1};return iterator;
            };
            return Object.groupBy(value,function(){});
        })()"#,
    ),
    (
        "groupBy is not a constructor",
        "new Object.groupBy([],function(){return 0})",
    ),
];

const GRAPH_ORACLE: &str = r#"
function bits(descriptor) {
  return 'D:'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
function isConstructor(value) {
  try { Reflect.construct(function(){}, [], value); return true; }
  catch (_) { return false; }
}
function callableMeta(value) {
  return value.name+':'+value.length+':' +
    (Object.getPrototypeOf(value)===Function.prototype)+':' +
    (typeof value==='function')+':'+isConstructor(value)+':' +
    Object.getOwnPropertyNames(value).join(',');
}
var selected=['length','name','prototype','create','getPrototypeOf','setPrototypeOf',
  'defineProperty','defineProperties','getOwnPropertyNames','getOwnPropertySymbols','groupBy'];
print('ctor-prefix='+Reflect.ownKeys(Object).filter(function(key){
  return selected.indexOf(key)>=0;
}).map(String).join(','));
print('groupBy='+callableMeta(Object.groupBy)+':' +
  bits(Object.getOwnPropertyDescriptor(Object,'groupBy')));
print('length='+bits(Object.getOwnPropertyDescriptor(Object.groupBy,'length')));
print('name='+bits(Object.getOwnPropertyDescriptor(Object.groupBy,'name')));
"#;

#[test]
fn object_group_by_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.groupBy oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("order", ORDER_CASES),
        ("errors", ERROR_CASES),
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
fn object_group_by_values_match_pinned_quickjs() {
    compare_cases("Object.groupBy values", VALUE_CASES);
}

#[test]
fn object_group_by_order_and_iterator_close_match_pinned_quickjs() {
    compare_cases("Object.groupBy order", ORDER_CASES);
}

#[test]
fn object_group_by_errors_match_pinned_quickjs() {
    compare_cases("Object.groupBy errors", ERROR_CASES);
}

#[test]
fn object_group_by_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.groupBy graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Object.groupBy graph or metadata drifted",
    );
}

#[test]
fn object_group_by_uses_its_defining_realm_and_preserves_user_throws() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let object = global_callable(&runtime, &mut defining, "Object");
    let group_by = property_callable(&runtime, &mut defining, object.as_object(), "groupBy");
    let source = eval_object(&mut caller, "[1,2]");
    let callback = eval_callable(
        &runtime,
        &mut caller,
        r#"(function(value){
            "use strict";
            this.groupByMarker=(this.groupByMarker||0)+value;
            return "x";
        })"#,
    );

    let Value::Object(groups) = caller
        .call(
            &group_by,
            Value::Undefined,
            &[
                Value::Object(source),
                Value::Object(callback.as_object().clone()),
            ],
        )
        .expect("cross-realm Object.groupBy")
    else {
        panic!("Object.groupBy did not return an object");
    };
    assert_eq!(runtime.get_prototype_of(&groups).unwrap(), None);
    let group = object_property(&runtime, &mut caller, &groups, "x");
    assert_eq!(
        runtime.get_prototype_of(&group).unwrap().as_ref(),
        Some(&defining.array_prototype().unwrap()),
        "group Array did not use Object.groupBy's defining realm",
    );
    let defining_global = defining.global_object().unwrap();
    assert_eq!(
        int_property(&runtime, &mut defining, &defining_global, "groupByMarker",),
        3,
    );
    let caller_global = caller.global_object().unwrap();
    assert!(matches!(
        caller
            .get_property(
                &caller_global,
                &runtime.intern_property_key("groupByMarker").unwrap(),
            )
            .unwrap(),
        Value::Undefined
    ));

    let defining_type_error = eval_object(&mut defining, "TypeError.prototype");
    let empty = caller.new_array().unwrap();
    assert!(matches!(
        caller.call(
            &group_by,
            Value::Undefined,
            &[Value::Object(empty), Value::Int(0)],
        ),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = caller
        .take_exception()
        .unwrap()
        .expect("missing Object.groupBy native error")
    else {
        panic!("Object.groupBy native error was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap().as_ref(),
        Some(&defining_type_error),
        "native callback validation error used the caller realm",
    );

    let sentinel = caller.new_object().unwrap();
    let sentinel_key = runtime.intern_property_key("groupBySentinel").unwrap();
    assert!(
        caller
            .set_property(
                &caller.global_object().unwrap(),
                &sentinel_key,
                Value::Object(sentinel.clone()),
            )
            .unwrap()
    );
    let throwing = eval_callable(&runtime, &mut caller, "(function(){throw groupBySentinel})");
    let singleton = caller.new_array_from_values(vec![Value::Int(1)]).unwrap();
    assert!(matches!(
        caller.call(
            &group_by,
            Value::Undefined,
            &[
                Value::Object(singleton),
                Value::Object(throwing.as_object().clone()),
            ],
        ),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "callback throw identity was not preserved",
    );
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
                    string_property(runtime, context, &error, "name"),
                    string_property(runtime, context, &error, "message"),
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
    ];
    let prefix = own_key_names(&runtime, object.as_object())
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let key = runtime.intern_property_key("groupBy").unwrap();
    let descriptor = data_descriptor(&runtime, object.as_object(), &key);
    let Value::Object(function) = descriptor.0 else {
        panic!("Object.groupBy was not an object");
    };
    assert!(runtime.as_callable(&function).unwrap().is_some());
    let length = data_descriptor(
        &runtime,
        &function,
        &runtime.intern_property_key("length").unwrap(),
    );
    let name = data_descriptor(
        &runtime,
        &function,
        &runtime.intern_property_key("name").unwrap(),
    );
    vec![
        format!("ctor-prefix={prefix}"),
        format!(
            "groupBy={}:{}:{}:{}:{}:{}:D:{}{}{}",
            string_property(&runtime, &mut context, &function, "name"),
            int_property(&runtime, &mut context, &function, "length"),
            runtime.get_prototype_of(&function).unwrap().as_ref() == Some(&function_prototype),
            runtime.as_callable(&function).unwrap().is_some(),
            runtime.is_constructor(&function).unwrap(),
            own_key_names(&runtime, &function).join(","),
            Number(descriptor.1),
            Number(descriptor.2),
            Number(descriptor.3),
        ),
        format!("length={}", data_bits(length.1, length.2, length.3)),
        format!("name={}", data_bits(name.1, name.2, name.3)),
    ]
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS Object.groupBy graph: {error}"));
    assert!(
        output.status.success(),
        "QuickJS Object.groupBy graph failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Object.groupBy graph output was not UTF-8")
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

fn eval_callable(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("{source:?} was not callable"))
}

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    object
}

fn object_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> ObjectRef {
    let Value::Object(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an object property");
    };
    value
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

fn data_bits(writable: bool, enumerable: bool, configurable: bool) -> String {
    format!(
        "D:{}{}{}",
        Number(writable),
        Number(enumerable),
        Number(configurable),
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
