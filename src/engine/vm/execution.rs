//! Scoped execution ownership. The registry contains identities, never Values
//! or Runtime-owning frames, and its guard unregisters during Rust unwinding.

use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::value::JsValue;
use crate::engine::vm::frame::FrameStore;
use crate::engine::vm::stack::SlotStore;
use std::cell::{Cell, RefCell};

thread_local! {
    static NEXT_EXECUTION: Cell<u64> = const { Cell::new(1) };
    static ACTIVE_EXECUTIONS: RefCell<Vec<ExecutionRegistration>> = const { RefCell::new(Vec::new()) };
    static HOST_BOUNDARIES: RefCell<Vec<HostBoundary>> = const { RefCell::new(Vec::new()) };
}

pub(super) struct ExecutionLimits {
    pub frames: usize,
    pub slots: usize,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        // Native recursion still uses its existing budget during migration.
        // Arena sizes are fallible and bounded by Rust's addressable storage;
        // S04 adds the explicit-call budget at the frame push boundary.
        Self {
            frames: u16::MAX as usize,
            slots: isize::MAX as usize
                / std::mem::size_of::<Option<crate::engine::vm::bindings::FrameBinding>>(),
        }
    }
}

impl ExecutionLimits {
    /// JavaScript-frame ceiling sampled from the runtime configuration. The
    /// slot budget keeps its default; the native host-stack budget is separate.
    pub(super) fn for_runtime(runtime: &Runtime) -> Self {
        Self {
            frames: runtime.recursion_limit(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExecutionRegistration {
    domain: u64,
    id: u64,
    host_boundary: Option<u64>,
}

#[derive(Clone, Copy)]
struct HostBoundary {
    domain: u64,
    id: u64,
    parent_execution: Option<u64>,
    parent_frame: Option<super::frames::ActiveFrameToken>,
}

fn next_identity() -> Result<u64, Error> {
    NEXT_EXECUTION.with(|next| {
        let id = next.get();
        next.set(
            id.checked_add(1)
                .ok_or_else(|| Error::internal("execution identity exhausted"))?,
        );
        Ok(id)
    })
}

fn active_execution(domain: u64) -> Option<u64> {
    ACTIVE_EXECUTIONS.with(|active| {
        active
            .borrow()
            .iter()
            .rev()
            .find(|entry| entry.domain == domain)
            .map(|entry| entry.id)
    })
}

/// A real synchronous host callback may create another entry, but cannot
/// transfer ownership of its suspended parent's frames. This guard records
/// identities only; all registry and heap borrows end before calling host code.
/// Internal native calls do not create these delimiters.
#[must_use]
pub(crate) struct HostBoundaryGuard {
    boundary: HostBoundary,
}

impl HostBoundaryGuard {
    pub(crate) fn enter(runtime: &Runtime) -> Result<Self, Error> {
        let domain = runtime.domain_id();
        let boundary = HostBoundary {
            domain,
            id: next_identity()?,
            parent_execution: active_execution(domain),
            parent_frame: runtime
                .0
                .state
                .borrow()
                .active_frames
                .last()
                .map(|frame| frame.token),
        };
        HOST_BOUNDARIES.with(|boundaries| -> Result<(), Error> {
            let mut boundaries = boundaries.borrow_mut();
            boundaries
                .try_reserve(1)
                .map_err(|_| Error::internal("host boundary registration allocation failed"))?;
            boundaries.push(boundary);
            Ok(())
        })?;
        Ok(Self { boundary })
    }

    pub(crate) fn finish(self, runtime: &Runtime) -> Result<(), Error> {
        let boundary = self.boundary;
        if runtime.domain_id() != boundary.domain {
            return Err(Error::internal("host boundary belongs to another runtime"));
        }
        let is_current = HOST_BOUNDARIES.with(|boundaries| {
            boundaries.borrow().last().is_some_and(|current| {
                current.id == boundary.id && current.domain == boundary.domain
            })
        });
        if !is_current {
            return Err(Error::internal("host boundaries returned out of order"));
        }
        let has_child = ACTIVE_EXECUTIONS.with(|active| {
            active.borrow().iter().any(|entry| {
                entry.domain == boundary.domain && entry.host_boundary == Some(boundary.id)
            })
        });
        if has_child || active_execution(boundary.domain) != boundary.parent_execution {
            return Err(Error::internal(
                "host callback left an execution registered",
            ));
        }
        let parent_frame = runtime
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .map(|frame| frame.token);
        if parent_frame != boundary.parent_frame {
            return Err(Error::internal(
                "host callback did not restore its parent frame",
            ));
        }
        Ok(())
    }
}

impl Drop for HostBoundaryGuard {
    fn drop(&mut self) {
        HOST_BOUNDARIES.with(|boundaries| {
            let mut boundaries = boundaries.borrow_mut();
            if let Some(index) = boundaries.iter().rposition(|boundary| {
                boundary.domain == self.boundary.domain && boundary.id == self.boundary.id
            }) {
                boundaries.remove(index);
            }
        });
    }
}

struct ExecutionGuard {
    registration: ExecutionRegistration,
}

impl ExecutionGuard {
    fn enter(runtime: &Runtime) -> Result<Self, Error> {
        let id = next_identity()?;
        let domain = runtime.domain_id();
        let parent = active_execution(domain);
        let host_boundary = HOST_BOUNDARIES.with(|boundaries| {
            boundaries
                .borrow()
                .iter()
                .rev()
                .find(|boundary| boundary.domain == domain)
                .filter(|boundary| boundary.parent_execution == parent)
                .map(|boundary| boundary.id)
        });
        if parent.is_some() && host_boundary.is_none() {
            return Err(Error::internal(
                "internal callback attempted a nested root execution",
            ));
        }
        let registration = ExecutionRegistration {
            domain,
            id,
            host_boundary,
        };
        ACTIVE_EXECUTIONS.with(|active| -> Result<(), Error> {
            let mut active = active.borrow_mut();
            active
                .try_reserve(1)
                .map_err(|_| Error::internal("execution registration allocation failed"))?;
            active.push(registration);
            Ok(())
        })?;
        Ok(Self { registration })
    }
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        ACTIVE_EXECUTIONS.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(index) = active.iter().rposition(|entry| *entry == self.registration) {
                active.remove(index);
            }
        });
    }
}

pub(super) struct RunningExecution {
    pub frames: FrameStore,
    pub slots: SlotStore,
    pub query_storage: super::proxy_get_driver::QueryStorage,
    pub call_storage: super::frame::CallStorage,
    /// Cold completion owns its payload before the active window is cleared.
    pub pending: Option<JsValue>,
    /// Retained GetField2 result's classification, consumed by the immediate Call.
    pub selected_native: Option<crate::engine::object::LinkedNativeSelection>,
    /// A typed root terminal result; never represented by a manufactured JS Value.
    pub root_descriptor: Option<super::entry::DescriptorReply>,
    pub root_query: Option<Box<super::proxy_get_driver::PendingProxyGet>>,
    // The execution never keeps its runtime alive; teardown releases through
    // the upgrade only when the runtime still exists.
    runtime: std::rc::Weak<crate::engine::heap::runtime::RuntimeInner>,
    _guard: ExecutionGuard,
}

impl Drop for RunningExecution {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime.upgrade().map(Runtime) else {
            // The runtime (and its whole heap) died first; no edge release can
            // observe anything. Discard the storage without accounting.
            self.pending = None;
            self.slots = SlotStore::new(0);
            return;
        };
        if let Some(pending) = self.pending.take() {
            // Teardown cannot report errors; invariant violations surface at
            // the deferred-drain boundary like every trusted release.
            let _ = runtime.release_jsvalue(pending);
        }
        while let Some(mut frame) = self.frames.pop_current() {
            // Clear this child's captures and operands while its activation
            // and every enclosing native query still own their roots.
            if self.slots.clear_frame(&runtime, frame.window.take()).is_err() {
                // A failed legacy handoff may have detached its Frame before
                // an allocation failure. Release any remaining arena owners
                // before unwinding parent native activations; never panic here.
                self.slots = SlotStore::new(0);
            }
            drop(frame.cold);
        }
        drop(self.root_query.take());
    }
}

