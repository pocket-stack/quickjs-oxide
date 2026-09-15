use super::*;

#[cfg(all(feature = "stack-vm", feature = "profiling"))]
#[test]
fn trusted_construct_prototype_getter_resumes_once_in_owned_driver() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let mut image = quickjs_ordinary_with_code_and_constants(QUICKJS_ORDINARY_CONSTRUCT_CODE, &[]);
    image[33] = 4;
    let construct = context.read_trusted_ordinary_function(&image, 0).unwrap();
    let constructor = context.eval("(function C(){return 0})").unwrap();
    let prototype = context.eval("var chosen={marker:42};chosen").unwrap();
    let new_target = context
        .eval("({chosen:chosen,get prototype(){return this.chosen}})")
        .unwrap();
    let profile = crate::engine::api::profiling::CostProfile::start();
    let Value::Object(instance) = context
        .call(&construct, Value::Undefined, &[constructor, new_target])
        .unwrap()
    else {
        panic!("expected instance")
    };
    let costs = profile.snapshot();
    assert_eq!(costs.owned_storage.frames_pushed, 3);
    assert_eq!(costs.owned_storage.maximum_frame_depth, 2);
    assert_eq!(costs.legacy_dispatches, 0);
    assert_eq!(costs.owned_bridge_exits, 0);
    assert_eq!(
        runtime
            .get_prototype_of(&instance)
            .unwrap()
            .map(Value::Object),
        Some(prototype)
    );
    assert!(runtime.0.state.borrow().active_frames.is_empty());
}
