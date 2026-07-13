use super::*;

#[test]
fn reduced_group_by_element_limit_checks_before_next_and_preserves_throw() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let iterable = eval_object(
        &mut context,
        r#"(function(){
            globalThis.groupByNextCount=0;
            globalThis.groupByCallbackCount=0;
            globalThis.groupByReturnCount=0;
            var iterator=Object();
            iterator.next=function(){
                groupByNextCount++;
                var result=Object();
                result.done=false;
                result.value=groupByNextCount;
                return result;
            };
            iterator.return=function(){
                groupByReturnCount++;
                throw "close replacement";
            };
            var iterable=Object();
            iterable[Symbol.iterator]=function(){return iterator};
            return iterable;
        })()"#,
    );
    let callback = eval_object(
        &mut context,
        r#"(function(value,index){
            groupByCallbackCount++;
            return "group";
        })"#,
    );
    let arguments = NativeArguments {
        actual_arg_count: 2,
        readable: vec![Value::Object(iterable), Value::Object(callback)],
    };

    let completion = runtime
        .call_object_group_by_with_element_limit(
            context.realm,
            NativeInvocation::Call {
                this_value: Value::Undefined,
            },
            &arguments,
            2,
        )
        .unwrap();
    let Completion::Throw(Value::Object(error)) = completion else {
        panic!("reduced Object.groupBy limit did not throw an Error object");
    };
    assert_eq!(
        string_property(&runtime, &mut context, &error, "name"),
        "TypeError",
    );
    assert_eq!(
        string_property(&runtime, &mut context, &error, "message"),
        "too many elements",
    );
    assert_eq!(eval_int(&mut context, "groupByNextCount"), 2);
    assert_eq!(eval_int(&mut context, "groupByCallbackCount"), 2);
    assert_eq!(eval_int(&mut context, "groupByReturnCount"), 1);
}

#[test]
fn recursive_group_by_callback_ceiling_is_catchable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context
        .eval(
            r#"(function(){
                function recurse(depth){
                    return Object.groupBy([depth],function(){
                        if(depth!==0)recurse(depth-1);
                        return "group";
                    });
                }
                recurse(8);
                try{recurse(9);return "missing"}
                catch(error){return "ok|"+error.name+":"+error.message}
            })()"#,
        )
        .unwrap();
    assert_eq!(
        value,
        Value::String(JsString::from_static("ok|InternalError:stack overflow",)),
    );
}

#[test]
fn object_keys_family_autoinit_preserves_pinned_metadata() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_key = runtime.intern_property_key("Object").unwrap();
    let Value::Object(object_constructor) = context.get_property(&global, &object_key).unwrap()
    else {
        panic!("global Object was not an object");
    };

    for (name, kind) in [
        ("keys", ObjectKeysKind::Keys),
        ("values", ObjectKeysKind::Values),
        ("entries", ObjectKeysKind::Entries),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        let state = runtime.0.state.borrow();
        let object = state.heap.object(object_constructor.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot_index].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::ObjectKeys(target_kind),
                name: target_name,
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm && *target_kind == kind && *target_name == name
        ));
    }
}

#[test]
fn object_extensibility_autoinit_preserves_pinned_metadata() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_key = runtime.intern_property_key("Object").unwrap();
    let Value::Object(object_constructor) = context.get_property(&global, &object_key).unwrap()
    else {
        panic!("global Object was not an object");
    };

    for (name, kind) in [
        ("isExtensible", ObjectExtensibilityKind::IsExtensible),
        (
            "preventExtensions",
            ObjectExtensibilityKind::PreventExtensions,
        ),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        let state = runtime.0.state.borrow();
        let object = state.heap.object(object_constructor.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot_index].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::ObjectExtensibility(target_kind),
                name: target_name,
                length: 1,
                min_readable_args: 1,
            })) if *realm == context.realm && *target_kind == kind && *target_name == name
        ));
    }
}

