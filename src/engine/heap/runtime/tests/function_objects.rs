use super::*;

#[test]
fn ordinary_function_object_properties_match_quickjs_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(function) = context.eval("(function(a, b) {})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert!(runtime.is_constructor(&function).unwrap());

    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let constructor = runtime.intern_property_key("constructor").unwrap();

    assert_eq!(
        runtime.own_property_keys(&function).unwrap(),
        vec![length.clone(), name.clone(), prototype_key.clone()]
    );

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::String(name_value),
        writable: false,
        enumerable: false,
        configurable: true,
    } = runtime.get_own_property(&function, &name).unwrap().unwrap()
    else {
        panic!("unexpected function name descriptor");
    };
    assert!(name_value.is_empty());
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Int(2),
        writable: false,
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(&function, &length)
        .unwrap()
        .unwrap()
    else {
        panic!("unexpected function length descriptor");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(prototype),
        writable: true,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(&function, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("unexpected function prototype descriptor");
    };
    assert_eq!(
        context.get_property(&prototype, &constructor).unwrap(),
        Value::Object(function.clone())
    );
    assert_eq!(
        runtime.get_prototype_of(&prototype).unwrap().unwrap(),
        context.object_prototype().unwrap()
    );

    drop(prototype);
    drop(function);
    assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 2);
}

#[test]
fn function_prototype_autoinit_preserves_keys_without_eager_object_cycle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let Value::Object(unread) = context.eval("(0, function(){})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);
    drop(unread);
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);

    let Value::Object(function) = context.eval("(0, function(){})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);

    let length = runtime.intern_property_key("length").unwrap();
    let name = runtime.intern_property_key("name").unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    assert_eq!(
        runtime.own_property_keys(&function).unwrap(),
        vec![length, name, prototype_key.clone()]
    );
    assert!(runtime.has_own_property(&function, &prototype_key).unwrap());
    assert!(!runtime.delete_property(&function, &prototype_key).unwrap());
    assert!(
        !runtime
            .define_own_property(
                &function,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(prototype),
        writable: true,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(&function, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("prototype autoinit produced the wrong descriptor");
    };
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(second),
        ..
    } = runtime
        .get_own_property(&function, &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("second prototype read did not return an object");
    };
    assert_eq!(prototype, second);

    drop(second);
    drop(prototype);
    drop(function);
    assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 2);
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
}

#[test]
fn compatible_define_materializes_function_prototype_but_value_override_releases_it() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let prototype_key = runtime.intern_property_key("prototype").unwrap();

    let Value::Object(empty_define) = context.eval("(0, function(){})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert!(
        runtime
            .define_own_property(
                &empty_define,
                &prototype_key,
                &OrdinaryPropertyDescriptor::new(),
            )
            .unwrap()
    );
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
    drop(empty_define);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);

    let Value::Object(value_define) = context.eval("(0, function(){})").unwrap() else {
        panic!("function expression did not produce an object");
    };
    assert!(
        runtime
            .define_own_property(
                &value_define,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(1)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);
    assert!(matches!(
        runtime
            .get_own_property(&value_define, &prototype_key)
            .unwrap()
            .unwrap(),
        CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(1),
            writable: true,
            enumerable: false,
            configurable: false,
        }
    ));
}

#[test]
fn autoinit_define_checks_lazy_flags_before_materializing_and_retries() {
    let runtime = Runtime::new();
    let call_key = runtime.intern_property_key("call").unwrap();

    let configurable_context = runtime.new_context();
    let configurable_fp = configurable_context.function_prototype().unwrap();
    assert!(
        runtime
            .is_auto_init_own_property(&configurable_fp, &call_key)
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &configurable_fp,
                &call_key,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(
        !runtime
            .is_auto_init_own_property(&configurable_fp, &call_key)
            .unwrap()
    );
    assert!(matches!(
        runtime
            .get_own_property(&configurable_fp, &call_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            writable: true,
            enumerable: false,
            configurable: true,
            ..
        })
    ));

    let enumerable_context = runtime.new_context();
    let enumerable_fp = enumerable_context.function_prototype().unwrap();
    assert!(
        runtime
            .define_own_property(
                &enumerable_fp,
                &call_key,
                &OrdinaryPropertyDescriptor {
                    enumerable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        runtime.get_own_property(&enumerable_fp, &call_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            writable: true,
            enumerable: true,
            configurable: true,
            ..
        })
    ));

    let mut accessor_context = runtime.new_context();
    let accessor_fp = accessor_context.function_prototype().unwrap();
    let Value::Object(getter) = accessor_context
        .eval("(function replacementCall(){ return 7; })")
        .unwrap()
    else {
        panic!("replacement getter was not an object");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    assert!(
        runtime
            .define_own_property(
                &accessor_fp,
                &call_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter.clone())),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        runtime.get_own_property(&accessor_fp, &call_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Accessor {
            get: Some(ref actual),
            set: None,
            enumerable: false,
            configurable: true,
        }) if actual == &getter
    ));
    assert_eq!(
        accessor_context
            .get_property(&accessor_fp, &call_key)
            .unwrap(),
        Value::Int(7)
    );

    let has_instance_context = runtime.new_context();
    let has_instance_fp = has_instance_context.function_prototype().unwrap();
    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    assert!(
        runtime
            .is_auto_init_own_property(&has_instance_fp, &has_instance_key)
            .unwrap()
    );
    assert!(
        !runtime
            .define_own_property(
                &has_instance_fp,
                &has_instance_key,
                &OrdinaryPropertyDescriptor {
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(
        runtime
            .is_auto_init_own_property(&has_instance_fp, &has_instance_key)
            .unwrap()
    );
}

#[test]
fn failed_autoinit_commits_undefined_and_releases_initializer_realm() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = context.new_object().unwrap();
    let key = runtime.intern_property_key("failureProbe").unwrap();
    let before = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(context.realm)
        .unwrap();
    runtime
        .define_failure_auto_init(&object, context.realm, "failureProbe")
        .unwrap();
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(before + 1)
    );

    assert!(matches!(
        runtime.get_own_property(&object, &key),
        Err(RuntimeError::Invariant("autoinit failure probe"))
    ));
    assert!(!runtime.is_auto_init_own_property(&object, &key).unwrap());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(before)
    );
    assert!(matches!(
        runtime.get_own_property(&object, &key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));
}

