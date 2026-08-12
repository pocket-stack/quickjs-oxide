//! QuickJS-shaped Test262 `$262.agent` host.
//!
//! The JavaScript engine is deliberately not made thread-safe here. Every
//! agent thread constructs and owns a fresh [`Runtime`] and [`Context`]. The
//! only cross-thread values are owned source text and this module's
//! `Arc`/`Mutex` coordinator; no `Runtime`, `Context`, `Value`, `ObjectRef`, or
//! other runtime root ever crosses a thread boundary.

use super::*;

use crate::shared_memory::SharedBufferHandle;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
#[cfg(not(target_family = "wasm"))]
use std::sync::Condvar;
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
                #[cfg(not(target_family = "wasm"))]
                workers_changed: Condvar::new(),
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
        // Pinned QuickJS links the zeroed agent record before pthread_create.
        // Publish the corresponding ordered delivery slot before spawning so
        // an immediate broadcast can never miss a successfully started agent.
        workers.slots.push(AgentWorkerSlot {
            sequence,
            pending: None,
            acknowledged_generation: 0,
        });
        let session = self.clone();
        let spawn = thread::Builder::new()
            .name(format!("test262-agent-{sequence}"))
            .stack_size(AGENT_STACK_SIZE)
            .spawn(move || run_agent_worker(&session, sequence, &source));
        let handle = match spawn {
            Ok(handle) => handle,
            Err(error) => {
                let removed = workers.slots.pop();
                debug_assert_eq!(removed.map(|slot| slot.sequence), Some(sequence));
                return Err(format!("spawn Test262 agent {sequence}: {error}"));
            }
        };
        // JavaScript calls start synchronously, so this vector is the exact
        // start/list insertion order used by pinned QuickJS's cleanup join.
        workers.handles.push(handle);
        Ok(())
    }

    #[cfg(not(target_family = "wasm"))]
    fn broadcast(&self, handle: SharedBufferHandle, value: i32) -> Result<(), String> {
        let mut workers = lock_unpoisoned(&self.inner.workers);
        while workers.broadcast_in_progress {
            workers = wait_unpoisoned(&self.inner.workers_changed, workers);
        }

        workers.last_generation = workers
            .last_generation
            .checked_add(1)
            .ok_or_else(|| "Test262 agent broadcast generation space exhausted".to_owned())?;
        let generation = workers.last_generation;
        let cohort_len = workers.slots.len();
        workers.broadcast_in_progress = true;
        for slot in workers.slots.iter_mut().take(cohort_len) {
            debug_assert!(slot.pending.is_none());
            slot.pending = Some(AgentBroadcast {
                generation,
                handle: handle.clone(),
                value,
            });
        }
        self.inner.workers_changed.notify_all();

        // pthread_cond_wait has no timeout in pinned QuickJS. Preserve that
        // contract: every agent in the invocation-time cohort must ACK after
        // taking its delivery and before entering JavaScript callback code.
        while workers.slots[..cohort_len]
            .iter()
            .any(|slot| slot.acknowledged_generation < generation)
        {
            workers = wait_unpoisoned(&self.inner.workers_changed, workers);
        }
        workers.broadcast_in_progress = false;
        self.inner.workers_changed.notify_all();
        Ok(())
    }

    #[cfg(target_family = "wasm")]
    fn broadcast(&self, _handle: SharedBufferHandle, _value: i32) -> Result<(), String> {
        Err("Test262 agent threads are unavailable on wasm targets".to_owned())
    }

    #[cfg(not(target_family = "wasm"))]
    fn wait_for_broadcast(&self, sequence: u64) -> Result<AgentBroadcast, String> {
        let mut workers = lock_unpoisoned(&self.inner.workers);
        loop {
            let slot = workers
                .slots
                .iter_mut()
                .find(|slot| slot.sequence == sequence)
                .ok_or_else(|| format!("Test262 agent {sequence} lost its delivery slot"))?;
            if let Some(delivery) = slot.pending.take() {
                slot.acknowledged_generation = delivery.generation;
                self.inner.workers_changed.notify_all();
                return Ok(delivery);
            }
            workers = wait_unpoisoned(&self.inner.workers_changed, workers);
        }
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
    #[cfg(not(target_family = "wasm"))]
    workers_changed: Condvar,
    clock_origin: Instant,
}

