//! QuickJS-shaped Test262 `$262.agent` Stage A host.
//!
//! The JavaScript engine is deliberately not made thread-safe here. Every
//! agent thread constructs and owns a fresh [`Runtime`] and [`Context`]. The
//! only cross-thread values are owned source text and this module's
//! `Arc`/`Mutex` coordinator; no `Runtime`, `Context`, `Value`, `ObjectRef`, or
//! other runtime root ever crosses a thread boundary.

use super::*;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
#[cfg(not(target_family = "wasm"))]
use std::thread;
use std::thread::JoinHandle;
use std::time::Instant;

#[cfg(not(target_family = "wasm"))]
const AGENT_STACK_SIZE: usize = 2 << 20;
#[cfg(not(target_family = "wasm"))]
const AGENT_EVAL_FILENAME: &str = "<evalScript>";

/// One Test262 execution's shared agent coordinator.
///
/// This is an opt-in test host, not an ECMAScript intrinsic. Clones share only
/// host coordination state; they never share an engine runtime or realm.
#[derive(Clone)]
pub struct Test262AgentSession {
    inner: Arc<AgentSessionInner>,
}

impl Default for Test262AgentSession {
    fn default() -> Self {
        Self::new()
    }
}

impl Test262AgentSession {
    /// Create an empty Test262 agent session.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AgentSessionInner {
                reports: Mutex::new(VecDeque::new()),
                workers: Mutex::new(AgentWorkers::default()),
                clock_origin: Instant::now(),
            }),
        }
    }

    /// Join every worker in start order and detach this session from all
    /// runtimes. Calling this more than once is harmless.
    pub fn join_workers(&self) -> Result<(), Test262AgentError> {
        let mut failures = Vec::new();
        let mut index = 0;
        loop {
            let handle = {
                let mut workers = lock_unpoisoned(&self.inner.workers);
                if workers.handles.is_empty() {
                    // Every previously removed handle has completed, so no
                    // worker remains that could append another start. This
                    // also covers createRealm's intentional main-role
                    // inheritance inside a worker without racing cleanup.
                    workers.finished = true;
                    None
                } else {
                    Some(workers.handles.remove(0))
                }
            };
            let Some(handle) = handle else {
                break;
            };
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(detail)) => failures.push(format!("agent {index}: {detail}")),
                Err(_) => failures.push(format!("agent {index}: worker thread panicked")),
            }
            index += 1;
        }
        clear_session_bindings(&self.inner);
        if failures.is_empty() {
            Ok(())
        } else {
            Err(Test262AgentError {
                detail: failures.join("; "),
            })
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn start_worker(&self, source: String) -> Result<(), String> {
        let mut workers = lock_unpoisoned(&self.inner.workers);
        if workers.finished {
            return Err("Test262 agent session has already joined its workers".to_owned());
        }
        let sequence = workers.next_sequence;
        workers.next_sequence = workers
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| "Test262 agent sequence space exhausted".to_owned())?;
        let session = self.clone();
        let handle = thread::Builder::new()
            .name(format!("test262-agent-{sequence}"))
            .stack_size(AGENT_STACK_SIZE)
            .spawn(move || run_agent_worker(&session, &source))
            .map_err(|error| format!("spawn Test262 agent {sequence}: {error}"))?;
        // JavaScript calls start synchronously, so this vector is the exact
        // start/list insertion order used by pinned QuickJS's cleanup join.
        workers.handles.push(handle);
        Ok(())
    }

    #[cfg(target_family = "wasm")]
    fn start_worker(&self, _source: String) -> Result<(), String> {
        Err("Test262 agent threads are unavailable on wasm targets".to_owned())
    }
}

/// Error reported after one or more Test262 agent workers fail or panic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Test262AgentError {
    detail: String,
}

impl fmt::Display for Test262AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl StdError for Test262AgentError {}

struct AgentSessionInner {
    reports: Mutex<VecDeque<String>>,
    workers: Mutex<AgentWorkers>,
    clock_origin: Instant,
}

