use super::*;

#[test]
fn named_function_expression_has_intrinsic_name_and_private_recursive_binding() {
    assert_eq!(
        evaluate_in_context("(function fact(n) { return n ? n * fact(n - 1) : 1; })(5)"),
        Value::Int(120)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(f) { return f() === f; })(function anonymous() { return anonymous; })"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        evaluate_in_context("(function named(named) { return named; })(42)"),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context("(function named() { var named = 42; return named; })()"),
        Value::Int(42)
    );
    assert_eq!(
        evaluate_in_context("(function named() {}), typeof named"),
        Value::String(JsString::from_static("undefined"))
    );

    let (name, writable, enumerable, configurable) =
        evaluate_function_name("(function named() {})");
    assert_eq!(name, JsString::from_static("named"));
    assert!(!writable);
    assert!(!enumerable);
    assert!(configurable);

    let (name, ..) = evaluate_function_name(
        "(function() { var inferred = function intrinsic() {}; return inferred; })()",
    );
    assert_eq!(name, JsString::from_static("intrinsic"));

    let script = compile_unlinked_script("(function unusedName() { return 1; })").unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().function_name_local, None);
    assert_eq!(function.metadata().local_count, 0);

    let script = compile_unlinked_script("(function self() { return self; })").unwrap();
    let function = script.constants()[0].as_child().unwrap();
    assert_eq!(function.metadata().function_name_local, Some(0));
    assert_eq!(function.metadata().local_count, 1);
}

#[test]
fn named_function_self_binding_captures_through_relays_and_is_per_instance() {
    let source = "(function(f) { return f()()() === f; })(function named() { return function() { return function() { return named; }; }; })";
    assert_eq!(evaluate_in_context(source), Value::Bool(true));
    assert_eq!(
        evaluate_in_context(
            "(function() { var make = function() { return function named() { return named; }; }; var a = make(), b = make(); return a() === a && b() === b && a !== b; })()"
        ),
        Value::Bool(true)
    );

    let script = compile_unlinked_script(
        "(function named() { return function() { return function() { return named; }; }; })",
    )
    .unwrap();
    let named = script.constants()[0].as_child().unwrap();
    let relay = named.constants()[0].as_child().unwrap();
    let inner = relay.constants()[0].as_child().unwrap();
    assert_eq!(named.metadata().function_name_local, Some(0));
    assert_eq!(named.metadata().local_count, 1);
    assert_eq!(relay.closure_variables().len(), 1);
    assert_eq!(
        relay.closure_variables()[0].source,
        ClosureSource::ParentLocal(0)
    );
    assert_eq!(
        relay.closure_variables()[0].kind,
        ClosureVariableKind::FunctionName
    );
    let ClosureVariableName::Constant(relay_name) = relay.closure_variables()[0].name else {
        panic!("function-name relay did not retain its source name");
    };
    assert_eq!(
        relay.constants()[usize::try_from(relay_name).unwrap()].as_primitive(),
        Some(&crate::engine::value::PrimitiveValue::String(
            JsString::from_static("named")
        ))
    );
    assert!(!relay.closure_variables()[0].is_const);
    assert_eq!(
        inner.closure_variables()[0].source,
        ClosureSource::ParentClosure(0)
    );
    assert_eq!(
        inner.closure_variables()[0].kind,
        ClosureVariableKind::FunctionName
    );
    let ClosureVariableName::Constant(inner_name) = inner.closure_variables()[0].name else {
        panic!("transitive function-name relay did not retain its source name");
    };
    assert_eq!(
        inner.constants()[usize::try_from(inner_name).unwrap()].as_primitive(),
        Some(&crate::engine::value::PrimitiveValue::String(
            JsString::from_static("named")
        ))
    );
}

#[test]
fn named_function_self_assignment_matches_quickjs_strict_and_sloppy_rules() {
    assert_eq!(
        evaluate_in_context("(function named() { return named = 1; })()"),
        Value::Int(1)
    );
    assert_eq!(
        evaluate_in_context(
            "(function(f) { return f() === f; })(function named() { named = 1; return named; })"
        ),
        Value::Bool(true)
    );
    // QuickJS carries JS_VAR_FUNCTION_NAME semantics from the defining
    // function through closure relays. A nested strict directive does not
    // turn a sloppy outer function-name binding into a throwing write.
    assert_eq!(
        evaluate_in_context(
            "(function(f) { return f()() === f; })(function named() { return function() { 'use strict'; named = 1; return named; }; })"
        ),
        Value::Bool(true)
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval("(function named() { 'use strict'; named = 1; })()"),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("strict function-name assignment did not materialize TypeError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("'named' is read-only"))
    );

    assert_eq!(
        context.eval("(function named() { 'use strict'; return function() { named = 1; }; })()()"),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("captured strict function-name assignment did not materialize TypeError");
    };
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("'named' is read-only"))
    );

    let strict = compile_unlinked_script(
        "(function named() { 'use strict'; return function() { return named; }; })",
    )
    .unwrap();
    let named = strict.constants()[0].as_child().unwrap();
    let child = named.constants()[1].as_child().unwrap();
    assert_eq!(
        child.closure_variables()[0].kind,
        ClosureVariableKind::FunctionName
    );
    assert!(child.closure_variables()[0].is_const);
    assert!(!child.closure_variables()[0].is_lexical);
}

#[test]
fn nested_capture_installs_parent_closure_relay_and_executes() {
    let source =
        "(function(a) { return function() { return function(b) { return a + b; }; }; })(20)()(22)";
    assert_eq!(evaluate_in_context(source), Value::Int(42));

    let script = compile_unlinked_script(source).unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let relay = outer.constants()[0].as_child().unwrap();
    let inner = relay.constants()[0].as_child().unwrap();

    assert!(outer.closure_variables().is_empty());
    assert_eq!(relay.closure_variables().len(), 1);
    assert_eq!(
        relay.closure_variables()[0].source,
        ClosureSource::ParentArgument(0)
    );
    assert_eq!(inner.closure_variables().len(), 1);
    assert_eq!(
        inner.closure_variables()[0].source,
        ClosureSource::ParentClosure(0)
    );
}

#[test]
fn function_local_var_capture_uses_parent_local_then_parent_closure() {
    let source = "(function() { var a = 20; return function() { return function(b) { return a + b; }; }; })()()(22)";
    assert_eq!(evaluate_in_context(source), Value::Int(42));

    let script = compile_unlinked_script(source).unwrap();
    let outer = script.constants()[0].as_child().unwrap();
    let relay = outer.constants()[0].as_child().unwrap();
    let inner = relay.constants()[0].as_child().unwrap();
    assert_eq!(outer.metadata().local_count, 1);
    assert_eq!(
        relay.closure_variables()[0].source,
        ClosureSource::ParentLocal(0)
    );
    assert_eq!(
        inner.closure_variables()[0].source,
        ClosureSource::ParentClosure(0)
    );
}

#[test]
fn ordinary_function_fallthrough_returns_undefined() {
    assert_eq!(
        evaluate_in_context("(function(a) { a; })(42)"),
        Value::Undefined
    );
}
