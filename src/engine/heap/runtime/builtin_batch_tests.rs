use super::Runtime;
use crate::engine::builtins::native::{DateNativeKind, NativeFunctionId};
use crate::engine::heap::{AutoInitProperty, PropertySlot, RawId, ShapeId};
use crate::engine::object::ObjectRef;
use crate::engine::object::builtin_properties::NativeBuiltinProperty;
use crate::engine::object::shape::PropertyFlags;
use crate::engine::value::Value;

fn method(name: &'static str) -> NativeBuiltinProperty {
    NativeBuiltinProperty::new(
        NativeFunctionId::Date(DateNativeKind::TimeValue),
        name,
        2,
        3,
    )
}

fn layout(runtime: &Runtime, object: &ObjectRef) -> (ShapeId, Vec<PropertySlot>) {
    let state = runtime.0.state.borrow();
    let object = state.heap.object(object.object_id()).unwrap();
    (object.shape, object.slots.clone())
}

#[test]
fn builtin_batch_preserves_order_flags_metadata_and_lazy_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let mut second = method("batch_second");
    second.flags = PropertyFlags::data(false, true, false);
    let methods = [method("batch_first"), second];
    runtime
        .define_native_builtin_auto_init_batch(&object, context.realm, methods)
        .unwrap();
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(object.object_id()).unwrap();
        let entries = state.heap.shape(object.shape).unwrap().entries();
        assert_eq!(entries.len(), 2);
        for (index, method) in methods.iter().enumerate() {
            assert_eq!(
                state
                    .atoms
                    .to_js_string(entries[index].atom)
                    .unwrap()
                    .to_string(),
                method.name
            );
            assert_eq!(entries[index].flags, method.flags);
            assert_eq!(
                object.slots[index],
                PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                    realm: context.realm,
                    target: method.target,
                    name: method.name,
                    length: method.length,
                    min_readable_args: method.min_readable_args,
                })
            );
        }
    }
    let key = runtime.intern_property_key("batch_first").unwrap();
    let first = context.get_property(&object, &key).unwrap();
    assert!(matches!(first, Value::Object(_)));
    assert_eq!(first, context.get_property(&object, &key).unwrap());
    assert!(matches!(
        layout(&runtime, &object).1[1],
        PropertySlot::AutoInit(_)
    ));
    let Value::Object(function) = first else {
        unreachable!()
    };
    let length = runtime.intern_property_key("length").unwrap();
    assert_eq!(
        context.get_property(&function, &length).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn builtin_batch_rejects_entire_invalid_table_without_leaking_keys() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    runtime
        .define_native_builtin_auto_init_batch(&object, context.realm, [method("batch_existing")])
        .unwrap();
    let original = layout(&runtime, &object);
    let atoms = runtime.test_atom_count();
    let mut accessor = method("batch_accessor");
    accessor.flags = PropertyFlags::accessor(false, true);
    for methods in [
        vec![method("batch_new"), method("batch_existing")],
        vec![method("batch_new"), method("batch_new")],
        vec![method("batch_new"), method("0")],
        vec![method("batch_new"), accessor],
    ] {
        assert!(
            runtime
                .define_native_builtin_auto_init_batch(&object, context.realm, methods)
                .is_err()
        );
        assert_eq!(layout(&runtime, &object), original);
        assert_eq!(runtime.test_atom_count(), atoms);
    }
    runtime
        .define_native_builtin_auto_init_batch(&object, context.realm, [])
        .unwrap();
    assert_eq!(layout(&runtime, &object), original);
    runtime.prevent_extensions(&object).unwrap();
    assert!(
        runtime
            .define_native_builtin_auto_init_batch(&object, context.realm, [method("batch_new")])
            .is_err()
    );
    assert_eq!(layout(&runtime, &object), original);
}

#[test]
fn builtin_batch_validates_receiver_domain_and_realm_lifetime() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let other = Runtime::new();
    let foreign = other.new_object(None).unwrap();
    assert!(
        runtime
            .define_native_builtin_auto_init_batch(
                &foreign,
                context.realm,
                [method("batch_foreign")]
            )
            .is_err()
    );
    let expired = runtime.new_context();
    let realm = expired.realm;
    drop(expired);
    runtime.run_gc().unwrap();
    let object = runtime.new_object(None).unwrap();
    let original = layout(&runtime, &object);
    let atoms = runtime.test_atom_count();
    assert!(
        runtime
            .define_native_builtin_auto_init_batch(&object, realm, [method("batch_stale")])
            .is_err()
    );
    assert_eq!(layout(&runtime, &object), original);
    assert_eq!(runtime.test_atom_count(), atoms);
}

