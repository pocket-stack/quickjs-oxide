//! Function-record planning inside the shared whole-image writer state machine.

use super::super::super::super::code::{CodeError, CodeResourceKind};
use super::super::super::super::function_envelope::{
    FunctionEnvelopeError, FunctionEnvelopeLimits, FunctionField, FunctionResourceKind,
};
use super::super::super::super::wire::BcTag;
use super::super::super::budget::{
    BytecodeImageBudgetError, BytecodeImageResourceKind, FunctionUsage,
};
use super::super::super::model::{FunctionId, FunctionRecord, ImageCode, ImageFunctionEnvelope};
use super::super::BytecodeImageEncodeError;
use super::{MAX_QUICKJS_POSITIVE_INT, PlanBuilder, PlanTask, PlannedToken, ValueRef};

const FUNCTION_FLAGS_MASK: u16 = 0x0fff;
const FUNCTION_HAS_DEBUG: u16 = 1 << 10;
const CLOSURE_FLAGS_MASK: u16 = 0x01ff;

impl<'a> PlanBuilder<'a> {
    pub(super) fn plan_function(
        &mut self,
        function: FunctionId,
        whole_depth: usize,
        graph_parent_depth: usize,
    ) -> Result<(), BytecodeImageEncodeError> {
        let record =
            self.image
                .function(function)
                .ok_or(BytecodeImageEncodeError::ForeignFunction {
                    function_index: function.zero_based(),
                })?;
        if self.active_functions.contains(&function) {
            return Err(BytecodeImageEncodeError::CircularFunction {
                function_index: function.zero_based(),
            });
        }
        self.charge_function_occurrence()?;
        if !self.seen_functions.contains(&function) {
            let expected = u32::try_from(self.seen_functions.len()).map_err(|_| {
                BytecodeImageBudgetError::CountOverflow {
                    kind: BytecodeImageResourceKind::Functions,
                }
            })?;
            if function.zero_based() != expected {
                return Err(BytecodeImageEncodeError::FunctionPreorder {
                    expected,
                    found: function.zero_based(),
                });
            }
        }
        self.authenticate_function_record(function, record)?;
        if !self.seen_functions.contains(&function) {
            self.seen_functions
                .try_reserve(1)
                .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
            self.seen_functions.insert(function);
        }
        self.active_functions
            .try_reserve(1)
            .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
        self.active_functions.insert(function);

        self.plan_function_prefix(function, record)?;

        self.push_task(PlanTask::LeaveFunction(function))?;
        self.reserve_tasks(Some(record.constants().len()))?;
        for constant in record.constants().iter().rev() {
            self.tasks.push(PlanTask::Value {
                value: ValueRef::Image(constant),
                whole_parent_depth: whole_depth,
                graph_parent_depth,
            });
        }
        Ok(())
    }

    fn plan_function_prefix(
        &mut self,
        function: FunctionId,
        record: &'a FunctionRecord,
    ) -> Result<(), BytecodeImageEncodeError> {
        let envelope = record.envelope();

        self.push_u8(BcTag::FunctionBytecode.to_byte())?;
        let flags =
            envelope.flags().raw() | (u16::from(envelope.debug().is_some()) * FUNCTION_HAS_DEBUG);
        self.push_token(PlannedToken::U16(flags))?;
        self.push_u8(envelope.js_mode().raw())?;
        self.plan_atom(envelope.name())?;
        self.push_uleb(u32::from(envelope.argument_count()))?;
        self.push_uleb(u32::from(envelope.variable_count()))?;
        self.push_uleb(u32::from(envelope.defined_argument_count()))?;
        self.push_uleb(u32::from(envelope.stack_size()))?;
        self.push_uleb(u32::from(envelope.variable_reference_count()))?;
        self.push_uleb(positive_u32(
            envelope.closures().len(),
            FunctionField::ClosureVariableCount,
        )?)?;
        self.push_uleb(positive_u32(
            record.constants().len(),
            FunctionField::ConstantPoolCount,
        )?)?;
        self.push_uleb(positive_u32(
            envelope.code().as_bytes().len(),
            FunctionField::ByteCodeLength,
        )?)?;
        self.push_uleb(positive_u32(
            envelope.locals().len(),
            FunctionField::LocalCount,
        )?)?;

        for local in envelope.locals() {
            self.plan_atom(local.name())?;
            let encoded =
                local
                    .scope_next()
                    .encode()
                    .ok_or(FunctionEnvelopeError::CountOverflow {
                        field: FunctionField::LocalScopeNext,
                    })?;
            self.push_uleb(encoded)?;
            self.push_uleb(u32::from(local.variable_reference_index()))?;
            self.push_u8(local.flags().raw())?;
        }
        for closure in envelope.closures() {
            self.plan_atom(closure.name())?;
            self.push_uleb(u32::from(closure.variable_index()))?;
            self.push_token(PlannedToken::U16(closure.flags().raw()))?;
        }

        for relocation in envelope.code().atom_relocations() {
            self.encounter_atom(relocation.atom())?;
        }
        self.push_token(PlannedToken::Code {
            function,
            code: envelope.code(),
        })?;

        if let Some(debug) = envelope.debug() {
            self.plan_atom(debug.filename())?;
            self.push_uleb(positive_u32(
                debug.pc2line().len(),
                FunctionField::Pc2LineLength,
            )?)?;
            self.push_token(PlannedToken::Bytes(debug.pc2line()))?;
            self.push_uleb(positive_u32(
                debug.source().len(),
                FunctionField::SourceLength,
            )?)?;
            self.push_token(PlannedToken::Bytes(debug.source()))?;
        }
        Ok(())
    }

