use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("Atomics test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

#[test]
fn atomics_native_cproto_matches_the_pinned_function_table() {
    for operation in [
        AtomicsOperationKind::Add,
        AtomicsOperationKind::And,
        AtomicsOperationKind::Or,
        AtomicsOperationKind::Sub,
        AtomicsOperationKind::Xor,
        AtomicsOperationKind::Exchange,
        AtomicsOperationKind::CompareExchange,
        AtomicsOperationKind::Load,
    ] {
        let descriptor =
            NativeFunctionId::Atomics(AtomicsNativeKind::Operation(operation)).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::GenericMagic);
        assert!(!descriptor.cproto.default_is_constructor());
    }
    for kind in [
        AtomicsNativeKind::Store,
        AtomicsNativeKind::IsLockFree,
        AtomicsNativeKind::Pause,
        AtomicsNativeKind::Wait,
        AtomicsNativeKind::Notify,
    ] {
        let descriptor = NativeFunctionId::Atomics(kind).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::Generic);
        assert!(!descriptor.cproto.default_is_constructor());
    }
}

#[test]
fn global_atomics_is_lazy_realm_local_and_has_the_pinned_surface() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_global = first.global_object().unwrap();
    let second_global = second.global_object().unwrap();
    let key = runtime.intern_property_key("Atomics").unwrap();

    for (global, realm) in [(&first_global, first.realm), (&second_global, second.realm)] {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(global.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot),
            Some(PropertySlot::AutoInit(AutoInitProperty::Atomics {
                realm: defining_realm,
            })) if *defining_realm == realm
        ));
    }

    let Value::Object(first_atomics) = first.get_property(&first_global, &key).unwrap() else {
        panic!("first realm Atomics did not materialize to an object");
    };
    let Value::Object(second_atomics) = second.get_property(&second_global, &key).unwrap() else {
        panic!("second realm Atomics did not materialize to an object");
    };
    assert_ne!(first_atomics, second_atomics);
    assert_eq!(
        runtime.get_prototype_of(&first_atomics).unwrap(),
        Some(first.object_prototype().unwrap()),
    );
    assert_eq!(
        runtime.get_prototype_of(&second_atomics).unwrap(),
        Some(second.object_prototype().unwrap()),
    );
    let load_key = runtime.intern_property_key("load").unwrap();
    let Value::Object(first_load) = first.get_property(&first_atomics, &load_key).unwrap() else {
        panic!("first realm Atomics.load did not materialize to a function");
    };
    let Value::Object(second_load) = second.get_property(&second_atomics, &load_key).unwrap()
    else {
        panic!("second realm Atomics.load did not materialize to a function");
    };
    assert_ne!(first_load, second_load);
    assert_eq!(
        runtime.get_prototype_of(&first_load).unwrap(),
        Some(first.function_prototype().unwrap()),
    );
    assert_eq!(
        runtime.get_prototype_of(&second_load).unwrap(),
        Some(second.function_prototype().unwrap()),
    );

    assert_script(
        &mut first,
        r#"(function(){
            var failures=[];
            function check(label,condition){if(!condition)failures.push(label)}
            var names=[
                "add","and","or","sub","xor","exchange","compareExchange",
                "load","store","isLockFree","pause","wait","notify"
            ];
            var lengths=[3,3,3,3,3,3,4,2,3,1,0,4,3];
            var keys=Reflect.ownKeys(Atomics);
            check("key count",keys.length===14);
            for(var i=0;i<names.length;i++){
                check("key "+i,keys[i]===names[i]);
                var descriptor=Object.getOwnPropertyDescriptor(Atomics,names[i]);
                check(names[i]+" callable",typeof descriptor.value==="function");
                check(names[i]+" name",descriptor.value.name===names[i]);
                check(names[i]+" length",descriptor.value.length===lengths[i]);
                check(names[i]+" writable",descriptor.writable===true);
                check(names[i]+" enumerable",descriptor.enumerable===false);
                check(names[i]+" configurable",descriptor.configurable===true);
                try{Reflect.construct(descriptor.value,[]);failures.push(names[i]+" ctor")}
                catch(error){check(names[i]+" ctor error",error.name==="TypeError")}
            }
            check("symbol key",keys[13]===Symbol.toStringTag);
            var tag=Object.getOwnPropertyDescriptor(Atomics,Symbol.toStringTag);
            check("tag value",tag.value==="Atomics");
            check("tag writable",tag.writable===false);
            check("tag enumerable",tag.enumerable===false);
            check("tag configurable",tag.configurable===true);
            check("object tag",Object.prototype.toString.call(Atomics)==="[object Atomics]");
            var globalDescriptor=Object.getOwnPropertyDescriptor(globalThis,"Atomics");
            check("global value",globalDescriptor.value===Atomics);
            check("global writable",globalDescriptor.writable===true);
            check("global enumerable",globalDescriptor.enumerable===false);
            check("global configurable",globalDescriptor.configurable===true);
            var globals=Reflect.ownKeys(globalThis);
            check("bootstrap order",
                globals.indexOf("DataView")<globals.indexOf("Atomics") &&
                globals.indexOf("Atomics")<globals.indexOf("Promise"));
            return failures.length?failures.join(","):"ok";
        })()"#,
    );
}

