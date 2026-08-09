//! Differential coverage for public class async-generator methods.
//!
//! Private methods, `yield*`, `for await`, and return across an active
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
        description: "instance and static shape names source descriptors and prototypes",
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
class C {
    async /*i0*/ * /*i1*/ fixed /*i2*/ (left, right = 2) {
        yield left + right;
    }
    async *["str-key"]() {}
    async *[1]() {}
    async *[described]() {}
    async *[anonymous]() {}
    async *[key]() { yield 42; }
    static async /*s0*/ * /*s1*/ fixedStatic /*s2*/ (value) {
        yield value;
    }
    static async *["static-key"]() {}
    static async *[2]() {}
    static async *[described]() {}
}
var method = C.prototype.fixed;
var staticMethod = C.fixedStatic;
var iterator = method(40);
var descriptor = Object.getOwnPropertyDescriptor(C.prototype, "fixed");
var staticDescriptor = Object.getOwnPropertyDescriptor(C, "fixedStatic");
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
    Reflect.construct(function () {}, [], staticMethod);
} catch (error) {
    newTargetError = error.name;
}
print([
    events.join(","),
    C.prototype.fixed.name,
    C.prototype["str-key"].name,
    C.prototype[1].name,
    C.prototype[described].name,
    C.prototype[anonymous].name,
    C.prototype.computed.name,
    C.fixedStatic.name,
    C["static-key"].name,
    C[2].name,
    C[described].name,
    typeof method,
    method.length,
    staticMethod.length,
    Object.prototype.toString.call(method),
    functionPrototype.constructor.name,
    functionPrototype === Object.getPrototypeOf(async function* () {}),
    Object.prototype.hasOwnProperty.call(method, "prototype"),
    Object.prototype.hasOwnProperty.call(staticMethod, "prototype"),
    instancePrototype === method.prototype,
    Object.getPrototypeOf(method.prototype) === asyncGeneratorPrototype,
    Object.prototype.toString.call(iterator),
    descriptor.writable,
    descriptor.enumerable,
    descriptor.configurable,
    staticDescriptor.writable,
    staticDescriptor.enumerable,
    staticDescriptor.configurable,
    prototypeDescriptor.writable,
    prototypeDescriptor.enumerable,
    prototypeDescriptor.configurable,
    Object.keys(C.prototype).length,
    Object.keys(C).length,
    Function.prototype.toString.call(method),
    Function.prototype.toString.call(staticMethod),
    constructError,
    newTargetError
].join("|"));
"#,
        expected_stdout: concat!(
            "key|fixed|str-key|1|[sym]||computed|fixedStatic|static-key|2|[sym]|",
            "function|1|1|[object AsyncGeneratorFunction]|AsyncGeneratorFunction|",
            "true|true|true|true|true|[object AsyncGenerator]|",
            "true|false|true|true|false|true|true|false|false|0|0|",
            "async /*i0*/ * /*i1*/ fixed /*i2*/ (left, right = 2) {\n",
            "        yield left + right;\n",
            "    }|async /*s0*/ * /*s1*/ fixedStatic /*s2*/ (value) {\n",
            "        yield value;\n",
            "    }|TypeError|TypeError\n",
        ),
    },
    SuccessCase {
        description: "computed constructor publishes while computed static prototype throws",
        source: r#"
var constructorCalls = 0;
var events = [];
class C {
    constructor() {
        constructorCalls++;
    }
    async *["constructor"]() {
        yield 40;
    }
    async *prototype() {
        yield 2;
    }
    static async *constructor() {
        yield 1;
    }
}
var instance = new C();
var constructorDescriptor =
    Object.getOwnPropertyDescriptor(C.prototype, "constructor");
var staticPrototypeError = "none";
var Broken;
try {
    Broken = class {
        static async *[(events.push("prototype-key"), "prototype")]() {}
    };
    events.push("defined");
} catch (error) {
    staticPrototypeError = error.name;
}
print([
    constructorCalls,
    instance instanceof C,
    C.name,
    instance.constructor.name,
    instance.prototype.name,
    C.constructor.name,
    constructorDescriptor.writable,
    constructorDescriptor.enumerable,
    constructorDescriptor.configurable,
    staticPrototypeError,
    events.join(","),
    String(Broken)
].join("|"));
instance.constructor().next().then(function (result) {
    print("computed-constructor=" + result.value + ":" + result.done);
});
instance.prototype().next().then(function (result) {
    print("instance-prototype=" + result.value + ":" + result.done);
});
C.constructor().next().then(function (result) {
    print("static-constructor=" + result.value + ":" + result.done);
});
"#,
        expected_stdout: concat!(
            "1|true|C|constructor|prototype|constructor|true|false|true|",
            "TypeError|prototype-key|undefined\n",
            "computed-constructor=40:false\n",
            "instance-prototype=2:false\n",
            "static-constructor=1:false\n",
        ),
    },
    SuccessCase {
        description: "parameters are synchronous while body await and yield are delayed",
        source: r#"
var events = [];
class C {
    async *ok(
        first,
        second = (events.push("parameter"), arguments[0] + 2)
    ) {
        events.push("body");
        await 0;
        events.push("after-await");
        var sent = yield [
            arguments.length,
            arguments[0],
            second,
            new.target === undefined
        ].join(",");
        return sent + 1;
    }
    async *fail(
        value = (
            events.push("throw-parameter"),
            function () { throw new RangeError("parameter"); }
        )()
    ) {
        events.push("miss-body");
        yield value;
    }
}
var value = new C();
var iterator = value.ok(40);
events.push("after-call");
var synchronous = "none";
try {
    value.fail();
    events.push("miss-call");
} catch (error) {
    synchronous = error.name + ":" + error.message;
    events.push("caught");
}
print("call=" + synchronous + "|" + events.join(","));
var first = iterator.next();
events.push("after-next");
print("next=" + events.join(","));
first.then(function (result) {
    print("first=" + result.value + ":" + result.done + "|" + events.join(","));
});
iterator.next(41).then(function (result) {
    print("second=" + result.value + ":" + result.done);
});
"#,
        expected_stdout: concat!(
            "call=RangeError:parameter|parameter,after-call,throw-parameter,caught\n",
            "next=parameter,after-call,throw-parameter,caught,body,after-next\n",
            "first=1,40,42,true:false|",
            "parameter,after-call,throw-parameter,caught,body,after-next,after-await\n",
            "second=42:true\n",
        ),
    },
    SuccessCase {
        description: "base and derived super retain home objects with borrowed receivers",
        source: r#"
class Standalone {
    constructor(seed) {
        this.seed = seed;
    }
    async *read(fromParameter = super.toString()) {
        await 0;
        var sent = yield [
            fromParameter,
            super.hasOwnProperty("seed"),
            this.seed
        ].join(",");
        return sent + this.seed;
    }
    static async *read(
        fromParameter = super.call === Function.prototype.call
    ) {
        await 0;
        yield [fromParameter, this.name].join(",");
    }
}
class Base {
    get answer() { return this.seed + 2; }
    add(delta) { return this.seed + delta; }
    static get answer() { return this.seed + 2; }
    static add(delta) { return this.seed + delta; }
}
class Derived extends Base {
    constructor(seed) {
        super();
        this.seed = seed;
    }
    async *read(fromParameter = super.answer) {
        var before = super.answer;
        await 0;
        var sent = yield [
            fromParameter,
            before,
            super.answer,
            super.add(2),
            this.seed
        ].join(",");
        return super.add(sent);
    }
    static async *read(fromParameter = super.answer) {
        var before = super.answer;
        await 0;
        var sent = yield [
            fromParameter,
            before,
            super.answer,
            super.add(2),
            this.seed
        ].join(",");
        return super.add(sent);
    }
}
Derived.seed = 40;

function consume(label, iterator, sent) {
    return iterator.next().then(function (first) {
        print(label + "-first=" + first.value + ":" + first.done);
        return iterator.next(sent);
    }).then(function (second) {
        print(label + "-second=" + second.value + ":" + second.done);
    });
}

print("sync=ready");
consume("base-instance", new Standalone(40).read(), 2)
.then(function () {
    return consume(
        "base-borrowed",
        Standalone.prototype.read.call({ seed: 100 }),
        2
    );
})
.then(function () {
    return consume(
        "base-static",
        Standalone.read.call({ name: "borrowed-static" }),
        0
    );
})
.then(function () {
    return consume("derived-instance", new Derived(40).read(), 2);
})
.then(function () {
    return consume(
        "derived-borrowed",
        Derived.prototype.read.call({ seed: 100 }),
        2
    );
})
.then(function () {
    return consume("derived-static", Derived.read(), 2);
})
.then(function () {
    return consume(
        "derived-static-borrowed",
        Derived.read.call({ seed: 100 }),
        2
    );
});
"#,
        expected_stdout: concat!(
            "sync=ready\n",
            "base-instance-first=[object Object],true,40:false\n",
            "base-instance-second=42:true\n",
            "base-borrowed-first=[object Object],true,100:false\n",
            "base-borrowed-second=102:true\n",
            "base-static-first=true,borrowed-static:false\n",
            "base-static-second=undefined:true\n",
            "derived-instance-first=42,42,42,42,40:false\n",
            "derived-instance-second=42:true\n",
            "derived-borrowed-first=102,102,102,102,100:false\n",
            "derived-borrowed-second=102:true\n",
            "derived-static-first=42,42,42,42,40:false\n",
            "derived-static-second=42:true\n",
            "derived-static-borrowed-first=102,102,102,102,100:false\n",
            "derived-static-borrowed-second=102:true\n",
        ),
    },
];

