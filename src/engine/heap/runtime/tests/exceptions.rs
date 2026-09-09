use super::*;

#[test]
fn published_exception_regions_catch_native_callee_and_accessor_throws() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let native_error = UnlinkedFunction::fixture(
        vec![
            Instruction::Catch(5),
            Instruction::Null,
            Instruction::GetField(0),
            Instruction::NipCatch,
            Instruction::Return,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::primitive(Value::String(JsString::from_static("field"))).unwrap()],
        FunctionMetadata {
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    );
    let native_error = runtime
        .publish_unlinked_function(context.realm, native_error)
        .unwrap();
    let native_error = runtime
        .new_bytecode_closure(context.realm, &native_error)
        .unwrap();
    let Value::Object(error) = context.call(&native_error, Value::Undefined, &[]).unwrap() else {
        panic!("caught native throw was not an Error object");
    };
    assert!(runtime.is_error_object(&error).unwrap());
    let name = runtime.intern_property_key("name").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    let stack = runtime.intern_property_key("stack").unwrap();
    assert!(matches!(
        context.get_property(&error, &stack).unwrap(),
        Value::String(_)
    ));

    let child = UnlinkedFunction::fixture(
        vec![Instruction::PushI32(17), Instruction::Throw],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let caller = UnlinkedFunction::fixture(
        vec![
            Instruction::Catch(5),
            Instruction::FClosure(0),
            Instruction::Call(0),
            Instruction::NipCatch,
            Instruction::Return,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(child)],
        FunctionMetadata {
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    );
    let caller = runtime
        .publish_unlinked_function(context.realm, caller)
        .unwrap();
    let caller = runtime
        .new_bytecode_closure(context.realm, &caller)
        .unwrap();
    assert_eq!(
        context.call(&caller, Value::Undefined, &[]).unwrap(),
        Value::Int(17)
    );

    let Value::Object(getter) = context.eval("(function(){throw 23})").unwrap() else {
        panic!("accessor getter was not callable");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let global = context.global_object().unwrap();
    let accessor_name = runtime.intern_property_key("__caught_accessor").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &accessor_name,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    let accessor = UnlinkedFunction::fixture_with_closure_variables(
        vec![
            Instruction::Catch(4),
            Instruction::GetVar(0),
            Instruction::NipCatch,
            Instruction::Return,
            Instruction::Return,
        ],
        vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("__caught_accessor")))
                .unwrap(),
        ],
        FunctionMetadata {
            closure_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        },
        vec![ClosureVariable {
            source: ClosureSource::Global,
            name: ClosureVariableName::Constant(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }],
    );
    let accessor = runtime
        .publish_unlinked_function(context.realm, accessor)
        .unwrap();
    let accessor = runtime
        .new_bytecode_closure(context.realm, &accessor)
        .unwrap();
    assert_eq!(
        context.call(&accessor, Value::Undefined, &[]).unwrap(),
        Value::Int(23)
    );
}

#[test]
fn pending_exception_slot_owns_and_transfers_object_roots() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let object_id = object.object_id();
    runtime
        .set_pending_exception(Value::Object(object.clone()))
        .unwrap();
    assert!(runtime.has_pending_exception());
    assert_eq!(
        runtime.0.state.borrow().heap.object_strong_count(object_id),
        Ok(2)
    );
    drop(object);

    let exception = runtime.take_pending_exception().unwrap().unwrap();
    assert!(!runtime.has_pending_exception());
    assert!(matches!(
        &exception,
        Value::Object(value) if value.object_id() == object_id
    ));
    assert_eq!(
        runtime.0.state.borrow().heap.object_strong_count(object_id),
        Ok(1)
    );
    drop(exception);
    assert_eq!(runtime.heap_counts().object_nodes, 0);
}

