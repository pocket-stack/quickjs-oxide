//! Archive-side admission and lowering for an ordinary synchronous leaf.
//!
//! The compatible reader first authenticates the complete BC5 image. This
//! layer then selects a child through the root function's constant pool and
//! emits an owned, heap-independent draft. It never publishes executable
//! bytecode and never exposes native opcodes, byte PCs, image identities, wire
//! strings, or runtime objects.

use std::fmt;

use super::bytecode_image::{
    BytecodeImage, BytecodeImageError, BytecodeImageLimits, ImageAtomError, ModuleLimits,
    decode_bytecode_image_body,
};
use super::code::{CodeError, CodeLimits};
use super::function_envelope::{FunctionEnvelopeError, FunctionEnvelopeLimits, FunctionKind};
use super::function_translate::{
    AtomOperand, AtomOperandClass, FunctionApplyKind, FunctionBinaryOp, FunctionCode, FunctionOp,
    FunctionPredicateOp, FunctionStackOp, FunctionTranslateError, FunctionUnaryOp,
    OperationDiagnostic, TranslationTarget, translate_function,
};
use super::graph::decode::DecodeError;
use super::graph::model::{
    ArrayBufferLayoutError, GraphError, GraphLimits, TypedArrayLayoutError, WireValue,
};
use super::wire::{ReaderMode, WireCursor, WireError, WireLimits, WireString};

const MAX_INPUT_BYTES: usize = 4096;
const MAX_DECLARED_STACK: u16 = 65_534;
const KNOWN_FUNCTION_FLAG_BITS: u16 = (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (1 << 3)
    | (0b11 << 4)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 9)
    | (1 << 11);
const KNOWN_JS_MODE_BITS: u8 = (1 << 0) | (1 << 2) | (1 << 3);

/// Select one constant in the authenticated root function's constant pool.
///
/// The field stays private so a caller cannot couple to an image-local
/// function identity. Selection is repeated against the just-decoded image on
/// every call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct RootFunctionConstantSelector(u32);

impl RootFunctionConstantSelector {
    #[must_use]
    pub(in crate::runtime) const fn from_zero_based(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub(in crate::runtime) const fn zero_based(self) -> u32 {
        self.0
    }
}

/// Owned metadata for one admitted ordinary synchronous leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct OrdinaryLeafMetadataDraft {
    argument_count: u16,
    defined_argument_count: u16,
    local_count: u16,
    max_stack: u16,
    is_strict: bool,
    has_simple_parameter_list: bool,
    has_prototype: bool,
    allows_new_target: bool,
    allows_arguments: bool,
    strip_variable_debug: bool,
}

impl OrdinaryLeafMetadataDraft {
    #[must_use]
    pub(in crate::runtime) const fn argument_count(self) -> u16 {
        self.argument_count
    }

    #[must_use]
    pub(in crate::runtime) const fn defined_argument_count(self) -> u16 {
        self.defined_argument_count
    }

    #[must_use]
    pub(in crate::runtime) const fn local_count(self) -> u16 {
        self.local_count
    }

    #[must_use]
    pub(in crate::runtime) const fn max_stack(self) -> u16 {
        self.max_stack
    }

    #[must_use]
    pub(in crate::runtime) const fn is_strict(self) -> bool {
        self.is_strict
    }

    #[must_use]
    pub(in crate::runtime) const fn has_simple_parameter_list(self) -> bool {
        self.has_simple_parameter_list
    }

    #[must_use]
    pub(in crate::runtime) const fn has_prototype(self) -> bool {
        self.has_prototype
    }

    #[must_use]
    pub(in crate::runtime) const fn allows_new_target(self) -> bool {
        self.allows_new_target
    }

    #[must_use]
    pub(in crate::runtime) const fn allows_arguments(self) -> bool {
        self.allows_arguments
    }

    #[must_use]
    pub(in crate::runtime) const fn strip_variable_debug(self) -> bool {
        self.strip_variable_debug
    }
}

/// Runtime-independent function-constant payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum DetachedPrimitive {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Float64Bits(u64),
    String(Box<[u16]>),
    /// Canonical signed little-endian bytes produced by the whole-image read.
    BigIntSignedLeCanonical(Box<[u8]>),
}

/// Owned UTF-16 spelling for an admitted atom-named terminal diagnostic.
///
/// The archive atom ID, input-table slot, and native string width are erased
/// before this value crosses the publication boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct DetachedAtomName(Box<[u16]>);

impl DetachedAtomName {
    pub(in crate::runtime) fn into_units(self) -> Box<[u16]> {
        self.0
    }
}

/// One sanitized instruction in an ordinary-leaf draft.
///
/// Branch targets are instruction indices in this owned array, never native
/// byte PCs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum OrdinaryLeafOp {
    Nop,
    Object,
    ToObject,
    PushI32(i32),
    PushConst(u32),
    PushUndefined,
    PushNull,
    PushBool(bool),
    PushBigIntI32(i32),
    PushEmptyString,
    Stack(OrdinaryLeafStackOp),
    Unary(OrdinaryLeafUnaryOp),
    PostDec,
    PostInc,
    GetLocal(u16),
    PutLocal(u16),
    SetLocal(u16),
    GetArgument(u16),
    PutArgument(u16),
    SetArgument(u16),
    Binary(OrdinaryLeafBinaryOp),
    Predicate(OrdinaryLeafPredicateOp),
    IfFalse(u32),
    IfTrue(u32),
    Goto(u32),
    Call(u16),
    TailCall(u16),
    Construct(u16),
    CallMethod(u16),
    TailCallMethod(u16),
    ArrayFrom(u16),
    Apply(OrdinaryLeafApplyKind),
    Return,
    ReturnUndefined,
    Throw,
    ThrowReadOnly(DetachedAtomName),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum OrdinaryLeafApplyKind {
    Call,
    Construct,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum OrdinaryLeafStackOp {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum OrdinaryLeafUnaryOp {
    Neg,
    Plus,
    Dec,
    Inc,
    BitNot,
    LogicalNot,
    TypeOf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum OrdinaryLeafBinaryOp {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum OrdinaryLeafPredicateOp {
    IsUndefinedOrNull,
    IsUndefined,
    IsNull,
    TypeOfIsUndefined,
    TypeOfIsFunction,
}

/// Owned archive-side handoff for the transactional publication boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct OrdinaryLeafDraft {
    metadata: OrdinaryLeafMetadataDraft,
    constants: Box<[DetachedPrimitive]>,
    code: Box<[OrdinaryLeafOp]>,
}

impl OrdinaryLeafDraft {
    #[must_use]
    pub(in crate::runtime) const fn metadata(&self) -> OrdinaryLeafMetadataDraft {
        self.metadata
    }

    #[must_use]
    pub(in crate::runtime) const fn constants(&self) -> &[DetachedPrimitive] {
        &self.constants
    }

    #[must_use]
    pub(in crate::runtime) const fn code(&self) -> &[OrdinaryLeafOp] {
        &self.code
    }

    pub(in crate::runtime) fn into_parts(
        self,
    ) -> (
        OrdinaryLeafMetadataDraft,
        Box<[DetachedPrimitive]>,
        Box<[OrdinaryLeafOp]>,
    ) {
        (self.metadata, self.constants, self.code)
    }
}

/// Failure classes preserved across the archive/publication boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum OrdinaryLeafReadError {
    Malformed(String),
    Type(String),
    Range(String),
    JsInternal(String),
    Unadmitted(String),
    Resource(String),
    Internal(String),
}

impl fmt::Display for OrdinaryLeafReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(message) => write!(formatter, "malformed BC5 object: {message}"),
            Self::Type(message) => write!(formatter, "invalid BC5 value: {message}"),
            Self::Range(message) => write!(formatter, "out-of-range BC5 value: {message}"),
            Self::JsInternal(message) => {
                write!(formatter, "BC5 reader internal error: {message}")
            }
            Self::Unadmitted(message) => write!(
                formatter,
                "BC5 object is not admitted as an ordinary synchronous leaf: {message}"
            ),
            Self::Resource(message) => {
                write!(formatter, "BC5 ordinary-leaf resource limit: {message}")
            }
            Self::Internal(message) => {
                write!(formatter, "BC5 ordinary-leaf internal failure: {message}")
            }
        }
    }
}