const VALID_CONTEXTUAL_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "instance and static async star prefixes",
        source: r#"
class C {
    async *instance() { yield await 20; }
    static async *method() { yield await 22; }
}
new C().instance().next().then(function (left) {
    C.method().next().then(function (right) {
        print(left.value + right.value);
    });
});
"#,
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "comment separators computed names and escaped property names",
        source: "var key = \"left\";\n\
                 class C {\n\
                     async /*\u{2028}*/ *[key]() { yield await 20; }\n\
                     static async /*\u{2029}*/ *r\\u0069ght() { yield await 22; }\n\
                 }\n\
                 new C().left().next().then(function (left) {\n\
                     C.right().next().then(function (right) {\n\
                         print(left.value + right.value);\n\
                     });\n\
                 });",
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "line terminators split async fields from generator methods",
        source: r#"
class C {
    async
    *instance() { yield 20; }
    static async
    *method() { yield 22; }
}
var value = new C();
print([
    Object.prototype.hasOwnProperty.call(value, "async"),
    String(value.async),
    Object.prototype.hasOwnProperty.call(C, "async"),
    String(C.async),
    Object.prototype.toString.call(C.prototype.instance),
    Object.prototype.toString.call(C.prototype.method),
    value.instance().next().value + value.method().next().value
].join("|"));
"#,
        expected_stdout: concat!(
            "true|undefined|true|undefined|",
            "[object GeneratorFunction]|[object GeneratorFunction]|42\n",
        ),
    },
    SuccessCase {
        description: "async immediately before parens remains an ordinary method name",
        source: r#"
class C {
    async() { return 20; }
    static async() { return 22; }
}
print(new C().async() + C.async() + "|" + [
    Object.prototype.toString.call(C.prototype.async),
    Object.prototype.toString.call(C.async)
].join("|"));
"#,
        expected_stdout: "42|[object Function]|[object Function]\n",
    },
];

