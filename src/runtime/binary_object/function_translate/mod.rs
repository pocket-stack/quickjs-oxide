//! Central release-pinned translation from native operand plans to sanitized code.
//!
//! `NativeCodePlan` has exactly one production consumer: this module. Narrow
//! public admission paths operate only on the semantic DTO from `dto` and keep
//! their pre-existing physical-opcode audiences unchanged.

mod capability;
mod dto;

use std::fmt;

use super::bytecode_image::{
    BytecodeImage, FunctionId, NativeAtomClass, NativeAtomRef, NativeCodePlan, NativeOperands,
    decode_native_code_plan,
};
use super::wire::WireString;
use capability::{Recipe, operand_shape, row_for};
use dto::InstructionAudience;

pub(in crate::runtime::binary_object) use dto::{
    AtomOperand, AtomOperandClass, FunctionCode, FunctionInstruction, FunctionOp, FunctionUnaryOp,
    OperandShape, OperationDiagnostic, TranslationBlocker,
};

/// Selects which unchanged public cohort may materialize translated operands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum TranslationTarget {
    Scalar,
    Ordinary,
}

impl TranslationTarget {
    const fn accepts(self, audience: InstructionAudience) -> bool {
        match self {
            Self::Scalar => audience.includes_scalar(),
            Self::Ordinary => audience.includes_ordinary(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FunctionTranslateErrorKind {
    NativePlan {
        diagnostic: String,
        label_target: bool,
    },
    RegistryDrift {
        diagnostic: &'static str,
        expected: OperandShape,
        descriptor: OperandShape,
        decoded: OperandShape,
    },
    AllocationFailed,
    InstructionCountOverflow,
    InvalidBranchTarget,
    AtomProjectionInvariant,
}

/// Translation failure without structured native PCs, opcode bytes, or image IDs.
/// Compatibility diagnostics from the sealed native-plan layer remain textual.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) struct FunctionTranslateError {
    kind: FunctionTranslateErrorKind,
}

impl FunctionTranslateError {
    fn native_plan(diagnostic: String, label_target: bool) -> Self {
        Self {
            kind: FunctionTranslateErrorKind::NativePlan {
                diagnostic,
                label_target,
            },
        }
    }

    fn registry_drift(
        diagnostic: &'static str,
        expected: OperandShape,
        descriptor: OperandShape,
        decoded: OperandShape,
    ) -> Self {
        Self {
            kind: FunctionTranslateErrorKind::RegistryDrift {
                diagnostic,
                expected,
                descriptor,
                decoded,
            },
        }
    }

    fn allocation_failed() -> Self {
        Self {
            kind: FunctionTranslateErrorKind::AllocationFailed,
        }
    }

    fn instruction_count_overflow() -> Self {
        Self {
            kind: FunctionTranslateErrorKind::InstructionCountOverflow,
        }
    }

    fn invalid_branch_target() -> Self {
        Self {
            kind: FunctionTranslateErrorKind::InvalidBranchTarget,
        }
    }

    fn atom_projection_invariant() -> Self {
        Self {
            kind: FunctionTranslateErrorKind::AtomProjectionInvariant,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn is_label_target_error(&self) -> bool {
        matches!(
            self.kind,
            FunctionTranslateErrorKind::NativePlan {
                label_target: true,
                ..
            }
        )
    }
}

impl fmt::Display for FunctionTranslateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            FunctionTranslateErrorKind::NativePlan { diagnostic, .. } => {
                formatter.write_str(diagnostic)
            }
            FunctionTranslateErrorKind::RegistryDrift {
                diagnostic,
                expected,
                descriptor,
                decoded,
            } => write!(
                formatter,
                "function capability row for {diagnostic} expects {expected:?}, pinned descriptor is {descriptor:?}, decoded operands are {decoded:?}"
            ),
            FunctionTranslateErrorKind::AllocationFailed => {
                formatter.write_str("function translation allocation failed")
            }
            FunctionTranslateErrorKind::InstructionCountOverflow => {
                formatter.write_str("function instruction count exceeds the sanitized index space")
            }
            FunctionTranslateErrorKind::InvalidBranchTarget => formatter
                .write_str("authenticated native label did not resolve in the instruction map"),
            FunctionTranslateErrorKind::AtomProjectionInvariant => {
                formatter.write_str("String atom projection contained no spelling")
            }
        }
    }
}

impl std::error::Error for FunctionTranslateError {}

enum PendingOperation<'image> {
    Ready(FunctionOp<'image>),
    IfFalse(u32),
    Goto(u32),
}

struct PendingInstruction<'image> {
    audience: InstructionAudience,
    diagnostic: OperationDiagnostic,
    operation: PendingOperation<'image>,
}

/// Translate one authenticated function under the semantic union of the two
/// existing admission cohorts. The target controls operand materialization;
/// the physical audience still prevents normalization from admitting an
/// alternate spelling downstream.
pub(in crate::runtime::binary_object) fn translate_function<'image>(
    image: &'image BytecodeImage,
    function: FunctionId,
    target: TranslationTarget,
) -> Result<FunctionCode<'image>, FunctionTranslateError> {
    let plan = decode_native_code_plan(image, function).map_err(|error| {
        FunctionTranslateError::native_plan(error.to_string(), error.is_label_target_error())
    })?;
    translate_native_plan(&plan, target)
}