impl std::error::Error for OrdinaryLeafReadError {}

/// Decode one complete pinned-QuickJS image and translate the selected child
/// into a sanitized ordinary-leaf draft.
pub(in crate::runtime) fn decode_trusted_ordinary_leaf(
    input: &[u8],
    selector: RootFunctionConstantSelector,
) -> Result<OrdinaryLeafDraft, OrdinaryLeafReadError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(OrdinaryLeafReadError::Resource(format!(
            "input has {} bytes, limit is {MAX_INPUT_BYTES}",
            input.len()
        )));
    }

    let limits = AdmissionLimits::for_input(input.len());
    let cursor = WireCursor::new(input, ReaderMode::QuickJsCompatible, limits.wire)
        .map_err(classify_wire_error)?;
    let (cursor, image) =
        decode_bytecode_image_body(cursor, limits.image, true).map_err(classify_image_error)?;
    cursor.finish().map_err(classify_wire_error)?;
    admit_image(&image, selector)
}

#[derive(Clone, Copy)]
struct AdmissionLimits {
    wire: WireLimits,
    image: BytecodeImageLimits,
}

impl AdmissionLimits {
    fn for_input(input_bytes: usize) -> Self {
        let bounded = input_bytes.max(1);
        let wire = WireLimits::new(
            MAX_INPUT_BYTES,
            u32::try_from(bounded).unwrap_or(u32::MAX),
            bounded,
            bounded,
        );
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
        let module = ModuleLimits::new(bounded, bounded, bounded, bounded);
        let image = BytecodeImageLimits::new(
            graph, envelope, module, bounded, bounded, bounded, bounded, bounded, bounded, bounded,
            bounded, bounded, bounded, bounded, bounded, bounded, bounded,
        );
        Self { wire, image }
    }
}

fn admit_image(
    image: &BytecodeImage,
    selector: RootFunctionConstantSelector,
) -> Result<OrdinaryLeafDraft, OrdinaryLeafReadError> {
    if !image.reference_table().is_empty() {
        return unadmitted("ordinary-leaf image carries an object-reference table");
    }
    if !image.modules().is_empty() {
        return unadmitted("ordinary-leaf image contains a Module record");
    }

    let root_id = image.root().function_id().ok_or_else(|| {
        OrdinaryLeafReadError::Unadmitted("root value is not FunctionBytecode".into())
    })?;
    let root = image.function(root_id).ok_or_else(|| {
        OrdinaryLeafReadError::Internal(
            "authenticated root function did not resolve in its source image".into(),
        )
    })?;
    let selected = root
        .constants()
        .get(selector.zero_based() as usize)
        .ok_or_else(|| {
            OrdinaryLeafReadError::Unadmitted(format!(
                "root constant selector {} is outside the constant pool",
                selector.zero_based()
            ))
        })?;
    let target_id = selected.function_id().ok_or_else(|| {
        OrdinaryLeafReadError::Unadmitted(format!(
            "root constant selector {} does not name FunctionBytecode",
            selector.zero_based()
        ))
    })?;
    if target_id == root_id {
        return unadmitted("root constant selector resolves back to the root function");
    }
    let target = image.function(target_id).ok_or_else(|| {
        OrdinaryLeafReadError::Internal(
            "authenticated child function did not resolve in its source image".into(),
        )
    })?;

    let envelope = target.envelope();
    let flags = envelope.flags();
    let js_mode = envelope.js_mode();
    if flags.raw() & !KNOWN_FUNCTION_FLAG_BITS != 0
        || flags.kind() != FunctionKind::Normal
        || !flags.has_prototype()
        || !flags.has_simple_parameter_list()
        || flags.is_derived_class_constructor()
        || flags.needs_home_object()
        || !flags.allows_new_target()
        || flags.allows_super_call()
        || flags.allows_super_property()
        || !flags.allows_arguments()
        || flags.is_direct_or_indirect_eval()
        || js_mode.raw() & !KNOWN_JS_MODE_BITS != 0
        || js_mode.is_async()
        || js_mode.is_backtrace_barrier()
        || !envelope.name_is_null()
        || envelope.defined_argument_count() != envelope.argument_count()
        || envelope.stack_size() > MAX_DECLARED_STACK
        || envelope.variable_reference_count() != 0
        || !envelope.closures().is_empty()
        || envelope.debug().is_some()
    {
        return unadmitted("function metadata is outside the ordinary synchronous leaf cohort");
    }

    let expected_locals = usize::from(envelope.argument_count())
        .checked_add(usize::from(envelope.variable_count()))
        .ok_or_else(|| {
            OrdinaryLeafReadError::Resource(
                "argument and local descriptor counts overflowed".into(),
            )
        })?;
    if envelope.locals().len() != expected_locals {
        return unadmitted("local descriptor count does not equal arguments plus local variables");
    }
    if envelope.locals().iter().any(|local| {
        !local.name_is_null() || local.variable_reference_index() != 0 || local.flags().raw() != 0
    }) {
        return unadmitted("local metadata carries a name, capture, or unsupported flag");
    }
    let metadata = OrdinaryLeafMetadataDraft {
        argument_count: envelope.argument_count(),
        defined_argument_count: envelope.defined_argument_count(),
        local_count: envelope.variable_count(),
        max_stack: envelope.stack_size(),
        is_strict: js_mode.is_strict(),
        has_simple_parameter_list: flags.has_simple_parameter_list(),
        has_prototype: flags.has_prototype(),
        allows_new_target: flags.allows_new_target(),
        allows_arguments: flags.allows_arguments(),
        strip_variable_debug: true,
    };
    let constants = preflight_constants(target.constants())?;
    let translated = translate_function(image, target_id, TranslationTarget::Ordinary)
        .map_err(classify_translation_error)?;
    let code = lower_code(
        &translated,
        metadata.argument_count,
        metadata.local_count,
        constants.len(),
        image.input_atom_slot_count(),
    )?;

    Ok(OrdinaryLeafDraft {
        metadata,
        constants,
        code,
    })
}

fn preflight_constants(
    constants: &[super::bytecode_image::ImageValue],
) -> Result<Box<[DetachedPrimitive]>, OrdinaryLeafReadError> {
    let mut output = Vec::new();
    output.try_reserve_exact(constants.len()).map_err(|_| {
        OrdinaryLeafReadError::Internal(
            "could not allocate the ordinary-leaf constant draft".into(),
        )
    })?;
    for constant in constants {
        let value = constant.as_wire().map_err(|_| {
            OrdinaryLeafReadError::Unadmitted(
                "ordinary-leaf constant pool contains a function or module identity".into(),
            )
        })?;
        output.push(project_primitive(value)?);
    }
    Ok(output.into_boxed_slice())
}

fn project_primitive(value: &WireValue) -> Result<DetachedPrimitive, OrdinaryLeafReadError> {
    match value {
        WireValue::Undefined => Ok(DetachedPrimitive::Undefined),
        WireValue::Null => Ok(DetachedPrimitive::Null),
        WireValue::Bool(value) => Ok(DetachedPrimitive::Bool(*value)),
        WireValue::Int32(value) => Ok(DetachedPrimitive::Int(*value)),
        WireValue::Float64Bits(bits) => Ok(DetachedPrimitive::Float64Bits(*bits)),
        WireValue::String(value) => copy_wire_string(value).map(DetachedPrimitive::String),
        WireValue::BigInt(bytes) => {
            copy_bigint(bytes).map(DetachedPrimitive::BigIntSignedLeCanonical)
        }
        WireValue::Node(_) => unadmitted("ordinary-leaf constant pool contains an object identity"),
    }
}

