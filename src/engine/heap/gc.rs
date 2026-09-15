//! Heap reference ownership, ordered weak-reference processing, and cycle collection.
//!
//! Publication and mutation share these edge and atom traversals with finalization.
//! Cleanup returns detached atom ownership to the runtime; it never mutates the
//! runtime atom table or invokes JavaScript callbacks while borrowing the arena.

use super::{
    AsyncGeneratorRequestData, Atom, AutoInitProperty, BytecodeConstant, ContextData, ContextId,
    FinalizationRegistryEntry, FunctionBytecodeData, FunctionBytecodeId, GeneratorActivationData,
    GeneratorFrameBinding, Hash, HashMap, Heap, HeapError, InternalCallableData, NativeErrorKind,
    Node, NodeData, ObjectData, ObjectId, ObjectPayload, PrimitiveKind, PrimitiveObjectData,
    PromiseCapabilityData, PromiseReaction, PropertySlot, RawId, RawModuleEvaluationState,
    RawModuleLinkRealm, RawModuleNamespaceState, RawModuleRecord, RawModuleRecordBody, RawValue,
    Shape, ShapeId, SlotState, TypedArrayElementKind, VarRefData, VarRefId, VecDeque,
    WeakCollectionKey, is_map_storable_value,
};

/// Resources finalized by a release, mutation, or collection operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeapCleanup {
    pub finalized_objects: usize,
    pub finalized_shapes: usize,
    pub finalized_var_refs: usize,
    pub finalized_contexts: usize,
    pub finalized_function_bytecodes: usize,
    /// Finalized shape identities for O(1) weak-cache unlinking.
    pub finalized_shape_ids: Vec<ShapeId>,
    /// Owned non-GC atom edges detached from shapes and symbol values.
    pub atoms: Vec<Atom>,
}

impl HeapCleanup {
    pub(super) fn merge(&mut self, mut other: Self) {
        self.finalized_objects = self
            .finalized_objects
            .saturating_add(other.finalized_objects);
        self.finalized_shapes = self.finalized_shapes.saturating_add(other.finalized_shapes);
        self.finalized_var_refs = self
            .finalized_var_refs
            .saturating_add(other.finalized_var_refs);
        self.finalized_contexts = self
            .finalized_contexts
            .saturating_add(other.finalized_contexts);
        self.finalized_function_bytecodes = self
            .finalized_function_bytecodes
            .saturating_add(other.finalized_function_bytecodes);
        self.finalized_shape_ids
            .append(&mut other.finalized_shape_ids);
        self.atoms.append(&mut other.atoms);
    }
}

/// Statistics and caller-owned cleanup produced by one cycle collection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GcStats {
    pub examined_nodes: usize,
    pub external_root_nodes: usize,
    pub candidate_nodes: usize,
    pub cleanup: HeapCleanup,
}

/// AtomTable interaction requested during the ordered weak-ref pass.
///
/// [`Heap::run_gc_with_finalization_sink`] uses one callback for both events
/// so a runtime can mutably borrow its AtomTable once. For `IsLive`, the hook
/// result reports current liveness. For `Release`, `true` means the hook
/// consumed the detached atom ownership immediately; `false` defers it into
/// [`HeapCleanup::atoms`] for the caller to release after collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeakSymbolGcEvent {
    IsLive(Atom),
    Release(Atom),
}

/// Whether an internal collection performs QuickJS's ordered weak-object
/// removal pass before trial deletion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeakObjectGcMode {
    Remove,
    PreserveForRuntimeTeardown,
}

/// One FinalizationRegistry callback job whose roots have already transferred
/// out of the registry entry and into the runtime boundary.
///
/// Publication through [`FinalizationJobSink`] transfers exactly one owned
/// callback, realm, and held-value root per record. A Runtime adapter must not
/// retain them again when it adopts the record.
#[derive(Debug, PartialEq)]
pub(crate) struct PreparedFinalizationJob {
    pub(crate) realm: ContextId,
    pub(crate) callback: ObjectId,
    pub(crate) held_value: RawValue,
}

/// Allocation boundary used by the weak-object pass to hand finalization work
/// directly to the Runtime. `try_reserve_one` runs before callback/realm roots
/// are retained or the held value leaves its registration. Once it succeeds,
/// `publish_preowned` must be infallible and must not retain the roots again.
pub(crate) trait FinalizationJobSink {
    fn try_reserve_one(&mut self) -> bool;
    fn publish_preowned(&mut self, job: PreparedFinalizationJob);
}

/// Used by heap-only GC callers which have no Runtime job queue. Returning
/// false deliberately takes QuickJS's silent job-drop path.
pub(super) struct DiscardFinalizationJobSink;

impl FinalizationJobSink for DiscardFinalizationJobSink {
    fn try_reserve_one(&mut self) -> bool {
        false
    }

    fn publish_preowned(&mut self, _job: PreparedFinalizationJob) {
        unreachable!("discarding finalization sink never reserves a slot")
    }
}

impl Heap {
    /// Duplicate one externally owned object reference.
    pub fn retain_object(&mut self, id: ObjectId) -> Result<(), HeapError> {
        self.retain_raw(RawId::Object(id), 1)
    }

    /// Duplicate one externally owned shape reference.
    pub fn retain_shape(&mut self, id: ShapeId) -> Result<(), HeapError> {
        self.retain_raw(RawId::Shape(id), 1)
    }

    /// Duplicate one frame or closure ownership of a captured-variable cell.
    pub fn retain_var_ref(&mut self, id: VarRefId) -> Result<(), HeapError> {
        self.retain_raw(RawId::VarRef(id), 1)
    }

    /// Duplicate one externally owned context reference.
    pub fn retain_context(&mut self, id: ContextId) -> Result<(), HeapError> {
        self.retain_raw(RawId::Context(id), 1)
    }

    /// Duplicate one externally owned function-bytecode reference.
    pub fn retain_function_bytecode(&mut self, id: FunctionBytecodeId) -> Result<(), HeapError> {
        self.retain_raw(RawId::FunctionBytecode(id), 1)
    }

