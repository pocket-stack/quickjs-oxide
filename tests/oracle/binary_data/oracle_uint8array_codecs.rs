use crate::runtime_observation::{
    property_callable_with_read_context as property_callable, take_exception_object,
};
use crate::runtime_oracle::eval_object;
use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, ObjectRef, Runtime, RuntimeError, Value};

// These vectors freeze the complete Uint8Array base64/hex codec surface in
// QuickJS 2026-06-04. In particular, they preserve the upstream decoder's
// complete-quantum capacity rule, partial writes before syntax errors, option
// getter ordering, WTF-8 rejection, and detached/out-of-bounds revalidation.
struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

const PRELUDE: &str = r#"
function __bit(value){return value?"1":"0"}
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    if(descriptor===undefined)return "missing";
    return __bit(descriptor.writable)+__bit(descriptor.enumerable)+
        __bit(descriptor.configurable);
}
function __isConstructor(value){
    try{Reflect.construct(function(){},[],value);return true}
    catch(_error){return false}
}
function __completion(thunk){
    try{return "return:"+String(thunk())}
    catch(error){
        if(error===__token)return "throw:token";
        return "throw:"+error.name+":"+error.message;
    }
}
function __bytes(value){return Array.prototype.join.call(value,",")}
function __result(value){return value.read+","+value.written}
function __metadata(owner,key){
    var fn=owner[key];
    return key+":"+fn.name+":"+fn.length+":"+__isConstructor(fn)+":"+
        __bits(owner,key)+":"+__bits(fn,"name")+":"+__bits(fn,"length")+":"+
        Reflect.ownKeys(fn).map(String).join(",");
}
var __token={};
"#;

