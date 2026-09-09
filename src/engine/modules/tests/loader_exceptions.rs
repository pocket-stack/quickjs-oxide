use std::cell::{Cell, RefCell};

use super::*;

#[test]
fn module_loader_exception_values_are_not_wrapped_and_resolution_retries() {
    assert_static_loader_exception(
        AbruptLoaderPhase::Normalize,
        |runtime| Value::Object(runtime.new_object(None).unwrap()),
        "import { answer } from './dependency.js'; globalThis.__abruptRetry = answer;",
    );
    assert_static_loader_exception(
        AbruptLoaderPhase::CheckAttributes,
        |_| Value::Int(42),
        "import { answer } from './dependency.js' with { type: 'javascript' }; globalThis.__abruptRetry = answer;",
    );
    assert_static_loader_exception(
        AbruptLoaderPhase::Load,
        |runtime| {
            Value::Symbol(
                runtime
                    .new_symbol(Some(JsString::from_static("load-reason")))
                    .unwrap(),
            )
        },
        "import { answer } from './dependency.js'; globalThis.__abruptRetry = answer;",
    );
}

#[test]
fn dynamic_import_preserves_module_loader_exception_identity() {
    let runtime = Runtime::new();
    let reason = runtime.new_object(None).unwrap();
    let (loader, _, loads) =
        AbruptModuleLoader::new(AbruptLoaderPhase::Load, Value::Object(reason.clone()));
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let promise = eval_dynamic_import(&mut context, "import('./dependency.js')", "pkg/entry.js");

    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    assert_eq!(
        runtime.root_raw_value(&snapshot.result).unwrap(),
        Value::Object(reason)
    );
    assert_eq!(loads.borrow().as_slice(), ["pkg/dependency.js"]);
    assert!(!context.has_exception());
    assert!(!runtime.is_job_pending());
}

#[test]
fn dynamic_import_attribute_checker_preserves_exception_identity() {
    let runtime = Runtime::new();
    let reason = runtime.new_object(None).unwrap();
    let (loader, _, loads) = AbruptModuleLoader::new(
        AbruptLoaderPhase::CheckAttributes,
        Value::Object(reason.clone()),
    );
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let promise = eval_dynamic_import(
        &mut context,
        "import('./dependency.js', { with: { type: 'javascript' } })",
        "pkg/entry.js",
    );

    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    assert_eq!(
        runtime.root_raw_value(&snapshot.result).unwrap(),
        Value::Object(reason)
    );
    assert!(loads.borrow().is_empty());
    assert!(!context.has_exception());
    assert!(!runtime.is_job_pending());
}

#[test]
fn foreign_runtime_module_loader_exceptions_are_rejected_before_publication() {
    for phase in [
        AbruptLoaderPhase::Normalize,
        AbruptLoaderPhase::CheckAttributes,
        AbruptLoaderPhase::Load,
    ] {
        let runtime = Runtime::new();
        let foreign = Runtime::new().new_object(None).unwrap();
        let (loader, _, _) = AbruptModuleLoader::new(phase, Value::Object(foreign));
        let _registration = runtime.set_module_loader(loader);
        let mut context = runtime.new_context();
        let source = if phase == AbruptLoaderPhase::CheckAttributes {
            "import './dependency.js' with { type: 'javascript' };"
        } else {
            "import './dependency.js';"
        };

        assert!(matches!(
            context.compile_module_with_filename(source, "pkg/entry.js"),
            Err(RuntimeError::WrongRuntime("module loader exception"))
        ));
        assert!(!context.has_exception());
    }
}

#[test]
fn dependency_attribute_exception_rolls_back_the_resolution_graph_for_retry() {
    let runtime = Runtime::new();
    let reason = runtime.new_object(None).unwrap();
    let failing = Rc::new(Cell::new(true));
    let loads = Rc::new(RefCell::new(Vec::new()));
    let loader = DependencyAttributeAbruptLoader {
        exception: Value::Object(reason.clone()),
        failing: failing.clone(),
        loads: loads.clone(),
    };
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let source = "import './dependency.js'; globalThis.__dependencyAbruptRetry = 42;";

    assert!(matches!(
        context.compile_module_with_filename(source, "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::Object(reason))
    );
    assert_eq!(loads.borrow().as_slice(), ["pkg/dependency.js"]);

    failing.set(false);
    let module = context
        .compile_module_with_filename(source, "pkg/entry.js")
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__dependencyAbruptRetry === 42");
    assert_eq!(
        loads.borrow().as_slice(),
        ["pkg/dependency.js", "pkg/dependency.js", "pkg/leaf.js"]
    );
}
