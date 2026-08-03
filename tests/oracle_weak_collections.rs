use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for the WeakMap and WeakSet surface exposed by pinned
// QuickJS 2026-06-04. The vectors use only primitive ASCII observations so
// they can be compared without sharing object identities across engines.

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

const PRELUDE: &str = r#"
function __bit(value){return value?"1":"0"}
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    if(descriptor===undefined)return "missing";
    return __bit(descriptor.writable)+__bit(descriptor.enumerable)+__bit(descriptor.configurable);
}
function __isConstructor(value){
    try{Reflect.construct(function(){},[],value);return true}catch(_error){return false}
}
function __completion(thunk){
    try{return "return:"+String(thunk())}
    catch(error){
        if(error!==null&&typeof error==="object")return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
function __keys(object){
    return Reflect.ownKeys(object).map(function(key){return String(key)}).join(",");
}
function __metadata(owner,key){
    var fn=owner[key];
    return String(key)+":"+fn.name+":"+fn.length+":"+__isConstructor(fn)+":"+
        __bits(owner,key)+":"+__bits(fn,"name")+":"+__bits(fn,"length")+":"+__keys(fn);
}
"#;

const CASES: &[Case] = &[
    Case {
        group: "graph",
        description: "globals constructors prototypes descriptors and own-key order",
        source: r#"(function(){
            function graph(name,constructor,prototype,methodNames){
                var methods=[],index;
                for(index=0;index<methodNames.length;index++)
                    methods[index]=__metadata(prototype,methodNames[index]);
                return [
                    name+"-global="+__bits(globalThis,name)+":"+
                        (Object.getOwnPropertyDescriptor(globalThis,name).value===constructor),
                    "constructor="+constructor.name+":"+constructor.length+":"+
                        __isConstructor(constructor)+":"+__keys(constructor),
                    "links="+(Object.getPrototypeOf(constructor)===Function.prototype)+":"+
                        (Object.getPrototypeOf(prototype)===Object.prototype)+":"+
                        (prototype.constructor===constructor),
                    "descriptors="+__bits(constructor,"prototype")+":"+
                        __bits(prototype,"constructor"),
                    "prototype-keys="+__keys(prototype),
                    "methods="+methods.join(";"),
                    "tag="+__bits(prototype,Symbol.toStringTag)+":"+
                        prototype[Symbol.toStringTag]+":"+Object.prototype.toString.call(prototype)+":"+
                        Object.prototype.toString.call(Reflect.construct(constructor,[]))
                ].join("|");
            }
            return graph("WeakMap",WeakMap,WeakMap.prototype,
                ["set","get","getOrInsert","getOrInsertComputed","has","delete"])+"||"+
                graph("WeakSet",WeakSet,WeakSet.prototype,["add","has","delete"]);
        })()"#,
        expected: concat!(
            "return|string|WeakMap-global=101:true|",
            "constructor=WeakMap:0:true:length,name,prototype|links=true:true:true|",
            "descriptors=000:101|",
            "prototype-keys=set,get,getOrInsert,getOrInsertComputed,has,delete,constructor,",
            "Symbol(Symbol.toStringTag)|",
            "methods=set:set:2:false:101:001:001:length,name;",
            "get:get:1:false:101:001:001:length,name;",
            "getOrInsert:getOrInsert:2:false:101:001:001:length,name;",
            "getOrInsertComputed:getOrInsertComputed:2:false:101:001:001:length,name;",
            "has:has:1:false:101:001:001:length,name;",
            "delete:delete:1:false:101:001:001:length,name|",
            "tag=001:WeakMap:[object WeakMap]:[object WeakMap]||",
            "WeakSet-global=101:true|constructor=WeakSet:0:true:length,name,prototype|",
            "links=true:true:true|descriptors=000:101|",
            "prototype-keys=add,has,delete,constructor,Symbol(Symbol.toStringTag)|",
            "methods=add:add:1:false:101:001:001:length,name;",
            "has:has:1:false:101:001:001:length,name;",
            "delete:delete:1:false:101:001:001:length,name|",
            "tag=001:WeakSet:[object WeakSet]:[object WeakSet]",
        ),
    },
    Case {
        group: "core",
        description: "object keys preserve identity update delete and fluent receivers",
        source: r#"(function(){
            var first=Object(),second=Object(),other=Object();
            var map=new WeakMap(),set=new WeakSet();
            var mapChain=map.set(first,1)===map;
            map.set(first,2).set(second,3);
            var setChain=set.add(first)===set;
            set.add(second);
            return [
                mapChain,map.get(first),map.get(second),map.get(other),
                map.has(first),map.has(other),map.delete(first),map.delete(first),
                map.has(first),map.get(second),
                setChain,set.has(first),set.has(second),set.has(other),
                set.delete(first),set.delete(first),set.has(first),set.has(second),
                __completion(function(){return map.set(1,1)}),
                __completion(function(){return set.add("x")}),
                map.get(1),map.has(1),map.delete(1),set.has("x"),set.delete("x")
            ].join("|");
        })()"#,
        expected: concat!(
            "return|string|true|2|3||true|false|true|false|false|3|true|true|true|",
            "false|true|false|false|true|",
            "throw:TypeError:invalid value used as WeakMap key|",
            "throw:TypeError:invalid value used as WeakSet key||",
            "false|false|false|false",
        ),
    },
    Case {
        group: "stale-key",
        description: "expired weak keys do not poison later ordinary property mutations",
        source: r#"(function(){
            var map=new WeakMap(),set=new WeakSet();
            map.set({},1);set.add({});
            map.marker=42;set.marker=42;
            return [map.marker,set.marker,map.has({}),set.has({})].join("|");
        })()"#,
        expected: "return|string|42|42|false|false",
    },
    Case {
        group: "symbols",
        description: "unregistered symbols are weak keys while registered symbols are rejected",
        source: r#"(function(){
            var local=Symbol("local"),wellKnown=Symbol.iterator,registered=Symbol.for("registered");
            var map=new WeakMap(),set=new WeakSet();
            map.set(local,1).set(wellKnown,2);
            set.add(local).add(wellKnown);
            return [
                map.get(local),map.get(wellKnown),map.has(local),map.has(wellKnown),
                set.has(local),set.has(wellKnown),
                map.delete(local),set.delete(local),map.has(local),set.has(local),
                __completion(function(){return map.set(registered,3)}),
                __completion(function(){return set.add(registered)}),
                __completion(function(){return map.getOrInsert(registered,4)}),
                __completion(function(){return map.getOrInsertComputed(registered,function(){return 5})}),
                map.get(registered),map.has(registered),map.delete(registered),
                set.has(registered),set.delete(registered)
            ].join("|");
        })()"#,
        expected: concat!(
            "return|string|1|2|true|true|true|true|true|true|false|false|",
            "throw:TypeError:invalid value used as WeakMap key|",
            "throw:TypeError:invalid value used as WeakSet key|",
            "throw:TypeError:invalid value used as WeakMap key|",
            "throw:TypeError:invalid value used as WeakMap key||",
            "false|false|false|false",
        ),
    },
    Case {
        group: "constructor",
        description: "constructors cache the adder and iterator next while observing access order",
        source: r#"(function(){
            function weakMapRun(){
                var log="",key=Object(),custom=Object.create(WeakMap.prototype);
                var iterator=Object(),count=0,originalSet=WeakMap.prototype.set;
                var NewTarget=(function(){}).bind(null);
                Object.defineProperty(NewTarget,"prototype",{
                    configurable:true,get:function(){log+="prototype;";return custom}
                });
                Object.defineProperty(custom,"set",{
                    configurable:true,get:function(){
                        log+="set-get;";
                        return function(observedKey,value){
                            log+="adder:"+(observedKey===key)+":"+value+";";
                            Object.defineProperty(iterator,"next",{
                                configurable:true,writable:true,value:function(){throw "changed-next"}
                            });
                            return originalSet.call(this,observedKey,value);
                        };
                    }
                });
                var entry=Object();
                Object.defineProperty(entry,"0",{get:function(){log+="key;";return key}});
                Object.defineProperty(entry,"1",{get:function(){log+="value;";return 9}});
                var iterable=Object();
                Object.defineProperty(iterable,Symbol.iterator,{get:function(){
                    log+="iterator-get;";return function(){log+="iterator-call;";return iterator};
                }});
                Object.defineProperty(iterator,"next",{configurable:true,get:function(){
                    log+="next-get;";return function(){
                        log+="next-call;";var done=count++!==0,result=Object();
                        Object.defineProperty(result,"done",{get:function(){log+="done:"+done+";";return done}});
                        Object.defineProperty(result,"value",{get:function(){log+="step-value;";return entry}});
                        return result;
                    };
                }});
                var map=Reflect.construct(WeakMap,[iterable],NewTarget);
                function Fallback(){}Fallback.prototype=17;
                var fallback=Reflect.construct(WeakMap,[],Fallback);
                return [log,Object.getPrototypeOf(map)===custom,map.get(key),
                    Object.getPrototypeOf(fallback)===WeakMap.prototype,
                    __completion(function(){return WeakMap()})].join("|");
            }
            function weakSetRun(){
                var log="",key=Object(),custom=Object.create(WeakSet.prototype);
                var iterator=Object(),count=0,originalAdd=WeakSet.prototype.add;
                var NewTarget=(function(){}).bind(null);
                Object.defineProperty(NewTarget,"prototype",{
                    configurable:true,get:function(){log+="prototype;";return custom}
                });
                Object.defineProperty(custom,"add",{
                    configurable:true,get:function(){
                        log+="add-get;";
                        return function(observedKey){
                            log+="adder:"+(observedKey===key)+";";
                            Object.defineProperty(iterator,"next",{
                                configurable:true,writable:true,value:function(){throw "changed-next"}
                            });
                            return originalAdd.call(this,observedKey);
                        };
                    }
                });
                var iterable=Object();
                Object.defineProperty(iterable,Symbol.iterator,{get:function(){
                    log+="iterator-get;";return function(){log+="iterator-call;";return iterator};
                }});
                Object.defineProperty(iterator,"next",{configurable:true,get:function(){
                    log+="next-get;";return function(){
                        log+="next-call;";var done=count++!==0,result=Object();
                        Object.defineProperty(result,"done",{get:function(){log+="done:"+done+";";return done}});
                        Object.defineProperty(result,"value",{get:function(){log+="step-value;";return key}});
                        return result;
                    };
                }});
                var set=Reflect.construct(WeakSet,[iterable],NewTarget);
                function Fallback(){}Fallback.prototype=17;
                var fallback=Reflect.construct(WeakSet,[],Fallback);
                return [log,Object.getPrototypeOf(set)===custom,set.has(key),
                    Object.getPrototypeOf(fallback)===WeakSet.prototype,
                    __completion(function(){return WeakSet()})].join("|");
            }
            return weakMapRun()+"||"+weakSetRun();
        })()"#,
        expected: concat!(
            "return|string|prototype;set-get;iterator-get;iterator-call;next-get;",
            "next-call;done:false;step-value;key;value;adder:true:9;next-call;done:true;|",
            "true|9|true|throw:TypeError:must be called with new||",
            "prototype;add-get;iterator-get;iterator-call;next-get;next-call;done:false;",
            "step-value;adder:true;next-call;done:true;|true|true|true|",
            "throw:TypeError:must be called with new",
        ),
    },
    Case {
        group: "close",
        description: "constructor element and adder failures close iterators at QuickJS boundaries",
        source: r#"(function(){
            function mapRun(mode){
                var log="",iterator=Object(),iterable=Object(),count=0,key=Object();
                iterable[Symbol.iterator]=function(){log+="iterator;";return iterator};
                iterator.return=function(){log+="return;";throw "close"};
                iterator.next=function(){
                    log+="next;";
                    if(mode===4)throw "next";
                    var result=Object();
                    Object.defineProperty(result,"done",{get:function(){
                        log+="done;";if(mode===5)throw "done";return count++!==0;
                    }});
                    Object.defineProperty(result,"value",{get:function(){
                        log+="step-value;";if(mode===6)throw "step-value";
                        if(mode===0)return 1;
                        var entry=Object();
                        Object.defineProperty(entry,"0",{get:function(){
                            log+="key;";if(mode===2)throw "key";return mode===1?1:key;
                        }});
                        Object.defineProperty(entry,"1",{get:function(){log+="value;";return 7}});
                        return entry;
                    }});
                    return result;
                };
                var target=WeakMap;
                if(mode===3){
                    var prototype=Object.create(WeakMap.prototype);
                    prototype.set=function(){log+="adder;";throw "adder"};
                    target=function(){};target.prototype=prototype;
                }
                try{Reflect.construct(WeakMap,[iterable],target);return log+"missing"}
                catch(error){return log+"catch:"+(error!==null&&typeof error==="object"?error.name:String(error))}
            }
            function setRun(mode){
                var log="",iterator=Object(),iterable=Object(),count=0,key=Object();
                iterable[Symbol.iterator]=function(){log+="iterator;";return iterator};
                iterator.return=function(){log+="return;";throw "close"};
                iterator.next=function(){
                    log+="next;";if(mode===2)throw "next";
                    return {done:count++!==0,value:mode===0?1:key};
                };
                var target=WeakSet;
                if(mode===1){
                    var prototype=Object.create(WeakSet.prototype);
                    prototype.add=function(){log+="adder;";throw "adder"};
                    target=function(){};target.prototype=prototype;
                }
                try{Reflect.construct(WeakSet,[iterable],target);return log+"missing"}
                catch(error){return log+"catch:"+(error!==null&&typeof error==="object"?error.name:String(error))}
            }
            var output=[],index;
            for(index=0;index<7;index++)output[index]="m"+index+":"+mapRun(index);
            for(index=0;index<3;index++)output[7+index]="s"+index+":"+setRun(index);
            return output.join("|");
        })()"#,
        expected: concat!(
            "return|string|m0:iterator;next;done;step-value;return;catch:TypeError|",
            "m1:iterator;next;done;step-value;key;value;return;catch:TypeError|",
            "m2:iterator;next;done;step-value;key;return;catch:key|",
            "m3:iterator;next;done;step-value;key;value;adder;return;catch:adder|",
            "m4:iterator;next;catch:next|m5:iterator;next;done;catch:done|",
            "m6:iterator;next;done;step-value;catch:step-value|",
            "s0:iterator;next;return;catch:TypeError|",
            "s1:iterator;next;adder;return;catch:adder|s2:iterator;next;catch:next",
        ),
    },
    Case {
        group: "upsert",
        description: "WeakMap upsert methods validate and replace callback reentrant entries",
        source: r#"(function(){
            var present=Object(),fresh=Object(),computedKey=Object(),side=Object(),thrown=Object();
            var invalid=Object(),map=new WeakMap(),called=0,callbackThis="unset",callbackKey="unset";
            map.set(present,undefined);
            var presentResult=map.getOrInsert(present,9);
            var inserted=map.getOrInsert(fresh,2);
            var computed=map.getOrInsertComputed(computedKey,function(key){
                "use strict";
                called++;callbackThis=this;callbackKey=key;
                map.set(computedKey,20);map.set(side,30);
                return 40;
            });
            var sentinel=Object(),sameThrow=false;
            try{
                map.getOrInsertComputed(thrown,function(){map.set(thrown,50);throw sentinel});
            }catch(error){sameThrow=error===sentinel}
            return [
                presentResult===undefined,inserted,called,callbackThis===undefined,
                callbackKey===computedKey,computed,map.get(computedKey),map.get(side),
                map.get(thrown),sameThrow,
                __completion(function(){return map.getOrInsertComputed(present,0)}),
                __completion(function(){return map.getOrInsertComputed(1,0)}),
                __completion(function(){return map.getOrInsertComputed(1,function(){return 1})}),
                __completion(function(){return map.getOrInsert(1,1)}),
                map.has(invalid)
            ].join("|");
        })()"#,
        expected: concat!(
            "return|string|true|2|1|true|true|40|40|30|50|true|",
            "throw:TypeError:not a function|throw:TypeError:not a function|",
            "throw:TypeError:invalid value used as WeakMap key|",
            "throw:TypeError:invalid value used as WeakMap key|false",
        ),
    },
    Case {
        group: "brands",
        description: "prototype methods enforce weak collection brands and are not constructors",
        source: r#"(function(){
            var key=Object(),map=new WeakMap(),set=new WeakSet();
            return [
                __completion(function(){return WeakMap.prototype.get.call(Object(),key)}),
                __completion(function(){return WeakMap.prototype.set.call(set,key,1)}),
                __completion(function(){return WeakSet.prototype.add.call(map,key)}),
                __completion(function(){return WeakSet.prototype.has.call(new Set(),key)}),
                __completion(function(){return new WeakMap.prototype.get()}),
                __completion(function(){return new WeakSet.prototype.add()}),
                __completion(function(){return WeakMap.prototype.set.call(map,key,1)===map}),
                __completion(function(){return WeakSet.prototype.add.call(set,key)===set}),
                __completion(function(){return WeakMap.prototype.getOrInsert.call(set,key,1)}),
                __completion(function(){return WeakMap.prototype.getOrInsertComputed.call(map,key,0)})
            ].join("|");
        })()"#,
        expected: concat!(
            "return|string|throw:TypeError:WeakMap object expected|",
            "throw:TypeError:WeakMap object expected|throw:TypeError:WeakSet object expected|",
            "throw:TypeError:WeakSet object expected|throw:TypeError:get is not a constructor|",
            "throw:TypeError:add is not a constructor|return:true|return:true|",
            "throw:TypeError:WeakMap object expected|throw:TypeError:not a function",
        ),
    },
];

