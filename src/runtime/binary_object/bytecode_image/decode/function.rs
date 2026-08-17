//! Function-record frames and table state inside the shared whole-image decoder.

use super::super::super::code::CodeImageParts;
use super::super::super::function_envelope::{
    FunctionEnvelope, FunctionEnvelopeError, FunctionEnvelopeParts,
    read_function_record_prefix_after_tag,
};
use super::super::super::graph::decode::{DataCompletion, DataMachineOutput, MachineSource};
use super::super::super::wire::WireCursor;
use super::super::atoms::{ImageAtomTable, ImageKey};
use super::super::budget::{
    BytecodeImageBudgetError, BytecodeImageLimits, BytecodeImageResourceKind, FunctionTotals,
    FunctionUsage, RemainingFunctionBudget,
};
use super::super::model::{
    FunctionRecord, ImageClosureVariable, ImageCode, ImageFunctionDebug, ImageFunctionEnvelope,
    ImageInstructionSpan, ImageLocalVariable, ImageRelocation, ImageValue,
};
use super::{AuthenticatedFunction, BytecodeImageError};

pub(super) struct FunctionFrame {
    function: FunctionSlot,
    envelope: ImageFunctionEnvelope,
    expected_constants: usize,
    constants: Vec<DataCompletion<ImageValue>>,
}

impl FunctionFrame {
    pub(super) fn push_constant(
        &mut self,
        value: DataCompletion<ImageValue>,
    ) -> Result<(), BytecodeImageError> {
        if self.constants.len() >= self.expected_constants {
            return Err(BytecodeImageError::InvalidCompletionTarget);
        }
        self.constants.push(value);
        Ok(())
    }

    pub(super) fn is_complete(&self) -> bool {
        self.constants.len() == self.expected_constants
    }
}

struct PendingFunctionRecord {
    envelope: ImageFunctionEnvelope,
    constants: Vec<DataCompletion<ImageValue>>,
}

#[derive(Clone, Copy)]
struct FunctionSlot {
    source: MachineSource,
    index: u32,
}

pub(super) struct FunctionTable {
    source: MachineSource,
    limits: BytecodeImageLimits,
    slots: Vec<Option<PendingFunctionRecord>>,
    totals: FunctionTotals,
}

impl FunctionTable {
    pub(super) fn new(source: MachineSource, limits: BytecodeImageLimits) -> Self {
        Self {
            source,
            limits,
            slots: Vec::new(),
            totals: FunctionTotals::default(),
        }
    }

    pub(super) fn begin_function(
        &mut self,
        cursor: &mut WireCursor<'_>,
        atoms: &ImageAtomTable,
        tag_offset: usize,
    ) -> Result<FunctionFrame, BytecodeImageError> {
        let requested =
            self.slots
                .len()
                .checked_add(1)
                .ok_or(BytecodeImageError::CountOverflow {
                    kind: BytecodeImageResourceKind::Functions,
                })?;
        self.limits
            .check(BytecodeImageResourceKind::Functions, requested)?;
        let index =
            u32::try_from(self.slots.len()).map_err(|_| BytecodeImageError::CountOverflow {
                kind: BytecodeImageResourceKind::Functions,
            })?;
        self.slots
            .try_reserve(1)
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        self.slots.push(None);
        let function = FunctionSlot {
            source: self.source,
            index,
        };

        let remaining = self.totals.remaining(self.limits)?;
        let envelope_limits = remaining.intersect(self.limits.envelope());
        let prefix =
            read_function_record_prefix_after_tag(cursor, atoms.raw_space(), envelope_limits)
                .map_err(|error| self.map_prefix_error(error, remaining))?;
        let (envelope, constant_count) = prefix.into_parts();
        let expected_constants = constant_count as usize;
        let totals = self.next_totals(&envelope, expected_constants)?;
        let envelope = self.relocate_envelope(atoms, envelope, tag_offset)?;
        self.totals = totals;

        let mut constants = Vec::new();
        constants
            .try_reserve_exact(expected_constants)
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        Ok(FunctionFrame {
            function,
            envelope,
            expected_constants,
            constants,
        })
    }

    pub(super) fn finish_frame(
        &mut self,
        frame: FunctionFrame,
    ) -> Result<AuthenticatedFunction, BytecodeImageError> {
        if frame.function.source != self.source || !frame.is_complete() {
            return Err(BytecodeImageError::InvalidFunctionState {
                function_index: frame.function.index,
            });
        }
        let Some(slot) = self.slots.get_mut(frame.function.index as usize) else {
            return Err(BytecodeImageError::InvalidFunctionState {
                function_index: frame.function.index,
            });
        };
        if slot.is_some() {
            return Err(BytecodeImageError::InvalidFunctionState {
                function_index: frame.function.index,
            });
        }
        *slot = Some(PendingFunctionRecord {
            envelope: frame.envelope,
            constants: frame.constants,
        });
        Ok(AuthenticatedFunction::new(
            frame.function.source,
            frame.function.index,
        ))
    }