#[test]
fn object_extensibility_preserves_primitives_and_updates_only_the_object_bit() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = context
        .eval(
            r#"(function(){
                var symbol=Symbol("marker");
                var primitiveChecks=[
                    Object.isExtensible(),Object.isExtensible(null),
                    Object.isExtensible(false),Object.isExtensible(1),
                    Object.isExtensible("x"),Object.isExtensible(1n),
                    Object.isExtensible(symbol),
                    Object.preventExtensions()===undefined,
                    Object.preventExtensions(null)===null,
                    Object.preventExtensions(false)===false,
                    Object.preventExtensions("x")==="x",
                    Object.preventExtensions(1n)===1n,
                    Object.preventExtensions(symbol)===symbol,
                    1/Object.preventExtensions(-0)===-Infinity,
                    Object.preventExtensions(NaN)!==Object.preventExtensions(NaN)
                ];
                var object=Object();
                object.existing=1;
                var prototype=Object.getPrototypeOf(object);
                var same=Object.preventExtensions(object)===object;
                var idempotent=Object.preventExtensions(object)===object;
                object.existing=2;
                var existing=object.existing;
                var rejected=false;
                try{object.added=3}catch(_){rejected=true}
                var absent=!("added" in object);
                var samePrototype=Object.setPrototypeOf(object,prototype)===object;
                var changedPrototypeThrows=false;
                try{Object.setPrototypeOf(object,Object())}
                catch(error){changedPrototypeThrows=error.name==="TypeError"}
                var deleted=delete object.existing;
                return primitiveChecks.join(",")+"|"+
                    [same,idempotent,Object.isExtensible(object),existing,rejected,absent,
                     samePrototype,changedPrototypeThrows,deleted,"existing" in object].join(",");
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        Value::String(JsString::from_static(
            "false,false,false,false,false,false,false,true,true,true,true,true,true,true,true|true,true,false,2,false,true,true,true,true,false",
        )),
    );
}

#[test]
fn object_descriptor_statics_autoinit_preserve_pinned_metadata() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_key = runtime.intern_property_key("Object").unwrap();
    let Value::Object(object_constructor) = context.get_property(&global, &object_key).unwrap()
    else {
        panic!("global Object was not an object");
    };

    for (name, target, length, min_readable_args) in [
        (
            "getOwnPropertyDescriptor",
            NativeFunctionId::ObjectGetOwnPropertyDescriptor,
            2,
            2,
        ),
        (
            "getOwnPropertyDescriptors",
            NativeFunctionId::ObjectGetOwnPropertyDescriptors,
            1,
            1,
        ),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        let state = runtime.0.state.borrow();
        let object = state.heap.object(object_constructor.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot_index].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: actual_target,
                name: target_name,
                length: actual_length,
                min_readable_args: actual_min_readable_args,
            })) if *realm == context.realm
                && *actual_target == target
                && *target_name == name
                && *actual_length == length
                && *actual_min_readable_args == min_readable_args
        ));
    }
}

#[test]
fn object_is_autoinit_and_same_value_semantics_match_pinned_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_key = runtime.intern_property_key("Object").unwrap();
    let Value::Object(object_constructor) = context.get_property(&global, &object_key).unwrap()
    else {
        panic!("global Object was not an object");
    };

    let key = runtime.intern_property_key("is").unwrap();
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(object_constructor.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot_index].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::ObjectIs,
                name: "is",
                length: 2,
                min_readable_args: 2,
            })) if *realm == context.realm
        ));
    }

    let result = context
        .eval(
            r#"(function(){
                var calls=0,probe=Object();
                probe.valueOf=function(){calls++;return 1};
                probe.toString=function(){calls++;return "1"};
                var object=Object(),other=Object(),symbol=Symbol("marker");
                return [
                    Object.is(),Object.is(undefined,undefined),Object.is(null,null),
                    Object.is(true,true),Object.is("x","x"),Object.is(1,1.0),
                    Object.is(1n,1n),Object.is(symbol,symbol),Object.is(object,object),
                    Object.is(NaN,NaN),Object.is(0,-0),Object.is(-0,-0),
                    Object.is(object,other),Object.is(1,"1"),
                    Object.is.call(probe,probe,probe),calls
                ].join(",");
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        Value::String(JsString::from_static(
            "true,true,true,true,true,true,true,true,true,true,false,true,false,false,true,0",
        )),
    );
}

