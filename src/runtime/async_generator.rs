//! Ordinary async-generator intrinsics and FIFO Promise driver.
//!
//! An async generator is neither a synchronous Generator with Promise-shaped
//! results nor an AsyncFunction which happens to suspend at `yield`. Pinned
//! QuickJS gives it an independent branded object, a serialized request queue,
//! and two kinds of Promise continuation: authored `await` and completed
//! `.return(value)` assimilation. This module owns that combined state machine.

use super::*;
use crate::heap::{
    AsyncGeneratorRealmData, AsyncGeneratorRequestData, AsyncGeneratorResumeKind,
    AsyncGeneratorState, InternalCallableData,
};
use crate::runtime::intrinsics::promise::RootedPromiseCapability;
use crate::runtime::vm_host::{
    EncodedVmActivation, RuntimeVmHost, VmActivationResume, VmRunOutcome,
};
use crate::vm::{CallInput, Vm, VmExit, VmResume, VmSuspendKind, VmSuspension};

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
    pub(super) fn initialize_async_generator_intrinsic(
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
    pub(super) fn start_async_generator_bytecode_callable(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        mut host: RuntimeVmHost,
        input: CallInput<'_>,
        active_frame: ActiveFrameGuard,
    ) -> Result<Completion, RuntimeError> {
        let result = Vm::new().start_published(input, &mut host);
        active_frame.finish()?;
        match result.map_err(RuntimeError::Engine)? {
            VmExit::Suspend(suspension) if suspension.kind() == VmSuspendKind::Initial => {
                self.finish_async_generator_function_call(caller_realm, callable, host, suspension)
            }
            VmExit::Suspend(_) => Err(RuntimeError::Invariant(
                "async-generator call did not stop at its initial-yield barrier",
            )),
            VmExit::Complete(Completion::Throw(value)) => Ok(Completion::Throw(value)),
            VmExit::Complete(Completion::Return(_)) => Err(RuntimeError::Invariant(
                "async-generator call completed before its initial-yield barrier",
            )),
        }
    }

    fn finish_async_generator_function_call(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        host: RuntimeVmHost,
        suspension: VmSuspension,
    ) -> Result<Completion, RuntimeError> {
        let prototype = match self.async_generator_instance_prototype(caller_realm, callable)? {
            NativeConversion::Value(prototype) => prototype,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let activation = host.encode_vm_activation(suspension)?;
        if activation.kind != VmSuspendKind::Initial {
            return Err(RuntimeError::Invariant(
                "new async-generator activation is not suspended at start",
            ));
        }
        let generator = self.allocate_async_generator_object(&prototype, activation)?;
        Ok(Completion::Return(Value::Object(generator)))
    }

    fn async_generator_instance_prototype(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype =
            match self.get_property_in_realm(caller_realm, callable.as_object(), &prototype_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
        if let Value::Object(prototype) = prototype {
            return Ok(NativeConversion::Value(prototype));
        }
        let realm = self.callable_realm(callable)?;
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .async_generator
            .ok_or(RuntimeError::Invariant(
                "async-generator callable realm has no AsyncGenerator intrinsics",
            ))?
            .prototype;
        Ok(NativeConversion::Value(ObjectRef::from_borrowed_handle(
            self.clone(),
            prototype,
        )?))
    }

    fn allocate_async_generator_object(
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

    pub(super) fn call_async_generator_prototype_resume(
        &self,
        realm: ContextId,
        kind: GeneratorResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "AsyncGenerator resume received a constructor invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "AsyncGenerator resume has no readable argument slot",
            ))?;
        let capability = self.new_default_promise_capability(realm)?;
        let promise = capability.promise.clone();

        let generator = match this_value {
            Value::Object(generator)
                if matches!(
                    self.0
                        .state
                        .borrow()
                        .heap
                        .object(generator.object_id())?
                        .payload,
                    ObjectPayload::AsyncGenerator(_)
                ) =>
            {
                generator
            }
            Value::Object(_)
            | Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => {
                let reason =
                    self.new_native_error(realm, NativeErrorKind::Type, "not an async generator")?;
                let _ =
                    self.call_internal(realm, &capability.reject, Value::Undefined, &[reason])?;
                return Ok(Completion::Return(Value::Object(promise)));
            }
        };

        self.enqueue_async_generator_request(&generator, kind, argument, &capability)?;
        let state = self
            .0
            .state
            .borrow()
            .heap
            .async_generator_snapshot(generator.object_id())?
            .state;
        if !matches!(
            state,
            AsyncGeneratorState::Executing | AsyncGeneratorState::AwaitingReturn
        ) {
            self.pump_async_generator(realm, &generator)?;
        }
        Ok(Completion::Return(Value::Object(promise)))
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

    fn pump_async_generator(
        &self,
        realm: ContextId,
        generator: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        loop {
            let snapshot = self
                .0
                .state
                .borrow()
                .heap
                .async_generator_snapshot(generator.object_id())?;
            let Some(request) = snapshot.queue.front() else {
                return Ok(());
            };
            match snapshot.state {
                AsyncGeneratorState::AwaitingReturn => {
                    return Ok(());
                }
                AsyncGeneratorState::Executing => {
                    // A request resolver may synchronously re-enter this
                    // generator through an inherited `then` getter. QuickJS's
                    // still-active outer driver then falls through its
                    // EXECUTING case and resumes the just-parked await with
                    // the untouched undefined stack slot. Ordinary protocol
                    // calls never enter this pump while Executing.
                }
                AsyncGeneratorState::SuspendedStart
                    if request.completion != GeneratorResumeKind::Next =>
                {
                    self.complete_async_generator(generator)?;
                    continue;
                }
                AsyncGeneratorState::Completed => match request.completion {
                    GeneratorResumeKind::Next => {
                        self.settle_front_async_generator_request(
                            realm,
                            generator,
                            AsyncGeneratorSettlement::Resolve {
                                value: Value::Undefined,
                                done: true,
                            },
                        )?;
                        return Ok(());
                    }
                    GeneratorResumeKind::Throw => {
                        let request = self.root_front_async_generator_request(generator)?;
                        self.settle_front_async_generator_request(
                            realm,
                            generator,
                            AsyncGeneratorSettlement::Reject(request.result),
                        )?;
                        return Ok(());
                    }
                    GeneratorResumeKind::Return => {
                        self.begin_async_generator_completed_return(realm, generator)?;
                        return Ok(());
                    }
                },
                AsyncGeneratorState::SuspendedStart | AsyncGeneratorState::SuspendedYield => {}
                AsyncGeneratorState::SuspendedYieldStar => {
                    return Err(RuntimeError::Invariant(
                        "async-generator yield* reached the pre-yield* driver",
                    ));
                }
            }

            let previous = snapshot.state;
            let suspend_kind = match previous {
                AsyncGeneratorState::SuspendedStart => VmSuspendKind::Initial,
                AsyncGeneratorState::SuspendedYield => VmSuspendKind::Yield,
                AsyncGeneratorState::SuspendedYieldStar => VmSuspendKind::YieldStar,
                AsyncGeneratorState::Executing => VmSuspendKind::Await,
                AsyncGeneratorState::AwaitingReturn | AsyncGeneratorState::Completed => {
                    unreachable!()
                }
            };
            let activation = snapshot
                .activation
                .as_deref()
                .ok_or(RuntimeError::Invariant(
                    "suspended AsyncGenerator has no activation",
                ))?;
            let rooted = RuntimeVmHost::decode_vm_activation(
                self.clone(),
                suspend_kind,
                realm,
                activation,
                FunctionKind::AsyncGenerator,
            )?;
            // Promote the front request before detaching the parked activation.
            // A handle/rooting failure must leave the generator resumable.
            let request = self.root_front_async_generator_request(generator)?;
            {
                let mut state = self.0.state.borrow_mut();
                let (began, _activation, cleanup) = state
                    .heap
                    .begin_async_generator_resume(generator.object_id())?;
                state.apply_cleanup(cleanup)?;
                if began != previous {
                    return Err(RuntimeError::Invariant(
                        "AsyncGenerator activation changed between snapshot and resume",
                    ));
                }
            }
            let resume = match previous {
                AsyncGeneratorState::SuspendedStart => VmActivationResume::Initial,
                AsyncGeneratorState::SuspendedYield => {
                    VmActivationResume::Generator(match request.completion {
                        GeneratorResumeKind::Next => VmResume::Next(request.result),
                        GeneratorResumeKind::Return => VmResume::Return(request.result),
                        GeneratorResumeKind::Throw => VmResume::Throw(request.result),
                    })
                }
                AsyncGeneratorState::Executing => {
                    VmActivationResume::AwaitFulfill(Value::Undefined)
                }
                AsyncGeneratorState::SuspendedYieldStar
                | AsyncGeneratorState::AwaitingReturn
                | AsyncGeneratorState::Completed => unreachable!(),
            };
            let outcome = match rooted.run(self, resume) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.complete_async_generator(generator)?;
                    return Err(error);
                }
            };
            if !self.handle_async_generator_vm_outcome(realm, generator, outcome)? {
                return Ok(());
            }
        }
    }

    fn handle_async_generator_vm_outcome(
        &self,
        realm: ContextId,
        generator: &ObjectRef,
        mut outcome: VmRunOutcome,
    ) -> Result<bool, RuntimeError> {
        loop {
            match outcome {
                VmRunOutcome::Complete(completion) => {
                    self.complete_async_generator(generator)?;
                    let settlement = match completion {
                        Completion::Return(value) => {
                            AsyncGeneratorSettlement::Resolve { value, done: true }
                        }
                        Completion::Throw(value) => AsyncGeneratorSettlement::Reject(value),
                    };
                    self.settle_front_async_generator_request(realm, generator, settlement)?;
                    return Ok(true);
                }
                VmRunOutcome::Suspend { value, activation } => match activation.kind {
                    VmSuspendKind::Yield => {
                        self.store_async_generator_suspension(
                            generator,
                            AsyncGeneratorState::SuspendedYield,
                            None,
                            &activation,
                        )?;
                        self.settle_front_async_generator_request(
                            realm,
                            generator,
                            AsyncGeneratorSettlement::Resolve { value, done: false },
                        )?;
                        drop(activation);
                        return Ok(true);
                    }
                    VmSuspendKind::Await => {
                        let Some(next_outcome) = self.suspend_async_generator_await(
                            generator,
                            realm,
                            value,
                            *activation,
                        )?
                        else {
                            return Ok(false);
                        };
                        // PromiseResolve abrupt completion resumes the same
                        // activation immediately in QuickJS. Keep that tail in
                        // this native loop so authored repeated poisoned awaits
                        // cannot grow the Rust call stack.
                        outcome = next_outcome;
                    }
                    VmSuspendKind::Initial | VmSuspendKind::YieldStar => {
                        self.complete_async_generator(generator)?;
                        return Err(RuntimeError::Invariant(
                            "AsyncGenerator stopped at an unsupported suspension",
                        ));
                    }
                },
            }
        }
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

    fn suspend_async_generator_await(
        &self,
        generator: &ObjectRef,
        realm: ContextId,
        awaited: Value,
        activation: EncodedVmActivation,
    ) -> Result<Option<VmRunOutcome>, RuntimeError> {
        if activation.kind != VmSuspendKind::Await {
            return Err(RuntimeError::Invariant(
                "AsyncGenerator published a non-await activation as await",
            ));
        }
        let resolution = match self.promise_resolve_intrinsic(realm, awaited) {
            Ok(resolution) => resolution,
            Err(error) => {
                self.complete_async_generator(generator)?;
                return Err(error);
            }
        };
        let promise = match resolution {
            Completion::Return(Value::Object(promise)) => promise,
            Completion::Return(_) => {
                self.complete_async_generator(generator)?;
                return Err(RuntimeError::Invariant(
                    "intrinsic PromiseResolve returned a non-object",
                ));
            }
            Completion::Throw(reason) => {
                return self
                    .resume_async_generator_await_rejection(generator, realm, reason, activation)
                    .map(Some);
            }
        };
        let fulfill = match self.new_async_generator_resume_callback(
            realm,
            generator,
            AsyncGeneratorResumeKind::AwaitFulfill,
        ) {
            Ok(callback) => callback,
            Err(error) => {
                self.complete_async_generator(generator)?;
                return Err(error);
            }
        };
        let reject = match self.new_async_generator_resume_callback(
            realm,
            generator,
            AsyncGeneratorResumeKind::AwaitReject,
        ) {
            Ok(callback) => callback,
            Err(error) => {
                self.complete_async_generator(generator)?;
                return Err(error);
            }
        };
        if let Err(error) = self.store_async_generator_suspension(
            generator,
            AsyncGeneratorState::Executing,
            Some(realm),
            &activation,
        ) {
            self.complete_async_generator(generator)?;
            return Err(error);
        }
        if let Err(error) =
            self.perform_promise_then_without_capability(realm, &promise, &fulfill, &reject)
        {
            self.complete_async_generator(generator)?;
            return Err(error);
        }
        drop(activation);
        Ok(None)
    }

    /// QuickJS feeds a failed intrinsic PromiseResolve straight back into the
    /// parked VM. It does not manufacture a rejected Promise/job for this
    /// exceptional setup path.
    fn resume_async_generator_await_rejection(
        &self,
        generator: &ObjectRef,
        realm: ContextId,
        reason: Value,
        activation: EncodedVmActivation,
    ) -> Result<VmRunOutcome, RuntimeError> {
        // The activation wrapper still owns every transient root here, so an
        // immediate rejection can decode directly without publishing a fake
        // parked await into the heap.
        let rooted = match RuntimeVmHost::decode_vm_activation(
            self.clone(),
            VmSuspendKind::Await,
            realm,
            &activation.data,
            FunctionKind::AsyncGenerator,
        ) {
            Ok(rooted) => rooted,
            Err(error) => {
                self.complete_async_generator(generator)?;
                return Err(error);
            }
        };
        drop(activation);
        let outcome = match rooted.run(self, VmActivationResume::AwaitReject(reason)) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.complete_async_generator(generator)?;
                return Err(error);
            }
        };
        Ok(outcome)
    }

    fn begin_async_generator_completed_return(
        &self,
        realm: ContextId,
        generator: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let request = self.root_front_async_generator_request(generator)?;
        if request.completion != GeneratorResumeKind::Return {
            return Err(RuntimeError::Invariant(
                "completed AsyncGenerator return await has the wrong request",
            ));
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .begin_async_generator_completed_return(generator.object_id(), realm)?;
        let resolution = match self.promise_resolve_intrinsic(realm, request.result) {
            Ok(resolution) => resolution,
            Err(error) => {
                self.finish_async_generator_completed_return(generator)?;
                return Err(error);
            }
        };
        let promise = match resolution {
            Completion::Return(Value::Object(promise)) => promise,
            Completion::Return(_) => {
                self.finish_async_generator_completed_return(generator)?;
                return Err(RuntimeError::Invariant(
                    "completed-return PromiseResolve returned a non-object",
                ));
            }
            Completion::Throw(reason) => match self.new_rejected_default_promise(realm, reason) {
                Ok(promise) => promise,
                Err(error) => {
                    self.finish_async_generator_completed_return(generator)?;
                    return Err(error);
                }
            },
        };
        let fulfill = match self.new_async_generator_resume_callback(
            realm,
            generator,
            AsyncGeneratorResumeKind::ReturnFulfill,
        ) {
            Ok(callback) => callback,
            Err(error) => {
                self.finish_async_generator_completed_return(generator)?;
                return Err(error);
            }
        };
        let reject = match self.new_async_generator_resume_callback(
            realm,
            generator,
            AsyncGeneratorResumeKind::ReturnReject,
        ) {
            Ok(callback) => callback,
            Err(error) => {
                self.finish_async_generator_completed_return(generator)?;
                return Err(error);
            }
        };
        if let Err(error) =
            self.perform_promise_then_without_capability(realm, &promise, &fulfill, &reject)
        {
            self.finish_async_generator_completed_return(generator)?;
            return Err(error);
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

    pub(super) fn call_async_generator_resume(
        &self,
        realm: ContextId,
        target_kind: AsyncGeneratorResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "AsyncGenerator continuation received a constructor invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "AsyncGenerator continuation argv was not padded",
            ))?;
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "AsyncGenerator continuation has no internal state",
            ))?;
        let InternalCallableData::AsyncGeneratorResume { generator, kind } = internal else {
            return Err(RuntimeError::Invariant(
                "AsyncGenerator continuation has the wrong internal state",
            ));
        };
        if kind != target_kind {
            return Err(RuntimeError::Invariant(
                "AsyncGenerator continuation target disagrees with its capture",
            ));
        }
        let generator = ObjectRef::from_borrowed_handle(self.clone(), generator)?;
        let snapshot = self
            .0
            .state
            .borrow()
            .heap
            .async_generator_snapshot(generator.object_id())?;

        match target_kind {
            AsyncGeneratorResumeKind::AwaitFulfill | AsyncGeneratorResumeKind::AwaitReject => {
                if snapshot.state != AsyncGeneratorState::Executing {
                    // QuickJS silently discards a stale await reaction when an
                    // outer reentrant driver has already advanced the frame.
                    return Ok(Completion::Return(Value::Undefined));
                }
                if snapshot.resume_realm.is_none() {
                    return Err(RuntimeError::Invariant(
                        "AsyncGenerator continuation has no installed awaiting realm",
                    ));
                }
                let activation = snapshot
                    .activation
                    .as_deref()
                    .ok_or(RuntimeError::Invariant(
                        "awaiting AsyncGenerator has no activation",
                    ))?;
                let rooted = RuntimeVmHost::decode_vm_activation(
                    self.clone(),
                    VmSuspendKind::Await,
                    realm,
                    activation,
                    FunctionKind::AsyncGenerator,
                )?;
                {
                    let mut state = self.0.state.borrow_mut();
                    let (previous, _activation, cleanup) = state
                        .heap
                        .begin_async_generator_resume(generator.object_id())?;
                    state.apply_cleanup(cleanup)?;
                    if previous != AsyncGeneratorState::Executing {
                        return Err(RuntimeError::Invariant(
                            "AsyncGenerator await state changed before resume",
                        ));
                    }
                }
                let resume = match target_kind {
                    AsyncGeneratorResumeKind::AwaitFulfill => {
                        VmActivationResume::AwaitFulfill(argument)
                    }
                    AsyncGeneratorResumeKind::AwaitReject => {
                        VmActivationResume::AwaitReject(argument)
                    }
                    AsyncGeneratorResumeKind::ReturnFulfill
                    | AsyncGeneratorResumeKind::ReturnReject => unreachable!(),
                };
                let outcome = match rooted.run(self, resume) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        self.complete_async_generator(&generator)?;
                        return Err(error);
                    }
                };
                if self.handle_async_generator_vm_outcome(realm, &generator, outcome)? {
                    self.pump_async_generator(realm, &generator)?;
                }
            }
            AsyncGeneratorResumeKind::ReturnFulfill | AsyncGeneratorResumeKind::ReturnReject => {
                if snapshot.resume_realm.is_none() {
                    return Err(RuntimeError::Invariant(
                        "AsyncGenerator continuation has no installed awaiting realm",
                    ));
                }
                if snapshot.state != AsyncGeneratorState::AwaitingReturn
                    || snapshot.activation.is_some()
                {
                    return Err(RuntimeError::Invariant(
                        "AsyncGenerator return continuation reached an invalid state",
                    ));
                }
                self.finish_async_generator_completed_return(&generator)?;
                let settlement = match target_kind {
                    AsyncGeneratorResumeKind::ReturnFulfill => AsyncGeneratorSettlement::Resolve {
                        value: argument,
                        done: true,
                    },
                    AsyncGeneratorResumeKind::ReturnReject => {
                        AsyncGeneratorSettlement::Reject(argument)
                    }
                    AsyncGeneratorResumeKind::AwaitFulfill
                    | AsyncGeneratorResumeKind::AwaitReject => unreachable!(),
                };
                self.settle_front_async_generator_request(realm, &generator, settlement)?;
                // QuickJS ends this driver entry after servicing one completed
                // request. Later requests stay parked until a future explicit
                // protocol call re-enters the driver.
            }
        }
        Ok(Completion::Return(Value::Undefined))
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

    fn settle_front_async_generator_request(
        &self,
        realm: ContextId,
        generator: &ObjectRef,
        settlement: AsyncGeneratorSettlement,
    ) -> Result<(), RuntimeError> {
        let request = self.root_front_async_generator_request(generator)?;
        let (target, value) = match settlement {
            AsyncGeneratorSettlement::Resolve { value, done } => (
                request.resolve,
                Value::Object(self.new_iterator_result(realm, value, done)?),
            ),
            AsyncGeneratorSettlement::Reject(reason) => (request.reject, reason),
        };
        // Allocate the iterator result before transferring the request out of
        // the queue. On an internal allocation failure the capability remains
        // reachable instead of being orphaned permanently.
        self.remove_front_async_generator_request(generator)?;
        let _ = self.call_internal(realm, &target, Value::Undefined, &[value])?;
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
