use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, Runtime,
    RuntimeError, Value,
};

// These observations pin QuickJS 2026-06-04's shared
// `js_create_typed_array_iterator` / `js_array_iterator_next` path. Keep
// entries, keys, values, and @@iterator together: they differ only in the
// iterator kind selected after the same TypedArray validation.
struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

const CASES: &[Case] = &[
    Case {
        description: "brand and initial buffer-state validation",
        source: r#"(function(){
            function completion(thunk){
                try{return "return:"+String(thunk())}
                catch(error){return "throw:"+error.name+":"+error.message}
            }
            var base=Object.getPrototypeOf(Uint8Array.prototype);
            var rab=new ArrayBuffer(4,{maxByteLength:8});
            var oob=new Uint8Array(rab,0,4);
            rab.resize(2);
            var detachedBuffer=new ArrayBuffer(4);
            var detached=new Uint8Array(detachedBuffer);
            detachedBuffer.transfer();
            return [
                completion(function(){return base.entries.call({})}),
                completion(function(){
                    return base.keys.call(new Proxy(oob,{}));
                }),
                completion(function(){return base.entries.call(oob)}),
                completion(function(){return base.keys.call(oob)}),
                completion(function(){return base.values.call(oob)}),
                completion(function(){return base[Symbol.iterator].call(oob)}),
                completion(function(){return base.entries.call(detached)}),
                base[Symbol.iterator]===base.values,
                (function(){
                    var savedAlias=base[Symbol.iterator];
                    var originalValues=base.values;
                    base.values=function(){return "replacement"};
                    var unchangedAfterReplace=
                        base[Symbol.iterator]===savedAlias &&
                        savedAlias===originalValues;
                    delete base.values;
                    var result=savedAlias.call(
                        new Uint8Array([13])
                    ).next();
                    return unchangedAfterReplace+":"+
                        (base[Symbol.iterator]===savedAlias)+":"+
                        result.value+":"+result.done;
                })()
            ].join("|");
        })()"#,
        expected: "throw:TypeError:not a TypedArray|throw:TypeError:not a TypedArray|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|true|true:true:13:false",
    },
    Case {
        description: "live RAB growth shrink detach and terminal state",
        source: r#"(function(){
            function step(result){
                return result.done ? "done" : "value:"+String(result.value);
            }
            function entry(result){
                return result.done
                    ? "done"
                    : "entry:"+result.value[0]+","+String(result.value[1]);
            }
            function completion(thunk){
                try{return "return:"+step(thunk())}
                catch(error){return "throw:"+error.name+":"+error.message}
            }
            var out=[];

            var buffer=new ArrayBuffer(2,{maxByteLength:4});
            var tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            var entries=tracking.entries();
            out.push(entry(entries.next()));
            buffer.resize(4);
            tracking[2]=3;
            tracking[3]=4;
            out.push(
                entry(entries.next()),
                entry(entries.next()),
                entry(entries.next()),
                entry(entries.next())
            );

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            var keys=tracking.keys();
            out.push(step(keys.next()),step(keys.next()));
            buffer.resize(1);
            out.push(step(keys.next()));
            buffer.resize(6);
            out.push(step(keys.next()));

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,0,4);
            fixed.set([5,6,7,8]);
            entries=fixed.entries();
            out.push(entry(entries.next()));
            buffer.resize(2);
            out.push(completion(function(){return entries.next()}));
            buffer.resize(4);
            out.push(
                entry(entries.next()),
                entry(entries.next()),
                entry(entries.next()),
                entry(entries.next())
            );

            buffer=new ArrayBuffer(4);
            tracking=new Uint8Array(buffer);
            var values=tracking.values();
            out.push(step(values.next()));
            buffer.transfer();
            out.push(
                completion(function(){return values.next()}),
                completion(function(){return values.next()})
            );

            buffer=new ArrayBuffer(1,{maxByteLength:4});
            tracking=new Uint8Array(buffer);
            keys=tracking.keys();
            out.push(step(keys.next()),step(keys.next()));
            buffer.resize(4);
            out.push(step(keys.next()));
            buffer.transfer();
            out.push(step(keys.next()));

            buffer=new ArrayBuffer(5,{maxByteLength:9});
            var words=new Uint16Array(buffer);
            words[0]=257;
            words[1]=514;
            entries=words.entries();
            out.push("length:"+words.length,entry(entries.next()));
            buffer.resize(7);
            out.push(
                "length:"+words.length,
                entry(entries.next()),
                entry(entries.next()),
                entry(entries.next())
            );

            return out.join("|");
        })()"#,
        expected: "entry:0,1|entry:1,2|entry:2,3|entry:3,4|done|value:0|value:1|done|done|entry:0,5|throw:TypeError:ArrayBuffer is detached or resized|entry:1,6|entry:2,0|entry:3,0|done|value:0|throw:TypeError:ArrayBuffer is detached or resized|throw:TypeError:ArrayBuffer is detached or resized|value:0|done|done|done|length:2|entry:0,257|length:3|entry:1,514|entry:2,0|done",
    },
    Case {
        description: "all concrete TypedArray classes share the iterator kernel",
        source: r#"(function(){
            var constructors=[
                Int8Array,Uint8Array,Uint8ClampedArray,
                Int16Array,Uint16Array,Int32Array,Uint32Array,
                BigInt64Array,BigUint64Array,
                Float16Array,Float32Array,Float64Array
            ];
            var arrayIteratorPrototype=Object.getPrototypeOf([].values());
            return constructors.map(function(C){
                var bigint=C===BigInt64Array || C===BigUint64Array;
                var value=new C(bigint ? [7n,9n] : [7,9]);
                var pair=value.entries().next().value;
                var key=value.keys().next().value;
                var item=value.values().next().value;
                var aliasItem=value[Symbol.iterator]().next().value;
                return C.name+":"+pair[0]+","+String(pair[1])+":"+key+":"+
                    String(item)+":"+String(aliasItem)+":"+
                    (Object.getPrototypeOf(value.entries())===
                        arrayIteratorPrototype);
            }).join("|");
        })()"#,
        expected: "Int8Array:0,7:0:7:7:true|Uint8Array:0,7:0:7:7:true|Uint8ClampedArray:0,7:0:7:7:true|Int16Array:0,7:0:7:7:true|Uint16Array:0,7:0:7:7:true|Int32Array:0,7:0:7:7:true|Uint32Array:0,7:0:7:7:true|BigInt64Array:0,7:0:7:7:true|BigUint64Array:0,7:0:7:7:true|Float16Array:0,7:0:7:7:true|Float32Array:0,7:0:7:7:true|Float64Array:0,7:0:7:7:true",
    },
];

