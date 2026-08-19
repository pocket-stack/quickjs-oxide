//! Exhaustive capability policy for the pinned final-opcode namespace.
//!
//! Each row is keyed only by its raw final-opcode byte. The mnemonic is never
//! consulted to choose a lowering path; it is retained by the pinned catalog
//! solely for compatibility diagnostics.

use crate::runtime::binary_object::pinned_opcodes::{
    OpcodeFormat, PINNED_OPCODE_COUNT, PinnedOpcode,
};

use super::TranslationBlocker;
use super::dto::{
    FunctionBinaryOp, FunctionPredicateOp, FunctionStackOp, FunctionUnaryOp, OperandShape,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StackRecipe {
    Direct(FunctionStackOp),
    Nip1,
    Dup2,
    Swap2,
    Rot3Left,
    Rot3Right,
    Rot5Left,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Recipe {
    PushI32,
    PushConstant,
    PushAtom,
    PushUndefined,
    PushNull,
    PushFalse,
    PushTrue,
    PushBigIntI32,
    PushEmptyString,
    Stack(StackRecipe),
    Unary(FunctionUnaryOp),
    PostDec,
    PostInc,
    GetLocal,
    PutLocal,
    SetLocal,
    GetArgument,
    PutArgument,
    SetArgument,
    Binary(FunctionBinaryOp),
    Predicate(FunctionPredicateOp),
    IfFalse,
    IfTrue,
    Goto,
    Call,
    TailCall,
    Construct,
    CallMethod,
    TailCallMethod,
    ArrayFrom,
    Apply,
    Return,
    ReturnUndefined,
    Throw,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CapabilityPolicy {
    Blocked(TranslationBlocker),
    ScalarOnly(Recipe),
    OrdinaryOnly(Recipe),
    Shared(Recipe),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CapabilityRow {
    pub(super) raw: u8,
    pub(super) expected_format: OpcodeFormat,
    pub(super) policy: CapabilityPolicy,
}

impl CapabilityRow {
    const fn new(raw: u8, expected_format: OpcodeFormat, policy: CapabilityPolicy) -> Self {
        Self {
            raw,
            expected_format,
            policy,
        }
    }
}

macro_rules! row {
    ($raw:literal, $format:ident, Blocked, $blocker:ident) => {
        CapabilityRow::new(
            $raw,
            OpcodeFormat::$format,
            CapabilityPolicy::Blocked(TranslationBlocker::$blocker),
        )
    };
    ($raw:literal, $format:ident, $audience:ident, $recipe:expr) => {
        CapabilityRow::new(
            $raw,
            OpcodeFormat::$format,
            CapabilityPolicy::$audience($recipe),
        )
    };
}

/// One explicit policy row for each QuickJS 2026-06-04 final opcode.
///
/// Counts are locked by tests: 116 Blocked, 1 ScalarOnly, 98 OrdinaryOnly,
/// and 29 Shared.
#[rustfmt::skip]
pub(super) const CAPABILITY_REGISTRY: [CapabilityRow; PINNED_OPCODE_COUNT] = [
    row!(0, None, Blocked, InvalidSentinel),
    row!(1, I32, Shared, Recipe::PushI32),
    row!(2, Const, Shared, Recipe::PushConstant),
    row!(3, Const, Blocked, FunctionGraph),
    row!(4, Atom, ScalarOnly, Recipe::PushAtom),
    row!(5, Atom, Blocked, ValueConstruction),
    row!(6, None, Shared, Recipe::PushUndefined),
    row!(7, None, Shared, Recipe::PushNull),
    row!(8, None, Blocked, ValueConstruction),
    row!(9, None, Shared, Recipe::PushFalse),
    row!(10, None, Shared, Recipe::PushTrue),
    row!(11, None, Blocked, ValueConstruction),
    row!(12, U8, Blocked, ValueConstruction),
    row!(13, U16, Blocked, ValueConstruction),
    row!(14, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Drop))),
    row!(15, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Nip))),
    row!(16, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Nip1)),
    row!(17, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Dup))),
    row!(18, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Dup1))),
    row!(19, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Dup2)),
    row!(20, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Dup3))),
    row!(21, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Insert2))),
    row!(22, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Insert3))),
    row!(23, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Insert4))),
    row!(24, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Perm3))),
    row!(25, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Perm4))),
    row!(26, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Perm5))),
    row!(27, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Swap))),
    row!(28, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Swap2)),
    row!(29, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Rot3Left)),
    row!(30, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Rot3Right)),
    row!(31, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Direct(FunctionStackOp::Rot4Left))),
    row!(32, None, OrdinaryOnly, Recipe::Stack(StackRecipe::Rot5Left)),
    row!(33, NPop, OrdinaryOnly, Recipe::Construct),
    row!(34, NPop, OrdinaryOnly, Recipe::Call),
    row!(35, NPop, OrdinaryOnly, Recipe::TailCall),
    row!(36, NPop, OrdinaryOnly, Recipe::CallMethod),
    row!(37, NPop, OrdinaryOnly, Recipe::TailCallMethod),
    row!(38, NPop, OrdinaryOnly, Recipe::ArrayFrom),
    row!(39, U16, OrdinaryOnly, Recipe::Apply),
    row!(40, None, Shared, Recipe::Return),
    row!(41, None, OrdinaryOnly, Recipe::ReturnUndefined),
    row!(42, None, Blocked, ObjectConstruction),
    row!(43, None, Blocked, ObjectConstruction),
    row!(44, None, Blocked, ObjectConstruction),
    row!(45, None, Blocked, ObjectConstruction),
    row!(46, None, Blocked, ObjectConstruction),
    row!(47, None, Blocked, Completion),
    row!(48, None, OrdinaryOnly, Recipe::Throw),
    row!(49, AtomU8, Blocked, Exception),
    row!(50, NPopU16, Blocked, EvalOrModule),
    row!(51, U16, Blocked, EvalOrModule),
    row!(52, None, Blocked, ValueConstruction),
    row!(53, None, Blocked, Property),
    row!(54, None, Blocked, EvalOrModule),
    row!(55, VarRef, Blocked, Binding),
    row!(56, VarRef, Blocked, Binding),
    row!(57, VarRef, Blocked, Binding),
    row!(58, VarRef, Blocked, Binding),
    row!(59, None, Blocked, Binding),
    row!(60, None, Blocked, Binding),
    row!(61, Atom, Blocked, Property),
    row!(62, Atom, Blocked, Property),
    row!(63, Atom, Blocked, Property),
    row!(64, None, Blocked, Property),
    row!(65, None, Blocked, Property),
    row!(66, None, Blocked, Property),
    row!(67, None, Blocked, Property),
    row!(68, None, Blocked, Property),
    row!(69, None, Blocked, Property),
    row!(70, None, Blocked, Property),
    row!(71, None, Blocked, Property),
    row!(72, None, Blocked, Property),
    row!(73, Atom, Blocked, Property),
    row!(74, Atom, Blocked, Property),
    row!(75, None, Blocked, ObjectConstruction),
    row!(76, None, Blocked, ObjectConstruction),
    row!(77, None, Blocked, ObjectConstruction),
    row!(78, None, Blocked, ObjectConstruction),
    row!(79, None, Blocked, ObjectConstruction),
    row!(80, U8, Blocked, ObjectConstruction),
    row!(81, AtomU8, Blocked, ObjectConstruction),
    row!(82, U8, Blocked, ObjectConstruction),
    row!(83, AtomU8, Blocked, ObjectConstruction),
    row!(84, AtomU8, Blocked, ObjectConstruction),
    row!(85, Loc, OrdinaryOnly, Recipe::GetLocal),
    row!(86, Loc, OrdinaryOnly, Recipe::PutLocal),
    row!(87, Loc, OrdinaryOnly, Recipe::SetLocal),
    row!(88, Arg, OrdinaryOnly, Recipe::GetArgument),
    row!(89, Arg, OrdinaryOnly, Recipe::PutArgument),
    row!(90, Arg, OrdinaryOnly, Recipe::SetArgument),
    row!(91, VarRef, Blocked, LexicalEnvironment),
    row!(92, VarRef, Blocked, LexicalEnvironment),
    row!(93, VarRef, Blocked, LexicalEnvironment),
    row!(94, Loc, Blocked, LexicalEnvironment),
    row!(95, Loc, Blocked, LexicalEnvironment),
    row!(96, Loc, Blocked, LexicalEnvironment),
    row!(97, Loc, Blocked, LexicalEnvironment),
    row!(98, Loc, Blocked, LexicalEnvironment),
    row!(99, Loc, Blocked, LexicalEnvironment),
    row!(100, VarRef, Blocked, LexicalEnvironment),
    row!(101, VarRef, Blocked, LexicalEnvironment),
    row!(102, VarRef, Blocked, LexicalEnvironment),
    row!(103, Loc, Blocked, LexicalEnvironment),
    row!(104, Label, OrdinaryOnly, Recipe::IfFalse),
    row!(105, Label, OrdinaryOnly, Recipe::IfTrue),
    row!(106, Label, OrdinaryOnly, Recipe::Goto),
    row!(107, Label, Blocked, ControlFlow),
    row!(108, Label, Blocked, ControlFlow),
    row!(109, None, Blocked, ControlFlow),
    row!(110, None, Blocked, ControlFlow),
    row!(111, None, Blocked, ValueConstruction),
    row!(112, None, Blocked, ValueConstruction),
    row!(113, AtomLabelU8, Blocked, DynamicScope),
    row!(114, AtomLabelU8, Blocked, DynamicScope),
    row!(115, AtomLabelU8, Blocked, DynamicScope),
    row!(116, AtomLabelU8, Blocked, DynamicScope),
    row!(117, AtomLabelU8, Blocked, DynamicScope),
    row!(118, AtomU16, Blocked, DynamicScope),
    row!(119, AtomU16, Blocked, DynamicScope),
    row!(120, AtomU16, Blocked, DynamicScope),
    row!(121, Atom, Blocked, DynamicScope),
    row!(122, None, Blocked, Iteration),
    row!(123, None, Blocked, Iteration),
    row!(124, None, Blocked, Iteration),
    row!(125, None, Blocked, Iteration),
    row!(126, U8, Blocked, Iteration),
    row!(127, None, Blocked, Iteration),
    row!(128, None, Blocked, Iteration),
    row!(129, None, Blocked, Iteration),
    row!(130, None, Blocked, Iteration),
    row!(131, None, Blocked, Iteration),
    row!(132, U8, Blocked, Iteration),
    row!(133, None, Blocked, Suspension),
    row!(134, None, Blocked, Suspension),
    row!(135, None, Blocked, Suspension),
    row!(136, None, Blocked, Suspension),
    row!(137, None, Blocked, Suspension),
    row!(138, None, Shared, Recipe::Unary(FunctionUnaryOp::Neg)),
    row!(139, None, Shared, Recipe::Unary(FunctionUnaryOp::Plus)),
    row!(140, None, Shared, Recipe::Unary(FunctionUnaryOp::Dec)),
    row!(141, None, Shared, Recipe::Unary(FunctionUnaryOp::Inc)),
    row!(142, None, OrdinaryOnly, Recipe::PostDec),
    row!(143, None, OrdinaryOnly, Recipe::PostInc),
    row!(144, Loc8, Blocked, Specialized),
    row!(145, Loc8, Blocked, Specialized),
    row!(146, Loc8, Blocked, Specialized),
    row!(147, None, Shared, Recipe::Unary(FunctionUnaryOp::BitNot)),
    row!(148, None, Shared, Recipe::Unary(FunctionUnaryOp::LogicalNot)),
    row!(149, None, Shared, Recipe::Unary(FunctionUnaryOp::TypeOf)),
    row!(150, None, Blocked, Operator),
    row!(151, Atom, Blocked, Binding),
    row!(152, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Mul)),
    row!(153, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Div)),
    row!(154, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Mod)),
    row!(155, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Add)),
    row!(156, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Sub)),
    row!(157, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Pow)),
    row!(158, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Shl)),
    row!(159, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Sar)),
    row!(160, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Shr)),
    row!(161, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::LessThan)),
    row!(162, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::LessThanOrEqual)),
    row!(163, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::GreaterThan)),
    row!(164, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::GreaterThanOrEqual)),
    row!(165, None, Blocked, Operator),
    row!(166, None, Blocked, Operator),
    row!(167, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Equal)),
    row!(168, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::NotEqual)),
    row!(169, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::StrictEqual)),
    row!(170, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::StrictNotEqual)),
    row!(171, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::BitAnd)),
    row!(172, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::BitXor)),
    row!(173, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::BitOr)),
    row!(174, None, OrdinaryOnly, Recipe::Predicate(FunctionPredicateOp::IsUndefinedOrNull)),
    row!(175, None, Blocked, Operator),
    row!(176, I32, Shared, Recipe::PushBigIntI32),
    row!(177, None, Blocked, Specialized),
    row!(178, NoneInt, Shared, Recipe::PushI32),
    row!(179, NoneInt, Shared, Recipe::PushI32),
    row!(180, NoneInt, Shared, Recipe::PushI32),
    row!(181, NoneInt, Shared, Recipe::PushI32),
    row!(182, NoneInt, Shared, Recipe::PushI32),
    row!(183, NoneInt, Shared, Recipe::PushI32),
    row!(184, NoneInt, Shared, Recipe::PushI32),
    row!(185, NoneInt, Shared, Recipe::PushI32),
    row!(186, NoneInt, Shared, Recipe::PushI32),
    row!(187, I8, Shared, Recipe::PushI32),
    row!(188, I16, Shared, Recipe::PushI32),
    row!(189, Const8, Shared, Recipe::PushConstant),
    row!(190, Const8, Blocked, FunctionGraph),
    row!(191, None, Shared, Recipe::PushEmptyString),
    row!(192, Loc8, OrdinaryOnly, Recipe::GetLocal),
    row!(193, Loc8, OrdinaryOnly, Recipe::PutLocal),
    row!(194, Loc8, OrdinaryOnly, Recipe::SetLocal),
    row!(195, NoneLoc, OrdinaryOnly, Recipe::GetLocal),
    row!(196, NoneLoc, OrdinaryOnly, Recipe::GetLocal),
    row!(197, NoneLoc, OrdinaryOnly, Recipe::GetLocal),
    row!(198, NoneLoc, OrdinaryOnly, Recipe::GetLocal),
    row!(199, NoneLoc, OrdinaryOnly, Recipe::PutLocal),
    row!(200, NoneLoc, OrdinaryOnly, Recipe::PutLocal),
    row!(201, NoneLoc, OrdinaryOnly, Recipe::PutLocal),
    row!(202, NoneLoc, OrdinaryOnly, Recipe::PutLocal),
    row!(203, NoneLoc, Shared, Recipe::SetLocal),
    row!(204, NoneLoc, OrdinaryOnly, Recipe::SetLocal),
    row!(205, NoneLoc, OrdinaryOnly, Recipe::SetLocal),
    row!(206, NoneLoc, OrdinaryOnly, Recipe::SetLocal),
    row!(207, NoneArg, OrdinaryOnly, Recipe::GetArgument),
    row!(208, NoneArg, OrdinaryOnly, Recipe::GetArgument),
    row!(209, NoneArg, OrdinaryOnly, Recipe::GetArgument),
    row!(210, NoneArg, OrdinaryOnly, Recipe::GetArgument),
    row!(211, NoneArg, OrdinaryOnly, Recipe::PutArgument),
    row!(212, NoneArg, OrdinaryOnly, Recipe::PutArgument),
    row!(213, NoneArg, OrdinaryOnly, Recipe::PutArgument),
    row!(214, NoneArg, OrdinaryOnly, Recipe::PutArgument),
    row!(215, NoneArg, OrdinaryOnly, Recipe::SetArgument),
    row!(216, NoneArg, OrdinaryOnly, Recipe::SetArgument),
    row!(217, NoneArg, OrdinaryOnly, Recipe::SetArgument),
    row!(218, NoneArg, OrdinaryOnly, Recipe::SetArgument),
    row!(219, NoneVarRef, Blocked, LexicalEnvironment),
    row!(220, NoneVarRef, Blocked, LexicalEnvironment),
    row!(221, NoneVarRef, Blocked, LexicalEnvironment),
    row!(222, NoneVarRef, Blocked, LexicalEnvironment),
    row!(223, NoneVarRef, Blocked, LexicalEnvironment),
    row!(224, NoneVarRef, Blocked, LexicalEnvironment),
    row!(225, NoneVarRef, Blocked, LexicalEnvironment),
    row!(226, NoneVarRef, Blocked, LexicalEnvironment),
    row!(227, NoneVarRef, Blocked, LexicalEnvironment),
    row!(228, NoneVarRef, Blocked, LexicalEnvironment),
    row!(229, NoneVarRef, Blocked, LexicalEnvironment),
    row!(230, NoneVarRef, Blocked, LexicalEnvironment),
    row!(231, None, Blocked, Property),
    row!(232, Label8, OrdinaryOnly, Recipe::IfFalse),
    row!(233, Label8, OrdinaryOnly, Recipe::IfTrue),
    row!(234, Label8, OrdinaryOnly, Recipe::Goto),
    row!(235, Label16, OrdinaryOnly, Recipe::Goto),
    row!(236, NPopX, OrdinaryOnly, Recipe::Call),
    row!(237, NPopX, OrdinaryOnly, Recipe::Call),
    row!(238, NPopX, OrdinaryOnly, Recipe::Call),
    row!(239, NPopX, OrdinaryOnly, Recipe::Call),
    row!(240, None, OrdinaryOnly, Recipe::Predicate(FunctionPredicateOp::IsUndefined)),
    row!(241, None, OrdinaryOnly, Recipe::Predicate(FunctionPredicateOp::IsNull)),
    row!(242, None, OrdinaryOnly, Recipe::Predicate(FunctionPredicateOp::TypeOfIsUndefined)),
    row!(243, None, OrdinaryOnly, Recipe::Predicate(FunctionPredicateOp::TypeOfIsFunction)),
];

