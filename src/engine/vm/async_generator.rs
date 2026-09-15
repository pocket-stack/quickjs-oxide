//! Ordinary async-generator intrinsics and FIFO Promise driver.
//!
//! An async generator is neither a synchronous Generator with Promise-shaped
//! results nor an AsyncFunction which happens to suspend at `yield`. Pinned
//! QuickJS gives it an independent branded object, a serialized request queue,
//! and two kinds of Promise continuation: authored `await` and completed
//! `.return(value)` assimilation. This module owns that combined state machine.

use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{DynamicFunctionKind, GeneratorResumeKind, NativeFunctionId};
use crate::engine::builtins::promise::RootedPromiseCapability;

use crate::engine::heap::{
    AsyncGeneratorRealmData, AsyncGeneratorRequestData, AsyncGeneratorResumeKind,
    AsyncGeneratorState, ContextId, InternalCallableData, ObjectData,
};
use crate::engine::object::shape::PropertyFlags;
use crate::engine::object::{CallableRef, ObjectRef, PropertyKey, WellKnownSymbol};
use crate::engine::value::Value;

use crate::engine::vm::call::{NativeArguments, NativeInvocation};
use crate::engine::vm::frames::ActiveFrameGuard;
use crate::engine::vm::host_bridge::RuntimeVmHost;
use crate::engine::vm::suspend::{self, EncodedVmActivation, VmRunOutcome};
use crate::engine::vm::{CallInput, Completion, VmSuspendKind};

mod operation;
pub(crate) use operation::{AsyncGeneratorResume, AsyncGeneratorStep};

struct RootedAsyncGeneratorRequest {
    completion: GeneratorResumeKind,
    result: Value,
    _promise: ObjectRef,
    resolve: CallableRef,
    reject: CallableRef,
}

enum AsyncGeneratorSettlement {
    Resolve { value: Value, done: bool },
    Reject(Value),
}

impl Runtime {
    pub(crate) fn initialize_async_generator_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let async_iterator_prototype = self.new_object(Some(object_prototype))?;
        let async_iterator_key =
            PropertyKey::from(self.well_known_symbol(WellKnownSymbol::AsyncIterator));
        self.define_native_builtin_auto_init_with_key(
            &async_iterator_prototype,
            realm,
            &async_iterator_key,
            NativeFunctionId::IteratorPrototypeIterator,
            "[Symbol.asyncIterator]",
            0,
            0,
            PropertyFlags::data(true, false, true),
        )?;

        let async_from_sync_iterator_prototype =
            self.new_object(Some(&async_iterator_prototype))?;
        for (kind, name) in [
            (GeneratorResumeKind::Next, "next"),
            (GeneratorResumeKind::Return, "return"),
            (GeneratorResumeKind::Throw, "throw"),
        ] {
            self.define_native_builtin_auto_init(
                &async_from_sync_iterator_prototype,
                realm,
                NativeFunctionId::AsyncFromSyncIteratorResume(kind),
                name,
                1,
                1,
            )?;
        }

        let async_generator_prototype = self.new_object(Some(&async_iterator_prototype))?;
        for (kind, name) in [
            (GeneratorResumeKind::Next, "next"),
            (GeneratorResumeKind::Return, "return"),
            (GeneratorResumeKind::Throw, "throw"),
        ] {
            self.define_native_builtin_auto_init(
                &async_generator_prototype,
                realm,
                NativeFunctionId::AsyncGeneratorPrototypeResume(kind),
                name,
                1,
                1,
            )?;
        }
        self.define_generator_to_string_tag(&async_generator_prototype, "AsyncGenerator")?;

        let async_generator_function_prototype = self.new_object(Some(function_prototype))?;
        self.define_generator_to_string_tag(
            &async_generator_function_prototype,
            "AsyncGeneratorFunction",
        )?;

