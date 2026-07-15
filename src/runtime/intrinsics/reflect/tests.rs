use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("Reflect test did not return a String");
    };
    value.to_string()
}

#[test]
fn reflect_native_cproto_matches_pinned_quickjs_table() {
    for kind in [
        ReflectKind::Apply,
        ReflectKind::Construct,
        ReflectKind::DeleteProperty,
        ReflectKind::Get,
        ReflectKind::Has,
        ReflectKind::OwnKeys,
        ReflectKind::Set,
        ReflectKind::SetPrototypeOf,
    ] {
        let descriptor = NativeFunctionId::Reflect(kind).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::Generic, "{kind:?}");
        assert!(!descriptor.cproto.default_is_constructor(), "{kind:?}");
    }
    for kind in [
        ReflectKind::DefineProperty,
        ReflectKind::GetOwnPropertyDescriptor,
        ReflectKind::GetPrototypeOf,
        ReflectKind::IsExtensible,
        ReflectKind::PreventExtensions,
    ] {
        let descriptor = NativeFunctionId::Reflect(kind).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::GenericMagic, "{kind:?}");
        assert!(!descriptor.cproto.default_is_constructor(), "{kind:?}");
    }
}

#[test]
fn global_reflect_is_realm_aware_lazy_and_complete() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_global = first.global_object().unwrap();
    let second_global = second.global_object().unwrap();
    let key = runtime.intern_property_key("Reflect").unwrap();

    for (global, realm) in [(&first_global, first.realm), (&second_global, second.realm)] {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(global.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot_index].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(AutoInitProperty::Reflect {
                realm: defining_realm,
            })) if *defining_realm == realm
        ));
    }

    let Value::Object(first_reflect) = first.get_property(&first_global, &key).unwrap() else {
        panic!("first realm Reflect did not materialize to an object");
    };
    let Value::Object(second_reflect) = second.get_property(&second_global, &key).unwrap() else {
        panic!("second realm Reflect did not materialize to an object");
    };
    assert_ne!(first_reflect, second_reflect);
    assert_eq!(
        runtime.get_prototype_of(&first_reflect).unwrap(),
        Some(first.object_prototype().unwrap()),
    );
    assert_eq!(
        runtime.get_prototype_of(&second_reflect).unwrap(),
        Some(second.object_prototype().unwrap()),
    );

    let expected = [
        (ReflectKind::Apply, "apply", 3),
        (ReflectKind::Construct, "construct", 2),
        (ReflectKind::DefineProperty, "defineProperty", 3),
        (ReflectKind::DeleteProperty, "deleteProperty", 2),
        (ReflectKind::Get, "get", 2),
        (
            ReflectKind::GetOwnPropertyDescriptor,
            "getOwnPropertyDescriptor",
            2,
        ),
        (ReflectKind::GetPrototypeOf, "getPrototypeOf", 1),
        (ReflectKind::Has, "has", 2),
        (ReflectKind::IsExtensible, "isExtensible", 1),
        (ReflectKind::OwnKeys, "ownKeys", 1),
        (ReflectKind::PreventExtensions, "preventExtensions", 1),
        (ReflectKind::Set, "set", 3),
        (ReflectKind::SetPrototypeOf, "setPrototypeOf", 2),
    ];
    for (kind, name, length) in expected {
        let method_key = runtime.intern_property_key(name).unwrap();
        let state = runtime.0.state.borrow();
        let object = state.heap.object(first_reflect.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(method_key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot_index].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::Reflect(target_kind),
                name: target_name,
                length: target_length,
                min_readable_args,
            })) if *realm == first.realm
                && *target_kind == kind
                && *target_name == name
                && *target_length == length
                && *min_readable_args == length
        ));
    }
}

#[test]
fn reflect_call_construct_and_argument_validation_follow_pinned_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        eval_string(
            &mut context,
            r#"(function(){
                function add(a,b){return this.base+a+b}
                function Box(value){this.value=value}
                var boxed=Reflect.construct(Box,[7]);
                return Reflect.apply(add,{base:1},[2,3])+"|"+boxed.value;
            })()"#,
        ),
        "6|7",
    );
    assert_eq!(
        eval_string(
            &mut context,
            r#"(function(){
                var log="";
                var list={};
                Object.defineProperty(list,"length",{
                    get:function(){log+="L";throw "args"}
                });
                try{Reflect.construct(1,list)}catch(error){return error+"|"+log}
                return "missing";
            })()"#,
        ),
        "args|L",
    );
    assert_eq!(
        eval_string(
            &mut context,
            r#"(function(){
                var log="";
                var list={};
                Object.defineProperty(list,"length",{
                    get:function(){log+="L";throw "args"}
                });
                try{Reflect.construct(function(){},list,1)}
                catch(error){return error.name+":"+error.message+"|"+log}
                return "missing";
            })()"#,
        ),
        "TypeError:not a constructor|",
    );
    assert_eq!(
        eval_string(
            &mut context,
            r#"(function(){
                try{Reflect.apply(function(){},null,null)}
                catch(error){return error.name+":"+error.message}
                return "missing";
            })()"#,
        ),
        "TypeError:not a object",
    );
}

