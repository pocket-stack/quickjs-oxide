use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, Runtime,
    RuntimeError, Value,
};

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
        panic!("expected a string");
    };
    value.to_utf8_lossy()
}

fn integer(value: Value) -> i32 {
    let Value::Int(value) = value else {
        panic!("expected an integer");
    };
    value
}

fn object(value: Value) -> ObjectRef {
    let Value::Object(value) = value else {
        panic!("expected an object");
    };
    value
}

fn drain(runtime: &Runtime) -> usize {
    let mut count = 0;
    while runtime.is_job_pending() {
        assert!(runtime.execute_pending_job().unwrap());
        count += 1;
    }
    count
}

fn define_global(context: &mut Context, name: &str, value: Value) {
    let runtime = context.runtime().clone();
    let key = runtime.intern_property_key(name).unwrap();
    let global = context.global_object().unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

#[test]
fn function_expression_await_names_match_pinned_quickjs_token_context() {
    const VALID: &str = r#"
async function await() {}
async function outer(fromDefault = function await(){}) {
    var ordinary = function await(){};
    var generator = function* await(){};
    var asynchronous = async function await(){};
    await 0;
    return [
        fromDefault.name,
        ordinary.name,
        generator.name,
        asynchronous.name,
        (async function strictParent() {
            "use strict";
            return (async function await(){}).name;
        })()
    ];
}
outer().then(async function(values) {
    values[4] = await values[4];
    print([globalThis.await.name].concat(values).join("|"));
});
"#;
    const INVALID: &[&str] = &[
        "(async function await(){})",
        "async function outer(){ function await(){} }",
        "async function outer(){ function* await(){} }",
        "async function outer(){ async function await(){} }",
    ];

    let oxide = Command::new(env!("CARGO_BIN_EXE_qjs"))
        .args(["-e", VALID])
        .output()
        .expect("run quickjs-oxide contextual-await probe");
    assert!(
        oxide.status.success(),
        "quickjs-oxide rejected the contextual-await probe: {}",
        String::from_utf8_lossy(&oxide.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&oxide.stdout),
        "await|await|await|await|await|await\n"
    );
    for source in INVALID {
        let output = Command::new(env!("CARGO_BIN_EXE_qjs"))
            .args(["-e", source])
            .output()
            .unwrap_or_else(|error| {
                panic!("run quickjs-oxide rejection probe {source:?}: {error}")
            });
        assert!(
            !output.status.success(),
            "quickjs-oxide accepted declaration/expression edge {source:?}"
        );
    }

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP contextual-await oracle differential: set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };
    let quickjs = Command::new(&oracle)
        .args(["-e", VALID])
        .output()
        .expect("run pinned QuickJS contextual-await probe");
    assert!(
        quickjs.status.success(),
        "pinned QuickJS rejected the contextual-await probe: {}",
        String::from_utf8_lossy(&quickjs.stderr)
    );
    assert_eq!(oxide.stdout, quickjs.stdout);
    for source in INVALID {
        let output = Command::new(&oracle)
            .args(["-e", source])
            .output()
            .unwrap_or_else(|error| {
                panic!("run pinned QuickJS rejection probe {source:?}: {error}")
            });
        assert!(
            !output.status.success(),
            "pinned QuickJS accepted declaration/expression edge {source:?}"
        );
    }
}

#[test]
fn ordinary_async_shape_starts_synchronously_and_returns_a_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
var events = [];
async function add(left, right) {
    events.push('body');
    return left + right;
}
var answer = 'pending';
var promise = add(19, 23);
events.push('after');
promise.then(function (value) {
    answer = value;
    events.push('then');
});
[
    typeof add,
    add.length,
    add.name,
    Object.prototype.hasOwnProperty.call(add, 'prototype'),
    Object.prototype.toString.call(add),
    Object.getPrototypeOf(add).constructor.name,
    Object.getPrototypeOf(promise) === Promise.prototype,
    events.join(','),
    answer
].join('|');
"#,
        )),
        "function|2|add|false|[object AsyncFunction]|AsyncFunction|true|body,after|pending"
    );
    assert_eq!(
        runtime.execute_pending_job_with_context().unwrap(),
        Some(context.realm_id())
    );
    assert_eq!(integer(eval(&mut context, "answer")), 42);
    assert_eq!(
        text(eval(&mut context, "events.join(',')")),
        "body,after,then"
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn fallthrough_throw_this_and_arguments_settle_the_outer_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var results = [];
async function omitted() {}
async function boom() { throw new Error('boom'); }
async function receiver(value) {
    return this.base + arguments[0] + arguments.length;
}
omitted().then(function (value) {
    results.push(typeof value);
});
boom().then(undefined, function (error) {
    results.push(error.name + ':' + error.message);
});
receiver.call({ base: 40 }, 1).then(function (value) {
    results.push(value);
});
"#,
    );
    assert_eq!(drain(&runtime), 3);
    assert_eq!(
        text(eval(&mut context, "results.join('|')")),
        "undefined|Error:boom|42"
    );
}

