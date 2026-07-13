use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, PropertyKey,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_object_fromEntries`. In particular, the
// implementation allocates a defining-realm ordinary result, acquires and
// caches `next`, reads entry[0] before entry[1], converts the key only after
// both reads, and routes every abrupt completion after iterator acquisition
// through `JS_IteratorClose(..., TRUE)`.
//
// Proxy, Map/Set, TypedArray, generator and module-namespace integration are
// recorded as explicit boundaries below; they do not silently weaken the
// ordinary iterator differential while those intrinsics remain unpublished.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "basic entries create a fresh ordinary object with data properties",
        r#"(function(){
            function bits(descriptor){
                return descriptor.writable+":"+descriptor.enumerable+":"+descriptor.configurable;
            }
            var first=Object.fromEntries.call(17,[["a",1],["b",2]]);
            var second=Object.fromEntries([["a",1]]);
            return (Object.getPrototypeOf(first)===Object.prototype)+"|"+
                (first!==second)+"|"+Object.getOwnPropertyNames(first).join(",")+"|"+
                first.a+":"+first.b+"|"+bits(Object.getOwnPropertyDescriptor(first,"a"));
        })()"#,
    ),
    (
        "duplicate keys overwrite without moving while integer keys stay canonical",
        r#"(function(){
            var result=Object.fromEntries([["z",1],[10,2],["2",3],["a",4],["z",5]]);
            return Object.getOwnPropertyNames(result).join(",")+"|"+
                result[2]+":"+result[10]+":"+result.z+":"+result.a;
        })()"#,
    ),
    (
        "Symbol and proto-spelled keys are direct own data properties",
        r#"(function(){
            var symbol=Symbol("entry"),payload=Object(),prototypeValue=Object();
            payload.kept=9;
            var result=Object.fromEntries([[symbol,payload],["__proto__",prototypeValue]]);
            var symbols=Object.getOwnPropertySymbols(result);
            var descriptor=Object.getOwnPropertyDescriptor(result,"__proto__");
            return (Object.getPrototypeOf(result)===Object.prototype)+"|"+
                (symbols.length===1&&symbols[0]===symbol)+"|"+(result[symbol]===payload)+"|"+
                (result.__proto__===prototypeValue)+"|"+result.hasOwnProperty("__proto__")+"|"+
                descriptor.writable+":"+descriptor.enumerable+":"+descriptor.configurable;
        })()"#,
    ),
    (
        "each yielded entry may be any object with array-like zero and one properties",
        r#"(function(){
            var arrayLike=Object();arrayLike[0]="arrayLike";arrayLike[1]=7;arrayLike[2]="ignored";
            var wrapped=Object("xy"),missing=["missing"],result;
            result=Object.fromEntries([wrapped,arrayLike,["extra",9,10],missing]);
            return Object.getOwnPropertyNames(result).join(",")+"|"+
                result.x+":"+result.arrayLike+":"+result.extra+":"+(result.missing===undefined);
        })()"#,
    ),
    (
        "missing entry fields become undefined and numeric keys use ToPropertyKey",
        r#"(function(){
            var empty=[],negativeZero=[-0,"zero"],nan=[NaN,"nan"];
            var result=Object.fromEntries([empty,negativeZero,nan]);
            return Object.getOwnPropertyNames(result).join(",")+"|"+
                (result.undefined===undefined)+":"+result[0]+":"+result.NaN;
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "iterator acquisition caches next and reads done value zero one then key",
        r#"(function(){
            var log="",calls=0,iterable=Object(),iterator=Object(),entry=Object(),key=Object();
            key[Symbol.toPrimitive]=function(hint){log+="key:"+hint+";";return "x"};
            entry.__defineGetter__("0",function(){log+="get-0;";return key});
            entry.__defineGetter__("1",function(){
                log+="get-1;";iterator.next=function(){throw "changed"};return 42;
            });
            iterable.__defineGetter__(Symbol.iterator,function(){
                log+="iterator-get;";
                return function(){log+="iterator-call;";return iterator};
            });
            iterator.__defineGetter__("next",function(){
                log+="next-get;";
                return function(){
                    log+="next-call;";
                    var step=Object();
                    if(calls++){
                        step.__defineGetter__("done",function(){log+="done:true;";return true});
                        step.__defineGetter__("value",function(){throw "finished value was read"});
                    }else{
                        step.__defineGetter__("done",function(){log+="done:false;";return false});
                        step.__defineGetter__("value",function(){log+="value;";return entry});
                    }
                    return step;
                };
            });
            iterator.return=function(){log+="return;"};
            var result=Object.fromEntries(iterable);
            return log+"|"+result.x+"|"+Object.getOwnPropertyNames(result).join(",");
        })()"#,
    ),
    (
        "CreateDataProperty bypasses an inherited setter",
        r#"(function(){
            var log="";
            Object.prototype.__defineSetter__("fromEntriesCreate",function(value){log+="set:"+value});
            try{
                var result=Object.fromEntries([["fromEntriesCreate",8]]);
                var descriptor=Object.getOwnPropertyDescriptor(result,"fromEntriesCreate");
                return log+"|"+result.fromEntriesCreate+"|"+result.hasOwnProperty("fromEntriesCreate")+"|"+
                    descriptor.writable+":"+descriptor.enumerable+":"+descriptor.configurable;
            }finally{delete Object.prototype.fromEntriesCreate}
        })()"#,
    ),
    (
        "iterator acquisition failures occur before an iterator can be closed",
        r#"(function(){
            function text(error){return typeof error+":"+error}
            function run(mode){
                var log="",iterable=Object();
                iterable.__defineGetter__(Symbol.iterator,function(){
                    log+="iterator-get;";
                    if(mode===0)throw "iterator-get";
                    return function(){log+="iterator-call;";if(mode===1)throw "iterator-call";return 1};
                });
                try{Object.fromEntries(iterable)}catch(error){return log+"throw:"+text(error)}
                return "missing";
            }
            return run(0)+"|"+run(1)+"|"+run(2);
        })()"#,
    ),
    (
        "next getter call result done and value failures all close the iterator",
        r#"(function(){
            function text(error){return typeof error+":"+error}
            function run(mode){
                var log="",iterable=Object(),iterator=Object(),step=Object();
                iterable[Symbol.iterator]=function(){log+="iterator;";return iterator};
                iterator.return=function(){log+="return;";throw "close"};
                if(mode===0){
                    iterator.__defineGetter__("next",function(){log+="next-get;";throw "next-get"});
                }else{
                    iterator.next=function(){
                        log+="next-call;";
                        if(mode===1)throw "next-call";
                        if(mode===2)return 1;
                        step.__defineGetter__("done",function(){
                            log+="done;";if(mode===3)throw "done";return false;
                        });
                        step.__defineGetter__("value",function(){
                            log+="value;";if(mode===4)throw "value";return 1;
                        });
                        return step;
                    };
                }
                try{Object.fromEntries(iterable)}catch(error){return log+"throw:"+text(error)}
                return "missing";
            }
            return run(0)+"|"+run(1)+"|"+run(2)+"|"+run(3)+"|"+run(4)+"|"+run(5);
        })()"#,
    ),
    (
        "entry zero one and property-key failures close after completed reads",
        r#"(function(){
            function text(error){return typeof error+":"+error}
            function run(mode){
                var log="",iterable=Object(),iterator=Object(),step=Object(),entry=Object(),key=Object();
                iterable[Symbol.iterator]=function(){return iterator};
                iterator.next=function(){step.done=false;step.value=entry;return step};
                iterator.return=function(){log+="return;";throw "close"};
                entry.__defineGetter__("0",function(){
                    log+="get-0;";if(mode===0)throw "get-0";return key;
                });
                entry.__defineGetter__("1",function(){
                    log+="get-1;";if(mode===1)throw "get-1";return 7;
                });
                key[Symbol.toPrimitive]=function(hint){
                    log+="key:"+hint+";";if(mode===2)throw "key";return Object();
                };
                try{Object.fromEntries(iterable)}catch(error){return log+"throw:"+text(error)}
                return "missing";
            }
            return run(0)+"|"+run(1)+"|"+run(2)+"|"+run(3);
        })()"#,
    ),
    (
        "IteratorClose preserves the pending failure over return failures",
        r#"(function(){
            function run(mode){
                var log="",iterable=Object(),iterator=Object(),step=Object();
                iterable[Symbol.iterator]=function(){return iterator};
                iterator.next=function(){step.done=false;step.value=1;return step};
                if(mode===0)iterator.__defineGetter__("return",function(){log+="return-get;";throw "close-get"});
                if(mode===1)iterator.return=function(){log+="return-call;";throw "close-call"};
                if(mode===2)iterator.return=1;
                if(mode===3)iterator.return=function(){log+="return-call;";return 1};
                try{Object.fromEntries(iterable)}catch(error){return log+error.name+":"+error.message}
                return "missing";
            }
            return run(0)+"|"+run(1)+"|"+run(2)+"|"+run(3)+"|"+run(4);
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("missing iterable", "Object.fromEntries()"),
    ("undefined iterable", "Object.fromEntries(undefined)"),
    ("null iterable", "Object.fromEntries(null)"),
    ("non-iterable number", "Object.fromEntries(1)"),
    (
        "non-callable iterator method",
        r#"(function(){var value=Object();value[Symbol.iterator]=1;return Object.fromEntries(value)})()"#,
    ),
    (
        "iterator method returns a primitive",
        r#"(function(){var value=Object();value[Symbol.iterator]=function(){return 1};return Object.fromEntries(value)})()"#,
    ),
    (
        "next is not callable",
        r#"(function(){
            var value=Object();
            value[Symbol.iterator]=function(){var iterator=Object();iterator.next=1;return iterator};
            return Object.fromEntries(value);
        })()"#,
    ),
    (
        "next returns a primitive",
        r#"(function(){
            var value=Object();
            value[Symbol.iterator]=function(){
                var iterator=Object();iterator.next=function(){return 1};return iterator;
            };
            return Object.fromEntries(value);
        })()"#,
    ),
    ("undefined entry", "Object.fromEntries([undefined])"),
    ("null entry", "Object.fromEntries([null])"),
    ("numeric entry", "Object.fromEntries([1])"),
    (
        "String iterator yields primitive entries",
        "Object.fromEntries(\"ab\")",
    ),
    (
        "entry key cannot become a property key",
        r#"(function(){var key=Object();key[Symbol.toPrimitive]=function(){return Object()};return Object.fromEntries([[key,1]])})()"#,
    ),
    (
        "fromEntries is not a constructor",
        "new Object.fromEntries([])",
    ),
];

const EXOTIC_ORACLE_ONLY_CASES: &[(&str, &str)] = &[
    (
        "Proxy entries expose zero and one Get trap ordering",
        r#"(function(){
            var log="",base=["x",7];
            var entry=new Proxy(base,{get:function(target,key,receiver){
                log+="get:"+String(key)+";";return Reflect.get(target,key,receiver);
            }});
            var result=Object.fromEntries([entry]);
            return log+result.x;
        })()"#,
    ),
    (
        "generator finally runs when a non-object entry triggers IteratorClose",
        r#"(function(){
            var log="";
            function* entries(){try{yield ["a",1];yield 2}finally{log+="finally;"}}
            try{Object.fromEntries(entries())}catch(error){return log+error.name+":"+error.message}
            return "missing";
        })()"#,
    ),
    (
        "Map supplies its native entry iterator",
        r#"(function(){
            var map=new Map([["x",1],["y",2]]),result=Object.fromEntries(map);
            return Object.getOwnPropertyNames(result).join(",")+":"+result.x+":"+result.y;
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
  'isExtensible','preventExtensions','getOwnPropertyDescriptor','getOwnPropertyDescriptors','is','assign',
  'seal','freeze','isSealed','isFrozen','fromEntries'];
print('prefix='+Reflect.ownKeys(Object).filter(function(key){return selected.indexOf(key)>=0}).join(','));
print('fromEntries='+meta('fromEntries'));
print('identity='+(Object.fromEntries===Object.fromEntries));
var fn=Object.fromEntries;
print('fromEntries-props='+bits(Object.getOwnPropertyDescriptor(fn,'length'))+':' +bits(Object.getOwnPropertyDescriptor(fn,'name')));
"#;

const FRESH_DELETE_ORACLE: &str = r#"
var deleted=delete Object.fromEntries;
print([deleted,'fromEntries' in Object,Object.prototype.hasOwnProperty.call(Object,'fromEntries'),typeof Object.fromEntries].join('|'));
"#;

#[test]
fn object_from_entries_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.fromEntries oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("order and close", ORDER_CASES),
        ("errors", ERROR_CASES),
        ("exotic boundary", EXOTIC_ORACLE_ONLY_CASES),
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
    assert_eq!(
        oracle_lines(
            &oracle,
            FRESH_DELETE_ORACLE,
            "Object.fromEntries fresh delete",
        )
        .len(),
        1,
    );
}

#[test]
fn object_from_entries_values_match_pinned_quickjs() {
    compare_cases("Object.fromEntries values", VALUE_CASES);
}

#[test]
fn object_from_entries_order_and_iterator_close_match_pinned_quickjs() {
    compare_cases("Object.fromEntries order and close", ORDER_CASES);
}

#[test]
fn object_from_entries_errors_match_pinned_quickjs() {
    compare_cases("Object.fromEntries errors", ERROR_CASES);
}

#[test]
fn object_from_entries_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.fromEntries graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Object.fromEntries graph or metadata drifted",
    );
}

#[test]
fn object_from_entries_autoinit_can_be_deleted_before_materialization() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.fromEntries AutoInit delete: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_lines(
        &oracle,
        FRESH_DELETE_ORACLE,
        "Object.fromEntries fresh delete",
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = global_callable(&runtime, &mut context, "Object");
    let key = runtime.intern_property_key("fromEntries").unwrap();
    let deleted = runtime
        .delete_property(object.as_object(), &key)
        .unwrap()
        .to_string();
    let Value::Bool(in_object) = context.eval("'fromEntries' in Object").unwrap() else {
        panic!("Object.fromEntries inherited-presence probe was not boolean");
    };
    let own = runtime
        .has_own_property(object.as_object(), &key)
        .unwrap()
        .to_string();
    let kind = value_type(
        &runtime,
        &context.get_property(object.as_object(), &key).unwrap(),
    );
    assert_eq!(
        vec![format!("{deleted}|{in_object}|{own}|{kind}")],
        expected,
    );
}

#[test]
fn object_from_entries_uses_defining_realm_and_preserves_user_throws() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object = global_callable(&runtime, &mut defining, "Object");
    let from_entries = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "fromEntries",
    );

    let payload = caller.new_object().unwrap();
    let pair = caller
        .new_array_from_values(vec![
            Value::String(JsString::try_from_utf8("value").unwrap()),
            Value::Object(payload.clone()),
        ])
        .unwrap();
    let entries = caller
        .new_array_from_values(vec![Value::Object(pair)])
        .unwrap();
    let Value::Object(result) = caller
        .call(&from_entries, Value::Undefined, &[Value::Object(entries)])
        .unwrap()
    else {
        panic!("cross-realm Object.fromEntries result was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining.object_prototype().unwrap()),
        "result did not use Object.fromEntries's defining realm",
    );
    assert_eq!(
        caller
            .get_property(&result, &runtime.intern_property_key("value").unwrap())
            .unwrap(),
        Value::Object(payload),
        "entry value identity was not preserved",
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_eq!(
        caller.call(&from_entries, Value::Undefined, &[Value::Null]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error.clone()),
        "native iterable rejection did not use the defining realm",
    );

    let bad_entry = caller.new_array_from_values(vec![Value::Int(1)]).unwrap();
    assert_eq!(
        caller.call(&from_entries, Value::Undefined, &[Value::Object(bad_entry)],),
        Err(RuntimeError::Exception),
    );
    let native_entry_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&native_entry_error).unwrap(),
        Some(defining_type_error),
        "native entry rejection did not use the defining realm",
    );

    let sentinel = caller.new_object().unwrap();
    let sentinel_key = runtime.intern_property_key("fromEntriesSentinel").unwrap();
    assert!(
        caller
            .set_property(
                &caller.global_object().unwrap(),
                &sentinel_key,
                Value::Object(sentinel.clone()),
            )
            .unwrap()
    );
    let throwing_pair = eval_object(
        &mut caller,
        r#"(function(){
            var pair=Object();
            pair.__defineGetter__("0",function(){throw fromEntriesSentinel});
            pair[1]=1;
            return pair;
        })()"#,
    );
    let throwing_entries = caller
        .new_array_from_values(vec![Value::Object(throwing_pair)])
        .unwrap();
    assert_eq!(
        caller.call(
            &from_entries,
            Value::Undefined,
            &[Value::Object(throwing_entries)],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "entry getter throw identity was not preserved",
    );

    assert_eq!(
        caller.construct(&from_entries, &[]),
        Err(RuntimeError::Exception),
    );
    let constructor_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&constructor_error).unwrap(),
        Some(caller_type_error),
        "non-constructor rejection did not use the caller realm",
    );
}

#[test]
fn object_from_entries_method_and_result_retain_then_release_their_realm() {
    let runtime = Runtime::new();
    let (from_entries, result) = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_object = global_callable(&runtime, &mut first, "Object");
        let second_object = global_callable(&runtime, &mut second, "Object");
        let first_method = property_callable(
            &runtime,
            &mut first,
            first_object.as_object(),
            "fromEntries",
        );
        let first_method_again = property_callable(
            &runtime,
            &mut first,
            first_object.as_object(),
            "fromEntries",
        );
        let second_method = property_callable(
            &runtime,
            &mut second,
            second_object.as_object(),
            "fromEntries",
        );
        assert_eq!(first_method, first_method_again);
        assert_ne!(first_method, second_method);
        assert_eq!(
            runtime.get_prototype_of(first_method.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        drop(second_method);

        let empty = second.new_array().unwrap();
        let Value::Object(result) = second
            .call(&first_method, Value::Undefined, &[Value::Object(empty)])
            .unwrap()
        else {
            panic!("Object.fromEntries did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&result).unwrap(),
            Some(first.object_prototype().unwrap()),
        );
        (first_method, result)
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(from_entries);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(result);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn object_from_entries_records_current_exotic_and_generator_gap() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                "typeof Proxy+'|'+typeof Map+'|'+typeof Set+'|'+typeof ArrayBuffer+'|'+typeof Uint8Array",
            )
            .unwrap(),
        Value::String(
            JsString::try_from_utf8("undefined|undefined|undefined|undefined|undefined").unwrap(),
        ),
        "activate the exotic Object.fromEntries oracle vectors as these intrinsics are published",
    );
    // The lexer recognizes generator context, but the current compiler does
    // not publish generator bytecode. The generator/finally oracle vector must
    // join the differential when that boundary moves.
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
        "assign",
        "seal",
        "freeze",
        "isSealed",
        "isFrozen",
        "fromEntries",
    ];
    let prefix = own_key_names(&runtime, object.as_object())
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let key = runtime.intern_property_key("fromEntries").unwrap();
    let descriptor = data_descriptor(&runtime, object.as_object(), &key);
    let Value::Object(function) = descriptor.0 else {
        panic!("Object.fromEntries was not an object");
    };
    let callable = runtime.as_callable(&function).unwrap();
    let function_again =
        property_callable(&runtime, &mut context, object.as_object(), "fromEntries");
    let mut output = vec![
        format!("prefix={prefix}"),
        format!(
            "fromEntries={}:{}:{}:{}:{}:{}:{}",
            string_property(&runtime, &mut context, &function, "name"),
            int_property(&runtime, &mut context, &function, "length"),
            runtime.get_prototype_of(&function).unwrap().as_ref() == Some(&function_prototype),
            callable.is_some(),
            runtime.is_constructor(&function).unwrap(),
            own_key_names(&runtime, &function).join(","),
            data_bits(descriptor.1, descriptor.2, descriptor.3),
        ),
        format!("identity={}", function_again.as_object() == &function),
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
        "fromEntries-props={}:{}",
        data_bits(length.1, length.2, length.3),
        data_bits(function_name.1, function_name.2, function_name.3),
    ));
    output
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    oracle_lines(oracle, GRAPH_ORACLE, "Object.fromEntries graph")
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

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    object
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
