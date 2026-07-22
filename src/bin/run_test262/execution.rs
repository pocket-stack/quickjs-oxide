use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use quickjs_oxide::{
    CompileOptions, Context, DescriptorField, ErrorKind, OrdinaryPropertyDescriptor, Runtime,
    RuntimeError, Value,
};

use super::metadata::{Metadata, parse_metadata};
use super::report::WorkerResult;
use super::{Variant, WorkerOptions, validate_relative_test_path};

const WORKER_HOST_FILENAME: &str = "<test262-worker-host>";
const WORKER_HOST_SOURCE: &str = r#"
globalThis.print = function print(value) {};
"#;

pub(super) fn run_isolated_worker(
    executable: &Path,
    suite: &Path,
    test: &Path,
    variant: Variant,
    timeout: Duration,
) -> Result<WorkerResult, String> {
    let mut child = Command::new(executable)
        .arg("--worker-one")
        .arg("--suite")
        .arg(suite)
        .arg("--test")
        .arg(test)
        .arg("--variant")
        .arg(variant.name())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn worker for {}: {error}", test.display()))?;
    let Some(stdout) = child.stdout.take() else {
        terminate_and_reap(&mut child);
        return Err("worker stdout pipe was missing".to_owned());
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_and_reap(&mut child);
        return Err("worker stderr pipe was missing".to_owned());
    };
    let stdout_reader = spawn_pipe_reader(stdout, "stdout");
    let stderr_reader = spawn_pipe_reader(stderr, "stderr");
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Err(error) => {
                terminate_and_reap(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format!("wait for {}: {error}", test.display()));
            }
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                terminate_and_reap(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Ok(WorkerResult::failure(
                    "timeout",
                    "host",
                    "",
                    format!("worker exceeded {} ms", timeout.as_millis()),
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
        }
    };
    let stdout = join_pipe_reader(stdout_reader, "stdout");
    let stderr = join_pipe_reader(stderr_reader, "stderr");
    let stdout = stdout?;
    let stderr = stderr?;
    if !status.success() {
        return Ok(WorkerResult::failure(
            "crash",
            "host",
            "",
            format!("worker exited with {status}: {}", stderr.trim()),
        ));
    }
    WorkerResult::decode(&stdout).map_err(|error| {
        format!(
            "decode worker for {}: {error}; stderr={:?}",
            test.display(),
            stderr.trim()
        )
    })
}

fn terminate_and_reap(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn spawn_pipe_reader(
    mut pipe: impl Read + Send + 'static,
    name: &'static str,
) -> thread::JoinHandle<Result<String, String>> {
    thread::spawn(move || {
        let mut output = String::new();
        pipe.read_to_string(&mut output)
            .map_err(|error| format!("read worker {name}: {error}"))?;
        Ok(output)
    })
}

fn join_pipe_reader(
    reader: thread::JoinHandle<Result<String, String>>,
    name: &str,
) -> Result<String, String> {
    reader
        .join()
        .map_err(|_| format!("worker {name} reader panicked"))?
}

pub(super) fn run_worker(options: &WorkerOptions) -> Result<WorkerResult, String> {
    validate_relative_test_path(&options.test)?;
    let path = options.suite.join(&options.test);
    let source =
        fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let metadata = parse_metadata(&source)?;
    if metadata.is_module() || metadata.is_async() {
        return Err("unsupported test reached worker".to_owned());
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    install_worker_host(&runtime, &mut context)?;
    // The progress baseline follows the pinned Test262 interpretation rather
    // than run-test262.c's raw-test deviation: raw means no harness and no
    // source rewriting. The qjs-compatible `print` surface above is a worker
    // host capability installed as its own script; harness files likewise
    // remain separate scripts and keep their own filenames.
    let mut includes = Vec::new();
    if !metadata.is_raw() {
        includes.extend(["assert.js".to_owned(), "sta.js".to_owned()]);
        includes.extend(metadata.includes.iter().cloned());
    }
    for include in includes {
        let include_path = options.suite.join("harness").join(&include);
        let harness = fs::read_to_string(&include_path)
            .map_err(|error| format!("read {}: {error}", include_path.display()))?;
        let compile_options = CompileOptions::new(include_path.to_string_lossy());
        let function = match context
            .compile_with_options_preserving_unsupported_diagnostics(&harness, &compile_options)
        {
            Ok(function) => function,
            Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Unsupported => {
                return Ok(WorkerResult::failure(
                    "unsupported-harness-parser",
                    "harness",
                    "Unsupported",
                    format!("{include}: {}", error.message()),
                ));
            }
            Err(RuntimeError::Exception) => {
                let (error_type, detail) =
                    take_error(&runtime, &mut context, RuntimeError::Exception);
                return Ok(WorkerResult::failure(
                    "harness-error",
                    "harness",
                    error_type,
                    format!("{include}: {detail}"),
                ));
            }
            Err(error) => {
                return Ok(engine_fault(
                    "harness-engine-fault",
                    "harness-compile",
                    error,
                    Some(&include),
                ));
            }
        };
        if let Err(error) = context.execute(&function) {
            return Ok(match error {
                RuntimeError::Engine(error) if error.kind() == ErrorKind::Unsupported => {
                    WorkerResult::failure(
                        "unsupported-harness-runtime",
                        "harness-runtime",
                        "Unsupported",
                        format!("{include}: {}", error.message()),
                    )
                }
                RuntimeError::Exception => {
                    let (error_type, detail) =
                        take_error(&runtime, &mut context, RuntimeError::Exception);
                    WorkerResult::failure(
                        "harness-error",
                        "harness-runtime",
                        error_type,
                        format!("{include}: {detail}"),
                    )
                }
                error => engine_fault(
                    "harness-engine-fault",
                    "harness-runtime",
                    error,
                    Some(&include),
                ),
            });
        }
    }

    let authored = if options.variant == Variant::Strict {
        format!("\"use strict\";\n{source}")
    } else {
        source
    };
    let filename = path.to_string_lossy();
    let compile_options = CompileOptions::new(filename.as_ref());
    let function = match context
        .compile_with_options_preserving_unsupported_diagnostics(&authored, &compile_options)
    {
        Ok(function) => function,
        Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Unsupported => {
            return Ok(WorkerResult::failure(
                "unsupported-parser",
                "parse",
                "Unsupported",
                error.message(),
            ));
        }
        Err(RuntimeError::Exception) => {
            let (error_type, detail) = take_error(&runtime, &mut context, RuntimeError::Exception);
            return Ok(classify_completion(
                &metadata,
                "parse",
                &error_type,
                &detail,
            ));
        }
        Err(error) => return Ok(engine_fault("engine-fault", "parse", error, None)),
    };
    if metadata
        .negative
        .as_ref()
        .and_then(|negative| negative.phase.as_deref())
        .is_some_and(|phase| matches!(phase, "parse" | "early"))
    {
        return Ok(classify_normal(&metadata));
    }
    match context.execute(&function) {
        Ok(_) => Ok(classify_normal(&metadata)),
        Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Unsupported => {
            Ok(WorkerResult::failure(
                "unsupported-runtime",
                "runtime",
                "Unsupported",
                error.message(),
            ))
        }
        Err(RuntimeError::Exception) => {
            let (error_type, detail) = take_error(&runtime, &mut context, RuntimeError::Exception);
            Ok(classify_completion(
                &metadata,
                "runtime",
                &error_type,
                &detail,
            ))
        }
        Err(error) => Ok(engine_fault("engine-fault", "runtime", error, None)),
    }
}

fn install_worker_host(runtime: &Runtime, context: &mut Context) -> Result<(), String> {
    let options = CompileOptions::new(WORKER_HOST_FILENAME);
    let function = context
        .compile_with_options_preserving_unsupported_diagnostics(WORKER_HOST_SOURCE, &options)
        .map_err(|error| worker_host_error(runtime, context, "compile", error))?;
    context
        .execute(&function)
        .map_err(|error| worker_host_error(runtime, context, "execute", error))?;
    install_code_point_range_host(runtime, context)
}

fn install_code_point_range_host(runtime: &Runtime, context: &mut Context) -> Result<(), String> {
    let object_262 = match context.new_object() {
        Ok(object) => object,
        Err(error) => return Err(worker_host_error(runtime, context, "create $262", error)),
    };
    let code_point_range = match context.new_code_point_range_function() {
        Ok(function) => function,
        Err(error) => {
            return Err(worker_host_error(
                runtime,
                context,
                "create $262.codePointRange",
                error,
            ));
        }
    };
    let code_point_range_key = runtime
        .intern_property_key("codePointRange")
        .map_err(|error| format!("intern Test262 host codePointRange key: {error}"))?;
    let defined = match context.define_own_property(
        &object_262,
        &code_point_range_key,
        &worker_host_data_property(Value::Object(code_point_range.as_object().clone())),
    ) {
        Ok(defined) => defined,
        Err(error) => {
            return Err(worker_host_error(
                runtime,
                context,
                "define $262.codePointRange",
                error,
            ));
        }
    };
    if !defined {
        return Err("Test262 worker host rejected $262.codePointRange".to_owned());
    }

    let object_262_key = runtime
        .intern_property_key("$262")
        .map_err(|error| format!("intern Test262 host $262 key: {error}"))?;
    let global = match context.global_object() {
        Ok(global) => global,
        Err(error) => {
            return Err(worker_host_error(
                runtime,
                context,
                "get Test262 global",
                error,
            ));
        }
    };
    let defined = match context.define_own_property(
        &global,
        &object_262_key,
        &worker_host_data_property(Value::Object(object_262)),
    ) {
        Ok(defined) => defined,
        Err(error) => {
            return Err(worker_host_error(
                runtime,
                context,
                "define global $262",
                error,
            ));
        }
    };
    if !defined {
        return Err("Test262 worker host rejected global $262".to_owned());
    }
    Ok(())
}

fn worker_host_data_property(value: Value) -> OrdinaryPropertyDescriptor {
    OrdinaryPropertyDescriptor {
        value: DescriptorField::Present(value),
        writable: DescriptorField::Present(true),
        enumerable: DescriptorField::Present(true),
        configurable: DescriptorField::Present(true),
        ..OrdinaryPropertyDescriptor::new()
    }
}

fn worker_host_error(
    runtime: &Runtime,
    context: &mut Context,
    phase: &str,
    error: RuntimeError,
) -> String {
    if error == RuntimeError::Exception {
        let (error_type, detail) = take_error(runtime, context, error);
        format!("Test262 worker host {phase} threw {error_type}: {detail}")
    } else {
        format!("Test262 worker host {phase} failed: {error}")
    }
}

fn engine_fault(
    outcome: &str,
    phase: &str,
    error: RuntimeError,
    prefix: Option<&str>,
) -> WorkerResult {
    let actual_type = match &error {
        RuntimeError::WrongRuntime(_) => "WrongRuntime",
        RuntimeError::Invariant(_) => "Invariant",
        RuntimeError::Exception => "MissingException",
        RuntimeError::Engine(_) => "EngineError",
        RuntimeError::Atom(_) => "AtomError",
        RuntimeError::Heap(_) => "HeapError",
        RuntimeError::Shape(_) => "ShapeError",
        RuntimeError::Property(_) => "PropertyError",
    };
    let detail = prefix.map_or_else(|| error.to_string(), |prefix| format!("{prefix}: {error}"));
    WorkerResult::failure(outcome, phase, actual_type, detail)
}

fn classify_normal(metadata: &Metadata) -> WorkerResult {
    if let Some(negative) = &metadata.negative {
        WorkerResult::failure(
            "fail-missing-throw",
            "normal",
            "",
            format!(
                "expected {} during {}",
                negative.error_type.as_deref().unwrap_or("an exception"),
                negative.phase.as_deref().unwrap_or("any phase")
            ),
        )
    } else {
        WorkerResult::pass("normal", "")
    }
}

fn classify_completion(
    metadata: &Metadata,
    actual_phase: &str,
    actual_type: &str,
    detail: &str,
) -> WorkerResult {
    let Some(negative) = &metadata.negative else {
        return WorkerResult::failure(
            format!("fail-{actual_phase}"),
            actual_phase,
            actual_type,
            detail,
        );
    };
    let expected_phase = negative.phase.as_deref();
    let phase_matches = match expected_phase {
        None => true,
        Some("parse" | "early") => actual_phase == "parse",
        Some("runtime") => actual_phase == "runtime",
        Some("resolution") => actual_phase == "resolution",
        Some(_) => false,
    };
    let type_matches = negative
        .error_type
        .as_deref()
        .is_none_or(|expected| expected == actual_type);
    if phase_matches && type_matches {
        WorkerResult::pass_with_detail(actual_phase, actual_type, detail)
    } else {
        WorkerResult::failure(
            "fail-negative-mismatch",
            actual_phase,
            actual_type,
            format!(
                "expected phase={} type={}; {detail}",
                expected_phase.unwrap_or("any"),
                negative.error_type.as_deref().unwrap_or("any")
            ),
        )
    }
}

fn take_error(runtime: &Runtime, context: &mut Context, error: RuntimeError) -> (String, String) {
    if error != RuntimeError::Exception {
        return ("EngineError".to_owned(), error.to_string());
    }
    let exception = match context.take_exception() {
        Ok(Some(exception)) => exception,
        Ok(None) => {
            return (
                "MissingException".to_owned(),
                "pending exception was empty".to_owned(),
            );
        }
        Err(error) => return ("EngineError".to_owned(), error.to_string()),
    };
    exception_text(runtime, context, exception)
}

fn exception_text(runtime: &Runtime, context: &mut Context, exception: Value) -> (String, String) {
    let Value::Object(object) = exception else {
        let kind = match &exception {
            Value::Undefined => "undefined",
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Int(_) | Value::Float(_) => "number",
            Value::BigInt(_) => "bigint",
            Value::String(_) => "string",
            Value::Symbol(_) => "symbol",
            Value::Object(_) => unreachable!(),
        };
        return (format!("Thrown{kind}"), primitive_text(exception));
    };

    let name_key = match runtime.intern_property_key("name") {
        Ok(key) => key,
        Err(error) => return ("EngineError".to_owned(), error.to_string()),
    };
    let message_key = match runtime.intern_property_key("message") {
        Ok(key) => key,
        Err(error) => return ("EngineError".to_owned(), error.to_string()),
    };
    let constructor_key = match runtime.intern_property_key("constructor") {
        Ok(key) => key,
        Err(error) => return ("EngineError".to_owned(), error.to_string()),
    };
    let mut name = String::new();
    if let Ok(Value::Object(constructor)) = context.get_property(&object, &constructor_key) {
        if let Ok(Value::String(value)) = context.get_property(&constructor, &name_key) {
            name = value.to_utf8_lossy();
        }
    }
    if name.is_empty() {
        name = match context.get_property(&object, &name_key) {
            Ok(Value::String(value)) if !value.is_empty() => value.to_utf8_lossy(),
            _ => String::new(),
        };
    }
    if name.is_empty() {
        name = "ThrownObject".to_owned();
    }
    let message = match context.get_property(&object, &message_key) {
        Ok(Value::String(value)) => value.to_utf8_lossy(),
        _ => String::new(),
    };
    (name, message)
}

fn primitive_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Symbol(_) => "Symbol()".to_owned(),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use quickjs_oxide::{CompileOptions, ErrorKind, Runtime, RuntimeError};

    use super::{classify_completion, run_worker};
    use crate::metadata::{Metadata, NegativeExpectation};
    use crate::{Variant, WorkerOptions};

    #[test]
    fn matching_negative_result_preserves_its_diagnostic_provenance() {
        let metadata = Metadata {
            negative: Some(NegativeExpectation {
                phase: Some("parse".to_owned()),
                error_type: Some("SyntaxError".to_owned()),
            }),
            ..Metadata::default()
        };
        let result = classify_completion(
            &metadata,
            "parse",
            "SyntaxError",
            "unexpected token in expression: '}'",
        );

        assert_eq!(result.outcome, "pass");
        assert_eq!(result.actual_phase, "parse");
        assert_eq!(result.actual_type, "SyntaxError");
        assert_eq!(result.detail, "unexpected token in expression: '}'");
    }

    #[test]
    fn parse_negative_that_compiles_is_not_executed() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-test262-{}-{unique}",
            std::process::id()
        ));
        let relative = PathBuf::from("test/parse-negative.js");
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::write(
            suite.join(&relative),
            "/*---\nflags: [raw]\nnegative:\n  phase: parse\n  type: SyntaxError\n---*/\nthrow 1;\n",
        )
        .unwrap();

        let result = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            variant: Variant::Sloppy,
        })
        .unwrap();
        fs::remove_dir_all(suite).unwrap();

        assert_eq!(result.outcome, "fail-missing-throw");
        assert_eq!(result.actual_phase, "normal");
        assert!(result.detail.contains("expected SyntaxError during parse"));
    }

    #[test]
    fn raw_worker_installs_print_host_for_coerce_global_style_tests() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-test262-print-{}-{unique}",
            std::process::id()
        ));
        let relative =
            PathBuf::from("test/built-ins/RegExp/prototype/Symbol.replace/coerce-global.js");
        let path = suite.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"/*---