#[test]
fn function_prototype_autoinit_owns_and_uses_closure_creation_realm() {
    let runtime = Runtime::new();
    let compiler_context = runtime.new_context();
    let creation_context = runtime.new_context();
    let creation_realm = creation_context.realm;
    let function = runtime
        .publish_unlinked_function(
            compiler_context.realm,
            UnlinkedFunction::fixture(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    has_prototype: true,
                    constructor_kind: ConstructorKind::Base,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let callable = runtime
        .new_bytecode_closure(creation_realm, &function)
        .unwrap();
    drop(creation_context);
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(creation_realm)
            .is_ok()
    );

    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(prototype),
        ..
    } = runtime
        .get_own_property(callable.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("prototype autoinit did not materialize an object");
    };
    let creation_object_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(creation_realm)
        .unwrap()
        .object_prototype;
    assert_eq!(
        runtime
            .get_prototype_of(&prototype)
            .unwrap()
            .unwrap()
            .object_id(),
        creation_object_prototype
    );

    drop(prototype);
    drop(callable);
    drop(function);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(creation_realm)
            .is_err()
    );
}

#[test]
fn function_prototype_is_callable_non_constructable_and_has_no_prototype_property() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let callable = runtime
        .callable_from_value(Value::Object(function_prototype.clone()))
        .unwrap();
    assert_eq!(
        context
            .call(&callable, Value::Undefined, &[Value::Int(1)])
            .unwrap(),
        Value::Undefined
    );
    assert!(!runtime.is_constructor(&function_prototype).unwrap());
    assert_eq!(
        runtime
            .get_prototype_of(&function_prototype)
            .unwrap()
            .unwrap(),
        context.object_prototype().unwrap()
    );

    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    let caller = runtime.intern_property_key("caller").unwrap();
    let arguments = runtime.intern_property_key("arguments").unwrap();
    let call = runtime.intern_property_key("call").unwrap();
    let apply = runtime.intern_property_key("apply").unwrap();
    let bind = runtime.intern_property_key("bind").unwrap();
    let to_string = runtime.intern_property_key("toString").unwrap();
    let file_name = runtime.intern_property_key("fileName").unwrap();
    let line_number = runtime.intern_property_key("lineNumber").unwrap();
    let column_number = runtime.intern_property_key("columnNumber").unwrap();
    let constructor = runtime.intern_property_key("constructor").unwrap();
    let has_instance = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert_eq!(
        runtime.own_property_keys(&function_prototype).unwrap(),
        vec![
            length.clone(),
            name.clone(),
            caller,
            arguments,
            call,
            apply,
            bind,
            to_string,
            file_name,
            line_number,
            column_number,
            constructor,
            has_instance,
        ]
    );
    assert!(matches!(
        runtime
            .get_own_property(&function_prototype, &name)
            .unwrap()
            .unwrap(),
        CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        } if value.is_empty()
    ));
    assert!(matches!(
        runtime
            .get_own_property(&function_prototype, &length)
            .unwrap()
            .unwrap(),
        CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(0),
            writable: false,
            enumerable: false,
            configurable: true,
        }
    ));
    assert_eq!(
        runtime
            .get_own_property(&function_prototype, &prototype)
            .unwrap(),
        None
    );
}

#[test]
fn strict_function_name_write_throws_a_type_error_from_the_defining_realm() {
    let runtime = Runtime::new();
    let mut defining_context = runtime.new_context();
    let mut caller_context = runtime.new_context();
    let defining_type_error = global_callable(&runtime, &mut defining_context, "TypeError");
    let caller_type_error = global_callable(&runtime, &mut caller_context, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(defining_type_error_prototype),
        ..
    } = runtime
        .get_own_property(defining_type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("defining-realm TypeError prototype was not an object");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(caller_type_error_prototype),
        ..
    } = runtime
        .get_own_property(caller_type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("caller-realm TypeError prototype was not an object");
    };
    assert_ne!(defining_type_error_prototype, caller_type_error_prototype);

    let Value::Object(function) = defining_context
        .eval("(0, function self(){ 'use strict'; self = 1; })")
        .unwrap()
    else {
        panic!("strict named function probe was not an object");
    };
    let function = runtime.as_callable(&function).unwrap().unwrap();
    assert_eq!(
        caller_context.call(&function, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(exception) = caller_context.take_exception().unwrap().unwrap() else {
        panic!("strict function-name write did not materialize an error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&exception).unwrap(),
        Some(defining_type_error_prototype)
    );
}
