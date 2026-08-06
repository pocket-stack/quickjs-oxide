use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use quickjs_oxide::{
    CompileOptions, Context, ErrorKind, Runtime, RuntimeError, Test262AgentSession, Value,
};

use super::metadata::{Metadata, parse_metadata};
use super::report::WorkerResult;
use super::requirements::{HostCapabilities, is_exact_agent_host_test};
use super::{Variant, WorkerOptions, validate_relative_test_path};

pub(super) const WORKER_HOST_CAPABILITIES: HostCapabilities = HostCapabilities {
    agent: false,
    can_block_false: true,
    create_realm: true,
    detach_array_buffer: true,
    eval_script: true,
    gc: true,
    global: true,
    is_html_dda: true,
};

const WORKER_HOST_FILENAME: &str = "<test262-worker-host>";
const WORKER_PRINT_LOG_PROPERTY: &str = "__quickjs_oxide_test262_print_log__";
const WORKER_HOST_SOURCE: &str = r#"
globalThis.print = function print(value) {};
"#;
const ASYNC_WORKER_HOST_SOURCE: &str = r#"
(function installTest262PrintHost() {
  var create = Object.create;
  var defineProperty = Object.defineProperty;
  var messages = Object.create(null);
  messages.length = 0;
  defineProperty(globalThis, "__quickjs_oxide_test262_print_log__", {
    get: function getTest262PrintLog() {
      var snapshot = create(null);
      defineProperty(snapshot, "length", { value: messages.length });
      for (var i = 0; i < messages.length; i += 1) {
        defineProperty(snapshot, i, { value: messages[i] });
      }
      return snapshot;
    },
    enumerable: false,
    configurable: false
  });
  globalThis.print = function print(value) {
    for (var i = 0; i < arguments.length; i += 1) {
      if (typeof arguments[i] === "string") {
        messages[messages.length] = arguments[i];
        messages.length += 1;
      }
    }
  };
})();
"#;
const ASYNC_COMPLETE: &str = "Test262:AsyncTestComplete";
const ASYNC_FAILURE_PREFIX: &str = "Test262:AsyncTestFailure:";

struct AgentRunGuard {
    session: Option<Test262AgentSession>,
}

impl AgentRunGuard {
    fn new(enabled: bool) -> Self {
        Self {
            session: enabled.then(Test262AgentSession::new),
        }
    }

    fn session(&self) -> Option<&Test262AgentSession> {
        self.session.as_ref()
    }

    fn finish(&mut self) -> Result<(), String> {
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        session
            .join_workers()
            .map_err(|error| format!("Test262 agent worker failed: {error}"))
    }
}

impl Drop for AgentRunGuard {
    fn drop(&mut self) {
        // Every early-return path still joins in start order. The normal tail
        // calls `finish` explicitly so agent failures remain observable.
        if let Some(session) = self.session.take() {
            let _ = session.join_workers();
        }
    }
}

