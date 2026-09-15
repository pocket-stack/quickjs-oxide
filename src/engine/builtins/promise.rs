//! `%Promise%`, resolving functions, and reaction semantics.
//!
//! This mirrors pinned QuickJS's `JSPromiseData`/runtime-job split: Promise
//! objects retain state and pending reactions in the heap, while executable
//! jobs live on the runtime FIFO and are drained only by an explicit host.

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{NativeFunctionId, PromiseNativeKind, PromiseResolvingKind};
use crate::engine::heap::{
    ContextId, InternalCallableData, ObjectData, ObjectId, ObjectPayload, PromiseCapabilityData,
    PromiseCapabilityExecutorData, PromiseReaction, PromiseReactionKind, PromiseRealmData,
    PromiseState, RawModuleRef, RawValue,
};
use crate::engine::object::{
    AccessorValue, CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor,
    PropertyKey, WellKnownSymbol,
};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{ConstructorRef, NativeArguments, NativeInvocation};
use std::cell::Cell;
use std::rc::Rc;

mod all;
mod convenience;
mod finally;
pub(crate) mod operation;

/// One notification from QuickJS's host Promise rejection tracker boundary.
///
/// `handled == false` reports a rejection which had no handler when it was
/// published. `handled == true` reports that a handler was attached later.
/// The Promise and reason are rooted for the duration of the callback and may
/// be cloned by the host when it needs to retain them.
#[derive(Clone)]
pub struct PromiseRejectionEvent {
    pub(crate) context: ContextId,
    pub(crate) promise: ObjectRef,
    pub(crate) reason: Value,
    pub(crate) handled: bool,
}

pub(crate) type HostPromiseRejectionTracker = Rc<dyn Fn(PromiseRejectionEvent)>;

/// Public snapshot of a genuine Promise's settled state and result.
///
/// This is the Rust counterpart of QuickJS `JS_PromiseState` plus
/// `JS_PromiseResult`. A pending Promise always reports `undefined`.
#[derive(Clone, Debug, PartialEq)]
pub struct PromiseSnapshot {
    state: PromiseState,
    result: Value,
}

impl PromiseSnapshot {
    #[must_use]
    pub const fn state(&self) -> PromiseState {
        self.state
    }

    #[must_use]
    pub const fn result(&self) -> &Value {
        &self.result
    }
}

impl PromiseRejectionEvent {
    #[must_use]
    pub const fn context(&self) -> ContextId {
        self.context
    }

    #[must_use]
    pub const fn promise(&self) -> &ObjectRef {
        &self.promise
    }

    #[must_use]
    pub const fn reason(&self) -> &Value {
        &self.reason
    }

    #[must_use]
    pub const fn is_handled(&self) -> bool {
        self.handled
    }
}

pub(crate) struct RootedPromiseCapability {
    pub(crate) promise: ObjectRef,
    pub(crate) resolve: CallableRef,
    pub(crate) reject: CallableRef,
}

impl RootedPromiseCapability {
    fn raw(&self) -> PromiseCapabilityData {
        PromiseCapabilityData {
            resolve: self.resolve.as_object().object_id(),
            reject: self.reject.as_object().object_id(),
        }
    }
}

