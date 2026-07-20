use quickjs_oxide::{CompileOptions, ErrorKind, Runtime, RuntimeError};

#[test]
fn context_can_preserve_an_implementation_frontier_without_a_js_exception() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let options = CompileOptions::new("unsupported.js");
    let RuntimeError::Engine(error) = context
        .compile_with_options_preserving_unsupported_diagnostics(
            "let [{value}] = [{value: 1}];",
            &options,
        )
        .unwrap_err()
    else {
        panic!("diagnostic compilation did not retain its engine error");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(
        error.message(),
        "object destructuring bindings are not implemented yet"
    );
    assert!(context.take_exception().unwrap().is_none());
}

#[test]
fn default_context_compilation_keeps_the_temporary_js_compatibility_boundary() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .compile("let [{value}] = [{value: 1}];")
            .unwrap_err(),
        RuntimeError::Exception
    );
    assert!(context.take_exception().unwrap().is_some());
}

#[test]
fn detached_nested_function_limit_is_an_engine_frontier() {
    let error = quickjs_oxide::compiler::compile_script("function nested() {}").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().contains("requires runtime publication"));
}