#[test]
fn object_descriptor_statics_publish_complete_fields_without_calling_accessors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = context
        .eval(
            r#"(function(){
                var calls=0,target=Object();
                var dataDescriptor=Object();
                dataDescriptor.value=17;dataDescriptor.writable=false;
                dataDescriptor.enumerable=false;dataDescriptor.configurable=true;
                Object.defineProperty(target,"data",dataDescriptor);
                function getter(){calls++;return 23}
                var accessorDescriptor=Object();
                accessorDescriptor.get=getter;accessorDescriptor.set=undefined;
                accessorDescriptor.enumerable=true;accessorDescriptor.configurable=false;
                Object.defineProperty(target,"accessor",accessorDescriptor);
                var data=Object.getOwnPropertyDescriptor(target,"data");
                var accessor=Object.getOwnPropertyDescriptor(target,"accessor");
                var missing=Object.getOwnPropertyDescriptor(target,"missing");
                var all=Object.getOwnPropertyDescriptors(target);
                var valueField=Object.getOwnPropertyDescriptor(data,"value");
                return Object.keys(data).join(",")+"|"+
                    [data.value,data.writable,data.enumerable,data.configurable].join(",")+"|"+
                    Object.keys(accessor).join(",")+"|"+
                    [accessor.get===getter,accessor.set===undefined,
                     accessor.enumerable,accessor.configurable,calls].join(",")+"|"+
                    (missing===undefined)+"|"+Object.keys(all).join(",")+"|"+
                    [valueField.writable,valueField.enumerable,valueField.configurable].join(",");
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        Value::String(JsString::from_static(
            "value,writable,enumerable,configurable|17,false,false,true|get,set,enumerable,configurable|true,true,true,false,0|true|data,accessor|true,true,true",
        )),
    );
}

#[test]
fn recursive_object_descriptor_key_coercion_is_catchable_before_host_stack_exhaustion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"var descriptorTarget=Object();descriptorTarget.x=1;
               function objectDescriptorRecurse(depth){
                   var key=Object();
                   key[Symbol.toPrimitive]=function(hint){
                       if(hint!=="string")throw "bad hint";
                       if(depth!==0)objectDescriptorRecurse(depth-1);
                       return "x";
                   };
                   return Object.getOwnPropertyDescriptor(descriptorTarget,key).value;
               }
               function objectDescriptorMixedRecurse(depth){
                   var key=Object();
                   key[Symbol.toPrimitive]=function(){
                       if(depth!==0){
                           var holder=Object(),descriptor=Object();
                           descriptor.enumerable=true;
                           descriptor.get=function(){
                               return objectDescriptorMixedRecurse(depth-1);
                           };
                           Object.defineProperty(holder,"value",descriptor);
                           Object.values(holder);
                       }
                       return "x";
                   };
                   return Object.getOwnPropertyDescriptor(descriptorTarget,key).value;
               }
               function objectDescriptorGroupByRecurse(depth){
                   var key=Object();
                   key[Symbol.toPrimitive]=function(){
                       if(depth!==0)Object.groupBy([1],function(){
                           objectDescriptorGroupByRecurse(depth-1);
                           return "group";
                       });
                       return "x";
                   };
                   return Object.getOwnPropertyDescriptor(descriptorTarget,key).value;
               }
               function objectDescriptorDefineRecurse(depth){
                   var key=Object();
                   key[Symbol.toPrimitive]=function(){
                       if(depth!==0){
                           var holder=Object(),descriptor=Object();
                           descriptor.enumerable=true;
                           descriptor.__defineGetter__("value",function(){
                               return objectDescriptorDefineRecurse(depth-1);
                           });
                           Object.defineProperty(holder,"value",descriptor);
                       }
                       return "x";
                   };
                   return Object.getOwnPropertyDescriptor(descriptorTarget,key).value;
               }"#,
        )
        .unwrap();
    assert_eq!(
        context.eval("objectDescriptorRecurse(8)").unwrap(),
        Value::Int(1),
    );
    for depth in [9, 10, 11] {
        let value = context
            .eval(&format!(
                r#"(function(){{
                    try{{objectDescriptorRecurse({depth});return "missing"}}
                    catch(error){{return error.name+":"+error.message}}
                }})()"#,
            ))
            .unwrap();
        assert_eq!(
            value,
            Value::String(JsString::from_static("InternalError:stack overflow")),
        );
    }
    for name in [
        "objectDescriptorMixedRecurse",
        "objectDescriptorGroupByRecurse",
        "objectDescriptorDefineRecurse",
    ] {
        assert_eq!(context.eval(&format!("{name}(4)")).unwrap(), Value::Int(1),);
        for depth in [5, 6, 7] {
            let value = context
                .eval(&format!(
                    r#"(function(){{
                        try{{{name}({depth});return "missing"}}
                        catch(error){{return error.name+":"+error.message}}
                    }})()"#,
                ))
                .unwrap();
            assert_eq!(
                value,
                Value::String(JsString::from_static("InternalError:stack overflow")),
                "mixed native recursion path {name} at depth {depth}",
            );
        }
    }
    assert_eq!(context.eval("1+1").unwrap(), Value::Int(2));
}

