use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, Runtime,
    RuntimeError, Value,
};

// These observations pin QuickJS 2026-06-04's `js_typed_array_from`,
// `js_typed_array_create`, and `js_array_from_iterator` paths. The upstream
// iterator path first materializes a hidden realm-local Array; Oxide's Vec
// has the same ordinary observable ordering, while allocator/OOM topology is
// intentionally outside these safe probes.
struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

const CASES: &[Case] = &[
    Case {
        description: "surface descriptors nullish source and receiver validation",
        source: r#"(function(){
            function completion(thunk){
                try{return "return:"+String(thunk())}
                catch(error){return "throw:"+error.name+":"+error.message}
            }
            function isConstructor(value){
                try{Reflect.construct(function(){},[],value);return true}
                catch(_error){return false}
            }
            var base=Object.getPrototypeOf(Uint8Array);
            var property=Object.getOwnPropertyDescriptor(base,"from");
            var length=Object.getOwnPropertyDescriptor(base.from,"length");
            var name=Object.getOwnPropertyDescriptor(base.from,"name");
            var unbound=base.from;
            return [
                base===Object.getPrototypeOf(Float64Array),
                base.from===Uint8Array.from,
                Uint8Array.hasOwnProperty("from"),
                property.writable,property.enumerable,property.configurable,
                base.from.name,base.from.length,isConstructor(base.from),
                length.writable,length.enumerable,length.configurable,
                name.writable,name.enumerable,name.configurable,
                completion(function(){return base.from.call(Uint8Array,undefined)}),
                completion(function(){return base.from.call(Uint8Array,null)}),
                completion(function(){return base.from()}),
                completion(function(){return unbound([])}),
                completion(function(){return base.from.call(1,[])}),
                completion(function(){return base.from.call({},[])}),
                completion(function(){return base.from.call({},null,1)}),
                completion(function(){return new base.from([])})
            ].join("|");
        })()"#,
        expected: "true|true|false|true|false|true|from|1|false|false|false|true|false|false|true|throw:TypeError:cannot read property 'Symbol.iterator' of undefined|throw:TypeError:cannot read property 'Symbol.iterator' of null|throw:TypeError:cannot read property 'Symbol.iterator' of undefined|throw:TypeError:not a function|throw:TypeError:not a function|throw:TypeError:not a constructor|throw:TypeError:not a function|throw:TypeError:from is not a constructor",
    },
    Case {
        description: "all twelve concrete classes and Number BigInt conversion",
        source: r#"(function(){
            function encode(value){
                if(typeof value==="bigint")return String(value)+"n";
                if(value!==value)return "NaN";
                if(value===Infinity)return "+Infinity";
                if(value===-Infinity)return "-Infinity";
                if(value===0 && 1/value<0)return "-0";
                return String(value);
            }
            function values(array){
                var out=[];
                for(var i=0;i<array.length;i++)out.push(encode(array[i]));
                return out.join(",");
            }
            var numericSource={
                0:-1,1:257,2:2.5,3:3.5,4:NaN,5:Infinity,6:-0,
                length:7
            };
            numericSource[Symbol.iterator]=null;
            var numbers=[
                Int8Array,Uint8Array,Uint8ClampedArray,
                Int16Array,Uint16Array,Int32Array,Uint32Array,
                Float16Array,Float32Array,Float64Array
            ];
            var out=numbers.map(function(C){
                return C.name+":"+values(C.from(numericSource));
            });
            var bigintObject={valueOf:function(){return 5n}};
            var bigintSource=[
                -1n,18446744073709551617n,"2",true,bigintObject
            ];
            out.push(
                "BigInt64Array:"+values(BigInt64Array.from(bigintSource)),
                "BigUint64Array:"+values(BigUint64Array.from(bigintSource))
            );
            return out.join("|");
        })()"#,
        expected: "Int8Array:-1,1,2,3,0,0,0|Uint8Array:255,1,2,3,0,0,0|Uint8ClampedArray:0,255,2,4,0,255,0|Int16Array:-1,257,2,3,0,0,0|Uint16Array:65535,257,2,3,0,0,0|Int32Array:-1,257,2,3,0,0,0|Uint32Array:4294967295,257,2,3,0,0,0|Float16Array:-1,257,2.5,3.5,NaN,+Infinity,-0|Float32Array:-1,257,2.5,3.5,NaN,+Infinity,-0|Float64Array:-1,257,2.5,3.5,NaN,+Infinity,-0|BigInt64Array:-1n,1n,2n,1n,5n|BigUint64Array:18446744073709551615n,1n,2n,1n,5n",
    },
    Case {
        description: "iterable materialization cached next and map write order",
        source: r#"(function(){
            var base=Object.getPrototypeOf(Uint8Array);
            var log="",step=0,target,mapThis={tag:"map-this"};
            var source={};
            Object.defineProperty(source,Symbol.iterator,{
                get:function(){
                    log+="G";
                    return function(){
                        log+="I"+(this===source);
                        var iterator={};
                        Object.defineProperty(iterator,"next",{
                            get:function(){
                                log+="X";
                                return function(){
                                    log+="N"+(this===iterator);
                                    step++;
                                    var current=step,result={};
                                    Object.defineProperty(result,"done",{
                                        get:function(){
                                            log+=(current>2?"E":"D");
                                            return current>2;
                                        }
                                    });
                                    Object.defineProperty(result,"value",{
                                        get:function(){
                                            log+=(current>2?"Q":"V");
                                            return current===1 ? 10 : 20;
                                        }
                                    });
                                    return result;
                                };
                            }
                        });
                        iterator.return=function(){log+="R";return {done:true}};
                        return iterator;
                    };
                }
            });
            function Result(length){
                log+="C"+arguments.length+":"+length+":"+
                    (new.target===Result);
                target=new Uint8Array(length);
                return target;
            }
            function mapper(value,index){
                log+="M"+index+":"+(this===mapThis);
                return {
                    valueOf:function(){
                        log+="T"+value;
                        return value+index;
                    }
                };
            }
            var result=base.from.call(Result,source,mapper,mapThis);
            return [
                log,result===target,target.length,target[0],target[1]
            ].join("|");
        })()"#,
        expected: "GItrueXNtrueDVNtrueDVNtrueEC1:2:trueM0:trueT10M1:trueT20|true|2|10|21",
    },
    Case {
        description: "array-like length construction property map and conversion order",
        source: r#"(function(){
            function completion(thunk){
                try{return "return:"+String(thunk())}
                catch(error){return "throw:"+error.name+":"+error.message}
            }
            var base=Object.getPrototypeOf(Uint8Array);
            var log="",target,mapThis={};
            var source={};
            Object.defineProperty(source,Symbol.iterator,{
                get:function(){log+="G";return null}
            });
            Object.defineProperty(source,"length",{
                get:function(){
                    log+="L";
                    return {valueOf:function(){log+="T";return 2.9}};
                }
            });
            Object.defineProperty(source,"0",{
                get:function(){log+="A";return 5}
            });
            Object.defineProperty(source,"1",{
                get:function(){log+="B";return 6}
            });
            function Result(length){
                log+="C"+length;
                target=new Uint8Array(length);
                return target;
            }
            function mapper(value,index){
                log+="M"+index+":"+(this===mapThis);
                return {
                    valueOf:function(){log+="V"+value;return value+index}
                };
            }
            var result=base.from.call(Result,source,mapper,mapThis);

            var untouched="",invalid={};
            Object.defineProperty(invalid,Symbol.iterator,{
                get:function(){untouched+="I";return null}
            });
            var invalidMap=completion(function(){
                return base.from.call({},invalid,1);
            });
            var explicitUndefined=Uint8Array.from(
                {0:9,length:1,[Symbol.iterator]:null},
                undefined
            );
            return [
                log,result===target,target.length,target[0],target[1],
                invalidMap,untouched,
                explicitUndefined.length,explicitUndefined[0],
                Uint8Array.from(1).length,
                Uint8Array.from(false).length,
                Uint8Array.from(Symbol("source")).length
            ].join("|");
        })()"#,
        expected: "GLTC2AM0:trueV5BM1:trueV6|true|2|5|7|throw:TypeError:not a function||1|9|0|0|0",
    },
    Case {
        description: "custom bound Proxy constructors and actual result element type",
        source: r#"(function(){
            function completion(thunk){
                try{return "return:"+String(thunk())}
                catch(error){
                    if(error===token)return "throw:token";
                    return "throw:"+error.name+":"+error.message;
                }
            }
            var base=Object.getPrototypeOf(Uint8Array),log="",target,token={};
            function Custom(length){
                log+="C"+arguments.length+":"+length+":"+
                    (new.target===Custom)+";";
                target=new Uint16Array(length+1);
                return target;
            }
            var result=base.from.call(Custom,[1,65538]);

            function BoundTarget(length){
                log+="B"+(new.target===BoundTarget)+";";
                return new Uint8Array(length);
            }
            var Bound=BoundTarget.bind(null);
            var bound=base.from.call(Bound,[3,4]);

            var proxyLog="",proxy;
            function ProxyTarget(){}
            proxy=new Proxy(ProxyTarget,{
                construct:function(target,args,newTarget){
                    proxyLog=
                        args.length+":"+args[0]+":"+(newTarget===proxy);
                    return new Uint8Array(args[0]);
                }
            });
            var proxied=base.from.call(proxy,[5,6,7]);

            var bigintTarget;
            function BigintTarget(length){
                bigintTarget=new BigInt64Array(length);
                return bigintTarget;
            }
            var bigintMismatch=completion(function(){
                return Uint8Array.from.call(BigintTarget,[1]);
            });
            var numberTarget;
            function NumberTarget(length){
                numberTarget=new Uint8Array(length);
                return numberTarget;
            }
            var numberMismatch=BigInt64Array.from.call(NumberTarget,[257]);

            function Ordinary(){}
            function ExplicitOrdinary(){return {}}
            function Short(){return new Uint8Array(1)}
            function Detached(length){
                var buffer=new ArrayBuffer(length);
                var value=new Uint8Array(buffer);
                buffer.transfer();
                return value;
            }
            function OutOfBounds(length){
                var buffer=new ArrayBuffer(length,{maxByteLength:length});
                var value=new Uint8Array(buffer,0,length);
                buffer.resize(0);
                return value;
            }
            function Abrupt(){throw token}
            var materialized=0,observed={};
            observed[Symbol.iterator]=function(){
                return {
                    next:function(){
                        materialized++;
                        return {done:true};
                    }
                };
            };

            return [
                log,
                result===target,result.length,result[0],result[1],result[2],
                bound.length,bound[0],bound[1],
                proxyLog,proxied.length,proxied[0],proxied[1],proxied[2],
                bigintMismatch,String(bigintTarget[0]),
                numberMismatch===numberTarget,
                numberTarget.length,numberTarget[0],
                completion(function(){return base.from.call(Ordinary,[1])}),
                completion(function(){return base.from.call(ExplicitOrdinary,[1])}),
                completion(function(){return base.from.call(Short,[1,2])}),
                completion(function(){return base.from.call(Detached,[1])}),
                completion(function(){return base.from.call(OutOfBounds,[1])}),
                completion(function(){return base.from.call(Abrupt,[1])}),
                completion(function(){return base.from.call({},observed)}),
                materialized
            ].join("|");
        })()"#,
        expected: "C1:2:true;Btrue;|true|3|1|2|0|2|3|4|1:3:true|3|5|6|7|throw:TypeError:cannot convert to bigint|0|true|1|1|throw:TypeError:not a TypedArray|throw:TypeError:not a TypedArray|throw:TypeError:TypedArray length is too small|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:token|throw:TypeError:not a constructor|1",
    },
    Case {
        description: "iterator abrupt identity no-close behavior and partial map writes",
        source: r#"(function(){
            var base=Object.getPrototypeOf(Uint8Array),token={};
            function completion(thunk){
                try{return "return:"+String(thunk())}
                catch(error){
                    if(error===token)return "throw:token";
                    return "throw:"+error.name+":"+error.message;
                }
            }
            function probe(mode){
                var log="",source={},iterator={};
                iterator.return=function(){log+="R";return {done:true}};
                if(mode==="next-get"){
                    Object.defineProperty(iterator,"next",{
                        get:function(){log+="X";throw token}
                    });
                }else{
                    iterator.next=function(){
                        log+="N";
                        if(mode==="next-result")return 1;
                        var result={};
                        Object.defineProperty(result,"done",{
                            get:function(){
                                log+="D";
                                if(mode==="done")throw token;
                                return false;
                            }
                        });
                        Object.defineProperty(result,"value",{
                            get:function(){log+="V";throw token}
                        });
                        return result;
                    };
                }
                source[Symbol.iterator]=function(){log+="I";return iterator};
                var result=completion(function(){
                    return base.from.call(Uint8Array,source);
                });
                return mode+":"+result+":"+log;
            }
            var getterLog="",getterSource={};
            Object.defineProperty(getterSource,Symbol.iterator,{
                get:function(){getterLog+="G";throw token}
            });
            var getterResult=completion(function(){
                return base.from.call(Uint8Array,getterSource);
            });

            var primitiveLog="",primitiveSource={};
            primitiveSource[Symbol.iterator]=function(){
                primitiveLog+="I";
                return 1;
            };
            var primitiveResult=completion(function(){
                return base.from.call(Uint8Array,primitiveSource);
            });

            var constructorLog="",constructorSource={};
            constructorSource[Symbol.iterator]=function(){
                var done=false;
                return {
                    next:function(){
                        constructorLog+="N";
                        if(done)return {done:true};
                        done=true;
                        return {done:false,value:1};
                    },
                    return:function(){
                        constructorLog+="R";
                        return {done:true};
                    }
                };
            };
            function AbruptConstructor(){constructorLog+="C";throw token}
            var constructorResult=completion(function(){
                return base.from.call(AbruptConstructor,constructorSource);
            });

            var mapLog="",mapTarget,mapSource={};
            mapSource[Symbol.iterator]=function(){
                var index=0;
                return {
                    next:function(){
                        mapLog+="N";
                        index++;
                        return index<=3
                            ? {done:false,value:index}
                            : {done:true};
                    },
                    return:function(){mapLog+="R";return {done:true}}
                };
            };
            function MapTarget(length){
                mapLog+="C";
                mapTarget=new Uint8Array(length);
                return mapTarget;
            }
            function mapper(value,index){
                mapLog+="M"+index;
                if(index===1)throw token;
                return value+9;
            }
            var mapResult=completion(function(){
                return base.from.call(MapTarget,mapSource,mapper);
            });
            return [
                getterResult,getterLog,
                primitiveResult,primitiveLog,
                probe("next-get"),
                probe("next-result"),
                probe("done"),
                probe("value"),
                constructorResult,constructorLog,
                mapResult,mapLog,
                mapTarget[0],mapTarget[1],mapTarget[2]
            ].join("|");
        })()"#,
        expected: "throw:token|G|throw:TypeError:not an object|I|next-get:throw:token:IX|next-result:throw:TypeError:iterator must return an object:IN|done:throw:token:IND|value:throw:token:INDV|throw:token|NNC|throw:token|NNNNCM0M1|10|0|0",
    },
    Case {
        description: "RAB shrink grow detach OOB and conversion writes",
        source: r#"(function(){
            var base=Object.getPrototypeOf(Uint8Array),out=[],log="";

            var buffer=new ArrayBuffer(3,{maxByteLength:4});
            var target=new Int8Array(buffer);
            function TrackingResult(){return target}
            function trackingMap(value,index){
                log+="M"+index;
                return {
                    valueOf:function(){
                        log+="V"+index;
                        if(index===0)buffer.resize(0);
                        if(index===1)buffer.resize(4);
                        return value;
                    }
                };
            }
            var result=base.from.call(
                TrackingResult,[1,2,3],trackingMap
            );
            out.push(
                result===target,target.length,
                target[0],target[1],target[2],target[3]
            );

            buffer=new ArrayBuffer(6,{maxByteLength:8});
            target=new Uint16Array(buffer,2,2);
            function FixedResult(){return target}
            function fixedMap(value,index){
                log+="F"+index;
                return {
                    valueOf:function(){
                        log+="W"+index;
                        buffer.resize(index===0 ? 2 : 6);
                        return value;
                    }
                };
            }
            result=base.from.call(FixedResult,[11,22],fixedMap);
            out.push(result===target,target.length,target[0],target[1]);

            buffer=new ArrayBuffer(3);
            target=new Uint8Array(buffer);
            function DetachedResult(){return target}
            function detachedMap(value,index){
                log+="D"+index;
                return {
                    valueOf:function(){
                        log+="T"+index;
                        if(index===0)buffer.transfer();
                        return value;
                    }
                };
            }
            result=base.from.call(
                DetachedResult,[31,32,33],detachedMap
            );
            out.push(
                result===target,buffer.detached,target.length,
                String(target[0]),String(target[1]),String(target[2])
            );
            return log+"|"+out.join(",");
        })()"#,
        expected: "M0V0M1V1M2V2F0W0F1W1D0T0D1T1D2T2|true,4,0,2,3,0,true,2,0,22,true,true,0,undefined,undefined,undefined",
    },
    Case {
        description: "zero and safe large iterable and array-like inputs",
        source: r#"(function(){
            var base=Object.getPrototypeOf(Uint8Array),log="";
            function Empty(length){
                log+=arguments.length+":"+length+":"+(new.target===Empty);
                return new Uint8Array(length);
            }
            var emptyIterable=base.from.call(Empty,[]);
            var emptyArrayLike=base.from.call(
                Empty,{length:-1,[Symbol.iterator]:null}
            );
            var input=[],i;
            for(i=0;i<512;i++)input[i]=i;
            var large=Uint8Array.from(input,function(value,index){
                return value+(index&1);
            });
            var sum=0;
            for(i=0;i<large.length;i+=17)sum+=large[i];
            return [
                log,emptyIterable.length,emptyArrayLike.length,
                large.length,large[0],large[1],
                large[255],large[256],large[511],sum
            ].join("|");
        })()"#,
        expected: "1:0:true1:0:true|0|0|512|0|2|0|0|0|3824",
    },
];

