use crate::runtime_oracle::run_cli;
#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MODULE_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct ModuleFixture {
    root: PathBuf,
}

impl ModuleFixture {
    fn new() -> Self {
        let id = NEXT_MODULE_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "quickjs-oxide-cli-module-{}-{id}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale CLI module fixture");
        }
        fs::create_dir_all(&root).expect("create CLI module fixture");
        Self { root }
    }

    fn write(&self, relative: &str, source: &str) -> PathBuf {
        self.write_bytes(relative, source.as_bytes())
    }

    fn write_bytes(&self, relative: &str, source: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create CLI module fixture directory");
        }
        fs::write(&path, source).expect("write CLI module fixture");
        path
    }
}

impl Drop for ModuleFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn qjs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_qjs"))
}

fn cli_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    return path.replace('\\', "/");
    #[cfg(not(windows))]
    path.into_owned()
}

fn run_file(arguments: &[&str], path: &Path) -> std::process::Output {
    qjs()
        .args(arguments)
        .arg(cli_path(path))
        .output()
        .expect("run qjs file")
}

fn expected_file_url(path: &Path) -> String {
    let filename = cli_path(path);
    if filename.contains(':') {
        filename
    } else {
        format!("file://{}", path.canonicalize().unwrap().display())
    }
}

