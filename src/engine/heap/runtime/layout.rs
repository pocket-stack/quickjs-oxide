//! Publication of a prepared object layout. Language-specific builders own
//! property order and descriptors; this boundary owns atom rollback and heap
//! publication. Raw slots are borrowed from roots kept alive by the caller.

use super::*;
use crate::engine::heap::ObjectData;

impl RuntimeState {
    pub(crate) fn allocate_object_with_layout(
        &mut self,
        prototype: Option<ObjectId>,
        entries: &[ShapeEntry],
        slots: Vec<PropertySlot>,
        build: impl FnOnce(ShapeId, Vec<PropertySlot>) -> ObjectData,
    ) -> Result<ObjectId, RuntimeError> {
        let shape = self.get_or_create_shape(prototype, entries)?;
        let atoms = match self.retain_slot_atoms(&slots) {
            Ok(atoms) => atoms,
            Err(error) => {
                let cleanup = self.heap.release_shape(shape)?;
                self.apply_cleanup(cleanup)?;
                return Err(error);
            }
        };
        let result = self.heap.allocate_object(build(shape, slots));
        if result.is_err() {
            self.release_atoms(atoms)?;
        }
        let cleanup = self.heap.release_shape(shape)?;
        self.apply_cleanup(cleanup)?;
        result.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::api::runtime::Runtime;
    use crate::engine::object::shape::PropertyFlags;

    #[test]
    fn prepared_layout_failure_rolls_back_shape_and_slot_atom_ownership() {
        let runtime = Runtime::new();
        let key = runtime.intern_property_key("prepared").unwrap();
        let symbol = runtime.new_symbol(None).unwrap();
        let mut state = runtime.0.state.borrow_mut();
        let before_key = state.atoms.resolve(key.atom()).unwrap().ref_count;
        let before_symbol = state.atoms.resolve(symbol.atom()).unwrap().ref_count;
        let entries = [ShapeEntry {
            atom: key.atom(),
            flags: PropertyFlags::accessor(true, true),
        }];
        let result = state.allocate_object_with_layout(
            None,
            &entries,
            vec![PropertySlot::Data(RawValue::Symbol(symbol.atom()))],
            ObjectData::ordinary,
        );
        assert!(
            result.is_err(),
            "mismatched storage must fail before publication"
        );
        assert_eq!(
            state.atoms.resolve(key.atom()).unwrap().ref_count,
            before_key
        );
        assert_eq!(
            state.atoms.resolve(symbol.atom()).unwrap().ref_count,
            before_symbol
        );
        assert!(state.shape_cache.is_empty());
        assert!(state.shape_fingerprints.is_empty());
    }
}
