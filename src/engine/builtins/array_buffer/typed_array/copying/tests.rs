use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray copying test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("TypedArray copying rejected {description}: {error}"))
    else {
        panic!("TypedArray copying {description} was not an Object");
    };
    object
}

#[test]
fn with_and_to_reversed_publish_quickjs_copying_surface() {
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
            for(var entry of [["with",2],["toReversed",0]]){
                var name=entry[0];
                var descriptor=Object.getOwnPropertyDescriptor(base,name);
                check(name+" descriptor",
                    descriptor.writable===true &&
                    descriptor.enumerable===false &&
                    descriptor.configurable===true);
                check(name+" length",base[name].length===entry[1]);
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
            var replaced=source.with(-2,99);
            check("with values",
                replaced.length===4 && replaced[0]===10 &&
                replaced[1]===20 && replaced[2]===99 && replaced[3]===40);
            check("with immutable",
                source[0]===10 && source[1]===20 &&
                source[2]===30 && source[3]===40);
            check("with distinct buffer",replaced.buffer!==source.buffer);

            var reversed=source.toReversed();
            check("toReversed values",
                reversed.length===4 && reversed[0]===40 &&
                reversed[1]===30 && reversed[2]===20 && reversed[3]===10);
            check("toReversed immutable",
                source[0]===10 && source[1]===20 &&
                source[2]===30 && source[3]===40);
            check("toReversed distinct buffer",reversed.buffer!==source.buffer);

            source.constructor={
                get [Symbol.species](){
                    throw new Error("species observed");
                }
            };
            check("with ignores species",source.with(0,7)[0]===7);
            check("toReversed ignores species",source.toReversed()[0]===40);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn with_matches_quickjs_coercion_resize_and_raw_copy_contracts() {
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

            var log="";
            var source=new Uint8Array([1,2,3]);
            var index={valueOf:function(){log+="I";return -2}};
            var value={valueOf:function(){log+="V";return 9}};
            var result=source.with(index,value);
            check("coercion order",log==="IV");
            check("negative index",result[0]===1 && result[1]===9 && result[2]===3);

            log="";
            source=new Uint8Array([1]);
            index={valueOf:function(){log+="I";return 4}};
            value={valueOf:function(){log+="V";return 9}};
            check("value before range",
                completion(function(){source.with(index,value)})===
                "RangeError:invalid array index");
            check("range coercion order",log==="IV");
            check("symbol replacement range wins",
                completion(function(){source.with(4,Symbol())})===
                "RangeError:invalid array index");

            buffer=new ArrayBuffer(0,{maxByteLength:1});
            source=new Uint8Array(buffer);
            check("grown source still converts for zero-length target",
                completion(function(){
                    source.with(0,{
                        valueOf:function(){
                            buffer.resize(1);
                            return Symbol();
                        }
                    });
                })==="TypeError:cannot convert symbol to number");
            check("replacement grew source",source.length===1);

            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            source=new Uint8Array(buffer);
            source.set([1,2,3,4]);
            value={
                valueOf:function(){
                    buffer.resize(2);
                    return 9;
                }
            };
            result=source.with(-4,value);
            check("pre-resize negative index",result[0]===9);
            check("retains initial result length",result.length===4);
            check("numeric missing tail",
                result[0]===9 && result[1]===2 &&
                result[2]===0 && result[3]===0);

            buffer=new ArrayBuffer(16,{maxByteLength:32});
            var floats=new Float32Array(buffer);
            floats.set([1,2,3,4]);
            value={
                valueOf:function(){
                    buffer.resize(8);
                    return 9;
                }
            };
            result=floats.with(-4,value);
            check("float shrink keeps initial length",result.length===4);
            check("float missing tail is NaN",
                result[0]===9 && result[1]===2 &&
                Number.isNaN(result[2]) && Number.isNaN(result[3]));

            buffer=new ArrayBuffer(32,{maxByteLength:64});
            var bigints=new BigInt64Array(buffer);
            bigints.set([1n,2n,3n,4n]);
            value={
                valueOf:function(){
                    buffer.resize(16);
                    return 9n;
                }
            };
            check("bigint missing tail throws",
                completion(function(){bigints.with(-4,value)})===
                "TypeError:cannot convert to bigint");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            source=new Uint8Array(buffer);
            value={
                valueOf:function(){
                    buffer.resize(0);
                    return 1;
                }
            };
            check("post-coercion oob is range",
                completion(function(){source.with(0,value)})===
                "RangeError:invalid array index");

            buffer=new ArrayBuffer(2);
            source=new Uint8Array(buffer);
            buffer.transfer();
            var hits=0;
            check("initial detached text",
                completion(function(){
                    source.with(
                        {valueOf:function(){hits++;return 0}},
                        {valueOf:function(){hits++;return 1}}
                    );
                })==="TypeError:ArrayBuffer is detached");
            check("initial detached skips coercion",hits===0);

            buffer=new ArrayBuffer(8);
            var words=new DataView(buffer);
            words.setUint32(0,0x7fc12345,true);
            words.setUint32(4,0x80000000,true);
            floats=new Float32Array(buffer);
            result=floats.with(1,1);
            var resultWords=new DataView(result.buffer);
            check("with preserves untouched NaN payload",
                resultWords.getUint32(0,true)===0x7fc12345);
            check("with writes replacement",
                result[1]===1 && resultWords.getUint32(4,true)===0x3f800000);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn to_reversed_preserves_raw_words_and_rejects_invalid_views() {
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

            var buffer=new ArrayBuffer(12);
            var words=new DataView(buffer);
            words.setUint32(0,0x7fc12345,true);
            words.setUint32(4,0x80000000,true);
            words.setUint32(8,0x3f800000,true);
            var source=new Float32Array(buffer);
            var result=source.toReversed();
            var copied=new DataView(result.buffer);
            check("raw reversed first",copied.getUint32(0,true)===0x3f800000);
            check("raw reversed negative zero",
                copied.getUint32(4,true)===0x80000000 && 1/result[1]===-Infinity);
            check("raw reversed NaN payload",
                copied.getUint32(8,true)===0x7fc12345);
            check("source unchanged",
                words.getUint32(0,true)===0x7fc12345 &&
                words.getUint32(4,true)===0x80000000 &&
                words.getUint32(8,true)===0x3f800000);

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,2,2);
            buffer.resize(1);
            check("oob text",
                completion(function(){fixed.toReversed()})===
                "TypeError:ArrayBuffer is detached or resized");
            check("constructor oob text",
                completion(function(){new Uint8Array(fixed)})===
                "TypeError:ArrayBuffer is detached or resized");

            buffer=new ArrayBuffer(2);
            source=new Uint8Array(buffer);
            buffer.transfer();
            check("detached text",
                completion(function(){source.toReversed()})===
                "TypeError:ArrayBuffer is detached or resized");
            check("constructor detached text",
                completion(function(){new Uint8Array(source)})===
                "TypeError:ArrayBuffer is detached or resized");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn copying_methods_use_the_method_defining_realm() {
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
    let defining_array_buffer = eval_object(
        &mut defining,
        "ArrayBuffer.prototype",
        "defining ArrayBuffer prototype",
    );
    let caller_array_buffer = eval_object(
        &mut caller,
        "ArrayBuffer.prototype",
        "caller ArrayBuffer prototype",
    );
    let with = runtime
        .as_callable(&eval_object(
            &mut defining,
            "Object.getPrototypeOf(Uint8Array.prototype).with",
            "defining with",
        ))
        .unwrap()
        .expect("defining with was not callable");
    let to_reversed = runtime
        .as_callable(&eval_object(
            &mut defining,
            "Object.getPrototypeOf(Uint8Array.prototype).toReversed",
            "defining toReversed",
        ))
        .unwrap()
        .expect("defining toReversed was not callable");
    let source = eval_object(&mut caller, "new Uint8Array([1,2])", "caller source");

    let Value::Object(with_result) = caller
        .call(
            &with,
            Value::Object(source.clone()),
            &[Value::Int(0), Value::Int(9)],
        )
        .expect("cross-realm with")
    else {
        panic!("cross-realm with did not return an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&with_result).unwrap(),
        Some(defining_uint8.clone()),
        "with did not use the method defining realm",
    );
    let buffer_key = runtime.intern_property_key("buffer").unwrap();
    let Value::Object(with_buffer) = caller.get_property(&with_result, &buffer_key).unwrap() else {
        panic!("cross-realm with result buffer was not an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&with_buffer).unwrap(),
        Some(defining_array_buffer.clone()),
        "with result buffer did not use the method defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&with_buffer).unwrap(),
        Some(caller_array_buffer.clone()),
        "with result buffer unexpectedly used the caller realm",
    );
    let Value::Object(reversed_result) = caller
        .call(&to_reversed, Value::Object(source), &[])
        .expect("cross-realm toReversed")
    else {
        panic!("cross-realm toReversed did not return an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&reversed_result).unwrap(),
        Some(defining_uint8),
        "toReversed did not use the method defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&reversed_result).unwrap(),
        Some(caller_uint8),
        "toReversed unexpectedly used the caller realm",
    );
    let Value::Object(reversed_buffer) =
        caller.get_property(&reversed_result, &buffer_key).unwrap()
    else {
        panic!("cross-realm toReversed result buffer was not an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&reversed_buffer).unwrap(),
        Some(defining_array_buffer),
        "toReversed result buffer did not use the method defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&reversed_buffer).unwrap(),
        Some(caller_array_buffer),
        "toReversed result buffer unexpectedly used the caller realm",
    );
}
