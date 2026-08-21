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
    AtomOperand, AtomOperandClass, FunctionApplyKind, FunctionBinaryOp, FunctionCode,
    FunctionInstruction, FunctionOp, FunctionPredicateOp, FunctionStackOp, FunctionUnaryOp,
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
    NonCanonicalApplyMagic(u16),
    UnadmittedThrowErrorSubtype(u8),
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

    fn non_canonical_apply_magic(magic: u16) -> Self {
        Self {
            kind: FunctionTranslateErrorKind::NonCanonicalApplyMagic(magic),
        }
    }

    fn unadmitted_throw_error_subtype(subtype: u8) -> Self {
        Self {
            kind: FunctionTranslateErrorKind::UnadmittedThrowErrorSubtype(subtype),
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

    #[must_use]
    pub(in crate::runtime::binary_object) const fn is_unadmitted_operand_error(&self) -> bool {
        matches!(
            self.kind,
            FunctionTranslateErrorKind::NonCanonicalApplyMagic(_)
                | FunctionTranslateErrorKind::UnadmittedThrowErrorSubtype(_)
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
            FunctionTranslateErrorKind::NonCanonicalApplyMagic(magic) => write!(
                formatter,
                "apply operand must be canonical 0 (call) or 1 (construct), found {magic}"
            ),
            FunctionTranslateErrorKind::UnadmittedThrowErrorSubtype(subtype) => write!(
                formatter,
                "throw_error subtype is outside the admitted read-only subtype 0: found {subtype}"
            ),
        }
    }
}

impl std::error::Error for FunctionTranslateError {}

enum PendingOperation<'image> {
    Ready(FunctionOp<'image>),
    IfFalse(u32),
    IfTrue(u32),
    Goto(u32),
}

struct PendingExpansion<'image> {
    operations: [Option<PendingOperation<'image>>; 4],
    len: u8,
}

impl<'image> PendingExpansion<'image> {
    fn one(operation: PendingOperation<'image>) -> Self {
        Self {
            operations: [Some(operation), None, None, None],
            len: 1,
        }
    }

    fn two(first: PendingOperation<'image>, second: PendingOperation<'image>) -> Self {
        Self {
            operations: [Some(first), Some(second), None, None],
            len: 2,
        }
    }

    fn three(
        first: PendingOperation<'image>,
        second: PendingOperation<'image>,
        third: PendingOperation<'image>,
    ) -> Self {
        Self {
            operations: [Some(first), Some(second), Some(third), None],
            len: 3,
        }
    }

    fn four(
        first: PendingOperation<'image>,
        second: PendingOperation<'image>,
        third: PendingOperation<'image>,
        fourth: PendingOperation<'image>,
    ) -> Self {
        Self {
            operations: [Some(first), Some(second), Some(third), Some(fourth)],
            len: 4,
        }
    }

    const fn len(&self) -> usize {
        self.len as usize
    }

    fn into_operations(self) -> impl Iterator<Item = PendingOperation<'image>> {
        self.operations
            .into_iter()
            .take(usize::from(self.len))
            .flatten()
    }
}

