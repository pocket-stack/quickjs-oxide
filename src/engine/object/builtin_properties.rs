//! Batch installation of named lazy builtin methods at explicit bootstrap boundaries.

use super::ObjectRef;
use super::shape::{PropertyFlags, PropertyStorageKind, ShapeEntry};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::heap::{AutoInitProperty, ContextId, HeapError, ObjectPayload, PropertySlot};
use std::collections::HashSet;

/// One named lazy method, keeping its descriptor and callable metadata together.
/// Symbol keys and alias materialization retain their explicit single-property paths.
#[derive(Clone, Copy)]
pub(crate) struct NativeBuiltinProperty {
    pub(crate) target: NativeFunctionId,
    pub(crate) name: &'static str,
    pub(crate) length: u8,
    pub(crate) min_readable_args: u8,
    pub(crate) flags: PropertyFlags,
}

impl NativeBuiltinProperty {
    pub(crate) const fn new(
        target: NativeFunctionId,
        name: &'static str,
        length: u8,
        min_readable_args: u8,
    ) -> Self {
        Self {
            target,
            name,
            length,
            min_readable_args,
            flags: PropertyFlags::data(true, false, true),
        }
    }
}

impl Runtime {
    /// Append an entire table without publishing intermediate layouts.
    ///
    /// This is a bootstrap primitive, not general [[DefineOwnProperty]]. It
    /// accepts only extensible ordinary/native-function/Array receivers and
    /// new non-index named data properties. No descriptor invokes user code.
    /// Validation failures leave the receiver unchanged. Publication and any
    /// subsequent invariant errors retain `replace_layout`'s existing contract.
    pub(crate) fn define_native_builtin_auto_init_batch(
        &self,
        object: &ObjectRef,
        realm: ContextId,
        methods: impl IntoIterator<Item = NativeBuiltinProperty>,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        // Consume descriptors and intern their keys before borrowing Runtime
        // state. These owning keys outlive the state borrow on every exit.
        let properties = methods
            .into_iter()
            .map(|method| {
                self.intern_property_key(method.name)
                    .map(|key| (key, method))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if properties.is_empty() {
            return Ok(());
        }
        self.validate_object_and_key(object, &properties[0].0)?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = object.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            if !object.extensible
                || !matches!(
                    object.payload,
                    ObjectPayload::Ordinary
                        | ObjectPayload::NativeFunction { .. }
                        | ObjectPayload::Array { .. }
                )
            {
                return Err(RuntimeError::Invariant("invalid builtin batch receiver"));
            }
            let shape = state.heap.shape(object.shape)?;
            let mut seen = HashSet::with_capacity(properties.len());
            for (key, method) in &properties {
                if method.flags.storage != PropertyStorageKind::Data
                    || state.atoms.array_index(key.atom())?.is_some()
                    || shape.find(key.atom()).is_some()
                    || !seen.insert(key.atom())
                {
                    return Err(RuntimeError::Invariant(
                        "invalid or duplicate builtin batch property",
                    ));
                }
            }
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries
            .try_reserve(properties.len())
            .map_err(|_| HeapError::Allocation {
                operation: "preparing builtin batch shape entries",
            })?;
        slots
            .try_reserve(properties.len())
            .map_err(|_| HeapError::Allocation {
                operation: "preparing builtin batch property slots",
            })?;
        for (key, method) in &properties {
            entries.push(ShapeEntry {
                atom: key.atom(),
                flags: method.flags,
            });
            slots.push(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
                realm,
                target: method.target,
                name: method.name,
                length: method.length,
                min_readable_args: method.min_readable_args,
            }));
        }
        state.replace_layout(object_id, prototype, &entries, slots)
    }
}
