use super::quickjs_typed_array_oracle::observe_string_value;
use crate::runtime_oracle::eval_callable;
use crate::runtime_oracle::eval_object;
use quickjs_oxide::{
    Context, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, Runtime, RuntimeError, Value,
};

// These vectors pin QuickJS 2026-06-04's `js_typed_array_of` and
// `js_typed_array_create` paths. They intentionally cover construction,
// validation, conversion, and integer-indexed writes as one observable
// operation.
struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

const CASES: &[Case] = &[
    Case {
        description: "surface descriptors and constructor receiver validation",
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
            var property=Object.getOwnPropertyDescriptor(base,"of");
            var length=Object.getOwnPropertyDescriptor(base.of,"length");
            var name=Object.getOwnPropertyDescriptor(base.of,"name");
            var unbound=base.of;
            var method={m(){}}.m;
            return [
                base===Object.getPrototypeOf(Float64Array),
                base.of===Uint8Array.of,
                Uint8Array.hasOwnProperty("of"),
                property.writable,property.enumerable,property.configurable,
                base.of.name,base.of.length,isConstructor(base.of),
                length.writable,length.enumerable,length.configurable,
                name.writable,name.enumerable,name.configurable,
                completion(function(){return unbound()}),
                completion(function(){return base.of.call(null)}),
                completion(function(){return base.of.call(false)}),
                completion(function(){return base.of.call(1)}),
                completion(function(){return base.of.call("x")}),
                completion(function(){return base.of.call(1n)}),
                completion(function(){return base.of.call(Symbol("receiver"))}),
                completion(function(){return base.of()}),
                completion(function(){return base.of.call({})}),
                completion(function(){return base.of.call(method)}),
                completion(function(){return new base.of()})
            ].join("|");
        })()"#,
        expected: "true|true|false|true|false|true|of|0|false|false|false|true|false|false|true|throw:TypeError:not a function|throw:TypeError:not a function|throw:TypeError:not a function|throw:TypeError:not a function|throw:TypeError:not a function|throw:TypeError:not a function|throw:TypeError:not a function|throw:TypeError:cannot be called|throw:TypeError:not a constructor|throw:TypeError:m is not a constructor|throw:TypeError:of is not a constructor",
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
            var numbers=[
                Int8Array,Uint8Array,Uint8ClampedArray,
                Int16Array,Uint16Array,Int32Array,Uint32Array,
                Float16Array,Float32Array,Float64Array
            ];
            var out=numbers.map(function(C){
                return C.name+":"+
                    values(C.of(-1,257,2.5,3.5,NaN,Infinity,-0));
            });
            var bigintObject={valueOf:function(){return 5n}};
            out.push(
                "BigInt64Array:"+
                    values(BigInt64Array.of(
                        -1n,18446744073709551617n,"2",true,bigintObject)),
                "BigUint64Array:"+
                    values(BigUint64Array.of(
                        -1n,18446744073709551617n,"2",true,bigintObject))
            );
            return out.join("|");
        })()"#,
        expected: "Int8Array:-1,1,2,3,0,0,0|Uint8Array:255,1,2,3,0,0,0|Uint8ClampedArray:0,255,2,4,0,255,0|Int16Array:-1,257,2,3,0,0,0|Uint16Array:65535,257,2,3,0,0,0|Int32Array:-1,257,2,3,0,0,0|Uint32Array:4294967295,257,2,3,0,0,0|Float16Array:-1,257,2.5,3.5,NaN,+Infinity,-0|Float32Array:-1,257,2.5,3.5,NaN,+Infinity,-0|Float64Array:-1,257,2.5,3.5,NaN,+Infinity,-0|BigInt64Array:-1n,1n,2n,1n,5n|BigUint64Array:18446744073709551615n,1n,2n,1n,5n",
    },
    Case {
        description: "static from shares only the direct constructor diagnostic seam",
        source: r#"(function(){
            function completion(thunk){
                try{return "return:"+String(thunk())}
                catch(error){return "throw:"+error.name+":"+error.message}
            }
            var base=Object.getPrototypeOf(Uint8Array),log="";
            var arrayLike={};
            Object.defineProperty(arrayLike,Symbol.iterator,{
                get:function(){log+="I";return undefined}
            });
            Object.defineProperty(arrayLike,"length",{
                get:function(){log+="L";return 0}
            });
            var arrayLikeResult=completion(function(){
                return base.from.call(1,arrayLike);
            });
            var arrayLikeLog=log;

            log="";
            var iterable={};
            Object.defineProperty(iterable,Symbol.iterator,{
                get:function(){
                    log+="G";
                    return function(){
                        log+="I";
                        return {
                            next:function(){
                                log+="N";
                                return {done:true};
                            }
                        };
                    };
                }
            });
            var iterableResult=completion(function(){
                return base.from.call(false,iterable);
            });
            var iterableLog=log;

            log="";
            var mapResult=completion(function(){
                return base.from.call({},arrayLike,1);
            });
            var mapLog=log;

            var source=new Uint8Array([1]);
            source.constructor={};
            source.constructor[Symbol.species]=1;
            var speciesResult=completion(function(){
                return source.slice(0);
            });
            return [
                arrayLikeResult,arrayLikeLog,
                iterableResult,iterableLog,
                mapResult,mapLog,speciesResult
            ].join("|");
        })()"#,
        expected: "throw:TypeError:not a function|IL|throw:TypeError:not a function|GIN|throw:TypeError:not a function||throw:TypeError:not a constructor",
    },
    Case {
        description: "custom constructors newTarget and result validation",
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
            var result=base.of.call(Custom,1,65538);

            function Wrapper(length){
                log+="W"+(new.target===Wrapper)+";";
                return Reflect.construct(Uint8Array,[length],new.target);
            }
            Object.setPrototypeOf(Wrapper,base);
            Wrapper.prototype=Object.create(Uint8Array.prototype);
            Object.defineProperty(
                Wrapper.prototype,"constructor",
                {value:Wrapper,writable:true,configurable:true}
            );
            var wrapped=Wrapper.of(9,10);

            function BoundTarget(length){
                log+="B"+(new.target===BoundTarget)+";";
                return new Uint8Array(length);
            }
            var Bound=BoundTarget.bind(null);
            var bound=base.of.call(Bound,3,4);

            var proxyLog="",proxy;
            function ProxyTarget(){}
            proxy=new Proxy(ProxyTarget,{
                construct:function(target,args,newTarget){
                    proxyLog=
                        args.length+":"+args[0]+":"+(newTarget===proxy);
                    return new Uint8Array(args[0]);
                }
            });
            var proxied=base.of.call(proxy,5,6,7);

            var bigintTarget;
            function BigintTarget(length){
                bigintTarget=new BigInt64Array(length);
                return bigintTarget;
            }
            var bigintMismatch=completion(function(){
                return Uint8Array.of.call(BigintTarget,1);
            });
            var numberTarget;
            function NumberTarget(length){
                numberTarget=new Uint8Array(length);
                return numberTarget;
            }
            var numberMismatch=BigInt64Array.of.call(NumberTarget,257);

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
            var proxyTarget=new Uint8Array(2);
            function ProxyResult(){return new Proxy(proxyTarget,{})}
            function Abrupt(){throw token}

            return [
                log,
                result===target,result.length,result[0],result[1],result[2],
                Object.getPrototypeOf(wrapped)===Wrapper.prototype,
                wrapped.length,wrapped[0],wrapped[1],
                bound.length,bound[0],bound[1],
                proxyLog,proxied.length,proxied[0],proxied[1],proxied[2],
                bigintMismatch,String(bigintTarget[0]),
                numberMismatch===numberTarget,
                numberTarget.length,numberTarget[0],
                completion(function(){return base.of.call(Ordinary,1)}),
                completion(function(){return base.of.call(ExplicitOrdinary,1)}),
                completion(function(){return base.of.call(Short,1,2)}),
                completion(function(){return base.of.call(Detached,1)}),
                completion(function(){return base.of.call(OutOfBounds,1)}),
                completion(function(){return base.of.call(ProxyResult,1,2)}),
                completion(function(){return base.of.call(Abrupt,1)})
            ].join("|");
        })()"#,
        expected: "C1:2:true;Wtrue;Btrue;|true|3|1|2|0|true|2|9|10|2|3|4|1:3:true|3|5|6|7|throw:TypeError:cannot convert to bigint|0|true|1|1|throw:TypeError:not a TypedArray|throw:TypeError:not a TypedArray|throw:TypeError:TypedArray length is too small|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:not a TypedArray|throw:token",
    },
    Case {
        description: "constructor conversion and abrupt completion ordering",
        source: r#"(function(){
            var base=Object.getPrototypeOf(Uint8Array);
            var log="",target,token={},ctorToken={};
            function Custom(length){
                log+="C"+arguments.length+":"+length+";";
                target=new Uint8Array(length);
                return target;
            }
            var first={
                valueOf:function(){log+="A"+target[0]+";";return 7}
            };
            var second={
                valueOf:function(){log+="B"+target[0]+";";throw token}
            };
            var third={
                valueOf:function(){log+="D";return 9}
            };
            var same=false;
            try{base.of.call(Custom,first,second,third)}
            catch(error){same=error===token}
            var partial=target[0]+","+target[1]+","+target[2];

            var untouched={
                valueOf:function(){log+="I";return 1}
            };
            function AbruptConstructor(){
                log+="X;";
                throw ctorToken;
            }
            var ctorSame=false;
            try{base.of.call(AbruptConstructor,untouched)}
            catch(error){ctorSame=error===ctorToken}

            var bigintTarget;
            function BigintResult(length){
                bigintTarget=new BigInt64Array(length);
                return bigintTarget;
            }
            var later={
                valueOf:function(){log+="L";return 3n}
            };
            var bigintError;
            try{base.of.call(BigintResult,1n,2,later);bigintError="missing"}
            catch(error){bigintError=error.name}
            return [
                log,same,partial,ctorSame,bigintError,
                String(bigintTarget[0]),String(bigintTarget[1]),
                String(bigintTarget[2])
            ].join("|");
        })()"#,
        expected: "C1:3;A0;B7;X;|true|7,0,0|true|TypeError|1|0|0",
    },
    Case {
        description: "RAB shrink grow detach and OOB writes",
        source: r#"(function(){
            var base=Object.getPrototypeOf(Uint8Array),out=[],log="";

            var buffer=new ArrayBuffer(3,{maxByteLength:4});
            var target=new Int8Array(buffer);
            function TrackingResult(){return target}
            var one={
                valueOf:function(){log+="a";buffer.resize(0);return 1}
            };
            var two={
                valueOf:function(){log+="b";buffer.resize(4);return 2}
            };
            var result=base.of.call(TrackingResult,one,two,3);
            out.push(
                result===target,target.length,
                target[0],target[1],target[2],target[3]
            );

            buffer=new ArrayBuffer(6,{maxByteLength:8});
            target=new Uint16Array(buffer,2,2);
            function FixedResult(){return target}
            one={
                valueOf:function(){log+="c";buffer.resize(2);return 11}
            };
            two={
                valueOf:function(){log+="d";buffer.resize(6);return 22}
            };
            result=base.of.call(FixedResult,one,two);
            out.push(result===target,target.length,target[0],target[1]);

            buffer=new ArrayBuffer(3);
            target=new Uint8Array(buffer);
            function DetachedResult(){return target}
            one={
                valueOf:function(){
                    log+="e";
                    buffer.transfer();
                    return 31;
                }
            };
            two={valueOf:function(){log+="f";return 32}};
            var three={valueOf:function(){log+="g";return 33}};
            result=base.of.call(DetachedResult,one,two,three);
            out.push(
                result===target,buffer.detached,target.length,
                String(target[0]),String(target[1]),String(target[2])
            );
            return log+"|"+out.join(",");
        })()"#,
        expected: "abcdefg|true,4,0,2,3,0,true,2,0,22,true,true,0,undefined,undefined,undefined",
    },
    Case {
        description: "zero and safe large argument counts",
        source: r#"(function(){
            var base=Object.getPrototypeOf(Uint8Array),log="";
            function Empty(length){
                log+=arguments.length+":"+length+":"+(new.target===Empty);
                return new Uint8Array(length);
            }
            var empty=base.of.call(Empty);
            var input=[],i;
            for(i=0;i<512;i++)input[i]=i;
            var large=Uint8Array.of.apply(Uint8Array,input);
            var sum=0;
            for(i=0;i<large.length;i+=17)sum+=large[i];
            return [
                log,empty.length,large.length,
                large[0],large[255],large[256],large[511],sum
            ].join("|");
        })()"#,
        expected: "1:0:true|0|512|0|255|0|255|4065",
    },
];

