use super::*;

#[test]
fn finalized_shapes_unlink_exact_weak_cache_entries() {
    let runtime = Runtime::new();
    let mut objects = Vec::new();
    for index in 0..2_000 {
        let object = runtime.new_object(None).unwrap();
        let key = runtime
            .intern_property_key(&format!("unique-{index}"))
            .unwrap();
        assert!(set_property(&runtime, &object, &key, Value::Int(index)).unwrap());
        objects.push(object);
    }
    assert!(runtime.0.state.borrow().shape_cache.len() >= objects.len());
    drop(objects);
    let state = runtime.0.state.borrow();
    assert!(state.shape_cache.is_empty());
    assert!(state.shape_fingerprints.is_empty());
}

#[test]
fn unique_shape_append_never_mutates_a_shared_shape() {
    let runtime = Runtime::new();
    let first = runtime.new_object(None).unwrap();
    let second = runtime.new_object(None).unwrap();
    let shared_keys = (0..crate::engine::object::properties::MIN_UNIQUE_SHAPE_APPEND_ENTRIES)
        .map(|index| {
            runtime
                .intern_property_key(&format!("shared-{index}"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let b = runtime.intern_property_key("unique-b").unwrap();
    let c = runtime.intern_property_key("unique-c").unwrap();

    for key in &shared_keys {
        assert!(set_property(&runtime, &first, key, Value::Int(1)).unwrap());
        assert!(set_property(&runtime, &second, key, Value::Int(2)).unwrap());
    }
    let shared_shape = {
        let state = runtime.0.state.borrow();
        let first_shape = state.heap.object(first.object_id()).unwrap().shape;
        let second_shape = state.heap.object(second.object_id()).unwrap().shape;
        assert_eq!(first_shape, second_shape);
        assert_eq!(state.heap.shape_strong_count(first_shape), Ok(2));
        first_shape
    };

    assert!(set_property(&runtime, &first, &b, Value::Int(3)).unwrap());
    let unique_shape = runtime
        .0
        .state
        .borrow()
        .heap
        .object(first.object_id())
        .unwrap()
        .shape;
    assert_ne!(unique_shape, shared_shape);
    assert!(set_property(&runtime, &first, &c, Value::Int(4)).unwrap());
    let state = runtime.0.state.borrow();
    assert_eq!(
        state.heap.object(first.object_id()).unwrap().shape,
        unique_shape,
        "the second unique addition should append to the existing shape"
    );
    assert_eq!(
        state.heap.object(second.object_id()).unwrap().shape,
        shared_shape,
        "mutating the unique successor must not alter the shared predecessor"
    );
    assert_eq!(
        state
            .heap
            .shape(shared_shape)
            .unwrap()
            .entries()
            .iter()
            .map(|entry| entry.atom)
            .collect::<Vec<_>>(),
        shared_keys
            .iter()
            .map(PropertyKey::atom)
            .collect::<Vec<_>>()
    );
    let mut unique_atoms = shared_keys
        .iter()
        .map(PropertyKey::atom)
        .collect::<Vec<_>>();
    unique_atoms.extend([b.atom(), c.atom()]);
    assert_eq!(
        state
            .heap
            .shape(unique_shape)
            .unwrap()
            .entries()
            .iter()
            .map(|entry| entry.atom)
            .collect::<Vec<_>>(),
        unique_atoms
    );
    assert!(!state.shape_fingerprints.contains_key(&unique_shape));
}

#[test]
fn unique_shape_append_preserves_key_categories_and_readd_order() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let fillers = (0..5)
        .map(|index| {
            runtime
                .intern_property_key(&format!("unique-fill-{index}"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let beta = runtime.intern_property_key("unique-beta").unwrap();
    let alpha = runtime.intern_property_key("unique-alpha").unwrap();
    let index = runtime.intern_property_key("2").unwrap();
    let symbol = runtime
        .new_symbol(Some(JsString::from_static("unique-symbol")))
        .unwrap();
    let symbol_key = PropertyKey::from(&symbol);
    for key in &fillers {
        assert!(set_property(&runtime, &object, key, Value::Int(0)).unwrap());
    }
    for key in [&beta, &symbol_key, &index, &alpha] {
        assert!(set_property(&runtime, &object, key, Value::Int(1)).unwrap());
    }
    let mut expected = vec![index.clone()];
    expected.extend(fillers.iter().cloned());
    expected.extend([beta.clone(), alpha.clone(), symbol_key.clone()]);
    assert_eq!(runtime.own_property_keys(&object).unwrap(), expected);

    assert!(runtime.delete_property(&object, &beta).unwrap());
    assert!(set_property(&runtime, &object, &beta, Value::Int(2)).unwrap());
    let mut expected = vec![index];
    expected.extend(fillers);
    expected.extend([alpha, beta, symbol_key]);
    assert_eq!(runtime.own_property_keys(&object).unwrap(), expected);
}

#[test]
fn unique_shape_append_releases_its_key_atom_with_the_final_object() {
    let runtime = Runtime::new();
    let object = runtime.new_object(None).unwrap();
    let seeds = (0..crate::engine::object::properties::MIN_UNIQUE_SHAPE_APPEND_ENTRIES)
        .map(|index| {
            runtime
                .intern_property_key(&format!("unique-append-seed-{index}"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let key = runtime
        .intern_property_key("unique-append-final-key")
        .unwrap();
    let atom = key.atom();
    for seed in &seeds {
        assert!(set_property(&runtime, &object, seed, Value::Int(1)).unwrap());
    }
    assert!(set_property(&runtime, &object, &key, Value::Int(2)).unwrap());
    drop(seeds);
    drop(key);
    assert!(
        runtime.0.state.borrow().atoms.resolve(atom).is_ok(),
        "the in-place shape must own its appended key atom"
    );

    drop(object);
    runtime.run_gc().unwrap();
    assert!(
        runtime.0.state.borrow().atoms.resolve(atom).is_err(),
        "final shape collection must release its appended key atom"
    );
}

#[test]
fn failed_unique_shape_append_restores_cache_and_atom_ownership() {
    let runtime = Runtime::new();
    let owner = runtime.new_object(None).unwrap();
    let stale = runtime.new_object(None).unwrap();
    let stale_id = stale.object_id();
    drop(stale);
    let key = runtime.intern_property_key("failed-unique-append").unwrap();
    let atom = key.atom();

    let mut state = runtime.0.state.borrow_mut();
    let shape = state.heap.object(owner.object_id()).unwrap().shape;
    assert_eq!(state.heap.shape_strong_count(shape), Ok(1));
    let before_ref_count = state.atoms.resolve(atom).unwrap().ref_count;
    let fingerprint = state
        .shape_fingerprints
        .get(&shape)
        .expect("the untouched empty shape should still be cached")
        .clone();
    assert_eq!(state.shape_cache.get(&fingerprint), Some(&shape));

    assert!(matches!(
        state.append_unique_layout(
            owner.object_id(),
            atom,
            PropertyFlags::data(true, true, true),
            PropertySlot::Data(RawValue::Object(stale_id)),
        ),
        Err(RuntimeError::Heap(HeapError::Stale { .. }))
    ));
    assert_eq!(
        state.atoms.resolve(atom).unwrap().ref_count,
        before_ref_count,
        "a rejected append must roll back the shape's tentative atom root"
    );
    assert!(state.heap.shape(shape).unwrap().entries().is_empty());
    assert_eq!(state.shape_fingerprints.get(&shape), Some(&fingerprint));
    assert_eq!(state.shape_cache.get(&fingerprint), Some(&shape));
}
