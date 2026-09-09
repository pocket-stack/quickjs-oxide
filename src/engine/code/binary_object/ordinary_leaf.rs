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
pub(in crate::engine::code) struct RootFunctionConstantSelector(u32);

impl RootFunctionConstantSelector {
    #[must_use]
    pub(in crate::engine::code) const fn from_zero_based(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub(in crate::engine::code) const fn zero_based(self) -> u32 {
        self.0
    }
}

/// Owned metadata for one admitted ordinary synchronous leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::engine::code) struct OrdinaryLeafMetadataDraft {
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
    pub(in crate::engine::code) const fn argument_count(self) -> u16 {
        self.argument_count
    }

    #[must_use]
    pub(in crate::engine::code) const fn defined_argument_count(self) -> u16 {
        self.defined_argument_count
    }

    #[must_use]
    pub(in crate::engine::code) const fn local_count(self) -> u16 {
        self.local_count
    }

    #[must_use]
    pub(in crate::engine::code) const fn max_stack(self) -> u16 {
        self.max_stack
    }

    #[must_use]
    pub(in crate::engine::code) const fn is_strict(self) -> bool {
        self.is_strict
    }

    #[must_use]
    pub(in crate::engine::code) const fn has_simple_parameter_list(self) -> bool {
        self.has_simple_parameter_list
    }

    #[must_use]
    pub(in crate::engine::code) const fn has_prototype(self) -> bool {
        self.has_prototype
    }

    #[must_use]
    pub(in crate::engine::code) const fn allows_new_target(self) -> bool {
        self.allows_new_target
    }

    #[must_use]
    pub(in crate::engine::code) const fn allows_arguments(self) -> bool {
        self.allows_arguments
    }

    #[must_use]
    pub(in crate::engine::code) const fn strip_variable_debug(self) -> bool {
        self.strip_variable_debug
    }
}

/// Runtime-independent function-constant payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::engine::code) enum DetachedPrimitive {
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
pub(in crate::engine::code) struct DetachedAtomName(Box<[u16]>);

impl DetachedAtomName {
    pub(in crate::engine::code) fn into_units(self) -> Box<[u16]> {
        self.0
    }
}

/// One sanitized instruction in an ordinary-leaf draft.
///
/// Branch targets are instruction indices in this owned array, never native
/// byte PCs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::engine::code) enum OrdinaryLeafOp {
    Nop,
    Object,
    ToObject,
    ToPropKey,
    PushThis,
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
pub(in crate::engine::code) enum OrdinaryLeafApplyKind {
    Call,
    Construct,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::engine::code) enum OrdinaryLeafStackOp {
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
pub(in crate::engine::code) enum OrdinaryLeafUnaryOp {
    Neg,
    Plus,
    Dec,
    Inc,
    BitNot,
    LogicalNot,
    TypeOf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::engine::code) enum OrdinaryLeafBinaryOp {
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
pub(in crate::engine::code) enum OrdinaryLeafPredicateOp {
    IsUndefinedOrNull,
    IsUndefined,
    IsNull,
    TypeOfIsUndefined,
    TypeOfIsFunction,
}

/// Owned archive-side handoff for the transactional publication boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::engine::code) struct OrdinaryLeafDraft {
    metadata: OrdinaryLeafMetadataDraft,
    constants: Box<[DetachedPrimitive]>,
    code: Box<[OrdinaryLeafOp]>,
}

impl OrdinaryLeafDraft {
    #[must_use]
    pub(in crate::engine::code) const fn metadata(&self) -> OrdinaryLeafMetadataDraft {
        self.metadata
    }

    #[must_use]
    pub(in crate::engine::code) const fn constants(&self) -> &[DetachedPrimitive] {
        &self.constants
    }

    #[must_use]
    pub(in crate::engine::code) const fn code(&self) -> &[OrdinaryLeafOp] {
        &self.code
    }

    pub(in crate::engine::code) fn into_parts(
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
pub(in crate::engine::code) enum OrdinaryLeafReadError {
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
pub(in crate::engine::code) fn decode_trusted_ordinary_leaf(
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
    validate_push_this_protocol(code)?;
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

/// Pinned QuickJS emits `push_this` as the first physical and semantic
/// instruction of an ordinary function and never exposes that prologue as an
/// explicit branch destination. Keep that compiler contract at the archive
/// boundary so a mechanically valid raw stream cannot manufacture an
/// alternate entry point or a second receiver conversion.
fn validate_push_this_protocol(code: &FunctionCode<'_>) -> Result<(), OrdinaryLeafReadError> {
    let mut push_this_count = 0_usize;
    let mut push_this_index = None;
    for (index, instruction) in code.instructions().iter().enumerate() {
        if matches!(instruction.operation(), FunctionOp::PushThis) {
            push_this_count += 1;
            push_this_index.get_or_insert(index);
        }
    }
    if push_this_count == 0 {
        return Ok(());
    }
    if push_this_count != 1 {
        return unadmitted("push_this must occur exactly once in an ordinary-leaf body");
    }
    if push_this_index != Some(0) {
        return unadmitted("push_this must be typed instruction zero in an ordinary-leaf body");
    }
    if code.instructions().iter().any(|instruction| {
        matches!(
            instruction.operation(),
            FunctionOp::IfFalse(0) | FunctionOp::IfTrue(0) | FunctionOp::Goto(0)
        )
    }) {
        return unadmitted(
            "ordinary-leaf control flow must not explicitly target the push_this prologue",
        );
    }
    Ok(())
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
        FunctionOp::ToPropKey => Ok(OrdinaryLeafOp::ToPropKey),
        FunctionOp::PushThis => Ok(OrdinaryLeafOp::PushThis),
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
mod tests;