/// First classify and normalize operands, then resolve semantic CFG targets.
/// Keeping these passes separate gives future multi-instruction recipes one
/// place to update the source-to-output instruction map.
fn translate_native_plan<'image>(
    plan: &NativeCodePlan<'image>,
    target: TranslationTarget,
) -> Result<FunctionCode<'image>, FunctionTranslateError> {
    let mut source_to_output = Vec::new();
    source_to_output
        .try_reserve_exact(plan.instructions().len())
        .map_err(|_| FunctionTranslateError::allocation_failed())?;
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(plan.instructions().len())
        .map_err(|_| FunctionTranslateError::allocation_failed())?;

    for instruction in plan.instructions() {
        let opcode = instruction.opcode();
        let row = row_for(opcode);
        let descriptor_shape = operand_shape(opcode.format());
        let decoded_shape = operand_shape(instruction.operands().format());
        let expected_shape = operand_shape(row.expected_format);
        if row.raw != opcode.raw()
            || row.expected_format != opcode.format()
            || row.expected_format != instruction.operands().format()
        {
            return Err(FunctionTranslateError::registry_drift(
                opcode.name(),
                expected_shape,
                descriptor_shape,
                decoded_shape,
            ));
        }

        let diagnostic = OperationDiagnostic::new(opcode.name(), expected_shape);
        let output_index = u32::try_from(pending.len())
            .map_err(|_| FunctionTranslateError::instruction_count_overflow())?;
        source_to_output.push(output_index);
        let (audience, operation) = match row.policy {
            capability::CapabilityPolicy::Blocked(blocker) => (
                InstructionAudience::Blocked,
                PendingOperation::Ready(FunctionOp::Blocked(blocker)),
            ),
            capability::CapabilityPolicy::ScalarOnly(recipe) => (
                InstructionAudience::ScalarOnly,
                operation_for_target(
                    InstructionAudience::ScalarOnly,
                    recipe,
                    target,
                    instruction.operands(),
                )?,
            ),
            capability::CapabilityPolicy::OrdinaryOnly(recipe) => (
                InstructionAudience::OrdinaryOnly,
                operation_for_target(
                    InstructionAudience::OrdinaryOnly,
                    recipe,
                    target,
                    instruction.operands(),
                )?,
            ),
            capability::CapabilityPolicy::Shared(recipe) => (
                InstructionAudience::Shared,
                operation_for_target(
                    InstructionAudience::Shared,
                    recipe,
                    target,
                    instruction.operands(),
                )?,
            ),
        };
        pending.push(PendingInstruction {
            audience,
            diagnostic,
            operation,
        });
    }

    let mut output = Vec::new();
    output
        .try_reserve_exact(pending.len())
        .map_err(|_| FunctionTranslateError::allocation_failed())?;
    for instruction in pending {
        let operation = match instruction.operation {
            PendingOperation::Ready(operation) => operation,
            PendingOperation::IfFalse(target) => {
                FunctionOp::IfFalse(resolve_target(&source_to_output, target)?)
            }
            PendingOperation::Goto(target) => {
                FunctionOp::Goto(resolve_target(&source_to_output, target)?)
            }
        };
        output.push(FunctionInstruction::new(
            instruction.audience,
            instruction.diagnostic,
            operation,
        ));
    }
    Ok(FunctionCode::new(output.into_boxed_slice()))
}

