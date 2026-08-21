//! Sanitized function-code projection shared by narrow admission policies.
//!
//! These types deliberately retain semantic operands and instruction-index
//! branch targets only. Native byte PCs, image identities, raw opcode bytes,
//! and runtime-owned handles stay behind the translation boundary.

use std::fmt;

/// The existing admission policies for which one physical opcode is enabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InstructionAudience {
    Blocked,
    ScalarOnly,
    OrdinaryOnly,
    Shared,
}

impl InstructionAudience {
    #[must_use]
    pub(super) const fn includes_scalar(self) -> bool {
        matches!(self, Self::ScalarOnly | Self::Shared)
    }

    #[must_use]
    pub(super) const fn includes_ordinary(self) -> bool {
        matches!(self, Self::OrdinaryOnly | Self::Shared)
    }
}

/// Sanitized spelling of a pinned operand layout for compatibility diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum OperandShape {
    None,
    NoneInt,
    NoneLoc,
    NoneArg,
    NoneVarRef,
    U8,
    I8,
    Loc8,
    Const8,
    Label8,
    U16,
    I16,
    Label16,
    NPop,
    NPopX,
    NPopU16,
    Loc,
    Arg,
    VarRef,
    U32,
    I32,
    Const,
    Label,
    Atom,
    AtomU8,
    AtomU16,
    AtomLabelU8,
    AtomLabelU16,
    LabelU16,
}

/// Non-numeric semantic class of an atom operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum AtomOperandClass {
    Null,
    Index,
    String,
    Private,
    Symbol,
}

/// Stable reason why a pinned physical opcode is outside translated semantics.
/// This class is bookkeeping only and never participates in admission dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum TranslationBlocker {
    InvalidSentinel,
    ValueConstruction,
    FunctionGraph,
    Completion,
    EvalOrModule,
    Binding,
    Property,
    ObjectConstruction,
    LexicalEnvironment,
    ControlFlow,
    DynamicScope,
    Iteration,
    Suspension,
    Operator,
    Specialized,
}

#[derive(Clone, Copy)]
enum AtomStringSpelling<'image> {
    Manifest(&'static str),
    ByteUnits(&'image [u8]),
    Utf16Units(&'image [u16]),
}

impl<'image> AtomStringSpelling<'image> {
    fn utf16_len(self) -> usize {
        match self {
            Self::Manifest(value) => value.encode_utf16().count(),
            Self::ByteUnits(value) => value.len(),
            Self::Utf16Units(value) => value.len(),
        }
    }

    fn units(self) -> AtomStringUnits<'image> {
        let remaining = self.utf16_len();
        let source = match self {
            Self::Manifest(value) => AtomStringUnitSource::Manifest(value.encode_utf16()),
            Self::ByteUnits(value) => AtomStringUnitSource::ByteUnits(value.iter()),
            Self::Utf16Units(value) => AtomStringUnitSource::Utf16Units(value.iter()),
        };
        AtomStringUnits { source, remaining }
    }
}

enum AtomStringUnitSource<'spelling> {
    Manifest(std::str::EncodeUtf16<'spelling>),
    ByteUnits(std::slice::Iter<'spelling, u8>),
    Utf16Units(std::slice::Iter<'spelling, u16>),
}

/// Iterator over a sealed atom spelling normalized to semantic UTF-16 units.
/// Its private source does not reveal the archive's original storage width.
pub(in crate::runtime::binary_object) struct AtomStringUnits<'spelling> {
    source: AtomStringUnitSource<'spelling>,
    remaining: usize,
}

impl Iterator for AtomStringUnits<'_> {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        let next = match &mut self.source {
            AtomStringUnitSource::Manifest(units) => units.next(),
            AtomStringUnitSource::ByteUnits(units) => units.next().copied().map(u16::from),
            AtomStringUnitSource::Utf16Units(units) => units.next().copied(),
        };
        if next.is_some() {
            self.remaining -= 1;
        }
        next
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for AtomStringUnits<'_> {}
impl std::iter::FusedIterator for AtomStringUnits<'_> {}

#[derive(Clone, Copy)]
enum AtomOperandValue<'image> {
    Null,
    Index(u32),
    String(AtomStringSpelling<'image>),
    Private,
    Symbol,
}

/// One semantic atom operand with only the provenance needed by admission.
///
/// String spellings are sealed borrowed views which expose only normalized
/// UTF-16 iteration. This DTO carries neither a wire model nor a pinned atom
/// ID, dynamic-table index, storage width, or runtime handle.
#[derive(Clone, Copy)]
pub(in crate::runtime::binary_object) struct AtomOperand<'image> {
    value: AtomOperandValue<'image>,
    from_input_atom_table: bool,
}

