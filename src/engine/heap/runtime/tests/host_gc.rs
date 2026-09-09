#[cfg(feature = "test262-host")]
use super::*;

#[cfg(feature = "test262-host")]
#[test]
fn test262_gc_host_is_quickjs_shaped_reentrant_and_preserves_active_roots() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let gc = context.new_test262_gc_function().unwrap();

    assert_eq!(
        NativeFunctionId::Test262Gc.descriptor().cproto,
        NativeCProto::Generic
    );
    assert!(!runtime.is_constructor(gc.as_object()).unwrap());
    assert_eq!(runtime.callable_realm(&gc).unwrap(), context.realm);
    assert_eq!(
        runtime.get_prototype_of(gc.as_object()).unwrap(),
        Some(context.function_prototype().unwrap())
    );

    let name = runtime.intern_property_key("name").unwrap();
    let length = runtime.intern_property_key("length").unwrap();
    assert!(matches!(
        runtime.get_own_property(gc.as_object(), &name).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            writable: false,
            enumerable: false,
            configurable: true,
        }) if value == JsString::from_static("gc")
    ));
    assert!(matches!(
        runtime.get_own_property(gc.as_object(), &length).unwrap(),
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(0),
            writable: false,
            enumerable: false,
            configurable: true,
        })
    ));

    runtime.run_gc().unwrap();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let cycle = context.new_object().unwrap();
    let self_key = runtime.intern_property_key("self").unwrap();
    assert!(
        context
            .define_own_property(
                &cycle,
                &self_key,
                &data_descriptor(Value::Object(cycle.clone()), true, true, true),
            )
            .unwrap()
    );
    drop(self_key);
    drop(cycle);
    let live_argument = context.new_object().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
    assert_eq!(
        context
            .call(
                &gc,
                Value::Object(live_argument.clone()),
                &[Value::Object(live_argument.clone()), Value::Int(42)],
            )
            .unwrap(),
        Value::Undefined
    );
    assert_eq!(
        runtime.heap_counts().object_nodes,
        baseline_objects + 1,
        "the native host hook did not run cycle collection"
    );

    let global = context.global_object().unwrap();
    let host_gc = runtime.intern_property_key("hostGc").unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &host_gc,
                &data_descriptor(Value::Object(gc.as_object().clone()), true, true, true),
            )
            .unwrap()
    );
    assert_eq!(
        context
            .eval(
                r#"
                (function activeFrame() {
                    var local = { answer: 42 };
                    activeFrame = null;
                    if (hostGc.call(local, local, "ignored") !== undefined) {
                        throw new Error("gc did not return undefined");
                    }
                    return local.answer;
                })()
                "#,
            )
            .unwrap(),
        Value::Int(42),
        "reentrant collection released an active frame or local root"
    );

    assert_eq!(context.construct(&gc, &[]), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("gc is not a constructor")
    );
}
