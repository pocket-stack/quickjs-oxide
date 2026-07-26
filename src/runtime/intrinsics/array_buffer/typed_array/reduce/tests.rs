use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray reduce test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("TypedArray reduce rejected {description}: {error}"))
    else {
        panic!("TypedArray reduce {description} was not an Object");
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
fn reduce_family_matches_quickjs_accumulator_and_descriptor_contracts() {
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

            var bytes=new Uint8Array([1,2,3,4]);
            var seen=[];
            var receiverMatches=true;
            var callbackThis="unset";
            var left=bytes.reduce(function(accumulator,value,index,receiver){
                "use strict";
                seen.push(accumulator+":"+value+":"+index);
                receiverMatches=receiverMatches && receiver===bytes;
                callbackThis=this;
                return accumulator+value;
            },10);
            check("reduce result",left===20);
            check("reduce order",
                seen.join(",")==="10:1:0,11:2:1,13:3:2,16:4:3");
            check("reduce receiver",receiverMatches);
            check("reduce this",callbackThis===undefined);

            seen=[];
            var right=bytes.reduceRight(function(accumulator,value,index){
                seen.push(accumulator+":"+value+":"+index);
                return accumulator-value;
            },20);
            check("reduceRight result",right===10);
            check("reduceRight order",
                seen.join(",")==="20:4:3,16:3:2,13:2:1,11:1:0");

            seen=[];
            check("default accumulator",
                new Uint8Array([2,3,4]).reduce(function(accumulator,value,index){
                    seen.push(accumulator+":"+value+":"+index);
                    return accumulator*value;
                })===24);
            check("default accumulator skips first",
                seen.join(",")==="2:3:1,6:4:2");
            seen=[];
            check("reverse default accumulator",
                new Uint8Array([2,3,4]).reduceRight(
                    function(accumulator,value,index){
                        seen.push(accumulator+":"+value+":"+index);
                        return accumulator-value;
                    }
                )===-1);
            check("reverse default skips last",
                seen.join(",")==="4:3:1,1:2:0");

            var called=false;
            check("single returns element",
                new Uint8Array([9]).reduce(function(){
                    called=true;
                })===9);
            check("single skips callback",called===false);
            check("empty explicit undefined",
                new Uint8Array(0).reduce(function(){
                    called=true;
                },undefined)===undefined);
            check("empty explicit skips callback",called===false);

            check("bigint reduce",
                new BigInt64Array([1n,2n,3n]).reduce(function(a,b){
                    return a+b;
                },0n)===6n);
            var marker={marker:true};
            check("arbitrary accumulator identity",
                new Uint8Array(0).reduce(function(){},marker)===marker);
            var caught;
            try{
                bytes.reduce(function(){throw marker},0);
            }catch(error){
                caught=error;
            }
            check("abrupt identity",caught===marker);

            var generatorBody=false;
            function* generatorCallback(){
                generatorBody=true;
            }
            var generatorResult=
                new Uint8Array([1]).reduce(generatorCallback,0);
            check("generator result retained",
                typeof generatorResult.next==="function");
            check("generator body not entered",generatorBody===false);

            var lengthHits=0;
            Object.defineProperty(bytes,"length",{
                configurable:true,
                get:function(){
                    lengthHits++;
                    return 0;
                }
            });
            check("internal length",
                bytes.reduce(function(a,b){return a+b},0)===10);
            check("length property ignored",lengthHits===0);
            bytes["1.5"]=99;
            seen=[];
            bytes.reduce(function(accumulator,value,index){
                seen.push(index);
                return accumulator+value;
            },0);
            check("noninteger property ignored",seen.join(",")==="0,1,2,3");

            check("exact brand error",
                errorText(function(){
                    Uint8Array.prototype.reduce.call({},function(){});
                })==="TypeError:not a TypedArray");
            check("proxy is not branded",
                errorText(function(){
                    Uint8Array.prototype.reduceRight.call(
                        new Proxy(bytes,{}),
                        function(){}
                    );
                })==="TypeError:not a TypedArray");
            check("empty validates callback first",
                errorText(function(){
                    new Uint8Array(0).reduce(0);
                })==="TypeError:not a function");
            check("empty without initial",
                errorText(function(){
                    new Uint8Array(0).reduce(function(){});
                })==="TypeError:empty array");
            check("reduce not constructor",
                errorText(function(){
                    new (Object.getPrototypeOf(Uint8Array.prototype).reduce)(
                        function(){}
                    );
                })==="TypeError:reduce is not a constructor");
            check("reduceRight not constructor",
                errorText(function(){
                    new (Object.getPrototypeOf(Uint8Array.prototype).reduceRight)(
                        function(){}
                    );
                })==="TypeError:reduceRight is not a constructor");

            var base=Object.getPrototypeOf(Uint8Array.prototype);
            check("QuickJS own-key order",
                Reflect.ownKeys(base).map(function(key){
                    return typeof key==="symbol" ? key.toString() : key;
                }).join("|")===
                "length|at|with|buffer|byteLength|byteOffset|set|values|keys|"+
                "entries|copyWithin|every|some|forEach|map|filter|reduce|"+
                "reduceRight|fill|find|findIndex|findLast|findLastIndex|"+
                "reverse|toReversed|slice|subarray|join|toLocaleString|"+
                "indexOf|lastIndexOf|includes|constructor|toString|"+
                "Symbol(Symbol.iterator)|Symbol(Symbol.toStringTag)");
            for(var name of ["reduce","reduceRight"]){
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
fn reduce_family_keeps_snapshot_range_but_reads_each_value_live() {
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
            function errorText(operation){
                try{operation();return "return"}
                catch(error){return error.name+":"+error.message}
            }

            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            var tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            var seen=[];
            var result=tracking.reduce(function(accumulator,value,index){
                seen.push(printable(value)+":"+index);
                if(index===0) buffer.resize(1);
                return accumulator+":"+printable(value);
            },"start");
            check("shrink result",
                result==="start:1:undefined:undefined:undefined");
            check("shrink visits snapshot range",
                seen.join(",")==="1:0,undefined:1,undefined:2,undefined:3");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            seen=[];
            tracking.reduceRight(function(accumulator,value,index){
                seen.push(printable(value)+":"+index);
                if(index===3) buffer.resize(1);
                return accumulator;
            },0);
            check("reverse shrink visits snapshot range",
                seen.join(",")==="4:3,undefined:2,undefined:1,1:0");

            buffer=new ArrayBuffer(2,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            seen=[];
            tracking.reduce(function(accumulator,value,index){
                seen.push(printable(value)+":"+index);
                if(index===0){
                    buffer.resize(4);
                    tracking[2]=7;
                    tracking[3]=8;
                }
                return accumulator;
            },0);
            check("grow does not extend snapshot range",
                seen.join(",")==="1:0,2:1");

            tracking=new Uint8Array([1,2,3]);
            seen=[];
            tracking.reduce(function(accumulator,value,index,receiver){
                seen.push(value);
                if(index===0) receiver[1]=9;
                return accumulator;
            },0);
            check("later writes are live",seen.join(",")==="1,9,3");

            buffer=new ArrayBuffer(4);
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            seen=[];
            tracking.reduce(function(accumulator,value,index){
                seen.push(printable(value)+":"+index);
                if(index===0) buffer.transfer();
                return accumulator;
            },0);
            check("detach visits snapshot range",
                seen.join(",")==="1:0,undefined:1,undefined:2,undefined:3");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,0,4);
            fixed.set([1,2,3,4]);
            seen=[];
            fixed.reduce(function(accumulator,value,index){
                seen.push(printable(value));
                if(index===0) buffer.resize(2);
                if(index===1) buffer.resize(4);
                return accumulator;
            },0);
            check("fixed oob can regrow during callbacks",
                seen.join(",")==="1,undefined,0,0");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            fixed=new Uint8Array(buffer,0,4);
            buffer.resize(2);
            check("initial oob precedes callback validation",
                errorText(function(){
                    fixed.reduce(0);
                })==="TypeError:ArrayBuffer is detached or resized");

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
            tracking.reduce(function(accumulator,value,index){
                seen.push(printable(value));
                if(index===0) buffer.resize(1);
                return accumulator;
            },0);
            delete base["2"];
            check("missing integer skips prototype",prototypeHits===0);
            check("prototype did not replace undefined",
                seen.join(",")==="1,undefined,undefined,undefined");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn reduce_errors_and_accumulators_keep_their_pinned_realms_and_identities() {
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
    let caller_object_prototype = caller.object_prototype().unwrap();
    let reduce_object = eval_object(
        &mut defining,
        "Object.getPrototypeOf(Uint8Array.prototype).reduce",
        "defining reduce",
    );
    let reduce = runtime
        .as_callable(&reduce_object)
        .unwrap()
        .expect("defining reduce was not callable");
    let empty = eval_object(&mut caller, "new Uint8Array(0)", "caller empty TypedArray");

    assert!(matches!(
        caller.call(&reduce, Value::Object(empty.clone()), &[Value::Int(0)]),
        Err(RuntimeError::Exception),
    ));
    let invalid_callback =
        take_exception_object(&mut caller, "TypedArray reduce callback TypeError");
    assert_eq!(
        runtime.get_prototype_of(&invalid_callback).unwrap(),
        Some(defining_type_error.clone()),
        "TypedArray reduce callback TypeError did not use the method defining realm",
    );

    let identity_object = eval_object(
        &mut caller,
        "(function(accumulator){return accumulator})",
        "caller identity callback",
    );
    let identity = runtime
        .as_callable(&identity_object)
        .unwrap()
        .expect("caller identity callback was not callable");
    assert!(matches!(
        caller.call(
            &reduce,
            Value::Object(empty.clone()),
            &[Value::Object(identity.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let empty_error = take_exception_object(&mut caller, "TypedArray reduce empty TypeError");
    assert_eq!(
        runtime.get_prototype_of(&empty_error).unwrap(),
        Some(defining_type_error),
        "TypedArray reduce empty TypeError did not use the method defining realm",
    );

    let marker = eval_object(&mut caller, "Object()", "caller initial accumulator");
    assert_eq!(
        caller
            .call(
                &reduce,
                Value::Object(empty),
                &[
                    Value::Object(identity.as_object().clone()),
                    Value::Object(marker.clone()),
                ],
            )
            .expect("empty TypedArray reduce with explicit accumulator"),
        Value::Object(marker),
        "TypedArray reduce replaced an untouched caller-realm accumulator",
    );

    let throwing_object = eval_object(
        &mut caller,
        "(function(){throw new TypeError('caller callback')})",
        "caller throwing callback",
    );
    let throwing = runtime
        .as_callable(&throwing_object)
        .unwrap()
        .expect("caller throwing callback was not callable");
    let one = eval_object(&mut caller, "new Uint8Array([1])", "caller one TypedArray");
    assert!(matches!(
        caller.call(
            &reduce,
            Value::Object(one.clone()),
            &[Value::Object(throwing.as_object().clone()), Value::Int(0)],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "TypedArray reduce callback error");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "TypedArray reduce replaced a callback throw with a defining-realm error",
    );

    let producer_object = eval_object(
        &mut caller,
        "(function(){return Object()})",
        "caller object-producing callback",
    );
    let producer = runtime
        .as_callable(&producer_object)
        .unwrap()
        .expect("caller object-producing callback was not callable");
    let Value::Object(produced) = caller
        .call(
            &reduce,
            Value::Object(one),
            &[Value::Object(producer.as_object().clone()), Value::Int(0)],
        )
        .expect("TypedArray reduce callback-produced accumulator")
    else {
        panic!("TypedArray reduce callback did not return an Object accumulator");
    };
    assert_eq!(
        runtime.get_prototype_of(&produced).unwrap(),
        Some(caller_object_prototype),
        "TypedArray reduce moved a callback-produced accumulator into the defining realm",
    );
}
