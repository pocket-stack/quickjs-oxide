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
    NativeCodePlan, NativeOperands, decode_bytecode_image_body, decode_native_code_plan,
};
use super::code::{CodeError, CodeLimits};
use super::function_envelope::{FunctionEnvelopeError, FunctionEnvelopeLimits, FunctionKind};
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

/// One sanitized instruction in an ordinary-leaf draft.
///
/// Branch targets are instruction indices in this owned array, never native
/// byte PCs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum OrdinaryLeafOp {
    PushI32(i32),
    PushConst(u32),
    GetLocal(u16),
    PutLocal(u16),
    SetLocal(u16),
    GetArgument(u16),
    PutArgument(u16),
    SetArgument(u16),
    Add,
    Sub,
    Div,
    GreaterThan,
    StrictEqual,
    IfFalse(u32),
    Goto(u32),
    Return,
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
    if image.input_atom_slot_count() != 0 {
        return unadmitted("ordinary-leaf image carries an input atom table");
    }
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
    let plan = decode_native_code_plan(image, target_id).map_err(|error| {
        if error.is_label_target_error() {
            OrdinaryLeafReadError::Unadmitted(
                "ordinary-leaf control flow has an invalid native label target".into(),
            )
        } else {
            let message = error.to_string();
            if message.is_empty() {
                OrdinaryLeafReadError::Internal(
                    "ordinary-leaf native plan failed without a diagnostic".into(),
                )
            } else {
                OrdinaryLeafReadError::Internal(message)
            }
        }
    })?;
    let code = lower_code(
        &plan,
        metadata.argument_count,
        metadata.local_count,
        constants.len(),
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

enum PendingOp {
    Ready(OrdinaryLeafOp),
    IfFalse(u32),
    Goto(u32),
}

fn lower_code(
    plan: &NativeCodePlan<'_>,
    argument_count: u16,
    local_count: u16,
    constant_count: usize,
) -> Result<Box<[OrdinaryLeafOp]>, OrdinaryLeafReadError> {
    let mut source_to_ir = Vec::new();
    source_to_ir
        .try_reserve_exact(plan.instructions().len())
        .map_err(|_| {
            OrdinaryLeafReadError::Internal(
                "could not allocate the ordinary-leaf instruction map".into(),
            )
        })?;
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(plan.instructions().len())
        .map_err(|_| {
            OrdinaryLeafReadError::Internal(
                "could not allocate the ordinary-leaf instruction draft".into(),
            )
        })?;

    for instruction in plan.instructions() {
        let ir_index = u32::try_from(pending.len()).map_err(|_| {
            OrdinaryLeafReadError::Resource(
                "ordinary-leaf instruction count exceeds the draft index space".into(),
            )
        })?;
        source_to_ir.push(ir_index);
        pending.push(lower_instruction(
            instruction.opcode().name(),
            instruction.operands(),
            argument_count,
            local_count,
            constant_count,
        )?);
    }

    let mut output = Vec::new();
    output.try_reserve_exact(pending.len()).map_err(|_| {
        OrdinaryLeafReadError::Internal("could not allocate the resolved ordinary-leaf code".into())
    })?;
    for operation in pending {
        output.push(match operation {
            PendingOp::Ready(operation) => operation,
            PendingOp::IfFalse(target) => {
                OrdinaryLeafOp::IfFalse(resolve_ir_target(&source_to_ir, target)?)
            }
            PendingOp::Goto(target) => {
                OrdinaryLeafOp::Goto(resolve_ir_target(&source_to_ir, target)?)
            }
        });
    }
    Ok(output.into_boxed_slice())
}

fn lower_instruction(
    name: &str,
    operands: &NativeOperands<'_>,
    argument_count: u16,
    local_count: u16,
    constant_count: usize,
) -> Result<PendingOp, OrdinaryLeafReadError> {
    let ready = |operation| Ok(PendingOp::Ready(operation));
    match (name, operands) {
        ("push_i32", NativeOperands::I32(value)) => ready(OrdinaryLeafOp::PushI32(*value)),
        (
            "push_minus1" | "push_0" | "push_1" | "push_2" | "push_3" | "push_4" | "push_5"
            | "push_6" | "push_7",
            NativeOperands::NoneInt(value),
        ) => ready(OrdinaryLeafOp::PushI32(*value)),
        ("push_i8", NativeOperands::I8(value)) => ready(OrdinaryLeafOp::PushI32(i32::from(*value))),
        ("push_i16", NativeOperands::I16(value)) => {
            ready(OrdinaryLeafOp::PushI32(i32::from(*value)))
        }
        ("push_const", NativeOperands::Const(index)) => lower_constant(*index, constant_count),
        ("push_const8", NativeOperands::Const8(index)) => {
            lower_constant(u32::from(*index), constant_count)
        }
        ("get_loc", NativeOperands::Loc(index)) => {
            lower_local(*index, local_count, OrdinaryLeafOp::GetLocal)
        }
        ("get_loc8", NativeOperands::Loc8(index)) => {
            lower_local(u16::from(*index), local_count, OrdinaryLeafOp::GetLocal)
        }
        ("get_loc0" | "get_loc1" | "get_loc2" | "get_loc3", NativeOperands::NoneLoc(index)) => {
            lower_local(*index, local_count, OrdinaryLeafOp::GetLocal)
        }
        ("put_loc", NativeOperands::Loc(index)) => {
            lower_local(*index, local_count, OrdinaryLeafOp::PutLocal)
        }
        ("put_loc8", NativeOperands::Loc8(index)) => {
            lower_local(u16::from(*index), local_count, OrdinaryLeafOp::PutLocal)
        }
        ("put_loc0" | "put_loc1" | "put_loc2" | "put_loc3", NativeOperands::NoneLoc(index)) => {
            lower_local(*index, local_count, OrdinaryLeafOp::PutLocal)
        }
        ("set_loc", NativeOperands::Loc(index)) => {
            lower_local(*index, local_count, OrdinaryLeafOp::SetLocal)
        }
        ("set_loc8", NativeOperands::Loc8(index)) => {
            lower_local(u16::from(*index), local_count, OrdinaryLeafOp::SetLocal)
        }
        ("set_loc0" | "set_loc1" | "set_loc2" | "set_loc3", NativeOperands::NoneLoc(index)) => {
            lower_local(*index, local_count, OrdinaryLeafOp::SetLocal)
        }
        ("get_arg", NativeOperands::Arg(index)) => {
            lower_argument(*index, argument_count, OrdinaryLeafOp::GetArgument)
        }
        ("get_arg0" | "get_arg1" | "get_arg2" | "get_arg3", NativeOperands::NoneArg(index)) => {
            lower_argument(*index, argument_count, OrdinaryLeafOp::GetArgument)
        }
        ("put_arg", NativeOperands::Arg(index)) => {
            lower_argument(*index, argument_count, OrdinaryLeafOp::PutArgument)
        }
        ("put_arg0" | "put_arg1" | "put_arg2" | "put_arg3", NativeOperands::NoneArg(index)) => {
            lower_argument(*index, argument_count, OrdinaryLeafOp::PutArgument)
        }
        ("set_arg", NativeOperands::Arg(index)) => {
            lower_argument(*index, argument_count, OrdinaryLeafOp::SetArgument)
        }
        ("set_arg0" | "set_arg1" | "set_arg2" | "set_arg3", NativeOperands::NoneArg(index)) => {
            lower_argument(*index, argument_count, OrdinaryLeafOp::SetArgument)
        }
        ("add", NativeOperands::None) => ready(OrdinaryLeafOp::Add),
        ("sub", NativeOperands::None) => ready(OrdinaryLeafOp::Sub),
        ("div", NativeOperands::None) => ready(OrdinaryLeafOp::Div),
        ("gt", NativeOperands::None) => ready(OrdinaryLeafOp::GreaterThan),
        ("strict_eq", NativeOperands::None) => ready(OrdinaryLeafOp::StrictEqual),
        ("if_false", NativeOperands::Label(label)) => {
            Ok(PendingOp::IfFalse(label.target_instruction()))
        }
        ("if_false8", NativeOperands::Label8(label)) => {
            Ok(PendingOp::IfFalse(label.target_instruction()))
        }
        ("goto", NativeOperands::Label(label)) => Ok(PendingOp::Goto(label.target_instruction())),
        ("goto8", NativeOperands::Label8(label)) => Ok(PendingOp::Goto(label.target_instruction())),
        ("goto16", NativeOperands::Label16(label)) => {
            Ok(PendingOp::Goto(label.target_instruction()))
        }
        ("return", NativeOperands::None) => ready(OrdinaryLeafOp::Return),
        _ => unadmitted(&format!(
            "native operation {name} with {:?} operands is outside the first ordinary-leaf cohort",
            operands.format()
        )),
    }
}

fn lower_constant(index: u32, constant_count: usize) -> Result<PendingOp, OrdinaryLeafReadError> {
    if (index as usize) >= constant_count {
        return unadmitted("ordinary-leaf constant operand is outside the constant pool");
    }
    Ok(PendingOp::Ready(OrdinaryLeafOp::PushConst(index)))
}

fn lower_local(
    index: u16,
    local_count: u16,
    operation: impl FnOnce(u16) -> OrdinaryLeafOp,
) -> Result<PendingOp, OrdinaryLeafReadError> {
    if index >= local_count {
        return unadmitted("ordinary-leaf local operand is outside the local slot table");
    }
    Ok(PendingOp::Ready(operation(index)))
}

fn lower_argument(
    index: u16,
    argument_count: u16,
    operation: impl FnOnce(u16) -> OrdinaryLeafOp,
) -> Result<PendingOp, OrdinaryLeafReadError> {
    if index >= argument_count {
        return unadmitted("ordinary-leaf argument operand is outside the argument slot table");
    }
    Ok(PendingOp::Ready(operation(index)))
}

fn resolve_ir_target(
    source_to_ir: &[u32],
    target_instruction: u32,
) -> Result<u32, OrdinaryLeafReadError> {
    source_to_ir
        .get(target_instruction as usize)
        .copied()
        .ok_or_else(|| {
            OrdinaryLeafReadError::Internal(
                "authenticated native label did not resolve in the instruction map".into(),
            )
        })
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

    fn lower_ready(name: &str, operands: NativeOperands<'_>) -> OrdinaryLeafOp {
        match lower_instruction(name, &operands, 4, 4, 4).unwrap() {
            PendingOp::Ready(operation) => operation,
            PendingOp::IfFalse(_) | PendingOp::Goto(_) => {
                panic!("test expected a non-branch operation")
            }
        }
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
                OrdinaryLeafOp::GreaterThan,
                OrdinaryLeafOp::IfFalse(30),
                OrdinaryLeafOp::GetArgument(0),
                OrdinaryLeafOp::PushI32(2),
                OrdinaryLeafOp::StrictEqual,
                OrdinaryLeafOp::IfFalse(19),
                OrdinaryLeafOp::GetLocal(0),
                OrdinaryLeafOp::GetLocal(1),
                OrdinaryLeafOp::Add,
                OrdinaryLeafOp::PushI32(1),
                OrdinaryLeafOp::Div,
                OrdinaryLeafOp::PutLocal(0),
                OrdinaryLeafOp::Goto(25),
                OrdinaryLeafOp::GetLocal(0),
                OrdinaryLeafOp::PushI32(1),
                OrdinaryLeafOp::Add,
                OrdinaryLeafOp::PushI32(1),
                OrdinaryLeafOp::Div,
                OrdinaryLeafOp::PutLocal(0),
                OrdinaryLeafOp::GetArgument(0),
                OrdinaryLeafOp::PushI32(1),
                OrdinaryLeafOp::Sub,
                OrdinaryLeafOp::PutArgument(0),
                OrdinaryLeafOp::Goto(4),
                OrdinaryLeafOp::GetLocal(0),
                OrdinaryLeafOp::PushConst(1),
                OrdinaryLeafOp::StrictEqual,
                OrdinaryLeafOp::IfFalse(36),
                OrdinaryLeafOp::PushI32(42),
                OrdinaryLeafOp::Return,
                OrdinaryLeafOp::PushI32(0),
                OrdinaryLeafOp::Return,
            ]
        );
    }

    #[test]
    fn normalizes_every_admitted_non_label_operand_width_by_name_and_format() {
        let cases = [
            (
                "push_i32",
                NativeOperands::I32(i32::MIN),
                OrdinaryLeafOp::PushI32(i32::MIN),
            ),
            (
                "push_minus1",
                NativeOperands::NoneInt(-1),
                OrdinaryLeafOp::PushI32(-1),
            ),
            (
                "push_i8",
                NativeOperands::I8(i8::MIN),
                OrdinaryLeafOp::PushI32(i32::from(i8::MIN)),
            ),
            (
                "push_i16",
                NativeOperands::I16(i16::MIN),
                OrdinaryLeafOp::PushI32(i32::from(i16::MIN)),
            ),
            (
                "push_const",
                NativeOperands::Const(3),
                OrdinaryLeafOp::PushConst(3),
            ),
            (
                "push_const8",
                NativeOperands::Const8(2),
                OrdinaryLeafOp::PushConst(2),
            ),
            (
                "get_loc",
                NativeOperands::Loc(3),
                OrdinaryLeafOp::GetLocal(3),
            ),
            (
                "get_loc8",
                NativeOperands::Loc8(2),
                OrdinaryLeafOp::GetLocal(2),
            ),
            (
                "get_loc1",
                NativeOperands::NoneLoc(1),
                OrdinaryLeafOp::GetLocal(1),
            ),
            (
                "put_loc",
                NativeOperands::Loc(3),
                OrdinaryLeafOp::PutLocal(3),
            ),
            (
                "put_loc8",
                NativeOperands::Loc8(2),
                OrdinaryLeafOp::PutLocal(2),
            ),
            (
                "put_loc1",
                NativeOperands::NoneLoc(1),
                OrdinaryLeafOp::PutLocal(1),
            ),
            (
                "set_loc",
                NativeOperands::Loc(3),
                OrdinaryLeafOp::SetLocal(3),
            ),
            (
                "set_loc8",
                NativeOperands::Loc8(2),
                OrdinaryLeafOp::SetLocal(2),
            ),
            (
                "set_loc1",
                NativeOperands::NoneLoc(1),
                OrdinaryLeafOp::SetLocal(1),
            ),
            (
                "get_arg",
                NativeOperands::Arg(3),
                OrdinaryLeafOp::GetArgument(3),
            ),
            (
                "get_arg1",
                NativeOperands::NoneArg(1),
                OrdinaryLeafOp::GetArgument(1),
            ),
            (
                "put_arg",
                NativeOperands::Arg(3),
                OrdinaryLeafOp::PutArgument(3),
            ),
            (
                "put_arg1",
                NativeOperands::NoneArg(1),
                OrdinaryLeafOp::PutArgument(1),
            ),
            (
                "set_arg",
                NativeOperands::Arg(3),
                OrdinaryLeafOp::SetArgument(3),
            ),
            (
                "set_arg1",
                NativeOperands::NoneArg(1),
                OrdinaryLeafOp::SetArgument(1),
            ),
            ("add", NativeOperands::None, OrdinaryLeafOp::Add),
            ("sub", NativeOperands::None, OrdinaryLeafOp::Sub),
            ("div", NativeOperands::None, OrdinaryLeafOp::Div),
            ("gt", NativeOperands::None, OrdinaryLeafOp::GreaterThan),
            (
                "strict_eq",
                NativeOperands::None,
                OrdinaryLeafOp::StrictEqual,
            ),
            ("return", NativeOperands::None, OrdinaryLeafOp::Return),
        ];
        for (name, operands, expected) in cases {
            assert_eq!(lower_ready(name, operands), expected);
        }

        assert!(matches!(
            lower_instruction("push_i32", &NativeOperands::U32(1), 4, 4, 4),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));
        assert!(matches!(
            lower_instruction("get_loc", &NativeOperands::Loc8(1), 4, 4, 4),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));
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
        unsupported_opcode[72] = 0x98; // mul in place of add
        assert!(matches!(
            decode(&unsupported_opcode),
            Err(OrdinaryLeafReadError::Unadmitted(_))
        ));

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
