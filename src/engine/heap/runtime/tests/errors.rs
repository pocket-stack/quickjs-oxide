use super::*;

#[test]
fn native_error_message_preserves_raw_printf_and_js_new_string_boundaries() {
    let mut embedded_nul = NativeErrorMessage::new();
    embedded_nul.push_utf8("P");
    embedded_nul.push_bytes([0, b'T', b'A', b'I', b'L']);
    assert_eq!(
        embedded_nul
            .to_js_string()
            .unwrap()
            .utf16_units()
            .collect::<Vec<_>>(),
        [u16::from(b'P')]
    );

    let mut invalid_run = NativeErrorMessage::new();
    invalid_run.push_c_string_bytes([0x80, b'A', 0, b'B']);
    assert_eq!(
        invalid_run
            .to_js_string()
            .unwrap()
            .utf16_units()
            .collect::<Vec<_>>(),
        [0xfffd]
    );

    let mut surrogate = NativeErrorMessage::new();
    surrogate.push_c_string_bytes([0xed, 0xa0, 0x80, 0]);
    assert_eq!(
        surrogate
            .to_js_string()
            .unwrap()
            .utf16_units()
            .collect::<Vec<_>>(),
        [0xd800]
    );
}

#[test]
fn native_error_sidecar_survives_atom_and_parser_materializers() {
    fn message_units(
        runtime: &Runtime,
        context: &mut crate::engine::api::context::Context,
        error: Value,
    ) -> Vec<u16> {
        let Value::Object(error) = error else {
            panic!("native Error materializer did not return an object");
        };
        let key = runtime.intern_property_key("message").unwrap();
        let Value::String(message) = context.get_property(&error, &key).unwrap() else {
            panic!("native Error message was not a String");
        };
        message.utf16_units().collect()
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let spelling = JsString::try_from_utf16(
        vec![u16::from(b'A'); 55]
            .into_iter()
            .chain([0xd83d, 0xde42]),
    )
    .unwrap();
    let key = runtime.intern_property_key_js_string(&spelling).unwrap();
    let atom_error = runtime
        .native_atom_error(ErrorKind::Reference, "'", &key, "' is not defined")
        .unwrap();
    assert_eq!(
        atom_error.message(),
        format!("'{}�' is not defined", "A".repeat(55))
    );
    let materialized = runtime
        .new_native_error_from_error(
            context.realm,
            NativeErrorKind::Reference,
            &atom_error.clone(),
        )
        .unwrap();
    assert_eq!(
        message_units(&runtime, &mut context, materialized),
        [
            vec![u16::from(b'\'')],
            vec![u16::from(b'A'); 55],
            vec![0xd83d],
            "' is not defined".encode_utf16().collect(),
        ]
        .concat()
    );

    let mut raw = NativeErrorMessage::new();
    raw.push_bytes([0xed, 0xa0, 0x80]);
    let syntax_error = Error::from_native_message(ErrorKind::Syntax, raw);
    let materialized = runtime
        .new_native_error_without_backtrace_from_error(
            context.realm,
            NativeErrorKind::Syntax,
            &syntax_error,
        )
        .unwrap();
    assert_eq!(
        message_units(&runtime, &mut context, materialized),
        vec![0xd800]
    );
}

#[test]
fn atom_named_vm_and_global_errors_use_the_runtime_atom_table() {
    fn expected(prefix: &str, suffix: &str) -> Vec<u16> {
        [
            prefix.encode_utf16().collect(),
            vec![u16::from(b'A'); 55],
            vec![0xd83d],
            suffix.encode_utf16().collect(),
        ]
        .concat()
    }

    fn global_get(
        runtime: &Runtime,
        context: &crate::engine::api::context::Context,
        name: &JsString,
        is_lexical: bool,
    ) -> crate::engine::api::FunctionBytecodeRef {
        runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::fixture_with_closure_variables(
                    vec![Instruction::GetVar(0), Instruction::Return],
                    vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
                    FunctionMetadata {
                        closure_count: 1,
                        max_stack: 1,
                        strict: true,
                        ..FunctionMetadata::default()
                    },
                    vec![ClosureVariable {
                        source: ClosureSource::Global,
                        name: ClosureVariableName::Constant(0),
                        is_lexical,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                    }],
                ),
            )
            .unwrap()
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let name = JsString::try_from_utf16(
        vec![u16::from(b'A'); 55]
            .into_iter()
            .chain([0xd83d, 0xde42]),
    )
    .unwrap();

    let read_only = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::fixture(
                vec![Instruction::Undefined, Instruction::ThrowReadOnly(0)],
                vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    assert_eq!(context.execute(&read_only), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context)
            .utf16_units()
            .collect::<Vec<_>>(),
        expected("'", "' is read-only")
    );

    let missing = global_get(&runtime, &context, &name, false);
    assert_eq!(context.execute(&missing), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context)
            .utf16_units()
            .collect::<Vec<_>>(),
        expected("'", "' is not defined")
    );

    runtime
        .create_global_lexical_js_string_for_test(context.realm, &name, false, None)
        .unwrap();
    let tdz = global_get(&runtime, &context, &name, true);
    assert_eq!(context.execute(&tdz), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context)
            .utf16_units()
            .collect::<Vec<_>>(),
        expected("", " is not initialized")
    );
}