fn copy_wire_string(value: &WireString) -> Result<Box<[u16]>, OrdinaryLeafReadError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(value.len())
        .map_err(|_| OrdinaryLeafReadError::JsInternal("out of memory".into()))?;
    match value {
        WireString::Narrow(bytes) => copy.extend(bytes.iter().copied().map(u16::from)),
        WireString::Wide(units) => copy.extend(units.iter().copied()),
    }
    Ok(copy.into_boxed_slice())
}

fn copy_bigint(bytes: &[u8]) -> Result<Box<[u8]>, OrdinaryLeafReadError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len()).map_err(|_| {
        OrdinaryLeafReadError::Internal("could not allocate the ordinary-leaf BigInt draft".into())
    })?;
    copy.extend_from_slice(bytes);
    Ok(copy.into_boxed_slice())
}

fn lower_code(
    code: &FunctionCode<'_>,
    argument_count: u16,
    local_count: u16,
    constant_count: usize,
    input_atom_slot_count: u32,
) -> Result<Box<[OrdinaryLeafOp]>, OrdinaryLeafReadError> {
    let mut input_atoms = InputAtomLedger::new(input_atom_slot_count)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(code.instructions().len())
        .map_err(|_| {
            OrdinaryLeafReadError::Internal(
                "could not allocate the resolved ordinary-leaf code".into(),
            )
        })?;
    for instruction in code.instructions() {
        if !instruction.supports_ordinary() {
            return Err(unsupported_operation(instruction.rejection_diagnostic()));
        }
        if let FunctionOp::ThrowReadOnly(atom) = instruction.operation() {
            input_atoms.observe(atom)?;
        }
        output.push(lower_operation(
            instruction.operation(),
            argument_count,
            local_count,
            constant_count,
            code.instructions().len(),
        )?);
    }
    input_atoms.finish()?;
    Ok(output.into_boxed_slice())
}

struct InputAtomLedger {
    declared_slots: u32,
    used_input_slot: bool,
}

impl InputAtomLedger {
    fn new(declared_slots: u32) -> Result<Self, OrdinaryLeafReadError> {
        if declared_slots > 1 {
            return unadmitted(&format!(
                "ordinary-leaf image contains {declared_slots} input atom slots instead of at most one"
            ));
        }
        Ok(Self {
            declared_slots,
            used_input_slot: false,
        })
    }

    fn observe(&mut self, atom: &AtomOperand<'_>) -> Result<(), OrdinaryLeafReadError> {
        if !atom.originates_from_input_atom_table() {
            return Ok(());
        }
        if self.declared_slots == 0 {
            return Err(OrdinaryLeafReadError::Internal(
                "native atom provenance names an absent input atom slot".into(),
            ));
        }
        self.used_input_slot = true;
        Ok(())
    }

    fn finish(self) -> Result<(), OrdinaryLeafReadError> {
        if self.declared_slots == 1 && !self.used_input_slot {
            return unadmitted(
                "bytecode image's sole input atom slot is not used by an admitted read-only diagnostic",
            );
        }
        Ok(())
    }
}

fn lower_operation(
    operation: &FunctionOp<'_>,
    argument_count: u16,
    local_count: u16,
    constant_count: usize,
    instruction_count: usize,
) -> Result<OrdinaryLeafOp, OrdinaryLeafReadError> {
    match operation {
        FunctionOp::Nop => Ok(OrdinaryLeafOp::Nop),
        FunctionOp::Object => Ok(OrdinaryLeafOp::Object),
        FunctionOp::ToObject => Ok(OrdinaryLeafOp::ToObject),
        FunctionOp::PushI32(value) => Ok(OrdinaryLeafOp::PushI32(*value)),
        FunctionOp::PushConstant(index) => lower_constant(*index, constant_count),
        FunctionOp::PushUndefined => Ok(OrdinaryLeafOp::PushUndefined),
        FunctionOp::PushNull => Ok(OrdinaryLeafOp::PushNull),
        FunctionOp::PushBool(value) => Ok(OrdinaryLeafOp::PushBool(*value)),
        FunctionOp::PushBigIntI32(value) => Ok(OrdinaryLeafOp::PushBigIntI32(*value)),
        FunctionOp::PushEmptyString => Ok(OrdinaryLeafOp::PushEmptyString),
        FunctionOp::Stack(operation) => Ok(OrdinaryLeafOp::Stack(match operation {
            FunctionStackOp::Drop => OrdinaryLeafStackOp::Drop,
            FunctionStackOp::Nip => OrdinaryLeafStackOp::Nip,
            FunctionStackOp::Dup => OrdinaryLeafStackOp::Dup,
            FunctionStackOp::Dup1 => OrdinaryLeafStackOp::Dup1,
            FunctionStackOp::Dup3 => OrdinaryLeafStackOp::Dup3,
            FunctionStackOp::Insert2 => OrdinaryLeafStackOp::Insert2,
            FunctionStackOp::Insert3 => OrdinaryLeafStackOp::Insert3,
            FunctionStackOp::Insert4 => OrdinaryLeafStackOp::Insert4,
            FunctionStackOp::Perm3 => OrdinaryLeafStackOp::Perm3,
            FunctionStackOp::Perm4 => OrdinaryLeafStackOp::Perm4,
            FunctionStackOp::Perm5 => OrdinaryLeafStackOp::Perm5,
            FunctionStackOp::Swap => OrdinaryLeafStackOp::Swap,
            FunctionStackOp::Rot4Left => OrdinaryLeafStackOp::Rot4Left,
        })),
        FunctionOp::Unary(operation) => Ok(OrdinaryLeafOp::Unary(match operation {
            FunctionUnaryOp::Neg => OrdinaryLeafUnaryOp::Neg,
            FunctionUnaryOp::Plus => OrdinaryLeafUnaryOp::Plus,
            FunctionUnaryOp::Dec => OrdinaryLeafUnaryOp::Dec,
            FunctionUnaryOp::Inc => OrdinaryLeafUnaryOp::Inc,
            FunctionUnaryOp::BitNot => OrdinaryLeafUnaryOp::BitNot,
            FunctionUnaryOp::LogicalNot => OrdinaryLeafUnaryOp::LogicalNot,
            FunctionUnaryOp::TypeOf => OrdinaryLeafUnaryOp::TypeOf,
        })),
        FunctionOp::PostDec => Ok(OrdinaryLeafOp::PostDec),
        FunctionOp::PostInc => Ok(OrdinaryLeafOp::PostInc),
        FunctionOp::GetLocal(index) => lower_local(*index, local_count, OrdinaryLeafOp::GetLocal),
        FunctionOp::PutLocal(index) => lower_local(*index, local_count, OrdinaryLeafOp::PutLocal),
        FunctionOp::SetLocal(index) => lower_local(*index, local_count, OrdinaryLeafOp::SetLocal),
        FunctionOp::GetArgument(index) => {
            lower_argument(*index, argument_count, OrdinaryLeafOp::GetArgument)
        }
        FunctionOp::PutArgument(index) => {
            lower_argument(*index, argument_count, OrdinaryLeafOp::PutArgument)
        }
        FunctionOp::SetArgument(index) => {
            lower_argument(*index, argument_count, OrdinaryLeafOp::SetArgument)
        }
        FunctionOp::Binary(operation) => Ok(OrdinaryLeafOp::Binary(match operation {
            FunctionBinaryOp::Add => OrdinaryLeafBinaryOp::Add,
            FunctionBinaryOp::Sub => OrdinaryLeafBinaryOp::Sub,
            FunctionBinaryOp::Mul => OrdinaryLeafBinaryOp::Mul,
            FunctionBinaryOp::Div => OrdinaryLeafBinaryOp::Div,
            FunctionBinaryOp::Mod => OrdinaryLeafBinaryOp::Mod,
            FunctionBinaryOp::Pow => OrdinaryLeafBinaryOp::Pow,
            FunctionBinaryOp::Shl => OrdinaryLeafBinaryOp::Shl,
            FunctionBinaryOp::Sar => OrdinaryLeafBinaryOp::Sar,
            FunctionBinaryOp::Shr => OrdinaryLeafBinaryOp::Shr,
            FunctionBinaryOp::LessThan => OrdinaryLeafBinaryOp::LessThan,
            FunctionBinaryOp::LessThanOrEqual => OrdinaryLeafBinaryOp::LessThanOrEqual,
            FunctionBinaryOp::GreaterThan => OrdinaryLeafBinaryOp::GreaterThan,
            FunctionBinaryOp::GreaterThanOrEqual => OrdinaryLeafBinaryOp::GreaterThanOrEqual,
            FunctionBinaryOp::Equal => OrdinaryLeafBinaryOp::Equal,
            FunctionBinaryOp::NotEqual => OrdinaryLeafBinaryOp::NotEqual,
            FunctionBinaryOp::StrictEqual => OrdinaryLeafBinaryOp::StrictEqual,
            FunctionBinaryOp::StrictNotEqual => OrdinaryLeafBinaryOp::StrictNotEqual,
            FunctionBinaryOp::BitAnd => OrdinaryLeafBinaryOp::BitAnd,
            FunctionBinaryOp::BitXor => OrdinaryLeafBinaryOp::BitXor,
            FunctionBinaryOp::BitOr => OrdinaryLeafBinaryOp::BitOr,
        })),
        FunctionOp::Predicate(operation) => Ok(OrdinaryLeafOp::Predicate(match operation {
            FunctionPredicateOp::IsUndefinedOrNull => OrdinaryLeafPredicateOp::IsUndefinedOrNull,
            FunctionPredicateOp::IsUndefined => OrdinaryLeafPredicateOp::IsUndefined,
            FunctionPredicateOp::IsNull => OrdinaryLeafPredicateOp::IsNull,
            FunctionPredicateOp::TypeOfIsUndefined => OrdinaryLeafPredicateOp::TypeOfIsUndefined,
            FunctionPredicateOp::TypeOfIsFunction => OrdinaryLeafPredicateOp::TypeOfIsFunction,
        })),
        FunctionOp::IfFalse(target) => {
            validate_ir_target(*target, instruction_count).map(OrdinaryLeafOp::IfFalse)
        }
        FunctionOp::IfTrue(target) => {
            validate_ir_target(*target, instruction_count).map(OrdinaryLeafOp::IfTrue)
        }
        FunctionOp::Goto(target) => {
            validate_ir_target(*target, instruction_count).map(OrdinaryLeafOp::Goto)
        }
        FunctionOp::Call(argument_count) => Ok(OrdinaryLeafOp::Call(*argument_count)),
        FunctionOp::TailCall(argument_count) => Ok(OrdinaryLeafOp::TailCall(*argument_count)),
        FunctionOp::Construct(argument_count) => Ok(OrdinaryLeafOp::Construct(*argument_count)),
        FunctionOp::CallMethod(argument_count) => Ok(OrdinaryLeafOp::CallMethod(*argument_count)),
        FunctionOp::TailCallMethod(argument_count) => {
            Ok(OrdinaryLeafOp::TailCallMethod(*argument_count))
        }
        FunctionOp::ArrayFrom(element_count) => Ok(OrdinaryLeafOp::ArrayFrom(*element_count)),
        FunctionOp::Apply(kind) => Ok(OrdinaryLeafOp::Apply(match kind {
            FunctionApplyKind::Call => OrdinaryLeafApplyKind::Call,
            FunctionApplyKind::Construct => OrdinaryLeafApplyKind::Construct,
        })),
        FunctionOp::Return => Ok(OrdinaryLeafOp::Return),
        FunctionOp::ReturnUndefined => Ok(OrdinaryLeafOp::ReturnUndefined),
        FunctionOp::Throw => Ok(OrdinaryLeafOp::Throw),
        FunctionOp::ThrowReadOnly(atom) => {
            copy_read_only_name(atom).map(OrdinaryLeafOp::ThrowReadOnly)
        }
        _ => Err(OrdinaryLeafReadError::Internal(
            "ordinary-capable translated operation has no ordinary-leaf lowering".into(),
        )),
    }
}