#[derive(Default)]
struct AgentWorkers {
    #[cfg(not(target_family = "wasm"))]
    next_sequence: u64,
    finished: bool,
    handles: Vec<JoinHandle<Result<(), String>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AgentRole {
    Main,
    Worker,
}

struct RuntimeAgentBinding {
    session: Weak<AgentSessionInner>,
    roles: HashMap<ContextId, AgentRole>,
}

static AGENT_BINDINGS: OnceLock<Mutex<HashMap<u64, RuntimeAgentBinding>>> = OnceLock::new();

fn bindings() -> &'static Mutex<HashMap<u64, RuntimeAgentBinding>> {
    AGENT_BINDINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn bind_realm(
    runtime: &Runtime,
    realm: ContextId,
    session: &Test262AgentSession,
    role: AgentRole,
) -> Result<(), RuntimeError> {
    let runtime_id = runtime.domain_id();
    let mut registry = lock_unpoisoned(bindings());
    let replace = registry
        .get(&runtime_id)
        .and_then(|binding| binding.session.upgrade())
        .is_none();
    if replace {
        registry.insert(
            runtime_id,
            RuntimeAgentBinding {
                session: Arc::downgrade(&session.inner),
                roles: HashMap::new(),
            },
        );
    }
    let binding = registry
        .get_mut(&runtime_id)
        .expect("agent binding was inserted");
    let Some(bound_session) = binding.session.upgrade() else {
        return Err(RuntimeError::Invariant(
            "Test262 agent binding lost its live session",
        ));
    };
    if !Arc::ptr_eq(&bound_session, &session.inner) {
        return Err(RuntimeError::Invariant(
            "runtime is already bound to another Test262 agent session",
        ));
    }
    match binding.roles.insert(realm, role) {
        Some(previous) if previous != role => Err(RuntimeError::Invariant(
            "Test262 agent realm role changed after installation",
        )),
        _ => Ok(()),
    }
}

fn registered_session_and_role(
    runtime: &Runtime,
    realm: ContextId,
) -> Option<(Test262AgentSession, AgentRole)> {
    let runtime_id = runtime.domain_id();
    let mut registry = lock_unpoisoned(bindings());
    let session = match registry
        .get(&runtime_id)
        .and_then(|binding| binding.session.upgrade())
    {
        Some(session) => session,
        None => {
            registry.remove(&runtime_id);
            return None;
        }
    };
    let binding = registry
        .get_mut(&runtime_id)
        .expect("live agent binding disappeared");
    // QuickJS's createRealm creates a fresh Context without copying the
    // worker's ContextOpaque. Therefore every newly inherited realm has main
    // role, including a realm created from inside an agent worker.
    let role = *binding.roles.entry(realm).or_insert(AgentRole::Main);
    Some((Test262AgentSession { inner: session }, role))
}

#[cfg(not(target_family = "wasm"))]
fn unregister_runtime(runtime_id: u64) {
    lock_unpoisoned(bindings()).remove(&runtime_id);
}

fn clear_session_bindings(session: &Arc<AgentSessionInner>) {
    lock_unpoisoned(bindings()).retain(|_, binding| {
        binding
            .session
            .upgrade()
            .is_some_and(|bound| !Arc::ptr_eq(&bound, session))
    });
}

#[cfg(not(target_family = "wasm"))]
struct RuntimeBindingGuard(u64);

#[cfg(not(target_family = "wasm"))]
impl Drop for RuntimeBindingGuard {
    fn drop(&mut self) {
        unregister_runtime(self.0);
    }
}

#[cfg(not(target_family = "wasm"))]
fn run_agent_worker(session: &Test262AgentSession, source: &str) -> Result<(), String> {
    // Only owned source text and Arc-backed host state entered this thread.
    // The engine runtime, context, bytecode, and every root are born and die
    // here, on the worker thread.
    let runtime = Runtime::new();
    runtime.set_can_block(true);
    let mut context = runtime.new_context();
    bind_realm(&runtime, context.realm_id(), session, AgentRole::Worker)
        .map_err(|error| format!("bind worker realm: {error}"))?;
    let _binding = RuntimeBindingGuard(runtime.domain_id());
    context
        .install_qjs_print()
        .map_err(|error| format!("install worker print host: {error}"))?;
    context
        .install_test262_host()
        .map_err(|error| format!("install worker Test262 host: {error}"))?;
    let function = context
        .compile_with_filename(source, AGENT_EVAL_FILENAME)
        .map_err(|error| format!("compile agent source: {error}"))?;
    context
        .execute(&function)
        .map_err(|error| format!("execute agent source: {error}"))?;
    while runtime.is_job_pending() {
        if !runtime
            .execute_pending_job()
            .map_err(|error| format!("execute agent job: {error}"))?
        {
            return Err("agent runtime reported a pending job but executed none".to_owned());
        }
    }
    Ok(())
}

impl Runtime {
    pub(in crate::runtime) fn new_registered_test262_agent_object(
        &self,
        realm: ContextId,
    ) -> Result<Option<ObjectRef>, RuntimeError> {
        let Some((_session, _role)) = registered_session_and_role(self, realm) else {
            return Ok(None);
        };
        let (object_prototype, function_prototype) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            (context.object_prototype, context.function_prototype)
        };
        let object_prototype = ObjectRef::from_borrowed_handle(self.clone(), object_prototype)?;
        let function_prototype = ObjectRef::from_borrowed_handle(self.clone(), function_prototype)?;
        let agent = self.new_object(Some(&object_prototype))?;
        for (name, length, kind) in [
            ("start", 1, Test262AgentKind::Start),
            ("getReport", 0, Test262AgentKind::GetReport),
            ("broadcast", 2, Test262AgentKind::Broadcast),
            ("report", 1, Test262AgentKind::Report),
            ("leaving", 0, Test262AgentKind::Leaving),
            ("receiveBroadcast", 1, Test262AgentKind::ReceiveBroadcast),
            ("sleep", 1, Test262AgentKind::Sleep),
            ("monotonicNow", 0, Test262AgentKind::MonotonicNow),
        ] {
            let function = self.new_native_builtin(
                &function_prototype,
                realm,
                NativeFunctionId::Test262Agent(kind),
                length,
                name,
                i32::from(length),
            )?;
            let key = self.intern_property_key(name)?;
            let descriptor = OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::Object(function.as_object().clone())),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            };
            if !self.define_own_property(&agent, &key, &descriptor)? {
                return Err(RuntimeError::Invariant(
                    "Test262 agent method definition was rejected",
                ));
            }
        }
        Ok(Some(agent))
    }

    pub(in crate::runtime) fn call_test262_agent(
        &self,
        realm: ContextId,
        kind: Test262AgentKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Test262 agent function received a constructor invocation",
            ));
        };
        let Some((session, role)) = registered_session_and_role(self, realm) else {
            return Err(RuntimeError::Invariant(
                "Test262 agent function has no registered session",
            ));
        };
        match kind {
            Test262AgentKind::Start => {
                if role == AgentRole::Worker {
                    return self
                        .test262_agent_type_error(realm, "cannot be called inside an agent");
                }
                let source = match self.native_to_js_string(realm, &arguments.readable[0])? {
                    NativeConversion::Value(source) => source,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let source = match String::from_utf16(&source.utf16_units().collect::<Vec<_>>()) {
                    Ok(source) => source,
                    Err(_) => {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Internal,
                            "agent source containing a lone UTF-16 surrogate is not implemented",
                        )?));
                    }
                };
                if let Err(error) = session.start_worker(source) {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        &error,
                    )?));
                }
                Ok(Completion::Return(Value::Undefined))
            }
            Test262AgentKind::GetReport => {
                let report = lock_unpoisoned(&session.inner.reports).pop_front();
                Ok(Completion::Return(match report {
                    Some(report) => Value::String(JsString::try_from_utf8(&report)?),
                    None => Value::Null,
                }))
            }
            Test262AgentKind::Broadcast => {
                if role == AgentRole::Worker {
                    return self
                        .test262_agent_type_error(realm, "cannot be called inside an agent");
                }
                self.test262_agent_type_error(
                    realm,
                    "broadcast is not implemented in Test262 agent Stage A",
                )
            }
            Test262AgentKind::Report => {
                // QuickJS's implementation does not enforce the comment's
                // worker-only role here; preserve the observable code behavior.
                let report = match self.native_to_js_string(realm, &arguments.readable[0])? {
                    NativeConversion::Value(report) => report.to_utf8_lossy(),
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                lock_unpoisoned(&session.inner.reports).push_back(report);
                Ok(Completion::Return(Value::Undefined))
            }
            Test262AgentKind::Leaving => {
                if role == AgentRole::Main {
                    return self.test262_agent_type_error(realm, "must be called inside an agent");
                }
                // Pinned QuickJS performs no state transition or signal here.
                Ok(Completion::Return(Value::Undefined))
            }
            Test262AgentKind::ReceiveBroadcast => {
                if role == AgentRole::Main {
                    return self.test262_agent_type_error(realm, "must be called inside an agent");
                }
                self.test262_agent_type_error(
                    realm,
                    "receiveBroadcast is not implemented in Test262 agent Stage A",
                )
            }
            Test262AgentKind::Sleep => {
                let duration = match self.native_to_number(realm, &arguments.readable[0])? {
                    NativeConversion::Value(duration) => Self::to_uint32_number(duration),
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                #[cfg(not(target_family = "wasm"))]
                thread::sleep(std::time::Duration::from_millis(u64::from(duration)));
                #[cfg(target_family = "wasm")]
                if duration != 0 {
                    return self
                        .test262_agent_type_error(realm, "sleep is unavailable on wasm targets");
                }
                Ok(Completion::Return(Value::Undefined))
            }
            Test262AgentKind::MonotonicNow => {
                let milliseconds = session.inner.clock_origin.elapsed().as_millis();
                #[allow(clippy::cast_precision_loss)]
                Ok(Completion::Return(Value::Float(milliseconds as f64)))
            }
        }
    }

    fn test262_agent_type_error(
        &self,
        realm: ContextId,
        message: &str,
    ) -> Result<Completion, RuntimeError> {
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            message,
        )?))
    }
}

