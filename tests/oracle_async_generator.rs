//! Differential coverage for ordinary async-generator function declarations and
//! expressions.
//!
//! This milestone deliberately does not claim `yield*`, `for await`, object/class/
//! private method syntax, or `return` crossing an active iterator. Those surfaces
//! need their own implementation milestones and focused oracles.

use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{
    Context, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, Runtime, RuntimeError, Value,
};

struct SuccessCase {
    description: &'static str,
    source: &'static str,
    expected_stdout: &'static str,
}

const CORE_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "intrinsic graph callable and iterator descriptors",
        source: r#"
async function* sample(left, right = 0) {
    yield left + right;
}
var iterator = sample(19, 23);
var functionPrototype = Object.getPrototypeOf(sample);
var instancePrototype = Object.getPrototypeOf(iterator);
var asyncGeneratorPrototype = Object.getPrototypeOf(instancePrototype);
var asyncIteratorPrototype = Object.getPrototypeOf(asyncGeneratorPrototype);
var callablePrototypeDescriptor =
    Object.getOwnPropertyDescriptor(sample, "prototype");
var nextDescriptor =
    Object.getOwnPropertyDescriptor(asyncGeneratorPrototype, "next");
var returnDescriptor =
    Object.getOwnPropertyDescriptor(asyncGeneratorPrototype, "return");
var throwDescriptor =
    Object.getOwnPropertyDescriptor(asyncGeneratorPrototype, "throw");