fn operation_for_target<'image>(
    audience: InstructionAudience,
    recipe: Recipe,
    target: TranslationTarget,
    operands: &NativeOperands<'image>,
) -> Result<PendingOperation<'image>, FunctionTranslateError> {
    if target.accepts(audience) {
        lower_operation(recipe, operands)
    } else {
        Ok(PendingOperation::Ready(FunctionOp::OutsideTarget))
    }
}

fn lower_operation<'image>(
    recipe: Recipe,
    operands: &NativeOperands<'image>,
) -> Result<PendingOperation<'image>, FunctionTranslateError> {
    let ready = |operation| Ok(PendingOperation::Ready(operation));
    match (recipe, operands) {
        (Recipe::PushI32, NativeOperands::I32(value) | NativeOperands::NoneInt(value)) => {
            ready(FunctionOp::PushI32(*value))
        }
        (Recipe::PushI32, NativeOperands::I8(value)) => {
            ready(FunctionOp::PushI32(i32::from(*value)))
        }
        (Recipe::PushI32, NativeOperands::I16(value)) => {
            ready(FunctionOp::PushI32(i32::from(*value)))
        }
        (Recipe::PushConstant, NativeOperands::Const(index)) => {
            ready(FunctionOp::PushConstant(*index))
        }
        (Recipe::PushConstant, NativeOperands::Const8(index)) => {
            ready(FunctionOp::PushConstant(u32::from(*index)))
        }
        (Recipe::PushAtom, NativeOperands::Atom(atom)) => {
            ready(FunctionOp::PushAtom(project_atom(*atom)?))
        }
        (Recipe::PushUndefined, NativeOperands::None) => ready(FunctionOp::PushUndefined),
        (Recipe::PushNull, NativeOperands::None) => ready(FunctionOp::PushNull),
        (Recipe::PushFalse, NativeOperands::None) => ready(FunctionOp::PushBool(false)),
        (Recipe::PushTrue, NativeOperands::None) => ready(FunctionOp::PushBool(true)),
        (Recipe::PushBigIntI32, NativeOperands::I32(value)) => {
            ready(FunctionOp::PushBigIntI32(*value))
        }
        (Recipe::PushEmptyString, NativeOperands::None) => ready(FunctionOp::PushEmptyString),
        (Recipe::Unary(operation), NativeOperands::None) => ready(FunctionOp::Unary(operation)),
        (Recipe::GetLocal, NativeOperands::Loc(index) | NativeOperands::NoneLoc(index)) => {
            ready(FunctionOp::GetLocal(*index))
        }
        (Recipe::GetLocal, NativeOperands::Loc8(index)) => {
            ready(FunctionOp::GetLocal(u16::from(*index)))
        }
        (Recipe::PutLocal, NativeOperands::Loc(index) | NativeOperands::NoneLoc(index)) => {
            ready(FunctionOp::PutLocal(*index))
        }
        (Recipe::PutLocal, NativeOperands::Loc8(index)) => {
            ready(FunctionOp::PutLocal(u16::from(*index)))
        }
        (Recipe::SetLocal, NativeOperands::Loc(index) | NativeOperands::NoneLoc(index)) => {
            ready(FunctionOp::SetLocal(*index))
        }
        (Recipe::SetLocal, NativeOperands::Loc8(index)) => {
            ready(FunctionOp::SetLocal(u16::from(*index)))
        }
        (Recipe::GetArgument, NativeOperands::Arg(index) | NativeOperands::NoneArg(index)) => {
            ready(FunctionOp::GetArgument(*index))
        }
        (Recipe::PutArgument, NativeOperands::Arg(index) | NativeOperands::NoneArg(index)) => {
            ready(FunctionOp::PutArgument(*index))
        }
        (Recipe::SetArgument, NativeOperands::Arg(index) | NativeOperands::NoneArg(index)) => {
            ready(FunctionOp::SetArgument(*index))
        }
        (Recipe::Add, NativeOperands::None) => ready(FunctionOp::Add),
        (Recipe::Sub, NativeOperands::None) => ready(FunctionOp::Sub),
        (Recipe::Div, NativeOperands::None) => ready(FunctionOp::Div),
        (Recipe::GreaterThan, NativeOperands::None) => ready(FunctionOp::GreaterThan),
        (Recipe::StrictEqual, NativeOperands::None) => ready(FunctionOp::StrictEqual),
        (Recipe::IfFalse, NativeOperands::Label(label)) => {
            Ok(PendingOperation::IfFalse(label.target_instruction()))
        }
        (Recipe::IfFalse, NativeOperands::Label8(label)) => {
            Ok(PendingOperation::IfFalse(label.target_instruction()))
        }
        (Recipe::Goto, NativeOperands::Label(label)) => {
            Ok(PendingOperation::Goto(label.target_instruction()))
        }
        (Recipe::Goto, NativeOperands::Label8(label)) => {
            Ok(PendingOperation::Goto(label.target_instruction()))
        }
        (Recipe::Goto, NativeOperands::Label16(label)) => {
            Ok(PendingOperation::Goto(label.target_instruction()))
        }
        (Recipe::Return, NativeOperands::None) => ready(FunctionOp::Return),
        _ => Err(FunctionTranslateError::registry_drift(
            "translated operation",
            operand_shape(operands.format()),
            operand_shape(operands.format()),
            operand_shape(operands.format()),
        )),
    }
}

