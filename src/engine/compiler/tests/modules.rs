use super::*;

#[derive(Default)]
struct RecordingModuleAttributeChecker {
    calls: Vec<Vec<(String, String)>>,
    failure: Option<ModuleCompileFailure>,
}

impl ModuleImportAttributeChecker for RecordingModuleAttributeChecker {
    fn publish_request(&mut self, _request: &ModuleRequest) -> Result<(), ModuleCompileFailure> {
        Ok(())
    }

    fn check(
        &mut self,
        attributes: &[crate::engine::code::module::ModuleImportAttribute],
    ) -> Result<(), ModuleCompileFailure> {
        self.calls.push(
            attributes
                .iter()
                .map(|attribute| {
                    (
                        attribute.key.to_utf8_lossy(),
                        attribute.value.to_utf8_lossy(),
                    )
                })
                .collect(),
        );
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecordedRequestAttributes {
    Absent,
    Present(Vec<(String, String)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModuleRequestLifecycleEvent {
    Published {
        specifier: String,
        attributes: RecordedRequestAttributes,
    },
    Checked(Vec<(String, String)>),
}

#[derive(Default)]
struct RecordingModuleRequestLifecycle {
    events: Vec<ModuleRequestLifecycleEvent>,
    fail_publish_at: Option<usize>,
    fail_check_at: Option<usize>,
    published: usize,
    checked: usize,
}

fn recorded_attributes(
    attributes: &[crate::engine::code::module::ModuleImportAttribute],
) -> Vec<(String, String)> {
    attributes
        .iter()
        .map(|attribute| {
            (
                attribute.key.to_utf8_lossy(),
                attribute.value.to_utf8_lossy(),
            )
        })
        .collect()
}

impl ModuleImportAttributeChecker for RecordingModuleRequestLifecycle {
    fn publish_request(&mut self, request: &ModuleRequest) -> Result<(), ModuleCompileFailure> {
        self.published += 1;
        self.events.push(ModuleRequestLifecycleEvent::Published {
            specifier: request.specifier.to_utf8_lossy(),
            attributes: match &request.attributes {
                ModuleImportAttributes::Absent => RecordedRequestAttributes::Absent,
                ModuleImportAttributes::Present(attributes) => {
                    RecordedRequestAttributes::Present(recorded_attributes(attributes))
                }
            },
        });
        if self.fail_publish_at == Some(self.published) {
            return Err(ModuleCompileFailure::Host);
        }
        Ok(())
    }

    fn check(
        &mut self,
        attributes: &[crate::engine::code::module::ModuleImportAttribute],
    ) -> Result<(), ModuleCompileFailure> {
        self.checked += 1;
        self.events
            .push(ModuleRequestLifecycleEvent::Checked(recorded_attributes(
                attributes,
            )));
        if self.fail_check_at == Some(self.checked) {
            return Err(ModuleCompileFailure::Host);
        }
        Ok(())
    }
}

#[test]
fn module_root_is_strict_has_no_script_completion_and_owns_declaration_slots() {
    let module = compile_unlinked_module_with_filename(
        "export {}; export { answer as value }; const answer = 42;",
        "file:///module.js",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let function = module.function();
    assert!(function.metadata().strict);
    assert!(function.metadata().is_module);
    assert_eq!(
        function.metadata().function_kind,
        BytecodeFunctionKind::Async
    );
    assert!(!module.has_top_level_await());
    assert_eq!(function.func_name(), None);
    assert!(function.local_definitions().is_empty());
    assert_eq!(function.closure_variables().len(), 1);
    assert_eq!(
        function.closure_variables()[0].source,
        ClosureSource::ModuleDeclaration
    );
    assert!(function.closure_variables()[0].is_lexical);
    assert!(function.closure_variables()[0].is_const);
    assert!(
        function
            .closure_variables()
            .iter()
            .all(|descriptor| !matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration | ClosureSource::Global
            ))
    );
    assert!(matches!(
        &function.code()[..4],
        [
            Instruction::PushThis,
            Instruction::IfFalse(4),
            Instruction::Undefined,
            Instruction::Return,
        ]
    ));
    assert!(function.code().windows(2).any(|window| matches!(
        window,
        [Instruction::PushI32(42), Instruction::InitializeVarRef(0)]
    )));
    assert!(matches!(
        function
            .code()
            .get(function.code().len().saturating_sub(2)..),
        Some([Instruction::Undefined, Instruction::Return])
    ));
    assert_eq!(module.exports().len(), 1);
    assert_eq!(module.exports()[0].export_name.to_utf8_lossy(), "value");
    assert_eq!(
        module.exports()[0].target,
        ModuleExportTarget::Local { closure_index: 0 }
    );
}

#[test]
fn module_export_duplicate_and_missing_binding_are_early_errors() {
    let duplicate = compile_unlinked_module_with_filename(
        "const a=1,b=2; export {a as value}; export {b as value};",
        "duplicate.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap_err();
    assert_eq!(duplicate.kind(), ErrorKind::Syntax);
    assert!(duplicate.message().contains("duplicate exported name"));

    let missing = compile_unlinked_module_with_filename(
        "export { missing };",
        "missing.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::Syntax);
    assert!(missing.message().contains("does not exist"));
}

#[test]
fn module_normal_bindings_follow_quickjs_lexical_scope_overlap() {
    for source in [
        "var value; { let value; }",
        "for (var value; false;) {} for (let value; false;) {}",
        "for await (var value of []) {} for await (let value of []) {}",
        "function value() {} { let value; }",
        "export var value; { const value = 1; }",
        // QuickJS keeps the first module-global record's declaration scope.
        "var value; { var value; let value; }",
        "import value from './dependency.js'; var value; { let value; }",
    ] {
        compile_unlinked_module_with_filename(
            source,
            "module-inner-lexical-shadow.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_or_else(|error| panic!("QuickJS accepts {source:?}: {error}"));
    }

    for source in [
        "var value; let value;",
        "{ var value; let value; }",
        "{ { var value; } let value; }",
        "let value; var value;",
        "export var value; export let value;",
        "var value; { let value; let value; }",
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "module-overlapping-lexical.mjs",
            DebugInfoMode::StripDebug,
        )
        .expect_err("overlapping module var and lexical declarations must conflict");
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
    }
}

#[test]
fn module_inner_lexical_shadow_keeps_the_exported_module_cell() {
    let module = compile_unlinked_module_with_filename(
        "for (var value of [1]) {} export { value }; for (let value of [42]) { void value; }",
        "module-export-inner-shadow.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();

    let [export] = module.exports() else {
        panic!("module var export table changed: {:?}", module.exports());
    };
    assert_eq!(export.export_name.to_utf8_lossy(), "value");
    let ModuleExportTarget::Local { closure_index } = export.target else {
        panic!("module var export stopped targeting its local cell");
    };
    let function = module.function();
    assert_eq!(function.closure_variables().len(), 1);
    let descriptor = function.closure_variables()[usize::from(closure_index)];
    assert_eq!(descriptor.source, ClosureSource::ModuleDeclaration);
    assert!(!descriptor.is_lexical);
    assert!(!descriptor.is_const);
    assert!(matches!(
        module.link_initializers(),
        [crate::engine::code::module::ModuleLinkInitializer {
            closure_index: actual,
            value: crate::engine::code::module::ModuleLinkInitializerValue::Undefined,
        }] if *actual == closure_index
    ));
    let value = JsString::from_static("value");
    assert!(
        function.local_definitions().iter().any(|definition| {
            definition.name.as_ref() == Some(&value) && definition.is_lexical
        })
    );
}

#[test]
fn module_string_export_names_reject_unpaired_surrogates_like_quickjs() {
    for source in [
        r#"const value = 42; export { value as "\ud800" };"#,
        r#"import { "\udfff" as value } from "dependency.js"; void value;"#,
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "unpaired-module-name.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax);
        assert_eq!(error.message(), "contains unpaired surrogate");
    }
}

#[test]
fn module_function_redeclarations_follow_quickjs_source_order() {
    // QuickJS 2026-06-04 retains the function initializer when a later `var`
    // reuses its module cell.
    let allowed = compile_unlinked_module_with_filename(
        "function value(){ return 42; } var value;",
        "function-then-var.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert_eq!(allowed.link_initializers().len(), 1);
    assert!(matches!(
        allowed.link_initializers()[0].value,
        crate::engine::code::module::ModuleLinkInitializerValue::Function { .. }
    ));

    for source in [
        "var value; function value(){}",
        "function value(){} function value(){}",
        "import value from './dependency.js'; var value; function value(){}",
        "var value; import value from './dependency.js'; function value(){}",
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "invalid-function-redeclaration.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert!(
            error
                .message()
                .contains("invalid redefinition of global identifier in module code"),
            "{source}: {}",
            error.message()
        );
    }

    // Preserve QuickJS's first-global-entry scope quirk. A var first seen in
    // a nested statement still uses the module cell, but does not make a
    // later Program-level function declaration an early error.
    for source in [
        "{ var value; } function value(){ return 42; }",
        "if (false) { var value; } function value(){ return 42; }",
        "for (;;) { var value; break; } function value(){ return 42; }",
        "{ var value; } var value; function value(){ return 42; }",
        "{ var value; } function value(){ return 1; } function value(){ return 42; }",
        "import value from './dependency.js'; { var value; } function value(){ return 42; }",
        "{ var value; } import value from './dependency.js'; function value(){ return 42; }",
    ] {
        let module = compile_unlinked_module_with_filename(
            source,
            "nested-var-then-function.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_or_else(|error| panic!("{source}: {error:?}"));
        assert!(matches!(
            module.link_initializers()[0].value,
            crate::engine::code::module::ModuleLinkInitializerValue::Function { .. }
        ));
    }

    let first_program_var_wins_the_check = compile_unlinked_module_with_filename(
        "var value; { var value; } function value(){}",
        "program-var-first.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap_err();
    assert_eq!(first_program_var_wins_the_check.kind(), ErrorKind::Syntax);

    for source in [
        "function value(){} function value( {",
        "function value(){} function value(){ @ }",
        "function value(){} function value",
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "redefinition-precedes-tail.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert!(
            error
                .message()
                .contains("invalid redefinition of global identifier in module code"),
            "{source}: {}",
            error.message()
        );
    }
}

#[test]
fn module_function_redeclaration_diagnostics_match_quickjs_header_position() {
    for (source, expected_column) in [
        ("var value; function value(){}", 26),
        ("var value; function* value(){}", 27),
        ("var value; async function value(){}", 32),
        ("var value; async function* value(){}", 33),
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "function-redeclaration-diagnostic.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(
            error.message(),
            "invalid redefinition of global identifier in module code",
            "{source}"
        );
        let start = error.span().expect("syntax error lost its span").start;
        assert_eq!((start.line, start.column), (1, expected_column), "{source}");
    }
}

#[test]
fn module_root_rejects_html_comments_and_strict_with_before_execution() {
    for source in ["<!-- hidden\nexport {};", "with ({}) {}"] {
        let error =
            compile_unlinked_module_with_filename(source, "strict.mjs", DebugInfoMode::StripDebug)
                .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
    }
}

#[test]
fn module_import_attributes_preserve_authored_state_order_and_decoded_strings() {
    let module = compile_unlinked_module_with_filename(
        r#"
        import "./absent.js";
        import "./empty.js" with {};
        import value from "./binding.js"
        with { if: "keyword", "double\u002dkey": '\u0078', 'single': "", };
        export { value as renamed } from "./indirect.js" with { first: "1" };
        export * from "./star.js" with { second: '2' };
        "#,
        "attributes.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();

    let requests = module.requested_modules();
    assert_eq!(requests.len(), 5);
    assert!(matches!(
        requests[0].attributes,
        ModuleImportAttributes::Absent
    ));
    assert_eq!(requests[0].attributes.syntactic(), None);
    assert_eq!(requests[0].attributes.effective(), None);

    let ModuleImportAttributes::Present(empty) = &requests[1].attributes else {
        panic!("empty with clause was not retained");
    };
    assert!(empty.is_empty());
    assert!(requests[1].attributes.syntactic().unwrap().is_empty());
    assert_eq!(requests[1].attributes.effective(), None);

    let binding = requests[2]
        .attributes
        .effective()
        .expect("binding import attributes");
    assert_eq!(
        binding
            .iter()
            .map(|attribute| (
                attribute.key.to_utf8_lossy(),
                attribute.value.to_utf8_lossy()
            ))
            .collect::<Vec<_>>(),
        [
            ("if".to_owned(), "keyword".to_owned()),
            ("double-key".to_owned(), "x".to_owned()),
            ("single".to_owned(), String::new()),
        ]
    );
    assert_eq!(
        requests[3].attributes.effective().unwrap()[0]
            .key
            .to_utf8_lossy(),
        "first"
    );
    assert_eq!(
        requests[4].attributes.effective().unwrap()[0]
            .value
            .to_utf8_lossy(),
        "2"
    );
}

#[test]
fn module_import_attribute_early_errors_match_pinned_quickjs() {
    for source in [
        r#"import "x" with { type: "json", "typ\u0065": "" };"#,
        r#"import value from "x" with { type: "json", 'typ\u0065': "" };"#,
        r#"export * from "x" with { type: "json", typ\u0065: "" };"#,
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "duplicate-attribute.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), "duplicate with key", "{source}");
        assert_eq!(
            error.span().unwrap().start.byte_offset,
            source.rfind("\"\"").unwrap(),
            "{source}"
        );
    }

    let invalid_key = r#"import "x" with { 0: "json" };"#;
    let error = compile_unlinked_module_with_filename(
        invalid_key,
        "invalid-key.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "identifier expected");
    assert_eq!(
        error.span().unwrap().start.byte_offset,
        invalid_key.find('0').unwrap()
    );

    let invalid_value = r#"import "x" with { type: json };"#;
    let error = compile_unlinked_module_with_filename(
        invalid_value,
        "invalid-value.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Syntax);
    assert_eq!(error.message(), "string expected");
    assert_eq!(
        error.span().unwrap().start.byte_offset,
        invalid_value.find("type").unwrap()
    );
}

#[test]
fn module_import_attribute_checker_is_synchronous_and_skips_empty_clauses() {
    let mut checker = RecordingModuleAttributeChecker::default();
    let module = compile_unlinked_module_with_name_and_attribute_checker(
        r#"
        import "empty" with {};
        import "first" with { a: "1", b: "2" };
        export * from "second" with { c: "3" };
        "#,
        JsString::from_static("checker.mjs"),
        DebugInfoMode::StripDebug,
        Some(&mut checker),
    )
    .unwrap();
    assert!(matches!(
        module.requested_modules()[0].attributes,
        ModuleImportAttributes::Present(ref attributes) if attributes.is_empty()
    ));
    assert_eq!(
        checker.calls,
        [
            vec![
                ("a".to_owned(), "1".to_owned()),
                ("b".to_owned(), "2".to_owned())
            ],
            vec![("c".to_owned(), "3".to_owned())],
        ]
    );

    let mut rejecting = RecordingModuleAttributeChecker {
        calls: Vec::new(),
        failure: Some(Error::new(ErrorKind::Type, "host rejected attributes").into()),
    };
    let ModuleCompileFailure::Engine(error) =
        compile_unlinked_module_with_name_and_attribute_checker(
            r#"import "first" with { type: "json" }; @"#,
            JsString::from_static("checker-order.mjs"),
            DebugInfoMode::StripDebug,
            Some(&mut rejecting),
        )
        .unwrap_err()
    else {
        panic!("message-based checker failure was not an engine diagnostic");
    };
    assert_eq!(error.kind(), ErrorKind::Type);
    assert_eq!(error.message(), "host rejected attributes");
    assert_eq!(error.span(), None);
    assert_eq!(
        rejecting.calls,
        [vec![("type".to_owned(), "json".to_owned())]]
    );

    let mut throwing = RecordingModuleAttributeChecker {
        calls: Vec::new(),
        failure: Some(ModuleCompileFailure::Host),
    };
    let failure = compile_unlinked_module_with_name_and_attribute_checker(
        r#"import "first" with { type: "json" }; @"#,
        JsString::from_static("checker-throw-order.mjs"),
        DebugInfoMode::StripDebug,
        Some(&mut throwing),
    )
    .unwrap_err();
    assert_eq!(failure, ModuleCompileFailure::Host);
    assert_eq!(
        throwing.calls,
        [vec![("type".to_owned(), "json".to_owned())]]
    );
}

#[test]
fn module_requests_publish_in_source_order_before_attribute_checks() {
    let mut lifecycle = RecordingModuleRequestLifecycle::default();
    let module = compile_unlinked_module_with_name_and_attribute_checker(
        r#"
        import "bare";
        import "empty" with {};
        export { value } from "checked" with { type: "json", flavor: "test" };
        export * from "tail";
        "#,
        JsString::from_static("request-lifecycle.mjs"),
        DebugInfoMode::StripDebug,
        Some(&mut lifecycle),
    )
    .unwrap();

    assert_eq!(module.requested_modules().len(), 4);
    assert_eq!(
        lifecycle.events,
        [
            ModuleRequestLifecycleEvent::Published {
                specifier: "bare".to_owned(),
                attributes: RecordedRequestAttributes::Absent,
            },
            ModuleRequestLifecycleEvent::Published {
                specifier: "empty".to_owned(),
                attributes: RecordedRequestAttributes::Present(vec![]),
            },
            ModuleRequestLifecycleEvent::Published {
                specifier: "checked".to_owned(),
                attributes: RecordedRequestAttributes::Present(vec![
                    ("type".to_owned(), "json".to_owned()),
                    ("flavor".to_owned(), "test".to_owned()),
                ]),
            },
            ModuleRequestLifecycleEvent::Checked(vec![
                ("type".to_owned(), "json".to_owned()),
                ("flavor".to_owned(), "test".to_owned()),
            ]),
            ModuleRequestLifecycleEvent::Published {
                specifier: "tail".to_owned(),
                attributes: RecordedRequestAttributes::Absent,
            },
        ]
    );
}

#[test]
fn module_request_publication_and_checker_failures_stop_without_repetition() {
    let mut publish_failure = RecordingModuleRequestLifecycle {
        fail_publish_at: Some(2),
        ..RecordingModuleRequestLifecycle::default()
    };
    let failure = compile_unlinked_module_with_name_and_attribute_checker(
        r#"
        import "prefix";
        import "rejected" with { type: "json" };
        import "unreached";
        "#,
        JsString::from_static("request-publish-failure.mjs"),
        DebugInfoMode::StripDebug,
        Some(&mut publish_failure),
    )
    .unwrap_err();
    assert_eq!(failure, ModuleCompileFailure::Host);
    assert_eq!(
        publish_failure.events,
        [
            ModuleRequestLifecycleEvent::Published {
                specifier: "prefix".to_owned(),
                attributes: RecordedRequestAttributes::Absent,
            },
            ModuleRequestLifecycleEvent::Published {
                specifier: "rejected".to_owned(),
                attributes: RecordedRequestAttributes::Present(vec![(
                    "type".to_owned(),
                    "json".to_owned(),
                )]),
            },
        ]
    );

    let mut check_failure = RecordingModuleRequestLifecycle {
        fail_check_at: Some(1),
        ..RecordingModuleRequestLifecycle::default()
    };
    let failure = compile_unlinked_module_with_name_and_attribute_checker(
        r#"
        import "rejected" with { type: "json" ;
        import "unreached";
        "#,
        JsString::from_static("request-check-failure.mjs"),
        DebugInfoMode::StripDebug,
        Some(&mut check_failure),
    )
    .unwrap_err();
    assert_eq!(failure, ModuleCompileFailure::Host);
    assert_eq!(
        check_failure.events,
        [
            ModuleRequestLifecycleEvent::Published {
                specifier: "rejected".to_owned(),
                attributes: RecordedRequestAttributes::Present(vec![(
                    "type".to_owned(),
                    "json".to_owned(),
                )]),
            },
            ModuleRequestLifecycleEvent::Checked(vec![("type".to_owned(), "json".to_owned(),)]),
        ]
    );
}

#[test]
fn module_import_meta_uses_one_hidden_module_cell() {
    let module = compile_unlinked_module_with_filename(
        "globalThis.meta = import.meta; globalThis.readMeta = () => import.meta;",
        "import-meta.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let meta_slots = module
        .function()
        .closure_variables()
        .iter()
        .enumerate()
        .filter_map(|(index, descriptor)| {
            (descriptor.source == ClosureSource::ModuleImportMeta).then_some((index, descriptor))
        })
        .collect::<Vec<_>>();
    let [(index, descriptor)] = meta_slots.as_slice() else {
        panic!("module did not retain exactly one import.meta cell: {meta_slots:?}");
    };
    assert!(descriptor.is_lexical);
    assert!(descriptor.is_const);
    assert_eq!(descriptor.kind, ClosureVariableKind::Normal);
    assert!(module.function().code().iter().any(|instruction| {
        matches!(instruction, Instruction::GetVarRefCheck(found) if usize::from(*found) == *index)
    }));

    let script = compile_unlinked_script_with_filename(
        "import.meta",
        "import-meta.js",
        DebugInfoMode::StripDebug,
    )
    .unwrap_err();
    assert_eq!(script.kind(), ErrorKind::Syntax);
    assert!(script.message().contains("only valid in module code"));

    for source in [
        "import.meta = 1;",
        "++import.meta;",
        "[import.meta] = [];",
        "for (import.meta in {}) {}",
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "invalid-import-meta-target.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}: {error}");
    }
}

#[test]
fn module_named_imports_publish_requests_live_cells_and_local_reexports() {
    let module = compile_unlinked_module_with_filename(
        r#"
        import "./side-effect.js";
        import { value, "external-name" as alias } from "./dependency.js";
        export { value as live };
        globalThis.imported = value + alias;
        "#,
        "file:///entry.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();

    assert_eq!(module.requested_modules().len(), 2);
    assert_eq!(
        module.requested_modules()[0].specifier.to_utf8_lossy(),
        "./side-effect.js"
    );
    assert_eq!(
        module.requested_modules()[1].specifier.to_utf8_lossy(),
        "./dependency.js"
    );
    assert_eq!(module.imports().len(), 2);
    assert_eq!(module.imports()[0].request.0, 1);
    assert!(matches!(
        &module.imports()[0].import_name,
        ModuleImportName::Name(name) if name.to_utf8_lossy() == "value"
    ));
    assert_eq!(module.imports()[0].closure_index, 0);
    assert!(matches!(
        &module.imports()[1].import_name,
        ModuleImportName::Name(name) if name.to_utf8_lossy() == "external-name"
    ));
    assert_eq!(module.imports()[1].closure_index, 1);
    let import_descriptors = module
        .function()
        .closure_variables()
        .iter()
        .filter(|descriptor| descriptor.source == ClosureSource::ModuleImport)
        .collect::<Vec<_>>();
    assert_eq!(import_descriptors.len(), 2);
    assert!(
        import_descriptors
            .iter()
            .all(|descriptor| descriptor.is_lexical
                && descriptor.is_const
                && descriptor.kind == ClosureVariableKind::ModuleImportView)
    );
    assert_eq!(
        module.exports()[0].target,
        ModuleExportTarget::Local { closure_index: 0 }
    );
}

#[test]
fn module_default_import_clauses_lower_to_named_default_cells_in_source_order() {
    let module = compile_unlinked_module_with_filename(
        r#"
        import onlyDefault from "./default.js";
        import defaultWithNamed, { value as renamed } from "./named.js";
        import defaultWithNamespace, * as namespace from "./namespace.js";
        void onlyDefault;
        void defaultWithNamed;
        void renamed;
        void defaultWithNamespace;
        void namespace;
        "#,
        "default-imports.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();

    assert_eq!(
        module
            .requested_modules()
            .iter()
            .map(|request| request.specifier.to_utf8_lossy())
            .collect::<Vec<_>>(),
        ["./default.js", "./named.js", "./namespace.js"]
    );
    assert_eq!(module.imports().len(), 5);

    for (index, request) in [0, 1, 1, 2, 2].into_iter().enumerate() {
        assert_eq!(module.imports()[index].request.0, request);
        assert_eq!(module.imports()[index].closure_index, index as u16);
    }
    for index in [0, 1, 3] {
        assert!(matches!(
            &module.imports()[index].import_name,
            ModuleImportName::Name(name) if name.to_utf8_lossy() == "default"
        ));
    }
    assert!(matches!(
        &module.imports()[2].import_name,
        ModuleImportName::Name(name) if name.to_utf8_lossy() == "value"
    ));
    assert!(matches!(
        &module.imports()[4].import_name,
        ModuleImportName::Namespace
    ));

    let import_descriptors = module
        .function()
        .closure_variables()
        .iter()
        .enumerate()
        .filter(|(_, descriptor)| descriptor.source == ClosureSource::ModuleImport)
        .collect::<Vec<_>>();
    assert_eq!(
        import_descriptors
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert!(import_descriptors.iter().all(|(_, descriptor)| {
        descriptor.is_lexical
            && descriptor.is_const
            && descriptor.kind == ClosureVariableKind::ModuleImportView
    }));
}

#[test]
fn module_namespace_import_uses_a_typed_target_and_fresh_declaration_cell() {
    let module = compile_unlinked_module_with_filename(
        r#"
        import { first } from "./named.js";
        import * as namespace from "./namespace.js";
        export { first };
        void namespace;
        "#,
        "namespace-import.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();

    assert_eq!(module.requested_modules().len(), 2);
    assert_eq!(
        module.requested_modules()[0].specifier.to_utf8_lossy(),
        "./named.js"
    );
    assert_eq!(
        module.requested_modules()[1].specifier.to_utf8_lossy(),
        "./namespace.js"
    );
    assert_eq!(module.imports().len(), 2);
    assert!(matches!(
        &module.imports()[0].import_name,
        ModuleImportName::Name(_)
    ));
    assert_eq!(module.imports()[0].closure_index, 0);
    assert!(matches!(
        &module.imports()[1].import_name,
        ModuleImportName::Namespace
    ));
    assert_eq!(module.imports()[1].request.0, 1);
    assert_eq!(module.imports()[1].closure_index, 1);

    let descriptor = module.function().closure_variables()[1];
    assert_eq!(descriptor.source, ClosureSource::ModuleDeclaration);
    assert!(descriptor.is_lexical);
    assert!(descriptor.is_const);
    assert_eq!(descriptor.kind, ClosureVariableKind::Normal);
    assert!(
        !module
            .function()
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::InitializeVarRef(1)))
    );
}

#[test]
fn module_indirect_and_star_exports_preserve_request_and_table_order() {
    let module = compile_unlinked_module_with_filename(
        r#"
        export { value as renamed, default as fallback } from "./named.js";
        export * from "./star.js";
        export * as namespace from "./namespace.js";
        "#,
        "reexports.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();

    assert_eq!(
        module
            .requested_modules()
            .iter()
            .map(|request| request.specifier.to_utf8_lossy())
            .collect::<Vec<_>>(),
        ["./named.js", "./star.js", "./namespace.js"]
    );
    assert_eq!(module.exports().len(), 3);
    assert_eq!(module.exports()[0].export_name.to_utf8_lossy(), "renamed");
    assert!(matches!(
        &module.exports()[0].target,
        ModuleExportTarget::Indirect {
            request,
            import_name: ModuleImportName::Name(name),
        } if request.0 == 0 && name.to_utf8_lossy() == "value"
    ));
    assert_eq!(module.exports()[1].export_name.to_utf8_lossy(), "fallback");
    assert!(matches!(
        &module.exports()[1].target,
        ModuleExportTarget::Indirect {
            request,
            import_name: ModuleImportName::Name(name),
        } if request.0 == 0 && name.to_utf8_lossy() == "default"
    ));
    assert_eq!(module.star_exports().len(), 1);
    assert_eq!(module.star_exports()[0].request.0, 1);
    assert_eq!(module.exports()[2].export_name.to_utf8_lossy(), "namespace");
    assert!(matches!(
        &module.exports()[2].target,
        ModuleExportTarget::Indirect {
            request,
            import_name: ModuleImportName::Namespace,
        } if request.0 == 2
    ));
}

#[test]
fn module_expression_default_export_owns_one_inaccessible_tdz_cell() {
    let null_default = compile_unlinked_module_with_filename(
        "export default null;",
        "null-default-expression.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert!(null_default.link_initializers().is_empty());
    assert!(matches!(
        null_default.function().closure_variables(),
        [descriptor]
            if descriptor.source == ClosureSource::ModuleDeclaration
                && descriptor.is_lexical
                && !descriptor.is_const
                && descriptor.kind == ClosureVariableKind::Normal
    ));
    assert!(
        null_default
            .function()
            .code()
            .windows(2)
            .any(|window| matches!(
                window,
                [Instruction::Null, Instruction::InitializeVarRef(0)]
            ))
    );

    let module = compile_unlinked_module_with_filename(
        r#"
        import * as namespace from "./namespace.js";
        const before = namespace;
        export default 6 * 7;
        const after = before;
        "#,
        "default-expression.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();

    assert_eq!(module.function().closure_variables().len(), 4);
    let default_descriptor = module.function().closure_variables()[2];
    assert_eq!(default_descriptor.source, ClosureSource::ModuleDeclaration);
    assert!(default_descriptor.is_lexical);
    assert!(!default_descriptor.is_const);
    assert_eq!(default_descriptor.kind, ClosureVariableKind::Normal);
    assert_eq!(module.exports().len(), 1);
    assert_eq!(module.exports()[0].export_name.to_utf8_lossy(), "default");
    assert_eq!(
        module.exports()[0].target,
        ModuleExportTarget::Local { closure_index: 2 }
    );
    assert!(module.function().code().windows(4).any(|window| matches!(
        window,
        [
            Instruction::PushI32(6),
            Instruction::PushI32(7),
            Instruction::Mul,
            Instruction::InitializeVarRef(2),
        ]
    )));

    let named = compile_unlinked_module_with_filename(
        "export default (() => 42);",
        "named-default-expression.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert!(
        named
            .function()
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::SetName(_)))
    );
}

#[test]
fn module_named_generator_export_keeps_the_function_link_initializer() {
    let module = compile_unlinked_module_with_filename(
        "export function* values() { yield 42; }",
        "generator-export.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();

    assert_eq!(module.exports().len(), 1);
    assert_eq!(module.exports()[0].export_name.to_utf8_lossy(), "values");
    assert_eq!(
        module.exports()[0].target,
        ModuleExportTarget::Local { closure_index: 0 }
    );
    assert!(matches!(
        module.link_initializers(),
        [crate::engine::code::module::ModuleLinkInitializer {
            closure_index: 0,
            value: crate::engine::code::module::ModuleLinkInitializerValue::Function { .. },
        }]
    ));
    let generator = module
        .function()
        .constants()
        .iter()
        .filter_map(crate::engine::code::function::UnlinkedConstant::as_child)
        .find(|child| child.metadata().function_kind == BytecodeFunctionKind::Generator)
        .expect("exported generator declaration lost its child bytecode");
    assert!(
        generator
            .code()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::InitialYield))
    );
}

#[test]
fn module_new_export_forms_share_the_duplicate_export_table() {
    for source in [
        "const value=1; export {value as name}; export {other as name} from './a.js';",
        "export {value as name} from './a.js'; export * as name from './b.js';",
        "const value=1; export default 0; export {value as default};",
        "export {a as name, b as name} from './a.js';",
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "duplicate-reexport.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert!(
            error.message().contains("duplicate exported name"),
            "{source}"
        );
    }
}

#[test]
fn module_default_function_declarations_keep_quickjs_link_hoists_and_name_inference() {
    for (source, expected_kind, expected_name) in [
        (
            "export default function () {}",
            BytecodeFunctionKind::Normal,
            None,
        ),
        (
            "export default function named() {}",
            BytecodeFunctionKind::Normal,
            Some("named"),
        ),
        (
            "export default function* () {}",
            BytecodeFunctionKind::Generator,
            None,
        ),
        (
            "export default function* named() {}",
            BytecodeFunctionKind::Generator,
            Some("named"),
        ),
        (
            "export default async function () {}",
            BytecodeFunctionKind::Async,
            None,
        ),
        (
            "export default async function named() {}",
            BytecodeFunctionKind::Async,
            Some("named"),
        ),
        (
            "export default async function* () {}",
            BytecodeFunctionKind::AsyncGenerator,
            None,
        ),
        (
            "export default async function* named() {}",
            BytecodeFunctionKind::AsyncGenerator,
            Some("named"),
        ),
    ] {
        let module = compile_unlinked_module_with_filename(
            source,
            "default-function-declaration.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));

        assert!(matches!(
            module.exports(),
            [export]
                if export.export_name.to_utf8_lossy() == "default"
                    && export.target == ModuleExportTarget::Local { closure_index: 0 }
        ));
        assert!(matches!(
            module.function().closure_variables(),
            [descriptor]
                if descriptor.source == ClosureSource::ModuleDeclaration
                    && !descriptor.is_lexical
                    && !descriptor.is_const
                    && descriptor.kind == ClosureVariableKind::Normal
        ));
        let [initializer] = module.link_initializers() else {
            panic!("{source}: default function lost its sole link initializer");
        };
        assert_eq!(initializer.closure_index, 0, "{source}");
        let crate::engine::code::module::ModuleLinkInitializerValue::Function {
            constant,
            inferred_name,
        } = initializer.value
        else {
            panic!("{source}: default function link value changed kind");
        };
        let child = module.function().constants()[constant as usize]
            .as_child()
            .expect("default function initializer lost its child bytecode");
        assert_eq!(child.metadata().function_kind, expected_kind, "{source}");
        assert_eq!(
            child.func_name().map(JsString::to_utf8_lossy),
            expected_name.map(str::to_owned),
            "{source}"
        );

        match expected_name {
            Some(_) => {
                assert_eq!(inferred_name, None, "{source}");
                assert!(module.function().code().windows(2).any(|window| matches!(
                    window,
                    [Instruction::FClosure(actual), Instruction::PutVarRef(0)]
                        if *actual == constant
                )));
            }
            None => {
                let name = inferred_name.expect("anonymous default function lost SetName");
                assert_eq!(
                    module.function().constants()[name as usize].as_primitive(),
                    Some(&crate::engine::value::PrimitiveValue::String(
                        JsString::from_static("default")
                    )),
                    "{source}"
                );
                assert!(module.function().code().windows(3).any(|window| matches!(
                    window,
                    [
                        Instruction::FClosure(actual),
                        Instruction::SetName(actual_name),
                        Instruction::PutVarRef(0),
                    ] if *actual == constant && *actual_name == name
                )));
            }
        }
    }
}

#[test]
fn module_default_class_declarations_keep_evaluation_tdz_and_default_class_name() {
    for (source, expected_name) in [
        ("export default class {}", "default"),
        ("export default class Named {}", "Named"),
    ] {
        let module = compile_unlinked_module_with_filename(
            source,
            "default-class-declaration.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));

        assert!(module.link_initializers().is_empty(), "{source}");
        assert!(matches!(
            module.exports(),
            [export]
                if export.export_name.to_utf8_lossy() == "default"
                    && export.target == ModuleExportTarget::Local { closure_index: 0 }
        ));
        assert!(matches!(
            module.function().closure_variables(),
            [descriptor]
                if descriptor.source == ClosureSource::ModuleDeclaration
                    && descriptor.is_lexical
                    && !descriptor.is_const
                    && descriptor.kind == ClosureVariableKind::Normal
        ));
        let class_name = module
            .function()
            .code()
            .iter()
            .find_map(|instruction| match instruction {
                Instruction::DefineClass { name, .. } => Some(*name),
                _ => None,
            })
            .expect("default class lost DefineClass");
        assert_eq!(
            module.function().constants()[class_name as usize].as_primitive(),
            Some(&crate::engine::value::PrimitiveValue::String(
                JsString::try_from_utf8(expected_name).unwrap()
            )),
            "{source}"
        );
        assert!(
            module
                .function()
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::InitializeVarRef(0))),
            "{source}"
        );
    }
}

#[test]
fn module_default_export_context_does_not_escape_into_nested_class_declarations() {
    for source in [
        "export default class { static { class {} } }",
        "export default class { method() { class {} } }",
        "export default class { field = (() => { class {} })(); }",
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "nested-anonymous-class.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert!(
            error
                .to_string()
                .contains("class statement requires a name"),
            "{source}: {error}"
        );
    }

    let module = compile_unlinked_module_with_filename(
        "export default class { static { class Nested {} } }",
        "nested-named-class.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert!(matches!(
        module.exports(),
        [export] if export.export_name.to_utf8_lossy() == "default"
    ));
}

#[test]
fn module_default_declarations_do_not_require_expression_terminators() {
    for source in [
        "export default function () {} export const answer = 42;",
        "export default class {} export const answer = 42;",
    ] {
        let module = compile_unlinked_module_with_filename(
            source,
            "default-declaration-terminator.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(
            module
                .exports()
                .iter()
                .map(|export| export.export_name.to_utf8_lossy())
                .collect::<Vec<_>>(),
            ["default", "answer"]
        );
    }
}

#[test]
fn module_import_attributes_and_invalid_clauses_preserve_binding_errors() {
    for source in [
        "import value from './dependency.js' with { type: 'json' };",
        "import value, { named } from './dependency.js' with { type: 'json' };",
        "import value, * as namespace from './dependency.js' with { type: 'json' };",
        "import { value } from './dependency.js' with { type: 'json' };",
    ] {
        let module = compile_unlinked_module_with_filename(
            source,
            "attribute-import.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_or_else(|error| panic!("{source}: {error}"));
        let [attribute] = module.requested_modules()[0]
            .attributes
            .effective()
            .expect("non-empty attributes")
        else {
            panic!("{source}: expected one attribute");
        };
        assert_eq!(attribute.key.to_utf8_lossy(), "type", "{source}");
        assert_eq!(attribute.value.to_utf8_lossy(), "json", "{source}");
    }

    for source in [
        "import value, from './dependency.js';",
        "import value from './dependency.js' globalThis.answer = value;",
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "invalid-import.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
    }

    for source in [
        "import value from './a.js'; import value from './b.js';",
        "import value, { other as value } from './a.js';",
        "import value, * as value from './a.js';",
        "import { value } from './a.js'; import { value } from './b.js';",
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "duplicate-import.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
    }
}

#[test]
fn module_import_eval_and_arguments_follow_quickjs_binding_diagnostics() {
    for (source, expected_column) in [
        ("import { eval } from './dependency.js';", 15),
        ("import { arguments } from './dependency.js';", 20),
        ("import { value as eval } from './dependency.js';", 24),
        ("import { value as arguments } from './dependency.js';", 29),
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "invalid-import-binding.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), "invalid import binding", "{source}");
        let start = error.span().expect("syntax error lost its span").start;
        assert_eq!((start.line, start.column), (1, expected_column), "{source}");
    }
}

#[test]
fn module_import_declaration_collisions_use_one_authenticated_import_slot() {
    let imports = [
        (
            "import value from './a.js';",
            ClosureVariableKind::ModuleImportView,
        ),
        (
            "import { value } from './a.js';",
            ClosureVariableKind::ModuleImportView,
        ),
        (
            "import * as value from './a.js';",
            ClosureVariableKind::Normal,
        ),
    ];
    let declarations = [
        (
            "var value;",
            ModuleImportCollisionDeclaration::Var,
            false,
            false,
        ),
        (
            "let value;",
            ModuleImportCollisionDeclaration::Lexical,
            true,
            false,
        ),
        (
            "const value = 1;",
            ModuleImportCollisionDeclaration::Lexical,
            true,
            false,
        ),
        (
            "class value {}",
            ModuleImportCollisionDeclaration::Lexical,
            true,
            false,
        ),
        (
            "function value() {}",
            ModuleImportCollisionDeclaration::Function,
            true,
            true,
        ),
    ];

    for (import, expected_kind) in imports {
        for (declaration_source, declaration, expects_initializer, expects_link_initializer) in
            declarations
        {
            for source in [
                format!("{import} {declaration_source}"),
                format!("{declaration_source} {import}"),
            ] {
                let module = compile_unlinked_module_with_filename(
                    &source,
                    "import-declaration-collision.mjs",
                    DebugInfoMode::StripDebug,
                )
                .unwrap_or_else(|error| panic!("{source}: {error}"));
                assert_eq!(module.function().closure_variables().len(), 1, "{source}");
                let descriptor = module.function().closure_variables()[0];
                assert_eq!(
                    descriptor.source,
                    ClosureSource::ModuleImportCollision,
                    "{source}"
                );
                assert!(descriptor.is_lexical && descriptor.is_const, "{source}");
                assert_eq!(descriptor.kind, expected_kind, "{source}");
                assert_eq!(
                    module.import_collisions(),
                    [crate::engine::code::module::ModuleImportCollision {
                        closure_index: 0,
                        declaration,
                    }],
                    "{source}"
                );
                assert_eq!(module.declaration_order(), [0], "{source}");
                let Instruction::IfFalse(body) = module.function().code()[1] else {
                    panic!("{source}: module collision root lost its dual entry");
                };
                let initializer_pc = module.function().code().iter().position(|instruction| {
                    matches!(instruction, Instruction::InitializeModuleImportCollision(0))
                });
                assert_eq!(initializer_pc.is_some(), expects_initializer, "{source}");
                if let Some(initializer_pc) = initializer_pc {
                    assert_eq!(
                        initializer_pc < body as usize,
                        expects_link_initializer,
                        "{source}"
                    );
                }
                assert_eq!(
                    module.link_initializers().len(),
                    usize::from(expects_link_initializer),
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn module_import_collisions_do_not_mask_declaration_redefinitions() {
    for source in [
        "var value; let value;",
        "function value() {} const value = 1;",
        "import value from './a.js'; var value; let value;",
        "var value; import value from './a.js'; class value {}",
        "import value from './a.js'; { var value; } function value() {} const value = 1;",
        "let value; import value from './a.js'; var value;",
        "import value from './a.js'; let value; function value() {}",
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "import-declaration-redefinition.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
    }
}

#[test]
fn module_function_collision_hoists_follow_declaration_not_import_order() {
    let module = compile_unlinked_module_with_filename(
        "import { value as first, value as second } from './a.js';\
         { var first; } function second() {} function first() {}",
        "import-function-declaration-order.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert_eq!(module.declaration_order(), [1, 0]);
    assert_eq!(
        module
            .link_initializers()
            .iter()
            .map(|initializer| initializer.closure_index)
            .collect::<Vec<_>>(),
        [1, 0]
    );
    let Instruction::IfFalse(body) = module.function().code()[1] else {
        panic!("module collision root lost its dual entry");
    };
    assert_eq!(
        module.function().code()[2..body as usize]
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::InitializeModuleImportCollision(index) => Some(*index),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [1, 0]
    );
}

#[test]
fn module_parse_negative_diagnostics_precede_unsupported_frontiers_like_quickjs() {
    for (source, message, column) in [
        (
            "export default var x = null; export default var x = null;",
            "unexpected token in expression: 'var'",
            16,
        ),
        (
            "export default const x = null;",
            "unexpected token in expression: 'const'",
            16,
        ),
        (
            "export default var x;",
            "unexpected token in expression: 'var'",
            16,
        ),
        ("await: 1;", "unexpected token in expression: ':'", 6),
        (r"\u0061wait: 1;", "'await' is a reserved identifier", 1),
    ] {
        let error = compile_unlinked_module_with_filename(
            source,
            "module-parse-negative-precedence.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
        assert_eq!(
            (
                error.span().unwrap().start.line,
                error.span().unwrap().start.column
            ),
            (1, column),
            "{source}"
        );
    }
}

#[test]
fn top_level_await_uses_async_module_lowering_and_is_sealed_separately() {
    for source in ["await 1;", "for await (const value of []) {}"] {
        let module = compile_unlinked_module_with_filename(
            source,
            "top-level-await.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap_or_else(|error| {
            panic!("top-level async module source rejected {source:?}: {error}")
        });
        assert!(module.has_top_level_await(), "{source}");
        assert_eq!(
            module.function().metadata().function_kind,
            BytecodeFunctionKind::Async,
            "{source}"
        );
        assert!(
            module
                .function()
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Await)),
            "{source}"
        );
    }

    let nested_only = compile_unlinked_module_with_filename(
        "async function nested() { await 1; } export { nested };",
        "nested-await.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert!(!nested_only.has_top_level_await());
    assert_eq!(
        nested_only.function().metadata().function_kind,
        BytecodeFunctionKind::Async
    );

    compile_unlinked_script("var await = 1; await += 41;")
        .expect("script-goal await identifier semantics drifted");
}

#[test]
fn module_var_and_function_use_the_link_only_dual_entry() {
    let module = compile_unlinked_module_with_filename(
        "export var value; export function answer(){ return 42; }",
        "dual-entry.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    let function = module.function();
    assert_eq!(function.closure_variables().len(), 2);
    assert!(
        function
            .closure_variables()
            .iter()
            .all(
                |descriptor| descriptor.source == ClosureSource::ModuleDeclaration
                    && !descriptor.is_lexical
                    && !descriptor.is_const
            )
    );
    assert!(matches!(function.code()[0], Instruction::PushThis));
    let Instruction::IfFalse(body) = function.code()[1] else {
        panic!("module root lost its link/evaluation entry guard");
    };
    let body = body as usize;
    assert!(matches!(
        &function.code()[2..body],
        [
            Instruction::Undefined,
            Instruction::PutVarRef(0),
            Instruction::FClosure(_),
            Instruction::PutVarRef(1),
            Instruction::Undefined,
            Instruction::Return,
        ]
    ));
    assert!(matches!(
        &function.code()[body..],
        [Instruction::Undefined, Instruction::Return]
    ));
}

#[test]
fn module_direct_eval_records_a_strict_local_variable_environment() {
    let module = compile_unlinked_module_with_filename(
        "let local=1; eval('var x; local');",
        "eval-module.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert_eq!(module.function().eval_environments().len(), 1);
    assert!(matches!(
        module.function().eval_environments()[0].variable_environment,
        EvalVariableEnvironment::StrictLocal(_)
    ));
}

#[test]
fn stripped_module_tree_retains_linker_binding_names() {
    let module = compile_unlinked_module_with_filename(
        "const local=1; export function read(){ return local; }",
        "stripped-module.mjs",
        DebugInfoMode::StripDebug,
    )
    .unwrap();
    assert!(matches!(
        module.function().closure_variables()[0].name,
        ClosureVariableName::Constant(_)
    ));
    let child = module
        .function()
        .constants()
        .iter()
        .find_map(|constant| constant.as_child())
        .expect("module function declaration has child bytecode");
    assert!(matches!(
        child.closure_variables(),
        [descriptor]
            if descriptor.source == ClosureSource::ParentClosure(0)
                && matches!(descriptor.name, ClosureVariableName::Constant(_))
    ));
}