pub(super) fn run_isolated_worker(
    executable: &Path,
    suite: &Path,
    test: &Path,
    variant: Variant,
    timeout: Duration,
    allow_async_host: bool,
    allow_agent_host: bool,
) -> Result<WorkerResult, String> {
    let mut command = Command::new(executable);
    command
        .arg("--worker-one")
        .arg("--suite")
        .arg(suite)
        .arg("--test")
        .arg(test)
        .arg("--variant")
        .arg(variant.name());
    if allow_async_host {
        command.arg("--allow-async-host");
    }
    if allow_agent_host {
        command.arg("--allow-agent-host");
    }
    let mut child = command
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
    if options.allow_agent_host && !is_exact_agent_host_test(&options.test, &source, &metadata)? {
        return Err(format!(
            "Test262 agent host worker rejected unaudited path: {}",
            options.test.display()
        ));
    }
    if metadata.is_module() || (metadata.is_async() && !options.allow_async_host) {
        return Err("unsupported test reached worker".to_owned());
    }
    let async_test = metadata.is_async();

    let runtime = Runtime::new();
    configure_runtime_can_block(&runtime, &metadata);
    let mut context = runtime.new_context();
    let mut agent_run = AgentRunGuard::new(options.allow_agent_host);
    install_worker_host(&runtime, &mut context, async_test, agent_run.session())?;
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
    if async_test {
        includes.push("doneprintHandle.js".to_owned());
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
    let result = match context.execute(&function) {
        Ok(_) if async_test => Ok(finish_async_test(&runtime, &mut context, &metadata)),
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
    };
    agent_run.finish()?;
    result
}

fn configure_runtime_can_block(runtime: &Runtime, metadata: &Metadata) {
    runtime.set_can_block(!metadata.flags.contains("CanBlockIsFalse"));
}

fn finish_async_test(
    runtime: &Runtime,
    context: &mut Context,
    metadata: &Metadata,
) -> WorkerResult {
    while runtime.is_job_pending() {
        match runtime.execute_pending_job() {
            Ok(true) => {}
            Ok(false) => {
                return WorkerResult::failure(
                    "async-job-invariant",
                    "async",
                    "Invariant",
                    "runtime reported a pending job but executed none",
                );
            }
            Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Unsupported => {
                return WorkerResult::failure(
                    "unsupported-runtime",
                    "async-job",
                    "Unsupported",
                    error.message(),
                );
            }
            Err(RuntimeError::Exception) => {
                let (error_type, detail) = take_error(runtime, context, RuntimeError::Exception);
                return classify_completion(metadata, "runtime", &error_type, &detail);
            }
            Err(error) => return engine_fault("engine-fault", "async-job", error, None),
        }
    }

    match read_worker_print_log(runtime, context) {
        Ok(messages) => classify_async_print_log(metadata, &messages),
        Err(error) => WorkerResult::failure("async-host-error", "async-host", "HostError", error),
    }
}

fn read_worker_print_log(runtime: &Runtime, context: &mut Context) -> Result<Vec<String>, String> {
    let global = context
        .global_object()
        .map_err(|error| worker_host_error(runtime, context, "read print global", error))?;
    let log_key = runtime
        .intern_property_key(WORKER_PRINT_LOG_PROPERTY)
        .map_err(|error| format!("intern Test262 print log key: {error}"))?;
    let log = match context
        .get_property(&global, &log_key)
        .map_err(|error| worker_host_error(runtime, context, "read print log", error))?
    {
        Value::Object(log) => log,
        _ => return Err("Test262 print log is not an object".to_owned()),
    };
    let length_key = runtime
        .intern_property_key("length")
        .map_err(|error| format!("intern Test262 print log length key: {error}"))?;
    let length = match context
        .get_property(&log, &length_key)
        .map_err(|error| worker_host_error(runtime, context, "read print log length", error))?
    {
        Value::Int(length) if length >= 0 => length as usize,
        Value::Float(length) if length.is_finite() && length >= 0.0 && length.fract() == 0.0 => {
            length as usize
        }
        _ => return Err("Test262 print log length is not a non-negative integer".to_owned()),
    };

    let mut messages = Vec::with_capacity(length);
    for index in 0..length {
        let key = runtime
            .intern_property_key(&index.to_string())
            .map_err(|error| format!("intern Test262 print log index {index}: {error}"))?;
        match context
            .get_property(&log, &key)
            .map_err(|error| worker_host_error(runtime, context, "read print log entry", error))?
        {
            Value::String(message) => messages.push(message.to_utf8_lossy()),
            _ => return Err(format!("Test262 print log entry {index} is not a string")),
        }
    }
    Ok(messages)
}

fn classify_async_print_log(metadata: &Metadata, messages: &[String]) -> WorkerResult {
    let reports = messages
        .iter()
        .filter(|message| {
            message.as_str() == ASYNC_COMPLETE || message.starts_with(ASYNC_FAILURE_PREFIX)
        })
        .collect::<Vec<_>>();
    if reports.len() != 1 {
        return WorkerResult::failure(
            "fail-async-done-count",
            "async",
            "TypeError",
            format!(
                "$DONE() must report exactly once; observed {} completion reports",
                reports.len()
            ),
        );
    }

    let report = reports[0];
    if report.as_str() == ASYNC_COMPLETE {
        return classify_normal(metadata);
    }
    let failure = report
        .strip_prefix(ASYNC_FAILURE_PREFIX)
        .expect("async report prefix was filtered");
    let (actual_type, detail) = failure
        .split_once(": ")
        .map_or(("Test262Error", failure), |(name, detail)| (name, detail));
    WorkerResult::failure("fail-async", "async", actual_type, detail)
}

fn install_worker_host(
    runtime: &Runtime,
    context: &mut Context,
    record_print: bool,
    agent_session: Option<&Test262AgentSession>,
) -> Result<(), String> {
    let options = CompileOptions::new(WORKER_HOST_FILENAME);
    let source = if record_print {
        ASYNC_WORKER_HOST_SOURCE
    } else {
        WORKER_HOST_SOURCE
    };
    let function = context
        .compile_with_options_preserving_unsupported_diagnostics(source, &options)
        .map_err(|error| worker_host_error(runtime, context, "compile", error))?;
    context
        .execute(&function)
        .map_err(|error| worker_host_error(runtime, context, "execute", error))?;
    let installed = match agent_session {
        Some(session) => context.install_test262_host_with_agent(session),
        None => context.install_test262_host(),
    };
    match installed {
        Ok(_) => Ok(()),
        Err(error) => Err(worker_host_error(runtime, context, "install $262", error)),
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

    use super::{
        classify_async_print_log, classify_completion, configure_runtime_can_block, run_worker,
    };
    use crate::metadata::{Metadata, NegativeExpectation};
    use crate::{Variant, WorkerOptions};

    #[test]
    fn worker_configures_can_block_from_test262_metadata() {
        for (flag, expected) in [
            (None, true),
            (Some("CanBlockIsTrue"), true),
            (Some("CanBlockIsFalse"), false),
        ] {
            let mut metadata = Metadata::default();
            if let Some(flag) = flag {
                metadata.flags.insert(flag.to_owned());
            }
            let runtime = Runtime::new();
            assert!(!runtime.can_block(), "runtime default changed for {flag:?}");

            configure_runtime_can_block(&runtime, &metadata);

            assert_eq!(runtime.can_block(), expected, "flag {flag:?}");
        }
    }

    #[test]
    fn agent_host_worker_flag_revalidates_path_and_source() {
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-agent-worker-revalidation-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let wrong_path = PathBuf::from("test/not-good-views.js");
        fs::create_dir_all(suite.join("test/built-ins/Atomics/wait")).unwrap();
        fs::write(
            suite.join(&wrong_path),
            "/*---\nincludes: [atomicsHelper.js]\nfeatures: [Atomics]\n---*/\n",
        )
        .unwrap();
        let error = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: wrong_path,
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: true,
        })
        .unwrap_err();
        assert!(error.contains("rejected unaudited path"), "{error}");

        let exact_path = PathBuf::from("test/built-ins/Atomics/wait/good-views.js");
        fs::write(
            suite.join(&exact_path),
            "/*---\nincludes: [atomicsHelper.js]\nfeatures: [Atomics]\n---*/\n// drift\n",
        )
        .unwrap();
        let error = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: exact_path,
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: true,
        })
        .unwrap_err();
        assert!(error.contains("source drifted"), "{error}");

        let broadcast_path =
            PathBuf::from("test/built-ins/Atomics/notify/notify-with-no-agents-waiting.js");
        fs::create_dir_all(suite.join("test/built-ins/Atomics/notify")).unwrap();
        fs::write(
            suite.join(&broadcast_path),
            "/*---\nincludes: [atomicsHelper.js]\nfeatures: [Atomics, SharedArrayBuffer, TypedArray]\n---*/\n// drift\n",
        )
        .unwrap();
        let error = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: broadcast_path,
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: true,
        })
        .unwrap_err();

        let bounded_wait_path =
            PathBuf::from("test/built-ins/Atomics/wait/true-for-timeout-agent.js");
        fs::write(
            suite.join(&bounded_wait_path),
            "/*---\nincludes: [atomicsHelper.js]\nfeatures: [Atomics, SharedArrayBuffer, TypedArray]\n---*/\n// drift\n",
        )
        .unwrap();
        let bounded_wait_error = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: bounded_wait_path,
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: true,
        })
        .unwrap_err();

        let wake_count_location_path =
            PathBuf::from("test/built-ins/Atomics/notify/bigint/notify-all-on-loc.js");
        fs::create_dir_all(suite.join("test/built-ins/Atomics/notify/bigint")).unwrap();
        fs::write(
            suite.join(&wake_count_location_path),
            "/*---\nincludes: [atomicsHelper.js]\nfeatures: [Atomics, BigInt, SharedArrayBuffer, TypedArray]\n---*/\n// drift\n",
        )
        .unwrap();
        let wake_count_location_error = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: wake_count_location_path,
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: true,
        })
        .unwrap_err();

        let fifo_wake_order_path =
            PathBuf::from("test/built-ins/Atomics/notify/notify-in-order.js");
        fs::write(
            suite.join(&fifo_wake_order_path),
            "/*---\nincludes: [atomicsHelper.js]\nfeatures: [Atomics, SharedArrayBuffer, TypedArray]\n---*/\n// drift\n",
        )
        .unwrap();
        let fifo_wake_order_error = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: fifo_wake_order_path,
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: true,
        })
        .unwrap_err();
        fs::remove_dir_all(suite).unwrap();
        assert!(
            error.contains("agent broadcast cohort A source drifted"),
            "{error}"
        );
        assert!(
            bounded_wait_error.contains("agent bounded wait cohort A source drifted"),
            "{bounded_wait_error}"
        );
        assert!(
            wake_count_location_error.contains("agent wake/count/location cohort source drifted"),
            "{wake_count_location_error}"
        );
        assert!(
            fifo_wake_order_error.contains("agent FIFO wake-order cohort source drifted"),
            "{fifo_wake_order_error}"
        );
    }

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
    fn async_completion_requires_exactly_one_done_report() {
        let metadata = Metadata::default();
        let complete = classify_async_print_log(
            &metadata,
            &[
                "ordinary output".to_owned(),
                "Test262:AsyncTestComplete".to_owned(),
            ],
        );
        assert_eq!(complete.outcome, "pass");

        for messages in [
            Vec::<String>::new(),
            vec![
                "Test262:AsyncTestComplete".to_owned(),
                "Test262:AsyncTestComplete".to_owned(),
            ],
        ] {
            let result = classify_async_print_log(&metadata, &messages);
            assert_eq!(result.outcome, "fail-async-done-count");
            assert_eq!(result.actual_type, "TypeError");
        }
    }

    #[test]
    fn async_done_failure_preserves_type_and_message() {
        let result = classify_async_print_log(
            &Metadata::default(),
            &["Test262:AsyncTestFailure:Test262Error: sentinel".to_owned()],
        );
        assert_eq!(result.outcome, "fail-async");
        assert_eq!(result.actual_phase, "async");
        assert_eq!(result.actual_type, "Test262Error");
        assert_eq!(result.detail, "sentinel");
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
            allow_async_host: false,
            allow_agent_host: false,
        })
        .unwrap();
        fs::remove_dir_all(suite).unwrap();

        assert_eq!(result.outcome, "fail-missing-throw");
        assert_eq!(result.actual_phase, "normal");
        assert!(result.detail.contains("expected SyntaxError during parse"));
    }

    #[test]
    fn async_worker_host_is_explicit_and_requires_one_done_report() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-test262-async-host-{}-{unique}",
            std::process::id()
        ));
        let relative = PathBuf::from("test/async-host.js");
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::create_dir_all(suite.join("harness")).unwrap();
        fs::write(
            suite.join("harness/doneprintHandle.js"),
            "function $DONE(error) { print(error ? 'Test262:AsyncTestFailure:Test262Error: failed' : 'Test262:AsyncTestComplete'); }\n",
        )
        .unwrap();
        fs::write(
            suite.join(&relative),
            "/*---\nflags: [async, raw]\n---*/\n$DONE();\n",
        )
        .unwrap();

        let denied = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative.clone(),
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: false,
        })
        .unwrap_err();
        assert_eq!(denied, "unsupported test reached worker");

        let result = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            variant: Variant::Sloppy,
            allow_async_host: true,
            allow_agent_host: false,
        })
        .unwrap();
        fs::remove_dir_all(suite).unwrap();

        assert_eq!(result.outcome, "pass", "{}", result.detail);
    }

    #[test]
    fn async_worker_print_scans_all_arguments_and_keeps_its_log_private() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-test262-async-print-{}-{unique}",
            std::process::id()
        ));
        let relative = PathBuf::from("test/async-print.js");
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::create_dir_all(suite.join("harness")).unwrap();
        fs::write(suite.join("harness/doneprintHandle.js"), "").unwrap();
        fs::write(
            suite.join(&relative),
            "/*---\nflags: [async, raw]\n---*/\nvar copy = globalThis.__quickjs_oxide_test262_print_log__;\ncopy.length = 0;\nprint('ordinary output', 'Test262:AsyncTestComplete');\n",
        )
        .unwrap();

        let result = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            variant: Variant::Sloppy,
            allow_async_host: true,
            allow_agent_host: false,
        })
        .unwrap();
        fs::remove_dir_all(suite).unwrap();

        assert_eq!(result.outcome, "pass", "{}", result.detail);
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
            allow_async_host: false,
            allow_agent_host: false,
        })
        .unwrap();
        fs::remove_dir_all(suite).unwrap();

        assert_eq!(result.outcome, "pass", "{}", result.detail);
        assert_eq!(result.actual_phase, "normal");
    }

    #[test]
    fn raw_worker_installs_quickjs_test262_hosts() {
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
if (typeof $262 !== "object" ||
    typeof $262.createRealm !== "function" ||
    typeof $262.detachArrayBuffer !== "function" ||
    typeof $262.evalScript !== "function" ||
    typeof $262.codePointRange !== "function" ||
    typeof $262.gc !== "function" ||
    $262.global !== globalThis) {
    throw new Error("QuickJS Test262 host surface is missing");
}
var globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "$262");
var createRealmDescriptor = Object.getOwnPropertyDescriptor($262, "createRealm");
var detachDescriptor = Object.getOwnPropertyDescriptor($262, "detachArrayBuffer");
var evalScriptDescriptor = Object.getOwnPropertyDescriptor($262, "evalScript");
var helperDescriptor = Object.getOwnPropertyDescriptor($262, "codePointRange");
var isHTMLDDADescriptor = Object.getOwnPropertyDescriptor($262, "IsHTMLDDA");
var gcDescriptor = Object.getOwnPropertyDescriptor($262, "gc");
var hostGlobalDescriptor = Object.getOwnPropertyDescriptor($262, "global");
if (!globalDescriptor.writable || !globalDescriptor.enumerable ||
    !globalDescriptor.configurable || !createRealmDescriptor.writable ||
    !createRealmDescriptor.enumerable || !createRealmDescriptor.configurable ||
    !detachDescriptor.writable ||
    !detachDescriptor.enumerable || !detachDescriptor.configurable ||
    !evalScriptDescriptor.writable || !evalScriptDescriptor.enumerable ||
    !evalScriptDescriptor.configurable ||
    !helperDescriptor.writable ||
    !helperDescriptor.enumerable || !helperDescriptor.configurable ||
    !isHTMLDDADescriptor.writable || !isHTMLDDADescriptor.enumerable ||
    !isHTMLDDADescriptor.configurable ||
    !gcDescriptor.writable || !gcDescriptor.enumerable ||
    !gcDescriptor.configurable || !hostGlobalDescriptor.writable ||
    !hostGlobalDescriptor.enumerable || !hostGlobalDescriptor.configurable) {
    throw new Error("QuickJS host property flags changed");
}
if ($262.createRealm.name !== "createRealm" ||
    $262.createRealm.length !== 0 ||
    Object.getPrototypeOf($262.createRealm) !== Function.prototype) {
    throw new Error("QuickJS createRealm function metadata changed");
}
if ($262.detachArrayBuffer.name !== "detachArrayBuffer" ||
    $262.detachArrayBuffer.length !== 1 ||
    Object.getPrototypeOf($262.detachArrayBuffer) !== Function.prototype) {
    throw new Error("QuickJS detachArrayBuffer function metadata changed");
}
if ($262.evalScript.name !== "evalScript" || $262.evalScript.length !== 1 ||
    Object.getPrototypeOf($262.evalScript) !== Function.prototype) {
    throw new Error("QuickJS evalScript function metadata changed");
}
if ($262.codePointRange.name !== "codePointRange" ||
    $262.codePointRange.length !== 2 ||
    Object.getPrototypeOf($262.codePointRange) !== Function.prototype) {
    throw new Error("QuickJS codePointRange function metadata changed");
}
var htmlDDA = isHTMLDDADescriptor.value;
if (htmlDDA !== $262.IsHTMLDDA || htmlDDA.name !== "IsHTMLDDA" ||
    htmlDDA.length !== 0 ||
    Object.getPrototypeOf(htmlDDA) !== Function.prototype ||
    typeof htmlDDA !== "undefined" || Boolean(htmlDDA) || !htmlDDA !== true ||
    (htmlDDA == null) !== true || (null == htmlDDA) !== true ||
    (htmlDDA == undefined) !== true || (undefined == htmlDDA) !== true ||
    htmlDDA === null || htmlDDA === undefined ||
    Object.is(htmlDDA, undefined) || htmlDDA !== htmlDDA ||
    (htmlDDA ?? 42) !== htmlDDA || htmlDDA() !== null) {
    throw new Error("QuickJS IsHTMLDDA semantics changed");
}
var boundHTMLDDA = htmlDDA.bind(null);
var proxyHTMLDDA = new Proxy(htmlDDA, {});
if (typeof boundHTMLDDA !== "function" || !Boolean(boundHTMLDDA) ||
    boundHTMLDDA() !== null || typeof proxyHTMLDDA !== "function" ||
    !Boolean(proxyHTMLDDA) || proxyHTMLDDA() !== null) {
    throw new Error("QuickJS IsHTMLDDA marker propagated to a new object");
}
if ($262.gc.name !== "gc" || $262.gc.length !== 0 ||
    Object.getPrototypeOf($262.gc) !== Function.prototype) {
    throw new Error("QuickJS gc function metadata changed");
}
var constructorThrew = false;
try {
    new $262.createRealm();
} catch (error) {
    constructorThrew = error instanceof TypeError;
}
if (!constructorThrew) {
    throw new Error("QuickJS createRealm became constructible");
}
constructorThrew = false;
try {
    new $262.codePointRange(0, 1);
} catch (error) {
    constructorThrew = error instanceof TypeError;
}
if (!constructorThrew) {
    throw new Error("QuickJS codePointRange became constructible");
}
constructorThrew = false;
try {
    new $262.detachArrayBuffer(null);
} catch (error) {
    constructorThrew = error instanceof TypeError;
}
if (!constructorThrew) {
    throw new Error("QuickJS detachArrayBuffer became constructible");
}
constructorThrew = false;
try {
    new $262.evalScript("0");
} catch (error) {
    constructorThrew = error instanceof TypeError;
}
if (!constructorThrew) {
    throw new Error("QuickJS evalScript became constructible");
}
constructorThrew = false;
try {
    new htmlDDA();
} catch (error) {
    constructorThrew = error instanceof TypeError;
}
if (!constructorThrew) {
    throw new Error("QuickJS IsHTMLDDA became constructible");
}
constructorThrew = false;
try {
    new $262.gc();
} catch (error) {
    constructorThrew = error instanceof TypeError;
}
if (!constructorThrew) {
    throw new Error("QuickJS gc became constructible");
}

