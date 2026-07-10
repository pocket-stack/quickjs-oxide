//! Immutable ordinary-object shape metadata.
//!
//! QuickJS stores an object's prototype and its ordered property keys/flags in
//! a `JSShape`, while the object owns a parallel array of property payloads.
//! This module keeps the same semantic split.  Shapes are immutable after
//! construction: adding, replacing, or deleting a property derives a new
//! shape.  That is a deliberate safe-Rust rewrite strategy for QuickJS's
//! shared-shape and shape-transition machinery.  It preserves observable
//! property order while avoiding mutation through aliases.
//!
//! Atom reference-count ownership is intentionally outside this metadata
//! type.  The runtime's shape interner/heap must retain atoms admitted to a
//! shape and release them when the interned shape is reclaimed.  Likewise,
//! [`Shape::ordered_own_keys`] returns an identity snapshot without changing
//! atom reference counts; a public API which lets the snapshot outlive the
//! shape must retain those atoms at the runtime boundary.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use crate::atom::{Atom, AtomError, AtomTable, PropertyKeyKind};
use crate::heap::ObjectId;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShapeEntry {
    pub atom: Atom,
    pub flags: PropertyFlags,
}

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

/// Immutable prototype and property-layout metadata shared by objects.
///
/// `entries` is insertion ordered.  `lookup` is derived exclusively by the
/// validated constructor and maps each key to its parallel payload-slot index.
#[derive(Clone, Debug)]
pub struct Shape {
    prototype: Option<ObjectId>,
    entries: Box<[ShapeEntry]>,
    lookup: HashMap<Atom, u32>,
}

impl Shape {
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
        let mut lookup = HashMap::with_capacity(lower_bound);

        for entry in iterator {
            if entry.atom.is_null() {
                return Err(ShapeError::NullAtom);
            }
            let index =
                u32::try_from(ordered.len()).map_err(|_| ShapeError::PropertyIndexOverflow)?;
            if lookup.insert(entry.atom, index).is_some() {
                return Err(ShapeError::DuplicateAtom(entry.atom));
            }
            ordered.push(entry);
        }

