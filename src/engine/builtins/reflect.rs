//! Pinned QuickJS `Reflect` intrinsic algorithms.
//!
//! The implementation follows the 2026-06-04 `js_reflect_funcs` table and
//! intentionally retains QuickJS's validation order where it differs from a
//! tempting shared helper. In particular, `Reflect.construct` validates an
//! explicit `newTarget`, then materializes the argument list, and only then
//! validates the target constructor.

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{NativeFunctionId, ReflectKind};
use crate::engine::heap::{AutoInitProperty, ContextId, HeapError, ObjectPayload, PropertySlot};
use crate::engine::object::shape::PropertyFlags;
use crate::engine::object::{
    DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};

#[cfg(test)]
mod tests;

const MAX_APPLY_ARGUMENTS: u64 = 65_534;

impl Runtime {
    /// Snapshot QuickJS's fast Array/Arguments storage in numeric-index order.
    ///
    /// Arrays expose their physical dense payload directly. Arguments retain
    /// their distinct mapped/unmapped shape-slot representation and are
    /// reconstructed without observable property lookup. The owning object
    /// stays rooted while raw values are promoted to public roots.
    pub(crate) fn fast_array_like_values(
        &self,
        object: &ObjectRef,
        expected_len: u32,
    ) -> Result<Option<Vec<Value>>, RuntimeError> {
        let raw_values = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            if let ObjectPayload::Array { dense } = &object_data.payload {
                let Some(dense) = dense else {
                    return Ok(None);
                };
                if dense.len() != expected_len as usize {
                    return Ok(None);
                }
                let mut values = Vec::new();
                values
                    .try_reserve_exact(dense.len())
                    .map_err(|_| HeapError::Allocation {
                        operation: "snapshotting fast Array values",
                    })?;
                values.extend_from_slice(dense);
                #[cfg(feature = "profiling")]
                {
                    crate::engine::api::profiling::record_call_buffer_capacity(
                        "arguments.fast_raw",
                        0,
                        values.capacity(),
                        size_of::<crate::engine::heap::RawValue>(),
                    );
                    crate::engine::api::profiling::record_call_buffer_initialized(
                        "arguments.fast_raw",
                        values.len(),
                    );
                }
                values
            } else {
                let (mapped, fast_len) = match &object_data.payload {
                    ObjectPayload::Arguments { mapped, fast_len } => (*mapped, *fast_len),
                    _ => return Ok(None),
                };
                if fast_len != Some(expected_len) {
                    return Ok(None);
                }
                let shape = state.heap.shape(object_data.shape)?;
                let capacity = usize::try_from(expected_len).map_err(|_| {
                    RuntimeError::Invariant("fast argument length does not fit usize")
                })?;
                let mut ordered = vec![None; capacity];
                #[cfg(feature = "profiling")]
                {
                    crate::engine::api::profiling::record_call_buffer_capacity(
                        "arguments.fast_ordering",
                        0,
                        ordered.capacity(),
                        size_of::<Option<crate::engine::heap::RawValue>>(),
                    );
                    crate::engine::api::profiling::record_call_buffer_initialized(
                        "arguments.fast_ordering",
                        ordered.len(),
                    );
                }
                for (entry, slot) in shape.entries().iter().zip(&object_data.slots) {
                    let Some(index) = state.atoms.array_index(entry.atom)? else {
                        continue;
                    };
                    if index >= expected_len {
                        continue;
                    }
                    if !entry.flags.writable || !entry.flags.enumerable || !entry.flags.configurable
                    {
                        return Err(RuntimeError::Invariant(
                            "fast argument index is not a C/W/E property",
                        ));
                    }
                    let value = match slot {
                        PropertySlot::VarRef(var_ref) if mapped => {
                            state.heap.var_ref(*var_ref)?.value.clone()
                        }
                        PropertySlot::Data(value) if !mapped => value.clone(),
                        _ => {
                            return Err(RuntimeError::Invariant(
                                "fast argument index has the wrong storage kind",
                            ));
                        }
                    };
                    let destination = ordered
                        .get_mut(usize::try_from(index).map_err(|_| {
                            RuntimeError::Invariant("fast argument index does not fit usize")
                        })?)
                        .ok_or(RuntimeError::Invariant(
                            "fast argument index escaped its dense prefix",
                        ))?;
                    if destination.replace(value).is_some() {
                        return Err(RuntimeError::Invariant(
                            "fast argument prefix contains a duplicate index",
                        ));
                    }
                }
                ordered
                    .into_iter()
                    .map(|value| {
                        value.ok_or(RuntimeError::Invariant(
                            "fast argument prefix is missing an indexed value",
                        ))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
        };

        #[cfg(feature = "profiling")]
        {
            // The Arguments ordering collect may reuse its input allocation;
            // observe this raw buffer without inventing a second allocation.
            crate::engine::api::profiling::record_call_buffer_observed(
                "arguments.fast_raw",
                raw_values.capacity(),
                size_of::<crate::engine::heap::RawValue>(),
            );
            crate::engine::api::profiling::record_call_raw_buffer_copies(
                "arguments.fast_raw",
                &raw_values,
            );
        }
        let values = raw_values
            .iter()
            .map(|value| self.root_raw_value(value))
            .collect::<Result<Vec<_>, _>>()?;
        #[cfg(feature = "profiling")]
        {
            // This iterator borrows raw_values and cannot reuse its backing Vec.
            crate::engine::api::profiling::record_call_buffer_observed(
                "arguments.fast_rooted",
                values.capacity(),
                size_of::<Value>(),
            );
            crate::engine::api::profiling::record_call_buffer_copies(
                "arguments.fast_rooted",
                &values,
            );
        }
        Ok(Some(values))
    }

    /// Install the global `Reflect` `JS_OBJECT_DEF` equivalent. The object is
    /// realm-owned and remains lazy until the global slot is first read.
    pub(crate) fn initialize_reflect_intrinsic(
        &self,
        realm: ContextId,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key("Reflect")?;
        self.store_property_slot(
            global_object,
            &key,
            PropertyFlags::data(true, false, true),
            PropertySlot::AutoInit(AutoInitProperty::Reflect { realm }),
        )
    }

    /// Materialize pinned QuickJS's complete `js_reflect_funcs` table in its
    /// defining realm. Each method remains an AutoInit native property.
    pub(crate) fn instantiate_reflect_intrinsic(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let reflect = self.new_ordinary_object_in_realm(realm)?;
        for (kind, name, length) in [
            (ReflectKind::Apply, "apply", 3),
            (ReflectKind::Construct, "construct", 2),
            (ReflectKind::DefineProperty, "defineProperty", 3),
            (ReflectKind::DeleteProperty, "deleteProperty", 2),
            (ReflectKind::Get, "get", 2),
            (
                ReflectKind::GetOwnPropertyDescriptor,
                "getOwnPropertyDescriptor",
                2,
            ),
            (ReflectKind::GetPrototypeOf, "getPrototypeOf", 1),
            (ReflectKind::Has, "has", 2),
            (ReflectKind::IsExtensible, "isExtensible", 1),
            (ReflectKind::OwnKeys, "ownKeys", 1),
            (ReflectKind::PreventExtensions, "preventExtensions", 1),
            (ReflectKind::Set, "set", 3),
            (ReflectKind::SetPrototypeOf, "setPrototypeOf", 2),
        ] {
            self.define_native_builtin_auto_init(
                &reflect,
                realm,
                NativeFunctionId::Reflect(kind),
                name,
                length,
                length,
            )?;
        }

        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            &reflect,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("Reflect"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Reflect toStringTag definition was rejected",
            ));
        }
        Ok(reflect)
    }

