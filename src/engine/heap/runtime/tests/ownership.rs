use super::*;

#[test]
fn rooted_handles_enforce_runtime_domain_and_dup_free_counts() {
    let first = Runtime::new();
    let second = Runtime::new();
    let first_key = first.intern_property_key("first").unwrap();
    let second_key = second.intern_property_key("second").unwrap();
    assert_eq!(first_key.atom().raw(), second_key.atom().raw());
    assert_ne!(first_key, second_key);

    let object = first.new_object(None).unwrap();
    assert!(matches!(
        second.define_own_property(
            &object,
            &second_key,
            &data_descriptor(Value::Int(1), true, true, true)
        ),
        Err(RuntimeError::WrongRuntime("object"))
    ));
    assert!(matches!(
        first.define_own_property(
            &object,
            &second_key,
            &data_descriptor(Value::Int(1), true, true, true)
        ),
        Err(RuntimeError::WrongRuntime("property key"))
    ));
    let foreign_object = second.new_object(None).unwrap();
    assert!(matches!(
        set_property(&first, &object, &first_key, Value::Object(foreign_object)),
        Err(RuntimeError::WrongRuntime("property value"))
    ));

    assert_eq!(
        first
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id()),
        Ok(1)
    );
    let value = Value::Object(object.clone());
    assert_eq!(
        first
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id()),
        Ok(2)
    );
    let duplicate = value.clone();
    assert_eq!(
        first
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id()),
        Ok(3)
    );
    drop(duplicate);
    drop(value);
    assert_eq!(
        first
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id()),
        Ok(1)
    );
}
