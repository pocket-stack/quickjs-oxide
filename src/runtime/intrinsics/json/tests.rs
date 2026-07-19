use super::*;

#[test]
fn json_native_cproto_matches_pinned_function_table() {
    for kind in [
        JsonNativeKind::IsRawJson,
        JsonNativeKind::Parse,
        JsonNativeKind::RawJson,
        JsonNativeKind::Stringify,
    ] {
        let descriptor = NativeFunctionId::Json(kind).descriptor();
        assert_eq!(descriptor.cproto, NativeCProto::Generic, "{kind:?}");
        assert!(!descriptor.cproto.default_is_constructor(), "{kind:?}");
    }
}

#[test]
fn global_json_is_realm_aware_lazy_and_reserves_the_pinned_table_order() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let second = runtime.new_context();
    let first_global = first.global_object().unwrap();
    let second_global = second.global_object().unwrap();
    let key = runtime.intern_property_key("JSON").unwrap();

    for (global, realm) in [(&first_global, first.realm), (&second_global, second.realm)] {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(global.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot),
            Some(PropertySlot::AutoInit(AutoInitProperty::Json {
                realm: defining_realm,
            })) if *defining_realm == realm
        ));
    }

    let Value::Object(json) = first.get_property(&first_global, &key).unwrap() else {
        panic!("JSON did not materialize to an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&json).unwrap(),
        Some(first.object_prototype().unwrap()),
    );
    let expected = [
        (JsonNativeKind::IsRawJson, "isRawJSON", 1),
        (JsonNativeKind::Parse, "parse", 2),
        (JsonNativeKind::RawJson, "rawJSON", 1),
        (JsonNativeKind::Stringify, "stringify", 3),
    ];
    for (kind, name, length) in expected {
        let method = runtime.intern_property_key(name).unwrap();
        let state = runtime.0.state.borrow();
        let object = state.heap.object(json.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot = usize::try_from(shape.find(method.atom()).unwrap()).unwrap();
        assert_eq!(
            shape.entries()[slot].flags,
            PropertyFlags::data(true, false, true),
        );
        assert!(matches!(
            object.slots.get(slot),
            Some(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: NativeFunctionId::Json(target),
                name: target_name,
                length: target_length,
                min_readable_args,
            })) if *realm == first.realm
                && *target == kind
                && *target_name == name
                && *target_length == length
                && *min_readable_args == length
        ));
    }
}

#[test]
fn deleting_lazy_global_json_releases_its_realm_edge() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("JSON").unwrap();
    let before = runtime
        .0
        .state
        .borrow()
        .heap
        .context_strong_count(context.realm)
        .unwrap();

    assert!(runtime.delete_property(&global, &key).unwrap());
    assert!(!runtime.has_own_property(&global, &key).unwrap());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm)
            .unwrap(),
        before - 1,
    );
}
