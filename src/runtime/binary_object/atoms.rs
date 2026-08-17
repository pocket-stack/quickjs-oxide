//! Checked atom-index translation for QuickJS 2026-06-04 binary objects.
//!
//! BC5 does not record whether an image contains bytecode. The caller-selected
//! mode determines the first header-backed atom: data images start at one,
//! while bytecode images reserve the release-pinned atoms below
//! [`FIRST_DYNAMIC_ATOM`]. Keeping that choice explicit prevents a valid image
//! from being silently reinterpreted in the other namespace.
//!
//! Metadata and opcode operands deliberately have separate entry points.
//! Metadata atoms are ULEB128 values whose low bit marks an immediate integer;
//! opcode atom operands are fixed-width raw `u32` atom values whose high bit
//! carries the integer tag.

use crate::atom::{ATOM_MAX_INT, ATOM_MAX_TABLE_INDEX, ATOM_TAG_INT};

use super::pinned_atoms::{FIRST_DYNAMIC_ATOM, PinnedAtomId};
use super::read_cursor::CheckedReadCursor;
#[cfg(test)]
use super::wire::WireCursor;
use super::wire::{WireError, WireWriter};

/// Caller-selected interpretation of a BC5 binary object's atom namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum BinaryObjectMode {
    /// Plain data images have no predefined-atom prefix beyond atom zero.
    Data,
    /// Bytecode images preserve all release-pinned QuickJS atom identities.
    Bytecode,
}

/// A checked zero-based position in this image's header atom table.
///
/// The field is private so arbitrary `u32` values cannot be mistaken for a
/// header slot. Use [`AtomIndexSpace::header_slot`] to obtain one.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime) struct HeaderAtomSlot(u32);

impl HeaderAtomSlot {
    /// Return the zero-based position in the binary-object header.
    #[must_use]
    pub(in crate::runtime) const fn index(self) -> u32 {
        self.0
    }
}

/// One decoded binary-object atom, independent of a runtime atom table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum BinaryAtom {
    /// QuickJS's reserved atom-zero sentinel.
    Null,
    /// A tagged, non-negative integer property key.
    Index(u32),
    /// A release-pinned atom used directly by a bytecode image.
    Predefined(PinnedAtomId),
    /// An atom whose string payload occupies a slot in the image header.
    Header(HeaderAtomSlot),
}

/// Checked mapping between BC5 wire indices and semantic atom categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct AtomIndexSpace {
    mode: BinaryObjectMode,
    first_atom: u32,
    atom_count: u32,
}

impl AtomIndexSpace {
    /// Construct the exact namespace selected by the caller's read/write mode.
    ///
    /// The header range is rejected if its final atom would exceed QuickJS's
    /// 30-bit table-index limit. No allocation is performed here.
    pub(in crate::runtime) const fn new(
        mode: BinaryObjectMode,
        header_count: u32,
    ) -> Result<Self, WireError> {
        let first_atom = match mode {
            BinaryObjectMode::Data => 1,
            BinaryObjectMode::Bytecode => FIRST_DYNAMIC_ATOM,
        };
        if header_count != 0 {
            let Some(last_atom) = first_atom.checked_add(header_count - 1) else {
                return Err(WireError::AtomIndexSpaceOverflow {
                    first_atom,
                    atom_count: header_count,
                    maximum: ATOM_MAX_TABLE_INDEX,
                });
            };
            if last_atom > ATOM_MAX_TABLE_INDEX {
                return Err(WireError::AtomIndexSpaceOverflow {
                    first_atom,
                    atom_count: header_count,
                    maximum: ATOM_MAX_TABLE_INDEX,
                });
            }
        }
        Ok(Self {
            mode,
            first_atom,
            atom_count: header_count,
        })
    }

    /// Return the mode chosen by the caller; it is not encoded in the header.
    #[must_use]
    pub(in crate::runtime) const fn mode(self) -> BinaryObjectMode {
        self.mode
    }

    /// Return the first raw atom index backed by the image header.
    #[must_use]
    pub(in crate::runtime) const fn first_atom(self) -> u32 {
        self.first_atom
    }

    /// Return the number of atom strings stored in the image header.
    #[must_use]
    pub(in crate::runtime) const fn header_count(self) -> u32 {
        self.atom_count
    }

    /// Validate a zero-based header position and return its strong slot type.
    #[must_use]
    pub(in crate::runtime) const fn header_slot(self, index: u32) -> Option<HeaderAtomSlot> {
        if index < self.atom_count {
            Some(HeaderAtomSlot(index))
        } else {
            None
        }
    }

