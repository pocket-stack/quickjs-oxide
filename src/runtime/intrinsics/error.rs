//! Error-family constructors and prototype intrinsics.

use super::super::*;
use super::object::ObjectIteratorStep;

impl Runtime {
    pub(in crate::runtime) fn initialize_error_intrinsics(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        error_prototype: &ObjectRef,
        native_error_prototypes: &[ObjectRef],
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        if native_error_prototypes.len() != NativeErrorKind::COUNT {
            return Err(RuntimeError::Invariant(
                "native Error prototype count did not match NativeErrorKind",
            ));
        }

        // JS_NewCConstructor installs prototype fields before the constructor
        // back-reference. Preserve that observable own-key order.
        self.define_native_builtin_auto_init(
            error_prototype,
            realm,
            NativeFunctionId::ErrorPrototypeToString,
            "toString",
            0,
            0,
        )?;
        self.define_string_auto_init(error_prototype, realm, "name", "Error")?;
        self.define_string_auto_init(error_prototype, realm, "message", "")?;

        for prototype in native_error_prototypes {
            self.define_string_auto_init(prototype, realm, "message", "")?;
        }

        let error_constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ErrorConstructor(ErrorConstructorKind::Error),
            1,
            "Error",
            1,
        )?;
        self.define_native_builtin_auto_init(
            error_constructor.as_object(),
            realm,
            NativeFunctionId::ErrorIsError,
            "isError",
            1,
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "Error",
            Value::Object(error_constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&error_constructor, error_prototype)?;

        for kind in NativeErrorKind::ALL {
            let prototype =
                native_error_prototypes
                    .get(kind.index())
                    .ok_or(RuntimeError::Invariant(
                        "native Error prototype index was out of bounds",
                    ))?;
            let is_aggregate = kind == NativeErrorKind::Aggregate;
            let readable_arguments = if is_aggregate { 2 } else { 1 };
            let constructor = self.new_native_builtin(
                error_constructor.as_object(),
                realm,
                NativeFunctionId::ErrorConstructor(ErrorConstructorKind::Native(kind)),
                readable_arguments,
                kind.name(),
                i32::from(readable_arguments),
            )?;
            self.define_function_data_property(
                global_object,
                kind.name(),
                Value::Object(constructor.as_object().clone()),
                true,
                true,
            )?;
            self.define_constructor_relationship(&constructor, prototype)?;
        }
        Ok(())
    }

