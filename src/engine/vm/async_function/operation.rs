//! Async body, intrinsic resolution and settlement are separate owned requests.
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::heap::{
    AsyncFunctionPhase, AsyncFunctionResumeKind, ContextId, InternalCallableData,
};
use crate::engine::object::{CallableRef, ObjectRef};
use crate::engine::value::Value;
use crate::engine::vm::{
    Completion, VmSuspendKind,
    suspend::{EncodedVmActivation, RootedVmActivation, VmActivationResume, VmRunOutcome},
};

pub(crate) enum AsyncStep {
    Complete(Completion),
    Run { resume: Box<AsyncResume> },
    Resolve { resume: Box<AsyncResume> },
    Call { resume: Box<AsyncResume> },
}

pub(crate) struct AsyncResume {
    pending_effect: AsyncStepPending,
    runtime: Runtime,
    state: ObjectRef,
    output: Value,
    phase: Phase,
    active: bool,
}
enum Phase {
    Body,
    Await(Box<EncodedVmActivation>),
    Settled,
}

impl AsyncResume {
    pub(in crate::engine::vm) fn start(
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<Box<Self>, RuntimeError> {
        let capability = runtime.new_default_promise_capability(realm)?;
        let state = runtime.allocate_async_function_state(realm, &capability)?;
        Ok(Box::new(Self {
            pending_effect: AsyncStepPending::default(),
            runtime: runtime.clone(),
            state,
            output: Value::Object(capability.promise),
            phase: Phase::Body,
            active: true,
        }))
    }
    pub(super) fn resumed(runtime: &Runtime, state: ObjectRef) -> Box<Self> {
        Box::new(Self {
            pending_effect: AsyncStepPending::default(),
            runtime: runtime.clone(),
            state,
            output: Value::Undefined,
            phase: Phase::Body,
            active: true,
        })
    }
    pub(crate) fn body(
        mut self: Box<Self>,
        outcome: VmRunOutcome,
    ) -> Result<AsyncStep, RuntimeError> {
        if !matches!(self.phase, Phase::Body) {
            return Err(RuntimeError::Invariant(
                "async body replied in the wrong phase",
            ));
        }
        match outcome {
            VmRunOutcome::Complete(completion) => self.settle(completion),
            VmRunOutcome::Suspend { value, activation } => {
                if activation.kind != VmSuspendKind::Await {
                    return Err(RuntimeError::Invariant(
                        "async function stopped at a non-await suspension",
                    ));
                }
                let realm = self
                    .runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .async_function_state_snapshot(self.state.object_id())?
                    .driver_realm;
                self.phase = Phase::Await(activation);
                Ok({
                    let __pending_field_value = value;
                    let __pending_field_realm = realm;
                    let __pending_field_resume = self;
                    AsyncStep::request_resolve(
                        __pending_field_value,
                        __pending_field_realm,
                        __pending_field_resume,
                    )
                })
            }
        }
    }
    fn settle(mut self: Box<Self>, completion: Completion) -> Result<AsyncStep, RuntimeError> {
        let snapshot = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .async_function_state_snapshot(self.state.object_id())?;
        if snapshot.phase == AsyncFunctionPhase::Completed {
            return Err(RuntimeError::Invariant(
                "async function settled more than once",
            ));
        }
        let (target, value) = match completion {
            Completion::Return(value) => (snapshot.outer_resolve, value),
            Completion::Throw(value) => (snapshot.outer_reject, value),
        };
        let target = ObjectRef::from_borrowed_handle(self.runtime.clone(), target)?;
        let callable = self
            .runtime
            .as_callable(&target)?
            .ok_or(RuntimeError::Invariant(
                "async function outer resolving function is not callable",
            ))?;
        self.runtime.complete_async_function_state(&self.state)?;
        self.active = false;
        self.phase = Phase::Settled;
        Ok({
            let __pending_field_callable = callable;
            let __pending_field_value = value;
            let __pending_field_resume = self;
            AsyncStep::request_call(
                __pending_field_callable,
                __pending_field_value,
                __pending_field_resume,
            )
        })
    }
    pub(crate) fn resume(
        mut self: Box<Self>,
        completion: Completion,
    ) -> Result<AsyncStep, RuntimeError> {
        match std::mem::replace(&mut self.phase, Phase::Body) {
            Phase::Body => self.body(VmRunOutcome::Complete(completion)),
            Phase::Settled => self.finish(), // Consume either JS completion from the internal resolving pair.
            Phase::Await(activation) => {
                let promise = match completion {
                    Completion::Throw(reason) => return self.settle(Completion::Throw(reason)),
                    Completion::Return(Value::Object(promise)) => promise,
                    Completion::Return(_) => {
                        return Err(RuntimeError::Invariant(
                            "intrinsic PromiseResolve returned a non-object",
                        ));
                    }
                };
                let realm = self
                    .runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .async_function_state_snapshot(self.state.object_id())?
                    .driver_realm;
                let make_resume = |kind| {
                    self.runtime.new_internal_promise_function(
                        realm,
                        NativeFunctionId::AsyncFunctionResume(kind),
                        1,
                        1,
                        InternalCallableData::AsyncFunctionResume {
                            state: self.state.object_id(),
                            kind,
                        },
                    )
                };
                let fulfill = make_resume(AsyncFunctionResumeKind::Fulfill)?;
                let reject = make_resume(AsyncFunctionResumeKind::Reject)?;
                self.runtime
                    .store_async_function_activation(&self.state, &activation)?;
                self.runtime
                    .perform_promise_then_without_capability(realm, &promise, &fulfill, &reject)?;
                self.active = false;
                self.finish()
            }
        }
    }
    fn finish(mut self: Box<Self>) -> Result<AsyncStep, RuntimeError> {
        Ok(AsyncStep::Complete(Completion::Return(std::mem::replace(
            &mut self.output,
            Value::Undefined,
        ))))
    }
}
impl Drop for AsyncResume {
    fn drop(&mut self) {
        if self.active {
            let _ = self.runtime.complete_async_function_state(&self.state);
        }
    }
}
impl AsyncStep {
    pub(crate) fn finish(
        self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<Completion, RuntimeError> {
        #[cfg(feature = "stack-vm")]
        {
            crate::engine::vm::execute_root(
                runtime.clone(),
                realm,
                crate::engine::vm::RootOperation::Async(self),
            )
            .map_err(RuntimeError::Engine)
        }
        #[cfg(not(feature = "stack-vm"))]
        {
            let mut step = self;
            loop {
                step = match step {
                    Self::Complete(completion) => return Ok(completion),
                    Self::Run { mut resume } => {
                        let activation = resume.take_run_activation();
                        let input = resume.take_run_input();
                        resume.body(activation.run(runtime, input)?)?
                    }
                    Self::Resolve { mut resume } => {
                        let value = resume.take_resolve_value();
                        let realm = resume.take_resolve_realm();
                        resume.resume(runtime.promise_resolve_intrinsic(realm, value)?)?
                    }
                    Self::Call { mut resume } => {
                        let callable = resume.take_call_callable();
                        let value = resume.take_call_value();
                        resume.resume(runtime.call_internal(
                            realm,
                            &callable,
                            Value::Undefined,
                            &[value],
                        )?)?
                    }
                };
            }
        }
    }
}

#[cfg(all(test, feature = "stack-vm", feature = "profiling"))]
mod tests {
    use crate::engine::{
        api::{profiling::CostProfile, runtime::Runtime},
        value::Value,
    };

