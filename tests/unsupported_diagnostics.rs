use quickjs_oxide::{ErrorKind, Runtime, Value};

#[test]
fn context_compiles_and_executes_catch_destructuring_bindings() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let bytecode = context
        .compile("try { throw {value: 42}; } catch ({value}) { value }")
        .unwrap();
    assert_eq!(context.execute(&bytecode).unwrap(), Value::Int(42));
}

#[test]
fn object_rest_bindings_preserve_lexical_conflict_diagnostics() {
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
fn object_rest_bindings_preserve_later_source_error_priority() {
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
