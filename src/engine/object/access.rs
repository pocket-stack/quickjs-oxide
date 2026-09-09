use crate::engine::api::error::{ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;
use crate::engine::builtins::native::PrimitiveKind;
use crate::engine::heap::runtime::RuntimeState;

use crate::engine::heap::{ContextId, ObjectId, PropertySlot, RawValue};
use crate::engine::object::operations::RawStringProperty;
use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;

impl Runtime {
    pub(crate) fn get_property_in_realm(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        self.internal_get(realm, object, key, Value::Object(object.clone()))
    }

    pub(crate) fn get_string_property_with_receiver(
        &self,
        realm: ContextId,
        string: &JsString,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Completion, RuntimeError> {
        let index = self.0.state.borrow().atoms.array_index(key.atom())?;
        if let Some(index) = index
            && let Ok(index) = usize::try_from(index)
            && let Some(unit) = string.code_unit_at(index)
        {
            return Ok(Completion::Return(Value::String(JsString::from_code_unit(
                unit,
            ))));
        }
        let length = self.intern_property_key("length")?;
        if key == &length {
            let length = i32::try_from(string.len())
                .map(Value::Int)
                .unwrap_or_else(|_| Value::number(string.len() as f64));
            return Ok(Completion::Return(length));
        }
        let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::String)?;
        self.internal_get(realm, &prototype, key, receiver)
    }

    pub(crate) fn get_value_property_in_realm(
        &self,
        realm: ContextId,
        receiver: Value,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        match &receiver {
            Value::Object(object) => self.internal_get(realm, object, key, receiver.clone()),
            Value::String(string) => {
                self.get_string_property_with_receiver(realm, string, key, receiver.clone())
            }
            Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::Symbol(_) => {
                let kind = match &receiver {
                    Value::Bool(_) => PrimitiveKind::Boolean,
                    Value::Int(_) | Value::Float(_) => PrimitiveKind::Number,
                    Value::BigInt(_) => PrimitiveKind::BigInt,
                    Value::Symbol(_) => PrimitiveKind::Symbol,
                    _ => unreachable!(),
                };
                let prototype = self.primitive_prototype_for_realm(realm, kind)?;
                self.internal_get(realm, &prototype, key, receiver.clone())
            }
            Value::Undefined | Value::Null => {
                let suffix = if matches!(receiver, Value::Null) {
                    "' of null"
                } else {
                    "' of undefined"
                };
                let error =
                    self.native_atom_error(ErrorKind::Type, "cannot read property '", key, suffix)?;
                Ok(Completion::Throw(self.new_native_error_from_error(
                    realm,
                    NativeErrorKind::Type,
                    &error,
                )?))
            }
        }
    }

    pub(crate) fn get_property_or_missing_in_realm(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<Completion>, RuntimeError> {
        match self.internal_get_or_missing(realm, object, key, Value::Object(object.clone()))? {
            NativeConversion::Value(Some(value)) => Ok(Some(Completion::Return(value))),
            NativeConversion::Value(None) => Ok(None),
            NativeConversion::Throw(value) => Ok(Some(Completion::Throw(value))),
        }
    }

    pub(crate) fn has_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<bool, RuntimeError> {
        let mut cursor = Some(object.clone());
        while let Some(current) = cursor {
            if self.has_own_property(&current, key)? {
                return Ok(true);
            }
            cursor = self.get_prototype_of(&current)?;
        }
        Ok(false)
    }

    /// Completion-aware `[[HasProperty]]` boundary used by source `in`.
    /// Proxy trap throws cross this boundary without changing the VM opcode
    /// contract.
    pub(crate) fn has_property_in_realm(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        Ok(match self.internal_has_property(realm, object, key)? {
            NativeConversion::Value(present) => Completion::Return(Value::Bool(present)),
            NativeConversion::Throw(value) => Completion::Throw(value),
        })
    }
}

pub(crate) fn raw_string_property_on_object(
    state: &RuntimeState,
    object: ObjectId,
    atom: Atom,
) -> Result<RawStringProperty, RuntimeError> {
    let object = state.heap.object(object)?;
    let shape = state.heap.shape(object.shape)?;
    let Some(index) = shape.find(atom) else {
        return Ok(RawStringProperty::Missing);
    };
    let slot = object
        .slots
        .get(index as usize)
        .ok_or(RuntimeError::Invariant(
            "backtrace name shape has no parallel property slot",
        ))?;
    Ok(match slot {
        PropertySlot::Data(RawValue::String(value)) if value.is_flat() => {
            RawStringProperty::String(value.clone())
        }
        PropertySlot::Data(RawValue::String(_)) => RawStringProperty::Other,
        PropertySlot::Data(_)
        | PropertySlot::VarRef(_)
        | PropertySlot::Accessor { .. }
        | PropertySlot::AutoInit(_) => RawStringProperty::Other,
    })
}

pub(crate) fn raw_string_property_one_level(
    state: &RuntimeState,
    object: ObjectId,
    atom: Atom,
) -> Result<Option<JsString>, RuntimeError> {
    match raw_string_property_on_object(state, object, atom)? {
        RawStringProperty::String(name) => return Ok(Some(name)),
        RawStringProperty::Other => return Ok(None),
        RawStringProperty::Missing => {}
    }

    let object = state.heap.object(object)?;
    let Some(prototype) = state.heap.shape(object.shape)?.prototype() else {
        return Ok(None);
    };
    Ok(
        match raw_string_property_on_object(state, prototype, atom)? {
            RawStringProperty::String(name) => Some(name),
            RawStringProperty::Missing | RawStringProperty::Other => None,
        },
    )
}
