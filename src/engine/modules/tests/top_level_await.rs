use std::cell::{Cell, RefCell};

use super::*;

#[test]
fn dependency_free_top_level_await_fulfills_the_evaluation_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval("globalThis.__tlaLog = []").unwrap();
    let module = context
        .compile_module(
            r#"
            globalThis.__tlaLog.push("start");
            const value = await 41;
            globalThis.__tlaLog.push("end:" + (value + 1));
            "#,
        )
        .unwrap();

    let promise = module_evaluation_promise(&mut context, &module);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert_script_true(&mut context, "globalThis.__tlaLog.join(',') === 'start'");

    assert!(drain_jobs(&runtime) > 0);
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    assert_eq!(
        runtime.root_raw_value(&snapshot.result).unwrap(),
        Value::Undefined
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaLog.join(',') === 'start,end:42'",
    );
    assert!(matches!(
        runtime.module_record(module.raw).unwrap().evaluation,
        ModuleEvaluationState::Evaluated
    ));

    let cached = module_evaluation_promise(&mut context, &module);
    assert_eq!(cached.object_id(), promise.object_id());
    assert!(!runtime.is_job_pending());
}

#[test]
fn async_dependency_does_not_block_a_sibling_but_delays_its_parent() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval("globalThis.__tlaOrder = []").unwrap();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/async.js",
            r#"
            globalThis.__asyncDependencyDone = false;
            globalThis.__tlaOrder.push("async:start");
            await 0;
            globalThis.__asyncDependencyDone = true;
            globalThis.__tlaOrder.push("async:end");
            export const answer = 42;
            "#,
        ),
        (
            "pkg/sibling.js",
            r#"
            globalThis.__tlaOrder.push("sibling");
            export const sawAsyncEnd = globalThis.__asyncDependencyDone;
            "#,
        ),
    ]);
    let _registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            r#"
            import { answer } from "./async.js";
            import { sawAsyncEnd } from "./sibling.js";
            globalThis.__tlaOrder.push("parent:" + answer + ":" + sawAsyncEnd);
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    let promise = module_evaluation_promise(&mut context, &module);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaOrder.join(',') === 'async:start,sibling'",
    );

    assert!(drain_jobs(&runtime) > 0);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaOrder.join(',') === 'async:start,sibling,async:end,parent:42:false'",
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn async_dependency_rejection_preserves_identity_and_skips_the_parent_body() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let reason = context
        .eval("globalThis.__tlaReason = {}; globalThis.__tlaReason")
        .unwrap();
    let (loader, _, _) =
        MapModuleLoader::new([("pkg/reject.js", "await 0; throw globalThis.__tlaReason;")]);
    let _registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            "import './reject.js'; globalThis.__tlaParentRan = true;",
            "pkg/entry.js",
        )
        .unwrap();
    let dependency = runtime.module_dependencies(&module).unwrap().remove(0);

    let promise = module_evaluation_promise(&mut context, &module);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(drain_jobs(&runtime) > 0);

    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    assert_eq!(runtime.root_raw_value(&snapshot.result).unwrap(), reason);
    assert_script_true(
        &mut context,
        "typeof globalThis.__tlaParentRan === 'undefined'",
    );
    for member in [&module, &dependency] {
        assert!(matches!(
            runtime.module_record(member.raw).unwrap().evaluation,
            ModuleEvaluationState::Errored(_)
        ));
    }

    let cached = module_evaluation_promise(&mut context, &module);
    assert_eq!(cached.object_id(), promise.object_id());
    let cached = promise_snapshot(&runtime, &cached);
    assert_eq!(cached.state, PromiseState::Rejected);
    assert_eq!(runtime.root_raw_value(&cached.result).unwrap(), reason);
    assert!(!runtime.is_job_pending());
}

