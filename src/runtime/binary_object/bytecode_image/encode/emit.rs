//! Length calculation and final emission for an authenticated bytecode-image plan.

use std::collections::HashMap;

use crate::atom::ATOM_MAX_INT;

use super::super::super::atoms::{AtomIndexSpace, BinaryAtom, HeaderAtomSlot};
use super::super::super::graph::model::AtomId;
use super::super::super::pinned_atoms::FIRST_DYNAMIC_ATOM;
use super::super::super::wire::{WireString, WireWriter};
use super::super::atoms::ImageAtom;
use super::super::model::{FunctionId, ImageCode};
use super::BytecodeImageEncodeError;
use super::plan::{AuthenticatedBytecodeImage, PlannedToken};

pub(super) fn encode_authenticated(
    proof: AuthenticatedBytecodeImage<'_>,
) -> Result<Vec<u8>, BytecodeImageEncodeError> {
    let AuthenticatedBytecodeImage {
        image: _,
        options: _,
        atoms,
        dynamic_slots,
        atom_space,
        tokens,
        encoded_length,
    } = proof;
    let mut writer = WireWriter::new(encoded_length);
    writer.write_header(atom_space.header_count())?;
    for atom in atoms {
        writer.write_string(atom)?;
    }
    for token in tokens {
        match token {
            PlannedToken::U8(value) => writer.write_u8(value)?,
            PlannedToken::U16(value) => writer.write_u16_le(value)?,
            PlannedToken::Uleb(value) => writer.write_uleb128(value)?,
            PlannedToken::I32(value) => writer.write_i32(value)?,
            PlannedToken::F64(bits) => writer.write_f64(f64::from_bits(bits))?,
            PlannedToken::String(value) => writer.write_string(value)?,
            PlannedToken::Bytes(bytes) => writer.write_bytes(bytes)?,
            PlannedToken::Atom(atom) => atom_space.encode_metadata_atom(
                &mut writer,
                planned_binary_atom(atom, &dynamic_slots, atom_space)?,
            )?,
            PlannedToken::Code { function, code } => {
                let bytes = canonical_code_bytes(function, code, &dynamic_slots, atom_space)?;
                writer.write_bytes(&bytes)?;
            }
        }
    }
    let actual = writer.as_bytes().len();
    if actual != encoded_length {
        return Err(BytecodeImageEncodeError::EncodedLengthMismatch {
            planned: encoded_length,
            actual,
        });
    }
    Ok(writer.into_bytes())
}

fn canonical_code_bytes(
    function: FunctionId,
    code: &ImageCode,
    dynamic_slots: &HashMap<AtomId, u32>,
    atom_space: AtomIndexSpace,
) -> Result<Vec<u8>, BytecodeImageEncodeError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(code.as_bytes().len())
        .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
    output.extend_from_slice(code.as_bytes());
    for instruction in code.instructions() {
        let Some(raw) = output.get_mut(instruction.offset() as usize) else {
            return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
                function_index: function.zero_based(),
                offset: instruction.offset(),
            });
        };
        *raw = instruction.opcode().raw();
    }
    for relocation in code.atom_relocations() {
        let offset = relocation.operand_offset() as usize;
        let end = offset.checked_add(size_of::<u32>()).ok_or(
            BytecodeImageEncodeError::InvalidCodeSidecar {
                function_index: function.zero_based(),
                offset: relocation.operand_offset(),
            },
        )?;
        let Some(destination) = output.get_mut(offset..end) else {
            return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
                function_index: function.zero_based(),
                offset: relocation.operand_offset(),
            });
        };
        let atom = planned_binary_atom(relocation.atom(), dynamic_slots, atom_space)?;
        let raw = atom_space.encode_opcode_atom(atom, offset)?;
        destination.copy_from_slice(&raw.to_le_bytes());
    }
    Ok(output)
}

