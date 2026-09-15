//! Error-family constructors and prototype intrinsics.

pub(crate) mod aggregate;
mod backtrace;
mod construction;
pub(crate) mod operation;

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{ErrorConstructorKind, NativeFunctionId};
use crate::engine::heap::ContextId;

use crate::engine::object::ObjectRef;
use crate::engine::value::Value;
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};

impl Runtime {
    pub(crate) fn initialize_error_intrinsics(
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

    pub(crate) fn call_error_constructor(
        &self,
        realm: ContextId,
        kind: ErrorConstructorKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::finish(
            self,
            realm,
            operation::ErrorStep::start(
                self,
                realm,
                operation::ErrorKind::Constructor(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    /// Pinned QuickJS's internal `Promise.any` AggregateError path: retain the
    /// exact errors Array without invoking the public constructor or backtrace.
    pub(crate) fn new_internal_aggregate_error(
        &self,
        realm: ContextId,
        errors: ObjectRef,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = {
            let state = self.0.state.borrow();
            state.heap.context(realm)?.native_error_prototypes[NativeErrorKind::Aggregate.index()]
                .ok_or(RuntimeError::Invariant(
                "realm has no AggregateError prototype",
            ))?
        };
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let object = self.new_error_object(&prototype)?;
        self.define_function_data_property(&object, "errors", Value::Object(errors), true, true)?;
        Ok(object)
    }

    /// Pinned QuickJS `iterator_to_array`, used only by AggregateError.
    /// Iterator-step and indexed-definition failures close an acquired
    /// iterator while preserving the original abrupt completion.

    pub(crate) fn call_error_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let arguments = NativeArguments {
            readable: Vec::new(),
            actual_arg_count: 0,
        };
        operation::finish(
            self,
            realm,
            operation::ErrorStep::start(
                self,
                realm,
                operation::ErrorKind::ToString,
                &invocation,
                &arguments,
            )?,
        )
    }

    pub(crate) fn call_error_is_error(
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