const INVALID_CONTEXTUAL_CASES: &[(&str, &str)] = &[
    (
        "an instance async-generator constructor is forbidden",
        "class C { async *constructor() {} }",
    ),
    (
        "a static async-generator prototype method is forbidden",
        "class C { static async *prototype() {} }",
    ),
    (
        "async star cannot prefix an accessor declaration",
        "class C { async *get value() {} }",
    ),
    (
        "escaped async cannot act as the contextual prefix",
        r"class C { as\u0079nc *method() {} }",
    ),
    (
        "strict class methods reject duplicate parameters",
        "class C { async *method(value, value) {} }",
    ),
    (
        "await is forbidden as an async-generator parameter binding",
        "class C { async *method(await) {} }",
    ),
    (
        "yield is forbidden as an async-generator parameter binding",
        "class C { async *method(yield) {} }",
    ),
    (
        "await expressions are forbidden in async-generator parameters",
        "class C { async *method(value = await 1) {} }",
    ),
    (
        "yield expressions are forbidden in async-generator parameters",
        "class C { async *method(value = yield 1) {} }",
    ),
    (
        "a non-simple parameter list forbids a strict body directive",
        "class C { async *method(value = 1) { 'use strict'; } }",
    ),
    (
        "super calls are forbidden in async-generator class methods",
        "class C extends Object { async *method() { super(); } }",
    ),
];

