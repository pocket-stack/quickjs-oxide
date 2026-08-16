//! Bounded, heap-independent scanner for QuickJS 2026-06-04 function code.
//!
//! This module recognizes only the release-pinned native opcode framing and
//! atom relocations. [`CodeImage`] is deliberately not an execution format or
//! a semantic verification token: a later admission layer must still translate
//! every opcode into the engine's instruction contract and run its verifier.

use std::fmt;

use super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode};
use super::pinned_opcodes::{PINNED_OPCODE_COUNT, PinnedOpcode};
use super::wire::WireError;

const ATOM_OPERAND_BYTES: usize = size_of::<u32>();
const PINNED_ATOM_OPERAND_OFFSET: u8 = 1;

/// Explicit resource limits for one native-code payload.
///
/// There is intentionally no `Default`: callers must choose limits appropriate
/// for the binary-object trust boundary that owns the containing function.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct CodeLimits {
    max_bytes: usize,
    max_instructions: usize,
    max_atom_relocations: usize,
}

impl CodeLimits {
    #[must_use]
    pub(in crate::runtime) const fn new(
        max_bytes: usize,
        max_instructions: usize,
        max_atom_relocations: usize,
    ) -> Self {
        Self {
            max_bytes,
            max_instructions,
            max_atom_relocations,
        }
    }
}

/// Independently budgeted resources consumed by the code scanner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum CodeResourceKind {
    Bytes,
    Instructions,
    AtomRelocations,
}

/// A structural failure while copying, scanning, or re-encoding native code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum CodeError {
    InvalidAtomMode {
        found: BinaryObjectMode,
    },
    ResourceLimit {
        kind: CodeResourceKind,
        requested: usize,
        limit: usize,
    },
    CountOverflow {
        kind: CodeResourceKind,
    },
    OffsetOverflow {
        offset: usize,
        addend: usize,
    },
    RelativeOffsetOverflow {
        offset: usize,
        maximum: u32,
    },
    InvalidOpcode {
        offset: usize,
        opcode: u8,
    },
    InvalidOpcodeLayout {
        offset: usize,
        opcode: u8,
        size: u8,
        atom_operand_offset: Option<u8>,
    },
    TruncatedInstruction {
        offset: usize,
        opcode: u8,
        needed: usize,
        remaining: usize,
    },
    InvalidAtomIndex {
        offset: usize,
        index: u32,
        first_atom: u32,
        atom_count: u32,
    },
    AtomCodecInvariant,
    InvalidSidecar {
        offset: u32,
    },
    AllocationFailed,
}

impl fmt::Display for CodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAtomMode { found } => write!(
                formatter,
                "function code requires the bytecode atom namespace, found {found:?}"
            ),
            Self::ResourceLimit {
                kind,
                requested,
                limit,
            } => write!(
                formatter,
                "{kind:?} code resource limit exceeded: requested {requested}, limit {limit}"
            ),
            Self::CountOverflow { kind } => {
                write!(formatter, "{kind:?} code resource count overflowed")
            }
            Self::OffsetOverflow { offset, addend } => write!(
                formatter,
                "native-code offset {offset} plus {addend} bytes overflowed"
            ),
            Self::RelativeOffsetOverflow { offset, maximum } => write!(
                formatter,
                "native-code relative offset {offset} exceeds {maximum}"
            ),
            Self::InvalidOpcode { offset, opcode } => {
                write!(formatter, "invalid native opcode {opcode} at byte {offset}")
            }
            Self::InvalidOpcodeLayout {
                offset,
                opcode,
                size,
                atom_operand_offset,
            } => write!(
                formatter,
                "invalid pinned layout for opcode {opcode} at byte {offset}: size {size}, atom operand {atom_operand_offset:?}"
            ),
            Self::TruncatedInstruction {
                offset,
                opcode,
                needed,
                remaining,
            } => write!(
                formatter,
                "native opcode {opcode} at byte {offset} needs {needed} bytes, {remaining} remain"
            ),
            Self::InvalidAtomIndex {
                offset,
                index,
                first_atom,
                atom_count,
            } => write!(
                formatter,
                "invalid native-code atom index {index} at byte {offset} (first atom {first_atom}, atom count {atom_count})"
            ),
            Self::AtomCodecInvariant => {
                formatter.write_str("native-code atom codec returned an unexpected error")
            }
            Self::InvalidSidecar { offset } => write!(
                formatter,
                "native-code sidecar points outside its owned bytes at relative byte {offset}"
            ),
            Self::AllocationFailed => formatter.write_str("native-code allocation failed"),
        }
    }
}

