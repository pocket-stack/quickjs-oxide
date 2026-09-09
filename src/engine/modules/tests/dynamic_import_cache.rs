use super::*;
use std::cell::RefCell;

#[test]
fn dynamic_import_load_job_samples_the_current_loader() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (first_loader, first_loads, _) =
        MapModuleLoader::new([("sampled.js", "export const source = 1;")]);
    let _first_registration = runtime.set_module_loader(first_loader);
    let promise = eval_dynamic_import(&mut context, "import('sampled.js')", "entry.js");

    let (second_loader, second_loads, _) =
        MapModuleLoader::new([("sampled.js", "export const source = 2;")]);
    let _second_registration = runtime.set_module_loader(second_loader);
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert!(first_loads.borrow().is_empty());
    assert_eq!(second_loads.borrow().as_slice(), ["sampled.js"]);
    assert!(runtime.execute_pending_job().unwrap().executed());

    let snapshot = promise_snapshot(&runtime, &promise);
    let Value::Object(namespace) = runtime.root_raw_value(&snapshot.result).unwrap() else {
        panic!("sampled dynamic import did not return a namespace");
    };
    let source = runtime.intern_property_key("source").unwrap();
    assert_eq!(
        runtime
            .get_property_in_realm(context.realm, &namespace, &source)
            .unwrap(),
        Completion::Return(Value::Int(2))
    );
}

#[test]
fn dynamic_import_load_samples_replacement_installed_by_normalize() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (replacement, replacement_loads, _) =
        MapModuleLoader::new([("pkg/value.js", "export const value = 42;")]);
    let initial_normalizations = Rc::new(RefCell::new(Vec::new()));
    let initial_loads = Rc::new(RefCell::new(Vec::new()));
    let loader = NormalizeReplacingModuleLoader {
        runtime: runtime.clone(),
        replacement: RefCell::new(Some(replacement)),
        replacement_registration: RefCell::new(None),
        normalizations: initial_normalizations.clone(),
        loads: initial_loads.clone(),
    };
    let _loader_registration = runtime.set_module_loader(loader);
    let promise = eval_dynamic_import(&mut context, "import('./value.js')", "pkg/entry.js");

    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(initial_normalizations.borrow().len(), 1);
    assert!(initial_loads.borrow().is_empty());
    assert_eq!(replacement_loads.borrow().as_slice(), &["pkg/value.js"]);
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
}

#[test]
fn dynamic_import_resolution_failure_retries_the_acyclic_source_graph() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, loads, _) = MapModuleLoader::new([("pkg/a.js", "import './missing.js';")]);
    let _loader_registration = runtime.set_module_loader(loader);

    for _ in 0..2 {
        let promise = eval_dynamic_import(&mut context, "import('./a.js')", "pkg/entry.js");
        assert!(runtime.execute_pending_job().unwrap().executed());
        assert_eq!(
            promise_snapshot(&runtime, &promise).state,
            PromiseState::Rejected
        );
        assert!(!runtime.is_job_pending());
    }

    assert_eq!(
        loads.borrow().as_slice(),
        &["pkg/a.js", "pkg/missing.js", "pkg/a.js", "pkg/missing.js"]
    );
}