print([
    typeof sample,
    sample.name,
    sample.length,
    Object.prototype.toString.call(sample),
    functionPrototype.constructor.name,
    functionPrototype.constructor.length,
    "AsyncGeneratorFunction" in globalThis,
    functionPrototype === Object.getPrototypeOf(async function* () {}),
    functionPrototype.prototype === asyncGeneratorPrototype,
    instancePrototype === sample.prototype,
    asyncIteratorPrototype[Symbol.asyncIterator].call(iterator) === iterator,
    Object.getPrototypeOf(asyncIteratorPrototype) === Object.prototype,
    Object.prototype.toString.call(iterator),
    callablePrototypeDescriptor.writable,
    callablePrototypeDescriptor.enumerable,
    callablePrototypeDescriptor.configurable,
    nextDescriptor.value.name,
    nextDescriptor.value.length,
    nextDescriptor.writable,
    nextDescriptor.enumerable,
    nextDescriptor.configurable,
    returnDescriptor.value.name,
    returnDescriptor.value.length,
    returnDescriptor.writable,
    returnDescriptor.enumerable,
    returnDescriptor.configurable,
    throwDescriptor.value.name,
    throwDescriptor.value.length,
    throwDescriptor.writable,
    throwDescriptor.enumerable,
    throwDescriptor.configurable,
    Object.getOwnPropertyDescriptor(
        asyncGeneratorPrototype,
        Symbol.toStringTag
    ).value,
    Object.getOwnPropertyDescriptor(
        asyncGeneratorPrototype,
        Symbol.toStringTag
    ).writable,
    Object.getOwnPropertyDescriptor(
        asyncGeneratorPrototype,
        Symbol.toStringTag
    ).enumerable,
    Object.getOwnPropertyDescriptor(
        asyncGeneratorPrototype,
        Symbol.toStringTag
    ).configurable
].join("|"));
"#,
        expected_stdout: concat!(
            "function|sample|1|[object AsyncGeneratorFunction]|",
            "AsyncGeneratorFunction|1|false|true|true|true|true|true|",
            "[object AsyncGenerator]|true|false|false|",
            "next|1|true|false|true|",
            "return|1|true|false|true|",
            "throw|1|true|false|true|",
            "AsyncGenerator|false|false|true\n",
        ),
    },
    SuccessCase {
        description: "parameters execute and throw synchronously before body start",
        source: r#"
var events = [];
async function* ok(
    value = (events.push("parameter"), 40)
) {
    events.push("body");
    yield value + 2;
}
async function* fail(
    value = (
        events.push("throw-parameter"),
        function () { throw new RangeError("parameter"); }
    )()
) {
    events.push("miss");
    yield value;
}
var iterator;
var synchronous = "none";
try {
    iterator = ok();
    events.push("after-ok-call");
} catch (error) {
    synchronous = "ok:" + error.name;
}
try {
    fail();
    events.push("miss-call");
} catch (error) {
    synchronous = error.name + ":" + error.message;
    events.push("caught");
}
print("call=" + [
    synchronous,
    events.join(","),
    Object.prototype.toString.call(iterator)
].join("|"));
var request = iterator.next();
events.push("after-next");
print("next=" + [
    Object.getPrototypeOf(request) === Promise.prototype,
    events.join(",")
].join("|"));
request.then(function (result) {
    events.push("settled");
    print("settled=" + [
        result.value,
        result.done,
        events.join(",")
    ].join("|"));
});
"#,
        expected_stdout: concat!(
            "call=RangeError:parameter|",
            "parameter,after-ok-call,throw-parameter,caught|",
            "[object AsyncGenerator]\n",
            "next=true|",
            "parameter,after-ok-call,throw-parameter,caught,body,after-next\n",
            "settled=42|false|",
            "parameter,after-ok-call,throw-parameter,caught,body,after-next,settled\n",
        ),
    },
    SuccessCase {
        description: "protocol methods always return promises including wrong receivers",
        source: r#"
var sample = (async function* () {})();
var intrinsic = Object.getPrototypeOf(Object.getPrototypeOf(sample));
var sync = [];
function invoke(label, callback) {
    try {
        var promise = callback();
        sync.push(
            label + ":" +
            (Object.getPrototypeOf(promise) === Promise.prototype)
        );
        return promise;
    } catch (error) {
        sync.push(label + ":throw:" + error.name);
        return Promise.reject(error);
    }
}
var requests = [
    invoke("next", function () {
        return (async function* () {})().next(1);
    }),
    invoke("return", function () {
        return (async function* () {})().return(20);
    }),
    invoke("throw", function () {
        return (async function* () {})().throw(22);
    }),
    invoke("wrong-next", function () {
        return intrinsic.next.call({});
    }),
    invoke("wrong-return", function () {
        return intrinsic.return.call({});
    }),
    invoke("wrong-throw", function () {
        return intrinsic.throw.call({});
    })
];
print("sync=" + sync.join("|"));
function result(promise) {
    return promise.then(
        function (record) {
            return "ok:" + String(record.value) + ":" + record.done;
        },
        function (error) {
            return "reject:" +
                (error && error.name ? error.name : String(error));
        }
    );
}
Promise.all(requests.map(result)).then(function (values) {
    print("settled=" + values.join("|"));
});
"#,
        expected_stdout: concat!(
            "sync=next:true|return:true|throw:true|",
            "wrong-next:true|wrong-return:true|wrong-throw:true\n",
            "settled=ok:undefined:true|ok:20:true|reject:22|",
            "reject:TypeError|reject:TypeError|reject:TypeError\n",
        ),
    },
    SuccessCase {
        description: "first next input is ignored and yield and return await assimilate",
        source: r#"
var events = [];
var thenable = {};
Object.defineProperty(thenable, "then", {
    get: function () {
        events.push("get");
        return function (resolve) {
            events.push("call");
            resolve(20);
        };
    }
});
async function* flow() {
    events.push("body");
    var resumed = yield await thenable;
    events.push("resume:" + resumed);
    return await Promise.resolve(resumed + 2);
}
var iterator = flow();
var first = iterator.next(999);
events.push("after-first");
print("sync=" + events.join("|"));
first.then(function (result) {
    events.push("first:" + result.value + ":" + result.done);
    print("first=" + events.join("|"));
    var second = iterator.next(40);
    events.push("after-second");
    second.then(function (result) {
        events.push("second:" + result.value + ":" + result.done);
        print("second=" + events.join("|"));
    });
});
"#,
        expected_stdout: concat!(
            "sync=body|get|after-first\n",
            "first=body|get|after-first|call|first:20:false\n",
            "second=body|get|after-first|call|first:20:false|",
            "resume:40|after-second|second:42:true\n",
        ),
    },
    SuccessCase {
        description: "queued requests resume await in FIFO order and drain after completion",
        source: r#"
var events = [];
var release;
var gate = new Promise(function (resolve) {
    release = resolve;
});
async function* queued() {
    events.push("start");
    var value = await gate;
    events.push("after-await:" + value);
    var resumed = yield value;
    events.push("after-yield:" + resumed);
    return Promise.resolve(value + 2);
}
var iterator = queued();
var first = iterator.next();
var second = iterator.next(100);
var third = iterator.next(200);
events.push("queued");
print("sync=" + [
    first,
    second,
    third
].every(function (promise) {
    return Object.getPrototypeOf(promise) === Promise.prototype;
}) + "|" + events.join("|"));
function show(result) {
    return String(result.value) + ":" + result.done;
}
Promise.all([first, second, third]).then(function (results) {
    print("drain=" + results.map(show).join("|") + "|" + events.join(","));
});
release(40);
"#,
        expected_stdout: concat!(
            "sync=true|start|queued\n",
            "drain=40:false|42:true|undefined:true|",
            "start,queued,after-await:40,after-yield:100\n",
        ),
    },
    SuccessCase {
        description: "return runs and awaits finally before settling",
        source: r#"
var events = [];
async function* guarded() {
    try {
        events.push("try");
        yield 1;
        events.push("miss");
    } finally {
        events.push("finally-start");
        await 0;
        events.push("finally-end");
    }
}
var iterator = guarded();
var first = iterator.next();
events.push("sync");
print("sync=" + events.join("|"));
first.then(function (result) {
    events.push("yield:" + result.value + ":" + result.done);
    var closing = iterator.return(Promise.resolve(42));
    events.push("after-return");
    closing.then(function (result) {
        events.push("return:" + result.value + ":" + result.done);
        print("settled=" + events.join("|"));
    });
});
"#,
        expected_stdout: concat!(
            "sync=try|sync\n",
            "settled=try|sync|yield:1:false|after-return|",
            "finally-start|finally-end|return:42:true\n",
        ),
    },
    SuccessCase {
        description: "poisoned Promise constructor rejects await in the same driver entry",
        source: r#"
var events = [];
var promise = Promise.resolve(1);
Object.defineProperty(promise, "constructor", {
    get: function () {
        events.push("get");
        throw "boom";
    }
});
async function* flow() {
    try {
        await promise;
    } catch (error) {
        events.push("catch:" + error);
    }
    events.push("after");
    yield 42;
}
var request = flow().next();
events.push("after-next");
print(events.join("|"));
request.then(function (result) {
    events.push("settled:" + result.value + ":" + result.done);
    print(events.join("|"));
});
"#,
        expected_stdout: concat!(
            "get|catch:boom|after|after-next\n",
            "get|catch:boom|after|after-next|settled:42:false\n",
        ),
    },
    SuccessCase {
        description: "poisoned completed return remains an asynchronous rejection",
        source: r#"
var events = [];
var promise = Promise.resolve(1);
Object.defineProperty(promise, "constructor", {
    get: function () {
        events.push("get");
        throw "boom";
    }
});
var iterator = (async function* () {})();
var request;
try {
    request = iterator.return(promise);
    events.push(
        "return:" +
        (Object.getPrototypeOf(request) === Promise.prototype)
    );
} catch (error) {
    events.push("throw:" + error);
}
events.push("after");
print(events.join("|"));
request.then(
    function () {
        events.push("ok");
    },
    function (error) {
        events.push("reject:" + error);
        print(events.join("|"));
    }
);
"#,
        expected_stdout: concat!(
            "get|return:true|after\n",
            "get|return:true|after|reject:boom\n",
        ),
    },
    SuccessCase {
        description: "iterator-result then getter reentry follows the active QuickJS driver",
        source: r#"
var iterator;
var events = [];
Object.defineProperty(Object.prototype, "then", {
    configurable: true,
    get: function () {
        events.push("get");
        delete Object.prototype.then;
        iterator.next().then(function (result) {
            events.push("nested:" + result.value + ":" + result.done);
        });
        return undefined;
    }
});
async function* reentrant() {
    yield 1;
    yield 2;
}
iterator = reentrant();
iterator.next().then(function (result) {
    events.push("outer:" + result.value + ":" + result.done);
});
Promise.resolve()
    .then(function () {})
    .then(function () {})
    .then(function () {
        print(events.join("|"));
    });
"#,
        expected_stdout: concat!("get|outer:1:false|", "nested:undefined:false\n",),
    },
];

