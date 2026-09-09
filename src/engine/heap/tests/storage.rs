use crate::engine::heap::native::{DateNativeKind, NativeCProto};

use super::*;

#[test]
fn prepared_array_dense_truncation_is_inert_until_commit() {
    let mut heap = Heap::new();
    let target_shape = empty_shape(&mut heap);
    let target = leaf(&mut heap, target_shape);
    let array_shape = one_slot_shape(&mut heap);
    let array = heap
        .allocate_object(ObjectData::array(
            array_shape,
            vec![PropertySlot::Data(RawValue::Int(0))],
        ))
        .unwrap();
    heap.append_fresh_array_dense_value(array, RawValue::Object(target))
        .unwrap();
    assert_eq!(heap.array_dense_len(array), Ok(Some(1)));
    assert_eq!(heap.object_strong_count(target), Ok(2));

    let prepared = heap.prepare_array_dense_truncation(array, 0).unwrap();
    assert_eq!(heap.array_dense_len(array), Ok(Some(1)));
    assert_eq!(heap.object_strong_count(target), Ok(2));
    drop(prepared);
    assert_eq!(heap.array_dense_len(array), Ok(Some(1)));
    assert_eq!(heap.object_strong_count(target), Ok(2));

    let prepared = heap.prepare_array_dense_truncation(array, 0).unwrap();
    heap.commit_array_dense_truncation(prepared).unwrap();
    assert_eq!(heap.array_dense_len(array), Ok(Some(0)));
    assert_eq!(heap.object_strong_count(target), Ok(1));

    heap.release_object(array).unwrap();
    heap.release_object(target).unwrap();
    heap.release_shape(array_shape).unwrap();
    heap.release_shape(target_shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn iterator_payload_mutations_retain_replacements_and_completion_keeps_edges() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let root = leaf(&mut heap, shape);
    let realm = heap
        .allocate_context(ContextData::new(
            root, root, root, root, root, root, root, root,
        ))
        .unwrap();
    let source = leaf(&mut heap, shape);
    let replacement_source = leaf(&mut heap, shape);
    let next = leaf(&mut heap, shape);
    let replacement_next = leaf(&mut heap, shape);
    let inner = leaf(&mut heap, shape);
    let replacement_inner = leaf(&mut heap, shape);
    let callback = heap
        .allocate_object(ObjectData::bound_native_function(
            shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            realm,
            1,
        ))
        .unwrap();
    let replacement_callback = heap
        .allocate_object(ObjectData::bound_native_function(
            shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            realm,
            1,
        ))
        .unwrap();

    let helper = heap
        .allocate_object(ObjectData::iterator_helper(
            shape,
            Vec::new(),
            IteratorHelperData {
                source,
                next: RawValue::Object(next),
                callback: RawValue::Object(callback),
                inner: Some(inner),
                count: 0,
                kind: IteratorHelperKind::FlatMap,
                executing: false,
                done: false,
            },
        ))
        .unwrap();
    for edge in [source, next, callback, inner] {
        assert_eq!(heap.object_strong_count(edge), Ok(2));
    }

    heap.set_iterator_helper_source(helper, replacement_source)
        .unwrap();
    heap.set_iterator_helper_next(helper, RawValue::Object(replacement_next))
        .unwrap();
    heap.set_iterator_helper_callback(helper, RawValue::Object(replacement_callback))
        .unwrap();
    heap.set_iterator_helper_inner(helper, Some(replacement_inner))
        .unwrap();
    for edge in [source, next, callback, inner] {
        assert_eq!(heap.object_strong_count(edge), Ok(1));
    }
    for edge in [
        replacement_source,
        replacement_next,
        replacement_callback,
        replacement_inner,
    ] {
        assert_eq!(heap.object_strong_count(edge), Ok(2));
    }

    heap.set_iterator_helper_count(helper, 7).unwrap();
    heap.set_iterator_helper_running(helper, true).unwrap();
    heap.set_iterator_helper_done_and_running(helper, true, false)
        .unwrap();
    assert_eq!(
        heap.iterator_helper_state(helper).unwrap(),
        IteratorHelperData {
            source: replacement_source,
            next: RawValue::Object(replacement_next),
            callback: RawValue::Object(replacement_callback),
            inner: Some(replacement_inner),
            count: 7,
            kind: IteratorHelperKind::FlatMap,
            executing: false,
            done: true,
        }
    );
    for edge in [
        replacement_source,
        replacement_next,
        replacement_callback,
        replacement_inner,
    ] {
        assert_eq!(heap.object_strong_count(edge), Ok(2));
    }

    heap.release_object(helper).unwrap();
    for edge in [
        replacement_source,
        replacement_next,
        replacement_callback,
        replacement_inner,
    ] {
        assert_eq!(heap.object_strong_count(edge), Ok(1));
    }

    for object in [
        source,
        replacement_source,
        next,
        replacement_next,
        inner,
        replacement_inner,
        callback,
        replacement_callback,
    ] {
        heap.release_object(object).unwrap();
    }
    heap.release_context(realm).unwrap();
    heap.release_object(root).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn iterator_wrap_source_and_cached_next_are_owned_edges() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let source = leaf(&mut heap, shape);
    let next = leaf(&mut heap, shape);
    let replacement_source = leaf(&mut heap, shape);
    let replacement_next = leaf(&mut heap, shape);
    let wrapper = heap
        .allocate_object(ObjectData::iterator_wrap(
            shape,
            Vec::new(),
            RawValue::Object(source),
            RawValue::Object(next),
        ))
        .unwrap();

    assert_eq!(heap.object_strong_count(source), Ok(2));
    assert_eq!(heap.object_strong_count(next), Ok(2));
    heap.set_iterator_wrap_source(wrapper, RawValue::Object(replacement_source))
        .unwrap();
    heap.set_iterator_wrap_next(wrapper, RawValue::Object(replacement_next))
        .unwrap();
    assert_eq!(
        heap.iterator_wrap_state(wrapper),
        Ok((
            RawValue::Object(replacement_source),
            RawValue::Object(replacement_next)
        ))
    );
    assert_eq!(heap.object_strong_count(source), Ok(1));
    assert_eq!(heap.object_strong_count(next), Ok(1));
    assert_eq!(heap.object_strong_count(replacement_source), Ok(2));
    assert_eq!(heap.object_strong_count(replacement_next), Ok(2));

    heap.release_object(wrapper).unwrap();
    assert_eq!(heap.object_strong_count(replacement_source), Ok(1));
    assert_eq!(heap.object_strong_count(replacement_next), Ok(1));

    let symbol = Atom::from_immediate_integer(17).unwrap();
    let symbol_wrapper = heap
        .allocate_object(ObjectData::iterator_wrap(
            shape,
            Vec::new(),
            RawValue::Object(source),
            RawValue::Symbol(symbol),
        ))
        .unwrap();
    let cleanup = heap
        .set_iterator_wrap_next(symbol_wrapper, RawValue::Undefined)
        .unwrap();
    assert_eq!(cleanup.atoms, vec![symbol]);
    heap.release_object(symbol_wrapper).unwrap();

    let source_symbol = Atom::from_immediate_integer(19).unwrap();
    let primitive_wrapper = heap
        .allocate_object(ObjectData::iterator_wrap(
            shape,
            Vec::new(),
            RawValue::Symbol(source_symbol),
            RawValue::Undefined,
        ))
        .unwrap();
    assert_eq!(
        heap.iterator_wrap_state(primitive_wrapper),
        Ok((RawValue::Symbol(source_symbol), RawValue::Undefined))
    );
    let cleanup = heap.release_object(primitive_wrapper).unwrap();
    assert_eq!(cleanup.atoms, vec![source_symbol]);

    for object in [source, next, replacement_source, replacement_next] {
        heap.release_object(object).unwrap();
    }
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn async_from_sync_iterator_owns_source_cached_next_and_symbol_atom() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let source = leaf(&mut heap, shape);
    let next = leaf(&mut heap, shape);
    let wrapper = heap
        .allocate_object(ObjectData::async_from_sync_iterator(
            shape,
            Vec::new(),
            source,
            RawValue::Object(next),
        ))
        .unwrap();

    assert_eq!(heap.object_strong_count(source), Ok(2));
    assert_eq!(heap.object_strong_count(next), Ok(2));
    assert_eq!(
        heap.async_from_sync_iterator_state(wrapper),
        Ok((source, RawValue::Object(next)))
    );
    heap.release_object(wrapper).unwrap();
    assert_eq!(heap.object_strong_count(source), Ok(1));
    assert_eq!(heap.object_strong_count(next), Ok(1));

    let symbol = Atom::from_immediate_integer(29).unwrap();
    let symbol_wrapper = heap
        .allocate_object(ObjectData::async_from_sync_iterator(
            shape,
            Vec::new(),
            source,
            RawValue::Symbol(symbol),
        ))
        .unwrap();
    let cleanup = heap.release_object(symbol_wrapper).unwrap();
    assert_eq!(cleanup.atoms, vec![symbol]);
    assert_eq!(heap.object_strong_count(source), Ok(1));

    heap.release_object(source).unwrap();
    heap.release_object(next).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn iterator_concat_releases_consumed_and_drained_edges_at_quickjs_boundaries() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let root = leaf(&mut heap, shape);
    let realm = heap
        .allocate_context(ContextData::new(
            root, root, root, root, root, root, root, root,
        ))
        .unwrap();
    let iterable_a = leaf(&mut heap, shape);
    let iterable_b = leaf(&mut heap, shape);
    let method_a = heap
        .allocate_object(ObjectData::bound_native_function(
            shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            realm,
            1,
        ))
        .unwrap();
    let method_b = heap
        .allocate_object(ObjectData::bound_native_function(
            shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            realm,
            1,
        ))
        .unwrap();
    let active_a = leaf(&mut heap, shape);
    let next_a = heap
        .allocate_object(ObjectData::bound_native_function(
            shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            realm,
            1,
        ))
        .unwrap();
    let active_b = leaf(&mut heap, shape);
    let next_b = heap
        .allocate_object(ObjectData::bound_native_function(
            shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            realm,
            1,
        ))
        .unwrap();
    let concat = heap
        .allocate_object(ObjectData::iterator_concat(
            shape,
            Vec::new(),
            vec![
                Some(IteratorConcatItem {
                    iterable: iterable_a,
                    method: RawValue::Object(method_a),
                }),
                Some(IteratorConcatItem {
                    iterable: iterable_b,
                    method: RawValue::Object(method_b),
                }),
            ],
        ))
        .unwrap();

    for edge in [iterable_a, method_a, iterable_b, method_b] {
        assert_eq!(heap.object_strong_count(edge), Ok(2));
    }
    heap.set_iterator_concat_iterator(concat, Some(active_a))
        .unwrap();
    heap.set_iterator_concat_next(concat, RawValue::Object(next_a))
        .unwrap();
    assert_eq!(heap.object_strong_count(active_a), Ok(2));
    assert_eq!(heap.object_strong_count(next_a), Ok(2));

    heap.advance_iterator_concat(concat).unwrap();
    for edge in [iterable_a, method_a, active_a, next_a] {
        assert_eq!(heap.object_strong_count(edge), Ok(1));
    }
    for edge in [iterable_b, method_b] {
        assert_eq!(heap.object_strong_count(edge), Ok(2));
    }
    assert_eq!(
        heap.iterator_concat_state(concat).unwrap(),
        IteratorConcatData {
            items: vec![
                None,
                Some(IteratorConcatItem {
                    iterable: iterable_b,
                    method: RawValue::Object(method_b),
                }),
            ],
            index: 1,
            iterator: None,
            next: RawValue::Undefined,
            running: false,
        }
    );

    heap.set_iterator_concat_iterator(concat, Some(active_b))
        .unwrap();
    heap.set_iterator_concat_next(concat, RawValue::Object(next_b))
        .unwrap();
    heap.clear_iterator_concat(concat).unwrap();
    for edge in [iterable_b, method_b, active_b, next_b] {
        assert_eq!(heap.object_strong_count(edge), Ok(1));
    }
    assert_eq!(
        heap.iterator_concat_state(concat).unwrap(),
        IteratorConcatData {
            items: vec![None, None],
            index: 2,
            iterator: None,
            next: RawValue::Undefined,
            running: false,
        }
    );

    heap.release_object(concat).unwrap();
    for object in [
        iterable_a, method_a, active_a, next_a, iterable_b, method_b, active_b, next_b,
    ] {
        heap.release_object(object).unwrap();
    }
    heap.release_context(realm).unwrap();
    heap.release_object(root).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn iterator_intrinsics_attach_transactionally_and_form_a_collectable_realm_cycle() {
    let mut heap = Heap::new();
    let empty = empty_shape(&mut heap);
    let root = leaf(&mut heap, empty);
    let realm = heap
        .allocate_context(ContextData::new(
            root, root, root, root, root, root, root, root,
        ))
        .unwrap();
    let intrinsic_shape = heap
        .allocate_shape(Shape::new(Some(root), []).unwrap())
        .unwrap();
    let constructor = heap
        .allocate_object(ObjectData::bound_native_function(
            intrinsic_shape,
            Vec::new(),
            NativeFunctionId::IteratorConstructor,
            realm,
            0,
        ))
        .unwrap();
    let concat_prototype = leaf(&mut heap, intrinsic_shape);
    let helper_prototype = leaf(&mut heap, intrinsic_shape);
    let wrap_prototype = leaf(&mut heap, intrinsic_shape);
    let iterator = IteratorRealmData {
        constructor,
        concat_prototype,
        helper_prototype,
        wrap_prototype,
    };
    let constructor_strong = heap.object_strong_count(constructor).unwrap();
    let concat_strong = heap.object_strong_count(concat_prototype).unwrap();
    let helper_strong = heap.object_strong_count(helper_prototype).unwrap();
    let wrap_strong = heap.object_strong_count(wrap_prototype).unwrap();

    assert_eq!(
        heap.attach_iterator_intrinsics(
            realm,
            IteratorRealmData {
                helper_prototype: root,
                ..iterator
            },
        ),
        Err(HeapError::Invariant(
            "Iterator Helper prototype is not an ordinary child of the realm's Iterator prototype",
        ))
    );
    assert_eq!(heap.context(realm).unwrap().iterator, None);

    heap.live_node_mut(RawId::Object(wrap_prototype))
        .unwrap()
        .strong = u32::MAX;
    assert_eq!(
        heap.attach_iterator_intrinsics(realm, iterator),
        Err(HeapError::Overflow {
            operation: "retaining outgoing heap edges",
        })
    );
    assert_eq!(heap.context(realm).unwrap().iterator, None);
    assert_eq!(
        heap.object_strong_count(constructor),
        Ok(constructor_strong)
    );
    assert_eq!(
        heap.object_strong_count(concat_prototype),
        Ok(concat_strong)
    );
    assert_eq!(
        heap.object_strong_count(helper_prototype),
        Ok(helper_strong)
    );
    heap.live_node_mut(RawId::Object(wrap_prototype))
        .unwrap()
        .strong = wrap_strong;

    heap.attach_iterator_intrinsics(realm, iterator).unwrap();
    assert_eq!(heap.context(realm).unwrap().iterator, Some(iterator));
    assert_eq!(
        heap.object_strong_count(constructor),
        Ok(constructor_strong + 1)
    );
    assert_eq!(
        heap.object_strong_count(concat_prototype),
        Ok(concat_strong + 1)
    );
    assert_eq!(
        heap.object_strong_count(helper_prototype),
        Ok(helper_strong + 1)
    );
    assert_eq!(
        heap.object_strong_count(wrap_prototype),
        Ok(wrap_strong + 1)
    );
    assert!(matches!(
        heap.attach_iterator_intrinsics(realm, iterator),
        Err(HeapError::Invariant(
            "context already has Iterator intrinsic roots"
        ))
    ));

    heap.release_object(constructor).unwrap();
    heap.release_object(concat_prototype).unwrap();
    heap.release_object(helper_prototype).unwrap();
    heap.release_object(wrap_prototype).unwrap();
    heap.release_context(realm).unwrap();
    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.cleanup.finalized_contexts, 1);
    assert_eq!(stats.cleanup.finalized_objects, 4);

    heap.release_shape(intrinsic_shape).unwrap();
    heap.release_object(root).unwrap();
    heap.release_shape(empty).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn iterator_native_descriptors_match_quickjs_protocols() {
    assert_eq!(
        NativeFunctionId::IteratorConstructor.descriptor().cproto,
        NativeCProto::ConstructorOrFunction
    );
    for target in [
        NativeFunctionId::IteratorConcat,
        NativeFunctionId::IteratorFrom,
        NativeFunctionId::IteratorConstructorAccessor,
        NativeFunctionId::IteratorPrototypeCreateHelper(IteratorHelperKind::Drop),
        NativeFunctionId::IteratorPrototypeConsume(IteratorConsumerKind::Every),
        NativeFunctionId::IteratorPrototypeReduce,
        NativeFunctionId::IteratorPrototypeToArray,
        NativeFunctionId::IteratorHelperResume(IteratorResumeKind::Next),
        NativeFunctionId::IteratorWrapResume(IteratorResumeKind::Return),
        NativeFunctionId::IteratorConcatReturn,
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
    }
    assert_eq!(
        NativeFunctionId::IteratorConcatNext.descriptor().cproto,
        NativeCProto::IteratorNext
    );
    assert!(
        !NativeFunctionId::IteratorConcatNext
            .descriptor()
            .cproto
            .default_is_constructor()
    );
}

struct RegExpFixture {
    heap: Heap,
    empty_shape: ShapeId,
    root: ObjectId,
    realm: ContextId,
    prototype_shape: ShapeId,
    prototype: ObjectId,
    string_iterator_prototype: ObjectId,
    function_shape: ShapeId,
    constructor: ObjectId,
    object_shape: ShapeId,
    last_index: Atom,
}

impl RegExpFixture {
    const fn realm_data(&self) -> RegExpRealmData {
        RegExpRealmData {
            prototype: self.prototype,
            constructor: self.constructor,
            string_iterator_prototype: self.string_iterator_prototype,
            object_shape: self.object_shape,
        }
    }

    fn dispose(mut self) -> GcStats {
        self.heap.release_shape(self.object_shape).unwrap();
        self.heap.release_object(self.prototype).unwrap();
        self.heap
            .release_object(self.string_iterator_prototype)
            .unwrap();
        self.heap.release_object(self.constructor).unwrap();
        self.heap.release_context(self.realm).unwrap();
        let stats = collect_heap(&mut self.heap).unwrap();

        self.heap.release_shape(self.function_shape).unwrap();
        self.heap.release_shape(self.prototype_shape).unwrap();
        self.heap.release_object(self.root).unwrap();
        self.heap.release_shape(self.empty_shape).unwrap();
        assert_eq!(self.heap.counts().live, 0);
        stats
    }
}

fn regexp_fixture() -> RegExpFixture {
    let mut heap = Heap::new();
    let empty_shape = empty_shape(&mut heap);
    let root = leaf(&mut heap, empty_shape);
    let realm = heap
        .allocate_context(ContextData::new(
            root, root, root, root, root, root, root, root,
        ))
        .unwrap();
    let prototype_shape = heap
        .allocate_shape(Shape::new(Some(root), []).unwrap())
        .unwrap();
    let prototype = heap
        .allocate_object(ObjectData::ordinary(prototype_shape, Vec::new()))
        .unwrap();
    let string_iterator_prototype = heap
        .allocate_object(ObjectData::ordinary(prototype_shape, Vec::new()))
        .unwrap();
    let function_shape = heap
        .allocate_shape(Shape::new(Some(root), []).unwrap())
        .unwrap();
    let constructor = heap
        .allocate_object(ObjectData::bound_native_function(
            function_shape,
            Vec::new(),
            NativeFunctionId::RegExp(RegExpNativeKind::Constructor),
            realm,
            2,
        ))
        .unwrap();
    let last_index = Atom::from_immediate_integer(0).unwrap();
    let object_shape = heap
        .allocate_shape(
            Shape::new(
                Some(prototype),
                [ShapeEntry {
                    atom: last_index,
                    flags: PropertyFlags::data(true, false, false),
                }],
            )
            .unwrap(),
        )
        .unwrap();
    RegExpFixture {
        heap,
        empty_shape,
        root,
        realm,
        prototype_shape,
        prototype,
        string_iterator_prototype,
        function_shape,
        constructor,
        object_shape,
        last_index,
    }
}

#[test]
fn regexp_intrinsics_attach_transactionally_once_and_finalize_with_realm() {
    let mut fixture = regexp_fixture();
    let realm_data = fixture.realm_data();
    let prototype_strong = fixture.heap.object_strong_count(fixture.prototype).unwrap();
    let constructor_strong = fixture
        .heap
        .object_strong_count(fixture.constructor)
        .unwrap();
    let string_iterator_prototype_strong = fixture
        .heap
        .object_strong_count(fixture.string_iterator_prototype)
        .unwrap();
    let object_shape_strong = fixture
        .heap
        .shape_strong_count(fixture.object_shape)
        .unwrap();

    fixture
        .heap
        .live_node_mut(RawId::Shape(fixture.object_shape))
        .unwrap()
        .strong = u32::MAX;
    assert_eq!(
        fixture
            .heap
            .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index,),
        Err(HeapError::Overflow {
            operation: "retaining outgoing heap edges",
        })
    );
    assert_eq!(fixture.heap.context(fixture.realm).unwrap().regexp, None);
    assert_eq!(
        fixture.heap.object_strong_count(fixture.prototype).unwrap(),
        prototype_strong
    );
    assert_eq!(
        fixture
            .heap
            .object_strong_count(fixture.constructor)
            .unwrap(),
        constructor_strong
    );
    assert_eq!(
        fixture
            .heap
            .object_strong_count(fixture.string_iterator_prototype)
            .unwrap(),
        string_iterator_prototype_strong
    );
    fixture
        .heap
        .live_node_mut(RawId::Shape(fixture.object_shape))
        .unwrap()
        .strong = object_shape_strong;

    fixture
        .heap
        .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
        .unwrap();
    assert_eq!(
        fixture.heap.context(fixture.realm).unwrap().regexp,
        Some(realm_data)
    );
    assert_eq!(
        fixture.heap.object_strong_count(fixture.prototype).unwrap(),
        prototype_strong + 1
    );
    assert_eq!(
        fixture
            .heap
            .object_strong_count(fixture.constructor)
            .unwrap(),
        constructor_strong + 1
    );
    assert_eq!(
        fixture
            .heap
            .object_strong_count(fixture.string_iterator_prototype)
            .unwrap(),
        string_iterator_prototype_strong + 1
    );
    assert_eq!(
        fixture
            .heap
            .shape_strong_count(fixture.object_shape)
            .unwrap(),
        object_shape_strong + 1
    );

    assert_eq!(
        fixture
            .heap
            .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index,),
        Err(HeapError::Invariant(
            "context already has RegExp intrinsic roots"
        ))
    );
    assert_eq!(
        fixture
            .heap
            .shape_strong_count(fixture.object_shape)
            .unwrap(),
        object_shape_strong + 1
    );

    let stats = fixture.dispose();
    assert_eq!(stats.cleanup.finalized_contexts, 1);
    assert_eq!(stats.cleanup.finalized_objects, 3);
    assert_eq!(stats.cleanup.finalized_shapes, 1);
}