const CASES: &[Case] = &[
    Case {
        description: "surface placement order descriptors arity and constructability",
        source: r##"(function(){
            return [
                "staticKeys="+Reflect.ownKeys(Uint8Array).map(String).join(","),
                "protoKeys="+Reflect.ownKeys(Uint8Array.prototype).
                    map(String).join(","),
                "static="+["fromBase64","fromHex"].map(function(key){
                    return __metadata(Uint8Array,key);
                }).join(";"),
                "proto="+[
                    "toBase64","toHex","setFromBase64","setFromHex"
                ].map(function(key){
                    return __metadata(Uint8Array.prototype,key);
                }).join(";")
            ].join("|");
        })()"##,
        expected: "staticKeys=length,name,BYTES_PER_ELEMENT,fromBase64,fromHex,prototype|protoKeys=BYTES_PER_ELEMENT,toBase64,toHex,setFromBase64,setFromHex,constructor|static=fromBase64:fromBase64:1:false:101:001:001:length,name;fromHex:fromHex:1:false:101:001:001:length,name|proto=toBase64:toBase64:0:false:101:001:001:length,name;toHex:toHex:0:false:101:001:001:length,name;setFromBase64:setFromBase64:1:false:101:001:001:length,name;setFromHex:setFromHex:1:false:101:001:001:length,name",
    },
    Case {
        description: "base64 alphabets padding whitespace hex case and result prototypes",
        source: r##"(function(){
            var one=new Uint8Array([102]);
            var value=new Uint8Array([0,1,2,253,254,255]);
            var base64=Uint8Array.fromBase64(" \tAAEC\n/f7/\r ");
            var base64url=Uint8Array.fromBase64(
                "AAEC_f7_",{alphabet:"base64url"});
            var hex=Uint8Array.fromHex("000102FdFeFf");
            return [
                one.toBase64(),one.toBase64({omitPadding:true}),
                value.toBase64(),
                value.toBase64({alphabet:"base64url"}),
                value.toHex(),
                __bytes(base64),__bytes(base64url),__bytes(hex),
                __bytes(Uint8Array.fromBase64("")),
                __bytes(Uint8Array.fromHex("")),
                Object.getPrototypeOf(base64)===Uint8Array.prototype,
                Object.getPrototypeOf(hex)===Uint8Array.prototype
            ].join("|");
        })()"##,
        expected: "Zg==|Zg|AAEC/f7/|AAEC_f7_|000102fdfeff|0,1,2,253,254,255|0,1,2,253,254,255|0,1,2,253,254,255|||true|true",
    },
    Case {
        description: "base64 loose strict and stop-before-partial final chunks",
        source: r##"(function(){
            function decode(input,options){
                return __completion(function(){
                    return __bytes(Uint8Array.fromBase64(input,options));
                });
            }
            return [
                decode("Zg"),
                decode("Zg",{lastChunkHandling:"strict"}),
                decode("Zg==",{lastChunkHandling:"strict"}),
                decode("Zg",{lastChunkHandling:"stop-before-partial"}),
                decode("Zm8"),
                decode("Zm8",{lastChunkHandling:"strict"}),
                decode("Zm8=",{lastChunkHandling:"strict"}),
                decode("Zm8",{lastChunkHandling:"stop-before-partial"}),
                decode("Zh==",{lastChunkHandling:"strict"}),
                decode("Zh=="),
                decode("Zm9",{lastChunkHandling:"strict"}),
                decode("Zm9"),
                decode("Zm9vYg==garbage"),
                decode("Zm9vYg==",{
                    lastChunkHandling:"stop-before-partial"
                }),
                decode(" Z m 9 v \n Y g == ",{
                    lastChunkHandling:"strict"
                }),
                decode("Z\u000bg=="),
                decode("Z\u00a0g=="),
                decode("Z\u000cg==")
            ].join("|");
        })()"##,
        expected: "return:102|throw:SyntaxError:invalid base64 string|return:102|return:|return:102,111|throw:SyntaxError:invalid base64 string|return:102,111|return:|throw:SyntaxError:invalid base64 string|return:102|throw:SyntaxError:invalid base64 string|return:102,111|throw:SyntaxError:invalid base64 string|return:102,111,111,98|return:102,111,111,98|throw:SyntaxError:invalid base64 string|throw:SyntaxError:invalid base64 string|return:102",
    },
    Case {
        description: "option getter order mutation and validation short circuits",
        source: r##"(function(){
            var log="",value=new Uint8Array([0]),toOptions={};
            Object.defineProperty(toOptions,"alphabet",{
                get:function(){
                    log+="A";value[0]=255;return "base64";
                }
            });
            Object.defineProperty(toOptions,"omitPadding",{
                get:function(){log+="O";return true}
            });
            var encoded=value.toBase64(toOptions);

            var fromOptions={};
            Object.defineProperty(fromOptions,"alphabet",{
                get:function(){log+="B";return "base64"}
            });
            Object.defineProperty(fromOptions,"lastChunkHandling",{
                get:function(){log+="L";return "loose"}
            });
            var decoded=Uint8Array.fromBase64("Zg==",fromOptions);

            var target=new Uint8Array(1),setOptions={};
            Object.defineProperty(setOptions,"alphabet",{
                get:function(){log+="C";return "base64"}
            });
            Object.defineProperty(setOptions,"lastChunkHandling",{
                get:function(){log+="H";return "strict"}
            });
            var setResult=target.setFromBase64("Zg==",setOptions);

            var evil={
                toString:function(){log+="T";throw __token}
            };
            var touch={};
            Object.defineProperty(touch,"alphabet",{
                get:function(){log+="X";throw __token}
            });
            var inputCheck=__completion(function(){
                return Uint8Array.fromBase64(evil,touch);
            });
            var setInputCheck=__completion(function(){
                return target.setFromBase64(evil,touch);
            });
            var brandCheck=__completion(function(){
                return Uint8Array.prototype.toBase64.call([],touch);
            });
            var setBrandCheck=__completion(function(){
                return Uint8Array.prototype.setFromBase64.call(
                    [],evil,touch);
            });
            var enumCheck=__completion(function(){
                return value.toBase64({alphabet:evil});
            });
            var invalidOrder={};
            Object.defineProperty(invalidOrder,"alphabet",{
                get:function(){log+="I";return "invalid"}
            });
            Object.defineProperty(invalidOrder,"lastChunkHandling",{
                get:function(){log+="J";throw __token}
            });
            var invalidOrderCheck=__completion(function(){
                return Uint8Array.fromBase64("Zg==",invalidOrder);
            });
            return [
                log,encoded,__bytes(decoded),__result(setResult),
                __bytes(target),inputCheck,setInputCheck,brandCheck,
                setBrandCheck,enumCheck,invalidOrderCheck
            ].join("|");
        })()"##,
        expected: "AOBLCHI|/w|102|4,1|102|throw:TypeError:expected string|throw:TypeError:expected string|throw:TypeError:not a Uint8Array|throw:TypeError:not a Uint8Array|throw:TypeError:expected string for alphabet|throw:TypeError:invalid alphabet",
    },
    Case {
        description: "invalid input option brand and constructor errors",
        source: r##"(function(){
            var calls=[
                function(){return Uint8Array.fromBase64()},
                function(){return Uint8Array.fromBase64(1)},
                function(){return Uint8Array.fromBase64(Symbol("x"))},
                function(){return Uint8Array.fromBase64("AA.A")},
                function(){return Uint8Array.fromBase64("Zg===")},
                function(){
                    return Uint8Array.fromBase64(
                        "Zg",{lastChunkHandling:"strict"});
                },
                function(){return Uint8Array.fromBase64("Zg==",null)},
                function(){
                    return Uint8Array.fromBase64(
                        "Zg==",{alphabet:"other"});
                },
                function(){
                    return Uint8Array.fromBase64(
                        "Zg==",{lastChunkHandling:"other"});
                },
                function(){return Uint8Array.fromHex()},
                function(){return Uint8Array.fromHex({})},
                function(){return Uint8Array.fromHex("abc")},
                function(){return Uint8Array.fromHex("0g")},
                function(){return Uint8Array.fromHex("aa aa")},
                function(){
                    return Uint8Array.prototype.toBase64.call([]);
                },
                function(){
                    return Uint8Array.prototype.toHex.call(
                        new Uint8ClampedArray(1));
                },
                function(){
                    return Uint8Array.prototype.setFromBase64.call(
                        new Int8Array(1),"AA==");
                },
                function(){
                    return Uint8Array.prototype.setFromHex.call({},"00");
                },
                function(){return new Uint8Array.fromBase64("AA==")},
                function(){return new Uint8Array.prototype.toHex()}
            ];
            return calls.map(__completion).join("|");
        })()"##,
        expected: "throw:TypeError:expected string|throw:TypeError:expected string|throw:TypeError:expected string|throw:SyntaxError:invalid base64 string|throw:SyntaxError:invalid base64 string|throw:SyntaxError:invalid base64 string|throw:TypeError:options must be an object|throw:TypeError:invalid alphabet|throw:TypeError:invalid lastChunkHandling option|throw:TypeError:expected string|throw:TypeError:expected string|throw:SyntaxError:invalid hex string|throw:SyntaxError:invalid hex string|throw:SyntaxError:invalid hex string|throw:TypeError:not a Uint8Array|throw:TypeError:not a Uint8Array|throw:TypeError:not a Uint8Array|throw:TypeError:not a Uint8Array|throw:TypeError:fromBase64 is not a constructor|throw:TypeError:toHex is not a constructor",
    },
    Case {
        description: "static receiver is ignored while subclass instances retain the brand",
        source: r##"(function(){
            var log="";
            class Subclass extends Uint8Array {
                constructor(value){log+="C";super(value)}
            }
            var trapReceiver=new Proxy(function(){},{
                get:function(){log+="G";throw __token}
            });
            var a=Uint8Array.fromBase64.call(trapReceiver,"AQI=");
            var b=Uint8Array.fromHex.call(null,"0304");
            var c=Subclass.fromBase64("BQY=");
            var d=Subclass.fromHex("0708");
            var subclass=new Subclass([9,10]);
            var proxy=new Proxy(new Uint8Array([11]),{});
            return [
                log,
                Object.getPrototypeOf(a)===Uint8Array.prototype,
                Object.getPrototypeOf(b)===Uint8Array.prototype,
                Object.getPrototypeOf(c)===Uint8Array.prototype,
                Object.getPrototypeOf(d)===Uint8Array.prototype,
                __bytes(a),__bytes(b),__bytes(c),__bytes(d),
                subclass.toBase64(),subclass.toHex(),
                __completion(function(){
                    return Uint8Array.prototype.toHex.call(proxy);
                }),
                __completion(function(){
                    return Uint8Array.prototype.setFromHex.call(proxy,"0b");
                })
            ].join("|");
        })()"##,
        expected: "C|true|true|true|true|1,2|3,4|5,6|7,8|CQo=|090a|throw:TypeError:not a Uint8Array|throw:TypeError:not a Uint8Array",
    },
    Case {
        description: "setFromBase64 capacity subarray records and partial writes",
        source: r##"(function(){
            function run(size,input,options){
                var target=new Uint8Array(size);
                target.fill(255);
                var result=target.setFromBase64(input,options);
                return __result(result)+":"+__bytes(target)+":"+
                    Reflect.ownKeys(result).join(",")+":"+
                    (Object.getPrototypeOf(result)===Object.prototype);
            }
            var base=new Uint8Array([255,255,255,255,255,255,255]);
            var subarray=base.subarray(2,5);
            var subResult=subarray.setFromBase64("Zm9vYmFy");

            var partial=new Uint8Array(6);
            partial.fill(255);
            var partialError=__completion(function(){
                return partial.setFromBase64("MjYyZm.9v");
            });

            var strict=new Uint8Array(6);
            strict.fill(255);
            var strictError=__completion(function(){
                return strict.setFromBase64(
                    "MjYyZg",{lastChunkHandling:"strict"});
            });
            return [
                run(5,"Zm9vYmFy"),
                run(0,"Zm9v"),
                run(0,"#"),
                run(1,"Zg=="),
                run(2,"Zm9v"),
                run(3," Zm9v "),
                run(3,"Zm9v#"),
                __result(subResult)+":"+__bytes(base),
                partialError+":"+__bytes(partial),
                strictError+":"+__bytes(strict)
            ].join("|");
        })()"##,
        expected: "4,3:102,111,111,255,255:read,written:true|0,0::read,written:true|0,0::read,written:true|4,1:102:read,written:true|0,0:255,255:read,written:true|5,3:102,111,111:read,written:true|4,3:102,111,111:read,written:true|4,3:255,255,102,111,111,255,255|throw:SyntaxError:invalid base64 string:50,54,50,255,255,255|throw:SyntaxError:invalid base64 string:50,54,50,255,255,255",
    },
    Case {
        description: "setFromHex capacity subarray records and error atomicity",
        source: r##"(function(){
            function run(size,input){
                var target=new Uint8Array(size);
                target.fill(255);
                var result=target.setFromHex(input);
                return __result(result)+":"+__bytes(target)+":"+
                    Reflect.ownKeys(result).join(",")+":"+
                    (Object.getPrototypeOf(result)===Object.prototype);
            }
            function fail(input){
                var target=new Uint8Array(5);
                target.fill(255);
                return __completion(function(){
                    return target.setFromHex(input);
                })+":"+__bytes(target);
            }
            var base=new Uint8Array([255,255,255,255,255,255,255]);
            var subarray=base.subarray(2,5);
            var subResult=subarray.setFromHex("aabbccdd");
            return [
                run(2,"aabbcc"),
                run(0,"aabb"),
                run(0,"G0"),
                run(3,"AaBbCc"),
                run(2,"aabbGG"),
                __result(subResult)+":"+__bytes(base),
                fail("aabbg0"),
                fail("aabbc"),
                fail("aaa "),
                (function(){
                    var empty=new Uint8Array(0);
                    return __completion(function(){
                        return empty.setFromHex("1");
                    })+":"+__bytes(empty);
                })()
            ].join("|");
        })()"##,
        expected: "4,2:170,187:read,written:true|0,0::read,written:true|0,0::read,written:true|6,3:170,187,204:read,written:true|4,2:170,187:read,written:true|6,3:255,255,170,187,204,255,255|throw:SyntaxError:invalid hex string:170,187,255,255,255|throw:SyntaxError:invalid hex string:255,255,255,255,255|throw:SyntaxError:invalid hex string:170,255,255,255,255|throw:SyntaxError:invalid hex string:",
    },
    Case {
        description: "detached and out-of-bounds views revalidate after base64 options",
        source: r##"(function(){
            var log="";
            var a=new Uint8Array([1]);
            var aOptions={
                get alphabet(){
                    log+="A";a.buffer.transfer();return "base64";
                },
                get omitPadding(){log+="O";return false}
            };
            var aResult=__completion(function(){
                return a.toBase64(aOptions);
            });

            var b=new Uint8Array([2]);
            b.buffer.transfer();
            var bOptions={
                get alphabet(){log+="B";return "base64"},
                get omitPadding(){log+="P";return false}
            };
            var bResult=__completion(function(){
                return b.toBase64(bOptions);
            });

            var c=new Uint8Array([3]);
            var cOptions={
                get alphabet(){
                    log+="C";c.buffer.transfer();return "base64";
                },
                get lastChunkHandling(){log+="H";return "loose"}
            };
            var cResult=__completion(function(){
                return c.setFromBase64("Aw==",cOptions);
            });

            var d=new Uint8Array([4]);
            d.buffer.transfer();
            var dResult=__completion(function(){return d.toHex()});
            var eResult=__completion(function(){
                return d.setFromHex("04");
            });

            var rab=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(rab,2,2);
            rab.resize(1);
            var fixedOptions={
                get alphabet(){log+="F";return "base64"},
                get omitPadding(){log+="Q";return false}
            };
            var fResult=__completion(function(){
                return fixed.toBase64(fixedOptions);
            });
            var gResult=__completion(function(){return fixed.toHex()});
            var hResult=__completion(function(){
                return fixed.setFromBase64("AA==");
            });
            var iResult=__completion(function(){
                return fixed.setFromHex("00");
            });

            var growable=new ArrayBuffer(4,{maxByteLength:8});
            var tracking=new Uint8Array(growable,2);
            growable.resize(1);
            var jResult=__completion(function(){
                return tracking.toBase64();
            });
            var kResult=__completion(function(){return tracking.toHex()});

            var changingBuffer=new ArrayBuffer(4,{maxByteLength:8});
            var changing=new Uint8Array(changingBuffer);
            var changingOptions={
                get alphabet(){
                    log+="G";changingBuffer.resize(2);return "base64";
                },
                get omitPadding(){log+="R";return true}
            };
            var lResult=__completion(function(){
                return changing.toBase64(changingOptions);
            });
            return [
                log,aResult,bResult,cResult,dResult,eResult,
                fResult,gResult,hResult,iResult,jResult,kResult,
                lResult,changing.length
            ].join("|");
        })()"##,
        expected: "AOBPCHFQGR|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|return:AAA|2",
    },
    Case {
        description: "lone surrogates and non-BMP input reject without string coercion",
        source: r##"(function(){
            function fromBase64(input){
                return __completion(function(){
                    return __bytes(Uint8Array.fromBase64(input));
                });
            }
            function fromHex(input){
                return __completion(function(){
                    return __bytes(Uint8Array.fromHex(input));
                });
            }
            function setBase64(input,size){
                if(size===undefined)size=5;
                var target=new Uint8Array(size);
                target.fill(255);
                return __completion(function(){
                    return __result(target.setFromBase64(input));
                })+":"+__bytes(target);
            }
            function setHex(input,size){
                if(size===undefined)size=5;
                var target=new Uint8Array(size);
                target.fill(255);
                return __completion(function(){
                    return __result(target.setFromHex(input));
                })+":"+__bytes(target);
            }
            var boxed=new String("Zg==");
            boxed.toString=function(){throw __token};
            return [
                fromBase64("Zm9v\uD800"),
                fromBase64("\uDC00"),
                fromBase64("Zm9v😀"),
                fromHex("aa\uD800"),
                fromHex("aabb\uD800g"),
                setBase64("Zm9v\uD800"),
                setHex("aabb\uD800g"),
                setHex("aaé"),
                setHex("aa\uD800"),
                setHex("aa😀"),
                setBase64("AAAA\uD800",3),
                setBase64("AAAA\uD800",4),
                setBase64("#",0),
                setHex("G0",0),
                setHex("1",0),
                __completion(function(){
                    return Uint8Array.fromBase64(boxed);
                }),
                __completion(function(){
                    return new Uint8Array([1]).toBase64({
                        alphabet:"\uD800"
                    });
                })
            ].join("|");
        })()"##,
        expected: "throw:SyntaxError:invalid base64 string|throw:SyntaxError:invalid base64 string|throw:SyntaxError:invalid base64 string|throw:SyntaxError:invalid hex string|throw:SyntaxError:invalid hex string|throw:SyntaxError:invalid base64 string:102,111,111,255,255|throw:SyntaxError:invalid hex string:170,187,255,255,255|throw:SyntaxError:invalid hex string:170,255,255,255,255|throw:SyntaxError:invalid hex string:255,255,255,255,255|throw:SyntaxError:invalid hex string:170,255,255,255,255|return:4,3:0,0,0|throw:SyntaxError:invalid base64 string:0,0,0,255|return:0,0:|return:0,0:|throw:SyntaxError:invalid hex string:|throw:TypeError:expected string|throw:TypeError:invalid alphabet",
    },
];

