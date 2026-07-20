use quickjs_oxide::{CompileOptions, ErrorKind, Runtime, RuntimeError};

#[test]
fn context_can_preserve_an_implementation_frontier_without_a_js_exception() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let options = CompileOptions::new("unsupported.js");
    let RuntimeError::Engine(error) = context
        .compile_with_options_preserving_unsupported_diagnostics(
            "let {...rest} = {value: 1};",
            &options,
        )
        .unwrap_err()
    else {
        panic!("diagnostic compilation did not retain its engine error");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(
        error.message(),
        "object rest destructuring bindings are not implemented yet"
    );
    assert!(context.take_exception().unwrap().is_none());
}

#[test]
fn default_context_compilation_keeps_the_temporary_js_compatibility_boundary() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.compile("let {...rest} = {value: 1};").unwrap_err(),
        RuntimeError::Exception
    );
    assert!(context.take_exception().unwrap().is_some());
}

#[test]
fn object_rest_frontier_preserves_prior_lexical_conflict_diagnostics() {
    for source in [
        "let {value, ...value} = {value: 1};",
        "const {value, ...value} = {value: 1};",
        "let value; var {...value} = {};",
    ] {
        let error = quickjs_oxide::compiler::compile_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid redefinition of lexical identifier",
            "{source}"
        );
    }
}

#[test]
fn object_rest_frontier_is_deferred_behind_later_source_errors() {
    for (source, expected) in [
        ("let {...rest} = ;", "unexpected token in expression: ';'"),
        (
            "for (let {...rest} of ) {}",
            "unexpected token in expression: ')'",
        ),
        (
            "let {...value} = {}, value;",
            "invalid redefinition of lexical identifier",
        ),
        (
            "let {...value} = {}; let value;",
            "invalid redefinition of lexical identifier",
        ),
        (
            "function f(){ var {...value} = {}; let value; }",
            "invalid redefinition of a variable",
        ),
    ] {
        let error = quickjs_oxide::compiler::compile_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), expected, "{source}");
    }
}

#[test]
fn detached_nested_function_limit_is_an_engine_frontier() {
    let error = quickjs_oxide::compiler::compile_script("function nested() {}").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().contains("requires runtime publication"));
}
