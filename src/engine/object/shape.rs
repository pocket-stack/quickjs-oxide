//! Ordinary-object shape metadata with shared-shape copy-on-write.
//!
//! QuickJS stores an object's prototype and its ordered property keys/flags in
//! a `JSShape`, while the object owns a parallel array of property payloads.
//! This module keeps the same semantic split. Shared shapes are immutable:
//! adding, replacing, or deleting a property derives a new shape. A shape
//! proven to have exactly one object owner may append a new property in place,
//! after its weak interner signature is removed. This copy-on-write strategy
//! preserves observable property order without mutation through aliases and
//! keeps long unique-object construction amortized linear.
//!
//! Atom reference-count ownership is intentionally outside this metadata
//! type.  The runtime's shape interner/heap must retain atoms admitted to a
//! shape and release them when the interned shape is reclaimed.  Likewise,
//! [`Shape::ordered_own_keys`] returns an identity snapshot without changing
//! atom reference counts; a public API which lets the snapshot outlive the
//! shape must retain those atoms at the runtime boundary.

use super::dictionary_order::DictionaryOrder;
use crate::engine::atom::{Atom, AtomError, AtomIdx, AtomTable, PropertyKeyKind};
use crate::engine::heap::ObjectId;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;

/// The representation used by the object's property-payload slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyStorageKind {
    /// The parallel slot contains a property value.
    Data,
    /// The parallel slot contains getter and setter values.
    Accessor,
}

/// Shape-resident attributes for one ordinary property.
///
/// `writable` has meaning only when `storage` is [`PropertyStorageKind::Data`].
/// Keeping it present for both variants makes shape comparison and transition
/// lookup compact, matching QuickJS's flag-oriented representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PropertyFlags {
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
    pub storage: PropertyStorageKind,
}

// `PropertyFlags` must stay compact: it shares the 8-byte `ShapeEntry` with
// the `AtomIdx` key.
const _: () = assert!(std::mem::size_of::<PropertyFlags>() == 4);

impl PropertyFlags {
    /// Construct flags for a data property.
    #[must_use]
    pub const fn data(writable: bool, enumerable: bool, configurable: bool) -> Self {
        Self {
            writable,
            enumerable,
            configurable,
            storage: PropertyStorageKind::Data,
        }
    }

    /// Construct flags for an accessor property.
    #[must_use]
    pub const fn accessor(enumerable: bool, configurable: bool) -> Self {
        Self {
            writable: false,
            enumerable,
            configurable,
            storage: PropertyStorageKind::Accessor,
        }
    }
}

/// One entry in a shape's insertion-ordered property metadata.
///
/// The key is the unbranded [`AtomIdx`]: the shape retains every admitted atom
/// for its own lifetime, so the entry itself needs no brand stamp.  Brand
/// checks run at boundaries which receive or expose an entry's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShapeEntry {
    pub atom: AtomIdx,
    pub flags: PropertyFlags,
}

const _: () = assert!(std::mem::size_of::<ShapeEntry>() == 8);

/// Failure to construct a valid immutable shape transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeError {
    /// The null sentinel is not an ECMAScript property key.
    NullAtom,
    /// A shape cannot contain the same property key twice.
    DuplicateAtom(Atom),
    /// A transition requested replacement or deletion of a missing property.
    MissingAtom(Atom),
    /// A property's position cannot be represented by the `u32` lookup table.
    PropertyIndexOverflow,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NullAtom => formatter.write_str("the null atom is not a property key"),
            Self::DuplicateAtom(atom) => {
                write!(formatter, "duplicate property atom {atom:?} in shape")
            }
            Self::MissingAtom(atom) => {
                write!(formatter, "property atom {atom:?} is not present in shape")
            }
            Self::PropertyIndexOverflow => {
                formatter.write_str("shape property index does not fit in u32")
            }
        }
    }
}

impl Error for ShapeError {}

/// Prototype and property-layout metadata shared copy-on-write by objects.
///
/// `entries` follows physical payload-slot order. Shared layouts use insertion
/// order; dictionaries maintain separate links so removal can move the last
/// slot in constant time. `lookup` always maps a key to its current slot.
/// Slot positions are internal and must be looked up again after mutations.
#[derive(Clone, Debug)]
pub struct Shape {
    layout_revision: u64,
    prototype: Option<ObjectId>,
    entries: Vec<ShapeEntry>,
    lookup: HashMap<AtomIdx, u32, crate::engine::hash::FxBuildHasher>,
    /// Present only for dynamic layouts; shared shapes pay one optional pointer.
    dictionary_order: Option<Box<DictionaryOrder>>,
}

