//! `%Promise%`, resolving functions, and reaction semantics.
//!
//! This mirrors pinned QuickJS's `JSPromiseData`/runtime-job split: Promise
//! objects retain state and pending reactions in the heap, while executable
//! jobs live on the runtime FIFO and are drained only by an explicit host.

use crate::heap::{
    InternalCallableData, PromiseCapabilityData, PromiseCapabilityExecutorData, PromiseNativeKind,
    PromiseReaction, PromiseReactionKind, PromiseRealmData, PromiseResolvingKind, PromiseState,
};

use super::super::*;

mod all;
mod convenience;
mod finally;

/// One notification from QuickJS's host Promise rejection tracker boundary.
///
/// `handled == false` reports a rejection which had no handler when it was
/// published. `handled == true` reports that a handler was attached later.
/// The Promise and reason are rooted for the duration of the callback and may
/// be cloned by the host when it needs to retain them.
#[derive(Clone)]
pub struct PromiseRejectionEvent {
    pub(super) context: ContextId,
    pub(super) promise: ObjectRef,
    pub(super) reason: Value,
    pub(super) handled: bool,
}

pub(in crate::runtime) type HostPromiseRejectionTracker = Rc<dyn Fn(PromiseRejectionEvent)>;

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

struct RootedPromiseCapability {
    promise: ObjectRef,
    resolve: CallableRef,
    reject: CallableRef,
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