impl RunningExecution {
    pub(super) fn new(runtime: &Runtime, limits: ExecutionLimits) -> Result<Self, Error> {
        let guard = ExecutionGuard::enter(runtime)?;
        Ok(Self {
            frames: FrameStore::new(guard.registration.id, limits.frames),
            slots: SlotStore::new(limits.slots),
            query_storage: super::proxy_get_driver::QueryStorage::default(),
            call_storage: super::frame::CallStorage::default(),
            pending: None,
            selected_native: None,
            root_query: None,
            root_descriptor: None,
            runtime: std::rc::Rc::downgrade(&runtime.0),
            _guard: guard,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ACTIVE_EXECUTIONS, ExecutionLimits, HOST_BOUNDARIES, HostBoundaryGuard, RunningExecution,
    };
    use crate::engine::api::Runtime;
    use std::rc::Rc;

    #[test]
    fn nested_execution_registration_is_removed_on_panic() {
        let runtime = Runtime::new();
        let outer = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let before = ACTIVE_EXECUTIONS.with(|active| active.borrow().clone());
        assert_eq!(before.len(), 1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _boundary = HostBoundaryGuard::enter(&runtime).unwrap();
            let _inner = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            assert_eq!(ACTIVE_EXECUTIONS.with(|active| active.borrow().len()), 2);
            panic!("exercise execution guard unwinding");
        }));
        assert!(result.is_err());
        assert_eq!(
            ACTIVE_EXECUTIONS.with(|active| active.borrow().clone()),
            before
        );
        drop(outer);
        assert!(ACTIVE_EXECUTIONS.with(|active| active.borrow().is_empty()));
    }