#[test]
fn regexp_intrinsics_reject_mismatched_constructor_prototype_and_shape() {
    let mut fixture = regexp_fixture();
    let realm_data = fixture.realm_data();

    {
        let ObjectPayload::NativeFunction { data, .. } = &mut fixture
            .heap
            .object_mut(fixture.constructor)
            .unwrap()
            .payload
        else {
            panic!("fixture constructor must be a native function");
        };
        data.target = NativeFunctionId::Date(DateNativeKind::Constructor);
    }
    assert!(
        fixture
            .heap
            .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
            .is_err()
    );
    {
        let ObjectPayload::NativeFunction { data, .. } = &mut fixture
            .heap
            .object_mut(fixture.constructor)
            .unwrap()
            .payload
        else {
            panic!("fixture constructor must be a native function");
        };
        data.target = NativeFunctionId::RegExp(RegExpNativeKind::Constructor);
    }

    {
        let ObjectPayload::NativeFunction { data, .. } = &mut fixture
            .heap
            .object_mut(fixture.constructor)
            .unwrap()
            .payload
        else {
            panic!("fixture constructor must be a native function");
        };
        data.realm = None;
    }
    assert!(
        fixture
            .heap
            .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
            .is_err()
    );
    {
        let ObjectPayload::NativeFunction { data, .. } = &mut fixture
            .heap
            .object_mut(fixture.constructor)
            .unwrap()
            .payload
        else {
            panic!("fixture constructor must be a native function");
        };
        data.realm = Some(fixture.realm);
    }

    fixture.heap.object_mut(fixture.prototype).unwrap().payload =
        ObjectPayload::RegExp(RegExpObjectData::Uninitialized);
    assert!(
        fixture
            .heap
            .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
            .is_err()
    );
    fixture.heap.object_mut(fixture.prototype).unwrap().payload = ObjectPayload::Ordinary;

    fixture
        .heap
        .object_mut(fixture.string_iterator_prototype)
        .unwrap()
        .payload = ObjectPayload::RegExp(RegExpObjectData::Uninitialized);
    assert!(
        fixture
            .heap
            .attach_regexp_intrinsics(fixture.realm, realm_data, fixture.last_index)
            .is_err()
    );
    fixture
        .heap
        .object_mut(fixture.string_iterator_prototype)
        .unwrap()
        .payload = ObjectPayload::Ordinary;

    assert!(
        fixture
            .heap
            .attach_regexp_intrinsics(
                fixture.realm,
                RegExpRealmData {
                    string_iterator_prototype: fixture.root,
                    ..realm_data
                },
                fixture.last_index,
            )
            .is_err()
    );

    let wrong_prototype_shape = fixture
        .heap
        .allocate_shape(
            Shape::new(
                Some(fixture.root),
                [ShapeEntry {
                    atom: fixture.last_index,
                    flags: PropertyFlags::data(true, false, false),
                }],
            )
            .unwrap(),
        )
        .unwrap();
    assert!(
        fixture
            .heap
            .attach_regexp_intrinsics(
                fixture.realm,
                RegExpRealmData {
                    object_shape: wrong_prototype_shape,
                    ..realm_data
                },
                fixture.last_index,
            )
            .is_err()
    );
    fixture.heap.release_shape(wrong_prototype_shape).unwrap();

    let wrong_flags_shape = fixture
        .heap
        .allocate_shape(
            Shape::new(
                Some(fixture.prototype),
                [ShapeEntry {
                    atom: fixture.last_index,
                    flags: DATA_FLAGS,
                }],
            )
            .unwrap(),
        )
        .unwrap();
    assert!(
        fixture
            .heap
            .attach_regexp_intrinsics(
                fixture.realm,
                RegExpRealmData {
                    object_shape: wrong_flags_shape,
                    ..realm_data
                },
                fixture.last_index,
            )
            .is_err()
    );
    fixture.heap.release_shape(wrong_flags_shape).unwrap();

    let wrong_atom = Atom::from_immediate_integer(1).unwrap();
    assert!(
        fixture
            .heap
            .attach_regexp_intrinsics(fixture.realm, realm_data, wrong_atom)
            .is_err()
    );
    assert_eq!(fixture.heap.context(fixture.realm).unwrap().regexp, None);
    fixture.dispose();
}