    /// QuickJS `build_arg_list`, shared by Function.prototype.apply and the
    /// two Reflect call/construct paths. Nullish exceptions remain a caller
    /// decision: this kernel always requires an object, as upstream does once
    /// it has entered `build_arg_list`.
    /// Classify an Array argument carrier without invoking length or index getters.
    /// None leaves the caller free to choose an explicit property-reading protocol.
    pub(crate) fn prepare_fast_array_arguments(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<Option<NativeConversion<Vec<Value>>>, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        let array = {
            let state = self.0.state.borrow();
            let data = state.heap.object(object.object_id())?;
            matches!(
                (data.kind, &data.payload),
                (
                    crate::engine::heap::ObjectKind::Array,
                    ObjectPayload::Array { .. }
                )
            )
        };
        if !array {
            return Ok(None);
        }
        let key = self.intern_property_key("length")?;
        let Some(crate::engine::object::CompleteOrdinaryPropertyDescriptor::Data { value, .. }) =
            self.get_own_property(object, &key)?
        else {
            return Err(RuntimeError::Invariant(
                "Array argument carrier has no data length",
            ));
        };
        let length = match value {
            Value::Int(value) if value >= 0 => u64::try_from(value).unwrap(),
            Value::Float(value)
                if value >= 0.0 && value <= f64::from(u32::MAX) && value.fract() == 0.0 =>
            {
                value as u64
            }
            _ => {
                return Err(RuntimeError::Invariant(
                    "Array argument carrier has an invalid length",
                ));
            }
        };
        if length > MAX_APPLY_ARGUMENTS {
            return Ok(Some(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "too many arguments in function call (only 65534 allowed)",
            )?)));
        }
        self.fast_array_like_values(object, length as u32)
            .map(|values| values.map(NativeConversion::Value))
    }

    pub(crate) fn build_array_like_argument_list(
        &self,
        realm: ContextId,
        array_argument: &Value,
    ) -> Result<NativeConversion<Vec<Value>>, RuntimeError> {
        super::function::arguments::finish(
            self,
            realm,
            super::function::arguments::ArgumentsStep::start(self, realm, array_argument.clone())?,
        )
    }

    pub(crate) fn call_reflect(
        &self,
        realm: ContextId,
        kind: ReflectKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Reflect method did not receive a generic invocation",
            ));
        };
        match kind {
            ReflectKind::Apply => self.call_reflect_apply(realm, arguments),
            ReflectKind::Construct => self.call_reflect_construct(realm, arguments),
            ReflectKind::DefineProperty => self.call_reflect_define_property(realm, arguments),
            ReflectKind::DeleteProperty => self.call_reflect_delete_property(realm, arguments),
            ReflectKind::Get => self.call_reflect_get(realm, arguments),
            ReflectKind::GetOwnPropertyDescriptor => {
                self.call_reflect_get_own_property_descriptor(realm, arguments)
            }
            ReflectKind::GetPrototypeOf => self.call_reflect_get_prototype_of(realm, arguments),
            ReflectKind::Has => self.call_reflect_has(realm, arguments),
            ReflectKind::IsExtensible => self.call_reflect_is_extensible(realm, arguments),
            ReflectKind::OwnKeys => self.call_reflect_own_keys(realm, arguments),
            ReflectKind::PreventExtensions => {
                self.call_reflect_prevent_extensions(realm, arguments)
            }
            ReflectKind::Set => self.call_reflect_set(realm, arguments),
            ReflectKind::SetPrototypeOf => self.call_reflect_set_prototype_of(realm, arguments),
        }
    }

    fn call_reflect_apply(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        super::function::invoke::finish(
            self,
            realm,
            super::function::invoke::InvokeStep::start(
                self,
                realm,
                super::function::invoke::InvokeKind::ReflectApply,
                &NativeInvocation::Call {
                    this_value: Value::Undefined,
                },
                arguments,
            )?,
        )
    }

    fn call_reflect_construct(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        super::function::invoke::finish(
            self,
            realm,
            super::function::invoke::InvokeStep::start(
                self,
                realm,
                super::function::invoke::InvokeKind::ReflectConstruct,
                &NativeInvocation::Call {
                    this_value: Value::Undefined,
                },
                arguments,
            )?,
        )
    }

    fn call_reflect_define_property(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        use super::object::property::{self, PropertyKind, PropertyStep};
        property::finish(
            self,
            realm,
            PropertyStep::start(self, realm, PropertyKind::Define, arguments)?,
        )
    }

    fn call_reflect_delete_property(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        use super::object::property::{self, PropertyKind, PropertyStep};
        property::finish(
            self,
            realm,
            PropertyStep::start(self, realm, PropertyKind::Delete, arguments)?,
        )
    }

    fn call_reflect_get(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        use super::object::property::{self, PropertyKind, PropertyStep};
        property::finish(
            self,
            realm,
            PropertyStep::start(self, realm, PropertyKind::Get, arguments)?,
        )
    }

    fn call_reflect_get_own_property_descriptor(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        use super::object::property::{self, PropertyKind, PropertyStep};
        property::finish(
            self,
            realm,
            PropertyStep::start(self, realm, PropertyKind::Descriptor, arguments)?,
        )
    }

    fn call_reflect_get_prototype_of(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        super::object::prototype::finish(
            self,
            realm,
            super::object::prototype::BuiltinPrototypeStep::start(
                self,
                realm,
                super::object::prototype::BuiltinPrototypeKind::ReflectGet,
                arguments,
            )?,
        )
    }

    fn call_reflect_has(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        use super::object::property::{self, PropertyKind, PropertyStep};
        property::finish(
            self,
            realm,
            PropertyStep::start(self, realm, PropertyKind::Has, arguments)?,
        )
    }

    fn call_reflect_is_extensible(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        use super::object::property::{self, PropertyKind, PropertyStep};
        property::finish(
            self,
            realm,
            PropertyStep::start(self, realm, PropertyKind::Extensible, arguments)?,
        )
    }

    fn call_reflect_own_keys(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        use super::object::property::{self, PropertyKind, PropertyStep};
        property::finish(
            self,
            realm,
            PropertyStep::start(self, realm, PropertyKind::Keys, arguments)?,
        )
    }

    fn call_reflect_prevent_extensions(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        use super::object::property::{self, PropertyKind, PropertyStep};
        property::finish(
            self,
            realm,
            PropertyStep::start(self, realm, PropertyKind::Prevent, arguments)?,
        )
    }

    fn call_reflect_set(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        use super::object::property::{self, PropertyKind, PropertyStep};
        property::finish(
            self,
            realm,
            PropertyStep::start(self, realm, PropertyKind::Set, arguments)?,
        )
    }

    fn call_reflect_set_prototype_of(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        super::object::prototype::finish(
            self,
            realm,
            super::object::prototype::BuiltinPrototypeStep::start(
                self,
                realm,
                super::object::prototype::BuiltinPrototypeKind::ReflectSet,
                arguments,
            )?,
        )
    }
}

