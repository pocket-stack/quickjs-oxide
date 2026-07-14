use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use quickjs_oxide::{CompileOptions, Context, ErrorKind, Runtime, RuntimeError, Value};

use super::metadata::{Metadata, parse_metadata};
use super::report::WorkerResult;
use super::{Variant, WorkerOptions, validate_relative_test_path};

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
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "worker stdout pipe was missing".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "worker stderr pipe was missing".to_owned())?;
    let stdout_reader = spawn_pipe_reader(stdout, "stdout");
    let stderr_reader = spawn_pipe_reader(stderr, "stderr");
    let started = Instant::now();
    let status = loop {
        match child
            .try_wait()
            .map_err(|error| format!("wait for {}: {error}", test.display()))?
        {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Ok(WorkerResult::failure(
                    "timeout",
                    "host",
                    "",
                    format!("worker exceeded {} ms", timeout.as_millis()),
                ));
            }
            None => thread::sleep(Duration::from_millis(5)),
        }
    };
    let stdout = join_pipe_reader(stdout_reader, "stdout")?;
    let stderr = join_pipe_reader(stderr_reader, "stderr")?;
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
    // The progress baseline follows the pinned Test262 interpretation rather
    // than run-test262.c's raw-test deviation: raw means no harness and no
    // source rewriting. Harness files remain separate scripts and keep their
    // own filenames.
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
            Err(error) => {
                let (error_type, detail) = take_error(&runtime, &mut context, error);
                return Ok(WorkerResult::failure(
                    "harness-error",
                    "harness",
                    error_type,
                    format!("{include}: {detail}"),
                ));
            }
        };
        if let Err(error) = context.execute(&function) {
            let (error_type, detail) = take_error(&runtime, &mut context, error);
            return Ok(WorkerResult::failure(
                "harness-error",
                "harness",
                error_type,
                format!("{include}: {detail}"),
            ));
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
        Err(error) => {
            let (error_type, detail) = take_error(&runtime, &mut context, error);
            return Ok(classify_completion(
                &metadata,
                "parse",
                &error_type,
                &detail,
            ));
        }
    };
    match context.execute(&function) {
        Ok(_) => Ok(classify_normal(&metadata)),
        Err(error) => {
            let (error_type, detail) = take_error(&runtime, &mut context, error);
            Ok(classify_completion(
                &metadata,
                "runtime",
                &error_type,
                &detail,
            ))
        }
    }
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
    use quickjs_oxide::{CompileOptions, ErrorKind, Runtime, RuntimeError};

    use super::classify_completion;
    use crate::metadata::{Metadata, NegativeExpectation};

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
    fn unsupported_parser_provenance_is_opt_in_at_the_context_boundary() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context.compile("with ({}) {}").unwrap_err(),
            RuntimeError::Exception
        );
        assert!(context.take_exception().unwrap().is_some());

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let options = CompileOptions::new("unsupported.js");
        let RuntimeError::Engine(error) = context
            .compile_with_options_preserving_unsupported_diagnostics("with ({}) {}", &options)
            .unwrap_err()
        else {
            panic!("diagnostic compile did not retain its engine error");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(context.take_exception().unwrap().is_none());
    }
}
