use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray sort test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("TypedArray sort rejected {description}: {error}"))
    else {
        panic!("TypedArray sort {description} was not an Object");
    };
    object
}

fn take_exception_object(context: &mut Context, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .take_exception()
        .unwrap()
        .unwrap_or_else(|| panic!("{description} did not leave an exception"))
    else {
        panic!("{description} exception was not an Object");
    };
    object
}

#[test]
fn sort_and_to_sorted_publish_the_quickjs_surface() {
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
            for(var name of ["sort","toSorted"]){
                var descriptor=Object.getOwnPropertyDescriptor(base,name);
                check(name+" descriptor",
                    descriptor.writable===true &&
                    descriptor.enumerable===false &&
                    descriptor.configurable===true);
                check(name+" length",base[name].length===1);
                check(name+" name",base[name].name===name);
                check(name+" no prototype",
                    !Object.prototype.hasOwnProperty.call(base[name],"prototype"));
                check(name+" not constructor",
                    completion(function(){new base[name]})===
                    "TypeError:"+name+" is not a constructor");
            }

            var source=new Int16Array([3,-1,2,-7,0]);
            var returned=source.sort();
            check("sort returns receiver",returned===source);
            check("signed default",source.join("|")==="-7|-1|0|2|3");

            var unsigned=new Uint32Array([4294967295,3,0,2147483648]);
            unsigned.sort();
            check("unsigned default",
                unsigned.join("|")==="0|3|2147483648|4294967295");

            var signedBig=new BigInt64Array([
                9223372036854775807n,-1n,-9223372036854775808n,0n,7n
            ]);
            signedBig.sort();
            check("signed bigint",
                signedBig.join("|")===
                "-9223372036854775808|-1|0|7|9223372036854775807");
            var unsignedBig=new BigUint64Array([
                18446744073709551615n,1n,0n,9223372036854775808n
            ]);
            unsignedBig.sort();
            check("unsigned bigint",
                unsignedBig.join("|")===
                "0|1|9223372036854775808|18446744073709551615");

            source=new Int16Array([3,-1,2]);
            var copied=source.toSorted();
            check("toSorted values",copied.join("|")==="-1|2|3");
            check("toSorted source",source.join("|")==="3|-1|2");
            check("toSorted distinct",
                copied!==source && copied.buffer!==source.buffer);
            check("toSorted concrete class",
                Object.getPrototypeOf(copied)===Int16Array.prototype);

            var calls=0;
            new Uint8Array(0).sort(function(){calls++});
            new Uint8Array(1).sort(function(){calls++});
            new Uint8Array(0).toSorted(function(){calls++});
            new Uint8Array(1).toSorted(function(){calls++});
            check("short arrays skip comparator",calls===0);

            check("QuickJS own-key order",
                Reflect.ownKeys(base).map(function(key){
                    return typeof key==="symbol" ? key.toString() : key;
                }).join("|")===
                "length|at|with|buffer|byteLength|byteOffset|set|values|keys|"+
                "entries|copyWithin|every|some|forEach|map|filter|reduce|"+
                "reduceRight|fill|find|findIndex|findLast|findLastIndex|"+
                "reverse|toReversed|slice|subarray|sort|toSorted|join|"+
                "toLocaleString|indexOf|lastIndexOf|includes|constructor|"+
                "toString|Symbol(Symbol.iterator)|Symbol(Symbol.toStringTag)");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn sort_and_to_sorted_keep_their_distinct_validation_order() {
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

            check("sort comparefn before brand",
                completion(function(){base.sort.call({},false)})===
                "TypeError:not a function");
            check("sort brand with undefined",
                completion(function(){base.sort.call({})})===
                "TypeError:not a TypedArray");
            check("sort proxy comparefn first",
                completion(function(){
                    base.sort.call(new Proxy(new Uint8Array([1]),{}),false);
                })==="TypeError:not a function");
            check("sort proxy brand",
                completion(function(){
                    base.sort.call(new Proxy(new Uint8Array([1]),{}));
                })==="TypeError:not a TypedArray");

            var detachedBuffer=new ArrayBuffer(2);
            var detached=new Uint8Array(detachedBuffer);
            detachedBuffer.transfer();
            check("sort detached invalid comparefn",
                completion(function(){base.sort.call(detached,false)})===
                "TypeError:not a function");
            check("sort detached",
                completion(function(){base.sort.call(detached)})===
                "TypeError:ArrayBuffer is detached or resized");
            check("toSorted detached invalid comparefn",
                completion(function(){base.toSorted.call(detached,false)})===
                "TypeError:ArrayBuffer is detached or resized");

            var fixedBuffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(fixedBuffer,1,2);
            fixedBuffer.resize(0);
            check("sort fixed OOB invalid comparefn",
                completion(function(){base.sort.call(fixed,false)})===
                "TypeError:not a function");
            check("sort fixed OOB",
                completion(function(){base.sort.call(fixed)})===
                "TypeError:ArrayBuffer is detached or resized");
            check("toSorted fixed OOB invalid comparefn",
                completion(function(){base.toSorted.call(fixed,false)})===
                "TypeError:ArrayBuffer is detached or resized");

            check("toSorted brand before comparefn",
                completion(function(){base.toSorted.call({},false)})===
                "TypeError:not a TypedArray");
            check("toSorted proxy brand",
                completion(function(){
                    base.toSorted.call(new Proxy(new Uint8Array([1]),{}),false);
                })==="TypeError:not a TypedArray");
            check("empty sort validates comparefn",
                completion(function(){new Uint8Array(0).sort(false)})===
                "TypeError:not a function");
            check("empty toSorted validates comparefn",
                completion(function(){new Uint8Array(0).toSorted(false)})===
                "TypeError:not a function");

            var marker={};
            var value=new Uint8Array([2,1]);
            Object.defineProperty(value,"length",{
                get:function(){throw marker},
                configurable:true
            });
            Object.defineProperty(value,"buffer",{
                get:function(){throw marker},
                configurable:true
            });
            check("sort ignores public accessors",
                value.sort().join("|")==="1|2");
            check("toSorted ignores public accessors",
                value.toSorted().join("|")==="1|2");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn default_sort_preserves_float_words_and_quickjs_equal_choreography() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function hex32(view,index){
                var value=view.getUint32(index*4,true).toString(16);
                return "00000000".slice(value.length)+value;
            }
            function words32(view,count){
                var result=[];
                for(var i=0;i<count;i++)result.push(hex32(view,i));
                return result.join("|");
            }

            var input=[
                0x7fa00001,0x80000000,0x7fc00002,0x00000000,
                0xffa00003,0xbf800000,0x3f800000
            ];
            var buffer=new ArrayBuffer(input.length*4);
            var view=new DataView(buffer);
            for(var i=0;i<input.length;i++)view.setUint32(i*4,input[i],true);
            var floats=new Float32Array(buffer);
            floats.sort();
            check("float32 raw order",
                words32(view,input.length)===
                "bf800000|80000000|00000000|3f800000|"+
                "7fa00001|ffa00003|7fc00002");
            check("negative zero before positive zero",
                Object.is(floats[1],-0) && Object.is(floats[2],0));

            buffer=new ArrayBuffer(input.length*4);
            view=new DataView(buffer);
            for(i=0;i<input.length;i++)view.setUint32(i*4,input[i],true);
            floats=new Float32Array(buffer);
            var sorted=floats.toSorted();
            var sortedView=new DataView(sorted.buffer);
            check("toSorted raw order",
                words32(sortedView,input.length)===
                "bf800000|80000000|00000000|3f800000|"+
                "7fa00001|ffa00003|7fc00002");
            check("toSorted raw source unchanged",
                words32(view,input.length)===
                "7fa00001|80000000|7fc00002|00000000|"+
                "ffa00003|bf800000|3f800000");

            buffer=new ArrayBuffer(40);
            view=new DataView(buffer);
            for(i=0;i<10;i++)view.setUint32(i*4,0x7fc00000+i,true);
            new Float32Array(buffer).sort();
            check("QuickJS equal-NaN rqsort choreography",
                words32(view,10)===
                "7fc00006|7fc00001|7fc00002|7fc00003|7fc00004|"+
                "7fc00005|7fc00000|7fc00007|7fc00008|7fc00009");

            function put64(view,index,high,low){
                view.setUint32(index*8,low,true);
                view.setUint32(index*8+4,high,true);
            }
            function hex64(view,index){
                return hex32(new DataView(view.buffer,view.byteOffset+index*8+4,4),0)+
                    hex32(new DataView(view.buffer,view.byteOffset+index*8,4),0);
            }
            function words64(view,count){
                var result=[];
                for(var i=0;i<count;i++)result.push(hex64(view,i));
                return result.join("|");
            }
            var words=[
                [0x7ff40000,0x00000001],[0x80000000,0x00000000],
                [0x7ff80000,0x00000002],[0x00000000,0x00000000],
                [0xfff40000,0x00000003],[0xbff00000,0x00000000],
                [0x3ff00000,0x00000000]
            ];
            buffer=new ArrayBuffer(words.length*8);
            view=new DataView(buffer);
            for(i=0;i<words.length;i++)put64(view,i,words[i][0],words[i][1]);
            new Float64Array(buffer).sort();
            check("float64 raw order",
                words64(view,words.length)===
                "bff0000000000000|8000000000000000|0000000000000000|"+
                "3ff0000000000000|7ff4000000000001|fff4000000000003|"+
                "7ff8000000000002");

            buffer=new ArrayBuffer(20);
            view=new DataView(buffer);
            view.setUint32(0,0xdeadbeef,true);
            view.setUint32(4,0x40400000,true);
            view.setUint32(8,0x7fc12345,true);
            view.setUint32(12,0x80000000,true);
            view.setUint32(16,0xbf800000,true);
            new Float32Array(buffer,4,4).sort();
            check("byteOffset sentinel",view.getUint32(0,true)===0xdeadbeef);
            check("byteOffset range",
                [hex32(view,1),hex32(view,2),hex32(view,3),hex32(view,4)]
                    .join("|")===
                "bf800000|80000000|40400000|7fc12345");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn custom_sort_uses_a_raw_snapshot_exact_rqsort_and_stable_indices() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let layout_source = eval_object(
        &mut context,
        "new Uint8Array([10,9,8,7])",
        "custom snapshot layout source",
    );
    let layout_state = runtime.typed_array_state(&layout_source).unwrap();
    let (raw_bytes, indices) = runtime
        .snapshot_custom_typed_array_sort(layout_state, layout_state.length)
        .unwrap();
    let layout_length = usize::try_from(layout_state.length).unwrap();
    assert_eq!(raw_bytes, [10, 9, 8, 7]);
    assert_eq!(indices, [0, 1, 2, 3]);
    assert_eq!(raw_bytes.len(), layout_length);
    assert_eq!(
        std::mem::size_of_val(indices.as_slice()),
        layout_length * std::mem::size_of::<u32>(),
    );

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }

            var log="";
            var source=new Uint8Array([10,9,8,7]);
            source.sort(function(left,right){
                log+=left+":"+right+",";
                return left-right;
            });
            check("pinned rqsort sequence",
                log==="10:9,10:8,9:8,10:7,9:7,8:7,");
            check("pinned rqsort result",source.join("|")==="7|8|9|10");

            log="";
            source=new Uint8Array([3,2,1]);
            source.sort(function(left,right){
                log+=left+":"+right+",";
                source.fill(9);
                return left-right;
            });
            check("snapshot comparator values",log==="3:2,3:1,2:1,");
            check("snapshot writeback overwrites mutation",
                source.join("|")==="1|2|3");

            var calls=0;
            new Uint8Array([1,1]).sort(function(){
                calls++;
                return 0;
            });
            check("identical values still compare",calls===1);
            var nanBuffer=new ArrayBuffer(8);
            var nanView=new DataView(nanBuffer);
            nanView.setUint32(0,0x7fc12345,true);
            nanView.setUint32(4,0x7fc54321,true);
            calls=0;
            new Float32Array(nanBuffer).sort(function(){
                calls++;
                return 0;
            });
            check("identical NaN semantics still compare",calls===1);
            check("custom zero preserves NaN words",
                nanView.getUint32(0,true)===0x7fc12345 &&
                nanView.getUint32(4,true)===0x7fc54321);

            source=new Uint8Array([21,11,22,12,23,13]);
            source.sort(function(left,right){
                return Math.floor(left/10)-Math.floor(right/10);
            });
            check("stable original indices",
                source.join("|")==="11|12|13|21|22|23");

            var strictThis="missing";
            var argumentKinds="";
            new BigInt64Array([2n,1n]).sort(function(left,right){
                "use strict";
                strictThis=this;
                argumentKinds=typeof left+":"+typeof right;
                return left<right ? -1 : left>right ? 1 : 0;
            });
            check("comparator this",strictThis===undefined);
            check("bigint comparator arguments",
                argumentKinds==="bigint:bigint");

            var converted=false;
            var transferBuffer=new ArrayBuffer(3);
            source=new Uint8Array(transferBuffer);
            source.set([3,2,1]);
            var transferred=false;
            source.sort(function(){
                if(!transferred){
                    transferred=true;
                    transferBuffer.transfer();
                }
                return {
                    [Symbol.toPrimitive]:function(){
                        converted=true;
                        return 0;
                    }
                };
            });
            check("ToNumber after detach",converted===true);
            check("detach succeeds without write",source.length===0);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn custom_sort_propagates_user_and_conversion_throws_without_sort_writeback() {
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

            var marker={kind:"callback"};
            var source=new Uint8Array([3,2,1]);
            var thrown;
            try{
                source.sort(function(){
                    source[0]=9;
                    throw marker;
                });
            }catch(error){thrown=error}
            check("callback throw identity",thrown===marker);
            check("callback mutation remains",source.join("|")==="9|2|1");

            marker={kind:"conversion"};
            source=new Uint8Array([3,2,1]);
            thrown=undefined;
            try{
                source.sort(function(){
                    source[0]=8;
                    return {
                        [Symbol.toPrimitive]:function(){throw marker}
                    };
                });
            }catch(error){thrown=error}
            check("conversion throw identity",thrown===marker);
            check("conversion throw no writeback",source.join("|")==="8|2|1");

            marker={kind:"detached conversion"};
            var buffer=new ArrayBuffer(3);
            source=new Uint8Array(buffer);
            source.set([3,2,1]);
            thrown=undefined;
            try{
                source.sort(function(){
                    buffer.transfer();
                    return {
                        [Symbol.toPrimitive]:function(){throw marker}
                    };
                });
            }catch(error){thrown=error}
            check("detached conversion throw identity",thrown===marker);

            check("bigint comparator result conversion",
                completion(function(){
                    new BigInt64Array([2n,1n]).sort(function(a,b){
                        return a-b;
                    });
                })==="TypeError:cannot convert bigint to number");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn custom_sort_matches_quickjs_final_rab_state_writeback() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }

            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            var source=new Uint8Array(buffer);
            source.set([10,9,8,7]);
            var first=true;
            var returned=source.sort(function(left,right){
                if(first){
                    first=false;
                    buffer.resize(2);
                }
                return left-right;
            });
            check("shrink returns receiver",returned===source);
            check("shrink clips sorted prefix",source.join("|")==="7|8");

            buffer=new ArrayBuffer(2,{maxByteLength:6});
            source=new Uint8Array(buffer);
            source.set([2,1]);
            first=true;
            source.sort(function(left,right){
                if(first){
                    first=false;
                    buffer.resize(6);
                }
                return left-right;
            });
            check("grow keeps old range",source.join("|")==="1|2|0|0|0|0");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            source=new Uint8Array(buffer,0,4);
            source.set([10,9,8,7]);
            var calls=0;
            source.sort(function(left,right){
                calls++;
                if(calls===1)buffer.resize(2);
                if(calls===2)buffer.resize(4);
                return left-right;
            });
            check("transient fixed OOB revalidates final state",
                source.join("|")==="7|8|9|10");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            source=new Uint8Array(buffer,0,4);
            source.set([10,9,8,7]);
            first=true;
            returned=source.sort(function(left,right){
                if(first){
                    first=false;
                    buffer.resize(2);
                }
                return left-right;
            });
            check("final fixed OOB returns receiver",returned===source);
            check("final fixed OOB length",source.length===0);
            buffer.resize(4);
            check("final fixed OOB skips writeback",
                source.join("|")==="10|9|0|0");

            buffer=new ArrayBuffer(4);
            source=new Uint8Array(buffer);
            source.set([10,9,8,7]);
            first=true;
            returned=source.sort(function(left,right){
                if(first){
                    first=false;
                    buffer.transfer();
                }
                return left-right;
            });
            check("detach returns receiver",returned===source);
            check("detach skips writeback",source.length===0);

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            source=new Uint8Array(buffer);
            source.set([10,9,8,7]);
            first=true;
            var copied=source.toSorted(function(left,right){
                if(first){
                    first=false;
                    buffer.resize(2);
                }
                return left-right;
            });
            check("toSorted private snapshot",copied.join("|")==="7|8|9|10");
            check("toSorted source resize",source.join("|")==="10|9");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn to_sorted_ignores_species_and_returns_fixed_defining_class_storage() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            var token={};
            class Sub extends Uint8Array {}
            var source=new Sub([3,1,2]);
            Object.defineProperty(source,"constructor",{
                get:function(){throw token}
            });
            var result=source.toSorted();
            check("constructor ignored",result.join("|")==="1|2|3");
            check("builtin prototype",
                Object.getPrototypeOf(result)===Uint8Array.prototype);
            check("not subclass",!(result instanceof Sub));
            check("source unchanged",source.join("|")==="3|1|2");
            check("fixed result buffer",
                result.buffer.resizable===false &&
                result.buffer.maxByteLength===3);

            var buffer=new ArrayBuffer(3,{maxByteLength:8});
            source=new Uint8Array(buffer);
            source.set([3,1,2]);
            result=source.toSorted();
            check("RAB source result",result.join("|")==="1|2|3");
            check("RAB result fixed",
                result.buffer.resizable===false &&
                result.buffer.maxByteLength===3);
            check("RAB source retained",source.join("|")==="3|1|2");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn sort_errors_and_to_sorted_results_use_the_method_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
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
    let sort = runtime
        .as_callable(&eval_object(
            &mut defining,
            "Object.getPrototypeOf(Uint8Array.prototype).sort",
            "defining sort",
        ))
        .unwrap()
        .expect("defining sort was not callable");
    let to_sorted = runtime
        .as_callable(&eval_object(
            &mut defining,
            "Object.getPrototypeOf(Uint8Array.prototype).toSorted",
            "defining toSorted",
        ))
        .unwrap()
        .expect("defining toSorted was not callable");
    let source = eval_object(
        &mut caller,
        r#"(function(){
            var value=new Uint8Array([3,1,2]);
            Object.defineProperty(value,"constructor",{
                get:function(){throw new Error("constructor observed")}
            });
            return value;
        })()"#,
        "caller TypedArray source",
    );

    assert!(matches!(
        caller.call(&sort, Value::Object(source.clone()), &[Value::Bool(false)],),
        Err(RuntimeError::Exception),
    ));
    let invalid_comparator = take_exception_object(&mut caller, "invalid sort comparator");
    assert_eq!(
        runtime.get_prototype_of(&invalid_comparator).unwrap(),
        Some(defining_type_error.clone()),
        "sort comparator TypeError did not use the method defining realm",
    );

    let Value::Object(result) = caller
        .call(&to_sorted, Value::Object(source), &[])
        .expect("cross-realm toSorted")
    else {
        panic!("cross-realm toSorted did not return an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_uint8),
        "toSorted did not use the method defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(caller_uint8),
        "toSorted unexpectedly used the caller realm",
    );
    let buffer_key = runtime.intern_property_key("buffer").unwrap();
    let Value::Object(result_buffer) = caller.get_property(&result, &buffer_key).unwrap() else {
        panic!("cross-realm toSorted result buffer was not an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result_buffer).unwrap(),
        Some(defining_array_buffer),
        "toSorted result buffer did not use the method defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&result_buffer).unwrap(),
        Some(caller_array_buffer),
        "toSorted result buffer unexpectedly used the caller realm",
    );

    let throwing = runtime
        .as_callable(&eval_object(
            &mut caller,
            "(function(){throw new TypeError('caller comparator')})",
            "caller throwing comparator",
        ))
        .unwrap()
        .expect("caller throwing comparator was not callable");
    let source = eval_object(
        &mut caller,
        "new Uint8Array([2,1])",
        "caller throwing source",
    );
    assert!(matches!(
        caller.call(
            &sort,
            Value::Object(source),
            &[Value::Object(throwing.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let callback_error = take_exception_object(&mut caller, "sort callback error");
    assert_eq!(
        runtime.get_prototype_of(&callback_error).unwrap(),
        Some(caller_type_error),
        "sort replaced a caller callback throw with a defining-realm error",
    );
}