#[test]
fn dynamic_import_reuses_cycle_root_rejection_promise_and_tracker_history() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, loads, _) = MapModuleLoader::new([
        ("cycle-a.js", "import 'cycle-b.js'; export const a = 1;"),
        ("cycle-b.js", "import 'cycle-a.js'; throw 42;"),
    ]);
    let _registration = runtime.set_module_loader(loader);
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    runtime.set_host_promise_rejection_tracker(move |event| {
        captured.borrow_mut().push((
            event.is_handled(),
            event.promise().object_id(),
            event.reason().clone(),
        ));
    });

    let first = eval_dynamic_import(
        &mut context,
        "globalThis.__cycleFirst = import('cycle-a.js'); __cycleFirst.catch(function () {}); __cycleFirst",
        "entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    {
        let events = events.borrow();
        assert_eq!(events.len(), 3);
        assert!(!events[0].0);
        assert!(!events[1].0);
        assert!(events[2].0);
        assert_ne!(events[0].1, events[1].1);
        assert_eq!(events[1].1, events[2].1);
        assert_eq!(events[0].2, Value::Int(42));
        assert_eq!(events[1].2, Value::Int(42));
        assert_eq!(events[2].2, Value::Int(42));
    }
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &first).state,
        PromiseState::Rejected
    );
    assert!(
        runtime.execute_pending_job().unwrap().executed(),
        "first catch reaction was missing"
    );
    assert!(!runtime.is_job_pending());

    let (cycle_a, cycle_b, root_promise) = {
        let state = runtime.0.state.borrow();
        let cycle_a = state
            .heap
            .first_loaded_module(context.realm, &JsString::from_static("cycle-a.js"))
            .unwrap()
            .unwrap();
        let cycle_b = state
            .heap
            .first_loaded_module(context.realm, &JsString::from_static("cycle-b.js"))
            .unwrap()
            .unwrap();
        let a = state.heap.loaded_module(cycle_a).unwrap();
        let b = state.heap.loaded_module(cycle_b).unwrap();
        assert_eq!(a.evaluation_cycle_root, Some(cycle_a.module));
        assert_eq!(b.evaluation_cycle_root, Some(cycle_a.module));
        assert!(b.evaluation_promise.is_none());
        (cycle_a, cycle_b, a.evaluation_promise.unwrap())
    };
    assert_ne!(cycle_a, cycle_b);

    let second = eval_dynamic_import(
        &mut context,
        "globalThis.__cycleSecond = import('cycle-b.js'); __cycleSecond.catch(function () {}); __cycleSecond",
        "entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &second).state,
        PromiseState::Rejected
    );
    assert!(!runtime.is_job_pending());
    assert_eq!(
        events.borrow().len(),
        3,
        "cached handled rejection retracked"
    );
    assert_eq!(loads.borrow().len(), 2, "cycle cache reloaded source text");
    assert_eq!(
        runtime.module_record(cycle_a).unwrap().evaluation_promise,
        Some(root_promise)
    );
    assert!(
        runtime
            .module_record(cycle_b)
            .unwrap()
            .evaluation_promise
            .is_none()
    );
}

#[test]
fn dynamic_import_successful_cycle_reuses_one_evaluation_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, loads, _) = MapModuleLoader::new([
        (
            "ok-cycle-a.js",
            "import 'ok-cycle-b.js'; export const a = 1;",
        ),
        (
            "ok-cycle-b.js",
            "import 'ok-cycle-a.js'; export const b = 2;",
        ),
    ]);
    let _registration = runtime.set_module_loader(loader);

    let first = eval_dynamic_import(&mut context, "import('ok-cycle-a.js')", "entry.js");
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &first).state,
        PromiseState::Fulfilled
    );
    let (a, b, root_promise) = {
        let state = runtime.0.state.borrow();
        let a = state
            .heap
            .first_loaded_module(context.realm, &JsString::from_static("ok-cycle-a.js"))
            .unwrap()
            .unwrap();
        let b = state
            .heap
            .first_loaded_module(context.realm, &JsString::from_static("ok-cycle-b.js"))
            .unwrap()
            .unwrap();
        let a_record = state.heap.loaded_module(a).unwrap();
        let b_record = state.heap.loaded_module(b).unwrap();
        assert_eq!(a_record.evaluation_cycle_root, Some(a.module));
        assert_eq!(b_record.evaluation_cycle_root, Some(a.module));
        assert!(b_record.evaluation_promise.is_none());
        (a, b, a_record.evaluation_promise.unwrap())
    };

    let second = eval_dynamic_import(&mut context, "import('ok-cycle-b.js')", "entry.js");
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &second).state,
        PromiseState::Fulfilled
    );
    assert_eq!(loads.borrow().len(), 2);
    assert_eq!(
        runtime.module_record(a).unwrap().evaluation_promise,
        Some(root_promise)
    );
    assert!(
        runtime
            .module_record(b)
            .unwrap()
            .evaluation_promise
            .is_none()
    );
}

