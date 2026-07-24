use std::ffi::OsStr;
use std::process::{Command, Output};

struct SuccessCase {
    description: &'static str,
    source: &'static str,
    expected_stdout: &'static str,
}

const SEMANTIC_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "shape source await and promise settlement",
        source: r#"
var events = [];
var answer = "pending";
var object = { async /*a*/ add /*b*/ (left, right) /*c*/ { events.push("body"); return await (left + right); } };
var method = object.add;
var promise = method(19, 23);
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
    Reflect.construct(function () {}, [], method);
} catch (error) {
    newTargetError = error.name;
}
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
    Function.prototype.toString.call(method),
    constructError,
    newTargetError,
    events.join(","),
    answer
].join("|"));
"#,
        expected_stdout: concat!(
            "shape=function|add|2|false|true|[object AsyncFunction]|AsyncFunction|",
            "true|true|async /*a*/ add /*b*/ (left, right) /*c*/ { ",
            "events.push(\"body\"); return await (left + right); }|",
            "TypeError|TypeError|body,after|pending\n",
            "settled=42|body,after,then\n",
        ),
    },
    SuccessCase {
        description: "parameter abrupt completion becomes a rejected promise",
        source: r#"
var events = [];
var outcome = "pending";
var synchronous = "none";
var object = {
    async fail(
        value = (
            events.push("parameter"),
            function () { throw new RangeError("parameter"); }
        )()
    ) {
        events.push("body");
        return value;
    }
};
var promise;
try {
    promise = object.fail();
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
        description: "home object super properties survive parameters and await",
        source: r#"
var base = {
    get answer() {
        return this.seed + 2;
    },
    add(delta) {
        return this.seed + delta;
    }
};
var object = {
    __proto__: base,
    seed: 40,
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
};
var own = "pending";
var borrowed = "pending";
object.read().then(function (value) {
    own = value;
    print("own=" + own);
});
object.read.call({ seed: 100 }).then(function (value) {
    borrowed = value;
    print("borrowed=" + borrowed);
});
print("sync=" + own + "|" + borrowed);
"#,
        expected_stdout: concat!(
            "sync=pending|pending\n",
            "own=42,42,42,42,40\n",
            "borrowed=102,102,102,102,100\n",
        ),
    },
];

const CONTEXTUAL_VALID_CASES: &[SuccessCase] = &[
    SuccessCase {
        description: "async before an identifier starts an async method",
        source: r#"
var object = { async method() { return await 42; } };
object.method().then(function (value) {
    print(value + "|" + Object.prototype.toString.call(object.method));
});
"#,
        expected_stdout: "42|[object AsyncFunction]\n",
    },
    SuccessCase {
        description: "a comment without a line terminator preserves the async prefix",
        source: r#"
var object = { async /* no LineTerminator */ method() { return await 42; } };
object.method().then(print);
"#,
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "QuickJS ignores U+2028 inside an async-prefix block comment",
        source: "var object = { async /*\u{2028}*/ method() { return await 42; } };\nobject.method().then(print);",
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "QuickJS ignores U+2029 inside an async-prefix block comment",
        source: "var object = { async /*\u{2029}*/ method() { return await 42; } };\nobject.method().then(print);",
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "async accepts a computed property name",
        source: r#"
var key = "method";
var object = { async [key]() { return await 42; } };
object.method().then(print);
"#,
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "the property name after async may contain an escape",
        source: r#"
var object = { async m\u0065thod() { return await 42; } };
object.method().then(print);
"#,
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "async immediately before parens is an ordinary method name",
        source: r#"
var object = { async() { return 42; } };
print(object.async() + "|" + Object.prototype.toString.call(object.async));
"#,
        expected_stdout: "42|[object Function]\n",
    },
    SuccessCase {
        description: "a line terminator before parens still leaves an ordinary async-named method",
        source: r#"
var object = { async
() { return 42; } };
print(object.async() + "|" + Object.prototype.toString.call(object.async));
"#,
        expected_stdout: "42|[object Function]\n",
    },
    SuccessCase {
        description: "escaped async remains an ordinary method name before parens",
        source: r#"
var object = { as\u0079nc() { return 42; } };
print(object.async() + "|" + Object.prototype.toString.call(object.async));
"#,
        expected_stdout: "42|[object Function]\n",
    },
    SuccessCase {
        description: "a comma separates an async shorthand from the following method",
        source: r#"
var async = 42;
var object = { async,
    method() { return this.async; }
};
print(object.method());
"#,
        expected_stdout: "42\n",
    },
    SuccessCase {
        description: "get and await are valid async method property names",
        source: r#"
var object = {
    async get() { return await 20; },
    async await() { return await 22; }
};
object.get().then(function (left) {
    object.await().then(function (right) {
        print(left + right);
    });
});
"#,
        expected_stdout: "42\n",
    },
];

const CONTEXTUAL_INVALID_CASES: &[(&str, &str)] = &[
    (
        "a line terminator prevents async from prefixing a following identifier",
        "var object = { async\nmethod() {} };",
    ),
    (
        "a multiline comment line terminator prevents the async prefix",
        "var object = { async /*\n*/ method() {} };",
    ),
    (
        "a direct U+2028 line terminator prevents the async prefix",
        "var object = { async\u{2028}method() {} };",
    ),
    (
        "a direct U+2029 line terminator prevents the async prefix",
        "var object = { async\u{2029}method() {} };",
    ),
    (
        "escaped async cannot act as the contextual prefix",
        r"var object = { as\u0079nc method() {} };",
    ),
    (
        "async cannot prefix an accessor declaration",
        "var object = { async get value() {} };",
    ),
    (
        "await is forbidden as an async method parameter binding",
        "var object = { async method(await) {} };",
    ),
    (
        "await expressions are forbidden in async method parameters",
        "var object = { async method(value = await 1) {} };",
    ),
    (
        "a line terminator prevents async from prefixing a generator star",
        "var object = { async\n*method() {} };",
    ),
];

#[test]
fn ordinary_async_object_method_semantics_match_pinned_quickjs() {
    compare_success_cases("ordinary async object method", SEMANTIC_CASES);
}

#[test]
fn async_object_method_contextual_boundaries_match_pinned_quickjs() {
    compare_success_cases("async object method token", CONTEXTUAL_VALID_CASES);

    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!(
            "SKIP async object method rejection differential: \
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
fn async_generator_object_methods_remain_an_explicit_frontier() {
    const SOURCE: &str = r#"
var object = {
    async *method() {
        yield 42;
    }
};
object.method().next().then(function (result) {
    print(result.value);
});
"#;

    let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), SOURCE);
    assert!(
        !oxide.status.success(),
        "quickjs-oxide accidentally accepted the async-generator frontier"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP async-generator frontier oracle check: \
             set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };
    let quickjs = run(&oracle, SOURCE);
    assert!(
        quickjs.status.success(),
        "pinned QuickJS rejected the recorded async-generator frontier: {}",
        String::from_utf8_lossy(&quickjs.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&quickjs.stdout), "42\n");
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
