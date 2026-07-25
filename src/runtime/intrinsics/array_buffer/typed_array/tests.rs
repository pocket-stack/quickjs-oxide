use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

#[test]
fn twelve_class_graph_and_descriptors_match_the_hidden_typed_array_family() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function errorName(operation){
                try{operation();return "none"}
                catch(error){return error.name}
            }
            function checkDataFlags(label,descriptor,writable,enumerable,configurable){
                check(label+" writable",descriptor.writable===writable);
                check(label+" enumerable",descriptor.enumerable===enumerable);
                check(label+" configurable",descriptor.configurable===configurable);
            }

            var constructors=[
                Uint8ClampedArray,Int8Array,Uint8Array,Int16Array,Uint16Array,
                Int32Array,Uint32Array,BigInt64Array,BigUint64Array,
                Float16Array,Float32Array,Float64Array
            ];
            var names=[
                "Uint8ClampedArray","Int8Array","Uint8Array","Int16Array",
                "Uint16Array","Int32Array","Uint32Array","BigInt64Array",
                "BigUint64Array","Float16Array","Float32Array","Float64Array"
            ];
            var widths=[1,1,1,2,2,4,4,8,8,2,4,8];
            var baseConstructor=Object.getPrototypeOf(Uint8Array);
            var basePrototype=Object.getPrototypeOf(Uint8Array.prototype);

            check("hidden constructor",!("TypedArray" in globalThis));
            check("base name",baseConstructor.name==="TypedArray");
            check("base length",baseConstructor.length===0);
            check("base parent",Object.getPrototypeOf(baseConstructor)===Function.prototype);
            check("base prototype parent",
                Object.getPrototypeOf(basePrototype)===Object.prototype);
            check("base call throws",
                errorName(function(){return baseConstructor()})==="TypeError");
            check("base construct throws",
                errorName(function(){return Reflect.construct(baseConstructor,[])})
                    ==="TypeError");

            var basePrototypeDescriptor=
                Object.getOwnPropertyDescriptor(baseConstructor,"prototype");
            check("base prototype value",
                basePrototypeDescriptor.value===basePrototype);
            checkDataFlags(
                "base prototype",basePrototypeDescriptor,false,false,false
            );
            var baseConstructorDescriptor=
                Object.getOwnPropertyDescriptor(basePrototype,"constructor");
            check("base constructor value",
                baseConstructorDescriptor.value===baseConstructor);
            checkDataFlags(
                "base constructor",baseConstructorDescriptor,true,false,true
            );

            var fromDescriptor=Object.getOwnPropertyDescriptor(baseConstructor,"from");
            var ofDescriptor=Object.getOwnPropertyDescriptor(baseConstructor,"of");
            check("from type",typeof fromDescriptor.value==="function");
            check("from name",fromDescriptor.value.name==="from");
            check("from length",fromDescriptor.value.length===1);
            checkDataFlags("from",fromDescriptor,true,false,true);
            check("of type",typeof ofDescriptor.value==="function");
            check("of name",ofDescriptor.value.name==="of");
            check("of length",ofDescriptor.value.length===0);
            checkDataFlags("of",ofDescriptor,true,false,true);

            var speciesDescriptor=Object.getOwnPropertyDescriptor(
                baseConstructor,Symbol.species
            );
            check("species getter",typeof speciesDescriptor.get==="function");
            check("species setter",speciesDescriptor.set===undefined);
            check("species enumerable",speciesDescriptor.enumerable===false);
            check("species configurable",speciesDescriptor.configurable===true);
            check("base species",baseConstructor[Symbol.species]===baseConstructor);

            for(var i=0;i<constructors.length;i++){
                var C=constructors[i];
                var name=names[i];
                var width=widths[i];
                var globalDescriptor=Object.getOwnPropertyDescriptor(globalThis,name);
                var constructorPrototypeDescriptor=
                    Object.getOwnPropertyDescriptor(C,"prototype");
                var prototypeConstructorDescriptor=
                    Object.getOwnPropertyDescriptor(C.prototype,"constructor");
                var constructorBpe=
                    Object.getOwnPropertyDescriptor(C,"BYTES_PER_ELEMENT");
                var prototypeBpe=
                    Object.getOwnPropertyDescriptor(C.prototype,"BYTES_PER_ELEMENT");
                var instance=new C(1);

                check(name+" global value",globalDescriptor.value===C);
                checkDataFlags(name+" global",globalDescriptor,true,false,true);
                check(name+" name",C.name===name);
                check(name+" length",C.length===3);
                check(name+" constructor parent",
                    Object.getPrototypeOf(C)===baseConstructor);
                check(name+" prototype parent",
                    Object.getPrototypeOf(C.prototype)===basePrototype);
                check(name+" prototype value",
                    constructorPrototypeDescriptor.value===C.prototype);
                checkDataFlags(
                    name+" prototype",constructorPrototypeDescriptor,false,false,false
                );
                check(name+" prototype constructor",
                    prototypeConstructorDescriptor.value===C);
                checkDataFlags(
                    name+" prototype constructor",
                    prototypeConstructorDescriptor,true,false,true
                );
                check(name+" constructor BPE",constructorBpe.value===width);
                checkDataFlags(name+" constructor BPE",constructorBpe,false,false,false);
                check(name+" prototype BPE",prototypeBpe.value===width);
                checkDataFlags(name+" prototype BPE",prototypeBpe,false,false,false);
                check(name+" inherited from",C.from===baseConstructor.from);
                check(name+" inherited of",C.of===baseConstructor.of);
                check(name+" species",C[Symbol.species]===C);
                check(name+" is view",ArrayBuffer.isView(instance)===true);
                check(name+" tag",
                    Object.prototype.toString.call(instance)==="[object "+name+"]");
            }

            var accessors=["length","buffer","byteLength","byteOffset"];
            for(var j=0;j<accessors.length;j++){
                var accessor=Object.getOwnPropertyDescriptor(
                    basePrototype,accessors[j]
                );
                check(accessors[j]+" getter",typeof accessor.get==="function");
                check(accessors[j]+" getter name",
                    accessor.get.name==="get "+accessors[j]);
                check(accessors[j]+" getter length",accessor.get.length===0);
                check(accessors[j]+" setter",accessor.set===undefined);
                check(accessors[j]+" enumerable",accessor.enumerable===false);
                check(accessors[j]+" configurable",accessor.configurable===true);
                check(accessors[j]+" brand",
                    errorName(function(){return accessor.get.call({})})==="TypeError");
            }

            var methods=[
                ["set",1],["values",0],["keys",0],["entries",0],
                ["copyWithin",2],["fill",1],["reverse",0]
            ];
            for(var k=0;k<methods.length;k++){
                var method=Object.getOwnPropertyDescriptor(
                    basePrototype,methods[k][0]
                );
                check(methods[k][0]+" type",typeof method.value==="function");
                check(methods[k][0]+" name",method.value.name===methods[k][0]);
                check(methods[k][0]+" length",method.value.length===methods[k][1]);
                checkDataFlags(methods[k][0],method,true,false,true);
                check(methods[k][0]+" not constructor",
                    errorName(function(){return Reflect.construct(method.value,[])})
                        ==="TypeError");
            }

            var tagDescriptor=Object.getOwnPropertyDescriptor(
                basePrototype,Symbol.toStringTag
            );
            check("tag getter",typeof tagDescriptor.get==="function");
            check("tag setter",tagDescriptor.set===undefined);
            check("tag enumerable",tagDescriptor.enumerable===false);
            check("tag configurable",tagDescriptor.configurable===true);
            check("unbranded tag",tagDescriptor.get.call({})===undefined);
            check("branded tag",
                tagDescriptor.get.call(new Int16Array(1))==="Int16Array");
            check("iterator alias",
                basePrototype[Symbol.iterator]===basePrototype.values);
            check("toString alias",
                basePrototype.toString===Array.prototype.toString);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn every_element_kind_converts_and_roundtrips_through_integer_indexed_access() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function errorName(operation){
                try{operation();return "none"}
                catch(error){return error.name}
            }
            function sameList(label,actual,expected){
                check(label+" length",actual.length===expected.length);
                for(var i=0;i<expected.length;i++){
                    var a=actual[i];
                    var e=expected[i];
                    check(label+" "+i,
                        a===e || (a!==a && e!==e) ||
                        (a===0 && e===0 && 1/a===1/e));
                }
            }

            sameList(
                "clamp",
                new Uint8ClampedArray([-1,0.5,1.5,2.5,254.6,300,NaN]),
                [0,0,2,2,255,255,0]
            );
            sameList("int8",new Int8Array([257,-129,-1]),[1,127,-1]);
            sameList("uint8",new Uint8Array([-1,258,NaN,Infinity]),[255,2,0,0]);
            sameList("int16",new Int16Array([65535,65537]),[-1,1]);
            sameList("uint16",new Uint16Array([-1,65537]),[65535,1]);
            sameList(
                "int32",new Int32Array([4294967295,4294967297]),[-1,1]
            );
            sameList(
                "uint32",new Uint32Array([-1,4294967297]),[4294967295,1]
            );

            var bigSigned=new BigInt64Array([0xffffffffffffffffn,-2n]);
            check("bigint64 first",bigSigned[0]===-1n);
            check("bigint64 second",bigSigned[1]===-2n);
            var bigUnsigned=new BigUint64Array([-1n,0x10000000000000001n]);
            check("biguint64 first",bigUnsigned[0]===0xffffffffffffffffn);
            check("biguint64 second",bigUnsigned[1]===1n);

            var float16=new Float16Array([1.5,-0,NaN]);
            check("float16 finite",float16[0]===1.5);
            check("float16 negative zero",1/float16[1]===-Infinity);
            check("float16 nan",float16[2]!==float16[2]);
            var float32=new Float32Array([1.5,-0,NaN]);
            check("float32 finite",float32[0]===1.5);
            check("float32 negative zero",1/float32[1]===-Infinity);
            check("float32 nan",float32[2]!==float32[2]);
            var float64=new Float64Array([1.5,-0,NaN]);
            check("float64 finite",float64[0]===1.5);
            check("float64 negative zero",1/float64[1]===-Infinity);
            check("float64 nan",float64[2]!==float64[2]);

            var copiedFloat=new Float64Array(float64);
            check("same-kind copy negative zero",1/copiedFloat[1]===-Infinity);
            check("same-kind copy nan",copiedFloat[2]!==copiedFloat[2]);

            var value=new Uint8Array(1);
            check("assignment result",(value[0]=258)===258);
            check("assignment conversion",value[0]===2);
            var descriptor=Object.getOwnPropertyDescriptor(value,"0");
            check("descriptor value",descriptor.value===2);
            check("descriptor writable",descriptor.writable===true);
            check("descriptor enumerable",descriptor.enumerable===true);
            check("descriptor configurable",descriptor.configurable===true);

            check("bigint rejects number",
                errorName(function(){
                    var array=new BigInt64Array(1);
                    array[0]=1;
                })==="TypeError");
            check("number rejects bigint",
                errorName(function(){
                    var array=new Uint8Array(1);
                    array[0]=1n;
                })==="TypeError");
            check("bigint constructor rejects number",
                errorName(function(){return new BigUint64Array([1])})==="TypeError");
            check("number constructor rejects bigint",
                errorName(function(){return new Float64Array([1n])})==="TypeError");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn constructors_cover_length_buffer_object_and_new_target_branches() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function errorName(operation){
                try{operation();return "none"}
                catch(error){return error.name}
            }

            var byLength=new Uint16Array(3);
            check("length constructor length",byLength.length===3);
            check("length constructor byteLength",byLength.byteLength===6);
            check("length constructor byteOffset",byLength.byteOffset===0);
            check("length constructor zero",
                byLength[0]===0 && byLength[1]===0 && byLength[2]===0);

            var buffer=new ArrayBuffer(10);
            var fixed=new Uint16Array(buffer,2,3);
            check("buffer identity",fixed.buffer===buffer);
            check("buffer offset",fixed.byteOffset===2);
            check("buffer element length",fixed.length===3);
            check("buffer byte length",fixed.byteLength===6);
            var remainder=new Uint16Array(buffer,2);
            check("fixed buffer remainder",remainder.length===4);
            check("unaligned offset",
                errorName(function(){return new Uint16Array(buffer,1)})
                    ==="RangeError");
            check("fixed buffer indivisible remainder",
                errorName(function(){
                    return new Uint16Array(new ArrayBuffer(5),2);
                })==="RangeError");

            var rab=new ArrayBuffer(5,{maxByteLength:12});
            var tracking=new Uint16Array(rab,2);
            var explicitUndefined=new Uint16Array(rab,2,undefined);
            var rabFixed=new Uint16Array(rab,0,2);
            check("tracking floors remainder",tracking.length===1);
            check("tracking byteLength",tracking.byteLength===2);
            check("undefined is tracking",explicitUndefined.length===1);
            check("explicit fixed length",rabFixed.length===2);
            rab.resize(8);
            check("tracking grows",tracking.length===3);
            check("undefined grows",explicitUndefined.length===3);
            check("fixed remains fixed",rabFixed.length===2);

            var source=new Uint16Array([1,257,65535]);
            var converted=new Uint8Array(source);
            check("typed source length",converted.length===3);
            check("typed source conversion",
                converted[0]===1 && converted[1]===1 && converted[2]===255);
            check("typed source owns buffer",converted.buffer!==source.buffer);

            var iterableLog="";
            var iterableIndex=0;
            var iterable={};
            iterable[Symbol.iterator]=function(){
                iterableLog+="I";
                return {
                    next:function(){
                        iterableLog+="N";
                        if(iterableIndex<2){
                            var value=++iterableIndex;
                            return {
                                done:false,
                                value:{
                                    valueOf:function(){
                                        iterableLog+="V";
                                        return value;
                                    }
                                }
                            };
                        }
                        return {done:true};
                    }
                };
            };
            var fromIterable=new Uint8Array(iterable);
            check("iterable drains before conversion",iterableLog==="INNNVV");
            check("iterable result",
                fromIterable.length===2 &&
                fromIterable[0]===1 && fromIterable[1]===2);

            var arrayLikeLog="";
            var arrayLike={length:2};
            Object.defineProperty(arrayLike,"0",{
                get:function(){
                    arrayLikeLog+="G";
                    return {valueOf:function(){arrayLikeLog+="V";return 3}};
                }
            });
            Object.defineProperty(arrayLike,"1",{
                get:function(){
                    arrayLikeLog+="G";
                    return {valueOf:function(){arrayLikeLog+="V";return 4}};
                }
            });
            var fromArrayLike=new Uint8Array(arrayLike);
            check("array-like interleaves conversion",arrayLikeLog==="GVGV");
            check("array-like result",
                fromArrayLike.length===2 &&
                fromArrayLike[0]===3 && fromArrayLike[1]===4);

            class DerivedUint8Array extends Uint8Array {}
            var derived=new DerivedUint8Array([5,6]);
            check("subclass prototype",
                Object.getPrototypeOf(derived)===DerivedUint8Array.prototype);
            check("subclass instance",derived instanceof DerivedUint8Array);
            check("subclass element kind",
                derived.length===2 && derived.byteLength===2 && derived[1]===6);

            var strange=Reflect.construct(Uint8Array,[2],Float64Array);
            strange[0]=257;
            check("newTarget public prototype",
                Object.getPrototypeOf(strange)===Float64Array.prototype);
            check("newTarget instanceof",strange instanceof Float64Array);
            check("newTarget keeps Uint8 element kind",
                strange.length===2 && strange.byteLength===2 && strange[0]===1);
            check("newTarget dynamic tag",
                Object.prototype.toString.call(strange)==="[object Uint8Array]");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn constructor_reentrancy_preserves_quickjs_branch_and_coercion_order() {
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
                catch(error){
                    return error && error.name ? error.name : "throw:"+error;
                }
            }

            var log="";
            var newTarget=new Proxy(Uint8Array,{
                get:function(target,key,receiver){
                    if(key==="prototype") log+="P";
                    return Reflect.get(target,key,receiver);
                }
            });
            check("invalid length",
                completion(function(){
                    return Reflect.construct(Uint8Array,[-1],newTarget);
                })==="RangeError");
            check("invalid length before newTarget prototype",log==="");
            log="";
            check("oversized backing length",
                completion(function(){
                    return Reflect.construct(
                        Uint8Array,[2147483648],newTarget
                    );
                })==="RangeError");
            check("backing limit after newTarget prototype",log==="P");

            log="";
            var buffer=new ArrayBuffer(4);
            var offset={valueOf:function(){log+="O";return 0}};
            var explicitLength={valueOf:function(){log+="L";return 2}};
            var bufferView=Reflect.construct(
                Uint8Array,[buffer,offset,explicitLength],newTarget
            );
            check("buffer newTarget and coercion order",log==="POL");
            check("buffer ordered result",bufferView.length===2);

            log="";
            var object={length:0};
            Object.defineProperty(object,Symbol.iterator,{
                get:function(){
                    log+="I";
                    return undefined;
                }
            });
            Reflect.construct(Uint8Array,[object],newTarget);
            check("object prototype before iterator",log==="PI");

            var returned=0;
            var abruptIterable={};
            abruptIterable[Symbol.iterator]=function(){
                return {
                    next:function(){
                        return {
                            done:false,
                            get value(){throw "constructor-value"}
                        };
                    },
                    return:function(){
                        returned++;
                        return {done:true};
                    }
                };
            };
            check("constructor iterator value throw",
                completion(function(){
                    return new Uint8Array(abruptIterable);
                })==="throw:constructor-value");
            check("constructor value throw skips iterator close",returned===0);

            var detached=new ArrayBuffer(4);
            detached.transfer();
            check("alignment before detached",
                completion(function(){
                    return new Uint16Array(detached,1);
                })==="RangeError");
            check("oversized offset after detached",
                completion(function(){
                    return new Uint8Array(detached,2147483648);
                })==="TypeError");
            log="";
            explicitLength={valueOf:function(){log+="L";return 1}};
            check("explicit length on detached",
                completion(function(){
                    return new Uint16Array(detached,0,explicitLength);
                })==="TypeError");
            check("length coerced before detach check",log==="L");
            check("oversized explicit length after detached",
                completion(function(){
                    return new Uint8Array(detached,0,2147483648);
                })==="TypeError");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var source=new Uint8Array(buffer);
            source.set([1,2,3,4]);
            log="";
            var shrinkingNewTarget=new Proxy(Uint8Array,{
                get:function(target,key,receiver){
                    if(key==="prototype"){
                        log+="P";
                        buffer.resize(2);
                    }
                    return Reflect.get(target,key,receiver);
                }
            });
            var shrunk=Reflect.construct(
                Uint8Array,[source],shrinkingNewTarget
            );
            check("typed source shrink getter",log==="P");
            check("typed source snapshots original count",shrunk.length===4);
            check("typed source shrink values",
                shrunk[0]===1 && shrunk[1]===2 &&
                shrunk[2]===0 && shrunk[3]===0);

            buffer=new ArrayBuffer(16,{maxByteLength:32});
            source=new Float32Array(buffer);
            source.set([1,2,3,4]);
            shrinkingNewTarget=new Proxy(Float32Array,{
                get:function(target,key,receiver){
                    if(key==="prototype") buffer.resize(8);
                    return Reflect.get(target,key,receiver);
                }
            });
            shrunk=Reflect.construct(
                Float32Array,[source],shrinkingNewTarget
            );
            check("same-kind float shrink converts missing tail",
                shrunk.length===4 && shrunk[0]===1 && shrunk[1]===2 &&
                Number.isNaN(shrunk[2]) && Number.isNaN(shrunk[3]));

            buffer=new ArrayBuffer(32,{maxByteLength:64});
            source=new BigInt64Array(buffer);
            source.set([1n,2n,3n,4n]);
            shrinkingNewTarget=new Proxy(BigInt64Array,{
                get:function(target,key,receiver){
                    if(key==="prototype") buffer.resize(16);
                    return Reflect.get(target,key,receiver);
                }
            });
            check("same-kind bigint shrink converts missing tail",
                completion(function(){
                    return Reflect.construct(
                        BigInt64Array,[source],shrinkingNewTarget
                    );
                })==="TypeError");

            buffer=new ArrayBuffer(2,{maxByteLength:8});
            source=new Uint8Array(buffer);
            source.set([5,6]);
            var growingNewTarget=new Proxy(Uint8Array,{
                get:function(target,key,receiver){
                    if(key==="prototype") buffer.resize(4);
                    return Reflect.get(target,key,receiver);
                }
            });
            var grown=Reflect.construct(Uint8Array,[source],growingNewTarget);
            check("typed source ignores growth",
                grown.length===2 && grown[0]===5 && grown[1]===6);

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            source=new Uint8Array(buffer,0,4);
            buffer.resize(2);
            var recoveringNewTarget=new Proxy(Uint8Array,{
                get:function(target,key,receiver){
                    if(key==="prototype") buffer.resize(4);
                    return Reflect.get(target,key,receiver);
                }
            });
            var recovered=Reflect.construct(
                Uint8Array,[source],recoveringNewTarget
            );
            check("initially oob source snapshots zero",recovered.length===0);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn static_from_and_of_construct_validate_map_and_convert_results() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function errorName(operation){
                try{operation();return "none"}
                catch(error){return error.name}
            }

            var ofResult=Uint8Array.of(1,257,-1);
            check("of result",
                ofResult.length===3 &&
                ofResult[0]===1 && ofResult[1]===1 && ofResult[2]===255);

            var log="";
            var values=[3,4];
            var index=0;
            var iterable={};
            Object.defineProperty(iterable,Symbol.iterator,{
                get:function(){
                    log+="I";
                    return function(){
                        return {
                            next:function(){
                                log+="N";
                                if(index<values.length){
                                    return {value:values[index++],done:false};
                                }
                                return {done:true};
                            }
                        };
                    };
                }
            });
            function ResultConstructor(length){
                log+="C";
                return new Uint8Array(length);
            }
            var mapped=Uint8Array.from.call(
                ResultConstructor,
                iterable,
                function(value,index){
                    log+="M";
                    return value+index;
                }
            );
            check("from drains then constructs then maps",log==="INNNCMM");
            check("from mapped result",
                mapped.length===2 && mapped[0]===3 && mapped[1]===5);

            class DerivedUint16Array extends Uint16Array {}
            var derived=DerivedUint16Array.from([1,65537]);
            check("from subclass",derived instanceof DerivedUint16Array);
            check("from subclass conversion",
                derived.length===2 && derived[0]===1 && derived[1]===1);
            var derivedOf=DerivedUint16Array.of(2,65538);
            check("of subclass",derivedOf instanceof DerivedUint16Array);
            check("of subclass conversion",
                derivedOf[0]===2 && derivedOf[1]===2);

            var iteratorTouches=0;
            var invalidMapSource={};
            Object.defineProperty(invalidMapSource,Symbol.iterator,{
                get:function(){
                    iteratorTouches++;
                    return undefined;
                }
            });
            check("map callable validation",
                errorName(function(){
                    return Uint8Array.from(invalidMapSource,1);
                })==="TypeError");
            check("map validation before iterator",iteratorTouches===0);

            var returned=0;
            var abruptIterable={};
            abruptIterable[Symbol.iterator]=function(){
                return {
                    next:function(){
                        return {
                            done:false,
                            get value(){throw new RangeError("from-value")}
                        };
                    },
                    return:function(){
                        returned++;
                        return {done:true};
                    }
                };
            };
            check("from iterator value throw",
                errorName(function(){
                    return Uint8Array.from(abruptIterable);
                })==="RangeError");
            check("from value throw skips iterator close",returned===0);

            function TooShort(){
                return new Uint8Array(0);
            }
            check("from validates result size",
                errorName(function(){
                    return Uint8Array.from.call(TooShort,[1]);
                })==="TypeError");
            check("of validates result size",
                errorName(function(){
                    return Uint8Array.of.call(TooShort,1);
                })==="TypeError");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn integer_indexed_exotic_methods_handle_canonical_keys_and_receivers() {
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
                catch(error){
                    return error && error.name ? error.name : "throw:"+error;
                }
            }

            var prototype={};
            var prototypeHits=0;
            var canonical=[
                "-0","1.5","NaN","Infinity","-1","2","4294967295"
            ];
            for(var i=0;i<canonical.length;i++){
                (function(key){
                    Object.defineProperty(prototype,key,{
                        configurable:true,
                        get:function(){
                            prototypeHits++;
                            return 99;
                        }
                    });
                })(canonical[i]);
            }
            prototype["01"]=41;
            prototype["+0"]=42;
            prototype["1.0"]=43;
            prototype["1e0"]=44;

            var array=new Uint8Array([10,20]);
            Object.setPrototypeOf(array,prototype);
            check("valid index read",array["0"]===10);
            check("valid index has","0" in array);
            for(var j=0;j<canonical.length;j++){
                check("canonical get "+canonical[j],array[canonical[j]]===undefined);
                check("canonical has "+canonical[j],!(canonical[j] in array));
            }
            check("canonical prototype terminal",prototypeHits===0);
            check("noncanonical 01",array["01"]===41 && "01" in array);
            check("noncanonical +0",array["+0"]===42 && "+0" in array);
            check("noncanonical 1.0",array["1.0"]===43 && "1.0" in array);
            check("noncanonical 1e0",array["1e0"]===44 && "1e0" in array);

            var descriptor=Object.getOwnPropertyDescriptor(array,"0");
            check("index descriptor value",descriptor.value===10);
            check("index descriptor writable",descriptor.writable===true);
            check("index descriptor enumerable",descriptor.enumerable===true);
            check("index descriptor configurable",descriptor.configurable===true);
            check("oob descriptor",
                Object.getOwnPropertyDescriptor(array,"2")===undefined);

            check("define value",
                Reflect.defineProperty(array,"0",{value:260})===true);
            check("define converted",array[0]===4);
            check("define empty descriptor",
                Reflect.defineProperty(array,"0",{})===true);
            check("define writable false rejected",
                Reflect.defineProperty(array,"0",{writable:false})===false);
            check("define enumerable false rejected",
                Reflect.defineProperty(array,"0",{enumerable:false})===false);
            check("define configurable false rejected",
                Reflect.defineProperty(array,"0",{configurable:false})===false);
            check("define accessor rejected",
                Reflect.defineProperty(array,"0",{get:function(){return 1}})
                    ===false);
            check("define oob rejected",
                Reflect.defineProperty(array,"2",{value:1})===false);
            check("delete valid rejected",
                Reflect.deleteProperty(array,"0")===false);
            check("delete oob accepted",
                Reflect.deleteProperty(array,"2")===true);

            var conversionCount=0;
            var receiver={};
            var unconverted={
                valueOf:function(){
                    conversionCount++;
                    return 7;
                }
            };
            check("valid receiver set",
                Reflect.set(array,"0",unconverted,receiver)===true);
            check("valid receiver keeps value",
                Object.getOwnPropertyDescriptor(receiver,"0").value===unconverted);
            check("valid receiver skips conversion",conversionCount===0);
            check("valid receiver leaves target",array[0]===4);

            receiver={};
            check("invalid receiver set",
                Reflect.set(array,"9",unconverted,receiver)===true);
            check("invalid receiver defines nothing",
                Object.getOwnPropertyDescriptor(receiver,"9")===undefined);
            check("invalid receiver skips conversion",conversionCount===0);

            var sameReceiverConversions=0;
            check("oob same receiver accepted",
                Reflect.set(
                    array,
                    "9",
                    {valueOf:function(){sameReceiverConversions++;return 3}},
                    array
                )===true);
            check("oob same receiver converts",sameReceiverConversions===1);
            check("oob same receiver defines nothing",
                Object.getOwnPropertyDescriptor(array,"9")===undefined);

            var invalidConversions=0;
            check("fractional same receiver accepted",
                Reflect.set(
                    array,
                    "1.1",
                    {valueOf:function(){invalidConversions++;return 3}},
                    array
                )===true);
            check("negative zero same receiver accepted",
                Reflect.set(
                    array,
                    "-0",
                    {valueOf:function(){invalidConversions++;return 3}},
                    array
                )===true);
            check("invalid canonical same receiver converts",
                invalidConversions===2);

            var bigArray=new BigInt64Array(1);
            check("oob bigint still converts",
                completion(function(){bigArray[9]=1})==="TypeError");
            check("fractional bigint still converts",
                completion(function(){bigArray["1.1"]=1})==="TypeError");

            var detachBuffer=new ArrayBuffer(1);
            var detachedByDefine=new Uint8Array(detachBuffer);
            var detachValue={
                valueOf:function(){
                    detachBuffer.transfer();
                    return 9;
                }
            };
            check("define accepts after conversion detach",
                Reflect.defineProperty(
                    detachedByDefine,"0",{value:detachValue}
                )===true);
            check("define detach has no write",detachedByDefine[0]===undefined);

            var keysArray=new Uint8Array([1,2]);
            keysArray.extra=3;
            var symbol=Symbol("marker");
            keysArray[symbol]=4;
            var keys=Reflect.ownKeys(keysArray);
            check("ownKeys length",keys.length===4);
            check("ownKeys first index",keys[0]==="0");
            check("ownKeys second index",keys[1]==="1");
            check("ownKeys ordinary string",keys[2]==="extra");
            check("ownKeys symbol",keys[3]===symbol);
            check("Object.keys indices and expando",
                Object.keys(keysArray).join(",")==="0,1,extra");

            var forInBuffer=new ArrayBuffer(1,{maxByteLength:2});
            var forInArray=new Uint8Array(forInBuffer);
            var forInPrototype=Object.create(Uint8Array.prototype);
            Object.defineProperty(forInPrototype,"1",{
                value:"prototype",
                enumerable:true,
                configurable:true
            });
            Object.setPrototypeOf(forInArray,forInPrototype);
            var forInKeys=[];
            for(var forInKey in forInArray){
                forInKeys.push(forInKey);
                if(forInKey==="0") forInBuffer.resize(2);
            }
            check("for-in refreshes grown RAB own indices",
                forInKeys.join(",")==="0" &&
                Object.hasOwn(forInArray,"1"));

            var fixed=new Uint8Array(2);
            check("fixed preventExtensions",
                Reflect.preventExtensions(fixed)===true);
            check("fixed no longer extensible",Object.isExtensible(fixed)===false);
            fixed[0]=9;
            check("fixed index remains writable",fixed[0]===9);
            var rab=new ArrayBuffer(2,{maxByteLength:4});
            var rabFixed=new Uint8Array(rab,0,2);
            var rabTracking=new Uint8Array(rab);
            check("RAB fixed preventExtensions rejected",
                Reflect.preventExtensions(rabFixed)===false);
            check("RAB fixed remains extensible",
                Object.isExtensible(rabFixed)===true);
            check("RAB tracking preventExtensions rejected",
                Reflect.preventExtensions(rabTracking)===false);
            check("RAB tracking remains extensible",
                Object.isExtensible(rabTracking)===true);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn detach_and_resizable_buffer_dynamics_revalidate_accessors_and_iterators() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function errorName(operation){
                try{operation();return "none"}
                catch(error){return error.name}
            }

            var buffer=new ArrayBuffer(6);
            var detached=new Uint16Array(buffer,2,2);
            detached[0]=7;
            var iteratorBeforeDetach=detached.values();
            buffer.transfer();
            check("detached buffer identity",detached.buffer===buffer);
            check("detached length",detached.length===0);
            check("detached byteLength",detached.byteLength===0);
            check("detached byteOffset",detached.byteOffset===0);
            check("detached index",detached[0]===undefined);
            check("detached index has",!("0" in detached));
            check("detached descriptor",
                Object.getOwnPropertyDescriptor(detached,"0")===undefined);
            check("detached remains view",ArrayBuffer.isView(detached)===true);
            check("detached iterator creation",
                errorName(function(){return detached.values()})==="TypeError");
            check("existing iterator revalidates detach",
                errorName(function(){return iteratorBeforeDetach.next()})
                    ==="TypeError");

            buffer=new ArrayBuffer(8,{maxByteLength:16});
            var fixed=new Uint16Array(buffer,2,2);
            var tracking=new Uint16Array(buffer,2);
            var explicitUndefined=new Uint16Array(buffer,2,undefined);
            fixed[0]=11;
            tracking[2]=33;
            check("initial fixed metadata",
                fixed.length===2 && fixed.byteLength===4 && fixed.byteOffset===2);
            check("initial tracking metadata",
                tracking.length===3 &&
                tracking.byteLength===6 && tracking.byteOffset===2);
            check("undefined tracks",explicitUndefined.length===3);

            var fixedIterator=fixed.values();
            buffer.resize(5);
            check("fixed oob length",fixed.length===0);
            check("fixed oob byteLength",fixed.byteLength===0);
            check("fixed oob byteOffset",fixed.byteOffset===0);
            check("fixed oob index",fixed[0]===undefined && !("0" in fixed));
            check("fixed oob iterator creation",
                errorName(function(){return fixed.values()})==="TypeError");
            check("existing fixed iterator sees oob",
                errorName(function(){return fixedIterator.next()})==="TypeError");
            check("tracking floors after shrink",
                tracking.length===1 &&
                tracking.byteLength===2 && tracking.byteOffset===2);

            buffer.resize(2);
            check("tracking at end",
                tracking.length===0 &&
                tracking.byteLength===0 && tracking.byteOffset===2);
            buffer.resize(1);
            check("tracking offset oob metadata",
                tracking.length===0 &&
                tracking.byteLength===0 && tracking.byteOffset===0);
            buffer.resize(8);
            check("fixed recovers",
                fixed.length===2 && fixed.byteLength===4 && fixed.byteOffset===2);
            check("tracking recovers",
                tracking.length===3 &&
                tracking.byteLength===6 && tracking.byteOffset===2);
            check("undefined tracking recovers",explicitUndefined.length===3);
            check("truncated bytes zero",fixed[0]===0 && tracking[2]===0);
            var recoveredStep=fixedIterator.next();
            check("iterator recovers after oob throw",
                recoveredStep.done===false && recoveredStep.value===0);

            var growingBuffer=new ArrayBuffer(2,{maxByteLength:4});
            var growing=new Uint8Array(growingBuffer);
            growing[0]=1;
            growing[1]=2;
            var growingIterator=growing.values();
            check("growth iterator first",
                growingIterator.next().value===1);
            growingBuffer.resize(4);
            growing[2]=3;
            growing[3]=4;
            check("growth iterator old second",
                growingIterator.next().value===2);
            check("growth iterator new third",
                growingIterator.next().value===3);
            check("growth iterator new fourth",
                growingIterator.next().value===4);
            check("growth iterator done",growingIterator.next().done===true);

            var shrinkingBuffer=new ArrayBuffer(3,{maxByteLength:4});
            var shrinking=new Uint8Array(shrinkingBuffer);
            var shrinkingIterator=shrinking.values();
            check("shrink iterator first",
                shrinkingIterator.next().done===false);
            shrinkingBuffer.resize(0);
            check("shrink iterator completes",
                shrinkingIterator.next().done===true);

            var pair=new Uint8Array([8,9]);
            var keyIterator=pair.keys();
            var entryIterator=pair.entries();
            check("keys first",keyIterator.next().value===0);
            var entry=entryIterator.next().value;
            check("entries first",
                entry[0]===0 && entry[1]===8);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn prototype_set_handles_array_like_order_overlap_and_reentrant_bounds() {
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
                catch(error){
                    return error && error.name ? error.name : "throw:"+error;
                }
            }

            var target=new Uint8Array(4);
            check("set return",target.set([1,2],1)===undefined);
            check("set array-like values",
                target[0]===0 && target[1]===1 &&
                target[2]===2 && target[3]===0);

            var log="";
            var source={};
            Object.defineProperty(source,"length",{
                get:function(){log+="L";return 2}
            });
            Object.defineProperty(source,"0",{
                get:function(){
                    log+="G";
                    return {valueOf:function(){log+="V";return 7}};
                }
            });
            Object.defineProperty(source,"1",{
                get:function(){
                    log+="G";
                    return {valueOf:function(){log+="V";return 8}};
                }
            });
            target=new Uint8Array(4);
            target.set(source,{valueOf:function(){log+="O";return 1}});
            check("set coercion order",log==="OLGVGV");
            check("set coercion values",target[1]===7 && target[2]===8);

            log="";
            source={};
            Object.defineProperty(source,"length",{
                get:function(){log+="L";return 5}
            });
            Object.defineProperty(source,"0",{
                get:function(){log+="G";return 1}
            });
            check("set bounds error before element get",
                completion(function(){
                    return new Uint8Array(2).set(source,0);
                })==="RangeError");
            check("set bounds log",log==="L");

            log="";
            source={};
            Object.defineProperty(source,"length",{
                get:function(){log+="L";return 1}
            });
            check("negative offset before source length",
                completion(function(){
                    return new Uint8Array(2).set(
                        source,
                        {valueOf:function(){log+="O";return -1}}
                    );
                })==="RangeError");
            check("negative offset log",log==="O");

            var buffer=new ArrayBuffer(2);
            target=new Uint8Array(buffer);
            log="";
            check("offset detach revalidated",
                completion(function(){
                    return target.set(
                        {length:0},
                        {
                            valueOf:function(){
                                log+="O";
                                buffer.transfer();
                                return 0;
                            }
                        }
                    );
                })==="TypeError");
            check("offset detach log",log==="O");

            buffer=new ArrayBuffer(2);
            target=new Uint8Array(buffer);
            buffer.transfer();
            log="";
            check("initial detach still coerces offset",
                completion(function(){
                    return target.set(
                        {length:0},
                        {valueOf:function(){log+="O";return 0}}
                    );
                })==="TypeError");
            check("initial detach offset log",log==="O");

            var sameType=new Uint8Array([1,2,3,4]);
            sameType.set(new Uint8Array(sameType.buffer,0,3),1);
            check("same type overlap memmove",
                sameType[0]===1 && sameType[1]===1 &&
                sameType[2]===2 && sameType[3]===3);

            // This deliberately records the pinned QuickJS 2026-06-04
            // deviation.  A spec clone of the source would produce [2,4].
            var overlapBuffer=new ArrayBuffer(4);
            var uint16Source=new Uint16Array(overlapBuffer);
            uint16Source[0]=2;
            uint16Source[1]=4;
            var uint8Target=new Uint8Array(overlapBuffer,2,2);
            uint8Target.set(uint16Source);
            check("pinned cross-kind overlap first",uint8Target[0]===2);
            check("pinned cross-kind overlap second",uint8Target[1]===2);

            check("number target rejects bigint source",
                completion(function(){
                    return new Uint8Array(2).set(new BigInt64Array([1n]));
                })==="TypeError");
            check("bigint target rejects number source",
                completion(function(){
                    return new BigInt64Array(2).set(new Uint8Array([1]));
                })==="TypeError");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn context_free_host_definition_converts_primitive_typed_array_values() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let key = runtime.intern_property_key("0").unwrap();

    let Value::Object(number_array) = context.eval("new Uint8Array(1)").unwrap() else {
        panic!("Uint8Array construction did not return an object");
    };
    let number_descriptor = OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(Value::Int(260)),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        runtime
            .define_own_property(&number_array, &key, &number_descriptor)
            .unwrap()
    );
    assert_eq!(
        runtime.typed_array_read_index(&number_array, 0).unwrap(),
        Some(Value::Int(4))
    );

    let Value::Object(bigint_array) = context.eval("new BigInt64Array(1)").unwrap() else {
        panic!("BigInt64Array construction did not return an object");
    };
    let bigint_descriptor = OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(Value::BigInt(crate::bigint::JsBigInt::from(-2_i64))),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        runtime
            .define_own_property(&bigint_array, &key, &bigint_descriptor)
            .unwrap()
    );
    assert_eq!(
        runtime.typed_array_read_index(&bigint_array, 0).unwrap(),
        Some(Value::BigInt(crate::bigint::JsBigInt::from(-2_i64)))
    );
}