impl std::error::Error for CodeError {}

/// One instruction boundary in the owned native-code payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct InstructionSpan {
    offset: u32,
    opcode: PinnedOpcode,
}

impl InstructionSpan {
    /// Return the instruction's byte offset relative to the code payload.
    #[must_use]
    pub(in crate::runtime) const fn offset(self) -> u32 {
        self.offset
    }

    /// Return the typed, release-pinned opcode at this boundary.
    #[must_use]
    pub(in crate::runtime) const fn opcode(self) -> PinnedOpcode {
        self.opcode
    }
}

/// One semantic atom operand detached from its raw runtime atom spelling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct AtomRelocation {
    operand_offset: u32,
    atom: BinaryAtom,
}

impl AtomRelocation {
    /// Return the operand's byte offset relative to the code payload.
    #[must_use]
    pub(in crate::runtime) const fn operand_offset(self) -> u32 {
        self.operand_offset
    }

    /// Return the heap-independent atom resolved in this image's namespace.
    #[must_use]
    pub(in crate::runtime) const fn atom(self) -> BinaryAtom {
        self.atom
    }
}

/// Owned native bytes plus structural instruction and atom-relocation sidecars.
///
/// The fields are private so consumers cannot silently detach the semantic
/// sidecars from their byte payload. This remains non-executable even after a
/// successful scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct CodeImage {
    bytes: Vec<u8>,
    instructions: Vec<InstructionSpan>,
    atom_relocations: Vec<AtomRelocation>,
    atom_space: AtomIndexSpace,
    payload_offset: usize,
}

impl CodeImage {
    /// Copy and structurally scan a complete native-code payload.
    ///
    /// The input is size-checked and copied in full before the first opcode is
    /// inspected. Consequently an invalid atom relocation reports the cursor
    /// at `payload_offset + payload.len()`, matching pinned QuickJS's phase
    /// order after it consumes the complete bytecode payload. A data-object
    /// atom namespace is rejected before either step because function code is
    /// meaningful only under QuickJS's bytecode flag.
    pub(in crate::runtime) fn scan(
        payload: &[u8],
        atom_space: AtomIndexSpace,
        payload_offset: usize,
        limits: CodeLimits,
    ) -> Result<Self, CodeError> {
        if atom_space.mode() != BinaryObjectMode::Bytecode {
            return Err(CodeError::InvalidAtomMode {
                found: atom_space.mode(),
            });
        }
        if payload.len() > limits.max_bytes {
            return Err(CodeError::ResourceLimit {
                kind: CodeResourceKind::Bytes,
                requested: payload.len(),
                limit: limits.max_bytes,
            });
        }
        if payload.len() > u32::MAX as usize {
            return Err(CodeError::RelativeOffsetOverflow {
                offset: payload.len(),
                maximum: u32::MAX,
            });
        }
        let diagnostic_end = checked_absolute_offset(payload_offset, payload.len())?;

        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(payload.len())
            .map_err(|_| CodeError::AllocationFailed)?;
        bytes.extend_from_slice(payload);

        let mut instructions = Vec::new();
        let mut atom_relocations = Vec::new();
        let mut relative_offset = 0_usize;

        while relative_offset < bytes.len() {
            let absolute_offset = checked_absolute_offset(payload_offset, relative_offset)?;
            let raw = bytes[relative_offset];
            if raw == 0 || usize::from(raw) >= PINNED_OPCODE_COUNT {
                return Err(CodeError::InvalidOpcode {
                    offset: absolute_offset,
                    opcode: raw,
                });
            }
            let Some(opcode) = PinnedOpcode::from_byte(raw) else {
                return Err(CodeError::InvalidOpcode {
                    offset: absolute_offset,
                    opcode: raw,
                });
            };

            let size = usize::from(opcode.size());
            let atom_operand_offset = opcode.atom_operand_offset();
            if size == 0
                || atom_operand_offset.is_some_and(|offset| {
                    offset != PINNED_ATOM_OPERAND_OFFSET
                        || usize::from(offset)
                            .checked_add(ATOM_OPERAND_BYTES)
                            .is_none_or(|end| end > size)
                })
            {
                return Err(CodeError::InvalidOpcodeLayout {
                    offset: absolute_offset,
                    opcode: raw,
                    size: opcode.size(),
                    atom_operand_offset,
                });
            }

            let instruction_end =
                relative_offset
                    .checked_add(size)
                    .ok_or(CodeError::OffsetOverflow {
                        offset: relative_offset,
                        addend: size,
                    })?;
            let Some(instruction_bytes) = bytes.get(relative_offset..instruction_end) else {
                return Err(CodeError::TruncatedInstruction {
                    offset: absolute_offset,
                    opcode: raw,
                    needed: size,
                    remaining: bytes.len() - relative_offset,
                });
            };

            push_instruction(
                &mut instructions,
                InstructionSpan {
                    offset: checked_relative_offset(relative_offset)?,
                    opcode,
                },
                limits.max_instructions,
            )?;

            if let Some(operand_delta) = atom_operand_offset {
                let operand_delta = usize::from(operand_delta);
                // The catalog layout check above proves this range is present
                // in the complete instruction slice.
                let operand_end = operand_delta.checked_add(ATOM_OPERAND_BYTES).ok_or(
                    CodeError::OffsetOverflow {
                        offset: operand_delta,
                        addend: ATOM_OPERAND_BYTES,
                    },
                )?;
                let raw_atom = u32::from_le_bytes(
                    instruction_bytes[operand_delta..operand_end]
                        .try_into()
                        .map_err(|_| CodeError::InvalidOpcodeLayout {
                            offset: absolute_offset,
                            opcode: raw,
                            size: opcode.size(),
                            atom_operand_offset,
                        })?,
                );
                let atom = atom_space
                    .resolve_opcode_atom(raw_atom, diagnostic_end)
                    .map_err(map_atom_error)?;
                let operand_offset = relative_offset.checked_add(operand_delta).ok_or(
                    CodeError::OffsetOverflow {
                        offset: relative_offset,
                        addend: operand_delta,
                    },
                )?;
                push_atom_relocation(
                    &mut atom_relocations,
                    AtomRelocation {
                        operand_offset: checked_relative_offset(operand_offset)?,
                        atom,
                    },
                    limits.max_atom_relocations,
                )?;
            }

            relative_offset = instruction_end;
        }

        Ok(Self {
            bytes,
            instructions,
            atom_relocations,
            atom_space,
            payload_offset,
        })
    }