#[test]
fn ordinary_async_generator_function_core_matches_pinned_quickjs() {
    compare_success_cases("ordinary async-generator function core", CORE_CASES);
}

#[test]
fn completed_state_services_one_trailing_request_per_driver_entry_like_quickjs() {
    const SOURCE: &str = r#"
var release;
var gate = new Promise(function (resolve) {
    release = resolve;
});
async function* queued() {
    var value = await gate;
    var resumed = yield value;
    return Promise.resolve(value + 2);
}
var iterator = queued();
var requests = [
    iterator.next(),
    iterator.next(100),
    iterator.next(200),
    iterator.next(300)
];
var settled = [false, false, false, false];
requests.forEach(function (request, index) {
    request.then(function () {
        settled[index] = true;
    });
});
requests[2].then(function (result) {
    Promise.resolve().then(function () {
        print(
            settled.join(",") + "|" +
            String(result.value) + ":" + result.done
        );
        var fifthSettled = false;
        var fifth = iterator.next();
        fifth.then(function () {
            fifthSettled = true;
        });
        requests[3].then(function (trailingResult) {
            Promise.resolve().then(function () {
                print(
                    settled.join(",") + "," + fifthSettled + "|" +
                    String(trailingResult.value) + ":" +
                    trailingResult.done
                );
            });
        });
    });
});
release(40);
"#;
    const EXPECTED_STDOUT: &str = concat!(
        "true,true,true,false|undefined:true\n",
        "true,true,true,true,false|undefined:true\n",
    );

    let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), SOURCE);
    assert!(
        oxide.status.success(),
        "quickjs-oxide rejected completed-state queue probe: {}\nsource:\n{}",
        String::from_utf8_lossy(&oxide.stderr),
        SOURCE
    );
    assert_eq!(
        String::from_utf8_lossy(&oxide.stdout),
        EXPECTED_STDOUT,
        "quickjs-oxide completed-state driver entry drifted from pinned QuickJS"
    );

    if let Some(oracle) = std::env::var_os("QJS_ORACLE") {
        let quickjs = run(&oracle, SOURCE);
        assert!(
            quickjs.status.success(),
            "pinned QuickJS rejected completed-state queue probe: {}",
            String::from_utf8_lossy(&quickjs.stderr)
        );
        assert_eq!(quickjs.stdout, oxide.stdout);
    }
}

