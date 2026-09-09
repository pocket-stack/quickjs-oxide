use super::*;

#[test]
fn proxy_revocation_releases_only_the_one_shot_closure_capture() {
    let mut heap = Heap::new();
    let null_shape = empty_shape(&mut heap);
    let root = heap
        .allocate_object(ObjectData::ordinary(null_shape, Vec::new()))
        .unwrap();
    let function_shape = heap
        .allocate_shape(Shape::new(Some(root), []).unwrap())
        .unwrap();
    let function_prototype = heap
        .allocate_bootstrap_native_function(ObjectData::native_function(
            function_shape,
            Vec::new(),
            NativeFunctionId::FunctionPrototype,
            0,
        ))
        .unwrap();
    let realm = heap
        .allocate_context(ContextData::new(
            root,
            function_prototype,
            root,
            root,
            root,
            root,
            root,
            root,
        ))
        .unwrap();
    heap.attach_native_function_realm(function_prototype, realm)
        .unwrap();

    let target = heap
        .allocate_object(ObjectData::ordinary(null_shape, Vec::new()))
        .unwrap();
    let handler = heap
        .allocate_object(ObjectData::ordinary(null_shape, Vec::new()))
        .unwrap();
    let proxy = heap
        .allocate_object(ObjectData::proxy(
            null_shape,
            Vec::new(),
            target,
            handler,
            false,
            false,
        ))
        .unwrap();
    assert_eq!(heap.object_strong_count(target), Ok(2));
    assert_eq!(heap.object_strong_count(handler), Ok(2));
    assert_eq!(
        heap.proxy_snapshot(proxy),
        Ok(ProxyData {
            target,
            handler,
            is_callable: false,
            is_revoked: false,
        })
    );

    let revoker = heap
        .allocate_object(ObjectData::bound_internal_native_function(
            function_shape,
            Vec::new(),
            NativeFunctionId::ProxyRevoke,
            realm,
            0,
            InternalCallableData::ProxyRevoke { proxy: Some(proxy) },
        ))
        .unwrap();
    assert_eq!(heap.object_strong_count(proxy), Ok(2));

    let (revoked, cleanup) = heap.revoke_proxy_from_callable(revoker).unwrap();
    assert!(revoked);
    assert_eq!(cleanup, HeapCleanup::default());
    assert_eq!(heap.object_strong_count(proxy), Ok(1));
    assert!(heap.proxy_snapshot(proxy).unwrap().is_revoked);
    assert_eq!(heap.object_strong_count(target), Ok(2));
    assert_eq!(heap.object_strong_count(handler), Ok(2));
    assert_eq!(
        heap.native_internal_callable(revoker),
        Ok(Some(InternalCallableData::ProxyRevoke { proxy: None }))
    );

    let (revoked_again, cleanup) = heap.revoke_proxy_from_callable(revoker).unwrap();
    assert!(!revoked_again);
    assert_eq!(cleanup, HeapCleanup::default());
    assert_eq!(heap.object_strong_count(target), Ok(2));
    assert_eq!(heap.object_strong_count(handler), Ok(2));

    let cleanup = heap.release_object(proxy).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(heap.object_strong_count(target), Ok(1));
    assert_eq!(heap.object_strong_count(handler), Ok(1));
}

