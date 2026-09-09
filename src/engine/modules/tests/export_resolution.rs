use super::*;

#[test]
fn missing_export_fails_during_retryable_link_before_module_bodies() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        "globalThis.__missingDependencyRan = true; export const present = 1;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { absent } from "./dependency.js";
            globalThis.__missingEntryRan = absent;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    for _ in 0..2 {
        assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
        assert!(matches!(
            context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));
    }
    assert_script_true(
        &mut context,
        "typeof __missingDependencyRan === 'undefined' && typeof __missingEntryRan === 'undefined'",
    );
}

#[test]
fn cyclic_link_failure_resets_every_active_scc_member() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/a.js",
            "import { b, absent } from './b.js'; export const a = b;",
        ),
        (
            "pkg/b.js",
            "import { a } from './a.js'; export const b = 2; globalThis.__cycleLinkBody = a;",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename("import { a } from './a.js'; void a;", "pkg/entry.js")
        .unwrap();
    let a = runtime.module_dependencies(&module).unwrap().remove(0);
    let b = runtime.module_dependencies(&a).unwrap().remove(0);

    for _ in 0..2 {
        assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
        assert!(matches!(
            context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));
        for member in [&module, &a, &b] {
            assert_eq!(
                runtime.module_record(member.raw).unwrap().link_status,
                ModuleLinkStatus::Unlinked
            );
        }
    }
    assert_script_true(&mut context, "typeof __cycleLinkBody === 'undefined'");
}

#[test]
fn exported_import_cycle_resolves_to_the_ultimate_live_cell() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/a.js", "import { x } from './b.js'; export { x };"),
        (
            "pkg/b.js",
            "import { c } from './c.js'; export const x = 42; export const b = c;",
        ),
        (
            "pkg/c.js",
            "import { x } from './a.js'; export const c = x;",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { b } from './b.js'; globalThis.__exportCycleBody = b;",
            "pkg/entry.js",
        )
        .unwrap();

    // Every import alias can be linked even though A's local export is an
    // imported binding whose own SCC member has not linked yet.
    context.link_module(&module).unwrap();
    let first = module_evaluation_promise(&mut context, &module);
    let first_snapshot = promise_snapshot(&runtime, &first);
    assert_eq!(first_snapshot.state, PromiseState::Rejected);
    assert!(matches!(first_snapshot.result, RawValue::Object(_)));
    let second = module_evaluation_promise(&mut context, &module);
    assert_eq!(first.object_id(), second.object_id());
    // Evaluation still observes the specified TDZ: C reads B.x before B's
    // body initializes it. The exception is cached instead of becoming a
    // missing-cell invariant or native crash.
    assert_script_true(&mut context, "typeof __exportCycleBody === 'undefined'");
}

#[test]
fn circular_exported_import_alias_is_a_retryable_syntax_error() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/a.js", "import { x } from './b.js'; export { x };"),
        ("pkg/b.js", "import { x } from './a.js'; export { x };"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { x } from './a.js'; globalThis.__circularAliasBody = x;",
            "pkg/entry.js",
        )
        .unwrap();

    for _ in 0..2 {
        assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
        assert!(matches!(
            context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));
    }
    assert_script_true(&mut context, "typeof __circularAliasBody === 'undefined'");
}

#[test]
fn resolve_export_keeps_same_binding_diamonds_unambiguous() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/source.js", "export const answer = 42;"),
        ("pkg/left.js", "export { answer } from './source.js';"),
        ("pkg/right.js", "export { answer } from './source.js';"),
        (
            "pkg/barrel.js",
            "export * from './left.js'; export * from './right.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './barrel.js'; globalThis.__diamondAnswer = answer;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__diamondAnswer === 42");
}

#[test]
fn namespace_exports_from_one_owner_share_quickjs_star_identity() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/a.js", "export const a = 1;"),
        ("pkg/b.js", "export const b = 2;"),
        (
            "pkg/source.js",
            "export * as left from './a.js'; export * as right from './b.js';",
        ),
        ("pkg/left.js", "export { left as x } from './source.js';"),
        ("pkg/right.js", "export { right as x } from './source.js';"),
        (
            "pkg/barrel.js",
            "export * from './left.js'; export * from './right.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { x } from './barrel.js'; globalThis.__namespaceIdentity = x;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        "__namespaceIdentity.a === 1 && !('b' in __namespaceIdentity)",
    );
}

#[test]
fn resolve_export_reports_distinct_star_bindings_as_ambiguous() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/left.js", "export const answer = 1;"),
        ("pkg/right.js", "export const answer = 2;"),
        (
            "pkg/barrel.js",
            "export * from './left.js'; export * from './right.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './barrel.js'; void answer;",
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("export 'answer' in module 'pkg/barrel.js' is ambiguous")
    );
}

#[test]
fn star_resolution_ignores_circular_and_not_found_branches() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/cycle-a.js",
            "export * from './cycle-b.js'; export const unrelated = 1;",
        ),
        (
            "pkg/cycle-b.js",
            "export * from './cycle-a.js'; export const other = 2;",
        ),
        ("pkg/empty.js", "export const absent = 3;"),
        ("pkg/source.js", "export const answer = 42;"),
        (
            "pkg/barrel.js",
            "export * from './cycle-a.js'; export * from './empty.js'; export * from './source.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './barrel.js'; globalThis.__starBranchAnswer = answer;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__starBranchAnswer === 42");
}

#[test]
fn module_namespace_omits_an_ambiguous_star_export() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/left.js",
            "export const answer = 1; export const left = 2;",
        ),
        (
            "pkg/right.js",
            "export const answer = 3; export const right = 4;",
        ),
        (
            "pkg/barrel.js",
            "export * from './left.js'; export * from './right.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import * as ns from './barrel.js'; globalThis.__ambiguousNamespace = ns;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        "!('answer' in __ambiguousNamespace) && __ambiguousNamespace.left === 2 && __ambiguousNamespace.right === 4",
    );
}

#[test]
fn default_is_not_resolved_through_star_exports() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/source.js", "export default 42;"),
        ("pkg/barrel.js", "export * from './source.js';"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { default as answer } from './barrel.js'; void answer;",
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("Could not find export 'default' in module 'pkg/barrel.js'")
    );
}

#[test]
fn indirect_export_preflight_blames_the_public_name_and_owner() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        "globalThis.__indirectDependencyRan = true; export const present = 1;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "export { absent as publicName } from './dependency.js';",
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("Could not find export 'publicName' in module 'pkg/entry.js'")
    );
    assert_script_true(
        &mut context,
        "typeof __indirectDependencyRan === 'undefined'",
    );
}

#[test]
fn circular_indirect_exports_fail_without_native_recursion() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/a.js", "export { answer } from './b.js';"),
        ("pkg/b.js", "export { answer } from './a.js';"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './a.js'; void answer;",
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static(
            "circular reference when looking for export 'answer' in module 'pkg/b.js'"
        )
    );
}
