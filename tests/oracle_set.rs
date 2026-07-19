use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for the complete strong Set surface exposed by pinned
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
function __setValues(set){
    var iterator=set.values(),step,values=[];
    while(!(step=iterator.next()).done){
        var value=step.value;
        if(typeof value==="number")values[values.length]=__number(value);
        else if(typeof value==="bigint")values[values.length]=String(value)+"n";
        else values[values.length]=String(value);
    }
    return values.join(",");
}
"#;

const CASES: &[Case] = &[
    Case {
        group: "graph",
        description: "constructor prototype aliases metadata descriptors and iterator graph",
        source: r#"(function(){
            function keys(object){
                return Reflect.ownKeys(object).map(function(key){return String(key)}).join(",");
            }
            function metadata(owner,key){
                var fn=owner[key];
                return String(key)+":"+fn.name+":"+fn.length+":"+__isConstructor(fn)+":"+
                    __bits(owner,key)+":"+__bits(fn,"name")+":"+__bits(fn,"length")+":"+keys(fn);
            }
            var methodNames=["add","has","delete","clear","forEach","isDisjointFrom",
                "isSubsetOf","isSupersetOf","intersection","difference","symmetricDifference",
                "union","values","keys","entries"],methods=[],index;
            for(index=0;index<methodNames.length;index++)
                methods[index]=metadata(Set.prototype,methodNames[index]);
            var sizeDescriptor=Object.getOwnPropertyDescriptor(Set.prototype,"size");
            var speciesDescriptor=Object.getOwnPropertyDescriptor(Set,Symbol.species);
            var sampleIterator=new Set().entries();
            var iteratorPrototype=Object.getPrototypeOf(sampleIterator);
            var arrayIteratorPrototype=Object.getPrototypeOf([][Symbol.iterator]());
            return [
                "global="+__bits(globalThis,"Set")+":"+
                    (Object.getOwnPropertyDescriptor(globalThis,"Set").value===Set),
                "constructor="+Set.name+":"+Set.length+":"+__isConstructor(Set)+":"+keys(Set),
                "links="+(Object.getPrototypeOf(Set)===Function.prototype)+":"+
                    (Object.getPrototypeOf(Set.prototype)===Object.prototype)+":"+
                    (Set.prototype.constructor===Set),
                "descriptors="+__bits(Set,"prototype")+":"+__bits(Set.prototype,"constructor"),
                "prototype-keys="+keys(Set.prototype),
                "prototype-brand="+Object.prototype.toString.call(Set.prototype)+":"+
                    __completion(function(){return Set.prototype.size}),
                "methods="+methods.join(";"),
                "aliases="+(Set.prototype.keys===Set.prototype.values)+":"+
                    (Set.prototype[Symbol.iterator]===Set.prototype.values)+":"+
                    (Set.prototype.entries!==Set.prototype.values)+":"+
                    Set.prototype[Symbol.iterator].name+":"+__bits(Set.prototype,Symbol.iterator),
                "size="+__bits(Set.prototype,"size")+":"+sizeDescriptor.get.name+":"+
                    sizeDescriptor.get.length+":"+__isConstructor(sizeDescriptor.get)+":"+
                    keys(sizeDescriptor.get),
                "tags="+__bits(Set.prototype,Symbol.toStringTag)+":"+
                    Set.prototype[Symbol.toStringTag]+":"+Object.prototype.toString.call(new Set()),
                "species="+__bits(Set,Symbol.species)+":"+speciesDescriptor.get.name+":"+
                    speciesDescriptor.get.length+":"+__isConstructor(speciesDescriptor.get)+":"+
                    keys(speciesDescriptor.get)+":"+(speciesDescriptor.get.call(17)===17),
                "groupBy="+metadata(Set,"groupBy")+":"+(Set.groupBy!==Map.groupBy),
                "iterator="+keys(iteratorPrototype)+":"+
                    metadata(iteratorPrototype,"next")+":"+
                    __bits(iteratorPrototype,Symbol.toStringTag)+":"+
                    iteratorPrototype[Symbol.toStringTag]+":"+
                    (Object.getPrototypeOf(iteratorPrototype)===Object.getPrototypeOf(arrayIteratorPrototype))+":"+
                    (sampleIterator[Symbol.iterator]()===sampleIterator)
            ].join("|");
        })()"#,
        expected: r#"return|string|global=101:true|constructor=Set:0:true:length,name,groupBy,prototype,Symbol(Symbol.species)|links=true:true:true|descriptors=000:101|prototype-keys=add,has,delete,clear,size,forEach,isDisjointFrom,isSubsetOf,isSupersetOf,intersection,difference,symmetricDifference,union,values,keys,entries,constructor,Symbol(Symbol.iterator),Symbol(Symbol.toStringTag)|prototype-brand=[object Set]:throw:TypeError:Set object expected|methods=add:add:1:false:101:001:001:length,name;has:has:1:false:101:001:001:length,name;delete:delete:1:false:101:001:001:length,name;clear:clear:0:false:101:001:001:length,name;forEach:forEach:1:false:101:001:001:length,name;isDisjointFrom:isDisjointFrom:1:false:101:001:001:length,name;isSubsetOf:isSubsetOf:1:false:101:001:001:length,name;isSupersetOf:isSupersetOf:1:false:101:001:001:length,name;intersection:intersection:1:false:101:001:001:length,name;difference:difference:1:false:101:001:001:length,name;symmetricDifference:symmetricDifference:1:false:101:001:001:length,name;union:union:1:false:101:001:001:length,name;values:values:0:false:101:001:001:length,name;keys:values:0:false:101:001:001:length,name;entries:entries:0:false:101:001:001:length,name|aliases=true:true:true:values:101|size=001:get size:0:false:length,name|tags=001:Set:[object Set]|species=001:get [Symbol.species]:0:false:length,name:true|groupBy=groupBy:groupBy:2:false:101:001:001:length,name:true|iterator=next,Symbol(Symbol.toStringTag):next:next:0:false:101:001:001:length,name:001:Set Iterator:true:true"#,
    },
    Case {
        group: "core",
        description: "SameValueZero add has delete clear entries and negative-zero normalization",
        source: r#"(function(){
            var firstObject=Object(),secondObject=Object(),set=new Set(),values=[],step,value;
            var sameReturn=set.add(NaN)===set;
            set.add(0/0).add(-0).add(+0).add(1n).add(BigInt("1")).add(1);
            set.add(firstObject).add(secondObject).add(undefined).add(firstObject);
            var iterator=set.values();
            while(!(step=iterator.next()).done){
                value=step.value;
                if(value===firstObject)values[values.length]="firstObject";
                else if(value===secondObject)values[values.length]="secondObject";
                else if(typeof value==="number")values[values.length]=__number(value);
                else if(typeof value==="bigint")values[values.length]=String(value)+"n";
                else values[values.length]=String(value);
            }
            var entry=new Set([firstObject]).entries().next().value;
            var deleted=set.delete(0/0),afterDelete=set.size,cleared=set.clear();
            set.add(-0);
            var normalized=set.values().next().value;
            return [sameReturn,values.join(","),set.has(+0),set.has(-0),set.has(NaN),
                set.has(1n),set.has(1),set.has(Object()),entry[0]===firstObject,
                entry[1]===firstObject,deleted,afterDelete,cleared===undefined,set.size,
                __number(normalized)].join("|");
        })()"#,
        expected: r#"return|string|true|NaN,+0,1n,1,firstObject,secondObject,undefined|true|true|false|false|false|false|true|true|true|6|true|1|+0"#,
    },
    Case {
        group: "constructor",
        description: "newTarget prototype cached adder and iterator observations follow QuickJS order",
        source: r#"(function(){
            var log="",custom=Object.create(Set.prototype),iterator=Object(),count=0;
            var originalAdd=Set.prototype.add;
            var NewTarget=(function(){}).bind(null);
            Object.defineProperty(NewTarget,"prototype",{
                configurable:true,
                get:function(){log+="prototype;";return custom}
            });
            Object.defineProperty(custom,"add",{
                configurable:true,
                get:function(){
                    log+="add-get;";
                    return function(value){
                        log+="adder:"+value+";";
                        Object.defineProperty(iterator,"next",{
                            configurable:true,writable:true,
                            value:function(){throw "changed-next"}
                        });
                        return originalAdd.call(this,value);
                    };
                }
            });
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
                            get:function(){log+="step-value;";return 7}
                        });
                        return result;
                    };
                }
            });
            var set=Reflect.construct(Set,[iterable],NewTarget);
            function Fallback(){}Fallback.prototype=17;
            var fallback=Reflect.construct(Set,[],Fallback);
            return [log,Object.getPrototypeOf(set)===custom,set.size,set.has(7),
                Object.getPrototypeOf(fallback)===Set.prototype,new Set(null).size,
                __completion(function(){return Set()})].join("|");
        })()"#,
        expected: r#"return|string|prototype;add-get;iterator-get;iterator-call;next-get;next-call;done:false;step-value;adder:7;next-call;done:true;|true|1|true|true|0|throw:TypeError:must be called with new"#,
    },
    Case {
        group: "constructor",
        description: "adder failure closes while iterator acquisition and step failures do not",
        source: r#"(function(){
            function invalidAdder(){
                var log="",prototype=Object.create(Set.prototype),iterable=Object();
                Object.defineProperty(prototype,"add",{get:function(){log+="add-get;";return 0}});
                Object.defineProperty(iterable,Symbol.iterator,{get:function(){log+="iterator-get;";throw "iterator"}});
                function Target(){}Target.prototype=prototype;
                try{Reflect.construct(Set,[iterable],Target);return log+"missing"}
                catch(error){return log+(error!==null&&typeof error==="object"?error.name:String(error))}
            }
            function primitiveIterator(){
                var log="",iterable=Object();
                Object.defineProperty(iterable,Symbol.iterator,{get:function(){
                    log+="iterator-get;";return function(){log+="iterator-call;";return 1};
                }});
                try{new Set(iterable);return log+"missing"}
                catch(error){return log+(error!==null&&typeof error==="object"?error.name:String(error))}
            }
            function run(mode){
                var log="",iterator=Object(),iterable=Object(),count=0;
                var prototype=Object.create(Set.prototype),originalAdd=Set.prototype.add;
                prototype.add=function(value){
                    log+="adder;";
                    if(mode===5)throw "adder";
                    return originalAdd.call(this,value);
                };
                function Target(){}Target.prototype=prototype;
                iterable[Symbol.iterator]=function(){log+="iterator;";return iterator};
                Object.defineProperty(iterator,"return",{get:function(){
                    log+="return-get;";
                    return function(){log+="return-call;";throw "close"};
                }});
                Object.defineProperty(iterator,"next",{get:function(){
                    log+="next-get;";
                    if(mode===1)throw "next-get";
                    return function(){
                        log+="next-call;";
                        if(mode===2)throw "next-call";
                        var result=Object(),done=count++!==0;
                        Object.defineProperty(result,"done",{get:function(){
                            log+="done;";if(mode===3)throw "done";return done;
                        }});
                        Object.defineProperty(result,"value",{get:function(){
                            log+="value;";if(mode===4)throw "value";return 9;
                        }});
                        return result;
                    };
                }});
                try{Reflect.construct(Set,[iterable],Target);return log+"ok"}
                catch(error){return log+"catch:"+(error!==null&&typeof error==="object"?error.name:String(error))}
            }
            var output=[invalidAdder(),primitiveIterator()],mode;
            for(mode=1;mode<=5;mode++)output[output.length]=mode+":"+run(mode);
            return output.join("|");
        })()"#,
        expected: r#"return|string|add-get;TypeError|iterator-get;iterator-call;TypeError|1:iterator;next-get;catch:next-get|2:iterator;next-get;next-call;catch:next-call|3:iterator;next-get;next-call;done;catch:done|4:iterator;next-get;next-call;done;value;catch:value|5:iterator;next-get;next-call;done;value;adder;return-get;return-call;catch:adder"#,
    },
    Case {
        group: "iteration",
        description: "delete clear re-add and exhaustion retain live Set Iterator behavior",
        source: r#"(function(){
            function value(step){return step.done?"done":String(step.value)}
            function entry(step){return step.done?"done":step.value[0]+":"+step.value[1]}
            var set=new Set(["a","b","c"]),output=[],iterator=set.values();
            output[output.length]=value(iterator.next());
            set.delete("a");set.delete("b");set.add("d");set.add("a");
            output[output.length]=value(iterator.next());
            output[output.length]=value(iterator.next());
            output[output.length]=value(iterator.next());
            set.clear();set.add("e");
            output[output.length]=value(iterator.next());
            output[output.length]=value(iterator.next());
            set.add("f");
            output[output.length]=value(iterator.next());

            set=new Set(["x","y"]);iterator=set.values();
            output[output.length]=value(iterator.next());
            set.clear();set.add("z");
            output[output.length]=value(iterator.next());
            output[output.length]=value(iterator.next());

            set=new Set(["p"]);iterator=set.entries();
            output[output.length]=entry(iterator.next());
            output[output.length]=entry(iterator.next());
            return output.join("|");
        })()"#,
        expected: r#"return|string|a|c|d|a|e|done|done|x|z|done|p:p|done"#,
    },
    Case {
        group: "iteration",
        description: "forEach locks current records and observes reentrant deletion and appends",
        source: r#"(function(){
            var set=new Set(["a","b"]),receiver={marker:1},log=[];
            set.forEach(function(value,key,observed){
                log[log.length]="outer:"+value+":"+(value===key)+":"+
                    (this===receiver)+":"+(observed===set);
                if(value==="a"){
                    observed.delete("b");observed.add("c");
                    observed.forEach(function(innerValue,innerKey,innerSet){
                        log[log.length]="inner:"+innerValue+":"+(innerValue===innerKey)+":"+
                            (this===receiver)+":"+(innerSet===set);
                        if(innerValue==="a")innerSet.add("d");
                        if(innerValue==="c")innerSet.delete("c");
                    },receiver);
                }
                if(value==="d"){
                    observed.delete("d");observed.add("e");
                }
            },receiver);
            return log.join(";")+"|"+set.size+"|"+set.has("a")+"|"+
                set.has("b")+"|"+set.has("c")+"|"+set.has("d")+"|"+set.has("e");
        })()"#,
        expected: r#"return|string|outer:a:true:true:true;inner:a:true:true:true;inner:c:true:true:true;inner:d:true:true:true;outer:d:true:true:true;outer:e:true:true:true|2|true|false|false|false|true"#,
    },
    Case {
        group: "methods",
        description: "all seven Set methods preserve order and bypass constructor species and add",
        source: r#"(function(){
            var a=new Set([1,2,3,-0,NaN]),b=new Set([3,4,+0,0/0]);
            var union=a.union(b),intersection=a.intersection(b),difference=a.difference(b);
            var symmetric=a.symmetricDifference(b);
            var subset=a.isSubsetOf(union),superset=a.isSupersetOf(intersection);
            var disjoint=a.isDisjointFrom(new Set([8,9]));
            var originalAdd=Set.prototype.add,addCalls=0,constructorGets=0;
            Set.prototype.add=function(value){
                addCalls++;return originalAdd.call(this,value);
            };
            Object.defineProperty(a,"constructor",{get:function(){constructorGets++;return Set}});
            var custom=Object.create(Set.prototype);
            Object.setPrototypeOf(a,custom);
            var direct=Set.prototype.union.call(a,b);
            Set.prototype.add=originalAdd;
            return [__setValues(union),__setValues(intersection),__setValues(difference),
                __setValues(symmetric),subset,superset,disjoint,
                Object.getPrototypeOf(direct)===Set.prototype,__setValues(direct),
                addCalls,constructorGets].join("|");
        })()"#,
        expected: r#"return|string|1,2,3,+0,NaN,4|3,+0,NaN|1,2|1,2,4|true|true|true|true|1,2,3,+0,NaN,4|0|0"#,
    },
    Case {
        group: "protocol",
        description: "GetSetRecord order and size-selected traversal match pinned QuickJS",
        source: r#"(function(){
            function run(method,left,right,declaredSize){
                var log="",index=0,iterator=Object(),other=Object();
                iterator.next=function(){
                    log+="next;";
                    if(index>=right.length)return {done:true,value:undefined};
                    return {done:false,value:right[index++]};
                };
                iterator.return=function(){log+="return;";return iterator};
                Object.defineProperty(other,"size",{get:function(){log+="size;";return declaredSize}});
                Object.defineProperty(other,"has",{get:function(){
                    log+="has-get;";
                    return function(value){
                        var found=false,candidate,i;
                        log+="has:"+value+";";
                        for(i=0;i<right.length;i++){
                            candidate=right[i];
                            if(candidate===value||(candidate!==candidate&&value!==value))found=true;
                        }
                        return found;
                    };
                }});
                Object.defineProperty(other,"keys",{get:function(){
                    log+="keys-get;";
                    return function(){log+="keys-call;";return iterator};
                }});
                var result=Set.prototype[method].call(new Set(left),other);
                return log+"=>"+(result!==null&&typeof result==="object"?__setValues(result):String(result));
            }
            var genuine=new Set([2]),genuineLog="",originalHas=Set.prototype.has;
            var originalKeys=Set.prototype.keys;
            Object.defineProperty(genuine,"size",{get:function(){genuineLog+="size;";return 99}});
            Object.defineProperty(genuine,"has",{get:function(){
                genuineLog+="has-get;";
                return function(value){genuineLog+="has-call;";return originalHas.call(this,value)};
            }});
            Object.defineProperty(genuine,"keys",{get:function(){
                genuineLog+="keys-get;";
                return function(){genuineLog+="keys-call;";return originalKeys.call(this)};
            }});
            var genuineResult=new Set([2]).isSubsetOf(genuine);
            return [
                run("isDisjointFrom",[1],[2,3],2),
                run("isDisjointFrom",[1,2,3],[2],1),
                run("intersection",[1,2,3],[3,2],2),
                run("intersection",[1,2],[2],3),
                genuineLog+"=>"+genuineResult
            ].join("|");
        })()"#,
        expected: r#"return|string|size;has-get;keys-get;has:1;=>true|size;has-get;keys-get;keys-call;next;return;=>false|size;has-get;keys-get;keys-call;next;next;next;=>3,2|size;has-get;keys-get;has:1;has:2;=>2|has-get;keys-get;has-call;=>true"#,
    },
    Case {
        group: "protocol",
        description: "GetSetRecord validation and iterator-close boundaries preserve abrupt order",
        source: r#"(function(){
            function record(size,has,keys,log){
                var object=Object();
                Object.defineProperty(object,"size",{get:function(){log.text+="size;";return size}});
                Object.defineProperty(object,"has",{get:function(){log.text+="has;";return has}});
                Object.defineProperty(object,"keys",{get:function(){log.text+="keys;";return keys}});
                return object;
            }
            function validation(size,has,keys){
                var log={text:""},other=record(size,has,keys,log);
                var completion=__completion(function(){return new Set().union(other)});
                return log.text+completion;
            }
            var brandLog={text:""},brandOther=record(0,function(){},function(){},brandLog);
            var brand=__completion(function(){return Set.prototype.union.call(Object(),brandOther)});

            var nextLog="",nextIterator=Object(),nextOther=Object();
            nextIterator.next=function(){nextLog+="next;";throw "next"};
            nextIterator.return=function(){nextLog+="return;";return nextIterator};
            nextOther.size=0;nextOther.has=function(){};
            nextOther.keys=function(){nextLog+="keys;";return nextIterator};
            var nextCompletion=__completion(function(){return new Set([1]).union(nextOther)});

            var closeLog="",closeIterator=Object(),closeOther=Object();
            closeIterator.next=function(){closeLog+="next;";return {done:false,value:2}};
            closeIterator.return=function(){closeLog+="return;";throw "close"};
            closeOther.size=1;closeOther.has=function(){};
            closeOther.keys=function(){closeLog+="keys;";return closeIterator};
            var closeCompletion=__completion(function(){
                return new Set([1,2]).isDisjointFrom(closeOther);
            });

            var primitiveLog="",originalNumberNext=Number.prototype.next;
            Number.prototype.next=function(){primitiveLog+="next:"+this+";";return {done:true}};
            var primitiveOther=Object();
            primitiveOther.size=0;primitiveOther.has=function(){};
            primitiveOther.keys=function(){primitiveLog+="keys;";return 7};
            var primitiveCompletion=__completion(function(){
                return new Set().union(primitiveOther).size;
            });
            if(originalNumberNext===undefined)delete Number.prototype.next;
            else Number.prototype.next=originalNumberNext;
            function nullishIterator(value){
                var other=Object();other.size=0;other.has=function(){};
                other.keys=function(){return value};
                return __completion(function(){return new Set().union(other)});
            }
            return [
                brandLog.text+brand,
                validation(NaN,function(){},function(){}),
                validation(-1,function(){},function(){}),
                validation(0n,function(){},function(){}),
                validation(0,undefined,function(){}),
                validation(0,function(){},undefined),
                __completion(function(){return new Set().union(null)}),
                __completion(function(){return new Set().union(undefined)}),
                nullishIterator(null),
                nullishIterator(undefined),
                nextLog+nextCompletion,
                closeLog+closeCompletion,
                primitiveLog+primitiveCompletion
            ].join("|");
        })()"#,
        expected: r#"return|string|throw:TypeError:Set object expected|size;throw:TypeError:.size is not a number|size;throw:RangeError:.size must be positive|size;throw:TypeError:cannot convert bigint to number|size;has;throw:TypeError:.has is undefined|size;has;keys;throw:TypeError:.keys is undefined|throw:TypeError:cannot read property 'size' of null|throw:TypeError:cannot read property 'size' of undefined|throw:TypeError:cannot read property 'next' of null|throw:TypeError:cannot read property 'next' of undefined|keys;next;throw:string:next|keys;next;return;return:false|keys;next:7;return:0"#,
    },
    Case {
        group: "methods",
        description: "symmetricDifference retains copied order under mutating set-like iteration",
        source: r#"(function(){
            var receiver=new Set([1,2]),index=0,iterator=Object(),other=Object();
            iterator.next=function(){
                if(index===0){index++;receiver.delete(2);return {done:false,value:2}}
                if(index===1){index++;return {done:false,value:3}}
                return {done:true,value:undefined};
            };
            other.size=2;
            other.has=function(){return false};
            other.keys=function(){return iterator};
            var result=receiver.symmetricDifference(other);
            return [__setValues(receiver),__setValues(result),result!==receiver,
                Object.getPrototypeOf(result)===Set.prototype].join("|");
        })()"#,
        expected: r#"return|string|1|1,2,3|true|true"#,
    },
    Case {
        group: "groupBy",
        description: "Set.groupBy is a distinct static function which returns a strong Map",
        source: r#"(function(){
            var objectKey=Object(),coerced=false,log="";
            objectKey.toString=function(){coerced=true;throw "coerced"};
            var values=[-0,+0,NaN,0/0,1n,1,"object"],keys=[];
            var grouped=Set.groupBy.call(Object(),values,function(value,index){
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
            return [Set.groupBy!==Map.groupBy,grouped instanceof Map,grouped instanceof Set,
                Object.getPrototypeOf(grouped)===Map.prototype,grouped.size,keys.join(","),
                grouped.get(-0).length,grouped.get(NaN).length,
                grouped.get(1n)[0]===1n,grouped.get(1)[0]===1,
                grouped.get(objectKey)[0],coerced,log].join("|");
        })()"#,
        expected: r#"return|string|true|true|false|true|5|+0,NaN,1n,1,object|2|2|true|true|object|false|true:0;true:1;true:2;true:3;true:4;true:5;true:6;"#,
    },
    Case {
        group: "groupBy",
        description: "Set.groupBy callback validation and abrupt iterator-close ordering match Map.groupBy",
        source: r#"(function(){
            function invalidCallback(){
                var log="",iterable=Object();
                Object.defineProperty(iterable,Symbol.iterator,{get:function(){log+="touched;";throw "iterator"}});
                try{Set.groupBy(iterable,0);return log+"missing"}
                catch(error){return log+(error!==null&&typeof error==="object"?error.name:String(error))}
            }
            function callbackThrow(){
                var log="",iterator=Object(),iterable=Object();
                iterable[Symbol.iterator]=function(){log+="iterator;";return iterator};
                iterator.next=function(){
                    log+="next;";var result=Object();result.done=false;result.value=7;return result;
                };
                iterator.return=function(){log+="return;";throw "close"};
                try{Set.groupBy(iterable,function(){log+="callback;";throw "callback"});return log+"missing"}
                catch(error){return log+String(error)}
            }
            function nextThrow(){
                var log="",iterator=Object(),iterable=Object();
                iterable[Symbol.iterator]=function(){return iterator};
                iterator.next=function(){log+="next;";throw "next"};
                iterator.return=function(){log+="return;"};
                try{Set.groupBy(iterable,function(){return 0});return log+"missing"}
                catch(error){return log+String(error)}
            }
            return invalidCallback()+"|"+callbackThrow()+"|"+nextThrow()+"|"+
                __completion(function(){return new Set.groupBy([],function(){return 0})});
        })()"#,
        expected: r#"return|string|TypeError|iterator;next;callback;return;callback|next;next|throw:TypeError:groupBy is not a constructor"#,
    },
    Case {
        group: "brands",
        description: "Set Map and iterator methods enforce independent brands and constructability",
        source: r#"(function(){
            var setIteratorNext=Object.getPrototypeOf(new Set().values()).next;
            var mapIteratorNext=Object.getPrototypeOf(new Map().entries()).next;
            var setIterator=new Set([1]).values(),mapIterator=new Map([[1,2]]).entries();
            return [
                __completion(function(){return Set.prototype.add.call(new Map(),1)}),
                __completion(function(){return Map.prototype.set.call(new Set(),1,2)}),
                __completion(function(){return Object.getOwnPropertyDescriptor(Set.prototype,"size").get.call(Object())}),
                __completion(function(){return Set.prototype.union.call(Object(),new Set())}),
                __completion(function(){return setIteratorNext.call(mapIterator)}),
                __completion(function(){return mapIteratorNext.call(setIterator)}),
                __completion(function(){return new Set.prototype.add()}),
                __completion(function(){return Set.prototype.add.call(new Set(),"x") instanceof Set}),
                __completion(function(){return new Set.groupBy([],function(){return 0})})
            ].join("|");
        })()"#,
        expected: r#"return|string|throw:TypeError:Set object expected|throw:TypeError:Map object expected|throw:TypeError:Set object expected|throw:TypeError:Set object expected|throw:TypeError:Set Iterator object expected|throw:TypeError:Map Iterator object expected|throw:TypeError:add is not a constructor|return:true|throw:TypeError:groupBy is not a constructor"#,
    },
];