var activeResult = (function activeFrame() {
    var local = { answer: 42 };
    activeFrame = null;
    if ($262.gc.call(local, local, "ignored") !== undefined) {
        throw new Error("QuickJS gc return value changed");
    }
    return local.answer;
})();
if (activeResult !== 42) {
    throw new Error("QuickJS gc released an active frame root");
}

var parentJobRan = false;
Promise.resolve().then(function() { parentJobRan = true; });
var child = $262.createRealm();
if (parentJobRan) {
    throw new Error("QuickJS createRealm drained the parent job queue");
}
if (typeof child !== "object" || child === $262 ||
    child.global === globalThis || child.global === $262.global ||
    child.global.Object === Object || child.global.Function === Function ||
    child.global.TypeError === TypeError || child.global.Promise === Promise) {
    throw new Error("QuickJS createRealm did not create distinct realm intrinsics");
}
if (child.global.$262 !== child || child.global.$262.global !== child.global ||
    child.global.globalThis !== child.global ||
    Object.getPrototypeOf(child) !== child.global.Object.prototype) {
    throw new Error("QuickJS child $262/global relationship changed");
}
if (child.createRealm.name !== "createRealm" || child.createRealm.length !== 0 ||
    Object.getPrototypeOf(child.createRealm) !== child.global.Function.prototype ||
    child.evalScript.name !== "evalScript" || child.evalScript.length !== 1 ||
    Object.getPrototypeOf(child.evalScript) !== child.global.Function.prototype ||
    child.IsHTMLDDA === htmlDDA || typeof child.IsHTMLDDA !== "undefined" ||
    child.IsHTMLDDA.name !== "IsHTMLDDA" || child.IsHTMLDDA.length !== 0 ||
    Object.getPrototypeOf(child.IsHTMLDDA) !== child.global.Function.prototype ||
    child.IsHTMLDDA() !== null) {
    throw new Error("QuickJS child host function realm or metadata changed");
}

