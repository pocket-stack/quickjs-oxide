use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for the complete strong Map surface exposed by pinned
// QuickJS 2026-06-04. The vectors intentionally stay inside quickjs-oxide's
// implemented grammar and print only primitive ASCII observations.

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
function __number(value){
    if(value!==value)return "NaN";
    if(value===0)return 1/value===-Infinity?"-0":"+0";
    if(value===Infinity)return "+Infinity";
    if(value===-Infinity)return "-Infinity";
    return String(value);
}
"#;

const CASES: &[Case] = &[
    Case {
        group: "graph",
        description: "constructor prototype iterator aliases metadata and descriptors",
        source: r#"(function(){
            function keys(object){
                return Reflect.ownKeys(object).map(function(key){return String(key)}).join(",");
            }
            function metadata(owner,key){
                var fn=owner[key];
                return String(key)+":"+fn.name+":"+fn.length+":"+__isConstructor(fn)+":"+
                    __bits(owner,key)+":"+__bits(fn,"name")+":"+__bits(fn,"length")+":"+keys(fn);
            }
            var methodNames=["set","get","getOrInsert","getOrInsertComputed","has","delete",
                "clear","forEach","values","keys","entries"],methods=[],index;
            for(index=0;index<methodNames.length;index++)
                methods[index]=metadata(Map.prototype,methodNames[index]);
            var sizeDescriptor=Object.getOwnPropertyDescriptor(Map.prototype,"size");
            var speciesDescriptor=Object.getOwnPropertyDescriptor(Map,Symbol.species);
            var sampleIterator=new Map().entries();
            var iteratorPrototype=Object.getPrototypeOf(sampleIterator);
            var arrayIteratorPrototype=Object.getPrototypeOf([][Symbol.iterator]());
            return [
                "global="+__bits(globalThis,"Map")+":"+
                    (Object.getOwnPropertyDescriptor(globalThis,"Map").value===Map),
                "constructor="+Map.name+":"+Map.length+":"+__isConstructor(Map)+":"+keys(Map),
                "links="+(Object.getPrototypeOf(Map)===Function.prototype)+":"+
                    (Object.getPrototypeOf(Map.prototype)===Object.prototype)+":"+
                    (Map.prototype.constructor===Map),
                "descriptors="+__bits(Map,"prototype")+":"+__bits(Map.prototype,"constructor"),
                "prototype-keys="+keys(Map.prototype),
                "prototype-brand="+Object.prototype.toString.call(Map.prototype)+":"+
                    __completion(function(){return Map.prototype.size}),
                "methods="+methods.join(";"),
                "aliases="+(Map.prototype[Symbol.iterator]===Map.prototype.entries)+":"+
                    Map.prototype[Symbol.iterator].name+":"+__bits(Map.prototype,Symbol.iterator)+":"+
                    (Map.prototype.keys!==Map.prototype.values),
                "size="+__bits(Map.prototype,"size")+":"+sizeDescriptor.get.name+":"+
                    sizeDescriptor.get.length+":"+__isConstructor(sizeDescriptor.get)+":"+
                    keys(sizeDescriptor.get),
                "tags="+__bits(Map.prototype,Symbol.toStringTag)+":"+
                    Map.prototype[Symbol.toStringTag]+":"+
                    Object.prototype.toString.call(new Map()),
                "species="+__bits(Map,Symbol.species)+":"+speciesDescriptor.get.name+":"+
                    speciesDescriptor.get.length+":"+__isConstructor(speciesDescriptor.get)+":"+
                    keys(speciesDescriptor.get)+":"+(speciesDescriptor.get.call(17)===17),
                "groupBy="+metadata(Map,"groupBy"),
                "iterator="+keys(iteratorPrototype)+":"+
                    metadata(iteratorPrototype,"next")+":"+
                    __bits(iteratorPrototype,Symbol.toStringTag)+":"+
                    iteratorPrototype[Symbol.toStringTag]+":"+
                    (Object.getPrototypeOf(iteratorPrototype)===Object.getPrototypeOf(arrayIteratorPrototype))+":"+
                    (sampleIterator[Symbol.iterator]()===sampleIterator)
            ].join("|");
        })()"#,
        expected: concat!(
            "return|string|global=101:true|",
            "constructor=Map:0:true:length,name,groupBy,prototype,Symbol(Symbol.species)|",
            "links=true:true:true|descriptors=000:101|",
            "prototype-keys=set,get,getOrInsert,getOrInsertComputed,has,delete,clear,size,",
            "forEach,values,keys,entries,constructor,Symbol(Symbol.iterator),",
            "Symbol(Symbol.toStringTag)|",
            "prototype-brand=[object Map]:throw:TypeError:Map object expected|",
            "methods=set:set:2:false:101:001:001:length,name;",
            "get:get:1:false:101:001:001:length,name;",
            "getOrInsert:getOrInsert:2:false:101:001:001:length,name;",
            "getOrInsertComputed:getOrInsertComputed:2:false:101:001:001:length,name;",
            "has:has:1:false:101:001:001:length,name;",
            "delete:delete:1:false:101:001:001:length,name;",
            "clear:clear:0:false:101:001:001:length,name;",
            "forEach:forEach:1:false:101:001:001:length,name;",
            "values:values:0:false:101:001:001:length,name;",
            "keys:keys:0:false:101:001:001:length,name;",
            "entries:entries:0:false:101:001:001:length,name|",
            "aliases=true:entries:101:true|",
            "size=001:get size:0:false:length,name|",
            "tags=001:Map:[object Map]|",
            "species=001:get [Symbol.species]:0:false:length,name:true|",
            "groupBy=groupBy:groupBy:2:false:101:001:001:length,name|",
            "iterator=next,Symbol(Symbol.toStringTag):next:next:0:false:101:001:001:",
            "length,name:001:Map Iterator:true:true",
        ),
    },
    Case {
        group: "keys",
        description: "SameValueZero normalizes NaN and negative zero while retaining BigInt identity",
        source: r#"(function(){
            var firstObject=Object(),secondObject=Object(),map=new Map(),keys=[],step,key;
            map.set(NaN,"nan-first");
            map.set(0/0,"nan-second");
            map.set(-0,"zero-first");
            map.set(+0,"zero-second");
            map.set(1n,"bigint-first");
            map.set(BigInt("1"),"bigint-second");
            map.set(1,"number");
            map.set(firstObject,"object-first");
            map.set(secondObject,"object-second");
            var iterator=map.keys();
            while(!(step=iterator.next()).done){
                key=step.value;
                if(key===firstObject)keys[keys.length]="firstObject";
                else if(key===secondObject)keys[keys.length]="secondObject";
                else if(typeof key==="number")keys[keys.length]=__number(key);
                else if(typeof key==="bigint")keys[keys.length]=String(key)+"n";
                else keys[keys.length]=String(key);
            }
            return [
                map.size,
                map.get(NaN),map.get(-0),map.get(+0),map.get(1n),map.get(1),
                map.has(0/0),map.has(BigInt("1")),map.has(firstObject),map.has(Object()),
                keys.join(","),
                map.delete(0/0),map.has(NaN),map.size
            ].join("|");
        })()"#,
        expected: concat!(
            "return|string|6|nan-second|zero-second|zero-second|bigint-second|number|",
            "true|true|true|false|NaN,+0,1n,1,firstObject,secondObject|true|false|5",
        ),
    },
    Case {
        group: "constructor",
        description: "newTarget prototype adder and iterator observations follow QuickJS order",
        source: r#"(function(){
            var log="",custom=Object.create(Map.prototype),iterator=Object(),count=0;
            var originalSet=Map.prototype.set;
            var NewTarget=(function(){}).bind(null);
            Object.defineProperty(NewTarget,"prototype",{
                configurable:true,
                get:function(){log+="prototype;";return custom}
            });
            Object.defineProperty(custom,"set",{
                configurable:true,
                get:function(){
                    log+="set-get;";
                    return function(key,value){
                        log+="adder:"+key+":"+value+";";
                        Object.defineProperty(iterator,"next",{
                            configurable:true,writable:true,
                            value:function(){throw "changed-next"}
                        });
                        return originalSet.call(this,key,value);
                    };
                }
            });
            var entry=Object();
            Object.defineProperty(entry,"0",{get:function(){log+="key;";return "k"}});
            Object.defineProperty(entry,"1",{get:function(){log+="entry-value;";return 9}});
            var iterable=Object();
            Object.defineProperty(iterable,Symbol.iterator,{
                get:function(){
                    log+="iterator-get;";
                    return function(){log+="iterator-call;";return iterator};
                }
            });
            Object.defineProperty(iterator,"next",{
                configurable:true,
                get:function(){
                    log+="next-get;";
                    return function(){
                        log+="next-call;";
                        var result=Object(),done=count++!==0;
                        Object.defineProperty(result,"done",{
                            get:function(){log+="done:"+done+";";return done}
                        });
                        Object.defineProperty(result,"value",{
                            get:function(){log+="step-value;";return entry}
                        });
                        return result;
                    };
                }
            });
            var map=Reflect.construct(Map,[iterable],NewTarget);
            function Fallback(){}Fallback.prototype=17;
            var fallback=Reflect.construct(Map,[],Fallback);
            return [log,Object.getPrototypeOf(map)===custom,map.size,map.get("k"),
                Object.getPrototypeOf(fallback)===Map.prototype,
                __completion(function(){return Map()})].join("|");
        })()"#,
        expected: concat!(
            "return|string|prototype;set-get;iterator-get;iterator-call;next-get;",
            "next-call;done:false;step-value;key;entry-value;adder:k:9;next-call;",
            "done:true;|true|1|9|true|throw:TypeError:must be called with new",
        ),
    },
    Case {
        group: "constructor",
        description: "entry and adder failures close but iterator step failures do not",
        source: r#"(function(){
            function run(mode){
                var log="",iterator=Object(),iterable=Object(),count=0;
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
                        log+="step-value;";
                        if(mode===6)throw "step-value";
                        if(mode===0)return 1;
                        var entry=Object();
                        Object.defineProperty(entry,"0",{get:function(){
                            log+="key;";if(mode===1||mode===7)throw "key";return "k";
                        }});
                        Object.defineProperty(entry,"1",{get:function(){
                            log+="entry-value;";if(mode===2)throw "entry-value";return 1;
                        }});
                        return entry;
                    }});
                    return result;
                };
                var target=Map;
                if(mode===3){
                    var prototype=Object.create(Map.prototype);
                    prototype.set=function(){log+="adder;";throw "adder"};
                    target=function(){};target.prototype=prototype;
                }
                try{Reflect.construct(Map,[iterable],target);return log+"missing"}
                catch(error){
                    return log+"catch:"+(error!==null&&typeof error==="object"?error.name:String(error));
                }
            }
            var output=[],index;
            for(index=0;index<8;index++)output[index]=index+":"+run(index);
            return output.join("|");
        })()"#,
        expected: concat!(
            "return|string|0:iterator;next;done;step-value;return;catch:TypeError|",
            "1:iterator;next;done;step-value;key;return;catch:key|",
            "2:iterator;next;done;step-value;key;entry-value;return;catch:entry-value|",
            "3:iterator;next;done;step-value;key;entry-value;adder;return;catch:adder|",
            "4:iterator;next;catch:next|5:iterator;next;done;catch:done|",
            "6:iterator;next;done;step-value;catch:step-value|",
            "7:iterator;next;done;step-value;key;return;catch:key",
        ),
    },
    Case {
        group: "iteration",
        description: "delete clear and re-add remain visible to live insertion-order iterators",
        source: r#"(function(){
            function entry(step){return step.done?"done":step.value[0]+":"+step.value[1]}
            var map=new Map(),output=[],iterator;
            map.set("a",1).set("b",2).set("c",3);
            iterator=map.entries();
            output[output.length]=entry(iterator.next());
            map.delete("b");map.set("d",4);
            output[output.length]=entry(iterator.next());
            map.clear();map.set("e",5);
            output[output.length]=entry(iterator.next());
            map.delete("e");map.set("e",6);
            output[output.length]=entry(iterator.next());
            output[output.length]=entry(iterator.next());
            map.set("f",7);
            output[output.length]=entry(iterator.next());

            map=new Map();map.set("a",1).set("b",2).set("c",3);
            iterator=map.keys();
            output[output.length]=iterator.next().value;
            map.delete("b");map.set("b",20);
            output[output.length]=iterator.next().value;
            output[output.length]=iterator.next().value;
            output[output.length]=iterator.next().done;
            return output.join("|");
        })()"#,
        expected: "return|string|a:1|c:3|e:5|e:6|done|done|a|c|b|true",
    },
    Case {
        group: "iteration",
        description: "forEach locks the current record and observes reentrant deletion and appends",
        source: r#"(function(){
            var map=new Map(),receiver={marker:1},log=[];
            map.set("a",1).set("b",2);
            map.forEach(function(value,key,observed){
                log[log.length]="outer:"+key+":"+value+":"+(this===receiver)+":"+(observed===map);
                if(key==="a"){
                    observed.delete("b");observed.set("c",3);
                    observed.forEach(function(innerValue,innerKey,innerMap){
                        log[log.length]="inner:"+innerKey+":"+innerValue+":"+
                            (this===receiver)+":"+(innerMap===map);
                        if(innerKey==="a")innerMap.set("d",4);
                        if(innerKey==="c")innerMap.delete("c");
                    },receiver);
                }
                if(key==="d"){
                    observed.delete("d");observed.set("e",5);
                }
            },receiver);
            return log.join(";")+"|"+map.size+"|"+map.has("b")+"|"+
                map.has("c")+"|"+map.has("d")+"|"+map.get("e");
        })()"#,
        expected: concat!(
            "return|string|outer:a:1:true:true;inner:a:1:true:true;",
            "inner:c:3:true:true;inner:d:4:true:true;outer:d:4:true:true;",
            "outer:e:5:true:true|2|false|false|false|5",
        ),
    },
    Case {
        group: "upsert",
        description: "getOrInsertComputed deletes callback insertion and appends the computed result",
        source: r#"(function(){
            function entries(map){
                var iterator=map.entries(),step,output=[];
                while(!(step=iterator.next()).done)
                    output[output.length]=String(step.value[0])+":"+String(step.value[1]);
                return output.join(",");
            }
            var map=new Map(),called=0,callbackThis="unset",callbackKey="unset";
            map.set("present",undefined).set("stay",1);
            var present=map.getOrInsert("present",9);
            var inserted=map.getOrInsert("new",2);
            var negativeZero=map.getOrInsert(-0,3);
            var positiveZero=map.getOrInsert(+0,4);
            var existing=map.getOrInsertComputed("stay",function(){called++;return 10});
            var computed=map.getOrInsertComputed("computed",function(key){
                "use strict";
                called++;callbackThis=this;callbackKey=key;
                map.set("computed",20);map.set("side",30);
                return 40;
            });
            var sentinel=Object(),sameThrow=false;
            try{
                map.getOrInsertComputed("thrown",function(){map.set("thrown",50);throw sentinel});
            }catch(error){sameThrow=error===sentinel}
            var validation=__completion(function(){
                return map.getOrInsertComputed("stay",0);
            });
            return [
                present===undefined,inserted,negativeZero,positiveZero,existing,called,
                callbackThis===undefined,callbackKey,computed,map.get("computed"),
                map.get("thrown"),sameThrow,validation,entries(map)
            ].join("|");
        })()"#,
        expected: concat!(
            "return|string|true|2|3|3|1|1|true|computed|40|40|50|true|",
            "throw:TypeError:not a function|",
            "present:undefined,stay:1,new:2,0:3,side:30,computed:40,thrown:50",
        ),
    },
    Case {
        group: "groupBy",
        description: "groupBy retains SameValueZero keys arrays callback indices and receiver independence",
        source: r#"(function(){
            var objectKey=Object(),coerced=false,log="";
            objectKey.toString=function(){coerced=true;throw "coerced"};
            var values=[-0,+0,NaN,0/0,1n,1,"object"],keys=[];
            var grouped=Map.groupBy.call(Object(),values,function(value,index){
                "use strict";
                log+=(this===globalThis)+":"+index+";";
                return index===6?objectKey:value;
            });
            var iterator=grouped.keys(),step,key;
            while(!(step=iterator.next()).done){
                key=step.value;
                if(key===objectKey)keys[keys.length]="object";
                else if(typeof key==="number")keys[keys.length]=__number(key);
                else if(typeof key==="bigint")keys[keys.length]=String(key)+"n";
                else keys[keys.length]=String(key);
            }
            return [
                Object.getPrototypeOf(grouped)===Map.prototype,
                grouped.size,keys.join(","),
                grouped.get(-0).length,grouped.get(NaN).length,
                grouped.get(1n)[0]===1n,grouped.get(1)[0]===1,
                grouped.get(objectKey)[0],coerced,log,
                Array.isArray(grouped.get(-0)),
                Object.getPrototypeOf(grouped.get(-0))===Array.prototype
            ].join("|");
        })()"#,
        expected: concat!(
            "return|string|true|5|+0,NaN,1n,1,object|2|2|true|true|object|false|",
            "true:0;true:1;true:2;true:3;true:4;true:5;true:6;|true|true",
        ),
    },
    Case {
        group: "groupBy",
        description: "callback validation and abrupt completion use the pinned iterator-close boundary",
        source: r#"(function(){
            function invalidCallback(){
                var log="",iterable=Object();
                Object.defineProperty(iterable,Symbol.iterator,{get:function(){log+="touched;";throw "iterator"}});
                try{Map.groupBy(iterable,0);return log+"missing"}
                catch(error){return log+(error!==null&&typeof error==="object"?error.name:String(error))}
            }
            function callbackThrow(){
                var log="",iterator=Object(),iterable=Object();
                iterable[Symbol.iterator]=function(){log+="iterator;";return iterator};
                iterator.next=function(){
                    log+="next;";var result=Object();result.done=false;result.value=7;return result;
                };
                iterator.return=function(){log+="return;";throw "close"};
                try{Map.groupBy(iterable,function(){log+="callback;";throw "callback"});return log+"missing"}
                catch(error){return log+String(error)}
            }
            function nextThrow(){
                var log="",iterator=Object(),iterable=Object();
                iterable[Symbol.iterator]=function(){return iterator};
                iterator.next=function(){log+="next;";throw "next"};
                iterator.return=function(){log+="return;"};
                try{Map.groupBy(iterable,function(){return 0});return log+"missing"}
                catch(error){return log+String(error)}
            }
            return invalidCallback()+"|"+callbackThrow()+"|"+nextThrow()+"|"+
                __completion(function(){return new Map.groupBy([],function(){return 0})});
        })()"#,
        expected: concat!(
            "return|string|TypeError|iterator;next;callback;return;callback|next;next|",
            "throw:TypeError:groupBy is not a constructor",
        ),
    },
    Case {
        group: "brands",
        description: "prototype methods iterator next and constructors enforce exact brands",
        source: r#"(function(){
            var iteratorNext=Object.getPrototypeOf(new Map().entries()).next;
            return [
                __completion(function(){return Map.prototype.get.call(Object(),1)}),
                __completion(function(){return Object.getOwnPropertyDescriptor(Map.prototype,"size").get.call(Object())}),
                __completion(function(){return Map.prototype.forEach.call(Object(),function(){})}),
                __completion(function(){return iteratorNext.call(Object())}),
                __completion(function(){return new Map.prototype.get()}),
                __completion(function(){return Map.prototype.set.call(new Map(),"x",1) instanceof Map})
            ].join("|");
        })()"#,
        expected: concat!(
            "return|string|throw:TypeError:Map object expected|",
            "throw:TypeError:Map object expected|throw:TypeError:Map object expected|",
            "throw:TypeError:Map Iterator object expected|",
            "throw:TypeError:get is not a constructor|return:true",
        ),
    },
];

