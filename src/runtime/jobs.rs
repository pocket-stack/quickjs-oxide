//! Runtime-wide FIFO job queue.
//!
//! QuickJS keeps jobs on `JSRuntime`, not on a realm, and asks the host to
//! execute one job at a time.  Evaluation therefore never drains this queue
//! implicitly: CLI and Test262 hosts opt in at their own boundary.

use super::*;
use crate::heap::{PromiseReaction, PromiseReactionKind};

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
}

/// A pending job failure paired with the realm which originated that job.
///
/// QuickJS exposes the same association through the `pctx` out-parameter of
/// `JS_ExecutePendingJob`, including when JavaScript execution throws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingJobError {
    context: ContextId,
    error: RuntimeError,
}

impl PendingJobError {
    #[must_use]
    pub const fn context(&self) -> ContextId {
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

#[derive(Clone, Debug)]
enum PendingJobRoot {
    Context(ContextId),
    Object(ObjectId),
    Value(RawValue),
}

impl PendingJob {
    const fn realm(&self) -> ContextId {
        match self {
            Self::PromiseReaction { realm, .. } | Self::PromiseResolveThenable { realm, .. } => {
                *realm
            }
        }
    }

    fn roots(&self) -> Vec<PendingJobRoot> {
        match self {
            Self::PromiseReaction {
                realm,
                reaction,
                argument,
            } => {
                let mut roots = Vec::with_capacity(6);
                roots.push(PendingJobRoot::Context(*realm));
                if let Some(handler) = reaction.handler {
                    roots.push(PendingJobRoot::Object(handler));
                }
                if let Some(capability) = reaction.capability {
                    roots.push(PendingJobRoot::Object(capability.resolve));
                    roots.push(PendingJobRoot::Object(capability.reject));
                }
                roots.push(PendingJobRoot::Value(argument.clone()));
                roots
            }
            Self::PromiseResolveThenable {
                realm,
                promise,
                thenable,
                then,
            } => vec![
                PendingJobRoot::Context(*realm),
                PendingJobRoot::Object(*promise),
                PendingJobRoot::Object(*thenable),
                PendingJobRoot::Object(*then),
            ],
        }
    }
}

impl RuntimeState {
    fn retain_pending_job_root(&mut self, root: &PendingJobRoot) -> Result<(), RuntimeError> {
        match root {
            PendingJobRoot::Context(context) => self.heap.retain_context(*context)?,
            PendingJobRoot::Object(object) => self.heap.retain_object(*object)?,
            PendingJobRoot::Value(value) => self.retain_raw_root(value)?,
        }
        Ok(())
    }

    fn release_pending_job_root(&mut self, root: PendingJobRoot) -> Result<(), RuntimeError> {
        match root {
            PendingJobRoot::Context(context) => {
                let cleanup = self.heap.release_context(context)?;
                self.apply_cleanup(cleanup)?;
            }
            PendingJobRoot::Object(object) => {
                let cleanup = self.heap.release_object(object)?;
                self.apply_cleanup(cleanup)?;
            }
            PendingJobRoot::Value(value) => self.release_owned_raw_root(value)?,
        }
        Ok(())
    }

    pub(super) fn retain_pending_job_roots(
        &mut self,
        job: &PendingJob,
    ) -> Result<(), RuntimeError> {
        let roots = job.roots();
        let mut retained = Vec::with_capacity(roots.len());
        for root in roots {
            if let Err(error) = self.retain_pending_job_root(&root) {
                for retained_root in retained.into_iter().rev() {
                    self.release_pending_job_root(retained_root)?;
                }
                return Err(error);
            }
            retained.push(root);
        }
        Ok(())
    }

    pub(super) fn release_pending_job_roots(
        &mut self,
        job: &PendingJob,
    ) -> Result<(), RuntimeError> {
        for root in job.roots().into_iter().rev() {
            self.release_pending_job_root(root)?;
        }
        Ok(())
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
            .map(|context| context.is_some())
            .map_err(PendingJobError::into_error)
    }

    /// Execute at most one FIFO job and return its originating realm.
    ///
    /// Later and newly-enqueued jobs remain at the FIFO tail. A JavaScript
    /// abrupt completion becomes the runtime's pending exception and is
    /// returned with the same originating realm, matching QuickJS's `pctx`
    /// result from `JS_ExecutePendingJob`.
    pub fn execute_pending_job_with_context(&self) -> Result<Option<ContextId>, PendingJobError> {
        let _operation = self.operation();
        let Some(job) = self.0.state.borrow_mut().pending_jobs.pop_front() else {
            return Ok(None);
        };
        let context = job.realm();

        // The manually retained job roots stay owned until after execution;
        // any borrowed `Value`/`ObjectRef` produced by the handler adds its
        // own temporary occurrence before these queue occurrences disappear.
        let execution = self.execute_pending_job_record(&job);
        let release = self.0.state.borrow_mut().release_pending_job_roots(&job);
        let completion = match (execution, release) {
            (Err(error), _) => return Err(PendingJobError { context, error }),
            (Ok(_), Err(error)) => return Err(PendingJobError { context, error }),
            (Ok(completion), Ok(())) => completion,
        };
        match completion {
            Completion::Return(_) => Ok(Some(context)),
            Completion::Throw(value) => {
                self.set_pending_exception(value)
                    .map_err(|error| PendingJobError { context, error })?;
                Err(PendingJobError {
                    context,
                    error: RuntimeError::Exception,
                })
            }
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
        }
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
}