    /// Release one object reference and iteratively drain zero-reference nodes.
    pub fn release_object(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::Object(id))
    }

    /// Release one shape reference and iteratively drain zero-reference nodes.
    pub fn release_shape(&mut self, id: ShapeId) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::Shape(id))
    }

    /// Release one frame or closure ownership of a captured-variable cell.
    // Keep the typed cleanup-returning entry point; Runtime uses optional cleanup.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn release_var_ref(&mut self, id: VarRefId) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::VarRef(id))
    }

    /// Release one context reference and iteratively drain cascading nodes.
    pub fn release_context(&mut self, id: ContextId) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::Context(id))
    }

    /// Release one function-bytecode reference and iteratively drain nodes.
    // Keep the typed cleanup-returning entry point; Runtime uses optional cleanup.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn release_function_bytecode(
        &mut self,
        id: FunctionBytecodeId,
    ) -> Result<HeapCleanup, HeapError> {
        self.release_and_drain(RawId::FunctionBytecode(id))
    }

    /// Snapshot the non-owning target currently stored by a genuine WeakRef.
    /// A stale identity remains visible here until the next weak-object pass;
    /// the runtime must still perform AtomTable liveness for Symbol targets.
    pub(crate) fn weak_ref_target(
        &self,
        id: ObjectId,
    ) -> Result<Option<WeakCollectionKey>, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::WeakRef { target } => Ok(*target),
            _ => Err(HeapError::Invariant(
                "WeakRef target lookup reached an object with the wrong class",
            )),
        }
    }

    /// Append one FinalizationRegistry registration in construction order.
    /// Weak target/token identities retain nothing; the held value transfers
    /// one already-owned Symbol atom and transactionally retained arena edges.
    pub(crate) fn finalization_registry_register(
        &mut self,
        id: ObjectId,
        target: WeakCollectionKey,
        held_value: RawValue,
        unregister_token: Option<WeakCollectionKey>,
    ) -> Result<(), HeapError> {
        if !is_map_storable_value(&held_value) {
            return Err(HeapError::Invariant(
                "FinalizationRegistry held value contains an internal sentinel",
            ));
        }
        if raw_value_matches_weak_key(&held_value, target) {
            return Err(HeapError::Invariant(
                "FinalizationRegistry held value aliases its weak target",
            ));
        }
        self.validate_live_weak_target(target)?;
        if let Some(token) = unregister_token {
            self.validate_live_weak_target(token)?;
        }
        let ObjectPayload::FinalizationRegistry(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "FinalizationRegistry registration reached an object with the wrong class",
            ));
        };
        data.entries
            .try_reserve(1)
            .map_err(|_| HeapError::Allocation {
                operation: "growing FinalizationRegistry entries",
            })?;

        self.retain_edges_transactionally(&raw_value_edges(&held_value))?;
        let ObjectPayload::FinalizationRegistry(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("FinalizationRegistry disappeared after retaining its held value")
        };
        data.entries.push(FinalizationRegistryEntry {
            target,
            held_value,
            unregister_token,
        });
        Ok(())
    }

    /// Remove every live registration carrying `unregister_token`, preserving
    /// the relative order of all remaining entries.
    pub(crate) fn finalization_registry_unregister(
        &mut self,
        id: ObjectId,
        unregister_token: WeakCollectionKey,
    ) -> Result<(bool, HeapCleanup), HeapError> {
        self.validate_live_weak_target(unregister_token)?;
        if !matches!(
            self.object(id)?.payload,
            ObjectPayload::FinalizationRegistry(_)
        ) {
            return Err(HeapError::Invariant(
                "FinalizationRegistry unregister reached an object with the wrong class",
            ));
        }

        // QuickJS deletes matching records in place. Avoid a temporary Vec so
        // unregister introduces no extra allocation/OOM boundary.
        let mut matched = false;
        let mut index = 0usize;
        let mut cleanup = HeapCleanup::default();
        loop {
            let held_value = {
                let ObjectPayload::FinalizationRegistry(data) = &mut self.object_mut(id)?.payload
                else {
                    unreachable!("FinalizationRegistry changed during unregister")
                };
                if index >= data.entries.len() {
                    break;
                }
                if data.entries[index].unregister_token == Some(unregister_token) {
                    Some(data.entries.remove(index).held_value)
                } else {
                    index = index.saturating_add(1);
                    None
                }
            };
            let Some(held_value) = held_value else {
                continue;
            };
            matched = true;
            cleanup.atoms.extend(raw_value_atom(&held_value));
            for edge in raw_value_edges(&held_value) {
                self.release_raw_no_drain(edge)?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok((matched, cleanup))
    }

    /// Number of registrations not yet unregistered or prepared as jobs.
    #[cfg(test)]
    pub(crate) fn finalization_registry_len(&self, id: ObjectId) -> Result<usize, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::FinalizationRegistry(data) => Ok(data.entries.len()),
            _ => Err(HeapError::Invariant(
                "FinalizationRegistry length reached an object with the wrong class",
            )),
        }
    }

    /// Release a batch previously published to a FinalizationJobSink. Each
    /// record already owns callback, realm, and held-value roots, so callers
    /// must either adopt them without retaining or return them here.
    #[cfg(test)]
    pub(crate) fn discard_finalization_jobs(
        &mut self,
        jobs: VecDeque<PreparedFinalizationJob>,
    ) -> Result<HeapCleanup, HeapError> {
        let mut cleanup = HeapCleanup::default();
        for job in jobs {
            cleanup.atoms.extend(raw_value_atom(&job.held_value));
            for edge in raw_value_edges(&job.held_value)
                .into_iter()
                .chain([RawId::Object(job.callback), RawId::Context(job.realm)])
            {
                self.release_raw_no_drain(edge)?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    pub(super) fn release_replaced_raw_value(
        &mut self,
        previous: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&previous));
        for edge in raw_value_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    pub(super) fn release_raw_values_into(
        &mut self,
        values: Vec<RawValue>,
        mut cleanup: HeapCleanup,
    ) -> Result<HeapCleanup, HeapError> {
        for value in values {
            cleanup.atoms.extend(raw_value_atom(&value));
            if let RawValue::Object(object) = value {
                self.release_raw_no_drain(RawId::Object(object))?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Runtime-facing collection entry point. The sink reserves its concrete
    /// job queue before the heap transfers any ownership, eliminating a
    /// second allocation boundary between weak removal and Runtime adoption.
    pub(crate) fn run_gc_with_finalization_sink<H, S>(
        &mut self,
        mut hook: H,
        sink: &mut S,
    ) -> Result<GcStats, HeapError>
    where
        H: FnMut(WeakSymbolGcEvent) -> Result<bool, HeapError>,
        S: FinalizationJobSink,
    {
        self.run_gc_with_weak_symbol_hooks_in_mode(WeakObjectGcMode::Remove, &mut hook, sink)
    }

    /// Runtime teardown counterpart of QuickJS
    /// `JS_RunGCInternal(rt, FALSE)`. The caller discards its existing Runtime
    /// jobs before entering here; this mode clears no weak targets, converts no
    /// registrations to jobs, and introduces no new user-code work.
    pub(crate) fn run_gc_for_runtime_teardown(&mut self) -> Result<GcStats, HeapError> {
        let mut hook = |event| {
            Ok(match event {
                WeakSymbolGcEvent::IsLive(_) => true,
                WeakSymbolGcEvent::Release(_) => false,
            })
        };
        let mut sink = DiscardFinalizationJobSink;
        self.run_gc_with_weak_symbol_hooks_in_mode(
            WeakObjectGcMode::PreserveForRuntimeTeardown,
            &mut hook,
            &mut sink,
        )
    }

    fn run_gc_with_weak_symbol_hooks_in_mode<H, S>(
        &mut self,
        mode: WeakObjectGcMode,
        hook: &mut H,
        sink: &mut S,
    ) -> Result<GcStats, HeapError>
    where
        H: FnMut(WeakSymbolGcEvent) -> Result<bool, HeapError>,
        S: FinalizationJobSink,
    {
        let mut cleanup = self.drain_zero_queue()?;
        if mode == WeakObjectGcMode::Remove {
            cleanup.merge(self.remove_dead_weak_objects(hook, sink)?);
        }
        if self
            .slots
            .iter()
            .any(|slot| matches!(slot.state, SlotState::Zombie { .. }))
        {
            return Err(HeapError::Invariant(
                "zombie survived past the preceding collection boundary",
            ));
        }

        let mut trial = vec![None; self.slots.len()];
        let mut examined_nodes = 0usize;
        for (index, slot) in self.slots.iter().enumerate() {
            if let SlotState::Live(node) = &slot.state {
                trial[index] = Some(node.strong);
                examined_nodes = examined_nodes.saturating_add(1);
            }
        }

        // Subtract every internal incoming edge.  The remainder is precisely
        // the external root count, matching QuickJS's gc_decref phase.
        for slot in &self.slots {
            let SlotState::Live(node) = &slot.state else {
                continue;
            };
            for edge in node.data.edges() {
                let target = self.live_index(edge)?;
                let count = trial[target].ok_or(HeapError::Invariant(
                    "live edge targeted a node outside the trial set",
                ))?;
                trial[target] = Some(count.checked_sub(1).ok_or(HeapError::Invariant(
                    "internal incoming references exceeded strong count",
                ))?);
            }
        }

        // Restore the closure reachable from nodes with external references,
        // equivalent to QuickJS's gc_scan phase.
        let mut reachable = vec![false; self.slots.len()];
        let mut work = VecDeque::new();
        let mut external_root_nodes = 0usize;
        for (index, count) in trial.iter().copied().enumerate() {
            if count.is_some_and(|count| count != 0) {
                reachable[index] = true;
                work.push_back(index);
                external_root_nodes = external_root_nodes.saturating_add(1);
            }
        }
        while let Some(index) = work.pop_front() {
            let SlotState::Live(node) = &self.slots[index].state else {
                return Err(HeapError::Invariant(
                    "mark worklist contained a non-live node",
                ));
            };
            for edge in node.data.edges() {
                let target = self.live_index(edge)?;
                if !reachable[target] {
                    reachable[target] = true;
                    work.push_back(target);
                }
            }
        }

        let mut candidate_nodes = 0usize;
        let mut anchors = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let SlotState::Live(node) = &slot.state else {
                continue;
            };
            if reachable[index] {
                continue;
            }
            candidate_nodes = candidate_nodes.saturating_add(1);
            let index = u32::try_from(index).map_err(|_| HeapError::Overflow {
                operation: "constructing a collection worklist",
            })?;
            match &node.data {
                NodeData::Object(_) => anchors.push(RawId::Object(ObjectId {
                    index,
                    generation: slot.generation,
                })),
                NodeData::FunctionBytecode(_) => {
                    anchors.push(RawId::FunctionBytecode(FunctionBytecodeId {
                        index,
                        generation: slot.generation,
                    }));
                }
                NodeData::Context(_) => anchors.push(RawId::Context(ContextId {
                    index,
                    generation: slot.generation,
                })),
                NodeData::Shape(_) | NodeData::VarRef(_) => {}
            }
        }

        // Objects, function bytecodes, and Context-owned loaded-module caches
        // are active anchors. Mark each as a zombie before dropping its
        // outgoing edges so other candidate nodes can still release the old
        // generation.
        for id in anchors {
            if self.is_live(id) {
                self.finalize_cycle_anchor(id, &mut cleanup)?;
                cleanup.merge(self.drain_zero_queue()?);
            }
        }
        cleanup.merge(self.drain_zero_queue()?);

        if self
            .slots
            .iter()
            .any(|slot| matches!(slot.state, SlotState::Zombie { .. }))
        {
            return Err(HeapError::Invariant(
                "cycle collection left an anchor zombie with incoming references",
            ));
        }

        Ok(GcStats {
            examined_nodes,
            external_root_nodes,
            candidate_nodes,
            cleanup,
        })
    }

    /// Remove stale weak targets in the same single construction-order pass
    /// as QuickJS's `gc_remove_weak_objects`. Releasing a WeakMap value can
    /// make a target in a *later* weak object stale during this traversal, but
    /// an earlier weak object is deliberately not revisited until the next GC.
    fn remove_dead_weak_objects<H, S>(
        &mut self,
        hook: &mut H,
        sink: &mut S,
    ) -> Result<HeapCleanup, HeapError>
    where
        H: FnMut(WeakSymbolGcEvent) -> Result<bool, HeapError>,
        S: FinalizationJobSink,
    {
        #[derive(Clone, Copy)]
        enum WeakObjectKind {
            Map,
            Set,
            Ref,
            Registry,
        }

        let mut cleanup = HeapCleanup::default();
        let mut current = self.weak_head;
        while let Some(id) = current {
            // Cache the intrusive successor before releasing any values. A
            // release may zero-queue either endpoint, but finalization stays
            // deferred until the complete weak-ref traversal has finished.
            let next = self.weak_registry_next(id)?;
            let kind = {
                let object = self.weak_object_for_prune(id)?;
                match &object.payload {
                    ObjectPayload::WeakMap { .. } => WeakObjectKind::Map,
                    ObjectPayload::WeakSet { .. } => WeakObjectKind::Set,
                    ObjectPayload::WeakRef { .. } => WeakObjectKind::Ref,
                    ObjectPayload::FinalizationRegistry(_) => WeakObjectKind::Registry,
                    _ => {
                        return Err(HeapError::Invariant(
                            "weak registry changed object class during traversal",
                        ));
                    }
                }
            };

            match kind {
                WeakObjectKind::Map | WeakObjectKind::Set => {
                    let mut key = match &self.weak_object_for_prune(id)?.payload {
                        ObjectPayload::WeakMap { records } => records.first_key(),
                        ObjectPayload::WeakSet { records } => records.first_key(),
                        _ => unreachable!("weak collection kind was authenticated above"),
                    };
                    while let Some(record_key) = key {
                        // Cache the record successor before detaching its
                        // value. A release can kill a later key in this pass.
                        let record_next = match (&self.weak_object_for_prune(id)?.payload, kind) {
                            (ObjectPayload::WeakMap { records }, WeakObjectKind::Map) => {
                                records.next_key(record_key)?
                            }
                            (ObjectPayload::WeakSet { records }, WeakObjectKind::Set) => {
                                records.next_key(record_key)?
                            }
                            _ => {
                                return Err(HeapError::Invariant(
                                    "weak registry changed class during record traversal",
                                ));
                            }
                        };
                        if !self.weak_target_is_live(record_key, hook)? {
                            match kind {
                                WeakObjectKind::Map => {
                                    let value = {
                                        let ObjectPayload::WeakMap { records } =
                                            &mut self.weak_object_for_prune_mut(id)?.payload
                                        else {
                                            return Err(HeapError::Invariant(
                                                "weak registry changed map class during pruning",
                                            ));
                                        };
                                        records.remove(&record_key)?.ok_or(HeapError::Invariant(
                                            "ordered weak-map record disappeared during pruning",
                                        ))?
                                    };
                                    if let Some(atom) = raw_value_atom(&value) {
                                        if !hook(WeakSymbolGcEvent::Release(atom))? {
                                            cleanup.atoms.push(atom);
                                        }
                                    }
                                    for edge in raw_value_edges(&value) {
                                        self.release_raw_no_drain(edge)?;
                                    }
                                }
                                WeakObjectKind::Set => {
                                    let ObjectPayload::WeakSet { records } =
                                        &mut self.weak_object_for_prune_mut(id)?.payload
                                    else {
                                        return Err(HeapError::Invariant(
                                            "weak registry changed set class during pruning",
                                        ));
                                    };
                                    if records.remove(&record_key)?.is_none() {
                                        return Err(HeapError::Invariant(
                                            "ordered weak-set record disappeared during pruning",
                                        ));
                                    }
                                }
                                WeakObjectKind::Ref | WeakObjectKind::Registry => unreachable!(),
                            }
                        }
                        key = record_next;
                    }
                }
                WeakObjectKind::Ref => {
                    let target = match &self.weak_object_for_prune(id)?.payload {
                        ObjectPayload::WeakRef { target } => *target,
                        _ => unreachable!("WeakRef kind was authenticated above"),
                    };
                    if let Some(target) = target
                        && !self.weak_target_is_live(target, hook)?
                    {
                        let ObjectPayload::WeakRef { target } =
                            &mut self.weak_object_for_prune_mut(id)?.payload
                        else {
                            return Err(HeapError::Invariant(
                                "weak registry changed WeakRef class during pruning",
                            ));
                        };
                        *target = None;
                    }
                }
                WeakObjectKind::Registry => {
                    let mut entry_index = 0usize;
                    while let Some((token, target)) = match &self.weak_object_for_prune(id)?.payload
                    {
                        ObjectPayload::FinalizationRegistry(data) => data
                            .entries
                            .get(entry_index)
                            .map(|entry| (entry.unregister_token, entry.target)),
                        _ => {
                            return Err(HeapError::Invariant(
                                "weak registry changed FinalizationRegistry class",
                            ));
                        }
                    } {
                        if let Some(token) = token
                            && !self.weak_target_is_live(token, hook)?
                        {
                            let ObjectPayload::FinalizationRegistry(data) =
                                &mut self.weak_object_for_prune_mut(id)?.payload
                            else {
                                unreachable!("FinalizationRegistry changed after validation")
                            };
                            data.entries[entry_index].unregister_token = None;
                        }

                        if self.weak_target_is_live(target, hook)? {
                            entry_index = entry_index.saturating_add(1);
                            continue;
                        }

                        // QuickJS's no-exception enqueue silently drops the
                        // callback if queue preparation fails. Decide this
                        // before retaining callback/realm or moving held.
                        if !sink.try_reserve_one() {
                            let entry = {
                                let ObjectPayload::FinalizationRegistry(data) =
                                    &mut self.weak_object_for_prune_mut(id)?.payload
                                else {
                                    unreachable!("FinalizationRegistry changed after validation")
                                };
                                data.entries.remove(entry_index)
                            };
                            if let Some(atom) = raw_value_atom(&entry.held_value)
                                && !hook(WeakSymbolGcEvent::Release(atom))?
                            {
                                cleanup.atoms.push(atom);
                            }
                            for edge in raw_value_edges(&entry.held_value) {
                                self.release_raw_no_drain(edge)?;
                            }
                            continue;
                        }

                        let (callback, realm) = match &self.weak_object_for_prune(id)?.payload {
                            ObjectPayload::FinalizationRegistry(data) => {
                                (data.callback, data.realm)
                            }
                            _ => unreachable!("FinalizationRegistry changed after validation"),
                        };
                        self.retain_edges_transactionally(&[
                            RawId::Object(callback),
                            RawId::Context(realm),
                        ])?;
                        let entry = {
                            let ObjectPayload::FinalizationRegistry(data) =
                                &mut self.weak_object_for_prune_mut(id)?.payload
                            else {
                                unreachable!("FinalizationRegistry changed after retaining roots")
                            };
                            data.entries.remove(entry_index)
                        };
                        // Moving held_value preserves its existing object and
                        // Atom ownership. callback/realm are the two newly
                        // retained roots which make the prepared job external
                        // to trial-deletion tracing.
                        sink.publish_preowned(PreparedFinalizationJob {
                            realm,
                            callback,
                            held_value: entry.held_value,
                        });
                    }
                }
            }
            current = next;
        }

        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Append a newly published WeakMap, WeakSet, WeakRef, or
    /// FinalizationRegistry to the runtime-local mixed weak-object list.
    /// QuickJS traverses this list once in construction order before trial
    /// deletion, so a recycled low arena slot still belongs at the tail.
    pub(super) fn link_weak_object(&mut self, id: ObjectId) -> Result<(), HeapError> {
        if self.weak_head.is_none() != self.weak_tail.is_none() {
            return Err(HeapError::Invariant(
                "weak-collection registry endpoints disagreed",
            ));
        }
        let index = self.weak_registry_slot_index(id)?;
        let slot = &self.slots[index];
        if slot.weak_prev.is_some() || slot.weak_next.is_some() {
            return Err(HeapError::Invariant(
                "weak collection was linked more than once",
            ));
        }
        if !matches!(
            &slot.state,
            SlotState::Live(Node {
                data: NodeData::Object(ObjectData {
                    payload: ObjectPayload::WeakMap { .. }
                        | ObjectPayload::WeakSet { .. }
                        | ObjectPayload::WeakRef { .. }
                        | ObjectPayload::FinalizationRegistry(_),
                    ..
                }),
                ..
            })
        ) {
            return Err(HeapError::Invariant(
                "weak registry received a non-weak object",
            ));
        }

        let previous = self.weak_tail;
        if let Some(previous) = previous {
            if previous == id {
                return Err(HeapError::Invariant(
                    "weak-collection registry linked a node to itself",
                ));
            }
            let previous_index = self.weak_registry_slot_index(previous)?;
            if self.slots[previous_index].weak_next.is_some() {
                return Err(HeapError::Invariant(
                    "weak-collection registry tail had a successor",
                ));
            }
            self.slots[previous_index].weak_next = Some(id);
        } else {
            self.weak_head = Some(id);
        }
        self.slots[index].weak_prev = previous;
        self.weak_tail = Some(id);
        Ok(())
    }

    /// Detach a weak object immediately before its payload releases its
    /// outgoing edges. The current slot may already be Vacant (zero-count
    /// finalization) or Zombie (cycle finalization), so registry identity is
    /// checked by generation rather than by the slot's transient state.
    fn unlink_weak_object(&mut self, id: ObjectId) -> Result<(), HeapError> {
        let index = self.weak_registry_slot_index(id)?;
        let (previous, next) = {
            let slot = &self.slots[index];
            (slot.weak_prev, slot.weak_next)
        };

        match previous {
            Some(previous) => {
                if self.weak_head == Some(id) {
                    return Err(HeapError::Invariant(
                        "weak-collection registry head had a predecessor",
                    ));
                }
                let previous_index = self.weak_registry_slot_index(previous)?;
                if self.slots[previous_index].weak_next != Some(id) {
                    return Err(HeapError::Invariant(
                        "weak-collection registry predecessor was inconsistent",
                    ));
                }
            }
            None if self.weak_head != Some(id) => {
                return Err(HeapError::Invariant(
                    "unlinked weak collection was not the registry head",
                ));
            }
            None => {}
        }
        match next {
            Some(next) => {
                if self.weak_tail == Some(id) {
                    return Err(HeapError::Invariant(
                        "weak-collection registry tail had a successor",
                    ));
                }
                let next_index = self.weak_registry_slot_index(next)?;
                if self.slots[next_index].weak_prev != Some(id) {
                    return Err(HeapError::Invariant(
                        "weak-collection registry successor was inconsistent",
                    ));
                }
            }
            None if self.weak_tail != Some(id) => {
                return Err(HeapError::Invariant(
                    "unlinked weak collection was not the registry tail",
                ));
            }
            None => {}
        }

        if let Some(previous) = previous {
            let previous_index = self.weak_registry_slot_index(previous)?;
            self.slots[previous_index].weak_next = next;
        } else {
            self.weak_head = next;
        }
        if let Some(next) = next {
            let next_index = self.weak_registry_slot_index(next)?;
            self.slots[next_index].weak_prev = previous;
        } else {
            self.weak_tail = previous;
        }
        self.slots[index].weak_prev = None;
        self.slots[index].weak_next = None;
        Ok(())
    }

    fn weak_registry_slot_index(&self, id: ObjectId) -> Result<usize, HeapError> {
        let index = id.index as usize;
        let slot = self.slots.get(index).ok_or(HeapError::Invariant(
            "weak-collection registry referenced a missing slot",
        ))?;
        if slot.generation != id.generation {
            return Err(HeapError::Invariant(
                "weak-collection registry referenced a stale generation",
            ));
        }
        Ok(index)
    }

    fn weak_registry_next(&self, id: ObjectId) -> Result<Option<ObjectId>, HeapError> {
        let index = self.weak_registry_slot_index(id)?;
        let slot = &self.slots[index];
        if slot.weak_prev.is_none() && self.weak_head != Some(id) {
            return Err(HeapError::Invariant(
                "weak-collection registry traversal left the linked list",
            ));
        }
        if slot.weak_next.is_none() && self.weak_tail != Some(id) {
            return Err(HeapError::Invariant(
                "weak-collection registry ended before its tail",
            ));
        }
        Ok(slot.weak_next)
    }

    fn weak_object_for_prune(&self, id: ObjectId) -> Result<&ObjectData, HeapError> {
        let index = self.weak_registry_slot_index(id)?;
        let node = match &self.slots[index].state {
            SlotState::Live(node) | SlotState::ZeroQueued(node) => node,
            _ => {
                return Err(HeapError::Invariant(
                    "weak-ref pass reached an object outside its deferred lifetime",
                ));
            }
        };
        match &node.data {
            NodeData::Object(object)
                if matches!(
                    &object.payload,
                    ObjectPayload::WeakMap { .. }
                        | ObjectPayload::WeakSet { .. }
                        | ObjectPayload::WeakRef { .. }
                        | ObjectPayload::FinalizationRegistry(_)
                ) =>
            {
                Ok(object)
            }
            _ => Err(HeapError::Invariant(
                "weak-ref pass reached a non-weak object",
            )),
        }
    }

    fn weak_object_for_prune_mut(&mut self, id: ObjectId) -> Result<&mut ObjectData, HeapError> {
        let index = self.weak_registry_slot_index(id)?;
        let node = match &mut self.slots[index].state {
            SlotState::Live(node) | SlotState::ZeroQueued(node) => node,
            _ => {
                return Err(HeapError::Invariant(
                    "weak-ref pass mutated an object outside its deferred lifetime",
                ));
            }
        };
        match &mut node.data {
            NodeData::Object(object)
                if matches!(
                    &object.payload,
                    ObjectPayload::WeakMap { .. }
                        | ObjectPayload::WeakSet { .. }
                        | ObjectPayload::WeakRef { .. }
                        | ObjectPayload::FinalizationRegistry(_)
                ) =>
            {
                Ok(object)
            }
            _ => Err(HeapError::Invariant(
                "weak-ref pass mutated a non-weak object",
            )),
        }
    }

    pub(super) fn validate_live_weak_target(
        &self,
        target: WeakCollectionKey,
    ) -> Result<(), HeapError> {
        if let WeakCollectionKey::Object(object) = target {
            self.object(object)?;
        }
        Ok(())
    }

    fn weak_target_is_live<H>(
        &self,
        target: WeakCollectionKey,
        hook: &mut H,
    ) -> Result<bool, HeapError>
    where
        H: FnMut(WeakSymbolGcEvent) -> Result<bool, HeapError>,
    {
        match target {
            WeakCollectionKey::Object(object) => Ok(self.is_live(RawId::Object(object))),
            WeakCollectionKey::Symbol(atom) => hook(WeakSymbolGcEvent::IsLive(atom)),
        }
    }

    pub(super) fn retain_edges_transactionally(
        &mut self,
        edges: &[RawId],
    ) -> Result<(), HeapError> {
        let mut counts = HashMap::<RawId, u32>::new();
        for &edge in edges {
            let count = counts.entry(edge).or_default();
            *count = count.checked_add(1).ok_or(HeapError::Overflow {
                operation: "counting outgoing heap edges",
            })?;
        }

        // A complete preflight makes the following increments infallible and
        // avoids a rollback path that could itself need to report atom cleanup.
        for (&edge, &additional) in &counts {
            let strong = self.live_node(edge)?.strong;
            strong.checked_add(additional).ok_or(HeapError::Overflow {
                operation: "retaining outgoing heap edges",
            })?;
        }
        for (edge, additional) in counts {
            self.retain_raw(edge, additional)
                .expect("preflighted heap edge retain failed before publication");
        }
        Ok(())
    }

    pub(super) fn retain_raw(&mut self, id: RawId, additional: u32) -> Result<(), HeapError> {
        let node = self.live_node_mut(id)?;
        node.strong = node
            .strong
            .checked_add(additional)
            .ok_or(HeapError::Overflow {
                operation: "retaining a heap reference",
            })?;
        Ok(())
    }

    pub(super) fn release_and_drain(&mut self, id: RawId) -> Result<HeapCleanup, HeapError> {
        Ok(self.release_reference(id)?.unwrap_or_default())
    }

    /// Release one reference, returning runtime cleanup only when the zero
    /// queue has work. Inspect the whole queue: an earlier no-drain release may
    /// have queued a different node even when this reference remains nonzero.
    #[inline]
    pub(super) fn release_reference(
        &mut self,
        id: RawId,
    ) -> Result<Option<HeapCleanup>, HeapError> {
        self.release_raw_no_drain(id)?;
        if self.zero_queue.is_empty() {
            return Ok(None);
        }
        self.drain_zero_queue().map(Some)
    }

    pub(super) fn release_raw_no_drain(&mut self, id: RawId) -> Result<(), HeapError> {
        let index = self.validate_slot_identity(id)?;
        let mut queue = false;
        let mut vacate_zombie = false;
        {
            let slot = &mut self.slots[index];
            match &mut slot.state {
                SlotState::Live(node) => {
                    node.strong = node.strong.checked_sub(1).ok_or(HeapError::Underflow {
                        kind: id.kind(),
                        index: id.index(),
                        generation: id.generation(),
                    })?;
                    if node.strong == 0 {
                        let state = std::mem::replace(&mut slot.state, SlotState::Vacant);
                        let SlotState::Live(node) = state else {
                            return Err(HeapError::Invariant(
                                "live node changed while entering the zero queue",
                            ));
                        };
                        slot.state = SlotState::ZeroQueued(node);
                        queue = true;
                    }
                }
                SlotState::Zombie { strong, .. } => {
                    *strong = strong.checked_sub(1).ok_or(HeapError::Underflow {
                        kind: id.kind(),
                        index: id.index(),
                        generation: id.generation(),
                    })?;
                    vacate_zombie = *strong == 0;
                }
                SlotState::Initializing { .. }
                | SlotState::ZeroQueued(_)
                | SlotState::Finalizing(_) => {
                    return Err(HeapError::Underflow {
                        kind: id.kind(),
                        index: id.index(),
                        generation: id.generation(),
                    });
                }
                SlotState::Vacant | SlotState::Retired => {
                    return Err(HeapError::Stale {
                        index: id.index(),
                        generation: id.generation(),
                    });
                }
            }
        }
        if queue {
            self.zero_queue.push_back(id);
        }
        if vacate_zombie {
            self.reclaim_slot(id.index())?;
        }
        Ok(())
    }

    pub(super) fn drain_zero_queue(&mut self) -> Result<HeapCleanup, HeapError> {
        let mut cleanup = HeapCleanup::default();
        while let Some(id) = self.zero_queue.pop_front() {
            let index = self.validate_slot_identity(id)?;
            let node = {
                let slot = &mut self.slots[index];
                let state = std::mem::replace(&mut slot.state, SlotState::Vacant);
                let SlotState::ZeroQueued(node) = state else {
                    return Err(HeapError::Invariant(
                        "zero queue referenced a node not in ZeroQueued state",
                    ));
                };
                if node.strong != 0 {
                    return Err(HeapError::Invariant(
                        "zero queue contained a nonzero reference count",
                    ));
                }
                slot.state = SlotState::Finalizing(node);

                let state = std::mem::replace(&mut slot.state, SlotState::Vacant);
                let SlotState::Finalizing(node) = state else {
                    return Err(HeapError::Invariant(
                        "node left Finalizing state without a callback boundary",
                    ));
                };
                node
            };

            // A zero-count node cannot have an incoming self-edge, so its slot
            // can be recycled before outgoing edges are processed.
            self.finish_node(id, node, &mut cleanup)?;
            self.reclaim_vacant_slot(id.index())?;
        }
        Ok(cleanup)
    }

    fn finish_node(
        &mut self,
        id: RawId,
        node: Node,
        cleanup: &mut HeapCleanup,
    ) -> Result<(), HeapError> {
        match node.data {
            NodeData::Object(object) => {
                if matches!(
                    &object.payload,
                    ObjectPayload::WeakMap { .. }
                        | ObjectPayload::WeakSet { .. }
                        | ObjectPayload::WeakRef { .. }
                        | ObjectPayload::FinalizationRegistry(_)
                ) {
                    let RawId::Object(object_id) = id else {
                        return Err(HeapError::Invariant(
                            "weak object finalized through a non-object handle",
                        ));
                    };
                    self.unlink_weak_object(object_id)?;
                }
                cleanup.finalized_objects = cleanup.finalized_objects.saturating_add(1);
                cleanup.atoms.extend(object_atoms(&object));
                for edge in object_edges(&object) {
                    self.release_raw_no_drain(edge)?;
                }
            }
            NodeData::Shape(shape) => {
                let RawId::Shape(shape_id) = id else {
                    return Err(HeapError::Invariant(
                        "shape payload finalized through a non-shape handle",
                    ));
                };
                cleanup.finalized_shapes = cleanup.finalized_shapes.saturating_add(1);
                cleanup.finalized_shape_ids.push(shape_id);
                cleanup
                    .atoms
                    .extend(shape.entries().iter().map(|entry| entry.atom));
                for edge in shape_edges(&shape) {
                    self.release_raw_no_drain(edge)?;
                }
            }
            NodeData::VarRef(var_ref) => {
                cleanup.finalized_var_refs = cleanup.finalized_var_refs.saturating_add(1);
                cleanup.atoms.extend(var_ref_atoms(&var_ref));
                for edge in var_ref_edges(&var_ref) {
                    self.release_raw_no_drain(edge)?;
                }
            }
            NodeData::Context(context) => {
                cleanup.finalized_contexts = cleanup.finalized_contexts.saturating_add(1);
                cleanup.atoms.extend(context_atoms(&context));
                for edge in context_edges(&context) {
                    self.release_raw_no_drain(edge)?;
                }
            }
            NodeData::FunctionBytecode(bytecode) => {
                cleanup.finalized_function_bytecodes =
                    cleanup.finalized_function_bytecodes.saturating_add(1);
                cleanup.atoms.extend(function_bytecode_atoms(&bytecode));
                for edge in function_bytecode_edges(&bytecode) {
                    self.release_raw_no_drain(edge)?;
                }
            }
        }
        Ok(())
    }

    fn finalize_cycle_anchor(
        &mut self,
        id: RawId,
        cleanup: &mut HeapCleanup,
    ) -> Result<(), HeapError> {
        if !matches!(
            id,
            RawId::Object(_) | RawId::FunctionBytecode(_) | RawId::Context(_)
        ) {
            return Err(HeapError::Invariant(
                "non-anchor node entered active cycle finalization",
            ));
        }
        let index = self.validate_slot_identity(id)?;
        let node = {
            let slot = &mut self.slots[index];
            let state = std::mem::replace(&mut slot.state, SlotState::Vacant);
            let SlotState::Live(node) = state else {
                return Err(HeapError::Invariant(
                    "cycle anchor was not live when finalization began",
                ));
            };
            if node.data.kind() != id.kind() {
                return Err(HeapError::WrongKind {
                    expected: id.kind(),
                    actual: node.data.kind(),
                });
            }
            slot.state = SlotState::Zombie {
                kind: id.kind(),
                strong: node.strong,
            };
            node
        };
        self.finish_node(id, node, cleanup)
    }

    fn reclaim_slot(&mut self, index: u32) -> Result<(), HeapError> {
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("reclaimed slot disappeared"))?;
        match slot.state {
            SlotState::Zombie { strong: 0, .. } => slot.state = SlotState::Vacant,
            _ => {
                return Err(HeapError::Invariant(
                    "attempted to reclaim a nonzero or non-zombie slot",
                ));
            }
        }
        self.reclaim_vacant_slot(index)
    }

    fn reclaim_vacant_slot(&mut self, index: u32) -> Result<(), HeapError> {
        let weak_id = self.slots.get(index as usize).map(|slot| ObjectId {
            index,
            generation: slot.generation,
        });
        let slot = self
            .slots
            .get_mut(index as usize)
            .ok_or(HeapError::Invariant("reclaimed slot disappeared"))?;
        if !matches!(slot.state, SlotState::Vacant) {
            return Err(HeapError::Invariant(
                "generation advanced before node payload was detached",
            ));
        }
        if slot.weak_prev.is_some()
            || slot.weak_next.is_some()
            || weak_id.is_some_and(|id| self.weak_head == Some(id) || self.weak_tail == Some(id))
        {
            return Err(HeapError::Invariant(
                "weak-collection slot was reclaimed while still linked",
            ));
        }
        if let Some(generation) = slot.generation.checked_add(1) {
            slot.generation = generation;
            self.free.push(index);
        } else {
            slot.state = SlotState::Retired;
        }
        Ok(())
    }
}

pub(super) fn object_layout_edges(shape: ShapeId, slots: &[PropertySlot]) -> Vec<RawId> {
    let mut edges = Vec::with_capacity(slots.len().saturating_add(1));
    for slot in slots {
        edges.extend(property_slot_edges(slot));
    }
    edges.push(RawId::Shape(shape));
    edges
}

pub(super) fn object_edges(object: &ObjectData) -> Vec<RawId> {
    let closure_count = match &object.payload {
        ObjectPayload::Array { dense } => dense.as_ref().map_or(0, |dense| {
            dense
                .iter()
                .filter(|value| matches!(value, RawValue::Object(_)))
                .count()
        }),
        ObjectPayload::Ordinary
        | ObjectPayload::RawJson
        | ObjectPayload::Arguments { .. }
        | ObjectPayload::ArrayIterator { .. }
        | ObjectPayload::ForInIterator(_)
        | ObjectPayload::Primitive(_)
        | ObjectPayload::Date(_)
        | ObjectPayload::RegExp(_)
        | ObjectPayload::ArrayBuffer(_)
        | ObjectPayload::SharedArrayBuffer(_)
        | ObjectPayload::GlobalObject { .. }
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. }
        | ObjectPayload::WeakSet { .. }
        | ObjectPayload::WeakRef { .. }
        | ObjectPayload::Generator { .. } => 0,
        ObjectPayload::DataView(_) | ObjectPayload::TypedArray(_) => 1,
        ObjectPayload::Proxy(_) => 2,
        ObjectPayload::AsyncGenerator(data) => data
            .activation
            .as_deref()
            .map_or(0, |activation| generator_activation_edges(activation).len())
            .saturating_add(
                data.queue
                    .iter()
                    .map(async_generator_request_edges)
                    .map(|edges| edges.len())
                    .sum::<usize>(),
            )
            .saturating_add(usize::from(data.resume_realm.is_some())),
        ObjectPayload::AsyncFunctionState(data) => 3usize.saturating_add(
            data.activation
                .as_deref()
                .map_or(0, |activation| generator_activation_edges(activation).len()),
        ),
        ObjectPayload::IteratorHelper(data) => 1usize
            .saturating_add(raw_value_edges(&data.next).len())
            .saturating_add(raw_value_edges(&data.callback).len())
            .saturating_add(usize::from(data.inner.is_some())),
        ObjectPayload::IteratorWrap(data) => raw_value_edges(&data.source)
            .len()
            .saturating_add(raw_value_edges(&data.next).len()),
        ObjectPayload::AsyncFromSyncIterator(data) => {
            1usize.saturating_add(raw_value_edges(&data.next).len())
        }
        ObjectPayload::IteratorConcat(data) => data
            .items
            .iter()
            .flatten()
            .fold(0_usize, |count, item| {
                count
                    .saturating_add(1)
                    .saturating_add(raw_value_edges(&item.method).len())
            })
            .saturating_add(usize::from(data.iterator.is_some()))
            .saturating_add(raw_value_edges(&data.next).len()),
        ObjectPayload::NativeFunction { internal, .. } => internal
            .as_ref()
            .map_or(0, |internal| internal_callable_edges(internal).len()),
        ObjectPayload::RegExpStringIterator { .. } => 1,
        ObjectPayload::Map { records, .. } => records
            .iter()
            .map(|record| {
                record
                    .key
                    .as_ref()
                    .map_or(0, |key| raw_value_edges(key).len())
                    .saturating_add(raw_value_edges(&record.value).len())
            })
            .sum(),
        ObjectPayload::MapIterator { .. } => 1,
        ObjectPayload::Set { records, .. } => records
            .iter()
            .filter_map(|record| record.key.as_ref())
            .map(|key| raw_value_edges(key).len())
            .sum(),
        ObjectPayload::SetIterator { .. } => 1,
        ObjectPayload::WeakMap { records } => records
            .values()
            .map(|value| raw_value_edges(value).len())
            .sum(),
        ObjectPayload::FinalizationRegistry(data) => data
            .entries
            .iter()
            .map(|entry| raw_value_edges(&entry.held_value).len())
            .sum::<usize>()
            .saturating_add(2),
        ObjectPayload::BoundFunction { arguments, .. } => arguments.len().saturating_add(2),
        ObjectPayload::BytecodeFunction { closure_slots, .. } => closure_slots.len(),
        ObjectPayload::Promise(data) => raw_value_edges(&data.result).len().saturating_add(
            data.fulfill_reactions
                .iter()
                .chain(&data.reject_reactions)
                .map(|reaction| {
                    usize::from(reaction.handler.is_some())
                        .saturating_add(reaction.capability.map_or(0, |_| 2))
                })
                .sum(),
        ),
    };
    let mut edges = Vec::with_capacity(
        object
            .slots
            .len()
            .saturating_add(closure_count)
            .saturating_add(3),
    );
    for slot in &object.slots {
        edges.extend(property_slot_edges(slot));
    }
    edges.push(RawId::Shape(object.shape));
    match &object.payload {
        ObjectPayload::Array { dense } => {
            if let Some(dense) = dense {
                for value in dense {
                    if let RawValue::Object(object) = value {
                        edges.push(RawId::Object(*object));
                    }
                }
            }
        }
        ObjectPayload::Ordinary
        | ObjectPayload::RawJson
        | ObjectPayload::Arguments { .. }
        | ObjectPayload::Primitive(_)
        | ObjectPayload::Date(_)
        | ObjectPayload::RegExp(_)
        | ObjectPayload::ArrayBuffer(_)
        | ObjectPayload::SharedArrayBuffer(_)
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. }
        | ObjectPayload::WeakSet { .. }
        | ObjectPayload::WeakRef { .. } => {}
        ObjectPayload::DataView(data) => {
            edges.push(RawId::Object(data.buffer));
        }
        ObjectPayload::TypedArray(data) => {
            edges.push(RawId::Object(data.view.buffer));
        }
        ObjectPayload::IteratorHelper(data) => {
            edges.push(RawId::Object(data.source));
            edges.extend(raw_value_edges(&data.next));
            edges.extend(raw_value_edges(&data.callback));
            edges.extend(data.inner.map(RawId::Object));
        }
        ObjectPayload::IteratorWrap(data) => {
            edges.extend(raw_value_edges(&data.source));
            edges.extend(raw_value_edges(&data.next));
        }
        ObjectPayload::AsyncFromSyncIterator(data) => {
            edges.push(RawId::Object(data.sync_iterator));
            edges.extend(raw_value_edges(&data.next));
        }
        ObjectPayload::IteratorConcat(data) => {
            // This order mirrors the class finalizer: active iterator, cached
            // next, then the unconsumed captured pairs.
            edges.extend(data.iterator.map(RawId::Object));
            edges.extend(raw_value_edges(&data.next));
            for item in data.items.iter().flatten() {
                edges.push(RawId::Object(item.iterable));
                edges.extend(raw_value_edges(&item.method));
            }
        }
        ObjectPayload::Proxy(data) => {
            // Revocation deliberately leaves both edges intact. This mirrors
            // QuickJS, where JSProxyData continues to own target and handler
            // until the Proxy itself is finalized.
            edges.push(RawId::Object(data.target));
            edges.push(RawId::Object(data.handler));
        }
        ObjectPayload::RegExpStringIterator { regexp, .. } => {
            edges.push(RawId::Object(*regexp));
        }
        ObjectPayload::ArrayIterator { object, .. } => {
            edges.extend(object.map(RawId::Object));
        }
        ObjectPayload::Map { records, .. } => {
            for record in records {
                if let Some(key) = &record.key {
                    edges.extend(raw_value_edges(key));
                    edges.extend(raw_value_edges(&record.value));
                }
            }
        }
        ObjectPayload::MapIterator { object, .. } => {
            edges.extend(object.map(RawId::Object));
        }
        ObjectPayload::Set { records, .. } => {
            for record in records {
                if let Some(key) = &record.key {
                    edges.extend(raw_value_edges(key));
                }
            }
        }
        ObjectPayload::SetIterator { object, .. } => {
            edges.extend(object.map(RawId::Object));
        }
        ObjectPayload::WeakMap { records } => {
            for value in records.values() {
                // Weak keys are intentionally absent from the graph. Values
                // retain their ordinary owned edges, matching QuickJS mark.
                edges.extend(raw_value_edges(value));
            }
        }
        ObjectPayload::FinalizationRegistry(data) => {
            edges.push(RawId::Object(data.callback));
            edges.push(RawId::Context(data.realm));
            for entry in &data.entries {
                // target and unregister_token are intentionally weak. Only
                // held values participate in ordinary trial-deletion tracing.
                edges.extend(raw_value_edges(&entry.held_value));
            }
        }
        ObjectPayload::ForInIterator(data) => {
            edges.extend(data.object.map(RawId::Object));
        }
        ObjectPayload::GlobalObject { uninitialized_vars } => {
            edges.push(RawId::Object(*uninitialized_vars))
        }
        ObjectPayload::NativeFunction { data, internal } => {
            edges.extend(data.realm.map(RawId::Context));
            if let Some(internal) = internal {
                edges.extend(internal_callable_edges(internal));
            }
        }
        ObjectPayload::BoundFunction {
            target,
            this_value,
            arguments,
        } => {
            edges.push(RawId::Object(*target));
            edges.extend(raw_value_edges(this_value));
            for argument in arguments.iter() {
                edges.extend(raw_value_edges(argument));
            }
        }
        ObjectPayload::BytecodeFunction {
            bytecode,
            home_object,
            class_instance_initializer,
            closure_slots,
            ..
        } => {
            if let Some(home_object) = home_object {
                edges.push(RawId::Object(*home_object));
            }
            if let Some(initializer) = class_instance_initializer {
                edges.push(RawId::Object(*initializer));
            }
            edges.push(RawId::FunctionBytecode(*bytecode));
            edges.extend(closure_slots.iter().copied().map(RawId::VarRef));
        }
        ObjectPayload::Generator { activation, .. } => {
            if let Some(activation) = activation.as_deref() {
                edges.extend(generator_activation_edges(activation));
            }
        }
        ObjectPayload::AsyncGenerator(data) => {
            if let Some(activation) = data.activation.as_deref() {
                edges.extend(generator_activation_edges(activation));
            }
            for request in &data.queue {
                edges.extend(async_generator_request_edges(request));
            }
            edges.extend(data.resume_realm.map(RawId::Context));
        }
        ObjectPayload::AsyncFunctionState(data) => {
            edges.push(RawId::Context(data.driver_realm));
            edges.push(RawId::Object(data.outer_resolve));
            edges.push(RawId::Object(data.outer_reject));
            if let Some(activation) = data.activation.as_deref() {
                edges.extend(generator_activation_edges(activation));
            }
        }
        ObjectPayload::Promise(data) => {
            edges.extend(raw_value_edges(&data.result));
            for reaction in data.fulfill_reactions.iter().chain(&data.reject_reactions) {
                edges.extend(promise_reaction_edges(reaction));
            }
        }
    }
    edges
}

fn promise_capability_edges(capability: &PromiseCapabilityData) -> [RawId; 2] {
    [
        RawId::Object(capability.resolve),
        RawId::Object(capability.reject),
    ]
}

pub(super) fn async_generator_request_edges(request: &AsyncGeneratorRequestData) -> Vec<RawId> {
    let mut edges = raw_value_edges(&request.result);
    edges.extend([
        RawId::Object(request.promise),
        RawId::Object(request.resolve),
        RawId::Object(request.reject),
    ]);
    edges
}

pub(super) fn promise_reaction_edges(reaction: &PromiseReaction) -> Vec<RawId> {
    reaction
        .handler
        .map(RawId::Object)
        .into_iter()
        .chain(
            reaction
                .capability
                .as_ref()
                .into_iter()
                .flat_map(promise_capability_edges),
        )
        .collect()
}

fn internal_callable_edges(internal: &InternalCallableData) -> Vec<RawId> {
    match internal {
        InternalCallableData::ProxyRevoke { proxy } => {
            proxy.map(RawId::Object).into_iter().collect()
        }
        InternalCallableData::AsyncFunctionResume { state, .. } => {
            vec![RawId::Object(*state)]
        }
        InternalCallableData::AsyncGeneratorResume { generator, .. } => {
            vec![RawId::Object(*generator)]
        }
        InternalCallableData::PromiseResolving { promise, .. } => {
            vec![RawId::Object(*promise)]
        }
        InternalCallableData::PromiseCapabilityExecutor(capture) => capture
            .resolve
            .iter()
            .chain(capture.reject.iter())
            .flat_map(raw_value_edges)
            .collect(),
        InternalCallableData::PromiseFinallyHandler {
            constructor,
            on_finally,
        } => constructor
            .map(RawId::Object)
            .into_iter()
            .chain(std::iter::once(RawId::Object(*on_finally)))
            .collect(),
        InternalCallableData::PromiseFinallyThunk { value } => raw_value_edges(value),
        InternalCallableData::PromiseAllResolveElement {
            values, resolve, ..
        } => vec![RawId::Object(*values), RawId::Object(*resolve)],
        InternalCallableData::PromiseAllSettledElement {
            values, resolve, ..
        } => vec![RawId::Object(*values), RawId::Object(*resolve)],
        InternalCallableData::PromiseAnyRejectElement { errors, reject, .. } => {
            vec![RawId::Object(*errors), RawId::Object(*reject)]
        }
        InternalCallableData::ModuleEvaluation { module, .. } => {
            vec![RawId::Context(module.cache)]
        }
        InternalCallableData::DynamicImportHandler {
            module,
            resolve,
            reject,
            ..
        } => vec![
            RawId::Context(module.cache),
            RawId::Object(*resolve),
            RawId::Object(*reject),
        ],
        InternalCallableData::AsyncFromSyncIteratorUnwrap { .. } => Vec::new(),
        InternalCallableData::AsyncFromSyncIteratorClose { sync_iterator } => {
            vec![RawId::Object(*sync_iterator)]
        }
    }
}

pub(super) fn generator_activation_edges(activation: &GeneratorActivationData) -> Vec<RawId> {
    let vm = &activation.vm;
    let mut edges = Vec::with_capacity(
        vm.stack
            .len()
            .saturating_add(activation.arguments.len())
            .saturating_add(activation.locals.len())
            .saturating_add(8),
    );
    edges.push(RawId::FunctionBytecode(activation.bytecode));
    edges.push(RawId::Context(vm.callee_realm));
    edges.push(RawId::Object(vm.current_function));
    edges.push(RawId::Object(vm.callee_global));
    for value in vm
        .stack
        .iter()
        .chain(std::iter::once(&vm.this_value))
        .chain(vm.normalized_this.iter())
        .chain(std::iter::once(&vm.new_target))
    {
        edges.extend(raw_value_edges(value));
    }
    for binding in activation.arguments.iter().chain(activation.locals.iter()) {
        match binding {
            GeneratorFrameBinding::Direct(value) => edges.extend(raw_value_edges(value)),
            GeneratorFrameBinding::PrivateCallable(object) => {
                edges.push(RawId::Object(*object));
            }
            GeneratorFrameBinding::Captured(var_ref) => {
                edges.push(RawId::VarRef(*var_ref));
            }
            GeneratorFrameBinding::Private(_) | GeneratorFrameBinding::Uninitialized => {}
        }
    }
    edges
}

pub(super) fn shape_edges(shape: &Shape) -> Vec<RawId> {
    shape
        .prototype()
        .map(|prototype| vec![RawId::Object(prototype)])
        .unwrap_or_default()
}

pub(super) fn var_ref_edges(var_ref: &VarRefData) -> Vec<RawId> {
    raw_value_edges(&var_ref.value)
}

pub(super) fn property_slot_edges(slot: &PropertySlot) -> Vec<RawId> {
    match slot {
        PropertySlot::Data(value) => raw_value_edges(value),
        PropertySlot::VarRef(var_ref) => vec![RawId::VarRef(*var_ref)],
        PropertySlot::Accessor { get, set } => get
            .iter()
            .chain(set.iter())
            .copied()
            .map(RawId::Object)
            .collect(),
        PropertySlot::AutoInit(
            AutoInitProperty::FunctionPrototype { realm }
            | AutoInitProperty::NativeBuiltin { realm, .. }
            | AutoInitProperty::String { realm, .. }
            | AutoInitProperty::ArrayUnscopables { realm }
            | AutoInitProperty::Math { realm }
            | AutoInitProperty::Reflect { realm }
            | AutoInitProperty::Json { realm }
            | AutoInitProperty::Atomics { realm },
        ) => vec![RawId::Context(*realm)],
        #[cfg(test)]
        PropertySlot::AutoInit(AutoInitProperty::FailureProbe { realm }) => {
            vec![RawId::Context(*realm)]
        }
    }
}

pub(super) fn raw_value_edges(value: &RawValue) -> Vec<RawId> {
    match value {
        RawValue::Object(object) => vec![RawId::Object(*object)],
        RawValue::Undefined
        | RawValue::Null
        | RawValue::Bool(_)
        | RawValue::Int(_)
        | RawValue::Float(_)
        | RawValue::BigInt(_)
        | RawValue::String(_)
        | RawValue::Symbol(_)
        | RawValue::Private(_)
        | RawValue::Uninitialized
        | RawValue::Exception => Vec::new(),
    }
}

pub(super) fn context_edges(context: &ContextData) -> Vec<RawId> {
    let mut edges = Vec::with_capacity(
        13usize
            .saturating_add(PrimitiveKind::COUNT)
            .saturating_add(NativeErrorKind::COUNT)
            .saturating_add(context.regexp.map_or(0, |_| 4))
            .saturating_add(context.map.map_or(0, |_| 2))
            .saturating_add(context.set.map_or(0, |_| 2))
            .saturating_add(context.weak_map.map_or(0, |_| 1))
            .saturating_add(context.weak_set.map_or(0, |_| 1))
            .saturating_add(context.weak_ref.map_or(0, |_| 2))
            .saturating_add(context.array_buffer.map_or(0, |_| 1))
            .saturating_add(context.shared_array_buffer.map_or(0, |_| 1))
            .saturating_add(context.data_view.map_or(0, |_| 1))
            .saturating_add(
                context
                    .typed_array
                    .map_or(0, |_| TypedArrayElementKind::COUNT),
            )
            .saturating_add(context.generator.map_or(0, |_| 2))
            .saturating_add(context.async_function.map_or(0, |_| 1))
            .saturating_add(context.async_generator.map_or(0, |_| 4))
            .saturating_add(context.promise.map_or(0, |_| 2))
            .saturating_add(context.iterator.map_or(0, |_| 4))
            .saturating_add(context.global_objects.len())
            .saturating_add(context.intrinsics.len())
            .saturating_add(context.initial_shapes.len()),
    );
    edges.push(RawId::Object(context.object_prototype));
    edges.push(RawId::Object(context.function_prototype));
    edges.push(RawId::Object(context.array_prototype));
    edges.push(RawId::Object(context.iterator_prototype));
    edges.push(RawId::Object(context.array_iterator_prototype));
    edges.push(RawId::Object(context.string_iterator_prototype));
    edges.extend(
        context
            .primitive_prototypes
            .iter()
            .flatten()
            .copied()
            .map(RawId::Object),
    );
    edges.extend(context.date_prototype.map(RawId::Object));
    if let Some(regexp) = context.regexp {
        edges.push(RawId::Object(regexp.prototype));
        edges.push(RawId::Object(regexp.constructor));
        edges.push(RawId::Object(regexp.string_iterator_prototype));
        edges.push(RawId::Shape(regexp.object_shape));
    }
    if let Some(map) = context.map {
        edges.push(RawId::Object(map.prototype));
        edges.push(RawId::Object(map.iterator_prototype));
    }
    if let Some(set) = context.set {
        edges.push(RawId::Object(set.prototype));
        edges.push(RawId::Object(set.iterator_prototype));
    }
    if let Some(weak_map) = context.weak_map {
        edges.push(RawId::Object(weak_map.prototype));
    }
    if let Some(weak_set) = context.weak_set {
        edges.push(RawId::Object(weak_set.prototype));
    }
    if let Some(weak_ref) = context.weak_ref {
        edges.push(RawId::Object(weak_ref.weak_ref_prototype));
        edges.push(RawId::Object(weak_ref.finalization_registry_prototype));
    }
    if let Some(array_buffer) = context.array_buffer {
        edges.push(RawId::Object(array_buffer.prototype));
    }
    if let Some(shared_array_buffer) = context.shared_array_buffer {
        edges.push(RawId::Object(shared_array_buffer.prototype));
    }
    if let Some(data_view) = context.data_view {
        edges.push(RawId::Object(data_view.prototype));
    }
    if let Some(typed_array) = context.typed_array {
        edges.extend(typed_array.prototypes.into_iter().map(RawId::Object));
    }
    if let Some(generator) = context.generator {
        edges.push(RawId::Object(generator.prototype));
        edges.push(RawId::Object(generator.function_prototype));
    }
    if let Some(async_function) = context.async_function {
        edges.push(RawId::Object(async_function.function_prototype));
    }
    if let Some(async_generator) = context.async_generator {
        edges.push(RawId::Object(async_generator.async_iterator_prototype));
        edges.push(RawId::Object(
            async_generator.async_from_sync_iterator_prototype,
        ));
        edges.push(RawId::Object(async_generator.prototype));
        edges.push(RawId::Object(async_generator.function_prototype));
    }
    if let Some(promise) = context.promise {
        edges.push(RawId::Object(promise.prototype));
        edges.push(RawId::Object(promise.constructor));
    }
    if let Some(iterator) = context.iterator {
        edges.push(RawId::Object(iterator.constructor));
        edges.push(RawId::Object(iterator.concat_prototype));
        edges.push(RawId::Object(iterator.helper_prototype));
        edges.push(RawId::Object(iterator.wrap_prototype));
    }
    edges.extend(context.function_constructor.map(RawId::Object));
    edges.extend(context.array_constructor.map(RawId::Object));
    edges.extend(context.array_prototype_values.map(RawId::Object));
    edges.extend(context.throw_type_error.map(RawId::Object));
    edges.extend(context.eval_function.map(RawId::Object));
    edges.push(RawId::Object(context.global_object));
    edges.push(RawId::Object(context.global_var_object));
    edges.extend(context.error_prototype.map(RawId::Object));
    edges.extend(
        context
            .native_error_prototypes
            .iter()
            .flatten()
            .copied()
            .map(RawId::Object),
    );
    edges.extend(context.global_objects.iter().copied().map(RawId::Object));
    for value in &context.intrinsics {
        edges.extend(raw_value_edges(value));
    }
    edges.extend(context.initial_shapes.iter().copied().map(RawId::Shape));
    for record in context.loaded_modules.records.iter().flatten() {
        edges.extend(raw_module_record_edges(record));
    }
    edges
}

pub(super) fn raw_module_record_edges(record: &RawModuleRecord) -> Vec<RawId> {
    let mut edges = Vec::new();
    if let Some(RawModuleLinkRealm::Other(realm)) = record.link_realm {
        edges.push(RawId::Context(realm));
    }
    match &record.body {
        RawModuleRecordBody::Parsing | RawModuleRecordBody::Aborted => {}
        RawModuleRecordBody::SourceText { function } => {
            edges.push(RawId::FunctionBytecode(*function));
        }
        RawModuleRecordBody::Json { default_value } => {
            edges.extend(raw_value_edges(default_value));
        }
    }
    edges.extend(record.import_meta.map(RawId::Object));
    if let Some(instance) = &record.instance {
        edges.extend(instance.slots.iter().flatten().copied().map(RawId::VarRef));
        edges.extend(instance.callable.map(RawId::Object));
    }
    match record.namespace {
        RawModuleNamespaceState::Empty => {}
        RawModuleNamespaceState::Building(namespace)
        | RawModuleNamespaceState::Ready(namespace) => edges.push(RawId::Object(namespace)),
    }
    if let RawModuleEvaluationState::Errored(exception) = &record.evaluation {
        edges.extend(raw_value_edges(exception));
    }
    edges.extend(record.evaluation_promise.map(RawId::Object));
    edges.extend(record.evaluation_resolve.map(RawId::Object));
    edges.extend(record.evaluation_reject.map(RawId::Object));
    edges
}

pub(super) fn function_bytecode_edges(bytecode: &FunctionBytecodeData) -> Vec<RawId> {
    let mut edges = Vec::with_capacity(bytecode.constants.len().saturating_add(1));
    for constant in bytecode.constants.iter() {
        match constant {
            BytecodeConstant::Value(value) => edges.extend(raw_value_edges(value)),
            BytecodeConstant::RegExp { .. } => {}
            BytecodeConstant::Function(function) => {
                edges.push(RawId::FunctionBytecode(*function));
            }
        }
    }
    edges.push(RawId::Context(bytecode.realm));
    edges
}

pub(super) fn property_slot_atoms(slot: &PropertySlot) -> impl Iterator<Item = Atom> + '_ {
    match slot {
        PropertySlot::Data(RawValue::Symbol(atom) | RawValue::Private(atom)) => Some(*atom),
        PropertySlot::Data(_)
        | PropertySlot::VarRef(_)
        | PropertySlot::Accessor { .. }
        | PropertySlot::AutoInit(_) => None,
    }
    .into_iter()
}