    pub(in crate::runtime) fn initialize_promise_intrinsic(
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
    ) {
        let tracker = self.0.promise_rejection_tracker.borrow().clone();
        if let Some(tracker) = tracker {
            tracker(PromiseRejectionEvent {
                context: realm,
                promise,
                reason,
                handled,
            });
        }
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

    fn new_internal_promise_function(
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

    fn new_default_promise_capability(
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

    fn new_promise_capability(
        &self,
        realm: ContextId,
        constructor: Option<&CallableRef>,
    ) -> Result<NativeConversion<RootedPromiseCapability>, RuntimeError> {
        let Some(constructor) = constructor else {
            return Ok(NativeConversion::Value(
                self.new_default_promise_capability(realm)?,
            ));
        };
        let executor = self.new_internal_promise_function(
            realm,
            NativeFunctionId::PromiseCapabilityExecutor,
            2,
            2,
            InternalCallableData::PromiseCapabilityExecutor(
                PromiseCapabilityExecutorData::default(),
            ),
        )?;
        let completion = self.construct_internal(
            realm,
            constructor,
            constructor,
            &[Value::Object(executor.as_object().clone())],
        )?;
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

    fn promise_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(new_target) = new_target else {
            return Err(RuntimeError::Invariant(
                "Promise constructor new.target was not an object",
            ));
        };
        let key = self.intern_property_key("prototype")?;
        match self.get_property_in_realm(realm, &new_target, &key)? {
            Completion::Return(Value::Object(prototype)) => Ok(NativeConversion::Value(prototype)),
            Completion::Return(_) => {
                let new_target = self.callable_from_value(Value::Object(new_target))?;
                let fallback_realm = self.callable_realm(&new_target)?;
                let prototype = self.promise_realm_data(fallback_realm)?.prototype;
                Ok(NativeConversion::Value(ObjectRef::from_borrowed_handle(
                    self.clone(),
                    prototype,
                )?))
            }
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    pub(in crate::runtime) fn call_promise_native(
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
            PromiseNativeKind::All => self.call_promise_all(realm, invocation, arguments),
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
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise constructor did not receive a constructor invocation",
            ));
        };
        let executor = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise executor argv was not padded",
            ))?;

        // Pinned QuickJS checks callability before the observable prototype
        // lookup on `new.target`.
        let executor = self.callable_from_value(executor)?;
        let prototype = match self.promise_prototype_from_new_target(realm, new_target)? {
            NativeConversion::Value(prototype) => prototype,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let promise = self.new_promise_object(&prototype)?;
        let (resolve, reject) = self.create_promise_resolving_functions(realm, &promise)?;
        let completion = self.call_internal(
            realm,
            &executor,
            Value::Undefined,
            &[
                Value::Object(resolve.as_object().clone()),
                Value::Object(reject.as_object().clone()),
            ],
        )?;
        if let Completion::Throw(reason) = completion {
            match self.call_internal(realm, &reject, Value::Undefined, &[reason])? {
                Completion::Return(_) => {}
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        Ok(Completion::Return(Value::Object(promise)))
    }

    pub(in crate::runtime) fn call_promise_resolving(
        &self,
        realm: ContextId,
        target_kind: PromiseResolvingKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise resolving function received a constructor invocation",
            ));
        };
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "Promise resolving function had no internal capture",
            ))?;
        let InternalCallableData::PromiseResolving {
            promise,
            already_resolved,
            kind,
        } = internal
        else {
            return Err(RuntimeError::Invariant(
                "Promise resolving function had the wrong internal capture",
            ));
        };
        if kind != target_kind {
            return Err(RuntimeError::Invariant(
                "Promise resolving target disagreed with its capture",
            ));
        }
        if already_resolved.replace(true) {
            return Ok(Completion::Return(Value::Undefined));
        }
        let resolution = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise resolving argv was not padded",
            ))?;
        let promise_root = ObjectRef::from_borrowed_handle(self.clone(), promise)?;

        if kind == PromiseResolvingKind::Reject {
            self.settle_promise(realm, &promise_root, PromiseState::Rejected, resolution)?;
            return Ok(Completion::Return(Value::Undefined));
        }
        let Value::Object(resolution_object) = resolution.clone() else {
            self.settle_promise(realm, &promise_root, PromiseState::Fulfilled, resolution)?;
            return Ok(Completion::Return(Value::Undefined));
        };
        if resolution_object == promise_root {
            let reason =
                self.new_native_error(realm, NativeErrorKind::Type, "promise self resolution")?;
            self.settle_promise(realm, &promise_root, PromiseState::Rejected, reason)?;
            return Ok(Completion::Return(Value::Undefined));
        }

        let then_key = self.intern_property_key("then")?;
        let then = match self.get_property_in_realm(realm, &resolution_object, &then_key)? {
            Completion::Return(value) => value,
            Completion::Throw(reason) => {
                self.settle_promise(realm, &promise_root, PromiseState::Rejected, reason)?;
                return Ok(Completion::Return(Value::Undefined));
            }
        };
        let then = match then {
            Value::Object(object) => self.as_callable(&object)?,
            _ => None,
        };
        if let Some(then) = then {
            self.enqueue_promise_resolve_thenable_job(
                realm,
                promise,
                resolution_object.object_id(),
                then.as_object().object_id(),
            )?;
        } else {
            self.settle_promise(realm, &promise_root, PromiseState::Fulfilled, resolution)?;
        }
        Ok(Completion::Return(Value::Undefined))
    }

    pub(in crate::runtime) fn call_promise_capability_executor(
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
        if let Err(error) = settlement {
            self.discard_prepared_jobs(prepared_jobs)?;
            return Err(error);
        }
        if state == PromiseState::Rejected && !was_handled {
            self.notify_host_promise_rejection_tracker(
                realm,
                promise.clone(),
                result.clone(),
                false,
            );
        }
        self.publish_prepared_jobs(prepared_jobs);
        drop(result);
        Ok(())
    }

    pub(in crate::runtime) fn execute_promise_resolve_thenable_job(
        &self,
        realm: ContextId,
        promise: ObjectId,
        thenable: ObjectId,
        then: ObjectId,
    ) -> Result<Completion, RuntimeError> {
        let promise = ObjectRef::from_borrowed_handle(self.clone(), promise)?;
        let thenable = ObjectRef::from_borrowed_handle(self.clone(), thenable)?;
        let then = ObjectRef::from_borrowed_handle(self.clone(), then)?;
        let then = self.as_callable(&then)?.ok_or(RuntimeError::Invariant(
            "queued Promise then action was no longer callable",
        ))?;
        // This pair must be fresh: the resolving function that enqueued this
        // job has already flipped its own shared first-call bit.
        let (resolve, reject) = self.create_promise_resolving_functions(realm, &promise)?;
        let completion = self.call_internal(
            realm,
            &then,
            Value::Object(thenable),
            &[
                Value::Object(resolve.as_object().clone()),
                Value::Object(reject.as_object().clone()),
            ],
        )?;
        match completion {
            Completion::Return(value) => Ok(Completion::Return(value)),
            Completion::Throw(reason) => {
                self.call_internal(realm, &reject, Value::Undefined, &[reason])
            }
        }
    }

    pub(in crate::runtime) fn execute_promise_reaction_job(
        &self,
        realm: ContextId,
        reaction: &PromiseReaction,
        argument: &RawValue,
    ) -> Result<Completion, RuntimeError> {
        let argument = self.root_raw_value(argument)?;
        let handler_completion = if let Some(handler) = reaction.handler {
            let handler = ObjectRef::from_borrowed_handle(self.clone(), handler)?;
            let handler = self.as_callable(&handler)?.ok_or(RuntimeError::Invariant(
                "queued Promise reaction handler was no longer callable",
            ))?;
            self.call_internal(realm, &handler, Value::Undefined, &[argument])?
        } else if reaction.kind == PromiseReactionKind::Reject {
            Completion::Throw(argument)
        } else {
            Completion::Return(argument)
        };
        let (target, value) = match handler_completion {
            Completion::Return(value) => (reaction.capability.resolve, value),
            Completion::Throw(value) => (reaction.capability.reject, value),
        };
        let target = ObjectRef::from_borrowed_handle(self.clone(), target)?;
        let target = self.as_callable(&target)?.ok_or(RuntimeError::Invariant(
            "Promise reaction capability was no longer callable",
        ))?;
        self.call_internal(realm, &target, Value::Undefined, &[value])
    }

    fn promise_species_constructor(
        &self,
        realm: ContextId,
        promise: &ObjectRef,
    ) -> Result<NativeConversion<Option<CallableRef>>, RuntimeError> {
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
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.prototype.then received a constructor invocation",
            ));
        };
        let Value::Object(promise) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a promise",
            )?));
        };
        if !matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(promise.object_id())?
                .payload,
            ObjectPayload::Promise(_)
        ) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a promise",
            )?));
        }
        let constructor = match self.promise_species_constructor(realm, &promise)? {
            NativeConversion::Value(constructor) => constructor,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let capability = match self.new_promise_capability(realm, constructor.as_ref())? {
            NativeConversion::Value(capability) => capability,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let handler_id = |value: &Value| -> Result<Option<ObjectId>, RuntimeError> {
            let Value::Object(object) = value else {
                return Ok(None);
            };
            Ok(self.as_callable(object)?.map(|_| object.object_id()))
        };
        let fulfill = PromiseReaction {
            kind: PromiseReactionKind::Fulfill,
            handler: handler_id(arguments.readable.first().ok_or(RuntimeError::Invariant(
                "Promise.then fulfill argv was not padded",
            ))?)?,
            capability: capability.raw(),
        };
        let reject = PromiseReaction {
            kind: PromiseReactionKind::Reject,
            handler: handler_id(arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "Promise.then reject argv was not padded",
            ))?)?,
            capability: capability.raw(),
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
                    );
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
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.prototype.catch received a constructor invocation",
            ));
        };
        let then_key = self.intern_property_key("then")?;
        let then = match self.get_value_property_in_realm(realm, this_value.clone(), &then_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let then = match then {
            Value::Object(object) => match self.as_callable(&object)? {
                Some(callable) => callable,
                None => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                }
            },
            _ => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            }
        };
        let on_rejected = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise.catch reject argv was not padded",
            ))?;
        self.call_internal(realm, &then, this_value, &[Value::Undefined, on_rejected])
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
        let Value::Object(constructor_object) = this_value.clone() else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise resolve/reject argv was not padded",
            ))?;
        if kind == PromiseNativeKind::Resolve
            && let Value::Object(promise) = &argument
            && matches!(
                self.0
                    .state
                    .borrow()
                    .heap
                    .object(promise.object_id())?
                    .payload,
                ObjectPayload::Promise(_)
            )
        {
            let constructor_key = self.intern_property_key("constructor")?;
            let promise_constructor =
                match self.get_property_in_realm(realm, promise, &constructor_key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            if promise_constructor.same_value(&this_value) {
                return Ok(Completion::Return(argument));
            }
        }
        let constructor =
            match self.constructor_from_value(realm, Value::Object(constructor_object))? {
                NativeConversion::Value(constructor) => constructor,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let capability = match self.new_promise_capability(realm, Some(&constructor))? {
            NativeConversion::Value(capability) => capability,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target = if kind == PromiseNativeKind::Reject {
            &capability.reject
        } else {
            &capability.resolve
        };
        match self.call_internal(realm, target, Value::Undefined, &[argument])? {
            Completion::Return(_) => Ok(Completion::Return(Value::Object(capability.promise))),
            Completion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }
}
