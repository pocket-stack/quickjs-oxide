//! Short, non-reentrant access to ordinary own slots. Slot positions never
//! leave this module and a write locates and commits under one state borrow.
#[cfg(feature = "stack-vm")]
mod ic;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;
use crate::engine::heap::runtime::RuntimeState;
use crate::engine::heap::{ObjectId, ObjectKind, ObjectPayload, PropertySlot};
use crate::engine::object::shape::PropertyFlags;
use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::Value;

/// Affine native payload fact selected together with an own property value.
/// Its callee remains retained by the result/operand owner; consumption checks
/// runtime and generational identity, never re-reads a property or payload.
pub(crate) struct LinkedNativeSelection {
    runtime: Runtime,
    function: ObjectId,
    data: crate::engine::builtins::native::NativeFunctionData,
}
impl LinkedNativeSelection {
    pub(crate) fn into_parts(
        self,
        function: &ObjectRef,
    ) -> Option<crate::engine::builtins::native::NativeFunctionData> {
        (function.belongs_to(&self.runtime) && function.object_id() == self.function)
            .then_some(self.data)
    }
}

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

// The shared physical selector used by both single-step Set and the local
// missing-receiver walk. It neither roots handles nor performs observable work.
enum BorrowedSet {
    Missing(Option<ObjectId>),
    Data(OwnSlot),
    Setter(Option<ObjectId>),
    Special(SpecialKind),
}
fn select_set_slot(
    state: &RuntimeState,
    id: ObjectId,
    atom: Atom,
) -> Result<BorrowedSet, RuntimeError> {
    let data = state.heap.object(id)?;
    let ordinary = is_ordinary(data);
    #[cfg(feature = "stack-vm")]
    let ordinary = ordinary
        || match &data.payload {
            // RegExp's lastIndex is an ordinary data property. The branded
            // executor changes its value but adds no exotic [[Set]] semantics.
            ObjectPayload::RegExp(_) => true,
            ObjectPayload::Array { .. } => {
                let shape = state.heap.shape(data.shape)?;
                state.atoms.array_index(atom)?.is_none()
                    && shape
                        .entries()
                        .first()
                        .is_some_and(|entry| entry.atom != atom)
            }
            _ => false,
        };
    if !ordinary {
        return Ok(BorrowedSet::Special(special_kind(data)));
    }
    Ok(match locate(state, id, atom)? {
        None => BorrowedSet::Missing(state.heap.shape(data.shape)?.prototype()),
        Some(slot) => match &data.slots[slot.index] {
            PropertySlot::Data(_) => BorrowedSet::Data(slot),
            PropertySlot::Accessor { set, .. } => BorrowedSet::Setter(*set),
            PropertySlot::AutoInit(_) | PropertySlot::VarRef(_) => {
                BorrowedSet::Special(SpecialKind::Other)
            }
        },
    })
}

#[cfg(feature = "stack-vm")]
enum MissingSelection {
    Define,
    Complete(SetProbe),
    Special(ObjectId, SpecialKind),
}
#[cfg(feature = "stack-vm")]
fn select_missing_prototypes(
    state: &RuntimeState,
    atom: Atom,
    mut prototype: Option<ObjectId>,
) -> Result<MissingSelection, RuntimeError> {
    while let Some(id) = prototype {
        let data = state.heap.object(id)?;
        if let ObjectPayload::Array { dense } = &data.payload
            && let Some(index) = state.atoms.array_index(atom)?
        {
            // Prototype lookup does not grow this Array. Dense elements are
            // writable data properties; missing indices continue the chain.
            if dense
                .as_ref()
                .is_some_and(|values| (index as usize) < values.len())
            {
                break;
            }
            if let Some(slot) = locate(state, id, atom)? {
                match &data.slots[slot.index] {
                    PropertySlot::Data(_) if slot.flags.writable => break,
                    PropertySlot::Data(_) => {
                        return Ok(MissingSelection::Complete(SetProbe::Stored(false)));
                    }
                    PropertySlot::Accessor { set, .. } => {
                        return Ok(MissingSelection::Complete(SetProbe::Setter(*set)));
                    }
                    _ => return Ok(MissingSelection::Special(id, SpecialKind::Other)),
                }
            }
            prototype = state.heap.shape(data.shape)?.prototype();
            continue;
        }
        match select_set_slot(state, id, atom)? {
            BorrowedSet::Missing(next) => prototype = next,
            BorrowedSet::Data(slot) if slot.flags.writable => break,
            BorrowedSet::Data(_) => return Ok(MissingSelection::Complete(SetProbe::Stored(false))),
            BorrowedSet::Setter(setter) => {
                return Ok(MissingSelection::Complete(SetProbe::Setter(setter)));
            }
            BorrowedSet::Special(kind) => return Ok(MissingSelection::Special(id, kind)),
        }
    }
    Ok(MissingSelection::Define)
}

/// Only a proof for the current uninterrupted borrow, never cached.
#[cfg(feature = "stack-vm")]
pub(super) fn prototypes_allow_dense_append(
    state: &RuntimeState,
    atom: Atom,
    prototype: Option<ObjectId>,
) -> Result<bool, RuntimeError> {
    Ok(matches!(
        select_missing_prototypes(state, atom, prototype)?,
        MissingSelection::Define
    ))
}

