use crate::engine::heap::native::{
    FinalizationRegistryNativeKind, NativeCProto, WeakMapNativeKind, WeakRefNativeKind,
    WeakSetNativeKind,
};

use super::*;

#[test]
fn weak_ref_target_is_non_owning_and_cleared_by_the_weak_pass() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let target = leaf(&mut heap, shape);
    let weak_target = WeakCollectionKey::Object(target);
    let weak_ref = heap
        .allocate_weak_ref_object(shape, Vec::new(), weak_target)
        .unwrap();

    assert_eq!(heap.object_strong_count(target), Ok(1));
    assert_eq!(heap.weak_ref_target(weak_ref), Ok(Some(weak_target)));
    assert_eq!(heap.release_object(target).unwrap().finalized_objects, 1);
    // The generational identity is cleared only by the ordered weak pass.
    assert_eq!(heap.weak_ref_target(weak_ref), Ok(Some(weak_target)));

    collect_heap(&mut heap).unwrap();
    assert_eq!(heap.weak_ref_target(weak_ref), Ok(None));

    heap.release_object(weak_ref).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn finalization_registry_transfers_held_and_job_roots_without_retain_on_adoption() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let (root, function_prototype, realm, callback) = finalization_test_realm(&mut heap, shape);
    let registry = heap
        .allocate_finalization_registry_object(shape, Vec::new(), callback, realm)
        .unwrap();
    let target = leaf(&mut heap, shape);
    let held = leaf(&mut heap, shape);
    heap.finalization_registry_register(
        registry,
        WeakCollectionKey::Object(target),
        RawValue::Object(held),
        None,
    )
    .unwrap();
    assert_eq!(heap.object_strong_count(held), Ok(2));
    heap.release_object(held).unwrap();
    heap.release_object(target).unwrap();

    let callback_before = heap.object_strong_count(callback).unwrap();
    let realm_before = heap.context_strong_count(realm).unwrap();
    let mut sink = RecordingFinalizationJobSink::default();
    heap.run_gc_with_finalization_sink(
        |event| Ok(matches!(event, WeakSymbolGcEvent::IsLive(_))),
        &mut sink,
    )
    .unwrap();

    assert_eq!(heap.finalization_registry_len(registry), Ok(0));
    assert_eq!(sink.jobs.len(), 1);
    assert_eq!(heap.object_strong_count(held), Ok(1));
    assert_eq!(heap.object_strong_count(callback), Ok(callback_before + 1));
    assert_eq!(heap.context_strong_count(realm), Ok(realm_before + 1));
    let job = sink.jobs.front().unwrap();
    assert_eq!(job.callback, callback);
    assert_eq!(job.realm, realm);
    assert_eq!(job.held_value, RawValue::Object(held));

    // Releasing the registry removes only its own callback/realm roots;
    // the already-published job keeps its roots without another retain.
    heap.release_object(registry).unwrap();
    assert_eq!(heap.object_strong_count(callback), Ok(callback_before));
    assert_eq!(heap.context_strong_count(realm), Ok(realm_before));
    let cleanup = heap.discard_finalization_jobs(sink.jobs).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert!(matches!(heap.object(held), Err(HeapError::Stale { .. })));

    release_finalization_test_realm(&mut heap, shape, root, function_prototype, realm, callback);
}

#[test]
fn finalization_job_moves_held_symbol_ownership_until_job_release() {
    let mut atoms = crate::engine::atom::AtomTable::new();
    let held = atoms.new_symbol(Some("finalization held value")).unwrap();
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let (root, function_prototype, realm, callback) = finalization_test_realm(&mut heap, shape);
    let registry = heap
        .allocate_finalization_registry_object(shape, Vec::new(), callback, realm)
        .unwrap();
    let target = leaf(&mut heap, shape);
    heap.finalization_registry_register(
        registry,
        WeakCollectionKey::Object(target),
        RawValue::Symbol(held),
        None,
    )
    .unwrap();
    heap.release_object(target).unwrap();

    let mut sink = RecordingFinalizationJobSink::default();
    heap.run_gc_with_finalization_sink(
        |event| {
            Ok(match event {
                WeakSymbolGcEvent::IsLive(atom) => atoms.is_live(atom),
                WeakSymbolGcEvent::Release(atom) => {
                    atoms
                        .release(atom)
                        .map_err(|_| HeapError::Invariant("held Symbol release failed"))?;
                    true
                }
            })
        },
        &mut sink,
    )
    .unwrap();
    assert_eq!(sink.jobs.len(), 1);
    assert_eq!(sink.jobs[0].held_value, RawValue::Symbol(held));
    assert!(atoms.is_live(held));

    heap.release_object(registry).unwrap();
    let cleanup = heap.discard_finalization_jobs(sink.jobs).unwrap();
    assert_eq!(cleanup.atoms, [held]);
    assert_eq!(
        atoms.release(held),
        Ok(crate::engine::atom::ReleaseOutcome::Removed)
    );
    release_finalization_test_realm(&mut heap, shape, root, function_prototype, realm, callback);
}

