//! Runtime creation, identity, and host configuration.

use crate::engine::atom::AtomTable;
use crate::engine::code::debug::DebugInfoMode;
use crate::engine::heap::Heap;

use crate::engine::heap::runtime::{NEXT_RUNTIME_DOMAIN_ID, RuntimeInner, RuntimeState};
use crate::engine::host::HostServices;
use crate::engine::object::WellKnownSymbol;
use crate::engine::vm::host_bridge as vm_host;
#[cfg(test)]
use quickjs_oxide_host::SystemHostServices;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::Ordering;

impl Runtime {
    #[must_use]
    #[cfg(test)]
    pub fn new() -> Self {
        Self::new_with_host_services(SystemHostServices::default())
    }

    /// Create a runtime with embedder-provided clock, time-zone, and random
    /// seed services.
    ///
    /// The services are called synchronously and retained for the runtime's
    /// lifetime. The application selects the concrete provider at this embedding boundary.
    #[must_use]
    pub fn new_with_host_services(host_services: impl HostServices + 'static) -> Self {
        Self::new_configured(
            host_services,
            #[cfg(feature = "profiling")]
            None,
        )
    }

    fn new_configured(
        host_services: impl HostServices + 'static,
        #[cfg(feature = "profiling")] trace: Option<super::profiling::AllocationTrace>,
    ) -> Self {
        let host_services: Rc<dyn HostServices> = Rc::new(host_services);
        let domain_id = NEXT_RUNTIME_DOMAIN_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(domain_id, 0, "runtime domain ID space exhausted");
        let mut atoms = AtomTable::with_static_atoms(vm_host::TYPEOF_STATIC_ATOMS)
            .expect("fixed typeof atom set fits the atom table");
        let mut well_known_symbols = HashMap::new();
        for symbol in WellKnownSymbol::ALL {
            let atom = atoms
                .new_static_symbol(Some(symbol.description()))
                .expect("fixed well-known symbol set fits the atom table");
            well_known_symbols.insert(symbol, atom);
        }
        Self(Rc::new(RuntimeInner {
            state: RefCell::new(RuntimeState {
                atoms,
                heap: {
                    #[cfg(feature = "profiling")]
                    {
                        Heap::with_allocation_trace(trace)
                    }
                    #[cfg(not(feature = "profiling"))]
                    Heap::new()
                },
                pending_exception: None,
                pending_jobs: VecDeque::new(),
                debug_info_mode: DebugInfoMode::Full,
                shape_cache: HashMap::new(),
                shape_fingerprints: HashMap::new(),
                well_known_symbols,
                active_frames: Vec::new(),
                active_collection_records: Vec::new(),
                next_active_frame_token: 1,
                next_module_async_evaluation_order: 0,
                #[cfg(test)]
                active_frame_probe_snapshots: Vec::new(),
                #[cfg(test)]
                iterator_result_allocations: 0,
            }),
            deferred_references: RefCell::new(VecDeque::new()),
            host_services,
            can_block: Cell::new(false),
            promise_rejection_tracker: RefCell::new(None),
            module_loader: RefCell::new(None),
            #[cfg(feature = "test262-host")]
            dynamic_import_bytecode_allowed: Cell::new(true),
            module_host_callback_depth: Cell::new(0),
            host_stack_top: Cell::new(None),
            proxy_method_depth: Cell::new(0),
            next_context_id: Cell::new(0),
            domain_id,
        }))
    }

    /// Start a bounded, partial allocation trace before runtime initialization.
    /// The returned handle owns diagnostic records, never runtime/JS roots.
    /// Read it after dropping every context, value and runtime handle to include
    /// teardown. See `AllocationTrace` for the exact coverage contract.
    #[cfg(feature = "profiling")]
    #[must_use]
    pub fn new_with_allocation_trace(
        host_services: impl HostServices + 'static,
        max_events: usize,
    ) -> (Self, super::profiling::AllocationTrace) {
        let trace = super::profiling::AllocationTrace::new(max_events);
        let runtime = Self::new_configured(host_services, Some(trace.clone()));
        trace.set_runtime_id(runtime.domain_id());
        (runtime, trace)
    }

    /// Set the runtime-wide debug information policy for future compilations.
    /// Existing bytecode is immutable and keeps the mode used when published.
    pub fn set_debug_info_mode(&self, mode: DebugInfoMode) {
        self.0.state.borrow_mut().debug_info_mode = mode;
    }

    /// Return the policy which the next compilation will sample.
    #[must_use]
    pub fn debug_info_mode(&self) -> DebugInfoMode {
        self.0.state.borrow().debug_info_mode
    }

    /// Set whether this runtime's host permits synchronous blocking operations.
    ///
    /// The setting is runtime-wide, so cloned handles and every context owned
    /// by this runtime observe the same value. As in QuickJS, new runtimes
    /// default to `false` and embedders must opt in explicitly.
    pub fn set_can_block(&self, can_block: bool) {
        self.0.can_block.set(can_block);
    }

    /// Return whether this runtime's host permits synchronous blocking.
    #[must_use]
    pub fn can_block(&self) -> bool {
        self.0.can_block.get()
    }

    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Stable identity used by rooted handle hashing and diagnostics.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.0.domain_id
    }
}

/// A single-threaded QuickJS-compatible runtime.
///
/// Cloning this handle does not clone the runtime; it creates another owner of
/// the same heap/atom domain so multiple contexts can share runtime resources.
#[derive(Clone)]
pub struct Runtime(pub(crate) Rc<RuntimeInner>);

#[cfg(test)]
impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}