#[test]
fn map_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Map oracle self-check: set QJS_ORACLE to pinned upstream qjs");
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
        "pinned QuickJS Map vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn map_graph_keys_and_constructor_match_pinned_quickjs() {
    compare_groups(&["graph", "keys", "constructor"]);
}

#[test]
fn map_live_iteration_and_for_each_match_pinned_quickjs() {
    compare_groups(&["iteration"]);
}

#[test]
fn map_upsert_and_group_by_match_pinned_quickjs() {
    compare_groups(&["upsert", "groupBy"]);
}

#[test]
fn map_brand_errors_match_pinned_quickjs() {
    compare_groups(&["brands"]);
}

#[test]
fn map_constructor_and_native_errors_use_exact_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let defining_constructor = global_callable(&runtime, &mut defining, "Map");
    let defining_map_prototype =
        eval_object(&mut defining, "Map.prototype", "defining Map prototype");
    let caller_map_prototype = eval_object(&mut caller, "Map.prototype", "caller Map prototype");
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
    assert_ne!(defining_map_prototype, caller_map_prototype);
    assert_ne!(defining_type_error, caller_type_error);

    let foreign_map = expect_object(
        caller.construct(&defining_constructor, &[]).unwrap(),
        "foreign Map construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&foreign_map).unwrap(),
        Some(defining_map_prototype.clone()),
        "Map construction did not use the foreign constructor prototype",
    );

    let caller_new_target = eval_callable(
        &runtime,
        &mut caller,
        "(function(){function F(){}F.prototype=17;return F})()",
        "caller primitive-prototype newTarget",
    );
    let fallback_map = expect_object(
        caller
            .construct_with_new_target(&defining_constructor, &caller_new_target, &[])
            .unwrap(),
        "cross-realm Map fallback construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&fallback_map).unwrap(),
        Some(caller_map_prototype),
        "primitive newTarget.prototype did not fall back to the newTarget realm",
    );

    let get = property_callable(&runtime, &mut defining, &defining_map_prototype, "get");
    assert!(matches!(
        caller.call(&get, Value::Object(foreign_map), &[Value::Int(1)]),
        Ok(Value::Undefined),
    ));

    let ordinary = eval_object(&mut caller, "({})", "caller ordinary object");
    assert_eq!(
        caller.call(&get, Value::Object(ordinary), &[Value::Int(1)]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "foreign Map.get TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Map branded native error used the calling realm",
    );
}

fn compare_groups(groups: &[&str]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Map differential: set QJS_ORACLE to pinned upstream qjs");
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
        "Map semantics drifted in {} case(s):\n\n{}",
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
