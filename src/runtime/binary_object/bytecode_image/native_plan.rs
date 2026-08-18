//! Generic, non-executable operand plan for pinned QuickJS function bytecode.
//!
//! This is the last archive-only stage before an executable bridge may decide
//! which capabilities it can lower. It decodes every upstream operand format,
//! authenticates native byte-PC boundaries and relative labels, and replaces
//! every raw atom operand with a sealed semantic reference. It deliberately
//! has no executable engine objects or runtime-owned String representation.

use std::fmt;

use super::{BytecodeImage, FunctionId, ImageAtom, ImageCode};
use crate::runtime::binary_object::pinned_atoms::{FIRST_DYNAMIC_ATOM, PinnedAtomKind};
use crate::runtime::binary_object::pinned_opcodes::{OpcodeFormat, PinnedOpcode};
use crate::runtime::binary_object::wire::WireString;

/// Semantic identity class of one relocated native-code atom.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum NativeAtomClass {
    Null,
    Index,
    String,
    Private,
    Symbol,
}

/// Sealed semantic reference to one atom in the owning bytecode image.
///
/// The private fields keep header slots, release-pinned numeric atom IDs, and
/// image-local arena indices out of the executable bridge. Dynamic spellings
/// borrow the image, so they cannot be detached from the table which gives
/// their semantic identity meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) struct NativeAtomRef<'image> {
    kind: NativeAtomRefKind<'image>,
    from_input_atom_table: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeAtomRefKind<'image> {
    Null,
    Index(u32),
    Manifest {
        class: NativeAtomClass,
        spelling: &'static str,
    },
    Dynamic(&'image WireString),
}

impl<'image> NativeAtomRef<'image> {
    fn new(
        atom: ImageAtom,
        dynamic_atoms: &'image [WireString],
        from_input_atom_table: bool,
        operand_pc: u32,
    ) -> Result<Self, NativePlanError> {
        let kind = match atom {
            ImageAtom::Null => NativeAtomRefKind::Null,
            ImageAtom::Index(index) => NativeAtomRefKind::Index(index),
            ImageAtom::Predefined(atom) => NativeAtomRefKind::Manifest {
                class: match atom.kind() {
                    PinnedAtomKind::String => NativeAtomClass::String,
                    PinnedAtomKind::Private => NativeAtomClass::Private,
                    PinnedAtomKind::Symbol => NativeAtomClass::Symbol,
                },
                spelling: atom.spelling(),
            },
            ImageAtom::Dynamic(index) => {
                let Some(spelling) = dynamic_atoms.get(index.as_usize()) else {
                    return Err(NativePlanError::InvalidDynamicAtom {
                        operand_pc,
                        index: index.zero_based(),
                        atom_count: dynamic_atoms.len(),
                    });
                };
                NativeAtomRefKind::Dynamic(spelling)
            }
        };
        Ok(Self {
            kind,
            from_input_atom_table,
        })
    }

    /// Whether the native operand named one of this image's input atom slots.
    ///
    /// The slot number and raw atom spelling remain sealed inside the archive
    /// decoder. This bit preserves the provenance of aliases which QuickJS
    /// interns to a predefined String or tagged decimal identity.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn originates_from_input_atom_table(self) -> bool {
        self.from_input_atom_table
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn class(self) -> NativeAtomClass {
        match self.kind {
            NativeAtomRefKind::Null => NativeAtomClass::Null,
            NativeAtomRefKind::Index(_) => NativeAtomClass::Index,
            NativeAtomRefKind::Manifest { class, .. } => class,
            NativeAtomRefKind::Dynamic(_) => NativeAtomClass::String,
        }
    }

    /// Return the semantic tagged-integer property index, if this is one.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn index(self) -> Option<u32> {
        match self.kind {
            NativeAtomRefKind::Index(index) => Some(index),
            NativeAtomRefKind::Null
            | NativeAtomRefKind::Manifest { .. }
            | NativeAtomRefKind::Dynamic(_) => None,
        }
    }

    /// Return an ordinary predefined String spelling, without its raw atom ID.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn manifest_string(self) -> Option<&'static str> {
        match self.kind {
            NativeAtomRefKind::Manifest {
                class: NativeAtomClass::String,
                spelling,
            } => Some(spelling),
            NativeAtomRefKind::Null
            | NativeAtomRefKind::Index(_)
            | NativeAtomRefKind::Manifest { .. }
            | NativeAtomRefKind::Dynamic(_) => None,
        }
    }

    /// Return an image-owned ordinary String spelling, if this is one.
    #[must_use]
    pub(in crate::runtime::binary_object) fn dynamic_string(self) -> Option<&'image WireString> {
        match self.kind {
            NativeAtomRefKind::Dynamic(spelling) => Some(spelling),
            NativeAtomRefKind::Null
            | NativeAtomRefKind::Index(_)
            | NativeAtomRefKind::Manifest { .. } => None,
        }
    }

    /// Return the manifest description of a private or well-known Symbol atom.
    ///
    /// Callers must still use [`Self::class`] to preserve identity class; this
    /// description is never an ordinary String atom projection.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn identity_description(
        self,
    ) -> Option<&'static str> {
        match self.kind {
            NativeAtomRefKind::Manifest {
                class: NativeAtomClass::Private | NativeAtomClass::Symbol,
                spelling,
            } => Some(spelling),
            NativeAtomRefKind::Null
            | NativeAtomRefKind::Index(_)
            | NativeAtomRefKind::Manifest { .. }
            | NativeAtomRefKind::Dynamic(_) => None,
        }
    }
}

/// One validated native relative-label operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) struct NativeLabel {
    operand_pc: u32,
    displacement: i32,
    target_pc: u32,
    target_instruction: u32,
}

