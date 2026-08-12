use std::cell::Cell;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use quickjs_oxide::{
    CompileOptions, CompleteOrdinaryPropertyDescriptor, Context, ErrorKind, JsString,
    ModuleImportAttributes, ModuleLoadResult, ModuleLoader, ModuleLoaderError, ObjectRef, Runtime,
    RuntimeError, Test262AgentSession, Value,
};

use super::admissions::{AdmissionCatalog, DynamicImportBytecodeExpectation, ModuleGraphRootGoal};
use super::metadata::{Metadata, parse_metadata};
use super::report::WorkerResult;
use super::requirements::{
    ExactModuleTest, HostCapabilities, exact_module_test, is_exact_agent_host_test,
    is_exact_dynamic_import_script_test, load_exact_dynamic_import_fixture,
    load_exact_module_fixture, normalize_exact_dynamic_import_request,
    normalize_exact_module_request,
};
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExceptionDiagnostic {
    error_type: String,
    message: String,
    line: Option<u32>,
    column: Option<u32>,
}

impl ExceptionDiagnostic {
    fn engine(message: impl Into<String>) -> Self {
        Self {
            error_type: "EngineError".to_owned(),
            message: message.into(),
            line: None,
            column: None,
        }
    }
}

#[derive(Debug)]
struct ExactTest262ModuleLoader {
    admissions: Rc<AdmissionCatalog>,
    suite: PathBuf,
    root: PathBuf,
    goal: ModuleGraphRootGoal,
    resolution_started: Rc<Cell<bool>>,
}

struct ExactModuleRun<'a> {
    path: &'a Path,
    relative_path: &'a Path,
    source: &'a str,
    metadata: &'a Metadata,
    exact_module: ExactModuleTest,
    resolution_started: &'a Cell<bool>,
}

impl ModuleLoader for ExactTest262ModuleLoader {
    fn normalize(
        &self,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        self.resolution_started.set(true);
        let base_name = exact_module_utf8_name(base_name)?;
        let specifier = exact_module_utf8_name(specifier)?;
        let normalized = match self.goal {
            ModuleGraphRootGoal::StaticModule => {
                normalize_exact_module_request(&self.admissions, &self.root, &base_name, &specifier)
            }
            ModuleGraphRootGoal::DynamicImportScript => normalize_exact_dynamic_import_request(
                &self.admissions,
                &self.root,
                &base_name,
                &specifier,
            ),
        }
        .map_err(ModuleLoaderError::new)?;
        JsString::try_from_utf8(&normalized)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        self.resolution_started.set(true);
        let normalized_name = exact_module_utf8_name(normalized_name)?;
        match self.goal {
            ModuleGraphRootGoal::StaticModule => load_exact_module_fixture(
                &self.admissions,
                &self.suite,
                &self.root,
                &normalized_name,
            ),
            ModuleGraphRootGoal::DynamicImportScript => load_exact_dynamic_import_fixture(
                &self.admissions,
                &self.suite,
                &self.root,
                &normalized_name,
            ),
        }
        .map_err(ModuleLoaderError::new)
    }

    fn load_with_attributes(
        &self,
        normalized_name: &JsString,
        attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let source = self.load(normalized_name)?;
        if exact_module_requests_json(attributes) {
            Ok(ModuleLoadResult::JsonText(source))
        } else {
            Ok(ModuleLoadResult::SourceText(source))
        }
    }
}

fn exact_module_requests_json(attributes: &ModuleImportAttributes) -> bool {
    attributes.effective().is_some_and(|attributes| {
        attributes.iter().any(|attribute| {
            exact_module_string_equals(&attribute.key, "type")
                && exact_module_string_equals(&attribute.value, "json")
        })
    })
}

fn exact_module_string_equals(value: &JsString, expected: &str) -> bool {
    value.utf16_units().eq(expected.encode_utf16())
}

fn exact_module_utf8_name(name: &JsString) -> Result<String, ModuleLoaderError> {
    String::from_utf16(&name.utf16_units().collect::<Vec<_>>())
        .map_err(|_| ModuleLoaderError::new("Test262 module name is not valid UTF-16"))
}

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

pub(super) struct IsolatedWorkerOptions<'a> {
    pub(super) executable: &'a Path,
    pub(super) suite: &'a Path,
    pub(super) test: &'a Path,
    pub(super) admissions: &'a Path,
    pub(super) admissions_sha256: &'a str,
    pub(super) variant: Variant,
    pub(super) timeout: Duration,
    pub(super) allow_async_host: bool,
    pub(super) allow_agent_host: bool,
}

