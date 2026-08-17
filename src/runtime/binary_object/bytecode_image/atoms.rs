//! Semantic atom identities for one complete BC5 bytecode image.
//!
//! Header slots remain a wire concern: function prefixes and opcode scanners
//! use the raw [`AtomIndexSpace`], then this table relocates their
//! [`BinaryAtom`] values into identities shared by every function in the
//! image. As in QuickJS `JS_ReadObjectAtoms` + `JS_NewAtomStr`, decimal index
//! spellings, predefined string atoms, and repeated dynamic strings can all
//! make several raw header slots alias one semantic atom.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};

use super::super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode, HeaderAtomSlot};
use super::super::graph::model::{
    AtomId, numeric_atom_index, semantic_atom_eq, semantic_atom_hash,
};
use super::super::pinned_atoms::{self, PinnedAtomId};
use super::super::read_cursor::CheckedReadCursor;
use super::super::wire::{ReaderMode, WireError, WireString};

/// One semantic atom identity shared by the entire bytecode image.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum ImageAtom {
    /// QuickJS's reserved atom-zero sentinel.
    Null,
    /// A tagged, non-negative integer property key.
    Index(u32),
    /// An identity from the release-pinned QuickJS atom manifest.
    Predefined(PinnedAtomId),
    /// A string atom interned from this image's header.
    Dynamic(AtomId),
}

/// A semantic property key. Atom zero is deliberately unrepresentable.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum ImageKey {
    Index(u32),
    Predefined(PinnedAtomId),
    Dynamic(AtomId),
}

/// Failures while reading or relocating the bytecode image's atom namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ImageAtomError {
    Wire(WireError),
    AtomIndexSpaceMismatch {
        expected: AtomIndexSpace,
        actual: AtomIndexSpace,
    },
    ForeignHeaderSlot {
        slot: u32,
        header_count: u32,
    },
    DynamicAtomCountOverflow {
        atom_count: usize,
    },
    NullPropertyKey {
        offset: usize,
    },
}

impl fmt::Display for ImageAtomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => fmt::Display::fmt(error, formatter),
            Self::AtomIndexSpaceMismatch { expected, actual } => write!(
                formatter,
                "bytecode image atom index-space shape mismatch: expected {expected:?}, got {actual:?}"
            ),
            Self::ForeignHeaderSlot { slot, header_count } => write!(
                formatter,
                "header atom slot {slot} does not belong to a table with {header_count} slots"
            ),
            Self::DynamicAtomCountOverflow { atom_count } => write!(
                formatter,
                "bytecode image dynamic atom count {atom_count} exceeds u32"
            ),
            Self::NullPropertyKey { offset } => {
                write!(formatter, "null property atom at byte {offset}")
            }
        }
    }
}

impl std::error::Error for ImageAtomError {}

impl From<WireError> for ImageAtomError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

/// Bounded semantic remap for the bytecode-mode atom header of one image.
///
/// `slot_atoms` has exactly one entry per raw header slot. `dynamic_atoms`
/// stores only distinct non-numeric strings which do not resolve to an
/// ordinary predefined string atom. Both allocations are reserved through
/// fallible APIs before their first append.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct ImageAtomTable {
    raw_space: AtomIndexSpace,
    slot_atoms: Vec<ImageAtom>,
    dynamic_atoms: Vec<WireString>,
}

impl ImageAtomTable {
    /// Read the BC5 header and all of its strings from the caller's cursor.
    ///
    /// The cursor is intentionally not finalized: on success it remains at the
    /// first value tag, ready for a future whole-image reader. All wire limits,
    /// including atom count and aggregate string code units, remain enforced
    /// by this same cursor.
    pub(super) fn read<'input, C>(cursor: &mut C) -> Result<Self, ImageAtomError>
    where
        C: CheckedReadCursor<'input>,
    {
        let header = cursor.read_header()?;
        let raw_space = AtomIndexSpace::new(BinaryObjectMode::Bytecode, header.atom_count)?;
        let atom_count = header.atom_count as usize;

        let mut slot_atoms = Vec::new();
        slot_atoms
            .try_reserve_exact(atom_count)
            .map_err(|_| WireError::AllocationFailed)?;
        let mut dynamic_atoms = Vec::new();
        dynamic_atoms
            .try_reserve_exact(atom_count)
            .map_err(|_| WireError::AllocationFailed)?;
        let mut dynamics_by_hash = HashMap::new();
        dynamics_by_hash
            .try_reserve(atom_count)
            .map_err(|_| WireError::AllocationFailed)?;
        let hash_builder = RandomState::new();

        for _ in 0..header.atom_count {
            let value = cursor.read_string()?;
            let atom = classify_header_atom(
                value,
                &mut dynamic_atoms,
                &mut dynamics_by_hash,
                &hash_builder,
            )?;
            slot_atoms.push(atom);
        }

        Ok(Self {
            raw_space,
            slot_atoms,
            dynamic_atoms,
        })
    }

    /// Return the raw bytecode atom namespace used by prefix and code scanners.
    #[must_use]
    pub(super) const fn raw_space(&self) -> AtomIndexSpace {
        self.raw_space
    }

    /// Return the semantic atom assigned to each raw header slot.
    #[must_use]
    pub(super) fn slot_atoms(&self) -> &[ImageAtom] {
        &self.slot_atoms
    }

