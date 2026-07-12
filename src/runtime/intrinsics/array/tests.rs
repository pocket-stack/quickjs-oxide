use super::*;

#[test]
fn reduced_flatten_target_limit_preserves_prefix_and_exact_error() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(&mut context, "[1,2,3]");
    let target = eval_object(&mut context, "Object()");

    let result = runtime
        .flatten_into_array_with_limits(
            context.realm,
            &target,
            source,
            3,
            0,
            None,
            &Value::Undefined,
            2,
            16,
        )
        .unwrap();
    let NativeConversion::Throw(Value::Object(error)) = result else {
        panic!("reduced flatten limit did not return an Error object");
    };
    assert_eq!(
        string_property(&runtime, &mut context, &error, "name"),
        "TypeError"
    );
    assert_eq!(
        string_property(&runtime, &mut context, &error, "message"),
        "Array too long",
    );
    assert_eq!(int_property(&runtime, &mut context, &target, "0"), 1);
    assert_eq!(int_property(&runtime, &mut context, &target, "1"), 2);
    assert!(
        runtime
            .get_own_property(&target, &runtime.intern_property_key("2").unwrap())
            .unwrap()
            .is_none(),
        "the failing element was defined past the reduced target limit",
    );
    assert!(
        runtime
            .get_own_property(&target, &runtime.intern_property_key("length").unwrap())
            .unwrap()
            .is_none(),
        "flatten performed a final length Set on an ordinary species target",
    );
}

#[test]
fn reduced_flatten_frame_limit_is_catchable_without_rust_recursion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "(function(){var source=[];source[0]=source;return source})()",
    );
    let target = eval_object(&mut context, "Object()");

    let result = runtime
        .flatten_into_array_with_limits(
            context.realm,
            &target,
            source,
            1,
            i32::MAX,
            None,
            &Value::Undefined,
            (1_u64 << 53) - 1,
            3,
        )
        .unwrap();
    let NativeConversion::Throw(Value::Object(error)) = result else {
        panic!("reduced flatten frame limit did not return an Error object");
    };
    assert_eq!(
        string_property(&runtime, &mut context, &error, "name"),
        "InternalError",
    );
    assert_eq!(
        string_property(&runtime, &mut context, &error, "message"),
        "stack overflow",
    );
}

#[test]
fn array_unscopables_autoinit_retains_then_releases_its_realm_edge() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array_prototype = context.array_prototype().unwrap();
    let key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Unscopables));

    let (slot_index, count_before) = {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(array_prototype.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(
                AutoInitProperty::ArrayUnscopables { realm }
            )) if *realm == context.realm
        ));
        (
            slot_index,
            state.heap.context_strong_count(context.realm).unwrap(),
        )
    };

    let Value::Object(unscopables) = context.get_property(&array_prototype, &key).unwrap() else {
        panic!("Array.prototype[Symbol.unscopables] was not an object");
    };
    let state = runtime.0.state.borrow();
    assert_eq!(
        state.heap.context_strong_count(context.realm).unwrap(),
        count_before - 1,
        "materialization did not release the autoinit's defining-realm edge",
    );
    let object = state.heap.object(array_prototype.object_id()).unwrap();
    assert!(matches!(
        object.slots.get(slot_index),
        Some(PropertySlot::Data(RawValue::Object(id))) if *id == unscopables.object_id()
    ));
    drop(state);
    assert_eq!(runtime.get_prototype_of(&unscopables).unwrap(), None);
}

#[test]
fn array_unscopables_metadata_and_delete_preserve_lazy_state() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let array_prototype = context.array_prototype().unwrap();
    let key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Unscopables));

    let count_before = {
        let state = runtime.0.state.borrow();
        state.heap.context_strong_count(context.realm).unwrap()
    };
    assert!(
        runtime
            .own_property_keys(&array_prototype)
            .unwrap()
            .contains(&key)
    );
    assert!(runtime.has_property(&array_prototype, &key).unwrap());
    {
        let state = runtime.0.state.borrow();
        let object = state.heap.object(array_prototype.object_id()).unwrap();
        let shape = state.heap.shape(object.shape).unwrap();
        let slot_index = usize::try_from(shape.find(key.atom()).unwrap()).unwrap();
        assert!(matches!(
            object.slots.get(slot_index),
            Some(PropertySlot::AutoInit(
                AutoInitProperty::ArrayUnscopables { realm }
            )) if *realm == context.realm
        ));
        assert_eq!(
            state.heap.context_strong_count(context.realm).unwrap(),
            count_before,
            "ownKeys or HasProperty materialized the autoinit slot",
        );
    }

    assert!(runtime.delete_property(&array_prototype, &key).unwrap());
    let state = runtime.0.state.borrow();
    assert_eq!(
        state.heap.context_strong_count(context.realm).unwrap(),
        count_before - 1,
        "deleting the lazy property did not release its defining-realm edge",
    );
    let object = state.heap.object(array_prototype.object_id()).unwrap();
    let shape = state.heap.shape(object.shape).unwrap();
    assert!(shape.find(key.atom()).is_none());
}

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    object
}

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let Value::Int(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an Int property");
    };
    value
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not a String property");
    };
    value.to_utf8_lossy()
}
