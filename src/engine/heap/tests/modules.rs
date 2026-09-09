use super::*;

fn parsing_module_record(
    realm: ContextId,
    name: &'static str,
    requests: &[&'static str],
    resolution: RawModuleResolutionState,
) -> RawModuleRecord {
    RawModuleRecord {
        name: JsString::from_static(name),
        body: RawModuleRecordBody::Parsing,
        import_meta: None,
        declaration_order: Rc::from([]),
        link_initializers: Rc::from([]),
        import_collisions: Rc::from([]),
        requested_modules: requests
            .iter()
            .map(|specifier| ModuleRequest {
                specifier: JsString::from_static(specifier),
                attributes: crate::engine::code::module::ModuleImportAttributes::Absent,
            })
            .collect::<Vec<_>>()
            .into(),
        imports: Rc::from([]),
        exports: Rc::from([]),
        star_exports: Rc::from([]),
        resolution,
        instance: None,
        namespace: RawModuleNamespaceState::Empty,
        link_status: RawModuleLinkStatus::Unlinked,
        evaluation: RawModuleEvaluationState::Unevaluated,
        has_top_level_await: false,
        evaluation_cycle_root: None,
        evaluation_promise: None,
        evaluation_resolve: None,
        evaluation_reject: None,
        pending_async_dependencies: 0,
        async_parent_modules: Vec::new(),
        async_evaluation_order: None,
        link_realm: None,
        compile_realm: realm,
    }
}

#[test]
fn parse_in_progress_module_replacements_preserve_prefix_and_resolution() {
    let mut heap = Heap::new();
    let realm = bytecode_test_realm(&mut heap);
    let initial = parsing_module_record(
        realm,
        "same.js",
        &["first.js"],
        RawModuleResolutionState::Unresolved,
    );
    let module = heap.publish_loaded_module(realm, initial.clone()).unwrap();

    let mut appended = initial.clone();
    appended.requested_modules = Rc::new(vec![
        ModuleRequest {
            specifier: JsString::from_static("first.js"),
            attributes: crate::engine::code::module::ModuleImportAttributes::Absent,
        },
        ModuleRequest {
            specifier: JsString::from_static("second.js"),
            attributes: crate::engine::code::module::ModuleImportAttributes::Absent,
        },
    ]);
    assert_eq!(
        heap.replace_loaded_module(module, appended.clone()),
        Ok(HeapCleanup::default())
    );
    assert_eq!(
        heap.transition_loaded_module(module, RawModuleTransition::BeginLink),
        Err(HeapError::Invariant(
            "parse-in-progress module received an executable-state transition"
        ))
    );

    let mut shortened = appended.clone();
    shortened.requested_modules = initial.requested_modules.clone();
    assert_eq!(
        heap.replace_loaded_module(module, shortened),
        Err(HeapError::Invariant(
            "parse-in-progress module replacement changed its request prefix"
        ))
    );

    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let function = heap
        .allocate_function_bytecode(bytecode(&code, realm, Vec::new(), Vec::new()))
        .unwrap();
    let mut completed = appended.clone();
    completed.body = RawModuleRecordBody::SourceText { function };
    let mut changed_resolution = completed.clone();
    changed_resolution.resolution = RawModuleResolutionState::Resolving;
    assert_eq!(
        heap.replace_loaded_module(module, changed_resolution),
        Err(HeapError::Invariant(
            "completed module changed its parse-time resolution state"
        ))
    );
    assert_eq!(
        heap.replace_loaded_module(module, completed.clone()),
        Ok(HeapCleanup::default())
    );

    let mut changed_ready_body = completed;
    changed_ready_body.body = RawModuleRecordBody::Json {
        default_value: RawValue::Undefined,
    };
    assert_eq!(
        heap.replace_loaded_module(module, changed_ready_body),
        Err(HeapError::Invariant(
            "loaded-module replacement changed its body state illegally"
        ))
    );

    assert_eq!(
        heap.transition_loaded_module(module, RawModuleTransition::BeginResolution),
        Ok(())
    );
    assert_eq!(
        heap.transition_loaded_module(module, RawModuleTransition::FailResolution),
        Err(HeapError::Invariant(
            "loaded-module resolution failure did not target Parsing state"
        ))
    );
}

#[test]
fn parsing_request_append_is_amortized_snapshot_safe_and_failure_latched() {
    let mut heap = Heap::new();
    let realm = bytecode_test_realm(&mut heap);
    let module = heap
        .publish_loaded_module(
            realm,
            parsing_module_record(
                realm,
                "many-requests.js",
                &[],
                RawModuleResolutionState::Unresolved,
            ),
        )
        .unwrap();
    let initial_requests = heap.loaded_module(module).unwrap().requested_modules;
    let initial_owner = Rc::as_ptr(&initial_requests);
    drop(initial_requests);

    for index in 0..2_048 {
        heap.append_parsing_module_request(
            module,
            ModuleRequest {
                specifier: JsString::try_from_utf8(&format!("dependency-{index}.js")).unwrap(),
                attributes: crate::engine::code::module::ModuleImportAttributes::Absent,
            },
        )
        .unwrap();
    }
    let shared_snapshot = heap.loaded_module(module).unwrap().requested_modules;
    assert_eq!(shared_snapshot.len(), 2_048);
    assert_eq!(Rc::as_ptr(&shared_snapshot), initial_owner);

    heap.append_parsing_module_request(
        module,
        ModuleRequest {
            specifier: JsString::from_static("last.js"),
            attributes: crate::engine::code::module::ModuleImportAttributes::Absent,
        },
    )
    .unwrap();
    let current = heap.loaded_module(module).unwrap();
    assert_eq!(shared_snapshot.len(), 2_048);
    assert_eq!(current.requested_modules.len(), 2_049);
    assert_ne!(Rc::as_ptr(&current.requested_modules), initial_owner);
    assert_eq!(
        current.requested_modules.last().unwrap().specifier,
        JsString::from_static("last.js")
    );
    drop(current);
    drop(shared_snapshot);

    assert_eq!(
        heap.transition_loaded_module(module, RawModuleTransition::BeginResolution),
        Ok(())
    );
    assert_eq!(
        heap.transition_loaded_module(module, RawModuleTransition::FailResolution),
        Ok(())
    );
    assert!(matches!(
        heap.loaded_module(module).unwrap().resolution,
        RawModuleResolutionState::Failed
    ));
    assert_eq!(
        heap.transition_loaded_module(module, RawModuleTransition::BeginResolution),
        Err(HeapError::Invariant(
            "loaded-module resolution did not begin from Unresolved"
        ))
    );
}

#[test]
fn parsing_abort_tombstones_or_retains_a_hidden_stable_identity() {
    let mut heap = Heap::new();
    let realm = bytecode_test_realm(&mut heap);

    let unreferenced = heap
        .publish_loaded_module(
            realm,
            parsing_module_record(
                realm,
                "unreferenced.js",
                &[],
                RawModuleResolutionState::Unresolved,
            ),
        )
        .unwrap();
    assert_eq!(
        heap.abort_parsing_loaded_module(unreferenced),
        Ok(HeapCleanup::default())
    );
    assert_eq!(heap.loaded_module_is_live(unreferenced), Ok(false));
    assert!(heap.loaded_module(unreferenced).is_err());
    assert_eq!(
        heap.first_loaded_module(realm, &JsString::from_static("unreferenced.js")),
        Ok(None)
    );

    let direct_abort = heap
        .publish_loaded_module(
            realm,
            parsing_module_record(
                realm,
                "direct-abort.js",
                &[],
                RawModuleResolutionState::Unresolved,
            ),
        )
        .unwrap();
    let direct_aborted = aborted_module_record(&heap.loaded_module(direct_abort).unwrap());
    assert_eq!(
        heap.replace_loaded_module(direct_abort, direct_aborted),
        Err(HeapError::Invariant(
            "module construction abort bypassed its ownership primitive"
        ))
    );
    assert_eq!(heap.loaded_module_is_live(direct_abort), Ok(true));
    assert_eq!(
        heap.first_loaded_module(realm, &JsString::from_static("direct-abort.js")),
        Ok(Some(direct_abort))
    );
    assert_eq!(
        heap.abort_parsing_loaded_module(direct_abort),
        Ok(HeapCleanup::default())
    );
    assert_eq!(heap.loaded_module_is_live(direct_abort), Ok(false));
    assert_eq!(
        heap.first_loaded_module(realm, &JsString::from_static("direct-abort.js")),
        Ok(None)
    );

    let first = heap
        .publish_loaded_module(
            realm,
            parsing_module_record(realm, "same.js", &[], RawModuleResolutionState::Unresolved),
        )
        .unwrap();
    let second = heap
        .publish_loaded_module(
            realm,
            parsing_module_record(realm, "same.js", &[], RawModuleResolutionState::Unresolved),
        )
        .unwrap();
    let dependent = heap
        .publish_loaded_module(
            realm,
            parsing_module_record(
                realm,
                "dependent.js",
                &["same.js"],
                RawModuleResolutionState::Resolved(Rc::from([first.module])),
            ),
        )
        .unwrap();

    assert_eq!(
        heap.abort_parsing_loaded_module(first),
        Ok(HeapCleanup::default())
    );
    assert_eq!(heap.loaded_module_is_live(first), Ok(false));
    assert!(matches!(
        heap.loaded_module(first).unwrap().body,
        RawModuleRecordBody::Aborted
    ));
    assert_eq!(
        heap.first_loaded_module(realm, &JsString::from_static("same.js")),
        Ok(Some(second))
    );
    assert_eq!(
        heap.loaded_modules(realm)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        vec![second.module, dependent.module]
    );

    let mut extended = heap.loaded_module(dependent).unwrap();
    extended.requested_modules = Rc::new(vec![
        ModuleRequest {
            specifier: JsString::from_static("same.js"),
            attributes: crate::engine::code::module::ModuleImportAttributes::Absent,
        },
        ModuleRequest {
            specifier: JsString::from_static("later.js"),
            attributes: crate::engine::code::module::ModuleImportAttributes::Absent,
        },
    ]);
    assert_eq!(
        heap.replace_loaded_module(dependent, extended),
        Ok(HeapCleanup::default()),
        "an Aborted dependency remains a valid append-only identity"
    );
    assert_eq!(
        heap.transition_loaded_module(first, RawModuleTransition::BeginResolution),
        Err(HeapError::Invariant(
            "loaded-module transition targeted an aborted identity"
        ))
    );
}
