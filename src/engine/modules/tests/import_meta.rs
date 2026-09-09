use super::*;

#[test]
fn import_meta_is_cached_per_defining_module() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        r#"
            globalThis.__dependencyMeta = import.meta;
            export function readMeta() { return import.meta; }
        "#,
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
                import { readMeta } from "./dependency.js";
                const local = import.meta;
                function localRead() { return import.meta; }
                const before = Reflect.ownKeys(local).length === 0;
                local.answer = 42;
                const descriptor = Object.getOwnPropertyDescriptor(local, "answer");
                globalThis.__lateReadMeta = readMeta;
                globalThis.__importMetaParity =
                    before &&
                    Object.getPrototypeOf(local) === null &&
                    Object.isExtensible(local) &&
                    local === import.meta && local === localRead() &&
                    readMeta() === globalThis.__dependencyMeta &&
                    readMeta() !== local &&
                    descriptor.value === 42 && descriptor.writable &&
                    descriptor.enumerable && descriptor.configurable &&
                    delete local.answer && !("answer" in local) &&
                    typeof local.resolve === "undefined";
            "#,
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__importMetaParity === true");
    drop(module);
    runtime.run_gc().unwrap();
    assert_script_true(&mut context, "__lateReadMeta() === __dependencyMeta");
}

#[test]
fn host_gets_the_canonical_import_meta_before_linking() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
                globalThis.__hostMeta = import.meta;
                globalThis.__hostMetaAnswer = import.meta.answer;
            "#,
            "pkg/host-meta.js",
        )
        .unwrap();

    let first = context.get_module_import_meta(&module).unwrap();
    let second = context.get_module_import_meta(&module).unwrap();
    assert_eq!(first, second);
    assert_eq!(runtime.get_prototype_of(&first).unwrap(), None);
    assert!(runtime.is_extensible(&first).unwrap());

    let answer = runtime.intern_property_key("answer").unwrap();
    assert!(
        context
            .define_own_property(
                &first,
                &answer,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(42)),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    // Ordinary host/user mutations remain valid before linking; every
    // following record replacement must keep accepting the same object.
    let prototype = context.new_object().unwrap();
    assert!(runtime.set_prototype_of(&first, Some(&prototype)).unwrap());
    runtime.prevent_extensions(&first).unwrap();

    context.execute_module(&module).unwrap();
    let global = context.global_object().unwrap();
    let observed = runtime.intern_property_key("__hostMeta").unwrap();
    assert_eq!(
        context.get_property(&global, &observed).unwrap(),
        Value::Object(first.clone())
    );
    assert_script_true(&mut context, "__hostMetaAnswer === 42");

    assert!(context.execute_module(&module).is_ok());
}

#[test]
fn module_record_owns_import_meta_through_gc_and_releases_cycles_with_its_cache() {
    let runtime = Runtime::new();
    let module = {
        let mut context = runtime.new_context();
        context.compile_module("export const answer = 42;").unwrap()
    };
    let mut host_context = runtime.new_context();
    let meta = host_context.get_module_import_meta(&module).unwrap();
    let self_key = runtime.intern_property_key("self").unwrap();
    assert!(
        host_context
            .set_property(&meta, &self_key, Value::Object(meta.clone()))
            .unwrap()
    );
    let meta_id = meta.object_id();
    drop(meta);
    runtime.run_gc().unwrap();
    assert!(runtime.0.state.borrow().heap.object(meta_id).is_ok());

    let observed = host_context.get_module_import_meta(&module).unwrap();
    assert_eq!(observed.object_id(), meta_id);
    drop(observed);
    drop(module);
    drop(host_context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
    assert_eq!(runtime.heap_counts().object_nodes, 0);
}

#[test]
fn loader_initializes_dependency_import_meta_before_source_completion() {
    let runtime = Runtime::new();
    let marker = runtime.new_object(None).unwrap();
    let dependency = ModuleLoadResult::SourceTextWithImportMeta {
        source: r#"
            globalThis.__dependencyMetaChecks = [
                import.meta.url,
                import.meta.main,
                import.meta.marker,
                Object.getOwnPropertyDescriptor(import.meta, "url")
            ];
            export const answer = 42;
        "#
        .to_owned(),
        properties: vec![
            ModuleImportMetaProperty::new(
                JsString::from_static("url"),
                Value::String(JsString::from_static("file:///pkg/dependency.js")),
            ),
            ModuleImportMetaProperty::new(JsString::from_static("main"), Value::Bool(false)),
            ModuleImportMetaProperty::new(
                JsString::from_static("marker"),
                Value::Object(marker.clone()),
            ),
        ],
    };
    let (loader, _, _) = JsonModuleLoader::new([("pkg/dependency.js", dependency)]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './dependency.js'; globalThis.__entryAnswer = answer;",
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();

    let marker_key = runtime
        .intern_property_key("__dependencyMetaChecks")
        .unwrap();
    let global = context.global_object().unwrap();
    let Value::Object(checks) = context.get_property(&global, &marker_key).unwrap() else {
        panic!("dependency import.meta checks were not published");
    };
    let zero = runtime.intern_property_key("0").unwrap();
    let one = runtime.intern_property_key("1").unwrap();
    let two = runtime.intern_property_key("2").unwrap();
    assert_eq!(
        context.get_property(&checks, &zero).unwrap(),
        Value::String(JsString::from_static("file:///pkg/dependency.js"))
    );
    assert_eq!(
        context.get_property(&checks, &one).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context.get_property(&checks, &two).unwrap(),
        Value::Object(marker)
    );
    assert_script_true(
        &mut context,
        r#"
            __entryAnswer === 42 &&
            __dependencyMetaChecks[3].writable &&
            __dependencyMetaChecks[3].enumerable &&
            __dependencyMetaChecks[3].configurable
        "#,
    );
}

#[test]
fn import_meta_host_values_must_belong_to_the_loading_runtime() {
    let runtime = Runtime::new();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let local = runtime.new_object(None).unwrap();
    let foreign = Runtime::new().new_object(None).unwrap();
    let result = ModuleLoadResult::SourceTextWithImportMeta {
        source: "export const answer = 42;".to_owned(),
        properties: vec![
            ModuleImportMetaProperty::new(
                JsString::from_static("local"),
                Value::Object(local.clone()),
            ),
            ModuleImportMetaProperty::new(JsString::from_static("foreign"), Value::Object(foreign)),
        ],
    };
    let (loader, _, _) = JsonModuleLoader::new([("pkg/dependency.js", result)]);
    let loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './dependency.js';", "pkg/entry.js",),
        Err(RuntimeError::WrongRuntime("descriptor value"))
    ));
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        2,
        "entry and dependency construction tombstones must both remain append-only"
    );
    drop(loader_registration);
    drop(local);
    drop(context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
}

#[test]
fn failed_deep_resolution_releases_published_dependency_import_meta() {
    let runtime = Runtime::new();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let marker = runtime.new_object(None).unwrap();
    let dependency = ModuleLoadResult::SourceTextWithImportMeta {
        source: "import './missing.js'; export const answer = 42;".to_owned(),
        properties: vec![ModuleImportMetaProperty::new(
            JsString::from_static("marker"),
            Value::Object(marker.clone()),
        )],
    };
    let (loader, _, _) = JsonModuleLoader::new([("pkg/dependency.js", dependency)]);
    let loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './dependency.js';", "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    assert!(context.take_exception().unwrap().is_some());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        2,
        "the failed entry and dependency remain only as cache tombstones"
    );
    drop(loader_registration);
    drop(marker);
    drop(context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
}