/// Continue an already-selected missing own property without releasing the
/// borrow. Any exotic boundary declines before changing the receiver; the
/// ordinary state machine then performs its original observable protocol.
#[cfg(feature = "stack-vm")]
fn set_missing_local(
    runtime: &Runtime,
    state: &mut RuntimeState,
    receiver: ObjectId,
    atom: Atom,
    value: &Value,
    prototype: Option<ObjectId>,
) -> Result<MissingSelection, RuntimeError> {
    match select_missing_prototypes(state, atom, prototype)? {
        MissingSelection::Define => {}
        selected => return Ok(selected),
    }
    use crate::engine::object::property::{
        PropertyDescriptor, validate_and_apply_property_descriptor,
    };
    let descriptor = PropertyDescriptor {
        value: Some(value),
        writable: Some(true),
        enumerable: Some(true),
        configurable: Some(true),
        ..PropertyDescriptor::new()
    };
    // Use the same descriptor permissions as DefineOwnProperty. Rejection stays
    // on the existing rejection path, which preserves its precise error reason.
    if validate_and_apply_property_descriptor(
        state.heap.object(receiver)?.extensible,
        &descriptor,
        None,
        &&Value::Undefined,
        |a, b| Value::same_value(a, b),
    )
    .is_err()
    {
        return Ok(MissingSelection::Complete(SetProbe::Rejected(
            crate::engine::object::operations::PropertySetRejection::NotExtensible,
        )));
    }
    let replacement = PropertySlot::Data(runtime.raw_property_value(value)?);
    state.store_selected_property_slot(
        receiver,
        atom,
        PropertyFlags::data(true, true, true),
        replacement,
        None,
    )?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event(
        "set_missing_committed_from_selection",
    );
    Ok(MissingSelection::Complete(SetProbe::Stored(true)))
}

