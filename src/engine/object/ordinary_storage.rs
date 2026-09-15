//! Short, non-reentrant access to ordinary own slots. Slot positions never
//! leave this module and a write locates and commits under one state borrow.
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;
use crate::engine::heap::runtime::RuntimeState;
use crate::engine::heap::{ObjectId, ObjectKind, ObjectPayload, PropertySlot};
use crate::engine::object::shape::PropertyFlags;
use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::Value;

struct OwnSlot {
    index: usize,
    flags: PropertyFlags,
}

// Payload alone is insufficient: module namespaces share Ordinary storage.
fn is_ordinary(data: &crate::engine::heap::ObjectData) -> bool {
    matches!(
        (data.kind, &data.payload),
        (ObjectKind::Ordinary, ObjectPayload::Ordinary)
    )
}

// The caller keeps the state borrowed until the located slot is consumed.
fn locate(
    state: &RuntimeState,
    object: ObjectId,
    atom: Atom,
) -> Result<Option<OwnSlot>, RuntimeError> {
    let data = state.heap.object(object)?;
    let shape = state.heap.shape(data.shape)?;
    let Some(index) = shape.find(atom) else {
        return Ok(None);
    };
    let index = index as usize;
    let entry = shape.entries().get(index).ok_or(RuntimeError::Invariant(
        "ordinary shape index is out of bounds",
    ))?;
    if data.slots.get(index).is_none() {
        return Err(RuntimeError::Invariant(
            "ordinary shape has no parallel slot",
        ));
    }
    Ok(Some(OwnSlot {
        index,
        flags: entry.flags,
    }))
}

#[derive(Clone, Copy)]
pub(super) enum SpecialKind {
    Proxy,
    TypedArray,
    ModuleNamespace,
    Other,
}

// Reuse only for the immediate fallback, before any observable operation.
fn special_kind(data: &crate::engine::heap::ObjectData) -> SpecialKind {
    match (data.kind, &data.payload) {
        (_, ObjectPayload::Proxy(_)) => SpecialKind::Proxy,
        (_, ObjectPayload::TypedArray(_)) => SpecialKind::TypedArray,
        (ObjectKind::ModuleNamespace, _) => SpecialKind::ModuleNamespace,
        _ => SpecialKind::Other,
    }
}

pub(super) enum SetProbe {
    Stored(bool),
    Writable,
    Setter(Option<ObjectId>),
    Missing(Option<ObjectRef>),
    Special(SpecialKind),
}

impl Runtime {
    /// The target stays rooted until the selected setter/prototype has been
    /// promoted. No callback, mutation or cleanup occurs between snapshot and
    /// promotion. Data writes locate and commit in a single mutable borrow.
    pub(super) fn ordinary_set_probe(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: &Value,
        receiver_is_target: bool,
    ) -> Result<SetProbe, RuntimeError> {
        enum Selected {
            Setter(Option<ObjectId>),
            Missing(Option<ObjectId>),
        }
        let selected = {
            let mut state = self.0.state.borrow_mut();
            let id = object.object_id();
            let data = state.heap.object(id)?;
            if !is_ordinary(data) {
                return Ok(SetProbe::Special(special_kind(data)));
            }
            match locate(&state, id, key.atom())? {
                None => {
                    let data = state.heap.object(id)?;
                    Selected::Missing(state.heap.shape(data.shape)?.prototype())
                }
                Some(slot) => match &state.heap.object(id)?.slots[slot.index] {
                    PropertySlot::Data(_) => {
                        if !slot.flags.writable {
                            return Ok(SetProbe::Stored(false));
                        }
                        if !receiver_is_target {
                            return Ok(SetProbe::Writable);
                        }
                        let replacement = PropertySlot::Data(self.raw_property_value(value)?);
                        replace_data(&mut state, id, slot, replacement)?;
                        return Ok(SetProbe::Stored(true));
                    }
                    PropertySlot::Accessor { set, .. } => Selected::Setter(*set),
                    PropertySlot::AutoInit(_) | PropertySlot::VarRef(_) => {
                        return Ok(SetProbe::Special(SpecialKind::Other));
                    }
                },
            }
        };
        Ok(match selected {
            Selected::Setter(set) => SetProbe::Setter(set),
            Selected::Missing(prototype) => SetProbe::Missing(
                prototype
                    .map(|id| ObjectRef::from_borrowed_handle(self.clone(), id))
                    .transpose()?,
            ),
        })
    }
}