    #[test]
    fn await_thenable_jobs_and_finally_stay_owned_across_gc() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        assert_eq!(context.eval("var asyncResult=0, asyncReads=0, asyncFinally=0; async function f(){try {return 2+await {get then(){asyncReads++;return resolve=>resolve(40)}};} finally {asyncFinally=[1,2].map(x=>x+1)[1]}} f().then(x=>asyncResult=x); asyncResult").unwrap(), Value::Int(0));
        let mut jobs = 0;
        while runtime.is_job_pending() {
            runtime.run_gc().unwrap();
            runtime.execute_pending_job().unwrap();
            jobs += 1;
        }
        assert!(jobs >= 3);
        assert_eq!(
            context
                .eval("asyncResult===42 && asyncReads===1 && asyncFinally===3")
                .unwrap(),
            Value::Bool(true)
        );
        let cost = profile.snapshot();
        assert_eq!(cost.legacy_dispatches, 0, "{cost:?}");
        assert_eq!(cost.owned_bridge_exits, 0, "{cost:?}");
        assert_eq!(cost.owned_sync_call_bridges, 0, "{cost:?}");
    }
}

#[derive(Default)]
struct AsyncStepPending {
    run_activation: Option<Box<RootedVmActivation>>,
    run_input: Option<VmActivationResume>,
    resolve_value: Option<Value>,
    resolve_realm: Option<ContextId>,
    call_callable: Option<CallableRef>,
    call_value: Option<Value>,
}
impl AsyncStep {
    pub(crate) fn request_run(
        activation: Box<RootedVmActivation>,
        input: VmActivationResume,
        mut resume: Box<AsyncResume>,
    ) -> Self {
        resume.pending_effect.run_activation = Some(activation);
        resume.pending_effect.run_input = Some(input);
        Self::Run { resume }
    }
    pub(crate) fn request_resolve(
        value: Value,
        realm: ContextId,
        mut resume: Box<AsyncResume>,
    ) -> Self {
        resume.pending_effect.resolve_value = Some(value);
        resume.pending_effect.resolve_realm = Some(realm);
        Self::Resolve { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        value: Value,
        mut resume: Box<AsyncResume>,
    ) -> Self {
        resume.pending_effect.call_callable = Some(callable);
        resume.pending_effect.call_value = Some(value);
        Self::Call { resume }
    }
}
impl AsyncResume {
    pub(crate) fn take_run_activation(&mut self) -> Box<RootedVmActivation> {
        self.pending_effect
            .run_activation
            .take()
            .expect("AsyncStep Run activation")
    }
    pub(crate) fn take_run_input(&mut self) -> VmActivationResume {
        self.pending_effect
            .run_input
            .take()
            .expect("AsyncStep Run input")
    }
    pub(crate) fn take_resolve_value(&mut self) -> Value {
        self.pending_effect
            .resolve_value
            .take()
            .expect("AsyncStep Resolve value")
    }
    pub(crate) fn take_resolve_realm(&mut self) -> ContextId {
        self.pending_effect
            .resolve_realm
            .take()
            .expect("AsyncStep Resolve realm")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.pending_effect
            .call_callable
            .take()
            .expect("AsyncStep Call callable")
    }
    pub(crate) fn take_call_value(&mut self) -> Value {
        self.pending_effect
            .call_value
            .take()
            .expect("AsyncStep Call value")
    }
}
const _: () = assert!(std::mem::size_of::<AsyncStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<AsyncStep>() <= 64);