impl fmt::Debug for AtomOperand<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AtomOperand")
            .field("class", &self.class())
            .field("from_input_atom_table", &self.from_input_atom_table)
            .finish_non_exhaustive()
    }
}

impl<'image> AtomOperand<'image> {
    pub(super) const fn null(from_input_atom_table: bool) -> Self {
        Self {
            value: AtomOperandValue::Null,
            from_input_atom_table,
        }
    }

    pub(super) const fn index(index: u32, from_input_atom_table: bool) -> Self {
        Self {
            value: AtomOperandValue::Index(index),
            from_input_atom_table,
        }
    }

    pub(super) const fn manifest_string(
        spelling: &'static str,
        from_input_atom_table: bool,
    ) -> Self {
        Self {
            value: AtomOperandValue::String(AtomStringSpelling::Manifest(spelling)),
            from_input_atom_table,
        }
    }

    pub(super) const fn byte_string(spelling: &'image [u8], from_input_atom_table: bool) -> Self {
        Self {
            value: AtomOperandValue::String(AtomStringSpelling::ByteUnits(spelling)),
            from_input_atom_table,
        }
    }

    pub(super) const fn utf16_string(spelling: &'image [u16], from_input_atom_table: bool) -> Self {
        Self {
            value: AtomOperandValue::String(AtomStringSpelling::Utf16Units(spelling)),
            from_input_atom_table,
        }
    }

    pub(super) const fn private(from_input_atom_table: bool) -> Self {
        Self {
            value: AtomOperandValue::Private,
            from_input_atom_table,
        }
    }