if ($262.evalScript("21 * 2") !== 42 ||
    $262.evalScript("globalThis") !== globalThis ||
    child.evalScript.call($262, "21 * 2") !== 42 ||
    child.evalScript.call($262, "globalThis") !== child.global) {
    throw new Error("QuickJS evalScript completion or defining realm changed");
}
var childObject = child.evalScript("({ answer: 42 })");
if (childObject.answer !== 42 || childObject instanceof Object ||
    !(childObject instanceof child.global.Object) ||
    Object.getPrototypeOf(childObject) !== child.global.Object.prototype) {
    throw new Error("QuickJS evalScript returned an object from the wrong realm");
}
var childError;
try {
    child.evalScript.call($262, "throw new TypeError('child realm')");
} catch (error) {
    childError = error;
}
if (!childError || childError instanceof TypeError ||
    !(childError instanceof child.global.TypeError) ||
    Object.getPrototypeOf(childError) !== child.global.TypeError.prototype) {
    throw new Error("QuickJS evalScript threw an error from the wrong realm");
}

child.evalScript(
    "globalThis.childJobRan = false; " +
    "Promise.resolve().then(function() { childJobRan = true; });"
);
var grandchild = child.createRealm();
if (grandchild === child || grandchild.global === child.global ||
    grandchild.global.$262 !== grandchild ||
    grandchild.global.$262.global !== grandchild.global ||
    Object.getPrototypeOf(grandchild.createRealm) !==
        grandchild.global.Function.prototype ||
    Object.getPrototypeOf(grandchild.evalScript) !==
        grandchild.global.Function.prototype) {
    throw new Error("QuickJS recursive createRealm host installation changed");
}
$262.gc();
child.gc();
if (parentJobRan || child.global.childJobRan) {
    throw new Error("QuickJS host hooks drained active Promise jobs");
}

