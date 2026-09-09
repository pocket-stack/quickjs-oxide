use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::heap::runtime::RuntimeState;

use crate::engine::heap::{GcStats, HeapCounts, WeakSymbolGcEvent};
use crate::engine::jobs;
#[cfg(feature = "test262-host")]
use crate::engine::value::Value;
#[cfg(feature = "test262-host")]
use crate::engine::vm::Completion;
#[cfg(feature = "test262-host")]
use crate::engine::vm::call::NativeInvocation;

impl Runtime {
    /// Run QuickJS-style cycle collection for this runtime.
    pub fn run_gc(&self) -> Result<GcStats, RuntimeError> {
        let _operation = self.operation();
        let mut state = self.0.state.borrow_mut();
        let mut atom_error = None;
        let mut stats = {
            let RuntimeState {
                atoms,
                heap,
                pending_jobs,
                ..
            } = &mut *state;
            let mut finalization_sink = jobs::RuntimeFinalizationJobSink::new(pending_jobs);
            heap.run_gc_with_finalization_sink(
                |event| {
                    Ok(match event {
                        WeakSymbolGcEvent::IsLive(atom) => atoms.is_live(atom),
                        WeakSymbolGcEvent::Release(atom) => {
                            if let Err(error) = atoms.release(atom) {
                                // A detached weak value owned this atom, so this
                                // can fail only after an ownership invariant has
                                // already been violated. Latch the exact error but
                                // continue without scheduling a double release.
                                atom_error.get_or_insert(error);
                            }
                            true
                        }
                    })
                },
                &mut finalization_sink,
            )?
        };
        if let Some(error) = atom_error {
            return Err(error.into());
        }
        let atoms = std::mem::take(&mut stats.cleanup.atoms);
        state.unlink_finalized_shapes(stats.cleanup.finalized_shape_ids.iter().copied());
        state.release_atoms(atoms)?;
        state.atoms.sweep_released_strings();
        Ok(stats)
    }

    /// Execute QuickJS's test262-only `js_gc` host callback.
    ///
    /// The callback runs collection synchronously on the current runtime and
    /// deliberately does not drain the pending-job queue. Active JavaScript
    /// and native frames keep their ordinary stack-owned roots while the
    /// collector runs.
    #[cfg(feature = "test262-host")]
    pub(crate) fn call_test262_gc(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Test262 gc received a constructor invocation",
            ));
        };
        self.run_gc()?;
        Ok(Completion::Return(Value::Undefined))
    }

    /// Runtime heap population for diagnostics and lifecycle tests.
    #[must_use]
    pub fn heap_counts(&self) -> HeapCounts {
        let _operation = self.operation();
        self.0.state.borrow().heap.counts()
    }
}