#[test]
fn integer_typed_array_operations_return_old_values_and_wrap_in_place() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){if(!condition)failures.push(label)}
            var classes=[
                Int8Array,Uint8Array,Int16Array,Uint16Array,
                Int32Array,Uint32Array,BigInt64Array,BigUint64Array
            ];
            for(var i=0;i<classes.length;i++){
                var C=classes[i], big=i>=6, a=new C(1);
                function value(n){return big?BigInt(n):n}
                a[0]=value(5);
                check(C.name+" load",Atomics.load(a,0)===value(5));
                check(C.name+" add old",Atomics.add(a,0,value(3))===value(5));
                check(C.name+" add new",a[0]===value(8));
                check(C.name+" and old",Atomics.and(a,0,value(6))===value(8));
                check(C.name+" and new",a[0]===value(0));
                check(C.name+" or old",Atomics.or(a,0,value(10))===value(0));
                check(C.name+" or new",a[0]===value(10));
                check(C.name+" xor old",Atomics.xor(a,0,value(3))===value(10));
                check(C.name+" xor new",a[0]===value(9));
                check(C.name+" sub old",Atomics.sub(a,0,value(4))===value(9));
                check(C.name+" sub new",a[0]===value(5));
                check(C.name+" exchange old",Atomics.exchange(a,0,value(7))===value(5));
                check(C.name+" exchange new",a[0]===value(7));
                check(C.name+" compare miss",
                    Atomics.compareExchange(a,0,value(6),value(11))===value(7));
                check(C.name+" compare miss value",a[0]===value(7));
                check(C.name+" compare hit",
                    Atomics.compareExchange(a,0,value(7),value(11))===value(7));
                check(C.name+" compare hit value",a[0]===value(11));
            }

            var int8=new Int8Array([127]);
            check("int8 wrap old",Atomics.add(int8,0,2)===127);
            check("int8 wrap new",int8[0]===-127);
            var uint8=new Uint8Array([255]);
            check("uint8 wrap old",Atomics.add(uint8,0,2)===255);
            check("uint8 wrap new",uint8[0]===1);
            var uint32=new Uint32Array([4294967295]);
            check("uint32 old",Atomics.add(uint32,0,2)===4294967295);
            check("uint32 wrap",uint32[0]===1);
            var bigint64=new BigInt64Array([9223372036854775807n]);
            check("bigint64 old",Atomics.add(bigint64,0,2n)===9223372036854775807n);
            check("bigint64 wrap",bigint64[0]===-9223372036854775807n);
            var biguint64=new BigUint64Array([18446744073709551615n]);
            check("biguint64 old",Atomics.add(biguint64,0,2n)===18446744073709551615n);
            check("biguint64 wrap",biguint64[0]===1n);
            return failures.length?failures.join(","):"ok";
        })()"#,
    );
}

