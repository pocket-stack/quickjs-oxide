use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::function::metadata::ClosureVariableKind;
use crate::engine::heap::roots::VarRefRoot;

use crate::engine::heap::{ObjectId, ObjectPayload, PropertySlot, RawValue};
use crate::engine::object::shape::{PropertyFlags, ShapeEntry};
use crate::engine::object::{
    AccessorValue, CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, properties,
};
use crate::engine::value::Value;

impl Runtime {
    pub(crate) fn validate_object_and_key(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("property key"));
        }
        Ok(())
    }

    pub(crate) fn validate_descriptor_domains(
        &self,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<(), RuntimeError> {
        if let DescriptorField::Present(value) = &descriptor.value {
            match value {
                Value::Object(object) if !object.belongs_to(self) => {
                    return Err(RuntimeError::WrongRuntime("descriptor value"));
                }
                Value::Symbol(symbol) if !symbol.belongs_to(self) => {
                    return Err(RuntimeError::WrongRuntime("descriptor value"));
                }
                _ => {}
            }
        }
        for accessor in [&descriptor.get, &descriptor.set] {
            if let DescriptorField::Present(AccessorValue::Callable(callable)) = accessor {
                if !callable.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("descriptor accessor"));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn validate_value_domain(
        &self,
        value: &Value,
        role: &'static str,
    ) -> Result<(), RuntimeError> {
        match value {
            Value::Object(object) if !object.belongs_to(self) => {
                Err(RuntimeError::WrongRuntime(role))
            }
            Value::Symbol(symbol) if !symbol.belongs_to(self) => {
                Err(RuntimeError::WrongRuntime(role))
            }
            _ => Ok(()),
        }
    }

    /// Return whether `value` carries QuickJS's identity-local Annex B
    /// `is_HTMLDDA` object bit.
    pub(crate) fn value_is_html_dda(&self, value: &Value) -> Result<bool, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(false);
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("IsHTMLDDA value"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .object(object.object_id())?
            .is_html_dda)
    }

    /// Test `[[Call]]` without creating a capability handle or entering a
    /// public runtime operation. Fused bytecode predicates use this after the
    /// HTMLDDA check, matching QuickJS's allocation-free tag path.
    pub(crate) fn value_is_callable(&self, value: &Value) -> Result<bool, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(false);
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("callable value"));
        }
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        Ok(matches!(
            &object.payload,
            ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
                | ObjectPayload::Proxy(crate::engine::heap::ProxyData {
                    is_callable: true,
                    ..
                })
        ))
    }

    /// Apply ECMAScript `ToBoolean`, including QuickJS's Annex B falsy
    /// `is_HTMLDDA` object exception.
    pub(crate) fn value_to_boolean(&self, value: &Value) -> Result<bool, RuntimeError> {
        if self.value_is_html_dda(value)? {
            Ok(false)
        } else {
            Ok(value.to_boolean_primitive())
        }
    }

    /// Mirror `JS_SetIsHTMLDDA` for a runtime-owned object.
    #[cfg(feature = "test262-host")]
    pub(crate) fn set_object_is_html_dda(&self, object: &ObjectRef) -> Result<(), RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("IsHTMLDDA object"));
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .set_object_is_html_dda(object.object_id())?;
        Ok(())
    }

    pub(crate) fn raw_property_value(&self, value: &Value) -> Result<RawValue, RuntimeError> {
        Ok(match value {
            Value::Undefined => RawValue::Undefined,
            Value::Null => RawValue::Null,
            Value::Bool(value) => RawValue::Bool(*value),
            Value::Int(value) => RawValue::Int(*value),
            Value::Float(value) => RawValue::Float(*value),
            Value::BigInt(value) => RawValue::BigInt(value.clone()),
            Value::String(value) => RawValue::String(value.clone()),
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("property value"));
                }
                RawValue::Symbol(symbol.atom())
            }
            Value::Object(object) => {
                if !object.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("property value"));
                }
                RawValue::Object(object.object_id())
            }
        })
    }

    pub(crate) fn store_complete_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        complete: CompleteOrdinaryPropertyDescriptor,
    ) -> Result<(), RuntimeError> {
        let global_hidden = {
            let state = self.0.state.borrow();
            match state.heap.object(object.object_id())?.payload {
                ObjectPayload::GlobalObject { uninitialized_vars } => Some(uninitialized_vars),
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
        if let Some(hidden) = global_hidden {
            return self.store_complete_global_property(object, hidden, key, complete);
        }

        let (flags, replacement) = match complete {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => (
                PropertyFlags::data(writable, enumerable, configurable),
                PropertySlot::Data(self.raw_property_value(&value)?),
            ),
            CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => (
                PropertyFlags::accessor(enumerable, configurable),
                PropertySlot::Accessor {
                    get: get.as_ref().map(|value| value.as_object().object_id()),
                    set: set.as_ref().map(|value| value.as_object().object_id()),
                },
            ),
        };
        self.store_property_slot(object, key, flags, replacement)
    }

    pub(crate) fn store_complete_global_property(
        &self,
        object: &ObjectRef,
        hidden_id: ObjectId,
        key: &PropertyKey,
        complete: CompleteOrdinaryPropertyDescriptor,
    ) -> Result<(), RuntimeError> {
        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden_id)?;
        match complete {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => {
                let global_root = self.own_var_ref_root(object, key)?;
                let hidden_root = if global_root.is_none() {
                    self.own_var_ref_root(&hidden, key)?
                } else {
                    None
                };
                let root = if let Some(root) = global_root {
                    self.write_var_ref(&root, value)?;
                    root
                } else if let Some(root) = hidden_root {
                    if !self.delete_property(&hidden, key)? {
                        return Err(RuntimeError::Invariant(
                            "hidden global VarRef property was not configurable",
                        ));
                    }
                    self.write_var_ref(&root, value)?;
                    root
                } else {
                    self.new_var_ref(value, false, !writable, ClosureVariableKind::Normal)?
                };
                self.set_var_ref_metadata(&root, false, !writable, ClosureVariableKind::Normal)?;
                self.store_property_slot(
                    object,
                    key,
                    PropertyFlags::data(writable, enumerable, configurable),
                    PropertySlot::VarRef(root.id()),
                )
            }
            CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => {
                if let Some(root) = self.own_var_ref_root(object, key)? {
                    let shared = self.0.state.borrow().heap.var_ref_strong_count(root.id())? > 2;
                    if shared {
                        if self.own_var_ref_root(&hidden, key)?.is_some() {
                            return Err(RuntimeError::Invariant(
                                "global property and hidden table contain distinct VarRefs",
                            ));
                        }
                        self.reset_var_ref_uninitialized(&root)?;
                        self.set_var_ref_metadata(
                            &root,
                            false,
                            false,
                            ClosureVariableKind::Normal,
                        )?;
                        self.store_property_slot(
                            &hidden,
                            key,
                            PropertyFlags::data(true, true, true),
                            PropertySlot::VarRef(root.id()),
                        )?;
                    }
                }
                self.store_property_slot(
                    object,
                    key,
                    PropertyFlags::accessor(enumerable, configurable),
                    PropertySlot::Accessor {
                        get: get.as_ref().map(|value| value.as_object().object_id()),
                        set: set.as_ref().map(|value| value.as_object().object_id()),
                    },
                )
            }
        }
    }

    pub(crate) fn own_var_ref_root(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<VarRefRoot>, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let id = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object.object_id())?;
            let shape = state.heap.shape(object.shape)?;
            let Some(index) = shape.find(key.atom()) else {
                return Ok(None);
            };
            match object.slots.get(index as usize) {
                Some(PropertySlot::VarRef(id)) => Some(*id),
                Some(
                    PropertySlot::Data(_)
                    | PropertySlot::Accessor { .. }
                    | PropertySlot::AutoInit(_),
                ) => None,
                None => {
                    return Err(RuntimeError::Invariant(
                        "shape property has no parallel object slot",
                    ));
                }
            }
        };
        id.map(|id| VarRefRoot::from_borrowed_handle(self.clone(), id).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn store_property_slot(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        flags: PropertyFlags,
        replacement: PropertySlot,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let object_id = object.object_id();
        let (shape_id, shape_len, existing) = {
            let object_data = state.heap.object(object_id)?;
            let shape = state.heap.shape(object_data.shape)?;
            (
                object_data.shape,
                shape.entries().len(),
                shape.find(key.atom()).map(|index| index as usize),
            )
        };
        if existing.is_none()
            && shape_len >= properties::MIN_UNIQUE_SHAPE_APPEND_ENTRIES
            && state.heap.shape_strong_count(shape_id)? == 1
        {
            return state.append_unique_layout(object_id, key.atom(), flags, replacement);
        }
        let (prototype, mut entries, mut slots, existing) = {
            let object_data = state.heap.object(object_id)?;
            let shape = state.heap.shape(object_data.shape)?;
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object_data.slots.clone(),
                shape.find(key.atom()).map(|index| index as usize),
            )
        };

        if let Some(index) = existing {
            let entry = entries.get(index).ok_or(RuntimeError::Invariant(
                "shape lookup index was out of bounds",
            ))?;
            if entry.flags == flags {
                let retained_atoms = state.retain_slot_atoms(std::slice::from_ref(&replacement))?;
                match state
                    .heap
                    .replace_object_slot(object_id, index, replacement)
                {
                    Ok(cleanup) => return state.apply_cleanup(cleanup),
                    Err(error) => {
                        state.release_atoms(retained_atoms)?;
                        return Err(error.into());
                    }
                }
            }
            entries[index].flags = flags;
            slots[index] = replacement;
        } else {
            entries.push(ShapeEntry {
                atom: key.atom(),
                flags,
            });
            slots.push(replacement);
        }
        state.replace_layout(object_id, prototype, &entries, slots)
    }
}
