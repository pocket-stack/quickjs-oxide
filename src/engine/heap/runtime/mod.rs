//! Runtime and context ownership boundaries.
//!
//! As in QuickJS, a runtime owns resources shared by multiple contexts, while
//! each context is a separate realm and execution surface. The heap and
//! intrinsics extend this boundary; they are not hidden in the compiler or VM.

mod layout;
use self::error::RuntimeError;
use self::intrinsics::promise::HostPromiseRejectionTracker;
use self::module::ModuleLoader;
use crate::engine::api::runtime_error as error;
use crate::engine::host::HostServices;

use crate::engine::{builtins as intrinsics, jobs, modules as module};

use crate::engine::atom::{Atom, AtomIdx, AtomTable};
use crate::engine::code::debug::DebugInfoMode;
use crate::engine::heap::{
    ContextId, FunctionBytecodeId, Heap, HeapCleanup, ObjectId, PropertySlot, RawValue, ShapeId,
    StringId, BigIntId, VarRefId,
};
use crate::engine::object::WellKnownSymbol;
use crate::engine::object::shape::{Shape, ShapeEntry};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::atomic::AtomicU64;

pub(crate) static NEXT_RUNTIME_DOMAIN_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct RuntimeInner {
    pub(crate) state: RefCell<RuntimeState>,
    /// Incremental activation count, readable without borrowing heap state.
    pub(crate) active_frame_depth: Rc<Cell<usize>>,
    pub(crate) deferred_references: super::deferred::DeferredOperations,
    pub(crate) host_services: Rc<dyn HostServices>,
    /// Embedder policy sampled by synchronous Atomics waits. QuickJS leaves
    /// this disabled until a host explicitly opts in.
    pub(crate) can_block: Cell<bool>,
    pub(crate) promise_rejection_tracker: RefCell<Option<HostPromiseRejectionTracker>>,
    pub(crate) module_loader: RefCell<Option<Weak<dyn ModuleLoader>>>,
    /// Optional host policy for bytecode which may contain or execute the
    /// dynamic-import opcode. Ordinary builds have no such policy; the
    /// non-default conformance host closes it until one isolated program has
    /// passed its external source and bytecode admission checks.
    #[cfg(feature = "test262-host")]
    pub(crate) dynamic_import_bytecode_allowed: Cell<bool>,
    /// Synchronous module-host callbacks currently on the native stack.
    /// Loader re-entry is part of the QuickJS contract, so this participates
    /// in the shared host-stack budget instead of acting as an exclusion lock.
    pub(crate) module_host_callback_depth: Cell<usize>,
    /// Address marker captured at the outermost JavaScript, native, or module-
    /// host entry. Nested guards compare against it using QuickJS's one-MiB
    /// host-stack budget; no pointer is dereferenced after the marker ends.
    pub(crate) host_stack_top: Cell<Option<usize>>,
    pub(crate) proxy_method_depth: Cell<usize>,
    /// Maximum number of installed JavaScript call frames for one top-level
    /// execution. Read at execution entry; the default matches `ExecutionLimits`.
    pub(crate) recursion_limit: Cell<usize>,
    pub(crate) next_context_id: Cell<u64>,
    pub(crate) domain_id: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DeferredRefOp {
    Object(ObjectId),
    Context(ContextId),
    FunctionBytecode(FunctionBytecodeId),
    VarRef(VarRefId),
    String(StringId),
    BigInt(BigIntId),
    /// Shared-release pass deferred because the table was mutably borrowed:
    /// decrement the counter, then remove the slot if it reached zero.
    AtomRelease(Atom),
    /// Slot removal for an atom whose counter already reached zero under a
    /// shared borrow.
    AtomRemove(Atom),
    ActiveFramePop {
        token: ActiveFrameToken,
        depth: usize,
    },
    ActiveCollectionRecordsTruncate {
        depth: usize,
    },
    BacktraceBarrierRestore {
        token: ActiveFrameToken,
        previous: bool,
    },
}

pub(crate) struct RuntimeOperation<'a>(pub(super) &'a Runtime);

impl Drop for RuntimeOperation<'_> {
    #[inline]
    fn drop(&mut self) {
        let result = self.0.drain_deferred_references();
        debug_assert!(result.is_ok(), "deferred root release failed: {result:?}");
    }
}