#[derive(Default)]
struct AgentWorkers {
    #[cfg(not(target_family = "wasm"))]
    next_sequence: u64,
    #[cfg(not(target_family = "wasm"))]
    last_generation: u64,
    #[cfg(not(target_family = "wasm"))]
    broadcast_in_progress: bool,
    finished: bool,
    handles: Vec<JoinHandle<Result<(), String>>>,
    #[cfg(not(target_family = "wasm"))]
    slots: Vec<AgentWorkerSlot>,
}

#[cfg(not(target_family = "wasm"))]
struct AgentWorkerSlot {
    sequence: u64,
    pending: Option<AgentBroadcast>,
    acknowledged_generation: u64,
}

#[cfg(not(target_family = "wasm"))]
struct AgentBroadcast {
    generation: u64,
    handle: SharedBufferHandle,
    value: i32,
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

#[cfg(not(target_family = "wasm"))]
fn wait_unpoisoned<'a, T>(condition: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    condition
        .wait(guard)
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

thread_local! {
    /// Worker callbacks are runtime roots and must never enter the Send/Sync
    /// session coordinator. The native receiveBroadcast call and the worker
    /// loop execute on the same agent thread, so a runtime-domain key is
    /// sufficient to retain and replace the callback in thread-local storage.
    static AGENT_WORKER_CALLBACKS: RefCell<HashMap<u64, CallableRef>> =
        RefCell::new(HashMap::new());
}

fn install_worker_callback(runtime_id: u64, callback: CallableRef) {
    AGENT_WORKER_CALLBACKS.with(|callbacks| {
        callbacks.borrow_mut().insert(runtime_id, callback);
    });
}

#[cfg(not(target_family = "wasm"))]
fn worker_callback(runtime_id: u64) -> Option<CallableRef> {
    AGENT_WORKER_CALLBACKS.with(|callbacks| callbacks.borrow().get(&runtime_id).cloned())
}

#[cfg(not(target_family = "wasm"))]
fn clear_worker_callback(runtime_id: u64) {
    AGENT_WORKER_CALLBACKS.with(|callbacks| {
        callbacks.borrow_mut().remove(&runtime_id);
    });
}

#[cfg(not(target_family = "wasm"))]
struct WorkerCallbackGuard(u64);

#[cfg(not(target_family = "wasm"))]
impl Drop for WorkerCallbackGuard {
    fn drop(&mut self) {
        clear_worker_callback(self.0);
    }
}

#[cfg(not(target_family = "wasm"))]
fn run_agent_worker(
    session: &Test262AgentSession,
    sequence: u64,
    source: &str,
) -> Result<(), String> {
    // Only owned source text and Arc-backed host state entered this thread.
    // The engine runtime, context, bytecode, and every root are born and die
    // here, on the worker thread.
    let runtime = Runtime::new();
    // Agent source is a second compilation domain whose text is not part of
    // the main worker's authenticated bytecode tree. Current admissions keep
    // agent-host and dynamic-import roots disjoint, so fail closed here until
    // an independently authenticated agent dynamic-import cohort exists.
    runtime.set_dynamic_import_bytecode_allowed(false);
    let runtime_id = runtime.domain_id();
    runtime.set_can_block(true);
    let mut context = runtime.new_context();
    bind_realm(&runtime, context.realm_id(), session, AgentRole::Worker)
        .map_err(|error| format!("bind worker realm: {error}"))?;
    let _binding = RuntimeBindingGuard(runtime_id);
    let _callback = WorkerCallbackGuard(runtime_id);
    context
        .install_qjs_print()
        .map_err(|error| format!("install worker print host: {error}"))?;
    context
        .install_test262_host()
        .map_err(|error| format!("install worker Test262 host: {error}"))?;

    let mut failures = Vec::new();
    match context.compile_with_filename(source, AGENT_EVAL_FILENAME) {
        Ok(function) => {
            if let Err(error) = context.execute(&function) {
                record_agent_failure(&mut context, &mut failures, "execute agent source", &error);
            }
        }
        Err(error) => {
            record_agent_failure(&mut context, &mut failures, "compile agent source", &error);
        }
    }

    let mut job_operation = "execute agent job";
    loop {
        if !drain_agent_jobs(&runtime, &mut context, &mut failures, job_operation) {
            break;
        }

        let Some(callback) = worker_callback(runtime_id) else {
            break;
        };

        // wait_for_broadcast takes the delivery and signals the main thread
        // while holding the coordinator mutex. Import and callback execution
        // therefore happen strictly after this worker's ACK.
        let delivery = session.wait_for_broadcast(sequence)?;
        let shared = match context.import_shared_array_buffer(delivery.handle) {
            Ok(shared) => shared,
            Err(error) => {
                record_agent_failure(
                    &mut context,
                    &mut failures,
                    "import agent broadcast buffer",
                    &error,
                );
                clear_worker_callback(runtime_id);
                break;
            }
        };
        if let Err(error) = context.call(
            &callback,
            Value::Undefined,
            &[Value::Object(shared), Value::Int(delivery.value)],
        ) {
            record_agent_failure(
                &mut context,
                &mut failures,
                "call agent broadcast callback",
                &error,
            );
        }

        // Pinned QuickJS clears broadcast_func immediately after JS_Call. A
        // synchronous replacement is therefore discarded, while a Promise
        // job can install the next callback during the following drain pass.
        clear_worker_callback(runtime_id);
        job_operation = "execute agent callback job";
    }
    finish_agent_failures(failures)
}

#[cfg(not(target_family = "wasm"))]
fn record_agent_failure(
    context: &mut Context,
    failures: &mut Vec<String>,
    operation: &str,
    error: &RuntimeError,
) {
    failures.push(format!("{operation}: {error}"));
    if context.has_exception() {
        if let Err(clear_error) = context.take_exception() {
            failures.push(format!("clear {operation} exception: {clear_error}"));
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn drain_agent_jobs(
    runtime: &Runtime,
    context: &mut Context,
    failures: &mut Vec<String>,
    operation: &str,
) -> bool {
    while runtime.is_job_pending() {
        match runtime.execute_pending_job() {
            Ok(true) => {}
            Ok(false) => {
                failures.push("agent runtime reported a pending job but executed none".to_owned());
                return false;
            }
            Err(error) => {
                record_agent_failure(context, failures, operation, &error);
                return false;
            }
        }
    }
    true
}

#[cfg(not(target_family = "wasm"))]
fn finish_agent_failures(failures: Vec<String>) -> Result<(), String> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
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
                // JS_GetArrayBuffer performs its ArrayBuffer/SAB brand and
                // detached checks before JS_ToInt32 can run user code.
                let handle = match self
                    .test262_agent_export_broadcast_buffer(realm, &arguments.readable[0])?
                {
                    NativeConversion::Value(handle) => handle,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let value = match self.native_to_number(realm, &arguments.readable[1])? {
                    NativeConversion::Value(value) => crate::number::to_int32(value),
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if let Err(error) = session.broadcast(handle, value) {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        &error,
                    )?));
                }
                Ok(Completion::Return(Value::Undefined))
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
                let callback = match &arguments.readable[0] {
                    Value::Object(object) => self.as_callable(object)?,
                    _ => None,
                };
                let Some(callback) = callback else {
                    return self.test262_agent_type_error(realm, "expecting function");
                };
                install_worker_callback(self.domain_id(), callback);
                Ok(Completion::Return(Value::Undefined))
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

    fn test262_agent_export_broadcast_buffer(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<SharedBufferHandle>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer object expected",
            )?));
        };
        let Some(access) = self.snapshot_buffer_access_if_branded(object)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer object expected",
            )?));
        };
        if access.state.detached {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        if !access.is_shared() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ordinary ArrayBuffer broadcast is unavailable across runtimes",
            )?));
        }
        if access.state.max_byte_length.is_some() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "growable SharedArrayBuffer broadcast is unavailable across runtimes",
            )?));
        }
        let handle =
            self.shared_array_buffer_handle_if_branded(object)?
                .ok_or(RuntimeError::Invariant(
                    "validated shared broadcast buffer lost its class payload",
                ))?;
        Ok(NativeConversion::Value(handle))
    }
}

