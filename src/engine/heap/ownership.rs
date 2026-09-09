use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::atom::{Atom, AtomError};
use crate::engine::heap::runtime::{DeferredRefOp, RuntimeOperation};
use crate::engine::heap::{ContextId, FunctionBytecodeId, HeapError, ObjectId, VarRefId};

impl Runtime {
    pub(crate) fn operation(&self) -> RuntimeOperation<'_> {
        let result = self.drain_deferred_references();
        debug_assert!(result.is_ok(), "deferred root release failed: {result:?}");
        RuntimeOperation(self)
    }

    pub(crate) fn drain_deferred_references(&self) -> Result<(), RuntimeError> {
        loop {
            let operation = self.0.deferred_references.borrow_mut().pop_front();
            let Some(operation) = operation else {
                return Ok(());
            };
            let Ok(mut state) = self.0.state.try_borrow_mut() else {
                self.0
                    .deferred_references
                    .borrow_mut()
                    .push_front(operation);
                return Ok(());
            };
            match operation {
                DeferredRefOp::Object(object) => {
                    let cleanup = state.heap.release_object(object)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::Context(context) => {
                    let cleanup = state.heap.release_context(context)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::FunctionBytecode(bytecode) => {
                    let cleanup = state.heap.release_function_bytecode(bytecode)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::VarRef(var_ref) => {
                    let cleanup = state.heap.release_var_ref(var_ref)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::Atom(atom) => {
                    state.atoms.release(atom)?;
                }
                DeferredRefOp::ActiveFramePop { token, depth } => {
                    if let Some(position) = state
                        .active_frames
                        .iter()
                        .rposition(|frame| frame.token == token)
                    {
                        state.active_frames.truncate(position);
                    } else if state.active_frames.len() > depth {
                        state.active_frames.truncate(depth);
                    }
                }
                DeferredRefOp::ActiveCollectionRecordsTruncate { depth } => {
                    state.active_collection_records.truncate(depth);
                }
                DeferredRefOp::BacktraceBarrierRestore { token, previous } => {
                    if let Some(frame) = state
                        .active_frames
                        .iter_mut()
                        .find(|frame| frame.token == token)
                    {
                        frame.flags.backtrace_barrier = previous;
                    }
                }
            }
        }
    }

    pub(crate) fn retain_object_handle(&self, id: ObjectId) -> Result<(), HeapError> {
        let mut state = self.0.state.try_borrow_mut().map_err(|_| {
            HeapError::Invariant("object root retained during a runtime state borrow")
        })?;
        state.heap.retain_object(id)
    }

    pub(crate) fn release_object_handle(&self, id: ObjectId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state.heap.release_object(id).map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::Object(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid object root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred object release failed: {drain:?}");
    }

    pub(crate) fn retain_atom_handle(&self, atom: Atom) -> Result<(), AtomError> {
        self.0.state.borrow_mut().atoms.retain(atom).map(drop)
    }

    pub(crate) fn retain_function_bytecode_handle(
        &self,
        id: FunctionBytecodeId,
    ) -> Result<(), HeapError> {
        let mut state = self.0.state.try_borrow_mut().map_err(|_| {
            HeapError::Invariant("function bytecode retained during a runtime state borrow")
        })?;
        state.heap.retain_function_bytecode(id)
    }

    pub(crate) fn retain_context_handle(&self, id: ContextId) -> Result<(), HeapError> {
        let mut state =
            self.0.state.try_borrow_mut().map_err(|_| {
                HeapError::Invariant("context retained during a runtime state borrow")
            })?;
        state.heap.retain_context(id)
    }

    pub(crate) fn release_context_handle(&self, id: ContextId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state.heap.release_context(id).map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::Context(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid context root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred context release failed: {drain:?}");
    }

    pub(crate) fn release_function_bytecode_handle(&self, id: FunctionBytecodeId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state
                .heap
                .release_function_bytecode(id)
                .map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::FunctionBytecode(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid bytecode root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred bytecode release failed: {drain:?}");
    }

    pub(crate) fn retain_var_ref_handle(&self, id: VarRefId) -> Result<(), HeapError> {
        let mut state =
            self.0.state.try_borrow_mut().map_err(|_| {
                HeapError::Invariant("VarRef retained during a runtime state borrow")
            })?;
        state.heap.retain_var_ref(id)
    }

    pub(crate) fn release_var_ref_handle(&self, id: VarRefId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state.heap.release_var_ref(id).map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::VarRef(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid VarRef root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred VarRef release failed: {drain:?}");
    }

    pub(crate) fn release_atom_handle(&self, atom: Atom) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            state.atoms.release(atom).map(drop)
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::Atom(atom));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid atom root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred atom release failed: {drain:?}");
    }
}
