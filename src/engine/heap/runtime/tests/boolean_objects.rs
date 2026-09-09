use super::*;

#[test]
fn boolean_wrappers_lookup_and_new_target_use_the_required_realms() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_constructor = global_callable(&runtime, &mut first, "Boolean");
    let second_constructor = global_callable(&runtime, &mut second, "Boolean");
    let first_prototype = first.boolean_prototype().unwrap();
    let second_prototype = second.boolean_prototype().unwrap();
    let first_object_prototype = first.object_prototype().unwrap();
    let first_object_value_of =
        property_callable(&runtime, &mut first, &first_object_prototype, "valueOf");
    let Value::Object(method_wrapper) = second
        .call(&first_object_value_of, Value::Bool(false), &[])
        .unwrap()
    else {
        panic!("cross-realm Object.prototype.valueOf did not box Boolean");
    };
    assert_eq!(
        runtime.get_prototype_of(&method_wrapper).unwrap(),
        Some(first_prototype.clone())
    );

    let Value::Object(cross_wrapper) = second
        .construct_with_new_target(
            &first_constructor,
            &second_constructor,
            &[Value::Bool(true)],
        )
        .unwrap()
    else {
        panic!("cross-realm Boolean construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&cross_wrapper).unwrap(),
        Some(second_prototype.clone())
    );
    let second_value_of = property_callable(&runtime, &mut second, &second_prototype, "valueOf");
    assert_eq!(
        first
            .call(&second_value_of, Value::Object(cross_wrapper.clone()), &[],)
            .unwrap(),
        Value::Bool(true)
    );

    let marker = runtime.intern_property_key("realmMarker").unwrap();
    assert!(
        first
            .define_own_property(
                &first_prototype,
                &marker,
                &data_descriptor(Value::Int(1), true, false, true),
            )
            .unwrap()
    );
    assert!(
        second
            .define_own_property(
                &second_prototype,
                &marker,
                &data_descriptor(Value::Int(2), true, false, true),
            )
            .unwrap()
    );
    let callable =
        |runtime: &Runtime, context: &mut crate::engine::api::context::Context, source: &str| {
            let Value::Object(function) = context.eval(source).unwrap() else {
                panic!("realm lookup probe did not produce a function");
            };
            runtime.as_callable(&function).unwrap().unwrap()
        };
    let first_reader = callable(
        &runtime,
        &mut first,
        "(function(){ return true.realmMarker; })",
    );
    let second_reader = callable(
        &runtime,
        &mut second,
        "(function(){ return true.realmMarker; })",
    );
    assert_eq!(
        second.call(&first_reader, Value::Undefined, &[]).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        first.call(&second_reader, Value::Undefined, &[]).unwrap(),
        Value::Int(2)
    );

    let custom_prototype = second.new_object().unwrap();
    let new_target = runtime
        .new_bound_native_function(
            &second.function_prototype().unwrap(),
            second.realm,
            NativeFunctionId::ConstructorProbe,
            0,
        )
        .unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    assert!(
        second
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &data_descriptor(Value::Object(custom_prototype.clone()), true, false, true,),
            )
            .unwrap()
    );
    let Value::Object(custom_wrapper) = first
        .construct_with_new_target(&first_constructor, &new_target, &[Value::Bool(false)])
        .unwrap()
    else {
        panic!("custom newTarget did not produce a Boolean wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&custom_wrapper).unwrap(),
        Some(custom_prototype)
    );
    assert!(
        second
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(fallback_wrapper) = first
        .construct_with_new_target(&first_constructor, &new_target, &[Value::Bool(false)])
        .unwrap()
    else {
        panic!("fallback newTarget did not produce a Boolean wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback_wrapper).unwrap(),
        Some(second_prototype.clone())
    );
    let throwing_getter = bytecode_callable(
        &runtime,
        &second,
        vec![Instruction::PushI32(77), Instruction::Throw],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        second
            .define_own_property(
                new_target.as_object(),
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(throwing_getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        first.construct_with_new_target(&first_constructor, &new_target, &[Value::Bool(false)],),
        Err(RuntimeError::Exception)
    );
    assert_eq!(first.take_exception().unwrap(), Some(Value::Int(77)));

    let escaped_this = callable(&runtime, &mut first, "(function(){ return this; })");
    let Value::Object(boxed_this) = second.call(&escaped_this, Value::Bool(false), &[]).unwrap()
    else {
        panic!("sloppy Boolean this did not escape as a wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&boxed_this).unwrap(),
        Some(first_prototype)
    );
    let stable_this = callable(
        &runtime,
        &mut first,
        "(function(){ return this === this; })",
    );
    assert_eq!(
        second.call(&stable_this, Value::Bool(false), &[]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn boolean_primitive_accessors_writes_and_delete_preserve_raw_receiver_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.boolean_prototype().unwrap();
    let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
    let strict_getter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let sloppy_getter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    for (name, getter) in [
        ("strictReceiver", strict_getter),
        ("sloppyReceiver", sloppy_getter),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(
                    &prototype,
                    &key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
    }
    assert_eq!(
        context.eval("false.strictReceiver").unwrap(),
        Value::Bool(false)
    );
    let Value::Object(sloppy_receiver) = context.eval("false.sloppyReceiver").unwrap() else {
        panic!("sloppy primitive getter did not receive a Boolean wrapper");
    };
    assert_eq!(
        context
            .call(&value_of, Value::Object(sloppy_receiver), &[])
            .unwrap(),
        Value::Bool(false)
    );

    let strict_setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Throw],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let sloppy_setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Throw],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    for (name, setter) in [("strictSink", strict_setter), ("sloppySink", sloppy_setter)] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(
                    &prototype,
                    &key,
                    &OrdinaryPropertyDescriptor {
                        set: DescriptorField::Present(AccessorValue::Callable(setter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
    }
    assert_eq!(
        context.eval("false.strictSink = 7"),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Bool(false)));
    assert_eq!(
        context.eval("false.sloppySink = 7"),
        Err(RuntimeError::Exception)
    );
    let Some(Value::Object(sloppy_receiver)) = context.take_exception().unwrap() else {
        panic!("sloppy primitive setter did not receive a Boolean wrapper");
    };
    assert_eq!(
        context
            .call(&value_of, Value::Object(sloppy_receiver), &[])
            .unwrap(),
        Value::Bool(false)
    );

    let writable = runtime.intern_property_key("writablePrimitive").unwrap();
    let read_only = runtime.intern_property_key("readOnlyPrimitive").unwrap();
    assert!(
        context
            .define_own_property(
                &prototype,
                &writable,
                &data_descriptor(Value::Int(1), true, false, true),
            )
            .unwrap()
    );
    assert!(
        context
            .define_own_property(
                &prototype,
                &read_only,
                &data_descriptor(Value::Int(1), false, false, true),
            )
            .unwrap()
    );
    assert_eq!(
        context.eval("false.writablePrimitive = 7").unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        context.eval("false.writablePrimitive").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("'use strict'; false.writablePrimitive = 7"),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("not an object")
    );
    assert_eq!(
        context.eval("'use strict'; false.readOnlyPrimitive = 7"),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("'readOnlyPrimitive' is read-only")
    );
    assert_eq!(
        context.eval("delete false.writablePrimitive").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context.eval("false.writablePrimitive").unwrap(),
        Value::Int(1)
    );
}

#[test]
fn object_prototype_boolean_methods_box_only_the_quickjs_paths() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object_prototype = context.object_prototype().unwrap();
    let boolean_prototype = context.boolean_prototype().unwrap();
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let object_value_of = property_callable(&runtime, &mut context, &object_prototype, "valueOf");
    let object_to_locale_string =
        property_callable(&runtime, &mut context, &object_prototype, "toLocaleString");
    let boolean_value_of = property_callable(&runtime, &mut context, &boolean_prototype, "valueOf");

    assert_eq!(
        context
            .call(&object_to_string, Value::Bool(false), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object Boolean]"))
    );
    assert_eq!(
        context
            .call(&object_to_locale_string, Value::Bool(false), &[])
            .unwrap(),
        Value::String(JsString::from_static("false"))
    );
    let Value::Object(first_wrapper) = context
        .call(&object_value_of, Value::Bool(false), &[])
        .unwrap()
    else {
        panic!("Object.prototype.valueOf did not box Boolean primitive");
    };
    let Value::Object(second_wrapper) = context
        .call(&object_value_of, Value::Bool(false), &[])
        .unwrap()
    else {
        panic!("second Object.prototype.valueOf did not box Boolean primitive");
    };
    assert_ne!(first_wrapper, second_wrapper);
    assert_eq!(
        runtime.get_prototype_of(&first_wrapper).unwrap(),
        Some(boolean_prototype.clone())
    );
    assert_eq!(
        context
            .call(&boolean_value_of, Value::Object(first_wrapper), &[])
            .unwrap(),
        Value::Bool(false)
    );

    let tag_receiver = runtime.intern_property_key("tagReceiver").unwrap();
    assert!(
        context
            .define_own_property(
                &context.global_object().unwrap(),
                &tag_receiver,
                &data_descriptor(Value::Undefined, true, true, true),
            )
            .unwrap()
    );
    let Value::Object(tag_getter) = context
        .eval("(function(){ 'use strict'; tagReceiver = typeof this; return 'CustomBoolean'; })")
        .unwrap()
    else {
        panic!("@@toStringTag probe did not produce a function");
    };
    let tag_getter = runtime.as_callable(&tag_getter).unwrap().unwrap();
    let to_string_tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    assert!(
        context
            .define_own_property(
                &boolean_prototype,
                &to_string_tag,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(tag_getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&object_to_string, Value::Bool(false), &[])
            .unwrap(),
        Value::String(JsString::from_static("[object CustomBoolean]"))
    );
    assert_eq!(
        context
            .get_property(&context.global_object().unwrap(), &tag_receiver)
            .unwrap(),
        Value::String(JsString::from_static("object"))
    );

    let locale_receiver = runtime.intern_property_key("localeReceiver").unwrap();
    assert!(
        context
            .define_own_property(
                &context.global_object().unwrap(),
                &locale_receiver,
                &data_descriptor(Value::Undefined, true, true, true),
            )
            .unwrap()
    );
    let Value::Object(locale_method) = context
        .eval("(function(){ 'use strict'; return typeof this; })")
        .unwrap()
    else {
        panic!("toLocaleString method probe did not produce a function");
    };
    let locale_method = runtime.as_callable(&locale_method).unwrap().unwrap();
    let locale_method_key = runtime.intern_property_key("localeMethod").unwrap();
    assert!(
        context
            .define_own_property(
                &context.global_object().unwrap(),
                &locale_method_key,
                &data_descriptor(
                    Value::Object(locale_method.as_object().clone()),
                    true,
                    true,
                    true,
                ),
            )
            .unwrap()
    );
    let Value::Object(locale_getter) = context
        .eval("(function(){ 'use strict'; localeReceiver = typeof this; return localeMethod; })")
        .unwrap()
    else {
        panic!("toLocaleString getter probe did not produce a function");
    };
    let locale_getter = runtime.as_callable(&locale_getter).unwrap().unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    assert!(
        context
            .define_own_property(
                &boolean_prototype,
                &to_string_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(locale_getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&object_to_locale_string, Value::Bool(false), &[])
            .unwrap(),
        Value::String(JsString::from_static("boolean"))
    );
    assert_eq!(
        context
            .get_property(&context.global_object().unwrap(), &locale_receiver)
            .unwrap(),
        Value::String(JsString::from_static("boolean"))
    );
}

#[test]
fn boolean_wrapper_keeps_its_realm_graph_alive_until_collection() {
    let runtime = Runtime::new();
    let wrapper = {
        let mut context = runtime.new_context();
        let constructor = global_callable(&runtime, &mut context, "Boolean");
        let Value::Object(wrapper) = context
            .construct(&constructor, &[Value::Bool(true)])
            .unwrap()
        else {
            panic!("Boolean construction did not return a wrapper");
        };
        wrapper
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(wrapper);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}