fn replace_data(
    state: &mut RuntimeState,
    id: ObjectId,
    slot: OwnSlot,
    replacement: PropertySlot,
) -> Result<(), RuntimeError> {
    state.replace_property_slot(id, slot.index, replacement)
}

pub(super) enum ReadProbe {
    Value(Value),
    Getter(Option<crate::engine::object::CallableRef>),
    Missing(Option<ObjectRef>),
    Special(SpecialKind),
}

pub(super) struct OwnFlags {
    pub(super) flags: PropertyFlags,
    pub(super) needs_materialization: bool,
}

impl Runtime {
    pub(super) fn ordinary_property_flags(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<Option<OwnFlags>>, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let state = self.0.state.borrow();
        let id = object.object_id();
        if !is_ordinary(state.heap.object(id)?) {
            return Ok(None);
        }
        Ok(Some(locate(&state, id, key.atom())?.map(|slot| OwnFlags {
            flags: slot.flags,
            needs_materialization: matches!(
                state.heap.object(id).expect("located live object").slots[slot.index],
                PropertySlot::AutoInit(_) | PropertySlot::VarRef(_)
            ),
        })))
    }

    pub(super) fn ordinary_property_snapshot(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<Option<crate::engine::object::operations::PropertySnapshot>>, RuntimeError>
    {
        use crate::engine::object::operations::PropertySnapshot;
        let state = self.0.state.borrow();
        let id = object.object_id();
        if !is_ordinary(state.heap.object(id)?) {
            return Ok(None);
        }
        let Some(slot) = locate(&state, id, key.atom())? else {
            return Ok(Some(None));
        };
        let flags = slot.flags;
        Ok(Some(Some(
            match &state.heap.object(id)?.slots[slot.index] {
                PropertySlot::Data(value) => PropertySnapshot::Data {
                    value: value.clone(),
                    flags,
                },
                PropertySlot::Accessor { get, set } => PropertySnapshot::Accessor {
                    get: *get,
                    set: *set,
                    flags,
                },
                PropertySlot::VarRef(var_ref) => PropertySnapshot::VarRef {
                    var_ref: *var_ref,
                    flags,
                },
                PropertySlot::AutoInit(_) => PropertySnapshot::AutoInit,
            },
        )))
    }

    pub(super) fn ordinary_read_probe(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<ReadProbe, RuntimeError> {
        enum Selected {
            Value(crate::engine::heap::RawValue),
            Getter(Option<ObjectId>),
            Missing(Option<ObjectId>),
        }
        let selected = {
            let state = self.0.state.borrow();
            let id = object.object_id();
            let data = state.heap.object(id)?;
            let is_array = matches!(
                (data.kind, &data.payload),
                (ObjectKind::Array, ObjectPayload::Array { .. })
            );
            // Dense elements are own data properties. Read the value under
            // this same classification borrow. Other own Array slots share
            // value/getter selection; exotic misses retain their fallback.
            if let Some(index) = key.atom().immediate_integer()
                && let ObjectPayload::Array { dense: Some(dense) } = &data.payload
                && let Some(value) = dense.get(index as usize)
            {
                Selected::Value(value.clone())
            } else if !is_ordinary(data) && !is_array {
                return Ok(ReadProbe::Special(special_kind(data)));
            } else {
                match locate(&state, id, key.atom())? {
                    // A miss may still be a non-immediate dense index. Leave
                    // exotic misses to the complete Array lookup algorithm.
                    None if is_array => return Ok(ReadProbe::Special(SpecialKind::Other)),
                    None => Selected::Missing(state.heap.shape(data.shape)?.prototype()),
                    Some(slot) => match &data.slots[slot.index] {
                        PropertySlot::Data(value) => Selected::Value(value.clone()),
                        PropertySlot::Accessor { get, .. } => Selected::Getter(*get),
                        PropertySlot::AutoInit(_) | PropertySlot::VarRef(_) => {
                            return Ok(ReadProbe::Special(SpecialKind::Other));
                        }
                    },
                }
            }
        };
        Ok(match selected {
            Selected::Value(value) => ReadProbe::Value(self.root_raw_value(&value)?),
            Selected::Getter(get) => ReadProbe::Getter(
                get.map(|id| {
                    ObjectRef::from_borrowed_handle(self.clone(), id)
                        .map(crate::engine::object::CallableRef::from_validated_object)
                })
                .transpose()?,
            ),
            Selected::Missing(prototype) => ReadProbe::Missing(
                prototype
                    .map(|id| ObjectRef::from_borrowed_handle(self.clone(), id))
                    .transpose()?,
            ),
        })
    }
}

impl Runtime {
    pub(super) fn try_define_ordinary_value(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &crate::engine::object::OrdinaryPropertyDescriptor,
    ) -> Result<Option<bool>, RuntimeError> {
        use crate::engine::object::DescriptorField;
        let DescriptorField::Present(value) = &descriptor.value else {
            return Ok(None);
        };
        if !matches!(descriptor.writable, DescriptorField::Absent)
            || !matches!(descriptor.enumerable, DescriptorField::Absent)
            || !matches!(descriptor.configurable, DescriptorField::Absent)
            || !matches!(descriptor.get, DescriptorField::Absent)
            || !matches!(descriptor.set, DescriptorField::Absent)
        {
            return Ok(None);
        }
        let mut state = self.0.state.borrow_mut();
        let id = object.object_id();
        if !is_ordinary(state.heap.object(id)?) {
            return Ok(None);
        }
        let Some(slot) = locate(&state, id, key.atom())? else {
            return Ok(None);
        };
        let PropertySlot::Data(old) = &state.heap.object(id)?.slots[slot.index] else {
            return Ok(None);
        };
        let raw = self.raw_property_value(value)?;
        if !crate::engine::object::property::data_value_update_allowed(
            slot.flags.configurable,
            slot.flags.writable,
            old,
            &raw,
            crate::engine::value::collection_key::same_value,
        ) {
            return Ok(Some(false));
        }
        replace_data(&mut state, id, slot, PropertySlot::Data(raw))?;
        Ok(Some(true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::object::{DescriptorField, OrdinaryPropertyDescriptor};

    #[test]
    fn ordinary_property_replacement_preserves_roots_and_rejects_foreign_values() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = runtime.new_object(None).unwrap();
        let old = runtime.new_object(None).unwrap();
        let key = runtime.intern_property_key("x").unwrap();
        runtime
            .define_own_property(
                &object,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Object(old.clone())),
                    writable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap();
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(old.object_id()),
            Ok(2)
        );
        assert!(context.set_property(&object, &key, Value::Int(42)).unwrap());
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(old.object_id()),
            Ok(1)
        );
        let foreign = Runtime::new().new_object(None).unwrap();
        assert!(
            context
                .set_property(&object, &key, Value::Object(foreign))
                .is_err()
        );
        assert_eq!(context.get_property(&object, &key).unwrap(), Value::Int(42));
    }

    #[test]
    fn ordinary_property_own_write_stops_before_revoked_proxy_prototype() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
            var rev = Proxy.revocable({}, {});
            var obj = Object.create(rev.proxy);
            Object.defineProperty(obj, 'x', {value: 1, writable: true});
            rev.revoke();
            obj.x = 42;
            obj.x;
        "#
                )
                .unwrap(),
            Value::Int(42)
        );
    }
}