#[test]
fn typed_array_of_vectors_match_frozen_quickjs_observations() {
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
fn typed_array_of_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP TypedArray.of oracle self-check: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    for case in CASES {
        assert_eq!(
            observe_string_value(&oracle, case.source, case.description),
            case.expected,
            "{}",
            case.description,
        );
    }
}

#[test]
fn typed_array_of_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP TypedArray.of differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    for case in CASES {
        assert_eq!(
            oxide_observation(case),
            observe_string_value(&oracle, case.source, case.description),
            "{}",
            case.description,
        );
    }
}

#[test]
fn typed_array_of_cross_realm_results_errors_and_abrupt_values() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let mut custom = runtime.new_context();

    let of = eval_callable(
        &runtime,
        &mut defining,
        "Object.getPrototypeOf(Uint8Array).of",
        "defining TypedArray.of",
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

    let result = caller
        .call(
            &of,
            Value::Object(constructor.as_object().clone()),
            &[Value::number(41.0), Value::number(65578.0)],
        )
        .expect("cross-realm TypedArray.of");
    let Value::Object(result) = result else {
        panic!("cross-realm TypedArray.of did not return an object");
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
            "__typedArrayOfResult",
            Value::Object(result),
            "[__typedArrayOfResult.length,__typedArrayOfResult[0],\
             __typedArrayOfResult[1]].join('|')",
        ),
        "2|41|42",
    );

    let ordinary = eval_object(&mut caller, "({})", "caller ordinary receiver");
    assert!(matches!(
        caller.call(&of, Value::Object(ordinary), &[]),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "cross-realm receiver TypeError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error.clone()),
        "receiver TypeError did not use the builtin defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error.clone()),
        "receiver TypeError leaked into the caller realm",
    );

    assert!(matches!(
        caller.call(&of, Value::number(1.0), &[]),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "cross-realm primitive receiver TypeError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error.clone()),
        "primitive receiver TypeError did not use the builtin defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error),
        "primitive receiver TypeError leaked into the caller realm",
    );

    let token = eval_object(&mut caller, "({token:1})", "caller abrupt token");
    let item = eval_object(
        &mut caller,
        "({valueOf:function(){throw __typedArrayOfToken}})",
        "caller abrupt item",
    );
    define_global(
        &runtime,
        &mut caller,
        "__typedArrayOfToken",
        Value::Object(token.clone()),
    );
    assert!(matches!(
        caller.call(
            &of,
            Value::Object(constructor.as_object().clone()),
            &[Value::Object(item)],
        ),
        Err(RuntimeError::Exception),
    ));
    let thrown = caller
        .take_exception()
        .expect("take cross-realm abrupt value")
        .expect("cross-realm abrupt value was missing");
    assert_eq!(thrown, Value::Object(token));

    let symbol = caller
        .eval("Symbol('typed-array-of')")
        .expect("caller symbol value");
    assert!(matches!(
        caller.call(
            &of,
            Value::Object(constructor.as_object().clone()),
            &[symbol],
        ),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "cross-realm conversion TypeError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error),
        "element conversion TypeError did not use the builtin defining realm",
    );
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