#[test]
fn weak_pass_can_prepare_a_later_zero_queued_registry_before_finalizing_it() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let (root, function_prototype, realm, callback) = finalization_test_realm(&mut heap, shape);
    // Construction order matters: the map releases the registry before
    // the same mixed-list pass reaches the registry.
    let weak_map = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let registry = heap
        .allocate_finalization_registry_object(shape, Vec::new(), callback, realm)
        .unwrap();
    let map_key = leaf(&mut heap, shape);
    let target = leaf(&mut heap, shape);
    let held = leaf(&mut heap, shape);
    heap.weak_map_set(
        weak_map,
        WeakCollectionKey::Object(map_key),
        RawValue::Object(registry),
    )
    .unwrap();
    heap.finalization_registry_register(
        registry,
        WeakCollectionKey::Object(target),
        RawValue::Object(held),
        None,
    )
    .unwrap();
    heap.release_object(map_key).unwrap();
    heap.release_object(target).unwrap();
    heap.release_object(held).unwrap();
    heap.release_object(registry).unwrap();

    let mut sink = RecordingFinalizationJobSink::default();
    let stats = heap
        .run_gc_with_finalization_sink(
            |event| Ok(matches!(event, WeakSymbolGcEvent::IsLive(_))),
            &mut sink,
        )
        .unwrap();
    assert_eq!(sink.jobs.len(), 1);
    assert!(stats.cleanup.finalized_objects >= 1);
    assert!(matches!(
        heap.object(registry),
        Err(HeapError::Stale { .. })
    ));
    assert_eq!(heap.object_strong_count(held), Ok(1));

    assert_eq!(
        heap.discard_finalization_jobs(sink.jobs)
            .unwrap()
            .finalized_objects,
        1
    );
    heap.release_object(weak_map).unwrap();
    release_finalization_test_realm(&mut heap, shape, root, function_prototype, realm, callback);
}

#[test]
fn mixed_weak_object_construction_order_controls_one_vs_two_pass_clear() {
    for weak_ref_first in [false, true] {
        let mut heap = Heap::new();
        let shape = empty_shape(&mut heap);
        let target = leaf(&mut heap, shape);
        let key = leaf(&mut heap, shape);
        let weak_target = WeakCollectionKey::Object(target);
        let weak_key = WeakCollectionKey::Object(key);

        let (weak_map, weak_ref) = if weak_ref_first {
            let weak_ref = heap
                .allocate_weak_ref_object(shape, Vec::new(), weak_target)
                .unwrap();
            let weak_map = heap
                .allocate_object(ObjectData::weak_map(shape, Vec::new()))
                .unwrap();
            (weak_map, weak_ref)
        } else {
            let weak_map = heap
                .allocate_object(ObjectData::weak_map(shape, Vec::new()))
                .unwrap();
            let weak_ref = heap
                .allocate_weak_ref_object(shape, Vec::new(), weak_target)
                .unwrap();
            (weak_map, weak_ref)
        };
        heap.weak_map_set(weak_map, weak_key, RawValue::Object(target))
            .unwrap();
        heap.release_object(key).unwrap();
        heap.release_object(target).unwrap();

        collect_heap(&mut heap).unwrap();
        if weak_ref_first {
            assert_eq!(heap.weak_ref_target(weak_ref), Ok(Some(weak_target)));
            collect_heap(&mut heap).unwrap();
        }
        assert_eq!(heap.weak_ref_target(weak_ref), Ok(None));

        heap.release_object(weak_ref).unwrap();
        heap.release_object(weak_map).unwrap();
        heap.release_shape(shape).unwrap();
        assert_eq!(heap.counts().live, 0);
    }
}