pub(crate) struct RuntimeState {
    pub(crate) atoms: AtomTable,
    pub(crate) pinned_atoms: crate::engine::atom::pinned::PinnedAtoms,
    pub(crate) heap: Heap,
    /// Runtime-owned pending JavaScript exception. Object and Symbol payloads
    /// carry one manually retained root; no public `Value::Exception` sentinel
    /// exists.
    pub(crate) pending_exception: Option<RawValue>,
    /// QuickJS `JSRuntime.job_list`: jobs are FIFO and own only raw arena/
    /// atom roots so the queue never creates an `Rc<RuntimeInner>` cycle.
    pub(crate) pending_jobs: VecDeque<jobs::PendingJob>,
    /// Observable function-debug portion of QuickJS `JS_SetStripInfo`, sampled
    /// by each subsequent compilation.
    pub(crate) debug_info_mode: DebugInfoMode,
    /// QuickJS's shape hash is non-owning. These generational IDs are likewise
    /// weak and are validated before reuse.
    pub(crate) shape_cache: HashMap<ShapeFingerprint, ShapeId>,
    pub(crate) shape_fingerprints: HashMap<ShapeId, ShapeFingerprint>,
    pub(crate) shape_transitions: HashMap<ShapeId, HashMap<ShapeEntry, ShapeId>>,
    pub(crate) shape_transition_parents: HashMap<ShapeId, Vec<(ShapeId, ShapeEntry)>>,
    pub(crate) well_known_symbols: HashMap<WellKnownSymbol, Atom>,
    /// One guarded handler-trap location cache per Proxy internal method.
    /// Indexed by the closed trap selector in `PinnedAtom::proxy_method`.
    pub(crate) proxy_trap_reads: [crate::engine::object::property_ic::PropertyReadCache;
        crate::engine::atom::pinned::PROXY_METHOD_COUNT],
    /// Unified QuickJS-style execution-frame chain. Records contain only raw
    /// stable identities and diagnostic state; the corresponding stack-local
    /// [`ActiveFrameGuard`] or its authenticated running frame owns the object
    /// and bytecode roots; the registry never owns Runtime roots.
    pub(crate) active_frames: crate::engine::vm::frames::ActiveFrames,
    /// Collection records retained across an active user callback. QuickJS
    /// keeps the current Map/Set record alive during `forEach` and direct Set
    /// method traversal, which makes a deletion transiently visible to
    /// `JS_PrintValue` as an empty record. [`ActiveCollectionRecordGuard`]
    /// restores this diagnostic stack on every exit path.
    pub(crate) active_collection_records: Vec<ActiveCollectionRecord>,
    pub(crate) next_active_frame_token: u64,
    /// QuickJS's runtime-global ordering source for async module evaluation.
    /// Zero is a valid first stamp; exhaustion is reported before publication.
    pub(crate) next_module_async_evaluation_order: u64,
    #[cfg(test)]
    pub(crate) active_frame_probe_snapshots: Vec<Vec<ActiveFrameRecord>>,
    /// Counts the ordinary `{ value, done }` wrappers produced by the native
    /// iterator-next call adapter.  The direct VM fast path deliberately does
    /// not increment this counter.
    #[cfg(test)]
    pub(crate) iterator_result_allocations: usize,
}