    pub(in crate::runtime) fn call_error_constructor(
        &self,
        realm: ContextId,
        kind: ErrorConstructorKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { mut new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Error constructor did not receive constructor-or-function invocation",
            ));
        };
        if matches!(new_target, Value::Undefined) {
            new_target = Value::Object(self.active_function()?);
        }
        let Value::Object(new_target_object) = new_target else {
            return Err(RuntimeError::Invariant(
                "Error constructor new.target was neither undefined nor an object",
            ));
        };
        let new_target_callable =
            self.callable_from_value(Value::Object(new_target_object.clone()))?;
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype =
            match self.get_property_in_realm(realm, &new_target_object, &prototype_key)? {
                Completion::Return(Value::Object(prototype)) => prototype,
                Completion::Return(_) => {
                    let fallback_realm = self.callable_realm(&new_target_callable)?;
                    let prototype = {
                        let state = self.0.state.borrow();
                        let context = state.heap.context(fallback_realm)?;
                        match kind {
                            ErrorConstructorKind::Error => context
                                .error_prototype
                                .ok_or(RuntimeError::Invariant("realm has no Error prototype"))?,
                            ErrorConstructorKind::Native(kind) => {
                                context.native_error_prototypes[kind.index()].ok_or(
                                    RuntimeError::Invariant("realm has no native Error prototype"),
                                )?
                            }
                        }
                    };
                    ObjectRef::from_borrowed_handle(self.clone(), prototype)?
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let object = self.new_error_object(&prototype)?;

        let is_aggregate = kind == ErrorConstructorKind::Native(NativeErrorKind::Aggregate);
        let message_index = usize::from(is_aggregate);
        let message = arguments
            .readable
            .get(message_index)
            .ok_or(RuntimeError::Invariant(
                "Error constructor readable argv was not padded to its message index",
            ))?;
        if !matches!(message, Value::Undefined) {
            let message = match self.native_to_js_string(realm, message)? {
                NativeConversion::Value(message) => message,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            self.define_function_data_property(
                &object,
                "message",
                Value::String(message),
                true,
                true,
            )?;
        }

        let options_index = message_index + 1;
        if arguments.actual_arg_count > options_index {
            if let Some(Value::Object(options)) = arguments.readable.get(options_index) {
                let cause_key = self.intern_property_key("cause")?;
                let has_cause = match self.has_property_in_realm(realm, options, &cause_key)? {
                    Completion::Return(Value::Bool(value)) => value,
                    Completion::Return(_) => {
                        return Err(RuntimeError::Invariant(
                            "Error cause HasProperty returned a non-Boolean",
                        ));
                    }
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if has_cause {
                    let cause = match self.get_property_in_realm(realm, options, &cause_key)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    self.define_function_data_property(&object, "cause", cause, true, true)?;
                }
            }
        }

        if is_aggregate {
            let errors = arguments
                .readable
                .first()
                .cloned()
                .ok_or(RuntimeError::Invariant(
                    "AggregateError errors argv was not padded to length two",
                ))?;
            let errors = match self.aggregate_error_iterator_to_array(realm, errors)? {
                Completion::Return(Value::Object(errors)) => errors,
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "AggregateError iterator conversion returned a non-Object",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            self.define_function_data_property(
                &object,
                "errors",
                Value::Object(errors),
                true,
                true,
            )?;
        }

        let value = Value::Object(object);
        // `js_error_constructor` snapshots the stack only after message,
        // cause, and AggregateError's iterable payload have completed.
        self.ensure_error_backtrace(&value, true, None)?;
        Ok(Completion::Return(value))
    }

    /// Pinned QuickJS `iterator_to_array`, used only by AggregateError.
    /// Iterator-step and indexed-definition failures close an acquired
    /// iterator while preserving the original abrupt completion.
    fn aggregate_error_iterator_to_array(
        &self,
        realm: ContextId,
        iterable: Value,
    ) -> Result<Completion, RuntimeError> {
        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        let iterator_method = match &iterable {
            Value::Null | Value::Undefined => {
                let base = if matches!(iterable, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    &format!("cannot read property 'Symbol.iterator' of {base}"),
                )?));
            }
            _ => match self.get_value_property_in_realm(realm, iterable.clone(), &iterator_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            },
        };
        let Value::Object(iterator_method) = iterator_method else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "value is not iterable",
            )?));
        };
        let Some(iterator_method) = self.as_callable(&iterator_method)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "value is not iterable",
            )?));
        };
        let iterator = match self.call_internal(realm, &iterator_method, iterable, &[])? {
            Completion::Return(Value::Object(iterator)) => iterator,
            Completion::Return(_) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let next_key = self.intern_property_key("next")?;
        let next_method = match self.get_property_in_realm(realm, &iterator, &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = self.new_array(realm)?;
        let mut index = 0_u64;
        loop {
            let value = match self.object_iterator_next(realm, &iterator, next_method.clone())? {
                ObjectIteratorStep::Yield(value) => value,
                ObjectIteratorStep::Done => {
                    return Ok(Completion::Return(Value::Object(result)));
                }
                ObjectIteratorStep::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            };
            let key = self.intern_property_key(&index.to_string())?;
            match self.define_own_property_in_realm(
                Some(realm),
                &result,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )? {
                PropertyDefineOutcome::Defined(true) => {}
                PropertyDefineOutcome::Defined(false) => {
                    return Err(RuntimeError::Invariant(
                        "fresh AggregateError Array rejected an indexed property",
                    ));
                }
                PropertyDefineOutcome::Throw(value) => {
                    self.close_iterator_preserving_throw(realm, &iterator)?;
                    return Ok(Completion::Throw(value));
                }
            }
            index = index.checked_add(1).ok_or(RuntimeError::Invariant(
                "AggregateError iterable exceeded Uint64 indices",
            ))?;
        }
    }

    pub(in crate::runtime) fn call_error_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Error.prototype.toString did not receive a generic invocation",
            ));
        };
        let Value::Object(object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let name_key = self.intern_property_key("name")?;
        let name = match self.get_property_in_realm(realm, &object, &name_key)? {
            Completion::Return(Value::Undefined) => JsString::from_static("Error"),
            Completion::Return(value) => match self.native_to_js_string(realm, &value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let message_key = self.intern_property_key("message")?;
        let message = match self.get_property_in_realm(realm, &object, &message_key)? {
            Completion::Return(Value::Undefined) => JsString::from_static(""),
            Completion::Return(value) => match self.native_to_js_string(realm, &value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = if name.is_empty() {
            message
        } else if message.is_empty() {
            name
        } else {
            name.try_concat(&JsString::from_static(": "))?
                .try_concat(&message)?
        };
        Ok(Completion::Return(Value::String(result)))
    }

    pub(in crate::runtime) fn call_error_is_error(
        &self,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Error.isError readable argv was not padded to length one",
        ))?;
        let is_error = match value {
            Value::Object(object) => self.is_error_object(object)?,
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => false,
        };
        Ok(Completion::Return(Value::Bool(is_error)))
    }
}
