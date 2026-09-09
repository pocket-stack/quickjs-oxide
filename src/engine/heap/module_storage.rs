use super::*;

impl Heap {
    /// Return the oldest live loaded module with `name` in this Context.
    pub(crate) fn first_loaded_module(
        &self,
        cache: ContextId,
        name: &JsString,
    ) -> Result<Option<RawModuleRef>, HeapError> {
        let context = self.context(cache)?;
        let Some(module) = context.loaded_modules.first_by_name.get(name).copied() else {
            return Ok(None);
        };
        let record = context
            .loaded_modules
            .records
            .get(module.0)
            .and_then(Option::as_ref)
            .ok_or(HeapError::Invariant(
                "loaded-module name index references a tombstone",
            ))?;
        if matches!(&record.body, RawModuleRecordBody::Aborted) {
            return Err(HeapError::Invariant(
                "loaded-module name index references an aborted record",
            ));
        }
        Ok(Some(RawModuleRef { cache, module }))
    }

    /// Clone a borrowed snapshot of one Context-owned module record.
    /// Raw identities in the result are not independently retained.
    pub(crate) fn loaded_module(&self, module: RawModuleRef) -> Result<RawModuleRecord, HeapError> {
        self.context(module.cache)?
            .loaded_modules
            .records
            .get(module.module.0)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or(HeapError::Invariant(
                "loaded-module identity is out of bounds or tombstoned",
            ))
    }

    /// Return whether an append-only module identity still names a live
    /// record. A rollback tombstone or retained `Aborted` identity is a stable
    /// non-live state; out-of-range identities and stale Contexts remain
    /// checked heap errors.
    pub(crate) fn loaded_module_is_live(&self, module: RawModuleRef) -> Result<bool, HeapError> {
        let record = self
            .context(module.cache)?
            .loaded_modules
            .records
            .get(module.module.0)
            .ok_or(HeapError::Invariant(
                "loaded-module identity is out of bounds",
            ))?;
        Ok(record
            .as_ref()
            .is_some_and(|record| !matches!(&record.body, RawModuleRecordBody::Aborted)))
    }

