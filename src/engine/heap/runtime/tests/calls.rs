use super::*;

#[test]
fn call_frame_loads_arguments_and_moves_values_through_locals() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = UnlinkedFunction::fixture(
        vec![
            Instruction::GetArg(0),
            Instruction::PutLocal(0),
            Instruction::GetLocal(0),
            Instruction::GetArg(1),
            Instruction::Add,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 2,
            local_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        },
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();
    let callable = runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap();

    assert_eq!(
        context
            .call(
                &callable,
                Value::Undefined,
                &[Value::Int(20), Value::Int(22)]
            )
            .unwrap(),
        Value::Int(42)
    );
}

#[test]
fn runtime_typeof_distinguishes_callable_and_ordinary_objects() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function = UnlinkedFunction::fixture(
        vec![
            Instruction::GetArg(0),
            Instruction::TypeOf,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        },
    );
    let function = runtime
        .publish_unlinked_function(context.realm, function)
        .unwrap();
    let callable = runtime
        .new_bytecode_closure(context.realm, &function)
        .unwrap();
    let ordinary = runtime.new_object(None).unwrap();

    let function_type = expect_string_value(
        context
            .call(
                &callable,
                Value::Undefined,
                &[Value::Object(callable.as_object().clone())],
            )
            .unwrap(),
    );
    assert_eq!(function_type, JsString::from_static("function"));
    let repeated_function_type = expect_string_value(
        context
            .call(
                &callable,
                Value::Undefined,
                &[Value::Object(callable.as_object().clone())],
            )
            .unwrap(),
    );
    assert!(function_type.same_representation(&repeated_function_type));
    let function_key = runtime.intern_property_key("function").unwrap();
    let canonical_function = runtime.property_key_to_js_string(&function_key).unwrap();
    assert!(function_type.same_representation(&canonical_function));

    let foreign_runtime = Runtime::new();
    let foreign_key = foreign_runtime.intern_property_key("function").unwrap();
    let foreign_function = foreign_runtime
        .property_key_to_js_string(&foreign_key)
        .unwrap();
    assert!(!function_type.same_representation(&foreign_function));
    assert_eq!(
        context
            .call(&callable, Value::Undefined, &[Value::Object(ordinary)],)
            .unwrap(),
        Value::String(JsString::from_static("object"))
    );
}