fn object_slot_atoms(object: &ObjectData) -> impl Iterator<Item = Atom> + '_ {
    object.slots.iter().flat_map(property_slot_atoms)
}

fn internal_callable_atoms(internal: &InternalCallableData) -> Vec<Atom> {
    match internal {
        InternalCallableData::PromiseCapabilityExecutor(capture) => capture
            .resolve
            .iter()
            .chain(capture.reject.iter())
            .filter_map(raw_value_atom)
            .collect(),
        InternalCallableData::PromiseFinallyThunk { value } => {
            raw_value_atom(value).into_iter().collect()
        }
        InternalCallableData::ProxyRevoke { .. }
        | InternalCallableData::AsyncFunctionResume { .. }
        | InternalCallableData::AsyncGeneratorResume { .. }
        | InternalCallableData::PromiseResolving { .. }
        | InternalCallableData::PromiseFinallyHandler { .. }
        | InternalCallableData::PromiseAllResolveElement { .. }
        | InternalCallableData::PromiseAllSettledElement { .. }
        | InternalCallableData::PromiseAnyRejectElement { .. }
        | InternalCallableData::ModuleEvaluation { .. }
        | InternalCallableData::DynamicImportHandler { .. }
        | InternalCallableData::AsyncFromSyncIteratorUnwrap { .. }
        | InternalCallableData::AsyncFromSyncIteratorClose { .. } => Vec::new(),
    }
}

