use super::*;

#[test]
fn error_constructors_to_string_is_error_and_cause_follow_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error = global_callable(&runtime, &mut context, "Error");
    let type_error = global_callable(&runtime, &mut context, "TypeError");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let message_key = runtime.intern_property_key("message").unwrap();
    let cause_key = runtime.intern_property_key("cause").unwrap();
    let is_error_key = runtime.intern_property_key("isError").unwrap();
    let to_string_key = runtime.intern_property_key("toString").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(error_prototype),
        ..
    } = runtime
        .get_own_property(error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("Error constructor had no object prototype");
    };
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(type_error_prototype),
        ..
    } = runtime
        .get_own_property(type_error.as_object(), &prototype_key)
        .unwrap()
        .unwrap()
    else {
        panic!("TypeError constructor had no object prototype");
    };
    let Value::Object(is_error_object) = context
        .get_property(error.as_object(), &is_error_key)
        .unwrap()
    else {
        panic!("Error.isError was not an object");
    };
    let is_error = runtime.as_callable(&is_error_object).unwrap().unwrap();
    let Value::Object(to_string_object) = context
        .get_property(&error_prototype, &to_string_key)
        .unwrap()
    else {
        panic!("Error.prototype.toString was not an object");
    };
    let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();

    let Value::Object(empty) = context.call(&error, Value::Undefined, &[]).unwrap() else {
        panic!("Error() did not return an object");
    };
    assert!(runtime.is_error_object(&empty).unwrap());
    assert_eq!(
        runtime.get_prototype_of(&empty).unwrap(),
        Some(error_prototype.clone())
    );
    assert!(!runtime.has_own_property(&empty, &message_key).unwrap());
    assert_eq!(
        context
            .call(&to_string, Value::Object(empty.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("Error"))
    );

    let Value::Object(with_message) = context
        .call(&error, Value::Undefined, &[Value::Int(42)])
        .unwrap()
    else {
        panic!("Error(42) did not return an object");
    };
    assert!(matches!(
        runtime.get_own_property(&with_message, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: true,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("42")
    ));
    assert_eq!(
        context
            .call(&to_string, Value::Object(with_message.clone()), &[],)
            .unwrap(),
        Value::String(JsString::from_static("Error: 42"))
    );

    let Value::Object(typed) = context
        .construct(&type_error, &[Value::String(JsString::from_static("boom"))])
        .unwrap()
    else {
        panic!("new TypeError did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&typed).unwrap(),
        Some(type_error_prototype)
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(typed.clone()), &[])
            .unwrap(),
        Value::String(JsString::from_static("TypeError: boom"))
    );
    assert_eq!(
        context
            .call(&is_error, Value::Undefined, &[Value::Object(typed.clone())],)
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context
            .call(
                &is_error,
                Value::Undefined,
                &[Value::Object(error_prototype.clone())],
            )
            .unwrap(),
        Value::Bool(false)
    );
    let spoof = context.new_object().unwrap();
    assert!(
        runtime
            .set_prototype_of(&spoof, Some(&error_prototype))
            .unwrap()
    );
    assert_eq!(
        context
            .call(&is_error, Value::Undefined, &[Value::Object(spoof)],)
            .unwrap(),
        Value::Bool(false)
    );

    let options = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &options,
                &cause_key,
                &data_descriptor(Value::Undefined, true, true, true),
            )
            .unwrap()
    );
    let Value::Object(with_cause) = context
        .call(
            &error,
            Value::Undefined,
            &[Value::Undefined, Value::Object(options)],
        )
        .unwrap()
    else {
        panic!("Error(undefined, options) did not return an object");
    };
    assert!(!runtime.has_own_property(&with_cause, &message_key).unwrap());
    assert!(matches!(
        runtime.get_own_property(&with_cause, &cause_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Undefined,
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));

    let inherited_cause_holder = context.new_object().unwrap();
    assert!(
        runtime
            .define_own_property(
                &inherited_cause_holder,
                &cause_key,
                &data_descriptor(Value::Int(5), true, true, true),
            )
            .unwrap()
    );
    let inherited_options = context.new_object().unwrap();
    assert!(
        runtime
            .set_prototype_of(&inherited_options, Some(&inherited_cause_holder))
            .unwrap()
    );
    let Value::Object(with_inherited_cause) = context
        .call(
            &error,
            Value::Undefined,
            &[Value::Undefined, Value::Object(inherited_options)],
        )
        .unwrap()
    else {
        panic!("inherited cause Error construction did not return an object");
    };
    assert!(matches!(
        runtime
            .get_own_property(&with_inherited_cause, &cause_key)
            .unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(5),
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));

    assert!(matches!(
        context.call(&to_string, Value::Int(1), &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("non-object Error.prototype.toString throw was not an object");
    };
    assert!(matches!(
        runtime.get_own_property(&exception, &message_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) if value == JsString::from_static("not an object")
    ));
    assert_eq!(
        own_stack_string(&runtime, &exception),
        JsString::from_static("    at toString (native)\n")
    );
}

#[test]
fn error_stack_eager_capture_matches_quickjs_frames_sites_and_descriptor() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source =
        "(function outer(){ return (function inner(){ return new Error(\"boom\"); })(); })()";
    let Value::Object(error) = context.eval_with_filename(source, "<cmdline>").unwrap() else {
        panic!("nested Error constructor did not return an object");
    };
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::from_static(
            "    at inner (<cmdline>:1:62)\n    at outer (<cmdline>:1:20)\n    at <eval> (<cmdline>:1:80)\n"
        )
    );
    assert_eq!(own_key_names(&runtime, &error), ["message", "stack"]);
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert!(matches!(
        runtime.get_own_property(&error, &stack_key).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(_),
            writable: true,
            enumerable: false,
            configurable: true,
        })
    ));

    let error_constructor = global_callable(&runtime, &mut context, "Error");
    let Value::Object(direct) = context
        .call(&error_constructor, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("direct Error() did not return an object");
    };
    assert_eq!(own_key_names(&runtime, &direct), ["stack"]);
    assert_eq!(
        own_stack_string(&runtime, &direct),
        JsString::from_static("")
    );
}

