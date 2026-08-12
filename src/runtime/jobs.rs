//! Runtime-wide FIFO job queue.
//!
//! QuickJS keeps jobs on `JSRuntime`, not on a realm, and asks the host to
//! execute one job at a time.  Evaluation therefore never drains this queue
//! implicitly: CLI and Test262 hosts opt in at their own boundary.

use super::intrinsics::promise::RootedPromiseCapability;
use super::*;
use crate::heap::{
    FinalizationJobSink, PreparedFinalizationJob, PromiseReaction, PromiseReactionKind,
};

#[derive(Clone, Debug)]
pub(super) enum PendingJob {
    PromiseReaction {
        realm: ContextId,
        reaction: PromiseReaction,
        argument: RawValue,
    },
    PromiseResolveThenable {
        realm: ContextId,
        promise: ObjectId,
        thenable: ObjectId,
        then: ObjectId,
    },
    FinalizationRegistryCleanup {
        realm: ContextId,
        callback: ObjectId,
        held_value: RawValue,
    },
    DynamicImportLoad {
        realm: ContextId,
        resolve: ObjectId,
        reject: ObjectId,
        base_name: Option<JsString>,
        specifier: JsString,
        attributes: ModuleImportAttributes,
    },
}

/// Direct adapter from the heap's ordered weak-object pass into the runtime
/// FIFO. A successful reserve makes `publish_preowned` infallible; the job's
/// roots were already retained/transferred by the heap and must not pass
/// through the ordinary retaining enqueue path again.
pub(super) struct RuntimeFinalizationJobSink<'a> {
    queue: &'a mut VecDeque<PendingJob>,
}

impl<'a> RuntimeFinalizationJobSink<'a> {
    pub(super) const fn new(queue: &'a mut VecDeque<PendingJob>) -> Self {
        Self { queue }
    }
}

impl FinalizationJobSink for RuntimeFinalizationJobSink<'_> {
    fn try_reserve_one(&mut self) -> bool {
        self.queue.try_reserve(1).is_ok()
    }

    fn publish_preowned(&mut self, job: PreparedFinalizationJob) {
        self.queue
            .push_back(PendingJob::FinalizationRegistryCleanup {
                realm: job.realm,
                callback: job.callback,
                held_value: job.held_value,
            });
    }
}

/// Result of attempting to execute one runtime-wide pending job.
///
/// `Executed { context: None }` is distinct from `NoJob`: pinned QuickJS can
/// execute a job successfully after its realm's last non-job reference has
/// disappeared, in which case `JS_ExecutePendingJob` returns `1` while storing
/// `NULL` in its obsolete `pctx` out-parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingJobOutcome {
    NoJob,
    Executed { context: Option<ContextId> },
}

impl PendingJobOutcome {
    #[must_use]
    pub const fn executed(self) -> bool {
        matches!(self, Self::Executed { .. })
    }

    /// Realm still alive after the queue's owned roots were released.
    #[must_use]
    pub const fn context(self) -> Option<ContextId> {
        match self {
            Self::NoJob => None,
            Self::Executed { context } => context,
        }
    }
}

/// A pending job failure paired with the realm which originated that job.
///
/// QuickJS exposes the same association through the `pctx` out-parameter of
/// `JS_ExecutePendingJob`, including when JavaScript execution throws. The
/// association is absent when releasing the job's roots destroys its realm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingJobError {
    context: Option<ContextId>,
    error: RuntimeError,
}

impl PendingJobError {
    #[must_use]
    pub const fn context(&self) -> Option<ContextId> {
        self.context
    }

    #[must_use]
    pub const fn error(&self) -> &RuntimeError {
        &self.error
    }

    #[must_use]
    pub fn into_error(self) -> RuntimeError {
        self.error
    }
}

impl std::fmt::Display for PendingJobError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.error, formatter)
    }
}

impl std::error::Error for PendingJobError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