    /// Borrowed snapshots of every live record in construction order.
    /// Stable `Aborted` identities are deliberately omitted.
    pub(crate) fn loaded_modules(
        &self,
        cache: ContextId,
    ) -> Result<Vec<(ModuleId, RawModuleRecord)>, HeapError> {
        Ok(self
            .context(cache)?
            .loaded_modules
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                record.as_ref().and_then(|record| {
                    (!matches!(&record.body, RawModuleRecordBody::Aborted))
                        .then(|| (ModuleId(index), record.clone()))
                })
            })
            .collect())
    }

    #[cfg(test)]
    pub(crate) fn loaded_module_slot_count(&self, cache: ContextId) -> Result<usize, HeapError> {
        Ok(self.context(cache)?.loaded_modules.records.len())
    }

    /// Apply one sealed metadata-only transition without allocating or
    /// perturbing any heap/atom ownership.
    pub(crate) fn transition_loaded_module(
        &mut self,
        module: RawModuleRef,
        transition: RawModuleTransition,
    ) -> Result<(), HeapError> {
        let current = self.loaded_module(module)?;
        match &current.body {
            RawModuleRecordBody::Aborted => {
                return Err(HeapError::Invariant(
                    "loaded-module transition targeted an aborted identity",
                ));
            }
            RawModuleRecordBody::Parsing
                if !matches!(
                    &transition,
                    RawModuleTransition::BeginResolution
                        | RawModuleTransition::FinishResolution(_)
                        | RawModuleTransition::FailResolution
                        | RawModuleTransition::ResetResolution
                ) =>
            {
                return Err(HeapError::Invariant(
                    "parse-in-progress module received an executable-state transition",
                ));
            }
            RawModuleRecordBody::Parsing
            | RawModuleRecordBody::SourceText { .. }
            | RawModuleRecordBody::Json { .. } => {}
        }
        if matches!(&transition, RawModuleTransition::FailResolution)
            && !matches!(&current.body, RawModuleRecordBody::Parsing)
        {
            return Err(HeapError::Invariant(
                "loaded-module resolution failure did not target Parsing state",
            ));
        }
        if let RawModuleTransition::FinishResolution(dependencies) = &transition {
            let context = self.context(module.cache)?;
            for dependency in dependencies.iter().copied() {
                if context
                    .loaded_modules
                    .records
                    .get(dependency.0)
                    .and_then(Option::as_ref)
                    .is_none()
                {
                    return Err(HeapError::Invariant(
                        "resolved loaded-module transition references a missing cache record",
                    ));
                }
            }
            if matches!(&current.body, RawModuleRecordBody::Parsing)
                && dependencies.len() > current.requested_modules.len()
            {
                return Err(HeapError::Invariant(
                    "parse-in-progress module resolved beyond its request prefix",
                ));
            }
        }
        if let RawModuleTransition::FinishNamespace(namespace) = &transition {
            if self.object(*namespace)?.kind != ObjectKind::ModuleNamespace {
                return Err(HeapError::Invariant(
                    "loaded-module namespace transition has the wrong object class",
                ));
            }
        }
        if let RawModuleTransition::FinishEvaluation { cycle_root } = &transition {
            if self
                .context(module.cache)?
                .loaded_modules
                .records
                .get(cycle_root.0)
                .and_then(Option::as_ref)
                .is_none()
            {
                return Err(HeapError::Invariant(
                    "loaded-module evaluation transition has a missing cycle root",
                ));
            }
        }

        if matches!(
            &transition,
            RawModuleTransition::BeginLink | RawModuleTransition::FinishLink
        ) {
            let record = current;
            self.validate_loaded_module_record(module.cache, Some(module.module), &record)?;
            let instance = record.instance.as_ref().ok_or(HeapError::Invariant(
                "loaded-module link transition has no instance",
            ))?;
            if matches!(&transition, RawModuleTransition::FinishLink) {
                if instance.slots.iter().any(Option::is_none) {
                    return Err(HeapError::Invariant(
                        "loaded-module link transition has unresolved closure slots",
                    ));
                }
                if matches!(record.body, RawModuleRecordBody::SourceText { .. })
                    && instance.callable.is_none()
                {
                    return Err(HeapError::Invariant(
                        "loaded-module link transition has no source callable",
                    ));
                }
            }
        }

        let record = self.loaded_module_record_mut(module)?;
        match transition {
            RawModuleTransition::BeginResolution => match record.resolution {
                RawModuleResolutionState::Unresolved => {
                    record.resolution = RawModuleResolutionState::Resolving;
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module resolution did not begin from Unresolved",
                    ));
                }
            },
            RawModuleTransition::FinishResolution(dependencies) => match record.resolution {
                RawModuleResolutionState::Resolving => {
                    record.resolution = RawModuleResolutionState::Resolved(dependencies);
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module resolution did not finish from Resolving",
                    ));
                }
            },
            RawModuleTransition::FailResolution => match record.resolution {
                RawModuleResolutionState::Resolving | RawModuleResolutionState::Resolved(_) => {
                    record.resolution = RawModuleResolutionState::Failed;
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module resolution failure did not target an active latch",
                    ));
                }
            },
            RawModuleTransition::ResetResolution => match record.resolution {
                RawModuleResolutionState::Resolving => {
                    record.resolution = RawModuleResolutionState::Unresolved;
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module resolution reset did not target Resolving",
                    ));
                }
            },
            RawModuleTransition::BeginLink => match record.link_status {
                RawModuleLinkStatus::Unlinked
                    if matches!(record.resolution, RawModuleResolutionState::Resolved(_))
                        && record.instance.is_some()
                        && record.link_realm.is_some() =>
                {
                    record.link_status = RawModuleLinkStatus::Linking;
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module link did not begin from resolved instantiated Unlinked state",
                    ));
                }
            },
            RawModuleTransition::FinishLink => match record.link_status {
                RawModuleLinkStatus::Linking if record.instance.is_some() => {
                    record.link_status = RawModuleLinkStatus::Linked;
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module link did not finish from instantiated Linking state",
                    ));
                }
            },
            RawModuleTransition::ResetLink => match record.link_status {
                RawModuleLinkStatus::Linking => record.link_status = RawModuleLinkStatus::Unlinked,
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module link reset did not target Linking",
                    ));
                }
            },
            RawModuleTransition::PoisonLink => match record.link_status {
                RawModuleLinkStatus::Linking => record.link_status = RawModuleLinkStatus::Poisoned,
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module link poison did not target Linking",
                    ));
                }
            },
            RawModuleTransition::BeginEvaluation => match record.evaluation {
                RawModuleEvaluationState::Unevaluated
                    if matches!(record.link_status, RawModuleLinkStatus::Linked)
                        && record.evaluation_cycle_root.is_none()
                        && record.async_evaluation_order.is_none()
                        && record.pending_async_dependencies == 0
                        && record.async_parent_modules.is_empty() =>
                {
                    record.evaluation = RawModuleEvaluationState::Evaluating;
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module evaluation did not begin from linked Unevaluated state",
                    ));
                }
            },
            RawModuleTransition::FinishEvaluation { cycle_root } => match record.evaluation {
                RawModuleEvaluationState::Evaluating
                    if matches!(record.link_status, RawModuleLinkStatus::Linked) =>
                {
                    if record.async_evaluation_order.is_some() {
                        record.evaluation = RawModuleEvaluationState::EvaluatingAsync;
                    } else {
                        if record.has_top_level_await || record.pending_async_dependencies != 0 {
                            return Err(HeapError::Invariant(
                                "synchronous module SCC finish retained async work",
                            ));
                        }
                        record.evaluation = RawModuleEvaluationState::Evaluated;
                    }
                    record.evaluation_cycle_root = Some(cycle_root);
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module evaluation did not finish from linked Evaluating state",
                    ));
                }
            },
            RawModuleTransition::BeginAsyncEvaluation { order } => match record.evaluation {
                RawModuleEvaluationState::Evaluating
                    if matches!(record.link_status, RawModuleLinkStatus::Linked)
                        && record.evaluation_cycle_root.is_none()
                        && record.async_evaluation_order.is_none()
                        && (record.has_top_level_await
                            || record.pending_async_dependencies != 0) =>
                {
                    record.async_evaluation_order = Some(order);
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module async evaluation did not begin from active async work",
                    ));
                }
            },
            RawModuleTransition::FinishAsyncEvaluation => match record.evaluation {
                RawModuleEvaluationState::EvaluatingAsync
                    if matches!(record.link_status, RawModuleLinkStatus::Linked)
                        && record.evaluation_cycle_root.is_some()
                        && record.async_evaluation_order.is_some()
                        && record.pending_async_dependencies == 0 =>
                {
                    record.evaluation = RawModuleEvaluationState::Evaluated;
                    record.async_evaluation_order = None;
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module async evaluation finished from an invalid state",
                    ));
                }
            },
            RawModuleTransition::PoisonEvaluation => match record.evaluation {
                RawModuleEvaluationState::Evaluating
                | RawModuleEvaluationState::EvaluatingAsync => {
                    record.evaluation = RawModuleEvaluationState::Poisoned;
                    if record.evaluation_promise.is_some() && record.evaluation_cycle_root.is_none()
                    {
                        record.evaluation_cycle_root = Some(module.module);
                    }
                    record.async_evaluation_order = None;
                    record.pending_async_dependencies = 0;
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module evaluation poison did not target Evaluating",
                    ));
                }
            },
            RawModuleTransition::FinishNamespace(namespace) => match record.namespace {
                RawModuleNamespaceState::Building(current) if current == namespace => {
                    record.namespace = RawModuleNamespaceState::Ready(namespace);
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "loaded-module namespace did not finish from matching Building state",
                    ));
                }
            },
        }
        Ok(())
    }

    /// Append one parser-discovered request without cloning the complete
    /// source-order prefix on every publication. Module requests own no arena
    /// edges or atoms, so the only fallible step is reserving vector capacity
    /// before the new request becomes visible.
    pub(crate) fn append_parsing_module_request(
        &mut self,
        module: RawModuleRef,
        request: ModuleRequest,
    ) -> Result<(), HeapError> {
        let record = self.loaded_module_record_mut(module)?;
        if !matches!(&record.body, RawModuleRecordBody::Parsing) {
            return Err(HeapError::Invariant(
                "module request publication did not target Parsing state",
            ));
        }
        if let Some(requests) = Rc::get_mut(&mut record.requested_modules) {
            requests.try_reserve(1).map_err(|_| HeapError::Allocation {
                operation: "growing a parse-in-progress module request prefix",
            })?;
            requests.push(request);
            return Ok(());
        }

        let mut requests = Vec::new();
        requests
            .try_reserve(record.requested_modules.len() + 1)
            .map_err(|_| HeapError::Allocation {
                operation: "copying a shared parse-in-progress module request prefix",
            })?;
        requests.extend(record.requested_modules.iter().cloned());
        requests.push(request);
        record.requested_modules = Rc::new(requests);
        Ok(())
    }

    pub(in crate::engine::heap) fn loaded_module_record_mut(
        &mut self,
        module: RawModuleRef,
    ) -> Result<&mut RawModuleRecord, HeapError> {
        let NodeData::Context(context) =
            &mut self.live_node_mut(RawId::Context(module.cache))?.data
        else {
            return Err(HeapError::Invariant(
                "loaded-module mutation reached another node payload",
            ));
        };
        context
            .loaded_modules
            .records
            .get_mut(module.module.0)
            .and_then(Option::as_mut)
            .ok_or(HeapError::Invariant(
                "loaded-module mutation reached a missing cache record",
            ))
    }

    pub(in crate::engine::heap) fn validate_loaded_module_record(
        &self,
        cache: ContextId,
        module: Option<ModuleId>,
        record: &RawModuleRecord,
    ) -> Result<(), HeapError> {
        self.context(cache)?;
        if record.compile_realm != cache {
            return Err(HeapError::Invariant(
                "loaded-module compilation realm disagrees with its Context cache",
            ));
        }
        match &record.body {
            RawModuleRecordBody::Parsing => {
                if !module_record_has_pristine_construction_metadata(record) {
                    return Err(HeapError::Invariant(
                        "parse-in-progress module retains completed metadata",
                    ));
                }
                if let RawModuleResolutionState::Resolved(dependencies) = &record.resolution {
                    if dependencies.len() > record.requested_modules.len() {
                        return Err(HeapError::Invariant(
                            "parse-in-progress module resolved beyond its request prefix",
                        ));
                    }
                    let context = self.context(cache)?;
                    for dependency in dependencies.iter().copied() {
                        if context
                            .loaded_modules
                            .records
                            .get(dependency.0)
                            .and_then(Option::as_ref)
                            .is_none()
                        {
                            return Err(HeapError::Invariant(
                                "parse-in-progress module dependency references a missing identity",
                            ));
                        }
                    }
                }
                return Ok(());
            }
            RawModuleRecordBody::Aborted => {
                if module.is_none()
                    || !record.requested_modules.is_empty()
                    || !matches!(record.resolution, RawModuleResolutionState::Unresolved)
                    || !module_record_has_pristine_construction_metadata(record)
                {
                    return Err(HeapError::Invariant(
                        "aborted module retained state beyond its name and realm",
                    ));
                }
                return Ok(());
            }
            RawModuleRecordBody::SourceText { .. } | RawModuleRecordBody::Json { .. } => {}
        }
        if record.instance.is_some() != record.link_realm.is_some() {
            return Err(HeapError::Invariant(
                "loaded-module instance and link realm ownership disagree",
            ));
        }
        if let Some(link_realm) = record.link_realm {
            match link_realm {
                RawModuleLinkRealm::Cache => {}
                RawModuleLinkRealm::Other(realm) => {
                    if realm == cache {
                        return Err(HeapError::Invariant(
                            "loaded-module cache realm escaped through an Other link edge",
                        ));
                    }
                    self.context(realm)?;
                }
            }
        }
        let source_function = match &record.body {
            RawModuleRecordBody::SourceText { function } => {
                if self.function_bytecode(*function)?.realm != cache {
                    return Err(HeapError::Invariant(
                        "loaded-module source bytecode belongs to another realm",
                    ));
                }
                Some(*function)
            }
            RawModuleRecordBody::Json { default_value } => {
                if record.has_top_level_await {
                    return Err(HeapError::Invariant(
                        "JSON module claims authored top-level await",
                    ));
                }
                validate_module_storable_value(default_value)?;
                None
            }
            RawModuleRecordBody::Parsing | RawModuleRecordBody::Aborted => {
                return Err(HeapError::Invariant(
                    "module construction state escaped ready-record validation",
                ));
            }
        };
        if let Some(import_meta) = record.import_meta
            && self.object(import_meta)?.kind != ObjectKind::Ordinary
        {
            return Err(HeapError::Invariant(
                "loaded-module import.meta has the wrong object class",
            ));
        }
        if let RawModuleEvaluationState::Errored(exception) = &record.evaluation {
            validate_module_storable_value(exception)?;
        }
        match &record.evaluation {
            RawModuleEvaluationState::Unevaluated => {
                if record.evaluation_cycle_root.is_some()
                    || record.async_evaluation_order.is_some()
                    || record.pending_async_dependencies != 0
                    || !record.async_parent_modules.is_empty()
                {
                    return Err(HeapError::Invariant(
                        "unevaluated loaded module retains async evaluation state",
                    ));
                }
            }
            RawModuleEvaluationState::Evaluating => {
                if record.evaluation_cycle_root.is_some() {
                    return Err(HeapError::Invariant(
                        "active module received a cycle root before SCC publication",
                    ));
                }
            }
            RawModuleEvaluationState::EvaluatingAsync => {
                if record.evaluation_cycle_root.is_none() || record.async_evaluation_order.is_none()
                {
                    return Err(HeapError::Invariant(
                        "async-evaluating module has incomplete SCC metadata",
                    ));
                }
            }
            RawModuleEvaluationState::Evaluated | RawModuleEvaluationState::Errored(_) => {
                if record.evaluation_cycle_root.is_none()
                    || record.async_evaluation_order.is_some()
                    || record.pending_async_dependencies != 0
                {
                    return Err(HeapError::Invariant(
                        "completed loaded-module evaluation retains active async metadata",
                    ));
                }
            }
            RawModuleEvaluationState::Poisoned => {
                if record.async_evaluation_order.is_some() || record.pending_async_dependencies != 0
                {
                    return Err(HeapError::Invariant(
                        "poisoned loaded module retains active async metadata",
                    ));
                }
            }
        }
        if let Some(cycle_root) = record.evaluation_cycle_root {
            if self
                .context(cache)?
                .loaded_modules
                .records
                .get(cycle_root.0)
                .and_then(Option::as_ref)
                .is_none()
            {
                return Err(HeapError::Invariant(
                    "loaded-module evaluation cycle root is missing",
                ));
            }
        }
        match (
            record.evaluation_promise,
            record.evaluation_resolve,
            record.evaluation_reject,
        ) {
            (None, None, None) => {}
            (Some(promise), Some(resolve), Some(reject)) => {
                self.validate_module_evaluation_capability(promise, resolve, reject)?;
                let Some(module) = module else {
                    return Err(HeapError::Invariant(
                        "new loaded module already retains an evaluation capability",
                    ));
                };
                if record
                    .evaluation_cycle_root
                    .is_some_and(|cycle_root| cycle_root != module)
                {
                    return Err(HeapError::Invariant(
                        "non-cycle-root module retains an evaluation capability",
                    ));
                }
            }
            _ => {
                return Err(HeapError::Invariant(
                    "loaded-module evaluation capability is partially published",
                ));
            }
        }
        let context = self.context(cache)?;
        for parent in record.async_parent_modules.iter().copied() {
            if context
                .loaded_modules
                .records
                .get(parent.0)
                .and_then(Option::as_ref)
                .is_none()
            {
                return Err(HeapError::Invariant(
                    "loaded-module async parent references a missing cache record",
                ));
            }
        }
        if let Some(instance) = &record.instance {
            for slot in instance.slots.iter().flatten().copied() {
                self.var_ref(slot)?;
            }
            match source_function {
                Some(function) => {
                    if instance.slots.len()
                        != self.function_bytecode(function)?.closure_variables.len()
                    {
                        return Err(HeapError::Invariant(
                            "loaded-module source instance has the wrong closure slot count",
                        ));
                    }
                    if let Some(callable) = instance.callable {
                        let ObjectPayload::BytecodeFunction {
                            bytecode,
                            closure_slots,
                            ..
                        } = &self.object(callable)?.payload
                        else {
                            return Err(HeapError::Invariant(
                                "loaded-module source callable is not a bytecode function",
                            ));
                        };
                        if *bytecode != function
                            || instance.slots.iter().copied().collect::<Option<Vec<_>>>()
                                != Some(closure_slots.clone())
                        {
                            return Err(HeapError::Invariant(
                                "loaded-module callable does not match its source instance",
                            ));
                        }
                    }
                }
                None => {
                    if instance.slots.len() != 1 || instance.callable.is_some() {
                        return Err(HeapError::Invariant(
                            "loaded-module JSON instance has an invalid environment",
                        ));
                    }
                }
            }
        }
        match record.namespace {
            RawModuleNamespaceState::Empty => {}
            RawModuleNamespaceState::Building(namespace)
            | RawModuleNamespaceState::Ready(namespace) => {
                if self.object(namespace)?.kind != ObjectKind::ModuleNamespace {
                    return Err(HeapError::Invariant(
                        "loaded-module namespace has the wrong object class",
                    ));
                }
                if record.instance.is_none() {
                    return Err(HeapError::Invariant(
                        "loaded-module namespace exists without an instance",
                    ));
                }
            }
        }
        if (record.instance.is_some()
            || !matches!(record.namespace, RawModuleNamespaceState::Empty)
            || !matches!(record.link_status, RawModuleLinkStatus::Unlinked))
            && !matches!(record.resolution, RawModuleResolutionState::Resolved(_))
        {
            return Err(HeapError::Invariant(
                "loaded-module active state is not resolved",
            ));
        }
        if matches!(record.link_status, RawModuleLinkStatus::Linked) {
            let Some(instance) = &record.instance else {
                return Err(HeapError::Invariant(
                    "linked loaded-module has no instantiated environment",
                ));
            };
            if instance.slots.iter().any(Option::is_none)
                || source_function.is_some() && instance.callable.is_none()
            {
                return Err(HeapError::Invariant(
                    "linked loaded-module has an incomplete instance",
                ));
            }
        }
        if !matches!(record.evaluation, RawModuleEvaluationState::Unevaluated)
            && !matches!(record.link_status, RawModuleLinkStatus::Linked)
        {
            return Err(HeapError::Invariant(
                "active loaded-module evaluation is not linked",
            ));
        }
        if record.evaluation_promise.is_some()
            && !matches!(record.link_status, RawModuleLinkStatus::Linked)
        {
            return Err(HeapError::Invariant(
                "loaded-module evaluation Promise exists before linking",
            ));
        }
        if let RawModuleResolutionState::Resolved(dependencies) = &record.resolution {
            let context = self.context(cache)?;
            for dependency in dependencies.iter().copied() {
                if context
                    .loaded_modules
                    .records
                    .get(dependency.0)
                    .and_then(Option::as_ref)
                    .is_none()
                {
                    return Err(HeapError::Invariant(
                        "loaded-module dependency references a missing cache record",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Publish a module at the tail of this Context's construction-ordered
    /// cache. All arena edges are retained before the record becomes visible.
    pub(crate) fn publish_loaded_module(
        &mut self,
        cache: ContextId,
        record: RawModuleRecord,
    ) -> Result<RawModuleRef, HeapError> {
        self.validate_loaded_module_record(cache, None, &record)?;
        let cache_index = self.live_index(RawId::Context(cache))?;
        {
            let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(cache))?.data
            else {
                return Err(HeapError::Invariant(
                    "loaded-module publication reached another node payload",
                ));
            };
            context
                .loaded_modules
                .records
                .try_reserve(1)
                .map_err(|_| HeapError::Allocation {
                    operation: "growing a loaded-module cache",
                })?;
            if !context
                .loaded_modules
                .first_by_name
                .contains_key(&record.name)
            {
                context
                    .loaded_modules
                    .first_by_name
                    .try_reserve(1)
                    .map_err(|_| HeapError::Allocation {
                        operation: "growing a loaded-module name index",
                    })?;
            }
        }
        let edges = raw_module_record_edges(&record);
        let name = record.name.clone();
        self.retain_edges_transactionally(&edges)?;

        let SlotState::Live(node) = &mut self.slots[cache_index].state else {
            unreachable!("authenticated loaded-module cache disappeared before publication")
        };
        let NodeData::Context(context) = &mut node.data else {
            unreachable!("authenticated loaded-module cache changed node kind before publication")
        };
        let module = ModuleId(context.loaded_modules.records.len());
        context.loaded_modules.records.push(Some(record));
        context
            .loaded_modules
            .first_by_name
            .entry(name)
            .or_insert(module);
        Ok(RawModuleRef { cache, module })
    }

    /// Atomically replace one module record. Replacement edges are retained
    /// before publication; detached edges are released only after the swap.
    pub(crate) fn replace_loaded_module(
        &mut self,
        module: RawModuleRef,
        replacement: RawModuleRecord,
    ) -> Result<HeapCleanup, HeapError> {
        let current = self.loaded_module(module)?;
        self.validate_loaded_module_record(module.cache, Some(module.module), &replacement)?;
        if replacement.name != current.name {
            return Err(HeapError::Invariant(
                "loaded-module replacement changed its cache name",
            ));
        }
        if replacement.compile_realm != module.cache
            || replacement.compile_realm != current.compile_realm
        {
            return Err(HeapError::Invariant(
                "loaded-module replacement changed its compilation cache",
            ));
        }
        validate_module_body_replacement(&current, &replacement)?;
        let cache_index = self.live_index(RawId::Context(module.cache))?;

        let old_edges = raw_module_record_edges(&current);
        let new_edges = raw_module_record_edges(&replacement);
        let added_edges = multiset_difference(
            &new_edges,
            &old_edges,
            "computing added loaded-module edges",
        )?;
        let removed_edges = multiset_difference(
            &old_edges,
            &new_edges,
            "computing removed loaded-module edges",
        )?;
        let old_atoms = raw_module_record_atoms(&current).collect::<Vec<_>>();
        let new_atoms = raw_module_record_atoms(&replacement).collect::<Vec<_>>();
        let removed_atoms = multiset_difference(
            &old_atoms,
            &new_atoms,
            "computing removed loaded-module atoms",
        )?;
        self.preflight_module_edge_releases(&removed_edges)?;
        let mut cleanup = HeapCleanup {
            atoms: removed_atoms,
            ..HeapCleanup::default()
        };
        self.retain_edges_transactionally(&added_edges)?;

        let SlotState::Live(node) = &mut self.slots[cache_index].state else {
            unreachable!("authenticated loaded-module cache disappeared before replacement")
        };
        let NodeData::Context(context) = &mut node.data else {
            unreachable!("authenticated loaded-module cache changed node kind before replacement")
        };
        let slot = context
            .loaded_modules
            .records
            .get_mut(module.module.0)
            .and_then(Option::as_mut)
            .expect("authenticated loaded-module record disappeared before replacement");
        let previous = std::mem::replace(slot, replacement);
        drop(previous);
        cleanup.merge(self.release_preflighted_module_edges(&removed_edges));
        Ok(cleanup)
    }

    /// Atomically abort one parse-in-progress module definition.
    ///
    /// An unreferenced identity becomes an ordinary tombstone. If another live
    /// record already refers to it, the slot instead retains an edge-free
    /// `Aborted` sentinel so those append-only references remain structurally
    /// valid while every public module operation observes a non-live handle.
    /// In both cases the record is removed from oldest-name lookup.
    pub(crate) fn abort_parsing_loaded_module(
        &mut self,
        module: RawModuleRef,
    ) -> Result<HeapCleanup, HeapError> {
        let current = self.loaded_module(module)?;
        if !matches!(&current.body, RawModuleRecordBody::Parsing) {
            return Err(HeapError::Invariant(
                "module construction abort did not target Parsing state",
            ));
        }
        let aborted = aborted_module_record(&current);
        self.validate_loaded_module_record(module.cache, Some(module.module), &aborted)?;

        let context = self.context(module.cache)?;
        context.loaded_modules.validate_first_by_name()?;
        let referenced =
            context
                .loaded_modules
                .records
                .iter()
                .enumerate()
                .any(|(index, record)| {
                    index != module.module.0
                        && record.as_ref().is_some_and(|record| {
                            !matches!(&record.body, RawModuleRecordBody::Aborted)
                                && module_record_references_identity(record, module.module)
                        })
                });
        let indexed_first = context
            .loaded_modules
            .first_by_name
            .get(&current.name)
            .copied();
        let fallback = (indexed_first == Some(module.module)).then(|| {
            context
                .loaded_modules
                .records
                .iter()
                .enumerate()
                .skip(module.module.0 + 1)
                .find_map(|(index, record)| {
                    record.as_ref().and_then(|record| {
                        (record.name == current.name
                            && !matches!(&record.body, RawModuleRecordBody::Aborted))
                        .then_some(ModuleId(index))
                    })
                })
        });

        let removed_edges = raw_module_record_edges(&current);
        let removed_atoms = raw_module_record_atoms(&current).collect::<Vec<_>>();
        debug_assert!(removed_atoms.is_empty());
        self.preflight_module_edge_releases(&removed_edges)?;
        let cache_index = self.live_index(RawId::Context(module.cache))?;
        let mut cleanup = HeapCleanup {
            atoms: removed_atoms,
            ..HeapCleanup::default()
        };

        let SlotState::Live(node) = &mut self.slots[cache_index].state else {
            unreachable!("authenticated loaded-module cache disappeared before abort")
        };
        let NodeData::Context(context) = &mut node.data else {
            unreachable!("authenticated loaded-module cache changed kind before abort")
        };
        let slot = context
            .loaded_modules
            .records
            .get_mut(module.module.0)
            .expect("authenticated parse-in-progress module disappeared before abort");
        let previous = if referenced {
            Some(std::mem::replace(
                slot.as_mut()
                    .expect("authenticated parse-in-progress module became a tombstone"),
                aborted,
            ))
        } else {
            slot.take()
        };
        drop(previous);
        if indexed_first == Some(module.module) {
            match fallback.flatten() {
                Some(fallback) => {
                    *context
                        .loaded_modules
                        .first_by_name
                        .get_mut(&current.name)
                        .expect("authenticated module name index disappeared before abort") =
                        fallback;
                }
                None => {
                    let removed = context.loaded_modules.first_by_name.remove(&current.name);
                    debug_assert_eq!(removed, Some(module.module));
                }
            }
        }

        cleanup.merge(self.release_preflighted_module_edges(&removed_edges));
        Ok(cleanup)
    }

    /// Atomically tombstone a validated set of records in one Context cache.
    /// The caller supplies the already-computed rollback set; its membership
    /// and ordering policy remain outside this ownership primitive.
    pub(crate) fn unpublish_loaded_modules(
        &mut self,
        cache: ContextId,
        modules: &[ModuleId],
    ) -> Result<HeapCleanup, HeapError> {
        let mut unique = HashSet::new();
        unique
            .try_reserve(modules.len())
            .map_err(|_| HeapError::Allocation {
                operation: "validating loaded-module removal batch",
            })?;
        let mut records = Vec::new();
        records
            .try_reserve(modules.len())
            .map_err(|_| HeapError::Allocation {
                operation: "preparing loaded-module removal batch",
            })?;
        for &module in modules {
            if !unique.insert(module) {
                return Err(HeapError::Invariant(
                    "loaded-module removal batch contains a duplicate identity",
                ));
            }
            let record = self.loaded_module(RawModuleRef { cache, module })?;
            if matches!(&record.body, RawModuleRecordBody::Aborted) {
                return Err(HeapError::Invariant(
                    "loaded-module removal batch contains an aborted identity",
                ));
            }
            records.push((module, record));
        }
        let context = self.context(cache)?;
        context.loaded_modules.validate_first_by_name()?;

        for (index, record) in context.loaded_modules.records.iter().enumerate() {
            if unique.contains(&ModuleId(index)) {
                continue;
            }
            if let Some(record) = record
                && !matches!(&record.body, RawModuleRecordBody::Aborted)
                && let RawModuleResolutionState::Resolved(dependencies) = &record.resolution
                && dependencies
                    .iter()
                    .any(|dependency| unique.contains(dependency))
            {
                return Err(HeapError::Invariant(
                    "loaded-module removal leaves a live dependency on a tombstone",
                ));
            }
            if let Some(record) = record
                && !matches!(&record.body, RawModuleRecordBody::Aborted)
                && record
                    .async_parent_modules
                    .iter()
                    .any(|parent| unique.contains(parent))
            {
                return Err(HeapError::Invariant(
                    "loaded-module removal leaves a live async-parent edge on a tombstone",
                ));
            }
            if let Some(record) = record
                && !matches!(&record.body, RawModuleRecordBody::Aborted)
                && record
                    .evaluation_cycle_root
                    .is_some_and(|cycle_root| unique.contains(&cycle_root))
            {
                return Err(HeapError::Invariant(
                    "loaded-module removal leaves a live evaluation-root edge on a tombstone",
                ));
            }
        }

        #[allow(clippy::mutable_key_type)]
        let rebuilt_first_by_name = context
            .loaded_modules
            .rebuild_first_by_name_excluding(|candidate| unique.contains(&candidate))?;

        let mut removed_edges = Vec::new();
        let mut removed_atoms = Vec::new();
        for (_, record) in &records {
            removed_edges.extend(raw_module_record_edges(record));
            removed_atoms.extend(raw_module_record_atoms(record));
        }
        self.preflight_module_edge_releases(&removed_edges)?;
        let cache_index = self.live_index(RawId::Context(cache))?;
        let mut cleanup = HeapCleanup {
            atoms: removed_atoms,
            ..HeapCleanup::default()
        };

        let SlotState::Live(node) = &mut self.slots[cache_index].state else {
            unreachable!("authenticated loaded-module cache disappeared before batch removal")
        };
        let NodeData::Context(context) = &mut node.data else {
            unreachable!("authenticated loaded-module cache changed kind before batch removal")
        };
        for &(module, _) in &records {
            context.loaded_modules.records[module.0]
                .take()
                .expect("authenticated loaded-module disappeared before batch removal");
        }
        context.loaded_modules.first_by_name = rebuilt_first_by_name;

        cleanup.merge(self.release_preflighted_module_edges(&removed_edges));
        Ok(cleanup)
    }

    /// Atomically clear namespace objects created by one namespace-building
    /// transaction. Both Building and Ready are accepted because a recursive
    /// member may finish before a later member makes the transaction fail.
    pub(crate) fn rollback_loaded_module_namespaces(
        &mut self,
        modules: &[RawModuleRef],
    ) -> Result<HeapCleanup, HeapError> {
        if modules.is_empty() {
            return Ok(HeapCleanup::default());
        }
        let cache = modules[0].cache;
        let mut unique = HashSet::new();
        unique
            .try_reserve(modules.len())
            .map_err(|_| HeapError::Allocation {
                operation: "validating module namespace rollback batch",
            })?;
        let mut namespaces = Vec::new();
        namespaces
            .try_reserve(modules.len())
            .map_err(|_| HeapError::Allocation {
                operation: "preparing module namespace rollback batch",
            })?;
        for &module in modules {
            if module.cache != cache || !unique.insert(module.module) {
                return Err(HeapError::Invariant(
                    "module namespace rollback batch has mixed or duplicate identities",
                ));
            }
            let namespace = match self.loaded_module(module)?.namespace {
                RawModuleNamespaceState::Building(namespace)
                | RawModuleNamespaceState::Ready(namespace) => namespace,
                RawModuleNamespaceState::Empty => {
                    return Err(HeapError::Invariant(
                        "module namespace rollback reached an empty record",
                    ));
                }
            };
            namespaces.push((module, namespace));
        }
        let removed_edges = namespaces
            .iter()
            .map(|(_, namespace)| RawId::Object(*namespace))
            .collect::<Vec<_>>();
        self.preflight_module_edge_releases(&removed_edges)?;
        for &(module, namespace) in &namespaces {
            let record = self
                .loaded_module_record_mut(module)
                .expect("authenticated namespace rollback record disappeared before commit");
            debug_assert!(matches!(
                record.namespace,
                RawModuleNamespaceState::Building(current)
                    | RawModuleNamespaceState::Ready(current) if current == namespace
            ));
            record.namespace = RawModuleNamespaceState::Empty;
        }
        Ok(self.release_preflighted_module_edges(&removed_edges))
    }

    /// Atomically install the cached evaluation Promise and its intrinsic
    /// resolving pair on a cycle root. All three object edges become owned by
    /// the Context record together, so an evaluator never needs to publish a
    /// partially rooted capability through a general record mutation.
    pub(crate) fn publish_loaded_module_evaluation_capability(
        &mut self,
        module: RawModuleRef,
        promise: ObjectId,
        resolve: ObjectId,
        reject: ObjectId,
    ) -> Result<(), HeapError> {
        self.validate_module_evaluation_capability(promise, resolve, reject)?;
        let record = self.loaded_module(module)?;
        if record.evaluation_promise.is_some()
            || record.evaluation_resolve.is_some()
            || record.evaluation_reject.is_some()
            || record.link_status != RawModuleLinkStatus::Linked
            || matches!(
                record.evaluation,
                RawModuleEvaluationState::Evaluating | RawModuleEvaluationState::Poisoned
            )
            || record
                .evaluation_cycle_root
                .is_some_and(|cycle_root| cycle_root != module.module)
        {
            return Err(HeapError::Invariant(
                "loaded-module evaluation capability has an invalid publication target",
            ));
        }
        let edges = [
            RawId::Object(promise),
            RawId::Object(resolve),
            RawId::Object(reject),
        ];
        self.retain_edges_transactionally(&edges)?;
        let record = self
            .loaded_module_record_mut(module)
            .expect("authenticated evaluation capability target disappeared before commit");
        record.evaluation_promise = Some(promise);
        record.evaluation_resolve = Some(resolve);
        record.evaluation_reject = Some(reject);
        Ok(())
    }

    /// Append one reverse async-dependency edge and increment its parent's
    /// pending count as one allocation-safe metadata transaction. Duplicate
    /// module identities are deliberately retained and counted separately.
    pub(crate) fn add_loaded_module_async_dependency(
        &mut self,
        dependency: RawModuleRef,
        parent: RawModuleRef,
    ) -> Result<u32, HeapError> {
        if dependency.cache != parent.cache || dependency.module == parent.module {
            return Err(HeapError::Invariant(
                "async module dependency has mixed caches or a self parent",
            ));
        }
        let dependency_record = self.loaded_module(dependency)?;
        let parent_record = self.loaded_module(parent)?;
        if dependency_record.async_evaluation_order.is_none()
            || !matches!(
                dependency_record.evaluation,
                RawModuleEvaluationState::Evaluating | RawModuleEvaluationState::EvaluatingAsync
            )
            || !matches!(
                parent_record.evaluation,
                RawModuleEvaluationState::Evaluating
            )
            || parent_record.evaluation_cycle_root.is_some()
        {
            return Err(HeapError::Invariant(
                "async module dependency edge has invalid evaluation states",
            ));
        }
        let pending = parent_record
            .pending_async_dependencies
            .checked_add(1)
            .ok_or(HeapError::Overflow {
                operation: "counting pending async module dependencies",
            })?;
        self.loaded_module_record_mut(dependency)?
            .async_parent_modules
            .try_reserve(1)
            .map_err(|_| HeapError::Allocation {
                operation: "growing async module parent edges",
            })?;
        self.loaded_module_record_mut(parent)?
            .pending_async_dependencies = pending;
        self.loaded_module_record_mut(dependency)?
            .async_parent_modules
            .push(parent.module);
        Ok(pending)
    }

    /// Consume one reverse dependency edge's pending-count contribution.
    /// The caller uses the returned zero to decide when the parent is ready.
    pub(crate) fn complete_loaded_module_async_dependency(
        &mut self,
        parent: RawModuleRef,
    ) -> Result<u32, HeapError> {
        let record = self.loaded_module_record_mut(parent)?;
        if !matches!(record.evaluation, RawModuleEvaluationState::EvaluatingAsync)
            || record.evaluation_cycle_root.is_none()
            || record.async_evaluation_order.is_none()
        {
            return Err(HeapError::Invariant(
                "async module dependency completed for an inactive parent",
            ));
        }
        record.pending_async_dependencies = record
            .pending_async_dependencies
            .checked_sub(1)
            .ok_or(HeapError::Invariant(
                "async module dependency count underflow",
            ))?;
        Ok(record.pending_async_dependencies)
    }

    pub(in crate::engine::heap) fn validate_module_evaluation_capability(
        &self,
        promise: ObjectId,
        resolve: ObjectId,
        reject: ObjectId,
    ) -> Result<(), HeapError> {
        if !matches!(self.object(promise)?.payload, ObjectPayload::Promise(_)) {
            return Err(HeapError::Invariant(
                "module evaluation capability has a non-Promise target",
            ));
        }
        let ObjectPayload::NativeFunction {
            data: resolve_data,
            internal:
                Some(InternalCallableData::PromiseResolving {
                    promise: resolve_promise,
                    already_resolved: resolve_cell,
                    kind: PromiseResolvingKind::Resolve,
                }),
        } = &self.object(resolve)?.payload
        else {
            return Err(HeapError::Invariant(
                "module evaluation capability has an invalid resolve function",
            ));
        };
        let ObjectPayload::NativeFunction {
            data: reject_data,
            internal:
                Some(InternalCallableData::PromiseResolving {
                    promise: reject_promise,
                    already_resolved: reject_cell,
                    kind: PromiseResolvingKind::Reject,
                }),
        } = &self.object(reject)?.payload
        else {
            return Err(HeapError::Invariant(
                "module evaluation capability has an invalid reject function",
            ));
        };
        if resolve_data.target != NativeFunctionId::PromiseResolving(PromiseResolvingKind::Resolve)
            || reject_data.target
                != NativeFunctionId::PromiseResolving(PromiseResolvingKind::Reject)
            || *resolve_promise != promise
            || *reject_promise != promise
            || !Rc::ptr_eq(resolve_cell, reject_cell)
        {
            return Err(HeapError::Invariant(
                "module evaluation capability resolving pair targets another Promise",
            ));
        }
        Ok(())
    }

    /// Atomically publish the same cached abrupt completion across one active
    /// evaluation SCC. Object edges are retained transactionally before any
    /// record changes; Symbol atom ownership is prepared by Runtime because it
    /// belongs to the separate atom table.
    pub(crate) fn publish_loaded_module_errors(
        &mut self,
        cache: ContextId,
        modules: &[ModuleId],
        cycle_root: ModuleId,
        exception: RawValue,
    ) -> Result<(), HeapError> {
        validate_module_storable_value(&exception)?;
        let mut unique = HashSet::new();
        unique
            .try_reserve(modules.len())
            .map_err(|_| HeapError::Allocation {
                operation: "validating module evaluation error batch",
            })?;
        for &module in modules {
            if !unique.insert(module) {
                return Err(HeapError::Invariant(
                    "module evaluation error batch contains a duplicate identity",
                ));
            }
            let record = self.loaded_module(RawModuleRef { cache, module })?;
            if !matches!(record.evaluation, RawModuleEvaluationState::Evaluating)
                || !matches!(record.link_status, RawModuleLinkStatus::Linked)
            {
                return Err(HeapError::Invariant(
                    "module evaluation error batch reached a non-evaluating linked record",
                ));
            }
        }
        if !unique.contains(&cycle_root) {
            return Err(HeapError::Invariant(
                "module evaluation error cycle root was not active",
            ));
        }
        let edge = raw_value_edges(&exception);
        let mut added_edges = Vec::new();
        added_edges
            .try_reserve(edge.len().saturating_mul(modules.len()))
            .map_err(|_| HeapError::Allocation {
                operation: "preparing module evaluation error edges",
            })?;
        for _ in modules {
            added_edges.extend(edge.iter().copied());
        }
        self.retain_edges_transactionally(&added_edges)?;
        for &module in modules {
            let record = self
                .loaded_module_record_mut(RawModuleRef { cache, module })
                .expect("authenticated evaluation error record disappeared before commit");
            record.evaluation = RawModuleEvaluationState::Errored(exception.clone());
            record.evaluation_cycle_root = Some(cycle_root);
            record.async_evaluation_order = None;
            record.pending_async_dependencies = 0;
        }
        Ok(())
    }

    /// Cache one rejection reason on an already-published async module. The
    /// record preserves its SCC cycle root and reverse parent list so Runtime
    /// can reproduce QuickJS's observable per-node reject-then-recurse order.
    ///
    /// As with `publish_loaded_module_errors`, Symbol atom ownership is
    /// prepared by Runtime; this operation retains all arena edges.
    pub(crate) fn publish_loaded_module_async_error(
        &mut self,
        module: RawModuleRef,
        exception: RawValue,
    ) -> Result<(), HeapError> {
        validate_module_storable_value(&exception)?;
        let record = self.loaded_module(module)?;
        if !matches!(record.evaluation, RawModuleEvaluationState::EvaluatingAsync)
            || record.evaluation_cycle_root.is_none()
            || record.async_evaluation_order.is_none()
            || record.link_status != RawModuleLinkStatus::Linked
        {
            return Err(HeapError::Invariant(
                "async module rejection reached a non-evaluating record",
            ));
        }
        let value_edges = raw_value_edges(&exception);
        self.retain_edges_transactionally(&value_edges)?;
        let record = self
            .loaded_module_record_mut(module)
            .expect("authenticated async module rejection target disappeared before commit");
        record.evaluation = RawModuleEvaluationState::Errored(exception);
        record.async_evaluation_order = None;
        record.pending_async_dependencies = 0;
        Ok(())
    }

    pub(in crate::engine::heap) fn preflight_module_edge_releases(
        &mut self,
        edges: &[RawId],
    ) -> Result<(), HeapError> {
        let mut counts = HashMap::<RawId, u32>::new();
        counts
            .try_reserve(edges.len())
            .map_err(|_| HeapError::Allocation {
                operation: "preflighting loaded-module edge releases",
            })?;
        for &edge in edges {
            let count = counts.entry(edge).or_default();
            *count = count.checked_add(1).ok_or(HeapError::Overflow {
                operation: "counting removed loaded-module edges",
            })?;
        }
        let mut newly_zero = 0usize;
        for (&edge, &removed) in &counts {
            let strong = self.live_node(edge)?.strong;
            let remaining = strong.checked_sub(removed).ok_or(HeapError::Underflow {
                kind: edge.kind(),
                index: edge.index(),
                generation: edge.generation(),
            })?;
            newly_zero = newly_zero.saturating_add(usize::from(remaining == 0));
        }
        self.zero_queue
            .try_reserve(newly_zero)
            .map_err(|_| HeapError::Allocation {
                operation: "reserving loaded-module release queue space",
            })?;
        Ok(())
    }

    pub(in crate::engine::heap) fn release_preflighted_module_edges(
        &mut self,
        edges: &[RawId],
    ) -> HeapCleanup {
        for &edge in edges {
            self.release_raw_no_drain(edge)
                .expect("preflighted loaded-module edge release failed after publication");
        }
        self.drain_zero_queue()
            .expect("preflighted loaded-module edge drain failed after publication")
    }
}
