//! Differential coverage for private class async-generator methods.
//!
//! Delegation, `for await`, and `.return()` while a nested iterator is active
//! remain separate fail-closed milestones. Destructuring parameters remain
//! outside this milestone's authenticated scope.

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
        description: "instance and static shapes names source prototypes and extraction",
        source: r##"
class C {
    async /*i0*/ * /*i1*/ #instance /*i2*/ (left, right = 2) {
        yield await (left + right);
    }
    exposeInstance() { return this.#instance; }
    invokeInstance(left, right) { return this.#instance(left, right); }
    hasInstance(value) { return #instance in value; }

    static async /*s0*/ * /*s1*/ #staticMethod /*s2*/ (value) {
        yield await value;
    }
    static exposeStatic() { return this.#staticMethod; }
    static invokeStatic(value) { return this.#staticMethod(value); }
    static hasStatic(value) { return #staticMethod in value; }
}
var instance = new C();
var instanceMethod = instance.exposeInstance();
var staticMethod = C.exposeStatic();
var instanceIterator = instanceMethod.call({ ignored: true }, 40);
var staticIterator = staticMethod.call({ ignored: true }, 42);
var instancePrototypeDescriptor =
    Object.getOwnPropertyDescriptor(instanceMethod, "prototype");
var staticPrototypeDescriptor =
    Object.getOwnPropertyDescriptor(staticMethod, "prototype");
var constructError = "none";
var newTargetError = "none";
try {
    new instanceMethod();
} catch (error) {
    constructError = error.name;
}
try {
    Reflect.construct(function () {}, [], staticMethod);
} catch (error) {
    newTargetError = error.name;
}
print([
    typeof instanceMethod,
    instanceMethod.name,
    instanceMethod.length,
    staticMethod.name,
    staticMethod.length,
    Object.prototype.toString.call(instanceMethod),
    Object.getPrototypeOf(instanceMethod).constructor.name,
    Object.getPrototypeOf(instanceMethod) === Object.getPrototypeOf(async function* () {}),
    Object.prototype.hasOwnProperty.call(instanceMethod, "prototype"),
    Object.prototype.hasOwnProperty.call(staticMethod, "prototype"),
    Object.getPrototypeOf(instanceIterator) === instanceMethod.prototype,
    Object.getPrototypeOf(staticIterator) === staticMethod.prototype,
    Object.prototype.toString.call(instanceIterator),
    instancePrototypeDescriptor.writable,
    instancePrototypeDescriptor.enumerable,
    instancePrototypeDescriptor.configurable,
    staticPrototypeDescriptor.writable,
    staticPrototypeDescriptor.enumerable,
    staticPrototypeDescriptor.configurable,
    Object.getOwnPropertyNames(C.prototype).indexOf("#instance") < 0,
    Object.getOwnPropertyNames(C).indexOf("#staticMethod") < 0,
    instance.hasInstance(instance),
    instance.hasInstance({}),
    C.hasStatic(C),
    C.hasStatic(class D {}),
    Function.prototype.toString.call(instanceMethod),
    Function.prototype.toString.call(staticMethod),
    constructError,
    newTargetError
].join("|"));
instanceIterator.next().then(function (result) {
    print("instance=" + result.value + ":" + result.done);
});
staticIterator.next().then(function (result) {
    print("static=" + result.value + ":" + result.done);
});
"##,
        expected_stdout: concat!(
            "function|#instance|1|#staticMethod|1|[object AsyncGeneratorFunction]|",
            "AsyncGeneratorFunction|true|true|true|true|true|",
            "[object AsyncGenerator]|true|false|false|true|false|false|",
            "true|true|true|false|true|false|",
            "async /*i0*/ * /*i1*/ #instance /*i2*/ (left, right = 2) {\n",
            "        yield await (left + right);\n",
            "    }|async /*s0*/ * /*s1*/ #staticMethod /*s2*/ (value) {\n",
            "        yield await value;\n",
            "    }|TypeError|TypeError\n",
            "instance=42:false\n",
            "static=42:false\n",
        ),
    },
    SuccessCase {
        description: "private brands and borrowed receivers cross await boundaries",
        source: r##"
var events = [];
class C {
    #value;
    constructor(value) {
        this.#value = value;
    }
    async *#read(delta = (events.push("instance-parameter"), 2)) {
        events.push("instance-body");
        await 0;
        yield this.#value + delta;
    }
    exposeInstance() { return this.#read; }
    invokeInstance() { return this.#read(); }
    readFrom(value) { return value.#read(); }
    hasInstance(value) { return #read in value; }

    static #staticValue = 40;
    static async *#staticRead(delta = (events.push("static-parameter"), 2)) {
        events.push("static-body");
        await 0;
        yield this.#staticValue + delta;
    }
    static exposeStatic() { return this.#staticRead; }
    static invokeStatic() { return this.#staticRead(); }
    static readFrom(value) { return value.#staticRead(); }
    static hasStatic(value) { return #staticRead in value; }
}
var instance = new C(40);
var other = new C(100);
var instanceMethod = instance.exposeInstance();
var staticMethod = C.exposeStatic();
var accessErrors = [];
try {
    C.prototype.exposeInstance.call({});
} catch (error) {
    accessErrors.push("instance=" + error.name);
}
try {
    C.exposeStatic.call(class D {});
} catch (error) {
    accessErrors.push("static=" + error.name);
}
try {
    instance.readFrom({});
} catch (error) {
    accessErrors.push("target-instance=" + error.name);
}
try {
    C.readFrom(class D {});
} catch (error) {
    accessErrors.push("target-static=" + error.name);
}
var ownIterator = instanceMethod.call(instance);
var otherIterator = instanceMethod.call(other);
var wrongInstanceIterator = instanceMethod.call({});
var ownStaticIterator = staticMethod.call(C);
var wrongStaticIterator = staticMethod.call(class D {});
print("sync=" + [
    accessErrors.join(","),
    instance.hasInstance(instance),
    instance.hasInstance({}),
    C.hasStatic(C),
    C.hasStatic(class D {}),
    events.join(",")
].join("|"));
ownIterator.next().then(function (result) {
    print("own=" + result.value + ":" + result.done);
});
otherIterator.next().then(function (result) {
    print("other=" + result.value + ":" + result.done);
});
wrongInstanceIterator.next().then(undefined, function (error) {
    print("wrong-instance=" + error.name);
});
ownStaticIterator.next().then(function (result) {
    print("own-static=" + result.value + ":" + result.done);
});
wrongStaticIterator.next().then(undefined, function (error) {
    print("wrong-static=" + error.name);
});
"##,
        expected_stdout: concat!(
            "sync=instance=TypeError,static=TypeError,target-instance=TypeError,",
            "target-static=TypeError|true|false|true|false|",
            "instance-parameter,instance-parameter,instance-parameter,",
            "static-parameter,static-parameter\n",
            "wrong-instance=TypeError\n",
            "wrong-static=TypeError\n",
            "own=42:false\n",
            "other=102:false\n",
            "own-static=42:false\n",
        ),
    },
    SuccessCase {
        description: "parameters are synchronous and next requests settle in FIFO order",
        source: r##"
var events = [];
class C {
    async *#sequence(
        value = (events.push("parameter"), 40)
    ) {
        events.push("body");
        await 0;
        events.push("after-await");
        var sent = yield value + 2;
        events.push("after-yield");
        return sent + 1;
    }
    async *#fail(
        value = (
            events.push("throw-parameter"),
            function () { throw new RangeError("parameter"); }
        )()
    ) {
        events.push("miss-body");
        yield value;
    }
    sequence() { return this.#sequence(); }
    fail() { return this.#fail(); }
}
var value = new C();
var iterator = value.sequence();
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
var firstPromise = iterator.next();
events.push("after-first-next");
var secondPromise = iterator.next(41);
events.push("after-second-next");
print("next=" + [
    Object.getPrototypeOf(firstPromise) === Promise.prototype,
    Object.getPrototypeOf(secondPromise) === Promise.prototype,
    events.join(",")
].join("|"));
firstPromise.then(function (result) {
    print("first=" + result.value + ":" + result.done + "|" + events.join(","));
});
secondPromise.then(function (result) {
    print("second=" + result.value + ":" + result.done + "|" + events.join(","));
});
"##,
        expected_stdout: concat!(
            "call=RangeError:parameter|parameter,after-call,throw-parameter,caught\n",
            "next=true|true|parameter,after-call,throw-parameter,caught,body,",
            "after-first-next,after-second-next\n",
            "first=42:false|parameter,after-call,throw-parameter,caught,body,",
            "after-first-next,after-second-next,after-await,after-yield\n",
            "second=42:true|parameter,after-call,throw-parameter,caught,body,",
            "after-first-next,after-second-next,after-await,after-yield\n",
        ),
    },
];

#[test]
fn pinned_quickjs_expectations_are_authenticated() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP private async-generator class-method expectation authentication: \
             set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };

    for case in SEMANTIC_CASES {
        let quickjs = run(&oracle, case.source);
        assert_success("pinned QuickJS", case, &quickjs);
    }
}

#[test]
fn private_async_generator_class_method_semantics_match_pinned_quickjs() {
    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!(
            "SKIP private async-generator class-method differential: \
             set QJS_ORACLE to pinned upstream qjs"
        );
    }

    for case in SEMANTIC_CASES {
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
                "private async-generator class method output differed for {}",
                case.description
            );
        }
    }
}

#[test]
fn suspended_private_class_methods_retain_home_objects_and_brands_across_gc() {
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
    #offset = 2;
    async *#instance() {
        await 0;
        yield super.answer + this.#offset;
    }
    exposeInstance() {
        return this.#instance;
    }
    static #staticOffset = 2;
    static async *#method() {
        await 0;
        yield super.answer + this.#staticOffset;
    }
    static exposeMethod() {
        return this.#method;
    }
};
var instance = new Derived();
instance.seed = 38;
Derived.seed = 38;
var instanceMethod = instance.exposeInstance();
var staticMethod = Derived.exposeMethod();
var instanceIterator = instanceMethod.call(instance);
var staticIterator = staticMethod.call(Derived);
instanceMethod = null;
staticMethod = null;
instance = null;
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