#[test]
fn finalization_job_reservation_failure_silently_drops_the_registration() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let (root, function_prototype, realm, callback) = finalization_test_realm(&mut heap, shape);
    let registry = heap
        .allocate_finalization_registry_object(shape, Vec::new(), callback, realm)
        .unwrap();
    let target = leaf(&mut heap, shape);
    let held = leaf(&mut heap, shape);
    heap.finalization_registry_register(
        registry,
        WeakCollectionKey::Object(target),
        RawValue::Object(held),
        None,
    )
    .unwrap();
    heap.release_object(target).unwrap();
    heap.release_object(held).unwrap();
    let callback_before = heap.object_strong_count(callback).unwrap();
    let realm_before = heap.context_strong_count(realm).unwrap();

    let mut sink = RecordingFinalizationJobSink {
        fail_next_reservation: true,
        ..RecordingFinalizationJobSink::default()
    };
    let stats = heap
        .run_gc_with_finalization_sink(
            |event| Ok(matches!(event, WeakSymbolGcEvent::IsLive(_))),
            &mut sink,
        )
        .unwrap();
    assert!(sink.jobs.is_empty());
    assert_eq!(heap.finalization_registry_len(registry), Ok(0));
    assert_eq!(stats.cleanup.finalized_objects, 1);
    assert!(matches!(heap.object(held), Err(HeapError::Stale { .. })));
    assert_eq!(heap.object_strong_count(callback), Ok(callback_before));
    assert_eq!(heap.context_strong_count(realm), Ok(realm_before));

    heap.release_object(registry).unwrap();
    release_finalization_test_realm(&mut heap, shape, root, function_prototype, realm, callback);
}

#[test]
fn runtime_teardown_gc_skips_weak_removal_from_the_start() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let (root, function_prototype, realm, callback) = finalization_test_realm(&mut heap, shape);
    let target = leaf(&mut heap, shape);
    let weak_target = WeakCollectionKey::Object(target);
    let weak_ref = heap
        .allocate_weak_ref_object(shape, Vec::new(), weak_target)
        .unwrap();
    let registry = heap
        .allocate_finalization_registry_object(shape, Vec::new(), callback, realm)
        .unwrap();
    let held = leaf(&mut heap, shape);
    heap.finalization_registry_register(registry, weak_target, RawValue::Object(held), None)
        .unwrap();
    heap.release_object(target).unwrap();
    heap.release_object(held).unwrap();

    heap.run_gc_for_runtime_teardown().unwrap();
    assert_eq!(heap.weak_ref_target(weak_ref), Ok(Some(weak_target)));
    assert_eq!(heap.finalization_registry_len(registry), Ok(1));
    assert_eq!(heap.object_strong_count(held), Ok(1));

    // A normal heap-only collection takes the no-queue drop path.
    collect_heap(&mut heap).unwrap();
    assert_eq!(heap.weak_ref_target(weak_ref), Ok(None));
    assert_eq!(heap.finalization_registry_len(registry), Ok(0));
    assert!(matches!(heap.object(held), Err(HeapError::Stale { .. })));

    heap.release_object(weak_ref).unwrap();
    heap.release_object(registry).unwrap();
    release_finalization_test_realm(&mut heap, shape, root, function_prototype, realm, callback);
}

#[test]
fn finalization_registry_unregister_is_allocation_free_and_stable() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let (root, function_prototype, realm, callback) = finalization_test_realm(&mut heap, shape);
    let registry = heap
        .allocate_finalization_registry_object(shape, Vec::new(), callback, realm)
        .unwrap();
    let target = leaf(&mut heap, shape);
    let token_a = leaf(&mut heap, shape);
    let token_b = leaf(&mut heap, shape);
    let token_c = leaf(&mut heap, shape);
    let held = (0..4).map(|_| leaf(&mut heap, shape)).collect::<Vec<_>>();
    for (held_value, token) in held
        .iter()
        .copied()
        .zip([token_a, token_b, token_a, token_c])
    {
        heap.finalization_registry_register(
            registry,
            WeakCollectionKey::Object(target),
            RawValue::Object(held_value),
            Some(WeakCollectionKey::Object(token)),
        )
        .unwrap();
        heap.release_object(held_value).unwrap();
    }

    let (matched, cleanup) = heap
        .finalization_registry_unregister(registry, WeakCollectionKey::Object(token_a))
        .unwrap();
    assert!(matched);
    assert_eq!(cleanup.finalized_objects, 2);
    let ObjectPayload::FinalizationRegistry(data) = &heap.object(registry).unwrap().payload else {
        unreachable!()
    };
    assert_eq!(data.entries.len(), 2);
    assert_eq!(data.entries[0].held_value, RawValue::Object(held[1]));
    assert_eq!(data.entries[1].held_value, RawValue::Object(held[3]));
    assert!(matches!(heap.object(held[0]), Err(HeapError::Stale { .. })));
    assert!(matches!(heap.object(held[2]), Err(HeapError::Stale { .. })));

    heap.release_object(registry).unwrap();
    for object in [target, token_a, token_b, token_c] {
        heap.release_object(object).unwrap();
    }
    release_finalization_test_realm(&mut heap, shape, root, function_prototype, realm, callback);
}