    /// Return the canonical little-endian bytes retained by this image.
    #[must_use]
    pub(in crate::runtime) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return every checked native instruction boundary.
    #[must_use]
    pub(in crate::runtime) fn instructions(&self) -> &[InstructionSpan] {
        &self.instructions
    }

    /// Return every semantic atom operand in instruction order.
    #[must_use]
    pub(in crate::runtime) fn atom_relocations(&self) -> &[AtomRelocation] {
        &self.atom_relocations
    }

    /// Re-encode in the exact atom namespace used during scanning.
    ///
    /// Opcode bytes and atom operands are regenerated from typed sidecars. The
    /// original raw bytes are therefore not trusted if they ever conflict with
    /// those sidecars; all other fixed-width operands retain their original
    /// canonical little-endian spelling.
    pub(in crate::runtime) fn canonical_bytes(&self) -> Result<Vec<u8>, CodeError> {
        let mut output = Vec::new();
        output
            .try_reserve_exact(self.bytes.len())
            .map_err(|_| CodeError::AllocationFailed)?;
        output.extend_from_slice(&self.bytes);

        for instruction in &self.instructions {
            let offset = instruction.offset as usize;
            let Some(raw) = output.get_mut(offset) else {
                return Err(CodeError::InvalidSidecar {
                    offset: instruction.offset,
                });
            };
            *raw = instruction.opcode.raw();
        }

        for relocation in &self.atom_relocations {
            let relative_offset = relocation.operand_offset as usize;
            let absolute_offset = checked_absolute_offset(self.payload_offset, relative_offset)?;
            let raw = self
                .atom_space
                .encode_opcode_atom(relocation.atom, absolute_offset)
                .map_err(map_atom_error)?;
            let end = relative_offset.checked_add(ATOM_OPERAND_BYTES).ok_or(
                CodeError::OffsetOverflow {
                    offset: relative_offset,
                    addend: ATOM_OPERAND_BYTES,
                },
            )?;
            let Some(destination) = output.get_mut(relative_offset..end) else {
                return Err(CodeError::InvalidSidecar {
                    offset: relocation.operand_offset,
                });
            };
            destination.copy_from_slice(&raw.to_le_bytes());
        }

        Ok(output)
    }
}