#[test]
fn weak_collection_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP weak collection oracle self-check: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for case in CASES {
        let actual = oracle_observation(&oracle, case);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nactual: {:?}\nexpected: {:?}",
                case.group, case.description, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "pinned QuickJS weak collection vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn weak_collection_graph_core_and_symbols_match_pinned_quickjs() {
    compare_groups(&["graph", "core", "stale-key", "symbols"]);
}

#[test]
fn weak_collection_constructors_and_iterator_close_match_pinned_quickjs() {
    compare_groups(&["constructor", "close"]);
}

#[test]
fn weak_map_upsert_and_weak_collection_brands_match_pinned_quickjs() {
    compare_groups(&["upsert", "brands"]);
}

#[test]
fn weak_collection_new_target_fallback_and_native_errors_use_exact_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let map_key = Value::Object(eval_object(&mut caller, "({})", "caller WeakMap key"));
    assert_cross_realm_collection(
        &runtime,
        &mut defining,
        &mut caller,
        "WeakMap",
        "get",
        &[map_key],
    );
    let set_key = Value::Object(eval_object(&mut caller, "({})", "caller WeakSet key"));
    assert_cross_realm_collection(
        &runtime,
        &mut defining,
        &mut caller,
        "WeakSet",
        "has",
        &[set_key],
    );
}