    #[test]
    fn identity_registration_cannot_keep_a_runtime_alive() {
        let runtime = Runtime::new();
        let weak = Rc::downgrade(&runtime.0);
        let execution = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let boundary = HostBoundaryGuard::enter(&runtime).unwrap();
        drop(runtime);
        assert!(weak.upgrade().is_none());
        drop(execution);
        drop(boundary);
        assert!(HOST_BOUNDARIES.with(|boundaries| boundaries.borrow().is_empty()));
        assert!(ACTIVE_EXECUTIONS.with(|active| active.borrow().is_empty()));
    }
    #[test]
    fn host_boundary_uses_parent_entry_and_runtime_identity() {
        let runtime = Runtime::new();
        let other_runtime = Runtime::new();
        let outer = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let boundary = HostBoundaryGuard::enter(&runtime).unwrap();
        let inner = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        assert_eq!(
            inner._guard.registration.host_boundary,
            Some(boundary.boundary.id)
        );
        let internal = RunningExecution::new(&runtime, ExecutionLimits::default());
        assert!(
            matches!(internal, Err(error) if error.message() == "internal callback attempted a nested root execution")
        );
        let foreign = RunningExecution::new(&other_runtime, ExecutionLimits::default()).unwrap();
        assert_eq!(foreign._guard.registration.host_boundary, None);
        drop((foreign, inner));
        boundary.finish(&runtime).unwrap();
        assert!(HOST_BOUNDARIES.with(|boundaries| boundaries.borrow().is_empty()));
        drop(outer);
    }

