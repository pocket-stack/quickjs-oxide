use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{NativeCProto, NativeFunctionId};
use crate::engine::code::function::metadata::ConstructorKind;
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::heap::roots::VarRefRoot;

use crate::engine::heap::{ContextId, ObjectPayload};
use crate::engine::object::{CallableRef, ObjectRef};
use crate::engine::value::Value;
use crate::engine::value::conversion::NativeConversion;
use crate::engine::vm::Completion;

impl Runtime {
    pub(crate) fn bytecode_for_callable(
        &self,
        callable: &CallableRef,
    ) -> Result<CallableExecution, RuntimeError> {
        let _operation = self.operation();
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("callable"));
        }
        let (bytecode, closure_slots) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(callable.as_object().object_id())?;
            match &object.payload {
                ObjectPayload::NativeFunction { data, .. } => {
                    let realm = data.realm.ok_or(RuntimeError::Invariant(
                        "native function was called before its defining realm was attached",
                    ))?;
                    state.heap.context(realm)?;
                    return Ok(CallableExecution::Native {
                        target: data.target,
                        realm,
                        min_readable_args: data.min_readable_args,
                    });
                }
                ObjectPayload::BoundFunction {
                    target,
                    this_value,
                    arguments,
                } => {
                    let target = *target;
                    let this_value = this_value.clone();
                    let arguments = arguments.clone();
                    drop(state);
                    let target = ObjectRef::from_borrowed_handle(self.clone(), target)?;
                    let target = CallableRef::from_validated_object(target);
                    let this_value = self.root_raw_value(&this_value)?;
                    let arguments = arguments
                        .iter()
                        .map(|argument| self.root_raw_value(argument))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(CallableExecution::Bound {
                        target,
                        this_value,
                        arguments,
                    });
                }
                ObjectPayload::BytecodeFunction {
                    bytecode,
                    closure_slots,
                    ..
                } => (*bytecode, closure_slots.clone()),
                ObjectPayload::Proxy(data) if data.is_callable => {
                    return Ok(CallableExecution::Proxy);
                }
                ObjectPayload::Proxy(_) => {
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Type,
                        "not a function",
                    )));
                }
                ObjectPayload::Ordinary
                | ObjectPayload::ArrayBuffer(_)
                | ObjectPayload::SharedArrayBuffer(_)
                | ObjectPayload::DataView(_)
                | ObjectPayload::TypedArray(_)
                | ObjectPayload::AsyncFunctionState(_)
                | ObjectPayload::RawJson
                | ObjectPayload::Promise(_)
                | ObjectPayload::Date(_)
                | ObjectPayload::RegExp(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::IteratorHelper(_)
                | ObjectPayload::IteratorWrap(_)
                | ObjectPayload::AsyncFromSyncIterator(_)
                | ObjectPayload::IteratorConcat(_)
                | ObjectPayload::Map { .. }
                | ObjectPayload::MapIterator { .. }
                | ObjectPayload::Set { .. }
                | ObjectPayload::WeakMap { .. }
                | ObjectPayload::WeakSet { .. }
                | ObjectPayload::WeakRef { .. }
                | ObjectPayload::FinalizationRegistry(_)
                | ObjectPayload::SetIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. }
                | ObjectPayload::Generator { .. }
                | ObjectPayload::AsyncGenerator(_) => {
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Type,
                        "not a function",
                    )));
                }
            }
        };
        let bytecode = FunctionBytecodeRef::from_borrowed_handle(self.clone(), bytecode)?;
        let closure_slots = closure_slots
            .into_iter()
            .map(|id| VarRefRoot::from_borrowed_handle(self.clone(), id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        })
    }

    /// Snapshot only a direct native callable. Bound and bytecode functions
    /// deliberately return `None`: QuickJS's iterator-next fast path tests the
    /// method object itself and does not unwrap wrappers before deciding which
    /// ABI to use.
    pub(crate) fn direct_native_callable_metadata(
        &self,
        callable: &CallableRef,
    ) -> Result<Option<(NativeFunctionId, ContextId, u8)>, RuntimeError> {
        let _operation = self.operation();
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("native callable"));
        }
        let state = self.0.state.borrow();
        let object = state.heap.object(callable.as_object().object_id())?;
        match &object.payload {
            ObjectPayload::NativeFunction { data, .. } => {
                let realm = data.realm.ok_or(RuntimeError::Invariant(
                    "native function was called before its defining realm was attached",
                ))?;
                state.heap.context(realm)?;
                Ok(Some((data.target, realm, data.min_readable_args)))
            }
            ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. }
            | ObjectPayload::Proxy(_) => Ok(None),
            ObjectPayload::Ordinary
            | ObjectPayload::ArrayBuffer(_)
            | ObjectPayload::SharedArrayBuffer(_)
            | ObjectPayload::DataView(_)
            | ObjectPayload::TypedArray(_)
            | ObjectPayload::AsyncFunctionState(_)
            | ObjectPayload::RawJson
            | ObjectPayload::Promise(_)
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::IteratorHelper(_)
            | ObjectPayload::IteratorWrap(_)
            | ObjectPayload::AsyncFromSyncIterator(_)
            | ObjectPayload::IteratorConcat(_)
            | ObjectPayload::Map { .. }
            | ObjectPayload::MapIterator { .. }
            | ObjectPayload::Set { .. }
            | ObjectPayload::WeakMap { .. }
            | ObjectPayload::WeakSet { .. }
            | ObjectPayload::WeakRef { .. }
            | ObjectPayload::FinalizationRegistry(_)
            | ObjectPayload::SetIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::Generator { .. }
            | ObjectPayload::AsyncGenerator(_) => Err(RuntimeError::Invariant(
                "validated callable no longer has a callable payload",
            )),
        }
    }

    /// Invoke a direct `NativeCProto::IteratorNext` method in the outer
    /// iterator operation's current realm while retaining its raw value/done
    /// result for the VM. Pinned QuickJS calls the C iterator-next pointer
    /// directly in `JS_IteratorNext2`, bypassing the ordinary C-function realm
    /// switch. All other callable shapes use the generic JavaScript call and
    /// iterator-result parsing path.
    pub(crate) fn try_call_native_iterator_next_raw(
        &self,
        realm: ContextId,
        callable: &CallableRef,
        iterator: Value,
    ) -> Result<Option<NativeInvokeOutcome>, RuntimeError> {
        self.validate_value_domain(&iterator, "iterator-next receiver")?;
        let Some((target, _defining_realm, min_readable_args)) =
            self.direct_native_callable_metadata(callable)?
        else {
            return Ok(None);
        };
        if target.descriptor().cproto != NativeCProto::IteratorNext {
            return Ok(None);
        }
        self.invoke_native_function(
            callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Call {
                this_value: iterator,
            },
            &[],
            NativeInvokeMode::IteratorNextRaw,
        )
        .map(Some)
    }

    pub(crate) fn callable_from_value(&self, value: Value) -> Result<CallableRef, RuntimeError> {
        let Value::Object(object) = value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("callable"));
        }
        let is_callable = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
                | ObjectPayload::Proxy(crate::engine::heap::ProxyData {
                    is_callable: true,
                    ..
                })
        );
        if !is_callable {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        }
        Ok(CallableRef::from_validated_object(object))
    }

    /// QuickJS `JS_CallConstructor2` entry for VM operands whose `newTarget`
    /// has not passed through the public Reflect/Context constructor check.
    pub(crate) fn construct_value_with_raw_new_target_internal(
        &self,
        caller_realm: ContextId,
        function: Value,
        new_target: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let constructor = match self.constructor_from_value(caller_realm, function)? {
            NativeConversion::Value(constructor) => constructor,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.construct_constructor_with_raw_new_target_internal(
            caller_realm,
            &constructor,
            new_target,
            arguments,
        )
    }

    /// Raw-newTarget counterpart used after `OP_apply` has already performed
    /// its earlier callability check and materialized the argument list.
    pub(crate) fn construct_callable_with_raw_new_target_internal(
        &self,
        caller_realm: ContextId,
        constructor: &CallableRef,
        new_target: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let constructor = match self
            .constructor_from_value(caller_realm, Value::Object(constructor.as_object().clone()))?
        {
            NativeConversion::Value(constructor) => constructor,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.construct_constructor_with_raw_new_target_internal(
            caller_realm,
            &constructor,
            new_target,
            arguments,
        )
    }

    pub(crate) fn construct_constructor_with_raw_new_target_internal(
        &self,
        caller_realm: ContextId,
        constructor: &ConstructorRef,
        new_target: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        self.construct_internal_with_new_target(
            caller_realm,
            constructor,
            ConstructNewTarget::Raw(new_target),
            arguments,
        )
    }

    pub(crate) fn constructor_from_value(
        &self,
        caller_realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<ConstructorRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("constructor"));
        }
        let is_constructor = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            object_data.is_constructor
        };
        if !is_constructor {
            let value = Value::Object(object);
            return Ok(NativeConversion::Throw(
                self.new_not_constructor_error(caller_realm, &value)?,
            ));
        }
        Ok(NativeConversion::Value(
            ConstructorRef::from_validated_object(object),
        ))
    }

    pub(crate) fn construct_internal(
        &self,
        caller_realm: ContextId,
        constructor: &CallableRef,
        new_target: &CallableRef,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        if !constructor.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("constructor"));
        }
        if !self.is_constructor(constructor.as_object())? {
            return Ok(Completion::Throw(self.new_not_constructor_error(
                caller_realm,
                &Value::Object(constructor.as_object().clone()),
            )?));
        }
        if !new_target.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("constructor"));
        }
        if !self.is_constructor(new_target.as_object())? {
            return Ok(Completion::Throw(self.new_not_constructor_error(
                caller_realm,
                &Value::Object(new_target.as_object().clone()),
            )?));
        }
        let constructor = ConstructorRef::from_validated_callable(constructor);
        let new_target = ConstructorRef::from_validated_callable(new_target);
        self.construct_constructor_internal(caller_realm, &constructor, &new_target, arguments)
    }

    pub(crate) fn construct_constructor_internal(
        &self,
        caller_realm: ContextId,
        constructor: &ConstructorRef,
        new_target: &ConstructorRef,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        self.construct_internal_with_new_target(
            caller_realm,
            constructor,
            ConstructNewTarget::Validated(new_target.clone()),
            arguments,
        )
    }

    pub(crate) fn construct_internal_with_new_target(
        &self,
        caller_realm: ContextId,
        constructor: &ConstructorRef,
        new_target: ConstructNewTarget,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        self.0.state.borrow().heap.context(caller_realm)?;
        if !constructor.as_object().belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("constructor"));
        }
        match &new_target {
            ConstructNewTarget::Validated(new_target) => {
                if !new_target.as_object().belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("constructor"));
                }
            }
            ConstructNewTarget::Raw(new_target) => {
                self.validate_value_domain(new_target, "raw construct new target")?;
            }
        }
        for argument in arguments {
            self.validate_value_domain(argument, "construct argument")?;
        }
        let mut constructor = constructor.clone();
        let mut new_target = new_target;
        let mut arguments = arguments.to_vec();
        loop {
            if !self.is_constructor(constructor.as_object())? {
                return Ok(Completion::Throw(self.new_not_constructor_error(
                    caller_realm,
                    &Value::Object(constructor.as_object().clone()),
                )?));
            }
            if self.is_proxy_object(constructor.as_object())? {
                return self.construct_proxy(caller_realm, &constructor, new_target, &arguments);
            }
            let callable = self.as_callable(constructor.as_object())?.ok_or_else(|| {
                RuntimeError::Engine(Error::new(ErrorKind::Type, "not a function"))
            })?;

            match self.bytecode_for_callable(&callable)? {
                CallableExecution::Bound {
                    target,
                    this_value: _,
                    arguments: bound_arguments,
                } => {
                    arguments = match self.concatenate_bound_arguments(
                        caller_realm,
                        &bound_arguments,
                        &arguments,
                    )? {
                        NativeConversion::Value(arguments) => arguments,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    new_target.retarget_bound_identity(&constructor, &target);
                    constructor = ConstructorRef::from_validated_callable(&target);
                }
                CallableExecution::Native {
                    target,
                    realm,
                    min_readable_args,
                } => {
                    let execution_realm = if target.uses_calling_realm() {
                        caller_realm
                    } else {
                        realm
                    };
                    return self.construct_native_function(
                        &callable,
                        execution_realm,
                        target,
                        min_readable_args,
                        new_target.value(),
                        &arguments,
                    );
                }
                CallableExecution::Bytecode {
                    bytecode,
                    closure_slots,
                } => {
                    let constructor_kind = self
                        .0
                        .state
                        .borrow()
                        .heap
                        .function_bytecode(bytecode.bytecode_id())?
                        .metadata
                        .constructor_kind;
                    match constructor_kind {
                        ConstructorKind::None => {
                            return Err(RuntimeError::Invariant(
                                "constructor bit disagrees with bytecode constructor metadata",
                            ));
                        }
                        ConstructorKind::Derived => {
                            let completion = self.execute_bytecode_callable(
                                caller_realm,
                                &callable,
                                Value::Undefined,
                                new_target.value(),
                                &arguments,
                                bytecode,
                                closure_slots,
                            )?;
                            return match completion {
                                Completion::Return(value @ Value::Object(_)) => {
                                    Ok(Completion::Return(value))
                                }
                                Completion::Throw(value) => Ok(Completion::Throw(value)),
                                Completion::Return(_) => Err(RuntimeError::Invariant(
                                    "derived constructor bytecode returned an unvalidated primitive",
                                )),
                            };
                        }
                        ConstructorKind::Base => {}
                    }
                    let raw_new_target = new_target.value();
                    let this_value =
                        match self.create_from_constructor_value(caller_realm, &raw_new_target)? {
                            Completion::Return(value) => value,
                            Completion::Throw(value) => return Ok(Completion::Throw(value)),
                        };
                    let completion = self.execute_bytecode_callable(
                        caller_realm,
                        &callable,
                        this_value.clone(),
                        raw_new_target,
                        &arguments,
                        bytecode,
                        closure_slots,
                    )?;
                    return Ok(match completion {
                        Completion::Return(value @ Value::Object(_)) => Completion::Return(value),
                        Completion::Throw(value) => Completion::Throw(value),
                        Completion::Return(_) => Completion::Return(this_value),
                    });
                }
                CallableExecution::Proxy => {
                    return Err(RuntimeError::Invariant(
                        "Proxy constructor bypassed constructor-only dispatch",
                    ));
                }
            }
        }
    }

    pub(crate) fn constructor_prototype_source(
        &self,
        caller_realm: ContextId,
        new_target: &Value,
    ) -> Result<NativeConversion<ConstructorPrototypeSource>, RuntimeError> {
        if matches!(new_target, Value::Undefined) {
            return Ok(NativeConversion::Value(ConstructorPrototypeSource::Realm(
                caller_realm,
            )));
        }
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype = match self.get_value_property_in_realm(
            caller_realm,
            new_target.clone(),
            &prototype_key,
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if let Value::Object(prototype) = prototype {
            return Ok(NativeConversion::Value(
                ConstructorPrototypeSource::Explicit(prototype),
            ));
        }
        self.function_realm_from_value(caller_realm, new_target)
            .map(|result| match result {
                NativeConversion::Value(realm) => {
                    NativeConversion::Value(ConstructorPrototypeSource::Realm(realm))
                }
                NativeConversion::Throw(value) => NativeConversion::Throw(value),
            })
    }

    pub(crate) fn create_from_constructor_value(
        &self,
        caller_realm: ContextId,
        new_target: &Value,
    ) -> Result<Completion, RuntimeError> {
        let prototype =
            match self.prototype_from_constructor_value(caller_realm, new_target, |realm| {
                let object_prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
                Ok(ObjectRef::from_borrowed_handle(
                    self.clone(),
                    object_prototype,
                )?)
            })? {
                NativeConversion::Value(prototype) => prototype,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        Ok(Completion::Return(Value::Object(
            self.new_object(Some(&prototype))?,
        )))
    }

    pub(crate) fn prototype_from_constructor_value(
        &self,
        caller_realm: ContextId,
        new_target: &Value,
        fallback: impl FnOnce(ContextId) -> Result<ObjectRef, RuntimeError>,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        match self.constructor_prototype_source(caller_realm, new_target)? {
            NativeConversion::Value(ConstructorPrototypeSource::Explicit(prototype)) => {
                Ok(NativeConversion::Value(prototype))
            }
            NativeConversion::Value(ConstructorPrototypeSource::Realm(realm)) => {
                fallback(realm).map(NativeConversion::Value)
            }
            NativeConversion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn construct_native_function(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        new_target: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let outcome = self.invoke_native_function(
            callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Construct { new_target },
            arguments,
            NativeInvokeMode::Ordinary,
        )?;
        Self::ordinary_native_completion(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_native_function(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let outcome = self.invoke_native_function(
            callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Call { this_value },
            arguments,
            NativeInvokeMode::Ordinary,
        )?;
        Self::ordinary_native_completion(outcome)
    }

    pub(crate) fn ordinary_native_completion(
        outcome: NativeInvokeOutcome,
    ) -> Result<Completion, RuntimeError> {
        match outcome {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { .. } => Err(RuntimeError::Invariant(
                "ordinary native call leaked an unwrapped iterator-next outcome",
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn invoke_native_function(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        invocation: NativeInvocation,
        arguments: &[Value],
        mode: NativeInvokeMode,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("native callable"));
        }
        self.0.state.borrow().heap.context(realm)?;

        // The callable root held by the caller owns the native payload and its
        // defining-realm edge for the whole invocation. Revalidate the
        // detached snapshot before recording raw identities in the frame.
        // Class-call and CFunctionData-style internal functions deliberately
        // execute in `realm`, which is the calling realm rather than the
        // separately retained defining realm.
        {
            let state = self.0.state.borrow();
            let object = state.heap.object(callable.as_object().object_id())?;
            let ObjectPayload::NativeFunction { data, .. } = &object.payload else {
                return Err(RuntimeError::Invariant(
                    "native invocation target was not a native function",
                ));
            };
            let defining_realm = data.realm.ok_or(RuntimeError::Invariant(
                "native function lost its defining realm",
            ))?;
            if data.target != target
                || (matches!(mode, NativeInvokeMode::Ordinary)
                    && !target.uses_calling_realm()
                    && defining_realm != realm)
                || data.min_readable_args != min_readable_args
            {
                return Err(RuntimeError::Invariant(
                    "native invocation metadata changed after snapshot",
                ));
            }
            state.heap.context(defining_realm)?;
        }

        let actual_arg_count = arguments.len();
        let available_arg_count = actual_arg_count.max(usize::from(min_readable_args));
        let mut readable = Vec::with_capacity(available_arg_count);
        readable.extend_from_slice(arguments);
        readable.resize(available_arg_count, Value::Undefined);
        let arguments = NativeArguments {
            actual_arg_count,
            readable,
        };
        let active_frame = match mode {
            NativeInvokeMode::Ordinary => self.push_native_active_frame(
                callable.as_object().clone(),
                realm,
                target,
                actual_arg_count,
                available_arg_count,
            )?,
            NativeInvokeMode::IteratorNextRaw => self.push_native_iterator_next_active_frame(
                callable.as_object().clone(),
                realm,
                target,
                actual_arg_count,
                available_arg_count,
            )?,
        };

        // JavaScript-style engine errors are materialized in the selected
        // execution realm while its frame is still visible. A pre-existing
        // Error returned as an ordinary Throw completion is not captured here:
        // QuickJS pops the C frame first and lets the enclosing bytecode
        // exception boundary add any missing stack.
        let result = (|| {
            let result = match mode {
                NativeInvokeMode::Ordinary => self
                    .dispatch_native_function(callable, target, realm, invocation, &arguments)
                    .map(NativeInvokeOutcome::Completion),
                NativeInvokeMode::IteratorNextRaw => {
                    if target.descriptor().cproto != NativeCProto::IteratorNext {
                        return Err(RuntimeError::Invariant(
                            "raw iterator-next dispatch targeted another native cproto",
                        ));
                    }
                    self.dispatch_native_iterator_next_raw(target, realm, invocation, &arguments)
                }
            };
            match result {
                Err(RuntimeError::Engine(error))
                    if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
                {
                    let kind = NativeErrorKind::from_javascript_error(error.kind())
                        .expect("guard proved this is a JavaScript-visible native error");
                    let value = self.new_native_error_from_error(realm, kind, &error)?;
                    Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)))
                }
                result => result,
            }
        })();
        active_frame.finish()?;
        result
    }

    pub(crate) fn active_function(&self) -> Result<ObjectRef, RuntimeError> {
        let function = self
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .ok_or(RuntimeError::Invariant(
                "active function was requested without an active frame",
            ))?
            .function;
        Ok(ObjectRef::from_borrowed_handle(self.clone(), function)?)
    }
}

pub(crate) enum NativeInvocation {
    Call { this_value: Value },
    Construct { new_target: Value },
    Getter { this_value: Value },
    Setter { this_value: Value },
}

pub(crate) enum NativeInvocationAdaptation {
    Invoke(NativeInvocation),
    Complete(Completion),
}

/// Result of invoking one native function before the public call adapter has
/// necessarily materialized its JavaScript result shape.
///
/// QuickJS's `JS_CFUNC_iterator_next` ABI returns the iterated value and a
/// side-channel `done` bit.  An ordinary JavaScript call wraps that pair in an
/// iterator-result object, while `JS_IteratorNext2` consumes it directly.  A
/// normal completion remains available for future iterator-next natives which
/// return an already-materialized result object (`pdone == 2`).
pub(crate) enum NativeInvokeOutcome {
    Completion(Completion),
    IteratorNextRaw { value: Value, done: bool },
}

#[derive(Clone, Copy)]
pub(crate) enum NativeInvokeMode {
    Ordinary,
    IteratorNextRaw,
}

pub(crate) struct NativeArguments {
    pub(crate) actual_arg_count: usize,
    pub(crate) readable: Vec<Value>,
}

/// Result of QuickJS `Get(newTarget, "prototype")` followed by
/// `JS_GetFunctionRealm` when the property is not an object.
///
/// Keeping the fallback realm separate lets each native constructor select
/// its own intrinsic prototype without first narrowing the raw `newTarget` to
/// a callable object.
pub(crate) enum ConstructorPrototypeSource {
    Explicit(ObjectRef),
    Realm(ContextId),
}

pub(crate) enum CallableExecution {
    Bytecode {
        bytecode: FunctionBytecodeRef,
        closure_slots: Vec<VarRefRoot>,
    },
    Native {
        target: NativeFunctionId,
        realm: ContextId,
        min_readable_args: u8,
    },
    Bound {
        target: CallableRef,
        this_value: Value,
        arguments: Vec<Value>,
    },
    Proxy,
}

/// A rooted object whose `[[Construct]]` capability bit has been validated.
///
/// QuickJS keeps `[[Call]]` and `[[Construct]]` independent: in particular a
/// Proxy may carry only the latter and still dispatch its `construct` trap.
/// Keep that capability private instead of weakening public `CallableRef`.
#[derive(Clone)]
pub(crate) struct ConstructorRef(ObjectRef);

impl ConstructorRef {
    pub(crate) fn from_validated_object(object: ObjectRef) -> Self {
        Self(object)
    }

    pub(crate) fn from_validated_callable(callable: &CallableRef) -> Self {
        Self(callable.as_object().clone())
    }

    pub(crate) fn as_object(&self) -> &ObjectRef {
        &self.0
    }
}

/// Whether one internal constructor entry carries an ECMAScript-validated
/// `newTarget` or QuickJS's raw `JS_CallConstructor2` value.
///
/// `OP_call_constructor`, `OP_apply` constructor mode, and derived `super()`
/// use the raw form. Public Context and Reflect entry points retain the
/// validated form and its existing constructor checks.
#[derive(Clone)]
pub(crate) enum ConstructNewTarget {
    Validated(ConstructorRef),
    Raw(Value),
}

impl ConstructNewTarget {
    pub(crate) fn value(&self) -> Value {
        match self {
            Self::Validated(constructor) => Value::Object(constructor.as_object().clone()),
            Self::Raw(value) => value.clone(),
        }
    }

    pub(crate) fn retarget_bound_identity(&mut self, bound: &ConstructorRef, target: &CallableRef) {
        let matches_bound = match self {
            Self::Validated(constructor) => constructor.as_object() == bound.as_object(),
            Self::Raw(Value::Object(object)) => object == bound.as_object(),
            Self::Raw(_) => false,
        };
        if !matches_bound {
            return;
        }
        match self {
            Self::Validated(constructor) => {
                *constructor = ConstructorRef::from_validated_callable(target);
            }
            Self::Raw(value) => *value = Value::Object(target.as_object().clone()),
        }
    }
}

/// Target selected by the bytecode `Call` path.
///
/// Pinned QuickJS deliberately enters a Proxy's call hook before checking the
/// cached callable bit, so a direct call of a non-callable Proxy can still
/// observe the handler's `apply` getter. Keep that narrow quirk out of
/// `CallableRef`, whose public invariant remains a genuine `[[Call]]`.
pub(crate) enum DirectCallTarget {
    Callable(CallableRef),
    NonCallableProxy(ObjectRef),
}
