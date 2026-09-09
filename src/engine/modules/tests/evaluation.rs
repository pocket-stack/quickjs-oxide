use super::*;

#[test]
fn dependency_free_module_links_then_evaluates_with_module_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module(
            r#"
            globalThis.__moduleThis = this;
            globalThis.__moduleVarBefore = value;
            globalThis.__moduleFunctionBefore = answer();
            var value = 7;
            function answer() { return 42; }
            let lexical = 9;
            globalThis.__moduleResult = value + lexical + answer();
            "#,
        )
        .unwrap();

    let snapshot = module_evaluation_snapshot(&mut context, &module);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    assert_eq!(snapshot.result, RawValue::Undefined);
    assert_script_true(
        &mut context,
        r#"
        __moduleThis === undefined &&
        __moduleVarBefore === undefined &&
        __moduleFunctionBefore === 42 &&
        __moduleResult === 58 &&
        typeof value === "undefined" &&
        typeof lexical === "undefined" &&
        typeof answer === "undefined"
        "#,
    );
}

#[test]
fn module_identity_evaluates_once_and_caches_abrupt_completion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval("globalThis.__moduleRuns = 0").unwrap();
    let once = context
        .compile_module("globalThis.__moduleRuns += 1")
        .unwrap();
    context.execute_module(&once).unwrap();
    context.execute_module(&once).unwrap();
    assert_script_true(&mut context, "__moduleRuns === 1");

    let abrupt = context.compile_module("throw 42").unwrap();
    let first = module_evaluation_promise(&mut context, &abrupt);
    let first_snapshot = promise_snapshot(&runtime, &first);
    assert_eq!(first_snapshot.state, PromiseState::Rejected);
    assert_eq!(first_snapshot.result, RawValue::Int(42));
    let second = module_evaluation_promise(&mut context, &abrupt);
    assert_eq!(first.object_id(), second.object_id());
}

#[test]
fn module_evaluation_caches_error_object_identity_across_contexts() {
    let runtime = Runtime::new();
    let module = {
        let mut compilation_context = runtime.new_context();
        compilation_context
            .compile_module("throw new Error('cached module error')")
            .unwrap()
    };

    let first_error_id = {
        let mut first_context = runtime.new_context();
        let snapshot = module_evaluation_snapshot(&mut first_context, &module);
        assert_eq!(snapshot.state, PromiseState::Rejected);
        let RawValue::Object(error) = snapshot.result else {
            panic!("module evaluation did not reject with an Error object");
        };
        error
    };
    runtime.run_gc().unwrap();

    let mut second_context = runtime.new_context();
    let snapshot = module_evaluation_snapshot(&mut second_context, &module);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    let RawValue::Object(second_error) = snapshot.result else {
        panic!("cached module evaluation did not retain an Error object");
    };
    assert_eq!(second_error, first_error_id);
}

#[test]
fn module_evaluation_cache_owns_symbol_atoms_until_the_cache_dies() {
    let runtime = Runtime::new();
    let baseline_atoms = runtime.test_atom_count();
    let module = {
        let mut compilation_context = runtime.new_context();
        compilation_context
            .compile_module("throw Symbol('cached module symbol')")
            .unwrap()
    };

    let first_symbol = {
        let mut first_context = runtime.new_context();
        let snapshot = module_evaluation_snapshot(&mut first_context, &module);
        assert_eq!(snapshot.state, PromiseState::Rejected);
        let RawValue::Symbol(symbol) = snapshot.result else {
            panic!("module evaluation did not reject with a Symbol");
        };
        symbol
    };
    runtime.run_gc().unwrap();

    let second_symbol = {
        let mut second_context = runtime.new_context();
        let snapshot = module_evaluation_snapshot(&mut second_context, &module);
        assert_eq!(snapshot.state, PromiseState::Rejected);
        let RawValue::Symbol(symbol) = snapshot.result else {
            panic!("cached module evaluation did not retain a Symbol");
        };
        symbol
    };
    assert_eq!(second_symbol, first_symbol);

    drop(module);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn direct_eval_uses_module_live_cells_without_leaking_eval_var() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module(
            r#"
            let live = 1;
            eval("live = 42; var evalScoped = live + 1; globalThis.__evalScopedInside = evalScoped");
            globalThis.__moduleLiveAfterEval = live;
            globalThis.__evalScopedOutside = typeof evalScoped;
            "#,
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        r#"
        __moduleLiveAfterEval === 42 &&
        __evalScopedInside === 43 &&
        __evalScopedOutside === "undefined" &&
        typeof live === "undefined" &&
        typeof evalScoped === "undefined"
        "#,
    );
}

#[test]
fn nested_var_preserves_quickjs_module_function_redeclaration_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module(
            r#"
            { var answer; }
            function answer() { return 1; }
            function answer() { return 42; }
            globalThis.__moduleRedeclaredAnswer = answer();
            "#,
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__moduleRedeclaredAnswer === 42");
}