impl Runtime {
    /// Inspect one genuine Promise without invoking user-overridable
    /// properties or handlers.
    pub fn promise_snapshot(
        &self,
        promise: &ObjectRef,
    ) -> Result<Option<PromiseSnapshot>, RuntimeError> {
        if !promise.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Promise"));
        }
        let snapshot = {
            let state = self.0.state.borrow();
            if !matches!(
                state.heap.object(promise.object_id())?.payload,
                ObjectPayload::Promise(_)
            ) {
                return Ok(None);
            }
            state.heap.promise_snapshot(promise.object_id())?
        };
        Ok(Some(PromiseSnapshot {
            state: snapshot.state,
            result: self.root_raw_value(&snapshot.result)?,
        }))
    }

    /// Install the runtime-wide host Promise rejection tracker.
    ///
    /// This mirrors `JS_SetHostPromiseRejectionTracker`. Replacing an existing
    /// tracker drops it immediately; use
    /// [`Runtime::clear_host_promise_rejection_tracker`] when tracking is no
    /// longer required.
    pub fn set_host_promise_rejection_tracker<F>(&self, tracker: F)
    where
        F: Fn(PromiseRejectionEvent) + 'static,
    {
        *self.0.promise_rejection_tracker.borrow_mut() = Some(Rc::new(tracker));
    }

    /// Remove the runtime-wide host Promise rejection tracker.
    pub fn clear_host_promise_rejection_tracker(&self) {
        self.0.promise_rejection_tracker.borrow_mut().take();
    }

    pub(crate) fn initialize_promise_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let promise_prototype = self.new_object(Some(object_prototype))?;
        self.define_native_builtin_auto_init(
            &promise_prototype,
            realm,
            NativeFunctionId::Promise(PromiseNativeKind::Then),
            "then",
            2,
            2,
        )?;
        self.define_native_builtin_auto_init(
            &promise_prototype,
            realm,
            NativeFunctionId::Promise(PromiseNativeKind::Catch),
            "catch",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            &promise_prototype,
            realm,
            NativeFunctionId::Promise(PromiseNativeKind::Finally),
            "finally",
            1,
            1,
        )?;
        self.define_promise_to_string_tag(&promise_prototype)?;

        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::Promise(PromiseNativeKind::Constructor),
            1,
            "Promise",
            1,
        )?;
        for (kind, name, length) in [
            (PromiseNativeKind::Resolve, "resolve", 1),
            (PromiseNativeKind::Reject, "reject", 1),
            (PromiseNativeKind::All, "all", 1),
            (PromiseNativeKind::AllSettled, "allSettled", 1),
            (PromiseNativeKind::Any, "any", 1),
            (PromiseNativeKind::Try, "try", 1),
            (PromiseNativeKind::Race, "race", 1),
            (PromiseNativeKind::WithResolvers, "withResolvers", 0),
        ] {
            self.define_native_builtin_auto_init(
                constructor.as_object(),
                realm,
                NativeFunctionId::Promise(kind),
                name,
                length,
                length,
            )?;
        }
        let species_getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::Promise(PromiseNativeKind::Species),
            0,
            "get [Symbol.species]",
            0,
        )?;
        let species = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
        if !self.define_own_property(
            constructor.as_object(),
            &species,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(species_getter)),
                set: DescriptorField::Present(AccessorValue::Undefined),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Promise species definition was rejected",
            ));
        }

        self.define_function_data_property(
            global_object,
            "Promise",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, &promise_prototype)?;
        self.0.state.borrow_mut().heap.attach_promise_intrinsics(
            realm,
            PromiseRealmData {
                prototype: promise_prototype.object_id(),
                constructor: constructor.as_object().object_id(),
            },
        )?;
        Ok(())
    }

    fn define_promise_to_string_tag(&self, object: &ObjectRef) -> Result<(), RuntimeError> {
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("Promise"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Promise toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    fn promise_realm_data(&self, realm: ContextId) -> Result<PromiseRealmData, RuntimeError> {
        self.0
            .state
            .borrow()
            .heap
            .context(realm)?
            .promise
            .ok_or(RuntimeError::Invariant("realm has no Promise intrinsics"))
    }

    fn notify_host_promise_rejection_tracker(
        &self,
        realm: ContextId,
        promise: ObjectRef,
        reason: Value,
        handled: bool,
    ) -> Result<(), RuntimeError> {
        let tracker = self.0.promise_rejection_tracker.borrow().clone();
        if let Some(tracker) = tracker {
            self.with_host_callback(|| {
                tracker(PromiseRejectionEvent {
                    context: realm,
                    promise,
                    reason,
                    handled,
                })
            })?;
        }
        Ok(())
    }

    fn new_promise_object(&self, prototype: &ObjectRef) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Promise prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::promise(shape, Vec::new()))
        {
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

    pub(crate) fn new_internal_promise_function(
        &self,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        length: i32,
        internal: InternalCallableData,
    ) -> Result<CallableRef, RuntimeError> {
        let _operation = self.operation();
        let function_prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .function_prototype;
        let function_prototype = ObjectRef::from_borrowed_handle(self.clone(), function_prototype)?;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(function_prototype.object_id()), &[])?;
        let retained_atoms = match &internal {
            InternalCallableData::PromiseFinallyThunk { value } => {
                match state.retain_raw_value_atoms(std::iter::once(value)) {
                    Ok(atoms) => atoms,
                    Err(error) => {
                        let cleanup = state.heap.release_shape(shape)?;
                        state.apply_cleanup(cleanup)?;
                        return Err(error);
                    }
                }
            }
            _ => Vec::new(),
        };
        let object = match state
            .heap
            .allocate_object(ObjectData::bound_internal_native_function(
                shape,
                Vec::new(),
                target,
                realm,
                min_readable_args,
                internal,
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
        let callable =
            CallableRef::from_validated_object(ObjectRef::from_owned_handle(self.clone(), object));
        self.define_function_data_property(
            callable.as_object(),
            "length",
            Value::Int(length),
            false,
            true,
        )?;
        self.define_function_data_property(
            callable.as_object(),
            "name",
            Value::String(JsString::from_static("")),
            false,
            true,
        )?;
        Ok(callable)
    }

    fn create_promise_resolving_functions(
        &self,
        realm: ContextId,
        promise: &ObjectRef,
    ) -> Result<(CallableRef, CallableRef), RuntimeError> {
        let already_resolved = Rc::new(Cell::new(false));
        let make = |kind| {
            self.new_internal_promise_function(
                realm,
                NativeFunctionId::PromiseResolving(kind),
                1,
                1,
                InternalCallableData::PromiseResolving {
                    promise: promise.object_id(),
                    already_resolved: already_resolved.clone(),
                    kind,
                },
            )
        };
        let resolve = make(PromiseResolvingKind::Resolve)?;
        let reject = make(PromiseResolvingKind::Reject)?;
        Ok((resolve, reject))
    }

    pub(crate) fn new_default_promise_capability(
        &self,
        realm: ContextId,
    ) -> Result<RootedPromiseCapability, RuntimeError> {
        let prototype = self.promise_realm_data(realm)?.prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let promise = self.new_promise_object(&prototype)?;
        let (resolve, reject) = self.create_promise_resolving_functions(realm, &promise)?;
        Ok(RootedPromiseCapability {
            promise,
            resolve,
            reject,
        })
    }

    /// Allocate the realm's intrinsic Promise capability and reject it without
    /// re-entering the native-call dispatcher.
    ///
    /// Async bytecode uses this at the host-stack preflight boundary: the VM
    /// body cannot safely start, but an async call must still return its
    /// caller-realm Promise. Calling the ordinary reject function here would
    /// repeat the same host-stack check before reaching Promise settlement.
    pub(crate) fn new_rejected_default_promise(
        &self,
        realm: ContextId,
        reason: Value,
    ) -> Result<ObjectRef, RuntimeError> {
        let capability = self.new_default_promise_capability(realm)?;
        let promise = capability.promise.clone();
        self.settle_promise(realm, &promise, PromiseState::Rejected, reason)?;
        Ok(promise)
    }

    fn new_promise_capability(
        &self,
        realm: ContextId,
        constructor: Option<&ConstructorRef>,
    ) -> Result<NativeConversion<RootedPromiseCapability>, RuntimeError> {
        let Some(constructor) = constructor else {
            return Ok(NativeConversion::Value(
                self.new_default_promise_capability(realm)?,
            ));
        };
        let executor = self.prepare_promise_capability_executor(realm)?;
        let completion = self.construct_constructor_internal(
            realm,
            constructor,
            constructor,
            &[Value::Object(executor.as_object().clone())],
        )?;
        self.finish_promise_capability(realm, &executor, completion)
    }

    fn prepare_promise_capability_executor(
        &self,
        realm: ContextId,
    ) -> Result<CallableRef, RuntimeError> {
        self.new_internal_promise_function(
            realm,
            NativeFunctionId::PromiseCapabilityExecutor,
            2,
            2,
            InternalCallableData::PromiseCapabilityExecutor(
                PromiseCapabilityExecutorData::default(),
            ),
        )
    }

    fn finish_promise_capability(
        &self,
        realm: ContextId,
        executor: &CallableRef,
        completion: Completion,
    ) -> Result<NativeConversion<RootedPromiseCapability>, RuntimeError> {
        let promise = match completion {
            Completion::Return(Value::Object(promise)) => promise,
            Completion::Return(_) => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            }
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let capture = self
            .0
            .state
            .borrow()
            .heap
            .promise_capability_capture(executor.as_object().object_id())?;
        let (Some(resolve), Some(reject)) = (capture.resolve, capture.reject) else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "resolving function is not callable",
            )?));
        };
        let resolve = self.root_raw_value(&resolve)?;
        let reject = self.root_raw_value(&reject)?;
        let resolve = match resolve {
            Value::Object(object) => match self.as_callable(&object)? {
                Some(callable) => callable,
                None => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "resolving function is not callable",
                    )?));
                }
            },
            _ => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "resolving function is not callable",
                )?));
            }
        };
        let reject = match reject {
            Value::Object(object) => match self.as_callable(&object)? {
                Some(callable) => callable,
                None => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "resolving function is not callable",
                    )?));
                }
            },
            _ => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "resolving function is not callable",
                )?));
            }
        };
        Ok(NativeConversion::Value(RootedPromiseCapability {
            promise,
            resolve,
            reject,
        }))
    }

    pub(crate) fn call_promise_native(
        &self,
        realm: ContextId,
        kind: PromiseNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            PromiseNativeKind::Constructor => {
                self.call_promise_constructor(realm, invocation, arguments)
            }
            PromiseNativeKind::Species => self.call_promise_species(invocation),
            PromiseNativeKind::Then => self.call_promise_then(realm, invocation, arguments),
            PromiseNativeKind::Catch => self.call_promise_catch(realm, invocation, arguments),
            PromiseNativeKind::Finally => self.call_promise_finally(realm, invocation, arguments),
            PromiseNativeKind::Resolve | PromiseNativeKind::Reject => {
                self.call_promise_static_resolve(realm, kind, invocation, arguments)
            }
            PromiseNativeKind::All | PromiseNativeKind::AllSettled | PromiseNativeKind::Any => {
                self.call_promise_aggregate(kind, realm, invocation, arguments)
            }
            PromiseNativeKind::Try => self.call_promise_try(realm, invocation, arguments),
            PromiseNativeKind::Race => self.call_promise_race(realm, invocation, arguments),
            PromiseNativeKind::WithResolvers => self.call_promise_with_resolvers(realm, invocation),
        }
    }

    fn call_promise_species(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise species did not receive a getter invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    fn call_promise_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::start(
            self,
            realm,
            NativeFunctionId::Promise(PromiseNativeKind::Constructor),
            &invocation,
            arguments,
        )?
        .finish(self, realm)
    }

    pub(crate) fn call_promise_resolving(
        &self,
        realm: ContextId,
        target_kind: PromiseResolvingKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::start(
            self,
            realm,
            NativeFunctionId::PromiseResolving(target_kind),
            &invocation,
            arguments,
        )?
        .finish(self, realm)
    }

    pub(crate) fn call_promise_capability_executor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise capability executor received a constructor invocation",
            ));
        };
        let active = self.active_function()?;
        let resolve = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise capability resolve argv was not padded",
            ))?;
        let reject = arguments
            .readable
            .get(1)
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise capability reject argv was not padded",
            ))?;
        let raw_resolve = self.raw_property_value(&resolve)?;
        let raw_reject = self.raw_property_value(&reject)?;
        let mut state = self.0.state.borrow_mut();
        let retained = state.retain_raw_value_atoms([&raw_resolve, &raw_reject])?;
        match state
            .heap
            .set_promise_capability_capture(active.object_id(), raw_resolve, raw_reject)
        {
            Ok(true) => {
                drop(state);
                drop(resolve);
                drop(reject);
                Ok(Completion::Return(Value::Undefined))
            }
            Ok(false) => {
                state.release_atoms(retained)?;
                drop(state);
                Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "resolving function already set",
                )?))
            }
            Err(error) => {
                state.release_atoms(retained)?;
                Err(error.into())
            }
        }
    }

    fn settle_promise(
        &self,
        realm: ContextId,
        promise: &ObjectRef,
        state: PromiseState,
        result: Value,
    ) -> Result<(), RuntimeError> {
        let snapshot = self
            .0
            .state
            .borrow()
            .heap
            .promise_snapshot(promise.object_id())?;
        if snapshot.state != PromiseState::Pending {
            return Ok(());
        }
        let was_handled = snapshot.is_handled;
        let reactions = match state {
            PromiseState::Fulfilled => snapshot.fulfill_reactions,
            PromiseState::Rejected => snapshot.reject_reactions,
            PromiseState::Pending => {
                return Err(RuntimeError::Invariant(
                    "Promise settlement requested the pending state",
                ));
            }
        };
        let raw = self.raw_property_value(&result)?;

        // Prepare job-owned roots before detaching the Promise's reactions,
        // but do not publish the jobs yet. QuickJS exposes the settled state to
        // its rejection tracker before the selected reactions enter the FIFO;
        // a reentrant tracker can therefore enqueue work ahead of them.
        let mut prepared_jobs = Vec::with_capacity(reactions.len());
        for reaction in reactions {
            let job = match self.prepare_promise_reaction_job(realm, reaction, raw.clone()) {
                Ok(job) => job,
                Err(error) => {
                    self.discard_prepared_jobs(prepared_jobs)?;
                    return Err(error);
                }
            };
            prepared_jobs.push(job);
        }

        let prepared_jobs = crate::engine::jobs::PreparedJobs::new(self, prepared_jobs);
        let settlement = (|| -> Result<(), RuntimeError> {
            let mut state_ref = self.0.state.borrow_mut();
            let retained_atom = if let RawValue::Symbol(atom) = &raw {
                state_ref.atoms.retain(*atom)?;
                Some(*atom)
            } else {
                None
            };
            let cleanup = match state_ref
                .heap
                .promise_settle(promise.object_id(), state, raw)
            {
                Ok(cleanup) => cleanup,
                Err(error) => {
                    if let Some(atom) = retained_atom {
                        state_ref.atoms.release(atom)?;
                    }
                    return Err(error.into());
                }
            };
            state_ref.apply_cleanup(cleanup)
        })();
        settlement?;
        if state == PromiseState::Rejected && !was_handled {
            self.notify_host_promise_rejection_tracker(
                realm,
                promise.clone(),
                result.clone(),
                false,
            )?;
        }
        prepared_jobs.publish();
        drop(result);
        Ok(())
    }

    pub(crate) fn execute_promise_resolve_thenable_job(
        &self,
        realm: ContextId,
        promise: ObjectId,
        thenable: ObjectId,
        then: ObjectId,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::thenable_job(self, realm, promise, thenable, then)?
            .finish(self, realm)
    }

    pub(crate) fn execute_promise_reaction_job(
        &self,
        realm: ContextId,
        reaction: &PromiseReaction,
        argument: &RawValue,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::reaction_job(self, realm, reaction, argument)?.finish(self, realm)
    }

    fn promise_species_constructor(
        &self,
        realm: ContextId,
        promise: &ObjectRef,
    ) -> Result<NativeConversion<Option<ConstructorRef>>, RuntimeError> {
        let constructor_key = self.intern_property_key("constructor")?;
        let constructor = match self.get_property_in_realm(realm, promise, &constructor_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(constructor, Value::Undefined) {
            return Ok(NativeConversion::Value(None));
        }
        let Value::Object(constructor) = constructor else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let species_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
        let species = match self.get_property_in_realm(realm, &constructor, &species_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(species, Value::Undefined | Value::Null) {
            return Ok(NativeConversion::Value(None));
        }
        self.constructor_from_value(realm, species)
            .map(|result| match result {
                NativeConversion::Value(constructor) => NativeConversion::Value(Some(constructor)),
                NativeConversion::Throw(value) => NativeConversion::Throw(value),
            })
    }

    fn call_promise_then(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::start(
            self,
            realm,
            NativeFunctionId::Promise(PromiseNativeKind::Then),
            &invocation,
            arguments,
        )?
        .finish(self, realm)
    }

    fn finish_promise_then(
        &self,
        realm: ContextId,
        promise: ObjectRef,
        handlers: [Value; 2],
        capability: RootedPromiseCapability,
    ) -> Result<Completion, RuntimeError> {
        let handler_id = |value: &Value| -> Result<Option<ObjectId>, RuntimeError> {
            let Value::Object(object) = value else {
                return Ok(None);
            };
            Ok(self.as_callable(object)?.map(|_| object.object_id()))
        };
        let fulfill = PromiseReaction {
            kind: PromiseReactionKind::Fulfill,
            handler: handler_id(&handlers[0])?,
            capability: Some(capability.raw()),
        };
        let reject = PromiseReaction {
            kind: PromiseReactionKind::Reject,
            handler: handler_id(&handlers[1])?,
            capability: Some(capability.raw()),
        };
        let snapshot = self
            .0
            .state
            .borrow()
            .heap
            .promise_snapshot(promise.object_id())?;
        match snapshot.state {
            PromiseState::Pending => self.0.state.borrow_mut().heap.promise_add_reactions(
                promise.object_id(),
                fulfill,
                reject,
            )?,
            PromiseState::Fulfilled => {
                self.enqueue_promise_reaction_job(realm, fulfill, snapshot.result)?;
            }
            PromiseState::Rejected => {
                if !snapshot.is_handled {
                    let reason = self.root_raw_value(&snapshot.result)?;
                    self.notify_host_promise_rejection_tracker(
                        realm,
                        promise.clone(),
                        reason,
                        true,
                    )?;
                }
                self.enqueue_promise_reaction_job(realm, reject, snapshot.result)?;
            }
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .promise_mark_handled(promise.object_id())?;
        Ok(Completion::Return(Value::Object(capability.promise)))
    }

    fn call_promise_catch(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::start(
            self,
            realm,
            NativeFunctionId::Promise(PromiseNativeKind::Catch),
            &invocation,
            arguments,
        )?
        .finish(self, realm)
    }

    fn call_promise_static_resolve(
        &self,
        realm: ContextId,
        kind: PromiseNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise resolve/reject received a constructor invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise resolve/reject argv was not padded",
            ))?;
        self.promise_static_resolve_core(realm, kind, this_value, argument)
    }

    fn promise_static_resolve_core(
        &self,
        realm: ContextId,
        kind: PromiseNativeKind,
        this_value: Value,
        argument: Value,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::static_resolve(self, realm, kind, this_value, argument)?
            .finish(self, realm)
    }

    /// QuickJS's `js_promise_resolve(ctx, ctx->promise_ctor, ...)` boundary
    /// used by `await`. The cached realm constructor is selected directly:
    /// replacing global `Promise` or its public `resolve` property cannot
    /// intercept async-function suspension.
    pub(crate) fn promise_resolve_intrinsic(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<Completion, RuntimeError> {
        self.prepare_intrinsic_promise_resolve(realm, value)?
            .finish(self, realm)
    }

    pub(crate) fn prepare_intrinsic_promise_resolve(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<operation::PromiseStep, RuntimeError> {
        let constructor = self.promise_realm_data(realm)?.constructor;
        let constructor = ObjectRef::from_borrowed_handle(self.clone(), constructor)?;
        operation::PromiseStep::static_resolve(
            self,
            realm,
            PromiseNativeKind::Resolve,
            Value::Object(constructor),
            value,
        )
    }

    /// Register the two private await continuations without allocating the
    /// spec's unobservable thrown-away capability. Pinned QuickJS deliberately
    /// represents that capability as two `undefined` resolving functions; a
    /// continuation completion is therefore consumed by the reaction job.
    pub(crate) fn perform_promise_then_without_capability(
        &self,
        realm: ContextId,
        promise: &ObjectRef,
        fulfill: &CallableRef,
        reject: &CallableRef,
    ) -> Result<(), RuntimeError> {
        self.perform_promise_then_internal(realm, promise, Some(fulfill), Some(reject), None)
    }

    /// Register internal handlers while settling a caller-owned Promise
    /// capability. Async-from-Sync iterator continuation uses this exact
    /// `PerformPromiseThen` boundary: it must not observe a mutable `.then`,
    /// perform species lookup, or allocate an otherwise visible extra Promise.
    pub(crate) fn perform_promise_then_with_capability(
        &self,
        realm: ContextId,
        promise: &ObjectRef,
        fulfill: Option<&CallableRef>,
        reject: Option<&CallableRef>,
        capability: &RootedPromiseCapability,
    ) -> Result<(), RuntimeError> {
        self.perform_promise_then_internal(realm, promise, fulfill, reject, Some(capability.raw()))
    }

    /// QuickJS's module evaluator calls its private `js_promise_then`, so the
    /// otherwise discarded result Promise still observes constructor and
    /// `@@species`. This helper performs that full front half before using the
    /// authenticated internal module callbacks as the reactions.
    pub(crate) fn attach_module_evaluation_handlers(
        &self,
        realm: ContextId,
        promise: &ObjectRef,
        fulfill: &CallableRef,
        reject: &CallableRef,
    ) -> Result<NativeConversion<()>, RuntimeError> {
        match operation::PromiseStep::module_then(
            self,
            realm,
            promise.clone(),
            fulfill.clone(),
            reject.clone(),
        )?
        .finish(self, realm)?
        {
            Completion::Return(_) => Ok(NativeConversion::Value(())),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    /// Attach QuickJS's private dynamic-import continuation to the cached
    /// module-evaluation Promise. The full Promise-then path is required here:
    /// TLA may leave the evaluation pending, while constructor/@@species on an
    /// already-settled Promise remains observable.
    pub(crate) fn attach_dynamic_import_finish(
        &self,
        realm: ContextId,
        promise: &ObjectRef,
        module: RawModuleRef,
        resolve: ObjectId,
        reject: ObjectId,
    ) -> Result<NativeConversion<()>, RuntimeError> {
        match operation::PromiseStep::dynamic_import_then(
            self,
            realm,
            promise.clone(),
            module,
            resolve,
            reject,
        )?
        .finish(self, realm)?
        {
            Completion::Return(_) => Ok(NativeConversion::Value(())),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    fn perform_promise_then_internal(
        &self,
        realm: ContextId,
        promise: &ObjectRef,
        fulfill: Option<&CallableRef>,
        reject: Option<&CallableRef>,
        capability: Option<PromiseCapabilityData>,
    ) -> Result<(), RuntimeError> {
        if !matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(promise.object_id())?
                .payload,
            ObjectPayload::Promise(_)
        ) {
            return Err(RuntimeError::Invariant(
                "internal await continuation target is not a Promise",
            ));
        }
        let fulfill = PromiseReaction {
            kind: PromiseReactionKind::Fulfill,
            handler: fulfill.map(|handler| handler.as_object().object_id()),
            capability,
        };
        let reject = PromiseReaction {
            kind: PromiseReactionKind::Reject,
            handler: reject.map(|handler| handler.as_object().object_id()),
            capability,
        };
        let snapshot = self
            .0
            .state
            .borrow()
            .heap
            .promise_snapshot(promise.object_id())?;
        match snapshot.state {
            PromiseState::Pending => self.0.state.borrow_mut().heap.promise_add_reactions(
                promise.object_id(),
                fulfill,
                reject,
            )?,
            PromiseState::Fulfilled => {
                self.enqueue_promise_reaction_job(realm, fulfill, snapshot.result)?;
            }
            PromiseState::Rejected => {
                if !snapshot.is_handled {
                    let reason = self.root_raw_value(&snapshot.result)?;
                    self.notify_host_promise_rejection_tracker(
                        realm,
                        promise.clone(),
                        reason,
                        true,
                    )?;
                }
                self.enqueue_promise_reaction_job(realm, reject, snapshot.result)?;
            }
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .promise_mark_handled(promise.object_id())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promise_snapshot_rejects_a_promise_from_another_runtime() {
        let owner = Runtime::new();
        let mut context = owner.new_context();
        let Value::Object(promise) = context.eval("Promise.resolve(42)").unwrap() else {
            panic!("Promise.resolve did not return an object");
        };
        let observer = Runtime::new();

        assert_eq!(
            observer.promise_snapshot(&promise),
            Err(RuntimeError::WrongRuntime("Promise"))
        );
    }

    #[test]
    fn promise_snapshot_returns_none_for_a_non_promise_object() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();

        assert_eq!(runtime.promise_snapshot(&object).unwrap(), None);
    }

    #[test]
    fn promise_snapshot_roots_an_object_result_across_gc() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(promise) = context
            .eval(
                r#"
                    Promise.resolve(function () {
                        const value = { marker: 42 };
                        value.self = value;
                        return value;
                    }())
                "#,
            )
            .unwrap()
        else {
            panic!("Promise.resolve did not return an object");
        };
        let snapshot = runtime.promise_snapshot(&promise).unwrap().unwrap();
        assert_eq!(snapshot.state(), PromiseState::Fulfilled);
        let Value::Object(result) = snapshot.result() else {
            panic!("fulfilled Promise snapshot did not retain its object result");
        };
        let result_id = result.object_id();
        drop(promise);

        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(result_id).is_ok());
        let marker = runtime.intern_property_key("marker").unwrap();
        assert_eq!(
            context.get_property(result, &marker).unwrap(),
            Value::Int(42)
        );

        drop(snapshot);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(result_id).is_err());
    }
}