impl Shape {
    pub(crate) const fn layout_revision(&self) -> u64 {
        self.layout_revision
    }

    pub(crate) fn invalidate_layout(&mut self) {
        // Saturation permanently disables IC admission; never wrap into stale facts.
        self.layout_revision = self.layout_revision.saturating_add(1);
    }

    /// Construct validated shape metadata.
    ///
    /// The constructor rejects null or duplicate atoms and proves that every
    /// entry position can be used as a `u32` property-slot index.  Atom liveness
    /// and runtime ownership are validated by the runtime before this boundary.
    pub fn new<I>(prototype: Option<ObjectId>, entries: I) -> Result<Self, ShapeError>
    where
        I: IntoIterator<Item = ShapeEntry>,
    {
        let iterator = entries.into_iter();
        let (lower_bound, _) = iterator.size_hint();
        let mut ordered = Vec::with_capacity(lower_bound);
        let mut lookup = HashMap::with_hasher(crate::engine::hash::FxBuildHasher::default());

        for entry in iterator {
            if entry.atom.is_null() {
                return Err(ShapeError::NullAtom);
            }
            let index =
                u32::try_from(ordered.len()).map_err(|_| ShapeError::PropertyIndexOverflow)?;
            if if ordered.len() <= 8 {
                ordered
                    .iter()
                    .any(|old: &ShapeEntry| old.atom == entry.atom)
            } else {
                lookup.contains_key(&entry.atom)
            } {
                // Entry keys arrive pre-validated at this boundary (callers
                // unbrand or convert trusted atoms before constructing), so
                // re-stamping the brand is unnecessary for the diagnostic.
                return Err(ShapeError::DuplicateAtom(Atom::from_raw(entry.atom.raw())));
            }
            ordered.push(entry);
            if ordered.len() == 9 {
                lookup.extend(ordered.iter().enumerate().map(|(i, e)| (e.atom, i as u32)));
            } else if ordered.len() > 9 {
                lookup.insert(entry.atom, index);
            }
        }

        Ok(Self {
            layout_revision: 0,
            prototype,
            entries: ordered,
            lookup,
            dictionary_order: None,
        })
    }

    /// Return the shape's `[[Prototype]]` object identity.
    #[must_use]
    pub const fn prototype(&self) -> Option<ObjectId> {
        self.prototype
    }

    /// Return property metadata in physical payload-slot order.
    /// Use `ordered_indices` when insertion order is observable.
    #[must_use]
    pub fn entries(&self) -> &[ShapeEntry] {
        &self.entries
    }

