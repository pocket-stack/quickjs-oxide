use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const FIXTURE: &str = include_str!("fixtures/r3q_promise_aggregates.js");
const EXPECTED: &str = include_str!("fixtures/r3q_promise_aggregates.quickjs-2026-06-04.txt");

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

fn global_value(context: &mut Context, name: &str) -> Value {
    let runtime = context.runtime();
    let key = runtime.intern_property_key(name).unwrap();
    let global = context.global_object().unwrap();
    context.get_property(&global, &key).unwrap()
}

fn global_object(context: &mut Context, name: &str) -> quickjs_oxide::ObjectRef {
    let Value::Object(object) = global_value(context, name) else {
        panic!("{name} was not an object");
    };
    object
}

#[test]
fn promise_all_settled_and_any_match_pinned_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    eval(&mut context, FIXTURE);
    runtime.run_gc().unwrap();
    while runtime.is_job_pending() {
        runtime.run_gc().unwrap();
        assert!(runtime.execute_pending_job().unwrap());
        runtime.run_gc().unwrap();
    }

    let Value::String(transcript) = eval(&mut context, "r3qTranscript.join('\\n') + '\\n'") else {
        panic!("R3q Promise aggregate transcript was not a string");
    };
    assert_eq!(transcript.to_utf8_lossy(), EXPECTED);
}

#[test]
fn promise_aggregate_internal_values_follow_quickjs_context_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    eval(
        &mut defining,
        r#"
var capturedSettledFulfill;
var capturedSettledReject;
var capturedSettledValues;
function SettledConstructor(executor) {
    var result = {};
    executor(
        function (values) {
            capturedSettledValues = values;
            result.values = values;
        },
        function (reason) {
            result.reason = reason;
        }
    );
    return result;
}
SettledConstructor.resolve = function (value) {
    return {
        then: function (onFulfilled, onRejected) {
            if (value === 0)
                capturedSettledFulfill = onFulfilled;
            else
                capturedSettledReject = onRejected;
        }
    };
};
var capturedSettledResult =
    Promise.allSettled.call(SettledConstructor, [0, 1]);

var capturedAnyReject0;
var capturedAnyReject1;
var capturedAnyError;
function AnyConstructor(executor) {
    var result = {};
    executor(
        function (value) {
            result.value = value;
        },
        function (reason) {
            capturedAnyError = reason;
            result.reason = reason;
        }
    );
    return result;
}
AnyConstructor.resolve = function (value) {
    return {
        then: function (_, onRejected) {
            if (value === 0)
                capturedAnyReject0 = onRejected;
            else
                capturedAnyReject1 = onRejected;
        }
    };
};
var capturedAnyResult = Promise.any.call(AnyConstructor, [0, 1]);
"#,
    );

    let settled_fulfill = global_object(&mut defining, "capturedSettledFulfill");
    let settled_fulfill = runtime
        .as_callable(&settled_fulfill)
        .unwrap()
        .expect("allSettled fulfill element was not callable");
    assert_eq!(
        caller.call(&settled_fulfill, Value::Undefined, &[Value::Int(42)]),
        Ok(Value::Undefined)
    );

    let settled_reject = global_object(&mut defining, "capturedSettledReject");
    let settled_reject = runtime
        .as_callable(&settled_reject)
        .unwrap()
        .expect("allSettled reject element was not callable");
    assert_eq!(
        caller.call(&settled_reject, Value::Undefined, &[Value::Int(43)]),
        Ok(Value::Undefined)
    );

    eval(
        &mut defining,
        r#"
var capturedSettledEntry0 = capturedSettledValues[0];
var capturedSettledEntry1 = capturedSettledValues[1];
"#,
    );
    let settled_values = global_object(&mut defining, "capturedSettledValues");
    let settled_entry0 = global_object(&mut defining, "capturedSettledEntry0");
    let settled_entry1 = global_object(&mut defining, "capturedSettledEntry1");
    let defining_array_prototype = global_object_from_eval(&mut defining, "Array.prototype");
    let defining_object_prototype = global_object_from_eval(&mut defining, "Object.prototype");
    let caller_object_prototype = global_object_from_eval(&mut caller, "Object.prototype");
    assert_ne!(defining_object_prototype, caller_object_prototype);
    assert_eq!(
        runtime.get_prototype_of(&settled_values).unwrap().as_ref(),
        Some(&defining_array_prototype),
        "allSettled values Array must use the aggregate call Context"
    );
    assert_eq!(
        runtime.get_prototype_of(&settled_entry0).unwrap().as_ref(),
        Some(&caller_object_prototype),
        "allSettled fulfillment record must use the callback call Context"
    );
    assert_eq!(
        runtime.get_prototype_of(&settled_entry1).unwrap().as_ref(),
        Some(&caller_object_prototype),
        "allSettled rejection record must use the callback call Context"
    );

    let any_reject0 = global_object(&mut defining, "capturedAnyReject0");
    let any_reject0 = runtime
        .as_callable(&any_reject0)
        .unwrap()
        .expect("Promise.any first reject element was not callable");
    assert_eq!(
        caller.call(&any_reject0, Value::Undefined, &[Value::Int(40)]),
        Ok(Value::Undefined)
    );
    let any_reject1 = global_object(&mut defining, "capturedAnyReject1");
    let any_reject1 = runtime
        .as_callable(&any_reject1)
        .unwrap()
        .expect("Promise.any second reject element was not callable");
    assert_eq!(
        caller.call(&any_reject1, Value::Undefined, &[Value::Int(41)]),
        Ok(Value::Undefined)
    );

    eval(
        &mut defining,
        "var capturedAnyErrors = capturedAnyError.errors;",
    );
    let any_error = global_object(&mut defining, "capturedAnyError");
    let any_errors = global_object(&mut defining, "capturedAnyErrors");
    let caller_aggregate_error_prototype =
        global_object_from_eval(&mut caller, "AggregateError.prototype");
    assert_eq!(
        runtime.get_prototype_of(&any_error).unwrap().as_ref(),
        Some(&caller_aggregate_error_prototype),
        "Promise.any AggregateError must use the final callback call Context"
    );
    assert_eq!(
        runtime.get_prototype_of(&any_errors).unwrap().as_ref(),
        Some(&defining_array_prototype),
        "Promise.any errors Array must use the aggregate call Context"
    );
}

fn global_object_from_eval(context: &mut Context, source: &str) -> quickjs_oxide::ObjectRef {
    let Value::Object(object) = eval(context, source) else {
        panic!("{source} was not an object");
    };
    object
}