flags: [raw]
features: [Symbol.replace]
---*/
if (typeof assert !== "undefined") {
    throw new Error("raw worker unexpectedly installed the Test262 harness");
}
if (typeof print !== "function" || print.name !== "print" || print.length !== 1) {
    throw new Error("qjs print host surface is missing");
}
if (print("discarded", 1, true) !== undefined) {
    throw new Error("qjs print host did not return undefined");
}

Array.print = print;
var r = /a/g;
Object.defineProperty(r, "global", { writable: true });
r.lastIndex = 0;
r.global = undefined;
if (r[Symbol.replace]("aa", "b") !== "ba") {
    throw new Error("coerce-global replacement did not complete");
}
"#,
        )
        .unwrap();

        let result = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            variant: Variant::Sloppy,
        })
        .unwrap();
        fs::remove_dir_all(suite).unwrap();

        assert_eq!(result.outcome, "pass", "{}", result.detail);
        assert_eq!(result.actual_phase, "normal");
    }

    #[test]
    fn raw_worker_installs_quickjs_code_point_range_host() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-test262-code-point-range-{}-{unique}",
            std::process::id()
        ));
        let relative = PathBuf::from("test/harness/code-point-range-host.js");
        let path = suite.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"/*---
flags: [raw]
---*/
if (typeof $262 !== "object" || typeof $262.codePointRange !== "function") {
    throw new Error("QuickJS codePointRange host surface is missing");
}
var globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "$262");
var helperDescriptor = Object.getOwnPropertyDescriptor($262, "codePointRange");
if (!globalDescriptor.writable || !globalDescriptor.enumerable ||
    !globalDescriptor.configurable || !helperDescriptor.writable ||
    !helperDescriptor.enumerable || !helperDescriptor.configurable) {
    throw new Error("QuickJS host property flags changed");
}
if ($262.codePointRange.name !== "codePointRange" ||
    $262.codePointRange.length !== 2 ||
    Object.getPrototypeOf($262.codePointRange) !== Function.prototype) {
    throw new Error("QuickJS codePointRange function metadata changed");
}
var constructorThrew = false;
try {
    new $262.codePointRange(0, 1);
} catch (error) {
    constructorThrew = error instanceof TypeError;
}
if (!constructorThrew) {
    throw new Error("QuickJS codePointRange became constructible");
}