pub(super) fn object_atoms(object: &ObjectData) -> impl Iterator<Item = Atom> + '_ {
    let payload = match &object.payload {
        ObjectPayload::Primitive(PrimitiveObjectData::Symbol(atom)) => vec![*atom],
        ObjectPayload::Primitive(
            PrimitiveObjectData::Number(_)
            | PrimitiveObjectData::String(_)
            | PrimitiveObjectData::Boolean(_)
            | PrimitiveObjectData::BigInt(_),
        ) => Vec::new(),
        ObjectPayload::BoundFunction {
            this_value,
            arguments,
            ..
        } => raw_value_atom(this_value)
            .into_iter()
            .chain(arguments.iter().filter_map(raw_value_atom))
            .collect::<Vec<_>>(),
        ObjectPayload::Map { records, .. } => records
            .iter()
            .flat_map(|record| {
                record
                    .key
                    .as_ref()
                    .and_then(raw_value_atom)
                    .into_iter()
                    .chain(raw_value_atom(&record.value))
            })
            .collect::<Vec<_>>(),
        ObjectPayload::Set { records, .. } => records
            .iter()
            .filter_map(|record| record.key.as_ref().and_then(raw_value_atom))
            .collect::<Vec<_>>(),
        ObjectPayload::WeakMap { records } => records
            .values()
            .filter_map(raw_value_atom)
            .collect::<Vec<_>>(),
        ObjectPayload::FinalizationRegistry(data) => data
            .entries
            .iter()
            .filter_map(|entry| raw_value_atom(&entry.held_value))
            .collect(),
        ObjectPayload::Generator { activation, .. } => activation
            .as_deref()
            .map(generator_activation_atoms)
            .unwrap_or_default(),
        ObjectPayload::AsyncGenerator(data) => data
            .activation
            .as_deref()
            .map(generator_activation_atoms)
            .into_iter()
            .flatten()
            .chain(
                data.queue
                    .iter()
                    .filter_map(|request| raw_value_atom(&request.result)),
            )
            .collect(),
        ObjectPayload::AsyncFunctionState(data) => data
            .activation
            .as_deref()
            .map(generator_activation_atoms)
            .unwrap_or_default(),
        ObjectPayload::Promise(data) => raw_value_atom(&data.result).into_iter().collect(),
        ObjectPayload::IteratorHelper(data) => raw_value_atom(&data.next)
            .into_iter()
            .chain(raw_value_atom(&data.callback))
            .collect(),
        ObjectPayload::IteratorWrap(data) => raw_value_atom(&data.source)
            .into_iter()
            .chain(raw_value_atom(&data.next))
            .collect(),
        ObjectPayload::AsyncFromSyncIterator(data) => {
            raw_value_atom(&data.next).into_iter().collect()
        }
        ObjectPayload::IteratorConcat(data) => raw_value_atom(&data.next)
            .into_iter()
            .chain(
                data.items
                    .iter()
                    .flatten()
                    .filter_map(|item| raw_value_atom(&item.method)),
            )
            .collect(),
        ObjectPayload::Array { dense } => {
            dense.iter().flatten().filter_map(raw_value_atom).collect()
        }
        ObjectPayload::Proxy(_) => Vec::new(),
        ObjectPayload::NativeFunction {
            internal: Some(internal),
            ..
        } => internal_callable_atoms(internal),
        ObjectPayload::Ordinary
        | ObjectPayload::RawJson
        | ObjectPayload::Arguments { .. }
        | ObjectPayload::ArrayIterator { .. }
        | ObjectPayload::ForInIterator(_)
        | ObjectPayload::Date(_)
        | ObjectPayload::RegExp(_)
        | ObjectPayload::ArrayBuffer(_)
        | ObjectPayload::SharedArrayBuffer(_)
        | ObjectPayload::DataView(_)
        | ObjectPayload::TypedArray(_)
        | ObjectPayload::RegExpStringIterator { .. }
        | ObjectPayload::MapIterator { .. }
        | ObjectPayload::SetIterator { .. }
        | ObjectPayload::WeakSet { .. }
        | ObjectPayload::WeakRef { .. }
        | ObjectPayload::GlobalObject { .. }
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. }
        | ObjectPayload::NativeFunction { .. }
        | ObjectPayload::BytecodeFunction { .. } => Vec::new(),
    };
    object_slot_atoms(object)
        .chain(payload)
        .chain(object.private_brand_home)
}