#[test]
fn reflect_property_and_prototype_operations_return_booleans() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        eval_string(
            &mut context,
            r#"(function(){
                var proto={inherited:3};
                var target=Object.create(proto);
                var receiver={};
                var symbol=Symbol("s");
                var defined=Reflect.defineProperty(target,"fixed",{
                    value:1,writable:false,enumerable:true,configurable:false
                });
                var setFixed=Reflect.set(target,"fixed",2);
                var setReceiver=Reflect.set(target,"fresh",4,receiver);
                target[symbol]=5;
                var keys=Reflect.ownKeys(target);
                var descriptor=Reflect.getOwnPropertyDescriptor(target,"fixed");
                var prevented=Reflect.preventExtensions(target);
                var addAfter=Reflect.defineProperty(target,"late",{value:9});
                var deleteFixed=Reflect.deleteProperty(target,"fixed");
                var setProto=Reflect.setPrototypeOf(target,null);
                return defined+"|"+setFixed+"|"+setReceiver+"|"+receiver.fresh+"|"+
                    Reflect.get(target,"inherited")+"|"+Reflect.has(target,"inherited")+"|"+
                    descriptor.value+":"+descriptor.writable+"|"+keys.length+":"+
                    (keys[1]===symbol)+"|"+prevented+":"+Reflect.isExtensible(target)+"|"+
                    addAfter+"|"+deleteFixed+"|"+setProto;
            })()"#,
        ),
        "true|false|true|4|3|true|1:false|2:true|true:false|false|false|false",
    );
}

#[test]
fn reflect_callback_reentry_stack_overflow_is_catchable_and_recovers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        eval_string(
            &mut context,
            r#"(function(){
                var key={};
                key[Symbol.toPrimitive]=function(){return Reflect.get({},key)};
                var caught;
                try{Reflect.get({},key)}
                catch(error){caught=error.name+":"+error.message}
                return caught+"|"+Reflect.get({answer:42},"answer");
            })()"#,
        ),
        "InternalError:stack overflow|42",
    );
    assert_eq!(
        eval_string(
            &mut context,
            r#"(function(){
                var descriptor={};
                Object.defineProperty(descriptor,"enumerable",{
                    get:function(){
                        return Reflect.defineProperty({},"recursive",descriptor)
                    }
                });
                var caught;
                try{Reflect.defineProperty({},"recursive",descriptor)}
                catch(error){caught=error.name+":"+error.message}
                var recovered={};
                var defined=Reflect.defineProperty(recovered,"answer",{value:42});
                return caught+"|"+defined+":"+recovered.answer;
            })()"#,
        ),
        "InternalError:stack overflow|true:42",
    );
}

#[test]
fn detached_reflect_method_retains_then_releases_its_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let defining_realm = defining.realm;
    let global = defining.global_object().unwrap();
    let reflect_key = runtime.intern_property_key("Reflect").unwrap();
    let Value::Object(reflect) = defining.get_property(&global, &reflect_key).unwrap() else {
        panic!("defining realm Reflect did not materialize to an object");
    };
    let get_key = runtime.intern_property_key("get").unwrap();
    let Value::Object(get_object) = defining.get_property(&reflect, &get_key).unwrap() else {
        panic!("Reflect.get did not materialize to an object");
    };
    let get = runtime
        .as_callable(&get_object)
        .unwrap()
        .expect("Reflect.get was not callable");

    drop(get_object);
    drop(reflect);
    drop(global);
    drop(defining);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .is_ok(),
        "detached Reflect.get did not retain its defining realm",
    );

    let mut caller = runtime.new_context();
    let target = caller.new_object().unwrap();
    let answer_key = runtime.intern_property_key("answer").unwrap();
    assert!(
        caller
            .set_property(&target, &answer_key, Value::Int(42))
            .unwrap()
    );
    assert_eq!(
        caller
            .call(
                &get,
                Value::Undefined,
                &[
                    Value::Object(target.clone()),
                    Value::String(JsString::from_static("answer")),
                ],
            )
            .unwrap(),
        Value::Int(42),
    );

    drop(target);
    drop(get);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .is_err(),
        "released Reflect.get left its defining realm alive",
    );
    assert_eq!(runtime.heap_counts().context_nodes, 1);
}

#[test]
fn deleting_lazy_global_reflect_releases_its_realm_edge() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("Reflect").unwrap();
    let count_before = runtime
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
        count_before - 1,
    );
}