#[test]
fn object_keys_descriptor_recheck_materializes_non_enumerable_autoinits() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let object_key = runtime.intern_property_key("Object").unwrap();
    let Value::Object(object_constructor) = context.get_property(&global, &object_key).unwrap()
    else {
        panic!("global Object was not an object");
    };

    for name in ["values", "entries"] {
        let key = runtime.intern_property_key(name).unwrap();
        let state = runtime.0.state.borrow();
        let object = state.heap.object(object_constructor.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(
                AutoInitProperty::NativeBuiltin { .. }
            ))
        ));
    }

    assert_eq!(
        context.eval("Object.keys(Object).length").unwrap(),
        Value::Int(0)
    );

    for (name, kind) in [
        ("values", ObjectKeysKind::Values),
        ("entries", ObjectKeysKind::Entries),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        let state = runtime.0.state.borrow();
        let object = state.heap.object(object_constructor.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        let Some(PropertySlot::Data(RawValue::Object(function))) = object.slots.get(slot_index)
        else {
            panic!("Object.{name} was not materialized during descriptor recheck");
        };
        let function = state.heap.object(*function).unwrap();
        assert!(matches!(
            &function.payload,
            ObjectPayload::NativeFunction { data }
                if data.target == NativeFunctionId::ObjectKeys(kind)
                    && data.realm == Some(context.realm)
                    && data.min_readable_args == 1
        ));
    }
}

#[test]
fn object_keys_family_filters_orders_and_boxes_string_code_units() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = context
        .eval(
            r#"(function(){
                var object=Object();
                object[2]="two";
                object[1]="one";
                object.beta="bee";
                var hidden=Object();
                hidden.value="hidden";
                hidden.enumerable=false;
                Object.defineProperty(object,"hidden",hidden);
                object[Symbol("symbol") ]="ignored";
                var keys=Object.keys(object);
                var values=Object.values(object);
                var entries=Object.entries(object);
                var stringKeys=Object.keys("A\uD800");
                var stringValues=Object.values("A\uD800");
                return keys.join(",")+"|"+values.join(",")+"|"+
                    Array.isArray(entries)+":"+Array.isArray(entries[0])+":"+
                    entries[0][0]+"="+entries[0][1]+","+
                    entries[1][0]+"="+entries[1][1]+","+
                    entries[2][0]+"="+entries[2][1]+"|"+
                    stringKeys.join(",")+":"+
                    stringValues[0].charCodeAt(0)+","+stringValues[1].charCodeAt(0);
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        Value::String(JsString::from_static(
            "1,2,beta|one,two,bee|true:true:1=one,2=two,beta=bee|0,1:65,55296",
        )),
    );
}

#[test]
fn object_values_and_entries_recheck_descriptors_before_get() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = context
        .eval(
            r#"(function(){
                var getterCalls=0;
                function make(){
                    var object=Object();
                    var first=Object();
                    first.enumerable=true;
                    first.configurable=true;
                    first.get=function(){
                        getterCalls++;
                        delete object.b;
                        var changed=Object();
                        changed.value="changed";
                        changed.enumerable=false;
                        changed.configurable=true;
                        Object.defineProperty(object,"c",changed);
                        object.d="late";
                        return "A";
                    };
                    Object.defineProperty(object,"a",first);
                    object.b="B";
                    object.c="C";
                    return object;
                }
                var keys=Object.keys(make()).join(",");
                var callsAfterKeys=getterCalls;
                var values=Object.values(make()).join(",");
                var entries=Object.entries(make());
                var marker=Object();
                var throwing=Object();
                var descriptor=Object();
                descriptor.enumerable=true;
                descriptor.get=function(){throw marker};
                Object.defineProperty(throwing,"x",descriptor);
                var keysSkippedGetter=Object.keys(throwing).join(",")==="x";
                var throwPreserved=false;
                try{Object.values(throwing)}catch(error){throwPreserved=error===marker}
                return keys+"|"+callsAfterKeys+"|"+values+"|"+
                    entries.length+":"+entries[0][0]+"="+entries[0][1]+"|"+
                    getterCalls+"|"+keysSkippedGetter+":"+throwPreserved;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        Value::String(JsString::from_static("a,b,c|0|A|1:a=A|2|true:true")),
    );
}