#[test]
fn each_await_yields_exactly_one_fifo_resume_job() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
var order = [];
var answer = 'pending';
async function sequence() {
    order.push('f0');
    var first = await 7;
    order.push('f1:' + first);
    var second = await Promise.resolve(8);
    order.push('f2:' + second);
    return first + second;
}
sequence().then(function (value) {
    answer = value;
    order.push('done:' + value);
});
order.push('sync');
order.join('|');
"#,
        )),
        "f0|sync"
    );

    for expected in ["f0|sync|f1:7", "f0|sync|f1:7|f2:8"] {
        assert_eq!(
            runtime.execute_pending_job_with_context().unwrap(),
            Some(context.realm_id())
        );
        assert_eq!(text(eval(&mut context, "order.join('|')")), expected);
        assert_eq!(text(eval(&mut context, "answer")), "pending");
    }
    assert_eq!(
        runtime.execute_pending_job_with_context().unwrap(),
        Some(context.realm_id())
    );
    assert_eq!(integer(eval(&mut context, "answer")), 15);
    assert_eq!(
        text(eval(&mut context, "order.join('|')")),
        "f0|sync|f1:7|f2:8|done:15"
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn await_rejection_uses_normal_vm_unwind_for_catch_and_finally() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
var events = [];
var answer = 'pending';
async function recover() {
    try {
        events.push('try');
        await Promise.reject('x');
        events.push('miss');
    } catch (error) {
        events.push('catch:' + error);
    } finally {
        events.push('finally');
    }
    return 42;
}
recover().then(function (value) {
    answer = value;
    events.push('done');
});
events.push('sync');
events.join('|');
"#,
        )),
        "try|sync"
    );
    assert_eq!(
        runtime.execute_pending_job_with_context().unwrap(),
        Some(context.realm_id())
    );
    assert_eq!(
        text(eval(&mut context, "events.join('|')")),
        "try|sync|catch:x|finally"
    );
    assert_eq!(text(eval(&mut context, "answer")), "pending");
    assert_eq!(
        runtime.execute_pending_job_with_context().unwrap(),
        Some(context.realm_id())
    );
    assert_eq!(integer(eval(&mut context, "answer")), 42);
    assert_eq!(
        text(eval(&mut context, "events.join('|')")),
        "try|sync|catch:x|finally|done"
    );
}

#[test]
fn await_assimilates_thenables_once_and_retains_the_graph_across_gc() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
var events = [];
var answer = 'pending';
var thenable = {};
Object.defineProperty(thenable, 'then', {
    get: function () {
        events.push('get');
        return function (resolve, reject) {
            events.push('call');
            resolve(42);
            reject('late');
            resolve(99);
        };
    }
});
async function consume() {
    var value = await thenable;
    events.push('resume:' + value);
    return value;
}
consume().then(
    function (value) {
        answer = value;
        events.push('done');
    },
    function (error) {
        answer = 'bad:' + error;
    }
);
events.push('sync');
events.join('|');
"#,
        )),
        "get|sync"
    );
    runtime.run_gc().unwrap();

    let checkpoints = [
        "get|sync|call",
        "get|sync|call|resume:42",
        "get|sync|call|resume:42|done",
    ];
    for expected in checkpoints {
        assert_eq!(
            runtime.execute_pending_job_with_context().unwrap(),
            Some(context.realm_id())
        );
        runtime.run_gc().unwrap();
        assert_eq!(text(eval(&mut context, "events.join('|')")), expected);
    }
    assert_eq!(integer(eval(&mut context, "answer")), 42);
    assert!(!runtime.is_job_pending());
}