fn assert_cross_realm_collection(
    runtime: &Runtime,
    defining: &mut Context,
    caller: &mut Context,
    constructor_name: &str,
    method_name: &str,
    arguments: &[Value],
) {
    let defining_constructor = global_callable(runtime, defining, constructor_name);
    let defining_prototype = eval_object(
        defining,
        &format!("{constructor_name}.prototype"),
        "defining weak collection prototype",
    );
    let caller_prototype = eval_object(
        caller,
        &format!("{constructor_name}.prototype"),
        "caller weak collection prototype",
    );
    let defining_type_error = eval_object(
        defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let caller_type_error =
        eval_object(caller, "TypeError.prototype", "caller TypeError prototype");
    assert_ne!(defining_prototype, caller_prototype);
    assert_ne!(defining_type_error, caller_type_error);

    let foreign_collection = expect_object(
        caller.construct(&defining_constructor, &[]).unwrap(),
        "foreign weak collection construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&foreign_collection).unwrap(),
        Some(defining_prototype.clone()),
        "{constructor_name} construction did not use the foreign constructor prototype",
    );

    let caller_new_target = eval_callable(
        runtime,
        caller,
        "(function(){function F(){}F.prototype=17;return F})()",
        "caller primitive-prototype newTarget",
    );
    let fallback_collection = expect_object(
        caller
            .construct_with_new_target(&defining_constructor, &caller_new_target, &[])
            .unwrap(),
        "cross-realm weak collection fallback construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&fallback_collection).unwrap(),
        Some(caller_prototype),
        "primitive newTarget.prototype did not fall back to the newTarget realm {constructor_name}",
    );

    let method = property_callable(runtime, defining, &defining_prototype, method_name);
    let ordinary = eval_object(caller, "({})", "caller ordinary object");
    assert_eq!(
        caller.call(&method, Value::Object(ordinary), arguments),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(caller, "foreign weak collection brand TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "{constructor_name} branded native error used the calling realm",
    );
}

fn compare_groups(groups: &[&str]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP weak collection differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for case in CASES.iter().filter(|case| groups.contains(&case.group)) {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let actual = rust_observation(&runtime, &mut context, case);
        let expected = oracle_observation(&oracle, case);
        if actual != expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\noxide: {:?}\noracle: {:?}",
                case.group, case.description, case.source, actual, expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "weak collection semantics drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn observed_source(source: &str) -> String {
    format!("{PRELUDE}\n{source}")
}

fn rust_observation(runtime: &Runtime, context: &mut Context, case: &Case) -> String {
    let source = observed_source(case.source);
    match context.eval(&source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value),
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| {
                    panic!(
                        "take Rust exception for {} / {}: {error}",
                        case.group, case.description,
                    )
                })
                .unwrap_or_else(|| {
                    panic!(
                        "Rust exception was missing for {} / {}",
                        case.group, case.description,
                    )
                });
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
        Err(error) => panic!(
            "Rust engine failure for {} / {} ({:?}): {error}",
            case.group, case.description, case.source,
        ),
    }
}

fn oracle_observation(oracle: &OsStr, case: &Case) -> String {
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
    let source = observed_source(case.source);
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, &source])
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "could not run QuickJS for {} / {}: {error}",
                case.group, case.description,
            )
        });
    assert!(
        output.status.success(),
        "QuickJS observer failed for {} / {}: {}",
        case.group,
        case.description,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| {
            panic!(
                "QuickJS output was not UTF-8 for {} / {}: {error}",
                case.group, case.description,
            )
        })
        .trim_end()
        .to_owned()
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
        .unwrap_or_else(|| panic!("{description} was not callable"))
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

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(object) = value else {
        panic!("{description} did not produce an object");
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