const MAX_PENDING_JOB_ROOTS: usize = 6;

#[derive(Clone, Copy, Debug)]
enum PendingJobRoot<'a> {
    Context(ContextId),
    Object(ObjectId),
    Value(&'a RawValue),
}

impl PendingJob {
    const fn realm(&self) -> ContextId {
        match self {
            Self::PromiseReaction { realm, .. }
            | Self::PromiseResolveThenable { realm, .. }
            | Self::FinalizationRegistryCleanup { realm, .. }
            | Self::DynamicImportLoad { realm, .. } => *realm,
        }
    }

    fn roots(&self) -> [Option<PendingJobRoot<'_>>; MAX_PENDING_JOB_ROOTS] {
        let mut roots = [None; MAX_PENDING_JOB_ROOTS];
        let mut root_count = 0usize;
        let mut push = |root| {
            debug_assert!(root_count < roots.len());
            roots[root_count] = Some(root);
            root_count += 1;
        };
        match self {
            Self::PromiseReaction {
                realm,
                reaction,
                argument,
            } => {
                push(PendingJobRoot::Context(*realm));
                if let Some(handler) = reaction.handler {
                    push(PendingJobRoot::Object(handler));
                }
                if let Some(capability) = reaction.capability {
                    push(PendingJobRoot::Object(capability.resolve));
                    push(PendingJobRoot::Object(capability.reject));
                }
                push(PendingJobRoot::Value(argument));
            }
            Self::PromiseResolveThenable {
                realm,
                promise,
                thenable,
                then,
            } => {
                push(PendingJobRoot::Context(*realm));
                push(PendingJobRoot::Object(*promise));
                push(PendingJobRoot::Object(*thenable));
                push(PendingJobRoot::Object(*then));
            }
            Self::FinalizationRegistryCleanup {
                realm,
                callback,
                held_value,
            } => {
                push(PendingJobRoot::Context(*realm));
                push(PendingJobRoot::Object(*callback));
                push(PendingJobRoot::Value(held_value));
            }
            Self::DynamicImportLoad {
                realm,
                resolve,
                reject,
                ..
            } => {
                push(PendingJobRoot::Context(*realm));
                push(PendingJobRoot::Object(*resolve));
                push(PendingJobRoot::Object(*reject));
            }
        }
        roots
    }
}

impl RuntimeState {
    fn retain_pending_job_root(&mut self, root: PendingJobRoot<'_>) -> Result<(), RuntimeError> {
        match root {
            PendingJobRoot::Context(context) => self.heap.retain_context(context)?,
            PendingJobRoot::Object(object) => self.heap.retain_object(object)?,
            PendingJobRoot::Value(value) => self.retain_raw_root(value)?,
        }
        Ok(())
    }

    fn release_pending_job_root(&mut self, root: PendingJobRoot<'_>) -> Result<(), RuntimeError> {
        match root {
            PendingJobRoot::Context(context) => {
                let cleanup = self.heap.release_context(context)?;
                self.apply_cleanup(cleanup)?;
            }
            PendingJobRoot::Object(object) => {
                let cleanup = self.heap.release_object(object)?;
                self.apply_cleanup(cleanup)?;
            }
            PendingJobRoot::Value(value) => match value {
                RawValue::Object(object) => {
                    let cleanup = self.heap.release_object(*object)?;
                    self.apply_cleanup(cleanup)?;
                }
                RawValue::Symbol(atom) => {
                    self.atoms.release(*atom)?;
                }
                RawValue::Private(_) => {
                    return Err(RuntimeError::Invariant(
                        "private-name identity occupied a pending job root",
                    ));
                }
                RawValue::Undefined
                | RawValue::Null
                | RawValue::Bool(_)
                | RawValue::Int(_)
                | RawValue::Float(_)
                | RawValue::BigInt(_)
                | RawValue::String(_) => {}
                RawValue::Uninitialized | RawValue::Exception => {
                    return Err(RuntimeError::Invariant(
                        "internal value sentinel occupied a pending job root",
                    ));
                }
            },
        }
        Ok(())
    }

