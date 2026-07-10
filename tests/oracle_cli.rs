use std::ffi::OsStr;
use std::process::{Command, Output};

#[test]
fn primitive_exception_dump_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP CLI dump differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (description, source) in [
        ("quoted string", "throw \"x\""),
        ("escaped string", "throw \"line\\n\\t\\\\\\\"\\0\\x7f\""),
        ("Unicode string", "throw \"é🙂中\""),
        ("short BigInt", "throw 1n"),
        ("heap BigInt", "throw 123456789012345678901234567890n"),
        ("negative zero", "throw -0"),
    ] {
        let rust = run(env!("CARGO_BIN_EXE_qjs").as_ref(), source, description);
        let quickjs = run(&oracle, source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}

fn run(program: &OsStr, source: &str, description: &str) -> Output {
    Command::new(program)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}
