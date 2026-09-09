use super::*;

#[test]
fn function_bind_and_to_string_use_quickjs_payload_and_source_paths() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let Value::Object(bind_object) = context
        .get_property(&function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
    let Value::Object(to_string_object) = context
        .get_property(&function_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Function.prototype.toString was not an object");
    };
    let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();

    let authored = "function /*keep*/ named(a, b) { return a + b; }";
    let Value::Object(target_object) = context.eval(&format!("({authored})")).unwrap() else {
        panic!("function source did not evaluate to an object");
    };
    assert_eq!(
        context
            .call(&to_string, Value::Object(target_object.clone()), &[],)
            .unwrap(),
        Value::String(JsString::try_from_utf8(authored).unwrap())
    );
    let name_key = runtime.intern_property_key("name").unwrap();
    assert!(
        context
            .define_own_property(
                &target_object,
                &name_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::String(JsString::from_static(
                        "changed"
                    ))),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(target_object.clone()), &[],)
            .unwrap(),
        Value::String(JsString::try_from_utf8(authored).unwrap()),
        "stored bytecode source must not read the mutable name property"
    );

    let Value::Object(zero_argument_bound) = context
        .call(&bind, Value::Object(target_object.clone()), &[])
        .unwrap()
    else {
        panic!("zero-argument bind did not return an object");
    };
    assert_eq!(
        own_data_value(&runtime, &zero_argument_bound, "length"),
        Value::Int(2)
    );
    assert_eq!(
        own_data_value(&runtime, &zero_argument_bound, "name"),
        Value::String(JsString::from_static("bound changed"))
    );

    let bound = context
        .call(
            &bind,
            Value::Object(target_object.clone()),
            &[Value::Undefined, Value::Int(4)],
        )
        .unwrap();
    let Value::Object(bound_object) = bound else {
        panic!("bind did not return an object");
    };
    let bound = runtime.as_callable(&bound_object).unwrap().unwrap();
    assert_eq!(
        context.call(&bound, Value::Null, &[Value::Int(5)]).unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        runtime.get_prototype_of(&bound_object).unwrap(),
        Some(function_prototype.clone())
    );
    assert_eq!(own_key_names(&runtime, &bound_object), ["length", "name"]);
    assert_eq!(
        own_data_value(&runtime, &bound_object, "length"),
        Value::Int(1)
    );
    assert_eq!(
        own_data_value(&runtime, &bound_object, "name"),
        Value::String(JsString::from_static("bound changed"))
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(bound_object.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function bound changed() {\n    [native code]\n}"
        ))
    );

    let Value::Object(sum_target_object) = context
        .eval("(function(a,b,c){return a * 100 + b * 10 + c;})")
        .unwrap()
    else {
        panic!("sum target was not a function");
    };
    let inner = context
        .call(
            &bind,
            Value::Object(sum_target_object),
            &[Value::Undefined, Value::Int(1)],
        )
        .unwrap();
    let Value::Object(inner_object) = inner else {
        panic!("inner bind was not an object");
    };
    let outer = context
        .call(
            &bind,
            Value::Object(inner_object),
            &[Value::Undefined, Value::Int(2)],
        )
        .unwrap();
    let Value::Object(outer_object) = outer else {
        panic!("outer bind was not an object");
    };
    let outer = runtime.as_callable(&outer_object).unwrap().unwrap();
    assert_eq!(
        context.call(&outer, Value::Null, &[Value::Int(3)]).unwrap(),
        Value::Int(123)
    );

    let first_this = context.new_object().unwrap();
    let second_this = context.new_object().unwrap();
    let Value::Object(this_target) = context.eval("(function(){return this;})").unwrap() else {
        panic!("this target was not a function");
    };
    let inner = context
        .call(
            &bind,
            Value::Object(this_target),
            &[Value::Object(first_this.clone())],
        )
        .unwrap();
    let Value::Object(inner) = inner else {
        panic!("bound this function was not an object");
    };
    let rebound = context
        .call(&bind, Value::Object(inner), &[Value::Object(second_this)])
        .unwrap();
    let Value::Object(rebound) = rebound else {
        panic!("rebound function was not an object");
    };
    let rebound = runtime.as_callable(&rebound).unwrap().unwrap();
    assert_eq!(
        context.call(&rebound, Value::Null, &[]).unwrap(),
        Value::Object(first_this)
    );

    let Value::Object(constructor_object) = context
        .eval("(function Constructor(){return new.target;})")
        .unwrap()
    else {
        panic!("constructor target was not a function");
    };
    let constructor = runtime.as_callable(&constructor_object).unwrap().unwrap();
    let Value::Object(bound_constructor) = context
        .call(
            &bind,
            Value::Object(constructor_object.clone()),
            &[Value::Undefined],
        )
        .unwrap()
    else {
        panic!("bound constructor was not an object");
    };
    let bound_constructor = runtime.as_callable(&bound_constructor).unwrap().unwrap();
    assert_eq!(
        context.construct(&bound_constructor, &[]).unwrap(),
        Value::Object(constructor_object)
    );
    let Value::Object(other_object) = context.eval("(function Other(){})").unwrap() else {
        panic!("explicit new target was not a function");
    };
    let other = runtime.as_callable(&other_object).unwrap().unwrap();
    assert_eq!(
        context
            .construct_with_new_target(&bound_constructor, &other, &[])
            .unwrap(),
        Value::Object(other_object)
    );
    drop(constructor);

    assert_eq!(
        context.eval("(function named(){}) + \"\"").unwrap(),
        Value::String(JsString::from_static("function named(){}"))
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(function_prototype.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("function () {\n    [native code]\n}"))
    );

    for (function_kind, expected) in [
        (
            crate::engine::code::function::metadata::FunctionKind::Generator,
            "function *fallback() {\n    [native code]\n}",
        ),
        (
            crate::engine::code::function::metadata::FunctionKind::Async,
            "async function fallback() {\n    [native code]\n}",
        ),
        (
            crate::engine::code::function::metadata::FunctionKind::AsyncGenerator,
            "async function *fallback() {\n    [native code]\n}",
        ),
    ] {
        let (code, metadata) = if matches!(
            function_kind,
            crate::engine::code::function::metadata::FunctionKind::Generator
                | crate::engine::code::function::metadata::FunctionKind::AsyncGenerator
        ) {
            (
                vec![
                    Instruction::InitialYield,
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                FunctionMetadata {
                    max_stack: 1,
                    function_kind,
                    has_prototype: true,
                    ..FunctionMetadata::default()
                },
            )
        } else {
            (
                vec![Instruction::Undefined, Instruction::Return],
                FunctionMetadata {
                    max_stack: 1,
                    function_kind,
                    ..FunctionMetadata::default()
                },
            )
        };
        let function = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::fixture(code, Vec::new(), metadata)
                    .with_name(Some(JsString::from_static("fallback"))),
            )
            .unwrap();
        let callable = runtime
            .new_bytecode_closure(context.realm, &function)
            .unwrap();
        assert_eq!(
            context
                .call(&to_string, Value::Object(callable.as_object().clone()), &[],)
                .unwrap(),
            Value::String(JsString::try_from_utf8(expected).unwrap())
        );
    }

    assert!(
        context
            .define_own_property(
                &to_string_object,
                &name_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(3)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(to_string_object.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static(
            "function 3() {\n    [native code]\n}"
        ))
    );

    let Value::Object(name_getter_object) = context.eval("(function(){throw \"NAME\";})").unwrap()
    else {
        panic!("name getter was not a function");
    };
    let name_getter = runtime.as_callable(&name_getter_object).unwrap().unwrap();
    let throwing_name = OrdinaryPropertyDescriptor {
        get: DescriptorField::Present(AccessorValue::Callable(name_getter.clone())),
        set: DescriptorField::Present(AccessorValue::Undefined),
        enumerable: DescriptorField::Present(false),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    };
    assert!(
        context
            .define_own_property(&target_object, &name_key, &throwing_name)
            .unwrap()
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(target_object), &[])
            .unwrap(),
        Value::String(JsString::try_from_utf8(authored).unwrap()),
        "stored bytecode source must bypass a throwing name getter"
    );
    assert!(
        context
            .define_own_property(&to_string_object, &name_key, &throwing_name)
            .unwrap()
    );
    assert_eq!(
        context.call(&to_string, Value::Object(to_string_object), &[],),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::String(JsString::from_static("NAME")))
    );

    let symbol_name = runtime
        .new_symbol(Some(JsString::from_static("native-name")))
        .unwrap();
    assert!(
        context
            .define_own_property(
                &bind_object,
                &name_key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Symbol(symbol_name)),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert_eq!(
        context.call(&to_string, Value::Object(bind_object), &[]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Symbol function name did not throw an Error object");
    };
    let message_key = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message_key).unwrap(),
        Value::String(JsString::from_static("cannot convert symbol to string"))
    );
}