fn checked_absolute_offset(offset: usize, addend: usize) -> Result<usize, CodeError> {
    offset
        .checked_add(addend)
        .ok_or(CodeError::OffsetOverflow { offset, addend })
}

fn checked_relative_offset(offset: usize) -> Result<u32, CodeError> {
    u32::try_from(offset).map_err(|_| CodeError::RelativeOffsetOverflow {
        offset,
        maximum: u32::MAX,
    })
}

fn push_instruction(
    instructions: &mut Vec<InstructionSpan>,
    instruction: InstructionSpan,
    limit: usize,
) -> Result<(), CodeError> {
    let requested = instructions
        .len()
        .checked_add(1)
        .ok_or(CodeError::CountOverflow {
            kind: CodeResourceKind::Instructions,
        })?;
    if requested > limit {
        return Err(CodeError::ResourceLimit {
            kind: CodeResourceKind::Instructions,
            requested,
            limit,
        });
    }
    instructions
        .try_reserve(1)
        .map_err(|_| CodeError::AllocationFailed)?;
    instructions.push(instruction);
    Ok(())
}

fn push_atom_relocation(
    relocations: &mut Vec<AtomRelocation>,
    relocation: AtomRelocation,
    limit: usize,
) -> Result<(), CodeError> {
    let requested = relocations
        .len()
        .checked_add(1)
        .ok_or(CodeError::CountOverflow {
            kind: CodeResourceKind::AtomRelocations,
        })?;
    if requested > limit {
        return Err(CodeError::ResourceLimit {
            kind: CodeResourceKind::AtomRelocations,
            requested,
            limit,
        });
    }
    relocations
        .try_reserve(1)
        .map_err(|_| CodeError::AllocationFailed)?;
    relocations.push(relocation);
    Ok(())
}

fn map_atom_error(error: WireError) -> CodeError {
    match error {
        WireError::InvalidAtomIndex {
            offset,
            index,
            first_atom,
            atom_count,
        } => CodeError::InvalidAtomIndex {
            offset,
            index,
            first_atom,
            atom_count,
        },
        _ => CodeError::AtomCodecInvariant,
    }
}

#[cfg(test)]
mod tests {
    use crate::atom::ATOM_TAG_INT;

    use super::super::atoms::BinaryObjectMode;
    use super::super::pinned_atoms::PinnedAtomId;
    use super::*;

    const TEST_LIMITS: CodeLimits = CodeLimits::new(1 << 20, 1_000, 100);

    fn atom_space(header_count: u32) -> AtomIndexSpace {
        AtomIndexSpace::new(BinaryObjectMode::Bytecode, header_count).unwrap()
    }

    fn first_atom_opcode() -> PinnedOpcode {
        (1..PINNED_OPCODE_COUNT)
            .find_map(|raw| {
                PinnedOpcode::from_byte(raw as u8).filter(|opcode| opcode.has_atom_operand())
            })
            .expect("the pinned catalog must contain atom opcodes")
    }

    fn first_plain_opcode() -> PinnedOpcode {
        (1..PINNED_OPCODE_COUNT)
            .find_map(|raw| {
                PinnedOpcode::from_byte(raw as u8).filter(|opcode| !opcode.has_atom_operand())
            })
            .expect("the pinned catalog must contain non-atom opcodes")
    }

    fn instruction_bytes(opcode: PinnedOpcode, raw_atom: Option<u32>) -> Vec<u8> {
        let mut bytes = vec![0; usize::from(opcode.size())];
        bytes[0] = opcode.raw();
        if let Some(offset) = opcode.atom_operand_offset() {
            let offset = usize::from(offset);
            let raw_atom = raw_atom.unwrap_or(0).to_le_bytes();
            bytes[offset..offset + raw_atom.len()].copy_from_slice(&raw_atom);
        } else {
            assert!(raw_atom.is_none());
        }
        bytes
    }

