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