fn copy_read_only_name(atom: &AtomOperand<'_>) -> Result<DetachedAtomName, OrdinaryLeafReadError> {
    if atom.class() != AtomOperandClass::String {
        return unadmitted("read-only diagnostic atom is not a String name");
    }
    let Some(length) = atom.string_utf16_len() else {
        return Err(OrdinaryLeafReadError::Internal(
            "String atom projection contained no spelling".into(),
        ));
    };
    let Some(units) = atom.string_utf16_units() else {
        return Err(OrdinaryLeafReadError::Internal(
            "String atom projection contained no spelling".into(),
        ));
    };
    let mut copy = Vec::new();
    copy.try_reserve_exact(length)
        .map_err(|_| OrdinaryLeafReadError::JsInternal("out of memory".into()))?;
    copy.extend(units);
    Ok(DetachedAtomName(copy.into_boxed_slice()))
}

fn lower_constant(
    index: u32,
    constant_count: usize,
) -> Result<OrdinaryLeafOp, OrdinaryLeafReadError> {
    if (index as usize) >= constant_count {
        return unadmitted("ordinary-leaf constant operand is outside the constant pool");
    }
    Ok(OrdinaryLeafOp::PushConst(index))
}

fn lower_local(
    index: u16,
    local_count: u16,
    operation: impl FnOnce(u16) -> OrdinaryLeafOp,
) -> Result<OrdinaryLeafOp, OrdinaryLeafReadError> {
    if index >= local_count {
        return unadmitted("ordinary-leaf local operand is outside the local slot table");
    }
    Ok(operation(index))
}

fn lower_argument(
    index: u16,
    argument_count: u16,
    operation: impl FnOnce(u16) -> OrdinaryLeafOp,
) -> Result<OrdinaryLeafOp, OrdinaryLeafReadError> {
    if index >= argument_count {
        return unadmitted("ordinary-leaf argument operand is outside the argument slot table");
    }
    Ok(operation(index))
}

fn validate_ir_target(
    target_instruction: u32,
    instruction_count: usize,
) -> Result<u32, OrdinaryLeafReadError> {
    if (target_instruction as usize) < instruction_count {
        Ok(target_instruction)
    } else {
        Err(OrdinaryLeafReadError::Internal(
            "authenticated native label did not resolve in the instruction map".into(),
        ))
    }
}

fn unsupported_operation(diagnostic: OperationDiagnostic) -> OrdinaryLeafReadError {
    OrdinaryLeafReadError::Unadmitted(format!(
        "native operation {} with {:?} operands is outside the admitted ordinary-leaf cohort",
        diagnostic.mnemonic(),
        diagnostic.operand_shape()
    ))
}

fn classify_translation_error(error: FunctionTranslateError) -> OrdinaryLeafReadError {
    if error.is_label_target_error() {
        return OrdinaryLeafReadError::Unadmitted(
            "ordinary-leaf control flow has an invalid native label target".into(),
        );
    }
    if error.is_unadmitted_operand_error() {
        return OrdinaryLeafReadError::Unadmitted(error.to_string());
    }
    let message = error.to_string();
    if message.is_empty() {
        OrdinaryLeafReadError::Internal(
            "ordinary-leaf native plan failed without a diagnostic".into(),
        )
    } else {
        OrdinaryLeafReadError::Internal(message)
    }
}

fn unadmitted<T>(message: &str) -> Result<T, OrdinaryLeafReadError> {
    Err(OrdinaryLeafReadError::Unadmitted(message.into()))
}

