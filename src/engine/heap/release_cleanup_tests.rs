use super::deferred::DeferredOperations;
use super::runtime::DeferredRefOp;
use crate::engine::api::Runtime;
use crate::engine::value::Value;

fn restoration(depth: usize) -> DeferredRefOp {
    DeferredRefOp::ActiveCollectionRecordsTruncate { depth }
}

fn restoration_depth(operation: Option<DeferredRefOp>) -> usize {
    let Some(DeferredRefOp::ActiveCollectionRecordsTruncate { depth }) = operation else {
        panic!("missing restoration operation");
    };
    depth
}

#[test]
fn deferred_queue_keeps_priority_and_work_enqueued_during_a_drain() {
    let queue = DeferredOperations::default();
    assert!(!queue.has_pending());
    queue.push_back(restoration(1));
    queue.push_back(restoration(2));
    queue.push_front(restoration(3));
    let guard = queue.try_start_draining().unwrap();
    assert!(queue.try_start_draining().is_none());
    assert_eq!(restoration_depth(queue.pop_front()), 3);
    // Work created while applying the first operation must retain front priority.
    queue.push_front(restoration(4));
    assert_eq!(restoration_depth(queue.pop_front()), 4);
    assert_eq!(restoration_depth(queue.pop_front()), 1);
    assert_eq!(restoration_depth(queue.pop_front()), 2);
    assert!(!queue.has_pending());
    // Popping the last item does not finish the active drainer: its application
    // may still create more work before the guard is dropped.
    queue.push_back(restoration(5));
    assert!(queue.has_pending());
    assert!(queue.try_start_draining().is_none());
    assert_eq!(restoration_depth(queue.pop_front()), 5);
    drop(guard);
    assert!(!queue.has_pending());
    assert!(queue.try_start_draining().is_some());
}

#[test]
fn deferred_drain_guard_resets_after_unwinding_without_losing_pending_work() {
    let queue = DeferredOperations::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = queue.try_start_draining().unwrap();
        queue.push_back(restoration(7));
        panic!("simulated unwind during cleanup");
    }));
    assert!(result.is_err());
    assert!(queue.has_pending());
    let _guard = queue.try_start_draining().unwrap();
    assert_eq!(restoration_depth(queue.pop_front()), 7);
    assert!(!queue.has_pending());
}

#[test]
fn idle_and_borrow_blocked_checkpoints_leave_the_queue_untouched() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let id = object.object_id();
    let state = runtime.0.state.borrow_mut();
    {
        let _queue_read = runtime.0.deferred_references.borrow();
        // Both borrows deliberately forbid a mutable borrow in the idle path.
        runtime.drain_deferred_references().unwrap();
    }
    drop(object);
    assert!(runtime.0.deferred_references.has_pending());
    {
        let queue_read = runtime.0.deferred_references.borrow();
        assert_eq!(queue_read.len(), 1);
        runtime.drain_deferred_references().unwrap();
        assert_eq!(queue_read.len(), 1);
    }
    assert!(state.heap.object(id).is_ok());
    drop(state);
    // The existing operation boundary, including nested boundaries, still drains.
    let _operation = runtime.operation();
    assert!(!runtime.0.deferred_references.has_pending());
    assert!(runtime.0.state.borrow().heap.object(id).is_err());
}

#[test]
fn failed_deferred_operation_releases_the_guard_and_keeps_remaining_work() {
    let runtime = Runtime::new();
    let stale = runtime.new_object(None).unwrap();
    let stale_id = stale.object_id();
    drop(stale);
    let live = runtime.new_object(None).unwrap();
    let live_id = live.object_id();
    runtime
        .0
        .deferred_references
        .push_back(DeferredRefOp::Object(stale_id));
    let state = runtime.0.state.borrow();
    drop(live);
    drop(state);
    assert!(runtime.drain_deferred_references().is_err());
    assert!(runtime.0.deferred_references.has_pending());
    assert!(runtime.0.state.borrow().heap.object(live_id).is_ok());
    runtime.drain_deferred_references().unwrap();
    assert!(!runtime.0.deferred_references.has_pending());
    assert!(runtime.0.state.borrow().heap.object(live_id).is_err());
}

#[test]
fn cascading_zero_reference_destruction_finishes_before_release_returns() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    // Warm the ordinary object path before recording the persistent baseline.
    drop(context.eval("({ next: null })").unwrap());
    let before = runtime.0.state.borrow().heap.counts().object_nodes;
    let root = context
        .eval("(() => { let root = null; for (let i = 0; i < 10000; i++) root = { next: root }; return root; })()")
        .unwrap();
    assert!(matches!(root, Value::Object(_)));
    assert_eq!(
        runtime.0.state.borrow().heap.counts().object_nodes,
        before + 10000
    );
    drop(root);
    assert_eq!(runtime.0.state.borrow().heap.counts().object_nodes, before);
    assert!(!runtime.0.deferred_references.has_pending());
}

#[test]
fn runtime_teardown_applies_queued_bytecode_context_and_atom_releases() {
    let runtime = Runtime::new();
    let weak = std::rc::Rc::downgrade(&runtime.0);
    let mut context = runtime.new_context();
    let bytecode = context.compile("({ value: 42 })").unwrap();
    let key = runtime.intern_property_key("queued_at_teardown").unwrap();
    let state = runtime.0.state.borrow_mut();
    drop(bytecode);
    drop(context);
    drop(key);
    assert!(runtime.0.deferred_references.has_pending());
    drop(state);
    // No intervening safe point: RuntimeInner::drop must use the same operation
    // application rules and satisfy its zero-live-node teardown assertion.
    drop(runtime);
    assert!(weak.upgrade().is_none());
}

#[test]
fn live_heap_reference_release_has_no_runtime_cleanup_payload() {
    use super::{Heap, ObjectData, RawId};
    use crate::engine::object::shape::Shape;

    let mut heap = Heap::new();
    let shape = heap.allocate_shape(Shape::new(None, []).unwrap()).unwrap();
    let object = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    heap.release_shape(shape).unwrap();
    heap.retain_object(object).unwrap();
    assert!(
        heap.release_reference(RawId::Object(object))
            .unwrap()
            .is_none()
    );
    assert_eq!(heap.object_strong_count(object).unwrap(), 1);
    let cleanup = heap
        .release_reference(RawId::Object(object))
        .unwrap()
        .unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
    assert!(heap.release_reference(RawId::Object(object)).is_err());
}

#[test]
fn nonzero_release_still_drains_previously_queued_nodes() {
    use super::{Heap, ObjectData, RawId};
    use crate::engine::object::shape::Shape;

    let mut heap = Heap::new();
    let shape = heap.allocate_shape(Shape::new(None, []).unwrap()).unwrap();
    let queued = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    let retained = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    heap.release_shape(shape).unwrap();
    heap.retain_object(retained).unwrap();
    heap.release_raw_no_drain(RawId::Object(queued)).unwrap();
    assert!(!heap.zero_queue.is_empty());
    let cleanup = heap
        .release_reference(RawId::Object(retained))
        .unwrap()
        .unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 0);
    assert!(heap.object(queued).is_err());
    assert_eq!(heap.object_strong_count(retained).unwrap(), 1);
    assert!(heap.zero_queue.is_empty());
    // The existing public API still returns the full cleanup counters.
    let cleanup = heap.release_object(retained).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}