#[test]
fn static_and_dynamic_entrypoints_share_the_cached_evaluation_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let static_module = context
        .compile_module_with_filename("export const value = 42;", "pkg/static.js")
        .unwrap();
    let static_result = module_evaluation_promise(&mut context, &static_module);
    let static_promise = runtime
        .module_record(static_module.raw)
        .unwrap()
        .evaluation_promise
        .unwrap();
    assert_eq!(static_result.object_id(), static_promise);

    let imported = eval_dynamic_import(&mut context, "import('./static.js')", "pkg/entry.js");
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &imported).state,
        PromiseState::Fulfilled
    );
    assert_eq!(
        runtime
            .module_record(static_module.raw)
            .unwrap()
            .evaluation_promise,
        Some(static_promise)
    );

    let (loader, _, _) =
        MapModuleLoader::new([("pkg/dynamic-first.js", "export const value = 7;")]);
    let _registration = runtime.set_module_loader(loader);
    let dynamic_first =
        eval_dynamic_import(&mut context, "import('./dynamic-first.js')", "pkg/entry.js");
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &dynamic_first).state,
        PromiseState::Fulfilled
    );
    let raw = runtime
        .0
        .state
        .borrow()
        .heap
        .first_loaded_module(
            context.realm,
            &JsString::from_static("pkg/dynamic-first.js"),
        )
        .unwrap()
        .unwrap();
    let dynamic_promise = runtime
        .module_record(raw)
        .unwrap()
        .evaluation_promise
        .unwrap();
    let handle = runtime.root_module(raw).unwrap();
    assert_eq!(
        module_evaluation_promise(&mut context, &handle).object_id(),
        dynamic_promise
    );
    assert_eq!(
        runtime.module_record(raw).unwrap().evaluation_promise,
        Some(dynamic_promise)
    );
}

#[test]
fn static_throw_then_cached_dynamic_import_preserves_both_promise_histories() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let reason = context
        .eval("globalThis.__sharedModuleReason = {}; __sharedModuleReason")
        .unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    runtime.set_host_promise_rejection_tracker(move |event| {
        captured.borrow_mut().push((
            event.is_handled(),
            event.promise().object_id(),
            event.reason().clone(),
        ));
    });

    let module = context
        .compile_module_with_filename(
            "throw globalThis.__sharedModuleReason;",
            "pkg/shared-throw.js",
        )
        .unwrap();
    let evaluation = module_evaluation_promise(&mut context, &module);
    let evaluation_snapshot = promise_snapshot(&runtime, &evaluation);
    assert_eq!(evaluation_snapshot.state, PromiseState::Rejected);
    assert_eq!(
        runtime.root_raw_value(&evaluation_snapshot.result).unwrap(),
        reason
    );
    {
        let events = events.borrow();
        assert_eq!(events.len(), 2);
        assert!(!events[0].0, "module-body Promise was already handled");
        assert!(!events[1].0, "evaluation Promise was already handled");
        assert_ne!(events[0].1, events[1].1);
        assert_eq!(events[0].2, reason);
        assert_eq!(events[1].2, reason);
    }
    assert_eq!(context.take_exception().unwrap(), None);
    assert_eq!(events.borrow().len(), 2);

    let imported = eval_dynamic_import(
        &mut context,
        "globalThis.__cachedThrowImport = import('./shared-throw.js'); __cachedThrowImport.catch(function () {}); __cachedThrowImport",
        "pkg/entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    {
        let events = events.borrow();
        assert_eq!(events.len(), 3);
        assert!(events[2].0);
        assert_eq!(events[2].1, events[1].1);
        assert_ne!(events[2].1, events[0].1);
        assert_eq!(events[2].2, reason);
    }
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &imported).state,
        PromiseState::Rejected
    );
    assert!(!runtime.is_job_pending());
    assert_eq!(events.borrow().len(), 3);
}
