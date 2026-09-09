//! Optional, read-only runtime diagnostics. No JavaScript code or GC is run.
//!
//! Byte accounting is deliberately partial: owned storage is measured by its
//! owner, shared code slices are deduplicated, and unknown totals stay unknown.
//! Allocation tracing observes the arena Vec's backing-storage transitions,
//! not JS object creation and not a process-wide malloc interceptor.

use super::Runtime;
use crate::engine::heap::HeapCounts;
use std::cell::RefCell;
use std::rc::Rc;

/// One disjoint storage category, except logical-only node categories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryCategory {
    pub name: &'static str,
    pub count: Option<usize>,
    /// Initialized inline storage only; excludes recursively owned resources.
    pub used_bytes: Option<usize>,
    /// Reserved inline storage only; excludes allocator bookkeeping.
    pub capacity_bytes: Option<usize>,
    pub basis: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub runtime_id: u64,
    pub heap: HeapCounts,
    pub pending_jobs: usize,
    pub categories: Vec<MemoryCategory>,
}

impl Runtime {
    /// Observe the current state without draining deferred releases, running
    /// jobs or invoking GC. The caller determines and labels the snapshot phase.
    #[must_use]
    pub fn memory_snapshot(&self) -> MemorySnapshot {
        let state = self.0.state.borrow();
        let mut categories = state.heap.memory_categories();
        categories.push(MemoryCategory {
            name: "atoms",
            count: Some(state.atoms.len()),
            used_bytes: None,
            capacity_bytes: None,
            basis: "live-table-entries-excluding-immediate-integers; shared-string-bytes-unavailable",
        });
        categories.push(MemoryCategory {
            name: "strings",
            count: None,
            used_bytes: None,
            capacity_bytes: None,
            basis: "unavailable: strings may be shared with atoms, bytecode and embedder values",
        });
        MemorySnapshot {
            runtime_id: self.domain_id(),
            heap: state.heap.counts(),
            pending_jobs: state.pending_jobs.len(),
            categories,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationEventKind {
    Allocate,
    Reallocate,
    Free,
}

/// One observed arena backing-storage transition. `allocation_id` identifies
/// the storage lifetime across growth; no physical addresses are exposed.
/// Capacity is Vec capacity * element size, not malloc usable/requested size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocationEvent {
    pub sequence: u64,
    pub allocation_id: u64,
    pub kind: AllocationEventKind,
    pub old_capacity_bytes: usize,
    pub capacity_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct AllocationTraceSnapshot {
    pub runtime_id: u64,
    pub events: Vec<AllocationEvent>,
    pub dropped_events: u64,
    pub event_limit: usize,
    pub requested_event_limit: usize,
    pub buffer_allocation_failed: bool,
    pub finished: bool,
}

struct TraceState {
    runtime_id: u64,
    events: Vec<AllocationEvent>,
    sequence: u64,
    dropped_events: u64,
    event_limit: usize,
    requested_event_limit: usize,
    buffer_allocation_failed: bool,
    finished: bool,
}

/// Bounded in-memory trace. Collection performs no formatting, I/O, callback,
/// or buffer growth. Failures to reserve the diagnostic buffer reduce its
/// effective event limit to zero; lost events are counted explicitly.
///
/// Coverage is **partial**: only successful arena Vec backing allocation,
/// growth and release are observable in safe Rust here. Allocator requests,
/// allocation failures (which may abort), physical movement, nested payload
/// allocations, atom storage, and process RSS are unavailable. A reallocation
/// event means the Vec's capacity changed, not that libc realloc was called.
#[derive(Clone)]
pub struct AllocationTrace(Rc<RefCell<TraceState>>);

impl AllocationTrace {
    pub(crate) fn new(event_limit: usize) -> Self {
        let mut events = Vec::new();
        let requested_event_limit = event_limit;
        let event_limit = if events.try_reserve_exact(event_limit).is_ok() {
            event_limit
        } else {
            0
        };
        Self(Rc::new(RefCell::new(TraceState {
            runtime_id: 0,
            events,
            sequence: 0,
            dropped_events: 0,
            event_limit,
            requested_event_limit,
            buffer_allocation_failed: event_limit != requested_event_limit,
            finished: false,
        })))
    }

    pub(crate) fn set_runtime_id(&self, id: u64) {
        self.0.borrow_mut().runtime_id = id;
    }

    pub(crate) fn record(&self, old_capacity_bytes: usize, capacity_bytes: usize) {
        if old_capacity_bytes == capacity_bytes {
            return;
        }
        let mut state = self.0.borrow_mut();
        state.sequence = state.sequence.saturating_add(1);
        if state.events.len() == state.event_limit {
            state.dropped_events = state.dropped_events.saturating_add(1);
            return;
        }
        let event = AllocationEvent {
            sequence: state.sequence,
            allocation_id: 1,
            kind: if old_capacity_bytes == 0 {
                AllocationEventKind::Allocate
            } else if capacity_bytes == 0 {
                AllocationEventKind::Free
            } else {
                AllocationEventKind::Reallocate
            },
            old_capacity_bytes,
            capacity_bytes,
        };
        state.events.push(event);
    }

    pub(crate) fn finish(&self) {
        self.0.borrow_mut().finished = true;
    }

    /// Copy diagnostics after execution, outside the measured hot path.
    #[must_use]
    pub fn snapshot(&self) -> AllocationTraceSnapshot {
        let state = self.0.borrow();
        AllocationTraceSnapshot {
            runtime_id: state.runtime_id,
            events: state.events.clone(),
            dropped_events: state.dropped_events,
            event_limit: state.event_limit,
            requested_event_limit: state.requested_event_limit,
            buffer_allocation_failed: state.buffer_allocation_failed,
            finished: state.finished,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickjs_oxide_host::SystemHostServices;

    fn category<'a>(snapshot: &'a MemorySnapshot, name: &str) -> &'a MemoryCategory {
        snapshot.categories.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn profiling_snapshot_is_read_only_and_counts_owned_storage_once() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.eval("globalThis.buffer = new ArrayBuffer(4096); globalThis.alias = buffer; globalThis.view = new Uint8Array(buffer); globalThis.array = [1, 2, 3]; globalThis.ran = 0; Object.defineProperty(globalThis, 'trap', {get() { throw 42; }}); Promise.resolve().then(() => ran++);").unwrap();
        let first = runtime.memory_snapshot();
        let second = runtime.memory_snapshot();
        assert_eq!(first, second);
        assert_eq!(
            category(&first, "array_buffer_bytes").used_bytes,
            Some(4096)
        );
        assert_eq!(category(&first, "strings").used_bytes, None);
        assert!(first.pending_jobs > 0);
        assert!(matches!(
            context.eval("ran").unwrap(),
            super::super::Value::Int(0)
        ));
        drop(context);
        runtime.run_gc().unwrap();
        // A pending job can retain its realm; taking a snapshot must not hide it.
        assert!(runtime.memory_snapshot().pending_jobs > 0);
    }

    #[test]
    fn profiling_trace_preserves_storage_lifetime_through_cloned_runtime() {
        let (runtime, trace) =
            Runtime::new_with_allocation_trace(SystemHostServices::default(), 128);
        let mut context = runtime.new_context();
        context
            .eval("globalThis.items = []; for (let i = 0; i < 200; i++) items.push({i});")
            .unwrap();
        let alias = runtime.clone();
        let during = trace.snapshot();
        assert!(!during.finished);
        assert_eq!(during.dropped_events, 0);
        assert_eq!(during.events[0].kind, AllocationEventKind::Allocate);
        assert!(
            during
                .events
                .iter()
                .any(|e| e.kind == AllocationEventKind::Reallocate)
        );
        drop(context);
        drop(runtime);
        assert!(!trace.snapshot().finished);
        drop(alias);
        let final_trace = trace.snapshot();
        assert!(final_trace.finished);
        assert_eq!(
            final_trace.events.last().unwrap().kind,
            AllocationEventKind::Free
        );
        let mut capacity = 0;
        for (index, event) in final_trace.events.iter().enumerate() {
            assert_eq!(event.sequence, index as u64 + 1);
            assert_eq!(event.old_capacity_bytes, capacity);
            capacity = event.capacity_bytes;
        }
        assert_eq!(capacity, 0);
    }

    #[test]
    fn profiling_trace_overflow_and_reservation_failure_are_explicit() {
        for limit in [0, 1, usize::MAX] {
            let (runtime, trace) =
                Runtime::new_with_allocation_trace(SystemHostServices::default(), limit);
            let context = runtime.new_context();
            drop(context);
            drop(runtime);
            let trace = trace.snapshot();
            assert!(trace.finished);
            assert!(trace.dropped_events > 0);
            assert!(trace.events.len() <= trace.event_limit);
            assert!(trace.event_limit <= 1);
            assert_eq!(trace.requested_event_limit, limit);
            assert_eq!(trace.buffer_allocation_failed, limit == usize::MAX);
        }
    }

    #[test]
    fn profiling_snapshot_shares_runtime_but_does_not_retain_contexts() {
        let runtime = Runtime::new();
        let left = runtime.new_context();
        let right = runtime.new_context();
        let before = runtime.memory_snapshot();
        assert!(before.heap.context_nodes >= 2);
        drop(left);
        drop(right);
        runtime.run_gc().unwrap();
        assert_eq!(runtime.memory_snapshot().heap.context_nodes, 0);
        // The saved diagnostic consists only of counts, not arena roots.
        assert!(before.heap.context_nodes >= 2);
    }
}