    fn charge_function_occurrence(&mut self) -> Result<(), BytecodeImageEncodeError> {
        let requested = self.emitted_functions.checked_add(1).ok_or(
            BytecodeImageBudgetError::CountOverflow {
                kind: BytecodeImageResourceKind::Functions,
            },
        )?;
        self.options
            .limits
            .check(BytecodeImageResourceKind::Functions, requested)?;
        self.emitted_functions = requested;
        Ok(())
    }

    fn authenticate_function_record(
        &mut self,
        function: FunctionId,
        record: &FunctionRecord,
    ) -> Result<(), BytecodeImageEncodeError> {
        let envelope = record.envelope();
        let remaining = self.function_totals.remaining(self.options.limits)?;
        let envelope_limits = remaining.intersect(self.options.limits.envelope());
        if let Err(error) = validate_envelope(envelope, record.constants().len(), envelope_limits) {
            if let Some(error) = self.function_totals.aggregate_error_for_envelope(
                &error,
                remaining,
                self.options.limits,
            ) {
                return Err(error.into());
            }
            return Err(error.into());
        }
        validate_code(function, envelope.code())?;

        let debug_bytes = envelope.debug().map_or(Ok(0), |debug| {
            debug
                .pc2line()
                .len()
                .checked_add(debug.source().len())
                .ok_or(BytecodeImageBudgetError::CountOverflow {
                    kind: BytecodeImageResourceKind::TotalDebugBytes,
                })
        })?;
        let usage = FunctionUsage::new(
            record.constants().len(),
            envelope.locals().len(),
            envelope.closures().len(),
            envelope.code().as_bytes().len(),
            envelope.code().instructions().len(),
            envelope.code().atom_relocations().len(),
            debug_bytes,
        );
        self.function_totals = self
            .function_totals
            .checked_add(usage, self.options.limits)?;
        Ok(())
    }
}

fn validate_envelope(
    envelope: &ImageFunctionEnvelope,
    constant_count: usize,
    function_limits: FunctionEnvelopeLimits,
) -> Result<(), FunctionEnvelopeError> {
    let invalid_flags = envelope.flags().raw() & (!FUNCTION_FLAGS_MASK | FUNCTION_HAS_DEBUG);
    if invalid_flags != 0 {
        return Err(FunctionEnvelopeError::InvalidModelBits {
            field: FunctionField::FunctionFlags,
            bits: invalid_flags,
        });
    }
    let expected_locals = usize::from(envelope.argument_count())
        .checked_add(usize::from(envelope.variable_count()))
        .ok_or(FunctionEnvelopeError::CountOverflow {
            field: FunctionField::LocalCount,
        })?;
    if !envelope.locals().is_empty() && envelope.locals().len() != expected_locals {
        return Err(FunctionEnvelopeError::NonCanonicalLocalTableLength {
            argument_count: envelope.argument_count(),
            variable_count: envelope.variable_count(),
            local_count: envelope.locals().len(),
        });
    }
    for local in envelope.locals() {
        if local.scope_next().encode().is_none() {
            return Err(FunctionEnvelopeError::CountOverflow {
                field: FunctionField::LocalScopeNext,
            });
        }
    }
    for closure in envelope.closures() {
        let invalid = closure.flags().raw() & !CLOSURE_FLAGS_MASK;
        if invalid != 0 {
            return Err(FunctionEnvelopeError::InvalidModelBits {
                field: FunctionField::ClosureFlags,
                bits: invalid,
            });
        }
    }

    check_function_limit(
        function_limits,
        FunctionResourceKind::LocalVariables,
        envelope.locals().len(),
    )?;
    check_function_limit(
        function_limits,
        FunctionResourceKind::ClosureVariables,
        envelope.closures().len(),
    )?;
    check_function_limit(
        function_limits,
        FunctionResourceKind::ConstantPoolEntries,
        constant_count,
    )?;
    check_code_limit(
        function_limits,
        CodeResourceKind::Bytes,
        envelope.code().as_bytes().len(),
    )?;
    check_code_limit(
        function_limits,
        CodeResourceKind::Instructions,
        envelope.code().instructions().len(),
    )?;
    check_code_limit(
        function_limits,
        CodeResourceKind::AtomRelocations,
        envelope.code().atom_relocations().len(),
    )?;
    if let Some(debug) = envelope.debug() {
        check_function_limit(
            function_limits,
            FunctionResourceKind::Pc2LineBytes,
            debug.pc2line().len(),
        )?;
        check_function_limit(
            function_limits,
            FunctionResourceKind::SourceBytes,
            debug.source().len(),
        )?;
        let total = debug
            .pc2line()
            .len()
            .checked_add(debug.source().len())
            .ok_or(FunctionEnvelopeError::CountOverflow {
                field: FunctionField::SourceLength,
            })?;
        check_function_limit(
            function_limits,
            FunctionResourceKind::TotalDebugBytes,
            total,
        )?;
    }
    for (value, field) in [
        (envelope.locals().len(), FunctionField::LocalCount),
        (
            envelope.closures().len(),
            FunctionField::ClosureVariableCount,
        ),
        (constant_count, FunctionField::ConstantPoolCount),
        (
            envelope.code().as_bytes().len(),
            FunctionField::ByteCodeLength,
        ),
    ] {
        let _ = positive_u32(value, field)?;
    }
    Ok(())
}

