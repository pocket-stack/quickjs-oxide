use std::ffi::OsStr;
use std::process::{Command, Output};

struct SuccessCase {
    description: &'static str,
    source: &'static str,
    expected_stdout: &'static str,
}

const SEMANTIC_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "instance and static shape source descriptors and settlement",
        source: r#"
var events = [];
var answer = "pending";
class C {
    async /*a*/ add /*b*/ (left, right) /*c*/ {
        events.push("body");
        return await (left + right);
    }
    static async /*d*/ ["sum"] /*e*/ (left, right) {
        return await (left + right);
    }
}
var method = C.prototype.add;
var staticMethod = C.sum;
var promise = new C().add(19, 23);
events.push("after");
promise.then(function (value) {
    answer = value;
    events.push("then");
    print("settled=" + answer + "|" + events.join(","));
});
var constructError = "none";
var newTargetError = "none";
try {
    new method(1, 2);
} catch (error) {
    constructError = error.name;
}
try {
    Reflect.construct(function () {}, [], staticMethod);
} catch (error) {
    newTargetError = error.name;
}
var descriptor = Object.getOwnPropertyDescriptor(C.prototype, "add");
var staticDescriptor = Object.getOwnPropertyDescriptor(C, "sum");
print("shape=" + [
    typeof method,
    method.name,
    method.length,
    Object.prototype.hasOwnProperty.call(method, "prototype"),
    method.prototype === undefined,
    Object.prototype.toString.call(method),
    Object.getPrototypeOf(method).constructor.name,
    Object.getPrototypeOf(method) === Object.getPrototypeOf(async function () {}),
    Object.getPrototypeOf(promise) === Promise.prototype,
    descriptor.enumerable,
    descriptor.configurable,
    descriptor.writable,
    staticMethod.name,
    staticMethod.length,
    staticDescriptor.enumerable,
    staticDescriptor.configurable,
    staticDescriptor.writable,
    Function.prototype.toString.call(method),
    Function.prototype.toString.call(staticMethod),
    constructError,
    newTargetError,
    events.join(","),
    answer
].join("|"));
"#,
        expected_stdout: concat!(
            "shape=function|add|2|false|true|[object AsyncFunction]|AsyncFunction|",
            "true|true|false|true|true|sum|2|false|true|true|",
            "async /*a*/ add /*b*/ (left, right) /*c*/ {\n",
            "        events.push(\"body\");\n",
            "        return await (left + right);\n",
            "    }|async /*d*/ [\"sum\"] /*e*/ (left, right) {\n",
            "        return await (left + right);\n",
            "    }|TypeError|TypeError|body,after|pending\n",
            "settled=42|body,after,then\n",
        ),
    },
    SuccessCase {
        description: "parameter abrupt completion becomes a rejected promise",
        source: r#"
var events = [];
var outcome = "pending";
var synchronous = "none";
class C {
    async fail(
        value = (
            events.push("parameter"),
            function () { throw new RangeError("parameter"); }
        )()
    ) {
        events.push("body");
        return value;
    }
}
var promise;
try {
    promise = new C().fail();
    events.push("after-call");
} catch (error) {
    synchronous = error.name + ":" + error.message;
}
promise.then(undefined, function (error) {
    outcome = error.name + ":" + error.message;
    events.push("rejected");
    print("settled=" + outcome + "|" + events.join(","));
});
print("sync=" + [
    synchronous,
    Object.getPrototypeOf(promise) === Promise.prototype,
    outcome,
    events.join(",")
].join("|"));
"#,
        expected_stdout: concat!(
            "sync=none|true|pending|parameter,after-call\n",
            "settled=RangeError:parameter|parameter,after-call,rejected\n",
        ),
    },
    SuccessCase {
        description: "instance and static home objects survive parameters await and borrowing",
        source: r#"
class Base {
    get answer() { return this.seed + 2; }
    add(delta) { return this.seed + delta; }
    static get answer() { return this.seed + 2; }
    static add(delta) { return this.seed + delta; }
}
class C extends Base {
    constructor(seed) {
        super();
        this.seed = seed;
    }
    async read(fromParameter = super.answer) {
        var before = super.answer;
        await 0;
        return [
            fromParameter,
            before,
            super.answer,
            super.add(2),
            this.seed
        ].join(",");
    }
    static async read(fromParameter = super.answer) {
        var before = super.answer;
        await 0;
        return [
            fromParameter,
            before,
            super.answer,
            super.add(2),
            this.seed
        ].join(",");
    }
}
C.seed = 40;
var instance = new C(40);
instance.read().then(function (value) {
    print("instance=" + value);
});
C.read().then(function (value) {
    print("static=" + value);
});
C.prototype.read.call({ seed: 100 }).then(function (value) {
    print("borrowed-instance=" + value);
});
C.read.call({ seed: 100 }).then(function (value) {
    print("borrowed-static=" + value);
});
print("sync=instance|static|borrowed-instance|borrowed-static");
"#,
        expected_stdout: concat!(
            "sync=instance|static|borrowed-instance|borrowed-static\n",
            "instance=42,42,42,42,40\n",
            "static=42,42,42,42,40\n",
            "borrowed-instance=102,102,102,102,100\n",
            "borrowed-static=102,102,102,102,100\n",
        ),
    },
];

