//! Differential and lifetime coverage for pinned QuickJS `for await ... of`.
//!
//! The fixture deliberately authenticates QuickJS 2026-06-04's observable
//! implementation, including its non-standard close behavior: completed async
//! iterators are closed, ordinary abrupt exits do not await `return()`, and
//! next/result failures occur while the iterator unwind record is disabled.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const EXPECTED: &str = include_str!("fixtures/r3ak_for_await_of.quickjs-2026-06-04.txt");

#[test]
fn for_await_of_transcript_matches_pinned_quickjs() {
    let fixture = fixture_path();
    let oxide = run_file(env!("CARGO_BIN_EXE_qjs").as_ref(), &fixture);
    assert_success("quickjs-oxide", &oxide);
    assert_eq!(
        String::from_utf8_lossy(&oxide.stdout),
        EXPECTED,
        "quickjs-oxide for-await transcript drifted"
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP for-await differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let quickjs = run_file(&oracle, &fixture);
    assert_success("pinned QuickJS", &quickjs);
    assert_eq!(
        String::from_utf8_lossy(&quickjs.stdout),
        EXPECTED,
        "pinned QuickJS transcript differs from the frozen fixture"
    );
    assert_eq!(
        oxide.stdout, quickjs.stdout,
        "quickjs-oxide for-await transcript differs from pinned QuickJS"
    );
}

#[test]
fn for_await_contextual_grammar_matches_pinned_quickjs() {
    const VALID: &[&str] = &[
        "async function f(xs){for await(var x of xs){}}",
        "async function* f(xs){for await(var x of xs){yield x;}}",
        "({async f(xs){for await(async of xs){}}})",
        "class C { static async f(xs){for await(const x of xs){}} }",
    ];
    const INVALID: &[&str] = &[
        "for await(var x of xs){}",
        "function f(xs){for await(var x of xs){}}",
        "function* f(xs){for await(var x of xs){}}",
        "async function f(xs){for await(var x in xs){}}",
        "async function f(xs){for aw\\u0061it(var x of xs){}}",
    ];

    let oracle = std::env::var_os("QJS_ORACLE");
    for source in VALID {
        let oxide = run_eval(env!("CARGO_BIN_EXE_qjs").as_ref(), source);
        assert_success("quickjs-oxide", &oxide);
        if let Some(oracle) = &oracle {
            let quickjs = run_eval(oracle, source);
            assert_success("pinned QuickJS", &quickjs);
        }
    }
    for source in INVALID {
        let oxide = run_eval(env!("CARGO_BIN_EXE_qjs").as_ref(), source);
        assert!(
            !oxide.status.success(),
            "quickjs-oxide accepted invalid for-await source: {source}"
        );
        if let Some(oracle) = &oracle {
            let quickjs = run_eval(oracle, source);
            assert!(
                !quickjs.status.success(),
                "pinned QuickJS accepted invalid for-await source: {source}"
            );
        }
    }
}

#[test]
fn pending_for_await_next_record_survives_repeated_gc() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var release;
var outcome = "pending";
var generatorRelease;
var generatorFirst = "pending";
var generatorReturn = "pending";
var gate = new Promise(function (resolve) {
    release = resolve;
});
var generatorGate = new Promise(function (resolve) {
    generatorRelease = resolve;
});
var source = {
    [Symbol.asyncIterator]: function () {
        return {
            token: 42,
            next: function () {
                var self = this;
                return gate.then(function () {
                    return { value: self.token, done: false };
                });
            }
        };
    }
};
(async function () {
    for await (var value of source) {
        outcome = String(value);
        break;
    }
})().then(
    function () { outcome += ":done"; },
    function (error) { outcome = error.name + ":" + error.message; }
);
source = null;
gate = null;

var generatorSource = {
    [Symbol.asyncIterator]: function () {
        return {
            token: 43,
            next: function () {
                var self = this;
                return generatorGate.then(function () {
                    return { value: self.token, done: false };
                });
            },
            return: function () {
                return Promise.resolve({});
            }
        };
    }
};
var generatorIterator = (async function* () {
    for await (var value of generatorSource) {
        yield value;
    }
})();
generatorIterator.next().then(function (result) {
    generatorFirst = result.value + ":" + result.done;
});
generatorIterator.return(8).then(function (result) {
    generatorReturn = result.value + ":" + result.done;
});
generatorSource = null;
generatorIterator = null;
generatorGate = null;
"#,
    );

    for _ in 0..3 {
        runtime.run_gc().unwrap();
    }
    eval(&mut context, "release(); generatorRelease();");
    while runtime.is_job_pending() {
        assert!(runtime.execute_pending_job().unwrap());
        runtime.run_gc().unwrap();
    }
    assert_eq!(
        text(eval(
            &mut context,
            "outcome + '|' + generatorFirst + '|' + generatorReturn"
        )),
        "42:done|43:false|8:true"
    );
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/r3ak_for_await_of.js")
}

fn run_file(executable: &OsStr, fixture: &PathBuf) -> Output {
    Command::new(executable)
        .arg(fixture)
        .output()
        .unwrap_or_else(|error| panic!("could not run {executable:?}: {error}"))
}

fn run_eval(executable: &OsStr, source: &str) -> Output {
    Command::new(executable)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run {executable:?}: {error}"))
}

fn assert_success(engine: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{engine} rejected for-await probe: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