impl Context {
    /// Install the QuickJS Test262 host surface with a Stage A agent session.
    ///
    /// The initial realm has main role. `createRealm` contexts inherit the same
    /// session and, matching QuickJS's null ContextOpaque behavior, also have
    /// main role. Agent worker runtimes are created only by `agent.start`.
    pub fn install_test262_host_with_agent(
        &mut self,
        session: &Test262AgentSession,
    ) -> Result<ObjectRef, RuntimeError> {
        bind_realm(&self.runtime, self.realm, session, AgentRole::Main)?;
        self.install_test262_host()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_string(context: &mut Context, source: &str) -> String {
        match context.eval(source).unwrap() {
            Value::String(value) => value.to_utf8_lossy(),
            value => panic!("{source:?} returned {value:?}, expected a string"),
        }
    }

    fn take_reports(session: &Test262AgentSession) -> Vec<String> {
        lock_unpoisoned(&session.inner.reports).drain(..).collect()
    }

    #[cfg(not(target_family = "wasm"))]
    fn wait_for_report(session: &Test262AgentSession, wanted: &str) {
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if lock_unpoisoned(&session.inner.reports)
                .iter()
                .any(|report| report == wanted)
            {
                return;
            }
            assert!(Instant::now() < deadline, "missing agent report {wanted:?}");
            thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn opt_in_agent_surface_matches_pinned_quickjs_shape_and_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.install_test262_host().unwrap();
        assert_eq!(eval_string(&mut context, "typeof $262.agent"), "undefined");

        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        assert_eq!(
            eval_string(&mut context, "Reflect.ownKeys($262).join(',')"),
            "detachArrayBuffer,evalScript,codePointRange,agent,global,createRealm,gc"
        );
        assert_eq!(
            eval_string(&mut context, "Reflect.ownKeys($262.agent).join(',')"),
            "start,getReport,broadcast,report,leaving,receiveBroadcast,sleep,monotonicNow"
        );
        assert_eq!(
            eval_string(
                &mut context,
                r#"(function () {
  var names = ["start", "getReport", "broadcast", "report", "leaving",
               "receiveBroadcast", "sleep", "monotonicNow"];
  var lengths = [1, 0, 2, 1, 0, 1, 1, 0];
  var agentDescriptor = Object.getOwnPropertyDescriptor($262, "agent");
  if (!agentDescriptor.writable || !agentDescriptor.enumerable ||
      !agentDescriptor.configurable) return "agent descriptor";
  if (Object.keys($262.agent).length !== 0) return "enumerable method";
  for (var i = 0; i < names.length; i += 1) {
    var method = Object.getOwnPropertyDescriptor($262.agent, names[i]);
    var name = Object.getOwnPropertyDescriptor(method.value, "name");
    var length = Object.getOwnPropertyDescriptor(method.value, "length");
    if (!method.writable || method.enumerable || !method.configurable)
      return "method descriptor " + names[i];
    if (name.value !== names[i] || name.writable || name.enumerable ||
        !name.configurable) return "name descriptor " + names[i];
    if (length.value !== lengths[i] || length.writable || length.enumerable ||
        !length.configurable) return "length descriptor " + names[i];
  }
  try { new $262.agent.start(""); } catch (error) {
    return error instanceof TypeError ? "ok" : "wrong constructor error";
  }
  return "constructible";
})()"#,
            ),
            "ok"
        );
        session.join_workers().unwrap();
    }

    #[test]
    fn report_queue_sleep_clock_and_main_role_are_quickjs_shaped() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();

        assert_eq!(
            eval_string(
                &mut context,
                r#"(function () {
  var first = $262.agent.getReport();
  $262.agent.report({ toString: function () { return "one"; } });
  $262.agent.report("two");
  return String(first) + "," + $262.agent.getReport() + "," +
         $262.agent.getReport() + "," + String($262.agent.getReport());
})()"#,
            ),
            "null,one,two,null"
        );
        assert_eq!(
            context
                .eval(
                    "var sleepCoercions = 0; $262.agent.sleep({ valueOf: function () { \
                     sleepCoercions += 1; return -4294967296; } }); sleepCoercions"
                )
                .unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            context
                .eval(
                    "var reportThrew = false; try { $262.agent.report({ toString: function () { \
                     throw 17; } }); } catch (error) { reportThrew = error === 17; } \
                     reportThrew && $262.agent.getReport() === null"
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            context.eval("$262.agent.sleep(NaN)").unwrap(),
            Value::Undefined
        );
        assert_eq!(
            context
                .eval(
                    "var before = $262.agent.monotonicNow(); $262.agent.sleep(2); \
                     $262.agent.monotonicNow() >= before"
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            eval_string(
                &mut context,
                r#"(function () {
  function message(callback) {
    try { callback(); return "missing"; }
    catch (error) { return error.name + ": " + error.message; }
  }
  return message(function () { $262.agent.leaving(); }) + "|" +
         message(function () { $262.agent.broadcast(); }) + "|" +
         message(function () { $262.agent.receiveBroadcast(); });
})()"#,
            ),
            "TypeError: must be called inside an agent|TypeError: broadcast is not implemented in Test262 agent Stage A|TypeError: must be called inside an agent"
        );
        session.join_workers().unwrap();
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn worker_has_fresh_blocking_runtime_and_quickjs_role_checks() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        context
            .eval(
                r#"$262.agent.start(`
  $262.agent.report(Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 0));
  function message(callback) {
    try { callback(); return "missing"; }
    catch (error) { return error.name + ": " + error.message; }
  }
  $262.agent.report(message(function () { $262.agent.start(""); }));
  $262.agent.report(message(function () { $262.agent.broadcast(); }));
  $262.agent.report(message(function () { $262.agent.receiveBroadcast(); }));
  $262.agent.leaving();
  $262.agent.report("after-leaving");
