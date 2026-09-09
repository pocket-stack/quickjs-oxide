//! Differential coverage for the contextual `await` boundary of synthetic
//! public/private, instance/static class-field initializer functions.

use std::ffi::OsStr;
use std::process::{Command, Output};

const MATRIX_SOURCE: &str = r#"
var aw\u0061it = 40;
async function build() {
    class Fields {
        instance = await + 1;
        static staticField = await + 2;
        #private = await + 3;
        static #staticPrivate = await + 4;
        arrow = () => await + 5;
        static escaped = aw\u0061it + 6;
        [await Promise.resolve("computed")] = 47;
        read() { return this.#private; }
        static read() { return this.#staticPrivate; }
    }
    return Fields;
}
build().then(function (Fields) {
    var instance = new Fields();
    print([
        instance.instance,
        Fields.staticField,
        instance.read(),
        Fields.read(),
        instance.arrow(),
        Fields.escaped,
        instance.computed
    ].join(","));
});
"#;

const EXPECTED_STDOUT: &str = "41,42,43,44,45,46,47\n";

#[test]
fn class_field_await_runtime_matrix_matches_pinned_quickjs() {
    let oracle = std::env::var_os("QJS_ORACLE");
    if oracle.is_none() {
        eprintln!("SKIP class-field await differential: set QJS_ORACLE to pinned upstream qjs");
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
                "class-field await output differed for {description}"
            );
        }
    }
}

fn assert_success(engine: &str, description: &str, source: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{engine} rejected the {description} class-field await matrix: {}\nsource:\n{source}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        EXPECTED_STDOUT,
        "{engine} output drifted for the {description} class-field await matrix"
    );
}

fn run(executable: &OsStr, source: &str) -> Output {
    Command::new(executable)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run {executable:?}: {error}"))
}