#[cfg(test)]
mod argument_preparation_tests {
    use super::*;

    #[test]
    fn array_argument_preflight_keeps_getters_out_of_the_fast_path() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for source in [
            "({get length(){throw 99}})",
            "Object.defineProperty([1],'0',{get(){throw 99}})",
            "Object.assign(Object.create({get 0(){throw 99}}),{length:1})",
        ] {
            let Value::Object(object) = context.eval(source).unwrap() else {
                panic!("expected carrier")
            };
            assert!(
                runtime
                    .prepare_fast_array_arguments(context.realm, &object)
                    .unwrap()
                    .is_none(),
                "{source}"
            );
        }
        let Value::Object(object) = context.eval("[40,2]").unwrap() else {
            panic!("expected array")
        };
        let Some(NativeConversion::Value(values)) = runtime
            .prepare_fast_array_arguments(context.realm, &object)
            .unwrap()
        else {
            panic!("expected snapshot")
        };
        assert_eq!(values, vec![Value::Int(40), Value::Int(2)]);
        let Value::Object(oversized) = context.eval("Array(65535)").unwrap() else {
            panic!("expected array")
        };
        assert!(matches!(
            runtime
                .prepare_fast_array_arguments(context.realm, &oversized)
                .unwrap(),
            Some(NativeConversion::Throw(_))
        ));
    }
}