#[test]
fn provisional_native_function_is_confined_to_transactional_realm_bootstrap() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let unbound =
        || ObjectData::native_function(shape, Vec::new(), NativeFunctionId::FunctionPrototype, 0);

    assert_eq!(
        heap.allocate_object(unbound()),
        Err(HeapError::Invariant(
            "an unbound native function may only be allocated during realm bootstrap"
        ))
    );
    assert_eq!(heap.counts().object_nodes, 0);

    assert_eq!(
        heap.allocate_bootstrap_native_function(ObjectData::native_function(
            shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            1,
        )),
        Err(HeapError::Invariant(
            "bootstrap native-function allocation requires an unbound Function.prototype"
        ))
    );
    assert_eq!(heap.counts().object_nodes, 0);

    let prototype = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    let regexp = heap
        .allocate_object(ObjectData::regexp(shape, Vec::new()))
        .unwrap();
    let native_shape = heap
        .allocate_shape(Shape::new(Some(prototype), []).unwrap())
        .unwrap();
    let function = heap
        .allocate_bootstrap_native_function(ObjectData::native_function(
            native_shape,
            Vec::new(),
            NativeFunctionId::FunctionPrototype,
            0,
        ))
        .unwrap();
    let realm = heap
        .allocate_context(ContextData::new(
            prototype, function, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();

    heap.attach_native_function_realm(function, realm).unwrap();
    assert_eq!(heap.context_strong_count(realm), Ok(2));
    assert!(heap.attach_native_function_realm(function, realm).is_err());
    assert_eq!(heap.context_strong_count(realm), Ok(2));
    assert!(heap.attach_native_function_realm(prototype, realm).is_err());
    assert_eq!(heap.context_strong_count(realm), Ok(2));
    assert!(heap.attach_native_function_realm(regexp, realm).is_err());
    assert_eq!(heap.context_strong_count(realm), Ok(2));

    heap.release_context(realm).unwrap();
    heap.release_object(function).unwrap();
    heap.release_object(regexp).unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_shape(native_shape).unwrap();
    heap.release_shape(shape).unwrap();
    collect_heap(&mut heap).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn math_random_state_is_realm_local_seeded_and_pinned() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    let first_realm = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    let second_realm = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();

    assert_eq!(
        heap.next_math_random_u64(first_realm),
        Err(HeapError::Invariant(
            "Math.random state was used before initialization"
        ))
    );
    heap.initialize_math_random_state(first_realm, 1).unwrap();
    heap.initialize_math_random_state(second_realm, 1).unwrap();
    assert_eq!(
        heap.initialize_math_random_state(first_realm, 2),
        Err(HeapError::Invariant(
            "Math.random state was initialized more than once"
        ))
    );
    let first = heap.next_math_random_u64(first_realm).unwrap();
    assert_eq!(first, 0x47e4_ce4b_896c_dd1d);
    assert_eq!(heap.next_math_random_u64(second_realm).unwrap(), first);
    let second = heap.next_math_random_u64(first_realm).unwrap();
    assert_eq!(heap.next_math_random_u64(second_realm).unwrap(), second);

    heap.release_context(first_realm).unwrap();
    heap.release_context(second_realm).unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn primitive_object_payload_category_is_structurally_validated() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let number_payload = PrimitiveObjectData::Number(f64::NAN);
    assert_eq!(number_payload.kind(), PrimitiveKind::Number);
    assert_eq!(
        PrimitiveObjectData::String(JsString::try_from_utf16([0x61, 0xd800, 0x62]).unwrap(),)
            .kind(),
        PrimitiveKind::String
    );
    assert_eq!(
        PrimitiveObjectData::Boolean(false).kind(),
        PrimitiveKind::Boolean
    );
    let symbol_atom = Atom::from_immediate_integer(47).unwrap();
    assert_eq!(
        PrimitiveObjectData::Symbol(symbol_atom).kind(),
        PrimitiveKind::Symbol
    );
    assert_eq!(
        PrimitiveObjectData::BigInt(JsBigInt::one()).kind(),
        PrimitiveKind::BigInt
    );

    let mut invalid = ObjectData::primitive(shape, Vec::new(), number_payload.clone());
    invalid.kind = ObjectKind::Ordinary;
    assert_eq!(
        heap.allocate_object(invalid),
        Err(HeapError::Invariant(
            "object kind does not match its class payload"
        ))
    );
    assert_eq!(heap.counts().object_nodes, 0);

    let boolean = heap
        .allocate_object(ObjectData::primitive(
            shape,
            Vec::new(),
            PrimitiveObjectData::Boolean(false),
        ))
        .unwrap();
    assert!(matches!(
        heap.object(boolean).unwrap().payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Boolean(false))
    ));

    let number = heap
        .allocate_object(ObjectData::primitive(shape, Vec::new(), number_payload))
        .unwrap();
    let number_data = heap.object(number).unwrap();
    assert!(matches!(
        &number_data.payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Number(value)) if value.is_nan()
    ));
    assert_eq!(object_edges(number_data), vec![RawId::Shape(shape)]);
    assert_eq!(object_atoms(number_data).count(), 0);

    let string_value = JsString::try_from_utf16([0x61, 0xd800, 0x62]).unwrap();
    let string = heap
        .allocate_object(ObjectData::primitive(
            shape,
            Vec::new(),
            PrimitiveObjectData::String(string_value.clone()),
        ))
        .unwrap();
    let string_data = heap.object(string).unwrap();
    assert!(matches!(
        &string_data.payload,
        ObjectPayload::Primitive(PrimitiveObjectData::String(value))
            if value == &string_value
    ));
    assert_eq!(object_edges(string_data), vec![RawId::Shape(shape)]);
    assert_eq!(object_atoms(string_data).count(), 0);

    let symbol = heap
        .allocate_object(ObjectData::primitive(
            shape,
            Vec::new(),
            PrimitiveObjectData::Symbol(symbol_atom),
        ))
        .unwrap();
    let symbol_data = heap.object(symbol).unwrap();
    assert!(matches!(
        symbol_data.payload,
        ObjectPayload::Primitive(PrimitiveObjectData::Symbol(atom)) if atom == symbol_atom
    ));
    assert_eq!(object_edges(symbol_data), vec![RawId::Shape(shape)]);
    assert_eq!(object_atoms(symbol_data).collect::<Vec<_>>(), [symbol_atom]);

    let bigint = heap
        .allocate_object(ObjectData::primitive(
            shape,
            Vec::new(),
            PrimitiveObjectData::BigInt(JsBigInt::from(i64::MAX)),
        ))
        .unwrap();
    let bigint_data = heap.object(bigint).unwrap();
    assert!(matches!(
        &bigint_data.payload,
        ObjectPayload::Primitive(PrimitiveObjectData::BigInt(value))
            if value == &JsBigInt::from(i64::MAX)
    ));
    assert_eq!(object_edges(bigint_data), vec![RawId::Shape(shape)]);
    assert_eq!(object_atoms(bigint_data).count(), 0);

    heap.release_object(bigint).unwrap();
    let symbol_cleanup = heap.release_object(symbol).unwrap();
    assert_eq!(symbol_cleanup.atoms, [symbol_atom]);
    let string_cleanup = heap.release_object(string).unwrap();
    assert!(string_cleanup.atoms.is_empty());
    heap.release_object(number).unwrap();
    heap.release_object(boolean).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn date_payload_is_branded_edge_free_and_mutable() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);

    let mut invalid = ObjectData::date(shape, Vec::new(), f64::NAN);
    invalid.kind = ObjectKind::Ordinary;
    assert_eq!(
        heap.allocate_object(invalid),
        Err(HeapError::Invariant(
            "object kind does not match its class payload"
        ))
    );
    assert_eq!(heap.counts().object_nodes, 0);

    let date = heap
        .allocate_object(ObjectData::date(shape, Vec::new(), f64::NAN))
        .unwrap();
    assert!(heap.date_value(date).unwrap().is_nan());
    let date_data = heap.object(date).unwrap();
    assert!(matches!(date_data.payload, ObjectPayload::Date(value) if value.is_nan()));
    assert_eq!(object_edges(date_data), vec![RawId::Shape(shape)]);
    assert_eq!(object_atoms(date_data).count(), 0);

    heap.set_date_value(date, -0.0).unwrap();
    assert_eq!(
        heap.date_value(date).unwrap().to_bits(),
        (-0.0f64).to_bits()
    );

    let ordinary = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    assert_eq!(
        heap.date_value(ordinary),
        Err(HeapError::Invariant(
            "Date value requested for an object with the wrong class"
        ))
    );
    assert_eq!(
        heap.set_date_value(ordinary, 1.0),
        Err(HeapError::Invariant(
            "Date value update reached an object with the wrong class"
        ))
    );

    heap.release_object(ordinary).unwrap();
    heap.release_object(date).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn regexp_payload_is_branded_edge_free_and_structurally_validated() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);

    let mut invalid = ObjectData::regexp(shape, Vec::new());
    invalid.kind = ObjectKind::Ordinary;
    assert_eq!(
        heap.allocate_object(invalid),
        Err(HeapError::Invariant(
            "object kind does not match its class payload"
        ))
    );
    assert_eq!(heap.counts().object_nodes, 0);

    let regexp = heap
        .allocate_object(ObjectData::regexp(shape, Vec::new()))
        .unwrap();
    let regexp_object = heap.object(regexp).unwrap();
    assert_eq!(regexp_object.kind, ObjectKind::RegExp);
    assert!(matches!(
        regexp_object.payload,
        ObjectPayload::RegExp(RegExpObjectData::Uninitialized)
    ));
    assert!(matches!(
        heap.regexp_data(regexp),
        Ok(RegExpObjectData::Uninitialized)
    ));
    assert_eq!(object_edges(regexp_object), vec![RawId::Shape(shape)]);
    assert_eq!(object_atoms(regexp_object).count(), 0);

    let ordinary = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    assert_eq!(
        heap.regexp_data(ordinary),
        Err(HeapError::Invariant(
            "RegExp data requested for an object with the wrong class"
        ))
    );
    assert_eq!(
        heap.replace_regexp_data(ordinary, RegExpObjectData::Uninitialized),
        Err(HeapError::Invariant(
            "RegExp data update reached an object with the wrong class"
        ))
    );

    heap.release_object(ordinary).unwrap();
    heap.release_object(regexp).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn compiled_regexp_payload_replacement_and_finalization_release_rc_leaves() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let pattern = JsString::try_from_utf16([u16::from(b'a'), 0xd800, u16::from(b'b')]).unwrap();
    let flags = JsString::from_static("");
    let program = Rc::new(crate::regexp::compile(&pattern, &flags).unwrap());
    assert_eq!(Rc::strong_count(&program), 1);

    let regexp = heap
        .allocate_object(ObjectData::regexp(shape, Vec::new()))
        .unwrap();
    let previous = heap
        .replace_regexp_data(
            regexp,
            RegExpObjectData::Compiled {
                pattern: pattern.clone(),
                program: program.clone(),
            },
        )
        .unwrap();
    assert_eq!(previous, RegExpObjectData::Uninitialized);
    assert_eq!(Rc::strong_count(&program), 2);
    let RegExpObjectData::Compiled {
        pattern: stored_pattern,
        program: stored_program,
    } = heap.regexp_data(regexp).unwrap()
    else {
        panic!("RegExp replacement did not install compiled data");
    };
    assert_eq!(stored_pattern, &pattern);
    assert!(Rc::ptr_eq(stored_program, &program));
    assert_eq!(
        object_edges(heap.object(regexp).unwrap()),
        vec![RawId::Shape(shape)]
    );
    assert_eq!(object_atoms(heap.object(regexp).unwrap()).count(), 0);

    let precompiled = heap
        .allocate_object(ObjectData::compiled_regexp(
            shape,
            Vec::new(),
            pattern.clone(),
            program.clone(),
        ))
        .unwrap();
    assert_eq!(Rc::strong_count(&program), 3);
    heap.release_object(precompiled).unwrap();
    assert_eq!(Rc::strong_count(&program), 2);

    let previous = heap
        .replace_regexp_data(regexp, RegExpObjectData::Uninitialized)
        .unwrap();
    assert_eq!(Rc::strong_count(&program), 2);
    let RegExpObjectData::Compiled {
        pattern: previous_pattern,
        program: previous_program,
    } = previous
    else {
        panic!("RegExp replacement did not return the previous compiled data");
    };
    assert_eq!(previous_pattern, pattern);
    assert!(Rc::ptr_eq(&previous_program, &program));
    drop(previous_program);
    assert_eq!(Rc::strong_count(&program), 1);

    heap.release_object(regexp).unwrap();
    assert_eq!(Rc::strong_count(&program), 1);
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn string_iterator_payload_advances_by_code_point_and_releases_at_end() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let source = JsString::try_from_utf16([
        u16::from(b'A'),
        0xd83d,
        0xde00,
        0xd800,
        u16::from(b'X'),
        0xdc00,
        u16::from(b'Z'),
    ])
    .unwrap();
    let iterator = heap
        .allocate_object(ObjectData::string_iterator(shape, Vec::new(), source))
        .unwrap();

    let mut next = || {
        heap.string_iterator_next(iterator)
            .unwrap()
            .map(|value| value.utf16_units().collect::<Vec<_>>())
    };
    assert_eq!(next(), Some(vec![u16::from(b'A')]));
    assert_eq!(next(), Some(vec![0xd83d, 0xde00]));
    assert_eq!(next(), Some(vec![0xd800]));
    assert_eq!(next(), Some(vec![u16::from(b'X')]));
    assert_eq!(next(), Some(vec![0xdc00]));
    assert_eq!(next(), Some(vec![u16::from(b'Z')]));
    assert_eq!(next(), None);
    assert_eq!(next(), None);
    assert!(matches!(
        &heap.object(iterator).unwrap().payload,
        ObjectPayload::StringIterator {
            string: None,
            next_index: 7
        }
    ));
    assert_eq!(
        object_edges(heap.object(iterator).unwrap()),
        vec![RawId::Shape(shape)]
    );

    heap.release_object(iterator).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn regexp_string_iterator_retains_matcher_after_completion_until_finalization() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let regexp = leaf(&mut heap, shape);
    let string = JsString::from_static("input");
    let iterator = heap
        .allocate_object(ObjectData::regexp_string_iterator(
            shape,
            Vec::new(),
            regexp,
            string.clone(),
            true,
            true,
        ))
        .unwrap();

    assert_eq!(heap.object_strong_count(regexp), Ok(2));
    heap.release_object(regexp).unwrap();
    assert_eq!(heap.object_strong_count(regexp), Ok(1));
    assert_eq!(
        heap.regexp_string_iterator_state(iterator),
        Ok((regexp, string, true, true, false))
    );
    assert_eq!(
        object_edges(heap.object(iterator).unwrap()),
        vec![RawId::Shape(shape), RawId::Object(regexp)]
    );

    heap.finish_regexp_string_iterator(iterator).unwrap();
    assert_eq!(
        heap.regexp_string_iterator_state(iterator),
        Ok((regexp, JsString::from_static("input"), true, true, true))
    );
    assert_eq!(heap.object_strong_count(regexp), Ok(1));
    heap.finish_regexp_string_iterator(iterator).unwrap();
    assert_eq!(heap.object_strong_count(regexp), Ok(1));

    let ordinary = leaf(&mut heap, shape);
    assert!(matches!(
        heap.regexp_string_iterator_state(ordinary),
        Err(HeapError::Invariant(
            "RegExp String Iterator state reached an object with the wrong class"
        ))
    ));
    assert!(matches!(
        heap.finish_regexp_string_iterator(ordinary),
        Err(HeapError::Invariant(
            "RegExp String Iterator completion reached an object with the wrong class"
        ))
    ));
    heap.release_object(ordinary).unwrap();

    let cleanup = heap.release_object(iterator).unwrap();
    assert_eq!(cleanup.finalized_objects, 2);
    assert!(matches!(heap.object(regexp), Err(HeapError::Stale { .. })));
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn array_iterator_payload_retains_its_source_until_completion() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let source = leaf(&mut heap, shape);
    let iterator = heap
        .allocate_object(ObjectData::array_iterator(
            shape,
            Vec::new(),
            source,
            ArrayIteratorKind::KeyAndValue,
        ))
        .unwrap();

    assert_eq!(heap.object_strong_count(source), Ok(2));
    heap.release_object(source).unwrap();
    assert_eq!(heap.object_strong_count(source), Ok(1));
    assert_eq!(
        heap.array_iterator_state(iterator),
        Ok((Some(source), 0, ArrayIteratorKind::KeyAndValue))
    );
    heap.set_array_iterator_index(iterator, 7).unwrap();
    assert_eq!(
        heap.array_iterator_state(iterator),
        Ok((Some(source), 7, ArrayIteratorKind::KeyAndValue))
    );

    let cleanup = heap.finish_array_iterator(iterator).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert!(matches!(heap.object(source), Err(HeapError::Stale { .. })));
    assert_eq!(
        heap.array_iterator_state(iterator),
        Ok((None, 7, ArrayIteratorKind::KeyAndValue))
    );
    assert_eq!(
        heap.finish_array_iterator(iterator).unwrap(),
        HeapCleanup::default()
    );
    assert!(matches!(
        heap.set_array_iterator_index(iterator, 8),
        Err(HeapError::Invariant(
            "completed Array Iterator was advanced"
        ))
    ));

    heap.release_object(iterator).unwrap();
    heap.release_shape(shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}