impl NativeLabel {
    #[must_use]
    pub(in crate::runtime::binary_object) const fn operand_pc(self) -> u32 {
        self.operand_pc
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn displacement(self) -> i32 {
        self.displacement
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn target_pc(self) -> u32 {
        self.target_pc
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn target_instruction(self) -> u32 {
        self.target_instruction
    }
}

/// Typed operands for every format declared by pinned `quickjs-opcode.h`.
///
/// Variants preserve the native width/family even when a later translator can
/// normalize several of them to one engine instruction. This prevents short
/// opcode arithmetic and label bases from leaking into that translator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum NativeOperands<'image> {
    None,
    NoneInt(i32),
    NoneLoc(u16),
    NoneArg(u16),
    NoneVarRef(u16),
    U8(u8),
    I8(i8),
    Loc8(u8),
    Const8(u8),
    Label8(NativeLabel),
    U16(u16),
    I16(i16),
    Label16(NativeLabel),
    NPop(u16),
    NPopX(u16),
    NPopU16 {
        pop_count: u16,
        environment: u16,
    },
    Loc(u16),
    Arg(u16),
    VarRef(u16),
    U32(u32),
    I32(i32),
    Const(u32),
    Label(NativeLabel),
    Atom(NativeAtomRef<'image>),
    AtomU8 {
        atom: NativeAtomRef<'image>,
        value: u8,
    },
    AtomU16 {
        atom: NativeAtomRef<'image>,
        value: u16,
    },
    AtomLabelU8 {
        atom: NativeAtomRef<'image>,
        label: NativeLabel,
        value: u8,
    },
    AtomLabelU16 {
        atom: NativeAtomRef<'image>,
        label: NativeLabel,
        value: u16,
    },
    LabelU16 {
        label: NativeLabel,
        value: u16,
    },
}

impl NativeOperands<'_> {
    #[must_use]
    pub(in crate::runtime::binary_object) const fn format(&self) -> OpcodeFormat {
        match self {
            Self::None => OpcodeFormat::None,
            Self::NoneInt(_) => OpcodeFormat::NoneInt,
            Self::NoneLoc(_) => OpcodeFormat::NoneLoc,
            Self::NoneArg(_) => OpcodeFormat::NoneArg,
            Self::NoneVarRef(_) => OpcodeFormat::NoneVarRef,
            Self::U8(_) => OpcodeFormat::U8,
            Self::I8(_) => OpcodeFormat::I8,
            Self::Loc8(_) => OpcodeFormat::Loc8,
            Self::Const8(_) => OpcodeFormat::Const8,
            Self::Label8(_) => OpcodeFormat::Label8,
            Self::U16(_) => OpcodeFormat::U16,
            Self::I16(_) => OpcodeFormat::I16,
            Self::Label16(_) => OpcodeFormat::Label16,
            Self::NPop(_) => OpcodeFormat::NPop,
            Self::NPopX(_) => OpcodeFormat::NPopX,
            Self::NPopU16 { .. } => OpcodeFormat::NPopU16,
            Self::Loc(_) => OpcodeFormat::Loc,
            Self::Arg(_) => OpcodeFormat::Arg,
            Self::VarRef(_) => OpcodeFormat::VarRef,
            Self::U32(_) => OpcodeFormat::U32,
            Self::I32(_) => OpcodeFormat::I32,
            Self::Const(_) => OpcodeFormat::Const,
            Self::Label(_) => OpcodeFormat::Label,
            Self::Atom(_) => OpcodeFormat::Atom,
            Self::AtomU8 { .. } => OpcodeFormat::AtomU8,
            Self::AtomU16 { .. } => OpcodeFormat::AtomU16,
            Self::AtomLabelU8 { .. } => OpcodeFormat::AtomLabelU8,
            Self::AtomLabelU16 { .. } => OpcodeFormat::AtomLabelU16,
            Self::LabelU16 { .. } => OpcodeFormat::LabelU16,
        }
    }
}

/// One universally decoded instruction at an authenticated native byte PC.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) struct NativeInstruction<'image> {
    byte_pc: u32,
    opcode: PinnedOpcode,
    operands: NativeOperands<'image>,
}

impl<'image> NativeInstruction<'image> {
    #[must_use]
    pub(in crate::runtime::binary_object) const fn byte_pc(&self) -> u32 {
        self.byte_pc
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn opcode(&self) -> PinnedOpcode {
        self.opcode
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn operands(&self) -> &NativeOperands<'image> {
        &self.operands
    }
}

/// Non-executable instruction/PC/atom plan for one authenticated function.
///
/// The function envelope and constant pool deliberately remain in the owning
/// [`BytecodeImage`]; this type is not a complete translation plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) struct NativeCodePlan<'image> {
    function: FunctionId,
    instructions: Box<[NativeInstruction<'image>]>,
    native_pc_map: Box<[u32]>,
}

impl<'image> NativeCodePlan<'image> {
    #[must_use]
    pub(in crate::runtime::binary_object) const fn function(&self) -> FunctionId {
        self.function
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn instructions(
        &self,
    ) -> &[NativeInstruction<'image>] {
        &self.instructions
    }

    /// Return native byte PCs in typed-instruction order.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn native_pc_map(&self) -> &[u32] {
        &self.native_pc_map
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn instruction_at_native_pc(
        &self,
        byte_pc: u32,
    ) -> Option<u32> {
        self.native_pc_map
            .binary_search(&byte_pc)
            .ok()
            .and_then(|index| u32::try_from(index).ok())
    }
}

/// Structural failure while deriving a generic native operand plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum NativePlanError {
    FunctionNotInImage,
    InvalidOpcode {
        instruction: usize,
        byte_pc: u32,
        opcode: u8,
    },
    InvalidFormatSize {
        byte_pc: u32,
        opcode: u8,
        format: OpcodeFormat,
        descriptor_size: u8,
        expected_size: u8,
    },
    InstructionBoundaryMismatch {
        instruction: usize,
        expected: u32,
        actual: u32,
    },
    OpcodeByteMismatch {
        byte_pc: u32,
        sidecar: u8,
        byte: u8,
    },
    CodeLengthMismatch {
        covered: u32,
        byte_length: u32,
    },
    CodeLengthOutOfRange {
        byte_length: usize,
    },
    AtomRelocationMismatch {
        relocation: usize,
        expected: Option<u32>,
        actual: Option<u32>,
    },
    InvalidDynamicAtom {
        operand_pc: u32,
        index: u32,
        atom_count: usize,
    },
    MissingAtomProjection {
        byte_pc: u32,
        opcode: u8,
        format: OpcodeFormat,
    },
    UnexpectedAtomProjection {
        byte_pc: u32,
        opcode: u8,
        format: OpcodeFormat,
    },
    InvalidImplicitOpcode {
        byte_pc: u32,
        opcode: u8,
        format: OpcodeFormat,
    },
    TruncatedOperand {
        byte_pc: u32,
        opcode: u8,
        operand_offset: u8,
        width: u8,
        instruction_size: usize,
    },
    LabelTargetOutOfRange {
        byte_pc: u32,
        operand_pc: u32,
        displacement: i32,
        byte_length: u32,
    },
    LabelTargetNotInstructionBoundary {
        byte_pc: u32,
        operand_pc: u32,
        displacement: i32,
        target_pc: u32,
    },
    OffsetOverflow {
        byte_pc: u32,
        operand_offset: u8,
    },
    CountOverflow {
        count: usize,
    },
    AllocationFailed,
}

impl NativePlanError {
    /// Whether this failure comes from the executable-plan label validation
    /// which the pinned QuickJS object reader itself does not perform.
    ///
    /// A narrow consumer can use this distinction to keep an otherwise
    /// understood, outside-cohort function unadmitted instead of reporting an
    /// archive invariant failure. The concrete target and displacement remain
    /// sealed in this archive module's diagnostic.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn is_label_target_error(&self) -> bool {
        matches!(
            self,
            Self::LabelTargetOutOfRange { .. } | Self::LabelTargetNotInstructionBoundary { .. }
        )
    }
}

impl fmt::Display for NativePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FunctionNotInImage => {
                formatter.write_str("function identity does not belong to this bytecode image")
            }
            Self::InvalidOpcode {
                instruction,
                byte_pc,
                opcode,
            } => write!(
                formatter,
                "invalid native opcode {opcode} at instruction {instruction}, byte {byte_pc}"
            ),
            Self::InvalidFormatSize {
                byte_pc,
                opcode,
                format,
                descriptor_size,
                expected_size,
            } => write!(
                formatter,
                "native opcode {opcode} at byte {byte_pc} declares {descriptor_size} bytes for {format:?}, expected {expected_size}"
            ),
            Self::InstructionBoundaryMismatch {
                instruction,
                expected,
                actual,
            } => write!(
                formatter,
                "native instruction {instruction} begins at byte {actual}, expected {expected}"
            ),
            Self::OpcodeByteMismatch {
                byte_pc,
                sidecar,
                byte,
            } => write!(
                formatter,
                "native instruction sidecar opcode {sidecar} disagrees with byte {byte} at {byte_pc}"
            ),
            Self::CodeLengthMismatch {
                covered,
                byte_length,
            } => write!(
                formatter,
                "native instruction boundaries cover {covered} bytes, payload has {byte_length}"
            ),
            Self::CodeLengthOutOfRange { byte_length } => write!(
                formatter,
                "native-code byte length {byte_length} cannot be represented by a native PC"
            ),
            Self::AtomRelocationMismatch {
                relocation,
                expected,
                actual,
            } => write!(
                formatter,
                "native atom relocation {relocation} is {actual:?}, expected {expected:?}"
            ),
            Self::InvalidDynamicAtom {
                operand_pc,
                index,
                atom_count,
            } => write!(
                formatter,
                "native atom at byte {operand_pc} references dynamic atom {index}, table has {atom_count} entries"
            ),
            Self::MissingAtomProjection {
                byte_pc,
                opcode,
                format,
            } => write!(
                formatter,
                "native opcode {opcode} at byte {byte_pc} with {format:?} lacks its semantic atom projection"
            ),
            Self::UnexpectedAtomProjection {
                byte_pc,
                opcode,
                format,
            } => write!(
                formatter,
                "native opcode {opcode} at byte {byte_pc} with {format:?} received an unexpected atom projection"
            ),
            Self::InvalidImplicitOpcode {
                byte_pc,
                opcode,
                format,
            } => write!(
                formatter,
                "native opcode {opcode} at byte {byte_pc} is not a valid {format:?} implicit operand"
            ),
            Self::TruncatedOperand {
                byte_pc,
                opcode,
                operand_offset,
                width,
                instruction_size,
            } => write!(
                formatter,
                "native opcode {opcode} at byte {byte_pc} needs {width} operand bytes at +{operand_offset}, instruction has {instruction_size} bytes"
            ),
            Self::LabelTargetOutOfRange {
                byte_pc,
                operand_pc,
                displacement,
                byte_length,
            } => write!(
                formatter,
                "native label at byte {operand_pc} in opcode at {byte_pc} has displacement {displacement} outside {byte_length} code bytes"
            ),
            Self::LabelTargetNotInstructionBoundary {
                byte_pc,
                operand_pc,
                displacement,
                target_pc,
            } => write!(
                formatter,
                "native label at byte {operand_pc} in opcode at {byte_pc} has displacement {displacement} to non-instruction byte {target_pc}"
            ),
            Self::OffsetOverflow {
                byte_pc,
                operand_offset,
            } => write!(
                formatter,
                "native byte PC {byte_pc} plus operand offset {operand_offset} overflowed"
            ),
            Self::CountOverflow { count } => {
                write!(formatter, "native instruction count {count} exceeds u32")
            }
            Self::AllocationFailed => formatter.write_str("native plan allocation failed"),
        }
    }
}

