use super::*;

#[test]
fn eval_uses_the_compiler_and_vm_path() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval("6 * 7").unwrap(), Value::Int(42));
    assert_eq!(
        context.eval("this").unwrap(),
        Value::Object(context.global_object().unwrap())
    );
}

#[test]
fn raw_script_context_apis_compile_and_evaluate() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    for function in [
        context.compile_bytes(b"6 * 7").unwrap(),
        context
            .compile_bytes_with_filename(b"40 + 2", "raw-filename.js")
            .unwrap(),
        context
            .compile_bytes_with_options(b"41 + 1", &CompileOptions::new("raw-options.js"))
            .unwrap(),
    ] {
        assert_eq!(context.execute(&function).unwrap(), Value::Int(42));
    }

    assert_eq!(context.eval_bytes(b"6 * 7").unwrap(), Value::Int(42));
    assert_eq!(
        context
            .eval_bytes_with_filename(b"40 + 2", "raw-filename.js")
            .unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context
            .eval_bytes_with_options(b"41 + 1", &EvalOptions::new("raw-options.js"))
            .unwrap(),
        Value::Int(42)
    );
}

#[test]
fn raw_script_syntax_error_uses_authored_bytes_and_filename() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert!(matches!(
        context.eval_bytes_with_filename(b"/*\x80*/@", "raw-parse.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("raw SyntaxError was not an object");
    };
    assert_eq!(
        own_data_value(&runtime, &error, "fileName"),
        Value::String(JsString::from_static("raw-parse.js"))
    );
    assert_eq!(
        own_data_value(&runtime, &error, "lineNumber"),
        Value::Int(1)
    );
    assert_eq!(
        own_data_value(&runtime, &error, "columnNumber"),
        Value::Int(5)
    );
    assert_eq!(
        own_stack_string(&runtime, &error),
        JsString::from_static("    at raw-parse.js:1:5\n")
    );
}

#[test]
fn raw_script_function_source_respects_debug_info_mode() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let to_string = property_callable(&runtime, &mut context, &function_prototype, "toString");
    let raw = b"(function raw(){/*\x80X*/return 42;})";

    let Value::Object(full) = context
        .eval_bytes_with_filename(raw, "raw-full.js")
        .unwrap()
    else {
        panic!("raw full-debug source did not return a function");
    };
    let expected = JsString::try_from_bytes(b"function raw(){/*\x80X*/return 42;}").unwrap();
    assert_eq!(
        context
            .call(&to_string, Value::Object(full.clone()), &[])
            .unwrap(),
        Value::String(expected.clone())
    );
    let expected_units = expected.utf16_units().collect::<Vec<_>>();
    assert!(expected_units.contains(&0xfffd));
    assert!(!expected_units.contains(&u16::from(b'X')));

    runtime.set_debug_info_mode(DebugInfoMode::StripSource);
    let Value::Object(source_stripped) = context
        .eval_bytes_with_filename(raw, "raw-source-stripped.js")
        .unwrap()
    else {
        panic!("raw source-stripped source did not return a function");
    };
    let file_name = runtime.intern_property_key("fileName").unwrap();
    assert_eq!(
        context.get_property(&source_stripped, &file_name).unwrap(),
        Value::String(JsString::from_static("raw-source-stripped.js"))
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(source_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function raw() {\n    [native code]\n}"
        ))
    );

    runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
    let Value::Object(debug_stripped) = context
        .eval_bytes_with_filename(raw, "raw-debug-stripped.js")
        .unwrap()
    else {
        panic!("raw debug-stripped source did not return a function");
    };
    assert_eq!(
        context.get_property(&debug_stripped, &file_name).unwrap(),
        Value::Undefined
    );
    assert_eq!(
        context
            .call(&to_string, Value::Object(debug_stripped), &[])
            .unwrap(),
        Value::String(JsString::from_static(
            "function raw() {\n    [native code]\n}"
        ))
    );
}