    pub(super) const fn symbol(from_input_atom_table: bool) -> Self {
        Self {
            value: AtomOperandValue::Symbol,
            from_input_atom_table,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn originates_from_input_atom_table(&self) -> bool {
        self.from_input_atom_table
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn class(&self) -> AtomOperandClass {
        match &self.value {
            AtomOperandValue::Null => AtomOperandClass::Null,
            AtomOperandValue::Index(_) => AtomOperandClass::Index,
            AtomOperandValue::String(_) => AtomOperandClass::String,
            AtomOperandValue::Private => AtomOperandClass::Private,
            AtomOperandValue::Symbol => AtomOperandClass::Symbol,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn index_value(&self) -> Option<u32> {
        match &self.value {
            AtomOperandValue::Index(index) => Some(*index),
            _ => None,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn string_utf16_len(&self) -> Option<usize> {
        match &self.value {
            AtomOperandValue::String(spelling) => Some(spelling.utf16_len()),
            _ => None,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn string_utf16_units(
        &self,
    ) -> Option<AtomStringUnits<'_>> {
        match &self.value {
            AtomOperandValue::String(spelling) => Some(spelling.units()),
            _ => None,
        }
    }
}

/// Unary semantics in the currently translated union of public cohorts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum FunctionUnaryOp {
    Neg,
    Plus,
    Dec,
    Inc,
    BitNot,
    LogicalNot,
    TypeOf,
}

/// One typed stack permutation in the sanitized instruction stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum FunctionStackOp {
    Drop,
    Nip,
    Dup,
    Dup1,
    Dup3,
    Insert2,
    Insert3,
    Insert4,
    Perm3,
    Perm4,
    Perm5,
    Swap,
    Rot4Left,
}

/// One typed binary operator in the sanitized instruction stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum FunctionBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Shl,
    Sar,
    Shr,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    StrictEqual,
    StrictNotEqual,
    BitAnd,
    BitXor,
    BitOr,
}

/// One typed tag or `typeof` predicate in the sanitized instruction stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum FunctionPredicateOp {
    IsUndefinedOrNull,
    IsUndefined,
    IsNull,
    TypeOfIsUndefined,
    TypeOfIsFunction,
}

/// Canonical semantic subset of QuickJS's raw `OP_apply` magic operand.
///
/// The pinned compiler emits only zero for a call and one for construction.
/// Keeping that distinction typed prevents malformed raw values from crossing
/// the archive translation boundary or being normalized by parity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum FunctionApplyKind {
    Call,
    Construct,
}

/// One sanitized semantic instruction.
#[derive(Clone, Debug)]
pub(in crate::runtime::binary_object) enum FunctionOp<'image> {
    Blocked(TranslationBlocker),
    OutsideTarget,
    Nop,
    Object,
    ToObject,
    PushThis,
    PushI32(i32),
    PushConstant(u32),
    PushAtom(AtomOperand<'image>),
    PushUndefined,
    PushNull,
    PushBool(bool),
    PushBigIntI32(i32),
    PushEmptyString,
    Stack(FunctionStackOp),
    Unary(FunctionUnaryOp),
    PostDec,
    PostInc,
    GetLocal(u16),
    PutLocal(u16),
    SetLocal(u16),
    GetArgument(u16),
    PutArgument(u16),
    SetArgument(u16),
    Binary(FunctionBinaryOp),
    Predicate(FunctionPredicateOp),
    IfFalse(u32),
    IfTrue(u32),
    Goto(u32),
    Call(u16),
    TailCall(u16),
    Construct(u16),
    CallMethod(u16),
    TailCallMethod(u16),
    ArrayFrom(u16),
    Apply(FunctionApplyKind),
    Return,
    ReturnUndefined,
    Throw,
    ThrowReadOnly(AtomOperand<'image>),
}

/// Compatibility-only rejection descriptor without an opcode byte or source location.
/// Capability and semantic lowering must never branch on either field.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(in crate::runtime::binary_object) struct OperationDiagnostic {
    mnemonic: &'static str,
    operand_shape: OperandShape,
}

impl OperationDiagnostic {
    pub(super) const fn new(mnemonic: &'static str, operand_shape: OperandShape) -> Self {
        Self {
            mnemonic,
            operand_shape,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn mnemonic(self) -> &'static str {
        self.mnemonic
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn operand_shape(self) -> OperandShape {
        self.operand_shape
    }
}

/// One translated instruction plus its unchanged admission audience.
#[derive(Clone)]
pub(in crate::runtime::binary_object) struct FunctionInstruction<'image> {
    audience: InstructionAudience,
    diagnostic: OperationDiagnostic,
    operation: FunctionOp<'image>,
}

impl<'image> FunctionInstruction<'image> {
    pub(super) const fn new(
        audience: InstructionAudience,
        diagnostic: OperationDiagnostic,
        operation: FunctionOp<'image>,
    ) -> Self {
        Self {
            audience,
            diagnostic,
            operation,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn supports_scalar(&self) -> bool {
        self.audience.includes_scalar()
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn supports_ordinary(&self) -> bool {
        self.audience.includes_ordinary()
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn rejection_diagnostic(
        &self,
    ) -> OperationDiagnostic {
        self.diagnostic
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn operation(&self) -> &FunctionOp<'image> {
        &self.operation
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn into_operation(self) -> FunctionOp<'image> {
        self.operation
    }
}

/// A translated function body whose branches address this instruction array.
#[derive(Clone)]
pub(in crate::runtime::binary_object) struct FunctionCode<'image> {
    instructions: Box<[FunctionInstruction<'image>]>,
}

impl<'image> FunctionCode<'image> {
    pub(super) const fn new(instructions: Box<[FunctionInstruction<'image>]>) -> Self {
        Self { instructions }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn instructions(
        &self,
    ) -> &[FunctionInstruction<'image>] {
        &self.instructions
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn into_instructions(
        self,
    ) -> Box<[FunctionInstruction<'image>]> {
        self.instructions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! assert_not_impl {
        ($type:ty, $trait:path) => {
            const _: fn() = || {
                trait AmbiguousIfImpl<Marker> {
                    fn marker() {}
                }

                impl<T: ?Sized> AmbiguousIfImpl<()> for T {}

                struct ImplementsTrait;
                impl<T: ?Sized + $trait> AmbiguousIfImpl<ImplementsTrait> for T {}

                let _ = <$type as AmbiguousIfImpl<_>>::marker;
            };
        };
    }

    // Equality or hashing on any translated container would make the private
    // spelling representation observable through derived field behavior.
    assert_not_impl!(AtomStringSpelling<'static>, PartialEq);
    assert_not_impl!(AtomStringSpelling<'static>, std::hash::Hash);
    assert_not_impl!(AtomOperandValue<'static>, PartialEq);
    assert_not_impl!(AtomOperandValue<'static>, std::hash::Hash);
    assert_not_impl!(AtomOperand<'static>, PartialEq);
    assert_not_impl!(AtomOperand<'static>, std::hash::Hash);
    assert_not_impl!(FunctionOp<'static>, PartialEq);
    assert_not_impl!(FunctionOp<'static>, std::hash::Hash);
    assert_not_impl!(FunctionInstruction<'static>, PartialEq);
    assert_not_impl!(FunctionInstruction<'static>, std::hash::Hash);
    assert_not_impl!(FunctionCode<'static>, PartialEq);
    assert_not_impl!(FunctionCode<'static>, std::hash::Hash);
    // An empty default would bypass the translator as the sole constructor.
    assert_not_impl!(FunctionCode<'static>, Default);

    // Diagnostics are available only through explicit rejection accessors;
    // container formatting must not become an alternate mnemonic channel.
    assert_not_impl!(OperationDiagnostic, fmt::Debug);
    assert_not_impl!(OperationDiagnostic, std::hash::Hash);
    assert_not_impl!(FunctionInstruction<'static>, fmt::Debug);
    assert_not_impl!(FunctionCode<'static>, fmt::Debug);

    #[test]
    fn atom_spelling_is_a_copyable_borrowed_utf16_view() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<AtomOperand<'static>>();

        let byte_units = [b'A', 0xe9];
        let byte_spelling = AtomOperand::byte_string(&byte_units, true);
        assert_eq!(byte_spelling.class(), AtomOperandClass::String);
        assert_eq!(byte_spelling.string_utf16_len(), Some(2));
        assert_eq!(
            byte_spelling
                .string_utf16_units()
                .expect("String spelling is present")
                .collect::<Vec<_>>(),
            [u16::from(b'A'), 0x00e9]
        );

        let utf16_units = [0xd800, 0, 0x0100];
        let utf16_spelling = AtomOperand::utf16_string(&utf16_units, false);
        assert_eq!(utf16_spelling.string_utf16_len(), Some(3));
        assert_eq!(
            utf16_spelling
                .string_utf16_units()
                .expect("String spelling is present")
                .collect::<Vec<_>>(),
            utf16_units
        );

        let manifest = AtomOperand::manifest_string("A😀", false);
        assert_eq!(manifest.string_utf16_len(), Some(3));
        assert_eq!(
            manifest
                .string_utf16_units()
                .expect("String spelling is present")
                .collect::<Vec<_>>(),
            "A😀".encode_utf16().collect::<Vec<_>>()
        );

        let debug = format!("{byte_spelling:?}");
        assert!(!debug.contains("ByteUnits"));
        assert!(!debug.contains("Utf16Units"));
        assert!(!debug.contains("Aé"));
    }
}