struct PendingInstruction<'image> {
    audience: InstructionAudience,
    diagnostic: OperationDiagnostic,
    expansion: PendingExpansion<'image>,
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
    let mut output_len = 0_usize;

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
        let output_index = u32::try_from(output_len)
            .map_err(|_| FunctionTranslateError::instruction_count_overflow())?;
        source_to_output.push(output_index);
        let (audience, expansion) = match row.policy {
            capability::CapabilityPolicy::Blocked(blocker) => (
                InstructionAudience::Blocked,
                PendingExpansion::one(PendingOperation::Ready(FunctionOp::Blocked(blocker))),
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
        output_len = output_len
            .checked_add(expansion.len())
            .ok_or_else(FunctionTranslateError::instruction_count_overflow)?;
        u32::try_from(output_len)
            .map_err(|_| FunctionTranslateError::instruction_count_overflow())?;
        pending.push(PendingInstruction {
            audience,
            diagnostic,
            expansion,
        });
    }

    let mut output = Vec::new();
    output
        .try_reserve_exact(output_len)
        .map_err(|_| FunctionTranslateError::allocation_failed())?;
    for instruction in pending {
        for operation in instruction.expansion.into_operations() {
            let operation = match operation {
                PendingOperation::Ready(operation) => operation,
                PendingOperation::IfFalse(target) => {
                    FunctionOp::IfFalse(resolve_target(&source_to_output, target)?)
                }
                PendingOperation::IfTrue(target) => {
                    FunctionOp::IfTrue(resolve_target(&source_to_output, target)?)
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
    }
    Ok(FunctionCode::new(output.into_boxed_slice()))
}

fn operation_for_target<'image>(
    audience: InstructionAudience,
    recipe: Recipe,
    target: TranslationTarget,
    operands: &NativeOperands<'image>,
) -> Result<PendingExpansion<'image>, FunctionTranslateError> {
    if target.accepts(audience) {
        lower_operation(recipe, operands)
    } else {
        Ok(PendingExpansion::one(PendingOperation::Ready(
            FunctionOp::OutsideTarget,
        )))
    }
}

fn lower_operation<'image>(
    recipe: Recipe,
    operands: &NativeOperands<'image>,
) -> Result<PendingExpansion<'image>, FunctionTranslateError> {
    let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));
    match (recipe, operands) {
        (Recipe::Nop, NativeOperands::None) => ready(FunctionOp::Nop),
        (Recipe::Object, NativeOperands::None) => ready(FunctionOp::Object),
        (Recipe::ToObject, NativeOperands::None) => ready(FunctionOp::ToObject),
        (Recipe::PushThis, NativeOperands::None) => ready(FunctionOp::PushThis),
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
        (Recipe::Stack(capability::StackRecipe::Direct(operation)), NativeOperands::None) => {
            ready(FunctionOp::Stack(operation))
        }
        (Recipe::Stack(capability::StackRecipe::Nip1), NativeOperands::None) => {
            Ok(PendingExpansion::two(
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Perm3)),
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Nip)),
            ))
        }
        (Recipe::Stack(capability::StackRecipe::Dup2), NativeOperands::None) => {
            Ok(PendingExpansion::three(
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Dup1)),
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Dup)),
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Perm3)),
            ))
        }
        (Recipe::Stack(capability::StackRecipe::Swap2), NativeOperands::None) => {
            Ok(PendingExpansion::two(
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Rot4Left)),
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Rot4Left)),
            ))
        }
        (Recipe::Stack(capability::StackRecipe::Rot3Left), NativeOperands::None) => {
            Ok(PendingExpansion::two(
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Perm3)),
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Swap)),
            ))
        }
        (Recipe::Stack(capability::StackRecipe::Rot3Right), NativeOperands::None) => {
            Ok(PendingExpansion::two(
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Swap)),
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Perm3)),
            ))
        }
        (Recipe::Stack(capability::StackRecipe::Rot5Left), NativeOperands::None) => {
            Ok(PendingExpansion::four(
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Perm4)),
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Perm4)),
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Perm5)),
                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Rot4Left)),
            ))
        }
        (Recipe::Unary(operation), NativeOperands::None) => ready(FunctionOp::Unary(operation)),
        (Recipe::PostDec, NativeOperands::None) => ready(FunctionOp::PostDec),
        (Recipe::PostInc, NativeOperands::None) => ready(FunctionOp::PostInc),
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
        (Recipe::Binary(operation), NativeOperands::None) => ready(FunctionOp::Binary(operation)),
        (Recipe::Predicate(operation), NativeOperands::None) => {
            ready(FunctionOp::Predicate(operation))
        }
        (Recipe::IfFalse, NativeOperands::Label(label)) => Ok(PendingExpansion::one(
            PendingOperation::IfFalse(label.target_instruction()),
        )),
        (Recipe::IfFalse, NativeOperands::Label8(label)) => Ok(PendingExpansion::one(
            PendingOperation::IfFalse(label.target_instruction()),
        )),
        (Recipe::IfTrue, NativeOperands::Label(label)) => Ok(PendingExpansion::one(
            PendingOperation::IfTrue(label.target_instruction()),
        )),
        (Recipe::IfTrue, NativeOperands::Label8(label)) => Ok(PendingExpansion::one(
            PendingOperation::IfTrue(label.target_instruction()),
        )),
        (Recipe::Goto, NativeOperands::Label(label)) => Ok(PendingExpansion::one(
            PendingOperation::Goto(label.target_instruction()),
        )),
        (Recipe::Goto, NativeOperands::Label8(label)) => Ok(PendingExpansion::one(
            PendingOperation::Goto(label.target_instruction()),
        )),
        (Recipe::Goto, NativeOperands::Label16(label)) => Ok(PendingExpansion::one(
            PendingOperation::Goto(label.target_instruction()),
        )),
        (
            Recipe::Call,
            NativeOperands::NPop(argument_count) | NativeOperands::NPopX(argument_count),
        ) => ready(FunctionOp::Call(*argument_count)),
        (Recipe::TailCall, NativeOperands::NPop(argument_count)) => {
            ready(FunctionOp::TailCall(*argument_count))
        }
        (Recipe::Construct, NativeOperands::NPop(argument_count)) => {
            ready(FunctionOp::Construct(*argument_count))
        }
        (Recipe::CallMethod, NativeOperands::NPop(argument_count)) => {
            ready(FunctionOp::CallMethod(*argument_count))
        }
        (Recipe::TailCallMethod, NativeOperands::NPop(argument_count)) => {
            ready(FunctionOp::TailCallMethod(*argument_count))
        }
        (Recipe::ArrayFrom, NativeOperands::NPop(argument_count)) => {
            ready(FunctionOp::ArrayFrom(*argument_count))
        }
        (Recipe::Apply, NativeOperands::U16(0)) => {
            ready(FunctionOp::Apply(FunctionApplyKind::Call))
        }
        (Recipe::Apply, NativeOperands::U16(1)) => {
            ready(FunctionOp::Apply(FunctionApplyKind::Construct))
        }
        (Recipe::Apply, NativeOperands::U16(magic)) => {
            Err(FunctionTranslateError::non_canonical_apply_magic(*magic))
        }
        (Recipe::Return, NativeOperands::None) => ready(FunctionOp::Return),
        (Recipe::ReturnUndefined, NativeOperands::None) => ready(FunctionOp::ReturnUndefined),
        (Recipe::Throw, NativeOperands::None) => ready(FunctionOp::Throw),
        (Recipe::ThrowReadOnly, NativeOperands::AtomU8 { atom, value: 0 }) => {
            ready(FunctionOp::ThrowReadOnly(project_atom(*atom)?))
        }
        (Recipe::ThrowReadOnly, NativeOperands::AtomU8 { value, .. }) => Err(
            FunctionTranslateError::unadmitted_throw_error_subtype(*value),
        ),
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
        let expansion = operation_for_target(
            InstructionAudience::ScalarOnly,
            Recipe::PushAtom,
            TranslationTarget::Ordinary,
            &NativeOperands::None,
        )
        .expect("an out-of-audience operand is not materialized");
        let mut operations = expansion.into_operations();
        assert!(matches!(
            operations.next(),
            Some(PendingOperation::Ready(FunctionOp::OutsideTarget))
        ));
        assert!(matches!(operations.next(), None));
    }

    #[test]
    fn semantic_lowering_has_no_mnemonic_or_diagnostic_input() {
        let expansion = lower_operation(Recipe::PushI32, &NativeOperands::I32(42)).unwrap();
        let mut operations = expansion.into_operations();
        assert!(matches!(
            operations.next(),
            Some(PendingOperation::Ready(FunctionOp::PushI32(42)))
        ));
        assert!(operations.next().is_none());
    }

    #[test]
    fn operand_free_nop_translation_is_one_typed_operation() {
        let expansion = lower_operation(Recipe::Nop, &NativeOperands::None).unwrap();
        assert_eq!(expansion.len(), 1);
        let mut operations = expansion.into_operations();
        assert!(matches!(
            operations.next(),
            Some(PendingOperation::Ready(FunctionOp::Nop))
        ));
        assert!(operations.next().is_none());

        let scalar = operation_for_target(
            InstructionAudience::OrdinaryOnly,
            Recipe::Nop,
            TranslationTarget::Scalar,
            &NativeOperands::None,
        )
        .unwrap();
        assert!(matches!(
            scalar.into_operations().next(),
            Some(PendingOperation::Ready(FunctionOp::OutsideTarget))
        ));
    }

    #[test]
    fn operand_free_object_translation_is_one_ordinary_typed_operation() {
        let expansion = lower_operation(Recipe::Object, &NativeOperands::None).unwrap();
        assert_eq!(expansion.len(), 1);
        let mut operations = expansion.into_operations();
        assert!(matches!(
            operations.next(),
            Some(PendingOperation::Ready(FunctionOp::Object))
        ));
        assert!(operations.next().is_none());

        let scalar = operation_for_target(
            InstructionAudience::OrdinaryOnly,
            Recipe::Object,
            TranslationTarget::Scalar,
            &NativeOperands::None,
        )
        .unwrap();
        assert!(matches!(
            scalar.into_operations().next(),
            Some(PendingOperation::Ready(FunctionOp::OutsideTarget))
        ));
    }

    #[test]
    fn operand_free_to_object_translation_is_one_ordinary_typed_operation() {
        let expansion = lower_operation(Recipe::ToObject, &NativeOperands::None).unwrap();
        assert_eq!(expansion.len(), 1);
        let mut operations = expansion.into_operations();
        assert!(matches!(
            operations.next(),
            Some(PendingOperation::Ready(FunctionOp::ToObject))
        ));
        assert!(operations.next().is_none());

        let scalar = operation_for_target(
            InstructionAudience::OrdinaryOnly,
            Recipe::ToObject,
            TranslationTarget::Scalar,
            &NativeOperands::None,
        )
        .unwrap();
        assert!(matches!(
            scalar.into_operations().next(),
            Some(PendingOperation::Ready(FunctionOp::OutsideTarget))
        ));
    }

    #[test]
    fn operand_free_push_this_translation_is_one_ordinary_typed_operation() {
        let expansion = lower_operation(Recipe::PushThis, &NativeOperands::None).unwrap();
        assert_eq!(expansion.len(), 1);
        let mut operations = expansion.into_operations();
        assert!(matches!(
            operations.next(),
            Some(PendingOperation::Ready(FunctionOp::PushThis))
        ));
        assert!(operations.next().is_none());

        let scalar = operation_for_target(
            InstructionAudience::OrdinaryOnly,
            Recipe::PushThis,
            TranslationTarget::Scalar,
            &NativeOperands::None,
        )
        .unwrap();
        assert!(matches!(
            scalar.into_operations().next(),
            Some(PendingOperation::Ready(FunctionOp::OutsideTarget))
        ));
    }

    #[test]
    fn plain_call_lowering_preserves_explicit_and_implicit_argument_counts() {
        for (operands, expected) in [
            (NativeOperands::NPop(4), 4),
            (NativeOperands::NPopX(0), 0),
            (NativeOperands::NPopX(1), 1),
            (NativeOperands::NPopX(2), 2),
            (NativeOperands::NPopX(3), 3),
        ] {
            let expansion = lower_operation(Recipe::Call, &operands).unwrap();
            assert_eq!(expansion.len(), 1);
            let mut operations = expansion.into_operations();
            assert!(matches!(
                operations.next(),
                Some(PendingOperation::Ready(FunctionOp::Call(argument_count)))
                    if argument_count == expected
            ));
            assert!(operations.next().is_none());
        }
    }

    #[test]
    fn non_tail_invocation_lowering_preserves_the_npop_operand() {
        for recipe in [Recipe::Construct, Recipe::CallMethod, Recipe::ArrayFrom] {
            let expansion = lower_operation(recipe, &NativeOperands::NPop(65_535)).unwrap();
            assert_eq!(expansion.len(), 1);
            let mut operations = expansion.into_operations();
            let Some(PendingOperation::Ready(actual)) = operations.next() else {
                panic!("invocation recipe produced a non-ready operation");
            };
            assert!(matches!(
                (recipe, actual),
                (Recipe::Construct, FunctionOp::Construct(65_535))
                    | (Recipe::CallMethod, FunctionOp::CallMethod(65_535))
                    | (Recipe::ArrayFrom, FunctionOp::ArrayFrom(65_535))
            ));
            assert!(operations.next().is_none());
        }
    }

    #[test]
    fn tail_invocation_lowering_preserves_the_npop_operand_and_kind() {
        for (recipe, expected_method) in [(Recipe::TailCall, false), (Recipe::TailCallMethod, true)]
        {
            let expansion = lower_operation(recipe, &NativeOperands::NPop(u16::MAX)).unwrap();
            assert_eq!(expansion.len(), 1);
            let mut operations = expansion.into_operations();
            let Some(PendingOperation::Ready(actual)) = operations.next() else {
                panic!("tail invocation recipe produced a non-ready operation");
            };
            assert!(matches!(
                (expected_method, actual),
                (false, FunctionOp::TailCall(u16::MAX))
                    | (true, FunctionOp::TailCallMethod(u16::MAX))
            ));
            assert!(operations.next().is_none());
        }
    }

    #[test]
    fn explicit_throw_lowering_is_typed_and_operand_free() {
        let expansion = lower_operation(Recipe::Throw, &NativeOperands::None).unwrap();
        assert_eq!(expansion.len(), 1);
        let mut operations = expansion.into_operations();
        assert!(matches!(
            operations.next(),
            Some(PendingOperation::Ready(FunctionOp::Throw))
        ));
        assert!(operations.next().is_none());
    }

    #[test]
    fn apply_lowering_accepts_only_the_two_canonical_magic_values() {
        for (magic, expected) in [
            (0, FunctionApplyKind::Call),
            (1, FunctionApplyKind::Construct),
        ] {
            let expansion = lower_operation(Recipe::Apply, &NativeOperands::U16(magic)).unwrap();
            assert_eq!(expansion.len(), 1);
            assert!(matches!(
                expansion.into_operations().next(),
                Some(PendingOperation::Ready(FunctionOp::Apply(actual))) if actual == expected
            ));
        }

        for magic in [2, u16::MAX] {
            let Err(error) = lower_operation(Recipe::Apply, &NativeOperands::U16(magic)) else {
                panic!("noncanonical apply magic was admitted");
            };
            assert!(error.is_unadmitted_operand_error());
            assert_eq!(
                error.to_string(),
                format!("apply operand must be canonical 0 (call) or 1 (construct), found {magic}")
            );
        }
    }

    #[test]
    fn reviewed_stack_recipes_expand_to_the_exact_typed_sequences() {
        fn expansion(recipe: capability::StackRecipe) -> Vec<FunctionStackOp> {
            lower_operation(Recipe::Stack(recipe), &NativeOperands::None)
                .unwrap()
                .into_operations()
                .map(|operation| match operation {
                    PendingOperation::Ready(FunctionOp::Stack(operation)) => operation,
                    _ => panic!("stack recipe produced a non-stack operation"),
                })
                .collect()
        }

        for operation in [
            FunctionStackOp::Drop,
            FunctionStackOp::Nip,
            FunctionStackOp::Dup,
            FunctionStackOp::Dup1,
            FunctionStackOp::Dup3,
            FunctionStackOp::Insert2,
            FunctionStackOp::Insert3,
            FunctionStackOp::Insert4,
            FunctionStackOp::Perm3,
            FunctionStackOp::Perm4,
            FunctionStackOp::Perm5,
            FunctionStackOp::Swap,
            FunctionStackOp::Rot4Left,
        ] {
            assert_eq!(
                expansion(capability::StackRecipe::Direct(operation)),
                [operation]
            );
        }

        assert_eq!(
            expansion(capability::StackRecipe::Nip1),
            [FunctionStackOp::Perm3, FunctionStackOp::Nip]
        );
        assert_eq!(
            expansion(capability::StackRecipe::Dup2),
            [
                FunctionStackOp::Dup1,
                FunctionStackOp::Dup,
                FunctionStackOp::Perm3,
            ]
        );
        assert_eq!(
            expansion(capability::StackRecipe::Swap2),
            [FunctionStackOp::Rot4Left, FunctionStackOp::Rot4Left]
        );
        assert_eq!(
            expansion(capability::StackRecipe::Rot3Left),
            [FunctionStackOp::Perm3, FunctionStackOp::Swap]
        );
        assert_eq!(
            expansion(capability::StackRecipe::Rot3Right),
            [FunctionStackOp::Swap, FunctionStackOp::Perm3]
        );
        assert_eq!(
            expansion(capability::StackRecipe::Rot5Left),
            [
                FunctionStackOp::Perm4,
                FunctionStackOp::Perm4,
                FunctionStackOp::Perm5,
                FunctionStackOp::Rot4Left,
            ]
        );
    }

    #[test]
    fn target_filter_keeps_one_physical_rejection_without_materializing_operands() {
        let expansion = operation_for_target(
            InstructionAudience::OrdinaryOnly,
            Recipe::Stack(capability::StackRecipe::Rot5Left),
            TranslationTarget::Scalar,
            &NativeOperands::I32(42),
        )
        .expect("outside-target stack operands are not materialized");
        assert_eq!(expansion.len(), 1);
        assert!(expansion.into_operations().all(|operation| matches!(
            operation,
            PendingOperation::Ready(FunctionOp::OutsideTarget)
        )));
    }
}