#[test]
fn builtin_batch_rolls_back_shape_and_realm_edges_on_retain_overflow() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let original = layout(&runtime, &object);
    let atoms = runtime.test_atom_count();
    let (strong, live, shapes) = {
        let mut state = runtime.0.state.borrow_mut();
        let counts = state.heap.counts();
        let node = state
            .heap
            .live_node_mut(RawId::Context(context.realm))
            .unwrap();
        let strong = node.strong;
        node.strong = u32::MAX;
        (strong, counts.live, counts.shape_nodes)
    };
    let result = runtime.define_native_builtin_auto_init_batch(
        &object,
        context.realm,
        [method("batch_one"), method("batch_two")],
    );
    {
        let mut state = runtime.0.state.borrow_mut();
        let node = state
            .heap
            .live_node_mut(RawId::Context(context.realm))
            .unwrap();
        let after = node.strong;
        node.strong = strong;
        assert_eq!(after, u32::MAX);
        assert_eq!(state.heap.counts().live, live);
        assert_eq!(state.heap.counts().shape_nodes, shapes);
    }
    assert!(result.is_err());
    assert_eq!(layout(&runtime, &object), original);
    assert_eq!(runtime.test_atom_count(), atoms);
}

#[test]
fn builtin_batch_date_keeps_aliases_descriptors_and_realm_functions() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"(() => {
        const p = Date.prototype;
        const iso = p.toISOString;
        const d = Object.getOwnPropertyDescriptor(p, 'setFullYear');
        return p.toGMTString === p.toUTCString && p.toGMTString.name === 'toUTCString'
            && iso === p.toISOString && iso.name === 'toISOString' && iso.length === 0
            && d.writable && !d.enumerable && d.configurable && d.value.length === 3
            && new Date(0).toISOString() === '1970-01-01T00:00:00.000Z';
    })()"#
            )
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn builtin_batch_keeps_context_functions_separate_on_a_shared_shape() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let a = runtime.new_object(None).unwrap();
    let b = runtime.new_object(None).unwrap();
    runtime
        .define_native_builtin_auto_init_batch(&a, first.realm, [method("batch_realm")])
        .unwrap();
    runtime
        .define_native_builtin_auto_init_batch(&b, second.realm, [method("batch_realm")])
        .unwrap();
    assert_eq!(layout(&runtime, &a).0, layout(&runtime, &b).0);
    let key = runtime.intern_property_key("batch_realm").unwrap();
    let a_function = first.get_property(&a, &key).unwrap();
    let b_function = second.get_property(&b, &key).unwrap();
    assert_ne!(a_function, b_function);
    assert_eq!(a_function, second.get_property(&a, &key).unwrap());
    assert_eq!(b_function, first.get_property(&b, &key).unwrap());
    let Value::Object(a_function) = a_function else {
        unreachable!()
    };
    let Value::Object(b_function) = b_function else {
        unreachable!()
    };
    let Value::Object(first_prototype) = first.eval("Function.prototype").unwrap() else {
        unreachable!()
    };
    let Value::Object(second_prototype) = second.eval("Function.prototype").unwrap() else {
        unreachable!()
    };
    let state = runtime.0.state.borrow();
    for (function, prototype) in [
        (&a_function, &first_prototype),
        (&b_function, &second_prototype),
    ] {
        let shape = state.heap.object(function.object_id()).unwrap().shape;
        assert_eq!(
            state.heap.shape(shape).unwrap().prototype(),
            Some(prototype.object_id())
        );
    }
}

#[test]
fn builtin_batch_rejects_exotic_receivers_without_observable_traps() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for source in [
        "new Date(0)",
        "new Uint8Array(2)",
        "new Proxy({}, {defineProperty() { throw 42; }})",
    ] {
        let Value::Object(object) = context.eval(source).unwrap() else {
            unreachable!()
        };
        let original = layout(&runtime, &object);
        let atoms = runtime.test_atom_count();
        assert!(
            runtime
                .define_native_builtin_auto_init_batch(
                    &object,
                    context.realm,
                    [method("batch_exotic")]
                )
                .is_err()
        );
        assert_eq!(layout(&runtime, &object), original);
        assert_eq!(runtime.test_atom_count(), atoms);
    }
}

#[test]
fn builtin_batch_array_and_typed_array_keep_order_aliases_and_calls() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"(() => {
        const p = Array.prototype;
        const t = Object.getPrototypeOf(Uint8Array.prototype);
        const keys = Object.getOwnPropertyNames(t);
        const d = Object.getOwnPropertyDescriptor(t, 'set');
        return p.values === p[Symbol.iterator] && t.values === t[Symbol.iterator]
            && t.toString === p.toString && Array.from.length === 1
            && Object.getPrototypeOf(Uint8Array).from.length === 1
            && keys.slice(0, 7).join(',') === 'length,at,with,buffer,byteLength,byteOffset,set'
            && d.writable && !d.enumerable && d.configurable && d.value.length === 1
            && [3, 1, 2].toSorted().join() === '1,2,3'
            && new Uint8Array([3, 1, 2]).toSorted().join() === '1,2,3'
            && Uint8Array.fromHex('0aff').toHex() === '0aff';
    })()"#
            )
            .unwrap(),
        Value::Bool(true)
    );
}