#[test]
fn store_returns_the_full_converted_value_while_writing_narrow_bits() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){if(!condition)failures.push(label)}
            var a=new Int8Array(1);
            var result=Atomics.store(a,0,257.9);
            check("fraction return",result===257);
            check("fraction write",a[0]===1);
            result=Atomics.store(a,0,NaN);
            check("NaN return",result===0 && 1/result===Infinity);
            check("NaN write",a[0]===0);
            result=Atomics.store(a,0,-0);
            check("negative zero return",result===0 && 1/result===Infinity);
            result=Atomics.store(a,0,Infinity);
            check("infinity return",result===Infinity);
            check("infinity write",a[0]===0);
            var calls=0;
            result=Atomics.store(a,0,{valueOf:function(){calls++;return 513.2}});
            check("object once",calls===1);
            check("object return",result===513);
            check("object write",a[0]===1);

            var signed=new BigInt64Array(1);
            var huge=18446744073709551617n;
            result=Atomics.store(signed,0,huge);
            check("bigint full return",result===huge);
            check("bigint signed write",signed[0]===1n);
            var unsigned=new BigUint64Array(1);
            result=Atomics.store(unsigned,0,-1n);
            check("biguint return",result===-1n);
            check("biguint write",unsigned[0]===18446744073709551615n);
            return failures.length?failures.join(","):"ok";
        })()"#,
    );
}

#[test]
fn access_and_operand_coercions_revalidate_at_the_quickjs_boundaries() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){if(!condition)failures.push(label)}
            function capture(operation){
                try{operation();return "return"}
                catch(error){return error.name+":"+error.message}
            }

            var buffer=new ArrayBuffer(4), array=new Int32Array(buffer), log=[];
            buffer.transfer();
            var outcome=capture(function(){
                Atomics.add(array,{valueOf:function(){log.push("index");return 0}},1)
            });
            check("initial detach error",outcome==="TypeError:ArrayBuffer is detached");
            check("initial detach order",log.length===0);

            buffer=new ArrayBuffer(4); array=new Int32Array(buffer); log=[];
            outcome=capture(function(){
                Atomics.add(array,{valueOf:function(){
                    log.push("index");buffer.transfer();return 0
                }},{valueOf:function(){log.push("value");return 1}})
            });
            check("index detach error",
                outcome==="TypeError:ArrayBuffer is detached or resized");
            check("index detach order",log.join(",")==="index");

            buffer=new ArrayBuffer(4); array=new Int32Array(buffer); log=[];
            outcome=capture(function(){
                Atomics.add(array,0,{valueOf:function(){
                    log.push("value");buffer.transfer();return 1
                }})
            });
            check("value detach error",outcome==="TypeError:ArrayBuffer is detached");
            check("value detach order",log.join(",")==="value");

            buffer=new ArrayBuffer(4); array=new Int32Array(buffer); log=[];
            outcome=capture(function(){
                Atomics.compareExchange(
                    array,0,
                    {valueOf:function(){log.push("expected");buffer.transfer();return 0}},
                    {valueOf:function(){log.push("replacement");return 1}}
                )
            });
            check("compare detach error",outcome==="TypeError:ArrayBuffer is detached");
            check("compare conversion order",log.join(",")==="expected,replacement");

            buffer=new ArrayBuffer(8,{maxByteLength:8});
            array=new Int32Array(buffer,0,2); log=[];
            outcome=capture(function(){
                Atomics.load(array,{valueOf:function(){
                    log.push("index");buffer.resize(0);return 0
                }})
            });
            check("fixed RAB index error",
                outcome==="TypeError:ArrayBuffer is detached or resized");

            buffer=new ArrayBuffer(8,{maxByteLength:8});
            array=new Int32Array(buffer); log=[];
            outcome=capture(function(){
                Atomics.load(array,{valueOf:function(){buffer.resize(4);return 1}})
            });
            check("tracking RAB current bound",outcome==="RangeError:out-of-bound access");

            buffer=new ArrayBuffer(8,{maxByteLength:8});
            array=new Int32Array(buffer,0,2); log=[];
            outcome=capture(function(){
                Atomics.store(array,0,{valueOf:function(){buffer.resize(0);return 1}})
            });
            check("post-value RAB error",outcome==="TypeError:ArrayBuffer is detached");
            return failures.length?failures.join(","):"ok";
        })()"#,
    );
}