pub(super) fn generator_activation_atoms(activation: &GeneratorActivationData) -> Vec<Atom> {
    let vm = &activation.vm;
    vm.stack
        .iter()
        .chain(std::iter::once(&vm.this_value))
        .chain(vm.normalized_this.iter())
        .chain(std::iter::once(&vm.new_target))
        .filter_map(raw_value_atom)
        .chain(
            activation
                .arguments
                .iter()
                .chain(activation.locals.iter())
                .filter_map(|binding| match binding {
                    GeneratorFrameBinding::Direct(value) => raw_value_atom(value),
                    GeneratorFrameBinding::Private(atom) => Some(*atom),
                    GeneratorFrameBinding::PrivateCallable(_)
                    | GeneratorFrameBinding::Uninitialized
                    | GeneratorFrameBinding::Captured(_) => None,
                }),
        )
        .collect()
}

pub(super) fn raw_value_atom(value: &RawValue) -> Option<Atom> {
    match value {
        RawValue::Symbol(atom) | RawValue::Private(atom) => Some(*atom),
        RawValue::Undefined
        | RawValue::Null
        | RawValue::Bool(_)
        | RawValue::Int(_)
        | RawValue::Float(_)
        | RawValue::BigInt(_)
        | RawValue::String(_)
        | RawValue::Object(_)
        | RawValue::Uninitialized
        | RawValue::Exception => None,
    }
}