#[test]
fn pinned_quickjs_expectations_are_authenticated() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP async-generator class-method expectation authentication: \
             set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };

    for case in SEMANTIC_CASES.iter().chain(VALID_CONTEXTUAL_CASES.iter()) {
        let quickjs = run(&oracle, case.source);
        assert_success("pinned QuickJS", case, &quickjs);
    }
    for (description, source) in INVALID_CONTEXTUAL_CASES {
        let quickjs = run(&oracle, source);
        assert!(
            !quickjs.status.success(),
            "pinned QuickJS accepted invalid {description}: {source:?}"
        );
    }
}

#[test]
fn public_async_generator_class_method_semantics_match_pinned_quickjs() {
    compare_success_cases("public async-generator class method", SEMANTIC_CASES);
}

#[test]
fn async_generator_class_method_contextual_boundaries_match_pinned_quickjs() {
    compare_success_cases(
        "async-generator class-method contextual token",
        VALID_CONTEXTUAL_CASES,
    );

    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!(
            "SKIP async-generator class-method rejection differential: \
             set QJS_ORACLE to pinned upstream qjs"
        );
    }
    for (description, source) in INVALID_CONTEXTUAL_CASES {
        if let Some(oracle) = &oracle {
            let quickjs = run(oracle, source);
            assert!(
                !quickjs.status.success(),
                "pinned QuickJS accepted invalid {description}: {source:?}"
            );
        }

        let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), source);
        assert!(
            !oxide.status.success(),
            "quickjs-oxide accepted invalid {description}: {source:?}"
        );
    }
}

#[test]
fn suspended_class_methods_retain_home_objects_across_gc() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var instanceOutcome = "pending";
var staticOutcome = "pending";
var Base = class {
    get answer() {
        return this.seed + 2;
    }
    static get answer() {
        return this.seed + 2;
    }
};
var Derived = class extends Base {
    async *instance() {
        await 0;
        yield super.answer;
    }
    static async *method() {
        await 0;
        yield super.answer;
    }
};
Derived.seed = 40;
var instanceIterator = Derived.prototype.instance.call({ seed: 40 });
var staticIterator = Derived.method.call({ seed: 40 });
Derived = null;
Base = null;
instanceIterator.next().then(function (result) {
    instanceOutcome = result.value + ":" + result.done;
});
staticIterator.next().then(function (result) {
    staticOutcome = result.value + ":" + result.done;
});
"#,
    );
    runtime.run_gc().unwrap();
    while runtime.is_job_pending() {
        runtime.execute_pending_job().unwrap();
        runtime.run_gc().unwrap();
    }
    assert_eq!(
        text(eval(&mut context, "instanceOutcome + '|' + staticOutcome")),
        "42:false|42:false"
    );
}

fn compare_success_cases(group: &str, cases: &[SuccessCase]) {
    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!("SKIP {group} differential: set QJS_ORACLE to pinned upstream qjs");
    }

    for case in cases {
        let quickjs = if let Some(oracle) = &oracle {
            let quickjs = run(oracle, case.source);
            assert_success("pinned QuickJS", case, &quickjs);
            Some(quickjs)
        } else {
            None
        };

        let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), case.source);
        assert_success("quickjs-oxide", case, &oxide);

        if let Some(quickjs) = quickjs {
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