#[test]
fn typed_array_from_vectors_match_frozen_quickjs_observations() {
    for case in CASES {
        assert_eq!(
            oxide_observation(case),
            case.expected,
            "{}",
            case.description
        );
    }
}

#[test]
fn typed_array_from_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP TypedArray.from oracle self-check: set QJS_ORACLE to pinned upstream qjs");
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
fn typed_array_from_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP TypedArray.from differential: set QJS_ORACLE to pinned upstream qjs");
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
fn typed_array_from_cross_realm_result_map_errors_and_abrupt_values() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let mut custom = runtime.new_context();

    let from = eval_callable(
        &runtime,
        &mut defining,
        "Object.getPrototypeOf(Uint8Array).from",
        "defining TypedArray.from",
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
    let custom_uint16_prototype = eval_object(
        &mut custom,
        "Uint16Array.prototype",
        "custom Uint16Array prototype",
    );
    let constructor = eval_callable(
        &runtime,
        &mut custom,
        "(function Result(length){return new Uint16Array(length)})",
        "custom result constructor",
    );
    let source = eval_object(
        &mut caller,
        "({0:40,1:41,length:2})",
        "caller array-like source",
    );
    let map_this = eval_object(&mut caller, "({mapThis:1})", "caller map this");
    define_global(
        &runtime,
        &mut caller,
        "__typedArrayFromMapThis",
        Value::Object(map_this.clone()),
    );
    let map = eval_callable(
        &runtime,
        &mut caller,
        "(function(value,index){\
            globalThis.__typedArrayFromSawThis=\
                this===__typedArrayFromMapThis;\
            return value+index;\
        })",
        "caller mapper",
    );

    let result = caller
        .call(
            &from,
            Value::Object(constructor.as_object().clone()),
            &[
                Value::Object(source),
                Value::Object(map.as_object().clone()),
                Value::Object(map_this),
            ],
        )
        .expect("cross-realm TypedArray.from");
    let Value::Object(result) = result else {
        panic!("cross-realm TypedArray.from did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(custom_uint16_prototype),
        "custom constructor result escaped its defining realm",
    );
    assert_eq!(
        eval_string_with_global(
            &runtime,
            &mut caller,
            "__typedArrayFromResult",
            Value::Object(result),
            "[__typedArrayFromResult.length,__typedArrayFromResult[0],\
             __typedArrayFromResult[1],__typedArrayFromSawThis].join('|')",
        ),
        "2|40|42|true",
    );

    let sloppy_map = eval_callable(
        &runtime,
        &mut caller,
        "(function(value){\
            globalThis.__typedArrayFromSloppyThis=this===globalThis;\
            return value;\
        })",
        "caller sloppy mapper",
    );
    let sloppy_source = eval_object(
        &mut caller,
        "({0:1,length:1})",
        "caller sloppy-mapper source",
    );
    caller
        .call(
            &from,
            Value::Object(constructor.as_object().clone()),
            &[
                Value::Object(sloppy_source),
                Value::Object(sloppy_map.as_object().clone()),
            ],
        )
        .expect("cross-realm sloppy mapper");
    assert_eq!(
        caller
            .eval("__typedArrayFromSloppyThis")
            .expect("sloppy mapper this observation"),
        Value::Bool(true),
        "sloppy mapper did not normalize undefined in its own realm",
    );

    let strict_map = eval_callable(
        &runtime,
        &mut caller,
        "(function(value){\
            'use strict';\
            globalThis.__typedArrayFromStrictThis=this===undefined;\
            return value;\
        })",
        "caller strict mapper",
    );
    let strict_source = eval_object(
        &mut caller,
        "({0:1,length:1})",
        "caller strict-mapper source",
    );
    caller
        .call(
            &from,
            Value::Object(constructor.as_object().clone()),
            &[
                Value::Object(strict_source),
                Value::Object(strict_map.as_object().clone()),
            ],
        )
        .expect("cross-realm strict mapper");
    assert_eq!(
        caller
            .eval("__typedArrayFromStrictThis")
            .expect("strict mapper this observation"),
        Value::Bool(true),
        "strict mapper did not preserve undefined",
    );

    assert!(matches!(
        caller.call(
            &from,
            Value::Object(constructor.as_object().clone()),
            &[Value::Null],
        ),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "cross-realm null source TypeError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error.clone()),
        "null source TypeError did not use the builtin defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error.clone()),
        "null source TypeError leaked into the caller realm",
    );

    let conversion_source = eval_object(
        &mut caller,
        "({0:Symbol('typed-array-from'),length:1})",
        "caller conversion-error source",
    );
    assert!(matches!(
        caller.call(
            &from,
            Value::Object(constructor.as_object().clone()),
            &[Value::Object(conversion_source)],
        ),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "cross-realm conversion TypeError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error),
        "element conversion TypeError did not use the builtin defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error),
        "element conversion TypeError leaked into the caller realm",
    );

    let token = eval_object(&mut caller, "({token:1})", "caller abrupt token");
    define_global(
        &runtime,
        &mut caller,
        "__typedArrayFromToken",
        Value::Object(token.clone()),
    );
    let abrupt_source = eval_object(
        &mut caller,
        "Object.defineProperty({},Symbol.iterator,{\
            get:function(){throw __typedArrayFromToken}\
        })",
        "caller abrupt iterator source",
    );
    assert!(matches!(
        caller.call(
            &from,
            Value::Object(constructor.as_object().clone()),
            &[Value::Object(abrupt_source)],
        ),
        Err(RuntimeError::Exception),
    ));
    let thrown = caller
        .take_exception()
        .expect("take cross-realm abrupt value")
        .expect("cross-realm abrupt value was missing");
    assert_eq!(thrown, Value::Object(token));
}

fn oxide_observation(case: &Case) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    match context.eval(case.source) {
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
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, case.source])
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

fn eval_string_with_global(
    runtime: &Runtime,
    context: &mut Context,
    name: &str,
    value: Value,
    source: &str,
) -> String {
    define_global(runtime, context, name, value);
    let Value::String(value) = context.eval(source).expect("cross-realm observation") else {
        panic!("cross-realm observation did not return a string");
    };
    value.to_utf8_lossy()
}

fn define_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let key = runtime.intern_property_key(name).unwrap();
    assert!(
        context
            .define_own_property(
                &context.global_object().unwrap(),
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap(),
        "could not define {name}",
    );
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
