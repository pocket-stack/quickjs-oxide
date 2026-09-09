use super::*;

#[test]
fn dynamic_import_load_and_finish_are_distinct_fifo_jobs_with_gc_roots() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, loads, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        "export const answer = 42; globalThis.__dynamicImportBodyRan = true;",
    )]);
    let _registration = runtime.set_module_loader(loader);

    let promise = eval_dynamic_import(&mut context, "import('./dependency.js')", "pkg/entry.js");
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    runtime.run_gc().unwrap();

    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(loads.borrow().as_slice(), ["pkg/dependency.js"]);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(
        runtime.is_job_pending(),
        "load did not enqueue the finish reaction"
    );
    assert_script_true(&mut context, "globalThis.__dynamicImportBodyRan === true");
    runtime.run_gc().unwrap();

    assert!(runtime.execute_pending_job().unwrap().executed());
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    let Value::Object(namespace) = runtime.root_raw_value(&snapshot.result).unwrap() else {
        panic!("dynamic import did not fulfill with a namespace object");
    };
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        runtime
            .get_property_in_realm(context.realm, &namespace, &answer)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn dynamic_import_waits_for_a_pending_tla_evaluation_and_reuses_it() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"
            globalThis.__dynamicTlaLog = [];
            globalThis.__dynamicTlaGate = new Promise(function (resolve) {
                globalThis.__releaseDynamicTlaGate = resolve;
            });
            "#,
        )
        .unwrap();
    let (loader, loads, _) = MapModuleLoader::new([(
        "pkg/wait.js",
        r#"
        globalThis.__dynamicTlaLog.push("start");
        await globalThis.__dynamicTlaGate;
        globalThis.__dynamicTlaLog.push("end");
        export const answer = 42;
        "#,
    )]);
    let _registration = runtime.set_module_loader(loader);

    let first = eval_dynamic_import(
        &mut context,
        "globalThis.__firstWaitingImport = import('./wait.js'); __firstWaitingImport",
        "pkg/entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(loads.borrow().as_slice(), ["pkg/wait.js"]);
    assert_eq!(
        promise_snapshot(&runtime, &first).state,
        PromiseState::Pending
    );
    assert_script_true(
        &mut context,
        "globalThis.__dynamicTlaLog.join(',') === 'start'",
    );
    assert!(
        !runtime.is_job_pending(),
        "an unresolved TLA gate left a runnable job"
    );

    let second = eval_dynamic_import(
        &mut context,
        "globalThis.__secondWaitingImport = import('./wait.js'); __secondWaitingImport",
        "pkg/entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(loads.borrow().as_slice(), ["pkg/wait.js"]);
    assert_eq!(
        promise_snapshot(&runtime, &first).state,
        PromiseState::Pending
    );
    assert_eq!(
        promise_snapshot(&runtime, &second).state,
        PromiseState::Pending
    );
    assert!(
        !runtime.is_job_pending(),
        "a cached pending evaluation left a runnable job"
    );

    runtime.run_gc().unwrap();
    context
        .eval("globalThis.__releaseDynamicTlaGate()")
        .unwrap();
    assert!(drain_jobs(&runtime) > 0);

    let first = promise_snapshot(&runtime, &first);
    let second = promise_snapshot(&runtime, &second);
    assert_eq!(first.state, PromiseState::Fulfilled);
    assert_eq!(second.state, PromiseState::Fulfilled);
    let Value::Object(first_namespace) = runtime.root_raw_value(&first.result).unwrap() else {
        panic!("first dynamic import did not fulfill with a namespace object");
    };
    let Value::Object(second_namespace) = runtime.root_raw_value(&second.result).unwrap() else {
        panic!("second dynamic import did not fulfill with a namespace object");
    };
    assert_eq!(first_namespace.object_id(), second_namespace.object_id());
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        runtime
            .get_property_in_realm(context.realm, &first_namespace, &answer)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_script_true(
        &mut context,
        "globalThis.__dynamicTlaLog.join(',') === 'start,end'",
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn dynamic_import_assimilates_a_namespace_then_export() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, _, _) = MapModuleLoader::new([(
        "thenable.js",
        "export function then(resolve) { resolve(42); }",
    )]);
    let _registration = runtime.set_module_loader(loader);
    let promise = eval_dynamic_import(&mut context, "import('thenable.js')", "entry.js");

    assert!(runtime.execute_pending_job().unwrap().executed());
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(
        runtime.is_job_pending(),
        "namespace then was not assimilated"
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    assert_eq!(
        runtime.root_raw_value(&snapshot.result).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn dynamic_import_internal_then_observes_species_and_ignored_capability() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, _, _) = MapModuleLoader::new([("species.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    context
        .eval(
            r#"
globalThis.__dynamicSpeciesLog = "";
Object.defineProperty(Promise, Symbol.species, {
configurable: true,
get: function () {
    __dynamicSpeciesLog += "species,";
    return function (executor) {
        __dynamicSpeciesLog += "constructor,";
        executor(
            function () { __dynamicSpeciesLog += "resolve,"; },
            function () { __dynamicSpeciesLog += "reject,"; }
        );
        return { ignored: true };
    };
}
});
"#,
        )
        .unwrap();
    let promise = eval_dynamic_import(&mut context, "import('species.js')", "entry.js");
    assert_script_true(&mut context, "__dynamicSpeciesLog === ''");

    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_script_true(
        &mut context,
        "__dynamicSpeciesLog === 'species,constructor,'",
    );
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_script_true(
        &mut context,
        "__dynamicSpeciesLog === 'species,constructor,resolve,'",
    );
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
}

#[test]
fn dynamic_import_discards_internal_then_species_abrupt_completion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, _, _) = MapModuleLoader::new([("species-throw.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    context
        .eval(
            r#"
globalThis.__dynamicSpeciesThrowLog = "";
Object.defineProperty(Promise, Symbol.species, {
configurable: true,
get: function () {
    __dynamicSpeciesThrowLog += "species,";
    throw 73;
}
});
"#,
        )
        .unwrap();
    let promise = eval_dynamic_import(&mut context, "import('species-throw.js')", "entry.js");

    assert!(runtime.execute_pending_job().unwrap().executed());
    assert_script_true(&mut context, "__dynamicSpeciesThrowLog === 'species,'");
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(!runtime.is_job_pending());
    assert!(context.has_exception());
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(73)));
}