impl std::error::Error for NativePlanError {}

struct DecodedCodePlan<'image> {
    instructions: Box<[NativeInstruction<'image>]>,
    native_pc_map: Box<[u32]>,
}

/// Decode one authenticated function's code without admitting it to execution.
pub(in crate::runtime::binary_object) fn decode_native_code_plan<'image>(
    image: &'image BytecodeImage,
    function: FunctionId,
) -> Result<NativeCodePlan<'image>, NativePlanError> {
    let record = image
        .function(function)
        .ok_or(NativePlanError::FunctionNotInImage)?;
    let code_plan = decode_code_plan(
        record.envelope().code(),
        image.atoms(),
        image.input_atom_slot_count(),
    )?;
    Ok(NativeCodePlan {
        function,
        instructions: code_plan.instructions,
        native_pc_map: code_plan.native_pc_map,
    })
}

fn decode_code_plan<'image>(
    code: &'image ImageCode,
    dynamic_atoms: &'image [WireString],
    input_atom_slot_count: u32,
) -> Result<DecodedCodePlan<'image>, NativePlanError> {
    let byte_length = u32::try_from(code.as_bytes().len()).map_err(|_| {
        NativePlanError::CodeLengthOutOfRange {
            byte_length: code.as_bytes().len(),
        }
    })?;
    let native_pc_map = validate_instruction_boundaries(code, byte_length)?;

    let mut instructions = Vec::new();
    instructions
        .try_reserve_exact(code.instructions().len())
        .map_err(|_| NativePlanError::AllocationFailed)?;
    let mut relocation_index = 0_usize;

    for span in code.instructions() {
        let byte_pc = span.offset();
        let opcode = span.opcode();
        let format = opcode.format();
        let start = byte_pc as usize;
        let end = start + usize::from(opcode.size());
        let instruction_bytes = &code.as_bytes()[start..end];
        let atom = if format.has_atom_operand() {
            let expected = byte_pc
                .checked_add(1)
                .ok_or(NativePlanError::OffsetOverflow {
                    byte_pc,
                    operand_offset: 1,
                })?;
            let actual = code
                .atom_relocations()
                .get(relocation_index)
                .map(|relocation| relocation.operand_offset());
            if actual != Some(expected) {
                return Err(NativePlanError::AtomRelocationMismatch {
                    relocation: relocation_index,
                    expected: Some(expected),
                    actual,
                });
            }
            let relocation = &code.atom_relocations()[relocation_index];
            relocation_index += 1;
            let raw_atom = read_u32(instruction_bytes, 1, byte_pc, opcode)?;
            let from_input_atom_table = raw_atom
                .checked_sub(FIRST_DYNAMIC_ATOM)
                .is_some_and(|slot| slot < input_atom_slot_count);
            Some(NativeAtomRef::new(
                relocation.atom(),
                dynamic_atoms,
                from_input_atom_table,
                expected,
            )?)
        } else {
            None
        };

        let operands = decode_operands(
            format,
            opcode,
            instruction_bytes,
            byte_pc,
            byte_length,
            &native_pc_map,
            atom,
        )?;
        instructions.push(NativeInstruction {
            byte_pc,
            opcode,
            operands,
        });
    }

    if relocation_index != code.atom_relocations().len() {
        return Err(NativePlanError::AtomRelocationMismatch {
            relocation: relocation_index,
            expected: None,
            actual: code
                .atom_relocations()
                .get(relocation_index)
                .map(|relocation| relocation.operand_offset()),
        });
    }

    Ok(DecodedCodePlan {
        instructions: instructions.into_boxed_slice(),
        native_pc_map: native_pc_map.into_boxed_slice(),
    })
}

