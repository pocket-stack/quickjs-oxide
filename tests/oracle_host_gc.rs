#![cfg(feature = "test262-host")]

use quickjs_oxide::{
    Context, DescriptorField, OrdinaryPropertyDescriptor, Runtime, RuntimeError, Value,
};

const FIXTURE: &str = include_str!("fixtures/host_gc_reentrant.js");
const QUICKJS_2026_06_04: &str = include_str!("fixtures/host_gc_reentrant.quickjs-2026-06-04.txt");

fn eval(context: &mut Context, source: &str) -> Value {
    context.eval(source).unwrap_or_else(|error| {
        if error == RuntimeError::Exception {
            panic!(
                "unexpected JavaScript exception: {:?}",
                context.take_exception()
            );
        }
        panic!("unexpected engine error: {error}");
    })
}

fn text(value: Value) -> String {
    let Value::String(value) = value else {
        panic!("expected a string value");
    };
    value.to_utf8_lossy()
}

fn data_property(value: Value) -> OrdinaryPropertyDescriptor {
    OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    }
}

fn install_test262_gc(context: &mut Context) {
    let runtime = context.runtime().clone();
    let object_262 = context.new_object().expect("allocate $262 object");
    let gc = context
        .new_test262_gc_function()
        .expect("allocate $262.gc host function");
    let gc_key = runtime.intern_property_key("gc").expect("intern gc key");
    assert!(
        context
            .define_own_property(
                &object_262,
                &gc_key,
                &data_property(Value::Object(gc.as_object().clone())),
            )
            .expect("define $262.gc")
    );
    let object_262_key = runtime
        .intern_property_key("$262")
        .expect("intern $262 key");
    let global = context.global_object().expect("get global object");
    assert!(
        context
            .define_own_property(
                &global,
                &object_262_key,
                &data_property(Value::Object(object_262)),
            )
            .expect("define global $262")
    );
}

fn drain_jobs(runtime: &Runtime, context: &mut Context) {
    let mut jobs = 0usize;
    while runtime.is_job_pending() {
        jobs += 1;
        assert!(jobs <= 64, "host GC fixture did not settle within 64 jobs");
        if let Err(error) = runtime.execute_pending_job() {
            if error == RuntimeError::Exception {
                panic!("host GC fixture job threw: {:?}", context.take_exception());
            }
            panic!("host GC fixture job failed: {error}");
        }
    }
    assert!(
        jobs > 0,
        "host GC fixture did not enqueue its promised work"
    );
}

#[test]
fn test262_gc_reentry_matches_pinned_quickjs_lifecycle_transcript() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    install_test262_gc(&mut context);

    eval(&mut context, FIXTURE);
    assert!(
        runtime.is_job_pending(),
        "$262.gc must leave Promise and finalization jobs queued"
    );
    drain_jobs(&runtime, &mut context);

    let transcript = text(eval(&mut context, "hostGcTranscript.join('\\n')"));
    assert_eq!(format!("{transcript}\n"), QUICKJS_2026_06_04);
}