fn project_atom<'image>(
    atom: NativeAtomRef<'image>,
) -> Result<AtomOperand<'image>, FunctionTranslateError> {
    let from_input_atom_table = atom.originates_from_input_atom_table();
    match atom.class() {
        NativeAtomClass::Null => Ok(AtomOperand::null(from_input_atom_table)),
        NativeAtomClass::Index => atom
            .index()
            .map(|index| AtomOperand::index(index, from_input_atom_table))
            .ok_or_else(FunctionTranslateError::atom_projection_invariant),
        NativeAtomClass::String => {
            if let Some(spelling) = atom.manifest_string() {
                return Ok(AtomOperand::manifest_string(
                    spelling,
                    from_input_atom_table,
                ));
            }
            let spelling = atom
                .dynamic_string()
                .ok_or_else(FunctionTranslateError::atom_projection_invariant)?;
            Ok(match spelling {
                WireString::Narrow(bytes) => AtomOperand::byte_string(bytes, from_input_atom_table),
                WireString::Wide(units) => AtomOperand::utf16_string(units, from_input_atom_table),
            })
        }
        NativeAtomClass::Private => Ok(AtomOperand::private(from_input_atom_table)),
        NativeAtomClass::Symbol => Ok(AtomOperand::symbol(from_input_atom_table)),
    }
}

fn resolve_target(
    source_to_output: &[u32],
    target_instruction: u32,
) -> Result<u32, FunctionTranslateError> {
    source_to_output
        .get(target_instruction as usize)
        .copied()
        .ok_or_else(FunctionTranslateError::invalid_branch_target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumer_filter_precedes_operand_materialization() {
        let operation = operation_for_target(
            InstructionAudience::ScalarOnly,
            Recipe::PushAtom,
            TranslationTarget::Ordinary,
            &NativeOperands::None,
        )
        .expect("an out-of-audience operand is not materialized");
        assert!(matches!(
            operation,
            PendingOperation::Ready(FunctionOp::OutsideTarget)
        ));
    }

    #[test]
    fn semantic_lowering_has_no_mnemonic_or_diagnostic_input() {
        let operation = lower_operation(Recipe::PushI32, &NativeOperands::I32(42)).unwrap();
        assert!(matches!(
            operation,
            PendingOperation::Ready(FunctionOp::PushI32(42))
        ));
    }
}