    pub(super) fn finish(
        self,
        output: &DataMachineOutput<ImageValue, ImageKey>,
    ) -> Result<Box<[FunctionRecord]>, BytecodeImageError> {
        let mut records = Vec::new();
        records
            .try_reserve_exact(self.slots.len())
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        for (index, slot) in self.slots.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| BytecodeImageError::CountOverflow {
                kind: BytecodeImageResourceKind::Functions,
            })?;
            let pending = slot.ok_or(BytecodeImageError::InvalidFunctionState {
                function_index: index,
            })?;
            let mut constants = Vec::new();
            constants
                .try_reserve_exact(pending.constants.len())
                .map_err(|_| BytecodeImageError::AllocationFailed)?;
            for completion in pending.constants {
                constants.push(output.unwrap_completion(completion)?);
            }
            records.push(FunctionRecord::new(
                pending.envelope,
                constants.into_boxed_slice(),
            ));
        }
        Ok(records.into_boxed_slice())
    }

    fn relocate_envelope(
        &self,
        atoms: &ImageAtomTable,
        envelope: FunctionEnvelope,
        diagnostic_offset: usize,
    ) -> Result<ImageFunctionEnvelope, BytecodeImageError> {
        let FunctionEnvelopeParts {
            atom_space,
            flags,
            js_mode,
            name,
            argument_count,
            variable_count,
            defined_argument_count,
            stack_size,
            variable_reference_count,
            locals,
            closures,
            code,
            debug,
        } = envelope.into_parts();

        let name = atoms.remap_atom(atom_space, name, diagnostic_offset)?;
        let mut image_locals = Vec::new();
        image_locals
            .try_reserve_exact(locals.len())
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        for local in locals {
            let (name, scope_next, variable_reference_index, flags) = local.into_parts();
            image_locals.push(ImageLocalVariable::new(
                atoms.remap_atom(atom_space, name, diagnostic_offset)?,
                scope_next,
                variable_reference_index,
                flags,
            ));
        }

        let mut image_closures = Vec::new();
        image_closures
            .try_reserve_exact(closures.len())
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        for closure in closures {
            let (name, variable_index, flags) = closure.into_parts();
            image_closures.push(ImageClosureVariable::new(
                atoms.remap_atom(atom_space, name, diagnostic_offset)?,
                variable_index,
                flags,
            ));
        }

        let code = relocate_code(atoms, code.into_parts())?;
        let debug = match debug {
            Some(debug) => {
                let (filename, pc2line, source) = debug.into_parts();
                Some(ImageFunctionDebug::new(
                    atoms.remap_atom(atom_space, filename, diagnostic_offset)?,
                    pc2line,
                    source,
                ))
            }
            None => None,
        };

        Ok(ImageFunctionEnvelope::new(
            flags,
            js_mode,
            name,
            argument_count,
            variable_count,
            defined_argument_count,
            stack_size,
            variable_reference_count,
            image_locals.into_boxed_slice(),
            image_closures.into_boxed_slice(),
            code,
            debug,
        ))
    }

    fn next_totals(
        &self,
        envelope: &FunctionEnvelope,
        constant_count: usize,
    ) -> Result<FunctionTotals, BytecodeImageError> {
        let additional_debug_bytes = match envelope.debug() {
            Some(debug) => debug
                .pc2line()
                .len()
                .checked_add(debug.source().len())
                .ok_or(BytecodeImageBudgetError::CountOverflow {
                    kind: BytecodeImageResourceKind::TotalDebugBytes,
                })?,
            None => 0,
        };
        self.totals
            .checked_add(
                FunctionUsage::new(
                    constant_count,
                    envelope.locals().len(),
                    envelope.closures().len(),
                    envelope.code().as_bytes().len(),
                    envelope.code().instructions().len(),
                    envelope.code().atom_relocations().len(),
                    additional_debug_bytes,
                ),
                self.limits,
            )
            .map_err(Into::into)
    }

    fn map_prefix_error(
        &self,
        error: FunctionEnvelopeError,
        remaining: RemainingFunctionBudget,
    ) -> BytecodeImageError {
        match self
            .totals
            .aggregate_error_for_envelope(&error, remaining, self.limits)
        {
            Some(error) => error.into(),
            None => BytecodeImageError::Envelope(error),
        }
    }
}

fn relocate_code(
    atoms: &ImageAtomTable,
    parts: CodeImageParts,
) -> Result<ImageCode, BytecodeImageError> {
    let mut instructions = Vec::new();
    instructions
        .try_reserve_exact(parts.instructions.len())
        .map_err(|_| BytecodeImageError::AllocationFailed)?;
    for instruction in parts.instructions {
        instructions.push(ImageInstructionSpan::new(
            instruction.offset(),
            instruction.opcode(),
        ));
    }

    let mut relocations = Vec::new();
    relocations
        .try_reserve_exact(parts.atom_relocations.len())
        .map_err(|_| BytecodeImageError::AllocationFailed)?;
    for relocation in parts.atom_relocations {
        let offset = parts
            .payload_offset
            .checked_add(relocation.operand_offset() as usize)
            .ok_or(BytecodeImageError::OffsetOverflow {
                offset: parts.payload_offset,
                addend: relocation.operand_offset() as usize,
            })?;
        relocations.push(ImageRelocation::new(
            relocation.operand_offset(),
            atoms.remap_atom(parts.atom_space, relocation.atom(), offset)?,
        ));
    }

    Ok(ImageCode::new(
        parts.bytes.into_boxed_slice(),
        instructions.into_boxed_slice(),
        relocations.into_boxed_slice(),
    ))
}
