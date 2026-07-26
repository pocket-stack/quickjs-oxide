use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray stringification test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap_or_else(|error| {
        panic!("TypedArray stringification rejected {description}: {error}")
    }) else {
        panic!("TypedArray stringification {description} was not an Object");
    };
    object
}

#[test]
fn join_and_to_locale_string_publish_quickjs_surface() {
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
            for(var entry of [["join",1],["toLocaleString",0]]){
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
            }

            check("join default",new Uint8Array([1,2,3]).join()==="1,2,3");
            check("join undefined",
                new Uint8Array([1,2,3]).join(undefined)==="1,2,3");
            check("join separator",new Uint8Array([1,2,3]).join("|")==="1|2|3");
            check("join empty",new Uint8Array(0).join("|")==="");
            check("join bigint",new BigInt64Array([1n,-2n]).join(":")==="1:-2");
            var numberToString=Number.prototype.toString;
            var bigintToString=BigInt.prototype.toString;
            Number.prototype.toString=function(){throw new Error("number toString")};
            BigInt.prototype.toString=function(){throw new Error("bigint toString")};
            try{
                check("join ignores Number prototype toString",
                    new Uint8Array([1,2]).join("|")==="1|2");
                check("join ignores BigInt prototype toString",
                    new BigInt64Array([1n,2n]).join("|")==="1|2");
            }finally{
                Number.prototype.toString=numberToString;
                BigInt.prototype.toString=bigintToString;
            }

            var numberLocale=Number.prototype.toLocaleString;
            var bigintLocale=BigInt.prototype.toLocaleString;
            var log=[];
            Number.prototype.toLocaleString=function(){
                "use strict";
                log.push(typeof this+":"+this+":"+arguments.length);
                return this*10;
            };
            BigInt.prototype.toLocaleString=function(){
                "use strict";
                log.push(typeof this+":"+this+":"+arguments.length);
                return this*10n;
            };
            try{
                check("locale number",
                    new Uint8Array([1,2]).toLocaleString("ignored") === "10,20");
                check("locale bigint",
                    new BigInt64Array([3n]).toLocaleString("ignored") === "30");
                check("locale receiver and args",
                    log.join("|")==="number:1:0|number:2:0|bigint:3:0");
            }finally{
                Number.prototype.toLocaleString=numberLocale;
                BigInt.prototype.toLocaleString=bigintLocale;
            }

            check("toString alias",base.toString===Array.prototype.toString);
            check("toString result",new Uint8Array([4,5]).toString()==="4,5");
            var custom=new Uint8Array([4,5]);
            custom.join=function(){return "custom"};
            check("toString observes join",custom.toString()==="custom");

            check("filtered QuickJS own-key order",
                Reflect.ownKeys(base).map(function(key){
                    return typeof key==="symbol" ? key.toString() : key;
                }).join("|")===
                "length|at|with|buffer|byteLength|byteOffset|set|values|keys|"+
                "entries|copyWithin|every|some|forEach|map|filter|reduce|"+
                "reduceRight|fill|find|findIndex|findLast|findLastIndex|"+
                "reverse|toReversed|slice|subarray|sort|toSorted|join|toLocaleString|"+
                "indexOf|lastIndexOf|includes|constructor|toString|"+
                "Symbol(Symbol.iterator)|Symbol(Symbol.toStringTag)");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn stringification_validates_brand_and_initial_buffer_state_before_coercion() {
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
                try{return "return:"+operation()}
                catch(error){return error.name+":"+error.message}
            }
            var base=Object.getPrototypeOf(Uint8Array.prototype);

            check("join plain brand",
                completion(function(){return base.join.call({},"|")})===
                "TypeError:not a TypedArray");
            check("locale plain brand",
                completion(function(){return base.toLocaleString.call({})})===
                "TypeError:not a TypedArray");
            check("join proxy brand",
                completion(function(){
                    return base.join.call(new Proxy(new Uint8Array([1]),{}),"|");
                })==="TypeError:not a TypedArray");
            check("locale proxy brand",
                completion(function(){
                    return base.toLocaleString.call(
                        new Proxy(new Uint8Array([1]),{}));
                })==="TypeError:not a TypedArray");

            var log="";
            var separator={toString:function(){log+="S";return "|"}};
            var buffer=new ArrayBuffer(2);
            var detached=new Uint8Array(buffer);
            buffer.transfer();
            check("join detached",
                completion(function(){return base.join.call(detached,separator)})===
                "TypeError:ArrayBuffer is detached or resized");
            check("join detached skips separator",log==="");
            check("locale detached",
                completion(function(){return base.toLocaleString.call(detached)})===
                "TypeError:ArrayBuffer is detached or resized");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,1,2);
            buffer.resize(0);
            check("join fixed oob",
                completion(function(){return base.join.call(fixed,separator)})===
                "TypeError:ArrayBuffer is detached or resized");
            check("join fixed oob skips separator",log==="");
            check("locale fixed oob",
                completion(function(){return base.toLocaleString.call(fixed)})===
                "TypeError:ArrayBuffer is detached or resized");

            check("symbol separator",
                completion(function(){
                    return new Uint8Array([1]).join(Symbol("separator"));
                })==="TypeError:cannot convert symbol to string");
            check("invalid separator primitive",
                completion(function(){
                    return new Uint8Array([1]).join({
                        [Symbol.toPrimitive]:function(){return {}}
                    });
                })==="TypeError:toPrimitive");
            var thrown;
            try{
                new Uint8Array([1]).join({
                    toString:function(){throw 77}
                });
            }catch(error){thrown=error}
            check("separator throw",thrown===77);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn join_observes_quickjs_separator_resize_snapshot_contract() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }

            var log="";
            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            var tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            var separator={
                toString:function(){
                    log+="S";
                    buffer.resize(2);
                    return "|";
                }
            };
            check("tracking shrink",tracking.join(separator)==="1|2||");
            check("separator once",log==="S");
            check("tracking shrink live length",tracking.length===2);

            buffer=new ArrayBuffer(8,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,0,4);
            fixed.set([1,2,3,4]);
            separator={
                toString:function(){
                    buffer.resize(2);
                    return "|";
                }
            };
            check("fixed shrink oob",fixed.join(separator)==="|||");
            check("fixed shrink live length",fixed.length===0);

            buffer=new ArrayBuffer(3);
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3]);
            separator={
                toString:function(){
                    buffer.transfer();
                    return "|";
                }
            };
            check("detach",tracking.join(separator)==="||");
            check("detach live length",tracking.length===0);

            buffer=new ArrayBuffer(2,{maxByteLength:4});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            separator={
                toString:function(){
                    buffer.resize(4);
                    tracking[2]=3;
                    tracking[3]=4;
                    return "|";
                }
            };
            check("grow remains old length",tracking.join(separator)==="1|2");
            check("grow live length",tracking.length===4);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn to_locale_string_observes_live_elements_with_a_fixed_old_length() {
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
                try{return "return:"+operation()}
                catch(error){return error.name+":"+error.message}
            }

            var original=Number.prototype.toLocaleString;
            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            var tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            var calls=0;
            Number.prototype.toLocaleString=function(){
                "use strict";
                calls++;
                if(calls===1) buffer.resize(2);
                return this+10;
            };
            try{
                check("tracking shrink",
                    tracking.toLocaleString({toString:function(){throw 1}})===
                    "11,12,,");
                check("tracking shrink calls",calls===2);
            }finally{
                Number.prototype.toLocaleString=original;
            }

            buffer=new ArrayBuffer(3);
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3]);
            calls=0;
            Number.prototype.toLocaleString=function(){
                "use strict";
                calls++;
                if(calls===1) buffer.transfer();
                return this;
            };
            try{
                check("detach after first",tracking.toLocaleString()==="1,,");
                check("detach calls",calls===1);
            }finally{
                Number.prototype.toLocaleString=original;
            }

            buffer=new ArrayBuffer(2,{maxByteLength:4});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            calls=0;
            Number.prototype.toLocaleString=function(){
                "use strict";
                calls++;
                if(calls===1){
                    buffer.resize(4);
                    tracking[2]=3;
                    tracking[3]=4;
                }
                return this;
            };
            try{
                check("grow remains old length",tracking.toLocaleString()==="1,2");
                check("grow calls",calls===2);
            }finally{
                Number.prototype.toLocaleString=original;
            }

            Number.prototype.toLocaleString=1;
            try{
                check("non-callable",
                    completion(function(){
                        return new Uint8Array([1]).toLocaleString();
                    })==="TypeError:not a function");
            }finally{
                Number.prototype.toLocaleString=original;
            }

            Number.prototype.toLocaleString=function(){return Symbol("result")};
            try{
                check("symbol result",
                    completion(function(){
                        return new Uint8Array([1]).toLocaleString();
                    })==="TypeError:cannot convert symbol to string");
            }finally{
                Number.prototype.toLocaleString=original;
            }

            Number.prototype.toLocaleString=function(){
                "use strict";
                return this===1 ? undefined : null;
            };
            try{
                check("undefined and null results",
                    new Uint8Array([1,2]).toLocaleString()==="undefined,null");
            }finally{
                Number.prototype.toLocaleString=original;
            }

            var marker={};
            Number.prototype.toLocaleString=function(){throw marker};
            try{
                var caught;
                try{new Uint8Array([1]).toLocaleString()}
                catch(error){caught=error}
                check("user throw identity",caught===marker);
            }finally{
                Number.prototype.toLocaleString=original;
            }

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn to_locale_string_uses_the_method_defining_realm() {
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
    defining
        .eval(
            r#"Number.prototype.toLocaleString=function(){
                "use strict";
                return "defining-"+this;
            }"#,
        )
        .unwrap();
    caller
        .eval(
            r#"Number.prototype.toLocaleString=function(){
                "use strict";
                return "caller-"+this;
            }"#,
        )
        .unwrap();
    let method = runtime
        .as_callable(&eval_object(
            &mut defining,
            "Object.getPrototypeOf(Uint8Array.prototype).toLocaleString",
            "defining toLocaleString",
        ))
        .unwrap()
        .expect("defining toLocaleString was not callable");
    let join = runtime
        .as_callable(&eval_object(
            &mut defining,
            "Object.getPrototypeOf(Uint8Array.prototype).join",
            "defining join",
        ))
        .unwrap()
        .expect("defining join was not callable");
    let to_string = runtime
        .as_callable(&eval_object(
            &mut defining,
            "Object.getPrototypeOf(Uint8Array.prototype).toString",
            "defining toString",
        ))
        .unwrap()
        .expect("defining toString was not callable");
    let source = eval_object(&mut caller, "new Uint8Array([1,2])", "caller TypedArray");

    assert_eq!(
        caller
            .call(&method, Value::Object(source), &[Value::Int(99)])
            .unwrap(),
        Value::String(JsString::from_static("defining-1,defining-2")),
    );

    let plain = eval_object(&mut caller, "Object()", "caller plain object");
    assert!(matches!(
        caller.call(&join, Value::Object(plain), &[]),
        Err(RuntimeError::Exception),
    ));
    let Value::Object(brand_error) = caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm TypedArray join brand error was not an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&brand_error).unwrap(),
        Some(defining_type_error),
        "TypedArray join brand error did not use the method defining realm",
    );

    let throwing_separator = eval_object(
        &mut caller,
        "({toString:function(){throw new TypeError('caller separator')}})",
        "caller throwing separator",
    );
    let throw_source = eval_object(
        &mut caller,
        "new Uint8Array([1])",
        "caller separator source",
    );
    assert!(matches!(
        caller.call(
            &join,
            Value::Object(throw_source),
            &[Value::Object(throwing_separator)],
        ),
        Err(RuntimeError::Exception),
    ));
    let Value::Object(user_error) = caller.take_exception().unwrap().unwrap() else {
        panic!("cross-realm TypedArray join user error was not an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "TypedArray join replaced a caller throw with a defining-realm error",
    );

    let custom_source = eval_object(
        &mut caller,
        r#"(function(){
            var value=new Uint8Array([3,4]);
            value.join=function(){return "caller-join"};
            return value;
        })()"#,
        "caller custom-join source",
    );
    assert_eq!(
        caller
            .call(&to_string, Value::Object(custom_source), &[])
            .unwrap(),
        Value::String(JsString::from_static("caller-join")),
        "borrowed TypedArray toString did not resolve join from its receiver",
    );
}

#[test]
fn typed_array_separator_overflow_stops_before_the_next_locale_call() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        r#"(function(){
            globalThis.typedLocaleOverflowCalls=0;
            Number.prototype.toLocaleString=function(){
                typedLocaleOverflowCalls++;
                if(typedLocaleOverflowCalls!==1) throw 88;
                return "aa";
            };
            return new Uint8Array([1,2]);
        })()"#,
        "locale overflow fixture",
    );

    let error = runtime
        .call_typed_array_join_with_string_limit(
            context.realm,
            ArrayJoinKind::ToLocaleString,
            NativeInvocation::Call {
                this_value: Value::Object(source),
            },
            &NativeArguments {
                actual_arg_count: 0,
                readable: Vec::new(),
            },
            2,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::Engine(ref error)
            if error.kind() == ErrorKind::JsInternal
                && error.message() == "string too long"
    ));
    assert_eq!(
        context.eval("typedLocaleOverflowCalls").unwrap(),
        Value::Int(1),
    );
}
