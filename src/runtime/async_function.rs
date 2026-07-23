//! Ordinary async-function intrinsics and suspension driver.
//!
//! Pinned QuickJS keeps `%AsyncFunction.prototype%` as a realm root and the
//! resumable call state as a GC-visible node.  The hidden dynamic constructor
//! is deliberately not installed on the global object.

use super::*;
use crate::heap::{
    AsyncFunctionPhase, AsyncFunctionRealmData, AsyncFunctionResumeKind, InternalCallableData,
    ObjectData,
};
use crate::runtime::intrinsics::promise::RootedPromiseCapability;
use crate::runtime::vm_host::{
    EncodedVmActivation, RuntimeVmHost, VmActivationResume, VmRunOutcome,
};
use crate::vm::{CallInput, Vm, VmExit, VmSuspendKind};

impl Runtime {
    /// Preserve the async-call Promise boundary when the host stack is already
    /// too deep to start another bytecode VM frame.
    ///
    /// Both the returned Promise and this pre-body error belong to the calling
    /// realm. Pinned QuickJS performs this preflight before `JS_CallInternal`
    /// switches to the bytecode's defining realm; errors raised after that
    /// switch still belong to the callee realm. Promise rejection is published
    /// directly so the native reject callback cannot trip the same stack
    /// preflight again.
    pub(super) fn reject_async_bytecode_stack_overflow(
        &self,
        caller_realm: ContextId,
    ) -> Result<Completion, RuntimeError> {
        let reason =
            self.new_native_error(caller_realm, NativeErrorKind::Internal, "stack overflow")?;
        let promise = self.new_rejected_default_promise(caller_realm, reason)?;
        Ok(Completion::Return(Value::Object(promise)))
    }

