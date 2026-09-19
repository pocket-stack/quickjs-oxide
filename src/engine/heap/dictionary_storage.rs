//! Exclusive dictionary property-layout mutations. All fallible validation and
//! retains precede publication; moved slots transfer ownership without cloning.

use super::*;

impl Heap {
    pub(crate) fn enable_object_dictionary(&mut self, id: ObjectId) -> Result<(), HeapError> {
        let shape = self.exclusive_dictionary_shape(id)?;

        self.invalidate_property_layout(id);
        self.shape_mut(shape)?.enable_dictionary();
        Ok(())
    }

    fn exclusive_dictionary_shape(&self, id: ObjectId) -> Result<ShapeId, HeapError> {
        let object = self.object(id)?;
        if !object.supports_dictionary_layout() {
            return Err(HeapError::Invariant(
                "dictionary mutation requires an ordinary object or slow Array",
            ));
        }
        let shape = self.shape(object.shape)?;
        if self.shape_strong_count(object.shape)? != 1
            || shape.entries().len() != object.slots.len()
        {
            return Err(HeapError::Invariant(
                "dictionary mutation requires an exclusive parallel layout",
            ));
        }
        Ok(object.shape)
    }

    pub(crate) fn delete_dictionary_property(
        &mut self,
        id: ObjectId,
        atom: Atom,
    ) -> Result<HeapCleanup, HeapError> {
        let shape_id = self.exclusive_dictionary_shape(id)?;
        let shape = self.shape(shape_id)?;
        let index = shape
            .find(AtomIdx::from_raw(atom.raw()))
            .ok_or(HeapError::Invariant(
                "dictionary deletion requires an existing property",
            ))? as usize;
        if !shape.is_dictionary() || !shape.entries()[index].flags.configurable {
            return Err(HeapError::Invariant(
                "dictionary deletion requires configurable dictionary storage",
            ));
        }

        self.invalidate_property_layout(id);
        self.shape_mut(shape_id)?
            .remove_dictionary_property(AtomIdx::from_raw(atom.raw()))
            .expect("dictionary key was validated before mutation");
        let slots = &mut self.object_mut(id)?.slots;
        let previous = slots.swap_remove(index);
        let len = slots.len();
        if slots.capacity() > len.saturating_mul(4).saturating_add(16) {
            slots.shrink_to(len.saturating_mul(2).saturating_add(8));
        }
        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.push(AtomIdx::from_raw(atom.raw()));
        cleanup.atoms.extend(property_slot_atoms(&previous));
        for edge in property_slot_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    pub(crate) fn replace_dictionary_property(
        &mut self,
        id: ObjectId,
        index: usize,
        flags: PropertyFlags,
        replacement: PropertySlot,
    ) -> Result<HeapCleanup, HeapError> {
        let shape_id = self.exclusive_dictionary_shape(id)?;
        let shape = self.shape(shape_id)?;
        if !shape.is_dictionary() || index >= shape.entries().len() {
            return Err(HeapError::Invariant(
                "dictionary replacement requires an existing dictionary slot",
            ));
        }
        if !slot_matches_storage(&replacement, flags.storage)
            || matches!(replacement, PropertySlot::Data(RawValue::Private(_)))
        {
            return Err(HeapError::Invariant(
                "invalid dictionary replacement storage",
            ));
        }
        self.retain_edges_transactionally(&property_slot_edges(&replacement))?;

        self.invalidate_property_layout(id);
        self.shape_mut(shape_id)?
            .replace_dictionary_flags(index, flags);
        self.replace_retained_object_slot(id, index, replacement)
    }
}
