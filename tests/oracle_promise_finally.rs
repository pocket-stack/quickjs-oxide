use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const FIXTURE: &str = include_str!("fixtures/r3o_promise_finally.js");
const EXPECTED: &str = include_str!("fixtures/r3o_promise_finally.quickjs-2026-06-04.txt");

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

#[test]
fn promise_finally_matches_pinned_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    eval(&mut context, FIXTURE);
    runtime.run_gc().unwrap();
    while runtime.is_job_pending() {
        runtime.run_gc().unwrap();
        assert!(runtime.execute_pending_job().unwrap());
        runtime.run_gc().unwrap();
    }

    let Value::String(transcript) = eval(&mut context, "r3oTranscript.join('\\n') + '\\n'") else {
        panic!("R3o Promise.finally transcript was not a string");
    };
    assert_eq!(transcript.to_utf8_lossy(), EXPECTED);
}

#[test]
fn promise_finally_cfunction_data_handler_uses_calling_context() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    // A generic receiver exposes the internal fulfill handler without
    // scheduling it. The undefined constructor makes invoking that handler
    // materialize a TypeError in whichever Context executes the CFunctionData.
    eval(
        &mut defining,
        r#"
var capturedFinallyHandler;
Promise.prototype.finally.call(
    {
        constructor: undefined,
        then: function (onFulfilled) {
            capturedFinallyHandler = onFulfilled;
            return 0;
        }
    },
    function () { return 7; }
);
"#,
    );

    let handler_key = runtime
        .intern_property_key("capturedFinallyHandler")
        .unwrap();
    let defining_global = defining.global_object().unwrap();
    let Value::Object(handler) = defining
        .get_property(&defining_global, &handler_key)
        .unwrap()
    else {
        panic!("Promise.finally did not expose its internal fulfill callback");
    };
    let handler = runtime
        .as_callable(&handler)
        .unwrap()
        .expect("Promise.finally fulfill callback was not callable");

    let Value::Object(defining_type_error_prototype) = eval(&mut defining, "TypeError.prototype")
    else {
        panic!("defining TypeError.prototype was not an object");
    };
    let Value::Object(caller_type_error_prototype) = eval(&mut caller, "TypeError.prototype")
    else {
        panic!("caller TypeError.prototype was not an object");
    };
    assert_ne!(
        defining_type_error_prototype, caller_type_error_prototype,
        "the regression requires distinct realm intrinsics"
    );

    assert_eq!(
        caller.call(&handler, Value::Undefined, &[Value::Int(42)]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller
        .take_exception()
        .unwrap()
        .expect("Promise.finally internal callback did not preserve its throw")
    else {
        panic!("Promise.finally internal callback did not throw an Error object");
    };
    let error_prototype = runtime.get_prototype_of(&error).unwrap();
    assert_eq!(
        error_prototype.as_ref(),
        Some(&caller_type_error_prototype),
        "QuickJS CFunctionData executes Promise.finally's internal callback in the caller context; \
         actual prototype used the defining context: {}",
        error_prototype.as_ref() == Some(&defining_type_error_prototype)
    );
}