#[must_use]
pub(super) const fn row_for(opcode: PinnedOpcode) -> CapabilityRow {
    CAPABILITY_REGISTRY[opcode.raw() as usize]
}

#[must_use]
pub(super) const fn operand_shape(format: OpcodeFormat) -> OperandShape {
    match format {
        OpcodeFormat::None => OperandShape::None,
        OpcodeFormat::NoneInt => OperandShape::NoneInt,
        OpcodeFormat::NoneLoc => OperandShape::NoneLoc,
        OpcodeFormat::NoneArg => OperandShape::NoneArg,
        OpcodeFormat::NoneVarRef => OperandShape::NoneVarRef,
        OpcodeFormat::U8 => OperandShape::U8,
        OpcodeFormat::I8 => OperandShape::I8,
        OpcodeFormat::Loc8 => OperandShape::Loc8,
        OpcodeFormat::Const8 => OperandShape::Const8,
        OpcodeFormat::Label8 => OperandShape::Label8,
        OpcodeFormat::U16 => OperandShape::U16,
        OpcodeFormat::I16 => OperandShape::I16,
        OpcodeFormat::Label16 => OperandShape::Label16,
        OpcodeFormat::NPop => OperandShape::NPop,
        OpcodeFormat::NPopX => OperandShape::NPopX,
        OpcodeFormat::NPopU16 => OperandShape::NPopU16,
        OpcodeFormat::Loc => OperandShape::Loc,
        OpcodeFormat::Arg => OperandShape::Arg,
        OpcodeFormat::VarRef => OperandShape::VarRef,
        OpcodeFormat::U32 => OperandShape::U32,
        OpcodeFormat::I32 => OperandShape::I32,
        OpcodeFormat::Const => OperandShape::Const,
        OpcodeFormat::Label => OperandShape::Label,
        OpcodeFormat::Atom => OperandShape::Atom,
        OpcodeFormat::AtomU8 => OperandShape::AtomU8,
        OpcodeFormat::AtomU16 => OperandShape::AtomU16,
        OpcodeFormat::AtomLabelU8 => OperandShape::AtomLabelU8,
        OpcodeFormat::AtomLabelU16 => OperandShape::AtomLabelU16,
        OpcodeFormat::LabelU16 => OperandShape::LabelU16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::binary_object::function_translate::InstructionAudience;

    #[test]
    fn registry_is_an_exhaustive_raw_indexed_pinned_format_contract() {
        assert_eq!(CAPABILITY_REGISTRY.len(), 244);
        for (raw, row) in CAPABILITY_REGISTRY.iter().copied().enumerate() {
            let opcode = PinnedOpcode::from_byte(raw as u8).expect("registry raw is pinned");
            assert_eq!(usize::from(row.raw), raw);
            assert_eq!(row.expected_format, opcode.format(), "raw opcode {raw}");
        }
    }

    #[test]
    fn registry_locks_the_current_physical_cohorts() {
        let mut blocked = 0;
        let mut scalar_only = 0;
        let mut ordinary_only = 0;
        let mut shared = 0;
        for row in CAPABILITY_REGISTRY {
            match row.policy {
                CapabilityPolicy::Blocked(_) => blocked += 1,
                CapabilityPolicy::ScalarOnly(_) => scalar_only += 1,
                CapabilityPolicy::OrdinaryOnly(_) => ordinary_only += 1,
                CapabilityPolicy::Shared(_) => shared += 1,
            }
        }
        assert_eq!(
            (blocked, scalar_only, ordinary_only, shared),
            (116, 1, 98, 29)
        );
        assert_eq!(scalar_only + shared, 30);
        assert_eq!(ordinary_only + shared, 127);
        assert_eq!(scalar_only + ordinary_only + shared, 128);
    }

    #[test]
    fn ordinary_leaf_addition_is_the_exact_reviewed_57_row_set() {
        const NEW_ORDINARY_PHYSICAL_ROWS: [u8; 57] = [
            6, 7, 9, 10, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
            32, 41, 105, 138, 139, 140, 141, 142, 143, 147, 148, 149, 152, 154, 157, 158, 159, 160,
            161, 162, 164, 167, 168, 170, 171, 172, 173, 174, 176, 191, 233, 240, 241, 242, 243,
        ];
        const FORMER_SCALAR_ROWS: [u8; 13] =
            [6, 7, 9, 10, 138, 139, 140, 141, 147, 148, 149, 176, 191];

        let mut seen = [false; PINNED_OPCODE_COUNT];
        for raw in NEW_ORDINARY_PHYSICAL_ROWS {
            assert!(!seen[usize::from(raw)], "duplicate reviewed raw {raw}");
            seen[usize::from(raw)] = true;
            let expected = if FORMER_SCALAR_ROWS.contains(&raw) {
                InstructionAudience::Shared
            } else {
                InstructionAudience::OrdinaryOnly
            };
            let actual = match CAPABILITY_REGISTRY[usize::from(raw)].policy {
                CapabilityPolicy::Blocked(_) => InstructionAudience::Blocked,
                CapabilityPolicy::ScalarOnly(_) => InstructionAudience::ScalarOnly,
                CapabilityPolicy::OrdinaryOnly(_) => InstructionAudience::OrdinaryOnly,
                CapabilityPolicy::Shared(_) => InstructionAudience::Shared,
            };
            assert_eq!(actual, expected, "reviewed raw {raw}");
        }

        assert!(matches!(
            CAPABILITY_REGISTRY[163].policy,
            CapabilityPolicy::OrdinaryOnly(Recipe::Binary(FunctionBinaryOp::GreaterThan))
        ));
        assert!(matches!(
            CAPABILITY_REGISTRY[169].policy,
            CapabilityPolicy::OrdinaryOnly(Recipe::Binary(FunctionBinaryOp::StrictEqual))
        ));
        assert!(matches!(
            CAPABILITY_REGISTRY[165].policy,
            CapabilityPolicy::Blocked(TranslationBlocker::Operator)
        ));
        assert!(matches!(
            CAPABILITY_REGISTRY[166].policy,
            CapabilityPolicy::Blocked(TranslationBlocker::Operator)
        ));
    }

    #[test]
    fn ordinary_plain_calls_are_the_exact_reviewed_five_row_set() {
        const PLAIN_CALL_ROWS: [(u8, OpcodeFormat); 5] = [
            (34, OpcodeFormat::NPop),
            (236, OpcodeFormat::NPopX),
            (237, OpcodeFormat::NPopX),
            (238, OpcodeFormat::NPopX),
            (239, OpcodeFormat::NPopX),
        ];

        let actual = CAPABILITY_REGISTRY
            .iter()
            .filter_map(|row| {
                matches!(row.policy, CapabilityPolicy::OrdinaryOnly(Recipe::Call))
                    .then_some(row.raw)
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, PLAIN_CALL_ROWS.map(|(raw, _)| raw));

        for (raw, expected_format) in PLAIN_CALL_ROWS {
            let row = CAPABILITY_REGISTRY[usize::from(raw)];
            assert_eq!(row.raw, raw);
            assert_eq!(row.expected_format, expected_format);
            assert!(matches!(
                row.policy,
                CapabilityPolicy::OrdinaryOnly(Recipe::Call)
            ));
        }
    }

    #[test]
    fn ordinary_invocation_addition_is_the_exact_reviewed_six_row_set() {
        const INVOCATION_ROWS: [(u8, OpcodeFormat, Recipe); 6] = [
            (33, OpcodeFormat::NPop, Recipe::Construct),
            (35, OpcodeFormat::NPop, Recipe::TailCall),
            (36, OpcodeFormat::NPop, Recipe::CallMethod),
            (37, OpcodeFormat::NPop, Recipe::TailCallMethod),
            (38, OpcodeFormat::NPop, Recipe::ArrayFrom),
            (39, OpcodeFormat::U16, Recipe::Apply),
        ];

        for (raw, expected_format, expected_recipe) in INVOCATION_ROWS {
            let row = CAPABILITY_REGISTRY[usize::from(raw)];
            assert_eq!(row.raw, raw);
            assert_eq!(row.expected_format, expected_format);
            assert_eq!(row.policy, CapabilityPolicy::OrdinaryOnly(expected_recipe));
        }
    }

    #[test]
    fn ordinary_explicit_throw_is_the_only_reviewed_exception_completion() {
        assert_eq!(
            CAPABILITY_REGISTRY[48].policy,
            CapabilityPolicy::OrdinaryOnly(Recipe::Throw)
        );
        assert!(matches!(
            CAPABILITY_REGISTRY[47].policy,
            CapabilityPolicy::Blocked(TranslationBlocker::Completion)
        ));
        assert!(matches!(
            CAPABILITY_REGISTRY[49].policy,
            CapabilityPolicy::Blocked(TranslationBlocker::Exception)
        ));
        assert!(matches!(
            CAPABILITY_REGISTRY[177].policy,
            CapabilityPolicy::Blocked(TranslationBlocker::Specialized)
        ));
    }

    #[test]
    fn blocked_frontier_has_stable_typed_category_counts() {
        let mut counts = [0_usize; 16];
        for row in CAPABILITY_REGISTRY {
            let CapabilityPolicy::Blocked(blocker) = row.policy else {
                continue;
            };
            let index = match blocker {
                TranslationBlocker::InvalidSentinel => 0,
                TranslationBlocker::ValueConstruction => 1,
                TranslationBlocker::FunctionGraph => 2,
                TranslationBlocker::Completion => 3,
                TranslationBlocker::Exception => 4,
                TranslationBlocker::EvalOrModule => 5,
                TranslationBlocker::Binding => 6,
                TranslationBlocker::Property => 7,
                TranslationBlocker::ObjectConstruction => 8,
                TranslationBlocker::LexicalEnvironment => 9,
                TranslationBlocker::ControlFlow => 10,
                TranslationBlocker::DynamicScope => 11,
                TranslationBlocker::Iteration => 12,
                TranslationBlocker::Suspension => 13,
                TranslationBlocker::Operator => 14,
                TranslationBlocker::Specialized => 15,
            };
            counts[index] += 1;
        }
        assert_eq!(counts, [1, 8, 2, 1, 1, 3, 7, 16, 15, 25, 4, 9, 11, 5, 4, 4]);
        assert!(counts.into_iter().all(|count| count != 0));
        assert_eq!(counts.into_iter().sum::<usize>(), 116);
    }
}