#[test]
fn error_constructor_skips_only_itself_and_preserves_other_native_frames() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let probe = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            probe.as_object(),
            "name",
            Value::String(JsString::from_static("probe")),
            false,
            true,
        )
        .unwrap();
    let probe_key = runtime.intern_property_key("probe").unwrap();
    let global = context.global_object().unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &probe_key,
                &data_descriptor(Value::Object(probe.as_object().clone()), true, true, true,),
            )
            .unwrap()
    );

    let source = "(function viaNative(){ return probe(function callback(){ return new Error(\"x\"); }); })()";
    let Value::Object(error) = context.eval_with_filename(source, "native.js").unwrap() else {
        panic!("native callback did not return an Error");
    };
    let callback_construct = source.find("Error").unwrap() + "Error".len() + 1;
    let outer_return = source.find("return").unwrap() + 1;
    let root_call = source.rfind("()").unwrap() + 1;
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::try_from_utf8(&format!(
            "    at callback (native.js:1:{callback_construct})\n    at probe (native)\n    at viaNative (native.js:1:{outer_return})\n    at <eval> (native.js:1:{root_call})\n"
        ))
        .unwrap()
    );
}

#[test]
fn native_rethrow_pops_its_frame_before_bytecode_captures_missing_stack() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error_constructor = global_callable(&runtime, &mut context, "Error");
    let Value::Object(error) = context
        .call(&error_constructor, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("Error() did not return an object");
    };
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert!(runtime.delete_property(&error, &stack_key).unwrap());

    let function_prototype = context.function_prototype().unwrap();
    let rethrow = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    runtime
        .define_function_data_property(
            rethrow.as_object(),
            "name",
            Value::String(JsString::from_static("rethrowProbe")),
            false,
            true,
        )
        .unwrap();

    assert_eq!(
        context.call(
            &rethrow,
            Value::Undefined,
            &[Value::Object(error.clone()), Value::Bool(false)],
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(direct) = context.take_exception().unwrap().unwrap() else {
        panic!("direct native rethrow lost its Error");
    };
    assert_eq!(direct, error);
    assert!(!runtime.has_own_property(&error, &stack_key).unwrap());

    let global = context.global_object().unwrap();
    for (name, value) in [
        ("rethrowProbe", Value::Object(rethrow.as_object().clone())),
        ("heldError", Value::Object(error.clone())),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert!(
            context
                .define_own_property(&global, &key, &data_descriptor(value, true, true, true),)
                .unwrap()
        );
    }
    let source = "rethrowProbe(heldError, false)";
    assert!(matches!(
        context.eval_with_filename(source, "rethrow.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(from_bytecode) = context.take_exception().unwrap().unwrap() else {
        panic!("bytecode native rethrow lost its Error");
    };
    assert_eq!(from_bytecode, error);
    let call_column = source.find('(').unwrap() + 1;
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::try_from_utf8(&format!("    at <eval> (rethrow.js:1:{call_column})\n")).unwrap()
    );
}

#[test]
fn vm_error_stack_uses_fault_tail_call_and_root_call_sites() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = "(function outer(){ return (function inner(){ return 1n + 1; })(); })()";
    assert!(matches!(
        context.eval_with_filename(source, "<cmdline>"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("VM TypeError was not an object");
    };
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::from_static(
            "    at inner (<cmdline>:1:56)\n    at outer (<cmdline>:1:20)\n    at <eval> (<cmdline>:1:69)\n"
        )
    );
    assert_eq!(own_key_names(&runtime, &error), ["message", "stack"]);
}

#[test]
fn syntax_error_stack_prepends_parse_location_and_metadata_in_quickjs_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert!(matches!(
        context.eval_with_filename("1 +", "parse.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("SyntaxError was not an object");
    };
    assert_eq!(
        own_key_names(&runtime, &error),
        ["message", "fileName", "lineNumber", "columnNumber", "stack"]
    );
    assert_eq!(
        own_data_value(&runtime, &error, "fileName"),
        Value::String(JsString::from_static("parse.js"))
    );
    assert_eq!(
        own_data_value(&runtime, &error, "lineNumber"),
        Value::Int(1)
    );
    assert_eq!(
        own_data_value(&runtime, &error, "columnNumber"),
        Value::Int(4)
    );
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::from_static("    at parse.js:1:4\n")
    );
}

#[test]
fn eval_backtrace_barrier_marks_only_the_preexisting_caller_frame() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let caller = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    let caller_frame = runtime
        .push_native_active_frame(
            caller.as_object().clone(),
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
            0,
        )
        .unwrap();
    let options = EvalOptions {
        filename: "barrier.js".to_owned(),
        backtrace_barrier: true,
    };

    assert!(matches!(
        context.eval_with_options("1n + 1", &options),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(runtime_error) = context.take_exception().unwrap().unwrap() else {
        panic!("barrier VM error was not an object");
    };
    assert_eq!(
        own_stack_string(&runtime, &runtime_error),
        JsString::from_static("    at <eval> (barrier.js:1:4)\n")
    );
    assert!(
        !runtime
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .unwrap()
            .flags
            .backtrace_barrier
    );

    assert!(matches!(
        context.eval_with_options("1 +", &options),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(parse_error) = context.take_exception().unwrap().unwrap() else {
        panic!("barrier parse error was not an object");
    };
    assert_eq!(
        own_stack_string(&runtime, &parse_error),
        JsString::from_static("    at barrier.js:1:4\n")
    );
    assert!(
        !runtime
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .unwrap()
            .flags
            .backtrace_barrier
    );
    caller_frame.finish().unwrap();
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn active_script_or_module_name_walks_scripts_modules_and_eval_roots() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(runtime.active_script_or_module_name().unwrap(), None);

    let outer = push_named_script_active_frame(&runtime, &mut context, "outer.js");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("outer.js"))
    );

    let nested = push_named_script_active_frame(&runtime, &mut context, "nested.js");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("nested.js"))
    );
    nested.finish().unwrap();

    let module = push_named_module_active_frame(&runtime, &context, "entry.mjs");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("entry.mjs"))
    );
    module.finish().unwrap();

    let direct =
        push_named_eval_active_frame(&runtime, &context, "direct-eval.js", EvalKind::Direct);
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("outer.js"))
    );
    let indirect =
        push_named_eval_active_frame(&runtime, &context, "indirect-eval.js", EvalKind::Indirect);
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("outer.js"))
    );
    indirect.finish().unwrap();
    direct.finish().unwrap();
    outer.finish().unwrap();

    assert_eq!(runtime.active_script_or_module_name().unwrap(), None);
}

