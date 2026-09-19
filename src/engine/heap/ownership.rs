use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::atom::{Atom, AtomError};
use crate::engine::heap::runtime::{DeferredRefOp, RuntimeOperation, RuntimeState};
use crate::engine::heap::{
    BigIntId, ContextId, FunctionBytecodeId, HeapError, ObjectId, RawId, RawValue, StringId, VarRefId,
};

impl Runtime {
    #[inline]
    pub(crate) fn operation(&self) -> RuntimeOperation<'_> {
        let result = self.drain_deferred_references();
        debug_assert!(result.is_ok(), "deferred root release failed: {result:?}");
        RuntimeOperation(self)
    }

    #[inline]
    pub(crate) fn drain_deferred_references(&self) -> Result<(), RuntimeError> {
        if !self.0.deferred_references.has_pending() {
            return Ok(());
        }
        self.drain_deferred_references_slow()
    }

    fn drain_deferred_references_slow(&self) -> Result<(), RuntimeError> {
        let deferred = &self.0.deferred_references;
        let Some(_drain) = deferred.try_start_draining() else {
            return Ok(());
        };
        loop {
            // Do not remove work until it can execute. In particular, blocked
            // drains must leave restoration operations at their original priority.
            let Ok(mut state) = self.0.state.try_borrow_mut() else {
                return Ok(());
            };
            let Some(operation) = deferred.pop_front() else {
                break;
            };
            state.apply_deferred_operation(operation)?;
        }
        Ok(())
    }

    #[inline]
    fn release_or_defer(&self, operation: DeferredRefOp) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            state.apply_deferred_operation(operation)
        } else {
            self.0.deferred_references.push_back(operation);
            // The state is still borrowed. The next existing operation boundary
            // (or a successful release) drains this work after the borrow ends.
            return;
        };
        debug_assert!(
            result.is_ok(),
            "invalid root release {operation:?}: {result:?}"
        );
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred root release failed: {drain:?}");
    }

    pub(crate) fn retain_object_handle(&self, id: ObjectId) -> Result<(), HeapError> {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            return state.heap.retain_object(id);
        }
        // A nested read may hold a shared state borrow (for example an
        // autoinit property materialization rooting its value).  The counter
        // lives in a `Cell`, so a validated shared-borrow retain is exact.
        let state = self.0.state.try_borrow().map_err(|_| {
            HeapError::Invariant("object root retained during a runtime state borrow")
        })?;
        state.heap.retain_object_shared(id)
    }

    pub(crate) fn release_object_handle(&self, id: ObjectId) {
        self.release_or_defer(DeferredRefOp::Object(id));
    }

    pub(crate) fn retain_atom_handle(&self, atom: Atom) -> Result<(), AtomError> {
        self.0.state.borrow().atoms.retain_shared(atom).map(drop)
    }

    pub(crate) fn retain_function_bytecode_handle(
        &self,
        id: FunctionBytecodeId,
    ) -> Result<(), HeapError> {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            return state.heap.retain_function_bytecode(id);
        }
        let state = self.0.state.try_borrow().map_err(|_| {
            HeapError::Invariant("function bytecode retained during a runtime state borrow")
        })?;
        state.heap.retain_function_bytecode_shared(id)
    }

    pub(crate) fn retain_context_handle(&self, id: ContextId) -> Result<(), HeapError> {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            return state.heap.retain_context(id);
        }
        let state = self.0.state.try_borrow().map_err(|_| {
            HeapError::Invariant("context retained during a runtime state borrow")
        })?;
        state.heap.retain_context_shared(id)
    }

    pub(crate) fn release_context_handle(&self, id: ContextId) {
        self.release_or_defer(DeferredRefOp::Context(id));
    }

    pub(crate) fn release_function_bytecode_handle(&self, id: FunctionBytecodeId) {
        self.release_or_defer(DeferredRefOp::FunctionBytecode(id));
    }

    pub(crate) fn retain_var_ref_handle(&self, id: VarRefId) -> Result<(), HeapError> {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            return state.heap.retain_var_ref(id);
        }
        let state = self.0.state.try_borrow().map_err(|_| {
            HeapError::Invariant("VarRef retained during a runtime state borrow")
        })?;
        state.heap.retain_var_ref_shared(id)
    }

    pub(crate) fn release_var_ref_handle(&self, id: VarRefId) {
        self.release_or_defer(DeferredRefOp::VarRef(id));
    }

    pub(crate) fn release_atom_handle(&self, atom: Atom) {
        // Shared-borrow release: the counter decrement runs immediately; when
        // the last reference drops, slot removal is deferred to the next
        // operation boundary through the existing deferred queue.  An invalid
        // release is re-deferred so the error surfaces at the drain boundary
        // exactly like the historical full-release path.
        let hit_zero = match self.0.state.try_borrow() {
            Ok(state) => match state.atoms.release_shared(atom) {
                Ok(hit_zero) => hit_zero,
                Err(_) => {
                    drop(state);
                    self.0
                        .deferred_references
                        .push_back(DeferredRefOp::AtomRelease(atom));
                    return;
                }
            },
            Err(_) => {
                // The table is mutably borrowed (interning/removal in
                // progress). Defer the whole shared-release pass; it runs at
                // the next safe point.
                self.0
                    .deferred_references
                    .push_back(DeferredRefOp::AtomRelease(atom));
                return;
            }
        };
        if hit_zero {
            self.release_or_defer(DeferredRefOp::AtomRemove(atom));
        }
    }

    pub(crate) fn retain_string_handle(&self, id: StringId) -> Result<(), HeapError> {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            return state.heap.retain_string(id);
        }
        let state = self.0.state.try_borrow().map_err(|_| {
            HeapError::Invariant("string node retained during a runtime state borrow")
        })?;
        state.heap.retain_string_shared(id)
    }

    pub(crate) fn release_string_handle(&self, id: StringId) {
        self.release_or_defer(DeferredRefOp::String(id));
    }

    pub(crate) fn retain_bigint_handle(&self, id: BigIntId) -> Result<(), HeapError> {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            return state.heap.retain_bigint(id);
        }
        let state = self.0.state.try_borrow().map_err(|_| {
            HeapError::Invariant("bigint node retained during a runtime state borrow")
        })?;
        state.heap.retain_bigint_shared(id)
    }

    pub(crate) fn release_bigint_handle(&self, id: BigIntId) {
        self.release_or_defer(DeferredRefOp::BigInt(id));
    }

    /// Release one producer-owned string/BigInt conversion edge after the
    /// transactional store has retained its own copy edge.
    pub(crate) fn release_converted_node_edge(&self, edge: RawId) {
        match edge {
            RawId::String(id) => self.release_or_defer(DeferredRefOp::String(id)),
            RawId::BigInt(id) => self.release_or_defer(DeferredRefOp::BigInt(id)),
            _ => unreachable!("conversion edges are only string or bigint node edges"),
        }
    }

    /// Release the conversion edge carried by a boundary-converted value, if
    /// any.  See [`RawValue::conversion_node_edge`].
    pub(crate) fn release_converted_value_edge(&self, value: &RawValue) {
        if let Some(edge) = value.conversion_node_edge() {
            self.release_converted_node_edge(edge);
        }
    }
}
impl RuntimeState {
    #[inline]
    fn release_heap_reference(&mut self, id: RawId) -> Result<(), RuntimeError> {
        if let Some(cleanup) = self.heap.release_reference(id)? {
            self.apply_cleanup(cleanup)?;
        }
        Ok(())
    }

