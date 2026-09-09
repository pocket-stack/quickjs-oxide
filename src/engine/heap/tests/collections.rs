use crate::engine::heap::native::{MapNativeKind, NativeCProto, SetNativeKind};

use super::*;

#[test]
fn map_records_retain_gc_edges_and_delete_releases_them() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let map = heap
        .allocate_object(ObjectData::map(shape, Vec::new()))
        .unwrap();
    let key = leaf(&mut heap, shape);
    let initial_value = leaf(&mut heap, shape);
    let replacement_value = leaf(&mut heap, shape);

    assert_eq!(
        heap.map_insert_record(map, RawValue::Object(key), RawValue::Object(initial_value),),
        Ok(HeapCleanup::default())
    );
    assert_eq!(heap.map_size(map), Ok(1));
    assert_eq!(heap.object_strong_count(key), Ok(2));
    assert_eq!(heap.object_strong_count(initial_value), Ok(2));
    heap.release_object(key).unwrap();
    heap.release_object(initial_value).unwrap();

    let cleanup = heap
        .map_replace_record_value(map, 0, RawValue::Object(replacement_value))
        .unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert!(matches!(
        heap.object(initial_value),
        Err(HeapError::Stale { .. })
    ));
    assert_eq!(heap.object_strong_count(replacement_value), Ok(2));
    heap.release_object(replacement_value).unwrap();

    let cleanup = heap.map_delete_record(map, 0).unwrap();
    assert_eq!(cleanup.finalized_objects, 2);
    assert!(matches!(heap.object(key), Err(HeapError::Stale { .. })));
    assert!(matches!(
        heap.object(replacement_value),
        Err(HeapError::Stale { .. })
    ));
    assert_eq!(heap.map_size(map), Ok(0));
    assert_eq!(
        heap.map_records(map),
        Ok(&[MapRecord {
            key: None,
            value: RawValue::Undefined,
        }][..])
    );

    heap.release_object(map).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn map_tombstones_preserve_readd_order_and_live_iterator_sees_appends() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let map = heap
        .allocate_object(ObjectData::map(shape, Vec::new()))
        .unwrap();
    heap.map_insert_record(
        map,
        RawValue::Int(1),
        RawValue::String(JsString::from_static("one")),
    )
    .unwrap();
    let iterator = heap
        .allocate_object(ObjectData::map_iterator(
            shape,
            Vec::new(),
            map,
            MapIteratorKind::KeyAndValue,
        ))
        .unwrap();

    heap.set_map_iterator_index(iterator, 1).unwrap();
    heap.map_insert_record(
        map,
        RawValue::Int(2),
        RawValue::String(JsString::from_static("first")),
    )
    .unwrap();
    assert_eq!(
        heap.map_records(map).unwrap()[1].key,
        Some(RawValue::Int(2))
    );
    heap.map_delete_record(map, 1).unwrap();
    heap.map_insert_record(
        map,
        RawValue::Int(2),
        RawValue::String(JsString::from_static("second")),
    )
    .unwrap();

    let records = heap.map_records(map).unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].key, Some(RawValue::Int(1)));
    assert_eq!(records[1].key, None);
    assert_eq!(records[1].value, RawValue::Undefined);
    assert_eq!(records[2].key, Some(RawValue::Int(2)));
    assert_eq!(
        records[2].value,
        RawValue::String(JsString::from_static("second"))
    );
    let (source, next_index, kind) = heap.map_iterator_state(iterator).unwrap();
    assert_eq!(source, Some(map));
    assert_eq!(next_index, 1);
    assert_eq!(kind, MapIteratorKind::KeyAndValue);
    assert_eq!(
        records[next_index..]
            .iter()
            .find_map(|record| record.key.as_ref()),
        Some(&RawValue::Int(2))
    );

    assert_eq!(heap.object_strong_count(map), Ok(2));
    heap.release_object(map).unwrap();
    assert_eq!(heap.object_strong_count(map), Ok(1));
    let cleanup = heap.finish_map_iterator(iterator).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert!(matches!(heap.object(map), Err(HeapError::Stale { .. })));
    assert_eq!(
        heap.map_iterator_state(iterator),
        Ok((None, 1, MapIteratorKind::KeyAndValue))
    );
    assert!(matches!(
        heap.set_map_iterator_index(iterator, 2),
        Err(HeapError::Invariant("completed Map Iterator was advanced"))
    ));

    heap.release_object(iterator).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn map_symbol_atoms_transfer_and_return_on_replace_delete_and_clear() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let map = heap
        .allocate_object(ObjectData::map(shape, Vec::new()))
        .unwrap();
    let first_key = Atom::from_immediate_integer(101).unwrap();
    let first_value = Atom::from_immediate_integer(102).unwrap();
    let replacement = Atom::from_immediate_integer(103).unwrap();
    let second_key = Atom::from_immediate_integer(104).unwrap();
    let second_value = Atom::from_immediate_integer(105).unwrap();

    heap.map_insert_record(
        map,
        RawValue::Symbol(first_key),
        RawValue::Symbol(first_value),
    )
    .unwrap();
    let cleanup = heap
        .map_replace_record_value(map, 0, RawValue::Symbol(replacement))
        .unwrap();
    assert_eq!(cleanup.atoms, vec![first_value]);
    heap.map_insert_record(
        map,
        RawValue::Symbol(second_key),
        RawValue::Symbol(second_value),
    )
    .unwrap();

    let cleanup = heap.map_delete_record(map, 0).unwrap();
    assert_eq!(cleanup.atoms, vec![first_key, replacement]);
    let cleanup = heap.map_clear(map).unwrap();
    assert_eq!(cleanup.atoms, vec![second_key, second_value]);
    assert_eq!(heap.map_size(map), Ok(0));

    heap.release_object(map).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn map_intrinsics_attach_transactionally_and_root_the_realm_graph() {
    let mut heap = Heap::new();
    let empty_shape = empty_shape(&mut heap);
    let root = leaf(&mut heap, empty_shape);
    let realm = heap
        .allocate_context(ContextData::new(
            root, root, root, root, root, root, root, root,
        ))
        .unwrap();
    let intrinsic_shape = heap
        .allocate_shape(Shape::new(Some(root), []).unwrap())
        .unwrap();
    let prototype = heap
        .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
        .unwrap();
    let iterator_prototype = heap
        .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
        .unwrap();
    let constructor = heap
        .allocate_object(ObjectData::bound_native_function(
            intrinsic_shape,
            Vec::new(),
            NativeFunctionId::Map(MapNativeKind::Constructor),
            realm,
            0,
        ))
        .unwrap();
    let map = MapRealmData {
        prototype,
        iterator_prototype,
    };
    let prototype_strong = heap.object_strong_count(prototype).unwrap();
    let constructor_strong = heap.object_strong_count(constructor).unwrap();
    let iterator_strong = heap.object_strong_count(iterator_prototype).unwrap();

    heap.live_node_mut(RawId::Object(iterator_prototype))
        .unwrap()
        .strong = u32::MAX;
    assert_eq!(
        heap.attach_map_intrinsics(realm, map),
        Err(HeapError::Overflow {
            operation: "retaining outgoing heap edges",
        })
    );
    assert_eq!(heap.context(realm).unwrap().map, None);
    assert_eq!(heap.object_strong_count(prototype), Ok(prototype_strong));
    assert_eq!(
        heap.object_strong_count(constructor),
        Ok(constructor_strong)
    );
    heap.live_node_mut(RawId::Object(iterator_prototype))
        .unwrap()
        .strong = iterator_strong;

    heap.attach_map_intrinsics(realm, map).unwrap();
    assert_eq!(heap.context(realm).unwrap().map, Some(map));
    assert_eq!(
        heap.object_strong_count(prototype),
        Ok(prototype_strong + 1)
    );
    assert_eq!(
        heap.object_strong_count(constructor),
        Ok(constructor_strong)
    );
    assert_eq!(
        heap.object_strong_count(iterator_prototype),
        Ok(iterator_strong + 1)
    );
    assert!(matches!(
        heap.attach_map_intrinsics(realm, map),
        Err(HeapError::Invariant(
            "context already has Map intrinsic roots"
        ))
    ));

    heap.release_object(prototype).unwrap();
    heap.release_object(iterator_prototype).unwrap();
    let constructor_cleanup = heap.release_object(constructor).unwrap();
    assert_eq!(constructor_cleanup.finalized_objects, 1);
    let context_cleanup = heap.release_context(realm).unwrap();
    assert_eq!(context_cleanup.finalized_contexts, 1);
    assert_eq!(context_cleanup.finalized_objects, 2);
    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.cleanup.finalized_contexts, 0);
    assert_eq!(stats.cleanup.finalized_objects, 0);
    heap.release_shape(intrinsic_shape).unwrap();
    heap.release_object(root).unwrap();
    heap.release_shape(empty_shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn map_native_descriptors_preserve_quickjs_call_protocols() {
    assert_eq!(
        NativeFunctionId::Map(MapNativeKind::Constructor)
            .descriptor()
            .cproto,
        NativeCProto::Constructor
    );
    for kind in [MapNativeKind::Species, MapNativeKind::Size] {
        assert_eq!(
            NativeFunctionId::Map(kind).descriptor().cproto,
            NativeCProto::Getter
        );
    }
    for kind in [
        MapNativeKind::GroupBy,
        MapNativeKind::Set,
        MapNativeKind::Get,
        MapNativeKind::GetOrInsert,
        MapNativeKind::GetOrInsertComputed,
        MapNativeKind::Has,
        MapNativeKind::Delete,
        MapNativeKind::Clear,
        MapNativeKind::ForEach,
        MapNativeKind::Iterator(MapIteratorKind::Key),
        MapNativeKind::Iterator(MapIteratorKind::Value),
        MapNativeKind::Iterator(MapIteratorKind::KeyAndValue),
    ] {
        assert_eq!(
            NativeFunctionId::Map(kind).descriptor().cproto,
            NativeCProto::Generic
        );
    }
    assert_eq!(
        NativeFunctionId::MapIteratorNext.descriptor().cproto,
        NativeCProto::IteratorNext
    );
}

#[test]
fn set_records_retain_key_edges_and_tombstone_with_undefined_value() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let set = heap
        .allocate_object(ObjectData::set(shape, Vec::new()))
        .unwrap();
    let key = leaf(&mut heap, shape);

    assert_eq!(
        heap.set_insert_record(set, RawValue::Object(key)),
        Ok(HeapCleanup::default())
    );
    assert_eq!(heap.set_size(set), Ok(1));
    assert_eq!(heap.object_strong_count(key), Ok(2));
    assert_eq!(
        heap.set_records(set),
        Ok(&[MapRecord {
            key: Some(RawValue::Object(key)),
            value: RawValue::Undefined,
        }][..])
    );
    heap.release_object(key).unwrap();

    let cleanup = heap.set_delete_record(set, 0).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert!(matches!(heap.object(key), Err(HeapError::Stale { .. })));
    assert_eq!(heap.set_size(set), Ok(0));
    assert_eq!(
        heap.set_records(set),
        Ok(&[MapRecord {
            key: None,
            value: RawValue::Undefined,
        }][..])
    );
    assert!(matches!(
        heap.set_delete_record(set, 0),
        Err(HeapError::Invariant(
            "Set deletion requires a live record index"
        ))
    ));
    assert!(matches!(
        heap.set_insert_record(set, RawValue::Uninitialized),
        Err(HeapError::Invariant(
            "Set record contains an internal value sentinel"
        ))
    ));
    assert!(matches!(
        heap.set_insert_record(set, RawValue::Exception),
        Err(HeapError::Invariant(
            "Set record contains an internal value sentinel"
        ))
    ));

    let map = heap
        .allocate_object(ObjectData::map(shape, Vec::new()))
        .unwrap();
    assert!(matches!(
        heap.set_insert_record(map, RawValue::Int(1)),
        Err(HeapError::Invariant(
            "Set insertion reached an object with the wrong class"
        ))
    ));
    heap.release_object(map).unwrap();
    heap.release_object(set).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn set_tombstones_preserve_readd_order_and_live_iterator_sees_appends() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let set = heap
        .allocate_object(ObjectData::set(shape, Vec::new()))
        .unwrap();
    heap.set_insert_record(set, RawValue::Int(1)).unwrap();
    let iterator = heap
        .allocate_object(ObjectData::set_iterator(
            shape,
            Vec::new(),
            set,
            SetIteratorKind::KeyAndValue,
        ))
        .unwrap();

    heap.set_set_iterator_index(iterator, 1).unwrap();
    heap.set_insert_record(set, RawValue::Int(2)).unwrap();
    heap.set_delete_record(set, 1).unwrap();
    heap.set_insert_record(set, RawValue::Int(2)).unwrap();

    let records = heap.set_records(set).unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].key, Some(RawValue::Int(1)));
    assert_eq!(records[1].key, None);
    assert_eq!(records[2].key, Some(RawValue::Int(2)));
    assert!(
        records
            .iter()
            .all(|record| matches!(record.value, RawValue::Undefined))
    );
    assert_eq!(heap.set_size(set), Ok(2));
    assert_eq!(
        heap.set_iterator_state(iterator),
        Ok((Some(set), 1, SetIteratorKind::KeyAndValue))
    );
    assert_eq!(
        records[1..].iter().find_map(|record| record.key.as_ref()),
        Some(&RawValue::Int(2))
    );

    assert_eq!(heap.object_strong_count(set), Ok(2));
    heap.release_object(set).unwrap();
    assert_eq!(heap.object_strong_count(set), Ok(1));
    let cleanup = heap.finish_set_iterator(iterator).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert!(matches!(heap.object(set), Err(HeapError::Stale { .. })));
    assert_eq!(
        heap.set_iterator_state(iterator),
        Ok((None, 1, SetIteratorKind::KeyAndValue))
    );
    assert_eq!(
        heap.finish_set_iterator(iterator).unwrap(),
        HeapCleanup::default()
    );
    assert!(matches!(
        heap.set_set_iterator_index(iterator, 2),
        Err(HeapError::Invariant("completed Set Iterator was advanced"))
    ));

    heap.release_object(iterator).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn set_symbol_atoms_transfer_and_return_on_delete_clear_and_finalize() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let set = heap
        .allocate_object(ObjectData::set(shape, Vec::new()))
        .unwrap();
    let first = Atom::from_immediate_integer(201).unwrap();
    let second = Atom::from_immediate_integer(202).unwrap();
    let third = Atom::from_immediate_integer(203).unwrap();

    heap.set_insert_record(set, RawValue::Symbol(first))
        .unwrap();
    heap.set_insert_record(set, RawValue::Symbol(second))
        .unwrap();
    let cleanup = heap.set_delete_record(set, 0).unwrap();
    assert_eq!(cleanup.atoms, vec![first]);
    let cleanup = heap.set_clear(set).unwrap();
    assert_eq!(cleanup.atoms, vec![second]);
    assert_eq!(heap.set_size(set), Ok(0));

    heap.set_insert_record(set, RawValue::Symbol(third))
        .unwrap();
    let cleanup = heap.release_object(set).unwrap();
    assert_eq!(cleanup.atoms, vec![third]);
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn set_layout_and_iterator_source_are_structurally_validated() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);

    let malformed = ObjectData {
        shape,
        slots: Vec::new(),
        private_brand_home: None,
        is_html_dda: false,
        extensible: true,
        immutable_prototype: false,
        is_constructor: false,
        kind: ObjectKind::Set,
        payload: ObjectPayload::Set {
            records: vec![MapRecord {
                key: Some(RawValue::Int(1)),
                value: RawValue::Int(2),
            }],
            live_indices: [0].into_iter().collect(),
            size: 1,
        },
    };
    assert!(matches!(
        heap.allocate_object(malformed),
        Err(HeapError::Invariant(
            "Set record value slot is not undefined"
        ))
    ));
    assert_eq!(heap.counts().object_nodes, 0);

    let map = heap
        .allocate_object(ObjectData::map(shape, Vec::new()))
        .unwrap();
    assert!(matches!(
        heap.allocate_object(ObjectData::set_iterator(
            shape,
            Vec::new(),
            map,
            SetIteratorKind::Value,
        )),
        Err(HeapError::Invariant(
            "Set Iterator source does not have the Set class"
        ))
    ));
    assert_eq!(heap.object_strong_count(map), Ok(1));

    let mut mismatched = ObjectData::set(shape, Vec::new());
    mismatched.kind = ObjectKind::Map;
    assert!(matches!(
        heap.allocate_object(mismatched),
        Err(HeapError::Invariant(
            "object kind does not match its class payload"
        ))
    ));

    heap.release_object(map).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn set_intrinsics_attach_transactionally_and_root_the_realm_graph() {
    let mut heap = Heap::new();
    let empty_shape = empty_shape(&mut heap);
    let root = leaf(&mut heap, empty_shape);
    let realm = heap
        .allocate_context(ContextData::new(
            root, root, root, root, root, root, root, root,
        ))
        .unwrap();
    let intrinsic_shape = heap
        .allocate_shape(Shape::new(Some(root), []).unwrap())
        .unwrap();
    let prototype = heap
        .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
        .unwrap();
    let iterator_prototype = heap
        .allocate_object(ObjectData::ordinary(intrinsic_shape, Vec::new()))
        .unwrap();
    let constructor = heap
        .allocate_object(ObjectData::bound_native_function(
            intrinsic_shape,
            Vec::new(),
            NativeFunctionId::Set(SetNativeKind::Constructor),
            realm,
            0,
        ))
        .unwrap();
    let set = SetRealmData {
        prototype,
        iterator_prototype,
    };
    let prototype_strong = heap.object_strong_count(prototype).unwrap();
    let constructor_strong = heap.object_strong_count(constructor).unwrap();
    let iterator_strong = heap.object_strong_count(iterator_prototype).unwrap();

    heap.live_node_mut(RawId::Object(iterator_prototype))
        .unwrap()
        .strong = u32::MAX;
    assert_eq!(
        heap.attach_set_intrinsics(realm, set),
        Err(HeapError::Overflow {
            operation: "retaining outgoing heap edges",
        })
    );
    assert_eq!(heap.context(realm).unwrap().set, None);
    assert_eq!(heap.object_strong_count(prototype), Ok(prototype_strong));
    assert_eq!(
        heap.object_strong_count(constructor),
        Ok(constructor_strong)
    );
    heap.live_node_mut(RawId::Object(iterator_prototype))
        .unwrap()
        .strong = iterator_strong;

    heap.attach_set_intrinsics(realm, set).unwrap();
    assert_eq!(heap.context(realm).unwrap().set, Some(set));
    assert_eq!(
        heap.object_strong_count(prototype),
        Ok(prototype_strong + 1)
    );
    assert_eq!(
        heap.object_strong_count(constructor),
        Ok(constructor_strong)
    );
    assert_eq!(
        heap.object_strong_count(iterator_prototype),
        Ok(iterator_strong + 1)
    );
    assert!(matches!(
        heap.attach_set_intrinsics(realm, set),
        Err(HeapError::Invariant(
            "context already has Set intrinsic roots"
        ))
    ));

    heap.release_object(prototype).unwrap();
    heap.release_object(iterator_prototype).unwrap();
    let constructor_cleanup = heap.release_object(constructor).unwrap();
    assert_eq!(constructor_cleanup.finalized_objects, 1);
    let context_cleanup = heap.release_context(realm).unwrap();
    assert_eq!(context_cleanup.finalized_contexts, 1);
    assert_eq!(context_cleanup.finalized_objects, 2);
    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.cleanup.finalized_contexts, 0);
    assert_eq!(stats.cleanup.finalized_objects, 0);
    heap.release_shape(intrinsic_shape).unwrap();
    heap.release_object(root).unwrap();
    heap.release_shape(empty_shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn set_native_descriptors_preserve_quickjs_call_protocols() {
    assert_eq!(
        NativeFunctionId::Set(SetNativeKind::Constructor)
            .descriptor()
            .cproto,
        NativeCProto::Constructor
    );
    for kind in [SetNativeKind::Species, SetNativeKind::Size] {
        assert_eq!(
            NativeFunctionId::Set(kind).descriptor().cproto,
            NativeCProto::Getter
        );
    }
    for kind in [
        SetNativeKind::GroupBy,
        SetNativeKind::Add,
        SetNativeKind::Has,
        SetNativeKind::Delete,
        SetNativeKind::Clear,
        SetNativeKind::ForEach,
        SetNativeKind::IsDisjointFrom,
        SetNativeKind::IsSubsetOf,
        SetNativeKind::IsSupersetOf,
        SetNativeKind::Intersection,
        SetNativeKind::Difference,
        SetNativeKind::SymmetricDifference,
        SetNativeKind::Union,
        SetNativeKind::Iterator(SetIteratorKind::Value),
        SetNativeKind::Iterator(SetIteratorKind::KeyAndValue),
    ] {
        assert_eq!(
            NativeFunctionId::Set(kind).descriptor().cproto,
            NativeCProto::Generic
        );
    }
    assert_eq!(
        NativeFunctionId::SetIteratorNext.descriptor().cproto,
        NativeCProto::IteratorNext
    );
}

#[test]
fn for_in_iterator_advances_snapshots_and_transfers_current_edges() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let source = leaf(&mut heap, shape);
    let iterator = heap
        .allocate_object(ObjectData::for_in_iterator(
            shape,
            Vec::new(),
            ForInIteratorData {
                object: Some(source),
                index: 0,
                properties: vec![ForInProperty {
                    name: JsString::from_static("a"),
                    enumerable: true,
                }],
                fast_array: false,
                array_count: 0,
                in_prototype_chain: false,
                visited: HashSet::new(),
            },
        ))
        .unwrap();

    assert_eq!(heap.object_strong_count(source), Ok(2));
    heap.release_object(source).unwrap();
    assert_eq!(
        heap.next_for_in_candidate(iterator),
        Ok(ForInCandidate::Property {
            object: source,
            name: JsString::from_static("a"),
        })
    );
    assert_eq!(
        heap.next_for_in_candidate(iterator),
        Ok(ForInCandidate::BaseComplete {
            object: source,
            fast_array: false,
        })
    );
    heap.enter_for_in_prototype_chain(iterator, None).unwrap();

    let prototype = leaf(&mut heap, shape);
    let cleanup = heap
        .replace_for_in_level(
            iterator,
            Some(prototype),
            vec![ForInProperty {
                name: JsString::from_static("b"),
                enumerable: true,
            }],
        )
        .unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert!(matches!(heap.object(source), Err(HeapError::Stale { .. })));
    heap.release_object(prototype).unwrap();
    assert_eq!(
        heap.next_for_in_candidate(iterator),
        Ok(ForInCandidate::Property {
            object: prototype,
            name: JsString::from_static("b"),
        })
    );
    assert_eq!(
        heap.next_for_in_candidate(iterator),
        Ok(ForInCandidate::LevelComplete(prototype))
    );
    let cleanup = heap
        .replace_for_in_level(iterator, None, Vec::new())
        .unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(
        heap.next_for_in_candidate(iterator),
        Ok(ForInCandidate::Done)
    );

    heap.release_object(iterator).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}
