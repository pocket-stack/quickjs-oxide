use super::*;

#[test]
fn deep_star_resolution_uses_an_explicit_frame_stack() {
    const MODULE_COUNT: usize = 1_024;

    std::thread::Builder::new()
        .name("deep-star-module-graph".to_owned())
        .stack_size(256 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let _loader_registration = runtime.set_module_loader(StarChainModuleLoader {
                module_count: MODULE_COUNT,
            });
            let mut context = runtime.new_context();
            let module = context
                .compile_module_with_filename(
                    "import * as ns from 's0'; globalThis.__deepStarAnswer = ns.answer;",
                    "entry.js",
                )
                .unwrap();
            context.execute_module(&module).unwrap();
            assert_script_true(&mut context, "__deepStarAnswer === 42");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_cyclic_graph_uses_explicit_resolve_link_and_evaluation_stacks() {
    const MODULE_COUNT: usize = 1_024;

    std::thread::Builder::new()
        .name("deep-module-graph".to_owned())
        .stack_size(256 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let _loader_registration = runtime.set_module_loader(CyclicChainModuleLoader {
                module_count: MODULE_COUNT,
            });
            let mut context = runtime.new_context();
            let module = context
                .compile_module_with_filename(
                    "import 'm0'; globalThis.__deepModuleEntry = true;",
                    "entry.js",
                )
                .unwrap();
            context.link_module(&module).unwrap();
            context.execute_module(&module).unwrap();
            assert_script_true(
                &mut context,
                &format!("__deepModuleRuns === {MODULE_COUNT} && __deepModuleEntry === true"),
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn dependency_evaluation_exception_is_cached_on_every_active_ancestor() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/abrupt.js",
        r#"
        globalThis.__abruptRuns = (globalThis.__abruptRuns || 0) + 1;
        throw 42;
        "#,
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import './abrupt.js'; globalThis.__ancestorRan = true;",
            "pkg/entry.js",
        )
        .unwrap();

    let first = module_evaluation_promise(&mut context, &module);
    let first_snapshot = promise_snapshot(&runtime, &first);
    assert_eq!(first_snapshot.state, PromiseState::Rejected);
    assert_eq!(first_snapshot.result, RawValue::Int(42));
    let second = module_evaluation_promise(&mut context, &module);
    assert_eq!(first.object_id(), second.object_id());
    assert_script_true(
        &mut context,
        "__abruptRuns === 1 && typeof __ancestorRan === 'undefined'",
    );
}

#[test]
fn cyclic_evaluation_exception_is_cached_on_the_complete_active_scc() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/a.js",
            r#"
            import "./b.js";
            globalThis.__cycleARuns = (globalThis.__cycleARuns || 0) + 1;
            throw 42;
            "#,
        ),
        (
            "pkg/b.js",
            r#"
            import "./a.js";
            globalThis.__cycleBRuns = (globalThis.__cycleBRuns || 0) + 1;
            "#,
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import './a.js'; globalThis.__cycleEntryRan = true;",
            "pkg/entry.js",
        )
        .unwrap();
    let a = runtime.module_dependencies(&module).unwrap().remove(0);
    let b = runtime.module_dependencies(&a).unwrap().remove(0);

    let first = module_evaluation_promise(&mut context, &module);
    let first_snapshot = promise_snapshot(&runtime, &first);
    assert_eq!(first_snapshot.state, PromiseState::Rejected);
    assert_eq!(first_snapshot.result, RawValue::Int(42));
    let second = module_evaluation_promise(&mut context, &module);
    assert_eq!(first.object_id(), second.object_id());
    for _ in 0..2 {
        for member in [&module, &a, &b] {
            assert!(matches!(
                runtime.module_record(member.raw).unwrap().evaluation,
                ModuleEvaluationState::Errored(RawValue::Int(42))
            ));
        }
    }
    assert_script_true(
        &mut context,
        "__cycleARuns === 1 && __cycleBRuns === 1 && typeof __cycleEntryRan === 'undefined'",
    );
}

#[test]
fn context_module_cache_is_oldest_first_and_loader_cache_is_per_context() {
    let runtime = Runtime::new();
    let (loader, loads, _) = MapModuleLoader::new([("pkg/loaded.js", "export const loaded = 42;")]);
    let _loader_registration = runtime.set_module_loader(loader);

    let mut first_context = runtime.new_context();
    first_context
        .compile_module_with_filename("export const value = 1;", "pkg/shared.js")
        .unwrap();
    first_context
        .compile_module_with_filename("export const value = 2;", "pkg/shared.js")
        .unwrap();
    let oldest = first_context
        .compile_module_with_filename(
            "import { value } from './shared.js'; globalThis.__oldest = value;",
            "pkg/oldest-entry.js",
        )
        .unwrap();
    first_context.execute_module(&oldest).unwrap();
    assert_script_true(&mut first_context, "__oldest === 1");

    let first_loaded = first_context
        .compile_module_with_filename(
            "import { loaded } from './loaded.js'; globalThis.__loaded = loaded;",
            "pkg/first-entry.js",
        )
        .unwrap();
    first_context.execute_module(&first_loaded).unwrap();

    let mut second_context = runtime.new_context();
    let second_loaded = second_context
        .compile_module_with_filename(
            "import { loaded } from './loaded.js'; globalThis.__loaded = loaded;",
            "pkg/second-entry.js",
        )
        .unwrap();
    second_context.execute_module(&second_loaded).unwrap();
    assert_eq!(&*loads.borrow(), &["pkg/loaded.js", "pkg/loaded.js"]);
    assert_script_true(&mut first_context, "__loaded === 42");
    assert_script_true(&mut second_context, "__loaded === 42");
}

#[test]
fn first_execute_context_owns_globals_for_the_complete_module_graph() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        "globalThis.__graphDependencyRealm = __realmMarker; export const value = 42;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut compilation_context = runtime.new_context();
    compilation_context
        .eval("globalThis.__realmMarker = 1")
        .unwrap();
    let module = compilation_context
        .compile_module_with_filename(
            "import { value } from './dependency.js'; globalThis.__graphRootRealm = __realmMarker + value;",
            "pkg/entry.js",
        )
        .unwrap();

    let mut execution_context = runtime.new_context();
    execution_context
        .eval("globalThis.__realmMarker = 2")
        .unwrap();
    execution_context.execute_module(&module).unwrap();
    assert_script_true(
        &mut execution_context,
        "__graphDependencyRealm === 2 && __graphRootRealm === 44",
    );
    assert_script_true(
        &mut compilation_context,
        "typeof __graphDependencyRealm === 'undefined' && typeof __graphRootRealm === 'undefined'",
    );
}