impl Context {
    /// Install the QuickJS Test262 host surface with an agent session.
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
            "detachArrayBuffer,evalScript,codePointRange,agent,global,createRealm,IsHTMLDDA,gc"
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
            "TypeError: must be called inside an agent|TypeError: ArrayBuffer object expected|TypeError: must be called inside an agent"
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
                "TypeError: expecting function",
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
    fn broadcast_handles_zero_and_invocation_time_worker_cohorts() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();

        assert_eq!(
            context
                .eval("$262.agent.broadcast(new SharedArrayBuffer(0))")
                .unwrap(),
            Value::Undefined
        );
        context
            .eval(
                r#"$262.agent.start(`
  $262.agent.receiveBroadcast(function (sab, value) {
    $262.agent.report("worker-0:" + value + ":" + sab.byteLength);
  });
  $262.agent.report("ready-0");
`);
$262.agent.start(`
  $262.agent.receiveBroadcast(function (sab, value) {
    $262.agent.report("worker-1:" + value + ":" + sab.byteLength);
  });
  $262.agent.report("ready-1");
`);
$262.agent.start(`
  $262.agent.receiveBroadcast(function (sab, value) {
    $262.agent.report("worker-2:" + value + ":" + sab.byteLength);
  });
  $262.agent.report("ready-2");
`);
var cohortBuffer = new SharedArrayBuffer(4);"#,
            )
            .unwrap();
        assert_eq!(lock_unpoisoned(&session.inner.workers).slots.len(), 3);
        wait_for_report(&session, "ready-0");
        wait_for_report(&session, "ready-1");
        wait_for_report(&session, "ready-2");
        let mut ready = take_reports(&session);
        ready.sort();
        assert_eq!(ready, ["ready-0", "ready-1", "ready-2"]);
        context.eval("$262.agent.broadcast(cohortBuffer)").unwrap();
        session.join_workers().unwrap();
        let mut reports = take_reports(&session);
        reports.sort();
        assert_eq!(reports, ["worker-0:0:4", "worker-1:0:4", "worker-2:0:4"]);
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn receiver_can_wait_before_broadcast_and_ack_precedes_callback_completion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        context
            .eval(
                r#"var ackGate = new SharedArrayBuffer(8);
var ackGateView = new Int32Array(ackGate);
$262.agent.start(`
  $262.agent.receiveBroadcast(function (sab, value) {
    var view = new Int32Array(sab);
    $262.agent.report("callback:" + value);
    var outcome = Atomics.wait(view, 1, 0, 1000);
    $262.agent.report("released:" + outcome);
  });
  $262.agent.report("ready");
`);"#,
            )
            .unwrap();
        wait_for_report(&session, "ready");

        // If broadcast waited for callback completion instead of its ACK,
        // the callback's finite wait would time out before this store ran.
        context
            .eval(
                "$262.agent.broadcast(ackGate, 17); \
                 Atomics.store(ackGateView, 1, 1); Atomics.notify(ackGateView, 1);",
            )
            .unwrap();
        session.join_workers().unwrap();
        let reports = take_reports(&session);
        assert_eq!(&reports[..2], ["ready", "callback:17"]);
        assert!(
            matches!(reports[2].as_str(), "released:ok" | "released:not-equal"),
            "unexpected callback wait outcome: {reports:?}"
        );
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn callback_replacement_role_checks_and_conversion_order_are_pinned() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();

        assert_eq!(
            eval_string(
                &mut context,
                r#"(function () {
  var invalidFirstTouched = 0;
  var validValueTouched = 0;
  var ordinaryTouched = 0;
  var growableTouched = 0;
  function message(callback) {
    try { callback(); return "missing"; }
    catch (error) { return error.name + ":" + error.message; }
  }
  var receiveRole = message(function () {
    $262.agent.receiveBroadcast({ valueOf: function () { throw "touched"; } });
  });
  var invalidFirst = message(function () {
    $262.agent.broadcast({}, { valueOf: function () {
      invalidFirstTouched += 1; return 1;
    } });
  });
  var fixed = new SharedArrayBuffer(4);
  $262.agent.broadcast(fixed, { valueOf: function () {
    validValueTouched += 1; return 4294967297;
  } });
  var bigint = message(function () { $262.agent.broadcast(fixed, 1n); });
  var ordinary = message(function () {
    $262.agent.broadcast(new ArrayBuffer(4), { valueOf: function () {
      ordinaryTouched += 1; return 1;
    } });
  });
  var growable = message(function () {
    $262.agent.broadcast(new SharedArrayBuffer(4, { maxByteLength: 8 }), {
      valueOf: function () { growableTouched += 1; return 1; }
    });
  });
  return receiveRole + "|" + invalidFirst + "|" + invalidFirstTouched +
         "|" + validValueTouched + "|" + bigint + "|" + ordinary + "|" +
         ordinaryTouched + "|" + growable + "|" + growableTouched;
})()"#,
            ),
            "TypeError:must be called inside an agent|TypeError:ArrayBuffer object expected|0|1|TypeError:cannot convert bigint to number|TypeError:ordinary ArrayBuffer broadcast is unavailable across runtimes|0|TypeError:growable SharedArrayBuffer broadcast is unavailable across runtimes|0"
        );

        context
            .eval(
                r#"var replacementBuffer = new SharedArrayBuffer(4);
$262.agent.start(`
  var roleTouched = 0;
  try {
    $262.agent.broadcast({}, { valueOf: function () { roleTouched += 1; } });
  } catch (error) {
    $262.agent.report("role:" + error.message + ":" + roleTouched);
  }
  $262.agent.receiveBroadcast(function () { $262.agent.report("old"); });
  $262.agent.receiveBroadcast(function (sab, value) {
    $262.agent.report("new:" + value);
  });
  try { $262.agent.receiveBroadcast(0); }
  catch (error) { $262.agent.report("callable:" + error.message); }
  $262.agent.leaving();
  $262.agent.report("replacement-ready");
`);"#,
            )
            .unwrap();
        wait_for_report(&session, "replacement-ready");
        context
            .eval("$262.agent.broadcast(replacementBuffer, -4294967295)")
            .unwrap();
        session.join_workers().unwrap();
        assert_eq!(
            take_reports(&session),
            [
                "role:cannot be called inside an agent:0",
                "callable:expecting function",
                "replacement-ready",
                "new:1",
            ]
        );
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn fixed_shared_backing_preserves_int32_and_bigint_across_runtimes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        context
            .eval(
                r#"var numericShared = new SharedArrayBuffer(16);
var numericInts = new Int32Array(numericShared, 0, 1);
var numericBigs = new BigInt64Array(numericShared, 8, 1);
numericInts[0] = 40;
numericBigs[0] = 9007199254740993n;
$262.agent.start(`
  $262.agent.receiveBroadcast(function (sab, value) {
    var ints = new Int32Array(sab, 0, 1);
    var bigs = new BigInt64Array(sab, 8, 1);
    $262.agent.report(ints[0] + ":" + bigs[0] + ":" + value);
    Atomics.add(ints, 0, 2);
    Atomics.add(bigs, 0, 3n);
  });
  $262.agent.report("numeric-ready");
`);"#,
            )
            .unwrap();
        wait_for_report(&session, "numeric-ready");
        context
            .eval("$262.agent.broadcast(numericShared, 4294967297)")
            .unwrap();
        session.join_workers().unwrap();
        assert_eq!(
            take_reports(&session),
            ["numeric-ready", "40:9007199254740993:1"]
        );
        assert_eq!(
            eval_string(&mut context, "numericInts[0] + ':' + numericBigs[0]"),
            "42:9007199254740996"
        );
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn callback_jobs_can_register_the_next_broadcast_generation() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        context
            .eval(
                r#"var generationBuffer = new SharedArrayBuffer(4);
$262.agent.start(`
  $262.agent.receiveBroadcast(function (sab, value) {
    $262.agent.report("first:" + value);
    Promise.resolve().then(function () {
      $262.agent.receiveBroadcast(function (nextSab, nextValue) {
        $262.agent.report("second:" + nextValue + ":" + nextSab.byteLength);
      });
      $262.agent.report("rearmed");
    });
  });
  $262.agent.report("generation-ready");
`);"#,
            )
            .unwrap();
        wait_for_report(&session, "generation-ready");
        context
            .eval(
                "$262.agent.broadcast(generationBuffer, 1); \
                 $262.agent.broadcast(generationBuffer, 2);",
            )
            .unwrap();
        session.join_workers().unwrap();
        assert_eq!(
            take_reports(&session),
            ["generation-ready", "first:1", "rearmed", "second:2:4",]
        );
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn synchronous_callback_replacement_is_discarded_after_the_current_call() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        context
            .eval(
                r#"var synchronousReplacementBuffer = new SharedArrayBuffer(4);
$262.agent.start(`
  $262.agent.receiveBroadcast(function () {
    $262.agent.receiveBroadcast(function () {
      $262.agent.report("synchronous-replacement-ran");
    });
    $262.agent.report("synchronous-current-ran");
  });
  $262.agent.report("synchronous-ready");
`);"#,
            )
            .unwrap();
        wait_for_report(&session, "synchronous-ready");
        context
            .eval("$262.agent.broadcast(synchronousReplacementBuffer, 1)")
            .unwrap();
        session.join_workers().unwrap();
        assert_eq!(
            take_reports(&session),
            ["synchronous-ready", "synchronous-current-ran"]
        );
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn worker_and_callback_exceptions_still_drain_jobs_and_clean_up_join() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let session = Test262AgentSession::new();
        context.install_test262_host_with_agent(&session).unwrap();
        context
            .eval(
                r#"var failureBuffer = new SharedArrayBuffer(4);
$262.agent.start(`
  $262.agent.receiveBroadcast(function () {
    $262.agent.report("callback-ran");
    Promise.resolve().then(function () { $262.agent.report("callback-job"); });
    throw new Error("callback failure");
  });
  $262.agent.report("failure-ready");
  throw new Error("source failure");
`);"#,
            )
            .unwrap();
        wait_for_report(&session, "failure-ready");
        context
            .eval("$262.agent.broadcast(failureBuffer, 0)")
            .unwrap();
        let error = session.join_workers().unwrap_err().to_string();
        assert!(error.contains("execute agent source"), "{error}");
        assert!(error.contains("call agent broadcast callback"), "{error}");
        assert_eq!(
            take_reports(&session),
            ["failure-ready", "callback-ran", "callback-job"]
        );
        session.join_workers().unwrap();
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

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn agent_source_rejects_dynamic_import_in_every_compile_path() {
        for source in [
            "import('./fixture.js')",
            "eval(\"import('./fixture.js')\")",
            "Function(\"return import('./fixture.js')\")",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let session = Test262AgentSession::new();
            context.install_test262_host_with_agent(&session).unwrap();
            context
                .eval(&format!(
                    "$262.agent.start({:?})",
                    format!("try {{ {source} }} catch (_) {{}}")
                ))
                .unwrap();
            let error = session.join_workers().unwrap_err().to_string();
            assert!(
                error.contains("dynamic-import bytecode policy"),
                "{source}: {error}"
            );
        }
    }
}