#[test]
fn await_handles_every_thenable_abrupt_and_first_settlement_boundary() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var getterResult = 'pending';
var callResult = 'pending';
var nonCallableResult = 'pending';
var repeatedResult = 'pending';
var getterThrow = {};
Object.defineProperty(getterThrow, 'then', {
    get: function () { throw 'getter'; }
});
var callThrow = {
    then: function () { throw 'call'; }
};
var nonCallable = { then: 1 };
var repeated = {
    then: function (resolve, reject) {
        resolve(42);
        reject('late reject');
        resolve(99);
        throw 'late throw';
    }
};
async function observe(value) {
    try {
        return await value;
    } catch (error) {
        return 'caught:' + error;
    }
}
observe(getterThrow).then(function (value) { getterResult = value; });
observe(callThrow).then(function (value) { callResult = value; });
observe(nonCallable).then(function (value) {
    nonCallableResult = value === nonCallable;
});
observe(repeated).then(function (value) { repeatedResult = value; });
"#,
    );
    assert!(drain(&runtime) >= 8);
    assert_eq!(
        text(eval(
            &mut context,
            r#"
[
    getterResult,
    callResult,
    nonCallableResult,
    repeatedResult
].join('|');
"#,
        )),
        "caught:getter|caught:call|true|42"
    );
}

#[test]
fn async_return_assimilates_promises_and_thenables_into_an_independent_outer_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        eval(
            &mut context,
            r#"
var inner = Promise.resolve(40);
var promiseResult = 'pending';
var thenableResult = 'pending';
var rejectionResult = 'pending';
async function returnPromise() { return inner; }
async function returnThenable() {
    return {
        then: function (resolve) { resolve(42); }
    };
}
async function returnRejectedPromise() {
    return Promise.reject('rejected');
}
var outer = returnPromise();
outer.then(function (value) { promiseResult = value; });
returnThenable().then(function (value) { thenableResult = value; });
returnRejectedPromise().then(undefined, function (error) {
    rejectionResult = error;
});
outer !== inner;
"#,
        ),
        Value::Bool(true)
    );
    assert!(drain(&runtime) >= 6);
    assert_eq!(
        text(eval(
            &mut context,
            "[promiseResult, thenableResult, rejectionResult].join('|')"
        )),
        "40|42|rejected"
    );
}

#[test]
fn await_uses_intrinsics_instead_of_mutable_promise_properties() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var IntrinsicPromise = Promise;
var originalThen = Promise.prototype.then;
var hits = [];
IntrinsicPromise.resolve = function () {
    hits.push('resolve');
    throw new Error('public resolve was called');
};
IntrinsicPromise.prototype.then = function () {
    hits.push('then');
    throw new Error('public then was called');
};
Promise = function FakePromise() {
    hits.push('constructor');
    throw new Error('global Promise was called');
};
var answer = 'pending';
async function intrinsicAwait() {
    return await 42;
}
var promise = intrinsicAwait();
originalThen.call(
    promise,
    function (value) { answer = value; },
    function (error) { answer = 'bad:' + error; }
);
"#,
    );
    assert_eq!(drain(&runtime), 2);
    assert_eq!(integer(eval(&mut context, "answer")), 42);
    assert_eq!(text(eval(&mut context, "hits.join('|')")), "");
}

#[test]
fn hidden_async_function_constructor_compiles_await_bodies() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
var AsyncFunctionConstructor =
    Object.getPrototypeOf(async function () {}).constructor;
var dynamic = AsyncFunctionConstructor('value', 'return await value');
var answer = 'pending';
dynamic(42).then(function (value) { answer = value; });
[
    'AsyncFunction' in globalThis,
    AsyncFunctionConstructor.name,
    AsyncFunctionConstructor.length,
    Object.getPrototypeOf(AsyncFunctionConstructor) === Function,
    AsyncFunctionConstructor.prototype ===
        Object.getPrototypeOf(async function () {}),
    dynamic.name,
    dynamic.length,
    Object.prototype.toString.call(dynamic),
    answer
].join('|');
"#,
        )),
        "false|AsyncFunction|1|true|true|anonymous|1|[object AsyncFunction]|pending"
    );
    assert_eq!(drain(&runtime), 2);
    assert_eq!(integer(eval(&mut context, "answer")), 42);
}

#[test]
fn direct_eval_keeps_async_function_variable_environments_alive() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var beforeResult = 'pending';
var afterResult = 'pending';
var parameterResult = 'pending';
async function beforeAwait() {
    let local = 40;
    eval('var added = 2');
    return local + added;
}
async function afterAwait() {
    let local = 40;
    await 0;
    return eval('local + 2');
}
async function parameterEval(seed = eval('var fromParameter = 40')) {
    return fromParameter + 2;
}
beforeAwait().then(function (value) { beforeResult = value; });
afterAwait().then(function (value) { afterResult = value; });
parameterEval().then(function (value) { parameterResult = value; });
"#,
    );
    assert!(drain(&runtime) >= 4);
    assert_eq!(
        text(eval(
            &mut context,
            "[beforeResult, afterResult, parameterResult].join('|')"
        )),
        "42|42|42"
    );
}