fn classify_image_error(error: BytecodeImageError) -> OrdinaryLeafReadError {
    let message = error.to_string();
    match error {
        BytecodeImageError::Wire(error) => classify_wire_error(error),
        BytecodeImageError::Atom(error) => classify_atom_error(error),
        BytecodeImageError::Data(error) => classify_data_error(error),
        BytecodeImageError::Envelope(error) => classify_envelope_error(error),
        BytecodeImageError::Module(_) | BytecodeImageError::ResourceLimit { .. } => {
            OrdinaryLeafReadError::Resource(message)
        }
        BytecodeImageError::CountOverflow { .. } => OrdinaryLeafReadError::Resource(message),
        BytecodeImageError::InvalidCompletionTarget
        | BytecodeImageError::InvalidFunctionState { .. }
        | BytecodeImageError::InvalidModuleState { .. }
        | BytecodeImageError::AllocationFailed => OrdinaryLeafReadError::Internal(message),
        BytecodeImageError::OffsetOverflow { .. } => OrdinaryLeafReadError::Malformed(message),
        BytecodeImageError::ModuleCountOutOfRange { .. } => {
            OrdinaryLeafReadError::JsInternal("out of memory".into())
        }
        BytecodeImageError::ModuleFieldOutOfRange { .. } => {
            OrdinaryLeafReadError::Unadmitted(message)
        }
    }
}

fn classify_atom_error(error: ImageAtomError) -> OrdinaryLeafReadError {
    let message = error.to_string();
    match error {
        ImageAtomError::Wire(error) => classify_wire_error(error),
        ImageAtomError::DynamicAtomCountOverflow { .. } => OrdinaryLeafReadError::Resource(message),
        ImageAtomError::AtomIndexSpaceMismatch { .. }
        | ImageAtomError::ForeignHeaderSlot { .. }
        | ImageAtomError::NullPropertyKey { .. } => OrdinaryLeafReadError::Malformed(message),
    }
}

fn classify_wire_error(error: WireError) -> OrdinaryLeafReadError {
    let message = error.to_string();
    match error {
        WireError::ResourceLimit { .. } => OrdinaryLeafReadError::Resource(message),
        WireError::AllocationFailed => OrdinaryLeafReadError::Internal(message),
        WireError::Truncated { .. } | WireError::MalformedUleb128 { .. } => {
            OrdinaryLeafReadError::Malformed("read after the end of the buffer".into())
        }
        WireError::InvalidAtomIndex { offset, .. } => {
            OrdinaryLeafReadError::Malformed(format!("invalid atom index (pos={offset})"))
        }
        WireError::StringTooLong { .. } => {
            OrdinaryLeafReadError::JsInternal("string too long".into())
        }
        _ => OrdinaryLeafReadError::Malformed(message),
    }
}

fn classify_data_error(
    error: DecodeError<super::bytecode_image::ImageOpaque>,
) -> OrdinaryLeafReadError {
    let message = error.to_string();
    match error {
        DecodeError::OpaqueObjectValue { .. } => {
            OrdinaryLeafReadError::Type("cannot convert to object".into())
        }
        DecodeError::OpaqueDateValue { .. } => {
            OrdinaryLeafReadError::Type("Number tag expected for date".into())
        }
        DecodeError::OpaqueTypedArrayBacking { .. } => {
            OrdinaryLeafReadError::Type("ArrayBuffer object expected".into())
        }
        DecodeError::InvalidArrayBuffer { reason, .. }
        | DecodeError::InvalidSharedArrayBuffer { reason, .. } => match reason {
            ArrayBufferLayoutError::MaximumTooSmall { .. } => {
                OrdinaryLeafReadError::Type("invalid array buffer".into())
            }
            ArrayBufferLayoutError::ByteLengthTooLarge { .. } => {
                OrdinaryLeafReadError::Range("invalid array buffer length".into())
            }
            ArrayBufferLayoutError::MaximumTooLarge { .. } => {
                OrdinaryLeafReadError::Range("invalid max array buffer length".into())
            }
        },
        DecodeError::InvalidTypedArrayKind { .. } => {
            OrdinaryLeafReadError::Type("invalid typed array".into())
        }
        DecodeError::InvalidTypedArrayBacking { .. } => {
            OrdinaryLeafReadError::Type("ArrayBuffer object expected".into())
        }
        DecodeError::InvalidTypedArray { reason, .. } => match reason {
            TypedArrayLayoutError::UnalignedByteOffset { .. } => {
                OrdinaryLeafReadError::Range("invalid offset".into())
            }
            TypedArrayLayoutError::ViewOutOfBounds { .. } => {
                OrdinaryLeafReadError::Range("invalid length".into())
            }
        },
        DecodeError::InvalidObjectValue { .. } => {
            OrdinaryLeafReadError::Type("cannot convert to object".into())
        }
        DecodeError::InvalidDate { .. } => {
            OrdinaryLeafReadError::Type("Number tag expected for date".into())
        }
        DecodeError::ObjectReferencesNotAllowed { .. }
        | DecodeError::SharedArrayBuffersNotAllowed { .. }
        | DecodeError::SharedArrayBufferArchive(_)
        | DecodeError::UnsupportedTag { .. }
        | DecodeError::InvalidObjectValueAlias { .. } => OrdinaryLeafReadError::Unadmitted(message),
        DecodeError::Wire(error) => classify_wire_error(error),
        DecodeError::Graph(GraphError::ResourceLimit { .. })
        | DecodeError::Graph(GraphError::CountOverflow { .. })
        | DecodeError::AtomCountOverflow { .. } => OrdinaryLeafReadError::Resource(message),
        DecodeError::Graph(GraphError::AllocationFailed)
        | DecodeError::MachineIdExhausted
        | DecodeError::InvalidCompletionTarget
        | DecodeError::InvalidNodeState { .. } => OrdinaryLeafReadError::Internal(message),
        DecodeError::Graph(
            GraphError::InvalidAtomIndex { .. } | GraphError::InvalidNodeIndex { .. },
        )
        | DecodeError::NullPropertyKey { .. }
        | DecodeError::NonCanonicalBigInt { .. } => OrdinaryLeafReadError::Malformed(message),
        DecodeError::Graph(GraphError::InvalidReferenceIndex {
            index,
            reference_count,
        }) => OrdinaryLeafReadError::Malformed(format!(
            "invalid object reference ({index} >= {reference_count})"
        )),
    }
}

fn classify_envelope_error(error: FunctionEnvelopeError) -> OrdinaryLeafReadError {
    let message = error.to_string();
    match error {
        FunctionEnvelopeError::Wire(error) => classify_wire_error(error),
        FunctionEnvelopeError::Code(error) => classify_code_error(error),
        FunctionEnvelopeError::FieldOutOfRange { .. } => {
            OrdinaryLeafReadError::JsInternal("out of memory".into())
        }
        FunctionEnvelopeError::ResourceLimit { .. }
        | FunctionEnvelopeError::CountOverflow { .. } => OrdinaryLeafReadError::Resource(message),
        FunctionEnvelopeError::AllocationFailed
        | FunctionEnvelopeError::InvalidAtomMode { .. }
        | FunctionEnvelopeError::InvalidModelBits { .. }
        | FunctionEnvelopeError::InvalidModelAtom { .. }
        | FunctionEnvelopeError::MismatchedAtomSpace { .. } => {
            OrdinaryLeafReadError::Internal(message)
        }
        _ => OrdinaryLeafReadError::Malformed(message),
    }
}

fn classify_code_error(error: CodeError) -> OrdinaryLeafReadError {
    let message = error.to_string();
    match error {
        CodeError::ResourceLimit { .. } | CodeError::CountOverflow { .. } => {
            OrdinaryLeafReadError::Resource(message)
        }
        CodeError::AllocationFailed
        | CodeError::InvalidAtomMode { .. }
        | CodeError::InvalidOpcodeLayout { .. }
        | CodeError::AtomCodecInvariant
        | CodeError::InvalidSidecar { .. } => OrdinaryLeafReadError::Internal(message),
        _ => OrdinaryLeafReadError::Malformed(message),
    }
}

