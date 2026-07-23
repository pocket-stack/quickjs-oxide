use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const FIXTURE: &str = include_str!("fixtures/r3p_promise_all.js");
const EXPECTED: &str = include_str!("fixtures/r3p_promise_all.quickjs-2026-06-04.txt");

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

#[test]
fn promise_all_matches_pinned_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    eval(&mut context, FIXTURE);
    runtime.run_gc().unwrap();
    while runtime.is_job_pending() {
        runtime.run_gc().unwrap();
        assert!(runtime.execute_pending_job().unwrap());
        runtime.run_gc().unwrap();
    }

    let Value::String(transcript) = eval(&mut context, "r3pTranscript.join('\\n') + '\\n'") else {
        panic!("R3p Promise.all transcript was not a string");
    };
    assert_eq!(transcript.to_utf8_lossy(), EXPECTED);
}

#[test]
fn promise_all_values_and_element_callback_use_quickjs_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    eval(
        &mut defining,
        r#"
var capturedAllElement;
var capturedAllValues;
function CapturingConstructor(executor) {
    var result = {};
    executor(
        function (values) {
            capturedAllValues = values;
            result.values = values;
        },
        function (reason) {
            result.reason = reason;
        }
    );
    return result;
}
CapturingConstructor.resolve = function (value) {
    return {
        then: function (onFulfilled) {
            capturedAllElement = onFulfilled;
        }
    };
};
var capturedAllResult = Promise.all.call(CapturingConstructor, [42]);
"#,
    );

    let Value::Object(element) = global_value(&mut defining, "capturedAllElement") else {
        panic!("Promise.all did not expose its internal element callback");
    };
    let element = runtime
        .as_callable(&element)
        .unwrap()
        .expect("Promise.all element callback was not callable");
    assert_eq!(
        caller.call(&element, Value::Undefined, &[Value::Int(42)]),
        Ok(Value::Undefined)
    );

    let Value::Object(values) = global_value(&mut defining, "capturedAllValues") else {
        panic!("Promise.all final resolve did not receive an Array");
    };
    let Value::Object(defining_array_prototype) = eval(&mut defining, "Array.prototype") else {
        panic!("defining Array.prototype was not an object");
    };
    let Value::Object(caller_array_prototype) = eval(&mut caller, "Array.prototype") else {
        panic!("caller Array.prototype was not an object");
    };
    assert_ne!(
        defining_array_prototype, caller_array_prototype,
        "the regression requires distinct realm intrinsics"
    );
    assert_eq!(
        runtime.get_prototype_of(&values).unwrap().as_ref(),
        Some(&defining_array_prototype),
        "Promise.all values Array did not use the builtin's defining realm"
    );

    eval(
        &mut defining,
        r#"
var capturedThrowingAllElement;
var throwingFinallyHandler;
Promise.prototype.finally.call(
    {
        constructor: undefined,
        then: function (onFulfilled) {
            throwingFinallyHandler = onFulfilled;
            return 0;
        }
    },
    function () { return 7; }
);
function ThrowingConstructor(executor) {
    executor(throwingFinallyHandler, function () {});
    return {};
}
ThrowingConstructor.resolve = function (value) {
    return {
        then: function (onFulfilled) {
            capturedThrowingAllElement = onFulfilled;
        }
    };
};
Promise.all.call(ThrowingConstructor, [42]);
"#,
    );

    let Value::Object(throwing_element) = global_value(&mut defining, "capturedThrowingAllElement")
    else {
        panic!("Promise.all did not expose its throwing element callback");
    };
    let throwing_element = runtime
        .as_callable(&throwing_element)
        .unwrap()
        .expect("Promise.all throwing element callback was not callable");
    assert_eq!(
        caller.call(&throwing_element, Value::Undefined, &[Value::Int(42)]),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = caller
        .take_exception()
        .unwrap()
        .expect("Promise.all element callback did not preserve its throw")
    else {
        panic!("Promise.all element callback did not throw an Error object");
    };
    let Value::Object(caller_type_error_prototype) = eval(&mut caller, "TypeError.prototype")
    else {
        panic!("caller TypeError.prototype was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap().as_ref(),
        Some(&caller_type_error_prototype),
        "QuickJS CFunctionData executes Promise.all's element callback in the calling Context"
    );
}