#[derive(Clone, Copy)]
pub(crate) enum SpecialKind {
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
    #[cfg(feature = "stack-vm")]
    Rejected(crate::engine::object::operations::PropertySetRejection),
    #[cfg(feature = "stack-vm")]
    SpecialAt(ObjectRef, SpecialKind),
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
        self.ordinary_set_probe_inner(object, key, value, receiver_is_target, true)
    }

    // OrdinarySetWithOwnDescriptor checks only Receiver's own descriptor. Its
    // prototype must not be consulted again after the target selected a writable
    // data descriptor (Reflect.set may have a completely different receiver).
    pub(super) fn ordinary_set_receiver_probe(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: &Value,
    ) -> Result<SetProbe, RuntimeError> {
        self.ordinary_set_probe_inner(object, key, value, true, false)
    }

    fn ordinary_set_probe_inner(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: &Value,
        receiver_is_target: bool,
        _walk_missing: bool,
    ) -> Result<SetProbe, RuntimeError> {
        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
        crate::engine::api::profiling::record_owned_execution_event("property_storage_set_probe");
        enum Selected {
            Setter(Option<ObjectId>),
            Missing(Option<ObjectId>),
            Dense(u32),
            #[cfg(feature = "stack-vm")]
            DenseAppend(u32),
            #[cfg(feature = "stack-vm")]
            SpecialAt(ObjectId, SpecialKind),
        }
        let selected = {
            let mut state = self.0.state.borrow_mut();
            let id = object.object_id();
            let data = state.heap.object(id)?;
            #[cfg(feature = "stack-vm")]
            let dense_index = if receiver_is_target && matches!(data.kind, ObjectKind::Array) {
                key.atom().immediate_integer().and_then(|index| {
                    if let ObjectPayload::Array { dense: Some(dense) } = &data.payload {
                        (index as usize <= dense.len()).then_some((index, dense.len()))
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            #[cfg(not(feature = "stack-vm"))]
            let dense_index: Option<(u32, usize)> = None;
            if let Some((index, dense_len)) = dense_index {
                if (index as usize) < dense_len {
                    Selected::Dense(index)
                } else {
                    #[cfg(feature = "stack-vm")]
                    {
                        let prototype = if _walk_missing {
                            state.heap.shape(data.shape)?.prototype()
                        } else {
                            None
                        };
                        match select_missing_prototypes(&state, key.atom(), prototype)? {
                            MissingSelection::Define => Selected::DenseAppend(index),
                            MissingSelection::Complete(result) => return Ok(result),
                            MissingSelection::Special(id, kind) => Selected::SpecialAt(id, kind),
                        }
                    }
                    #[cfg(not(feature = "stack-vm"))]
                    unreachable!("dense local selection is stack-vm only")
                }
            } else {
                match select_set_slot(&state, id, key.atom())? {
                    BorrowedSet::Missing(prototype) => {
                        #[cfg(feature = "stack-vm")]
                        if receiver_is_target {
                            match set_missing_local(
                                self,
                                &mut state,
                                id,
                                key.atom(),
                                value,
                                if _walk_missing { prototype } else { None },
                            )? {
                                MissingSelection::Complete(result) => return Ok(result),
                                MissingSelection::Special(id, kind) => {
                                    Selected::SpecialAt(id, kind)
                                }
                                MissingSelection::Define => {
                                    unreachable!("missing receiver definition is consumed locally")
                                }
                            }
                        } else {
                            Selected::Missing(prototype)
                        }
                        #[cfg(not(feature = "stack-vm"))]
                        Selected::Missing(prototype)
                    }
                    BorrowedSet::Data(slot) => {
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
                    BorrowedSet::Setter(set) => Selected::Setter(set),
                    BorrowedSet::Special(kind) => return Ok(SetProbe::Special(kind)),
                }
            }
        };
        Ok(match selected {
            Selected::Dense(index) => {
                // No callback or owner release occurs between selection and
                // this authoritative transaction, which rechecks dense bounds.
                self.replace_dense_array_value(object, index, value)?;
                SetProbe::Stored(true)
            }
            #[cfg(feature = "stack-vm")]
            Selected::SpecialAt(id, kind) => {
                SetProbe::SpecialAt(ObjectRef::from_borrowed_handle(self.clone(), id)?, kind)
            }
            #[cfg(feature = "stack-vm")]
            Selected::DenseAppend(index) => {
                // This is the exact missing element selected above, with no
                // callback or owner release before the shared Array definition.
                match self.define_selected_dense_array_append(object, index, value)? {
                    None => SetProbe::Stored(true),
                    Some(reason) => SetProbe::Rejected(reason),
                }
            }
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
        self.ordinary_read_probe_atom(object, key.atom(), false, None)
    }

    fn ordinary_read_probe_atom(
        &self,
        object: &ObjectRef,
        atom: Atom,
        own_only: bool,
        mut native: Option<&mut Option<LinkedNativeSelection>>,
    ) -> Result<ReadProbe, RuntimeError> {
        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
        crate::engine::api::profiling::record_owned_execution_event("property_storage_read_probe");
        enum Selected {
            Value(crate::engine::heap::RawValue),
            Getter(Option<ObjectId>),
            Missing(Option<ObjectId>),
        }
        let mut native_data = None;
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
            let selected = if let Some(index) = atom.immediate_integer()
                && let Some(value) = data.dense_array_value(index)
            {
                Selected::Value(value.clone())
            } else if !is_ordinary(data) && !is_array {
                return Ok(ReadProbe::Special(special_kind(data)));
            } else {
                match locate(&state, id, atom)? {
                    // Numeric misses may still select non-immediate dense indices.
                    // Named/symbol misses have ordinary prototype lookup and need no exotic call.
                    None if is_array && state.atoms.array_index(atom)?.is_some() => {
                        return Ok(ReadProbe::Special(SpecialKind::Other));
                    }
                    None => Selected::Missing(if own_only {
                        None
                    } else {
                        state.heap.shape(data.shape)?.prototype()
                    }),
                    Some(slot) => match &data.slots[slot.index] {
                        PropertySlot::Data(value) => Selected::Value(value.clone()),
                        PropertySlot::Accessor { get, .. } => Selected::Getter(*get),
                        PropertySlot::AutoInit(_) | PropertySlot::VarRef(_) => {
                            return Ok(ReadProbe::Special(SpecialKind::Other));
                        }
                    },
                }
            };
            #[cfg(feature = "stack-vm")]
            if native.is_some()
                && let Selected::Value(crate::engine::heap::RawValue::Object(id)) = &selected
            {
                // Invalid native metadata must still fail at the original Call,
                // not during this GetField. Such payloads simply get no hint.
                native_data = state.heap.object(*id).ok().and_then(|object| {
                    let ObjectPayload::NativeFunction { data, .. } = &object.payload else {
                        return None;
                    };
                    let realm = data.realm?;
                    (data.operation().is_some() && state.heap.context(realm).is_ok())
                        .then_some(*data)
                });
            }
            selected
        };
        Ok(match selected {
            Selected::Value(value) => {
                let value = self.root_raw_value(&value)?;
                if let (Some(output), Some(data), Value::Object(function)) =
                    (native.as_mut(), native_data, &value)
                {
                    **output = Some(LinkedNativeSelection {
                        runtime: self.clone(),
                        function: function.object_id(),
                        data,
                    });
                }
                #[cfg(all(feature = "profiling", feature = "stack-vm"))]
                crate::engine::api::profiling::record_owned_execution_event(match &value {
                    Value::Object(_) => "property_read_root_materialized.Object",
                    Value::Symbol(_) => "property_read_root_materialized.Symbol",
                    Value::String(_) => "property_read_root_materialized.String",
                    Value::BigInt(_) => "property_read_root_materialized.BigInt",
                    _ => "property_read_root_materialized.Immediate",
                });
                ReadProbe::Value(value)
            }
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

#[cfg(all(test, feature = "stack-vm"))]
mod dense_set_tests {
    use super::*;
    use crate::engine::object::operations::PropertySetAction;
    use crate::engine::object::ordinary::SetStep;

    #[test]
    fn existing_dense_set_preserves_hole_prototype_descriptor_and_receiver_rules() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let calls=0, seen, a=[1,2];
            const proto=Object.create(Array.prototype);
            Object.defineProperty(proto,'0',{set(v){calls++;seen=[this,v]},configurable:true});
            Object.setPrototypeOf(a,proto);
            a[0]=7;
            if(a[0]!==7 || calls!==0)return false;
            delete a[0]; a[0]=8;
            if(calls!==1 || seen[0]!==a || seen[1]!==8 || Object.hasOwn(a,'0'))return false;
            let b=[1]; Object.defineProperty(b,'length',{writable:false}); b[0]=9;
            if(b[0]!==9 || Reflect.set(b,'1',2))return false;
            Object.freeze(b); if(Reflect.set(b,'0',10) || b[0]!==9)return false;
            let c=[3]; Object.defineProperty(c,'0',{set(v){calls++;seen=v},get(){return 4}});
            c[0]=11; if(c[0]!==4 || seen!==11)return false;
            let d=[5], receiver={};
            if(!Reflect.set(d,'0',12,receiver) || d[0]!==5 || receiver[0]!==12)return false;
            let marker={}, trace='';
            let proxy=new Proxy({}, {getOwnPropertyDescriptor(){trace+='d';throw marker}});
            try{Reflect.set(d,'0',13,proxy);return false}catch(e){if(e!==marker)return false}
            return trace==='d' && d[0]===5;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn existing_dense_set_finishes_without_waiting_and_releases_last_owners() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let Value::Object(array) = context.eval("[{}]").unwrap() else {
            panic!("expected array")
        };
        let array_id = array.object_id();
        let key = runtime.intern_property_key("0").unwrap();
        let ReadProbe::Value(Value::Object(old)) =
            runtime.ordinary_read_probe(&array, &key).unwrap()
        else {
            panic!("expected old value")
        };
        let old_id = old.object_id();
        drop(old);
        let replacement = runtime.new_object(None).unwrap();
        let replacement_id = replacement.object_id();
        let released = runtime.new_object(None).unwrap();
        let released_id = released.object_id();
        let operation = runtime.operation();
        {
            let _borrow = runtime.0.state.borrow();
            drop(released);
        }
        assert!(runtime.0.deferred_references.has_pending());
        let action = SetStep::start_into(
            &runtime,
            Some(context.realm),
            array.clone(),
            key.clone(),
            Value::Object(replacement),
            Value::Object(array.clone()),
            |_| panic!("dense overwrite suspended"),
        )
        .unwrap();
        assert!(matches!(action, Some(PropertySetAction::Complete)));
        drop(operation);
        assert!(!runtime.0.deferred_references.has_pending());
        runtime.run_gc().unwrap();
        for id in [old_id, released_id] {
            assert!(runtime.0.state.borrow().heap.object(id).is_err());
        }
        assert!(runtime.0.state.borrow().heap.object(replacement_id).is_ok());
        let other = Runtime::new();
        let foreign = other.new_object(None).unwrap();
        assert!(matches!(
            SetStep::start_into(
                &runtime,
                Some(context.realm),
                array.clone(),
                key.clone(),
                Value::Object(foreign),
                Value::Object(array.clone()),
                |_| panic!("foreign write suspended")
            ),
            Err(RuntimeError::WrongRuntime(_))
        ));
        let ReadProbe::Value(Value::Object(current)) =
            runtime.ordinary_read_probe(&array, &key).unwrap()
        else {
            panic!("replacement disappeared")
        };
        assert_eq!(current.object_id(), replacement_id);
        drop(current);
        drop(array);
        drop(key);
        runtime.run_gc().unwrap();
        for id in [array_id, replacement_id] {
            assert!(runtime.0.state.borrow().heap.object(id).is_err());
        }
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }
}

#[cfg(feature = "stack-vm")]
fn immediate_value(raw: &crate::engine::heap::RawValue) -> Option<Value> {
    use crate::engine::heap::RawValue;
    Some(match raw {
        RawValue::Undefined => Value::Undefined,
        RawValue::Null => Value::Null,
        RawValue::Bool(value) => Value::Bool(*value),
        RawValue::Int(value) => Value::Int(*value),
        RawValue::Float(value) => Value::Float(*value),
        _ => return None,
    })
}

#[cfg(feature = "stack-vm")]
fn linked_field_atom(
    runtime: &Runtime,
    executable: &crate::engine::code::runtime::PublishedFunctionSnapshot,
    index: u32,
) -> Option<Atom> {
    if !executable.belongs_to(runtime) {
        return None;
    }
    executable
        .property_key_atoms
        .as_ref()?
        .get(index as usize)
        .copied()
        .filter(|atom| !atom.is_null())
}

impl Runtime {
    /// A published same-domain bytecode root owns this linked static atom.
    /// Own-slot classification and projection share the ordinary read kernel;
    /// every decline leaves input owners, lazy properties and prototypes alone.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn try_ordinary_field_immediate_read(
        &self,
        base: &Value,
        executable: &crate::engine::code::runtime::PublishedFunctionSnapshot,
        index: u32,
    ) -> Option<Value> {
        use crate::engine::heap::SlotReleaseReadiness;
        let atom = linked_field_atom(self, executable, index)?;
        if !matches!(
            self.slot_value_release_readiness(base),
            Ok(SlotReleaseReadiness::Ready)
        ) {
            return None;
        }
        let state = self.0.state.try_borrow().ok()?;
        if let Value::String(string) = base {
            let info = state.atoms.resolve(atom).ok()?;
            let crate::engine::atom::AtomSpelling::Text(name) = info.spelling else {
                return None;
            };
            return (info.kind == crate::engine::atom::AtomKind::String
                && name.len() == 6
                && name.utf16_units().eq("length".encode_utf16()))
            .then(|| Value::number(string.len() as f64));
        }
        let Value::Object(object) = base else {
            return None;
        };
        let id = object.object_id();
        let data = state.heap.object(id).ok()?;
        if matches!(
            (data.kind, &data.payload),
            (ObjectKind::Array, ObjectPayload::Array { .. })
        ) {
            let first = state.heap.shape(data.shape).ok()?.entries().first()?;
            if first.atom == atom {
                let (length, _) =
                    Self::array_length_state_in_heap(&state.heap, id, atom).ok()??;
                return Some(Self::array_length_value(length));
            }
            return None;
        }
        if !is_ordinary(data) {
            return None;
        }
        let slot = locate(&state, id, atom).ok()??;
        let PropertySlot::Data(value) = &data.slots[slot.index] else {
            return None;
        };
        immediate_value(value)
    }
    /// A published function already owns its static key. Only the selected
    /// result/getter is promoted here; fallback will acquire an owning key.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn prepare_linked_own_read(
        &self,
        base: &Value,
        executable: &crate::engine::code::runtime::PublishedFunctionSnapshot,
        index: u32,
    ) -> Result<Option<crate::engine::object::OrdinaryRead>, RuntimeError> {
        self.prepare_linked_own_read_selected(base, executable, index, None)
    }
    #[cfg(feature = "stack-vm")]
    pub(crate) fn prepare_linked_own_read_selected(
        &self,
        base: &Value,
        executable: &crate::engine::code::runtime::PublishedFunctionSnapshot,
        index: u32,
        native: Option<&mut Option<LinkedNativeSelection>>,
    ) -> Result<Option<crate::engine::object::OrdinaryRead>, RuntimeError> {
        let Some(atom) = linked_field_atom(self, executable, index) else {
            return Ok(None);
        };
        let Value::Object(object) = base else {
            return Ok(None);
        };
        let _operation = self.operation();
        self.validate_value_domain(base, "property receiver")?;
        Ok(
            match self.ordinary_read_probe_atom(object, atom, true, native)? {
                ReadProbe::Value(value) => {
                    Some(crate::engine::object::OrdinaryRead::Complete(Some(value)))
                }
                ReadProbe::Getter(None) => Some(crate::engine::object::OrdinaryRead::Complete(
                    Some(Value::Undefined),
                )),
                ReadProbe::Getter(Some(getter)) => {
                    let receiver = base.clone();
                    #[cfg(all(feature = "profiling", feature = "stack-vm"))]
                    crate::engine::api::profiling::record_owned_execution_event(
                        "linked_read_owner_clone.ReceiverObject",
                    );
                    Some(crate::engine::object::OrdinaryRead::Call { getter, receiver })
                }
                ReadProbe::Missing(_) | ReadProbe::Special(_) => None,
            },
        )
    }
    /// Only an existing writable own scalar slot reaches the ordinary Set
    /// replacement transaction. There are no callback or owner-bearing edges.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn try_ordinary_field_immediate_write(
        &self,
        base: &Value,
        executable: &crate::engine::code::runtime::PublishedFunctionSnapshot,
        index: u32,
        value: &Value,
    ) -> bool {
        use crate::engine::heap::SlotReleaseReadiness;
        if !matches!(
            value,
            Value::Undefined | Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_)
        ) {
            return false;
        }
        let Some(atom) = linked_field_atom(self, executable, index) else {
            return false;
        };
        let Value::Object(object) = base else {
            return false;
        };
        if !matches!(
            self.slot_value_release_readiness(base),
            Ok(SlotReleaseReadiness::Ready)
        ) {
            return false;
        }
        let Ok(mut state) = self.0.state.try_borrow_mut() else {
            return false;
        };
        let id = object.object_id();
        let Ok(data) = state.heap.object(id) else {
            return false;
        };
        if !is_ordinary(data) {
            return false;
        }
        let Ok(Some(slot)) = locate(&state, id, atom) else {
            return false;
        };
        if !slot.flags.writable {
            return false;
        }
        let PropertySlot::Data(old) = &data.slots[slot.index] else {
            return false;
        };
        if immediate_value(old).is_none() {
            return false;
        }
        let Ok(raw) = self.raw_property_value(value) else {
            return false;
        };
        // The canonical transaction validates shape storage before publication.
        // With scalar old/new values it retains/releases no edges or atoms;
        // the already-empty zero queue makes post-commit cleanup infallible.
        replace_data(&mut state, id, slot, PropertySlot::Data(raw)).is_ok()
    }
}

impl Runtime {
    /// Borrow an existing dense own immediate value while proving that the VM
    /// can subsequently release its base operand without draining heap work.
    /// Every decline leaves owners and storage untouched; the general property
    /// lookup retains all missing/exotic/reference-valued cases.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn try_dense_array_immediate_read(&self, base: &Value, index: u32) -> Option<Value> {
        self.try_array_immediate_read_kind(base, index, false)
    }

    /// One release proof and heap borrow select either existing array kernel.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn try_array_immediate_read(&self, base: &Value, index: u32) -> Option<Value> {
        self.try_array_immediate_read_kind(base, index, true)
    }

    #[cfg(feature = "stack-vm")]
    fn try_array_immediate_read_kind(
        &self,
        base: &Value,
        index: u32,
        include_typed: bool,
    ) -> Option<Value> {
        use crate::engine::heap::SlotReleaseReadiness;
        let Value::Object(object) = base else {
            return None;
        };
        if !matches!(
            self.slot_value_release_readiness(base),
            Ok(SlotReleaseReadiness::Ready)
        ) {
            return None;
        }
        let mut state = self.0.state.try_borrow_mut().ok()?;
        let data = state.heap.object(object.object_id()).ok()?;
        if matches!(
            (data.kind, &data.payload),
            (ObjectKind::Array, ObjectPayload::Array { .. })
        ) {
            return immediate_value(data.dense_array_value(index)?);
        }
        if !include_typed {
            return None;
        }
        if matches!(data.payload, ObjectPayload::Arguments { .. }) {
            let atom = Atom::from_immediate_integer(index)?;
            let slot = locate(&state, object.object_id(), atom).ok()??;
            return match &data.slots[slot.index] {
                PropertySlot::Data(value) => immediate_value(value),
                PropertySlot::VarRef(cell) => {
                    immediate_value(&state.heap.var_ref(*cell).ok()?.value)
                }
                PropertySlot::Accessor { .. } | PropertySlot::AutoInit(_) => None,
            };
        }
        let value =
            Self::typed_array_number_read_in_heap(&mut state.heap, object.object_id(), index)?;
        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
        crate::engine::api::profiling::record_owned_execution_event("typed_array_number_read_leaf");
        Some(value)
    }
}

#[cfg(all(test, feature = "stack-vm"))]
mod dense_array_read_tests {
    use super::*;

    fn receiver(runtime: &Runtime, expression: &str) -> Value {
        let mut context = runtime.new_context();
        let value = context.eval(expression).unwrap();
        drop(context);
        runtime.run_gc().unwrap();
        value
    }

    #[test]
    fn dense_array_read_leaf_reads_scalars_without_changing_owners() {
        let runtime = Runtime::new();
        let base = receiver(&runtime, "[undefined,null,true,17,-0,NaN]");
        let keep = base.clone();
        let Value::Object(object) = &base else {
            panic!("array");
        };
        let count = runtime
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id());
        for (index, expected) in [
            Value::Undefined,
            Value::Null,
            Value::Bool(true),
            Value::Int(17),
            Value::Float(-0.0),
        ]
        .iter()
        .enumerate()
        {
            let result = runtime
                .try_dense_array_immediate_read(&base, index as u32)
                .unwrap();
            assert!(result.same_quickjs_representation(expected));
        }
        assert!(
            matches!(runtime.try_dense_array_immediate_read(&base, 5), Some(Value::Float(v)) if v.is_nan())
        );
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(object.object_id()),
            count
        );
        drop(keep);
        assert!(runtime.try_dense_array_immediate_read(&base, 0).is_none());
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(object.object_id())
                .is_ok()
        );
    }

    #[test]
    fn dense_array_read_leaf_declines_missing_slow_reference_and_exotic_values() {
        for expression in [
            "[,1]",
            "Object.defineProperty([1], '0', {get(){throw 71}})",
            "Object.defineProperty([1], '0', {writable:false})",
            "[{}]",
            "['x']",
            "[1n]",
            "[Symbol()]",
            "new Proxy([1], {get(){throw 72}})",
            "new Uint8Array([1])",
            "({0:1,length:1})",
        ] {
            let runtime = Runtime::new();
            let base = receiver(&runtime, expression);
            let keep = base.clone();
            let Value::Object(object) = &base else {
                panic!("object");
            };
            let count = runtime
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(object.object_id());
            assert!(
                runtime.try_dense_array_immediate_read(&base, 0).is_none(),
                "{expression}"
            );
            assert_eq!(
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .object_strong_count(object.object_id()),
                count
            );
            assert!(
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .object(object.object_id())
                    .is_ok()
            );
            drop(keep);
        }
        let runtime = Runtime::new();
        let base = receiver(&runtime, "[1]");
        let _keep = base.clone();
        assert!(runtime.try_dense_array_immediate_read(&base, 1).is_none());
        assert!(
            runtime
                .try_dense_array_immediate_read(&base, u32::MAX)
                .is_none()
        );
        let other = Runtime::new();
        assert!(other.try_dense_array_immediate_read(&base, 0).is_none());
    }

    #[test]
    fn dense_array_read_leaf_keeps_frozen_materialized_values_on_canonical_path() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let base = context
            .eval("globalThis.frozenRead = Object.freeze([7, -0])")
            .unwrap();
        runtime.run_gc().unwrap();
        assert!(runtime.try_dense_array_immediate_read(&base, 0).is_none());
        assert_eq!(
            context
                .eval("frozenRead[0] === 7 && 1 / frozenRead[1] === -Infinity")
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn dense_array_read_leaf_does_not_drain_deferred_or_borrowed_state() {
        let runtime = Runtime::new();
        let base = receiver(&runtime, "[1]");
        let _keep = base.clone();
        {
            let _borrow = runtime.0.state.borrow();
            assert!(runtime.try_dense_array_immediate_read(&base, 0).is_none());
        }
        let released = runtime.new_object(None).unwrap();
        let released_id = released.object_id();
        {
            let _borrow = runtime.0.state.borrow();
            drop(released);
        }
        assert!(runtime.0.deferred_references.has_pending());
        assert!(runtime.try_dense_array_immediate_read(&base, 0).is_none());
        assert!(runtime.0.deferred_references.has_pending());
        assert!(runtime.0.state.borrow().heap.object(released_id).is_ok());
        runtime.run_gc().unwrap();
        assert!(matches!(
            runtime.try_dense_array_immediate_read(&base, 0),
            Some(Value::Int(1))
        ));
    }
}