#[cfg(test)]
mod tests {
    use super::super::graph::model::NodeId;
    use super::super::wire::{BcTag, WireWriter};
    use super::*;

    // QuickJS 2026-06-04, JS_WriteObject(JS_WRITE_OBJ_BYTECODE) for:
    // (function(a,b){var acc=.5;var step=b;while(a>0){if(a===2)
    // acc=(acc+step)/1;else acc=(acc+1)/1;a=a-1;}return acc===5.5?42:0;})
    const REAL_ORDINARY_LEAF_HEX: &str = concat!(
        "05000c000200a80100010001000001040100000000be00cb28",
        "0c43020000020202020000022e040001000000010000000000",
        "0000010000bd00c7d0c8cfb3a3e81acfb5a9e809c3c49bb4",
        "99c7ea07c3b49bb499c7cfb49cd3eae3c3bd01a9e804bb2a",
        "28b32806000000000000e03f060000000000001640",
    );
    // Property-free raw49/subtype0 wire mechanically reduced from pinned
    // QuickJS output for `(function(){'use strict';const x=0;x=1;})`.
    const READ_ONLY_LEAF_HEX: &str = concat!(
        "050102780c000200a801000100010000",
        "01040100000000be00cb280c43020100",
        "00000000000000060031f300000000",
    );
    const NATURAL_READ_ONLY_LEAF_HEX: &str = concat!(
        "050102780c000200a801000100010000",
        "01040100000000be00cb280c43020100",
        "000100020000000d01000000b05e0000",
        "b3c7b41131f300000000",
    );
    const MINIMAL_FUNCTION_RECORD: [u8; 23] = [
        0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01,
        0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
    ];

