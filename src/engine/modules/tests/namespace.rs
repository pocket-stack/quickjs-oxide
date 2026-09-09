use super::*;

#[test]
fn namespace_cache_preserves_cycles_identity_and_live_cells() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/a.js",
            "export * as b from './b.js'; export let value = 1; export function bump() { value = 42; }",
        ),
        (
            "pkg/b.js",
            "export * as a from './a.js'; export const marker = 2;",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import * as a from './a.js';
            import * as b from './b.js';
            globalThis.__namespaceA = a;
            globalThis.__namespaceB = b;
            globalThis.__namespaceBefore = a.value;
            a.bump();
            globalThis.__namespaceAfter = a.value;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        r#"
        __namespaceA.b === __namespaceB &&
        __namespaceB.a === __namespaceA &&
        __namespaceBefore === 1 && __namespaceAfter === 42 &&
        Object.getPrototypeOf(__namespaceA) === null &&
        Object.isExtensible(__namespaceA) === false &&
        Reflect.ownKeys(__namespaceA).slice(0, 3).join(',') === 'b,bump,value' &&
        Reflect.ownKeys(__namespaceA)[3] === Symbol.toStringTag
        "#,
    );
}

#[test]
fn self_namespace_import_export_keeps_the_preallocated_cell() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/self.js",
        "import * as self from './self.js'; export { self }; export const answer = 42;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import * as ns from './self.js'; globalThis.__selfNamespace = ns;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        "__selfNamespace.self === __selfNamespace && __selfNamespace.answer === 42",
    );
}

#[test]
fn failed_namespace_build_rolls_back_its_placeholder_for_retry() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([("pkg/dependency.js", "export const present = 1;")]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "export { absent as publicName } from './dependency.js';",
            "pkg/entry.js",
        )
        .unwrap();
    runtime
        .prepare_module_instance(module.raw, context.realm)
        .unwrap();

    for _ in 0..2 {
        assert_eq!(
            runtime.get_module_namespace(&module, context.realm),
            Err(RuntimeError::Exception)
        );
        assert!(matches!(
            runtime.module_record(module.raw).unwrap().namespace,
            ModuleNamespaceState::Empty
        ));
        assert!(matches!(
            context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));
    }
}