`);"#,
            )
            .unwrap();
        session.join_workers().unwrap();
        assert_eq!(
            take_reports(&session),
            [
                "timed-out",
                "TypeError: cannot be called inside an agent",
                "TypeError: cannot be called inside an agent",
                "TypeError: receiveBroadcast is not implemented in Test262 agent Stage A",
                "after-leaving",
            ]
        );
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn create_realm_inherits_session_with_quickjs_main_role() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        context
            .eval(
                r#"var child262 = $262.createRealm();
child262.agent.start("$262.agent.report('main-child')");"#,
            )
            .unwrap();
        session.join_workers().unwrap();
        assert_eq!(take_reports(&session), ["main-child"]);

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        context
            .eval(
                r#"$262.agent.start(`
  $262.agent.report("outer-ready");
  $262.agent.sleep(100);
  var child262 = $262.createRealm();
  try { child262.agent.leaving(); }
  catch (error) { $262.agent.report("child-role: " + error.message); }
  child262.agent.start("$262.agent.report('nested-worker')");
  $262.agent.report("outer-worker");
`);"#,
            )
            .unwrap();
        // Begin cleanup while the outer worker is still live. Its inherited
        // main-role realm must remain allowed to append the nested worker.
        wait_for_report(&session, "outer-ready");
        session.join_workers().unwrap();
        let mut reports = take_reports(&session);
        reports.sort();
        assert_eq!(
            reports,
            [
                "child-role: must be called inside an agent",
                "nested-worker",
                "outer-ready",
                "outer-worker",
            ]
        );
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn join_surfaces_worker_failures() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        context
            .eval("$262.agent.start(\"throw new Error('worker failure')\")")
            .unwrap();
        let error = session.join_workers().unwrap_err().to_string();
        assert!(error.contains("agent 0: execute agent source"), "{error}");
    }
}
