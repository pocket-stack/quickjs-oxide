//! Ordinary async-function intrinsics and suspension driver.
//!
//! Pinned QuickJS keeps `%AsyncFunction.prototype%` as a realm root and the
//! resumable call state as a GC-visible node.  The hidden dynamic constructor
//! is deliberately not installed on the global object.

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{DynamicFunctionKind, NativeFunctionId};
use crate::engine::builtins::promise::RootedPromiseCapability;
use crate::engine::code::function::metadata::FunctionKind;

use crate::engine::heap::{
    AsyncFunctionPhase, AsyncFunctionRealmData, AsyncFunctionResumeKind, ContextId,
    InternalCallableData, ObjectData,
};
use crate::engine::object::{
    DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
};
use crate::engine::value::{JsString, Value};
use crate::engine::vm::call::{NativeArguments, NativeInvocation};
use crate::engine::vm::suspend::{self, EncodedVmActivation, VmActivationResume};
use crate::engine::vm::{Completion, VmSuspendKind};

mod operation;
pub(crate) use operation::{AsyncResume, AsyncStep};

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
    pub(crate) fn reject_async_bytecode_stack_overflow(
        &self,
        caller_realm: ContextId,
    ) -> Result<Completion, RuntimeError> {
        let reason =
            self.new_native_error_jsvalue(caller_realm, NativeErrorKind::Internal, "stack overflow")?;
        let promise = self.new_rejected_default_promise(caller_realm, reason)?;
        Ok(Completion::Return(Value::Object(promise)))
    }

    pub(crate) fn initialize_async_function_intrinsic(
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
    pub(in crate::engine::vm) fn start_async_bytecode_callable(
        &self,
        caller_realm: ContextId,
        entry: super::frame::FrameEntry,
    ) -> Result<Completion, RuntimeError> {
        let resume = AsyncResume::start(self, caller_realm)?;
        let outcome = super::driver::execute(
            self.clone(),
            entry,
            super::execution::ExecutionLimits::for_runtime(self),
        )
        .and_then(|exit| exit.finish_suspending(self.clone()))
        .map_err(RuntimeError::Engine);
        resume.body(outcome?)?.finish(self, caller_realm)
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

    fn store_async_function_activation(
        &self,
        state_object: &ObjectRef,
        activation: &EncodedVmActivation,
    ) -> Result<(), RuntimeError> {
        let atoms = {
            let state = self.0.state.borrow();
            activation.atoms(&state.atoms)?
        };
        let mut state = self.0.state.borrow_mut();
        let mut retained_atoms = Vec::with_capacity(atoms.len());
        for atom in atoms {
            if let Err(error) = state.atoms.retain(atom) {
                state.release_atoms(retained_atoms)?;
                activation.release_conversion_edges(self);
                return Err(error.into());
            }
            retained_atoms.push(atom);
        }
        if let Err(error) = state
            .heap
            .suspend_async_function(state_object.object_id(), activation.data.clone())
        {
            state.release_atoms(retained_atoms)?;
            activation.release_conversion_edges(self);
            return Err(error.into());
        }
        // The heap record retained its own activation edges, so the
        // caller-owned conversion edges can drop.
        activation.release_conversion_edges(self);
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

    pub(crate) fn call_async_function_resume(
        &self,
        realm: ContextId,
        target_kind: AsyncFunctionResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.start_async_function_resume(realm, target_kind, invocation, arguments)?
            .finish(self, realm)
    }

    pub(crate) fn start_async_function_resume(
        &self,
        realm: ContextId,
        target_kind: AsyncFunctionResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<AsyncStep, RuntimeError> {
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
        let rooted = suspend::thaw(
            self.clone(),
            VmSuspendKind::Await,
            realm,
            activation,
            FunctionKind::Async,
        )?;
        let cleanup = {
            let mut runtime_state = self.0.state.borrow_mut();
            let (_moved, cleanup) = runtime_state.heap.begin_async_function_resume(state)?;
            cleanup
        };
        let continuation = AsyncResume::resumed(self, state_object);
        self.0.state.borrow_mut().apply_cleanup(cleanup)?;
        let resume = match target_kind {
            AsyncFunctionResumeKind::Fulfill => VmActivationResume::AwaitFulfill(argument),
            AsyncFunctionResumeKind::Reject => VmActivationResume::AwaitReject(argument),
        };
        Ok(AsyncStep::request_run(
            Box::new(rooted),
            resume,
            continuation,
        ))
    }
}