pub(super) fn raw_value_matches_weak_key(value: &RawValue, key: WeakCollectionKey) -> bool {
    match (value, key) {
        (RawValue::Object(value), WeakCollectionKey::Object(key)) => {
            value.index == key.index && value.generation == key.generation
        }
        (RawValue::Symbol(value), WeakCollectionKey::Symbol(key)) => *value == key,
        _ => false,
    }
}

fn context_atoms(context: &ContextData) -> impl Iterator<Item = Atom> + '_ {
    context.intrinsics.iter().filter_map(raw_value_atom).chain(
        context
            .loaded_modules
            .records
            .iter()
            .flatten()
            .flat_map(raw_module_record_atoms),
    )
}

pub(super) fn raw_module_record_atoms(record: &RawModuleRecord) -> impl Iterator<Item = Atom> + '_ {
    let body = match &record.body {
        RawModuleRecordBody::Json { default_value } => raw_value_atom(default_value),
        RawModuleRecordBody::Parsing
        | RawModuleRecordBody::SourceText { .. }
        | RawModuleRecordBody::Aborted => None,
    };
    let evaluation = match &record.evaluation {
        RawModuleEvaluationState::Errored(exception) => raw_value_atom(exception),
        RawModuleEvaluationState::Unevaluated
        | RawModuleEvaluationState::Evaluating
        | RawModuleEvaluationState::EvaluatingAsync
        | RawModuleEvaluationState::Evaluated
        | RawModuleEvaluationState::Poisoned => None,
    };
    body.into_iter().chain(evaluation)
}

fn function_bytecode_atoms(bytecode: &FunctionBytecodeData) -> impl Iterator<Item = Atom> + '_ {
    bytecode
        .auxiliary_atoms
        .iter()
        .copied()
        .chain(
            bytecode
                .constants
                .iter()
                .filter_map(|constant| match constant {
                    BytecodeConstant::Value(value) => raw_value_atom(value),
                    BytecodeConstant::RegExp { .. } | BytecodeConstant::Function(_) => None,
                }),
        )
}

fn var_ref_atoms(var_ref: &VarRefData) -> impl Iterator<Item = Atom> + '_ {
    raw_value_atom(&var_ref.value).into_iter()
}
