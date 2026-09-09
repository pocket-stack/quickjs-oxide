use crate::engine::api::error::{Error, NativeErrorKind, NativeErrorMessage};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::heap::{ContextId, ObjectData, ObjectKind};
use crate::engine::object::{DescriptorField, ObjectRef, OrdinaryPropertyDescriptor};
use crate::engine::value::Value;
use crate::engine::vm::frames::ActiveFrameKind;

impl Runtime {
    pub(crate) fn new_native_error(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        message: &str,
    ) -> Result<Value, RuntimeError> {
        self.new_native_error_from_message(realm, kind, NativeErrorMessage::from_utf8(message))
    }

    pub(crate) fn new_native_error_from_error(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        error: &Error,
    ) -> Result<Value, RuntimeError> {
        let message = error
            .native_message()
            .cloned()
            .unwrap_or_else(|| NativeErrorMessage::from_utf8(error.message()));
        self.new_native_error_from_message(realm, kind, message)
    }

    pub(crate) fn new_native_error_from_message(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        message: NativeErrorMessage,
    ) -> Result<Value, RuntimeError> {
        let value = self.new_native_error_without_backtrace_from_message(realm, kind, message)?;
        let capture_now = self
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .is_none_or(|frame| matches!(frame.kind, ActiveFrameKind::Native { .. }));
        if capture_now {
            self.ensure_error_backtrace(&value, false, None)?;
        }
        Ok(value)
    }

    /// `JS_ThrowError2(..., add_backtrace = FALSE)` construction path used by
    /// parser diagnostics, which prepend their explicit filename location
    /// before adding the active frame chain.
    pub(crate) fn new_native_error_without_backtrace_from_error(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        error: &Error,
    ) -> Result<Value, RuntimeError> {
        let message = error
            .native_message()
            .cloned()
            .unwrap_or_else(|| NativeErrorMessage::from_utf8(error.message()));
        self.new_native_error_without_backtrace_from_message(realm, kind, message)
    }

    pub(crate) fn new_native_error_without_backtrace_from_message(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        message: NativeErrorMessage,
    ) -> Result<Value, RuntimeError> {
        let prototype = {
            let state = self.0.state.borrow();
            state.heap.context(realm)?.native_error_prototypes[kind.index()].ok_or(
                RuntimeError::Invariant("realm has no native Error prototype"),
            )?
        };
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let object = self.new_error_object(&prototype)?;
        let key = self.intern_property_key("message")?;
        let defined = self.define_own_property(
            &object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(message.to_js_string()?)),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Invariant(
                "native Error message definition was rejected",
            ));
        }
        Ok(Value::Object(object))
    }

    pub(crate) fn new_error_object(
        &self,
        prototype: &ObjectRef,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Error prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::error(shape, Vec::new()))
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

    /// Return whether `object` carries the native Error class tag. Prototype
    /// spoofing alone does not make an object an Error.
    pub fn is_error_object(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        Ok(self.0.state.borrow().heap.object(object.object_id())?.kind == ObjectKind::Error)
    }
}
