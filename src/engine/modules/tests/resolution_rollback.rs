use super::*;

#[test]
fn failed_resolution_unpublishes_the_root_from_the_context_cache() {
    let runtime = Runtime::new();
    let (loader, loads, _) = MapModuleLoader::new([]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './missing.js';", "pkg/shared.js",),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(&*loads.borrow(), &["pkg/missing.js"]);

    context
        .compile_module_with_filename("export const value = 42;", "pkg/shared.js")
        .unwrap();
    let importer = context
        .compile_module_with_filename(
            "import { value } from './shared.js'; globalThis.__recoveredModule = value;",
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&importer).unwrap();
    assert_script_true(&mut context, "__recoveredModule === 42");
    assert_eq!(&*loads.borrow(), &["pkg/missing.js"]);
}

#[test]
fn failed_resolution_leaves_a_permanent_module_cache_tombstone() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './missing.js';", "pkg/failed.js"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap();
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        1
    );

    let replacement = context
        .compile_module_with_filename("export const ok = true;", "pkg/failed.js")
        .unwrap();
    assert_eq!(replacement.raw.module.0, 1);
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        2
    );
}

#[test]
fn escaped_module_handle_reports_aborted_after_resolution_rollback() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([]);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let name = JsString::from_static("pkg/aborted-entry.js");
    let ModuleCompilation::Published(raw) = runtime
        .compile_module_record_in_realm(context.realm, "import './missing.js';", &name, None)
        .unwrap()
    else {
        panic!("ordinary source unexpectedly threw during compilation");
    };
    let handle = runtime.root_module(raw).unwrap();

    assert!(matches!(
        runtime.resolve_module_graph(context.realm, raw),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap();
    assert_eq!(handle.name(), &name);
    assert_eq!(handle, handle.clone());
    assert_eq!(
        context.get_module_import_meta(&handle),
        Err(RuntimeError::AbortedModule)
    );
    assert_eq!(
        context.link_module(&handle),
        Err(RuntimeError::AbortedModule)
    );
    assert_eq!(
        context.execute_module(&handle),
        Err(RuntimeError::AbortedModule)
    );
}

#[test]
fn failed_resolution_rolls_back_every_active_loaded_module() {
    let runtime = Runtime::new();
    let (loader, sources, loads) = MutableMapModuleLoader::new([
        ("pkg/a.js", "import './b.js'; export const a = 1;"),
        ("pkg/b.js", "import './missing.js'; export const b = 1;"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './a.js';", "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(
        &*loads.borrow(),
        &["pkg/a.js", "pkg/b.js", "pkg/missing.js"]
    );

    sources
        .borrow_mut()
        .insert("pkg/b.js".to_owned(), "export const b = 42;".to_owned());
    let importer = context
        .compile_module_with_filename(
            "import { b } from './b.js'; globalThis.__activeRollback = b;",
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&importer).unwrap();
    assert_script_true(&mut context, "__activeRollback === 42");
    assert_eq!(
        &*loads.borrow(),
        &["pkg/a.js", "pkg/b.js", "pkg/missing.js", "pkg/b.js"]
    );
}

#[test]
fn failed_resolution_preserves_an_independently_completed_dependency() {
    let runtime = Runtime::new();
    let (loader, sources, loads) =
        MutableMapModuleLoader::new([("pkg/complete.js", "export const value = 42;")]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename(
            "import './complete.js'; import './missing.js';",
            "pkg/entry.js",
        ),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    sources.borrow_mut().insert(
        "pkg/complete.js".to_owned(),
        "export const value = 99;".to_owned(),
    );

    let importer = context
        .compile_module_with_filename(
            "import { value } from './complete.js'; globalThis.__completedCache = value;",
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&importer).unwrap();
    assert_script_true(&mut context, "__completedCache === 42");
    assert_eq!(&*loads.borrow(), &["pkg/complete.js", "pkg/missing.js"]);
}

#[test]
fn failed_resolution_unpublishes_cycle_members_that_reference_the_root() {
    let runtime = Runtime::new();
    let (loader, loads, _) = MapModuleLoader::new([
        ("pkg/a.js", "export const a = 41;"),
        (
            "pkg/b.js",
            "import { a } from './a.js'; export const b = a + 1;",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context
            .compile_module_with_filename("import './b.js'; import './missing.js';", "pkg/a.js",),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(&*loads.borrow(), &["pkg/b.js", "pkg/missing.js"]);

    let importer = context
        .compile_module_with_filename(
            "import { b } from './b.js'; globalThis.__cycleRecovered = b;",
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&importer).unwrap();
    assert_script_true(&mut context, "__cycleRecovered === 42");
    assert_eq!(
        &*loads.borrow(),
        &["pkg/b.js", "pkg/missing.js", "pkg/b.js", "pkg/a.js"]
    );
}