    fn bytes(hex: &str) -> Vec<u8> {
        assert_eq!(hex.len() % 2, 0);
        hex.as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let digit = |byte| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'A'..=b'F' => byte - b'A' + 10,
                    _ => panic!("test vector must be hexadecimal"),
                };
                (digit(pair[0]) << 4) | digit(pair[1])
            })
            .collect()
    }

    fn oracle() -> Vec<u8> {
        let object = bytes(REAL_ORDINARY_LEAF_HEX);
        assert_eq!(object.len(), 119);
        object
    }

    fn decode(input: &[u8]) -> Result<OrdinaryLeafDraft, OrdinaryLeafReadError> {
        decode_trusted_ordinary_leaf(input, RootFunctionConstantSelector::from_zero_based(0))
    }

    fn lower_ready(operation: FunctionOp<'_>) -> OrdinaryLeafOp {
        lower_operation(&operation, 4, 4, 4, 8).unwrap()
    }

    fn encode_primitive(value: &WireValue) -> Vec<u8> {
        let mut writer = WireWriter::new(256);
        match value {
            WireValue::Undefined => writer.write_tag(BcTag::Undefined).unwrap(),
            WireValue::Null => writer.write_tag(BcTag::Null).unwrap(),
            WireValue::Bool(false) => writer.write_tag(BcTag::BoolFalse).unwrap(),
            WireValue::Bool(true) => writer.write_tag(BcTag::BoolTrue).unwrap(),
            WireValue::Int32(value) => {
                writer.write_tag(BcTag::Int32).unwrap();
                writer.write_i32(*value).unwrap();
            }
            WireValue::Float64Bits(bits) => {
                writer.write_tag(BcTag::Float64).unwrap();
                writer.write_u64_le(*bits).unwrap();
            }
            WireValue::String(value) => {
                writer.write_tag(BcTag::String).unwrap();
                writer.write_string(value).unwrap();
            }
            WireValue::BigInt(bytes) => {
                writer.write_tag(BcTag::BigInt).unwrap();
                writer.write_uleb128(bytes.len() as u32).unwrap();
                writer.write_bytes(bytes).unwrap();
            }
            WireValue::Node(_) => panic!("node is not a primitive constant entry"),
        }
        writer.into_bytes()
    }

    fn oracle_with_both_constants(value: &WireValue) -> Vec<u8> {
        let entry = encode_primitive(value);
        let mut object = oracle();
        object.truncate(101);
        object.extend_from_slice(&entry);
        object.extend_from_slice(&entry);
        object
    }

    #[test]
    fn lowers_the_real_pinned_quickjs_leaf_with_ir_control_flow_targets() {
        let draft = decode(&oracle()).expect("pinned QuickJS ordinary leaf must be admitted");
        assert_eq!(
            draft.metadata(),
            OrdinaryLeafMetadataDraft {
                argument_count: 2,
                defined_argument_count: 2,
                local_count: 2,
                max_stack: 2,
                is_strict: false,
                has_simple_parameter_list: true,
                has_prototype: true,
                allows_new_target: true,
                allows_arguments: true,
                strip_variable_debug: true,
            }
        );
        assert_eq!(
            draft.constants(),
            &[
                DetachedPrimitive::Float64Bits(0.5_f64.to_bits()),
                DetachedPrimitive::Float64Bits(5.5_f64.to_bits()),
            ]
        );
        assert_eq!(
            draft.code(),
            &[
                OrdinaryLeafOp::PushConst(0),
                OrdinaryLeafOp::PutLocal(0),
                OrdinaryLeafOp::GetArgument(1),
                OrdinaryLeafOp::PutLocal(1),
                OrdinaryLeafOp::GetArgument(0),
                OrdinaryLeafOp::PushI32(0),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::GreaterThan),
                OrdinaryLeafOp::IfFalse(30),
                OrdinaryLeafOp::GetArgument(0),
                OrdinaryLeafOp::PushI32(2),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::StrictEqual),
                OrdinaryLeafOp::IfFalse(19),
                OrdinaryLeafOp::GetLocal(0),
                OrdinaryLeafOp::GetLocal(1),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Add),
                OrdinaryLeafOp::PushI32(1),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Div),
                OrdinaryLeafOp::PutLocal(0),
                OrdinaryLeafOp::Goto(25),
                OrdinaryLeafOp::GetLocal(0),
                OrdinaryLeafOp::PushI32(1),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Add),
                OrdinaryLeafOp::PushI32(1),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Div),
                OrdinaryLeafOp::PutLocal(0),
                OrdinaryLeafOp::GetArgument(0),
                OrdinaryLeafOp::PushI32(1),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Sub),
                OrdinaryLeafOp::PutArgument(0),
                OrdinaryLeafOp::Goto(4),
                OrdinaryLeafOp::GetLocal(0),
                OrdinaryLeafOp::PushConst(1),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::StrictEqual),
                OrdinaryLeafOp::IfFalse(36),
                OrdinaryLeafOp::PushI32(42),
                OrdinaryLeafOp::Return,
                OrdinaryLeafOp::PushI32(0),
                OrdinaryLeafOp::Return,
            ]
        );
    }

    #[test]
    fn lowers_property_free_read_only_with_owned_input_atom_spelling() {
        let object = bytes(READ_ONLY_LEAF_HEX);
        assert_eq!(object.len(), 47);
        let draft = decode(&object).expect("property-free raw49 leaf must be admitted");
        assert_eq!(draft.metadata().local_count(), 0);
        assert_eq!(draft.metadata().max_stack(), 0);
        assert!(draft.constants().is_empty());
        let [OrdinaryLeafOp::ThrowReadOnly(name)] = draft.code() else {
            panic!("raw49 did not lower to its owned read-only operation");
        };
        assert_eq!(name.0.as_ref(), "x".encode_utf16().collect::<Vec<_>>());
    }

    #[test]
    fn read_only_rejects_other_subtypes_non_string_atoms_and_atom_table_drift() {
        let original = bytes(READ_ONLY_LEAF_HEX);

        for subtype in [1, 2, 3, 4, 5, u8::MAX] {
            let mut object = original.clone();
            object[46] = subtype;
            let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&object) else {
                panic!("throw_error subtype {subtype} was admitted");
            };
            assert!(
                message.contains("admitted read-only subtype 0"),
                "{message}"
            );
        }

        for (label, raw_atom) in [
            ("null", 0_u32),
            ("index", 0x8000_002a),
            ("private", 229),
            ("symbol", 230),
        ] {
            let mut object = original.clone();
            object[42..46].copy_from_slice(&raw_atom.to_le_bytes());
            let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&object) else {
                panic!("{label} read-only atom was admitted");
            };
            assert!(message.contains("not a String name"), "{label}: {message}");
        }

        let mut unused = original.clone();
        unused[39] = 1;
        unused.truncate(41);
        unused.push(0x29); // return_undef, leaving the sole header atom unused
        let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&unused) else {
            panic!("unused input atom slot was admitted");
        };
        assert!(message.contains("not used by an admitted read-only diagnostic"));

        let mut multiple = original;
        multiple[1] = 2;
        multiple.splice(4..4, [0x02, b'y']);
        let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&multiple) else {
            panic!("multiple input atom slots were admitted");
        };
        assert!(message.contains("instead of at most one"));
    }

    #[test]
    fn read_only_accepts_only_string_names_under_zero_or_one_slot_provenance() {
        let original = bytes(READ_ONLY_LEAF_HEX);

        let mut predefined = original.clone();
        predefined[1] = 0;
        predefined.drain(2..4);
        // With the header removed, raw50 is the pinned `length` String atom.
        predefined[40..44].copy_from_slice(&50_u32.to_le_bytes());
        let draft = decode(&predefined).expect("predefined String needs no input atom slot");
        let [OrdinaryLeafOp::ThrowReadOnly(name)] = draft.code() else {
            panic!("predefined read-only atom did not lower");
        };
        assert_eq!(name.0.as_ref(), "length".encode_utf16().collect::<Vec<_>>());

        let mut manifest_alias = original.clone();
        manifest_alias.splice(2..4, [0x0c, b'l', b'e', b'n', b'g', b't', b'h']);
        let draft = decode(&manifest_alias)
            .expect("the sole header slot may intern to a predefined String");
        let [OrdinaryLeafOp::ThrowReadOnly(name)] = draft.code() else {
            panic!("manifest-alias read-only atom did not lower");
        };
        assert_eq!(name.0.as_ref(), "length".encode_utf16().collect::<Vec<_>>());

        let mut decimal_alias = original;
        decimal_alias.splice(2..4, [0x04, b'4', b'2']);
        let Err(OrdinaryLeafReadError::Unadmitted(message)) = decode(&decimal_alias) else {
            panic!("a decimal header alias was admitted as a String name");
        };
        assert!(message.contains("not a String name"));
    }

    #[test]
    fn natural_read_only_wire_remains_outside_the_nonlexical_leaf_cohort() {
        let object = bytes(NATURAL_READ_ONLY_LEAF_HEX);
        assert_eq!(object.len(), 58);
        assert!(matches!(
            decode(&object),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));
    }

    #[test]
    fn lowers_representative_sanitized_operations_without_consulting_diagnostics() {
        let cases = [
            (FunctionOp::Nop, OrdinaryLeafOp::Nop),
            (FunctionOp::Object, OrdinaryLeafOp::Object),
            (FunctionOp::ToObject, OrdinaryLeafOp::ToObject),
            (
                FunctionOp::PushI32(i32::MIN),
                OrdinaryLeafOp::PushI32(i32::MIN),
            ),
            (FunctionOp::PushConstant(3), OrdinaryLeafOp::PushConst(3)),
            (FunctionOp::GetLocal(3), OrdinaryLeafOp::GetLocal(3)),
            (FunctionOp::PutLocal(2), OrdinaryLeafOp::PutLocal(2)),
            (FunctionOp::SetLocal(1), OrdinaryLeafOp::SetLocal(1)),
            (FunctionOp::GetArgument(3), OrdinaryLeafOp::GetArgument(3)),
            (FunctionOp::PutArgument(2), OrdinaryLeafOp::PutArgument(2)),
            (FunctionOp::SetArgument(1), OrdinaryLeafOp::SetArgument(1)),
            (
                FunctionOp::Binary(FunctionBinaryOp::Add),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Add),
            ),
            (
                FunctionOp::Binary(FunctionBinaryOp::Sub),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Sub),
            ),
            (
                FunctionOp::Binary(FunctionBinaryOp::Div),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::Div),
            ),
            (
                FunctionOp::Binary(FunctionBinaryOp::GreaterThan),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::GreaterThan),
            ),
            (
                FunctionOp::Binary(FunctionBinaryOp::StrictEqual),
                OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::StrictEqual),
            ),
            (FunctionOp::IfFalse(7), OrdinaryLeafOp::IfFalse(7)),
            (FunctionOp::IfTrue(7), OrdinaryLeafOp::IfTrue(7)),
            (FunctionOp::Goto(0), OrdinaryLeafOp::Goto(0)),
            (FunctionOp::Return, OrdinaryLeafOp::Return),
            (FunctionOp::ReturnUndefined, OrdinaryLeafOp::ReturnUndefined),
            (FunctionOp::Throw, OrdinaryLeafOp::Throw),
        ];
        for (operation, expected) in cases {
            assert_eq!(lower_ready(operation), expected);
        }

        assert!(matches!(
            lower_operation(&FunctionOp::PushConstant(4), 4, 4, 4, 8,),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));
        assert!(matches!(
            lower_operation(&FunctionOp::GetLocal(4), 4, 4, 4, 8,),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));
        assert!(matches!(
            lower_operation(&FunctionOp::Goto(8), 4, 4, 4, 8,),
            Err(OrdinaryLeafReadError::Internal(_))
        ));
    }

    #[test]
    fn plain_call_argument_count_reaches_the_ordinary_dto_unchanged() {
        for argument_count in 0..=4 {
            assert_eq!(
                lower_ready(FunctionOp::Call(argument_count)),
                OrdinaryLeafOp::Call(argument_count)
            );
        }
    }

    #[test]
    fn non_tail_invocation_operands_reach_the_ordinary_dto_unchanged() {
        for (operation, expected) in [
            (
                FunctionOp::Construct(65_535),
                OrdinaryLeafOp::Construct(65_535),
            ),
            (
                FunctionOp::CallMethod(65_535),
                OrdinaryLeafOp::CallMethod(65_535),
            ),
            (
                FunctionOp::ArrayFrom(65_535),
                OrdinaryLeafOp::ArrayFrom(65_535),
            ),
        ] {
            assert_eq!(lower_ready(operation), expected);
        }
    }

    #[test]
    fn tail_invocation_operands_reach_the_ordinary_dto_unchanged() {
        for (operation, expected) in [
            (
                FunctionOp::TailCall(u16::MAX),
                OrdinaryLeafOp::TailCall(u16::MAX),
            ),
            (
                FunctionOp::TailCallMethod(u16::MAX),
                OrdinaryLeafOp::TailCallMethod(u16::MAX),
            ),
        ] {
            assert_eq!(lower_ready(operation), expected);
        }
    }

    #[test]
    fn apply_kind_reaches_the_ordinary_dto_without_raw_magic() {
        for (operation, expected) in [
            (
                FunctionOp::Apply(FunctionApplyKind::Call),
                OrdinaryLeafOp::Apply(OrdinaryLeafApplyKind::Call),
            ),
            (
                FunctionOp::Apply(FunctionApplyKind::Construct),
                OrdinaryLeafOp::Apply(OrdinaryLeafApplyKind::Construct),
            ),
        ] {
            assert_eq!(lower_ready(operation), expected);
        }
    }

    #[test]
    fn root_constant_selector_authenticates_the_child_without_fixing_an_image_id() {
        assert!(matches!(
            decode_trusted_ordinary_leaf(
                &oracle(),
                RootFunctionConstantSelector::from_zero_based(1)
            ),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));

        let mut non_function = oracle();
        // Replace the root constant's FunctionBytecode tag with Null. The
        // compatible reader deliberately accepts the now-trailing child bytes;
        // selection must still reject the non-function constant.
        non_function[25] = 0x01;
        assert!(matches!(
            decode(&non_function),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));

        assert!(matches!(
            decode(&[0x05, 0x00, 0x05, 0x54]),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));

        // Insert a complete sibling before the target in root cpool order.
        // The same target is now FunctionId 2 instead of FunctionId 1; only
        // the root selector changes, and no image identity crosses the API.
        let mut reindexed = oracle();
        reindexed[14] = 0x02;
        reindexed.splice(25..25, MINIMAL_FUNCTION_RECORD);
        let reindexed = decode_trusted_ordinary_leaf(
            &reindexed,
            RootFunctionConstantSelector::from_zero_based(1),
        )
        .unwrap();
        assert_eq!(reindexed.code()[34], OrdinaryLeafOp::PushI32(42));
    }

    #[test]
    fn detaches_every_runtime_independent_primitive_without_losing_bits() {
        let cases = [
            (WireValue::Undefined, DetachedPrimitive::Undefined),
            (WireValue::Null, DetachedPrimitive::Null),
            (WireValue::Bool(false), DetachedPrimitive::Bool(false)),
            (WireValue::Bool(true), DetachedPrimitive::Bool(true)),
            (WireValue::Int32(i32::MIN), DetachedPrimitive::Int(i32::MIN)),
            (
                WireValue::Float64Bits(0x8000_0000_0000_0000),
                DetachedPrimitive::Float64Bits(0x8000_0000_0000_0000),
            ),
            (
                WireValue::Float64Bits(0xfff8_0000_0000_0042),
                DetachedPrimitive::Float64Bits(0xfff8_0000_0000_0042),
            ),
            (
                WireValue::String(WireString::Narrow(Box::from([0x00, 0xff]))),
                DetachedPrimitive::String(Box::from([0x0000, 0x00ff])),
            ),
            (
                WireValue::String(WireString::Wide(Box::from([
                    0x0100, 0x0000, 0xd800, 0xdc00,
                ]))),
                DetachedPrimitive::String(Box::from([0x0100, 0x0000, 0xd800, 0xdc00])),
            ),
            (
                WireValue::BigInt(Box::from([])),
                DetachedPrimitive::BigIntSignedLeCanonical(Box::from([])),
            ),
            (
                WireValue::BigInt(Box::from([0x00, 0x80])),
                DetachedPrimitive::BigIntSignedLeCanonical(Box::from([0x00, 0x80])),
            ),
            (
                WireValue::BigInt(Box::from([0xff, 0x7f])),
                DetachedPrimitive::BigIntSignedLeCanonical(Box::from([0xff, 0x7f])),
            ),
        ];
        for (wire, expected) in cases {
            assert_eq!(project_primitive(&wire), Ok(expected.clone()));
            let draft = decode(&oracle_with_both_constants(&wire)).unwrap();
            assert_eq!(draft.constants(), &[expected.clone(), expected]);
        }

        // Compatible whole-image decoding normalizes redundant BigInt sign
        // extension before the detached primitive crosses this boundary.
        let draft = decode(&oracle_with_both_constants(&WireValue::BigInt(Box::from(
            [0x01, 0x00],
        ))))
        .unwrap();
        assert_eq!(
            draft.constants(),
            &[
                DetachedPrimitive::BigIntSignedLeCanonical(Box::from([0x01])),
                DetachedPrimitive::BigIntSignedLeCanonical(Box::from([0x01])),
            ]
        );
        assert!(matches!(
            project_primitive(&WireValue::Node(NodeId::from_zero_based(0))),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));
    }

    #[test]
    fn rejects_metadata_outside_the_structural_leaf_cohort() {
        let mutations = [
            (26, 0x42), // no prototype
            (26, 0x53), // generator kind
            (28, 0x04), // async JS mode
            (29, 0x01), // non-null function name
            (32, 0x01), // defined arguments differ from arguments
            (34, 0x01), // captured variable-reference count
            (39, 0x01), // named local descriptor
            (42, 0x01), // non-zero local flags
        ];
        for (offset, replacement) in mutations {
            let mut object = oracle();
            object[offset] = replacement;
            assert!(
                matches!(decode(&object), Err(OrdinaryLeafReadError::Unadmitted(_))),
                "metadata mutation at byte {offset} must be rejected"
            );
        }

        let mut strict = oracle();
        strict[28] = 0x01;
        assert!(decode(&strict).unwrap().metadata().is_strict());
    }

    #[test]
    fn rejects_outside_opcodes_operands_and_native_cfg_targets() {
        let mut unsupported_opcode = oracle();
        unsupported_opcode[72] = 0xa5; // instanceof in place of add
        assert_eq!(
            decode(&unsupported_opcode),
            Err(OrdinaryLeafReadError::Unadmitted(
                "native operation instanceof with None operands is outside the admitted ordinary-leaf cohort"
                    .into()
            ))
        );

        let mut scalar_only_opcode = oracle();
        scalar_only_opcode[55] = 0x04; // replace push_const with equal-width push_atom_value
        assert_eq!(
            decode(&scalar_only_opcode),
            Err(OrdinaryLeafReadError::Unadmitted(
                "native operation push_atom_value with Atom operands is outside the admitted ordinary-leaf cohort"
                    .into()
            ))
        );

        let mut constant_out_of_bounds = oracle();
        constant_out_of_bounds[56] = 0x02;
        assert!(matches!(
            decode(&constant_out_of_bounds),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));

        let mut argument_out_of_bounds = oracle();
        argument_out_of_bounds[60] = 0xd2; // get_arg3 in a two-argument leaf
        assert!(matches!(
            decode(&argument_out_of_bounds),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));

        let mut local_out_of_bounds = oracle();
        local_out_of_bounds[70] = 0xc6; // get_loc3 in a two-local leaf
        assert!(matches!(
            decode(&local_out_of_bounds),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));

        let mut non_boundary_label = oracle();
        non_boundary_label[64] = 0x05;
        assert!(matches!(
            decode(&non_boundary_label),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));

        let mut opaque_constant = oracle();
        let second_float = opaque_constant[110..119].to_vec();
        opaque_constant.truncate(101);
        opaque_constant.extend_from_slice(&MINIMAL_FUNCTION_RECORD);
        opaque_constant.extend_from_slice(&second_float);
        assert!(matches!(
            decode(&opaque_constant),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));

        let mut object_constant = oracle();
        let second_float = object_constant[110..119].to_vec();
        object_constant.truncate(101);
        object_constant.extend_from_slice(&[BcTag::Object.to_byte(), 0x00]);
        object_constant.extend_from_slice(&second_float);
        assert!(matches!(
            decode(&object_constant),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));
    }

    #[test]
    fn admission_is_structural_not_a_source_hash_or_exact_byte_vector() {
        let mut different_immediate = oracle();
        different_immediate[97] = 41;
        let draft = decode(&different_immediate).unwrap();
        assert_eq!(draft.code()[34], OrdinaryLeafOp::PushI32(41));

        let mut different_float_bits = oracle();
        different_float_bits[102..110].copy_from_slice(&(-0.0_f64).to_bits().to_le_bytes());
        let draft = decode(&different_float_bits).unwrap();
        assert_eq!(
            draft.constants()[0],
            DetachedPrimitive::Float64Bits((-0.0_f64).to_bits())
        );

        let mut different_scope_link = oracle();
        different_scope_link[40] = 0x02;
        assert!(decode(&different_scope_link).is_ok());

        let mut trailing = oracle();
        trailing.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        assert!(decode(&trailing).is_ok());
    }

    #[test]
    fn preserves_reader_and_resource_error_classes_before_cohort_admission() {
        assert!(matches!(
            decode(&oracle()[..118]),
            Err(OrdinaryLeafReadError::Malformed(_))
        ));
        assert!(matches!(
            decode(&vec![0; MAX_INPUT_BYTES + 1]),
            Err(OrdinaryLeafReadError::Resource(_))
        ));
    }
}
