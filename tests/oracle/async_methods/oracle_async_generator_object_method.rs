//! Differential coverage for ordinary object-literal async-generator methods.
//!
//! Class/private methods, `yield*`, `for await`, and return across an active
//! iterator remain separate fail-closed milestones. Destructuring parameters
//! remain outside this milestone's authenticated scope.

use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

struct SuccessCase {
    description: &'static str,
    source: &'static str,
    expected_stdout: &'static str,
}

const SEMANTIC_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "shape source descriptors prototype graph and nonconstructibility",
        source: r#"
var object = {
    async /*a*/ * /*b*/ method /*c*/ (left, right = 2) {
        yield left + right;
    }
};
var method = object.method;
var iterator = method(40);
var descriptor = Object.getOwnPropertyDescriptor(object, "method");
var prototypeDescriptor = Object.getOwnPropertyDescriptor(method, "prototype");
var functionPrototype = Object.getPrototypeOf(method);
var instancePrototype = Object.getPrototypeOf(iterator);
var asyncGeneratorPrototype = Object.getPrototypeOf(instancePrototype);
var constructError = "none";
var newTargetError = "none";
try {
    new method();
} catch (error) {
    constructError = error.name;
}
try {
    Reflect.construct(function () {}, [], method);
} catch (error) {
    newTargetError = error.name;
}
print([
    typeof method,
    method.name,
    method.length,
    Object.prototype.toString.call(method),
    functionPrototype.constructor.name,
    functionPrototype === Object.getPrototypeOf(async function* () {}),
    Object.prototype.hasOwnProperty.call(method, "prototype"),
    instancePrototype === method.prototype,
    Object.getPrototypeOf(method.prototype) === asyncGeneratorPrototype,
    Object.prototype.toString.call(iterator),
    descriptor.writable,
    descriptor.enumerable,
    descriptor.configurable,
    prototypeDescriptor.writable,
    prototypeDescriptor.enumerable,
    prototypeDescriptor.configurable,
    Function.prototype.toString.call(method),
    constructError,
    newTargetError
].join("|"));
"#,
        expected_stdout: concat!(
            "function|method|1|[object AsyncGeneratorFunction]|",
            "AsyncGeneratorFunction|true|true|true|true|[object AsyncGenerator]|",
            "true|true|true|true|false|false|",
            "async /*a*/ * /*b*/ method /*c*/ (left, right = 2) {\n",
            "        yield left + right;\n",
            "    }|TypeError|TypeError\n",
        ),
    },
    SuccessCase {
        description: "computed and symbol names preserve ordering and proto spelling",
        source: r#"
var events = [];
var key = {
    toString: function () {
        events.push("key");
        return "computed";
    }
};
var described = Symbol("sym");
var anonymous = Symbol();
var object = {
    before: (events.push("before"), 0),
    async *fixed() {},
    async *["str-key"]() {},
    async *[1]() {},
    async *[described]() {},
    async *[anonymous]() {},
    async *[key]() { yield 42; },
    async *__proto__() {
        yield Object.getPrototypeOf(this) === Object.prototype;
    },
    after: (events.push("after"), 0)
};
print([
    events.join(","),
    object.fixed.name,
    object["str-key"].name,
    object[1].name,
    object[described].name,
    object[anonymous].name,
    object.computed.name,
    object.__proto__.name,
    Object.getPrototypeOf(object) === Object.prototype,
    Object.keys(object).join(","),
    Reflect.ownKeys(object).length
].join("|"));
object.computed().next().then(function (result) {
    print(result.value + ":" + result.done);
});
object.__proto__().next().then(function (result) {
    print(result.value + ":" + result.done);
});
"#,
        expected_stdout: concat!(
            "before,key,after|fixed|str-key|1|[sym]||computed|__proto__|true|",
            "1,before,fixed,str-key,computed,__proto__,after|9\n",
            "42:false\n",
            "true:false\n",
        ),
    },
    SuccessCase {
        description: "parameters and home object survive await yield and borrowing",
        source: r#"
var events = [];
var base = {
    get x() {
        return this.seed + 2;
    },
    add(value) {
        return this.seed + value;
    }
};
var object = {
    __proto__: base,
    seed: 40,
    async *method(
        left = (events.push("parameter"), super.x),
        right = 2
    ) {
        events.push("body");
        var before = super.x;
        await 0;
        var sent = yield left + right;
        return [
            before,
            super.x,
            super.add(2),
            this.seed,
            sent
        ].join(",");
    }
};
var method = object.method;
var iterator = method.call({ seed: 100 });
events.push("after-call");
print("call=" + events.join(","));
var first = iterator.next();
events.push("after-next");
print("next=" + events.join(","));
first.then(function (result) {
    print("first=" + result.value + ":" + result.done);
});
iterator.next(7).then(function (result) {
    print("second=" + result.value + ":" + result.done);
});
"#,
        expected_stdout: concat!(
            "call=parameter,after-call\n",
            "next=parameter,after-call,body,after-next\n",
            "first=104:false\n",
            "second=102,102,102,100,7:true\n",
        ),
    },
    SuccessCase {
        description: "parameter abrupt completion is synchronous and body stays suspended",
        source: r#"
var events = [];
var object = {
    async *ok(value = (events.push("parameter"), 42)) {
        events.push("body");
        yield value;
    },
    async *fail(
        value = (
            events.push("throw-parameter"),
            function () { throw new RangeError("parameter"); }
        )()
    ) {
        events.push("miss-body");
        yield value;
    }
};
var iterator = object.ok();
events.push("after-ok");
var synchronous = "none";
try {
    object.fail();
    events.push("miss-call");
} catch (error) {
    synchronous = error.name + ":" + error.message;
    events.push("caught");
}
print("call=" + synchronous + "|" + events.join(","));
iterator.next().then(function (result) {
    events.push("settled");
    print("next=" + result.value + ":" + result.done + "|" + events.join(","));
});
"#,
        expected_stdout: concat!(
            "call=RangeError:parameter|parameter,after-ok,throw-parameter,caught\n",
            "next=42:false|parameter,after-ok,throw-parameter,caught,body,settled\n",
        ),
    },
];