var conversionLog = "";
var start = Object();
start.valueOf = function() { conversionLog += "s"; return 65.9; };
var end = Object();
end.valueOf = function() { conversionLog += "e"; return 68.9; };
var extra = Object();
extra.valueOf = function() { conversionLog += "x"; throw new Error("extra coerced"); };
if ($262.codePointRange.call(null, start, end, extra) !== "ABC" ||
    conversionLog !== "se") {
    throw new Error("QuickJS codePointRange conversion order changed");
}
var marker = Object();
start.valueOf = function() { conversionLog = "S"; throw marker; };
end.valueOf = function() { conversionLog += "E"; return 68; };
try {
    $262.codePointRange(start, end);
    throw new Error("QuickJS codePointRange swallowed a conversion throw");
} catch (error) {
    if (error !== marker || conversionLog !== "S") {
        throw new Error("QuickJS codePointRange throw order changed");
    }
}
if ($262.codePointRange(4294967361, 68) !== "ABC" ||
    $262.codePointRange(-1, 68) !== "") {
    throw new Error("QuickJS codePointRange ToUint32 behavior changed");
}
var surrogate = $262.codePointRange(0xD7FF, 0xD801);
if (surrogate.length !== 2 || surrogate.charCodeAt(0) !== 0xD7FF ||
    surrogate.charCodeAt(1) !== 0xD800) {
    throw new Error("QuickJS codePointRange surrogate behavior changed");
}
var capped = $262.codePointRange(0x10FFFF, -1);
if (capped.length !== 2 || capped.codePointAt(0) !== 0x10FFFF) {
    throw new Error("QuickJS codePointRange Unicode cap changed");
}
"#,
        )
        .unwrap();

        let result = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            variant: Variant::Sloppy,
        })
        .unwrap();
        fs::remove_dir_all(suite).unwrap();

        assert_eq!(result.outcome, "pass", "{}", result.detail);
        assert_eq!(result.actual_phase, "normal");
    }

    #[test]
    fn unsupported_parser_provenance_is_opt_in_at_the_context_boundary() {
        const UNSUPPORTED_SOURCE: &str = "class C { #field = 1; }";

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context.compile(UNSUPPORTED_SOURCE).unwrap_err(),
            RuntimeError::Exception
        );
        assert!(context.take_exception().unwrap().is_some());

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let options = CompileOptions::new("unsupported.js");
        let RuntimeError::Engine(error) = context
            .compile_with_options_preserving_unsupported_diagnostics(UNSUPPORTED_SOURCE, &options)
            .unwrap_err()
        else {
            panic!("diagnostic compile did not retain its engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert_eq!(
            error.message(),
            "private class elements are not implemented yet"
        );
        assert!(context.take_exception().unwrap().is_none());
    }
}