pub(super) fn run_isolated_worker(
    options: IsolatedWorkerOptions<'_>,
) -> Result<WorkerResult, String> {
    let IsolatedWorkerOptions {
        executable,
        suite,
        test,
        admissions,
        admissions_sha256,
        variant,
        timeout,
        allow_async_host,
        allow_agent_host,
    } = options;
    let mut command = Command::new(executable);
    command
        .arg("--worker-one")
        .arg("--suite")
        .arg(suite)
        .arg("--test")
        .arg(test)
        .arg("--admissions")
        .arg(admissions)
        .arg("--admissions-sha256")
        .arg(admissions_sha256)
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
    let admissions = Rc::new(AdmissionCatalog::load(
        &options.admissions,
        &options.admissions_sha256,
    )?);
    let path = options.suite.join(&options.test);
    let source =
        fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let metadata = parse_metadata(&source)?;
    let exact_module = exact_module_test(
        &admissions,
        &options.suite,
        &options.test,
        &source,
        &metadata,
    )?;
    let exact_dynamic_import = is_exact_dynamic_import_script_test(
        &admissions,
        &options.suite,
        &options.test,
        &source,
        &metadata,
    )?;
    let dynamic_import_expectation = if exact_dynamic_import {
        Some(
            admissions
                .dynamic_import_root(&options.test)
                .and_then(|root| root.dynamic_import_expectation)
                .ok_or_else(|| {
                    format!(
                        "authenticated dynamic import root has no bytecode expectation: {}",
                        options.test.display()
                    )
                })?,
        )
    } else {
        None
    };
    if options.allow_agent_host
        && !is_exact_agent_host_test(&admissions, &options.test, &source, &metadata)?
    {
        return Err(format!(
            "Test262 agent host worker rejected unaudited path: {}",
            options.test.display()
        ));
    }
    if (metadata.is_module() && exact_module.is_none())
        || (metadata.is_async() && !options.allow_async_host)
    {
        return Err("unsupported test reached worker".to_owned());
    }
    let async_test = metadata.is_async();

    let runtime = Runtime::new();
    runtime.set_dynamic_import_bytecode_allowed(false);
    let module_resolution_started = Rc::new(Cell::new(false));
    let graph_loader_goal = if exact_module == Some(ExactModuleTest::FixtureGraph) {
        Some(ModuleGraphRootGoal::StaticModule)
    } else if exact_dynamic_import {
        Some(ModuleGraphRootGoal::DynamicImportScript)
    } else {
        None
    };
    let _module_loader_registration = graph_loader_goal.map(|goal| {
        runtime.set_module_loader(ExactTest262ModuleLoader {
            admissions: admissions.clone(),
            suite: options.suite.clone(),
            root: options.test.clone(),
            goal,
            resolution_started: module_resolution_started.clone(),
        })
    });
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
        let function = match context.compile_with_options(&harness, &compile_options) {
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
                let diagnostic = take_error(&runtime, &mut context, RuntimeError::Exception);
                return Ok(WorkerResult::failure_with_diagnostic(
                    "harness-error",
                    "harness",
                    diagnostic.error_type,
                    format!("{include}: {}", diagnostic.message),
                    diagnostic.line,
                    diagnostic.column,
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
                    let diagnostic = take_error(&runtime, &mut context, RuntimeError::Exception);
                    WorkerResult::failure_with_diagnostic(
                        "harness-error",
                        "harness-runtime",
                        diagnostic.error_type,
                        format!("{include}: {}", diagnostic.message),
                        diagnostic.line,
                        diagnostic.column,
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

    if let Some(exact_module) = exact_module {
        let result = run_exact_module(
            &runtime,
            &mut context,
            ExactModuleRun {
                path: &path,
                relative_path: &options.test,
                source: &source,
                metadata: &metadata,
                exact_module,
                resolution_started: &module_resolution_started,
            },
        );
        agent_run.finish()?;
        return Ok(result);
    }

    let authored = if options.variant == Variant::Strict {
        format!("\"use strict\";\n{source}")
    } else {
        source
    };
    let filename = if exact_dynamic_import {
        options.test.to_string_lossy()
    } else {
        path.to_string_lossy()
    };
    let compile_options = CompileOptions::new(filename.as_ref());
    // Initial Script publication is host-side and executes no JavaScript.
    // Open the capability only for this compile so the immutable tree can be
    // inspected below, then close it again before any authored code runs.
    runtime.set_dynamic_import_bytecode_allowed(true);
    let compilation = context.compile_with_options(&authored, &compile_options);
    runtime.set_dynamic_import_bytecode_allowed(false);
    let function = match compilation {
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
            let diagnostic = take_error(&runtime, &mut context, RuntimeError::Exception);
            return Ok(classify_completion(&metadata, "parse", &diagnostic));
        }
        Err(error) => return Ok(engine_fault("engine-fault", "parse", error, None)),
    };
    authenticate_dynamic_import_bytecode(
        &runtime,
        &function,
        dynamic_import_expectation,
        &options.test,
    )?;
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
            let diagnostic = take_error(&runtime, &mut context, RuntimeError::Exception);
            Ok(classify_completion(&metadata, "runtime", &diagnostic))
        }
        Err(error) => Ok(engine_fault("engine-fault", "runtime", error, None)),
    };
    agent_run.finish()?;
    result
}

fn authenticate_dynamic_import_bytecode(
    runtime: &Runtime,
    function: &quickjs_oxide::FunctionBytecodeRef,
    expectation: Option<DynamicImportBytecodeExpectation>,
    path: &Path,
) -> Result<(), String> {
    let contains_dynamic_import = runtime
        .bytecode_tree_contains_dynamic_import(function)
        .map_err(|error| {
            format!(
                "inspect Test262 Script bytecode for {}: {error}",
                path.display()
            )
        })?;
    match (contains_dynamic_import, expectation) {
        (true, None) => Err(format!(
            "Test262 dynamic-import worker rejected unaudited path: {}",
            path.display()
        )),
        (false, Some(DynamicImportBytecodeExpectation::InitialImportTree)) => Err(format!(
            "authenticated Test262 dynamic-import root compiled without dynamic-import bytecode: {}",
            path.display()
        )),
        (true, Some(DynamicImportBytecodeExpectation::RuntimeCompiledImport)) => Err(format!(
            "runtime-compiled Test262 dynamic-import root unexpectedly contained initial dynamic-import bytecode: {}",
            path.display()
        )),
        (true, Some(DynamicImportBytecodeExpectation::InitialImportTree))
        | (false, Some(DynamicImportBytecodeExpectation::RuntimeCompiledImport)) => {
            runtime.set_dynamic_import_bytecode_allowed(true);
            Ok(())
        }
        (false, None) => Ok(()),
    }
}

fn run_exact_module(
    runtime: &Runtime,
    context: &mut Context,
    run: ExactModuleRun<'_>,
) -> WorkerResult {
    let ExactModuleRun {
        path,
        relative_path,
        source,
        metadata,
        exact_module,
        resolution_started,
    } = run;
    let filename = if exact_module == ExactModuleTest::FixtureGraph {
        relative_path.to_string_lossy()
    } else {
        path.to_string_lossy()
    };
    let compile_options = CompileOptions::new(filename.as_ref());
    let module = match context.compile_module_with_options(source, &compile_options) {
        Ok(module) => module,
        Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Unsupported => {
            let phase = module_compile_failure_phase(exact_module, resolution_started);
            return WorkerResult::failure(
                if phase == "resolution" {
                    "unsupported-resolution"
                } else {
                    "unsupported-parser"
                },
                phase,
                "Unsupported",
                error.message(),
            );
        }
        Err(RuntimeError::Exception) => {
            let diagnostic = take_error(runtime, context, RuntimeError::Exception);
            return classify_completion(
                metadata,
                module_compile_failure_phase(exact_module, resolution_started),
                &diagnostic,
            );
        }
        Err(error) => {
            return engine_fault(
                "engine-fault",
                module_compile_failure_phase(exact_module, resolution_started),
                error,
                None,
            );
        }
    };
    if metadata
        .negative
        .as_ref()
        .and_then(|negative| negative.phase.as_deref())
        .is_some_and(|phase| matches!(phase, "parse" | "early"))
    {
        return classify_normal(metadata);
    }
    match context.link_module(&module) {
        Ok(()) => {}
        Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Unsupported => {
            return WorkerResult::failure(
                "unsupported-resolution",
                "resolution",
                "Unsupported",
                error.message(),
            );
        }
        Err(RuntimeError::Exception) => {
            let diagnostic = take_error(runtime, context, RuntimeError::Exception);
            return classify_completion(metadata, "resolution", &diagnostic);
        }
        Err(error) => return engine_fault("engine-fault", "resolution", error, None),
    }
    if metadata
        .negative
        .as_ref()
        .and_then(|negative| negative.phase.as_deref())
        == Some("resolution")
    {
        return classify_normal(metadata);
    }
    match context.execute_module(&module) {
        Ok(_) => classify_normal(metadata),
        Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Unsupported => {
            WorkerResult::failure(
                "unsupported-runtime",
                "runtime",
                "Unsupported",
                error.message(),
            )
        }
        Err(RuntimeError::Exception) => {
            let diagnostic = take_error(runtime, context, RuntimeError::Exception);
            classify_completion(metadata, "runtime", &diagnostic)
        }
        Err(error) => engine_fault("engine-fault", "runtime", error, None),
    }
}

fn module_compile_failure_phase(
    exact_module: ExactModuleTest,
    resolution_started: &Cell<bool>,
) -> &'static str {
    if exact_module == ExactModuleTest::FixtureGraph && resolution_started.get() {
        "resolution"
    } else {
        "parse"
    }
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
                let diagnostic = take_error(runtime, context, RuntimeError::Exception);
                return classify_completion(metadata, "runtime", &diagnostic);
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
        .compile_with_options(source, &options)
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
        let diagnostic = take_error(runtime, context, error);
        format!(
            "Test262 worker host {phase} threw {}: {}",
            diagnostic.error_type, diagnostic.message
        )
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
    diagnostic: &ExceptionDiagnostic,
) -> WorkerResult {
    let Some(negative) = &metadata.negative else {
        return WorkerResult::failure_with_diagnostic(
            format!("fail-{actual_phase}"),
            actual_phase,
            &diagnostic.error_type,
            &diagnostic.message,
            diagnostic.line,
            diagnostic.column,
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
        .is_none_or(|expected| expected == diagnostic.error_type);
    if phase_matches && type_matches {
        WorkerResult::pass_with_diagnostic(
            actual_phase,
            &diagnostic.error_type,
            &diagnostic.message,
            diagnostic.line,
            diagnostic.column,
        )
    } else {
        WorkerResult::failure_with_diagnostic(
            "fail-negative-mismatch",
            actual_phase,
            &diagnostic.error_type,
            &diagnostic.message,
            diagnostic.line,
            diagnostic.column,
        )
    }
}

fn take_error(
    runtime: &Runtime,
    context: &mut Context,
    error: RuntimeError,
) -> ExceptionDiagnostic {
    if error != RuntimeError::Exception {
        return ExceptionDiagnostic::engine(error.to_string());
    }
    let exception = match context.take_exception() {
        Ok(Some(exception)) => exception,
        Ok(None) => {
            return ExceptionDiagnostic {
                error_type: "MissingException".to_owned(),
                message: "pending exception was empty".to_owned(),
                line: None,
                column: None,
            };
        }
        Err(error) => return ExceptionDiagnostic::engine(error.to_string()),
    };
    exception_diagnostic(runtime, exception)
}

fn exception_diagnostic(runtime: &Runtime, exception: Value) -> ExceptionDiagnostic {
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
        return ExceptionDiagnostic {
            error_type: format!("Thrown{kind}"),
            message: primitive_text(exception),
            line: None,
            column: None,
        };
    };

    let error_type = match side_effect_free_error_name(runtime, &object) {
        Ok(Some(value)) => value,
        Ok(None) => "ThrownObject".to_owned(),
        Err(error) => return ExceptionDiagnostic::engine(error.to_string()),
    };
    let message = match own_string_property(runtime, &object, "message") {
        Ok(Some(value)) => value,
        Ok(None) => String::new(),
        Err(error) => return ExceptionDiagnostic::engine(error.to_string()),
    };
    let line = match own_positive_u32_property(runtime, &object, "lineNumber") {
        Ok(value) => value,
        Err(error) => return ExceptionDiagnostic::engine(error.to_string()),
    };
    let column = match own_positive_u32_property(runtime, &object, "columnNumber") {
        Ok(value) => value,
        Err(error) => return ExceptionDiagnostic::engine(error.to_string()),
    };
    ExceptionDiagnostic {
        error_type,
        message,
        line,
        column,
    }
}

enum OwnDiagnosticString {
    Missing,
    String(String),
    Other,
}

fn side_effect_free_error_name(
    runtime: &Runtime,
    object: &ObjectRef,
) -> Result<Option<String>, RuntimeError> {
    let name_key = runtime.intern_property_key("name")?;
    match own_diagnostic_string(runtime, object, &name_key)? {
        OwnDiagnosticString::String(name) if !name.is_empty() => return Ok(Some(name)),
        OwnDiagnosticString::String(_) | OwnDiagnosticString::Other => return Ok(None),
        OwnDiagnosticString::Missing => {}
    }

    let Some(prototype) = runtime.get_prototype_of(object)? else {
        return Ok(None);
    };
    match own_diagnostic_string(runtime, &prototype, &name_key)? {
        OwnDiagnosticString::String(name) if !name.is_empty() => return Ok(Some(name)),
        OwnDiagnosticString::String(_) | OwnDiagnosticString::Other => return Ok(None),
        OwnDiagnosticString::Missing => {}
    }

    let constructor_key = runtime.intern_property_key("constructor")?;
    let constructor = match runtime.get_own_property(&prototype, &constructor_key)? {
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(constructor),
            ..
        }) => constructor,
        Some(CompleteOrdinaryPropertyDescriptor::Data { .. })
        | Some(CompleteOrdinaryPropertyDescriptor::Accessor { .. })
        | None => return Ok(None),
    };
    Ok(
        match own_diagnostic_string(runtime, &constructor, &name_key)? {
            OwnDiagnosticString::String(name) if !name.is_empty() && name != "Object" => Some(name),
            OwnDiagnosticString::Missing
            | OwnDiagnosticString::String(_)
            | OwnDiagnosticString::Other => None,
        },
    )
}

