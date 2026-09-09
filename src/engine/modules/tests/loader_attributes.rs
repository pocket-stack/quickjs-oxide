use super::*;

#[test]
fn default_module_normalizer_matches_quickjs_leading_dot_rules() {
    for (base, specifier, expected) in [
        ("pkg/entry.js", "bare", "bare"),
        ("pkg/entry.js", "./dep.js", "pkg/dep.js"),
        ("pkg/deep/entry.js", "../dep.js", "pkg/dep.js"),
        ("pkg/deep/entry.js", "../../dep.js", "dep.js"),
        ("entry.js", "../dep.js", "../dep.js"),
        ("pkg/entry.js", ".hidden", "pkg/.hidden"),
        ("./entry.js", "../dep.js", "./../dep.js"),
        ("../entry.js", "../dep.js", "../../dep.js"),
    ] {
        let base = JsString::try_from_utf8(base).unwrap();
        let specifier = JsString::try_from_utf8(specifier).unwrap();
        assert_eq!(
            default_module_normalize_name(&base, &specifier)
                .unwrap()
                .to_utf8_lossy(),
            expected
        );
    }
}

#[test]
fn import_attribute_states_preserve_syntax_and_fold_empty_for_hosts() {
    let absent = ModuleImportAttributes::Absent;
    let empty = ModuleImportAttributes::Present(Vec::new().into_boxed_slice());
    let present = ModuleImportAttributes::Present(
        vec![ModuleImportAttribute {
            key: JsString::from_static("type"),
            value: JsString::from_static("javascript"),
        }]
        .into_boxed_slice(),
    );

    assert!(absent.syntactic().is_none());
    assert!(absent.effective().is_none());
    assert_eq!(empty.syntactic(), Some([].as_slice()));
    assert!(empty.effective().is_none());
    assert_eq!(
        present.effective().map(recorded_attribute_pairs).unwrap(),
        vec![("type".to_owned(), "javascript".to_owned())]
    );
}

#[test]
fn loader2_observes_effective_attributes_only_on_cache_miss() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .compile_module_with_filename("export const value = 39;", "pkg/cached.js")
        .unwrap();
    let (loader, controls) = AttributeModuleLoader::new([
        ("pkg/shared.js", "export const value = 0;"),
        ("pkg/absent.js", "export const value = 1;"),
        ("pkg/empty.js", "export const value = 1;"),
        ("pkg/present.js", "export const value = 1;"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            r#"
            import { value as cached } from "./cached.js" with { cache: "hit" };
            import "./shared.js" with { flavor: "first" };
            import "./shared.js" with { flavor: "second" };
            import { value as absent } from "./absent.js";
            import { value as empty } from "./empty.js" with {};
            import { value as present } from "./present.js" with {
                first: "one",
                second: "two",
            };
            globalThis.__attributeLoader2 = cached + absent + empty + present;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__attributeLoader2 === 42");
    assert_eq!(
        &*controls.checks.borrow(),
        &[
            vec![("cache".to_owned(), "hit".to_owned())],
            vec![("flavor".to_owned(), "first".to_owned())],
            vec![("flavor".to_owned(), "second".to_owned())],
            vec![
                ("first".to_owned(), "one".to_owned()),
                ("second".to_owned(), "two".to_owned()),
            ],
        ]
    );
    assert_eq!(
        &*controls.loads.borrow(),
        &[
            RecordedAttributeLoad {
                name: "pkg/shared.js".to_owned(),
                attributes: Some(vec![("flavor".to_owned(), "first".to_owned())]),
            },
            RecordedAttributeLoad {
                name: "pkg/absent.js".to_owned(),
                attributes: None,
            },
            RecordedAttributeLoad {
                name: "pkg/empty.js".to_owned(),
                attributes: None,
            },
            RecordedAttributeLoad {
                name: "pkg/present.js".to_owned(),
                attributes: Some(vec![
                    ("first".to_owned(), "one".to_owned()),
                    ("second".to_owned(), "two".to_owned()),
                ]),
            },
        ]
    );
    assert_eq!(controls.normalizations.borrow().len(), 6);
}

#[test]
fn attribute_check_precedes_following_syntax_and_all_resolution_callbacks() {
    let runtime = Runtime::new();
    let (loader, controls) =
        AttributeModuleLoader::new([("pkg/dependency.js", "export const value = 42;")]);
    controls.reject_checks.set(true);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename(
            r#"import "./dependency.js" with { unsupported: "x" }; let = ;"#,
            "pkg/entry.js",
        ),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("attribute check failure did not materialize a TypeError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("fixture rejected import attributes"))
    );
    assert_eq!(
        &*controls.checks.borrow(),
        &[vec![("unsupported".to_owned(), "x".to_owned())]]
    );
    assert!(controls.normalizations.borrow().is_empty());
    assert!(controls.loads.borrow().is_empty());

    controls.reject_checks.set(false);
    let module = context
        .compile_module_with_filename(
            r#"
            import { value } from "./dependency.js" with { type: "javascript" };
            globalThis.__attributeCheckRetry = value;
            "#,
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__attributeCheckRetry === 42");
    assert_eq!(controls.loads.borrow().len(), 1);
}

#[test]
fn dependency_attribute_check_failure_rolls_back_graph_for_retry() {
    let runtime = Runtime::new();
    let (loader, controls) = AttributeModuleLoader::new([
        (
            "pkg/a.js",
            r#"
            import { value } from "./dependency.js" with { type: "javascript" };
            export const answer = value + 1;
            "#,
        ),
        ("pkg/dependency.js", "export const value = 41;"),
    ]);
    controls.reject_checks.set(true);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename(
            "import { answer } from './a.js'; export { answer };",
            "pkg/entry.js",
        ),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(
        controls
            .loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/a.js"]
    );

    controls.reject_checks.set(false);
    let module = context
        .compile_module_with_filename(
            "import { answer } from './a.js'; globalThis.__attributeRollback = answer;",
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__attributeRollback === 42");
    assert_eq!(
        controls
            .loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/a.js", "pkg/a.js", "pkg/dependency.js"]
    );
    assert_eq!(controls.checks.borrow().len(), 2);
}

#[test]
fn loader2_failure_unpublishes_root_and_retries_with_same_attributes() {
    let runtime = Runtime::new();
    let (loader, controls) =
        AttributeModuleLoader::new([("pkg/dependency.js", "export const value = 42;")]);
    controls.fail_loads.set(true);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let source = r#"
        import { value } from "./dependency.js" with { type: "javascript" };
        globalThis.__loader2Retry = value;
    "#;

    assert!(matches!(
        context.compile_module_with_filename(source, "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("could not load module 'pkg/dependency.js': fixture loader2 failure")
    );
    controls.fail_loads.set(false);

    let module = context
        .compile_module_with_filename(source, "pkg/entry.js")
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__loader2Retry === 42");
    assert_eq!(controls.checks.borrow().len(), 2);
    assert_eq!(controls.loads.borrow().len(), 2);
    assert!(controls.loads.borrow().iter().all(
        |load| load.attributes == Some(vec![("type".to_owned(), "javascript".to_owned())])
    ));
}