        let function_constructor = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .function_constructor
            .ok_or(RuntimeError::Invariant(
                "AsyncGenerator initialization requires the Function constructor",
            ))?;
        let function_constructor =
            ObjectRef::from_borrowed_handle(self.clone(), function_constructor)?;
        let constructor = self.new_native_builtin(
            &function_constructor,
            realm,
            NativeFunctionId::FunctionConstructor(DynamicFunctionKind::AsyncGenerator),
            1,
            "AsyncGeneratorFunction",
            1,
        )?;

        self.define_function_data_property(
            constructor.as_object(),
            "prototype",
            Value::Object(async_generator_function_prototype.clone()),
            false,
            false,
        )?;
        self.define_function_data_property(
            &async_generator_function_prototype,
            "constructor",
            Value::Object(constructor.as_object().clone()),
            false,
            true,
        )?;
        self.define_function_data_property(
            &async_generator_function_prototype,
            "prototype",
            Value::Object(async_generator_prototype.clone()),
            false,
            true,
        )?;
        self.define_function_data_property(
            &async_generator_prototype,
            "constructor",
            Value::Object(async_generator_function_prototype.clone()),
            false,
            true,
        )?;

        self.0
            .state
            .borrow_mut()
            .heap
            .attach_async_generator_intrinsics(
                realm,
                AsyncGeneratorRealmData {
                    async_iterator_prototype: async_iterator_prototype.object_id(),
                    async_from_sync_iterator_prototype: async_from_sync_iterator_prototype
                        .object_id(),
                    prototype: async_generator_prototype.object_id(),
                    function_prototype: async_generator_function_prototype.object_id(),
                },
            )?;
        Ok(())
    }

    /// Execute parameters synchronously through the unique InitialYield
    /// barrier, then choose the public function `.prototype` and allocate the
    /// branded async-generator object.
    #[inline(never)]
    pub(crate) fn start_async_generator_bytecode_callable(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        host: RuntimeVmHost,
        input: CallInput,
        active_frame: ActiveFrameGuard,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let result = suspend::start(host, input, arguments);
        active_frame.finish()?;
        match result? {
            VmRunOutcome::Suspend { activation, .. }
                if activation.kind == VmSuspendKind::Initial =>
            {
                self.finish_async_generator_function_call(caller_realm, callable, *activation)
            }
            VmRunOutcome::Suspend { .. } => Err(RuntimeError::Invariant(
                "async-generator call did not stop at its initial-yield barrier",
            )),
            VmRunOutcome::Complete(Completion::Throw(value)) => Ok(Completion::Throw(value)),
            VmRunOutcome::Complete(Completion::Return(_)) => Err(RuntimeError::Invariant(
                "async-generator call completed before its initial-yield barrier",
            )),
        }
    }

    fn finish_async_generator_function_call(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        activation: EncodedVmActivation,
    ) -> Result<Completion, RuntimeError> {
        suspend::creation::GeneratorCreation {
            realm: caller_realm,
            callable: callable.clone(),
            asynchronous: true,
        }
        .frozen(self, Box::new(activation))?
        .finish(self, caller_realm)
    }

    pub(super) fn allocate_async_generator_object(
        &self,
        prototype: &ObjectRef,
        activation: EncodedVmActivation,
    ) -> Result<ObjectRef, RuntimeError> {
        let atoms = activation.atoms();
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let mut retained_atoms = Vec::with_capacity(atoms.len());
        for atom in atoms {
            if let Err(error) = state.atoms.retain(atom) {
                state.release_atoms(retained_atoms)?;
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
            retained_atoms.push(atom);
        }
        let object = match state.heap.allocate_object(ObjectData::async_generator(
            shape,
            Vec::new(),
            activation.data.clone(),
        )) {
            Ok(object) => object,
            Err(error) => {
                state.release_atoms(retained_atoms)?;
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(activation);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    pub(crate) fn call_async_generator_prototype_resume(
        &self,
        realm: ContextId,
        kind: GeneratorResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        AsyncGeneratorStep::start(
            self,
            realm,
            NativeFunctionId::AsyncGeneratorPrototypeResume(kind),
            &invocation,
            arguments,
        )?
        .finish(self)
    }

    fn enqueue_async_generator_request(
        &self,
        generator: &ObjectRef,
        completion: GeneratorResumeKind,
        result: Value,
        capability: &RootedPromiseCapability,
    ) -> Result<(), RuntimeError> {
        self.validate_value_domain(&result, "AsyncGenerator request")?;
        let result = self.raw_property_value(&result)?;
        let request = AsyncGeneratorRequestData {
            completion,
            result: result.clone(),
            promise: capability.promise.object_id(),
            resolve: capability.resolve.as_object().object_id(),
            reject: capability.reject.as_object().object_id(),
        };
        let mut state = self.0.state.borrow_mut();
        let retained_atoms = state.retain_raw_value_atoms([&result])?;
        if let Err(error) = state
            .heap
            .async_generator_enqueue(generator.object_id(), request)
        {
            state.release_atoms(retained_atoms)?;
            return Err(error.into());
        }
        Ok(())
    }

    fn store_async_generator_suspension(
        &self,
        generator: &ObjectRef,
        generator_state: AsyncGeneratorState,
        resume_realm: Option<ContextId>,
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
        if let Err(error) = state.heap.suspend_async_generator(
            generator.object_id(),
            generator_state,
            activation.data.clone(),
            resume_realm,
        ) {
            state.release_atoms(retained_atoms)?;
            return Err(error.into());
        }
        Ok(())
    }

    fn new_async_generator_resume_callback(
        &self,
        realm: ContextId,
        generator: &ObjectRef,
        kind: AsyncGeneratorResumeKind,
    ) -> Result<CallableRef, RuntimeError> {
        self.new_internal_promise_function(
            realm,
            NativeFunctionId::AsyncGeneratorResume(kind),
            1,
            1,
            InternalCallableData::AsyncGeneratorResume {
                generator: generator.object_id(),
                kind,
            },
        )
    }

    pub(crate) fn call_async_generator_resume(
        &self,
        realm: ContextId,
        target_kind: AsyncGeneratorResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        AsyncGeneratorStep::start(
            self,
            realm,
            NativeFunctionId::AsyncGeneratorResume(target_kind),
            &invocation,
            arguments,
        )?
        .finish(self)
    }

    fn root_front_async_generator_request(
        &self,
        generator: &ObjectRef,
    ) -> Result<RootedAsyncGeneratorRequest, RuntimeError> {
        let request = self
            .0
            .state
            .borrow()
            .heap
            .async_generator_front_request(generator.object_id())?
            .ok_or(RuntimeError::Invariant(
                "AsyncGenerator request queue is empty",
            ))?;
        let result = self.root_raw_value(&request.result)?;
        let promise = ObjectRef::from_borrowed_handle(self.clone(), request.promise)?;
        let resolve = ObjectRef::from_borrowed_handle(self.clone(), request.resolve)?;
        let resolve = self.as_callable(&resolve)?.ok_or(RuntimeError::Invariant(
            "AsyncGenerator request resolve is not callable",
        ))?;
        let reject = ObjectRef::from_borrowed_handle(self.clone(), request.reject)?;
        let reject = self.as_callable(&reject)?.ok_or(RuntimeError::Invariant(
            "AsyncGenerator request reject is not callable",
        ))?;
        Ok(RootedAsyncGeneratorRequest {
            completion: request.completion,
            result,
            _promise: promise,
            resolve,
            reject,
        })
    }

    fn remove_front_async_generator_request(
        &self,
        generator: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let (_raw, cleanup) = state
            .heap
            .async_generator_pop_front(generator.object_id())?;
        state.apply_cleanup(cleanup)?;
        Ok(())
    }

    fn complete_async_generator(&self, generator: &ObjectRef) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let cleanup = state.heap.complete_async_generator(generator.object_id())?;
        state.apply_cleanup(cleanup)
    }

    fn finish_async_generator_completed_return(
        &self,
        generator: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let cleanup = state
            .heap
            .finish_async_generator_completed_return(generator.object_id())?;
        state.apply_cleanup(cleanup)
    }
}