var ordinary = {};
if ($262.detachArrayBuffer() !== undefined ||
    $262.detachArrayBuffer(ordinary, "ignored") !== undefined ||
    ordinary.detached !== undefined) {
    throw new Error("QuickJS detachArrayBuffer non-buffer behavior changed");
}
var buffer = new ArrayBuffer(4, { maxByteLength: 8 });
if (buffer.byteLength !== 4 || buffer.maxByteLength !== 8 ||
    !buffer.resizable || buffer.detached) {
    throw new Error("ArrayBuffer pre-detach state changed");
}
if ($262.detachArrayBuffer.call(ordinary, buffer) !== undefined ||
    buffer.byteLength !== 0 || buffer.maxByteLength !== 8 ||
    !buffer.resizable || !buffer.detached) {
    throw new Error("QuickJS detachArrayBuffer state transition changed");
}
if ($262.detachArrayBuffer(buffer) !== undefined || !buffer.detached) {
    throw new Error("QuickJS detachArrayBuffer is no longer idempotent");
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
            allow_async_host: false,
            allow_agent_host: false,
        })
        .unwrap();
        fs::remove_dir_all(suite).unwrap();

        assert_eq!(result.outcome, "pass", "{}", result.detail);
        assert_eq!(result.actual_phase, "normal");
    }

    #[test]
    fn unsupported_parser_provenance_is_opt_in_at_the_context_boundary() {
        const UNSUPPORTED_SOURCE: &str = "import('fixture');";

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
        assert_eq!(error.message(), "import syntax is not implemented yet");
        assert!(context.take_exception().unwrap().is_none());
    }
}