    /// Apply one raw release or restoration operation at an existing safe point.
    #[inline]
    pub(crate) fn apply_deferred_operation(
        &mut self,
        operation: DeferredRefOp,
    ) -> Result<(), RuntimeError> {
        match operation {
            DeferredRefOp::Object(object) => self.release_heap_reference(RawId::Object(object)),
            DeferredRefOp::Context(context) => self.release_heap_reference(RawId::Context(context)),
            DeferredRefOp::FunctionBytecode(bytecode) => {
                self.release_heap_reference(RawId::FunctionBytecode(bytecode))
            }
            DeferredRefOp::VarRef(var_ref) => self.release_heap_reference(RawId::VarRef(var_ref)),
            DeferredRefOp::String(id) => self.release_heap_reference(RawId::String(id)),
            DeferredRefOp::BigInt(id) => self.release_heap_reference(RawId::BigInt(id)),
            DeferredRefOp::AtomRelease(atom) => {
                if self.atoms.release_shared(atom)? {
                    self.atoms.remove_released(atom)?;
                }
                Ok(())
            }
            DeferredRefOp::AtomRemove(atom) => self.atoms.remove_released(atom).map_err(Into::into),
            DeferredRefOp::ActiveFramePop { token, depth } => {
                self.active_frames.retire(token, depth);
                Ok(())
            }
            DeferredRefOp::ActiveCollectionRecordsTruncate { depth } => {
                self.active_collection_records.truncate(depth);
                Ok(())
            }
            DeferredRefOp::BacktraceBarrierRestore { token, previous } => {
                if let Some(frame) = self
                    .active_frames
                    .iter_mut()
                    .find(|frame| frame.token == token)
                {
                    frame.flags.backtrace_barrier = previous;
                }
                Ok(())
            }
        }
    }
}