#[test]
fn active_script_or_module_name_skips_hidden_native_frames_but_stops_at_visible_native() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let outer = push_named_script_active_frame(&runtime, &mut context, "caller.js");
    let barrier = runtime.install_backtrace_barrier(true).unwrap();
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("caller.js")),
        "backtrace barriers do not delimit ScriptOrModule lookup"
    );

    let function_prototype = context.function_prototype().unwrap();
    let native = runtime
        .new_bound_native_function(
            &function_prototype,
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
        )
        .unwrap();
    let hidden = runtime
        .push_active_frame(
            native.as_object().clone(),
            None,
            context.realm,
            ActiveFrameFlags {
                backtrace_hidden: true,
                ..ActiveFrameFlags::default()
            },
            ActiveFrameKind::Native {
                target: NativeFunctionId::ActiveFrameProbe,
                actual_arg_count: 0,
                readable_arg_count: 0,
            },
            false,
        )
        .unwrap();
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("caller.js"))
    );
    hidden.finish().unwrap();

    let visible = runtime
        .push_native_active_frame(
            native.as_object().clone(),
            context.realm,
            NativeFunctionId::ActiveFrameProbe,
            0,
            0,
        )
        .unwrap();
    assert_eq!(runtime.active_script_or_module_name().unwrap(), None);
    visible.finish().unwrap();

    barrier.finish().unwrap();
    outer.finish().unwrap();
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}