#[test]
fn finalization_registry_held_edges_participate_in_trial_deletion() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let cycle_shape = one_slot_shape(&mut heap);
    let (root, function_prototype, realm, callback) = finalization_test_realm(&mut heap, shape);
    let registry = heap
        .allocate_finalization_registry_object(shape, Vec::new(), callback, realm)
        .unwrap();
    let target = leaf(&mut heap, shape);
    let held = heap
        .allocate_object(ObjectData::ordinary(
            cycle_shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();
    heap.finalization_registry_register(
        registry,
        WeakCollectionKey::Object(target),
        RawValue::Object(held),
        None,
    )
    .unwrap();
    heap.replace_object_slot(held, 0, PropertySlot::Data(RawValue::Object(registry)))
        .unwrap();
    assert_eq!(heap.object_strong_count(registry), Ok(2));
    assert_eq!(heap.object_strong_count(held), Ok(2));

    heap.release_object(registry).unwrap();
    heap.release_object(held).unwrap();
    let stats = collect_heap(&mut heap).unwrap();
    assert!(stats.cleanup.finalized_objects >= 2);
    assert!(matches!(
        heap.object(registry),
        Err(HeapError::Stale { .. })
    ));
    assert!(matches!(heap.object(held), Err(HeapError::Stale { .. })));

    heap.release_object(target).unwrap();
    heap.release_shape(cycle_shape).unwrap();
    release_finalization_test_realm(&mut heap, shape, root, function_prototype, realm, callback);
}

#[test]
fn weak_object_keys_are_not_retained_and_gc_prunes_their_values() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let weak_map = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let weak_set = heap
        .allocate_object(ObjectData::weak_set(shape, Vec::new()))
        .unwrap();
    let key = leaf(&mut heap, shape);
    let value = leaf(&mut heap, shape);

    let weak_key = WeakCollectionKey::Object(key);
    heap.weak_map_set(weak_map, weak_key, RawValue::Object(value))
        .unwrap();
    assert!(heap.weak_set_add(weak_set, weak_key).unwrap());
    assert_eq!(heap.object_strong_count(key), Ok(1));
    assert_eq!(heap.object_strong_count(value), Ok(2));

    assert_eq!(heap.release_object(key).unwrap().finalized_objects, 1);
    heap.release_object(value).unwrap();
    assert_eq!(heap.object_strong_count(value), Ok(1));
    assert!(heap.weak_map_get(weak_map, weak_key).unwrap().is_some());
    assert!(heap.weak_set_has(weak_set, weak_key).unwrap());

    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.cleanup.finalized_objects, 1);
    assert!(heap.weak_map_get(weak_map, weak_key).unwrap().is_none());
    assert!(!heap.weak_set_has(weak_set, weak_key).unwrap());
    assert!(matches!(heap.object(value), Err(HeapError::Stale { .. })));

    heap.release_object(weak_map).unwrap();
    heap.release_object(weak_set).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn weak_ref_pass_does_not_revisit_an_earlier_map_after_a_cascade() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    // QuickJS appends these states to rt->weakref_list in constructor
    // order, so m2 is visited before m1 despite the dependency names.
    let m2 = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let m1 = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let k2 = leaf(&mut heap, shape);
    let value = leaf(&mut heap, shape);
    let k1 = leaf(&mut heap, shape);
    let weak_k2 = WeakCollectionKey::Object(k2);
    let weak_k1 = WeakCollectionKey::Object(k1);

    heap.weak_map_set(m2, weak_k2, RawValue::Object(value))
        .unwrap();
    heap.weak_map_set(m1, weak_k1, RawValue::Object(k2))
        .unwrap();
    heap.release_object(k1).unwrap();
    heap.release_object(k2).unwrap();
    heap.release_object(value).unwrap();

    let first = collect_heap(&mut heap).unwrap();
    assert_eq!(first.cleanup.finalized_objects, 1);
    assert!(heap.weak_map_get(m1, weak_k1).unwrap().is_none());
    // m2 was already visited while k2 was still held by m1. It is not
    // revisited after m1 releases k2 during the same weak-ref pass.
    assert!(heap.weak_map_get(m2, weak_k2).unwrap().is_some());
    assert_eq!(heap.object_strong_count(value), Ok(1));

    let second = collect_heap(&mut heap).unwrap();
    assert_eq!(second.cleanup.finalized_objects, 1);
    assert!(heap.weak_map_get(m2, weak_k2).unwrap().is_none());
    assert!(matches!(heap.object(value), Err(HeapError::Stale { .. })));

    heap.release_object(m2).unwrap();
    heap.release_object(m1).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
    assert_eq!(heap.weak_head, None);
    assert_eq!(heap.weak_tail, None);
}