#[test]
fn cross_realm_call_uses_caller_promise_and_jobs_but_callee_body_realm() {
    let runtime = Runtime::new();
    let mut callee = runtime.new_context();
    let mut caller = runtime.new_context();
    let caller_realm = caller.realm_id();

    let Value::Object(function) = eval(
        &mut callee,
        r#"
(async function crossRealm() {
    await 0;
    throw new TypeError('callee');
})
"#,
    ) else {
        panic!("async source did not produce a function");
    };
    let function: CallableRef = runtime.as_callable(&function).unwrap().unwrap();
    let callee_type_error_prototype = object(eval(&mut callee, "TypeError.prototype"));
    let promise = object(
        caller
            .call(&function, Value::Undefined, &[])
            .expect("cross-realm async call"),
    );
    let caller_promise_prototype = object(eval(&mut caller, "Promise.prototype"));
    assert_eq!(
        runtime.get_prototype_of(&promise).unwrap(),
        Some(caller_promise_prototype)
    );

    define_global(&mut caller, "crossPromise", Value::Object(promise));
    eval(
        &mut caller,
        r#"
var crossReason;
crossPromise.then(undefined, function (error) {
    crossReason = error;
});
"#,
    );
    drop(callee);
    runtime.run_gc().unwrap();

    assert_eq!(
        runtime.execute_pending_job_with_context().unwrap(),
        Some(caller_realm),
        "the private await reaction belongs to the caller realm"
    );
    assert_eq!(
        runtime.execute_pending_job_with_context().unwrap(),
        Some(caller_realm),
        "the public Promise reaction also belongs to the caller realm"
    );
    let reason = object(eval(&mut caller, "crossReason"));
    assert_eq!(
        runtime.get_prototype_of(&reason).unwrap(),
        Some(callee_type_error_prototype),
        "the resumed body still executes in its defining realm"
    );
    assert_eq!(text(eval(&mut caller, "crossReason.message")), "callee");
    assert!(!runtime.is_job_pending());
}

#[test]
fn stack_preflight_returns_a_caller_promise_rejected_with_a_caller_error() {
    let runtime = Runtime::new();
    let mut callee = runtime.new_context();
    let mut caller = runtime.new_context();

    let Value::Object(function) = eval(
        &mut callee,
        "(async function stackBoundary() { return 42; })",
    ) else {
        panic!("async source did not produce a function");
    };
    let callee_internal_error_prototype = object(eval(&mut callee, "InternalError.prototype"));
    let caller_internal_error_prototype = object(eval(&mut caller, "InternalError.prototype"));
    define_global(&mut caller, "stackBoundary", Value::Object(function));

    assert_eq!(
        eval(
            &mut caller,
            r#"
var stackPromises = [];
var stackSynchronous;
var ordinaryOverflow;
function descendUntilOrdinaryOverflow(depth) {
    var promise;
    try {
        promise = stackBoundary();
    } catch (error) {
        stackSynchronous = error;
        return depth;
    }
    stackPromises.push(promise);
    try {
        return descendUntilOrdinaryOverflow(depth + 1);
    } catch (error) {
        ordinaryOverflow = error;
        return depth;
    }
}
descendUntilOrdinaryOverflow(0);
var stackReason;
for (var index = 0; index < stackPromises.length; index++) {
    stackPromises[index].then(undefined, function (error) {
        if (stackReason === undefined) {
            stackReason = error;
        }
    });
}
stackSynchronous === undefined &&
ordinaryOverflow instanceof InternalError &&
stackPromises.length !== 0 &&
stackPromises.every(function (promise) {
    return Object.getPrototypeOf(promise) === Promise.prototype;
});
"#,
        ),
        Value::Bool(true),
        "the host-stack preflight escaped as a synchronous throw"
    );
    let mut jobs = 0;
    while runtime.is_job_pending() {
        assert_eq!(
            runtime.execute_pending_job_with_context().unwrap(),
            Some(caller.realm_id())
        );
        jobs += 1;
    }
    assert!(jobs > 1);
    let reason = object(eval(&mut caller, "stackReason"));
    assert_eq!(
        runtime.get_prototype_of(&reason).unwrap(),
        Some(caller_internal_error_prototype),
        "the async preflight error was not allocated in the caller realm"
    );
    assert_ne!(
        runtime.get_prototype_of(&reason).unwrap(),
        Some(callee_internal_error_prototype),
        "the async preflight ran after switching to the callee realm"
    );
    assert_eq!(
        text(eval(
            &mut caller,
            "stackReason.name + ':' + stackReason.message"
        )),
        "InternalError:stack overflow"
    );
    assert!(!runtime.is_job_pending());
}