const CONTEXTUAL_VALID_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "instance and static identifiers start async methods",
        source: r#"
class C {
    async instance() { return await 20; }
    static async method() { return await 22; }
}
new C().instance().then(function (left) {
    C.method().then(function (right) {
        print(left + right);
    });
});
"#,
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "QuickJS ignores Unicode separators inside prefix block comments",
        source: "class C { async /*\u{2028}*/ instance() { return await 20; } static async /*\u{2029}*/ method() { return await 22; } }\nnew C().instance().then(function (left) { C.method().then(function (right) { print(left + right); }); });",
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "computed and escaped property names retain async execution",
        source: r#"
var key = "left";
class C {
    async [key]() { return await 20; }
    static async r\u0069ght() { return await 22; }
}
new C().left().then(function (left) {
    C.right().then(function (right) {
        print(left + right);
    });
});
"#,
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "async immediately before parens remains a synchronous method name",
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
    SuccessCase {
        description: "line terminators split async fields from following synchronous methods",
        source: r#"
class C {
    async
    instance() { return 20; }
    static async
    method() { return 22; }
}
var value = new C();
print(String(value.async) + "|" + String(C.async) + "|" + value.instance() + C.prototype.method());
"#,
        expected_stdout: "undefined|undefined|2022\n",
    },
    SuccessCase {
        description: "get await and static constructor are ordinary async method names",
        source: r#"
class C {
    async get() { return await 10; }
    async await() { return await 11; }
    static async constructor() { return await 21; }
}
var value = new C();
value.get().then(function (first) {
    value.await().then(function (second) {
        C.constructor().then(function (third) {
            print(first + second + third);
        });
    });
});
"#,
        expected_stdout: "42\n",
    },
];

const CONTEXTUAL_INVALID_CASES: &[(&str, &str)] = &[
    (
        "an instance async constructor is not the class constructor",
        "class C { async constructor() {} }",
    ),
    (
        "a static async prototype method is forbidden",
        "class C { static async prototype() {} }",
    ),
    (
        "async cannot prefix an accessor declaration",
        "class C { async get value() {} }",
    ),
    (
        "await is forbidden as an async method parameter binding",
        "class C { async method(await) {} }",
    ),
    (
        "await expressions are forbidden in async method parameters",
        "class C { async method(value = await 1) {} }",
    ),
    (
        "escaped async cannot act as the contextual prefix",
        r"class C { as\u0079nc method() {} }",
    ),
    (
        "strict class methods reject duplicate parameters",
        "class C { async method(value, value) {} }",
    ),
    (
        "QuickJS does not fall back to an async-named field before semicolon",
        "class C { async; }",
    ),
    (
        "super calls are forbidden in async class methods",
        "class C extends Object { async method() { super(); } }",
    ),
];

#[test]
fn ordinary_async_class_method_semantics_match_pinned_quickjs() {
    compare_success_cases("ordinary async class method", SEMANTIC_CASES);
}

#[test]
fn async_class_method_contextual_boundaries_match_pinned_quickjs() {
    compare_success_cases("async class method token", CONTEXTUAL_VALID_CASES);

    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!(
            "SKIP async class method rejection differential: \
             set QJS_ORACLE to pinned upstream qjs"
        );
    }
    for (description, source) in CONTEXTUAL_INVALID_CASES {
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
fn async_generator_class_methods_remain_an_explicit_frontier() {
    let source = r#"
class C {
    async *method() { yield 42; }
}
new C().method().next().then(function (result) {
    print(result.value);
});
"#;

    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!(
            "SKIP async-generator class frontier oracle check: \
             set QJS_ORACLE to pinned upstream qjs"
        );
    }
    let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), source);
    assert!(
        !oxide.status.success(),
        "quickjs-oxide accidentally accepted the async-generator frontier"
    );

    if let Some(oracle) = &oracle {
        let quickjs = run(oracle, source);
        assert!(
            quickjs.status.success(),
            "pinned QuickJS rejected the recorded async-generator frontier: {}",
            String::from_utf8_lossy(&quickjs.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&quickjs.stdout), "42\n");
    }
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