    #[test]
    fn host_boundary_restores_registry_after_callback_panic() {
        let runtime = Runtime::new();
        let outer = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        let before = ACTIVE_EXECUTIONS.with(|active| active.borrow().clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _boundary = HostBoundaryGuard::enter(&runtime).unwrap();
            let _inner = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
            panic!("exercise host callback unwinding");
        }));
        assert!(result.is_err());
        assert_eq!(
            ACTIVE_EXECUTIONS.with(|active| active.borrow().clone()),
            before
        );
        assert!(HOST_BOUNDARIES.with(|boundaries| boundaries.borrow().is_empty()));
        drop(outer);
    }

    #[test]
    fn host_boundary_rejects_a_live_reentry_on_return() {
        let runtime = Runtime::new();
        let boundary = HostBoundaryGuard::enter(&runtime).unwrap();
        let inner = RunningExecution::new(&runtime, ExecutionLimits::default()).unwrap();
        assert_eq!(
            boundary.finish(&runtime).unwrap_err().message(),
            "host callback left an execution registered"
        );
        assert!(HOST_BOUNDARIES.with(|boundaries| boundaries.borrow().is_empty()));
        drop(inner);
        assert!(ACTIVE_EXECUTIONS.with(|active| active.borrow().is_empty()));
    }

    #[test]
    fn module_host_reentry_preserves_owned_parent_bindings_and_unwinds() {
        use crate::engine::api::{
            Context, JsString, ModuleImportAttributes, ModuleLoadResult, ModuleLoader,
            ModuleLoaderError, Value,
        };
        use std::cell::{Cell, RefCell};

        #[derive(Clone, Copy, Debug)]
        enum Outcome {
            Return,
            Reject,
            Panic,
        }
        #[derive(Debug)]
        struct Loader {
            outcome: Outcome,
            calls: Rc<Cell<usize>>,
        }
        impl ModuleLoader for Loader {
            fn load(
                &self,
                context: &mut Context,
                _name: &JsString,
                _attributes: &ModuleImportAttributes,
            ) -> Result<ModuleLoadResult, ModuleLoaderError> {
                self.calls.set(self.calls.get() + 1);
                let before = ACTIVE_EXECUTIONS.with(|active| active.borrow().clone());
                let boundary =
                    HOST_BOUNDARIES.with(|boundaries| *boundaries.borrow().last().unwrap());
                // Host-triggered compilation runs while the bytecode caller
                // retains its captured bindings in the owned slot arena.
                assert_eq!(before.len(), 1);
                assert_eq!(boundary.parent_execution, Some(before[0].id));
                assert!(context.runtime().0.state.borrow().active_frames.len() >= 2);
                assert_eq!(context.eval("reenter()").unwrap(), Value::Int(41));
                context.runtime().run_gc().unwrap();
                assert_eq!(
                    ACTIVE_EXECUTIONS.with(|active| active.borrow().clone()),
                    before
                );
                match self.outcome {
                    Outcome::Return => Ok(ModuleLoadResult::SourceText("export {};".to_owned())),
                    Outcome::Reject => Err(ModuleLoaderError::exception(Value::Int(99))),
                    Outcome::Panic => panic!("exercise a real module-host callback panic"),
                }
            }
        }

        for outcome in [Outcome::Return, Outcome::Reject, Outcome::Panic] {
            let runtime = Runtime::new();
            let calls = Rc::new(Cell::new(0));
            let _registration = runtime.set_module_loader(Loader {
                outcome,
                calls: calls.clone(),
            });
            let mut context = runtime.new_context();
            let trigger = context.eval("Promise.reject.bind(Promise)").unwrap();
            let Value::Object(function) = context
                .eval(
                    r#"
                var reenter;
                (function(trigger){
                    let captured=1;
                    reenter=function(){captured=41;return captured};
                    var pending=trigger(1);
                    return captured+1;
                })
            "#,
                )
                .unwrap()
            else {
                panic!("expected function")
            };
            let callable = runtime.as_callable(&function).unwrap().unwrap();
            // The existing rejection-tracker ABI is the synchronous host
            // trigger. Module compilation then enters the guarded loader;
            // dynamic import itself would defer loading to a later job.
            let host_context = RefCell::new(context.clone());
            struct ClearTracker(Runtime);
            impl Drop for ClearTracker {
                fn drop(&mut self) {
                    self.0.clear_host_promise_rejection_tracker();
                }
            }
            let tracker_guard = ClearTracker(runtime.clone());
            runtime.set_host_promise_rejection_tracker(move |event| {
                if event.is_handled() {
                    return;
                }
                let mut context = host_context.borrow_mut();
                let result = context
                    .compile_module_with_filename("import './owned-host.js';", "host-entry.js");
                match outcome {
                    Outcome::Return => {
                        result.unwrap();
                    }
                    Outcome::Reject => {
                        assert!(result.is_err());
                        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(99)));
                    }
                    Outcome::Panic => unreachable!("loader should have panicked"),
                }
            });
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                context.call(&callable, Value::Undefined, &[trigger])
            }));
            drop(tracker_guard);
            match outcome {
                Outcome::Panic => {
                    let payload = result.expect_err("expected host callback panic");
                    assert_eq!(
                        payload.downcast_ref::<&str>().copied(),
                        Some("exercise a real module-host callback panic")
                    );
                }
                _ => assert_eq!(result.unwrap().unwrap(), Value::Int(42)),
            }
            assert_eq!(calls.get(), 1);
            assert!(ACTIVE_EXECUTIONS.with(|active| active.borrow().is_empty()));
            assert!(HOST_BOUNDARIES.with(|boundaries| boundaries.borrow().is_empty()));
            assert!(runtime.0.state.borrow().active_frames.is_empty());
            assert_eq!(runtime.0.module_host_callback_depth.get(), 0);
            assert_eq!(context.eval("reenter()").unwrap(), Value::Int(41));
            assert_eq!(context.eval("6*7").unwrap(), Value::Int(42));
        }
    }
}
