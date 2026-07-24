use std::ffi::OsStr;
use std::process::{Command, Output};

struct SuccessCase {
    description: &'static str,
    source: &'static str,
    expected_stdout: &'static str,
}

const SEMANTIC_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "private async shape source reflection and settlement",
        source: r##"
var events = [];
class C {
    async /*a*/ #sum /*b*/ (left, right = 0) {
        events.push("body");
        return await (left + right);
    }
    expose() { return this.#sum; }
    invoke(left, right) { return this.#sum(left, right); }
    static async /*c*/ #sumStatic /*d*/ (left, right) {
        return await (left + right);
    }
    static expose() { return this.#sumStatic; }
    static invoke(left, right) { return this.#sumStatic(left, right); }
}
var instanceMethod = new C().expose();
var staticMethod = C.expose();
var promise = new C().invoke(19, 23);
var staticPromise = C.invoke(20, 22);
events.push("after");
var constructError = "none";
try {
    new instanceMethod();
} catch (error) {
    constructError = error.name;
}
print("shape=" + [
    typeof instanceMethod,
    instanceMethod.name,
    instanceMethod.length,
    Object.prototype.hasOwnProperty.call(instanceMethod, "prototype"),
    instanceMethod.prototype === undefined,
    Object.prototype.toString.call(instanceMethod),
    Object.getPrototypeOf(instanceMethod).constructor.name,
    Object.getPrototypeOf(instanceMethod) === Object.getPrototypeOf(async function () {}),
    Object.getPrototypeOf(promise) === Promise.prototype,
    staticMethod.name,
    staticMethod.length,
    Object.getOwnPropertyNames(C.prototype).indexOf("#sum") < 0,
    Object.getOwnPropertyNames(C).indexOf("#sumStatic") < 0,
    Function.prototype.toString.call(instanceMethod),
    Function.prototype.toString.call(staticMethod),
    constructError,
    events.join(",")
].join("|"));
promise.then(function (value) {
    print("instance=" + value);
});
staticPromise.then(function (value) {
    print("static=" + value);
});
"##,
        expected_stdout: concat!(
            "shape=function|#sum|1|false|true|[object AsyncFunction]|AsyncFunction|",
            "true|true|#sumStatic|2|true|true|",
            "async /*a*/ #sum /*b*/ (left, right = 0) {\n",
            "        events.push(\"body\");\n",
            "        return await (left + right);\n",
            "    }|async /*c*/ #sumStatic /*d*/ (left, right) {\n",
            "        return await (left + right);\n",
            "    }|TypeError|body,after\n",
            "instance=42\n",
            "static=42\n",
        ),
    },
    SuccessCase {
        description: "private brands and home objects survive await and extraction",
        source: r##"
class Base {
    get answer() { return this.seed + 2; }
    static get answer() { return this.seed + 2; }
}
class C extends Base {
    constructor(seed) {
        super();
        this.seed = seed;
    }
    async #instance(fromParameter = super.answer) {
        var before = super.answer;
        await 0;
        return [fromParameter, before, super.answer, this.seed].join(",");
    }
    exposeInstance() { return this.#instance; }
    invokeInstance() { return this.#instance(); }
    hasInstance(value) { return #instance in value; }
    static async #static(fromParameter = super.answer) {
        var before = super.answer;
        await 0;
        return [fromParameter, before, super.answer, this.seed].join(",");
    }
    static exposeStatic() { return this.#static; }
    static invokeStatic() { return this.#static(); }
    static hasStatic(value) { return #static in value; }
}
C.seed = 40;
var instance = new C(40);
var instanceMethod = instance.exposeInstance();
var staticMethod = C.exposeStatic();
var instanceBrand = "none";
var staticBrand = "none";
try {
    C.prototype.invokeInstance.call({ seed: 100 });
} catch (error) {
    instanceBrand = error.name + ":" + error.message;
}
try {
    C.invokeStatic.call({ seed: 100 });
} catch (error) {
    staticBrand = error.name + ":" + error.message;
}
print("sync=" + [
    instance.hasInstance(instance),
    instance.hasInstance({}),
    C.hasStatic(C),
    C.hasStatic(class D {}),
    instanceBrand,
    staticBrand
].join("|"));
instance.invokeInstance().then(function (value) {
    print("instance=" + value);
});
C.invokeStatic().then(function (value) {
    print("static=" + value);
});
instanceMethod.call({ seed: 100 }).then(function (value) {
    print("borrowed-instance=" + value);
});
staticMethod.call({ seed: 100 }).then(function (value) {
    print("borrowed-static=" + value);
});
"##,
        expected_stdout: concat!(
            "sync=true|false|true|false|TypeError:invalid brand on object|",
            "TypeError:invalid brand on object\n",
            "instance=42,42,42,40\n",
            "static=42,42,42,40\n",
            "borrowed-instance=102,102,102,100\n",
            "borrowed-static=102,102,102,100\n",
        ),
    },
    SuccessCase {
        description: "private access timing and async rejection boundaries",
        source: r##"
var events = [];
var synchronous = [];
class C {
    #value = 42;
    async #parameter(
        value = (
            events.push("parameter"),
            function () { throw new RangeError("parameter"); }
        )()
    ) {
        return value;
    }
    async #body() {
        events.push("body");
        await 0;
        return this.#value;
    }
    callParameter() { return this.#parameter(); }
    callBodyOn(value) { return value.#body(events.push("argument")); }
    exposeBody() { return this.#body; }
    assignBody() { this.#body = 1; }
}
var instance = new C();
var parameterPromise;
try {
    parameterPromise = instance.callParameter();
    events.push("after-parameter");
} catch (error) {
    synchronous.push("parameter=" + error.name);
}
try {
    instance.callBodyOn({});
} catch (error) {
    synchronous.push("access=" + error.name + ":" + error.message);
}
var bodyPromise;
try {
    bodyPromise = instance.exposeBody().call({});
    events.push("after-body");
} catch (error) {
    synchronous.push("body=" + error.name);
}
try {
    instance.assignBody();
} catch (error) {
    synchronous.push("assign=" + error.name + ":" + error.message);
}
print("sync=" + synchronous.join(",") + "|" + events.join(","));
parameterPromise.then(undefined, function (error) {
    print("parameter=" + error.name + ":" + error.message + "|" + events.join(","));
});
bodyPromise.then(undefined, function (error) {
    print("body=" + error.name + ":" + error.message + "|" + events.join(","));
});
"##,
        expected_stdout: concat!(
            "sync=access=TypeError:invalid brand on object,",
            "assign=TypeError:'#body' is read-only|",
            "parameter,after-parameter,body,after-body\n",
            "parameter=RangeError:parameter|parameter,after-parameter,body,after-body\n",
            "body=TypeError:private class field '#value' does not exist|",
            "parameter,after-parameter,body,after-body\n",
        ),
    },
];

#[test]
fn ordinary_private_async_class_method_semantics_match_pinned_quickjs() {
    compare_success_cases("private async class method", SEMANTIC_CASES);
}

#[test]
fn private_async_generators_remain_an_explicit_frontier() {
    let source = r##"
class C {
    async *#method() { yield 42; }
    read() { return this.#method(); }
    static async *#staticMethod() { yield 42; }
    static read() { return this.#staticMethod(); }
}
new C().read().next().then(function (result) {
    C.read().next().then(function (staticResult) {
        print(result.value + staticResult.value);
    });
});
"##;
    let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), source);
    assert!(
        !oxide.status.success(),
        "quickjs-oxide accidentally accepted private async generators"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP private async-generator frontier oracle check: \
             set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };
    let quickjs = run(&oracle, source);
    assert!(
        quickjs.status.success(),
        "pinned QuickJS rejected private async generators: {}",
        String::from_utf8_lossy(&quickjs.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&quickjs.stdout), "84\n");
    assert!(
        String::from_utf8_lossy(&oxide.stderr)
            .contains("async generator class methods are not implemented yet")
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
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", executable.to_string_lossy()))
}