#[test]
fn shared_async_dependency_rejects_evaluation_promises_in_forward_parent_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let reason = context
        .eval(
            r#"
            globalThis.__sharedBranchReason = {};
            globalThis.__sharedBranchGate = new Promise(function (_, reject) {
                globalThis.__rejectSharedBranchGate = reject;
            });
            globalThis.__sharedBranchReason;
            "#,
        )
        .unwrap();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/shared-branch.js",
        "await globalThis.__sharedBranchGate; export const value = 42;",
    )]);
    let _registration = runtime.set_module_loader(loader);
    let first = context
        .compile_module_with_filename(
            "import './shared-branch.js'; globalThis.__firstBranchRan = true;",
            "pkg/first-branch.js",
        )
        .unwrap();
    let second = context
        .compile_module_with_filename(
            "import './shared-branch.js'; globalThis.__secondBranchRan = true;",
            "pkg/second-branch.js",
        )
        .unwrap();
    let first_promise = module_evaluation_promise(&mut context, &first);
    let second_promise = module_evaluation_promise(&mut context, &second);
    assert_eq!(
        promise_snapshot(&runtime, &first_promise).state,
        PromiseState::Pending
    );
    assert_eq!(
        promise_snapshot(&runtime, &second_promise).state,
        PromiseState::Pending
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let first_promise_id = first_promise.object_id();
    let second_raw = second.raw;
    let second_was_pending = Rc::new(Cell::new(false));
    let captured_second_was_pending = second_was_pending.clone();
    let observing_runtime = runtime.clone();
    runtime.set_host_promise_rejection_tracker(move |event| {
        if !event.is_handled() && event.promise().object_id() == first_promise_id {
            captured_second_was_pending.set(matches!(
                observing_runtime
                    .module_record(second_raw)
                    .expect("reentrant rejection tracker lost the second parent")
                    .evaluation,
                ModuleEvaluationState::EvaluatingAsync
            ));
        }
        captured.borrow_mut().push((
            event.is_handled(),
            event.promise().object_id(),
            event.reason().clone(),
        ));
    });

    context
        .eval("globalThis.__rejectSharedBranchGate(globalThis.__sharedBranchReason)")
        .unwrap();
    assert!(drain_jobs(&runtime) > 0);

    assert_eq!(
        promise_snapshot(&runtime, &first_promise).state,
        PromiseState::Rejected
    );
    assert_eq!(
        promise_snapshot(&runtime, &second_promise).state,
        PromiseState::Rejected
    );
    assert_script_true(
        &mut context,
        "typeof globalThis.__firstBranchRan === 'undefined' && typeof globalThis.__secondBranchRan === 'undefined'",
    );
    assert_eq!(
        events.borrow().as_slice(),
        &[
            (false, first_promise.object_id(), reason.clone()),
            (false, second_promise.object_id(), reason),
        ]
    );
    assert!(
        second_was_pending.get(),
        "first rejection tracker callback observed the later parent already errored"
    );
    runtime.clear_host_promise_rejection_tracker();
    assert!(!runtime.is_job_pending());
}