    /// Physical slot order can differ from insertion order in a dictionary.
    pub(crate) fn ordered_indices(&self) -> impl Iterator<Item = usize> + '_ {
        let first = self.dictionary_order.as_ref().map_or_else(
            || (!self.entries.is_empty()).then_some(0),
            |order| order.first(),
        );
        std::iter::successors(first, |&index| {
            self.dictionary_order.as_ref().map_or_else(
                || (index + 1 < self.entries.len()).then_some(index + 1),
                |order| order.next(index),
            )
        })
    }

    pub(crate) fn is_dictionary(&self) -> bool {
        self.dictionary_order.is_some()
    }

    /// Caller owns this metadata exclusively or is preparing an unpublished copy.
    pub(crate) fn enable_dictionary(&mut self) {
        if self.dictionary_order.is_none() {
            self.dictionary_order = Some(Box::new(DictionaryOrder::new(self.entries.len())));
        }
    }

    pub(crate) fn dictionary_layout_is_valid(&self) -> bool {
        self.dictionary_order.as_ref().is_none_or(|order| {
            order.is_valid(self.entries.len())
                && (self.entries.len() <= 8 || self.lookup.len() == self.entries.len())
                && self
                    .entries
                    .iter()
                    .enumerate()
                    .all(|(index, entry)| self.find(entry.atom) == u32::try_from(index).ok())
        })
    }

    pub(crate) fn remove_dictionary_property(
        &mut self,
        atom: AtomIdx,
    ) -> Result<ShapeEntry, ShapeError> {
        let index = self
            .find(atom)
            .ok_or(ShapeError::MissingAtom(Atom::from_raw(atom.raw())))?
            as usize;
        self.dictionary_order
            .as_mut()
            .expect("dictionary removal requires dictionary metadata")
            .swap_remove(index);
        let removed = self.entries.swap_remove(index);
        if self.entries.len() <= 8 {
            self.lookup.clear();
        } else {
            self.lookup.remove(&atom);
            if let Some(moved) = self.entries.get(index) {
                self.lookup.insert(moved.atom, index as u32);
            }
        }
        let len = self.entries.len();
        if self.entries.capacity() > len.saturating_mul(4).saturating_add(16) {
            self.entries
                .shrink_to(len.saturating_mul(2).saturating_add(8));
        }
        if self.lookup.capacity() > len.saturating_mul(4).saturating_add(16) {
            self.lookup
                .shrink_to(len.saturating_mul(2).saturating_add(8));
        }
        Ok(removed)
    }

    pub(crate) fn replace_dictionary_flags(&mut self, index: usize, flags: PropertyFlags) {
        debug_assert!(self.is_dictionary());
        self.entries[index].flags = flags;
    }

    /// Find a property and return its parallel payload-slot index.
    #[must_use]
    pub fn find(&self, atom: AtomIdx) -> Option<u32> {
        if self.entries.len() <= 8 {
            self.entries
                .iter()
                .position(|entry| entry.atom == atom)
                .map(|index| index as u32)
        } else {
            self.lookup.get(&atom).copied()
        }
    }

    /// Derive the shape produced by appending a new property.
    ///
    /// Appending is semantically important: after deletion, adding the same
    /// non-index string or symbol again places it at the end of its own-key
    /// category, as required by ECMAScript and QuickJS.
    #[cfg(test)]
    pub fn derive_add(&self, atom: AtomIdx, flags: PropertyFlags) -> Result<Self, ShapeError> {
        if atom.is_null() {
            return Err(ShapeError::NullAtom);
        }
        if self.find(atom).is_some() {
            return Err(ShapeError::DuplicateAtom(Atom::from_raw(atom.raw())));
        }

        let index = self.unique_append_index(atom)?;
        let mut result = self.clone();
        result.append_unique_property(atom, flags, index);
        Ok(result)
    }

    /// Append one property to a shape which is exclusively owned by a single
    /// object.
    ///
    /// The runtime authenticates exclusive ownership and atom liveness before
    /// reaching this mutation boundary. Keeping spare `Vec` capacity makes a
    /// long sequence of unique-object additions amortized linear while shared
    /// shapes continue to use immutable transitions.
    pub(crate) fn unique_append_index(&self, atom: AtomIdx) -> Result<u32, ShapeError> {
        if atom.is_null() {
            return Err(ShapeError::NullAtom);
        }
        if self.find(atom).is_some() {
            return Err(ShapeError::DuplicateAtom(Atom::from_raw(atom.raw())));
        }
        u32::try_from(self.entries.len()).map_err(|_| ShapeError::PropertyIndexOverflow)
    }

    pub(crate) fn append_unique_property(&mut self, atom: AtomIdx, flags: PropertyFlags, index: u32) {
        debug_assert_eq!(usize::try_from(index), Ok(self.entries.len()));
        debug_assert!(!atom.is_null() && self.find(atom).is_none());
        self.entries.push(ShapeEntry { atom, flags });
        if self.entries.len() == 9 {
            self.lookup.extend(
                self.entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| (e.atom, i as u32)),
            );
        } else if self.entries.len() > 9 {
            self.lookup.insert(atom, index);
        }
        if let Some(order) = &mut self.dictionary_order {
            order.append();
        }
    }

    /// Derive a shape with updated flags for an existing property.
    ///
    /// Replacement preserves insertion order and payload-slot position.
    #[cfg(test)]
    pub fn derive_replace(&self, atom: AtomIdx, flags: PropertyFlags) -> Result<Self, ShapeError> {
        if atom.is_null() {
            return Err(ShapeError::NullAtom);
        }
        let index = usize::try_from(
            self.find(atom)
                .ok_or(ShapeError::MissingAtom(Atom::from_raw(atom.raw())))?,
        )
        .map_err(|_| ShapeError::PropertyIndexOverflow)?;
        let mut entries = self.entries.to_vec();
        entries[index].flags = flags;
        Ok(Self {
            layout_revision: 0,
            prototype: self.prototype,
            entries,
            lookup: self.lookup.clone(),
            dictionary_order: self.dictionary_order.clone(),
        })
    }

    /// Derive a shape with one existing property removed.
    ///
    /// Payload slots after the removed property shift left, so the lookup table
    /// is rebuilt by the validated constructor.
    #[cfg(test)]
    pub fn derive_delete(&self, atom: AtomIdx) -> Result<Self, ShapeError> {
        if atom.is_null() {
            return Err(ShapeError::NullAtom);
        }
        let index = usize::try_from(
            self.find(atom)
                .ok_or(ShapeError::MissingAtom(Atom::from_raw(atom.raw())))?,
        )
        .map_err(|_| ShapeError::PropertyIndexOverflow)?;
        if self.is_dictionary() {
            let mut result = self.clone();
            result.remove_dictionary_property(atom)?;
            return Ok(result);
        }
        let mut entries = Vec::with_capacity(self.entries.len().saturating_sub(1));
        entries.extend_from_slice(&self.entries[..index]);
        entries.extend_from_slice(&self.entries[index + 1..]);
        Self::new(self.prototype, entries)
    }

    /// Snapshot own keys in ECMAScript `[[OwnPropertyKeys]]` order.
    ///
    /// Array-index strings come first in ascending numeric order, followed by
    /// all other strings in insertion order, then symbols in insertion order.
    /// Private names are internal and are omitted.  Every stored key index is
    /// fully re-validated (branded) against `atoms` at this public exit, but
    /// this metadata-level operation does not retain it.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError`] if the shape contains an atom which is not live in
    /// the supplied runtime-local table.
    pub fn ordered_own_keys(&self, atoms: &AtomTable) -> Result<Vec<Atom>, AtomError> {
        let mut indices = Vec::new();
        let mut strings = Vec::with_capacity(self.entries.len());
        let mut symbols = Vec::new();

        for (insertion_index, slot) in self.ordered_indices().enumerate() {
            let entry = &self.entries[slot];
            // Public exit boundary: the stored unbranded index gets the full
            // brand (generation/table-ID) check before leaving the shape.
            let atom = atoms.brand(entry.atom)?;
            match atoms.property_key_kind(atom)? {
                PropertyKeyKind::String => {
                    if let Some(array_index) = atoms.array_index(atom)? {
                        indices.push((array_index, insertion_index, atom));
                    } else {
                        strings.push(atom);
                    }
                }
                PropertyKeyKind::Symbol => symbols.push(atom),
                PropertyKeyKind::Private => {}
            }
        }

        // Distinct property atoms cannot denote the same canonical array index,
        // but insertion position is a deterministic tie-breaker if a malformed
        // table implementation ever violates that invariant.
        indices.sort_unstable_by_key(|&(array_index, insertion_index, _)| {
            (array_index, insertion_index)
        });

        let mut keys = Vec::with_capacity(indices.len() + strings.len() + symbols.len());
        keys.extend(indices.into_iter().map(|(_, _, atom)| atom));
        keys.extend(strings);
        keys.extend(symbols);
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_DATA: PropertyFlags = PropertyFlags::data(true, true, true);

    fn entry(atom: AtomIdx) -> ShapeEntry {
        ShapeEntry {
            atom,
            flags: DEFAULT_DATA,
        }
    }

    fn immediate(value: u32) -> AtomIdx {
        AtomIdx::from_immediate_integer(value).unwrap()
    }

    #[test]
    fn small_shape_lookup_allocates_only_above_eight_entries() {
        let atoms = (1..=9).map(immediate).collect::<Vec<_>>();
        let small = Shape::new(None, atoms[..8].iter().copied().map(entry)).unwrap();
        assert_eq!(small.lookup.capacity(), 0);
        for (index, atom) in atoms[..8].iter().enumerate() {
            assert_eq!(small.find(*atom), Some(index as u32));
        }
        let large = small.derive_add(atoms[8], DEFAULT_DATA).unwrap();
        assert!(large.lookup.capacity() >= 9);
        assert_eq!(large.find(atoms[8]), Some(8));
        let small = large.derive_delete(atoms[8]).unwrap();
        assert_eq!(small.lookup.capacity(), 0);
    }

    #[test]
    fn constructor_builds_lookup_and_rejects_invalid_entries() {
        let first = immediate(1);
        let second = immediate(2);
        let shape = Shape::new(None, [entry(first), entry(second)]).unwrap();

        assert_eq!(shape.entries(), &[entry(first), entry(second)]);
        assert_eq!(shape.find(first), Some(0));
        assert_eq!(shape.find(second), Some(1));
        assert_eq!(shape.find(immediate(3)), None);
        assert!(shape.prototype().is_none());

        assert!(matches!(
            Shape::new(None, [entry(first), entry(first)]),
            Err(ShapeError::DuplicateAtom(atom)) if atom.raw() == first.raw()
        ));
        assert!(matches!(
            Shape::new(None, [entry(AtomIdx::NULL)]),
            Err(ShapeError::NullAtom)
        ));
    }

    #[test]
    fn transitions_preserve_or_update_insertion_positions() {
        let mut atoms = AtomTable::new();
        let alpha = AtomIdx::from_raw(atoms.intern("alpha").unwrap().raw());
        let beta = AtomIdx::from_raw(atoms.intern("beta").unwrap().raw());
        let shape = Shape::new(None, [entry(alpha), entry(beta)]).unwrap();

        let accessor = PropertyFlags::accessor(false, true);
        let replaced = shape.derive_replace(alpha, accessor).unwrap();
        assert_eq!(replaced.find(alpha), Some(0));
        assert_eq!(replaced.entries()[0].flags, accessor);
        assert_eq!(shape.entries()[0].flags, DEFAULT_DATA);

        let deleted = replaced.derive_delete(alpha).unwrap();
        assert_eq!(deleted.find(beta), Some(0));
        let readded = deleted.derive_add(alpha, DEFAULT_DATA).unwrap();
        assert_eq!(readded.entries(), &[entry(beta), entry(alpha)]);
        assert_eq!(readded.find(alpha), Some(1));

        assert!(matches!(
            readded.derive_add(alpha, DEFAULT_DATA),
            Err(ShapeError::DuplicateAtom(atom)) if atom.raw() == alpha.raw()
        ));
        let missing = AtomIdx::from_raw(atoms.intern("missing").unwrap().raw());
        assert!(matches!(
            readded.derive_replace(missing, DEFAULT_DATA),
            Err(ShapeError::MissingAtom(atom)) if atom.raw() == missing.raw()
        ));
        assert!(matches!(
            readded.derive_delete(missing),
            Err(ShapeError::MissingAtom(atom)) if atom.raw() == missing.raw()
        ));
    }

    #[test]
    fn own_keys_follow_array_string_symbol_order_and_hide_private_names() {
        let mut atoms = AtomTable::new();
        let key = |atoms: &mut AtomTable, text: &str| {
            AtomIdx::from_raw(atoms.intern(text).unwrap().raw())
        };
        let beta = key(&mut atoms, "beta");
        let index_10 = key(&mut atoms, "10");
        let symbol_a = AtomIdx::from_raw(atoms.new_symbol(Some("a")).unwrap().raw());
        let index_2 = key(&mut atoms, "2");
        let noncanonical_index = key(&mut atoms, "01");
        let private = AtomIdx::from_raw(atoms.new_private_symbol(Some("hidden")).unwrap().raw());
        let global_symbol =
            AtomIdx::from_raw(atoms.intern_global_symbol("shared").unwrap().raw());
        let largest_index = key(&mut atoms, "4294967294");
        let excluded_index = key(&mut atoms, "4294967295");

        let shape = Shape::new(
            None,
            [
                beta,
                index_10,
                symbol_a,
                index_2,
                noncanonical_index,
                private,
                global_symbol,
                largest_index,
                excluded_index,
            ]
            .map(entry),
        )
        .unwrap();

        let branded = |index: AtomIdx| atoms.brand(index).unwrap();
        assert_eq!(
            shape.ordered_own_keys(&atoms).unwrap(),
            vec![
                branded(index_2),
                branded(index_10),
                branded(largest_index),
                branded(beta),
                branded(noncanonical_index),
                branded(excluded_index),
                branded(symbol_a),
                branded(global_symbol),
            ]
        );
    }

    #[test]
    fn delete_then_readd_moves_non_index_key_to_category_end() {
        let mut atoms = AtomTable::new();
        let first = AtomIdx::from_raw(atoms.intern("first").unwrap().raw());
        let second = AtomIdx::from_raw(atoms.intern("second").unwrap().raw());
        let shape = Shape::new(None, [entry(first), entry(second)]).unwrap();
        let changed = shape
            .derive_delete(first)
            .unwrap()
            .derive_add(first, DEFAULT_DATA)
            .unwrap();

        let branded = |index: AtomIdx| atoms.brand(index).unwrap();
        assert_eq!(
            changed.ordered_own_keys(&atoms).unwrap(),
            [branded(second), branded(first)]
        );
        assert_eq!(
            shape.ordered_own_keys(&atoms).unwrap(),
            [branded(first), branded(second)]
        );
    }

    #[test]
    fn own_key_snapshot_validates_the_runtime_local_atom_table() {
        let mut owner = AtomTable::new();
        let foreign = AtomTable::new();
        let key = AtomIdx::from_raw(owner.intern("owner-only").unwrap().raw());
        let shape = Shape::new(None, [entry(key)]).unwrap();

        assert!(matches!(
            shape.ordered_own_keys(&foreign),
            Err(AtomError::UnknownAtom(atom)) if atom.raw() == key.raw()
        ));
    }
}