#[test]
fn set_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Set oracle self-check: set QJS_ORACLE to pinned upstream qjs");
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
        "pinned QuickJS Set vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn set_graph_core_and_constructor_match_pinned_quickjs() {
    compare_groups(&["graph", "core", "constructor"]);
}

#[test]
fn set_live_iteration_and_for_each_match_pinned_quickjs() {
    compare_groups(&["iteration"]);
}

#[test]
fn set_methods_and_set_like_protocol_match_pinned_quickjs() {
    compare_groups(&["methods", "protocol"]);
}

#[test]
fn set_group_by_and_brand_errors_match_pinned_quickjs() {
    compare_groups(&["groupBy", "brands"]);
}

#[test]
fn set_constructor_and_native_errors_use_exact_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let defining_constructor = global_callable(&runtime, &mut defining, "Set");
    let defining_set_prototype =
        eval_object(&mut defining, "Set.prototype", "defining Set prototype");
    let caller_set_prototype = eval_object(&mut caller, "Set.prototype", "caller Set prototype");
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
    assert_ne!(defining_set_prototype, caller_set_prototype);
    assert_ne!(defining_type_error, caller_type_error);

    let foreign_set = expect_object(
        caller.construct(&defining_constructor, &[]).unwrap(),
        "foreign Set construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&foreign_set).unwrap(),
        Some(defining_set_prototype.clone()),
        "Set construction did not use the foreign constructor prototype",
    );

    let caller_new_target = eval_callable(
        &runtime,
        &mut caller,
        "(function(){function F(){}F.prototype=17;return F})()",
        "caller primitive-prototype newTarget",
    );
    let fallback_set = expect_object(
        caller
            .construct_with_new_target(&defining_constructor, &caller_new_target, &[])
            .unwrap(),
        "cross-realm Set fallback construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&fallback_set).unwrap(),
        Some(caller_set_prototype),
        "primitive newTarget.prototype did not fall back to the newTarget realm",
    );

    let add = property_callable(&runtime, &mut defining, &defining_set_prototype, "add");
    let added = caller
        .call(&add, Value::Object(foreign_set), &[Value::Int(1)])
        .unwrap();
    assert!(matches!(added, Value::Object(_)));

    let ordinary = eval_object(&mut caller, "({})", "caller ordinary object");
    assert_eq!(
        caller.call(&add, Value::Object(ordinary), &[Value::Int(1)]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "foreign Set.add TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Set branded native error used the calling realm",
    );
}

fn compare_groups(groups: &[&str]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Set differential: set QJS_ORACLE to pinned upstream qjs");
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
        "Set semantics drifted in {} case(s):\n\n{}",
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