fn own_diagnostic_string(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &quickjs_oxide::PropertyKey,
) -> Result<OwnDiagnosticString, RuntimeError> {
    Ok(match runtime.get_own_property(object, key)? {
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) => OwnDiagnosticString::String(value.to_utf8_lossy()),
        Some(CompleteOrdinaryPropertyDescriptor::Data { .. })
        | Some(CompleteOrdinaryPropertyDescriptor::Accessor { .. }) => OwnDiagnosticString::Other,
        None => OwnDiagnosticString::Missing,
    })
}

fn own_string_property(
    runtime: &Runtime,
    object: &ObjectRef,
    name: &str,
) -> Result<Option<String>, RuntimeError> {
    let key = runtime.intern_property_key(name)?;
    Ok(match runtime.get_own_property(object, &key)? {
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(value),
            ..
        }) => Some(value.to_utf8_lossy()),
        Some(CompleteOrdinaryPropertyDescriptor::Data { .. })
        | Some(CompleteOrdinaryPropertyDescriptor::Accessor { .. })
        | None => None,
    })
}

fn own_positive_u32_property(
    runtime: &Runtime,
    object: &ObjectRef,
    name: &str,
) -> Result<Option<u32>, RuntimeError> {
    let key = runtime.intern_property_key(name)?;
    Ok(match runtime.get_own_property(object, &key)? {
        Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(value),
            ..
        }) if value > 0 => u32::try_from(value).ok(),
        Some(CompleteOrdinaryPropertyDescriptor::Data { .. })
        | Some(CompleteOrdinaryPropertyDescriptor::Accessor { .. })
        | None => None,
    })
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
    use std::cell::Cell;
    use std::fs;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use quickjs_oxide::{
        CompileOptions, Context, JsString, ModuleImportAttribute, ModuleImportAttributes,
        ModuleLoadResult, ModuleLoader, Runtime, RuntimeError, Value,
    };

    use super::{
        ExactTest262ModuleLoader, ExceptionDiagnostic, authenticate_dynamic_import_bytecode,
        classify_async_print_log, classify_completion, configure_runtime_can_block, run_worker,
        take_error,
    };
    use crate::admissions::{AdmissionCatalog, DynamicImportBytecodeExpectation, sha256};
    use crate::metadata::{Metadata, NegativeExpectation};
    use crate::{Variant, WorkerOptions};

    fn admissions_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dev-support/test262/admissions.tsv")
    }

    fn admissions_sha256() -> String {
        sha256(&fs::read(admissions_path()).expect("read checked-in admissions"))
    }

    fn admission_row(fields: [&str; 16]) -> String {
        fields
            .map(|field| if field.is_empty() { "-" } else { field })
            .join("\t")
    }

    fn json_loader_catalog(json_sha256: &str) -> AdmissionCatalog {
        const HEADER: &str = "kind\tgroup\tpath\tsource_sha256\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tclosure_file_count\tpriority\trequest_index\tspecifier\tnormalized_path\tpolicy\tcohort";
        const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";
        let mut rows = [
            admission_row([
                "graph-file",
                "json-loader",
                "test/data_FIXTURE.json",
                json_sha256,
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
            admission_row([
                "graph-file",
                "json-loader",
                "test/root.js",
                SHA,
                "",
                "module",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
            admission_row([
                "graph-request",
                "json-loader",
                "test/root.js",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "0",
                "./data_FIXTURE.json",
                "test/data_FIXTURE.json",
                "",
                "",
            ]),
            admission_row([
                "graph-root",
                "json-loader",
                "test/root.js",
                "",
                "",
                "",
                "",
                "",
                "",
                "2",
                "0",
                "",
                "",
                "",
                "",
                "",
            ]),
        ];
        rows.sort();
        AdmissionCatalog::parse(&format!("{HEADER}\n{}\n", rows.join("\n"))).unwrap()
    }

    fn dynamic_import_admissions(root_source: &str, fixture_source: &str) -> String {
        const HEADER: &str = "kind\tgroup\tpath\tsource_sha256\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tclosure_file_count\tpriority\trequest_index\tspecifier\tnormalized_path\tpolicy\tcohort";
        let root_sha256 = sha256(root_source.as_bytes());
        let fixture_sha256 = sha256(fixture_source.as_bytes());
        let mut rows = [
            admission_row([
                "dynamic-import-root",
                "dynamic-import-worker",
                "test/dynamic.js",
                "",
                "",
                "",
                "",
                "",
                "",
                "2",
                "0",
                "",
                "",
                "",
                "initial-import-tree",
                "",
            ]),
            admission_row([
                "graph-file",
                "dynamic-import-worker",
                "test/dynamic.js",
                &root_sha256,
                "",
                "async,raw",
                "dynamic-import",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
            admission_row([
                "graph-file",
                "dynamic-import-worker",
                "test/fixture_FIXTURE.js",
                &fixture_sha256,
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
            admission_row([
                "graph-request",
                "dynamic-import-worker",
                "test/dynamic.js",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "0",
                "./fixture_FIXTURE.js",
                "test/fixture_FIXTURE.js",
                "",
                "",
            ]),
        ];
        rows.sort();
        format!("{HEADER}\n{}\n", rows.join("\n"))
    }

    fn import_attributes(entries: &[(&'static str, &'static str)]) -> ModuleImportAttributes {
        ModuleImportAttributes::Present(
            entries
                .iter()
                .map(|(key, value)| ModuleImportAttribute {
                    key: JsString::try_from_utf8(key).unwrap(),
                    value: JsString::try_from_utf8(value).unwrap(),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    }

    fn execute_thrown(
        runtime: &Runtime,
        context: &mut Context,
        source: &str,
    ) -> ExceptionDiagnostic {
        let function = context
            .compile_with_options(source, &CompileOptions::new("diagnostic-test.js"))
            .unwrap();
        let error = context.execute(&function).unwrap_err();
        assert_eq!(error, RuntimeError::Exception);
        take_error(runtime, context, error)
    }

    fn evaluate(context: &mut Context, source: &str) -> Value {
        let function = context
            .compile_with_options(source, &CompileOptions::new("diagnostic-observer.js"))
            .unwrap();
        context.execute(&function).unwrap()
    }

    #[test]
    fn exact_test262_loader_selects_json_text_only_for_type_json() {
        const JSON_SOURCE: &str = "{\"note\":\"/*--- raw JSON, not Test262 metadata\"}\n";
        const JSON_SHA256: &str =
            "8b784bbd9f9603a60109942d4c921d11179a674463fa073416a3d2f38235802f";
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-json-loader-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::write(suite.join("test/data_FIXTURE.json"), JSON_SOURCE).unwrap();

        let resolution_started = Rc::new(Cell::new(false));
        let loader = ExactTest262ModuleLoader {
            admissions: Rc::new(json_loader_catalog(JSON_SHA256)),
            suite: suite.clone(),
            root: PathBuf::from("test/root.js"),
            goal: crate::admissions::ModuleGraphRootGoal::StaticModule,
            resolution_started: Rc::clone(&resolution_started),
        };
        let name = JsString::try_from_utf8("test/data_FIXTURE.json").unwrap();
        let absent = ModuleImportAttributes::Absent;
        let empty = import_attributes(&[]);
        let javascript = import_attributes(&[("type", "javascript")]);
        let wrong_key = import_attributes(&[("Type", "json")]);
        let json = import_attributes(&[("integrity", "pinned"), ("type", "json")]);

        assert!(loader.check_attributes(json.effective().unwrap()).is_ok());
        for attributes in [&absent, &empty, &javascript, &wrong_key] {
            assert_eq!(
                loader.load_with_attributes(&name, attributes).unwrap(),
                ModuleLoadResult::SourceText(JSON_SOURCE.to_owned())
            );
        }
        assert_eq!(
            loader.load_with_attributes(&name, &json).unwrap(),
            ModuleLoadResult::JsonText(JSON_SOURCE.to_owned())
        );
        assert!(resolution_started.get());

        fs::remove_dir_all(suite).unwrap();
    }

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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
    fn worker_rejects_every_unauthenticated_dynamic_import_test() {
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-dynamic-import-worker-deny-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let relative = PathBuf::from("test/unadmitted-dynamic-import.js");
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::write(
            suite.join(&relative),
            "/*---\nflags: [raw]\nfeatures: [source-phase-imports]\n---*/\nimport('./fixture_FIXTURE.js');\n",
        )
        .unwrap();

        let error = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
            variant: Variant::Sloppy,
            allow_async_host: true,
            allow_agent_host: false,
        })
        .unwrap_err();
        fs::remove_dir_all(suite).unwrap();

        assert!(
            error.contains("dynamic-import worker rejected unaudited path"),
            "{error}"
        );
    }

    #[test]
    fn worker_rejects_runtime_compiled_dynamic_import_bypasses() {
        for (case, source) in [
            ("eval-live", "eval(\"import('./fixture_FIXTURE.js')\");\n"),
            (
                "eval-dead",
                "eval(\"if (false) import('./fixture_FIXTURE.js')\");\n",
            ),
            (
                "function-constructor",
                "Function(\"return import('./fixture_FIXTURE.js')\");\n",
            ),
            (
                "eval-script",
                "$262.evalScript(\"if (false) import('./fixture_FIXTURE.js')\");\n",
            ),
        ] {
            let suite = std::env::temp_dir().join(format!(
                "quickjs-oxide-dynamic-import-worker-{case}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let relative = PathBuf::from(format!("test/{case}.js"));
            fs::create_dir_all(suite.join("test")).unwrap();
            fs::write(
                suite.join(&relative),
                format!("/*---\nflags: [raw]\n---*/\ntry {{ {source} }} catch (_) {{}}\n"),
            )
            .unwrap();

            let result = run_worker(&WorkerOptions {
                suite: suite.clone(),
                test: relative,
                admissions: admissions_path(),
                admissions_sha256: admissions_sha256(),
                variant: Variant::Sloppy,
                allow_async_host: false,
                allow_agent_host: false,
            })
            .unwrap();
            fs::remove_dir_all(suite).unwrap();

            assert_ne!(result.outcome, "pass", "{case} escaped the policy");
            assert!(
                result.detail.contains("dynamic-import bytecode policy"),
                "{case}: {}",
                result.detail
            );
        }
    }

    #[test]
    fn runtime_compiled_dynamic_import_expectation_is_bidirectional() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let initial = context.compile("0;").unwrap();
        let path = PathBuf::from("test/runtime-compiled-import.js");

        runtime.set_dynamic_import_bytecode_allowed(false);
        authenticate_dynamic_import_bytecode(
            &runtime,
            &initial,
            Some(DynamicImportBytecodeExpectation::RuntimeCompiledImport),
            &path,
        )
        .unwrap();

        let runtime_compilation = context
            .compile("Function(\"return import('./fixture_FIXTURE.js')\");")
            .unwrap();
        context.execute(&runtime_compilation).unwrap();

        let unexpected_initial = context.compile("import('./fixture_FIXTURE.js');").unwrap();
        let error = authenticate_dynamic_import_bytecode(
            &runtime,
            &unexpected_initial,
            Some(DynamicImportBytecodeExpectation::RuntimeCompiledImport),
            &path,
        )
        .unwrap_err();
        assert!(error.contains("unexpectedly contained initial"), "{error}");
    }

    #[test]
    fn worker_dynamic_import_bytecode_guard_does_not_confuse_import_named_members() {
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-dynamic-import-worker-member-name-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let relative = PathBuf::from("test/import-member-names.js");
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::write(
            suite.join(&relative),
            "/*---\nflags: [raw]\n---*/\nvar object = { get import() { return 1; }, set import(value) {}, import() {} };\nclass C { import() {} static import() {} get import() { return 1; } set import(value) {} }\n",
        )
        .unwrap();

        let result = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: false,
        });
        fs::remove_dir_all(suite).unwrap();
        let result = result.unwrap();

        assert_eq!(result.outcome, "pass", "{}", result.detail);
    }

    #[test]
    fn dynamic_import_parse_negative_is_classified_before_bytecode_admission() {
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-dynamic-import-worker-parse-negative-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let relative = PathBuf::from("test/invalid-import-call.js");
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::write(
            suite.join(&relative),
            "/*---\nflags: [raw]\nfeatures: [dynamic-import]\nnegative:\n  phase: parse\n  type: SyntaxError\n---*/\nimport(,);\n",
        )
        .unwrap();

        let result = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: false,
        });
        fs::remove_dir_all(suite).unwrap();
        let result = result.unwrap();

        assert_eq!(result.outcome, "pass", "{}", result.detail);
        assert_eq!(result.actual_phase, "parse");
        assert_eq!(result.actual_type, "SyntaxError");
    }

    #[test]
    fn authenticated_dynamic_import_root_runs_as_a_relative_named_async_script() {
        const ROOT_SOURCE: &str = "/*---\nflags: [async, raw]\nfeatures: [dynamic-import]\n---*/\nfunction load() { return import(\"./fixture_FIXTURE.js\"); }\nvar dynamicLoad = Function(\"return import({ toString: function () { throw 42; } })\");\ndynamicLoad().then(function() {\n  throw new Error(\"dynamic import unexpectedly fulfilled\");\n}, function(reason) {\n  if (reason !== 42) throw new Error(\"wrong dynamic rejection\");\n  return load();\n}).then(function(ns) {\n  if (ns.value !== 42) throw new Error(\"wrong authored namespace\");\n  $DONE();\n}, $DONE);\n";
        const FIXTURE_SOURCE: &str = "export const value = 42;\n";
        const DONE_HANDLE: &str = "function $DONE(error) { print(error ? 'Test262:AsyncTestFailure:' + error : 'Test262:AsyncTestComplete'); }\n";
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-dynamic-import-worker-allow-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let relative = PathBuf::from("test/dynamic.js");
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::create_dir_all(suite.join("harness")).unwrap();
        fs::write(suite.join(&relative), ROOT_SOURCE).unwrap();
        fs::write(suite.join("test/fixture_FIXTURE.js"), FIXTURE_SOURCE).unwrap();
        fs::write(suite.join("harness/doneprintHandle.js"), DONE_HANDLE).unwrap();
        let admissions = dynamic_import_admissions(ROOT_SOURCE, FIXTURE_SOURCE);
        let admissions_path = suite.join("admissions.tsv");
        fs::write(&admissions_path, &admissions).unwrap();

        let result = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            admissions: admissions_path,
            admissions_sha256: sha256(admissions.as_bytes()),
            variant: Variant::Sloppy,
            allow_async_host: true,
            allow_agent_host: false,
        });
        fs::remove_dir_all(suite).unwrap();
        let result = result.unwrap();

        assert_eq!(result.outcome, "pass", "{}", result.detail);
        assert_eq!(result.actual_phase, "normal");
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
            &ExceptionDiagnostic {
                error_type: "SyntaxError".to_owned(),
                message: "unexpected token in expression: '}'".to_owned(),
                line: Some(3),
                column: Some(7),
            },
        );

        assert_eq!(result.outcome, "pass");
        assert_eq!(result.actual_phase, "parse");
        assert_eq!(result.actual_type, "SyntaxError");
        assert_eq!(result.detail, "unexpected token in expression: '}'");
        assert_eq!(
            (result.actual_line, result.actual_column),
            (Some(3), Some(7))
        );
    }

    #[test]
    fn native_syntax_error_own_data_exposes_message_and_location() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let error = context
            .compile_module_with_filename(
                "import { eval } from './dependency.js';",
                "invalid-import-binding.mjs",
            )
            .unwrap_err();
        assert_eq!(error, RuntimeError::Exception);
        let diagnostic = take_error(&runtime, &mut context, error);
        assert_eq!(diagnostic.error_type, "SyntaxError");
        assert_eq!(diagnostic.message, "invalid import binding");
        assert_eq!((diagnostic.line, diagnostic.column), (Some(1), Some(15)));
    }

    #[test]
    fn test262_error_uses_its_side_effect_free_constructor_name() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let diagnostic = execute_thrown(
            &runtime,
            &mut context,
            r#"
function Test262Error(message) {
  this.message = message || "";
}
Test262Error.prototype.toString = function () {
  return "Test262Error: " + this.message;
};
throw new Test262Error("sentinel");
"#,
        );

        assert_eq!(diagnostic.error_type, "Test262Error");
        assert_eq!(diagnostic.message, "sentinel");
    }

    #[test]
    fn ordinary_thrown_object_does_not_claim_the_object_constructor_name() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let diagnostic = execute_thrown(&runtime, &mut context, "throw {};");

        assert_eq!(diagnostic.error_type, "ThrownObject");
    }

    #[test]
    fn diagnostic_constructor_fallback_does_not_run_getters_or_proxy_traps() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let getter = execute_thrown(
            &runtime,
            &mut context,
            r#"
var diagnosticGetterReads = 0;
var getterPrototype = {};
Object.defineProperty(getterPrototype, "constructor", {
  get: function () {
    diagnosticGetterReads += 1;
    return function GetterConstructor() {};
  }
});
throw Object.create(getterPrototype);
"#,
        );
        assert_eq!(getter.error_type, "ThrownObject");
        assert_eq!(
            evaluate(&mut context, "diagnosticGetterReads;"),
            Value::Int(0)
        );

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let proxy = execute_thrown(
            &runtime,
            &mut context,
            r#"
var diagnosticProxyReads = 0;
var proxyPrototype = new Proxy(
  { constructor: function ProxyConstructor() {} },
  {
    get: function (target, key, receiver) {
      diagnosticProxyReads += 1;
      return Reflect.get(target, key, receiver);
    },
    getOwnPropertyDescriptor: function (target, key) {
      diagnosticProxyReads += 1;
      return Reflect.getOwnPropertyDescriptor(target, key);
    },
    getPrototypeOf: function (target) {
      diagnosticProxyReads += 1;
      return Reflect.getPrototypeOf(target);
    }
  }
);
throw Object.create(proxyPrototype);
"#,
        );
        assert_eq!(proxy.error_type, "ThrownObject");
        assert_eq!(
            evaluate(&mut context, "diagnosticProxyReads;"),
            Value::Int(0)
        );
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
    fn dependency_free_request_shaped_module_negatives_stop_during_parse() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        // Each source contains a static import/export request, but this exact
        // cohort is admitted only as dependency-free parse-negative input.
        // The worker therefore installs no fixture loader and must observe the
        // SyntaxError while compiling, before link or module resolution.
        for relative in [
            "test/language/export/escaped-as-export-specifier.js",
            "test/language/import/dup-bound-names.js",
            "test/language/module-code/early-dup-export-as-star-as.js",
            "test/language/module-code/parse-err-semi-named-export-from.js",
        ] {
            let result = run_worker(&WorkerOptions {
                suite: suite.clone(),
                test: PathBuf::from(relative),
                admissions: admissions_path(),
                admissions_sha256: admissions_sha256(),
                variant: Variant::Sloppy,
                allow_async_host: false,
                allow_agent_host: false,
            })
            .unwrap_or_else(|error| panic!("run {relative}: {error}"));
            assert_eq!(result.outcome, "pass", "{relative}: {}", result.detail);
            assert_eq!(result.actual_phase, "parse", "{relative}");
            assert_eq!(result.actual_type, "SyntaxError", "{relative}");
        }
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
            variant: Variant::Sloppy,
            allow_async_host: false,
            allow_agent_host: false,
        })
        .unwrap_err();
        assert_eq!(denied, "unsupported test reached worker");

        let result = run_worker(&WorkerOptions {
            suite: suite.clone(),
            test: relative,
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
            admissions: admissions_path(),
            admissions_sha256: admissions_sha256(),
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
    fn valid_import_call_compilation_is_uniform_at_the_context_boundary() {
        const SOURCE: &str = "import('fixture');";

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .compile(SOURCE)
            .expect("default compile rejected a valid ImportCall");
        assert!(context.take_exception().unwrap().is_none());

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let options = CompileOptions::new("dynamic-import.js");
        context
            .compile_with_options(SOURCE, &options)
            .expect("named compile rejected a valid ImportCall");
        assert!(context.take_exception().unwrap().is_none());
    }
}
