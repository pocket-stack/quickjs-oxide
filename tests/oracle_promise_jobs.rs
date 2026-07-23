use std::cell::RefCell;
use std::rc::Rc;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

fn text(value: Value) -> String {
    let Value::String(value) = value else {
        panic!("expected string value");
    };
    value.to_utf8_lossy()
}

fn integer(value: Value) -> i32 {
    let Value::Int(value) = value else {
        panic!("expected integer value");
    };
    value
}

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

fn drain(runtime: &Runtime) -> usize {
    let mut count = 0;
    while runtime.is_job_pending() {
        assert!(runtime.execute_pending_job().unwrap());
        count += 1;
    }
    count
}

#[test]
fn promise_constructor_and_internal_functions_have_quickjs_shapes() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let facts = text(eval(
        &mut context,
        r#"
var resolveFunction;
var rejectFunction;
new Promise(function (resolve, reject) {
    resolveFunction = resolve;
    rejectFunction = reject;
});
var capabilityExecutor;
function CustomPromise(executor) {
    capabilityExecutor = executor;
    executor(function () {}, function () {});
}
Promise.resolve.call(CustomPromise, 1);
[
    typeof Promise,
    Promise.length,
    Promise.name,
    Object.getOwnPropertyNames(resolveFunction).join(','),
    resolveFunction.length,
    resolveFunction.name,
    Object.getPrototypeOf(resolveFunction) === Function.prototype,
    Object.prototype.hasOwnProperty.call(resolveFunction, 'prototype'),
    Object.getOwnPropertyNames(rejectFunction).join(','),
    capabilityExecutor.length,
    capabilityExecutor.name,
    Object.getPrototypeOf(capabilityExecutor) === Function.prototype,
    Object.prototype.hasOwnProperty.call(capabilityExecutor, 'prototype')
].join('|');
"#,
    ));
    assert_eq!(
        facts,
        "function|1|Promise|length,name|1||true|false|length,name|2||true|false"
    );
}

#[test]
fn eval_does_not_drain_and_execute_pending_job_is_fifo_one_at_a_time() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
var order = [];
var settled = Promise.resolve(0);
settled.then(function () {
    order.push('A');
    settled.then(function () { order.push('nested'); });
});
settled.then(function () { order.push('B'); });
order.join('|');
"#,
        )),
        ""
    );
    assert!(runtime.is_job_pending());
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(text(eval(&mut context, "order.join('|')")), "A");
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(text(eval(&mut context, "order.join('|')")), "A|B");
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(text(eval(&mut context, "order.join('|')")), "A|B|nested");
    assert!(!runtime.is_job_pending());
    assert!(!runtime.execute_pending_job().unwrap());
}

#[test]
fn promise_chains_thenables_rejections_and_self_resolution() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
var events = [];
var thenable = {};
Object.defineProperty(thenable, 'then', {
    get: function () {
        events.push('get');
        return function (resolve, reject) {
            events.push('call');
            resolve(40);
            reject(99);
        };
    }
});
var answer = 0;
Promise.resolve(thenable)
    .then(function (value) { events.push('first:' + value); return value + 2; })
    .then(function (value) { answer = value; events.push('answer:' + value); });
var selfResolve;
var self = new Promise(function (resolve) { selfResolve = resolve; });
var selfError = '';
self.then(undefined, function (error) { selfError = error.name; });
selfResolve(self);
events.join('|');
"#,
        )),
        "get"
    );
    assert!(drain(&runtime) >= 4);
    assert_eq!(integer(eval(&mut context, "answer")), 42);
    assert_eq!(text(eval(&mut context, "selfError")), "TypeError");
    assert_eq!(
        text(eval(&mut context, "events.join('|')")),
        "get|call|first:40|answer:42"
    );
}

#[test]
fn queued_jobs_retain_their_graph_across_gc() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var gcAnswer = 0;
(function () {
    Promise.resolve(41).then(function (value) { gcAnswer = value + 1; });
})();
"#,
    );
    runtime.run_gc().unwrap();
    assert!(drain(&runtime) >= 1);
    assert_eq!(integer(eval(&mut context, "gcAnswer")), 42);
}

#[test]
fn static_identity_catch_and_species_follow_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
class DerivedPromise extends Promise {}
class SpeciesPromise extends Promise {
    static get [Symbol.species]() { return Promise; }
}
var base = Promise.resolve(1);
var derived = new DerivedPromise(function (resolve) { resolve(2); });
var speciesSource = new SpeciesPromise(function (resolve) { resolve(3); });
var speciesResult = speciesSource.then(function (value) { return value; });
var catchArgs = '';
var receiver = {
    then: function (fulfilled, rejected) {
        catchArgs = String(fulfilled) + ':' + rejected;
        return 42;
    }
};
var catchResult = Promise.prototype.catch.call(receiver, 'reject-handler');
[
    Promise.resolve(base) === base,
    DerivedPromise.resolve(derived) === derived,
    Promise.resolve(derived) !== derived,
    derived.then(function (value) { return value; }) instanceof DerivedPromise,
    speciesResult instanceof Promise,
    speciesResult instanceof SpeciesPromise,
    catchResult,
    catchArgs
].join('|');
"#,
        )),
        "true|true|true|true|true|false|42|undefined:reject-handler"
    );
    drain(&runtime);
}

#[test]
fn host_rejection_tracker_reports_unhandled_then_late_handled() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let expected_context = context.realm_id();
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let callback_context = Rc::new(RefCell::new(context.clone()));
    let callback_realm = callback_context.clone();
    runtime.set_host_promise_rejection_tracker(move |event| {
        captured
            .borrow_mut()
            .push((event.context(), event.is_handled(), event.reason().clone()));
        if event.is_handled() {
            eval(
                &mut callback_realm.borrow_mut(),
                "Promise.resolve().then(function () { trackerOrder.push('tracker'); });",
            );
        }
    });

    eval(
        &mut context,
        r#"
var trackerOrder = [];
var late = Promise.reject('late');
late.then(undefined, function () { trackerOrder.push('original'); });
var rejectEarly;
var early = new Promise(function (_, reject) { rejectEarly = reject; });
early.then(undefined, function () {});
rejectEarly('early');
"#,
    );
    runtime.clear_host_promise_rejection_tracker();

    let events = events.borrow();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].0, expected_context);
    assert!(!events[0].1);
    assert_eq!(text(events[0].2.clone()), "late");
    assert_eq!(events[1].0, expected_context);
    assert!(events[1].1);
    assert_eq!(text(events[1].2.clone()), "late");
    drop(events);
    drain(&runtime);
    assert_eq!(
        text(eval(&mut context, "trackerOrder.join('|')")),
        "tracker|original"
    );
}

#[test]
fn pending_job_reports_its_originating_context_on_success_and_throw() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    eval(
        &mut first,
        r#"
var source = Promise.resolve();
source.constructor = {
    [Symbol.species]: function ThrowingCapability(executor) {
        executor(
            function () { throw 'from first'; },
            function () { throw 'from first reject'; }
        );
        return {};
    }
};
source.then(function () { return 1; });
"#,
    );
    eval(
        &mut second,
        "Promise.resolve().then(function () { return 42; });",
    );

    let failure = runtime.execute_pending_job_with_context().unwrap_err();
    assert_eq!(failure.context(), first.realm_id());
    assert_eq!(failure.error(), &RuntimeError::Exception);
    assert_eq!(
        text(first.take_exception().unwrap().expect("job exception")),
        "from first"
    );

    assert_eq!(
        runtime.execute_pending_job_with_context().unwrap(),
        Some(second.realm_id())
    );
}
