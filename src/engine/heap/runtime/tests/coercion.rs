use super::*;

#[test]
fn object_to_primitive_string_drives_error_message_and_to_string_values() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error = global_callable(&runtime, &mut context, "Error");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let message_key = runtime.intern_property_key("message").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(error_prototype),
        ..
    } = runtime
        .get_own_property(error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error prototype was not an object");
    };
    let Value::Object(error_to_string_object) = context
        .get_property(&error_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Error.prototype.toString was not an object");
    };
    let error_to_string = runtime
        .as_callable(&error_to_string_object)
        .unwrap()
        .unwrap();

    let ordinary = context.new_object().unwrap();
    let Value::Object(ordinary_error) = context
        .call(&error, Value::Undefined, &[Value::Object(ordinary)])
        .unwrap()
    else {
        panic!("Error(object) did not return an object");
    };
    assert!(matches!(
        runtime
            .get_own_property(&ordinary_error, &message_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("[object Object]")
    ));

    let Value::Object(custom_to_string_object) =
        context.eval("(0, function(){ return 'custom'; })").unwrap()
    else {
        panic!("custom toString probe was not a function");
    };
    let custom = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &custom,
                &to_string_key,
                &data_descriptor(Value::Object(custom_to_string_object), true, true, true,),
            )
            .unwrap()
    );
    let Value::Object(custom_error) = context
        .call(&error, Value::Undefined, &[Value::Object(custom)])
        .unwrap()
    else {
        panic!("Error(custom object) did not return an object");
    };
    assert!(matches!(
        runtime.get_own_property(&custom_error, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("custom")
    ));

    let Value::Object(exotic_method_object) =
        context.eval("(0, function(hint){ return hint; })").unwrap()
    else {
        panic!("@@toPrimitive probe was not a function");
    };
    let exotic = context.new_object().unwrap();
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
    assert!(
        runtime
            .define_own_property(
                &exotic,
                &to_primitive,
                &data_descriptor(Value::Object(exotic_method_object), true, true, true),
            )
            .unwrap()
    );
    let Value::Object(exotic_error) = context
        .call(&error, Value::Undefined, &[Value::Object(exotic)])
        .unwrap()
    else {
        panic!("Error(exotic object) did not return an object");
    };
    assert!(matches!(
        runtime.get_own_property(&exotic_error, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("string")
    ));

    let Value::Object(name_conversion_object) =
        context.eval("(0, function(){ return 'N'; })").unwrap()
    else {
        panic!("name conversion probe was not a function");
    };
    let Value::Object(message_conversion_object) =
        context.eval("(0, function(){ return 'M'; })").unwrap()
    else {
        panic!("message conversion probe was not a function");
    };
    let name_value = context.new_object().unwrap();
    let message_value = context.new_object().unwrap();
    for (object, method) in [
        (&name_value, name_conversion_object),
        (&message_value, message_conversion_object),
    ] {
        assert!(
            runtime
                .define_own_property(
                    object,
                    &to_string_key,
                    &data_descriptor(Value::Object(method), true, true, true),
                )
                .unwrap()
        );
    }
    let receiver = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &receiver,
                &name_key,
                &data_descriptor(Value::Object(name_value), true, true, true),
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &receiver,
                &message_key,
                &data_descriptor(Value::Object(message_value), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&error_to_string, Value::Object(receiver), &[],)
            .unwrap(),
        Value::String(JsString::from_static("N: M"))
    );
}

#[test]
fn to_primitive_string_rejects_exotic_failures_and_skips_noncallable_ordinary_methods() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error = global_callable(&runtime, &mut context, "Error");
    let message_key = runtime.intern_property_key("message").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let value_of_key = runtime.intern_property_key("valueOf").unwrap();
    let to_primitive = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));

    let noncallable = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &noncallable,
                &to_primitive,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(&error, Value::Undefined, &[Value::Object(noncallable)],),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("noncallable @@toPrimitive did not throw an Error object");
    };
    assert!(matches!(
        runtime.get_own_property(&exception, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("not a function")
    ));

    let Value::Object(object_result_method) = context
        .eval("(0, function(){ return (0, function(){}); })")
        .unwrap()
    else {
        panic!("object-result @@toPrimitive probe was not a function");
    };
    let object_result = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &object_result,
                &to_primitive,
                &data_descriptor(Value::Object(object_result_method), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(&error, Value::Undefined, &[Value::Object(object_result)],),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("object-result @@toPrimitive did not throw an Error object");
    };
    assert!(matches!(
        runtime.get_own_property(&exception, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("toPrimitive")
    ));

    let Value::Object(value_of_method) = context.eval("(0, function(){ return 7; })").unwrap()
    else {
        panic!("valueOf probe was not a function");
    };
    let ordinary = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &ordinary,
                &to_string_key,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    assert!(
        runtime
            .define_own_property(
                &ordinary,
                &value_of_key,
                &data_descriptor(Value::Object(value_of_method), true, true, true),
            )
            .unwrap()
    );
    let Value::Object(converted) = context
        .call(&error, Value::Undefined, &[Value::Object(ordinary)])
        .unwrap()
    else {
        panic!("ordinary valueOf fallback did not create an Error object");
    };
    assert!(matches!(
        runtime.get_own_property(&converted, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("7")
    ));
}
