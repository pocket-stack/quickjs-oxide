use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray iteration test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

#[test]
fn callback_iteration_matches_quickjs_callback_and_descriptor_contracts() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function completion(operation){
                try{operation();return "return"}
                catch(error){return error.name}
            }

            var bytes=new Uint8Array([2,4,7,8]);
            var everyOrder=[];
            check("every false",bytes.every(function(value,index){
                everyOrder.push(index);
                return value%2===0;
            })===false);
            check("every short circuits",everyOrder.join(",")==="0,1,2");

            var someOrder=[];
            check("some true",bytes.some(function(value,index){
                someOrder.push(index);
                return value===7;
            })===true);
            check("some short circuits",someOrder.join(",")==="0,1,2");
            check("every true",
                new Uint8Array([2,4,6]).every(function(value){
                    return value%2===0;
                })===true);
            check("some false",
                new Uint8Array([1,3,5]).some(function(value){
                    return value%2===0;
                })===false);
            var sentinel={sentinel:true};
            var forEachOrder=[];
            check("forEach undefined",bytes.forEach(function(value,index){
                forEachOrder.push(value+":"+index);
                return index===0;
            })===undefined);
            check("forEach never short circuits",
                forEachOrder.join(",")==="2:0,4:1,7:2,8:3");
            var forEachThis=true;
            var forEachReceiver=true;
            bytes.forEach(function(value,index,receiver){
                "use strict";
                forEachThis=forEachThis && this===sentinel;
                forEachReceiver=forEachReceiver &&
                    receiver===bytes && receiver[index]===value;
            },sentinel);
            check("forEach thisArg",forEachThis);
            check("forEach receiver",forEachReceiver);
            check("bigint every",
                new BigInt64Array([1n,2n,3n]).every(function(value){
                    return value>0n;
                })===true);
            check("bigint some",
                new BigInt64Array([1n,-2n,3n]).some(function(value){
                    return value<0n;
                })===true);
            var bigintTotal=0n;
            new BigInt64Array([1n,2n,3n]).forEach(function(value){
                bigintTotal+=value;
            });
            check("bigint forEach",bigintTotal===6n);

            var seenValues=[];
            var seenIndices=[];
            var receiverMatches=true;
            var thisMatches=true;
            check("callback result",bytes.every(function(value,index,receiver){
                "use strict";
                seenValues.push(value);
                seenIndices.push(index);
                receiverMatches=receiverMatches && receiver===bytes;
                thisMatches=thisMatches && this===sentinel;
                return true;
            },sentinel)===true);
            check("callback values",seenValues.join(",")==="2,4,7,8");
            check("callback indices",seenIndices.join(",")==="0,1,2,3");
            check("callback receiver",receiverMatches);
            check("callback thisArg",thisMatches);

            var lengthHits=0;
            Object.defineProperty(bytes,"length",{
                configurable:true,
                get:function(){
                    lengthHits++;
                    return 0;
                }
            });
            check("internal length snapshot",
                bytes.every(function(){return true})===true);
            check("length property ignored",lengthHits===0);

            var omittedThis=null;
            bytes.some(function(){
                "use strict";
                omittedThis=this;
                return true;
            });
            check("omitted thisArg",omittedThis===undefined);

            var called=false;
            check("empty every true",
                new Uint8Array(0).every(function(){
                    called=true;
                    return false;
                })===true);
            check("empty some false",
                new Uint8Array(0).some(function(){
                    called=true;
                    return true;
                })===false);
            check("empty skips callback",called===false);
            check("empty validates callback",
                completion(function(){
                    new Uint8Array(0).every();
                })==="TypeError");

            var coerced=false;
            var truthy={
                valueOf:function(){
                    coerced=true;
                    throw new Error("not reached");
                }
            };
            check("callback object is truthy",
                new Uint8Array([1]).every(function(){return truthy})===true);
            check("truthiness has no coercion",coerced===false);

            var marker={marker:true};
            var caught;
            try{
                bytes.some(function(){throw marker});
            }catch(error){
                caught=error;
            }
            check("callback abrupt identity",caught===marker);
            var forEachAbrupt=[];
            caught=undefined;
            try{
                bytes.forEach(function(value,index){
                    forEachAbrupt.push(index);
                    if(index===1) throw marker;
                });
            }catch(error){
                caught=error;
            }
            check("forEach abrupt identity",caught===marker);
            check("forEach abrupt stops",forEachAbrupt.join(",")==="0,1");

            var generatorBody=false;
            function* generatorCallback(){
                generatorBody=true;
            }
            check("forEach ignores generator result",
                bytes.forEach(generatorCallback)===undefined);
            check("generator body not entered",generatorBody===false);

            check("brand validation",
                completion(function(){
                    Uint8Array.prototype.every.call({},function(){return true});
                })==="TypeError");
            check("proxy is not branded",
                completion(function(){
                    Uint8Array.prototype.some.call(
                        new Proxy(bytes,{}),
                        function(){return true}
                    );
                })==="TypeError");
            var forEachError;
            try{
                Uint8Array.prototype.forEach.call({},function(){});
            }catch(error){
                forEachError=error.name+":"+error.message;
            }
            check("forEach exact brand error",
                forEachError==="TypeError:not a TypedArray");
            check("every not constructor",
                completion(function(){
                    new (Object.getPrototypeOf(Uint8Array.prototype).every)(
                        function(){return true}
                    );
                })==="TypeError");
            forEachError=undefined;
            try{
                new (Object.getPrototypeOf(Uint8Array.prototype).forEach)(
                    function(){}
                );
            }catch(error){
                forEachError=error.name+":"+error.message;
            }
            check("forEach exact constructor error",
                forEachError==="TypeError:forEach is not a constructor");

            var base=Object.getPrototypeOf(Uint8Array.prototype);
            check("QuickJS own-key order",
                Reflect.ownKeys(base).map(function(key){
                    return typeof key==="symbol" ? key.toString() : key;
                }).join("|")===
                "length|at|buffer|byteLength|byteOffset|set|values|keys|"+
                "entries|copyWithin|every|some|forEach|map|filter|reduce|"+
                "reduceRight|fill|find|findIndex|findLast|findLastIndex|"+
                "reverse|slice|subarray|indexOf|lastIndexOf|includes|"+
                "constructor|toString|Symbol(Symbol.iterator)|"+
                "Symbol(Symbol.toStringTag)");
            for(var name of ["every","some","forEach","map","filter"]){
                var descriptor=Object.getOwnPropertyDescriptor(base,name);
                check(name+" value",typeof descriptor.value==="function");
                check(name+" name",descriptor.value.name===name);
                check(name+" length",descriptor.value.length===1);
                check(name+" writable",descriptor.writable===true);
                check(name+" enumerable",descriptor.enumerable===false);
                check(name+" configurable",descriptor.configurable===true);
            }

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn callback_iteration_keeps_snapshot_range_but_reads_each_value_live() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function completion(operation){
                try{operation();return "return"}
                catch(error){return error.name}
            }
            function printable(value){
                return value===undefined ? "undefined" : String(value);
            }

            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            var tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            var seen=[];
            check("forEach result",tracking.forEach(function(value,index){
                seen.push(printable(value)+":"+index);
                if(index===0) buffer.resize(1);
                return false;
            })===undefined);
            check("shrink visits snapshot range",
                seen.join(",")==="1:0,undefined:1,undefined:2,undefined:3");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            check("some can match disappeared slot",
                tracking.some(function(value,index){
                    if(index===0) buffer.resize(1);
                    return index===2 && value===undefined;
                })===true);

            buffer=new ArrayBuffer(2,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            seen=[];
            tracking.every(function(value,index){
                seen.push(printable(value)+":"+index);
                if(index===0){
                    buffer.resize(4);
                    tracking[2]=7;
                    tracking[3]=8;
                }
                return true;
            });
            check("grow does not extend snapshot range",
                seen.join(",")==="1:0,2:1");

            tracking=new Uint8Array([1,2,3]);
            seen=[];
            tracking.every(function(value,index,receiver){
                seen.push(value);
                if(index===0) receiver[1]=9;
                return true;
            });
            check("later writes are live",seen.join(",")==="1,9,3");

            buffer=new ArrayBuffer(4);
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            seen=[];
            tracking.forEach(function(value,index){
                seen.push(printable(value)+":"+index);
                if(index===0) buffer.transfer();
            });
            check("detach visits snapshot range",
                seen.join(",")==="1:0,undefined:1,undefined:2,undefined:3");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,0,4);
            fixed.set([1,2,3,4]);
            seen=[];
            fixed.every(function(value,index){
                seen.push(printable(value));
                if(index===0) buffer.resize(2);
                if(index===1) buffer.resize(4);
                return true;
            });
            check("fixed oob can regrow during callbacks",
                seen.join(",")==="1,undefined,0,0");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            fixed=new Uint8Array(buffer,0,4);
            buffer.resize(2);
            called=false;
            check("initial fixed oob rejects",
                completion(function(){
                    fixed.some(function(){called=true;return false});
                })==="TypeError");
            check("initial fixed oob skips callback",called===false);
            var priority;
            try{
                fixed.forEach(0);
            }catch(error){
                priority=error.name+":"+error.message;
            }
            check("forEach oob precedes callable validation",
                priority==="TypeError:ArrayBuffer is detached or resized");
            var emptyPriority;
            try{
                new Uint8Array(0).forEach(0);
            }catch(error){
                emptyPriority=error.name+":"+error.message;
            }
            check("empty forEach validates callable after receiver",
                emptyPriority==="TypeError:not a function");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            var base=Object.getPrototypeOf(Uint8Array.prototype);
            var prototypeHits=0;
            Object.defineProperty(base,"2",{
                configurable:true,
                get:function(){
                    prototypeHits++;
                    return 88;
                }
            });
            seen=[];
            tracking.every(function(value,index){
                seen.push(printable(value));
                if(index===0) buffer.resize(1);
                return true;
            });
            delete base["2"];
            check("missing integer skips prototype",prototypeHits===0);
            check("prototype did not replace undefined",
                seen.join(",")==="1,undefined,undefined,undefined");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}