#[test]
fn eval_executes_the_rust_compiler_and_vm() {
    let output = qjs().args(["-e", "(6 + 1) * 6"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn print_result_exposes_the_completion_value_without_changing_eval_default() {
    let output = qjs()
        .args([
            "--print-result",
            "-e",
            "(function(a) { return a + 1; })(41)",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn qjs_keeps_quickjs_default_non_blocking_host_policy() {
    let output = qjs()
        .args([
            "-e",
            "Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 1, 0)",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "TypeError: cannot block in this thread\n    at wait (native)\n    at <eval> (<cmdline>:1:13)\n"
    );
}

#[test]
fn eval_executes_source_level_functions_and_formats_native_errors() {
    let function = qjs()
        .args(["-e", "(function(a, b) { return a + b; })(20, 22)"])
        .output()
        .unwrap();
    assert!(function.status.success());

    let error = qjs().args(["-e", "1n + 1"]).output().unwrap();
    assert_eq!(error.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(error.stderr).unwrap(),
        "TypeError: cannot convert bigint to number\n    at <eval> (<cmdline>:1:4)\n"
    );
}

#[test]
fn unparenthesized_power_unary_error_omits_a_source_frame_like_quickjs() {
    for source in ["-2 ** 2", "-value++ ** 2"] {
        let output = qjs().args(["-e", source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{source}");
        assert!(output.stdout.is_empty(), "{source}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "SyntaxError: unparenthesized unary expression can't appear on the left-hand side of '**'\n\n",
            "{source}"
        );
    }

    let dynamic = qjs()
        .args(["-e", "Function(\"return -2 ** 2\")"])
        .output()
        .unwrap();
    assert_eq!(dynamic.status.code(), Some(1));
    assert!(dynamic.stdout.is_empty());
    assert_eq!(
        String::from_utf8(dynamic.stderr).unwrap(),
        "SyntaxError: unparenthesized unary expression can't appear on the left-hand side of '**'\n    at Function (native)\n    at <eval> (<cmdline>:1:9)\n"
    );
}

#[test]
fn eval_executes_the_dynamic_function_constructor_path() {
    for source in [
        "throw Function(\"a\", \"return a + 1\")(41)",
        "throw new Function(\"return 42\")()",
    ] {
        let output = qjs().args(["-e", source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap(), "42\n");
    }
}

#[test]
fn exception_output_quotes_strings_and_marks_bigints() {
    for (source, expected) in [
        ("throw \"x\"", "\"x\"\n"),
        (
            "throw 123456789012345678901234567890n",
            "123456789012345678901234567890n\n",
        ),
        ("throw -0", "-0\n"),
    ] {
        let output = qjs().args(["-e", source]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{source}");
        assert!(output.stdout.is_empty(), "{source}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn unsupported_source_fails_instead_of_falling_back_to_an_external_engine() {
    let output = qjs().args(["-e", "answer"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("'answer' is not defined"));
}

#[test]
fn dynamic_import_reaches_the_async_host_rejection_path() {
    let output = qjs()
        .args([
            "-e",
            "import('fixture').catch(function(error) { print(error.name + ':' + error.message); });",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostic = String::from_utf8(output.stdout).unwrap();
    assert!(diagnostic.starts_with("ReferenceError:"), "{diagnostic:?}");
    assert!(
        diagnostic.contains("module filename 'fixture'"),
        "{diagnostic:?}"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn eval_dynamic_import_uses_the_process_file_loader() {
    let fixture = ModuleFixture::new();
    let dependency = fixture.write(
        "eval-dependency.mjs",
        "export const answer = 42; export const main = import.meta.main;\n",
    );
    let specifier = dependency.to_string_lossy();
    let source = format!(
        "import({specifier:?}).then(function(module) {{ print(module.answer, module.main); }});"
    );
    let output = qjs().args(["-e", &source]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42 false\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn eval_module_matches_qjs_platform_cmdline_import_meta_initialization() {
    let output = qjs()
        .args(["-m", "-e", "print(import.meta.url, import.meta.main)"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    #[cfg(windows)]
    assert_eq!(output.stdout, b"file://<cmdline> true\n");
    #[cfg(not(windows))]
    assert_eq!(output.stdout, b"undefined undefined\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn explicit_module_modes_load_relative_files_and_wait_for_top_level_await() {
    let fixture = ModuleFixture::new();
    let dependency = fixture.write(
        "dependency.js",
        concat!(
            "await Promise.resolve();\n",
            "export const answer = 42;\n",
            "export const dependencyMain = import.meta.main;\n",
            "export const dependencyUrl = import.meta.url;\n",
        ),
    );
    let entry = fixture.write(
        "entry.js",
        concat!(
            "import { answer, dependencyMain, dependencyUrl } from './dependency.js';\n",
            "print(answer);\n",
            "print(import.meta.main);\n",
            "print(import.meta.url);\n",
            "print(dependencyMain);\n",
            "print(dependencyUrl);\n",
        ),
    );
    let expected = format!(
        "42\ntrue\n{}\nfalse\n{}\n",
        expected_file_url(&entry),
        expected_file_url(&dependency),
    );

    for arguments in [["-m"].as_slice(), ["--module"].as_slice()] {
        let output = run_file(arguments, &entry);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn file_loader_matches_quickjs_json_json5_classification_and_rejects_unknown_keys() {
    let fixture = ModuleFixture::new();
    fixture.write("by-extension.json", r#"{"answer":40}"#);
    fixture.write("by-attribute.data", r#"{"answer":2}"#);
    fixture.write("by-json5-attribute.data", "{answer:+3,}");
    fixture.write("script.json5", "export default 4;\n");
    fixture.write("strict-override.json5", r#"{"answer":5}"#);
    fixture.write("extended-override.json", "{answer:0b110,}");
    fixture.write("unknown-on-json.json", r#"{"answer":7}"#);
    fixture.write("unknown-on-data.data", "export default 8;\n");
    let entry = fixture.write(
        "json-entry.mjs",
        concat!(
            "import extension from './by-extension.json';\n",
            "import attribute from './by-attribute.data' with { type: 'json' };\n",
            "import json5 from './by-json5-attribute.data' with { type: 'json5' };\n",
            "import script from './script.json5';\n",
            "import strictOverride from './strict-override.json5' with { type: 'json' };\n",
            "import extendedOverride from './extended-override.json' with { type: 'json5' };\n",
            "import unknownJson from './unknown-on-json.json' with { type: 'other' };\n",
            "import unknownData from './unknown-on-data.data' with { type: 'other' };\n",
            "print([extension.answer, attribute.answer, json5.answer, script, ",
            "strictOverride.answer, extendedOverride.answer, unknownJson.answer, ",
            "unknownData].join(','));\n",
        ),
    );
    let json = run_file(&[], &entry);
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    assert_eq!(json.stdout, b"40,2,3,4,5,6,7,8\n");
    assert!(json.stderr.is_empty());

    let rejected = fixture.write(
        "bad-attribute.mjs",
        "import './by-extension.json' with { integrity: 'x' };\n",
    );
    let rejected = run_file(&[], &rejected);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(rejected.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("TypeError: import attribute 'integrity' is not supported"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
}

#[test]
fn file_goal_autodetects_mjs_and_leading_static_module_syntax() {
    let fixture = ModuleFixture::new();
    fixture.write("static-dependency.js", "print('static import');\n");
    let extension = fixture.write(
        "extension.mjs",
        "await Promise.resolve(); print('extension');\n",
    );
    let syntax = fixture.write(
        "syntax.js",
        "// leading trivia\nexport const answer = 42; print(answer);\n",
    );
    let hashbang = fixture.write(
        "hashbang.js",
        "#!/usr/bin/env qjs\nexport const answer = 42; print(answer);\n",
    );
    let static_import = fixture.write("static-import.js", "import './static-dependency.js';\n");
    let dotfile = fixture.write(".mjs", "await Promise.resolve(); print('dotfile');\n");

    for (path, expected) in [
        (&extension, b"extension\n".as_slice()),
        (&syntax, b"42\n"),
        (&hashbang, b"42\n"),
        (&static_import, b"static import\n"),
        (&dotfile, b"dotfile\n"),
    ] {
        let output = run_file(&[], path);
        assert!(
            output.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected, "{}", path.display());
        assert!(output.stderr.is_empty(), "{}", path.display());
    }
}

#[test]
fn file_goal_preserves_raw_bytes_during_detection_and_evaluation() {
    let fixture = ModuleFixture::new();
    let raw_script = fixture.write_bytes("raw-script.js", b"/*\x80*/print(42);\n");
    let raw_module = fixture.write_bytes(
        "raw-module.js",
        b"/*\xff*/export const answer = 42; print(answer);\n",
    );
    let raw_hashbang_module = fixture.write_bytes(
        "raw-hashbang.js",
        b"#!\x80\xff\nexport const answer = 42; print(answer);\n",
    );

    for path in [&raw_script, &raw_module, &raw_hashbang_module] {
        let output = run_file(&[], path);
        assert!(
            output.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"42\n", "{}", path.display());
        assert!(output.stderr.is_empty(), "{}", path.display());
    }

    let nul_comment = fixture.write_bytes(
        "nul-comment.js",
        b"/*\0*/export const answer = 42; print(answer);\n",
    );
    let automatic = run_file(&[], &nul_comment);
    assert_eq!(automatic.status.code(), Some(1));
    assert!(automatic.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&automatic.stderr).contains("export"),
        "{}",
        String::from_utf8_lossy(&automatic.stderr)
    );

    let forced_module = run_file(&["--module"], &nul_comment);
    assert!(
        forced_module.status.success(),
        "{}",
        String::from_utf8_lossy(&forced_module.stderr)
    );
    assert_eq!(forced_module.stdout, b"42\n");
    assert!(forced_module.stderr.is_empty());

    let forced_script = run_file(&["--script"], &raw_module);
    assert_eq!(forced_script.status.code(), Some(1));
    assert!(forced_script.stdout.is_empty());
    assert!(String::from_utf8_lossy(&forced_script.stderr).contains("export"));
}

#[test]
fn file_module_loader_preserves_raw_dependency_bytes() {
    let fixture = ModuleFixture::new();
    fixture.write_bytes("raw-dependency.js", b"/*\x80*/export const answer = 42;\n");
    let entry = fixture.write(
        "raw-dependency-entry.mjs",
        "import { answer } from './raw-dependency.js'; print(answer);\n",
    );

    let output = run_file(&[], &entry);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn file_module_loader_preserves_raw_json_dependency_bytes() {
    let fixture = ModuleFixture::new();
    fixture.write_bytes(
        "raw-value.json",
        b"{\"wtf\":\"\xed\xa0\x80\",\"cesu\":\"\xed\xa0\xbd\xed\xb8\x80\"}",
    );
    fixture.write_bytes(
        "raw-value.data",
        b"/*\x80*/{answer:42,marker:'\xed\xa0\x80',}",
    );
    let entry = fixture.write(
        "raw-json-entry.mjs",
        concat!(
            "import strict from './raw-value.json';\n",
            "import extended from './raw-value.data' with { type: 'json5' };\n",
            "const exact = strict.wtf.length === 1 && ",
            "strict.wtf.charCodeAt(0) === 0xd800 && ",
            "strict.cesu.length === 2 && ",
            "strict.cesu.codePointAt(0) === 0x1f600 && ",
            "extended.marker.length === 1 && ",
            "extended.marker.charCodeAt(0) === 0xd800;\n",
            "print(exact ? extended.answer : 0);\n",
        ),
    );

    let output = run_file(&[], &entry);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());

    let malformed = fixture.write_bytes("malformed.json", b"{\"x\":\"\x80\"}");
    let malformed_entry = fixture.write(
        "malformed-entry.mjs",
        "import value from './malformed.json'; print(value);\n",
    );
    let malformed_output = run_file(&[], &malformed_entry);
    assert_eq!(malformed_output.status.code(), Some(1));
    assert!(malformed_output.stdout.is_empty());
    let diagnostic = String::from_utf8(malformed_output.stderr).unwrap();
    assert!(
        diagnostic.contains("SyntaxError: Bad UTF-8 sequence"),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains(&format!("{}:1:7", cli_path(&malformed))),
        "{diagnostic}"
    );
}

#[test]
fn script_override_wins_over_mjs_module_detection() {
    let fixture = ModuleFixture::new();
    let entry = fixture.write("forced-script.mjs", "export const answer = 42;\n");

    let output = run_file(&["--script"], &entry);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("export"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let module_last = run_file(&["--script", "-m"], &entry);
    assert!(
        module_last.status.success(),
        "{}",
        String::from_utf8_lossy(&module_last.stderr)
    );
    assert!(module_last.stdout.is_empty());
    assert!(module_last.stderr.is_empty());

    let script_last = run_file(&["-m", "--script"], &entry);
    assert_eq!(script_last.status.code(), Some(1));
    assert!(script_last.stdout.is_empty());
    assert!(String::from_utf8_lossy(&script_last.stderr).contains("export"));
}

#[test]
fn dynamic_import_stays_script_goal_and_uses_the_file_loader() {
    let fixture = ModuleFixture::new();
    fixture.write("dependency.mjs", "export const answer = 42;\n");
    let entry = fixture.write(
        "dynamic.js",
        concat!(
            "import('./dependency.mjs').then(function(module) { print(module.answer); });\n",
            "print(this === globalThis);\n",
        ),
    );

    let output = run_file(&[], &entry);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"true\n42\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn rejected_module_promise_is_reported_once() {
    let fixture = ModuleFixture::new();
    let entry = fixture.write("rejected.mjs", "await Promise.resolve(); throw 42;\n");

    let output = run_file(&[], &entry);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"42\n");
}

#[test]
fn missing_main_file_uses_the_qjs_path_diagnostic_shape() {
    let fixture = ModuleFixture::new();
    let missing = fixture.root.join("missing.js");
    let output = run_file(&[], &missing);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.starts_with(&format!("{}: ", cli_path(&missing))));
    assert!(!diagnostic.starts_with("qjs:"));
}

#[test]
fn tracked_file_module_demo_returns_42() {
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/module-42.mjs");
    let output = run_file(&[], &demo);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn tracked_file_module_demo_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP file-module differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/module-42.mjs");
    let filename = cli_path(&demo);
    let oxide = run_file(&[], &demo);
    let quickjs = Command::new(oracle)
        .arg(&filename)
        .output()
        .expect("run QuickJS file-module demo");

    assert_eq!(oxide.status.code(), quickjs.status.code());
    assert_eq!(oxide.stdout, quickjs.stdout);
    assert_eq!(oxide.stderr, quickjs.stderr);
}

#[test]
fn version_names_the_pinned_compatibility_target() {
    let output = qjs().arg("--version").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("QuickJS 2026-06-04"));
}

#[test]
fn strip_flags_match_quickjs_debug_stack_behavior_and_last_option_wins() {
    let source = "1n + 1";
    let located = "TypeError: cannot convert bigint to number\n    at <eval> (<cmdline>:1:4)\n";
    let stripped = "TypeError: cannot convert bigint to number\n    at <eval>\n";
    for (arguments, expected) in [
        (vec!["--strip-source", "-e", source], located),
        (vec!["-s", "-e", source], stripped),
        (vec!["-s", "--strip-source", "-e", source], located),
        (vec!["--strip-source", "-s", "-e", source], stripped),
        (vec!["-e", source, "-s"], stripped),
        (vec!["-se", source], stripped),
        (vec!["-e1n + 1", "--strip-source"], located),
    ] {
        let output = qjs().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
    }

    for arguments in [vec!["-sq"], vec!["-qs"], vec!["-q", "-s"]] {
        let output = qjs().args(arguments).output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

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
        ("invalid prefix update operand", "++1"),
        (
            "postfix under unary power early error has no source frame",
            "-value++ ** 2",
        ),
        (
            "strict private postfix update return marker",
            "(function named(){ 'use strict'; return named++; })()",
        ),
    ] {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), &[], source, description);
        let quickjs = run_cli(&oracle, &[], source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}
