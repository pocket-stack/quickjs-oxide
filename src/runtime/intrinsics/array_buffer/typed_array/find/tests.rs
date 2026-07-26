use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray find test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

#[test]
fn find_family_matches_quickjs_callback_and_descriptor_contracts() {
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

            var bytes=new Uint8Array([1,2,3,2]);
            check("find",bytes.find(function(value){
                return value===2;
            })===2);
            check("findIndex",bytes.findIndex(function(value){
                return value===2;
            })===1);
            check("findLast",bytes.findLast(function(value){
                return value===2;
            })===2);
            check("findLastIndex",bytes.findLastIndex(function(value){
                return value===2;
            })===3);
            check("find miss",bytes.find(function(){
                return false;
            })===undefined);
            check("findIndex miss",bytes.findIndex(function(){
                return false;
            })===-1);
            check("findLastIndex miss",bytes.findLastIndex(function(){
                return false;
            })===-1);

            var bigints=new BigInt64Array([1n,-2n,3n]);
            check("bigint find",bigints.find(function(value){
                return value<0n;
            })===-2n);
            check("bigint findLastIndex",bigints.findLastIndex(function(value){
                return value>0n;
            })===2);

            var sentinel={sentinel:true};
            var seenValues=[];
            var seenIndices=[];
            var receiverMatches=true;
            var thisMatches=true;
            var result=bytes.find(function(value,index,receiver){
                "use strict";
                seenValues.push(value);
                seenIndices.push(index);
                receiverMatches=receiverMatches && receiver===bytes;
                thisMatches=thisMatches && this===sentinel;
                return index===2;
            },sentinel);
            check("callback result",result===3);
            check("callback values",seenValues.join(",")==="1,2,3");
            check("callback indices",seenIndices.join(",")==="0,1,2");
            check("callback receiver",receiverMatches);
            check("callback thisArg",thisMatches);

            var omittedThis=null;
            bytes.find(function(){
                "use strict";
                omittedThis=this;
                return true;
            });
            check("omitted thisArg",omittedThis===undefined);

            var reverseIndices=[];
            bytes.findLast(function(value,index){
                reverseIndices.push(index);
                return index===1;
            });
            check("reverse callback order",reverseIndices.join(",")==="3,2,1");

            var marker={marker:true};
            var caught;
            try{
                bytes.find(function(){throw marker});
            }catch(error){
                caught=error;
            }
            check("callback abrupt identity",caught===marker);

            var captured=new Uint8Array([7]);
            check("find returns captured value",
                captured.find(function(value,index,receiver){
                    receiver[index]=9;
                    return true;
                })===7);
            check("callback mutation landed",captured[0]===9);

            check("empty validates callback",
                completion(function(){
                    new Uint8Array(0).find();
                })==="TypeError");
            check("brand validation",
                completion(function(){
                    Uint8Array.prototype.find.call({},function(){return true});
                })==="TypeError");
            check("proxy is not branded",
                completion(function(){
                    Uint8Array.prototype.find.call(
                        new Proxy(bytes,{}),
                        function(){return true}
                    );
                })==="TypeError");
            check("find not constructor",
                completion(function(){
                    new (Object.getPrototypeOf(Uint8Array.prototype).find)(
                        function(){return true}
                    );
                })==="TypeError");

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
            for(var name of ["find","findIndex","findLast","findLastIndex"]){
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
fn find_family_keeps_snapshot_range_but_reads_each_value_live() {
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
            tracking.find(function(value,index){
                seen.push(printable(value)+":"+index);
                if(index===0) buffer.resize(1);
                return false;
            });
            check("shrink visits snapshot range",
                seen.join(",")==="1:0,undefined:1,undefined:2,undefined:3");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            check("findIndex can match disappeared slot",
                tracking.findIndex(function(value,index){
                    if(index===0) buffer.resize(1);
                    return index===2 && value===undefined;
                })===2);

            buffer=new ArrayBuffer(2,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            seen=[];
            tracking.find(function(value,index){
                seen.push(printable(value)+":"+index);
                if(index===0){
                    buffer.resize(4);
                    tracking[2]=7;
                    tracking[3]=8;
                }
                return false;
            });
            check("grow does not extend snapshot range",
                seen.join(",")==="1:0,2:1");

            tracking=new Uint8Array([1,2,3]);
            seen=[];
            tracking.find(function(value,index,receiver){
                seen.push(value);
                if(index===0) receiver[1]=9;
                return false;
            });
            check("later writes are live",seen.join(",")==="1,9,3");

            buffer=new ArrayBuffer(4);
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            seen=[];
            tracking.find(function(value,index){
                seen.push(printable(value)+":"+index);
                if(index===0) buffer.transfer();
                return false;
            });
            check("detach visits snapshot range",
                seen.join(",")==="1:0,undefined:1,undefined:2,undefined:3");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            seen=[];
            tracking.findLast(function(value,index){
                seen.push(printable(value)+":"+index);
                if(index===3) buffer.resize(1);
                return false;
            });
            check("reverse shrink order",
                seen.join(",")==="4:3,undefined:2,undefined:1,1:0");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            check("findLastIndex can match disappeared slot",
                tracking.findLastIndex(function(value,index){
                    if(index===3) buffer.resize(1);
                    return index===2 && value===undefined;
                })===2);

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,0,4);
            fixed.set([1,2,3,4]);
            seen=[];
            fixed.find(function(value,index){
                seen.push(printable(value));
                if(index===0) buffer.resize(2);
                if(index===1) buffer.resize(4);
                return false;
            });
            check("fixed oob can regrow during callbacks",
                seen.join(",")==="1,undefined,0,0");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            fixed=new Uint8Array(buffer,0,4);
            buffer.resize(2);
            var called=false;
            check("initial fixed oob rejects",
                completion(function(){
                    fixed.find(function(){called=true;return false});
                })==="TypeError");
            check("initial fixed oob skips callback",called===false);

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
            tracking.find(function(value,index){
                seen.push(printable(value));
                if(index===0) buffer.resize(1);
                return false;
            });
            delete base["2"];
            check("missing integer skips prototype",prototypeHits===0);
            check("prototype did not replace undefined",
                seen.join(",")==="1,undefined,undefined,undefined");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}