    /// Decode one metadata atom from its low-bit-tagged ULEB128 spelling.
    ///
    /// ULEB errors (including strict-mode non-canonical encodings) are reported
    /// before atom-index validation, matching the wire reader's phase order.
    pub(in crate::runtime) fn decode_metadata_atom<'input, C>(
        self,
        cursor: &mut C,
    ) -> Result<BinaryAtom, WireError>
    where
        C: CheckedReadCursor<'input>,
    {
        let encoded = cursor.read_uleb128()?;
        self.resolve_metadata_atom(encoded, cursor.position())
    }

    /// Resolve an already-read low-bit-tagged metadata atom.
    ///
    /// Keeping the wire read separate lets checked compound cursors reuse the
    /// exact atom namespace rules without exposing their underlying cursor.
    pub(in crate::runtime) fn resolve_metadata_atom(
        self,
        encoded: u32,
        diagnostic_offset: usize,
    ) -> Result<BinaryAtom, WireError> {
        if encoded & 1 != 0 {
            return Ok(BinaryAtom::Index(encoded >> 1));
        }
        self.resolve_table_atom(encoded >> 1, diagnostic_offset)
    }

    /// Canonically encode one metadata atom as a low-bit-tagged ULEB128 value.
    pub(in crate::runtime) fn encode_metadata_atom(
        self,
        writer: &mut WireWriter,
        atom: BinaryAtom,
    ) -> Result<(), WireError> {
        let offset = writer.as_bytes().len();
        let encoded = self.metadata_atom_encoding(atom, offset)?;
        writer.write_uleb128(encoded)
    }

    /// Check that one semantic metadata atom belongs to this namespace.
    ///
    /// This performs no allocation and writes no bytes, so compound encoders
    /// can reject an inconsistent internal model before mutating their output.
    pub(in crate::runtime) fn validate_metadata_atom(
        self,
        atom: BinaryAtom,
        diagnostic_offset: usize,
    ) -> Result<(), WireError> {
        self.metadata_atom_encoding(atom, diagnostic_offset)
            .map(|_| ())
    }

    fn metadata_atom_encoding(self, atom: BinaryAtom, offset: usize) -> Result<u32, WireError> {
        match atom {
            BinaryAtom::Index(index) => {
                if index > ATOM_MAX_INT {
                    return Err(self.invalid_index(offset, index));
                }
                Ok((index << 1) | 1)
            }
            atom => Ok(self.encode_table_atom(atom, offset)? << 1),
        }
    }

    /// Resolve a fixed-width raw atom operand embedded in function bytecode.
    ///
    /// `diagnostic_offset` is the reader cursor used for an invalid-index
    /// diagnostic. Pinned QuickJS consumes the complete bytecode payload before
    /// relocating its atom operands, so a future image reader must pass that
    /// end-of-payload cursor rather than the operand's position.
    pub(in crate::runtime) fn resolve_opcode_atom(
        self,
        raw: u32,
        diagnostic_offset: usize,
    ) -> Result<BinaryAtom, WireError> {
        if raw & ATOM_TAG_INT != 0 {
            return Ok(BinaryAtom::Index(raw & ATOM_MAX_INT));
        }
        self.resolve_table_atom(raw, diagnostic_offset)
    }

    /// Encode one atom as the fixed-width raw value used by opcode operands.
    ///
    /// The caller supplies the operand byte offset so an invalid atom retains
    /// precise diagnostics before a bytecode buffer is mutated.
    pub(in crate::runtime) fn encode_opcode_atom(
        self,
        atom: BinaryAtom,
        offset: usize,
    ) -> Result<u32, WireError> {
        match atom {
            BinaryAtom::Index(index) => {
                if index > ATOM_MAX_INT {
                    return Err(self.invalid_index(offset, index));
                }
                Ok(ATOM_TAG_INT | index)
            }
            atom => self.encode_table_atom(atom, offset),
        }
    }

    fn resolve_table_atom(self, index: u32, offset: usize) -> Result<BinaryAtom, WireError> {
        if index == 0 {
            return Ok(BinaryAtom::Null);
        }
        if index < self.first_atom {
            return PinnedAtomId::from_raw(index)
                .map(BinaryAtom::Predefined)
                .ok_or_else(|| self.invalid_index(offset, index));
        }
        let slot = index - self.first_atom;
        self.header_slot(slot)
            .map(BinaryAtom::Header)
            .ok_or_else(|| self.invalid_index(offset, index))
    }

    fn encode_table_atom(self, atom: BinaryAtom, offset: usize) -> Result<u32, WireError> {
        match atom {
            BinaryAtom::Null => Ok(0),
            BinaryAtom::Predefined(atom) if atom.raw() < self.first_atom => Ok(atom.raw()),
            BinaryAtom::Predefined(atom) => Err(self.invalid_index(offset, atom.raw())),
            BinaryAtom::Header(slot) if slot.index() < self.atom_count => {
                // `new` proved the complete header range fits this addition.
                Ok(self.first_atom + slot.index())
            }
            BinaryAtom::Header(slot) => {
                Err(self.invalid_index(offset, self.first_atom.saturating_add(slot.index())))
            }
            BinaryAtom::Index(index) => Err(self.invalid_index(offset, index)),
        }
    }

    const fn invalid_index(self, offset: usize, index: u32) -> WireError {
        WireError::InvalidAtomIndex {
            offset,
            index,
            first_atom: self.first_atom,
            atom_count: self.atom_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::wire::{ReaderMode, ResourceKind, WireLimits};
    use super::*;

    const TEST_LIMITS: WireLimits = WireLimits::new(64, 16, 16, 16);

    fn strict_cursor(input: &[u8]) -> WireCursor<'_> {
        WireCursor::new(input, ReaderMode::Strict, TEST_LIMITS).unwrap()
    }

    fn pinned(raw: u32) -> PinnedAtomId {
        PinnedAtomId::from_raw(raw).expect("test atom must be release-pinned")
    }

    fn assert_metadata_round_trip(space: AtomIndexSpace, atom: BinaryAtom, expected: &[u8]) {
        let mut writer = WireWriter::new(expected.len());
        space.encode_metadata_atom(&mut writer, atom).unwrap();
        assert_eq!(writer.as_bytes(), expected);

        let mut cursor = strict_cursor(expected);
        assert_eq!(space.decode_metadata_atom(&mut cursor), Ok(atom));
        cursor.finish().unwrap();
    }

    #[test]
    fn modes_define_distinct_explicit_index_spaces() {
        let data = AtomIndexSpace::new(BinaryObjectMode::Data, 2).unwrap();
        assert_eq!(data.mode(), BinaryObjectMode::Data);
        assert_eq!(data.first_atom(), 1);
        assert_eq!(data.header_count(), 2);

        let bytecode = AtomIndexSpace::new(BinaryObjectMode::Bytecode, 2).unwrap();
        assert_eq!(bytecode.mode(), BinaryObjectMode::Bytecode);
        assert_eq!(bytecode.first_atom(), FIRST_DYNAMIC_ATOM);
        assert_eq!(bytecode.header_count(), 2);
    }

    #[test]
    fn metadata_codec_covers_null_integer_predefined_and_header_atoms() {
        let space = AtomIndexSpace::new(BinaryObjectMode::Bytecode, 2).unwrap();
        let first = space.header_slot(0).unwrap();
        let second = space.header_slot(1).unwrap();
        assert_eq!(first.index(), 0);
        assert_eq!(second.index(), 1);
        assert_eq!(space.header_slot(2), None);

        assert_metadata_round_trip(space, BinaryAtom::Null, &[0]);
        assert_metadata_round_trip(space, BinaryAtom::Index(42), &[85]);
        assert_metadata_round_trip(space, BinaryAtom::Predefined(pinned(4)), &[8]);
        assert_metadata_round_trip(space, BinaryAtom::Header(first), &[0xe6, 0x03]);
        assert_metadata_round_trip(space, BinaryAtom::Header(second), &[0xe8, 0x03]);
    }

    #[test]
    fn opcode_codec_uses_raw_high_bit_integer_tag() {
        let space = AtomIndexSpace::new(BinaryObjectMode::Bytecode, 1).unwrap();
        let header = BinaryAtom::Header(space.header_slot(0).unwrap());
        let cases = [
            (BinaryAtom::Null, 0),
            (BinaryAtom::Index(42), ATOM_TAG_INT | 42),
            (BinaryAtom::Predefined(pinned(4)), 4),
            (header, FIRST_DYNAMIC_ATOM),
        ];
        for (atom, raw) in cases {
            assert_eq!(space.encode_opcode_atom(atom, 9), Ok(raw));
            assert_eq!(space.resolve_opcode_atom(raw, 9), Ok(atom));
        }

        // The same numeric 85 means integer 42 in metadata, but predefined
        // atom 85 when stored as a raw opcode operand.
        let mut metadata = strict_cursor(&[85]);
        assert_eq!(
            space.decode_metadata_atom(&mut metadata),
            Ok(BinaryAtom::Index(42))
        );
        assert_eq!(
            space.resolve_opcode_atom(85, 0),
            Ok(BinaryAtom::Predefined(pinned(85)))
        );
    }

    #[test]
    fn integer_and_table_length_boundaries_are_canonical() {
        let data = AtomIndexSpace::new(BinaryObjectMode::Data, ATOM_MAX_TABLE_INDEX).unwrap();
        let last_slot = data.header_slot(ATOM_MAX_TABLE_INDEX - 1).unwrap();
        assert_metadata_round_trip(
            data,
            BinaryAtom::Header(last_slot),
            &[0xfe, 0xff, 0xff, 0xff, 0x07],
        );
        assert_metadata_round_trip(
            data,
            BinaryAtom::Index(ATOM_MAX_INT),
            &[0xff, 0xff, 0xff, 0xff, 0x0f],
        );
        assert_eq!(
            data.encode_opcode_atom(BinaryAtom::Header(last_slot), 0),
            Ok(ATOM_MAX_TABLE_INDEX)
        );
        assert_eq!(
            data.encode_opcode_atom(BinaryAtom::Index(ATOM_MAX_INT), 0),
            Ok(u32::MAX)
        );
    }

    #[test]
    fn index_space_rejects_a_header_range_past_quickjs_table_limit() {
        let data_overflow = ATOM_MAX_TABLE_INDEX + 1;
        assert_eq!(
            AtomIndexSpace::new(BinaryObjectMode::Data, data_overflow),
            Err(WireError::AtomIndexSpaceOverflow {
                first_atom: 1,
                atom_count: data_overflow,
                maximum: ATOM_MAX_TABLE_INDEX,
            })
        );

        let bytecode_max = ATOM_MAX_TABLE_INDEX - FIRST_DYNAMIC_ATOM + 1;
        let bytecode = AtomIndexSpace::new(BinaryObjectMode::Bytecode, bytecode_max).unwrap();
        let last_slot = bytecode.header_slot(bytecode_max - 1).unwrap();
        assert_eq!(
            bytecode.encode_opcode_atom(BinaryAtom::Header(last_slot), 0),
            Ok(ATOM_MAX_TABLE_INDEX)
        );
        assert_eq!(
            AtomIndexSpace::new(BinaryObjectMode::Bytecode, bytecode_max + 1),
            Err(WireError::AtomIndexSpaceOverflow {
                first_atom: FIRST_DYNAMIC_ATOM,
                atom_count: bytecode_max + 1,
                maximum: ATOM_MAX_TABLE_INDEX,
            })
        );
    }

    #[test]
    fn mode_is_never_inferred_from_an_atom_value() {
        let data = AtomIndexSpace::new(BinaryObjectMode::Data, 1).unwrap();
        let bytecode = AtomIndexSpace::new(BinaryObjectMode::Bytecode, 1).unwrap();

        // Raw index one is a data header slot but pinned atom `null` in
        // bytecode mode. Both are valid; no codec may guess which was meant.
        let mut data_cursor = strict_cursor(&[2]);
        assert_eq!(
            data.decode_metadata_atom(&mut data_cursor),
            Ok(BinaryAtom::Header(data.header_slot(0).unwrap()))
        );
        let mut bytecode_cursor = strict_cursor(&[2]);
        assert_eq!(
            bytecode.decode_metadata_atom(&mut bytecode_cursor),
            Ok(BinaryAtom::Predefined(pinned(1)))
        );

        // Conversely, bytecode's first dynamic index is outside an empty data
        // header instead of triggering an automatic mode switch.
        let empty_data = AtomIndexSpace::new(BinaryObjectMode::Data, 0).unwrap();
        let mut dynamic = strict_cursor(&[0xe6, 0x03]);
        assert_eq!(
            empty_data.decode_metadata_atom(&mut dynamic),
            Err(WireError::InvalidAtomIndex {
                offset: 2,
                index: FIRST_DYNAMIC_ATOM,
                first_atom: 1,
                atom_count: 0,
            })
        );
    }

    #[test]
    fn wrong_mode_and_cross_space_values_fail_before_output_mutation() {
        let data = AtomIndexSpace::new(BinaryObjectMode::Data, 0).unwrap();
        let mut writer = WireWriter::new(8);
        writer.write_u8(0xaa).unwrap();
        assert_eq!(
            data.encode_metadata_atom(&mut writer, BinaryAtom::Predefined(pinned(4))),
            Err(WireError::InvalidAtomIndex {
                offset: 1,
                index: 4,
                first_atom: 1,
                atom_count: 0,
            })
        );
        assert_eq!(writer.as_bytes(), &[0xaa]);

        let larger = AtomIndexSpace::new(BinaryObjectMode::Data, 2).unwrap();
        let foreign_slot = larger.header_slot(1).unwrap();
        assert_eq!(
            data.encode_opcode_atom(BinaryAtom::Header(foreign_slot), 17),
            Err(WireError::InvalidAtomIndex {
                offset: 17,
                index: 2,
                first_atom: 1,
                atom_count: 0,
            })
        );
    }

    #[test]
    fn invalid_indices_preserve_metadata_and_opcode_offsets() {
        let space = AtomIndexSpace::new(BinaryObjectMode::Data, 0).unwrap();
        let mut cursor = strict_cursor(&[0xaa, 2]);
        cursor.read_u8().unwrap();
        assert_eq!(
            space.decode_metadata_atom(&mut cursor),
            Err(WireError::InvalidAtomIndex {
                offset: 2,
                index: 1,
                first_atom: 1,
                atom_count: 0,
            })
        );
        assert_eq!(cursor.position(), 2);

        assert_eq!(
            space.resolve_opcode_atom(ATOM_MAX_TABLE_INDEX, 37),
            Err(WireError::InvalidAtomIndex {
                offset: 37,
                index: ATOM_MAX_TABLE_INDEX,
                first_atom: 1,
                atom_count: 0,
            })
        );
        assert_eq!(
            space.encode_opcode_atom(BinaryAtom::Index(ATOM_MAX_INT + 1), 41),
            Err(WireError::InvalidAtomIndex {
                offset: 41,
                index: ATOM_MAX_INT + 1,
                first_atom: 1,
                atom_count: 0,
            })
        );
    }

    #[test]
    fn metadata_wire_errors_precede_atom_resolution() {
        let space = AtomIndexSpace::new(BinaryObjectMode::Data, 0).unwrap();
        let mut noncanonical = strict_cursor(&[0x82, 0x00]);
        assert_eq!(
            space.decode_metadata_atom(&mut noncanonical),
            Err(WireError::NonCanonicalUleb128 { offset: 0 })
        );
        assert_eq!(noncanonical.position(), 0);

        let mut compatible =
            WireCursor::new(&[0x82, 0x00], ReaderMode::QuickJsCompatible, TEST_LIMITS).unwrap();
        assert_eq!(
            space.decode_metadata_atom(&mut compatible),
            Err(WireError::InvalidAtomIndex {
                offset: 2,
                index: 1,
                first_atom: 1,
                atom_count: 0,
            })
        );

        let mut truncated = strict_cursor(&[0x80]);
        assert_eq!(
            space.decode_metadata_atom(&mut truncated),
            Err(WireError::Truncated {
                offset: 1,
                needed: 1,
                remaining: 0,
            })
        );
    }

    #[test]
    fn metadata_writer_honors_output_limit_without_partial_append() {
        let space = AtomIndexSpace::new(BinaryObjectMode::Data, 0).unwrap();
        let mut writer = WireWriter::new(0);
        assert_eq!(
            space.encode_metadata_atom(&mut writer, BinaryAtom::Null),
            Err(WireError::ResourceLimit {
                kind: ResourceKind::OutputBytes,
                requested: 1,
                limit: 0,
            })
        );
        assert!(writer.as_bytes().is_empty());
    }

    #[test]
    fn atom_errors_have_stable_diagnostic_text() {
        let overflow = WireError::AtomIndexSpaceOverflow {
            first_atom: FIRST_DYNAMIC_ATOM,
            atom_count: ATOM_MAX_TABLE_INDEX,
            maximum: ATOM_MAX_TABLE_INDEX,
        };
        assert_eq!(
            overflow.to_string(),
            "atom index space starting at 243 with 1073741823 header atoms exceeds maximum table index 1073741823"
        );

        let invalid = WireError::InvalidAtomIndex {
            offset: 7,
            index: 244,
            first_atom: FIRST_DYNAMIC_ATOM,
            atom_count: 1,
        };
        assert_eq!(
            invalid.to_string(),
            "invalid atom index 244 at byte 7 (first atom 243, atom count 1)"
        );
    }
}