#[test]
fn borrowed_object_entries_uses_its_defining_realm_for_arrays_and_errors() {
    let runtime = Runtime::new();
    let mut defining_context = runtime.new_context();
    let method = eval_object(&mut defining_context, "Object.entries");
    let method = runtime.as_callable(&method).unwrap().unwrap();
    let defining_array_prototype = defining_context.array_prototype().unwrap();
    let defining_type_error_prototype = eval_object(&mut defining_context, "TypeError.prototype");
    let mut caller_context = runtime.new_context();

    let completion = runtime
        .call_internal(
            caller_context.realm,
            &method,
            Value::Undefined,
            &[Value::String(JsString::from_static("x"))],
        )
        .unwrap();
    let Completion::Return(Value::Object(result)) = completion else {
        panic!("borrowed Object.entries did not return an Array");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype.clone()),
    );
    let zero = runtime.intern_property_key("0").unwrap();
    let Value::Object(entry) = caller_context.get_property(&result, &zero).unwrap() else {
        panic!("borrowed Object.entries result did not contain an entry pair");
    };
    assert_eq!(
        runtime.get_prototype_of(&entry).unwrap(),
        Some(defining_array_prototype),
    );

    let completion = runtime
        .call_internal(
            caller_context.realm,
            &method,
            Value::Undefined,
            &[Value::Undefined],
        )
        .unwrap();
    let Completion::Throw(Value::Object(error)) = completion else {
        panic!("borrowed Object.entries nullish conversion did not throw");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error_prototype),
    );
    assert_eq!(
        string_property(&runtime, &mut caller_context, &error, "message"),
        "cannot convert to object",
    );
}

#[test]
fn recursive_object_keys_family_ceiling_protects_the_heaviest_measured_path() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"function objectKeysHeavyRecurse(depth){
                    var object=Object();
                    var descriptor=Object();
                    descriptor.enumerable=true;
                    descriptor.get=function(){
                        if(depth===0)return 0;
                        return objectKeysHeavyRecurse(depth-1);
                    };
                    Object.defineProperty(object,"value",descriptor);
                    return Object.values(object)[0];
                }
                function objectKeysDirectRecurse(reentries){
                    var remaining=reentries;
                    var object=Object();
                    var descriptor=Object();
                    descriptor.enumerable=true;
                    descriptor.get=function(){
                        if(remaining===0)return 0;
                        remaining--;
                        if(remaining%2===0)return Object.values(object)[0];
                        return Object.entries(object)[0][1];
                    };
                    Object.defineProperty(object,"value",descriptor);
                    return Object.values(object)[0];
                }"#,
        )
        .unwrap();

    assert_eq!(
        context.eval("objectKeysHeavyRecurse(8)").unwrap(),
        Value::Int(0)
    );
    for depth in [9, 10, 11] {
        let value = context
            .eval(&format!(
                r#"(function(){{
                    try{{objectKeysHeavyRecurse({depth});return "missing"}}
                    catch(error){{return error.name+":"+error.message}}
                }})()"#,
            ))
            .unwrap();
        assert_eq!(
            value,
            Value::String(JsString::from_static("InternalError:stack overflow")),
        );
    }

    let value = context
        .eval(
            r#"(function(){
                try{objectKeysDirectRecurse(80);return "missing"}
                catch(error){return error.name+":"+error.message}
            })()"#,
        )
        .unwrap();
    assert_eq!(
        value,
        Value::String(JsString::from_static("InternalError:stack overflow")),
    );
}

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    object
}

fn eval_int(context: &mut Context, source: &str) -> i32 {
    let Value::Int(value) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an Int");
    };
    value
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not a String property");
    };
    value.to_utf8_lossy()
}