#[test]
fn pending_exception_roots_survive_gc_and_preserve_symbol_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let object_id = object.object_id();
    let self_key = runtime.intern_property_key("self").unwrap();
    assert!(
        context
            .set_property(&object, &self_key, Value::Object(object.clone()))
            .unwrap()
    );
    runtime
        .set_pending_exception(Value::Object(object.clone()))
        .unwrap();
    drop(object);

    assert_eq!(runtime.run_gc().unwrap().cleanup.finalized_objects, 0);
    let exception = runtime.take_pending_exception().unwrap().unwrap();
    assert!(matches!(
        &exception,
        Value::Object(object) if object.object_id() == object_id
    ));
    drop(exception);
    assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 1);

    let symbol = runtime
        .new_symbol(Some(JsString::from_static("boom")))
        .unwrap();
    let expected = symbol.clone();
    runtime
        .set_pending_exception(Value::Symbol(symbol))
        .unwrap();
    let exception = runtime.take_pending_exception().unwrap().unwrap();
    assert!(matches!(exception, Value::Symbol(symbol) if symbol == expected));
}

#[test]
fn throw_completion_moves_the_value_into_the_runtime_exception_slot() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval("throw 9"), Err(RuntimeError::Exception));
    assert!(context.has_exception());
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
    assert!(!context.has_exception());
}

#[test]
fn vm_fault_materializes_a_native_error_in_the_callee_realm() {
    let runtime = Runtime::new();
    let mut compiler_context = runtime.new_context();
    let function = compiler_context.compile("1n + 1").unwrap();
    let expected_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(compiler_context.realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Type.index()]
    .unwrap();
    let mut caller_context = runtime.new_context();
    let caller_prototype = runtime
        .0
        .state
        .borrow()
        .heap
        .context(caller_context.realm)
        .unwrap()
        .native_error_prototypes[NativeErrorKind::Type.index()]
    .unwrap();
    assert_ne!(expected_prototype, caller_prototype);

    assert_eq!(
        caller_context.execute(&function),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller_context.take_exception().unwrap().unwrap() else {
        panic!("expected a native Error object");
    };
    assert!(runtime.is_error_object(&error).unwrap());
    assert!(matches!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .object(error.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Error
    ));
    assert_eq!(
        runtime
            .get_prototype_of(&error)
            .unwrap()
            .unwrap()
            .object_id(),
        expected_prototype
    );

    let message = runtime.intern_property_key("message").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value,
        writable,
        enumerable,
        configurable,
    } = runtime.get_own_property(&error, &message).unwrap().unwrap()
    else {
        panic!("native Error message must be an own data property");
    };
    assert_eq!(
        value,
        Value::String(JsString::from_static("cannot convert bigint to number"))
    );
    assert!(writable);
    assert!(!enumerable);
    assert!(configurable);

    let name = runtime.intern_property_key("name").unwrap();
    assert_eq!(
        caller_context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    let prototype = runtime.get_prototype_of(&error).unwrap().unwrap();
    assert!(!runtime.is_error_object(&prototype).unwrap());
}

#[test]
fn nested_fault_non_callable_and_compile_syntax_use_exception_completion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context.eval("(function(){ return 1n + 1; })()"),
        Err(RuntimeError::Exception)
    );
    let Value::Object(nested) = context.take_exception().unwrap().unwrap() else {
        panic!("expected nested TypeError");
    };
    assert!(runtime.is_error_object(&nested).unwrap());

    assert_eq!(context.eval("(1)()"), Err(RuntimeError::Exception));
    let Value::Object(not_callable) = context.take_exception().unwrap().unwrap() else {
        panic!("expected non-callable TypeError");
    };
    assert!(matches!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .object(not_callable.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Error
    ));

    assert_eq!(context.compile("throw\n9"), Err(RuntimeError::Exception));
    let Value::Object(syntax) = context.take_exception().unwrap().unwrap() else {
        panic!("expected SyntaxError");
    };
    assert!(matches!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .object(syntax.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Error
    ));
    let name = runtime.intern_property_key("name").unwrap();
    assert_eq!(
        context.get_property(&syntax, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );
}