#[test]
fn error_constructor_fallback_uses_explicit_new_target_realm() {
    let runtime = Runtime::new();
    let mut constructor_context = runtime.new_context();
    let mut target_context = runtime.new_context();
    let type_error = global_callable(&runtime, &mut constructor_context, "TypeError");
    let target_type_error = global_callable(&runtime, &mut target_context, "TypeError");
    let aggregate_error = global_callable(&runtime, &mut constructor_context, "AggregateError");
    let target_aggregate_error = global_callable(&runtime, &mut target_context, "AggregateError");
    let constructor_array = global_callable(&runtime, &mut constructor_context, "Array");
    let target_array = global_callable(&runtime, &mut target_context, "Array");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(target_type_error_prototype),
        ..
    } = runtime
        .get_own_property(target_type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("target-realm TypeError prototype was not an object");
    };
    let Value::Object(new_target_object) = target_context.eval("(0, function(){})").unwrap() else {
        panic!("new.target probe did not produce a function");
    };
    let new_target = runtime.as_callable(&new_target_object).unwrap().unwrap();
    assert!(
        runtime
            .define_own_property(
                &new_target_object,
                &prototype_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Null),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let Value::Object(instance) = constructor_context
        .construct_with_new_target(&type_error, &new_target, &[])
        .unwrap()
    else {
        panic!("cross-realm TypeError construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(target_type_error_prototype)
    );
    assert!(runtime.is_error_object(&instance).unwrap());

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(target_aggregate_error_prototype),
        ..
    } = runtime
        .get_own_property(target_aggregate_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("target-realm AggregateError prototype was not an object");
    };
    let errors = constructor_context.eval("[1, 2]").unwrap();
    let Value::Object(instance) = constructor_context
        .construct_with_new_target(&aggregate_error, &new_target, &[errors])
        .unwrap()
    else {
        panic!("cross-realm AggregateError construction did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&instance).unwrap(),
        Some(target_aggregate_error_prototype)
    );
    assert!(runtime.is_error_object(&instance).unwrap());
    let Value::Object(errors) = own_data_value(&runtime, &instance, "errors") else {
        panic!("cross-realm AggregateError errors was not an object");
    };
    assert!(runtime.is_array_object(&errors).unwrap());
    let Value::Object(constructor_array_prototype) =
        own_data_value(&runtime, constructor_array.as_object(), "prototype")
    else {
        panic!("constructor-realm Array prototype was not an object");
    };
    let Value::Object(target_array_prototype) =
        own_data_value(&runtime, target_array.as_object(), "prototype")
    else {
        panic!("target-realm Array prototype was not an object");
    };
    assert_ne!(constructor_array_prototype, target_array_prototype);
    assert_eq!(
        runtime.get_prototype_of(&errors).unwrap(),
        Some(constructor_array_prototype)
    );
}

#[test]
fn error_constructor_preserves_getter_throw_and_defining_realm_conversion_error() {
    let runtime = Runtime::new();
    let mut defining_context = runtime.new_context();
    let mut caller_context = runtime.new_context();
    let error = global_callable(&runtime, &mut defining_context, "Error");
    let type_error = global_callable(&runtime, &mut defining_context, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let cause_key = runtime.intern_property_key("cause").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(error_prototype),
        ..
    } = runtime
        .get_own_property(error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("defining-realm Error prototype was not an object");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(type_error_prototype),
        ..
    } = runtime
        .get_own_property(type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("defining-realm TypeError prototype was not an object");
    };

    let Value::Object(cross_realm_error) = caller_context
        .call(&error, Value::Undefined, &[Value::Int(7)])
        .unwrap()
    else {
        panic!("cross-realm Error call did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&cross_realm_error).unwrap(),
        Some(error_prototype)
    );

    let Value::Object(getter_object) = caller_context.eval("(0, function(){ throw 9; })").unwrap()
    else {
        panic!("cause getter probe was not a function");
    };
    let getter = runtime.as_callable(&getter_object).unwrap().unwrap();
    let options = caller_context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &options,
                &cause_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        caller_context.call(
            &error,
            Value::Undefined,
            &[Value::Undefined, Value::Object(options)],
        ),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        caller_context.take_exception().unwrap(),
        Some(Value::Int(9))
    );

    let symbol = runtime.new_symbol(None).unwrap();
    assert!(matches!(
        caller_context.call(&error, Value::Undefined, &[Value::Symbol(symbol)],),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = caller_context.take_exception().unwrap().unwrap() else {
        panic!("symbol ToString failure was not an Error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&exception).unwrap(),
        Some(type_error_prototype)
    );
}