pub(super) fn encoded_plan_length(
    atoms: &[&WireString],
    dynamic_slots: &HashMap<AtomId, u32>,
    atom_space: AtomIndexSpace,
    tokens: &[PlannedToken<'_>],
) -> Result<usize, BytecodeImageEncodeError> {
    let mut length = 1usize;
    add_length(&mut length, uleb_length(atom_space.header_count()))?;
    for atom in atoms {
        add_length(&mut length, wire_string_length(atom)?)?;
    }
    for token in tokens {
        let token_length = match token {
            PlannedToken::U8(_) => 1,
            PlannedToken::U16(_) => 2,
            PlannedToken::Uleb(value) => uleb_length(*value),
            PlannedToken::I32(value) => uleb_length(zigzag_i32(*value)),
            PlannedToken::F64(_) => 8,
            PlannedToken::String(value) => wire_string_length(value)?,
            PlannedToken::Bytes(bytes) => bytes.len(),
            PlannedToken::Atom(atom) => {
                let raw =
                    metadata_atom_encoding(planned_binary_atom(*atom, dynamic_slots, atom_space)?)?;
                uleb_length(raw)
            }
            PlannedToken::Code { code, .. } => code.as_bytes().len(),
        };
        add_length(&mut length, token_length)?;
    }
    Ok(length)
}

fn planned_binary_atom(
    atom: ImageAtom,
    dynamic_slots: &HashMap<AtomId, u32>,
    atom_space: AtomIndexSpace,
) -> Result<BinaryAtom, BytecodeImageEncodeError> {
    match atom {
        ImageAtom::Null => Ok(BinaryAtom::Null),
        ImageAtom::Index(index) if index <= ATOM_MAX_INT => Ok(BinaryAtom::Index(index)),
        ImageAtom::Index(index) => Err(BytecodeImageEncodeError::IntegerAtomOutOfRange { index }),
        ImageAtom::Predefined(atom) => Ok(BinaryAtom::Predefined(atom)),
        ImageAtom::Dynamic(atom) => {
            let index = dynamic_slots.get(&atom).copied().ok_or(
                BytecodeImageEncodeError::DynamicAtomOutOfRange {
                    index: atom.zero_based(),
                    atom_count: dynamic_slots.len(),
                },
            )?;
            atom_space.header_slot(index).map(BinaryAtom::Header).ok_or(
                BytecodeImageEncodeError::DynamicAtomOutOfRange {
                    index: atom.zero_based(),
                    atom_count: dynamic_slots.len(),
                },
            )
        }
    }
}

fn metadata_atom_encoding(atom: BinaryAtom) -> Result<u32, BytecodeImageEncodeError> {
    match atom {
        BinaryAtom::Null => Ok(0),
        BinaryAtom::Index(index) if index <= ATOM_MAX_INT => Ok((index << 1) | 1),
        BinaryAtom::Index(index) => Err(BytecodeImageEncodeError::IntegerAtomOutOfRange { index }),
        BinaryAtom::Predefined(atom) => Ok(atom.raw() << 1),
        BinaryAtom::Header(slot) => header_atom_encoding(slot),
    }
}

fn header_atom_encoding(slot: HeaderAtomSlot) -> Result<u32, BytecodeImageEncodeError> {
    // AtomIndexSpace construction separately proves the complete header range.
    FIRST_DYNAMIC_ATOM
        .checked_add(slot.index())
        .and_then(|raw| raw.checked_shl(1))
        .ok_or(BytecodeImageEncodeError::EncodedLengthOverflow)
}

fn wire_string_length(value: &WireString) -> Result<usize, BytecodeImageEncodeError> {
    let units = value.len();
    let bytes = match value {
        WireString::Narrow(_) => units,
        WireString::Wide(_) => units
            .checked_mul(2)
            .ok_or(BytecodeImageEncodeError::EncodedLengthOverflow)?,
    };
    let encoded_units = u32::try_from(units)
        .ok()
        .and_then(|units| units.checked_shl(1))
        .map(|units| units | u32::from(value.is_wide()))
        .ok_or(BytecodeImageEncodeError::EncodedLengthOverflow)?;
    uleb_length(encoded_units)
        .checked_add(bytes)
        .ok_or(BytecodeImageEncodeError::EncodedLengthOverflow)
}

fn add_length(total: &mut usize, addend: usize) -> Result<(), BytecodeImageEncodeError> {
    *total = total
        .checked_add(addend)
        .ok_or(BytecodeImageEncodeError::EncodedLengthOverflow)?;
    Ok(())
}

const fn zigzag_i32(value: i32) -> u32 {
    ((value << 1) ^ (value >> 31)) as u32
}

const fn uleb_length(mut value: u32) -> usize {
    let mut length = 1usize;
    while value >= 0x80 {
        value >>= 7;
        length += 1;
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planned_string_lengths_match_emission_at_uleb_boundaries() {
        for units in [0, 1, 63, 64, 8_191, 8_192] {
            for value in [
                WireString::Narrow(vec![0; units].into_boxed_slice()),
                WireString::Wide(vec![0; units].into_boxed_slice()),
            ] {
                let planned = wire_string_length(&value).unwrap();
                let mut writer = WireWriter::new(planned);
                writer.write_string(&value).unwrap();
                assert_eq!(writer.as_bytes().len(), planned);
            }
        }
    }
}