#[test]
fn active_script_or_module_name_observes_strip_source_and_strip_debug() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let source_stripped =
        push_named_script_active_frame(&runtime, &mut context, "source-stripped.js");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("source-stripped.js"))
    );
    source_stripped.finish().unwrap();

    runtime.set_debug_info_mode(DebugInfoMode::Full);
    let outer = push_named_script_active_frame(&runtime, &mut context, "debug-caller.js");
    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let debug_stripped =
        push_named_script_active_frame(&runtime, &mut context, "debug-stripped.js");
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        None,
        "a non-eval bytecode frame without debug data is terminal"
    );
    debug_stripped.finish().unwrap();
    assert_eq!(
        runtime.active_script_or_module_name().unwrap(),
        Some(JsString::from_static("debug-caller.js"))
    );
    outer.finish().unwrap();
}

#[test]
fn backtrace_capture_respects_own_stack_and_real_error_class() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let error_constructor = global_callable(&runtime, &mut context, "Error");
    let Value::Object(error) = context
        .call(&error_constructor, Value::Undefined, &[])
        .unwrap()
    else {
        panic!("Error() did not return an object");
    };
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert!(
        runtime
            .define_own_property(
                &error,
                &stack_key,
                &data_descriptor(Value::Undefined, true, false, true),
            )
            .unwrap()
    );

    let held_key = runtime.intern_property_key("heldError").unwrap();
    let global = context.global_object().unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &held_key,
                &data_descriptor(Value::Object(error.clone()), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        context.eval_with_filename("throw heldError", "throw.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(first_throw) = context.take_exception().unwrap().unwrap() else {
        panic!("held Error throw lost its object identity");
    };
    assert_eq!(first_throw, error);
    assert_eq!(
        own_data_value(&runtime, &first_throw, "stack"),
        Value::Undefined
    );

    assert!(runtime.delete_property(&error, &stack_key).unwrap());
    assert!(matches!(
        context.eval_with_filename("throw heldError", "throw.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(second_throw) = context.take_exception().unwrap().unwrap() else {
        panic!("rethrow lost its Error object");
    };
    assert_eq!(second_throw, error);
    assert_eq!(
        own_stack_string(&runtime, &second_throw),
        JsString::from_static("    at <eval> (throw.js:1:1)\n")
    );

    let Value::Object(error_prototype) =
        own_data_value(&runtime, error_constructor.as_object(), "prototype")
    else {
        panic!("Error.prototype was not an object");
    };
    let spoof = context
        .new_object_with_prototype(Some(&error_prototype))
        .unwrap();
    let spoof_key = runtime.intern_property_key("spoofError").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &spoof_key,
                &data_descriptor(Value::Object(spoof.clone()), true, true, true),
            )
            .unwrap()
    );
    assert!(matches!(
        context.eval("throw spoofError"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(thrown_spoof) = context.take_exception().unwrap().unwrap() else {
        panic!("ordinary spoof throw lost its object");
    };
    assert_eq!(thrown_spoof, spoof);
    assert!(!runtime.has_own_property(&spoof, &stack_key).unwrap());
}

#[test]
fn backtrace_function_name_lookup_is_raw_and_only_one_prototype_deep() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = "(function leaf(){ return 1n + 1; })";
    let Value::Object(leaf_object) = context.eval_with_filename(source, "name.js").unwrap() else {
        panic!("leaf function was not an object");
    };
    let leaf = runtime.as_callable(&leaf_object).unwrap().unwrap();
    let Value::Object(getter_object) = context
        .eval("(function nameGetter(){ throw \"name getter ran\"; })")
        .unwrap()
    else {
        panic!("name getter was not an object");
    };
    let getter = runtime.as_callable(&getter_object).unwrap().unwrap();
    let name_key = runtime.intern_property_key("name").unwrap();
    assert!(
        runtime
            .define_own_property(
                leaf.as_object(),
                &name_key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
    assert!(matches!(
        context.call(&leaf, Value::Undefined, &[]),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("leaf TypeError was replaced by the name getter");
    };
    let plus_column = source.find('+').unwrap() + 1;
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::try_from_utf8(&format!("    at <anonymous> (name.js:1:{plus_column})\n"))
            .unwrap()
    );

    for (name, expected) in [
        (
            JsString::try_from_utf16([u16::from(b'a'), 0, u16::from(b'b')]).unwrap(),
            "a",
        ),
        (
            JsString::try_from_utf16([0, u16::from(b'a'), u16::from(b'b')]).unwrap(),
            "<anonymous>",
        ),
    ] {
        assert!(
            runtime
                .define_own_property(
                    leaf.as_object(),
                    &name_key,
                    &data_descriptor(Value::String(name), false, false, true),
                )
                .unwrap()
        );
        assert!(matches!(
            context.call(&leaf, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("renamed leaf TypeError was not an object");
        };
        assert_eq!(
            own_stack_string(&runtime, &error),
            JsString::try_from_utf8(&format!("    at {expected} (name.js:1:{plus_column})\n"))
                .unwrap()
        );
    }
}

#[test]
fn cross_realm_backtrace_uses_each_bytecode_filename_and_throwing_realm_error() {
    let runtime = Runtime::new();
    let mut realm_a = runtime.new_context();
    let mut realm_b = runtime.new_context();
    let source_a = "(function inA(){ return 1n + 1; })";
    let Value::Object(in_a) = realm_a.eval_with_filename(source_a, "a.js").unwrap() else {
        panic!("realm A function was not an object");
    };
    let global_b = realm_b.global_object().unwrap();
    let in_a_key = runtime.intern_property_key("inA").unwrap();
    assert!(
        realm_b
            .define_own_property(
                &global_b,
                &in_a_key,
                &data_descriptor(Value::Object(in_a), true, true, true),
            )
            .unwrap()
    );

    let source_b = "(function inB(){ return inA(); })()";
    assert!(matches!(
        realm_b.eval_with_filename(source_b, "b.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = realm_b.take_exception().unwrap().unwrap() else {
        panic!("cross-realm TypeError was not an object");
    };
    let plus_column = source_a.find('+').unwrap() + 1;
    let return_column = source_b.find("return").unwrap() + 1;
    let root_call_column = source_b.rfind("()").unwrap() + 1;
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::try_from_utf8(&format!(
            "    at inA (a.js:1:{plus_column})\n    at inB (b.js:1:{return_column})\n    at <eval> (b.js:1:{root_call_column})\n"
        ))
        .unwrap()
    );

    let type_error_a = global_callable(&runtime, &mut realm_a, "TypeError");
    let Value::Object(type_error_prototype_a) =
        own_data_value(&runtime, type_error_a.as_object(), "prototype")
    else {
        panic!("realm A TypeError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(type_error_prototype_a)
    );
}