#[test]
fn wait_notify_pause_and_lock_free_keep_the_non_shared_quickjs_contract() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){if(!condition)failures.push(label)}
            function capture(operation){
                try{return "return:"+String(operation())}
                catch(error){return "throw:"+error.name+":"+error.message}
            }

            var buffer=new ArrayBuffer(4), array=new Int32Array(buffer), log=[];
            var outcome=capture(function(){return Atomics.wait(
                array,
                {valueOf:function(){log.push("index");return 0}},
                {valueOf:function(){log.push("value");return 0}},
                {valueOf:function(){log.push("timeout");return 0}}
            )});
            check("wait error",
                outcome==="throw:TypeError:not a SharedArrayBuffer TypedArray");
            check("wait order",log.length===0);

            log=[];
            outcome=capture(function(){return Atomics.notify(
                array,
                {valueOf:function(){log.push("index");buffer.transfer();return 0}},
                {valueOf:function(){log.push("count");return 1}}
            )});
            check("notify return",outcome==="return:0");
            check("notify order",log.join(",")==="index,count");

            log=[];
            outcome=capture(function(){return Atomics.pause({
                valueOf:function(){log.push("pause");return 1}
            })});
            check("pause error",outcome==="throw:TypeError:not an integral number");
            check("pause no coercion",log.length===0);
            check("pause undefined",Atomics.pause()===undefined);
            check("pause integer",Atomics.pause(2)===undefined);
            check("pause fraction",
                capture(function(){return Atomics.pause(1.5)})===
                    "throw:TypeError:not an integral number");

            log=[];
            check("lock free coercion",Atomics.isLockFree({
                valueOf:function(){log.push("size");return 8}
            })===true);
            check("lock free order",log.join(",")==="size");
            check("lock free matrix",
                [0,1,2,3,4,8,9,Infinity,NaN].map(Atomics.isLockFree).join(",")===
                "false,true,true,false,true,true,false,false,false");

            check("clamped rejection",
                capture(function(){return Atomics.load(new Uint8ClampedArray(1),0)})===
                    "throw:TypeError:integer TypedArray expected");
            check("float rejection",
                capture(function(){return Atomics.load(new Float32Array(1),0)})===
                    "throw:TypeError:integer TypedArray expected");
            check("notify class rejection",
                capture(function(){return Atomics.notify(new Uint32Array(1),0,1)})===
                    "throw:TypeError:integer TypedArray expected");
            return failures.length?failures.join(","):"ok";
        })()"#,
    );
}

#[test]
fn lazy_and_materialized_atomics_edges_are_released() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("Atomics").unwrap();
    let before = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(context.realm)
        .unwrap();
    assert!(runtime.delete_property(&global, &key).unwrap());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm)
            .unwrap(),
        before - 1,
    );

    let mut materialized = runtime.new_context();
    let materialized_realm = materialized.realm;
    materialized
        .eval("void Atomics.load; delete globalThis.Atomics")
        .unwrap();
    drop(materialized);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(materialized_realm)
            .is_err(),
        "deleted materialized Atomics graph retained its realm",
    );
}
