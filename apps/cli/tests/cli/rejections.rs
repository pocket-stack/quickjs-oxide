use super::*;

fn run_file(options: &[&str], path: &Path) -> Output {
    qjs()
        .args(options)
        .arg(cli_path(path))
        .output()
        .expect("run qjs file")
}

/// Host promise-rejection tracking (quickjs-libc.c
/// `js_std_promise_rejection_tracker` + `js_std_promise_rejection_check`):
/// rejections still without a handler once the job queue drains are printed to
/// stderr with the `Possibly unhandled promise rejection: ` prefix and the
/// process exits 1; attaching a handler before the drain removes the entry.
struct RejectionCase {
    description: &'static str,
    options: &'static [&'static str],
    source: &'static str,
    expected_status: i32,
    expected_stderr: &'static [u8],
}

const REJECTION_CASES: &[RejectionCase] = &[
    RejectionCase {
        description: "an Error rejection reports its attached stack frame",
        options: &[],
        source: "Promise.reject(new Error(\"reject-boom\"));",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: Error: reject-boom\n    at <eval> (<cmdline>:1:25)\n",
    },
    RejectionCase {
        description: "an async function rejection reports its awaited frames",
        options: &[],
        source: "async function af(){ throw new Error(\"async-boom\"); } af();",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: Error: async-boom\n    at af (<cmdline>:1:37)\n    at <eval> (<cmdline>:1:57)\n",
    },
    RejectionCase {
        description: "a string reason is quoted like JS_PrintValue",
        options: &[],
        source: "Promise.reject(\"s\");",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: \"s\"\n",
    },
    RejectionCase {
        description: "an undefined reason is printed verbatim",
        options: &[],
        source: "Promise.reject(undefined);",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: undefined\n",
    },
    RejectionCase {
        description: "a number reason is printed verbatim",
        options: &[],
        source: "Promise.reject(42);",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: 42\n",
    },
    RejectionCase {
        description: "primitive and object reasons print in publication order",
        options: &[],
        source: "Promise.reject(Symbol(\"s\")); Promise.reject(null); Promise.reject({a:1}); Promise.reject(123n);",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: Symbol(s)\n\
Possibly unhandled promise rejection: null\n\
Possibly unhandled promise rejection: { a: 1 }\n\
Possibly unhandled promise rejection: 123n\n",
    },
    RejectionCase {
        description: "a rejection handled before the drain stays silent",
        options: &[],
        source: "var p = Promise.reject(new Error(\"handled\")); p.catch(function(){});",
        expected_status: 0,
        expected_stderr: b"",
    },
    RejectionCase {
        description: "a handler attached from a pending job clears the entry",
        options: &[],
        source: "var p = Promise.reject(new Error(\"late\")); \
Promise.resolve().then(function(){ p.catch(function(){}); });",
        expected_status: 0,
        expected_stderr: b"",
    },
    RejectionCase {
        description: "then with only onFulfilled derives a reported rejection",
        options: &[],
        source: "Promise.reject(new Error(\"down\")).then(function(v){});",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: Error: down\n    at <eval> (<cmdline>:1:25)\n",
    },
    RejectionCase {
        description: "a reaction-job throw and a direct rejection keep FIFO order",
        options: &[],
        source: "Promise.resolve().then(function(){ throw new Error(\"job-boom\"); }); \
Promise.reject(\"r\");",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: \"r\"\n\
Possibly unhandled promise rejection: Error: job-boom\n    at <anonymous> (<cmdline>:1:51)\n",
    },
    RejectionCase {
        description: "a rejection created inside a reaction job is reported",
        options: &[],
        source: "Promise.resolve().then(function(){ Promise.reject(new Error(\"nested\")); });",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: Error: nested\n    at <anonymous> (<cmdline>:1:60)\n",
    },
    RejectionCase {
        description: "a catch callback returning a rejected promise is reported",
        options: &[],
        source: "Promise.reject(1).catch(function(){ return Promise.reject(new Error(\"rethrown\")); });",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: Error: rethrown\n    at <anonymous> (<cmdline>:1:68)\n",
    },
    RejectionCase {
        description: "a throwing then getter omits the resolving-function frame like QuickJS",
        options: &[],
        source: "Promise.resolve({get then(){throw new Error(\"gt\")}});",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: Error: gt\n    at get then (<cmdline>:1:44)\n    at resolve (native)\n    at <eval> (<cmdline>:1:16)\n",
    },
    RejectionCase {
        description: "a failed dynamic import reports the loader's reference error verbatim",
        options: &[],
        source: "import(\"fixture-missing-xyz\");",
        expected_status: 1,
        expected_stderr: b"Possibly unhandled promise rejection: ReferenceError: could not load module filename 'fixture-missing-xyz'\n\n",
    },
];

#[test]
fn unhandled_promise_rejections_match_pinned_golden() {
    for case in REJECTION_CASES {
        let output = run_cli(
            env!("CARGO_BIN_EXE_qjs").as_ref(),
            case.options,
            case.source,
            case.description,
        );
        assert_eq!(
            output.status.code(),
            Some(case.expected_status),
            "{}: {}",
            case.description,
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(output.stdout.is_empty(), "{}", case.description);
        assert_eq!(output.stderr, case.expected_stderr, "{}", case.description);
    }
}

#[test]
fn unhandled_promise_rejections_match_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP promise-rejection differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for case in REJECTION_CASES {
        let quickjs = run_cli(&oracle, case.options, case.source, case.description);
        let oxide = run_cli(
            env!("CARGO_BIN_EXE_qjs").as_ref(),
            case.options,
            case.source,
            case.description,
        );
        assert_eq!(
            oxide.status.code(),
            quickjs.status.code(),
            "{}",
            case.description
        );
        assert_eq!(oxide.stdout, quickjs.stdout, "{}", case.description);
        assert_eq!(oxide.stderr, quickjs.stderr, "{}", case.description);
    }
}

