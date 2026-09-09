use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray transform test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("TypedArray transform rejected {description}: {error}"))
    else {
        panic!("TypedArray transform {description} was not an Object");
    };
    object
}

fn take_exception_object(context: &mut Context, description: &str) -> ObjectRef {
    let Value::Object(error) = context
        .take_exception()
        .unwrap_or_else(|failure| panic!("take {description}: {failure}"))
        .unwrap_or_else(|| panic!("{description} was missing"))
    else {
        panic!("{description} was not an Object");
    };
    error
}

#[test]
fn map_and_filter_match_quickjs_species_and_callback_contracts() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function values(array){
                var result=[];
                for(var index=0;index<array.length;index++)
                    result.push(String(array[index]));
                return result.join(",");
            }
            function errorText(operation){
                try{operation();return "return"}
                catch(error){return error.name+":"+error.message}
            }

            var source=new Uint8Array([1,2,3,4]);
            var sentinel={sentinel:true};
            var seen=[];
            var receiverMatches=true;
            var thisMatches=true;
            var mapped=source.map(function(value,index,receiver){
                "use strict";
                seen.push(value+":"+index);
                receiverMatches=receiverMatches && receiver===source;
                thisMatches=thisMatches && this===sentinel;
                return value*2+index;
            },sentinel);
            check("map type",mapped instanceof Uint8Array);
            check("map identity",mapped!==source);
            check("map buffer",mapped.buffer!==source.buffer);
            check("map values",values(mapped)==="2,5,8,11");
            check("map callback values",seen.join(",")==="1:0,2:1,3:2,4:3");
            check("map receiver",receiverMatches);
            check("map thisArg",thisMatches);

            seen=[];
            var filtered=source.filter(function(value,index,receiver){
                "use strict";
                seen.push(value+":"+index);
                receiverMatches=receiverMatches && receiver===source;
                thisMatches=thisMatches && this===sentinel;
                return value%2===0 ? {truthy:true} : 0;
            },sentinel);
            check("filter type",filtered instanceof Uint8Array);
            check("filter identity",filtered!==source);
            check("filter buffer",filtered.buffer!==source.buffer);
            check("filter preserves source values",values(filtered)==="2,4");
            check("filter callback values",
                seen.join(",")==="1:0,2:1,3:2,4:3");

            var bigintMapped=new BigInt64Array([1n,2n]).map(function(value){
                return value+2n;
            });
            check("BigInt map",values(bigintMapped)==="3,4");
            var bigintFiltered=
                new BigInt64Array([1n,2n,3n]).filter(function(value){
                    return value>1n;
                });
            check("BigInt filter",values(bigintFiltered)==="2,3");

            var mapLog=[];
            var mapOrderSource=new Uint8Array([3,4]);
            var mapConstructor={};
            Object.defineProperty(mapConstructor,Symbol.species,{
                get:function(){
                    mapLog.push("species");
                    return function(length){
                        mapLog.push("construct:"+length);
                        return new Uint8Array(length);
                    };
                }
            });
            Object.defineProperty(mapOrderSource,"constructor",{
                get:function(){
                    mapLog.push("constructor");
                    return mapConstructor;
                }
            });
            mapOrderSource.map(function(value,index){
                mapLog.push("callback:"+index);
                return value;
            });
            check("map allocates before callbacks",
                mapLog.join(",")==="constructor,species,construct:2,"+
                    "callback:0,callback:1");

            var filterLog=[];
            var filterOrderSource=new Uint8Array([3,4]);
            var filterResult=new Uint8Array(3);
            filterResult.set=function(selected){
                filterLog.push("set:"+selected.length+":"+selected[0]);
            };
            var filterConstructor={};
            Object.defineProperty(filterConstructor,Symbol.species,{
                get:function(){
                    filterLog.push("species");
                    return function(length){
                        filterLog.push("construct:"+length);
                        return filterResult;
                    };
                }
            });
            Object.defineProperty(filterOrderSource,"constructor",{
                get:function(){
                    filterLog.push("constructor");
                    return filterConstructor;
                }
            });
            var returned=filterOrderSource.filter(function(value,index){
                filterLog.push("callback:"+index);
                return index===1;
            });
            check("filter result identity",returned===filterResult);
            check("filter callbacks precede species",
                filterLog.join(",")==="callback:0,callback:1,constructor,"+
                    "species,construct:1,set:1:4");
            check("filter uses public set",values(returned)==="0,0,0");
            filterLog=[];
            filterOrderSource.filter(function(value,index){
                filterLog.push("callback:"+index);
                return false;
            });
            check("empty filter still calls public set",
                filterLog.join(",")==="callback:0,callback:1,constructor,"+
                    "species,construct:0,set:0:undefined");

            var touched=false;
            var invalidCallbackSource=new Uint8Array(0);
            Object.defineProperty(invalidCallbackSource,"constructor",{
                get:function(){
                    touched=true;
                    throw new Error("not reached");
                }
            });
            check("map validates callback before species",
                errorText(function(){
                    invalidCallbackSource.map(0);
                })==="TypeError:not a function");
            check("invalid callback skipped species",touched===false);

            var marker={marker:true};
            touched=false;
            var abruptFilter=new Uint8Array([1]);
            Object.defineProperty(abruptFilter,"constructor",{
                get:function(){
                    touched=true;
                    return Uint8Array;
                }
            });
            var caught;
            try{
                abruptFilter.filter(function(){throw marker});
            }catch(error){
                caught=error;
            }
            check("filter abrupt identity",caught===marker);
            check("filter abrupt skips species",touched===false);

            var crossSource=new Uint8Array([1,2]);
            crossSource.constructor={
                [Symbol.species]:function(length){
                    return new BigInt64Array(length);
                }
            };
            var crossMapped=crossSource.map(function(value){
                return BigInt(value);
            });
            check("cross content map",values(crossMapped)==="1,2");
            var emptyCross=new Uint8Array(0);
            emptyCross.constructor={
                [Symbol.species]:function(length){
                    return new BigInt64Array(length);
                }
            };
            var emptyFiltered=emptyCross.filter(function(){return true});
            check("empty cross content filter",
                emptyFiltered instanceof BigInt64Array &&
                emptyFiltered.length===0);
            var crossFilterSource=new Uint8Array([5]);
            var crossFilterResult=new BigInt64Array(1);
            var crossFilterArgument;
            crossFilterResult.set=function(values){
                crossFilterArgument=values;
            };
            crossFilterSource.constructor={
                [Symbol.species]:function(){
                    return crossFilterResult;
                }
            };
            var crossFiltered=crossFilterSource.filter(function(){return true});
            check("cross content filter public set wins",
                crossFiltered===crossFilterResult &&
                crossFilterArgument[0]===5 &&
                values(crossFiltered)==="0");

            var bad=new Uint8Array([1,2]);
            bad.constructor=1;
            check("primitive constructor",
                errorText(function(){
                    bad.map(function(value){return value});
                })==="TypeError:not an object");
            bad=new Uint8Array([1,2]);
            bad.constructor={[Symbol.species]:1};
            check("nonconstructor species",
                errorText(function(){
                    bad.map(function(value){return value});
                })==="TypeError:not a constructor");
            bad=new Uint8Array([1,2]);
            bad.constructor={
                [Symbol.species]:function(){return {}}
            };
            check("non TypedArray species result",
                errorText(function(){
                    bad.map(function(value){return value});
                })==="TypeError:not a TypedArray");
            bad=new Uint8Array([1,2]);
            bad.constructor={
                [Symbol.species]:function(){return new Uint8Array(1)}
            };
            var shortCallbackCalls=0;
            check("short species result",
                errorText(function(){
                    bad.map(function(value){
                        shortCallbackCalls++;
                        return value;
                    });
                })==="TypeError:TypedArray length is too small");
            check("map short result precedes callback",shortCallbackCalls===0);
            bad=new Uint8Array([1,2]);
            bad.constructor={[Symbol.species]:null};
            check("null species uses default",
                bad.map(function(value){return value}) instanceof Uint8Array);

            var base=Object.getPrototypeOf(Uint8Array.prototype);
            check("map not constructor",
                errorText(function(){
                    new base.map(function(value){return value});
                })==="TypeError:map is not a constructor");
            check("filter not constructor",
                errorText(function(){
                    new base.filter(function(value){return value});
                })==="TypeError:filter is not a constructor");
            for(var name of ["map","filter"]){
                var descriptor=Object.getOwnPropertyDescriptor(base,name);
                check(name+" value",typeof descriptor.value==="function");
                check(name+" name",descriptor.value.name===name);
                check(name+" length",descriptor.value.length===1);
                check(name+" writable",descriptor.writable===true);
                check(name+" enumerable",descriptor.enumerable===false);
                check(name+" configurable",descriptor.configurable===true);
                check(name+" no prototype",
                    Object.prototype.hasOwnProperty.call(
                        descriptor.value,"prototype"
                    )===false);
            }

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn map_and_filter_keep_snapshot_ranges_and_live_source_and_target_effects() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function printable(value){
                return value===undefined ? "undefined" : String(value);
            }
            function values(array){
                var result=[];
                for(var index=0;index<array.length;index++)
                    result.push(printable(array[index]));
                return result.join(",");
            }
            function errorText(operation){
                try{operation();return "return"}
                catch(error){return error.name+":"+error.message}
            }

            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,0,4);
            fixed.set([1,2,3,4]);
            var seen=[];
            fixed.constructor={
                [Symbol.species]:function(length){
                    buffer.resize(2);
                    return new Uint8Array(length);
                }
            };
            var mapped=fixed.map(function(value,index){
                seen.push(printable(value)+":"+index);
                return 0;
            });
            check("map species shrink visits snapshot",
                seen.join(",")==="undefined:0,undefined:1,"+
                    "undefined:2,undefined:3");
            check("map species shrink result",values(mapped)==="0,0,0,0");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            seen=[];
            tracking.constructor={
                [Symbol.species]:function(length){
                    buffer.resize(2);
                    return new Uint8Array(length);
                }
            };
            tracking.map(function(value,index){
                seen.push(printable(value)+":"+index);
                return 0;
            });
            check("tracking species shrink stays live",
                seen.join(",")==="1:0,2:1,undefined:2,undefined:3");

            var source=new Uint8Array([1,2,3]);
            var destinationBuffer=new ArrayBuffer(3);
            var destination=new Uint8Array(destinationBuffer);
            source.constructor={
                [Symbol.species]:function(){return destination}
            };
            var calls=0;
            var conversions=0;
            mapped=source.map(function(value,index){
                calls++;
                if(index===0) destinationBuffer.transfer();
                return {
                    valueOf:function(){
                        conversions++;
                        return value;
                    }
                };
            });
            check("map detached destination identity",mapped===destination);
            check("map continues after target oob",calls===3);
            check("map converts dropped writes",conversions===3);
            check("map target remains empty",mapped.length===0);

            buffer=new ArrayBuffer(2,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            seen=[];
            tracking.map(function(value,index){
                seen.push(value+":"+index);
                if(index===0){
                    buffer.resize(4);
                    tracking[2]=7;
                    tracking[3]=8;
                }
                return value;
            });
            check("map grow does not extend snapshot",
                seen.join(",")==="1:0,2:1");

            source=new Uint8Array([1,2,3]);
            seen=[];
            source.map(function(value,index,receiver){
                seen.push(value);
                if(index===0) receiver[1]=9;
                return value;
            });
            check("map later writes are live",seen.join(",")==="1,9,3");

            buffer=new ArrayBuffer(4);
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            seen=[];
            var filtered=tracking.filter(function(value,index){
                seen.push(printable(value)+":"+index);
                if(index===0) buffer.transfer();
                return true;
            });
            check("filter detach visits snapshot",
                seen.join(",")==="1:0,undefined:1,undefined:2,undefined:3");
            check("filter captured values survive",values(filtered)==="1,0,0,0");

            source=new Uint8Array([1,2]);
            var lateResult=new Uint8Array(2);
            seen=[];
            filtered=source.filter(function(value,index,receiver){
                seen.push(index);
                if(index===1){
                    receiver.constructor={
                        [Symbol.species]:function(){return lateResult}
                    };
                }
                return true;
            });
            check("filter late species identity",filtered===lateResult);
            check("filter late species values",values(filtered)==="1,2");
            check("filter callbacks completed",seen.join(",")==="0,1");

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
            tracking.filter(function(value,index){
                seen.push(printable(value));
                if(index===0) buffer.resize(1);
                return false;
            });
            delete base["2"];
            check("filter skips numeric prototype",prototypeHits===0);
            check("filter missing values stay undefined",
                seen.join(",")==="1,undefined,undefined,undefined");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            fixed=new Uint8Array(buffer,0,4);
            buffer.resize(2);
            check("initial oob before callback",
                errorText(function(){
                    fixed.map(0);
                })==="TypeError:ArrayBuffer is detached or resized");
            check("empty validates callback",
                errorText(function(){
                    new Uint8Array(0).filter(0);
                })==="TypeError:not a function");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn map_filter_species_and_hidden_arrays_keep_quickjs_realms() {
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
    let defining_array = defining.array_prototype().unwrap();
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let map_object = eval_object(
        &mut defining,
        "Object.getPrototypeOf(Uint8Array.prototype).map",
        "defining map",
    );
    let map = runtime
        .as_callable(&map_object)
        .unwrap()
        .expect("defining map was not callable");
    let filter_object = eval_object(
        &mut defining,
        "Object.getPrototypeOf(Uint8Array.prototype).filter",
        "defining filter",
    );
    let filter = runtime
        .as_callable(&filter_object)
        .unwrap()
        .expect("defining filter was not callable");
    let identity_object = eval_object(
        &mut caller,
        "(function(value){return value})",
        "caller identity callback",
    );
    let identity = runtime
        .as_callable(&identity_object)
        .unwrap()
        .expect("caller identity callback was not callable");

    let default_source = eval_object(
        &mut caller,
        "(globalThis.defaultSource=new Uint8Array([1]),\
          defaultSource.constructor=undefined,defaultSource)",
        "caller source with default species",
    );
    let Value::Object(default_result) = caller
        .call(
            &map,
            Value::Object(default_source),
            &[Value::Object(identity.as_object().clone())],
        )
        .expect("cross-realm map with default species")
    else {
        panic!("cross-realm default map did not return an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&default_result).unwrap(),
        Some(defining_uint8),
        "default TypedArray species did not use the method defining realm",
    );

    let inherited_source = eval_object(
        &mut caller,
        "new Uint8Array([1])",
        "caller source with inherited species",
    );
    let Value::Object(inherited_result) = caller
        .call(
            &map,
            Value::Object(inherited_source),
            &[Value::Object(identity.as_object().clone())],
        )
        .expect("cross-realm map with inherited species")
    else {
        panic!("cross-realm inherited map did not return an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&inherited_result).unwrap(),
        Some(caller_uint8),
        "inherited TypedArray species did not follow the source constructor realm",
    );

    let filter_source = eval_object(
        &mut caller,
        r#"(globalThis.filterResult=new Uint8Array(1),
            filterResult.set=function(values){
                globalThis.capturedFilterValues=values;
            },
            globalThis.filterSource=new Uint8Array([1]),
            filterSource.constructor={
                [Symbol.species]:function(){return filterResult}
            },
            filterSource)"#,
        "caller filter species source",
    );
    caller
        .call(
            &filter,
            Value::Object(filter_source),
            &[Value::Object(identity.as_object().clone())],
        )
        .expect("cross-realm filter");
    let captured = eval_object(
        &mut caller,
        "capturedFilterValues",
        "captured filter temporary Array",
    );
    assert_eq!(
        runtime.get_prototype_of(&captured).unwrap(),
        Some(defining_array),
        "filter temporary Array did not use the method defining realm",
    );

    let bad_source = eval_object(
        &mut caller,
        "(globalThis.badSource=new Uint8Array([1]),\
          badSource.constructor=1,badSource)",
        "caller source with primitive constructor",
    );
    assert!(matches!(
        caller.call(
            &map,
            Value::Object(bad_source),
            &[Value::Object(identity.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "TypedArray map species TypeError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error),
        "TypedArray map species TypeError did not use the method defining realm",
    );
}