fn validate_instruction_boundaries(
    code: &ImageCode,
    byte_length: u32,
) -> Result<Vec<u32>, NativePlanError> {
    let mut native_pc_map = Vec::new();
    native_pc_map
        .try_reserve_exact(code.instructions().len())
        .map_err(|_| NativePlanError::AllocationFailed)?;
    let mut expected = 0_u32;

    for (instruction, span) in code.instructions().iter().enumerate() {
        let byte_pc = span.offset();
        let opcode = span.opcode();
        if opcode.raw() == 0 {
            return Err(NativePlanError::InvalidOpcode {
                instruction,
                byte_pc,
                opcode: opcode.raw(),
            });
        }
        if byte_pc != expected {
            return Err(NativePlanError::InstructionBoundaryMismatch {
                instruction,
                expected,
                actual: byte_pc,
            });
        }
        let expected_size = format_size(opcode.format());
        if opcode.size() != expected_size {
            return Err(NativePlanError::InvalidFormatSize {
                byte_pc,
                opcode: opcode.raw(),
                format: opcode.format(),
                descriptor_size: opcode.size(),
                expected_size,
            });
        }
        let Some(byte) = code.as_bytes().get(byte_pc as usize).copied() else {
            return Err(NativePlanError::CodeLengthMismatch {
                covered: expected,
                byte_length,
            });
        };
        if byte != opcode.raw() {
            return Err(NativePlanError::OpcodeByteMismatch {
                byte_pc,
                sidecar: opcode.raw(),
                byte,
            });
        }
        native_pc_map.push(byte_pc);
        expected = byte_pc.checked_add(u32::from(opcode.size())).ok_or(
            NativePlanError::OffsetOverflow {
                byte_pc,
                operand_offset: opcode.size(),
            },
        )?;
        if expected > byte_length {
            return Err(NativePlanError::CodeLengthMismatch {
                covered: expected,
                byte_length,
            });
        }
    }

    if expected != byte_length {
        return Err(NativePlanError::CodeLengthMismatch {
            covered: expected,
            byte_length,
        });
    }
    Ok(native_pc_map)
}