#[test]
fn retain_and_release_leaf_uses_explicit_strong_count() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let object = leaf(&mut heap, shape);
    assert_eq!(heap.object_strong_count(object), Ok(1));
    assert_eq!(heap.shape_strong_count(shape), Ok(2));

    heap.retain_object(object).unwrap();
    assert_eq!(heap.object_strong_count(object), Ok(2));
    assert_eq!(heap.release_object(object).unwrap(), HeapCleanup::default());
    assert_eq!(heap.object_strong_count(object), Ok(1));

    let cleanup = heap.release_object(object).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert!(matches!(heap.object(object), Err(HeapError::Stale { .. })));
    assert_eq!(heap.shape_strong_count(shape), Ok(1));
    assert_eq!(heap.release_shape(shape).unwrap().finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn mapped_arguments_payload_roots_varrefs_and_tracks_fast_state() {
    let mut heap = Heap::new();
    let shape = one_slot_shape(&mut heap);
    let cell = heap
        .allocate_var_ref(VarRefData::local(RawValue::Int(7)))
        .unwrap();
    let arguments = heap
        .allocate_object(ObjectData::arguments(
            shape,
            vec![PropertySlot::VarRef(cell)],
            true,
            1,
        ))
        .unwrap();
    assert_eq!(heap.arguments_state(arguments), Ok((true, Some(1))));
    assert_eq!(heap.var_ref_strong_count(cell), Ok(2));

    heap.set_arguments_fast_len(arguments, None).unwrap();
    assert_eq!(heap.arguments_state(arguments), Ok((true, None)));
    heap.release_var_ref(cell).unwrap();
    assert_eq!(heap.var_ref_strong_count(cell), Ok(1));

    let cleanup = heap.release_object(arguments).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_var_refs, 1);
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn context_roots_intrinsic_prototypes_until_realm_finalization() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let object_prototype = leaf(&mut heap, shape);
    let function_prototype = leaf(&mut heap, shape);
    let array_prototype = leaf(&mut heap, shape);
    let iterator_prototype = leaf(&mut heap, shape);
    let array_iterator_prototype = leaf(&mut heap, shape);
    let string_iterator_prototype = leaf(&mut heap, shape);
    let date_prototype = leaf(&mut heap, shape);
    let global_object = leaf(&mut heap, shape);
    let global_var_object = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(
            ContextData::new(
                object_prototype,
                function_prototype,
                array_prototype,
                iterator_prototype,
                array_iterator_prototype,
                string_iterator_prototype,
                global_object,
                global_var_object,
            )
            .with_date_prototype(date_prototype),
        )
        .unwrap();
    assert_eq!(
        heap.context(context).unwrap().array_prototype,
        array_prototype
    );
    assert_eq!(
        heap.context(context).unwrap().iterator_prototype,
        iterator_prototype
    );
    assert_eq!(
        heap.context(context).unwrap().array_iterator_prototype,
        array_iterator_prototype
    );
    assert_eq!(
        heap.context(context).unwrap().string_iterator_prototype,
        string_iterator_prototype
    );
    assert_eq!(
        heap.context(context).unwrap().date_prototype,
        Some(date_prototype)
    );
    assert!(matches!(
        heap.object(date_prototype).unwrap().payload,
        ObjectPayload::Ordinary
    ));

    for object in [
        object_prototype,
        function_prototype,
        array_prototype,
        iterator_prototype,
        array_iterator_prototype,
        string_iterator_prototype,
        date_prototype,
        global_object,
        global_var_object,
    ] {
        assert_eq!(heap.release_object(object).unwrap(), HeapCleanup::default());
        assert_eq!(heap.object_strong_count(object), Ok(1));
    }
    let cleanup = heap.release_context(context).unwrap();
    assert_eq!(cleanup.finalized_contexts, 1);
    assert_eq!(cleanup.finalized_objects, 9);
    assert_eq!(heap.release_shape(shape).unwrap().finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn hundred_thousand_node_chain_finalizes_iteratively() {
    const LENGTH: usize = 100_000;

    let mut heap = Heap::new();
    let shape = one_slot_shape(&mut heap);
    let mut head = heap
        .allocate_object(ObjectData::ordinary(
            shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();

    for _ in 1..LENGTH {
        let next = heap
            .allocate_object(ObjectData::ordinary(
                shape,
                vec![PropertySlot::Data(RawValue::Object(head))],
            ))
            .unwrap();
        assert_eq!(heap.release_object(head).unwrap(), HeapCleanup::default());
        head = next;
    }

    let cleanup = heap.release_object(head).unwrap();
    assert_eq!(cleanup.finalized_objects, LENGTH);
    assert_eq!(heap.counts().object_nodes, 0);
    assert_eq!(heap.release_shape(shape).unwrap().finalized_shapes, 1);
}

#[test]
fn self_cycle_requires_trial_deletion() {
    let mut heap = Heap::new();
    let shape = one_slot_shape(&mut heap);
    let object = heap
        .allocate_object(ObjectData::ordinary(
            shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();
    heap.replace_object_slot(object, 0, PropertySlot::Data(RawValue::Object(object)))
        .unwrap();

    // The object now owns the only shape reference, matching a runtime
    // shape cache whose entry is weak.
    assert_eq!(heap.release_shape(shape).unwrap(), HeapCleanup::default());

    assert_eq!(heap.release_object(object).unwrap(), HeapCleanup::default());
    assert_eq!(heap.object_strong_count(object), Ok(1));
    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.candidate_nodes, 2);
    assert_eq!(stats.cleanup.finalized_objects, 1);
    assert_eq!(stats.cleanup.finalized_shapes, 1);
    assert!(matches!(heap.object(object), Err(HeapError::Stale { .. })));
}

#[test]
fn object_slot_replacement_rejects_private_name_payloads() {
    let mut heap = Heap::new();
    let shape = one_slot_shape(&mut heap);
    let object = heap
        .allocate_object(ObjectData::ordinary(
            shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();
    let private = Atom::from_raw(91);

    assert_eq!(
        heap.replace_object_slot(object, 0, PropertySlot::Data(RawValue::Private(private)),),
        Err(HeapError::Invariant(
            "private-name identity escaped into an object value slot"
        ))
    );
    assert!(matches!(
        heap.object(object).unwrap().slots[0],
        PropertySlot::Data(RawValue::Undefined)
    ));

    assert_eq!(heap.release_object(object).unwrap().finalized_objects, 1);
    assert_eq!(heap.release_shape(shape).unwrap().finalized_shapes, 1);
}

#[test]
fn two_node_cycle_requires_trial_deletion() {
    let mut heap = Heap::new();
    let shape = one_slot_shape(&mut heap);
    let first = heap
        .allocate_object(ObjectData::ordinary(
            shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();
    let second = heap
        .allocate_object(ObjectData::ordinary(
            shape,
            vec![PropertySlot::Data(RawValue::Object(first))],
        ))
        .unwrap();
    heap.replace_object_slot(first, 0, PropertySlot::Data(RawValue::Object(second)))
        .unwrap();

    assert_eq!(heap.release_shape(shape).unwrap(), HeapCleanup::default());

    heap.release_object(first).unwrap();
    heap.release_object(second).unwrap();
    assert_eq!(heap.counts().object_nodes, 2);

    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.cleanup.finalized_objects, 2);
    assert_eq!(stats.cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().object_nodes, 0);
}

#[test]
fn external_root_preserves_cycle_closure_until_released() {
    let mut heap = Heap::new();
    let shape = one_slot_shape(&mut heap);
    let first = heap
        .allocate_object(ObjectData::ordinary(
            shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();
    let second = heap
        .allocate_object(ObjectData::ordinary(
            shape,
            vec![PropertySlot::Data(RawValue::Object(first))],
        ))
        .unwrap();
    heap.replace_object_slot(first, 0, PropertySlot::Data(RawValue::Object(second)))
        .unwrap();

    assert_eq!(heap.release_shape(shape).unwrap(), HeapCleanup::default());

    heap.retain_object(first).unwrap();
    heap.release_object(first).unwrap();
    heap.release_object(second).unwrap();

    let preserved = collect_heap(&mut heap).unwrap();
    assert_eq!(preserved.candidate_nodes, 0);
    assert_eq!(preserved.external_root_nodes, 1);
    assert_eq!(heap.counts().object_nodes, 2);

    heap.release_object(first).unwrap();
    let collected = collect_heap(&mut heap).unwrap();
    assert_eq!(collected.cleanup.finalized_objects, 2);
    assert_eq!(collected.cleanup.finalized_shapes, 1);
}

#[test]
fn reclaimed_generation_rejects_stale_handles() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let stale = leaf(&mut heap, shape);
    let index = stale.debug_index();
    let generation = stale.debug_generation();
    heap.release_object(stale).unwrap();

    let replacement = leaf(&mut heap, shape);
    assert_eq!(replacement.debug_index(), index);
    assert_eq!(replacement.debug_generation(), generation + 1);
    assert!(matches!(
        heap.retain_object(stale),
        Err(HeapError::Stale { .. })
    ));

    heap.release_object(replacement).unwrap();
    heap.release_shape(shape).unwrap();
}

#[test]
fn forged_cross_kind_handle_reports_wrong_kind() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let forged = ObjectId {
        index: shape.index,
        generation: shape.generation,
    };

    assert_eq!(
        heap.object(forged),
        Err(HeapError::WrongKind {
            expected: HeapNodeKind::Object,
            actual: HeapNodeKind::Shape,
        })
    );
    heap.release_shape(shape).unwrap();
}

#[test]
fn shape_prototype_edge_participates_in_cycle_collection() {
    let mut heap = Heap::new();
    let original_shape = empty_shape(&mut heap);
    let object = leaf(&mut heap, original_shape);

    let prototype_shape = heap
        .allocate_shape(Shape::new(Some(object), []).unwrap())
        .unwrap();
    heap.replace_object_layout(object, prototype_shape, Vec::new())
        .unwrap();

    assert_eq!(
        heap.release_shape(original_shape).unwrap().finalized_shapes,
        1
    );
    assert_eq!(
        heap.release_shape(prototype_shape).unwrap(),
        HeapCleanup::default()
    );
    assert_eq!(heap.release_object(object).unwrap(), HeapCleanup::default());

    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.candidate_nodes, 2);
    assert_eq!(stats.cleanup.finalized_objects, 1);
    assert_eq!(stats.cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}
