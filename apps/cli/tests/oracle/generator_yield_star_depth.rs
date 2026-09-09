//! Differential coverage for deep synchronous `yield*` delegation.
//!
//! This keeps the stack-budget repair tied to observable QuickJS behavior:
//! delegated result identity plus `next`, `return`, and `throw` propagation.

use std::ffi::OsStr;
use std::process::{Command, Output};

const MATRIX_SOURCE: &str = r#"
function* chain(depth, delegate) {
    return yield* (depth ? chain(depth - 1, delegate) : delegate);
}

var output = [];
var yielded = { value: 1 };
var completed = { value: 34, done: true };
var index = 0;
var delegate = {
    next: function () { return index++ === 0 ? yielded : completed; },
    [Symbol.iterator]: function () { return this; }
};
var nextIterator = chain(10, delegate);
var nextYielded = nextIterator.next();
var nextCompleted = nextIterator.next();
output.push(
    nextYielded === yielded,
    Object.hasOwn(nextYielded, "done"),
    nextCompleted.value,
    nextCompleted.done
);

var finallyTrace = "";
function* returnLeaf() {
    try {
        yield "seed";
    } finally {
        finallyTrace += "F";
    }
}
var returnIterator = chain(10, returnLeaf());
var returnYielded = returnIterator.next();
var returnCompleted = returnIterator.return(42);
output.push(
    returnYielded.value,
    returnYielded.done,
    finallyTrace,
    returnCompleted.value,
    returnCompleted.done
);

var marker = {};
function* throwLeaf() {
    try {
        yield "seed";
    } catch (error) {
        return error === marker ? 42 : -1;
    }
}
var throwIterator = chain(10, throwLeaf());
throwIterator.next();
var throwCompleted = throwIterator.throw(marker);
output.push(throwCompleted.value, throwCompleted.done);

print(output.join("|"));
"#;

const EXPECTED_STDOUT: &str = "true|false|34|true|seed|false|F|42|true|42|true\n";

#[test]
fn deep_yield_star_protocol_matches_pinned_quickjs() {
    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!("SKIP deep yield-star differential: set QJS_ORACLE to pinned upstream qjs");
    }

    for (description, source) in [
        ("sloppy script", MATRIX_SOURCE.to_owned()),
        ("strict script", format!("\"use strict\";\n{MATRIX_SOURCE}")),
    ] {
        let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), &source);
        assert_success("quickjs-oxide", description, &source, &oxide);

        if let Some(oracle) = &oracle {
            let quickjs = run(oracle, &source);
            assert_success("pinned QuickJS", description, &source, &quickjs);
            assert_eq!(
                oxide.stdout, quickjs.stdout,
                "deep yield-star output differed for {description}"
            );
        }
    }
}

fn assert_success(engine: &str, description: &str, source: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{engine} rejected the {description} deep yield-star matrix: {}\nsource:\n{source}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        EXPECTED_STDOUT,
        "{engine} output drifted for the {description} deep yield-star matrix"
    );
}

fn run(executable: &OsStr, source: &str) -> Output {
    Command::new(executable)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run {executable:?}: {error}"))
}
