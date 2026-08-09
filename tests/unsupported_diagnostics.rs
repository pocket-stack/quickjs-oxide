use quickjs_oxide::{ErrorKind, Runtime, RuntimeError, Value};

mod support;

use support::compile_syntax_error;

fn assert_unsupported<T>(
    label: &str,
    result: Result<T, RuntimeError>,
    context: &mut quickjs_oxide::Context,
    expected_message: &str,
) {
    let Err(RuntimeError::Engine(error)) = result else {
        panic!("{label} did not retain the Unsupported engine diagnostic");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(error.message(), expected_message);
    assert!(context.take_exception().unwrap().is_none());
}

#[test]
fn public_and_conformance_entrypoints_share_unsupported_semantics() {
    const IMPORT_SOURCE: &str = "import('dependency')";
    const IMPORT_MESSAGE: &str = "import syntax is not implemented yet";

    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let result = context.compile(IMPORT_SOURCE);
    assert_unsupported("Context::compile", result, &mut context, IMPORT_MESSAGE);

    let result = context.eval(IMPORT_SOURCE);
    assert_unsupported("Context::eval", result, &mut context, IMPORT_MESSAGE);

    let result = context.eval(r#"Function("import('dependency')")"#);
    assert_unsupported("Function constructor", result, &mut context, IMPORT_MESSAGE);

    context.install_test262_host().unwrap();
    let result = context.eval(r#"$262.evalScript("import('dependency')")"#);
    assert_unsupported("$262.evalScript", result, &mut context, IMPORT_MESSAGE);

    let result = context.compile_module("await 1;");
    assert_unsupported(
        "Context::compile_module",
        result,
        &mut context,
        "top-level await is not implemented in this synchronous module slice",
    );
}

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
        assert_eq!(
            compile_syntax_error(source),
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
        assert_eq!(compile_syntax_error(source), expected, "{source}");
    }
}
