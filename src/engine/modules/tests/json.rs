use super::*;

#[test]
fn json_module_default_export_is_cached_by_normalized_name_and_keeps_json_semantics() {
    let runtime = Runtime::new();
    let (loader, _, loads) = JsonModuleLoader::new([
        (
            "pkg/value.json",
            ModuleLoadResult::JsonText(
                r#"{"answer":40,"nested":[2],"__proto__":{"polluted":true}}"#.to_owned(),
            ),
        ),
        (
            "pkg/indirect.js",
            ModuleLoadResult::SourceText(
                "export { default } from './value.json' with { type: 'json' };".to_owned(),
            ),
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import first from "./value.json" with { type: "json" };
            import { default as second } from "./value.json" with { type: "json" };
            import indirect from "./indirect.js";
            import * as namespace from "./value.json" with { type: "json" };
            const proto = Object.getOwnPropertyDescriptor(first, "__proto__");
            globalThis.__jsonModuleParity =
                first === second && second === indirect && namespace.default === first &&
                Reflect.ownKeys(namespace).length === 2 &&
                Object.keys(namespace).join(",") === "default" &&
                namespace[Symbol.toStringTag] === "Module" &&
                first.answer + first.nested[0] === 42 &&
                Object.getPrototypeOf(first) === Object.prototype &&
                Object.isExtensible(first) &&
                proto.value.polluted === true && proto.enumerable &&
                proto.writable && proto.configurable;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__jsonModuleParity === true");
    assert_eq!(
        loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/value.json", "pkg/indirect.js"]
    );
    assert_eq!(
        loads.borrow()[0].attributes,
        Some(vec![("type".to_owned(), "json".to_owned())])
    );
}

#[test]
fn json5_module_default_export_is_cached_and_uses_quickjs_extended_grammar() {
    let runtime = Runtime::new();
    let (loader, _, loads) = JsonModuleLoader::new([
        (
            "pkg/value.data",
            ModuleLoadResult::Json5Text(
                "/* host selected */ {answer:+40, nested:[2,], marker:'json5',}".to_owned(),
            ),
        ),
        (
            "pkg/indirect.js",
            ModuleLoadResult::SourceText(
                "export { default } from './value.data' with { type: 'json5' };".to_owned(),
            ),
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import first from "./value.data" with { type: "json5" };
            import { default as second } from "../pkg/value.data" with { type: "json5" };
            import indirect from "./indirect.js";
            import * as namespace from "./value.data" with { type: "json5" };
            globalThis.__json5ModuleParity =
                first === second && second === indirect && namespace.default === first &&
                Object.keys(namespace).join(",") === "default" &&
                namespace[Symbol.toStringTag] === "Module" &&
                Object.getPrototypeOf(first) === Object.prototype &&
                first.answer + first.nested[0] === 42 && first.marker === "json5";
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__json5ModuleParity === true");
    assert_eq!(
        loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/value.data", "pkg/indirect.js"]
    );
    assert_eq!(
        loads.borrow()[0].attributes,
        Some(vec![("type".to_owned(), "json5".to_owned())])
    );
}

#[test]
fn dynamic_import_can_load_a_host_selected_json5_module() {
    let runtime = Runtime::new();
    let (loader, _, loads) = JsonModuleLoader::new([(
        "pkg/value.data",
        ModuleLoadResult::Json5Text("{answer: 0x2a,}".to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let promise = eval_dynamic_import(
        &mut context,
        "import('./value.data', { with: { type: 'json5' } })",
        "pkg/entry.js",
    );

    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(drain_jobs(&runtime) > 0);
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    let Value::Object(namespace) = runtime.root_raw_value(&snapshot.result).unwrap() else {
        panic!("dynamic JSON5 import did not fulfill with a module namespace");
    };
    let default = runtime.intern_property_key("default").unwrap();
    let Value::Object(value) = context.get_property(&namespace, &default).unwrap() else {
        panic!("dynamic JSON5 module did not expose its default object");
    };
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        context.get_property(&value, &answer).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        loads.borrow().as_slice(),
        [RecordedAttributeLoad {
            name: "pkg/value.data".to_owned(),
            attributes: Some(vec![("type".to_owned(), "json5".to_owned())]),
        }]
    );
}

#[test]
fn invalid_json5_module_reports_quickjs_location_and_retries() {
    let runtime = Runtime::new();
    let (loader, modules, loads) = JsonModuleLoader::new([(
        "pkg/value.data",
        ModuleLoadResult::Json5Text("{é: 1}".to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let source = r#"
        import value from "./value.data" with { type: "json5" };
        globalThis.__json5Retry = value.answer;
    "#;

    assert!(matches!(
        context.compile_module_with_filename(source, "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("invalid JSON5 module did not throw a SyntaxError");
    };
    for (name, expected) in [
        (
            "message",
            Value::String(JsString::from_static("unexpected character")),
        ),
        (
            "fileName",
            Value::String(JsString::from_static("pkg/value.data")),
        ),
        ("lineNumber", Value::Int(1)),
        ("columnNumber", Value::Int(2)),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(context.get_property(&error, &key).unwrap(), expected);
    }

    modules.borrow_mut().insert(
        "pkg/value.data".to_owned(),
        ModuleLoadResult::Json5Text("{answer:+42,}".to_owned()),
    );
    let module = context
        .compile_module_with_filename(source, "pkg/entry.js")
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__json5Retry === 42");
    assert_eq!(
        loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/value.data", "pkg/value.data"]
    );
}

#[test]
fn json_module_live_cell_is_undefined_after_link_and_initialized_during_evaluation() {
    let runtime = Runtime::new();
    let (loader, _, _) = JsonModuleLoader::new([(
        "pkg/value.json",
        ModuleLoadResult::JsonText(r#"{"answer":42}"#.to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import value from "./value.json" with { type: "json" };
            globalThis.__jsonEvaluationValue = value.answer;
            "#,
            "pkg/entry.js",
        )
        .unwrap();
    let dependency = runtime.module_dependencies(&module).unwrap().remove(0);

    context.link_module(&module).unwrap();
    let namespace = runtime
        .get_module_namespace(&dependency, context.realm)
        .unwrap();
    let default = runtime.intern_property_key("default").unwrap();
    assert_eq!(
        context.get_property(&namespace, &default).unwrap(),
        Value::Undefined
    );

    context.execute_module(&module).unwrap();
    let Value::Object(first) = context.get_property(&namespace, &default).unwrap() else {
        panic!("evaluated JSON module default was not the parsed object");
    };
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        context.get_property(&first, &answer).unwrap(),
        Value::Int(42)
    );
    assert_script_true(&mut context, "__jsonEvaluationValue === 42");

    context.execute_module(&module).unwrap();
    assert_eq!(
        context.get_property(&namespace, &default).unwrap(),
        Value::Object(first)
    );
}

#[test]
fn json_module_named_import_fails_during_retryable_link() {
    let runtime = Runtime::new();
    let (loader, _, _) = JsonModuleLoader::new([(
        "pkg/value.json",
        ModuleLoadResult::JsonText(r#"{"name":"not an export"}"#.to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { name } from "./value.json" with { type: "json" };
            globalThis.__jsonNamedImportBody = name;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    for _ in 0..2 {
        assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from_static("Could not find export 'name' in module 'pkg/value.json'")
        );
    }
    assert_script_true(&mut context, "typeof __jsonNamedImportBody === 'undefined'");
}

#[test]
fn invalid_json_module_reports_fixture_location_and_rolls_back_for_retry() {
    let runtime = Runtime::new();
    let (loader, modules, loads) = JsonModuleLoader::new([(
        "pkg/value.json",
        ModuleLoadResult::JsonText("{\n  notJson: 0\n}\n".to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let source = r#"
        import value from "./value.json" with { type: "json" };
        globalThis.__jsonRetry = value.answer;
    "#;

    assert!(matches!(
        context.compile_module_with_filename(source, "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("invalid JSON module did not throw a SyntaxError");
    };
    for (name, expected) in [
        (
            "message",
            Value::String(JsString::from_static("expecting property name")),
        ),
        (
            "fileName",
            Value::String(JsString::from_static("pkg/value.json")),
        ),
        ("lineNumber", Value::Int(2)),
        ("columnNumber", Value::Int(3)),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(context.get_property(&error, &key).unwrap(), expected);
    }

    modules.borrow_mut().insert(
        "pkg/value.json".to_owned(),
        ModuleLoadResult::JsonText(r#"{"answer":42}"#.to_owned()),
    );
    let module = context
        .compile_module_with_filename(source, "pkg/entry.js")
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__jsonRetry === 42");
    assert_eq!(
        loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/value.json", "pkg/value.json"]
    );
}
