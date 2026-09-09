use super::*;

/// Runtime-internal module identity. `cache` is the defining Context whose
/// loaded-module cache owns `module`; all dependency indices in that record
/// refer to the same cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RawModuleRef {
    pub(crate) cache: ContextId,
    pub(crate) module: ModuleId,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawPublishedModuleExport {
    pub(crate) export_name: JsString,
    pub(crate) target: RawPublishedModuleExportTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RawPublishedModuleExportTarget {
    SourceTextLocal {
        closure_index: u16,
    },
    SyntheticLocal {
        cell_index: u16,
    },
    Indirect {
        request: ModuleRequestIndex,
        import_name: ModuleImportName,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RawModuleRecordBody {
    /// Source-text module definition published before parsing begins.
    ///
    /// The record may expose only its source-order requested-module prefix;
    /// every heap-owning and executable field remains pristine until the
    /// compiler atomically replaces this body with `SourceText`.
    Parsing,
    SourceText {
        function: FunctionBytecodeId,
    },
    Json {
        default_value: RawValue,
    },
    /// Stable append-only identity retained after construction rollback.
    ///
    /// This state is used only when another live module already names the
    /// identity. It owns no heap edges and is excluded from name lookup.
    Aborted,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RawModuleResolutionState {
    Unresolved,
    Resolving,
    /// QuickJS latches `resolved` before invoking host callbacks and does not
    /// retry after a callback failure which the host subsequently clears.
    /// Rust keeps that state explicit instead of retaining unsafe partial raw
    /// dependency pointers.
    Failed,
    Resolved(Rc<[ModuleId]>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawModuleInstance {
    pub(crate) slots: Vec<Option<VarRefId>>,
    pub(crate) callable: Option<ObjectId>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RawModuleNamespaceState {
    Empty,
    Building(ObjectId),
    Ready(ObjectId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RawModuleLinkStatus {
    Unlinked,
    Linking,
    Linked,
    Poisoned,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RawModuleEvaluationState {
    Unevaluated,
    Evaluating,
    EvaluatingAsync,
    Evaluated,
    Errored(RawValue),
    Poisoned,
}

/// First-execution realm retained by a linked module record. The defining
/// cache realm is represented without a heap edge because the Context already
/// owns the loaded-module record; only a distinct realm is an outgoing edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RawModuleLinkRealm {
    Cache,
    Other(ContextId),
}

/// Sealed, allocation-free metadata transitions for one loaded module. Every
/// variant changes exactly one non-owning field after authenticating its
/// precise predecessor state; ownership-bearing record changes must use the
/// transactional publication APIs instead.
pub(crate) enum RawModuleTransition {
    BeginResolution,
    FinishResolution(Rc<[ModuleId]>),
    FailResolution,
    ResetResolution,
    BeginLink,
    FinishLink,
    ResetLink,
    PoisonLink,
    BeginEvaluation,
    /// Publish one completed evaluation SCC. An already-assigned async order
    /// selects `EvaluatingAsync`; its absence selects `Evaluated`.
    FinishEvaluation {
        cycle_root: ModuleId,
    },
    /// Mark an active module as transitively async before its SCC is
    /// published. The order is the runtime-global QuickJS evaluation stamp.
    BeginAsyncEvaluation {
        order: u64,
    },
    /// Complete a successfully executed async module after every dependency
    /// has become available.
    FinishAsyncEvaluation,
    PoisonEvaluation,
    FinishNamespace(ObjectId),
}

/// Raw Context-owned counterpart of QuickJS's `JSModuleDef`.
///
/// Cloning this structure creates only a borrowed snapshot: arena identities
/// and Symbol atoms are retained exclusively by the containing ContextData.
/// A snapshot must therefore not outlive the cache root or be used after a
/// mutation releases one of its raw fields unless that field was promoted to
/// an owning runtime root first.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RawModuleRecord {
    pub(crate) name: JsString,
    pub(crate) body: RawModuleRecordBody,
    /// Lazily allocated canonical `import.meta` object.
    ///
    /// This is the direct counterpart of QuickJS `JSModuleDef::meta_obj`:
    /// the module record owns the object independently from the hidden
    /// closure cell used by source text which actually reads `import.meta`.
    pub(crate) import_meta: Option<ObjectId>,
    pub(crate) declaration_order: Rc<[u16]>,
    pub(crate) link_initializers: Rc<[ModuleLinkInitializer]>,
    pub(crate) import_collisions: Rc<[ModuleImportCollision]>,
    pub(crate) requested_modules: Rc<Vec<ModuleRequest>>,
    pub(crate) imports: Rc<[ModuleImport]>,
    pub(crate) exports: Rc<[RawPublishedModuleExport]>,
    pub(crate) star_exports: Rc<[ModuleStarExport]>,
    pub(crate) resolution: RawModuleResolutionState,
    pub(crate) instance: Option<RawModuleInstance>,
    pub(crate) namespace: RawModuleNamespaceState,
    pub(crate) link_status: RawModuleLinkStatus,
    pub(crate) evaluation: RawModuleEvaluationState,
    /// Exact counterpart of QuickJS `JSModuleDef::has_tla`. Every source-text
    /// module executes through async bytecode, so function kind alone cannot
    /// distinguish authored top-level await.
    pub(crate) has_top_level_await: bool,
    /// Synchronous evaluation SCC root, assigned when the SCC completes (or
    /// to the requested root for active records on abrupt completion).
    pub(crate) evaluation_cycle_root: Option<ModuleId>,
    /// Cached Promise created for the first module evaluation attempt.
    ///
    /// This mirrors QuickJS's `JSModuleDef::promise`: the Context-owned
    /// record retains it independently of callers, and every later dynamic
    /// import observes the same evaluation identity and settlement history.
    pub(crate) evaluation_promise: Option<ObjectId>,
    /// The resolving pair belonging to `evaluation_promise`. The three fields
    /// are published atomically and retained for the lifetime of the cycle
    /// root, matching QuickJS `JSModuleDef::resolving_funcs`.
    pub(crate) evaluation_resolve: Option<ObjectId>,
    pub(crate) evaluation_reject: Option<ObjectId>,
    /// Number of async dependency roots which have not yet become available.
    pub(crate) pending_async_dependencies: u32,
    /// Reverse dependency edges. Duplicates are semantically significant:
    /// each occurrence balances one pending dependency count on its parent.
    pub(crate) async_parent_modules: Vec<ModuleId>,
    /// Runtime-global ordering stamp while `async_evaluation` is true in
    /// QuickJS. Successful or abrupt completion clears the stamp.
    pub(crate) async_evaluation_order: Option<u64>,
    pub(crate) link_realm: Option<RawModuleLinkRealm>,
    pub(crate) compile_realm: ContextId,
}

pub(in crate::engine::heap) fn module_record_has_pristine_construction_metadata(
    record: &RawModuleRecord,
) -> bool {
    record.import_meta.is_none()
        && record.declaration_order.is_empty()
        && record.link_initializers.is_empty()
        && record.import_collisions.is_empty()
        && record.imports.is_empty()
        && record.exports.is_empty()
        && record.star_exports.is_empty()
        && record.instance.is_none()
        && matches!(record.namespace, RawModuleNamespaceState::Empty)
        && matches!(record.link_status, RawModuleLinkStatus::Unlinked)
        && matches!(record.evaluation, RawModuleEvaluationState::Unevaluated)
        && !record.has_top_level_await
        && record.evaluation_cycle_root.is_none()
        && record.evaluation_promise.is_none()
        && record.evaluation_resolve.is_none()
        && record.evaluation_reject.is_none()
        && record.pending_async_dependencies == 0
        && record.async_parent_modules.is_empty()
        && record.async_evaluation_order.is_none()
        && record.link_realm.is_none()
}

pub(in crate::engine::heap) fn module_record_references_identity(
    record: &RawModuleRecord,
    module: ModuleId,
) -> bool {
    matches!(
        &record.resolution,
        RawModuleResolutionState::Resolved(dependencies)
            if dependencies.contains(&module)
    ) || record.async_parent_modules.contains(&module)
        || record.evaluation_cycle_root == Some(module)
}

pub(in crate::engine::heap) fn validate_module_body_replacement(
    current: &RawModuleRecord,
    replacement: &RawModuleRecord,
) -> Result<(), HeapError> {
    match (&current.body, &replacement.body) {
        (RawModuleRecordBody::Parsing, RawModuleRecordBody::Parsing) => {
            if replacement.requested_modules.len() < current.requested_modules.len()
                || !replacement
                    .requested_modules
                    .starts_with(current.requested_modules.as_slice())
            {
                return Err(HeapError::Invariant(
                    "parse-in-progress module replacement changed its request prefix",
                ));
            }
            if replacement.resolution != current.resolution {
                return Err(HeapError::Invariant(
                    "parse-in-progress module replacement changed its resolution state",
                ));
            }
        }
        (RawModuleRecordBody::Parsing, RawModuleRecordBody::SourceText { .. }) => {
            if replacement.requested_modules != current.requested_modules {
                return Err(HeapError::Invariant(
                    "completed module changed its published request prefix",
                ));
            }
            if replacement.resolution != current.resolution {
                return Err(HeapError::Invariant(
                    "completed module changed its parse-time resolution state",
                ));
            }
        }
        (RawModuleRecordBody::Parsing, RawModuleRecordBody::Aborted) => {
            return Err(HeapError::Invariant(
                "module construction abort bypassed its ownership primitive",
            ));
        }
        (RawModuleRecordBody::SourceText { .. }, RawModuleRecordBody::SourceText { .. })
        | (RawModuleRecordBody::Json { .. }, RawModuleRecordBody::Json { .. }) => {}
        (RawModuleRecordBody::Aborted, _) => {
            return Err(HeapError::Invariant(
                "aborted module identity cannot be replaced",
            ));
        }
        _ => {
            return Err(HeapError::Invariant(
                "loaded-module replacement changed its body state illegally",
            ));
        }
    }
    Ok(())
}

pub(in crate::engine::heap) fn aborted_module_record(record: &RawModuleRecord) -> RawModuleRecord {
    RawModuleRecord {
        name: record.name.clone(),
        body: RawModuleRecordBody::Aborted,
        import_meta: None,
        declaration_order: Rc::from([]),
        link_initializers: Rc::from([]),
        import_collisions: Rc::from([]),
        requested_modules: Rc::new(Vec::new()),
        imports: Rc::from([]),
        exports: Rc::from([]),
        star_exports: Rc::from([]),
        resolution: RawModuleResolutionState::Unresolved,
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
        compile_realm: record.compile_realm,
    }
}

/// Construction-ordered loaded modules for one Context. Slots are never
/// compacted or reused: rollback changes an unreferenced slot to `None` and a
/// referenced slot to `Aborted`, while the name map continues to point at the
/// oldest remaining live record.
#[derive(Debug, PartialEq)]
pub(crate) struct LoadedModuleCache {
    pub(in crate::engine::heap) records: Vec<Option<RawModuleRecord>>,
    pub(in crate::engine::heap) first_by_name: HashMap<JsString, ModuleId>,
}

impl LoadedModuleCache {
    pub(in crate::engine::heap) fn new() -> Self {
        Self {
            records: Vec::new(),
            first_by_name: HashMap::new(),
        }
    }

    pub(in crate::engine::heap) fn validate_first_by_name(&self) -> Result<(), HeapError> {
        for record in self.records.iter().flatten() {
            if matches!(&record.body, RawModuleRecordBody::Aborted) {
                continue;
            }
            if !self.first_by_name.contains_key(&record.name) {
                return Err(HeapError::Invariant(
                    "loaded-module oldest-name index is incomplete",
                ));
            }
        }
        for (name, first) in &self.first_by_name {
            let record =
                self.records
                    .get(first.0)
                    .and_then(Option::as_ref)
                    .ok_or(HeapError::Invariant(
                        "loaded-module oldest-name index references a tombstone",
                    ))?;
            if matches!(&record.body, RawModuleRecordBody::Aborted) {
                return Err(HeapError::Invariant(
                    "loaded-module oldest-name index references an aborted record",
                ));
            }
            if &record.name != name {
                return Err(HeapError::Invariant(
                    "loaded-module oldest-name index references another name",
                ));
            }
            if self.records[..first.0].iter().flatten().any(|earlier| {
                !matches!(&earlier.body, RawModuleRecordBody::Aborted) && &earlier.name == name
            }) {
                return Err(HeapError::Invariant(
                    "loaded-module name index does not reference the oldest live record",
                ));
            }
        }
        Ok(())
    }

    pub(in crate::engine::heap) fn rebuild_first_by_name_excluding(
        &self,
        mut excluded: impl FnMut(ModuleId) -> bool,
    ) -> Result<HashMap<JsString, ModuleId>, HeapError> {
        // `JsString` mutates only transparent rope caches; its UTF-16
        // equality/hash identity is stable while used as a module-name key.
        #[allow(clippy::mutable_key_type)]
        let mut rebuilt = HashMap::new();
        rebuilt
            .try_reserve(self.first_by_name.len())
            .map_err(|_| HeapError::Allocation {
                operation: "rebuilding loaded-module oldest-name index",
            })?;
        for (index, record) in self.records.iter().enumerate() {
            let id = ModuleId(index);
            if excluded(id) {
                continue;
            }
            if let Some(record) = record
                && !matches!(&record.body, RawModuleRecordBody::Aborted)
            {
                rebuilt.entry(record.name.clone()).or_insert(id);
            }
        }
        Ok(rebuilt)
    }
}

pub(in crate::engine::heap) fn validate_module_storable_value(
    value: &RawValue,
) -> Result<(), HeapError> {
    if matches!(
        value,
        RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception
    ) {
        return Err(HeapError::Invariant(
            "loaded-module record contains an internal value sentinel",
        ));
    }
    Ok(())
}