#[test]
fn bound_function_payload_owns_symbols_and_cycles_across_layout_changes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind_object) = context
        .get_property(&function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("Function.prototype.bind was not an object");
    };
    let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
    let Value::Object(target_object) = context.eval("(function(value){return value;})").unwrap()
    else {
        panic!("bound payload target was not a function");
    };
    let baseline_atoms = runtime.test_atom_count();
    let symbol = runtime
        .new_symbol(Some(JsString::from_static("bound-payload")))
        .unwrap();
    let Value::Object(bound_object) = context
        .call(
            &bind,
            Value::Object(target_object.clone()),
            &[Value::Undefined, Value::Symbol(symbol.clone())],
        )
        .unwrap()
    else {
        panic!("symbol-bound function was not an object");
    };
    let extra_key = runtime.intern_property_key("bound-extra").unwrap();
    assert!(
        context
            .define_own_property(
                &bound_object,
                &extra_key,
                &data_descriptor(Value::Int(1), true, true, true),
            )
            .unwrap()
    );
    let bound = runtime.as_callable(&bound_object).unwrap().unwrap();
    drop(symbol);
    let returned = context.call(&bound, Value::Undefined, &[]).unwrap();
    assert!(matches!(returned, Value::Symbol(_)));
    drop(returned);
    drop(bound);
    drop(bound_object);
    drop(extra_key);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);

    runtime.run_gc().unwrap();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let argument = context.new_object().unwrap();
    let Value::Object(bound_object) = context
        .call(
            &bind,
            Value::Object(target_object.clone()),
            &[Value::Undefined, Value::Object(argument.clone())],
        )
        .unwrap()
    else {
        panic!("cycle-bound function was not an object");
    };
    let back_key = runtime.intern_property_key("bound-back").unwrap();
    assert!(
        set_property(
            &runtime,
            &argument,
            &back_key,
            Value::Object(bound_object.clone()),
        )
        .unwrap()
    );
    drop(bound_object);
    drop(argument);
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
    let stats = runtime.run_gc().unwrap();
    assert!(stats.cleanup.finalized_objects >= 2);
    assert!(runtime.as_callable(&target_object).unwrap().is_some());
    assert_eq!(
        runtime.heap_counts().object_nodes,
        baseline_objects,
        "unexpected GC delta: {stats:?}"
    );
}