#[test]
fn module_cells_use_the_link_context_while_bytecode_keeps_its_compile_realm() {
    let runtime = Runtime::new();
    let mut compilation_context = runtime.new_context();
    let compilation_object_prototype = compilation_context.eval("Object.prototype").unwrap();
    let compilation_function_prototype = compilation_context.eval("Function.prototype").unwrap();
    let compilation_array_prototype = compilation_context.eval("Array.prototype").unwrap();
    let compilation_type_error_prototype = compilation_context.eval("TypeError.prototype").unwrap();
    let module = compilation_context
        .compile_module(
            r#"
            globalThis.__moduleRealmObject = {};
            globalThis.__moduleRealmFunction = function () {};
            globalThis.__moduleRealmArray = [];
            try { null.value; } catch (error) { globalThis.__moduleRealmError = error; }
            "#,
        )
        .unwrap();

    let mut link_context = runtime.new_context();
    let link_object_prototype = link_context.eval("Object.prototype").unwrap();
    let link_function_prototype = link_context.eval("Function.prototype").unwrap();
    let link_array_prototype = link_context.eval("Array.prototype").unwrap();
    let link_type_error_prototype = link_context.eval("TypeError.prototype").unwrap();
    link_context.execute_module(&module).unwrap();
    let module_object_prototype = link_context
        .eval("Object.getPrototypeOf(__moduleRealmObject)")
        .unwrap();
    let module_function_prototype = link_context
        .eval("Object.getPrototypeOf(__moduleRealmFunction)")
        .unwrap();
    let module_array_prototype = link_context
        .eval("Object.getPrototypeOf(__moduleRealmArray)")
        .unwrap();
    let module_error_prototype = link_context
        .eval("Object.getPrototypeOf(__moduleRealmError)")
        .unwrap();

    // QuickJS creates the module closure and its global cells with the
    // linking Context, while the immutable function bytecode retains the
    // Context which compiled it. Object literals therefore use the latter
    // realm even though `globalThis` resolves through the former's cell.
    assert_eq!(module_object_prototype, compilation_object_prototype);
    assert_eq!(module_function_prototype, compilation_function_prototype);
    assert_eq!(module_array_prototype, compilation_array_prototype);
    assert_eq!(module_error_prototype, compilation_type_error_prototype);
    assert_ne!(module_object_prototype, link_object_prototype);
    assert_ne!(module_function_prototype, link_function_prototype);
    assert_ne!(module_array_prototype, link_array_prototype);
    assert_ne!(module_error_prototype, link_type_error_prototype);
    assert_script_true(
        &mut compilation_context,
        "typeof __moduleRealmObject === 'undefined'",
    );
}
