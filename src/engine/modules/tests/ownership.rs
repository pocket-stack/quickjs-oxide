use super::*;
use crate::engine::heap::HeapError;

#[test]
fn module_handle_rejects_another_runtime() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context.compile_module("").unwrap();
    let mut other = Runtime::new().new_context();
    assert_eq!(
        other.execute_module(&module),
        Err(RuntimeError::WrongRuntime("module bytecode"))
    );
}

#[test]
fn first_execute_context_owns_module_global_resolution_and_evaluates_once() {
    let runtime = Runtime::new();
    let mut compilation_context = runtime.new_context();
    compilation_context
        .eval("globalThis.__realmMarker = 1")
        .unwrap();
    let module = compilation_context
        .compile_module(
            r#"
            globalThis.__moduleLinkMarker = __realmMarker;
            globalThis.__moduleLinkRuns = (globalThis.__moduleLinkRuns || 0) + 1;
            "#,
        )
        .unwrap();

    let mut first_execute_context = runtime.new_context();
    first_execute_context
        .eval("globalThis.__realmMarker = 2")
        .unwrap();
    let mut later_context = runtime.new_context();
    later_context.eval("globalThis.__realmMarker = 3").unwrap();

    let first = module_evaluation_promise(&mut first_execute_context, &module);
    assert_script_true(
        &mut first_execute_context,
        "__moduleLinkMarker === 2 && __moduleLinkRuns === 1",
    );
    assert_script_true(
        &mut compilation_context,
        "typeof __moduleLinkMarker === 'undefined' && typeof __moduleLinkRuns === 'undefined'",
    );
    assert_script_true(
        &mut later_context,
        "typeof __moduleLinkMarker === 'undefined' && typeof __moduleLinkRuns === 'undefined'",
    );

    let second = module_evaluation_promise(&mut later_context, &module);
    assert_eq!(first.object_id(), second.object_id());
    assert_script_true(
        &mut first_execute_context,
        "__moduleLinkMarker === 2 && __moduleLinkRuns === 1",
    );
    assert_script_true(
        &mut later_context,
        "typeof __moduleLinkMarker === 'undefined' && typeof __moduleLinkRuns === 'undefined'",
    );
}

#[test]
fn cloned_module_handle_roots_compilation_and_first_link_realms() {
    let runtime = Runtime::new();
    let module = {
        let mut context = runtime.new_context();
        context
            .compile_module("globalThis.__rootedModuleRealm = 42")
            .unwrap()
    };
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    let surviving_handle = module.clone();
    drop(module);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);

    {
        let mut link_context = runtime.new_context();
        assert_eq!(runtime.heap_counts().context_nodes, 2);
        let snapshot = module_evaluation_snapshot(&mut link_context, &surviving_handle);
        assert_eq!(snapshot.state, PromiseState::Fulfilled);
        assert_eq!(snapshot.result, RawValue::Undefined);
        assert_script_true(&mut link_context, "__rootedModuleRealm === 42");
    }

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 2);

    drop(surviving_handle);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
}

#[test]
fn cross_linked_module_caches_do_not_leak_a_context_cycle() {
    let runtime = Runtime::new();
    let mut first_context = runtime.new_context();
    let mut second_context = runtime.new_context();
    let first_module = first_context
        .compile_module("globalThis.__firstCrossCacheModule = 1")
        .unwrap();
    let second_module = second_context
        .compile_module("globalThis.__secondCrossCacheModule = 2")
        .unwrap();

    second_context.execute_module(&first_module).unwrap();
    first_context.execute_module(&second_module).unwrap();

    assert_eq!(
        runtime.module_record(first_module.raw).unwrap().link_realm,
        Some(RawModuleLinkRealm::Other(second_context.realm))
    );
    assert_eq!(
        runtime.module_record(second_module.raw).unwrap().link_realm,
        Some(RawModuleLinkRealm::Other(first_context.realm))
    );
    assert_eq!(runtime.heap_counts().context_nodes, 2);

    drop(first_module);
    drop(second_module);
    drop(first_context);
    drop(second_context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
}

#[test]
fn loaded_module_validator_rejects_internal_sentinels_and_cache_self_edges_atomically() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context.compile_module("export const answer = 42").unwrap();
    let raw = module.raw;

    assert!(matches!(
        runtime.mutate_module_record(raw, |record| {
            record.evaluation = ModuleEvaluationState::Errored(RawValue::Exception);
            Ok(())
        }),
        Err(RuntimeError::Heap(HeapError::Invariant(
            "loaded-module record contains an internal value sentinel"
        )))
    ));
    assert!(matches!(
        runtime.module_record(raw).unwrap().evaluation,
        ModuleEvaluationState::Unevaluated
    ));

    assert!(matches!(
        runtime.mutate_module_record(raw, |record| {
            record.instance = Some(ModuleInstance {
                slots: Vec::new(),
                callable: None,
            });
            record.link_realm = Some(RawModuleLinkRealm::Other(raw.cache));
            Ok(())
        }),
        Err(RuntimeError::Heap(HeapError::Invariant(
            "loaded-module cache realm escaped through an Other link edge"
        )))
    ));
    let record = runtime.module_record(raw).unwrap();
    assert!(record.instance.is_none());
    assert!(record.link_realm.is_none());
}

#[test]
fn json_module_handle_roots_its_parse_realm_across_context_gc() {
    let runtime = Runtime::new();
    let (loader, _, _) = JsonModuleLoader::new([(
        "pkg/value.json",
        ModuleLoadResult::JsonText(r#"{"answer":1}"#.to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let module = {
        let mut compilation_context = runtime.new_context();
        compilation_context
            .eval("Object.prototype.__jsonParseRealm = 41")
            .unwrap();
        compilation_context
            .compile_module_with_filename(
                r#"
                import value from "./value.json" with { type: "json" };
                globalThis.__jsonParseRealm =
                    Object.getPrototypeOf(value).__jsonParseRealm + value.answer;
                globalThis.__jsonParsePrototype = Object.getPrototypeOf(value);
                "#,
                "pkg/entry.js",
            )
            .unwrap()
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);

    {
        let mut execution_context = runtime.new_context();
        execution_context.execute_module(&module).unwrap();
        assert_script_true(
            &mut execution_context,
            "__jsonParseRealm === 42 && __jsonParsePrototype !== Object.prototype",
        );
    }

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 2);
    drop(module);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
}

#[test]
fn module_root_stack_frame_is_anonymous_and_retains_filename() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename("throw new Error(\"x\")", "module-stack.mjs")
        .unwrap();

    let snapshot = module_evaluation_snapshot(&mut context, &module);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    let RawValue::Object(error) = snapshot.result else {
        panic!("module evaluation did not reject with an Error object");
    };
    let error = ObjectRef::from_borrowed_handle(runtime.clone(), error).unwrap();
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert_eq!(
        runtime
            .raw_string_property_for_diagnostics(&error, &stack_key)
            .unwrap(),
        Some(JsString::from_static(
            "    at <anonymous> (module-stack.mjs:1:16)\n"
        ))
    );
}