#[allow(clippy::too_many_arguments)]
fn decode_operands<'image>(
    format: OpcodeFormat,
    opcode: PinnedOpcode,
    instruction: &[u8],
    byte_pc: u32,
    byte_length: u32,
    native_pc_map: &[u32],
    atom: Option<NativeAtomRef<'image>>,
) -> Result<NativeOperands<'image>, NativePlanError> {
    if format.has_atom_operand() != atom.is_some() {
        return Err(if format.has_atom_operand() {
            NativePlanError::MissingAtomProjection {
                byte_pc,
                opcode: opcode.raw(),
                format,
            }
        } else {
            NativePlanError::UnexpectedAtomProjection {
                byte_pc,
                opcode: opcode.raw(),
                format,
            }
        });
    }
    let atom = || {
        atom.ok_or(NativePlanError::MissingAtomProjection {
            byte_pc,
            opcode: opcode.raw(),
            format,
        })
    };
    let label8 = |offset| {
        read_i8(instruction, offset, byte_pc, opcode).and_then(|displacement| {
            decode_label(
                byte_pc,
                offset,
                i32::from(displacement),
                byte_length,
                native_pc_map,
            )
        })
    };
    let label16 = |offset| {
        read_i16(instruction, offset, byte_pc, opcode).and_then(|displacement| {
            decode_label(
                byte_pc,
                offset,
                i32::from(displacement),
                byte_length,
                native_pc_map,
            )
        })
    };
    let label32 = |offset| {
        read_i32(instruction, offset, byte_pc, opcode).and_then(|displacement| {
            decode_label(byte_pc, offset, displacement, byte_length, native_pc_map)
        })
    };

    Ok(match format {
        OpcodeFormat::None => NativeOperands::None,
        OpcodeFormat::NoneInt => NativeOperands::NoneInt(implicit_integer(opcode, byte_pc)?),
        OpcodeFormat::NoneLoc => NativeOperands::NoneLoc(implicit_slot(
            opcode,
            byte_pc,
            format,
            &["get_loc", "put_loc", "set_loc"],
        )?),
        OpcodeFormat::NoneArg => NativeOperands::NoneArg(implicit_slot(
            opcode,
            byte_pc,
            format,
            &["get_arg", "put_arg", "set_arg"],
        )?),
        OpcodeFormat::NoneVarRef => NativeOperands::NoneVarRef(implicit_slot(
            opcode,
            byte_pc,
            format,
            &["get_var_ref", "put_var_ref", "set_var_ref"],
        )?),
        OpcodeFormat::U8 => NativeOperands::U8(read_u8(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::I8 => NativeOperands::I8(read_i8(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::Loc8 => NativeOperands::Loc8(read_u8(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::Const8 => NativeOperands::Const8(read_u8(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::Label8 => NativeOperands::Label8(label8(1)?),
        OpcodeFormat::U16 => NativeOperands::U16(read_u16(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::I16 => NativeOperands::I16(read_i16(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::Label16 => NativeOperands::Label16(label16(1)?),
        OpcodeFormat::NPop => NativeOperands::NPop(read_u16(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::NPopX => {
            NativeOperands::NPopX(implicit_slot(opcode, byte_pc, format, &["call"])?)
        }
        OpcodeFormat::NPopU16 => NativeOperands::NPopU16 {
            pop_count: read_u16(instruction, 1, byte_pc, opcode)?,
            environment: read_u16(instruction, 3, byte_pc, opcode)?,
        },
        OpcodeFormat::Loc => NativeOperands::Loc(read_u16(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::Arg => NativeOperands::Arg(read_u16(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::VarRef => NativeOperands::VarRef(read_u16(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::U32 => NativeOperands::U32(read_u32(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::I32 => NativeOperands::I32(read_i32(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::Const => NativeOperands::Const(read_u32(instruction, 1, byte_pc, opcode)?),
        OpcodeFormat::Label => NativeOperands::Label(label32(1)?),
        OpcodeFormat::Atom => NativeOperands::Atom(atom()?),
        OpcodeFormat::AtomU8 => NativeOperands::AtomU8 {
            atom: atom()?,
            value: read_u8(instruction, 5, byte_pc, opcode)?,
        },
        OpcodeFormat::AtomU16 => NativeOperands::AtomU16 {
            atom: atom()?,
            value: read_u16(instruction, 5, byte_pc, opcode)?,
        },
        OpcodeFormat::AtomLabelU8 => NativeOperands::AtomLabelU8 {
            atom: atom()?,
            // QuickJS bases this displacement at the label operand itself,
            // which is opcode_pc + 5 after the relocated atom. It is not the
            // opcode_pc + 1 base used by ordinary label formats.
            label: label32(5)?,
            value: read_u8(instruction, 9, byte_pc, opcode)?,
        },
        OpcodeFormat::AtomLabelU16 => NativeOperands::AtomLabelU16 {
            atom: atom()?,
            label: label32(5)?,
            value: read_u16(instruction, 9, byte_pc, opcode)?,
        },
        OpcodeFormat::LabelU16 => NativeOperands::LabelU16 {
            label: label32(1)?,
            value: read_u16(instruction, 5, byte_pc, opcode)?,
        },
    })
}

fn implicit_integer(opcode: PinnedOpcode, byte_pc: u32) -> Result<i32, NativePlanError> {
    if opcode.name() == "push_minus1" {
        return Ok(-1);
    }
    let Some(suffix) = opcode.name().strip_prefix("push_") else {
        return Err(invalid_implicit(opcode, byte_pc, OpcodeFormat::NoneInt));
    };
    let [digit] = suffix.as_bytes() else {
        return Err(invalid_implicit(opcode, byte_pc, OpcodeFormat::NoneInt));
    };
    if !(b'0'..=b'7').contains(digit) {
        return Err(invalid_implicit(opcode, byte_pc, OpcodeFormat::NoneInt));
    }
    Ok(i32::from(*digit - b'0'))
}

fn implicit_slot(
    opcode: PinnedOpcode,
    byte_pc: u32,
    format: OpcodeFormat,
    prefixes: &[&str],
) -> Result<u16, NativePlanError> {
    for prefix in prefixes {
        if let Some(suffix) = opcode.name().strip_prefix(prefix) {
            let [digit] = suffix.as_bytes() else {
                continue;
            };
            if (b'0'..=b'3').contains(digit) {
                return Ok(u16::from(*digit - b'0'));
            }
        }
    }
    Err(invalid_implicit(opcode, byte_pc, format))
}

const fn invalid_implicit(
    opcode: PinnedOpcode,
    byte_pc: u32,
    format: OpcodeFormat,
) -> NativePlanError {
    NativePlanError::InvalidImplicitOpcode {
        byte_pc,
        opcode: opcode.raw(),
        format,
    }
}

fn decode_label(
    byte_pc: u32,
    operand_offset: u8,
    displacement: i32,
    byte_length: u32,
    native_pc_map: &[u32],
) -> Result<NativeLabel, NativePlanError> {
    let operand_pc =
        byte_pc
            .checked_add(u32::from(operand_offset))
            .ok_or(NativePlanError::OffsetOverflow {
                byte_pc,
                operand_offset,
            })?;
    let target = i64::from(operand_pc) + i64::from(displacement);
    if !(0..i64::from(byte_length)).contains(&target) {
        return Err(NativePlanError::LabelTargetOutOfRange {
            byte_pc,
            operand_pc,
            displacement,
            byte_length,
        });
    }
    let target_pc = target as u32;
    let target_instruction = native_pc_map.binary_search(&target_pc).map_err(|_| {
        NativePlanError::LabelTargetNotInstructionBoundary {
            byte_pc,
            operand_pc,
            displacement,
            target_pc,
        }
    })?;
    let target_instruction =
        u32::try_from(target_instruction).map_err(|_| NativePlanError::CountOverflow {
            count: native_pc_map.len(),
        })?;
    Ok(NativeLabel {
        operand_pc,
        displacement,
        target_pc,
        target_instruction,
    })
}

const fn format_size(format: OpcodeFormat) -> u8 {
    match format {
        OpcodeFormat::None
        | OpcodeFormat::NoneInt
        | OpcodeFormat::NoneLoc
        | OpcodeFormat::NoneArg
        | OpcodeFormat::NoneVarRef
        | OpcodeFormat::NPopX => 1,
        OpcodeFormat::U8
        | OpcodeFormat::I8
        | OpcodeFormat::Loc8
        | OpcodeFormat::Const8
        | OpcodeFormat::Label8 => 2,
        OpcodeFormat::U16
        | OpcodeFormat::I16
        | OpcodeFormat::Label16
        | OpcodeFormat::NPop
        | OpcodeFormat::Loc
        | OpcodeFormat::Arg
        | OpcodeFormat::VarRef => 3,
        OpcodeFormat::NPopU16
        | OpcodeFormat::U32
        | OpcodeFormat::I32
        | OpcodeFormat::Const
        | OpcodeFormat::Label
        | OpcodeFormat::Atom => 5,
        OpcodeFormat::AtomU8 => 6,
        OpcodeFormat::AtomU16 | OpcodeFormat::LabelU16 => 7,
        OpcodeFormat::AtomLabelU8 => 10,
        OpcodeFormat::AtomLabelU16 => 11,
    }
}

fn read_u8(
    instruction: &[u8],
    offset: u8,
    byte_pc: u32,
    opcode: PinnedOpcode,
) -> Result<u8, NativePlanError> {
    instruction
        .get(usize::from(offset))
        .copied()
        .ok_or_else(|| truncated_operand(byte_pc, opcode, offset, 1, instruction.len()))
}

fn read_i8(
    instruction: &[u8],
    offset: u8,
    byte_pc: u32,
    opcode: PinnedOpcode,
) -> Result<i8, NativePlanError> {
    read_u8(instruction, offset, byte_pc, opcode).map(|value| value as i8)
}

fn read_u16(
    instruction: &[u8],
    offset: u8,
    byte_pc: u32,
    opcode: PinnedOpcode,
) -> Result<u16, NativePlanError> {
    read_array::<2>(instruction, offset, byte_pc, opcode).map(u16::from_le_bytes)
}

fn read_i16(
    instruction: &[u8],
    offset: u8,
    byte_pc: u32,
    opcode: PinnedOpcode,
) -> Result<i16, NativePlanError> {
    read_array::<2>(instruction, offset, byte_pc, opcode).map(i16::from_le_bytes)
}

fn read_u32(
    instruction: &[u8],
    offset: u8,
    byte_pc: u32,
    opcode: PinnedOpcode,
) -> Result<u32, NativePlanError> {
    read_array::<4>(instruction, offset, byte_pc, opcode).map(u32::from_le_bytes)
}

fn read_i32(
    instruction: &[u8],
    offset: u8,
    byte_pc: u32,
    opcode: PinnedOpcode,
) -> Result<i32, NativePlanError> {
    read_array::<4>(instruction, offset, byte_pc, opcode).map(i32::from_le_bytes)
}

fn read_array<const WIDTH: usize>(
    instruction: &[u8],
    offset: u8,
    byte_pc: u32,
    opcode: PinnedOpcode,
) -> Result<[u8; WIDTH], NativePlanError> {
    let start = usize::from(offset);
    let end = start.checked_add(WIDTH).ok_or_else(|| {
        truncated_operand(byte_pc, opcode, offset, WIDTH as u8, instruction.len())
    })?;
    instruction
        .get(start..end)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| truncated_operand(byte_pc, opcode, offset, WIDTH as u8, instruction.len()))
}

const fn truncated_operand(
    byte_pc: u32,
    opcode: PinnedOpcode,
    operand_offset: u8,
    width: u8,
    instruction_size: usize,
) -> NativePlanError {
    NativePlanError::TruncatedOperand {
        byte_pc,
        opcode: opcode.raw(),
        operand_offset,
        width,
        instruction_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::binary_object::bytecode_image::{
        BytecodeImageLimits, ImageInstructionSpan, ImageRelocation, ModuleLimits,
        decode_bytecode_image_body,
    };
    use crate::runtime::binary_object::code::CodeLimits;
    use crate::runtime::binary_object::function_envelope::FunctionEnvelopeLimits;
    use crate::runtime::binary_object::graph::model::{AtomId, GraphLimits};
    use crate::runtime::binary_object::pinned_atoms::PinnedAtomId;
    use crate::runtime::binary_object::pinned_opcodes::PINNED_OPCODE_COUNT;
    use crate::runtime::binary_object::wire::{ReaderMode, WireCursor, WireLimits};

    const RETURN_42: [u8; 25] = [
        0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
    ];

    fn opcode(raw: u8) -> PinnedOpcode {
        PinnedOpcode::from_byte(raw).expect("test opcode is in the pinned table")
    }

    fn opcode_named(name: &str) -> PinnedOpcode {
        (0..PINNED_OPCODE_COUNT)
            .map(|raw| opcode(raw as u8))
            .find(|candidate| candidate.name() == name)
            .unwrap_or_else(|| panic!("missing pinned opcode {name}"))
    }

    fn semantic_atom<'image>(dynamic_atoms: &'image [WireString]) -> NativeAtomRef<'image> {
        NativeAtomRef::new(ImageAtom::Index(7), dynamic_atoms, false, 1).unwrap()
    }

    fn make_code(
        bytes: Vec<u8>,
        spans: impl IntoIterator<Item = (u32, PinnedOpcode)>,
        relocations: impl IntoIterator<Item = (u32, ImageAtom)>,
    ) -> ImageCode {
        ImageCode::new(
            bytes.into_boxed_slice(),
            spans
                .into_iter()
                .map(|(offset, opcode)| ImageInstructionSpan::new(offset, opcode))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            relocations
                .into_iter()
                .map(|(offset, atom)| ImageRelocation::new(offset, atom))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    }

    fn single_opcode_code(opcode: PinnedOpcode) -> ImageCode {
        let format = opcode.format();
        let mut bytes = vec![0; usize::from(opcode.size())];
        bytes[0] = opcode.raw();
        seed_label_to_zero(&mut bytes, format);
        let relocations = format
            .has_atom_operand()
            .then_some((1, ImageAtom::Index(7)));
        make_code(bytes, [(0, opcode)], relocations)
    }

    fn seed_label_to_zero(bytes: &mut [u8], format: OpcodeFormat) {
        match format {
            OpcodeFormat::Label8 => bytes[1] = (-1_i8) as u8,
            OpcodeFormat::Label16 => bytes[1..3].copy_from_slice(&(-1_i16).to_le_bytes()),
            OpcodeFormat::Label | OpcodeFormat::LabelU16 => {
                bytes[1..5].copy_from_slice(&(-1_i32).to_le_bytes());
            }
            OpcodeFormat::AtomLabelU8 | OpcodeFormat::AtomLabelU16 => {
                bytes[5..9].copy_from_slice(&(-5_i32).to_le_bytes());
            }
            OpcodeFormat::None
            | OpcodeFormat::NoneInt
            | OpcodeFormat::NoneLoc
            | OpcodeFormat::NoneArg
            | OpcodeFormat::NoneVarRef
            | OpcodeFormat::U8
            | OpcodeFormat::I8
            | OpcodeFormat::Loc8
            | OpcodeFormat::Const8
            | OpcodeFormat::U16
            | OpcodeFormat::I16
            | OpcodeFormat::NPop
            | OpcodeFormat::NPopX
            | OpcodeFormat::NPopU16
            | OpcodeFormat::Loc
            | OpcodeFormat::Arg
            | OpcodeFormat::VarRef
            | OpcodeFormat::U32
            | OpcodeFormat::I32
            | OpcodeFormat::Const
            | OpcodeFormat::Atom
            | OpcodeFormat::AtomU8
            | OpcodeFormat::AtomU16 => {}
        }
    }

    #[test]
    fn all_244_descriptors_have_a_data_driven_plan_outcome() {
        let mut decoded = 0_usize;
        for raw in 0..PINNED_OPCODE_COUNT {
            let opcode = opcode(raw as u8);
            let code = single_opcode_code(opcode);
            if raw == 0 {
                assert!(matches!(
                    decode_code_plan(&code, &[], 0),
                    Err(NativePlanError::InvalidOpcode { opcode: 0, .. })
                ));
            } else {
                let plan = decode_code_plan(&code, &[], 0)
                    .unwrap_or_else(|error| panic!("{} ({raw}) failed: {error}", opcode.name()));
                assert_eq!(plan.native_pc_map.as_ref(), [0]);
                assert_eq!(plan.instructions.len(), 1);
                assert_eq!(plan.instructions[0].opcode(), opcode);
                assert_eq!(plan.instructions[0].operands().format(), opcode.format());
                decoded += 1;
            }
        }
        assert_eq!(decoded, PINNED_OPCODE_COUNT - 1);
    }

    #[test]
    fn all_29_operand_formats_decode_independently_of_admission() {
        let cases = [
            (OpcodeFormat::None, "nop"),
            (OpcodeFormat::NoneInt, "push_3"),
            (OpcodeFormat::NoneLoc, "set_loc2"),
            (OpcodeFormat::NoneArg, "get_arg1"),
            (OpcodeFormat::NoneVarRef, "put_var_ref3"),
            (OpcodeFormat::U8, "special_object"),
            (OpcodeFormat::I8, "push_i8"),
            (OpcodeFormat::Loc8, "get_loc8"),
            (OpcodeFormat::Const8, "push_const8"),
            (OpcodeFormat::Label8, "goto8"),
            (OpcodeFormat::U16, "rest"),
            (OpcodeFormat::I16, "push_i16"),
            (OpcodeFormat::Label16, "goto16"),
            (OpcodeFormat::NPop, "call"),
            (OpcodeFormat::NPopX, "call2"),
            (OpcodeFormat::NPopU16, "eval"),
            (OpcodeFormat::Loc, "get_loc"),
            (OpcodeFormat::Arg, "get_arg"),
            (OpcodeFormat::VarRef, "get_var_ref"),
            // U32 occurs only in temporary upstream opcodes. Reuse a
            // same-width final descriptor to exercise its pinned wire layout;
            // the descriptor sweep above covers I32 and Const directly.
            (OpcodeFormat::U32, "push_i32"),
            (OpcodeFormat::I32, "push_i32"),
            (OpcodeFormat::Const, "push_const"),
            (OpcodeFormat::Label, "goto"),
            (OpcodeFormat::Atom, "push_atom_value"),
            (OpcodeFormat::AtomU8, "throw_error"),
            (OpcodeFormat::AtomU16, "make_loc_ref"),
            (OpcodeFormat::AtomLabelU8, "with_get_var"),
            (OpcodeFormat::AtomLabelU16, "with_get_var"),
            (OpcodeFormat::LabelU16, "goto"),
        ];
        assert_eq!(cases.len(), 29);

        for (format, name) in cases {
            let opcode = opcode_named(name);
            let mut instruction = vec![0; usize::from(format_size(format))];
            instruction[0] = opcode.raw();
            seed_label_to_zero(&mut instruction, format);
            let atom = format.has_atom_operand().then(|| semantic_atom(&[]));
            let operands = decode_operands(
                format,
                opcode,
                &instruction,
                0,
                format_size(format).into(),
                &[0],
                atom,
            )
            .unwrap_or_else(|error| panic!("{format:?} failed: {error}"));
            assert_eq!(operands.format(), format);
        }
    }

    #[test]
    fn ordinary_and_atom_label_bases_follow_the_operand_pc() {
        let goto = opcode_named("goto");
        let nop = opcode_named("nop");
        let mut ordinary = vec![0; 6];
        ordinary[0] = goto.raw();
        ordinary[1..5].copy_from_slice(&4_i32.to_le_bytes());
        ordinary[5] = nop.raw();
        let ordinary = make_code(ordinary, [(0, goto), (5, nop)], []);
        let plan = decode_code_plan(&ordinary, &[], 0).unwrap();
        let NativeOperands::Label(label) = plan.instructions[0].operands() else {
            panic!("goto did not decode a label");
        };
        assert_eq!(label.operand_pc(), 1);
        assert_eq!(label.target_pc(), 5);
        assert_eq!(label.target_instruction(), 1);

        let with_get = opcode_named("with_get_var");
        let mut atom_label = vec![0; 11];
        atom_label[0] = with_get.raw();
        atom_label[5..9].copy_from_slice(&5_i32.to_le_bytes());
        atom_label[10] = nop.raw();
        let atom_label = make_code(
            atom_label,
            [(0, with_get), (10, nop)],
            [(1, ImageAtom::Index(7))],
        );
        let plan = decode_code_plan(&atom_label, &[], 0).unwrap();
        let NativeOperands::AtomLabelU8 { label, .. } = plan.instructions[0].operands() else {
            panic!("with_get_var did not decode an atom label");
        };
        assert_eq!(label.operand_pc(), 5);
        assert_eq!(label.displacement(), 5);
        assert_eq!(label.target_pc(), 10);
        assert_eq!(label.target_instruction(), 1);
    }

    #[test]
    fn opcode_plus_one_atom_label_mutation_is_rejected() {
        let with_get = opcode_named("with_get_var");
        let nop = opcode_named("nop");
        let mut bytes = vec![0; 11];
        bytes[0] = with_get.raw();
        // This would target byte 10 only under the incorrect opcode_pc + 1
        // rule. The pinned atom-label base is its operand at byte 5, so the
        // real target is byte 14 and must be rejected.
        bytes[5..9].copy_from_slice(&9_i32.to_le_bytes());
        bytes[10] = nop.raw();
        let code = make_code(
            bytes,
            [(0, with_get), (10, nop)],
            [(1, ImageAtom::Index(7))],
        );
        assert!(matches!(
            decode_code_plan(&code, &[], 0),
            Err(NativePlanError::LabelTargetOutOfRange {
                operand_pc: 5,
                displacement: 9,
                ..
            })
        ));
    }

    #[test]
    fn labels_reject_target_interior_and_out_of_range() {
        let goto16 = opcode_named("goto16");
        let nop = opcode_named("nop");
        let code = make_code(
            vec![goto16.raw(), 0, 0, nop.raw()],
            [(0, goto16), (3, nop)],
            [],
        );
        assert!(matches!(
            decode_code_plan(&code, &[], 0),
            Err(NativePlanError::LabelTargetNotInstructionBoundary {
                operand_pc: 1,
                target_pc: 1,
                ..
            })
        ));

        let goto8 = opcode_named("goto8");
        let code = make_code(vec![goto8.raw(), 127], [(0, goto8)], []);
        assert!(matches!(
            decode_code_plan(&code, &[], 0),
            Err(NativePlanError::LabelTargetOutOfRange {
                operand_pc: 1,
                displacement: 127,
                ..
            })
        ));
    }

    #[test]
    fn instruction_sidecar_is_an_exact_ordered_bijection() {
        let nop = opcode_named("nop");
        let missing = make_code(vec![nop.raw()], [], []);
        assert!(matches!(
            decode_code_plan(&missing, &[], 0),
            Err(NativePlanError::CodeLengthMismatch {
                covered: 0,
                byte_length: 1,
            })
        ));

        let extra = make_code(vec![nop.raw()], [(0, nop), (1, nop)], []);
        assert!(matches!(
            decode_code_plan(&extra, &[], 0),
            Err(NativePlanError::CodeLengthMismatch {
                covered: 1,
                byte_length: 1,
            })
        ));

        let duplicate = make_code(vec![nop.raw()], [(0, nop), (0, nop)], []);
        assert!(matches!(
            decode_code_plan(&duplicate, &[], 0),
            Err(NativePlanError::InstructionBoundaryMismatch {
                instruction: 1,
                expected: 1,
                actual: 0,
            })
        ));

        let return_undef = opcode_named("return_undef");
        let mismatch = make_code(vec![return_undef.raw()], [(0, nop)], []);
        assert!(matches!(
            decode_code_plan(&mismatch, &[], 0),
            Err(NativePlanError::OpcodeByteMismatch {
                byte_pc: 0,
                sidecar,
                byte,
            })
            if sidecar == nop.raw() && byte == return_undef.raw()
        ));
    }

    #[test]
    fn atom_relocations_are_an_exact_ordered_bijection() {
        let push_atom = opcode_named("push_atom_value");
        let missing = make_code(vec![push_atom.raw(), 0, 0, 0, 0], [(0, push_atom)], []);
        assert!(matches!(
            decode_code_plan(&missing, &[], 0),
            Err(NativePlanError::AtomRelocationMismatch {
                relocation: 0,
                expected: Some(1),
                actual: None,
            })
        ));

        let nop = opcode_named("nop");
        let extra = make_code(vec![nop.raw()], [(0, nop)], [(0, ImageAtom::Index(7))]);
        assert!(matches!(
            decode_code_plan(&extra, &[], 0),
            Err(NativePlanError::AtomRelocationMismatch {
                relocation: 0,
                expected: None,
                actual: Some(0),
            })
        ));

        let duplicate = make_code(
            vec![push_atom.raw(), 0, 0, 0, 0],
            [(0, push_atom)],
            [(1, ImageAtom::Index(7)), (1, ImageAtom::Index(7))],
        );
        assert!(matches!(
            decode_code_plan(&duplicate, &[], 0),
            Err(NativePlanError::AtomRelocationMismatch {
                relocation: 1,
                expected: None,
                actual: Some(1),
            })
        ));
    }

    #[test]
    fn sealed_atom_projection_preserves_identity_classes_and_width() {
        let dynamic_atoms = [WireString::Wide(Box::from([0x100, b'x' as u16]))];
        let dynamic = NativeAtomRef::new(
            ImageAtom::Dynamic(AtomId::from_zero_based(0)),
            &dynamic_atoms,
            true,
            1,
        )
        .unwrap();
        assert_eq!(dynamic.class(), NativeAtomClass::String);
        assert!(dynamic.originates_from_input_atom_table());
        assert_eq!(dynamic.dynamic_string(), Some(&dynamic_atoms[0]));
        assert_eq!(dynamic.manifest_string(), None);

        let private = NativeAtomRef::new(
            ImageAtom::Predefined(PinnedAtomId::from_raw(229).unwrap()),
            &[],
            false,
            1,
        )
        .unwrap();
        assert_eq!(private.class(), NativeAtomClass::Private);
        assert!(!private.originates_from_input_atom_table());
        assert_eq!(private.identity_description(), Some("<brand>"));
        assert_eq!(private.manifest_string(), None);

        let symbol = NativeAtomRef::new(
            ImageAtom::Predefined(PinnedAtomId::from_raw(230).unwrap()),
            &[],
            false,
            1,
        )
        .unwrap();
        assert_eq!(symbol.class(), NativeAtomClass::Symbol);
        assert_eq!(symbol.identity_description(), Some("Symbol.toPrimitive"));

        assert_eq!(
            NativeAtomRef::new(
                ImageAtom::Dynamic(AtomId::from_zero_based(0)),
                &[],
                false,
                17,
            ),
            Err(NativePlanError::InvalidDynamicAtom {
                operand_pc: 17,
                index: 0,
                atom_count: 0,
            })
        );
    }

    #[test]
    fn authenticated_function_id_is_required_for_plan_construction() {
        let first = decode_image(&RETURN_42);
        let second = decode_image(&RETURN_42);
        let function = first.root().function_id().unwrap();
        let plan = decode_native_code_plan(&first, function).unwrap();
        assert_eq!(plan.function(), function);
        assert_eq!(plan.native_pc_map(), [0, 2, 3]);
        assert_eq!(plan.instruction_at_native_pc(2), Some(1));
        assert_eq!(plan.instruction_at_native_pc(1), None);
        assert_eq!(
            decode_native_code_plan(&second, function),
            Err(NativePlanError::FunctionNotInImage)
        );
    }

    fn decode_image(input: &[u8]) -> BytecodeImage {
        let bounded = input.len().max(1);
        let wire = WireLimits::new(bounded, u32::try_from(bounded).unwrap(), bounded, bounded);
        let graph = GraphLimits::new(
            bounded, bounded, bounded, bounded, bounded, bounded, bounded, bounded, bounded,
        );
        let envelope = FunctionEnvelopeLimits::new(
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            CodeLimits::new(bounded, bounded, bounded),
        );
        let image = BytecodeImageLimits::new(
            graph,
            envelope,
            ModuleLimits::new(bounded, bounded, bounded, bounded),
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
            bounded,
        );
        let cursor = WireCursor::new(input, ReaderMode::QuickJsCompatible, wire).unwrap();
        let (cursor, image) = decode_bytecode_image_body(cursor, image, true).unwrap();
        cursor.finish().unwrap();
        image
    }
}