    pub(super) fn retain_pending_job_roots(
        &mut self,
        job: &PendingJob,
    ) -> Result<(), RuntimeError> {
        let roots = job.roots();
        for (retained, root) in roots.iter().flatten().copied().enumerate() {
            if let Err(error) = self.retain_pending_job_root(root) {
                for retained_root in roots[..retained].iter().rev().flatten().copied() {
                    self.release_pending_job_root(retained_root)?;
                }
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) fn release_pending_job_roots(
        &mut self,
        job: &PendingJob,
    ) -> Result<(), RuntimeError> {
        self.release_pending_job_roots_with_context(job).map(drop)
    }

    /// Release roots in the reverse of their retain order and report whether
    /// the explicit realm root had another owner immediately before it was
    /// released. This is pinned QuickJS's `js_rc(ctx)->ref_count > 1` check.
    fn release_pending_job_roots_with_context(
        &mut self,
        job: &PendingJob,
    ) -> Result<Option<ContextId>, RuntimeError> {
        let roots = job.roots();
        let mut context_after_release = None;
        for root in roots.iter().rev().flatten().copied() {
            if let PendingJobRoot::Context(context) = root {
                let survives = self.heap.context_strong_count(context)? > 1;
                self.release_pending_job_root(root)?;
                context_after_release = Some(survives.then_some(context));
            } else {
                self.release_pending_job_root(root)?;
            }
        }
        context_after_release.ok_or(RuntimeError::Invariant(
            "pending job had no explicit realm root",
        ))
    }
}

impl Runtime {
    pub(super) fn enqueue_pending_job(&self, job: PendingJob) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        let mut state = self.0.state.borrow_mut();
        state.retain_pending_job_roots(&job)?;
        state.pending_jobs.push_back(job);
        Ok(())
    }

    /// Return whether QuickJS's runtime-wide FIFO contains a pending job.
    #[must_use]
    pub fn is_job_pending(&self) -> bool {
        let _operation = self.operation();
        !self.0.state.borrow().pending_jobs.is_empty()
    }

    /// Execute at most one pending job and report whether one was present.
    ///
    /// This convenience adapter preserves the original compact API. Embedders
    /// with multiple contexts should use
    /// [`Runtime::execute_pending_job_with_context`] so exceptions remain
    /// associated with the realm which originated the job.
    pub fn execute_pending_job(&self) -> Result<bool, RuntimeError> {
        self.execute_pending_job_with_context()
            .map(PendingJobOutcome::executed)
            .map_err(PendingJobError::into_error)
    }

    /// Execute at most one FIFO job and report its surviving originating realm.
    ///
    /// Later and newly-enqueued jobs remain at the FIFO tail. A JavaScript
    /// abrupt completion becomes the runtime's pending exception and is
    /// returned with the same optional originating realm, matching both the
    /// integer result and obsolete `pctx` out-parameter of
    /// `JS_ExecutePendingJob`.
    pub fn execute_pending_job_with_context(&self) -> Result<PendingJobOutcome, PendingJobError> {
        let _operation = self.operation();
        let Some(job) = self.0.state.borrow_mut().pending_jobs.pop_front() else {
            return Ok(PendingJobOutcome::NoJob);
        };
        let context = job.realm();

        // QuickJS frees a successful result, or leaves a thrown value in the
        // runtime exception slot, before testing whether the job realm has any
        // owner besides the queue. Do the same before releasing the argv-like
        // roots so a discarded return value cannot keep `pctx` spuriously live.
        let execution =
            self.execute_pending_job_record(&job)
                .and_then(|completion| match completion {
                    Completion::Return(value) => {
                        drop(value);
                        Ok(false)
                    }
                    Completion::Throw(value) => {
                        self.set_pending_exception(value)?;
                        Ok(true)
                    }
                });
        let release = self
            .0
            .state
            .borrow_mut()
            .release_pending_job_roots_with_context(&job);
        let (threw, context) = match (execution, release) {
            (Err(error), Ok(context)) => return Err(PendingJobError { context, error }),
            (Err(error), Err(_)) => {
                return Err(PendingJobError {
                    context: Some(context),
                    error,
                });
            }
            (Ok(_), Err(error)) => {
                return Err(PendingJobError {
                    context: Some(context),
                    error,
                });
            }
            (Ok(threw), Ok(context)) => (threw, context),
        };
        if threw {
            Err(PendingJobError {
                context,
                error: RuntimeError::Exception,
            })
        } else {
            Ok(PendingJobOutcome::Executed { context })
        }
    }

    fn execute_pending_job_record(&self, job: &PendingJob) -> Result<Completion, RuntimeError> {
        match job {
            PendingJob::PromiseReaction {
                realm,
                reaction,
                argument,
            } => self.execute_promise_reaction_job(*realm, reaction, argument),
            PendingJob::PromiseResolveThenable {
                realm,
                promise,
                thenable,
                then,
            } => self.execute_promise_resolve_thenable_job(*realm, *promise, *thenable, *then),
            PendingJob::FinalizationRegistryCleanup {
                realm,
                callback,
                held_value,
            } => self.execute_finalization_registry_cleanup_job(*realm, *callback, held_value),
            PendingJob::DynamicImportLoad {
                realm,
                resolve,
                reject,
                base_name,
                specifier,
                attributes,
            } => self.execute_dynamic_import_load_job(
                *realm,
                *resolve,
                *reject,
                base_name.as_ref(),
                specifier,
                attributes,
            ),
        }
    }

    fn execute_finalization_registry_cleanup_job(
        &self,
        realm: ContextId,
        callback: ObjectId,
        held_value: &RawValue,
    ) -> Result<Completion, RuntimeError> {
        let callback = ObjectRef::from_borrowed_handle(self.clone(), callback)?;
        let callback = self.as_callable(&callback)?.ok_or(RuntimeError::Invariant(
            "FinalizationRegistry job callback lost its callable brand",
        ))?;
        let held_value = self.root_raw_value(held_value)?;
        self.call_internal(
            realm,
            &callback,
            Value::Undefined,
            std::slice::from_ref(&held_value),
        )
    }

    pub(super) fn enqueue_promise_reaction_job(
        &self,
        realm: ContextId,
        reaction: PromiseReaction,
        argument: RawValue,
    ) -> Result<(), RuntimeError> {
        let job = self.prepare_promise_reaction_job(realm, reaction, argument)?;
        self.publish_prepared_jobs([job]);
        Ok(())
    }

    pub(super) fn prepare_promise_reaction_job(
        &self,
        realm: ContextId,
        reaction: PromiseReaction,
        argument: RawValue,
    ) -> Result<PendingJob, RuntimeError> {
        debug_assert!(matches!(
            reaction.kind,
            PromiseReactionKind::Fulfill | PromiseReactionKind::Reject
        ));
        let job = PendingJob::PromiseReaction {
            realm,
            reaction,
            argument,
        };
        let _operation = self.operation();
        self.0.state.borrow_mut().retain_pending_job_roots(&job)?;
        Ok(job)
    }

    pub(super) fn publish_prepared_jobs(&self, jobs: impl IntoIterator<Item = PendingJob>) {
        let _operation = self.operation();
        self.0.state.borrow_mut().pending_jobs.extend(jobs);
    }

    pub(super) fn discard_prepared_jobs(
        &self,
        jobs: impl IntoIterator<Item = PendingJob>,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        let mut state = self.0.state.borrow_mut();
        for job in jobs {
            state.release_pending_job_roots(&job)?;
        }
        Ok(())
    }

    pub(super) fn enqueue_promise_resolve_thenable_job(
        &self,
        realm: ContextId,
        promise: ObjectId,
        thenable: ObjectId,
        then: ObjectId,
    ) -> Result<(), RuntimeError> {
        self.enqueue_pending_job(PendingJob::PromiseResolveThenable {
            realm,
            promise,
            thenable,
            then,
        })
    }

    pub(super) fn enqueue_dynamic_import_load_job(
        &self,
        realm: ContextId,
        capability: &RootedPromiseCapability,
        base_name: Option<JsString>,
        specifier: JsString,
        attributes: ModuleImportAttributes,
    ) -> Result<(), RuntimeError> {
        self.enqueue_pending_job(PendingJob::DynamicImportLoad {
            realm,
            resolve: capability.resolve.as_object().object_id(),
            reject: capability.reject.as_object().object_id(),
            base_name,
            specifier,
            attributes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allocate_job_only_realm(runtime: &Runtime, source: ContextId) -> ContextId {
        let roots = {
            let state = runtime.0.state.borrow();
            let source = state.heap.context(source).unwrap();
            (
                source.object_prototype,
                source.function_prototype,
                source.array_prototype,
                source.iterator_prototype,
                source.array_iterator_prototype,
                source.string_iterator_prototype,
                source.global_object,
                source.global_var_object,
            )
        };
        runtime
            .0
            .state
            .borrow_mut()
            .heap
            .allocate_context(ContextData::new(
                roots.0, roots.1, roots.2, roots.3, roots.4, roots.5, roots.6, roots.7,
            ))
            .unwrap()
    }

    fn enqueue_cleanup_with_job_only_realm(
        runtime: &Runtime,
        realm: ContextId,
        callback: &CallableRef,
    ) {
        runtime
            .enqueue_pending_job(PendingJob::FinalizationRegistryCleanup {
                realm,
                callback: callback.as_object().object_id(),
                held_value: RawValue::Undefined,
            })
            .unwrap();
        let mut state = runtime.0.state.borrow_mut();
        assert_eq!(state.heap.context_strong_count(realm), Ok(2));
        let cleanup = state.heap.release_context(realm).unwrap();
        state.apply_cleanup(cleanup).unwrap();
        assert_eq!(state.heap.context_strong_count(realm), Ok(1));
    }

    fn eval_callable(context: &mut Context, source: &str) -> CallableRef {
        let Value::Object(callback) = context.eval(source).unwrap() else {
            panic!("callback source was not an object");
        };
        context
            .runtime()
            .as_callable(&callback)
            .unwrap()
            .expect("callback source was not callable")
    }

    #[test]
    fn pending_job_reports_null_context_after_its_last_realm_root_on_success_and_throw() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        let success = eval_callable(&mut context, "(function () { return 42; })");
        let success_realm = allocate_job_only_realm(&runtime, context.realm_id());
        enqueue_cleanup_with_job_only_realm(&runtime, success_realm, &success);
        assert_eq!(
            runtime.execute_pending_job_with_context().unwrap(),
            PendingJobOutcome::Executed { context: None }
        );
        assert!(matches!(
            runtime.0.state.borrow().heap.context(success_realm),
            Err(HeapError::Stale { .. })
        ));

        let throwing = eval_callable(&mut context, "(function () { throw 17; })");
        let throwing_realm = allocate_job_only_realm(&runtime, context.realm_id());
        enqueue_cleanup_with_job_only_realm(&runtime, throwing_realm, &throwing);
        let error = runtime.execute_pending_job_with_context().unwrap_err();
        assert_eq!(error.context(), None);
        assert_eq!(error.error(), &RuntimeError::Exception);
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(17)));
        assert!(matches!(
            runtime.0.state.borrow().heap.context(throwing_realm),
            Err(HeapError::Stale { .. })
        ));
        assert_eq!(
            runtime.execute_pending_job_with_context().unwrap(),
            PendingJobOutcome::NoJob
        );
    }
}