#[test]
fn shared_tla_completion_executes_cross_linked_parents_in_callback_realm() {
    let runtime = Runtime::new();
    let mut first_context = runtime.new_context();
    first_context
        .eval(
            r#"
            globalThis.__crossRealmGate = new Promise(function (resolve) {
                globalThis.__releaseCrossRealmGate = resolve;
            });
            "#,
        )
        .unwrap();
    let dependency = first_context
        .compile_module_with_filename(
            "await globalThis.__crossRealmGate; export const value = 42;",
            "pkg/cross-realm-dependency.js",
        )
        .unwrap();
    let dependency_promise = module_evaluation_promise(&mut first_context, &dependency);
    assert_eq!(
        promise_snapshot(&runtime, &dependency_promise).state,
        PromiseState::Pending
    );

    let parent = first_context
        .compile_module_with_filename(
            "import './cross-realm-dependency.js'; throw 42;",
            "pkg/cross-realm-parent.js",
        )
        .unwrap();
    let async_parent = first_context
        .compile_module_with_filename(
            "import './cross-realm-dependency.js'; await 0;",
            "pkg/cross-realm-async-parent.js",
        )
        .unwrap();
    let first_realm = first_context.realm;
    let mut second_context = runtime.new_context();
    let parent_promise = module_evaluation_promise(&mut second_context, &parent);
    let async_parent_promise = module_evaluation_promise(&mut second_context, &async_parent);
    assert_eq!(
        promise_snapshot(&runtime, &parent_promise).state,
        PromiseState::Pending
    );
    assert_eq!(
        promise_snapshot(&runtime, &async_parent_promise).state,
        PromiseState::Pending
    );
    first_context
        .eval(
            r#"
            globalThis.__crossRealmSpecies = [];
            Object.defineProperty(Promise, Symbol.species, {
                configurable: true,
                get() {
                    globalThis.__crossRealmSpecies.push("A");
                    return Promise;
                },
            });
            "#,
        )
        .unwrap();
    second_context
        .eval(
            r#"
            globalThis.__crossRealmSpecies = [];
            Object.defineProperty(Promise, Symbol.species, {
                configurable: true,
                get() {
                    globalThis.__crossRealmSpecies.push("B");
                    return Promise;
                },
            });
            "#,
        )
        .unwrap();

    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    runtime.set_host_promise_rejection_tracker(move |event| {
        if !event.is_handled() {
            captured
                .borrow_mut()
                .push((event.context(), event.reason().clone()));
        }
    });
    first_context
        .eval("globalThis.__releaseCrossRealmGate()")
        .unwrap();
    assert!(drain_jobs(&runtime) > 0);

    assert_eq!(
        promise_snapshot(&runtime, &dependency_promise).state,
        PromiseState::Fulfilled
    );
    let parent_snapshot = promise_snapshot(&runtime, &parent_promise);
    assert_eq!(parent_snapshot.state, PromiseState::Rejected);
    assert_eq!(parent_snapshot.result, RawValue::Int(42));
    assert_eq!(
        promise_snapshot(&runtime, &async_parent_promise).state,
        PromiseState::Fulfilled
    );
    assert_eq!(
        events.borrow().as_slice(),
        &[(first_realm, Value::Int(42)), (first_realm, Value::Int(42)),]
    );
    assert_script_true(
        &mut first_context,
        "globalThis.__crossRealmSpecies.join(',') === 'A'",
    );
    assert_script_true(
        &mut second_context,
        "globalThis.__crossRealmSpecies.length === 0",
    );
    runtime.clear_host_promise_rejection_tracker();
    assert!(!runtime.is_job_pending());
}