#[test]
fn typed_array_iterator_vectors_match_frozen_quickjs_observations() {
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
fn typed_array_iterator_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP TypedArray iterator oracle self-check: set QJS_ORACLE to pinned upstream qjs"
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
fn typed_array_iterators_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP TypedArray iterator differential: set QJS_ORACLE to pinned upstream qjs");
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
fn typed_array_entries_pair_and_errors_use_the_builtin_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let defining_array_iterator_prototype = eval_object(
        &mut defining,
        "Object.getPrototypeOf([].values())",
        "defining Array Iterator prototype",
    );
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let caller_array_prototype = caller.array_prototype().unwrap();
    let entries = eval_callable(
        &runtime,
        &mut defining,
        "Object.getPrototypeOf(Uint8Array.prototype).entries",
        "defining TypedArray entries",
    );

    let receiver = eval_object(
        &mut caller,
        "new Uint8Array([41,42])",
        "caller TypedArray receiver",
    );
    let Value::Object(iterator) = caller
        .call(&entries, Value::Object(receiver), &[])
        .expect("cross-realm TypedArray entries call")
    else {
        panic!("TypedArray entries did not return an iterator object");
    };
    assert_eq!(
        runtime.get_prototype_of(&iterator).unwrap(),
        Some(defining_array_iterator_prototype),
        "TypedArray entries allocated its iterator outside the builtin defining realm",
    );

    let next = property_callable(&runtime, &mut caller, &iterator, "next");
    let Value::Object(result) = caller
        .call(&next, Value::Object(iterator), &[])
        .expect("cross-realm TypedArray entries next")
    else {
        panic!("TypedArray entries next did not return an iterator result object");
    };
    let pair = object_property(&runtime, &mut caller, &result, "value");
    assert_eq!(
        runtime.get_prototype_of(&pair).unwrap(),
        Some(defining_array_prototype.clone()),
        "TypedArray entries pair did not use the builtin defining realm Array",
    );
    assert_ne!(
        runtime.get_prototype_of(&pair).unwrap(),
        Some(caller_array_prototype.clone()),
        "TypedArray entries pair leaked into the caller realm",
    );

    // QuickJS's JS_IteratorNext2 fast path calls the foreign native `next`
    // pointer with the outer operation context. A direct `.next()` above
    // therefore uses the builtin realm, while caller bytecode consuming the
    // same kind of iterator must allocate entries pairs in the caller realm.
    let receiver = eval_object(
        &mut caller,
        "new Uint8Array([43])",
        "caller TypedArray fast-path receiver",
    );
    let Value::Object(foreign_iterator) = caller
        .call(&entries, Value::Object(receiver), &[])
        .expect("cross-realm TypedArray entries fast-path call")
    else {
        panic!("TypedArray entries fast-path probe did not return an iterator object");
    };
    let foreign_key = runtime.intern_property_key("__foreignEntries").unwrap();
    assert!(
        caller
            .define_own_property(
                &caller.global_object().unwrap(),
                &foreign_key,
                &data_descriptor(Value::Object(foreign_iterator)),
            )
            .unwrap()
    );
    let spread = eval_object(
        &mut caller,
        "[...__foreignEntries]",
        "caller spread of foreign TypedArray entries",
    );
    let caller_pair = object_property(&runtime, &mut caller, &spread, "0");
    assert_eq!(
        runtime.get_prototype_of(&caller_pair).unwrap(),
        Some(caller_array_prototype),
        "foreign TypedArray entries fast path did not keep the outer operation realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&caller_pair).unwrap(),
        Some(defining_array_prototype),
        "foreign TypedArray entries fast path reused the next builtin realm",
    );

    let ordinary = eval_object(&mut caller, "({})", "caller ordinary receiver");
    assert!(matches!(
        caller.call(&entries, Value::Object(ordinary), &[]),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut caller, "cross-realm TypedArray brand TypeError");
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error),
        "TypedArray iterator brand TypeError did not use the builtin defining realm",
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

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(function) = context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read callable {name}: {error}"))
    else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&function)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn object_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> ObjectRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(value) = context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read object property {name}: {error}"))
    else {
        panic!("{name} was not an object");
    };
    value
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

fn data_descriptor(value: Value) -> OrdinaryPropertyDescriptor {
    OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    }
}