const VALID_CONTEXTUAL_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "comment separators and computed keyword names",
        source: "var object = { async /*\u{2028}*/ * ['await']() { yield 42; } };\n\
                 object.await().next().then(function (result) { print(result.value); });",
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "async before parens stays an ordinary method name",
        source: r#"
var object = { async() { return 42; } };
print(object.async() + "|" + Object.prototype.toString.call(object.async));
"#,
        expected_stdout: "42|[object Function]\n",
    },
];

const INVALID_CONTEXTUAL_CASES: &[(&str, &str)] = &[
    (
        "a line terminator prevents async from prefixing the star",
        "var object = { async\n*method() {} };",
    ),
    (
        "a multiline-comment terminator prevents the async prefix",
        "var object = { async /*\n*/ *method() {} };",
    ),
    (
        "a direct U+2028 terminator prevents the async prefix",
        "var object = { async\u{2028}*method() {} };",
    ),
    (
        "escaped async cannot act as the contextual prefix",
        r"var object = { as\u0079nc *method() {} };",
    ),
    (
        "duplicate parameters are forbidden",
        "var object = { async *method(value, value) {} };",
    ),
    (
        "await is forbidden as a parameter binding",
        "var object = { async *method(await) {} };",
    ),
    (
        "yield is forbidden as a parameter binding",
        "var object = { async *method(yield) {} };",
    ),
    (
        "await expressions are forbidden in parameters",
        "var object = { async *method(value = await 1) {} };",
    ),
    (
        "yield expressions are forbidden in parameters",
        "var object = { async *method(value = yield 1) {} };",
    ),
    (
        "a non-simple parameter list forbids a strict body directive",
        "var object = { async *method(value = 1) { 'use strict'; } };",
    ),
    (
        "super calls are forbidden in object methods",
        "var object = { async *method() { super(); } };",
    ),
];

#[test]
fn ordinary_async_generator_object_method_semantics_match_pinned_quickjs() {
    compare_success_cases("async-generator object method", SEMANTIC_CASES);
}

#[test]
fn async_generator_object_method_contextual_boundaries_match_pinned_quickjs() {
    compare_success_cases(
        "async-generator object-method contextual token",
        VALID_CONTEXTUAL_CASES,
    );

    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!(
            "SKIP async-generator object-method rejection differential: \
             set QJS_ORACLE to pinned upstream qjs"
        );
    }
    for (description, source) in INVALID_CONTEXTUAL_CASES {
        let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), source);
        assert!(
            !oxide.status.success(),
            "quickjs-oxide accepted invalid {description}: {source:?}"
        );
        if let Some(oracle) = &oracle {
            let quickjs = run(oracle, source);
            assert!(
                !quickjs.status.success(),
                "pinned QuickJS accepted invalid {description}: {source:?}"
            );
        }
    }
}

#[test]
fn suspended_method_retains_its_home_object_across_gc() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var outcome = "pending";
var base = {
    get answer() {
        return this.seed + 2;
    }
};
var holder = {
    __proto__: base,
    async *method() {
        await 0;
        yield super.answer;
    }
};
var iterator = holder.method.call({ seed: 40 });
holder = null;
base = null;
iterator.next().then(function (result) {
    outcome = result.value + ":" + result.done;
});
"#,
    );
    runtime.run_gc().unwrap();
    while runtime.is_job_pending() {
        runtime.execute_pending_job().unwrap();
        runtime.run_gc().unwrap();
    }
    assert_eq!(text(eval(&mut context, "outcome")), "42:false");
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