    pub(super) fn initialize_async_function_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let async_function_prototype = self.new_object(Some(function_prototype))?;
        let tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            &async_function_prototype,
            &tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static(
                    "AsyncFunction",
                ))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "AsyncFunction intrinsic toStringTag definition was rejected",
            ));
        }

        let function_constructor = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .function_constructor
            .ok_or(RuntimeError::Invariant(
                "AsyncFunction initialization requires the Function constructor",
            ))?;
        let function_constructor =
            ObjectRef::from_borrowed_handle(self.clone(), function_constructor)?;
        let constructor = self.new_native_builtin(
            &function_constructor,
            realm,
            NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Async),
            1,
            "AsyncFunction",
            1,
        )?;

        self.define_function_data_property(
            constructor.as_object(),
            "prototype",
            Value::Object(async_function_prototype.clone()),
            false,
            false,
        )?;
        self.define_function_data_property(
            &async_function_prototype,
            "constructor",
            Value::Object(constructor.as_object().clone()),
            false,
            true,
        )?;

        self.0
            .state
            .borrow_mut()
            .heap
            .attach_async_function_intrinsics(
                realm,
                AsyncFunctionRealmData {
                    function_prototype: async_function_prototype.object_id(),
                },
            )?;
        Ok(())
    }

    /// Start an ordinary async bytecode call immediately, but return the
    /// caller-realm Promise which owns its eventual completion. JavaScript
    /// throws from the body are converted to rejection after the active
    /// bytecode frame has been popped.
    #[inline(never)]
    pub(super) fn start_async_bytecode_callable(
        &self,
        caller_realm: ContextId,
        mut host: RuntimeVmHost,
        input: CallInput<'_>,
        active_frame: ActiveFrameGuard,
    ) -> Result<Completion, RuntimeError> {
        let capability = self.new_default_promise_capability(caller_realm)?;
        let state = self.allocate_async_function_state(caller_realm, &capability)?;
        let result = Vm::new().start_published(input, &mut host);
        active_frame.finish()?;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.complete_async_function_state(&state)?;
                return Err(RuntimeError::Engine(error));
            }
        };
        match result {
            VmExit::Complete(completion) => {
                self.settle_async_function(&state, completion)?;
            }
            VmExit::Suspend(mut suspension) => {
                if suspension.kind() != VmSuspendKind::Await {
                    self.complete_async_function_state(&state)?;
                    return Err(RuntimeError::Invariant(
                        "async function stopped at a non-await suspension",
                    ));
                }
                let awaited = suspension.take_awaited().map_err(RuntimeError::Engine)?;
                let activation = host.encode_vm_activation(suspension)?;
                self.suspend_async_function(&state, awaited, activation)?;
            }
        }
        Ok(Completion::Return(Value::Object(capability.promise)))
    }

    fn allocate_async_function_state(
        &self,
        driver_realm: ContextId,
        capability: &RootedPromiseCapability,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(None, &[])?;
        let object = match state.heap.allocate_object(ObjectData::async_function_state(
            shape,
            Vec::new(),
            driver_realm,
            capability.resolve.as_object().object_id(),
            capability.reject.as_object().object_id(),
        )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn suspend_async_function(
        &self,
        state_object: &ObjectRef,
        awaited: Value,
        activation: EncodedVmActivation,
    ) -> Result<(), RuntimeError> {
        if activation.kind != VmSuspendKind::Await {
            return Err(RuntimeError::Invariant(
                "async function published a non-await activation",
            ));
        }
        let driver_realm = self
            .0
            .state
            .borrow()
            .heap
            .async_function_state_snapshot(state_object.object_id())?
            .driver_realm;
        let promise = match self.promise_resolve_intrinsic(driver_realm, awaited)? {
            Completion::Return(Value::Object(promise)) => promise,
            Completion::Return(_) => {
                self.complete_async_function_state(state_object)?;
                return Err(RuntimeError::Invariant(
                    "intrinsic PromiseResolve returned a non-object",
                ));
            }
            Completion::Throw(reason) => {
                self.settle_async_function(state_object, Completion::Throw(reason))?;
                return Ok(());
            }
        };

        let make_resume = |kind| {
            self.new_internal_promise_function(
                driver_realm,
                NativeFunctionId::AsyncFunctionResume(kind),
                1,
                1,
                InternalCallableData::AsyncFunctionResume {
                    state: state_object.object_id(),
                    kind,
                },
            )
        };
        let fulfill = make_resume(AsyncFunctionResumeKind::Fulfill)?;
        let reject = make_resume(AsyncFunctionResumeKind::Reject)?;
        self.store_async_function_activation(state_object, &activation)?;
        if let Err(error) =
            self.perform_promise_then_without_capability(driver_realm, &promise, &fulfill, &reject)
        {
            self.complete_async_function_state(state_object)?;
            return Err(error);
        }
        drop(activation);
        Ok(())
    }

    fn store_async_function_activation(
        &self,
        state_object: &ObjectRef,
        activation: &EncodedVmActivation,
    ) -> Result<(), RuntimeError> {
        let atoms = activation.atoms();
        let mut state = self.0.state.borrow_mut();
        let mut retained_atoms = Vec::with_capacity(atoms.len());
        for atom in atoms {
            if let Err(error) = state.atoms.retain(atom) {
                state.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
            retained_atoms.push(atom);
        }
        if let Err(error) = state
            .heap
            .suspend_async_function(state_object.object_id(), activation.data.clone())
        {
            state.release_atoms(retained_atoms)?;
            return Err(error.into());
        }
        Ok(())
    }

    fn complete_async_function_state(&self, state_object: &ObjectRef) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let cleanup = state
            .heap
            .complete_async_function(state_object.object_id())?;
        state.apply_cleanup(cleanup)?;
        Ok(())
    }

    fn settle_async_function(
        &self,
        state_object: &ObjectRef,
        completion: Completion,
    ) -> Result<(), RuntimeError> {
        let snapshot = self
            .0
            .state
            .borrow()
            .heap
            .async_function_state_snapshot(state_object.object_id())?;
        if snapshot.phase == AsyncFunctionPhase::Completed {
            return Err(RuntimeError::Invariant(
                "async function settled more than once",
            ));
        }
        let (target, value) = match completion {
            Completion::Return(value) => (snapshot.outer_resolve, value),
            Completion::Throw(value) => (snapshot.outer_reject, value),
        };
        let target = ObjectRef::from_borrowed_handle(self.clone(), target)?;
        let target = self.as_callable(&target)?.ok_or(RuntimeError::Invariant(
            "async function outer resolving function is not callable",
        ))?;
        self.complete_async_function_state(state_object)?;
        // The Promise resolving pair is internally infallible at the
        // JavaScript-completion boundary. Match QuickJS by consuming its
        // return value; arena/engine failures still propagate.
        let _ = self.call_internal(snapshot.driver_realm, &target, Value::Undefined, &[value])?;
        Ok(())
    }

    pub(super) fn call_async_function_resume(
        &self,
        realm: ContextId,
        target_kind: AsyncFunctionResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "AsyncFunction resume callback received a constructor invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "AsyncFunction resume callback argv was not padded",
            ))?;
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "AsyncFunction resume callback has no internal state",
            ))?;
        let InternalCallableData::AsyncFunctionResume { state, kind } = internal else {
            return Err(RuntimeError::Invariant(
                "AsyncFunction resume callback has the wrong internal state",
            ));
        };
        if kind != target_kind {
            return Err(RuntimeError::Invariant(
                "AsyncFunction resume target disagrees with its capture",
            ));
        }
        let state_object = ObjectRef::from_borrowed_handle(self.clone(), state)?;
        let snapshot = self
            .0
            .state
            .borrow()
            .heap
            .async_function_state_snapshot(state)?;
        if snapshot.phase != AsyncFunctionPhase::Awaiting || snapshot.driver_realm != realm {
            return Err(RuntimeError::Invariant(
                "AsyncFunction continuation ran outside its awaiting realm",
            ));
        }
        let activation = snapshot
            .activation
            .as_deref()
            .ok_or(RuntimeError::Invariant(
                "awaiting AsyncFunction has no activation",
            ))?;
        let rooted = RuntimeVmHost::decode_vm_activation(
            self.clone(),
            VmSuspendKind::Await,
            realm,
            activation,
            FunctionKind::Async,
        )?;
        {
            let mut runtime_state = self.0.state.borrow_mut();
            let (_moved, cleanup) = runtime_state.heap.begin_async_function_resume(state)?;
            runtime_state.apply_cleanup(cleanup)?;
        }
        let resume = match target_kind {
            AsyncFunctionResumeKind::Fulfill => VmActivationResume::AwaitFulfill(argument),
            AsyncFunctionResumeKind::Reject => VmActivationResume::AwaitReject(argument),
        };
        let outcome = match rooted.run(self, resume) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.complete_async_function_state(&state_object)?;
                return Err(error);
            }
        };
        match outcome {
            VmRunOutcome::Complete(completion) => {
                self.settle_async_function(&state_object, completion)?;
            }
            VmRunOutcome::Suspend { value, activation } => {
                self.suspend_async_function(&state_object, value, *activation)?;
            }
        }
        Ok(Completion::Return(Value::Undefined))
    }
}