#[test]
fn repeated_poisoned_promise_awaits_use_a_bounded_native_stack() {
    const SOURCE: &str = r#"
var promise = Promise.resolve(0);
Object.defineProperty(promise, "constructor", {
    get: function () { throw 0; }
});
async function* repeated() {
    var caught = 0;
    for (var index = 0; index < 20000; index++) {
        try {
            await promise;
        } catch (error) {
            caught++;
        }
    }
    yield caught;
}
repeated().next().then(function (result) {
    print(result.value + ":" + result.done);
});
"#;
    let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), SOURCE);
    assert!(
        oxide.status.success(),
        "quickjs-oxide overflowed or rejected repeated immediate await rejection: {}",
        String::from_utf8_lossy(&oxide.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&oxide.stdout), "20000:false\n");
}

#[test]
fn cross_realm_settlement_resumes_in_the_job_realm_and_executes_in_the_body_realm() {
    let runtime = Runtime::new();
    let mut body = runtime.new_context();
    let mut settler = runtime.new_context();
    let settler_realm = settler.realm_id();

    let gate = object(eval(
        &mut settler,
        r#"
var release;
var gate = new Promise(function (resolve) {
    release = resolve;
});
gate
"#,
    ));
    define_global(&mut body, "foreignGate", Value::Object(gate));
    eval(
        &mut body,
        r#"
var crossRealmReason;
async function* crossRealm() {
    await foreignGate;
    throw new TypeError("callee");
}
crossRealm().next().then(undefined, function (error) {
    crossRealmReason = error;
});
"#,
    );

    // PromiseResolve in the body realm first installs the thenable bridge.
    while runtime.is_job_pending() {
        runtime.execute_pending_job_with_context().unwrap();
    }
    eval(&mut settler, "release(1)");

    let mut saw_settler_realm = false;
    while runtime.is_job_pending() {
        let job_realm = runtime
            .execute_pending_job_with_context()
            .unwrap()
            .expect("pending job had no realm");
        saw_settler_realm |= job_realm == settler_realm;
    }
    assert!(
        saw_settler_realm,
        "foreign Promise settlement did not carry its actual job realm"
    );

    let reason = object(eval(&mut body, "crossRealmReason"));
    let body_type_error_prototype = object(eval(&mut body, "TypeError.prototype"));
    assert_eq!(
        runtime.get_prototype_of(&reason).unwrap(),
        Some(body_type_error_prototype),
        "the resumed async-generator body did not retain its defining realm"
    );
    assert_eq!(text(eval(&mut body, "crossRealmReason.message")), "callee");
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

fn object(value: Value) -> ObjectRef {
    let Value::Object(value) = value else {
        panic!("expected an object");
    };
    value
}

fn text(value: Value) -> String {
    let Value::String(value) = value else {
        panic!("expected a string");
    };
    value.to_utf8_lossy()
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

fn compare_success_cases(group: &str, cases: &[SuccessCase]) {
    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!("SKIP {group} differential: set QJS_ORACLE to pinned upstream qjs");
    }

    for case in cases {
        let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), case.source);
        assert_success("quickjs-oxide", case, &oxide);

        if let Some(oracle) = &oracle {
            let quickjs = run(oracle, case.source);
            assert_success("pinned QuickJS", case, &quickjs);
            assert_eq!(
                oxide.stdout, quickjs.stdout,
                "{group} output differed for {}",
                case.description
            );
        }
    }
}

fn assert_success(engine: &str, case: &SuccessCase, output: &Output) {
    assert!(
        output.status.success(),
        "{engine} rejected {}: {}\nsource:\n{}",
        case.description,
        String::from_utf8_lossy(&output.stderr),
        case.source
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        case.expected_stdout,
        "{engine} output drifted for {}",
        case.description
    );
}

fn run(executable: &OsStr, source: &str) -> Output {
    Command::new(executable)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run {executable:?}: {error}"))
}
