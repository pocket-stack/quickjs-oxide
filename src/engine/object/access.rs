use crate::engine::api::error::{ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::{Atom, AtomIdx};
use crate::engine::builtins::native::PrimitiveKind;
use crate::engine::heap::runtime::RuntimeState;

use crate::engine::heap::{ContextId, ObjectId, PropertySlot, RawValue};
use crate::engine::object::operations::RawStringProperty;
use crate::engine::object::ordinary::OrdinaryRead;
use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;

impl Runtime {
    /// Virtual own properties of primitive bases; object deletion has its own protocol.
    pub(crate) fn primitive_delete_property(
        &self,
        base: &Value,
        key: &PropertyKey,
    ) -> Result<bool, RuntimeError> {
        self.validate_value_domain(base, "delete base")?;
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("delete property key"));
        }
        Ok(match base {
            Value::Null | Value::Undefined => {
                return Err(RuntimeError::Engine(crate::engine::api::Error::new(
                    ErrorKind::Type,
                    "cannot convert to object",
                )));
            }
            Value::Object(_) => {
                return Err(RuntimeError::Invariant(
                    "primitive Delete received an object",
                ));
            }
            Value::String(string) => {
                let index = self.0.state.borrow().atoms.array_index(key.atom())?;
                let indexed = index.is_some_and(|index| {
                    usize::try_from(index).is_ok_and(|index| index < string.len())
                });
                !indexed
                    && key
                        != &self
                            .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?
            }
            _ => true,
        })
    }

    pub(crate) fn finish_property_delete(
        &self,
        result: NativeConversion<bool>,
        strict: bool,
    ) -> Result<Completion, RuntimeError> {
        match result {
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
            NativeConversion::Value(false) if strict => Err(RuntimeError::Engine(
                crate::engine::api::Error::new(ErrorKind::Type, "could not delete property"),
            )),
            NativeConversion::Value(value) => Ok(Completion::Return(Value::Bool(value))),
        }
    }

    pub(crate) fn get_property_in_realm(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        self.internal_get(realm, object, key, Value::Object(object.clone()))
    }

    fn prepare_string_property_read(
        &self,
        realm: ContextId,
        string: &JsString,
        key: &PropertyKey,
        receiver: &Value,
        native: Option<&mut Option<crate::engine::object::LinkedNativeSelection>>,
    ) -> Result<OrdinaryRead, RuntimeError> {
        let index = self.0.state.borrow().atoms.array_index(key.atom())?;
        if let Some(index) = index
            && let Ok(index) = usize::try_from(index)
            && let Some(unit) = string.code_unit_at(index)
        {
            // A fresh string payload is a genuine creation point: publish it
            // as one owned arena node.
            let id = self
                .0
                .state
                .borrow_mut()
                .heap
                .allocate_string(JsString::from_code_unit(unit))?;
            return Ok(OrdinaryRead::Complete(Some(
                crate::engine::value::JsValue::String(id),
            )));
        }
        let length = self.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?;
        if key == &length {
            let length = i32::try_from(string.len())
                .map(crate::engine::value::JsValue::Int)
                .unwrap_or_else(|_| {
                    crate::engine::value::JsValue::Float(string.len() as f64)
                });
            return Ok(OrdinaryRead::Complete(Some(length)));
        }
        let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::String)?;
        self.prepare_ordinary_read_selected(&prototype, key, receiver, native)
    }

    /// Select a read without invoking its getter. Primitive receivers stay
    /// primitive; String own units/length retain the existing unboxed kernel.
    pub(crate) fn prepare_value_property_read(
        &self,
        realm: ContextId,
        receiver: Value,
        key: &PropertyKey,
    ) -> Result<OrdinaryRead, RuntimeError> {
        self.prepare_value_property_read_borrowed(realm, &receiver, key)
    }

    pub(crate) fn prepare_value_property_read_borrowed(
        &self,
        realm: ContextId,
        receiver: &Value,
        key: &PropertyKey,
    ) -> Result<OrdinaryRead, RuntimeError> {
        self.prepare_value_property_read_selected(realm, receiver, key, None)
    }
    /// Internal-value receiver form: the slow-path read roots the receiver
    /// once for accessor/prototype Call selection.
    pub(crate) fn prepare_value_property_read_selected_jsvalue(
        &self,
        realm: ContextId,
        receiver: &crate::engine::value::JsValue,
        key: &PropertyKey,
        native: Option<&mut Option<crate::engine::object::LinkedNativeSelection>>,
    ) -> Result<OrdinaryRead, RuntimeError> {
        let receiver_root = self.root_value(receiver)?;
        self.prepare_value_property_read_selected(realm, &receiver_root, key, native)
    }

    pub(crate) fn prepare_value_property_read_borrowed_jsvalue(
        &self,
        realm: ContextId,
        receiver: &crate::engine::value::JsValue,
        key: &PropertyKey,
    ) -> Result<OrdinaryRead, RuntimeError> {
        self.prepare_value_property_read_selected_jsvalue(realm, receiver, key, None)
    }

    pub(crate) fn prepare_value_property_read_selected(
        &self,
        realm: ContextId,
        receiver: &Value,
        key: &PropertyKey,
        native: Option<&mut Option<crate::engine::object::LinkedNativeSelection>>,
    ) -> Result<OrdinaryRead, RuntimeError> {
        self.validate_value_domain(receiver, "property receiver")?;
        match receiver {
            Value::Object(object) => {
                self.prepare_ordinary_read_selected(object, key, receiver, native)
            }
            Value::String(string) => {
                self.prepare_string_property_read(realm, string, key, receiver, native)
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
                self.prepare_ordinary_read_selected(&prototype, key, receiver, native)
            }
            Value::Undefined | Value::Null => {
                let suffix = if matches!(receiver, Value::Null) {
                    "' of null"
                } else {
                    "' of undefined"
                };
                Err(RuntimeError::Engine(self.native_atom_error(
                    ErrorKind::Type,
                    "cannot read property '",
                    key,
                    suffix,
                )?))
            }
        }
    }

    fn finish_value_property_read(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        read: OrdinaryRead,
    ) -> Result<Completion, RuntimeError> {
        Ok(match self.finish_prepared_read(realm, key, read)? {
            NativeConversion::Value(value) => Completion::Return(value.unwrap_or(Value::Undefined)),
            NativeConversion::Throw(value) => Completion::Throw(value),
        })
    }

    /// Keep JavaScript-visible read failures as replies to the selected operation.
    pub(crate) fn prepare_value_property_read_completion(
        &self,
        realm: ContextId,
        receiver: Value,
        key: &PropertyKey,
    ) -> Result<NativeConversion<OrdinaryRead>, RuntimeError> {
        let nullish = matches!(receiver, Value::Null | Value::Undefined);
        match self.prepare_value_property_read(realm, receiver, key) {
            Ok(read) => Ok(NativeConversion::Value(read)),
            Err(RuntimeError::Engine(error)) if nullish && error.kind() == ErrorKind::Type => {
                Ok(NativeConversion::Throw(self.new_native_error_from_error(
                    realm,
                    NativeErrorKind::Type,
                    &error,
                )?))
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn get_value_property_in_realm(
        &self,
        realm: ContextId,
        receiver: Value,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        match self.prepare_value_property_read_completion(realm, receiver, key)? {
            NativeConversion::Value(read) => self.finish_value_property_read(realm, key, read),
            NativeConversion::Throw(reason) => Ok(Completion::Throw(reason)),
        }
    }

    #[cfg(test)]
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
}

pub(crate) fn raw_string_property_on_object(
    state: &RuntimeState,
    object: ObjectId,
    atom: Atom,
) -> Result<RawStringProperty, RuntimeError> {
    let object = state.heap.object(object)?;
    let shape = state.heap.shape(object.shape)?;
    let Some(index) = shape.find(AtomIdx::from_raw(atom.raw())) else {
        return Ok(RawStringProperty::Missing);
    };
    let slot = object
        .slots
        .get(index as usize)
        .ok_or(RuntimeError::Invariant(
            "backtrace name shape has no parallel property slot",
        ))?;
    Ok(match slot {
        PropertySlot::Data(RawValue::String(value)) => {
            let string = state.heap.string(*value)?;
            if string.is_flat() {
                RawStringProperty::String(string.clone())
            } else {
                RawStringProperty::Other
            }
        }
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
