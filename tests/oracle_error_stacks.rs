use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, RuntimeError, Value};

#[test]
fn implemented_error_backtraces_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Error stack oracle differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let cases = [
        (
            "nested VM fault and tail-call sites",
            "(function outer(){ return (function inner(){ return 1n + 1; })(); })()",
        ),
        (
            "eager Error constructor stack",
            "throw (function outer(){ return (function inner(){ return new Error(\"boom\"); })(); })()",
        ),
        ("parse location", "1 +"),
        (
            "strict missing assignment inherits the LHS site",
            "\"use strict\"; missing = 1",
        ),
        (
            "assignment inherits an RHS identifier marker",
            "\"use strict\"; missing = Error",
        ),
        (
            "assignment inherits an RHS operator marker",
            "\"use strict\"; missing = 1 + 2",
        ),
        (
            "assignment inherits a private self-binding marker",
            "(function f(){ \"use strict\"; missing = f; })()",
        ),
        ("fixed member null fault", "null.x"),
        ("computed member undefined fault", "(void 0)['x']"),
        (
            "non-tail method call site",
            "(function(){ var value = Function.name(); return value; })()",
        ),
        (
            "tail method call site",
            "(function(){ return Function.name(); })()",
        ),
        ("fixed member assignment fault", "null.x = 1"),
        ("computed member assignment fault", "null['x'] = 1"),
        ("fixed member delete fault", "delete null.x"),
        ("computed member delete fault", "delete (void 0)['x']"),
        (
            "strict fixed member assignment rejection",
            "\"use strict\";\nFunction.prototype = 1",
        ),
        (
            "strict fixed member delete rejection",
            "\"use strict\";\ndelete Function.prototype",
        ),
        (
            "native member setter fault",
            "Function.prototype.caller = 1",
        ),
        (
            "strict fixed compound assignment rejection",
            "\"use strict\";\nFunction.prototype += 1",
        ),
        (
            "strict computed compound assignment rejection",
            "\"use strict\";\nFunction['prototype'] += 1",
        ),
        ("compound nullish pre-key fault", "null[true] += 1"),
        (
            "compound getter fault keeps member site",
            "Function.prototype.caller += 1",
        ),
        (
            "CR remains a debug column",
            "(function f(){\rreturn 1n + 1;\r})()",
        ),
        (
            "CRLF advances only at LF",
            "(function f(){\r\nreturn 1n + 1;\r\n})()",
        ),
        (
            "line separator remains a debug column",
            "(function f(){\u{2028}return 1n + 1;\u{2028}})()",
        ),
        (
            "UTF-8 columns count lead bytes",
            "(function f(){ \"é🙂中\"; return 1n + 1; })()",
        ),
    ];

    for (description, source) in cases {
        assert_eq!(
            rust_stack(source),
            oracle_stack(&oracle, source, description),
            "QuickJS Error stack drifted for {description}: {source:?}",
        );
    }
}

fn rust_stack(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval_with_filename(source, "<cmdline>"),
        Err(RuntimeError::Exception),
        "Rust source unexpectedly completed: {source:?}",
    );
    let Value::Object(error) = context
        .take_exception()
        .expect("take Rust exception")
        .expect("Rust exception is present")
    else {
        panic!("Rust exception was not an object for {source:?}");
    };
    assert!(
        runtime
            .is_error_object(&error)
            .expect("classify Rust Error")
    );
    let stack = runtime.intern_property_key("stack").expect("stack key");
    let Value::String(stack) = context
        .get_property(&error, &stack)
        .expect("read Rust Error.stack")
    else {
        panic!("Rust Error.stack was not a string for {source:?}");
    };
    stack.to_utf8_lossy()
}

fn oracle_stack(oracle: &OsStr, source: &str, description: &str) -> String {
    let output = Command::new(oracle)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        !output.status.success(),
        "QuickJS unexpectedly completed {description}: {source:?}",
    );
    let stderr = String::from_utf8(output.stderr)
        .unwrap_or_else(|error| panic!("QuickJS emitted non-UTF-8 stderr: {error}"));
    let start = stderr
        .find("    at ")
        .unwrap_or_else(|| panic!("QuickJS emitted no Error stack for {description}: {stderr:?}"));
    stderr[start..].to_owned()
}