#[test]
fn uint8array_codec_vectors_match_frozen_observations() {
    for case in CASES {
        assert_eq!(
            oxide_observation(case),
            case.expected,
            "{}",
            case.description,
        );
    }
}

#[test]
fn uint8array_codec_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP Uint8Array codec oracle self-check: \
             set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };
    for case in CASES {
        assert_eq!(
            oracle_observation(&oracle, case),
            case.expected,
            "{}",
            case.description,
        );
    }
}

#[test]
fn uint8array_codecs_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP Uint8Array codec differential: \
             set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };
    for case in CASES {
        assert_eq!(
            oxide_observation(case),
            oracle_observation(&oracle, case),
            "{}",
            case.description,
        );
    }
}

#[test]
fn uint8array_codec_results_and_errors_use_the_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let defining_constructor = eval_object(&mut defining, "Uint8Array", "defining Uint8Array");
    let defining_prototype = eval_object(
        &mut defining,
        "Uint8Array.prototype",
        "defining Uint8Array prototype",
    );
    let defining_object_prototype = eval_object(
        &mut defining,
        "Object.prototype",
        "defining Object prototype",
    );
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let defining_syntax_error = eval_object(
        &mut defining,
        "SyntaxError.prototype",
        "defining SyntaxError prototype",
    );
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );

    let from_base64 =
        property_callable(&runtime, &mut defining, &defining_constructor, "fromBase64");
    let to_base64 = property_callable(&runtime, &mut defining, &defining_prototype, "toBase64");
    let to_hex = property_callable(&runtime, &mut defining, &defining_prototype, "toHex");
    let set_from_base64 = property_callable(
        &runtime,
        &mut defining,
        &defining_prototype,
        "setFromBase64",
    );

    let input = caller.eval("'AQI='").expect("caller base64 input");
    let Value::Object(decoded) = caller
        .call(&from_base64, Value::Undefined, &[input])
        .expect("cross-realm Uint8Array.fromBase64")
    else {
        panic!("cross-realm Uint8Array.fromBase64 was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&decoded).unwrap(),
        Some(defining_prototype.clone()),
        "Uint8Array.fromBase64 result did not use the method defining realm",
    );
    assert_eq!(int_property(&runtime, &mut caller, &decoded, "0"), 1);
    assert_eq!(int_property(&runtime, &mut caller, &decoded, "1"), 2);

    let receiver = eval_object(
        &mut caller,
        "new Uint8Array([10,11])",
        "caller Uint8Array receiver",
    );
    let Value::String(hex) = caller
        .call(&to_hex, Value::Object(receiver.clone()), &[])
        .expect("cross-realm Uint8Array.prototype.toHex")
    else {
        panic!("cross-realm Uint8Array.prototype.toHex was not a string");
    };
    assert_eq!(hex.to_utf8_lossy(), "0a0b");

    let set_input = caller.eval("'DA0='").expect("caller set input");
    let Value::Object(record) = caller
        .call(
            &set_from_base64,
            Value::Object(receiver.clone()),
            &[set_input],
        )
        .expect("cross-realm Uint8Array.prototype.setFromBase64")
    else {
        panic!("cross-realm setFromBase64 record was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&record).unwrap(),
        Some(defining_object_prototype),
        "setFromBase64 record did not use the method defining realm",
    );
    assert_eq!(int_property(&runtime, &mut caller, &record, "read"), 4);
    assert_eq!(int_property(&runtime, &mut caller, &record, "written"), 2);
    assert_eq!(int_property(&runtime, &mut caller, &receiver, "0"), 12);
    assert_eq!(int_property(&runtime, &mut caller, &receiver, "1"), 13);

    let invalid = caller.eval("'!'").expect("caller invalid base64");
    assert!(matches!(
        caller.call(&from_base64, Value::Undefined, &[invalid]),
        Err(RuntimeError::Exception),
    ));
    let syntax_error = take_exception_object(&mut caller, "Uint8Array.fromBase64 SyntaxError");
    assert_eq!(
        runtime.get_prototype_of(&syntax_error).unwrap(),
        Some(defining_syntax_error),
        "Uint8Array.fromBase64 SyntaxError did not use the defining realm",
    );

    let wrong_receiver = eval_object(&mut caller, "Object()", "caller wrong receiver");
    assert!(matches!(
        caller.call(&to_hex, Value::Object(wrong_receiver), &[]),
        Err(RuntimeError::Exception),
    ));
    let type_error = take_exception_object(&mut caller, "Uint8Array.prototype.toHex TypeError");
    assert_eq!(
        runtime.get_prototype_of(&type_error).unwrap(),
        Some(defining_type_error),
        "Uint8Array.prototype.toHex TypeError did not use the defining realm",
    );

    let throwing_options = eval_object(
        &mut caller,
        "({get alphabet(){throw new TypeError('caller getter')}})",
        "caller throwing codec options",
    );
    assert!(matches!(
        caller.call(
            &to_base64,
            Value::Object(receiver),
            &[Value::Object(throwing_options)],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Uint8Array codec user getter error");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Uint8Array codec replaced a caller getter throw",
    );
}

fn observed_source(source: &str) -> String {
    format!("{PRELUDE}\n{source}")
}

fn oxide_observation(case: &Case) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    match context.eval(&observed_source(case.source)) {
        Ok(Value::String(value)) => value.to_utf8_lossy(),
        Ok(value) => panic!(
            "Oxide returned a non-string for {}: {value:?}",
            case.description,
        ),
        Err(RuntimeError::Exception) => {
            let exception = context.take_exception().unwrap();
            panic!("Oxide threw for {}: {exception:?}", case.description);
        }
        Err(error) => panic!("Oxide failed for {}: {error}", case.description),
    }
}

fn oracle_observation(oracle: &OsStr, case: &Case) -> String {
    let wrapper = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print(String(value));
} catch (error) {
  print("UNEXPECTED THROW: " + error.name + ": " + error.message);
}
"#;
    let source = observed_source(case.source);
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, &source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {}: {error}", case.description));
    assert!(
        output.status.success(),
        "QuickJS failed for {}: {}",
        case.description,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Int(value) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an Int property");
    };
    value
}
