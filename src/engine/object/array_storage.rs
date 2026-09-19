//! Batched sparse Array storage changes after observable length conversion.
//!
//! Only genuine Arrays reach this path. It runs no JavaScript: accessor values
//! are removed without being read, and layout publication owns retain/release.
//! Length validation, conversion and rollback stay in the property algorithm.

use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::heap::ObjectPayload;
use crate::engine::object::ObjectRef;

impl Runtime {
    /// Remove configurable sparse indices in descending-delete semantics with
    /// one layout replacement. Return the highest blocking index, if any.
    ///
    /// Preparation is linear in layout width; publication retains the surviving
    /// slots before releasing the old layout. No per-index layout is published.
    pub(super) fn truncate_sparse_array_indices(
        &self,
        object: &ObjectRef,
        minimum: u32,
    ) -> Result<Option<u32>, RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let object_id = object.object_id();
        let (prototype, entries, slots, blocker) = {
            let data = state.heap.object(object_id)?;
            if !matches!(data.payload, ObjectPayload::Array { .. }) {
                return Err(RuntimeError::Invariant(
                    "sparse Array truncation reached a non-Array object",
                ));
            }
            let shape = state.heap.shape(data.shape)?;
            if shape.entries().len() != data.slots.len() {
                return Err(RuntimeError::Invariant(
                    "Array shape and value slots have different lengths",
                ));
            }
            let indices = shape
                .entries()
                .iter()
                .map(|entry| state.atoms.array_index(state.atoms.brand(entry.atom)?))
                .collect::<Result<Vec<_>, _>>()?;
            // Descending deletion stops at the highest non-configurable index.
            // Everything above it is removed; nothing at or below it is touched.
            let blocker = shape
                .entries()
                .iter()
                .zip(&indices)
                .filter_map(|(entry, index)| {
                    index.filter(|index| *index >= minimum && !entry.flags.configurable)
                })
                .max();
            let remove = |index: Option<u32>| {
                index.is_some_and(|index| {
                    index >= minimum && blocker.is_none_or(|blocked| index > blocked)
                })
            };
            let removed = indices.iter().filter(|&&index| remove(index)).count();
            if removed == 0 {
                return Ok(blocker);
            }
            let survivors = data.slots.len() - removed;
            let mut entries = Vec::with_capacity(survivors);
            let mut slots = Vec::with_capacity(survivors);
            for ((entry, slot), index) in shape.entries().iter().zip(&data.slots).zip(indices) {
                if !remove(index) {
                    entries.push(*entry);
                    slots.push(slot.clone());
                }
            }
            (shape.prototype(), entries, slots, blocker)
        };
        state.replace_layout(object_id, prototype, &entries, slots)?;
        Ok(blocker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::value::{JsString, Value};
    #[test]
    fn sparse_truncation_preserves_highest_blocker_named_order_and_accessor_silence() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context.eval(r#"(() => {
        const a = [];
        a.first = 1;
        a[200] = 200;
        Object.defineProperty(a, '120', { get() { throw Error('getter called'); }, configurable: true });
        Object.defineProperty(a, '90', { value: 90, configurable: false });
        Object.defineProperty(a, '40', { value: 40, configurable: false });
        a[10] = 10;
        a.last = 2;
        const ok = Reflect.defineProperty(a, 'length', { value: 5, writable: false });
        return [ok, a.length, Object.getOwnPropertyDescriptor(a, 'length').writable,
            Reflect.ownKeys(a).join(','), a[90], a[40]].join('|');
    })()"#).unwrap();
        assert_eq!(
            result,
            Value::String(JsString::from_static(
                "false|91|false|10,40,90,length,first,last|90|40"
            ))
        );
    }
}
