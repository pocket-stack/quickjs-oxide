use super::*;

#[test]
fn context_invokes_getters_and_setters_with_the_original_receiver() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = runtime.new_object(None).unwrap();
    let child = runtime.new_object(Some(&prototype)).unwrap();
    let explicit_receiver = runtime.new_object(None).unwrap();

    let getter_key = runtime.intern_property_key("getter").unwrap();
    let getter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushThis, Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &getter_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert_eq!(
        context.get_property(&child, &getter_key).unwrap(),
        Value::Object(child.clone())
    );
    assert_eq!(
        context
            .get_property_with_receiver(
                &prototype,
                &getter_key,
                Value::Object(explicit_receiver.clone())
            )
            .unwrap(),
        Value::Object(explicit_receiver)
    );

    let setter_key = runtime.intern_property_key("setter").unwrap();
    let setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushFalse, Instruction::Return],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &setter_key,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert!(
        context
            .set_property(&child, &setter_key, Value::Int(7))
            .unwrap()
    );

    let throwing_key = runtime.intern_property_key("throwing-setter").unwrap();
    let throwing_setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::GetArg(0), Instruction::Throw],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &throwing_key,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(throwing_setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert_eq!(
        context.set_property(&child, &throwing_key, Value::Int(9)),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));

    let faulting_key = runtime.intern_property_key("faulting-setter").unwrap();
    let faulting_setter = bytecode_callable(
        &runtime,
        &context,
        vec![
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::Add,
            Instruction::Return,
        ],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &prototype,
                &faulting_key,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(faulting_setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );
    assert_eq!(
        context.set_property(&child, &faulting_key, Value::BigInt(JsBigInt::one())),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("expected setter TypeError");
    };
    assert!(runtime.is_error_object(&error).unwrap());
}

#[test]
fn prepared_getter_action_keeps_callable_alive_after_property_deletion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let key = runtime.intern_property_key("x").unwrap();
    let getter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::PushI32(42), Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &object,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );

    let action = runtime.prepare_get_property(&object, &key).unwrap();
    assert!(runtime.delete_property(&object, &key).unwrap());
    let PropertyGetAction::Call { getter, receiver } = action else {
        panic!("expected a rooted getter action");
    };
    assert_eq!(
        context.call(&getter, receiver, &[]).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context.get_property(&object, &key).unwrap(),
        Value::Undefined
    );
}

#[test]
fn prepared_setter_action_roots_callable_receiver_and_argument() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let argument = runtime.new_object(None).unwrap();
    let key = runtime.intern_property_key("x").unwrap();
    let setter = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::GetArg(0), Instruction::Return],
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    assert!(
        runtime
            .define_own_property(
                &object,
                &key,
                &OrdinaryPropertyDescriptor {
                    set: DescriptorField::Present(AccessorValue::Callable(setter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            )
            .unwrap()
    );

    let action = runtime
        .prepare_set_property(&object, &key, Value::Object(argument.clone()))
        .unwrap();
    assert!(runtime.delete_property(&object, &key).unwrap());
    drop(argument);
    let super::PropertySetAction::Call {
        setter,
        receiver,
        argument,
    } = action
    else {
        panic!("expected a rooted setter action");
    };
    let returned = context.call(&setter, receiver, &[argument]).unwrap();
    assert!(matches!(returned, Value::Object(_)));
    assert_eq!(
        context.get_property(&object, &key).unwrap(),
        Value::Undefined
    );
}
