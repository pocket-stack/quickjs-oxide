use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{
    DynamicFunctionKind, FunctionDebugPosition, NativeFunctionId,
};
use crate::engine::code::dynamic_source::DynamicSourceBuilder;
use crate::engine::code::function::metadata::FunctionKind;

use crate::engine::heap::{ContextId, ObjectPayload};
use crate::engine::object::{CallableRef, ObjectRef, PropertyKey, WellKnownSymbol};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};

impl Runtime {
    pub(crate) fn call_throw_type_error(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "%ThrowTypeError% did not receive a generic invocation",
            ));
        };

        // QuickJS keeps the ES5-compatible sloppy ordinary-function getter
        // exception: reading `.caller`/`.arguments` returns undefined when
        // the receiver has normal-function bytecode, is non-strict, has a
        // prototype and the shared poison function was invoked without a
        // setter argument. Oxide separately uses `has_prototype` for generator
        // callables' own prototype object, so the bytecode kind must remain an
        // explicit part of QuickJS's `b->has_prototype` test.
        let sloppy_legacy_get = if arguments.actual_arg_count == 0 {
            match this_value {
                Value::Object(object) => {
                    let state = self.0.state.borrow();
                    let object = state.heap.object(object.object_id())?;
                    match object.payload {
                        ObjectPayload::BytecodeFunction { bytecode, .. } => {
                            let metadata = state.heap.function_bytecode(bytecode)?.metadata;
                            metadata.function_kind == FunctionKind::Normal
                                && !metadata.strict
                                && metadata.has_prototype
                        }
                        ObjectPayload::Ordinary
                        | ObjectPayload::ArrayBuffer(_)
                        | ObjectPayload::SharedArrayBuffer(_)
                        | ObjectPayload::DataView(_)
                        | ObjectPayload::TypedArray(_)
                        | ObjectPayload::Proxy(_)
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
                        | ObjectPayload::BoundFunction { .. }
                        | ObjectPayload::NativeFunction { .. }
                        | ObjectPayload::Generator { .. }
                        | ObjectPayload::AsyncGenerator(_) => false,
                    }
                }
                Value::Undefined
                | Value::Null
                | Value::Bool(_)
                | Value::Int(_)
                | Value::Float(_)
                | Value::String(_)
                | Value::BigInt(_)
                | Value::Symbol(_) => false,
            }
        } else {
            false
        };
        if sloppy_legacy_get {
            return Ok(Completion::Return(Value::Undefined));
        }
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            "invalid property access",
        )?))
    }

    pub(crate) fn call_function_constructor(
        &self,
        realm: ContextId,
        kind: DynamicFunctionKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function constructor did not receive constructor-or-function invocation",
            ));
        };

        // Match js_function_constructor byte-for-byte: all parameter strings
        // precede the final body string, argc == 0 reads no padded argument,
        // and the complete wrapper is parsed as one indirect-eval unit so a
        // body strict directive can retroactively validate the parameters.
        let mut source = DynamicSourceBuilder::new();
        source.push_str("(")?;
        match kind {
            DynamicFunctionKind::Normal | DynamicFunctionKind::Generator => {}
            DynamicFunctionKind::Async | DynamicFunctionKind::AsyncGenerator => {
                source.push_str("async ")?;
            }
        }
        source.push_str("function")?;
        if matches!(
            kind,
            DynamicFunctionKind::Generator | DynamicFunctionKind::AsyncGenerator
        ) {
            source.push_str("*")?;
        }
        source.push_str(" anonymous(")?;

        let parameter_count = arguments.actual_arg_count.saturating_sub(1);
        for index in 0..parameter_count {
            if index != 0 {
                source.push_str(",")?;
            }
            let parameter =
                match self.native_to_dynamic_source_fragment(realm, &arguments.readable[index])? {
                    NativeConversion::Value(parameter) => parameter,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            source.push_js_string(&parameter)?;
        }
        source.push_str("\n) {\n")?;
        if arguments.actual_arg_count != 0 {
            let body_index = arguments.actual_arg_count - 1;
            let body = match self
                .native_to_dynamic_source_fragment(realm, &arguments.readable[body_index])?
            {
                NativeConversion::Value(body) => body,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            source.push_js_string(&body)?;
        }
        source.push_str("\n})")?;
        let source = source.finish()?;

        let value = match self.execute_indirect_string_eval(realm, &source)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        if matches!(new_target, Value::Undefined) {
            return Ok(Completion::Return(value));
        }
        let prototype =
            match self.prototype_from_constructor_value(realm, &new_target, |fallback_realm| {
                let prototype = match kind {
                    DynamicFunctionKind::Normal => {
                        self.0
                            .state
                            .borrow()
                            .heap
                            .context(fallback_realm)?
                            .function_prototype
                    }
                    DynamicFunctionKind::Generator => {
                        self.0
                            .state
                            .borrow()
                            .heap
                            .context(fallback_realm)?
                            .generator
                            .ok_or(RuntimeError::Invariant(
                                "dynamic GeneratorFunction realm has no Generator intrinsics",
                            ))?
                            .function_prototype
                    }
                    DynamicFunctionKind::Async => {
                        self.0
                            .state
                            .borrow()
                            .heap
                            .context(fallback_realm)?
                            .async_function
                            .ok_or(RuntimeError::Invariant(
                                "dynamic AsyncFunction realm has no AsyncFunction intrinsics",
                            ))?
                            .function_prototype
                    }
                    DynamicFunctionKind::AsyncGenerator => self
                        .0
                        .state
                        .borrow()
                        .heap
                        .context(fallback_realm)?
                        .async_generator
                        .ok_or(RuntimeError::Invariant(
                            "dynamic AsyncGeneratorFunction realm has no AsyncGenerator intrinsics",
                        ))?
                        .function_prototype,
                };
                Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
            })? {
                NativeConversion::Value(prototype) => prototype,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let Value::Object(function) = value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        if !self.set_prototype_of(&function, Some(&prototype))? {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "prototype is immutable",
            )?));
        }
        Ok(Completion::Return(Value::Object(function)))
    }

    pub(crate) fn call_function_prototype_apply(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.apply did not receive a generic invocation",
            ));
        };
        let Value::Object(target) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(target) = self.as_callable(&target)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let this_argument = arguments.readable[0].clone();
        let array_argument = &arguments.readable[1];
        if matches!(array_argument, Value::Undefined | Value::Null) {
            return self.call_internal(realm, &target, this_argument, &[]);
        }
        let forwarded = match self.build_array_like_argument_list(realm, array_argument)? {
            NativeConversion::Value(values) => values,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.call_internal(realm, &target, this_argument, &forwarded)
    }

    pub(crate) fn call_function_prototype_bind(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.bind did not receive a generic invocation",
            ));
        };
        let Value::Object(target_object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(target) = self.as_callable(&target_object)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let bound_argument_count = arguments.actual_arg_count.saturating_sub(1);
        let bound_arguments = if arguments.actual_arg_count > 1 {
            &arguments.readable[1..arguments.actual_arg_count]
        } else {
            &[]
        };
        let bound =
            self.new_bound_function(realm, &target, &arguments.readable[0], bound_arguments)?;

        let length_key = self.intern_property_key("length")?;
        let has_own_length =
            match self.internal_has_own_property(realm, &target_object, &length_key)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let length = if has_own_length {
            let value = match self.get_property_in_realm(realm, &target_object, &length_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            bound_function_length(&value, bound_argument_count)?
        } else {
            Value::Int(0)
        };
        self.define_function_data_property(bound.as_object(), "length", length, false, true)?;

        let name_key = self.intern_property_key("name")?;
        let name = match self.get_property_in_realm(realm, &target_object, &name_key)? {
            Completion::Return(Value::String(name)) => name,
            Completion::Return(_) => JsString::from_static(""),
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let name = JsString::from_static("bound ").try_concat(&name)?;
        self.define_function_data_property(
            bound.as_object(),
            "name",
            Value::String(name),
            false,
            true,
        )?;
        Ok(Completion::Return(Value::Object(bound.into_object())))
    }

    pub(crate) fn call_function_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.toString did not receive a generic invocation",
            ));
        };
        let Value::Object(function) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let (is_callable, source, function_kind) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(function.object_id())?;
            match &object.payload {
                ObjectPayload::BytecodeFunction { bytecode, .. } => {
                    let bytecode = state.heap.function_bytecode(*bytecode)?;
                    (
                        true,
                        bytecode
                            .debug
                            .as_ref()
                            .and_then(|debug| debug.source.clone()),
                        bytecode.metadata.function_kind,
                    )
                }
                ObjectPayload::NativeFunction { .. } | ObjectPayload::BoundFunction { .. } => {
                    (true, None, FunctionKind::Normal)
                }
                ObjectPayload::Proxy(data) => (data.is_callable, None, FunctionKind::Normal),
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
                | ObjectPayload::AsyncGenerator(_) => (false, None, FunctionKind::Normal),
            }
        };
        if !is_callable {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        }
        if let Some(source) = source {
            return Ok(Completion::Return(Value::String(JsString::try_from_bytes(
                &source,
            )?)));
        }

        let name_key = self.intern_property_key("name")?;
        let name = match self.get_property_in_realm(realm, &function, &name_key)? {
            Completion::Return(Value::Undefined) => JsString::from_static(""),
            Completion::Return(value) => match self.native_to_js_string(realm, &value)? {
                NativeConversion::Value(name) => name,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let prefix = match function_kind {
            FunctionKind::Normal => "function ",
            FunctionKind::Generator => "function *",
            FunctionKind::Async => "async function ",
            FunctionKind::AsyncGenerator => "async function *",
        };
        let source = JsString::from_static(prefix)
            .try_concat(&name)?
            .try_concat(&JsString::from_static("() {\n    [native code]\n}"))?;
        Ok(Completion::Return(Value::String(source)))
    }

    pub(crate) fn call_function_prototype_file_name(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.fileName getter received the wrong native invocation",
            ));
        };
        let Value::Object(function) = this_value else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let filename = {
            let state = self.0.state.borrow();
            let object = state.heap.object(function.object_id())?;
            let ObjectPayload::BytecodeFunction { bytecode, .. } = &object.payload else {
                return Ok(Completion::Return(Value::Undefined));
            };
            let bytecode = state.heap.function_bytecode(*bytecode)?;
            bytecode
                .debug
                .as_ref()
                .map(|debug| state.atoms.to_js_string(debug.filename))
                .transpose()?
        };
        Ok(Completion::Return(
            filename.map_or(Value::Undefined, Value::String),
        ))
    }

    pub(crate) fn call_function_prototype_position(
        &self,
        invocation: NativeInvocation,
        selector: FunctionDebugPosition,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype position getter received the wrong native invocation",
            ));
        };
        let Value::Object(function) = this_value else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let position = {
            let state = self.0.state.borrow();
            let object = state.heap.object(function.object_id())?;
            let ObjectPayload::BytecodeFunction { bytecode, .. } = &object.payload else {
                return Ok(Completion::Return(Value::Undefined));
            };
            let bytecode = state.heap.function_bytecode(*bytecode)?;
            bytecode
                .debug
                .as_ref()
                .map(|debug| debug.pc2line.as_ref().map(|table| table.lookup(None)))
        };
        let Some(position) = position else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let Some(position) = position else {
            return Ok(Completion::Return(Value::Int(0)));
        };
        let (line, column) = position.one_based().ok_or(RuntimeError::Invariant(
            "function definition position cannot be represented one-based",
        ))?;
        let selected = match selector {
            FunctionDebugPosition::Line => line,
            FunctionDebugPosition::Column => column,
        };
        let selected = i32::try_from(selected).map_err(|_| {
            RuntimeError::Invariant("function definition position does not fit Int32")
        })?;
        Ok(Completion::Return(Value::Int(selected)))
    }

    /// QuickJS `JS_IsInstanceOf`: observe `@@hasInstance` before the legacy
    /// callable fallback, call a custom method with the RHS as receiver, and
    /// preserve arbitrary thrown values as completions.
    pub(crate) fn is_instance_of(
        &self,
        realm: ContextId,
        candidate: Value,
        target: ObjectRef,
    ) -> Result<Completion, RuntimeError> {
        let has_instance = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::HasInstance));
        let method = match self.get_property_in_realm(realm, &target, &has_instance)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !matches!(method, Value::Undefined | Value::Null) {
            let method = self.callable_from_value(method)?;
            return Ok(
                match self.call_internal(
                    realm,
                    &method,
                    Value::Object(target),
                    std::slice::from_ref(&candidate),
                )? {
                    Completion::Return(value) => {
                        Completion::Return(Value::Bool(self.value_to_boolean(&value)?))
                    }
                    Completion::Throw(value) => Completion::Throw(value),
                },
            );
        }

        let Some(target) = self.as_callable(&target)? else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "invalid 'instanceof' right operand",
            )));
        };
        self.ordinary_is_instance_of(realm, &target, candidate)
    }

    pub(crate) fn call_function_prototype_has_instance(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype[Symbol.hasInstance] did not receive a generic invocation",
            ));
        };
        let Value::Object(target) = this_value else {
            return Ok(Completion::Return(Value::Bool(false)));
        };
        let Some(_target_callable) = self.as_callable(&target)? else {
            return Ok(Completion::Return(Value::Bool(false)));
        };
        let target = CallableRef::from_validated_object(target);
        self.ordinary_is_instance_of(
            realm,
            &target,
            arguments
                .readable
                .first()
                .cloned()
                .unwrap_or(Value::Undefined),
        )
    }

    pub(crate) fn ordinary_is_instance_of(
        &self,
        mut realm: ContextId,
        target: &CallableRef,
        candidate: Value,
    ) -> Result<Completion, RuntimeError> {
        let mut target = target.clone();
        let mut delegated_standard_frames = Vec::new();
        let has_instance = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::HasInstance));
        let result = (|| -> Result<Completion, RuntimeError> {
            loop {
                let bound_target = {
                    let state = self.0.state.borrow();
                    let object = state.heap.object(target.as_object().object_id())?;
                    match &object.payload {
                        ObjectPayload::BoundFunction { target, .. } => Some(*target),
                        ObjectPayload::NativeFunction { .. }
                        | ObjectPayload::BytecodeFunction { .. }
                        | ObjectPayload::Proxy(_) => None,
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
                            return Err(RuntimeError::Invariant(
                                "ordinary instanceof received a non-callable target",
                            ));
                        }
                    }
                };

                let Some(bound_target) = bound_target else {
                    let Value::Object(candidate) = &candidate else {
                        return Ok(Completion::Return(Value::Bool(false)));
                    };
                    let prototype_key = self.intern_property_key("prototype")?;
                    let prototype = match self.get_property_in_realm(
                        realm,
                        target.as_object(),
                        &prototype_key,
                    )? {
                        Completion::Return(Value::Object(prototype)) => prototype,
                        Completion::Return(_) => {
                            return Ok(Completion::Throw(self.new_native_error(
                                realm,
                                NativeErrorKind::Type,
                                "operand 'prototype' property is not an object",
                            )?));
                        }
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };

                    let mut cursor = match self.internal_get_prototype_of(realm, candidate)? {
                        NativeConversion::Value(prototype) => prototype,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    while let Some(current) = cursor {
                        if current == prototype {
                            return Ok(Completion::Return(Value::Bool(true)));
                        }
                        cursor = match self.internal_get_prototype_of(realm, &current)? {
                            NativeConversion::Value(prototype) => prototype,
                            NativeConversion::Throw(value) => {
                                return Ok(Completion::Throw(value));
                            }
                        };
                    }
                    return Ok(Completion::Return(Value::Bool(false)));
                };

                // QuickJS bound OrdinaryHasInstance delegates through the full
                // JS_IsInstanceOf path. Perform every observable GetMethod in
                // order, but trampoline direct calls to the inherited standard
                // method so a deep bound chain cannot recurse through Rust's
                // host stack. Synthetic native frames preserve backtraces.
                let target_object = ObjectRef::from_borrowed_handle(self.clone(), bound_target)?;
                let method =
                    match self.get_property_in_realm(realm, &target_object, &has_instance)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                if matches!(method, Value::Undefined | Value::Null) {
                    let Some(next_target) = self.as_callable(&target_object)? else {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "invalid 'instanceof' right operand",
                        )?));
                    };
                    target = next_target;
                    continue;
                }

                let Value::Object(method_object) = method else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                };
                let Some(method) = self.as_callable(&method_object)? else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                };
                let standard_method = {
                    let state = self.0.state.borrow();
                    let object = state.heap.object(method.as_object().object_id())?;
                    match &object.payload {
                        ObjectPayload::NativeFunction { data, .. }
                            if data.target == NativeFunctionId::FunctionPrototypeHasInstance =>
                        {
                            Some((
                                data.realm.ok_or(RuntimeError::Invariant(
                                    "standard hasInstance method has no defining realm",
                                ))?,
                                data.min_readable_args,
                            ))
                        }
                        ObjectPayload::Ordinary
                        | ObjectPayload::ArrayBuffer(_)
                        | ObjectPayload::SharedArrayBuffer(_)
                        | ObjectPayload::DataView(_)
                        | ObjectPayload::TypedArray(_)
                        | ObjectPayload::Proxy(_)
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
                        | ObjectPayload::NativeFunction { .. }
                        | ObjectPayload::BoundFunction { .. }
                        | ObjectPayload::BytecodeFunction { .. }
                        | ObjectPayload::Generator { .. }
                        | ObjectPayload::AsyncGenerator(_) => None,
                    }
                };
                if let Some((method_realm, min_readable_args)) = standard_method {
                    let readable_arg_count = 1usize.max(usize::from(min_readable_args));
                    delegated_standard_frames.push(self.push_native_active_frame(
                        method.as_object().clone(),
                        method_realm,
                        NativeFunctionId::FunctionPrototypeHasInstance,
                        1,
                        readable_arg_count,
                    )?);
                    let Some(next_target) = self.as_callable(&target_object)? else {
                        return Ok(Completion::Return(Value::Bool(false)));
                    };
                    realm = method_realm;
                    target = next_target;
                    continue;
                }

                return Ok(
                    match self.call_internal(
                        realm,
                        &method,
                        Value::Object(target_object),
                        std::slice::from_ref(&candidate),
                    )? {
                        Completion::Return(value) => {
                            Completion::Return(Value::Bool(self.value_to_boolean(&value)?))
                        }
                        Completion::Throw(value) => Completion::Throw(value),
                    },
                );
            }
        })();

        let mut frame_error = None;
        while let Some(frame) = delegated_standard_frames.pop() {
            if let Err(error) = frame.finish() {
                if frame_error.is_none() {
                    frame_error = Some(error);
                }
            }
        }
        if let Some(error) = frame_error {
            return Err(error);
        }
        result
    }
}

pub(crate) fn bound_function_length(
    value: &Value,
    bound_argument_count: usize,
) -> Result<Value, RuntimeError> {
    let count = u32::try_from(bound_argument_count)
        .map_err(|_| RuntimeError::Invariant("bound argument count does not fit u32"))?;
    Ok(match value {
        Value::Int(length) => {
            let length = i64::from(*length);
            let count = i64::from(count);
            if length <= count {
                Value::Int(0)
            } else {
                Value::Int(i32::try_from(length - count).map_err(|_| {
                    RuntimeError::Invariant("bound function integer length does not fit i32")
                })?)
            }
        }
        Value::Float(length) => {
            let length = if length.is_nan() {
                0.0
            } else {
                let length = length.trunc();
                if length <= f64::from(count) {
                    0.0
                } else {
                    length - f64::from(count)
                }
            };
            Value::number(length)
        }
        Value::Undefined
        | Value::Null
        | Value::Bool(_)
        | Value::BigInt(_)
        | Value::String(_)
        | Value::Symbol(_)
        | Value::Object(_) => Value::Int(0),
    })
}