#[test]
fn bound_function_uses_bind_realm_but_delegates_function_realm_and_has_instance() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_function_prototype = first.function_prototype().unwrap();
    let bind_key = runtime.intern_property_key("bind").unwrap();
    let Value::Object(bind_object) = first
        .get_property(&first_function_prototype, &bind_key)
        .unwrap()
    else {
        panic!("first realm bind was not an object");
    };
    let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
    let Value::Object(target_object) = second.eval("(function Target(){})").unwrap() else {
        panic!("second realm target was not a function");
    };

    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let Value::Object(custom_method) = second.eval("(function(value){return value;})").unwrap()
    else {
        panic!("custom hasInstance method was not a function");
    };
    assert!(
        second
            .define_own_property(
                &target_object,
                &has_instance_key,
                &data_descriptor(Value::Object(custom_method), true, false, true),
            )
            .unwrap()
    );

    let Value::Object(bound_object) = first
        .call(&bind, Value::Object(target_object), &[Value::Undefined])
        .unwrap()
    else {
        panic!("cross-realm bound function was not an object");
    };
    let bound = runtime.as_callable(&bound_object).unwrap().unwrap();
    assert_eq!(
        runtime.get_prototype_of(&bound_object).unwrap(),
        Some(first_function_prototype.clone()),
        "bound [[Prototype]] must come from the bind method realm"
    );
    assert_eq!(runtime.callable_realm(&bound).unwrap(), second.realm);

    let Value::Object(nested_bound_object) = first
        .call(
            &bind,
            Value::Object(bound_object.clone()),
            &[Value::Undefined],
        )
        .unwrap()
    else {
        panic!("nested cross-realm bound function was not an object");
    };
    let nested_bound = runtime.as_callable(&nested_bound_object).unwrap().unwrap();
    assert_eq!(runtime.callable_realm(&nested_bound).unwrap(), second.realm);

    let Value::Object(has_instance_object) = first
        .get_property(&first_function_prototype, &has_instance_key)
        .unwrap()
    else {
        panic!("Function.prototype[Symbol.hasInstance] was not an object");
    };
    let has_instance = runtime.as_callable(&has_instance_object).unwrap().unwrap();
    assert_eq!(
        first
            .call(
                &has_instance,
                Value::Object(nested_bound_object),
                &[Value::Int(1)],
            )
            .unwrap(),
        Value::Bool(true),
        "bound ordinary hasInstance must delegate the primitive candidate to target @@hasInstance"
    );
}