    #[test]
    fn empty_payload_is_a_valid_non_executable_image() {
        let image = CodeImage::scan(&[], atom_space(0), 73, CodeLimits::new(0, 0, 0)).unwrap();
        assert!(image.as_bytes().is_empty());
        assert!(image.instructions().is_empty());
        assert!(image.atom_relocations().is_empty());
        assert_eq!(image.canonical_bytes().unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn rejects_the_data_atom_namespace_before_ambiguous_resolution() {
        let opcode = first_atom_opcode();
        let payload = instruction_bytes(opcode, Some(1));
        let data_space = AtomIndexSpace::new(BinaryObjectMode::Data, 1).unwrap();
        let expected = Err(CodeError::InvalidAtomMode {
            found: BinaryObjectMode::Data,
        });

        assert_eq!(
            CodeImage::scan(&payload, data_space, 0, TEST_LIMITS),
            expected
        );
        assert_eq!(
            CodeImage::scan(&[], data_space, 0, CodeLimits::new(0, 0, 0)),
            expected
        );
    }

    #[test]
    fn rejects_reserved_and_out_of_range_opcodes() {
        assert_eq!(
            CodeImage::scan(&[0], atom_space(0), 9, TEST_LIMITS),
            Err(CodeError::InvalidOpcode {
                offset: 9,
                opcode: 0,
            })
        );
        assert_eq!(
            CodeImage::scan(&[244], atom_space(0), 12, TEST_LIMITS),
            Err(CodeError::InvalidOpcode {
                offset: 12,
                opcode: 244,
            })
        );
    }

    #[test]
    fn rejects_truncated_instruction_at_its_absolute_offset() {
        let opcode = first_atom_opcode();
        let mut bytes = instruction_bytes(opcode, Some(0));
        bytes.pop();
        assert_eq!(
            CodeImage::scan(&bytes, atom_space(0), 41, TEST_LIMITS),
            Err(CodeError::TruncatedInstruction {
                offset: 41,
                opcode: opcode.raw(),
                needed: usize::from(opcode.size()),
                remaining: bytes.len(),
            })
        );
    }

    #[test]
    fn rejects_an_absolute_payload_end_that_cannot_be_represented() {
        let payload = instruction_bytes(first_plain_opcode(), None);

        assert_eq!(
            CodeImage::scan(&payload, atom_space(0), usize::MAX, TEST_LIMITS),
            Err(CodeError::OffsetOverflow {
                offset: usize::MAX,
                addend: payload.len(),
            })
        );
    }

    #[test]
    fn scans_mixed_instruction_boundaries_and_atom_operands() {
        let plain = first_plain_opcode();
        let atom = first_atom_opcode();
        let plain_bytes = instruction_bytes(plain, None);
        let atom_bytes = instruction_bytes(atom, Some(1));
        let mut payload = Vec::new();
        payload.extend_from_slice(&plain_bytes);
        payload.extend_from_slice(&atom_bytes);
        payload.extend_from_slice(&plain_bytes);

        let image = CodeImage::scan(&payload, atom_space(0), 100, TEST_LIMITS).unwrap();
        assert_eq!(image.instructions().len(), 3);
        assert_eq!(image.instructions()[0].offset(), 0);
        assert_eq!(
            image.instructions()[1].offset(),
            u32::try_from(plain_bytes.len()).unwrap()
        );
        assert_eq!(
            image.instructions()[2].offset(),
            u32::try_from(plain_bytes.len() + atom_bytes.len()).unwrap()
        );
        assert_eq!(image.instructions()[1].opcode(), atom);
        assert_eq!(image.atom_relocations().len(), 1);
        assert_eq!(
            image.atom_relocations()[0].operand_offset(),
            u32::try_from(plain_bytes.len() + 1).unwrap()
        );
        assert_eq!(
            image.atom_relocations()[0].atom(),
            BinaryAtom::Predefined(PinnedAtomId::from_raw(1).unwrap())
        );
    }

    #[test]
    fn resolves_predefined_header_and_tagged_integer_atoms() {
        let opcode = first_atom_opcode();
        let space = atom_space(1);
        let raw_atoms = [1, space.first_atom(), ATOM_TAG_INT | 42];
        let mut payload = Vec::new();
        for raw in raw_atoms {
            payload.extend_from_slice(&instruction_bytes(opcode, Some(raw)));
        }

        let image = CodeImage::scan(&payload, space, 0, TEST_LIMITS).unwrap();
        assert_eq!(
            image
                .atom_relocations()
                .iter()
                .map(|relocation| relocation.atom())
                .collect::<Vec<_>>(),
            vec![
                BinaryAtom::Predefined(PinnedAtomId::from_raw(1).unwrap()),
                BinaryAtom::Header(space.header_slot(0).unwrap()),
                BinaryAtom::Index(42),
            ]
        );
    }

    #[test]
    fn invalid_atom_diagnostic_uses_end_of_consumed_payload() {
        let opcode = first_atom_opcode();
        let space = atom_space(1);
        let invalid = space.first_atom() + space.header_count();
        let payload = instruction_bytes(opcode, Some(invalid));
        let payload_offset = 500;

        assert_eq!(
            CodeImage::scan(&payload, space, payload_offset, TEST_LIMITS),
            Err(CodeError::InvalidAtomIndex {
                offset: payload_offset + payload.len(),
                index: invalid,
                first_atom: space.first_atom(),
                atom_count: space.header_count(),
            })
        );
    }

    #[test]
    fn enforces_independent_byte_instruction_and_relocation_limits() {
        let plain_payload = instruction_bytes(first_plain_opcode(), None);
        assert_eq!(
            CodeImage::scan(
                &plain_payload,
                atom_space(0),
                0,
                CodeLimits::new(plain_payload.len() - 1, 1, 0),
            ),
            Err(CodeError::ResourceLimit {
                kind: CodeResourceKind::Bytes,
                requested: plain_payload.len(),
                limit: plain_payload.len() - 1,
            })
        );
        assert_eq!(
            CodeImage::scan(
                &plain_payload,
                atom_space(0),
                0,
                CodeLimits::new(plain_payload.len(), 0, 0),
            ),
            Err(CodeError::ResourceLimit {
                kind: CodeResourceKind::Instructions,
                requested: 1,
                limit: 0,
            })
        );

        let atom_payload = instruction_bytes(first_atom_opcode(), Some(0));
        assert_eq!(
            CodeImage::scan(
                &atom_payload,
                atom_space(0),
                0,
                CodeLimits::new(atom_payload.len(), 1, 0),
            ),
            Err(CodeError::ResourceLimit {
                kind: CodeResourceKind::AtomRelocations,
                requested: 1,
                limit: 0,
            })
        );
    }

    #[test]
    fn canonical_reencode_uses_typed_sidecars_not_conflicting_raw_bytes() {
        let opcode = first_atom_opcode();
        let space = atom_space(1);
        let payload = instruction_bytes(opcode, Some(space.first_atom()));
        let mut image = CodeImage::scan(&payload, space, 17, TEST_LIMITS).unwrap();
        assert_eq!(image.canonical_bytes().unwrap(), payload);

        image.bytes[0] = 0;
        let operand = image.atom_relocations[0].operand_offset as usize;
        image.bytes[operand..operand + ATOM_OPERAND_BYTES].fill(0xff);

        assert_ne!(image.as_bytes(), payload);
        assert_eq!(image.canonical_bytes().unwrap(), payload);
    }

    #[test]
    fn every_pinned_catalog_entry_has_exact_framing() {
        let space = atom_space(0);
        let mut payload = Vec::new();
        let mut expected_offsets = Vec::new();
        let mut expected_atom_count = 0_usize;

        for raw in 1..PINNED_OPCODE_COUNT {
            let opcode = PinnedOpcode::from_byte(raw as u8)
                .expect("every byte from 1 through 243 is a pinned opcode");
            assert_eq!(opcode.raw(), raw as u8);
            assert_ne!(opcode.size(), 0);
            if opcode.has_atom_operand() {
                expected_atom_count += 1;
                assert_eq!(opcode.atom_operand_offset(), Some(1));
                assert!(usize::from(opcode.size()) >= 1 + ATOM_OPERAND_BYTES);
            } else {
                assert_eq!(opcode.atom_operand_offset(), None);
            }
            expected_offsets.push(u32::try_from(payload.len()).unwrap());
            payload.extend_from_slice(&instruction_bytes(opcode, None));
        }

        assert_eq!(expected_atom_count, 21);
        let limits = CodeLimits::new(payload.len(), PINNED_OPCODE_COUNT - 1, expected_atom_count);
        let image = CodeImage::scan(&payload, space, 31, limits).unwrap();
        assert_eq!(image.instructions().len(), PINNED_OPCODE_COUNT - 1);
        assert_eq!(image.atom_relocations().len(), expected_atom_count);
        assert_eq!(
            image
                .instructions()
                .iter()
                .map(|instruction| instruction.offset())
                .collect::<Vec<_>>(),
            expected_offsets
        );
        assert_eq!(image.canonical_bytes().unwrap(), payload);
        // Zero has a catalog descriptor for table parity, but `scan` rejects
        // it as the non-emittable `invalid` opcode.
        assert!(PinnedOpcode::from_byte(0).is_some());
        assert!(PinnedOpcode::from_byte(244).is_none());
    }
}