        Ok(Self {
            prototype,
            entries: ordered.into_boxed_slice(),
            lookup,
        })
    }

    /// Return the shape's `[[Prototype]]` object identity.
    #[must_use]
    pub const fn prototype(&self) -> Option<ObjectId> {
        self.prototype
    }

    /// Return property metadata in insertion order.
    #[must_use]
    pub fn entries(&self) -> &[ShapeEntry] {
        &self.entries
    }

    /// Find a property and return its parallel payload-slot index.
    #[must_use]
    pub fn find(&self, atom: Atom) -> Option<u32> {
        self.lookup.get(&atom).copied()
    }

    /// Derive the shape produced by appending a new property.
    ///
    /// Appending is semantically important: after deletion, adding the same
    /// non-index string or symbol again places it at the end of its own-key
    /// category, as required by ECMAScript and QuickJS.
    pub fn derive_add(&self, atom: Atom, flags: PropertyFlags) -> Result<Self, ShapeError> {
        if atom.is_null() {
            return Err(ShapeError::NullAtom);
        }
        if self.lookup.contains_key(&atom) {
            return Err(ShapeError::DuplicateAtom(atom));
        }

        let index =
            u32::try_from(self.entries.len()).map_err(|_| ShapeError::PropertyIndexOverflow)?;
        let mut entries = Vec::with_capacity(self.entries.len().saturating_add(1));
        entries.extend_from_slice(&self.entries);
        entries.push(ShapeEntry { atom, flags });

        let mut lookup = self.lookup.clone();
        lookup.insert(atom, index);
        Ok(Self {
            prototype: self.prototype,
            entries: entries.into_boxed_slice(),
            lookup,
        })
    }

    /// Derive a shape with updated flags for an existing property.
    ///
    /// Replacement preserves insertion order and payload-slot position.
    pub fn derive_replace(&self, atom: Atom, flags: PropertyFlags) -> Result<Self, ShapeError> {
        if atom.is_null() {
            return Err(ShapeError::NullAtom);
        }
        let index = usize::try_from(self.find(atom).ok_or(ShapeError::MissingAtom(atom))?)
            .map_err(|_| ShapeError::PropertyIndexOverflow)?;
        let mut entries = self.entries.to_vec();
        entries[index].flags = flags;
        Ok(Self {
            prototype: self.prototype,
            entries: entries.into_boxed_slice(),
            lookup: self.lookup.clone(),
        })
    }

    /// Derive a shape with one existing property removed.
    ///
    /// Payload slots after the removed property shift left, so the lookup table
    /// is rebuilt by the validated constructor.
    pub fn derive_delete(&self, atom: Atom) -> Result<Self, ShapeError> {
        if atom.is_null() {
            return Err(ShapeError::NullAtom);
        }
        let index = usize::try_from(self.find(atom).ok_or(ShapeError::MissingAtom(atom))?)
            .map_err(|_| ShapeError::PropertyIndexOverflow)?;
        let mut entries = Vec::with_capacity(self.entries.len().saturating_sub(1));
        entries.extend_from_slice(&self.entries[..index]);
        entries.extend_from_slice(&self.entries[index + 1..]);
        Self::new(self.prototype, entries)
    }

    /// Snapshot own keys in ECMAScript `[[OwnPropertyKeys]]` order.
    ///
    /// Array-index strings come first in ascending numeric order, followed by
    /// all other strings in insertion order, then symbols in insertion order.
    /// Private names are internal and are omitted.  Every atom is validated
    /// against `atoms`, but this metadata-level operation does not retain it.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError`] if the shape contains an atom which is not live in
    /// the supplied runtime-local table.
    pub fn ordered_own_keys(&self, atoms: &AtomTable) -> Result<Vec<Atom>, AtomError> {
        let mut indices = Vec::new();
        let mut strings = Vec::new();
        let mut symbols = Vec::new();

        for (insertion_index, entry) in self.entries.iter().enumerate() {
            match atoms.property_key_kind(entry.atom)? {
                PropertyKeyKind::String => {
                    if let Some(array_index) = atoms.array_index(entry.atom)? {
                        indices.push((array_index, insertion_index, entry.atom));
                    } else {
                        strings.push(entry.atom);
                    }
                }
                PropertyKeyKind::Symbol => symbols.push(entry.atom),
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

    fn entry(atom: Atom) -> ShapeEntry {
        ShapeEntry {
            atom,
            flags: DEFAULT_DATA,
        }
    }

    #[test]
    fn constructor_builds_lookup_and_rejects_invalid_entries() {
        let first = Atom::from_immediate_integer(1).unwrap();
        let second = Atom::from_immediate_integer(2).unwrap();
        let shape = Shape::new(None, [entry(first), entry(second)]).unwrap();

        assert_eq!(shape.entries(), &[entry(first), entry(second)]);
        assert_eq!(shape.find(first), Some(0));
        assert_eq!(shape.find(second), Some(1));
        assert_eq!(shape.find(Atom::from_immediate_integer(3).unwrap()), None);
        assert!(shape.prototype().is_none());

        assert!(matches!(
            Shape::new(None, [entry(first), entry(first)]),
            Err(ShapeError::DuplicateAtom(atom)) if atom == first
        ));
        assert!(matches!(
            Shape::new(None, [entry(Atom::NULL)]),
            Err(ShapeError::NullAtom)
        ));
    }

    #[test]
    fn transitions_preserve_or_update_insertion_positions() {
        let mut atoms = AtomTable::new();
        let alpha = atoms.intern("alpha").unwrap();
        let beta = atoms.intern("beta").unwrap();
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
            Err(ShapeError::DuplicateAtom(atom)) if atom == alpha
        ));
        let missing = atoms.intern("missing").unwrap();
        assert!(matches!(
            readded.derive_replace(missing, DEFAULT_DATA),
            Err(ShapeError::MissingAtom(atom)) if atom == missing
        ));
        assert!(matches!(
            readded.derive_delete(missing),
            Err(ShapeError::MissingAtom(atom)) if atom == missing
        ));
    }

    #[test]
    fn own_keys_follow_array_string_symbol_order_and_hide_private_names() {
        let mut atoms = AtomTable::new();
        let beta = atoms.intern("beta").unwrap();
        let index_10 = atoms.intern("10").unwrap();
        let symbol_a = atoms.new_symbol(Some("a")).unwrap();
        let index_2 = atoms.intern("2").unwrap();
        let noncanonical_index = atoms.intern("01").unwrap();
        let private = atoms.new_private_symbol(Some("hidden")).unwrap();
        let global_symbol = atoms.intern_global_symbol("shared").unwrap();
        let largest_index = atoms.intern("4294967294").unwrap();
        let excluded_index = atoms.intern("4294967295").unwrap();

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

        assert_eq!(
            shape.ordered_own_keys(&atoms).unwrap(),
            vec![
                index_2,
                index_10,
                largest_index,
                beta,
                noncanonical_index,
                excluded_index,
                symbol_a,
                global_symbol,
            ]
        );
    }

    #[test]
    fn delete_then_readd_moves_non_index_key_to_category_end() {
        let mut atoms = AtomTable::new();
        let first = atoms.intern("first").unwrap();
        let second = atoms.intern("second").unwrap();
        let shape = Shape::new(None, [entry(first), entry(second)]).unwrap();
        let changed = shape
            .derive_delete(first)
            .unwrap()
            .derive_add(first, DEFAULT_DATA)
            .unwrap();

        assert_eq!(changed.ordered_own_keys(&atoms).unwrap(), [second, first]);
        assert_eq!(shape.ordered_own_keys(&atoms).unwrap(), [first, second]);
    }

    #[test]
    fn own_key_snapshot_validates_the_runtime_local_atom_table() {
        let mut owner = AtomTable::new();
        let foreign = AtomTable::new();
        let key = owner.intern("owner-only").unwrap();
        let shape = Shape::new(None, [entry(key)]).unwrap();

        assert!(matches!(
            shape.ordered_own_keys(&foreign),
            Err(AtomError::UnknownAtom(atom)) if atom == key
        ));
    }
}