#[test]
fn fclosure_call_and_call_method_follow_quickjs_stack_layout() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let add_one = UnlinkedFunction::fixture(
        vec![
            Instruction::GetArg(0),
            Instruction::PushI32(1),
            Instruction::Add,
            Instruction::Return,
        ],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let caller = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::PushI32(41),
            Instruction::Call(1),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(add_one)],
        FunctionMetadata {
            max_stack: 2,
            strict: true,
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
        Value::Int(42)
    );

    let return_this = UnlinkedFunction::fixture(
        vec![Instruction::PushThis, Instruction::Return],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let method_caller = UnlinkedFunction::fixture(
        vec![
            Instruction::PushThis,
            Instruction::FClosure(0),
            Instruction::CallMethod(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(return_this)],
        FunctionMetadata {
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let method_caller = runtime
        .publish_unlinked_function(context.realm, method_caller)
        .unwrap();
    let method_caller = runtime
        .new_bytecode_closure(context.realm, &method_caller)
        .unwrap();
    let receiver = runtime.new_object(None).unwrap();
    assert_eq!(
        context
            .call(&method_caller, Value::Object(receiver.clone()), &[])
            .unwrap(),
        Value::Object(receiver)
    );
}

#[test]
fn nested_call_propagates_throw_without_publishing_it_early() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let throwing = UnlinkedFunction::fixture(
        vec![Instruction::PushI32(9), Instruction::Throw],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let caller = UnlinkedFunction::fixture(
        vec![
            Instruction::FClosure(0),
            Instruction::Call(0),
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(throwing)],
        FunctionMetadata {
            max_stack: 1,
            strict: true,
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
        context.call(&caller, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
}

#[test]
fn published_tail_call_uses_backtrace_and_current_activation_catch_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let wrapper = UnlinkedFunction::fixture(
        vec![Instruction::GetArg(0), Instruction::TailCall(0)],
        Vec::new(),
        FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    )
    .with_debug(UnlinkedFunctionDebug {
        filename: JsString::from_static("tail-wrapper.js"),
        pc2line: Some(Pc2LineTable::new(
            LineColumn::new(0, 0),
            vec![Pc2LineEntry {
                pc: 1,
                position: LineColumn::new(2, 4),
            }],
        )),
        source: None,
    });
    let wrapper = runtime
        .publish_unlinked_function(context.realm, wrapper)
        .unwrap();
    let wrapper = runtime
        .new_bytecode_closure(context.realm, &wrapper)
        .unwrap();
    let target = context
        .eval_with_filename(
            "(function tailTarget(){ return 1n + 1; })",
            "tail-target.js",
        )
        .unwrap();
    assert_eq!(
        context.call(&wrapper, Value::Undefined, &[target]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("tail target TypeError was not an object");
    };
    let stack = own_stack_string(&runtime, &error).to_utf8_lossy();
    assert!(stack.contains("tail-target.js:"), "{stack}");
    assert!(stack.contains("tail-wrapper.js:3:5"), "{stack}");
    assert!(!context.has_exception());

    let throwing = UnlinkedFunction::fixture(
        vec![Instruction::PushI32(17), Instruction::Throw],
        Vec::new(),
        FunctionMetadata {
            max_stack: 1,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let catcher = UnlinkedFunction::fixture(
        vec![
            Instruction::Catch(4),
            Instruction::FClosure(0),
            Instruction::TailCall(0),
            Instruction::Drop,
            Instruction::Return,
        ],
        vec![UnlinkedConstant::child(throwing)],
        FunctionMetadata {
            max_stack: 2,
            strict: true,
            ..FunctionMetadata::default()
        },
    );
    let catcher = runtime
        .publish_unlinked_function(context.realm, catcher)
        .unwrap();
    let catcher = runtime
        .new_bytecode_closure(context.realm, &catcher)
        .unwrap();
    assert_eq!(
        context.call(&catcher, Value::Undefined, &[]).unwrap(),
        Value::Int(17)
    );
    assert!(!context.has_exception());
}

#[test]
fn push_this_applies_strict_and_sloppy_callee_realm_rules() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let code = vec![Instruction::PushThis, Instruction::Return];

    let sloppy = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::fixture(
                code.clone(),
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let sloppy = runtime
        .new_bytecode_closure(context.realm, &sloppy)
        .unwrap();
    assert_eq!(
        context.call(&sloppy, Value::Undefined, &[]).unwrap(),
        Value::Object(global)
    );
    let boxed_number = context.call(&sloppy, Value::Int(1), &[]).unwrap();
    let Value::Object(boxed_number) = boxed_number else {
        panic!("sloppy Number this did not escape as a wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&boxed_number).unwrap(),
        Some(context.number_prototype().unwrap())
    );
    assert!(matches!(
        &runtime
            .0
            .state
            .borrow()
            .heap
            .object(boxed_number.object_id())
            .unwrap()
            .payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Number(value)) if *value == 1.0
    ));
    let ignores_this = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::fixture(
                vec![Instruction::PushI32(7), Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let ignores_this = runtime
        .new_bytecode_closure(context.realm, &ignores_this)
        .unwrap();
    assert_eq!(
        context.call(&ignores_this, Value::Int(1), &[]).unwrap(),
        Value::Int(7)
    );

    let strict = runtime
        .publish_unlinked_function(
            context.realm,
            UnlinkedFunction::fixture(
                code,
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    ..FunctionMetadata::default()
                },
            ),
        )
        .unwrap();
    let strict = runtime
        .new_bytecode_closure(context.realm, &strict)
        .unwrap();
    assert_eq!(
        context.call(&strict, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context.call(&strict, Value::Int(1), &[]).unwrap(),
        Value::Int(1)
    );
}
