use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray slice test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("TypedArray slice rejected {description}: {error}"))
    else {
        panic!("TypedArray slice {description} was not an Object");
    };
    object
}

#[test]
fn slice_and_subarray_publish_the_quickjs_surface_and_basic_results() {
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
                catch(error){return error.name+":"+error.message}
            }
            var base=Object.getPrototypeOf(Uint8Array.prototype);
            for(var name of ["slice","subarray"]){
                var descriptor=Object.getOwnPropertyDescriptor(base,name);
                check(name+" descriptor",
                    descriptor.writable===true &&
                    descriptor.enumerable===false &&
                    descriptor.configurable===true);
                check(name+" length",base[name].length===2);
                check(name+" name",base[name].name===name);
                check(name+" no prototype",
                    !Object.prototype.hasOwnProperty.call(base[name],"prototype"));
                check(name+" not constructor",
                    completion(function(){new base[name]})===
                    "TypeError:"+name+" is not a constructor");
                check(name+" brand",
                    completion(function(){base[name].call({})})===
                    "TypeError:not a TypedArray");
            }

            var source=new Uint16Array([10,20,30,40]);
            var copied=source.slice(1,-1);
            check("slice values",
                copied.length===2 && copied[0]===20 && copied[1]===30);
            check("slice distinct buffer",copied.buffer!==source.buffer);
            copied[0]=99;
            check("slice independent",source[1]===20);

            var view=source.subarray(1,-1);
            check("subarray values",
                view.length===2 && view[0]===20 && view[1]===30);
            check("subarray shared buffer",view.buffer===source.buffer);
            check("subarray offset",view.byteOffset===source.byteOffset+2);
            view[0]=77;
            check("subarray aliases",source[1]===77);
            var whole=source.slice();
            check("omitted bounds",
                whole.length===4 && whole[0]===10 && whole[1]===77 &&
                whole[2]===30 && whole[3]===40);
            check("empty range",source.slice(3,1).length===0);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn slice_matches_quickjs_species_reentrancy_and_raw_copy_contracts() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function errorText(operation){
                try{operation();return "return"}
                catch(error){return error.name+":"+error.message}
            }

            var buffer=new ArrayBuffer(2);
            var detached=new Uint8Array(buffer);
            buffer.transfer();
            var startHits=0;
            check("initial detached",
                errorText(function(){
                    detached.slice({
                        valueOf:function(){startHits++;return 0}
                    });
                })==="TypeError:ArrayBuffer is detached or resized");
            check("initial detached before start",startHits===0);

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,2,2);
            buffer.resize(1);
            check("initial oob",
                errorText(function(){fixed.slice()})===
                "TypeError:ArrayBuffer is detached or resized");

            var order="";
            var ordered=new Uint8Array([1,2,3]);
            Object.defineProperty(ordered,"constructor",{
                configurable:true,
                get:function(){
                    order+="C";
                    return {
                        get [Symbol.species](){
                            order+="P";
                            return function(length){
                                order+="S";
                                return new Uint8Array(length);
                            };
                        }
                    };
                }
            });
            ordered.slice(
                {valueOf:function(){order+="B";return 0}},
                {valueOf:function(){order+="E";return 2}}
            );
            check("coercion species order",order==="BECPS");

            buffer=new ArrayBuffer(1);
            var emptyDetach=new Uint8Array(buffer);
            emptyDetach.constructor={
                [Symbol.species]:function(length){
                    buffer.transfer();
                    return new Uint8Array(length);
                }
            };
            check("empty skips source post validation",
                emptyDetach.slice(0,0).length===0);

            buffer=new ArrayBuffer(1);
            var nonemptyDetach=new Uint8Array(buffer);
            nonemptyDetach[0]=7;
            nonemptyDetach.constructor={
                [Symbol.species]:function(length){
                    buffer.transfer();
                    return new Uint8Array(length);
                }
            };
            check("nonempty validates source after species",
                errorText(function(){nonemptyDetach.slice()})===
                "TypeError:ArrayBuffer is detached or resized");

            buffer=new ArrayBuffer(1);
            var shortFirst=new Uint8Array(buffer);
            shortFirst.constructor={
                [Symbol.species]:function(){
                    buffer.transfer();
                    return new Uint8Array(0);
                }
            };
            check("short species precedes source detach",
                errorText(function(){shortFirst.slice()})===
                "TypeError:TypedArray length is too small");

            buffer=new ArrayBuffer(6,{maxByteLength:8});
            var tracking=new Uint8Array(buffer);
            for(var i=0;i<6;i++) tracking[i]=i+1;
            tracking.constructor={
                [Symbol.species]:function(length){
                    buffer.resize(3);
                    return new Uint8Array(length);
                }
            };
            var shrunk=tracking.slice(1,6);
            check("tracking shrink keeps result length",shrunk.length===5);
            check("tracking shrink truncates copy",
                shrunk[0]===2 && shrunk[1]===3 &&
                shrunk[2]===0 && shrunk[3]===0 && shrunk[4]===0);

            buffer=new ArrayBuffer(6);
            var overlap=new Uint8Array(buffer);
            overlap[0]=10;overlap[1]=20;overlap[2]=30;
            overlap[3]=40;overlap[4]=50;overlap[5]=60;
            overlap.constructor={
                [Symbol.species]:function(){
                    return new Uint8Array(buffer,2,4);
                }
            };
            var propagated=overlap.slice(1,4);
            check("forward overlap result",
                propagated[0]===20 && propagated[1]===20 &&
                propagated[2]===20 && propagated[3]===60);
            check("forward overlap backing",
                overlap[0]===10 && overlap[1]===20 &&
                overlap[2]===20 && overlap[3]===20 &&
                overlap[4]===20 && overlap[5]===60);

            buffer=new ArrayBuffer(8);
            var bits=new DataView(buffer);
            bits.setUint32(0,0x7fc12345,true);
            bits.setUint32(4,0x80000000,true);
            var floats=new Float32Array(buffer);
            var floatCopy=floats.slice();
            var copiedBits=new DataView(floatCopy.buffer);
            check("slice preserves NaN payload",
                copiedBits.getUint32(0,true)===0x7fc12345);
            check("slice preserves negative zero",
                copiedBits.getUint32(4,true)===0x80000000 &&
                1/floatCopy[1]===-Infinity);

            var numbers=new Uint8Array([3,4]);
            numbers.constructor={[Symbol.species]:Float64Array};
            var widened=numbers.slice();
            check("different kind converts",
                widened instanceof Float64Array &&
                widened.length===2 && widened[0]===3 && widened[1]===4);

            var mismatch=new Uint8Array([1]);
            mismatch.constructor={[Symbol.species]:BigInt64Array};
            check("nonempty content mismatch",
                errorText(function(){mismatch.slice()})===
                "TypeError:cannot convert to bigint");
            var emptyMismatch=new Uint8Array(0);
            emptyMismatch.constructor={[Symbol.species]:BigInt64Array};
            check("empty content mismatch succeeds",
                emptyMismatch.slice() instanceof BigInt64Array);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn subarray_matches_quickjs_view_species_and_detached_contracts() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function errorText(operation){
                try{operation();return "return"}
                catch(error){return error.name+":"+error.message}
            }

            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            var tracking=new Uint8Array(buffer);
            tracking.constructor=undefined;
            var automatic=tracking.subarray(1);
            var explicitUndefined=tracking.subarray(1,undefined);
            var fixedEnd=tracking.subarray(1,3);
            buffer.resize(7);
            check("omitted end tracks",automatic.length===6);
            check("explicit undefined tracks",explicitUndefined.length===6);
            check("explicit end fixed",fixedEnd.length===2);
            check("views share buffer",
                automatic.buffer===buffer && fixedEnd.buffer===buffer);

            var captured=[];
            var arbitrary=new Float64Array(0);
            function Species(){
                captured.push(Array.prototype.slice.call(arguments));
                return arbitrary;
            }
            tracking.constructor={[Symbol.species]:Species};
            check("tracking species accepts arbitrary result",
                tracking.subarray(2)===arbitrary);
            check("tracking species argc",captured[0].length===2);
            check("tracking species buffer",captured[0][0]===buffer);
            check("tracking species offset",captured[0][1]===2);
            check("explicit species accepts zero length",
                tracking.subarray(2,5)===arbitrary);
            check("explicit species argc",captured[1].length===3);
            check("explicit species count",captured[1][2]===3);

            var fixedSource=new Uint8Array(buffer,0,4);
            fixedSource.constructor={[Symbol.species]:Species};
            fixedSource.subarray(1);
            check("fixed source passes count",captured[2].length===3);
            check("fixed source count",captured[2][2]===3);

            buffer=new ArrayBuffer(2);
            var detached=new Uint8Array(buffer);
            var order="";
            var begin={valueOf:function(){order+="B";return 0}};
            var end={valueOf:function(){order+="E";return 0}};
            Object.defineProperty(detached,"constructor",{
                configurable:true,
                get:function(){order+="C";return undefined}
            });
            buffer.transfer();
            check("detached default constructor",
                errorText(function(){detached.subarray(begin,end)})===
                "TypeError:ArrayBuffer is detached");
            check("detached still coerces and resolves species",order==="BEC");

            buffer=new ArrayBuffer(2);
            detached=new Uint8Array(buffer);
            var replacement=new Int16Array([9]);
            detached.constructor={
                [Symbol.species]:function(){
                    return replacement;
                }
            };
            buffer.transfer();
            check("custom species may ignore detached source",
                detached.subarray()===replacement);

            buffer=new ArrayBuffer(8);
            var bytes=new Uint8Array(buffer);
            bytes[0]=1;bytes[1]=2;bytes[2]=3;bytes[3]=4;
            bytes.constructor={[Symbol.species]:Uint16Array};
            var reinterpreted=bytes.subarray(0,4);
            check("cross content reinterprets",
                reinterpreted instanceof Uint16Array &&
                reinterpreted.buffer===buffer &&
                reinterpreted.length===4);

            var odd=new Uint8Array(buffer,1,4);
            odd.constructor={[Symbol.species]:Uint16Array};
            check("cross kind invalid offset text",
                errorText(function(){odd.subarray(0,1)})===
                "RangeError:invalid offset");

            buffer=new ArrayBuffer(3);
            var tooLong=new Uint8Array(buffer);
            tooLong.constructor={[Symbol.species]:Uint16Array};
            check("cross kind invalid length text",
                errorText(function(){tooLong.subarray(0,3)})===
                "RangeError:invalid length");
            check("constructor invalid offset text",
                errorText(function(){
                    new Uint16Array(new ArrayBuffer(4),1,1);
                })==="RangeError:invalid offset");
            var lengthHits=0;
            check("constructor alignment precedes length coercion",
                errorText(function(){
                    new Uint16Array(new ArrayBuffer(4),1,{
                        valueOf:function(){lengthHits++;return 1}
                    });
                })==="RangeError:invalid offset");
            check("invalid offset skips length coercion",lengthHits===0);
            check("constructor invalid length text",
                errorText(function(){
                    new Uint16Array(new ArrayBuffer(4),0,3);
                })==="RangeError:invalid length");
            check("constructor backing limit text",
                errorText(function(){
                    new Uint8Array(2147483648);
                })==="RangeError:invalid array buffer length");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn slice_and_subarray_default_species_use_the_method_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_uint8 = eval_object(
        &mut defining,
        "Uint8Array.prototype",
        "defining Uint8Array prototype",
    );
    let caller_uint8 = eval_object(
        &mut caller,
        "Uint8Array.prototype",
        "caller Uint8Array prototype",
    );
    let slice_object = eval_object(
        &mut defining,
        "Object.getPrototypeOf(Uint8Array.prototype).slice",
        "defining slice",
    );
    let slice = runtime
        .as_callable(&slice_object)
        .unwrap()
        .expect("defining slice was not callable");
    let subarray_object = eval_object(
        &mut defining,
        "Object.getPrototypeOf(Uint8Array.prototype).subarray",
        "defining subarray",
    );
    let subarray = runtime
        .as_callable(&subarray_object)
        .unwrap()
        .expect("defining subarray was not callable");

    let default_source = eval_object(
        &mut caller,
        "(globalThis.defaultSource=new Uint8Array([1,2]),\
          defaultSource.constructor=undefined,defaultSource)",
        "caller source with default species",
    );
    let Value::Object(slice_result) = caller
        .call(&slice, Value::Object(default_source.clone()), &[])
        .expect("cross-realm default slice")
    else {
        panic!("cross-realm default slice did not return an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&slice_result).unwrap(),
        Some(defining_uint8.clone()),
        "default slice species did not use the method defining realm",
    );
    let Value::Object(subarray_result) = caller
        .call(&subarray, Value::Object(default_source), &[])
        .expect("cross-realm default subarray")
    else {
        panic!("cross-realm default subarray did not return an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&subarray_result).unwrap(),
        Some(defining_uint8),
        "default subarray species did not use the method defining realm",
    );

    let inherited_source = eval_object(
        &mut caller,
        "new Uint8Array([1,2])",
        "caller source with inherited species",
    );
    let Value::Object(inherited_result) = caller
        .call(&slice, Value::Object(inherited_source), &[])
        .expect("cross-realm inherited slice")
    else {
        panic!("cross-realm inherited slice did not return an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&inherited_result).unwrap(),
        Some(caller_uint8),
        "inherited slice species did not follow the source constructor realm",
    );
}
