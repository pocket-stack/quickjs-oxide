use super::*;

#[test]
fn native_function_retains_and_dispatches_in_its_defining_realm() {
    let runtime = Runtime::new();
    let defining_context = runtime.new_context();
    let defining_realm = defining_context.realm;
    let function_prototype = defining_context.function_prototype().unwrap();
    let callable = runtime
        .callable_from_value(Value::Object(function_prototype.clone()))
        .unwrap();

    let before_context_drop = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(defining_realm)
        .unwrap();
    drop(defining_context);
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(defining_realm),
        Ok(before_context_drop - 1)
    );
    assert!(matches!(
        runtime.bytecode_for_callable(&callable).unwrap(),
        CallableExecution::Native {
            target: NativeFunctionId::FunctionPrototype,
            realm,
            min_readable_args: 0,
        } if realm == defining_realm
    ));

    let mut caller_context = runtime.new_context();
    assert_eq!(
        caller_context
            .call(&callable, Value::Undefined, &[])
            .unwrap(),
        Value::Undefined
    );

    drop(callable);
    drop(function_prototype);
    runtime.run_gc().unwrap();
    assert!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .is_err()
    );
    assert_eq!(runtime.heap_counts().context_nodes, 1);
}

#[test]
fn native_call_preserves_actual_argc_padding_and_restores_active_frame() {
    let runtime = Runtime::new();
    let defining_context = runtime.new_context();
    let defining_realm = defining_context.realm;
    let function_prototype = defining_context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            defining_realm,
            NativeFunctionId::ArgumentProbe,
            2,
        )
        .unwrap();
    runtime
        .define_function_data_property(probe.as_object(), "length", Value::Int(99), false, true)
        .unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    assert!(runtime.delete_property(probe.as_object(), &length).unwrap());
    assert_eq!(
        runtime
            .get_own_property(probe.as_object(), &length)
            .unwrap(),
        None
    );

    let caller_context = runtime.new_context();
    let no_args = runtime
        .call_internal(caller_context.realm, &probe, Value::Undefined, &[])
        .unwrap();
    assert_eq!(
        no_args,
        Completion::Return(Value::String(JsString::from_static("0|2|2|false")))
    );
    let extra_args = runtime
        .call_internal(
            caller_context.realm,
            &probe,
            Value::Undefined,
            &[Value::Int(1), Value::Int(2), Value::Int(3)],
        )
        .unwrap();
    assert_eq!(
        extra_args,
        Completion::Return(Value::String(JsString::from_static("3|3|0|false")))
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert_eq!(
        runtime
            .call_internal(
                caller_context.realm,
                &probe,
                Value::Undefined,
                &[Value::Bool(false)],
            )
            .unwrap(),
        Completion::Throw(Value::String(JsString::from_static("native probe throw")))
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert!(matches!(
        runtime.call_internal(
            caller_context.realm,
            &probe,
            Value::Undefined,
            &[Value::Bool(true)],
        ),
        Err(RuntimeError::Invariant("native probe engine error"))
    ));
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn native_constructor_bit_is_independent_from_generic_cproto() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ArgumentProbe,
            0,
        )
        .unwrap();

    assert!(!runtime.is_constructor(probe.as_object()).unwrap());
    runtime
        .set_constructor_bit(probe.as_object(), true)
        .unwrap();
    assert!(runtime.is_constructor(probe.as_object()).unwrap());
    assert_eq!(
        context.construct(&probe, &[]).unwrap(),
        Value::String(JsString::from_static("0|0|0|true"))
    );
    runtime
        .set_constructor_bit(probe.as_object(), false)
        .unwrap();
    assert!(!runtime.is_constructor(probe.as_object()).unwrap());

    let ordinary = context.new_object().unwrap();
    runtime.set_constructor_bit(&ordinary, true).unwrap();
    assert!(runtime.is_constructor(&ordinary).unwrap());
}

#[test]
fn native_float_cproto_construct_adapter_uses_the_call_kernel() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let math_key = runtime.intern_property_key("Math").unwrap();
    let Value::Object(math) = context.get_property(&global, &math_key).unwrap() else {
        panic!("global Math was not an object");
    };
    let abs = property_callable(&runtime, &mut context, &math, "abs");
    let pow = property_callable(&runtime, &mut context, &math, "pow");

    for function in [&abs, &pow] {
        assert!(!runtime.is_constructor(function.as_object()).unwrap());
        runtime
            .set_constructor_bit(function.as_object(), true)
            .unwrap();
    }
    assert_eq!(
        context.construct(&abs, &[Value::Int(-3)]).unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        context
            .construct(&pow, &[Value::Int(2), Value::Int(5)])
            .unwrap(),
        Value::Int(32)
    );
    for function in [&abs, &pow] {
        runtime
            .set_constructor_bit(function.as_object(), false)
            .unwrap();
        assert!(!runtime.is_constructor(function.as_object()).unwrap());
    }
}

#[test]
fn native_constructor_cproto_adapters_use_defining_realm_and_restore_frames() {
    let runtime = Runtime::new();
    let defining_context = runtime.new_context();
    let defining_realm = defining_context.realm;
    let function_prototype = defining_context.function_prototype().unwrap();
    let constructor_only = runtime
        .new_bound_native_function(
            &function_prototype,
            defining_realm,
            NativeFunctionId::ConstructorProbe,
            0,
        )
        .unwrap();
    let constructor_or_function = runtime
        .new_bound_native_function(
            &function_prototype,
            defining_realm,
            NativeFunctionId::ConstructorOrFunctionProbe,
            0,
        )
        .unwrap();
    let caller_context = runtime.new_context();

    let called_without_new = runtime
        .call_internal(
            caller_context.realm,
            &constructor_only,
            Value::Undefined,
            &[],
        )
        .unwrap();
    let Completion::Throw(Value::Object(exception)) = called_without_new else {
        panic!("constructor-only native did not throw an object");
    };
    let defining_type_error_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(defining_realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Type.index()]
    .unwrap();
    assert_eq!(
        runtime
            .get_prototype_of(&exception)
            .unwrap()
            .unwrap()
            .object_id(),
        defining_type_error_prototype
    );
    let message = runtime.intern_property_key("message").unwrap();
    assert!(matches!(
        runtime.get_own_property(&exception, &message).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("must be called with new")
    ));
    assert!(runtime.0.state.borrow().active_frames.is_empty());

    assert_eq!(
        runtime
            .construct_internal(
                caller_context.realm,
                &constructor_only,
                &constructor_only,
                &[],
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("0|0|0|true")))
    );
    assert_eq!(
        runtime
            .call_internal(
                caller_context.realm,
                &constructor_or_function,
                Value::Undefined,
                &[],
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("0|0|0|false")))
    );
    assert_eq!(
        runtime
            .construct_internal(
                caller_context.realm,
                &constructor_or_function,
                &constructor_or_function,
                &[],
            )
            .unwrap(),
        Completion::Return(Value::String(JsString::from_static("0|0|0|true")))
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}