#[test]
fn weak_map_prunes_records_incrementally_in_insertion_order() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let weak_map = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let first_key = leaf(&mut heap, shape);
    let second_key = leaf(&mut heap, shape);
    let held_value = leaf(&mut heap, shape);
    let weak_first = WeakCollectionKey::Object(first_key);
    let weak_second = WeakCollectionKey::Object(second_key);

    // Removing the first record releases the only strong edge to the
    // second record's key. QuickJS then observes that later key as dead
    // during the same insertion-order map walk.
    heap.weak_map_set(weak_map, weak_first, RawValue::Object(second_key))
        .unwrap();
    heap.weak_map_set(weak_map, weak_second, RawValue::Object(held_value))
        .unwrap();
    heap.release_object(first_key).unwrap();
    heap.release_object(second_key).unwrap();
    heap.release_object(held_value).unwrap();

    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.cleanup.finalized_objects, 2);
    assert!(heap.weak_map_get(weak_map, weak_first).unwrap().is_none());
    assert!(heap.weak_map_get(weak_map, weak_second).unwrap().is_none());
    assert!(matches!(
        heap.object(held_value),
        Err(HeapError::Stale { .. })
    ));

    heap.release_object(weak_map).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn weak_symbol_hook_release_precedes_later_record_liveness_query() {
    let mut atoms = crate::engine::atom::AtomTable::new();
    let symbol = atoms.new_symbol(Some("ordered weak key")).unwrap();
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let weak_map = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let first_key = leaf(&mut heap, shape);
    let held_value = leaf(&mut heap, shape);
    let weak_first = WeakCollectionKey::Object(first_key);
    let weak_symbol = WeakCollectionKey::Symbol(symbol);

    // The first value owns the symbol used non-owningly by the next key.
    heap.weak_map_set(weak_map, weak_first, RawValue::Symbol(symbol))
        .unwrap();
    heap.weak_map_set(weak_map, weak_symbol, RawValue::Object(held_value))
        .unwrap();
    heap.release_object(first_key).unwrap();
    heap.release_object(held_value).unwrap();

    let mut events = Vec::new();
    let stats = heap
        .run_gc_with_finalization_sink(
            |event| {
                events.push(event);
                match event {
                    WeakSymbolGcEvent::IsLive(atom) => Ok(atoms.is_live(atom)),
                    WeakSymbolGcEvent::Release(atom) => {
                        atoms.release(atom).map_err(|_| {
                            HeapError::Invariant("weak-symbol test hook release failed")
                        })?;
                        Ok(true)
                    }
                }
            },
            &mut gc::DiscardFinalizationJobSink,
        )
        .unwrap();
    assert_eq!(
        events,
        vec![
            WeakSymbolGcEvent::Release(symbol),
            WeakSymbolGcEvent::IsLive(symbol)
        ]
    );
    assert!(!atoms.is_live(symbol));
    assert!(!stats.cleanup.atoms.contains(&symbol));
    assert_eq!(stats.cleanup.finalized_objects, 1);
    assert!(heap.weak_map_get(weak_map, weak_first).unwrap().is_none());
    assert!(heap.weak_map_get(weak_map, weak_symbol).unwrap().is_none());

    heap.release_object(weak_map).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn gc_release_hook_can_defer_detached_value_atoms() {
    let mut atoms = crate::engine::atom::AtomTable::new();
    let symbol = atoms.new_symbol(Some("deferred weak value")).unwrap();
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let weak_map = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let key = leaf(&mut heap, shape);
    heap.weak_map_set(
        weak_map,
        WeakCollectionKey::Object(key),
        RawValue::Symbol(symbol),
    )
    .unwrap();
    heap.release_object(key).unwrap();

    let stats = heap
        .run_gc_with_finalization_sink(
            |event| {
                Ok(match event {
                    WeakSymbolGcEvent::IsLive(atom) => atoms.is_live(atom),
                    WeakSymbolGcEvent::Release(_) => false,
                })
            },
            &mut gc::DiscardFinalizationJobSink,
        )
        .unwrap();
    assert!(atoms.is_live(symbol));
    assert_eq!(stats.cleanup.atoms, vec![symbol]);
    assert!(atoms.release(symbol).is_ok());

    heap.release_object(weak_map).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn weak_record_delete_and_readd_appends_without_tombstones() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let weak_set = heap
        .allocate_object(ObjectData::weak_set(shape, Vec::new()))
        .unwrap();
    let first = leaf(&mut heap, shape);
    let second = leaf(&mut heap, shape);
    let third = leaf(&mut heap, shape);
    let first = WeakCollectionKey::Object(first);
    let second = WeakCollectionKey::Object(second);
    let third = WeakCollectionKey::Object(third);
    assert!(heap.weak_set_add(weak_set, first).unwrap());
    assert!(heap.weak_set_add(weak_set, second).unwrap());
    assert!(heap.weak_set_add(weak_set, third).unwrap());
    assert!(heap.weak_set_delete(weak_set, second).unwrap());
    assert!(heap.weak_set_add(weak_set, second).unwrap());

    let ObjectPayload::WeakSet { records } = &heap.object(weak_set).unwrap().payload else {
        unreachable!()
    };
    assert_eq!(records.len(), 3);
    assert_eq!(records.head, Some(first));
    assert_eq!(records.next_key(first), Ok(Some(third)));
    assert_eq!(records.next_key(third), Ok(Some(second)));
    assert_eq!(records.next_key(second), Ok(None));
    assert_eq!(records.tail, Some(second));

    heap.release_object(weak_set).unwrap();
    for key in [first, second, third] {
        let WeakCollectionKey::Object(key) = key else {
            unreachable!()
        };
        heap.release_object(key).unwrap();
    }
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn weak_ref_registry_preserves_construction_order_across_slot_reuse() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let recycled = heap
        .allocate_object(ObjectData::weak_set(shape, Vec::new()))
        .unwrap();
    let m2 = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    assert!(recycled.debug_index() < m2.debug_index());
    heap.release_object(recycled).unwrap();

    // m1 reuses the lower physical slot but is newer than m2, so it must
    // append after m2 instead of being visited first by arena index.
    let m1 = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    assert_eq!(m1.debug_index(), recycled.debug_index());
    assert!(m1.debug_index() < m2.debug_index());
    assert_eq!(heap.weak_head, Some(m2));
    assert_eq!(heap.weak_tail, Some(m1));
    assert_eq!(heap.slots[m2.debug_index() as usize].weak_next, Some(m1));
    assert_eq!(heap.slots[m1.debug_index() as usize].weak_prev, Some(m2));

    let k2 = leaf(&mut heap, shape);
    let value = leaf(&mut heap, shape);
    let k1 = leaf(&mut heap, shape);
    let weak_k2 = WeakCollectionKey::Object(k2);
    let weak_k1 = WeakCollectionKey::Object(k1);
    heap.weak_map_set(m2, weak_k2, RawValue::Object(value))
        .unwrap();
    heap.weak_map_set(m1, weak_k1, RawValue::Object(k2))
        .unwrap();
    heap.release_object(k1).unwrap();
    heap.release_object(k2).unwrap();
    heap.release_object(value).unwrap();

    assert_eq!(
        collect_heap(&mut heap).unwrap().cleanup.finalized_objects,
        1
    );
    assert!(heap.weak_map_get(m2, weak_k2).unwrap().is_some());
    assert!(heap.weak_map_get(m1, weak_k1).unwrap().is_none());
    assert_eq!(
        collect_heap(&mut heap).unwrap().cleanup.finalized_objects,
        1
    );
    assert!(heap.weak_map_get(m2, weak_k2).unwrap().is_none());

    heap.release_object(m2).unwrap();
    assert_eq!(heap.weak_head, Some(m1));
    assert_eq!(heap.weak_tail, Some(m1));
    heap.release_object(m1).unwrap();
    assert_eq!(heap.weak_head, None);
    assert_eq!(heap.weak_tail, None);
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn weak_map_value_edge_can_keep_its_key_alive() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let value_shape = one_slot_shape(&mut heap);
    let key = leaf(&mut heap, shape);
    let value = heap
        .allocate_object(ObjectData::ordinary(
            value_shape,
            vec![PropertySlot::Data(RawValue::Object(key))],
        ))
        .unwrap();
    let weak_map = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let weak_key = WeakCollectionKey::Object(key);
    heap.weak_map_set(weak_map, weak_key, RawValue::Object(value))
        .unwrap();

    assert_eq!(heap.object_strong_count(key), Ok(2));
    assert_eq!(heap.object_strong_count(value), Ok(2));
    heap.release_object(key).unwrap();
    heap.release_object(value).unwrap();
    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.cleanup.finalized_objects, 0);
    assert_eq!(heap.object_strong_count(key), Ok(1));
    assert_eq!(heap.object_strong_count(value), Ok(1));
    assert!(heap.weak_map_get(weak_map, weak_key).unwrap().is_some());

    let (deleted, cleanup) = heap.weak_map_delete(weak_map, weak_key).unwrap();
    assert!(deleted);
    assert_eq!(cleanup.finalized_objects, 2);
    heap.release_object(weak_map).unwrap();
    heap.release_shape(value_shape).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn cycle_collected_weak_key_is_pruned_on_the_following_gc() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let cycle_shape = one_slot_shape(&mut heap);
    let first = heap
        .allocate_object(ObjectData::ordinary(
            cycle_shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();
    let second = heap
        .allocate_object(ObjectData::ordinary(
            cycle_shape,
            vec![PropertySlot::Data(RawValue::Object(first))],
        ))
        .unwrap();
    heap.replace_object_slot(first, 0, PropertySlot::Data(RawValue::Object(second)))
        .unwrap();
    let held_value = leaf(&mut heap, shape);
    let weak_map = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let weak_key = WeakCollectionKey::Object(first);
    heap.weak_map_set(weak_map, weak_key, RawValue::Object(held_value))
        .unwrap();
    heap.release_object(first).unwrap();
    heap.release_object(second).unwrap();
    heap.release_object(held_value).unwrap();

    let first_gc = collect_heap(&mut heap).unwrap();
    assert_eq!(first_gc.cleanup.finalized_objects, 2);
    assert!(heap.weak_map_get(weak_map, weak_key).unwrap().is_some());
    assert_eq!(heap.object_strong_count(held_value), Ok(1));

    let second_gc = collect_heap(&mut heap).unwrap();
    assert_eq!(second_gc.cleanup.finalized_objects, 1);
    assert!(heap.weak_map_get(weak_map, weak_key).unwrap().is_none());
    assert!(matches!(
        heap.object(held_value),
        Err(HeapError::Stale { .. })
    ));

    heap.release_object(weak_map).unwrap();
    heap.release_shape(cycle_shape).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn stale_symbol_keys_are_pruned_without_owning_the_atom() {
    let mut atoms = crate::engine::atom::AtomTable::new();
    let symbol = atoms.new_symbol(Some("weak key")).unwrap();
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let weak_map = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let weak_set = heap
        .allocate_object(ObjectData::weak_set(shape, Vec::new()))
        .unwrap();
    let value = leaf(&mut heap, shape);
    let weak_key = WeakCollectionKey::Symbol(symbol);
    heap.weak_map_set(weak_map, weak_key, RawValue::Object(value))
        .unwrap();
    assert!(heap.weak_set_add(weak_set, weak_key).unwrap());
    heap.release_object(value).unwrap();

    assert!(atoms.is_live(symbol));
    assert_eq!(
        atoms.release(symbol),
        Ok(crate::engine::atom::ReleaseOutcome::Removed)
    );
    assert!(!atoms.is_live(symbol));
    let stats = heap
        .run_gc_with_finalization_sink(
            |event| {
                Ok(match event {
                    WeakSymbolGcEvent::IsLive(atom) => atoms.is_live(atom),
                    WeakSymbolGcEvent::Release(_) => false,
                })
            },
            &mut gc::DiscardFinalizationJobSink,
        )
        .unwrap();
    assert_eq!(stats.cleanup.finalized_objects, 1);
    assert!(!stats.cleanup.atoms.contains(&symbol));
    assert!(heap.weak_map_get(weak_map, weak_key).unwrap().is_none());
    assert!(!heap.weak_set_has(weak_set, weak_key).unwrap());

    heap.release_object(weak_map).unwrap();
    heap.release_object(weak_set).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn weak_map_hash_storage_handles_the_deep_staging_scale() {
    const LENGTH: usize = 100_000;

    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let weak_map = heap
        .allocate_object(ObjectData::weak_map(shape, Vec::new()))
        .unwrap();
    let mut keys = Vec::with_capacity(LENGTH);
    for index in 0..LENGTH {
        let key = leaf(&mut heap, shape);
        let weak_key = WeakCollectionKey::Object(key);
        heap.weak_map_set(
            weak_map,
            weak_key,
            RawValue::Int(i32::try_from(index).unwrap()),
        )
        .unwrap();
        keys.push(key);
    }
    for (index, key) in keys.iter().copied().enumerate() {
        assert_eq!(
            heap.weak_map_get(weak_map, WeakCollectionKey::Object(key)),
            Ok(Some(&RawValue::Int(i32::try_from(index).unwrap())))
        );
    }

    for key in keys {
        assert_eq!(heap.release_object(key).unwrap().finalized_objects, 1);
    }
    collect_heap(&mut heap).unwrap();
    assert!(matches!(
        &heap.object(weak_map).unwrap().payload,
        ObjectPayload::WeakMap { records } if records.is_empty()
    ));

    heap.release_object(weak_map).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn weak_collection_native_descriptors_match_quickjs_protocols() {
    assert_eq!(
        NativeFunctionId::WeakMap(WeakMapNativeKind::Constructor)
            .descriptor()
            .cproto,
        NativeCProto::Constructor
    );
    assert_eq!(
        NativeFunctionId::WeakSet(WeakSetNativeKind::Constructor)
            .descriptor()
            .cproto,
        NativeCProto::Constructor
    );
    for target in [
        NativeFunctionId::WeakMap(WeakMapNativeKind::Set),
        NativeFunctionId::WeakMap(WeakMapNativeKind::GetOrInsertComputed),
        NativeFunctionId::WeakSet(WeakSetNativeKind::Add),
        NativeFunctionId::WeakSet(WeakSetNativeKind::Delete),
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
}

#[test]
fn weak_reference_native_descriptors_match_quickjs_protocols() {
    for target in [
        NativeFunctionId::WeakRef(WeakRefNativeKind::Constructor),
        NativeFunctionId::FinalizationRegistry(FinalizationRegistryNativeKind::Constructor),
    ] {
        assert_eq!(
            target.descriptor().cproto,
            NativeCProto::ConstructorOrFunction
        );
        assert!(target.descriptor().cproto.default_is_constructor());
        assert!(!target.uses_calling_realm());
    }
    for target in [
        NativeFunctionId::WeakRef(WeakRefNativeKind::Deref),
        NativeFunctionId::FinalizationRegistry(FinalizationRegistryNativeKind::Register),
        NativeFunctionId::FinalizationRegistry(FinalizationRegistryNativeKind::Unregister),
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
        assert!(!target.uses_calling_realm());
    }
}

#[test]
fn weak_reference_intrinsics_attach_atomically_and_root_both_prototypes() {
    let mut heap = Heap::new();
    let empty_shape = empty_shape(&mut heap);
    let root = leaf(&mut heap, empty_shape);
    let realm = heap
        .allocate_context(ContextData::new(
            root, root, root, root, root, root, root, root,
        ))
        .unwrap();
    assert_eq!(heap.context(realm).unwrap().weak_ref, None);

    let intrinsic_shape = heap
        .allocate_shape(Shape::new(Some(root), []).unwrap())
        .unwrap();
    let weak_ref_prototype = heap
        .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
        .unwrap();
    let finalization_registry_prototype = heap
        .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
        .unwrap();
    let roots = WeakRefRealmData {
        weak_ref_prototype,
        finalization_registry_prototype,
    };
    let weak_ref_strong = heap.object_strong_count(weak_ref_prototype).unwrap();
    let finalization_registry_strong = heap
        .object_strong_count(finalization_registry_prototype)
        .unwrap();

    assert_eq!(
        heap.attach_weak_ref_intrinsics(
            realm,
            WeakRefRealmData {
                weak_ref_prototype,
                finalization_registry_prototype: weak_ref_prototype,
            },
        ),
        Err(HeapError::Invariant(
            "WeakRef and FinalizationRegistry prototypes share one identity",
        ))
    );
    assert_eq!(heap.context(realm).unwrap().weak_ref, None);

    assert_eq!(
        heap.attach_weak_ref_intrinsics(
            realm,
            WeakRefRealmData {
                weak_ref_prototype: root,
                finalization_registry_prototype,
            },
        ),
        Err(HeapError::Invariant(
            "WeakRef prototype is not an ordinary child of Object.prototype",
        ))
    );
    assert_eq!(heap.context(realm).unwrap().weak_ref, None);
    assert_eq!(
        heap.object_strong_count(weak_ref_prototype),
        Ok(weak_ref_strong)
    );
    assert_eq!(
        heap.object_strong_count(finalization_registry_prototype),
        Ok(finalization_registry_strong)
    );

    heap.live_node_mut(RawId::Object(finalization_registry_prototype))
        .unwrap()
        .strong = u32::MAX;
    assert_eq!(
        heap.attach_weak_ref_intrinsics(realm, roots),
        Err(HeapError::Overflow {
            operation: "retaining outgoing heap edges",
        })
    );
    assert_eq!(heap.context(realm).unwrap().weak_ref, None);
    assert_eq!(
        heap.object_strong_count(weak_ref_prototype),
        Ok(weak_ref_strong)
    );
    heap.live_node_mut(RawId::Object(finalization_registry_prototype))
        .unwrap()
        .strong = finalization_registry_strong;

    heap.attach_weak_ref_intrinsics(realm, roots).unwrap();
    assert_eq!(heap.context(realm).unwrap().weak_ref, Some(roots));
    assert_eq!(
        heap.object_strong_count(weak_ref_prototype),
        Ok(weak_ref_strong + 1)
    );
    assert_eq!(
        heap.object_strong_count(finalization_registry_prototype),
        Ok(finalization_registry_strong + 1)
    );
    assert_eq!(
        heap.attach_weak_ref_intrinsics(realm, roots),
        Err(HeapError::Invariant(
            "context already has WeakRef intrinsic roots",
        ))
    );

    heap.release_object(weak_ref_prototype).unwrap();
    heap.release_object(finalization_registry_prototype)
        .unwrap();
    let cleanup = heap.release_context(realm).unwrap();
    assert_eq!(cleanup.finalized_contexts, 1);
    assert_eq!(cleanup.finalized_objects, 2);
    heap.release_shape(intrinsic_shape).unwrap();
    heap.release_object(root).unwrap();
    heap.release_shape(empty_shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}
