use super::*;

#[test]
fn base_construct_uses_explicit_new_target_prototype_and_realm_fallback() {
    let runtime = Runtime::new();
    let mut constructor_context = runtime.new_context();
    let mut target_context = runtime.new_context();

    let Value::Object(constructor_object) = constructor_context
        .eval("(0, function(){ return 1; })")
        .unwrap()
    else {
        panic!("constructor source did not produce an object");
    };
    let constructor = runtime.as_callable(&constructor_object).unwrap().unwrap();
    let Value::Object(target_object) = target_context.eval("(0, function(){})").unwrap() else {
        panic!("new-target source did not produce an object");
    };
    let new_target = runtime.as_callable(&target_object).unwrap().unwrap();
    let prototype_key = runtime.intern_property_key("prototype").unwrap();

    let explicit_prototype = target_context.new_object().unwrap();
    assert!(
        target_context
            .define_own_property(
                &target_object,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Object(explicit_prototype.clone())),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(instance) = constructor_context
        .construct_with_new_target(&constructor, &new_target, &[])
        .unwrap()
    else {
        panic!("base constructor did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(explicit_prototype)
    );

    assert!(
        target_context
            .define_own_property(
                &target_object,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Null),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(fallback_instance) = constructor_context
        .construct_with_new_target(&constructor, &new_target, &[])
        .unwrap()
    else {
        panic!("base constructor fallback did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback_instance).unwrap(),
        Some(target_context.object_prototype().unwrap())
    );
    assert_ne!(
        runtime.get_prototype_of(&fallback_instance).unwrap(),
        Some(constructor_context.object_prototype().unwrap())
    );
}

#[test]
fn new_target_prototype_getter_throw_short_circuits_constructor_body() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::Object(constructor_object) = context
        .eval("(0, function(){ return function(){}; })")
        .unwrap()
    else {
        panic!("constructor source did not produce an object");
    };
    let constructor = runtime.as_callable(&constructor_object).unwrap().unwrap();
    let new_target = bytecode_callable(
        &runtime,
        &context,
        vec![Instruction::Undefined, Instruction::Return],
        FunctionMetadata {
            max_stack: 1,
            constructor_kind: ConstructorKind::Base,
            ..FunctionMetadata::default()
        },
    );
    let Value::Object(getter_object) = context.eval("(0, function(){ throw 9; })").unwrap() else {
        panic!("getter source did not produce an object");
    };
    let getter = runtime.as_callable(&getter_object).unwrap().unwrap();
    let prototype = runtime.intern_property_key("prototype").unwrap();
    assert!(
        context
            .define_own_property(
                new_target.as_object(),
                &prototype,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    assert_eq!(
        context.construct_with_new_target(&constructor, &new_target, &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
}

#[test]
fn construct_rejects_non_constructor_callable_with_caller_realm_type_error() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let callable = runtime.as_callable(&function_prototype).unwrap().unwrap();

    assert_eq!(
        context.construct(&callable, &[]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("construct failure did not materialize TypeError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static(" is not a constructor"))
    );
}