#[test]
fn no_unhandled_rejection_flag_silences_the_report_and_exit_failure() {
    let source = "Promise.reject(new Error(\"ignored\"));";

    let silenced = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        &["--no-unhandled-rejection"],
        source,
        "--no-unhandled-rejection silences the report",
    );
    assert_eq!(silenced.status.code(), Some(0));
    assert!(silenced.stdout.is_empty());
    assert!(silenced.stderr.is_empty());

    let reported = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        &[],
        source,
        "default tracking reports the rejection",
    );
    assert_eq!(reported.status.code(), Some(1));
    assert_eq!(
        reported.stderr,
        b"Possibly unhandled promise rejection: Error: ignored\n    at <eval> (<cmdline>:1:25)\n"
    );
}

#[test]
fn no_unhandled_rejection_flag_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP --no-unhandled-rejection differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let source = "Promise.reject(new Error(\"ignored\"));";
    let quickjs = run_cli(&oracle, &["--no-unhandled-rejection"], source, "flag");
    let oxide = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        &["--no-unhandled-rejection"],
        source,
        "flag",
    );
    assert_eq!(oxide.status.code(), quickjs.status.code());
    assert_eq!(oxide.stdout, quickjs.stdout);
    assert_eq!(oxide.stderr, quickjs.stderr);
}

#[test]
fn module_evaluation_rejection_is_reported_even_when_import_is_handled() {
    let fixture = ModuleFixture::new();
    fixture.write("b.mjs", "print('b ok');\n");
    fixture.write("a.mjs", "import './b.mjs';\nthrow new Error('a-fails');\n");
    let entry = fixture.write(
        "entry.mjs",
        "await import('./a.mjs').then(function(){ print('resolved'); }, \
function(e){ print('rejected', e.name, e.message); });\n\
print('entry continues');\n",
    );

    let oxide = run_file(&[], &entry);
    assert_eq!(oxide.status.code(), Some(1));
    assert_eq!(
        oxide.stdout,
        b"b ok\nrejected Error a-fails\nentry continues\n"
    );
    let diagnostic = String::from_utf8(oxide.stderr.clone()).unwrap();
    assert!(
        diagnostic.starts_with("Possibly unhandled promise rejection: Error: a-fails\n"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("a.mjs:2:16"), "{diagnostic}");

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP module-rejection differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let quickjs = Command::new(oracle)
        .arg(cli_path(&entry))
        .output()
        .expect("run QuickJS module rejection case");
    assert_eq!(oxide.status.code(), quickjs.status.code());
    assert_eq!(oxide.stdout, quickjs.stdout);
    assert_eq!(oxide.stderr, quickjs.stderr);
}

/// A2-1 (cross-family review): a dynamic import of a missing file from a
/// module must surface the host loader's `ReferenceError`
/// ("could not load module filename '<name>'") exactly once. Previously the
/// engine re-wrapped the host message, doubling it. Covers both the rejection
/// line on stderr and the caught channel (name/message), byte-exact vs qjs.
#[test]
fn failed_dynamic_import_message_is_not_doubled() {
    let fixture = ModuleFixture::new();
    let entry = fixture.write("entry.mjs", "import('./missing-xyz.mjs');\n");

    // Golden: the phrase "could not load module" appears exactly once (no
    // double wrap), and the line keeps the normalized specifier.
    let unhandled = run_file(&[], &entry);
    assert_eq!(unhandled.status.code(), Some(1));
    assert!(unhandled.stdout.is_empty());
    let report = String::from_utf8(unhandled.stderr).unwrap();
    assert!(
        report.starts_with("Possibly unhandled promise rejection: "),
        "{report}"
    );
    assert!(
        report.contains("could not load module filename '"),
        "{report}"
    );
    assert_eq!(
        report.matches("could not load module").count(),
        1,
        "{report}"
    );
    // The loader error carries an empty `stack` (it is raised from a host
    // callback without a JS frame), exactly like pinned qjs, so the report
    // ends with two newlines.
    assert!(report.ends_with("missing-xyz.mjs'\n\n"), "{report}");

    // Caught channel: error.name/message survive unchanged, exit 0. The
    // module goal normalizes the relative specifier to an absolute filename.
    let missing_path = fixture.root.join("missing-xyz.mjs");
    let caught_entry = fixture.write(
        "caught.mjs",
        "import('./missing-xyz.mjs').catch(function(error) {\n\
         print(error.name);\n\
         print(error.message);\n\
         });\n",
    );
    let caught = run_file(&[], &caught_entry);
    assert!(
        caught.status.success(),
        "{}",
        String::from_utf8_lossy(&caught.stderr)
    );
    assert_eq!(
        String::from_utf8(caught.stdout).unwrap(),
        format!(
            "ReferenceError\ncould not load module filename '{}'\n",
            cli_path(&missing_path)
        )
    );
    assert!(caught.stderr.is_empty());

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP module-load-failure differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for path in [&entry, &caught_entry] {
        let quickjs = Command::new(&oracle)
            .arg(cli_path(path))
            .output()
            .expect("run QuickJS module load failure case");
        let oxide = run_file(&[], path);
        assert_eq!(
            oxide.status.code(),
            quickjs.status.code(),
            "{}",
            path.display()
        );
        assert_eq!(oxide.stdout, quickjs.stdout, "{}", path.display());
        assert_eq!(oxide.stderr, quickjs.stderr, "{}", path.display());
    }
}