#[test]
fn late_tla_fulfillment_does_not_overwrite_a_cached_sibling_rejection() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let reason = context
        .eval(
            r#"
            globalThis.__lateTlaLog = [];
            globalThis.__lateTlaReason = {};
            globalThis.__lateTlaGate = new Promise(function (resolve) {
                globalThis.__releaseLateTlaGate = resolve;
            });
            globalThis.__lateTlaReason;
            "#,
        )
        .unwrap();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/late-wait.js",
            r#"
            globalThis.__lateTlaLog.push("wait:start");
            await globalThis.__lateTlaGate;
            globalThis.__lateTlaLog.push("wait:end");
            "#,
        ),
        (
            "pkg/late-throw.js",
            r#"
            globalThis.__lateTlaLog.push("throw");
            throw globalThis.__lateTlaReason;
            "#,
        ),
    ]);
    let _registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            r#"
            import "./late-wait.js";
            import "./late-throw.js";
            globalThis.__lateTlaParentRan = true;
            "#,
            "pkg/late-entry.js",
        )
        .unwrap();
    let dependencies = runtime.module_dependencies(&module).unwrap();
    let waiting = dependencies[0].clone();
    let throwing = dependencies[1].clone();

    let promise = module_evaluation_promise(&mut context, &module);
    let initial = promise_snapshot(&runtime, &promise);
    assert_eq!(initial.state, PromiseState::Rejected);
    assert_eq!(runtime.root_raw_value(&initial.result).unwrap(), reason);
    assert_script_true(
        &mut context,
        "globalThis.__lateTlaLog.join(',') === 'wait:start,throw' && typeof globalThis.__lateTlaParentRan === 'undefined'",
    );
    assert!(matches!(
        runtime.module_record(waiting.raw).unwrap().evaluation,
        ModuleEvaluationState::EvaluatingAsync
    ));
    for member in [&module, &throwing] {
        let ModuleEvaluationState::Errored(raw_reason) =
            runtime.module_record(member.raw).unwrap().evaluation
        else {
            panic!("synchronous module failure was not cached on its active ancestor");
        };
        assert_eq!(runtime.root_raw_value(&raw_reason).unwrap(), reason);
    }

    context.eval("globalThis.__releaseLateTlaGate()").unwrap();
    assert!(drain_jobs(&runtime) > 0);

    assert_script_true(
        &mut context,
        "globalThis.__lateTlaLog.join(',') === 'wait:start,throw,wait:end' && typeof globalThis.__lateTlaParentRan === 'undefined'",
    );
    assert!(matches!(
        runtime.module_record(waiting.raw).unwrap().evaluation,
        ModuleEvaluationState::Evaluated
    ));
    for member in [&module, &throwing] {
        let ModuleEvaluationState::Errored(raw_reason) =
            runtime.module_record(member.raw).unwrap().evaluation
        else {
            panic!("late TLA fulfillment changed the cached rejection state");
        };
        assert_eq!(runtime.root_raw_value(&raw_reason).unwrap(), reason);
    }
    let cached = module_evaluation_promise(&mut context, &module);
    assert_eq!(cached.object_id(), promise.object_id());
    let cached = promise_snapshot(&runtime, &cached);
    assert_eq!(cached.state, PromiseState::Rejected);
    assert_eq!(runtime.root_raw_value(&cached.result).unwrap(), reason);
    assert!(!runtime.is_job_pending());
}

#[test]
fn top_level_await_inside_a_cycle_unblocks_the_cycle_before_its_outer_parent() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval("globalThis.__tlaCycleOrder = []").unwrap();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/a.js",
            "import './b.js'; globalThis.__tlaCycleOrder.push('a');",
        ),
        (
            "pkg/b.js",
            r#"
            import "./a.js";
            globalThis.__tlaCycleOrder.push("b:start");
            await 0;
            globalThis.__tlaCycleOrder.push("b:end");
            "#,
        ),
    ]);
    let _registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            "import './a.js'; globalThis.__tlaCycleOrder.push('entry');",
            "pkg/entry.js",
        )
        .unwrap();
    let a = runtime.module_dependencies(&module).unwrap().remove(0);
    let b = runtime.module_dependencies(&a).unwrap().remove(0);

    let promise = module_evaluation_promise(&mut context, &module);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaCycleOrder.join(',') === 'b:start'",
    );
    for member in [&module, &a, &b] {
        assert!(matches!(
            runtime.module_record(member.raw).unwrap().evaluation,
            ModuleEvaluationState::EvaluatingAsync
        ));
    }
    assert_eq!(
        runtime
            .module_record(module.raw)
            .unwrap()
            .evaluation_cycle_root,
        Some(module.raw.module)
    );
    assert_eq!(
        runtime.module_record(a.raw).unwrap().evaluation_cycle_root,
        Some(a.raw.module)
    );
    assert_eq!(
        runtime.module_record(b.raw).unwrap().evaluation_cycle_root,
        Some(a.raw.module)
    );

    assert!(drain_jobs(&runtime) > 0);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaCycleOrder.join(',') === 'b:start,b:end,a,entry'",
    );
    for member in [&module, &a, &b] {
        assert!(matches!(
            runtime.module_record(member.raw).unwrap().evaluation,
            ModuleEvaluationState::Evaluated
        ));
    }
    assert!(!runtime.is_job_pending());
}
