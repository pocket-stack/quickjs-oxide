use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{DynamicFunctionKind, FunctionDebugPosition};
use crate::engine::code::function::metadata::FunctionKind;

use crate::engine::heap::{ContextId, ObjectPayload};
use crate::engine::object::{CallableRef, ObjectRef};
use crate::engine::value::Value;
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};

pub(super) mod arguments;
pub(super) mod bind;
pub(crate) mod dynamic;
pub(super) mod instance;
pub(super) mod invoke;
pub(crate) mod text;

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
        dynamic::finish(
            self,
            realm,
            dynamic::DynamicFunctionStep::start(self, realm, kind, &invocation, arguments)?,
        )
    }

    pub(crate) fn call_function_prototype_apply(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        super::function::invoke::finish(
            self,
            realm,
            super::function::invoke::InvokeStep::start(
                self,
                realm,
                super::function::invoke::InvokeKind::Apply,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_function_prototype_bind(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        bind::finish(
            self,
            realm,
            bind::BindStep::start(self, realm, &invocation, arguments)?,
        )
    }

    pub(crate) fn call_function_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        text::finish(
            self,
            realm,
            text::FunctionTextStep::start(self, realm, &invocation)?,
        )
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
        instance::finish(
            self,
            realm,
            instance::InstanceStep::start(self, realm, candidate, target)?,
        )
    }
    pub(crate) fn call_function_prototype_has_instance(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        instance::finish(
            self,
            realm,
            instance::InstanceStep::native(self, realm, &invocation, arguments)?,
        )
    }
    pub(crate) fn ordinary_is_instance_of(
        &self,
        realm: ContextId,
        target: &CallableRef,
        candidate: Value,
    ) -> Result<Completion, RuntimeError> {
        instance::finish(
            self,
            realm,
            instance::InstanceStep::ordinary(self, realm, target, candidate)?,
        )
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
