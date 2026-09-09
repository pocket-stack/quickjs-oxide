use super::*;

#[test]
fn contexts_share_runtime_but_have_distinct_identity() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let second = runtime.new_context();
    assert_ne!(first.id(), second.id());
    assert!(first.runtime().is_same_runtime(second.runtime()));
    let first_prototype = first.object_prototype().unwrap();
    let second_prototype = second.object_prototype().unwrap();
    assert_ne!(first_prototype, second_prototype);
    let function_prototype = first.function_prototype().unwrap();
    let global_object = first.global_object().unwrap();
    let global_var_object = first.global_var_object().unwrap();
    assert_eq!(
        runtime.get_prototype_of(&function_prototype).unwrap(),
        Some(first_prototype.clone())
    );
    assert_eq!(
        runtime.get_prototype_of(&global_object).unwrap(),
        Some(first_prototype.clone())
    );
    assert_eq!(runtime.get_prototype_of(&global_var_object).unwrap(), None);
    assert!(runtime.set_prototype_of(&global_var_object, None).unwrap());
    assert!(
        !runtime
            .set_prototype_of(&global_var_object, Some(&first_prototype))
            .unwrap()
    );
    let object = first.new_object().unwrap();
    assert_eq!(
        runtime.get_prototype_of(&object).unwrap(),
        Some(first_prototype.clone())
    );
    assert!(runtime.set_prototype_of(&first_prototype, None).unwrap());
    assert!(
        !runtime
            .set_prototype_of(&first_prototype, Some(&object))
            .unwrap()
    );
}

#[test]
fn executing_foreign_runtime_bytecode_is_rejected_before_instantiation() {
    let first = Runtime::new();
    let second = Runtime::new();
    let mut compiler_context = first.new_context();
    let function = compiler_context.compile("42").unwrap();
    let mut caller_context = second.new_context();
    let caller_realm_objects = second.heap_counts().object_nodes;

    assert!(matches!(
        caller_context.execute(&function),
        Err(RuntimeError::WrongRuntime("function bytecode"))
    ));
    assert_eq!(second.heap_counts().object_nodes, caller_realm_objects);
}