#[test]
fn deep_standard_bound_has_instance_delegation_is_host_stack_safe() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let has_instance_key =
        PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
    let Value::Object(has_instance_object) = context
        .get_property(&function_prototype, &has_instance_key)
        .unwrap()
    else {
        panic!("Function.prototype[Symbol.hasInstance] was not an object");
    };
    let has_instance = runtime.as_callable(&has_instance_object).unwrap().unwrap();
    let Value::Object(target_object) = context.eval("(function Target(){})").unwrap() else {
        panic!("target was not a function");
    };
    let mut target = runtime.as_callable(&target_object).unwrap().unwrap();

    // Pinned QuickJS still completes at this depth with its default stack
    // budget. The Rust path must preserve that result without recursively
    // consuming the host stack.
    for _ in 0..512 {
        target = runtime
            .new_bound_function(context.realm, &target, &Value::Undefined, &[])
            .unwrap();
    }
    assert_eq!(
        context
            .call(
                &has_instance,
                Value::Object(target.into_object()),
                &[Value::Int(1)],
            )
            .unwrap(),
        Value::Bool(false)
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn error_intrinsic_graph_and_lazy_methods_match_quickjs_descriptors() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error = global_callable(&runtime, &mut context, "Error");
    let type_error = global_callable(&runtime, &mut context, "TypeError");
    let aggregate_error = global_callable(&runtime, &mut context, "AggregateError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let constructor_key = runtime.intern_property_key("constructor").unwrap();
    let length_key = runtime.intern_property_key("length").unwrap();
    let is_error_key = runtime.intern_property_key("isError").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    let aggregate_key = runtime.intern_property_key("AggregateError").unwrap();

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(error_prototype),
        writable: false,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error.prototype descriptor did not match QuickJS");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(type_error_prototype),
        writable: false,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("TypeError.prototype descriptor did not match QuickJS");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(aggregate_error_prototype),
        writable: false,
        enumerable: false,
        configurable: false,
    } = runtime
        .get_own_property(aggregate_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("AggregateError.prototype descriptor did not match QuickJS");
    };

    let object_count = runtime.heap_counts().object_nodes;
    let realm_strong_count = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(context.realm)
        .unwrap();
    assert_eq!(
        own_key_names(&runtime, error.as_object()),
        ["length", "name", "isError", "prototype"]
    );
    assert_eq!(
        own_key_names(&runtime, &error_prototype),
        ["toString", "name", "message", "constructor"]
    );
    assert_eq!(
        own_key_names(&runtime, aggregate_error.as_object()),
        ["length", "name", "prototype"]
    );
    assert_eq!(
        own_key_names(&runtime, &aggregate_error_prototype),
        ["name", "message", "constructor"]
    );
    assert!(
        runtime
            .has_own_property(error.as_object(), &is_error_key)
            .unwrap()
    );
    assert!(
        runtime
            .has_own_property(&error_prototype, &to_string_key)
            .unwrap()
    );
    assert_eq!(runtime.heap_counts().object_nodes, object_count);

    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(is_error),
        writable: true,
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(error.as_object(), &is_error_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error.isError did not materialize as a native data property");
    };
    assert_eq!(runtime.heap_counts().object_nodes, object_count + 1);
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(realm_strong_count)
    );
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(to_string),
        writable: true,
        enumerable: false,
        configurable: true,
    } = runtime
        .get_own_property(&error_prototype, &to_string_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error.prototype.toString did not materialize as native data");
    };
    assert_eq!(runtime.heap_counts().object_nodes, object_count + 2);
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(realm_strong_count)
    );
    assert!(runtime.as_callable(&is_error).unwrap().is_some());
    assert!(runtime.as_callable(&to_string).unwrap().is_some());

    assert!(matches!(
        runtime.get_own_property(&error_prototype, &name_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("Error")
    ));
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm),
        Ok(realm_strong_count - 1)
    );

    assert_eq!(
        runtime.get_prototype_of(error.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(type_error.as_object()).unwrap(),
        Some(error.as_object().clone())
    );
    assert_eq!(
        runtime.get_prototype_of(&error_prototype).unwrap(),
        Some(context.object_prototype().unwrap())
    );
    assert_eq!(
        runtime.get_prototype_of(&type_error_prototype).unwrap(),
        Some(error_prototype.clone())
    );
    assert_eq!(
        runtime
            .get_prototype_of(aggregate_error.as_object())
            .unwrap(),
        Some(error.as_object().clone())
    );
    assert_eq!(
        runtime
            .get_prototype_of(&aggregate_error_prototype)
            .unwrap(),
        Some(error_prototype.clone())
    );
    assert!(matches!(
        runtime
            .get_own_property(&error_prototype, &constructor_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == *error.as_object()
    ));
    assert!(matches!(
        runtime
            .get_own_property(&type_error_prototype, &constructor_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == *type_error.as_object()
    ));
    assert!(matches!(
        runtime
            .get_own_property(&aggregate_error_prototype, &constructor_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == *aggregate_error.as_object()
    ));
    assert!(matches!(
        runtime
            .get_own_property(aggregate_error.as_object(), &length_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(2),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));
    assert!(!runtime.is_error_object(&error_prototype).unwrap());
    assert!(!runtime.is_error_object(&type_error_prototype).unwrap());
    assert!(!runtime.is_error_object(&aggregate_error_prototype).unwrap());
    assert_eq!(
        context
            .get_property(&context.global_object().unwrap(), &aggregate_key)
            .unwrap(),
        Value::Object(aggregate_error.as_object().clone())
    );
}