impl RuntimeState {
    pub(crate) fn preflight_atom_releases(&self, atoms: &[Atom]) -> Result<(), RuntimeError> {
        let mut counts = HashMap::<Atom, u32>::new();
        counts.try_reserve(atoms.len()).map_err(|_| {
            RuntimeError::Invariant("module atom release preflight allocation failed")
        })?;
        for &atom in atoms {
            let count = counts.entry(atom).or_default();
            *count = count.checked_add(1).ok_or(RuntimeError::Invariant(
                "module atom release count overflow",
            ))?;
        }
        for (atom, removed) in counts {
            let info = self.atoms.resolve(atom)?;
            if let Some(ref_count) = info.ref_count
                && ref_count < removed
            {
                return Err(RuntimeError::Invariant(
                    "module atom release exceeds its owned occurrences",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn apply_committed_cleanup(&mut self, cleanup: HeapCleanup) {
        self.unlink_finalized_shapes(cleanup.finalized_shape_ids);
        self.release_atom_indices(cleanup.atoms)
            .expect("committed heap cleanup atom release failed");
    }

    pub(crate) fn release_owned_raw_root_committed(&mut self, value: RawValue) {
        match value {
            RawValue::Object(object) => {
                let cleanup = self
                    .heap
                    .release_object(object)
                    .expect("committed pending-exception object release failed");
                self.apply_committed_cleanup(cleanup);
            }
            RawValue::Symbol(index) => {
                self.atoms
                    .release_index(index)
                    .expect("committed pending-exception Symbol release failed");
            }
            RawValue::String(id) => {
                let cleanup = self
                    .heap
                    .release_string(id)
                    .expect("committed pending-exception string release failed");
                self.apply_committed_cleanup(cleanup);
            }
            RawValue::BigInt(id) => {
                let cleanup = self
                    .heap
                    .release_bigint(id)
                    .expect("committed pending-exception bigint release failed");
                self.apply_committed_cleanup(cleanup);
            }
            RawValue::Undefined
            | RawValue::Null
            | RawValue::Bool(_)
            | RawValue::Int(_)
            | RawValue::Float(_) => {}
            RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception => {
                unreachable!("internal value occupied committed pending-exception storage")
            }
        }
    }

    pub(crate) fn retain_raw_root(&mut self, value: &RawValue) -> Result<(), RuntimeError> {
        match value {
            RawValue::Object(object) => self.heap.retain_object(*object)?,
            RawValue::Symbol(index) => {
                self.atoms.retain_index(*index)?;
            }
            RawValue::String(id) => self.heap.retain_string(*id)?,
            RawValue::BigInt(id) => self.heap.retain_bigint(*id)?,
            RawValue::Private(_) => {
                return Err(RuntimeError::Invariant(
                    "private-name identity cannot become a public runtime root",
                ));
            }
            RawValue::Undefined
            | RawValue::Null
            | RawValue::Bool(_)
            | RawValue::Int(_)
            | RawValue::Float(_) => {}
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel cannot become a runtime root",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn release_owned_raw_root(&mut self, value: RawValue) -> Result<(), RuntimeError> {
        match value {
            RawValue::Object(object) => {
                let cleanup = self.heap.release_object(object)?;
                self.apply_cleanup(cleanup)?;
            }
            RawValue::Symbol(index) => {
                self.atoms.release_index(index)?;
            }
            RawValue::String(id) => {
                let cleanup = self.heap.release_string(id)?;
                self.apply_cleanup(cleanup)?;
            }
            RawValue::BigInt(id) => {
                let cleanup = self.heap.release_bigint(id)?;
                self.apply_cleanup(cleanup)?;
            }
            RawValue::Private(_) => {
                return Err(RuntimeError::Invariant(
                    "private-name identity occupied a public runtime root",
                ));
            }
            RawValue::Undefined
            | RawValue::Null
            | RawValue::Bool(_)
            | RawValue::Int(_)
            | RawValue::Float(_) => {}
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel occupied a runtime root",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn get_or_create_shape(
        &mut self,
        prototype: Option<ObjectId>,
        entries: &[ShapeEntry],
    ) -> Result<ShapeId, RuntimeError> {
        let fingerprint = ShapeFingerprint {
            prototype,
            entries: entries.into(),
        };
        if let Some(&shape) = self.shape_cache.get(&fingerprint) {
            if self.heap.shape(shape).is_ok() {
                self.heap.retain_shape(shape)?;
                return Ok(shape);
            }
            self.shape_cache.remove(&fingerprint);
            self.shape_fingerprints.remove(&shape);
        }

        let retained_atoms = self.retain_shape_atoms(entries)?;

        let shape = match Shape::new(prototype, entries.iter().copied()) {
            Ok(shape) => shape,
            Err(error) => {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
        };
        let shape = match self.heap.allocate_shape(shape) {
            Ok(shape) => shape,
            Err(error) => {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
        };
        self.shape_cache.insert(fingerprint.clone(), shape);
        self.shape_fingerprints.insert(shape, fingerprint);
        Ok(shape)
    }

    /// Append-only edges borrow both shapes; collection/mutation unlinks them.
    pub(crate) fn append_transition(
        &mut self,
        parent: ShapeId,
        entry: ShapeEntry,
    ) -> Result<ShapeId, RuntimeError> {
        if let Some(&target) = self
            .shape_transitions
            .get(&parent)
            .and_then(|edges| edges.get(&entry))
        {
            if self.heap.shape(target).is_ok() {
                self.heap.retain_shape(target)?;
                return Ok(target);
            }
            // A weak target may have entered zero-queue before its cleanup was
            // delivered to the runtime. Never revive a stale location blindly.
            self.unlink_shape_transitions(target);
        }
        let source = self.heap.shape(parent)?;
        let prototype = source.prototype();
        let mut entries = source.entries().to_vec();
        entries.push(entry);
        let target = self.get_or_create_shape(prototype, &entries)?;
        self.shape_transitions
            .entry(parent)
            .or_default()
            .insert(entry, target);
        self.shape_transition_parents
            .entry(target)
            .or_default()
            .push((parent, entry));
        Ok(target)
    }

    pub(crate) fn unlink_shape_transitions(&mut self, shape: ShapeId) {
        if let Some(edges) = self.shape_transitions.remove(&shape) {
            for (entry, target) in edges {
                if let Some(parents) = self.shape_transition_parents.get_mut(&target) {
                    parents.retain(|pair| *pair != (shape, entry));
                    if parents.is_empty() {
                        self.shape_transition_parents.remove(&target);
                    }
                }
            }
        }
        if let Some(parents) = self.shape_transition_parents.remove(&shape) {
            for (parent, entry) in parents {
                if let Some(edges) = self.shape_transitions.get_mut(&parent) {
                    edges.remove(&entry);
                    if edges.is_empty() {
                        self.shape_transitions.remove(&parent);
                    }
                }
            }
        }
    }

    pub(crate) fn retain_shape_atoms(
        &mut self,
        entries: &[ShapeEntry],
    ) -> Result<Vec<Atom>, RuntimeError> {
        let mut retained_atoms = Vec::with_capacity(entries.len());
        for entry in entries {
            // Shape entries hold unbranded indices under the retain invariant;
            // the index operation itself validates the slot before counting.
            if let Err(error) = self.atoms.retain_index(entry.atom) {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
            retained_atoms.push(Atom::from_raw(entry.atom.raw()));
        }

        Ok(retained_atoms)
    }

    pub(crate) fn retain_slot_atoms(
        &mut self,
        slots: &[PropertySlot],
    ) -> Result<Vec<Atom>, RuntimeError> {
        let indices = slots
            .iter()
            .filter_map(|slot| match slot {
                PropertySlot::Data(RawValue::Symbol(index) | RawValue::Private(index)) => {
                    Some(*index)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut retained = Vec::with_capacity(indices.len());
        for index in indices {
            if let Err(error) = self.atoms.retain_index(index) {
                self.release_atoms(retained)?;
                return Err(error.into());
            }
            retained.push(crate::engine::atom::Atom::from_raw(index.raw()));
        }
        Ok(retained)
    }

    pub(crate) fn retain_raw_value_atoms<'a>(
        &mut self,
        values: impl IntoIterator<Item = &'a RawValue>,
    ) -> Result<Vec<Atom>, RuntimeError> {
        let indices = values.into_iter().filter_map(|value| match value {
            RawValue::Symbol(index) | RawValue::Private(index) => Some(*index),
            _ => None,
        });
        let mut retained = Vec::new();
        for index in indices {
            if let Err(error) = self.atoms.retain_index(index) {
                self.release_atoms(retained)?;
                return Err(error.into());
            }
            retained.push(crate::engine::atom::Atom::from_raw(index.raw()));
        }
        Ok(retained)
    }

    pub(crate) fn replace_layout(
        &mut self,
        object: ObjectId,
        prototype: Option<ObjectId>,
        entries: &[ShapeEntry],
        slots: Vec<PropertySlot>,
    ) -> Result<(), RuntimeError> {
        if self
            .heap
            .shape(self.heap.object(object)?.shape)?
            .is_dictionary()
        {
            return self.replace_dictionary_layout(object, prototype, entries, slots);
        }
        let previous = self.heap.object(object)?.shape;
        let source = self.heap.shape(previous)?;
        let appended = entries.len() == source.entries().len() + 1
            && prototype == source.prototype()
            && entries[..source.entries().len()] == *source.entries();
        let shape = if appended {
            self.append_transition(previous, *entries.last().expect("append entry"))?
        } else {
            self.unlink_shape_transitions(previous);
            self.get_or_create_shape(prototype, entries)?
        };
        self.replace_layout_with_owned_shape(object, shape, slots)
    }

    /// Consume the caller's shape reference on both success and rollback.
    pub(crate) fn replace_layout_with_owned_shape(
        &mut self,
        object: ObjectId,
        shape: ShapeId,
        slots: Vec<PropertySlot>,
    ) -> Result<(), RuntimeError> {
        let retained_atoms = match self.retain_slot_atoms(&slots) {
            Ok(atoms) => atoms,
            Err(error) => {
                let cleanup = self.heap.release_shape(shape)?;
                self.apply_cleanup(cleanup)?;
                return Err(error);
            }
        };

        let layout_cleanup = match self.heap.replace_object_layout(object, shape, slots) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                self.release_atoms(retained_atoms)?;
                let cleanup = self.heap.release_shape(shape)?;
                self.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let shape_cleanup = self.heap.release_shape(shape)?;
        self.apply_cleanup(layout_cleanup)?;
        self.apply_cleanup(shape_cleanup)
    }

    pub(crate) fn materialize_array_layout(
        &mut self,
        object: ObjectId,
        prototype: Option<ObjectId>,
        entries: &[ShapeEntry],
    ) -> Result<(), RuntimeError> {
        let shape = self.get_or_create_shape(prototype, entries)?;
        let layout_cleanup = match self.heap.materialize_array_dense_shape(object, shape) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                let cleanup = self.heap.release_shape(shape)?;
                self.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let shape_cleanup = self.heap.release_shape(shape)?;
        self.apply_cleanup(layout_cleanup)?;
        self.apply_cleanup(shape_cleanup)?;
        // Preserve QuickJS's slow-form flag, but do not keep rebuilding every
        // property after a middle deletion or a non-default descriptor.
        self.ensure_dictionary_layout(object)
    }

    /// One slot transaction, including Atom ownership. Once published, a
    /// cleanup failure cannot roll back the new slot's Atom references.
    pub(crate) fn replace_property_slot(
        &mut self,
        object: ObjectId,
        index: usize,
        replacement: PropertySlot,
    ) -> Result<(), RuntimeError> {
        let atoms = self.retain_slot_atoms(std::slice::from_ref(&replacement))?;
        match self
            .heap
            .replace_object_slot_with_status(object, index, replacement)
        {
            Ok(cleanup) => self.apply_cleanup(cleanup),
            Err(failure) => {
                if !failure.published {
                    self.release_atoms(atoms)?;
                }
                Err(failure.error.into())
            }
        }
    }

    pub(crate) fn apply_cleanup(&mut self, cleanup: HeapCleanup) -> Result<(), RuntimeError> {
        self.unlink_finalized_shapes(cleanup.finalized_shape_ids);
        self.release_atom_indices(cleanup.atoms)
    }

    pub(crate) fn unlink_finalized_shapes(&mut self, shapes: impl IntoIterator<Item = ShapeId>) {
        for shape in shapes {
            self.unlink_shape_transitions(shape);
            let Some(fingerprint) = self.shape_fingerprints.remove(&shape) else {
                continue;
            };
            if self.shape_cache.get(&fingerprint) == Some(&shape) {
                self.shape_cache.remove(&fingerprint);
            }
        }
    }

    /// Release a rollback list produced by the `retain_*` helpers above.
    ///
    /// Those lists may carry unbranded `Atom::from_raw` reconstructions (the
    /// helpers retain by unbranded index), so release goes through the index
    /// operation — which re-validates the slot — rather than the branded
    /// public [`AtomTable::release`].
    pub(crate) fn release_atoms(
        &mut self,
        atoms: impl IntoIterator<Item = Atom>,
    ) -> Result<(), RuntimeError> {
        for atom in atoms {
            self.atoms.release_index(AtomIdx::from_raw(atom.raw()))?;
        }
        Ok(())
    }

    /// Release unbranded atom indices returned from heap cleanup.  Each index
    /// was owned by the finalized node; the table validates the slot again on
    /// the way out.
    pub(crate) fn release_atom_indices(
        &mut self,
        indices: impl IntoIterator<Item = AtomIdx>,
    ) -> Result<(), RuntimeError> {
        for index in indices {
            self.atoms.release_index(index)?;
        }
        Ok(())
    }
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        let state = self.state.get_mut();
        let deferred = &self.deferred_references;
        while let Some(operation) = deferred.pop_front() {
            let result = state.apply_deferred_operation(operation);
            debug_assert!(
                result.is_ok(),
                "runtime deferred teardown failed: {result:?}"
            );
        }
        while let Some(job) = state.pending_jobs.pop_front() {
            let result = state.release_pending_job_roots(&job);
            debug_assert!(result.is_ok(), "pending job teardown failed: {result:?}");
        }
        if let Some(exception) = state.pending_exception.take() {
            let result = state.release_owned_raw_root(exception);
            debug_assert!(
                result.is_ok(),
                "pending exception teardown failed: {result:?}"
            );
        }
        let result = state
            .heap
            .run_gc_for_runtime_teardown()
            .map_err(RuntimeError::Heap)
            .and_then(|mut stats| {
                let atom_indices = std::mem::take(&mut stats.cleanup.atoms);
                state.release_atom_indices(atom_indices)
            });
        debug_assert!(result.is_ok(), "runtime teardown failed: {result:?}");
        debug_assert_eq!(
            state.heap.counts().live,
            0,
            "runtime teardown left live heap nodes"
        );
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod static_property_key_tests;

#[cfg(test)]
mod builtin_batch_tests;

use crate::engine::vm::frames::*;

use crate::engine::object::operations::*;

use crate::engine::api::runtime::Runtime;