#[cfg(all(test, feature = "stack-vm"))]
mod ordinary_field_leaf_tests {
    use super::*;
    use crate::engine::code::{bytecode::Instruction, runtime::PublishedFunctionSnapshot};

    fn executable(runtime: &Runtime, field: &str) -> (PublishedFunctionSnapshot, u32) {
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(
                context
                    .eval(&format!("(function(o,v){{o.{field}=v;return o.{field}}})"))
                    .unwrap(),
            )
            .unwrap();
        let crate::engine::vm::call::CallableExecution::Bytecode { bytecode, .. } =
            runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("bytecode")
        };
        let executable = runtime.snapshot_function_bytecode(&bytecode).unwrap();
        let index = executable
            .code
            .iter()
            .find_map(|op| match op {
                Instruction::GetField(index) => Some(*index),
                _ => None,
            })
            .unwrap();
        (executable, index)
    }

    #[test]
    fn ordinary_field_leaf_preserves_scalar_values_flags_and_declines_nonlocal_rules() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (executable, index) = executable(&runtime, "x");
        for source in [
            "({x:undefined})",
            "({x:null})",
            "({x:true})",
            "({x:42})",
            "({x:1.5})",
            "({x:-0})",
        ] {
            let base = context.eval(source).unwrap();
            let _retained = base.clone();
            assert!(
                runtime
                    .try_ordinary_field_immediate_read(&base, &executable, index)
                    .is_some()
            );
            assert!(runtime.try_ordinary_field_immediate_write(
                &base,
                &executable,
                index,
                &Value::Int(17)
            ));
            assert_eq!(
                runtime.try_ordinary_field_immediate_read(&base, &executable, index),
                Some(Value::Int(17))
            );
        }
        for source in [
            "({get x(){throw 42}})",
            "Object.create({x:42})",
            "new Proxy({x:42},{get(){throw 42},set(){throw 42}})",
            "({x:{}})",
            "({x:'text'})",
            "({x:42n})",
            "({x:Symbol()})",
            "([])",
            "new Uint8Array(1)",
            "globalThis",
        ] {
            let base = context.eval(source).unwrap();
            let _retained = base.clone();
            assert!(
                runtime
                    .try_ordinary_field_immediate_read(&base, &executable, index)
                    .is_none(),
                "{source}"
            );
            assert!(
                !runtime.try_ordinary_field_immediate_write(
                    &base,
                    &executable,
                    index,
                    &Value::Int(17)
                ),
                "{source}"
            );
        }
        for source in [
            "Object.freeze({x:42})",
            "Object.defineProperty({},'x',{value:42,writable:false})",
        ] {
            let base = context.eval(source).unwrap();
            let _retained = base.clone();
            assert_eq!(
                runtime.try_ordinary_field_immediate_read(&base, &executable, index),
                Some(Value::Int(42))
            );
            assert!(!runtime.try_ordinary_field_immediate_write(
                &base,
                &executable,
                index,
                &Value::Int(17)
            ));
            assert_eq!(
                runtime.try_ordinary_field_immediate_read(&base, &executable, index),
                Some(Value::Int(42))
            );
        }
    }

    #[test]
    fn ordinary_field_leaf_checks_published_atom_domain_and_release_guards() {
        let runtime = Runtime::new();
        let foreign = Runtime::new();
        let mut context = runtime.new_context();
        let (code, index) = executable(&runtime, "x");
        let (foreign_code, foreign_index) = executable(&foreign, "x");
        let base = context.eval("({x:42})").unwrap();
        assert!(
            runtime
                .try_ordinary_field_immediate_read(&base, &code, index)
                .is_none()
        );
        assert!(!runtime.try_ordinary_field_immediate_write(&base, &code, index, &Value::Int(17)));
        let _retained = base.clone();
        assert!(
            runtime
                .try_ordinary_field_immediate_read(&base, &foreign_code, foreign_index)
                .is_none()
        );
        assert!(!runtime.try_ordinary_field_immediate_write(
            &base,
            &foreign_code,
            foreign_index,
            &Value::Int(17)
        ));
        assert!(
            runtime
                .try_ordinary_field_immediate_read(&base, &code, u32::MAX)
                .is_none()
        );
        let unrooted = PublishedFunctionSnapshot::empty_for_test(context.realm);
        assert!(
            runtime
                .try_ordinary_field_immediate_read(&base, &unrooted, 0)
                .is_none()
        );
        let queued = runtime.new_object(None).unwrap();
        {
            let _state = runtime.0.state.borrow();
            assert!(
                runtime
                    .try_ordinary_field_immediate_read(&base, &code, index)
                    .is_none()
            );
            assert!(!runtime.try_ordinary_field_immediate_write(
                &base,
                &code,
                index,
                &Value::Int(17)
            ));
            drop(queued);
        }
        assert!(
            runtime
                .try_ordinary_field_immediate_read(&base, &code, index)
                .is_none()
        );
        assert!(!runtime.try_ordinary_field_immediate_write(&base, &code, index, &Value::Int(17)));
        assert!(runtime.0.deferred_references.has_pending());
        runtime.drain_deferred_references().unwrap();
        assert_eq!(
            runtime.try_ordinary_field_immediate_read(&base, &code, index),
            Some(Value::Int(42))
        );
        let object_value = Value::Object(runtime.new_object(None).unwrap());
        assert!(!runtime.try_ordinary_field_immediate_write(&base, &code, index, &object_value));
        assert_eq!(
            runtime.try_ordinary_field_immediate_read(&base, &code, index),
            Some(Value::Int(42))
        );
        let (lazy_code, lazy_index) = executable(&runtime, "min");
        let math = context.eval("Math").unwrap();
        assert!(
            runtime
                .try_ordinary_field_immediate_read(&math, &lazy_code, lazy_index)
                .is_none()
        );
        assert!(!runtime.try_ordinary_field_immediate_write(
            &math,
            &lazy_code,
            lazy_index,
            &Value::Int(17)
        ));
        assert!(matches!(
            context.eval("typeof Math.min").unwrap(),
            Value::String(_)
        ));
    }
    #[test]
    fn recovery_length_leaf_preserves_utf16_brand_and_final_owner_boundaries() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (code, index) = executable(&runtime, "length");
        for (source, expected) in [("[1,,3]", 3), ("'a\\ud83d\\ude00'", 3)] {
            let base = context.eval(source).unwrap();
            let retained = base.clone();
            assert_eq!(
                runtime.try_ordinary_field_immediate_read(&base, &code, index),
                Some(Value::Int(expected))
            );
            drop(retained);
        }
        let proxy = context.eval("new Proxy([], {get(){throw 91}})").unwrap();
        let _retained = proxy.clone();
        assert!(
            runtime
                .try_ordinary_field_immediate_read(&proxy, &code, index)
                .is_none()
        );
        let unique = Value::String(crate::engine::value::JsString::from_owned_utf16(vec![
            97, 0xd800,
        ]));
        assert!(
            runtime
                .try_ordinary_field_immediate_read(&unique, &code, index)
                .is_none()
        );
        let retained = unique.clone();
        assert_eq!(
            runtime.try_ordinary_field_immediate_read(&unique, &code, index),
            Some(Value::Int(2))
        );
        drop(retained);
    }

    #[test]
    fn recovery_arguments_leaf_reads_current_cell_and_declines_redefinitions() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let base = context
            .eval(
                "globalThis.args=(function(a){globalThis.change=x=>a=x;return arguments})(1);args",
            )
            .unwrap();
        let _retained = base.clone();
        assert_eq!(
            runtime.try_array_immediate_read(&base, 0),
            Some(Value::Int(1))
        );
        context.eval("change(7)").unwrap();
        assert_eq!(
            runtime.try_array_immediate_read(&base, 0),
            Some(Value::Int(7))
        );
        context
            .eval("Object.defineProperty(args,'0',{value:8,writable:false});change(9)")
            .unwrap();
        assert_eq!(
            runtime.try_array_immediate_read(&base, 0),
            Some(Value::Int(8))
        );
        context
            .eval("Object.defineProperty(args,'0',{get(){return 11},configurable:true})")
            .unwrap();
        assert!(runtime.try_array_immediate_read(&base, 0).is_none());
        assert_eq!(context.eval("args[0]").unwrap(), Value::Int(11));
        context.eval("delete args[0]").unwrap();
        assert!(runtime.try_array_immediate_read(&base, 0).is_none());
    }

    #[test]
    fn linked_native_fact_is_bound_to_the_selected_callee_not_a_property_cache() {
        use crate::engine::builtins::native::{MathMinMaxKind, NativeFunctionId};
        use crate::engine::object::OrdinaryRead;
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (code, index) = executable(&runtime, "x");
        let base = context
            .eval("globalThis.selectedNative={x:Math.min};selectedNative")
            .unwrap();
        let mut fact = None;
        let Some(OrdinaryRead::Complete(Some(Value::Object(callee)))) = runtime
            .prepare_linked_own_read_selected(&base, &code, index, Some(&mut fact))
            .unwrap()
        else {
            panic!("own native")
        };
        context.eval("selectedNative.x=Math.max").unwrap();
        let data = fact.take().unwrap().into_parts(&callee).unwrap();
        assert_eq!(
            data.target,
            NativeFunctionId::MathMinMax(MathMinMaxKind::Min)
        );
        let _new_read = runtime
            .prepare_linked_own_read_selected(&base, &code, index, Some(&mut fact))
            .unwrap();
        assert!(fact.take().unwrap().into_parts(&callee).is_none());
        let _new_read = runtime
            .prepare_linked_own_read_selected(&base, &code, index, Some(&mut fact))
            .unwrap();
        let foreign = Runtime::new();
        let mut foreign_context = foreign.new_context();
        let Value::Object(foreign_callee) = foreign_context.eval("Math.max").unwrap() else {
            panic!("native")
        };
        assert!(fact.take().unwrap().into_parts(&foreign_callee).is_none());
    }

    #[test]
    fn recovery_linked_read_keeps_the_selected_getter_and_return_owner() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (code, index) = executable(&runtime, "x");
        let base = context
            .eval("globalThis.readLog=0;globalThis.o={get x(){readLog++;return 7}};o")
            .unwrap();
        let read = runtime
            .prepare_linked_own_read(&base, &code, index)
            .unwrap()
            .unwrap();
        assert_eq!(context.eval("readLog").unwrap(), Value::Int(0));
        context
            .eval("Object.defineProperty(o,'x',{get(){throw 99}})")
            .unwrap();
        let key = PropertyKey::from_borrowed_atom(
            runtime.clone(),
            code.property_key_atoms.as_ref().unwrap()[index as usize],
        )
        .unwrap();
        let result = runtime
            .finish_prepared_read(context.realm, &key, read)
            .unwrap();
        assert!(matches!(
            result,
            crate::engine::value::conversion::NativeConversion::Value(Some(Value::Int(7)))
        ));
        assert_eq!(context.eval("readLog").unwrap(), Value::Int(1));
    }
}