fn validate_code(function: FunctionId, code: &ImageCode) -> Result<(), BytecodeImageEncodeError> {
    let mut expected_offset = 0usize;
    let mut relocation_index = 0usize;
    for instruction in code.instructions() {
        let offset = instruction.offset() as usize;
        if offset != expected_offset || instruction.opcode().raw() == 0 {
            return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
                function_index: function.zero_based(),
                offset: instruction.offset(),
            });
        }
        let size = usize::from(instruction.opcode().size());
        let end = offset
            .checked_add(size)
            .ok_or(BytecodeImageEncodeError::InvalidCodeSidecar {
                function_index: function.zero_based(),
                offset: instruction.offset(),
            })?;
        if size == 0 || end > code.as_bytes().len() {
            return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
                function_index: function.zero_based(),
                offset: instruction.offset(),
            });
        }
        if let Some(delta) = instruction.opcode().atom_operand_offset() {
            let operand = offset.checked_add(usize::from(delta)).ok_or(
                BytecodeImageEncodeError::InvalidCodeSidecar {
                    function_index: function.zero_based(),
                    offset: instruction.offset(),
                },
            )?;
            let relocation = code.atom_relocations().get(relocation_index).ok_or(
                BytecodeImageEncodeError::InvalidCodeSidecar {
                    function_index: function.zero_based(),
                    offset: u32::try_from(operand).unwrap_or(u32::MAX),
                },
            )?;
            if relocation.operand_offset() as usize != operand {
                return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
                    function_index: function.zero_based(),
                    offset: relocation.operand_offset(),
                });
            }
            relocation_index += 1;
        }
        expected_offset = end;
    }
    if expected_offset != code.as_bytes().len() || relocation_index != code.atom_relocations().len()
    {
        return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
            function_index: function.zero_based(),
            offset: u32::try_from(expected_offset).unwrap_or(u32::MAX),
        });
    }
    Ok(())
}

fn positive_u32(value: usize, field: FunctionField) -> Result<u32, FunctionEnvelopeError> {
    if value > MAX_QUICKJS_POSITIVE_INT {
        return Err(FunctionEnvelopeError::CountOverflow { field });
    }
    Ok(value as u32)
}

fn check_function_limit(
    limits: FunctionEnvelopeLimits,
    kind: FunctionResourceKind,
    requested: usize,
) -> Result<(), FunctionEnvelopeError> {
    let limit = limits.limit(kind);
    if requested > limit {
        return Err(FunctionEnvelopeError::ResourceLimit {
            kind,
            requested,
            limit,
        });
    }
    Ok(())
}

fn check_code_limit(
    limits: FunctionEnvelopeLimits,
    kind: CodeResourceKind,
    requested: usize,
) -> Result<(), FunctionEnvelopeError> {
    let limit = limits.code_limit(kind);
    if requested > limit {
        // The decoder knows byte length from the prefix, but discovers
        // instructions and atom relocations one at a time while scanning.
        // Preserve that observable requested count under stricter re-encode
        // policies instead of reporting the final sidecar length.
        let requested = match kind {
            CodeResourceKind::Bytes => requested,
            CodeResourceKind::Instructions | CodeResourceKind::AtomRelocations => {
                limit.checked_add(1).ok_or(FunctionEnvelopeError::Code(
                    CodeError::CountOverflow { kind },
                ))?
            }
        };
        return Err(FunctionEnvelopeError::Code(CodeError::ResourceLimit {
            kind,
            requested,
            limit,
        }));
    }
    Ok(())
}