    /// Return distinct image-local dynamic atom strings in allocation order.
    #[must_use]
    pub(super) fn dynamic_atoms(&self) -> &[WireString] {
        &self.dynamic_atoms
    }

    /// Consume the remap table after every raw atom has been relocated.
    pub(super) fn into_dynamic_atoms(self) -> Box<[WireString]> {
        self.dynamic_atoms.into_boxed_slice()
    }

    /// Relocate one raw scanner atom into the whole-image semantic namespace.
    ///
    /// `source_space` must have the mode and header count obtained from
    /// [`Self::raw_space`]. This rejects wrong-mode and differently sized
    /// scans, but [`AtomIndexSpace`] is structural and carries no unique image
    /// provenance. The future whole-image owner must therefore remap scanner
    /// output immediately instead of retaining raw atoms across images.
    pub(super) fn remap_atom(
        &self,
        source_space: AtomIndexSpace,
        atom: BinaryAtom,
        diagnostic_offset: usize,
    ) -> Result<ImageAtom, ImageAtomError> {
        self.check_source_space(source_space)?;
        if let BinaryAtom::Header(slot) = atom {
            return self.resolve_header_slot(slot);
        }
        source_space.validate_metadata_atom(atom, diagnostic_offset)?;
        match atom {
            BinaryAtom::Null => Ok(ImageAtom::Null),
            BinaryAtom::Index(index) => Ok(ImageAtom::Index(index)),
            BinaryAtom::Predefined(atom) => Ok(ImageAtom::Predefined(atom)),
            BinaryAtom::Header(_) => unreachable!("header atoms return after their slot check"),
        }
    }

    /// Relocate one raw scanner atom as a property key.
    ///
    /// Strict admission rejects atom zero. QuickJS-compatible admission
    /// returns `None`, directing the future object reader to consume the value
    /// but omit the property, exactly as pinned QuickJS does. No `ImageKey`
    /// variant can represent atom zero.
    pub(super) fn remap_key(
        &self,
        source_space: AtomIndexSpace,
        atom: BinaryAtom,
        mode: ReaderMode,
        diagnostic_offset: usize,
    ) -> Result<Option<ImageKey>, ImageAtomError> {
        match self.remap_atom(source_space, atom, diagnostic_offset)? {
            ImageAtom::Null => match mode {
                ReaderMode::Strict => Err(ImageAtomError::NullPropertyKey {
                    offset: diagnostic_offset,
                }),
                ReaderMode::QuickJsCompatible => Ok(None),
            },
            ImageAtom::Index(index) => Ok(Some(ImageKey::Index(index))),
            ImageAtom::Predefined(atom) => Ok(Some(ImageKey::Predefined(atom))),
            ImageAtom::Dynamic(atom) => Ok(Some(ImageKey::Dynamic(atom))),
        }
    }

    fn check_source_space(&self, actual: AtomIndexSpace) -> Result<(), ImageAtomError> {
        if actual == self.raw_space {
            Ok(())
        } else {
            Err(ImageAtomError::AtomIndexSpaceMismatch {
                expected: self.raw_space,
                actual,
            })
        }
    }

    fn resolve_header_slot(&self, slot: HeaderAtomSlot) -> Result<ImageAtom, ImageAtomError> {
        self.slot_atoms.get(slot.index() as usize).copied().ok_or(
            ImageAtomError::ForeignHeaderSlot {
                slot: slot.index(),
                header_count: self.raw_space.header_count(),
            },
        )
    }
}

fn classify_header_atom(
    value: WireString,
    dynamic_atoms: &mut Vec<WireString>,
    dynamics_by_hash: &mut HashMap<u64, AtomId>,
    hash_builder: &RandomState,
) -> Result<ImageAtom, ImageAtomError> {
    if let Some(index) = numeric_atom_index(&value) {
        return Ok(ImageAtom::Index(index));
    }
    if let Some(atom) = pinned_atoms::lookup_string(&value) {
        return Ok(ImageAtom::Predefined(atom));
    }

    let mut hasher = hash_builder.build_hasher();
    semantic_atom_hash(&value, &mut hasher);
    let hash = hasher.finish();
    if let Some(first) = dynamics_by_hash.get(&hash).copied() {
        if semantic_atom_eq(&dynamic_atoms[first.as_usize()], &value) {
            return Ok(ImageAtom::Dynamic(first));
        }
        if let Some((index, _)) = dynamic_atoms
            .iter()
            .enumerate()
            .find(|(_, candidate)| semantic_atom_eq(candidate, &value))
        {
            let index =
                u32::try_from(index).map_err(|_| ImageAtomError::DynamicAtomCountOverflow {
                    atom_count: dynamic_atoms.len(),
                })?;
            return Ok(ImageAtom::Dynamic(AtomId::from_zero_based(index)));
        }
    }

    let index = u32::try_from(dynamic_atoms.len()).map_err(|_| {
        ImageAtomError::DynamicAtomCountOverflow {
            atom_count: dynamic_atoms.len(),
        }
    })?;
    let atom = AtomId::from_zero_based(index);
    dynamics_by_hash.entry(hash).or_insert(atom);
    dynamic_atoms.push(value);
    Ok(ImageAtom::Dynamic(atom))
}
