//! Narrow semantic admission for trusted scalar-script BC5 objects.
//!
//! This layer always completes the release-pinned whole-image read before it
//! considers execution eligibility. A successful result is an inert scalar
//! draft, not verified bytecode and not a runtime or heap identity.

use std::fmt;

use super::bytecode_image::{
    BytecodeImage, BytecodeImageError, BytecodeImageLimits, ImageAtomError,
    ImageStringAtomProjectionError, ModuleLimits, decode_bytecode_image_body,
};
use super::code::{CodeError, CodeLimits};
use super::function_envelope::{FunctionEnvelopeError, FunctionEnvelopeLimits};
use super::graph::decode::DecodeError;
use super::graph::model::{
    ArrayBufferLayoutError, GraphError, GraphLimits, TypedArrayLayoutError, WireValue,
};
use super::wire::{ReaderMode, WireCursor, WireError, WireLimits};

const MAX_INPUT_BYTES: usize = 4096;

const OP_PUSH_I32: u8 = 0x01;
const OP_PUSH_CONST: u8 = 0x02;
const OP_PUSH_ATOM_VALUE: u8 = 0x04;
const OP_UNDEFINED: u8 = 0x06;
const OP_NULL: u8 = 0x07;
const OP_PUSH_FALSE: u8 = 0x09;
const OP_PUSH_TRUE: u8 = 0x0a;
const OP_RETURN: u8 = 0x28;
const OP_PUSH_BIGINT_I32: u8 = 0xb0;
const OP_PUSH_MINUS1: u8 = 0xb2;
const OP_PUSH_0: u8 = 0xb3;
const OP_PUSH_7: u8 = 0xba;
const OP_PUSH_I8: u8 = 0xbb;
const OP_PUSH_I16: u8 = 0xbc;
const OP_PUSH_CONST8: u8 = 0xbd;
const OP_PUSH_EMPTY_STRING: u8 = 0xbf;
const OP_NEG: u8 = 0x8a;
const OP_SET_LOC0: u8 = 0xcb;

/// Runtime-independent result of the executable BC5 scalar admission cohort.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ScalarScriptDraft {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Float64Bits(u64),
    BigIntI32(i32),
    BigIntBytes(Box<[u8]>),
    NegatedBigIntI32(i32),
    NegatedBigIntBytes(Box<[u8]>),
    EmptyString,
    ConstantString(ScalarStringDraft),
    AtomString(ScalarStringDraft),
    IntegerAtomString(u32),
}

/// Runtime-independent UTF-16 value crossing the archive/publication bridge.
/// The private representation prevents the scalar reader from leaking either
/// BC5 wire width or an image/runtime atom identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ScalarStringDraft(Box<[u16]>);

impl ScalarStringDraft {
    pub(in crate::runtime) fn into_units(self) -> Box<[u16]> {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ScalarPush {
    Direct(ScalarScriptDraft),
    Constant(u32),
    AtomValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BigIntPush {
    DirectI32(i32),
    Constant(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ScalarSequence {
    Plain { push: ScalarPush, width: u32 },
    NegatedBigInt { push: BigIntPush, width: u32 },
}

/// Failure classes preserved across the archive/runtime publication boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ScalarScriptReadError {
    /// The compatible reader rejected the BC5 structure.
    Malformed(String),
    /// The compatible reader rejected a decoded value with QuickJS's
    /// authenticated `TypeError` semantics.
    Type(String),
    /// The compatible reader rejected a decoded value with QuickJS's
    /// release-pinned `RangeError` semantics.
    Range(String),
    /// The compatible reader rejected a decoded value with QuickJS's
    /// JavaScript-visible `InternalError` semantics.
    JsInternal(String),
    /// The structure is understood but outside this executable cohort.
    Unadmitted(String),
    /// A caller-owned resource policy rejected the input.
    Resource(String),
    /// A supposedly unreachable codec or allocation invariant failed.
    Internal(String),
}

impl fmt::Display for ScalarScriptReadError {
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
                "BC5 object is not admitted as a trusted scalar script: {message}"
            ),
            Self::Resource(message) => {
                write!(formatter, "BC5 scalar-script resource limit: {message}")
            }
            Self::Internal(message) => {
                write!(formatter, "BC5 scalar-script internal failure: {message}")
            }
        }
    }
}

impl std::error::Error for ScalarScriptReadError {}

/// Decode one complete pinned-QuickJS object and admit the branch-free direct
/// scalar script shape. Compatibility mode is semantic: pinned QuickJS accepts
/// non-minimal ULEB values and trailing bytes, so this path must do the same.
pub(in crate::runtime) fn decode_trusted_scalar_script(
    input: &[u8],
) -> Result<ScalarScriptDraft, ScalarScriptReadError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(ScalarScriptReadError::Resource(format!(
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
    admit_image(&image)
}

#[derive(Clone, Copy)]
struct AdmissionLimits {
    wire: WireLimits,
    image: BytecodeImageLimits,
}

impl AdmissionLimits {
    fn for_input(input_bytes: usize) -> Self {
        // Every aggregate count consumes at least one input byte. Tying all
        // inner caps to the already bounded input keeps valid small objects
        // fully classifiable while avoiding a second, hidden allocation
        // policy inside scalar admission.
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

fn admit_image(image: &BytecodeImage) -> Result<ScalarScriptDraft, ScalarScriptReadError> {
    if !image.reference_table().is_empty() {
        return unadmitted("object-reference table is not empty");
    }
    if !image.modules().is_empty() {
        return unadmitted("root image contains a Module record");
    }

    let [function] = image.functions() else {
        return unadmitted("image does not contain exactly one FunctionBytecode record");
    };
    let Some(root) = image.root().function_id() else {
        return unadmitted("root value is not FunctionBytecode");
    };
    if root.zero_based() != 0 {
        return Err(ScalarScriptReadError::Internal(
            "single function root did not retain index zero".into(),
        ));
    }
    if image.function(root) != Some(function) {
        return Err(ScalarScriptReadError::Internal(
            "authenticated function root did not resolve in its source image".into(),
        ));
    }

    let envelope = function.envelope();
    if envelope.flags().raw() != 0x0200
        || envelope.js_mode().raw() != 0
        || !envelope.name_is_pinned_eval()
        || envelope.argument_count() != 0
        || envelope.variable_count() != 1
        || envelope.defined_argument_count() != 0
        || envelope.stack_size() != 1
        || envelope.variable_reference_count() != 0
        || !envelope.closures().is_empty()
        || envelope.debug().is_some()
    {
        return unadmitted("function metadata is outside the stripped scalar-script cohort");
    }

    let [local] = envelope.locals() else {
        return unadmitted("function does not have the single completion local");
    };
    if !local.name_is_null()
        || local.scope_next().value() != -1
        || local.variable_reference_index() != 0
        || local.flags().raw() != 0
    {
        return unadmitted("completion-local metadata is outside the admitted shape");
    }

    let native_payload = envelope.code();
    let Some(sequence) = decode_scalar_sequence(native_payload.as_bytes()) else {
        return unadmitted("native payload opcode sequence is outside the admitted shape");
    };
    if !matches!(
        sequence,
        ScalarSequence::Plain {
            push: ScalarPush::AtomValue,
            ..
        }
    ) && (image.input_atom_slot_count() != 0 || !native_payload.atom_relocations().is_empty())
    {
        return unadmitted("atom-free scalar shape carries an atom table or relocation");
    }
    let sidecars_match = match (&sequence, native_payload.instructions()) {
        (ScalarSequence::Plain { width, .. }, [push, set_completion, return_value]) => {
            push.offset() == 0
                && push.opcode().raw() == native_payload.as_bytes()[0]
                && set_completion.offset() == *width
                && set_completion.opcode().raw() == OP_SET_LOC0
                && return_value.offset() == *width + 1
                && return_value.opcode().raw() == OP_RETURN
        }
        (
            ScalarSequence::NegatedBigInt { width, .. },
            [push, negate, set_completion, return_value],
        ) => {
            push.offset() == 0
                && push.opcode().raw() == native_payload.as_bytes()[0]
                && negate.offset() == *width
                && negate.opcode().raw() == OP_NEG
                && set_completion.offset() == *width + 1
                && set_completion.opcode().raw() == OP_SET_LOC0
                && return_value.offset() == *width + 2
                && return_value.opcode().raw() == OP_RETURN
        }
        _ => false,
    };
    if !sidecars_match {
        return Err(ScalarScriptReadError::Internal(
            "instruction sidecars disagree with their owned native bytes".into(),
        ));
    }

    let draft = match (sequence, function.constants()) {
        (
            ScalarSequence::Plain {
                push: ScalarPush::Direct(draft),
                ..
            },
            [],
        ) => draft,
        (
            ScalarSequence::Plain {
                push: ScalarPush::Direct(_),
                ..
            },
            _,
        ) => {
            return unadmitted("direct scalar opcode carries a function constant");
        }
        (
            ScalarSequence::Plain {
                push: ScalarPush::Constant(0),
                ..
            },
            [constant],
        ) => match constant.as_wire() {
            Ok(WireValue::Float64Bits(bits)) => ScalarScriptDraft::Float64Bits(*bits),
            Ok(WireValue::BigInt(bytes)) => {
                ScalarScriptDraft::BigIntBytes(copy_bigint_bytes(bytes)?)
            }
            Ok(WireValue::String(value)) => {
                ScalarScriptDraft::ConstantString(copy_wire_string(value)?)
            }
            Ok(_) => {
                return unadmitted("scalar constant is not a Float64, BigInt, or String value");
            }
            Err(_) => return unadmitted("scalar constant is not a data value"),
        },
        (
            ScalarSequence::Plain {
                push: ScalarPush::Constant(_),
                ..
            },
            [_],
        ) => {
            return unadmitted("scalar constant opcode does not reference index zero");
        }
        (
            ScalarSequence::Plain {
                push: ScalarPush::Constant(_),
                ..
            },
            _,
        ) => {
            return unadmitted("scalar constant opcode requires exactly one function constant");
        }
        (
            ScalarSequence::Plain {
                push: ScalarPush::AtomValue,
                ..
            },
            [],
        ) => project_atom_string(image, root)?,
        (
            ScalarSequence::Plain {
                push: ScalarPush::AtomValue,
                ..
            },
            _,
        ) => return unadmitted("atom-value scalar opcode carries a function constant"),
        (
            ScalarSequence::NegatedBigInt {
                push: BigIntPush::DirectI32(value),
                ..
            },
            [],
        ) => ScalarScriptDraft::NegatedBigIntI32(value),
        (
            ScalarSequence::NegatedBigInt {
                push: BigIntPush::DirectI32(_),
                ..
            },
            _,
        ) => return unadmitted("direct negated BigInt opcode carries a function constant"),
        (
            ScalarSequence::NegatedBigInt {
                push: BigIntPush::Constant(0),
                ..
            },
            [constant],
        ) => match constant.as_wire() {
            Ok(WireValue::BigInt(bytes)) => {
                ScalarScriptDraft::NegatedBigIntBytes(copy_bigint_bytes(bytes)?)
            }
            Ok(_) => return unadmitted("negated scalar constant is not a BigInt value"),
            Err(_) => return unadmitted("negated scalar constant is not a data value"),
        },
        (
            ScalarSequence::NegatedBigInt {
                push: BigIntPush::Constant(_),
                ..
            },
            [_],
        ) => return unadmitted("negated BigInt opcode does not reference index zero"),
        (
            ScalarSequence::NegatedBigInt {
                push: BigIntPush::Constant(_),
                ..
            },
            _,
        ) => {
            return unadmitted("negated BigInt opcode requires exactly one function constant");
        }
    };

    Ok(draft)
}

fn project_atom_string(
    image: &BytecodeImage,
    function: super::bytecode_image::FunctionId,
) -> Result<ScalarScriptDraft, ScalarScriptReadError> {
    let projection = image
        .project_single_string_atom(function)
        .map_err(classify_string_atom_projection_error)?;
    if projection.operand_offset() != 1 {
        return Err(ScalarScriptReadError::Internal(
            "atom-value relocation did not retain operand offset one".into(),
        ));
    }
    if let Some(value) = projection.canonical_decimal() {
        return Ok(ScalarScriptDraft::IntegerAtomString(value));
    }
    if let Some(value) = projection.manifest_spelling() {
        return copy_utf16(value.encode_utf16(), value.encode_utf16().count())
            .map(ScalarScriptDraft::AtomString);
    }
    if let Some(value) = projection.dynamic_string() {
        return copy_wire_string(value).map(ScalarScriptDraft::AtomString);
    }
    Err(ScalarScriptReadError::Internal(
        "String atom projection contained no spelling".into(),
    ))
}

fn classify_string_atom_projection_error(
    error: ImageStringAtomProjectionError,
) -> ScalarScriptReadError {
    let message = error.to_string();
    match error {
        ImageStringAtomProjectionError::NullAtom
        | ImageStringAtomProjectionError::PrivateAtom
        | ImageStringAtomProjectionError::SymbolAtom
        | ImageStringAtomProjectionError::AtomRelocationCount { .. }
        | ImageStringAtomProjectionError::InputAtomSlotCount { .. }
        | ImageStringAtomProjectionError::UnpairedInputAtomSlot => {
            ScalarScriptReadError::Unadmitted(message)
        }
        ImageStringAtomProjectionError::FunctionNotInImage
        | ImageStringAtomProjectionError::MissingAtomOperand
        | ImageStringAtomProjectionError::MissingDynamicString => {
            ScalarScriptReadError::Internal(message)
        }
    }
}

fn copy_wire_string(
    value: &super::wire::WireString,
) -> Result<ScalarStringDraft, ScalarScriptReadError> {
    match value {
        super::wire::WireString::Narrow(bytes) => {
            copy_utf16(bytes.iter().copied().map(u16::from), bytes.len())
        }
        super::wire::WireString::Wide(units) => copy_utf16(units.iter().copied(), units.len()),
    }
}

fn copy_utf16(
    units: impl IntoIterator<Item = u16>,
    length: usize,
) -> Result<ScalarStringDraft, ScalarScriptReadError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(length)
        .map_err(|_| ScalarScriptReadError::JsInternal("out of memory".into()))?;
    copy.extend(units);
    Ok(ScalarStringDraft(copy.into_boxed_slice()))
}

/// Preserve the normalized signed payload without coupling the archival
/// decoder to the runtime BigInt representation. The whole-image model owns
/// its constants, so the narrow DTO needs one explicit, fallible copy before
/// that model is dropped.
fn copy_bigint_bytes(bytes: &[u8]) -> Result<Box<[u8]>, ScalarScriptReadError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len()).map_err(|_| {
        ScalarScriptReadError::Internal("could not allocate the scalar BigInt draft".into())
    })?;
    copy.extend_from_slice(bytes);
    Ok(copy.into_boxed_slice())
}

/// Decode the release-pinned scalar sequence without separating a constant-
/// pool operand from the pool entry that admission authenticates.
fn decode_scalar_sequence(bytes: &[u8]) -> Option<ScalarSequence> {
    match bytes {
        [OP_PUSH_CONST8, index, OP_NEG, OP_SET_LOC0, OP_RETURN] => {
            Some(ScalarSequence::NegatedBigInt {
                push: BigIntPush::Constant(u32::from(*index)),
                width: 2,
            })
        }
        [
            OP_PUSH_CONST,
            byte_0,
            byte_1,
            byte_2,
            byte_3,
            OP_NEG,
            OP_SET_LOC0,
            OP_RETURN,
        ] => Some(ScalarSequence::NegatedBigInt {
            push: BigIntPush::Constant(u32::from_le_bytes([*byte_0, *byte_1, *byte_2, *byte_3])),
            width: 5,
        }),
        [
            OP_PUSH_BIGINT_I32,
            byte_0,
            byte_1,
            byte_2,
            byte_3,
            OP_NEG,
            OP_SET_LOC0,
            OP_RETURN,
        ] => Some(ScalarSequence::NegatedBigInt {
            push: BigIntPush::DirectI32(i32::from_le_bytes([*byte_0, *byte_1, *byte_2, *byte_3])),
            width: 5,
        }),
        _ => decode_scalar_push(bytes).map(|(push, width)| ScalarSequence::Plain { push, width }),
    }
}

/// Decode a release-pinned scalar push without a following unary operation.
fn decode_scalar_push(bytes: &[u8]) -> Option<(ScalarPush, u32)> {
    match bytes {
        [OP_PUSH_ATOM_VALUE, _, _, _, _, OP_SET_LOC0, OP_RETURN] => {
            Some((ScalarPush::AtomValue, 5))
        }
        [OP_PUSH_CONST8, index, OP_SET_LOC0, OP_RETURN] => {
            Some((ScalarPush::Constant(u32::from(*index)), 2))
        }
        [
            OP_PUSH_CONST,
            byte_0,
            byte_1,
            byte_2,
            byte_3,
            OP_SET_LOC0,
            OP_RETURN,
        ] => Some((
            ScalarPush::Constant(u32::from_le_bytes([*byte_0, *byte_1, *byte_2, *byte_3])),
            5,
        )),
        _ => decode_direct_scalar(bytes).map(|(draft, width)| (ScalarPush::Direct(draft), width)),
    }
}

/// Decode the release-pinned atom-free scalar opcode cohort.
fn decode_direct_scalar(bytes: &[u8]) -> Option<(ScalarScriptDraft, u32)> {
    match bytes {
        [OP_UNDEFINED, OP_SET_LOC0, OP_RETURN] => Some((ScalarScriptDraft::Undefined, 1)),
        [OP_NULL, OP_SET_LOC0, OP_RETURN] => Some((ScalarScriptDraft::Null, 1)),
        [OP_PUSH_FALSE, OP_SET_LOC0, OP_RETURN] => Some((ScalarScriptDraft::Bool(false), 1)),
        [OP_PUSH_TRUE, OP_SET_LOC0, OP_RETURN] => Some((ScalarScriptDraft::Bool(true), 1)),
        [
            OP_PUSH_BIGINT_I32,
            byte_0,
            byte_1,
            byte_2,
            byte_3,
            OP_SET_LOC0,
            OP_RETURN,
        ] => Some((
            ScalarScriptDraft::BigIntI32(i32::from_le_bytes([*byte_0, *byte_1, *byte_2, *byte_3])),
            5,
        )),
        [OP_PUSH_EMPTY_STRING, OP_SET_LOC0, OP_RETURN] => Some((ScalarScriptDraft::EmptyString, 1)),
        _ => {
            decode_direct_int32(bytes).map(|(value, width)| (ScalarScriptDraft::Int(value), width))
        }
    }
}

/// Decode the complete release-pinned direct Int32 opcode family.
///
/// QuickJS canonicalizes source literals to the shortest spelling, but its
/// bytecode reader also accepts wider spellings for the same value. Admission
/// therefore keys on semantic width, not compiler canonicality.
fn decode_direct_int32(bytes: &[u8]) -> Option<(i32, u32)> {
    match bytes {
        [opcode, OP_SET_LOC0, OP_RETURN] if (OP_PUSH_MINUS1..=OP_PUSH_7).contains(opcode) => {
            Some((i32::from(*opcode) - i32::from(OP_PUSH_0), 1))
        }
        [OP_PUSH_I8, immediate, OP_SET_LOC0, OP_RETURN] => Some((i32::from(*immediate as i8), 2)),
        [OP_PUSH_I16, low, high, OP_SET_LOC0, OP_RETURN] => {
            Some((i32::from(i16::from_le_bytes([*low, *high])), 3))
        }
        [
            OP_PUSH_I32,
            byte_0,
            byte_1,
            byte_2,
            byte_3,
            OP_SET_LOC0,
            OP_RETURN,
        ] => Some((i32::from_le_bytes([*byte_0, *byte_1, *byte_2, *byte_3]), 5)),
        _ => None,
    }
}

fn unadmitted<T>(message: &str) -> Result<T, ScalarScriptReadError> {
    Err(ScalarScriptReadError::Unadmitted(message.into()))
}

fn classify_image_error(error: BytecodeImageError) -> ScalarScriptReadError {
    let message = error.to_string();
    match error {
        BytecodeImageError::Wire(error) => classify_wire_error(error),
        BytecodeImageError::Atom(error) => classify_atom_error(error),
        BytecodeImageError::Data(error) => classify_data_error(error),
        BytecodeImageError::Envelope(error) => classify_envelope_error(error),
        BytecodeImageError::Module(_) | BytecodeImageError::ResourceLimit { .. } => {
            ScalarScriptReadError::Resource(message)
        }
        BytecodeImageError::CountOverflow { .. } => ScalarScriptReadError::Resource(message),
        BytecodeImageError::InvalidCompletionTarget
        | BytecodeImageError::InvalidFunctionState { .. }
        | BytecodeImageError::InvalidModuleState { .. }
        | BytecodeImageError::AllocationFailed => ScalarScriptReadError::Internal(message),
        BytecodeImageError::OffsetOverflow { .. } => ScalarScriptReadError::Malformed(message),
        BytecodeImageError::ModuleCountOutOfRange { .. } => {
            ScalarScriptReadError::JsInternal("out of memory".into())
        }
        BytecodeImageError::ModuleFieldOutOfRange { .. } => {
            ScalarScriptReadError::Unadmitted(message)
        }
    }
}

fn classify_atom_error(error: ImageAtomError) -> ScalarScriptReadError {
    let message = error.to_string();
    match error {
        ImageAtomError::Wire(error) => classify_wire_error(error),
        ImageAtomError::DynamicAtomCountOverflow { .. } => ScalarScriptReadError::Resource(message),
        ImageAtomError::AtomIndexSpaceMismatch { .. }
        | ImageAtomError::ForeignHeaderSlot { .. }
        | ImageAtomError::NullPropertyKey { .. } => ScalarScriptReadError::Malformed(message),
    }
}

fn classify_wire_error(error: WireError) -> ScalarScriptReadError {
    let message = error.to_string();
    match error {
        WireError::ResourceLimit { .. } => ScalarScriptReadError::Resource(message),
        WireError::AllocationFailed => ScalarScriptReadError::Internal(message),
        WireError::Truncated { .. } | WireError::MalformedUleb128 { .. } => {
            ScalarScriptReadError::Malformed("read after the end of the buffer".into())
        }
        WireError::InvalidAtomIndex { offset, .. } => {
            ScalarScriptReadError::Malformed(format!("invalid atom index (pos={offset})"))
        }
        WireError::StringTooLong { .. } => {
            ScalarScriptReadError::JsInternal("string too long".into())
        }
        _ => ScalarScriptReadError::Malformed(message),
    }
}

fn classify_data_error(
    error: DecodeError<super::bytecode_image::ImageOpaque>,
) -> ScalarScriptReadError {
    let message = error.to_string();
    match error {
        // The pinned C oracle covers all three cases with a fully decoded
        // FunctionBytecode child. Preserve QuickJS's public error class and
        // wording instead of collapsing these semantic rejections into the
        // reader's generic malformed-input class.
        DecodeError::OpaqueObjectValue { .. } => {
            ScalarScriptReadError::Type("cannot convert to object".into())
        }
        DecodeError::OpaqueDateValue { .. } => {
            ScalarScriptReadError::Type("Number tag expected for date".into())
        }
        DecodeError::OpaqueTypedArrayBacking { .. } => {
            ScalarScriptReadError::Type("ArrayBuffer object expected".into())
        }
        DecodeError::InvalidArrayBuffer { reason, .. }
        | DecodeError::InvalidSharedArrayBuffer { reason, .. } => match reason {
            ArrayBufferLayoutError::MaximumTooSmall { .. } => {
                ScalarScriptReadError::Type("invalid array buffer".into())
            }
            ArrayBufferLayoutError::ByteLengthTooLarge { .. } => {
                ScalarScriptReadError::Range("invalid array buffer length".into())
            }
            ArrayBufferLayoutError::MaximumTooLarge { .. } => {
                ScalarScriptReadError::Range("invalid max array buffer length".into())
            }
        },
        DecodeError::InvalidTypedArrayKind { .. } => {
            ScalarScriptReadError::Type("invalid typed array".into())
        }
        DecodeError::InvalidTypedArrayBacking { .. } => {
            ScalarScriptReadError::Type("ArrayBuffer object expected".into())
        }
        DecodeError::InvalidTypedArray { reason, .. } => match reason {
            TypedArrayLayoutError::UnalignedByteOffset { .. } => {
                ScalarScriptReadError::Range("invalid offset".into())
            }
            TypedArrayLayoutError::ViewOutOfBounds { .. } => {
                ScalarScriptReadError::Range("invalid length".into())
            }
        },
        DecodeError::InvalidObjectValue { .. } => {
            ScalarScriptReadError::Type("cannot convert to object".into())
        }
        DecodeError::InvalidDate { .. } => {
            ScalarScriptReadError::Type("Number tag expected for date".into())
        }
        DecodeError::ObjectReferencesNotAllowed { .. }
        | DecodeError::SharedArrayBuffersNotAllowed { .. }
        | DecodeError::SharedArrayBufferArchive(_)
        | DecodeError::UnsupportedTag { .. }
        | DecodeError::InvalidObjectValueAlias { .. } => ScalarScriptReadError::Unadmitted(message),
        DecodeError::Wire(error) => classify_wire_error(error),
        DecodeError::Graph(GraphError::ResourceLimit { .. })
        | DecodeError::Graph(GraphError::CountOverflow { .. })
        | DecodeError::AtomCountOverflow { .. } => ScalarScriptReadError::Resource(message),
        DecodeError::Graph(GraphError::AllocationFailed)
        | DecodeError::MachineIdExhausted
        | DecodeError::InvalidCompletionTarget
        | DecodeError::InvalidNodeState { .. } => ScalarScriptReadError::Internal(message),
        DecodeError::Graph(
            GraphError::InvalidAtomIndex { .. } | GraphError::InvalidNodeIndex { .. },
        )
        | DecodeError::NullPropertyKey { .. }
        | DecodeError::NonCanonicalBigInt { .. } => ScalarScriptReadError::Malformed(message),
        DecodeError::Graph(GraphError::InvalidReferenceIndex {
            index,
            reference_count,
        }) => ScalarScriptReadError::Malformed(format!(
            "invalid object reference ({index} >= {reference_count})"
        )),
    }
}

fn classify_envelope_error(error: FunctionEnvelopeError) -> ScalarScriptReadError {
    let message = error.to_string();
    match error {
        FunctionEnvelopeError::Wire(error) => classify_wire_error(error),
        FunctionEnvelopeError::Code(error) => classify_code_error(error),
        // Compatible-mode positive-int reads preserve QuickJS's u32-to-int32
        // assignment. Any high-bit count makes the native allocation-size
        // computation exceed INT32_MAX and reaches JS_ThrowOutOfMemory.
        FunctionEnvelopeError::FieldOutOfRange { .. } => {
            ScalarScriptReadError::JsInternal("out of memory".into())
        }
        FunctionEnvelopeError::ResourceLimit { .. }
        | FunctionEnvelopeError::CountOverflow { .. } => ScalarScriptReadError::Resource(message),
        FunctionEnvelopeError::AllocationFailed
        | FunctionEnvelopeError::InvalidAtomMode { .. }
        | FunctionEnvelopeError::InvalidModelBits { .. }
        | FunctionEnvelopeError::InvalidModelAtom { .. }
        | FunctionEnvelopeError::MismatchedAtomSpace { .. } => {
            ScalarScriptReadError::Internal(message)
        }
        _ => ScalarScriptReadError::Malformed(message),
    }
}

fn classify_code_error(error: CodeError) -> ScalarScriptReadError {
    let message = error.to_string();
    match error {
        CodeError::ResourceLimit { .. } | CodeError::CountOverflow { .. } => {
            ScalarScriptReadError::Resource(message)
        }
        CodeError::AllocationFailed
        | CodeError::InvalidAtomMode { .. }
        | CodeError::InvalidOpcodeLayout { .. }
        | CodeError::AtomCodecInvariant
        | CodeError::InvalidSidecar { .. } => ScalarScriptReadError::Internal(message),
        _ => ScalarScriptReadError::Malformed(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RETURN_42: [u8; 25] = [
        0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
    ];

    #[test]
    fn admits_the_complete_direct_int32_opcode_family_without_canonicality_checks() {
        for opcode in OP_PUSH_MINUS1..=OP_PUSH_7 {
            let object = scalar_with_code(&[opcode, OP_SET_LOC0, OP_RETURN]);
            assert_eq!(
                decode_trusted_scalar_script(&object),
                Ok(ScalarScriptDraft::Int(
                    i32::from(opcode) - i32::from(OP_PUSH_0)
                ))
            );
        }

        let cases: &[(&[u8], i32)] = &[
            (&[OP_PUSH_I8, 0x80, OP_SET_LOC0, OP_RETURN], -128),
            (&[OP_PUSH_I8, 0x7f, OP_SET_LOC0, OP_RETURN], 127),
            (&[OP_PUSH_I16, 0x7f, 0xff, OP_SET_LOC0, OP_RETURN], -129),
            (&[OP_PUSH_I16, 0x00, 0x80, OP_SET_LOC0, OP_RETURN], -32_768),
            (&[OP_PUSH_I16, 0xff, 0x7f, OP_SET_LOC0, OP_RETURN], 32_767),
            // QuickJS's reader accepts non-canonical wider encodings.
            (&[OP_PUSH_I16, 0x01, 0x00, OP_SET_LOC0, OP_RETURN], 1),
            (
                &[OP_PUSH_I32, 0xff, 0x7f, 0xff, 0xff, OP_SET_LOC0, OP_RETURN],
                -32_769,
            ),
            (
                &[OP_PUSH_I32, 0x00, 0x80, 0x00, 0x00, OP_SET_LOC0, OP_RETURN],
                32_768,
            ),
            (
                &[OP_PUSH_I32, 0xff, 0xff, 0xff, 0x7f, OP_SET_LOC0, OP_RETURN],
                i32::MAX,
            ),
            (
                &[OP_PUSH_I32, 0x00, 0x00, 0x00, 0x80, OP_SET_LOC0, OP_RETURN],
                i32::MIN,
            ),
            (
                &[OP_PUSH_I32, 0x01, 0x00, 0x00, 0x00, OP_SET_LOC0, OP_RETURN],
                1,
            ),
        ];
        for (code, expected) in cases {
            assert_eq!(
                decode_trusted_scalar_script(&scalar_with_code(code)),
                Ok(ScalarScriptDraft::Int(*expected))
            );
        }
    }

    #[test]
    fn admits_the_direct_atom_free_scalar_primitive_cohort() {
        let cases: &[(&[u8], ScalarScriptDraft)] = &[
            (
                &[OP_UNDEFINED, OP_SET_LOC0, OP_RETURN],
                ScalarScriptDraft::Undefined,
            ),
            (&[OP_NULL, OP_SET_LOC0, OP_RETURN], ScalarScriptDraft::Null),
            (
                &[OP_PUSH_FALSE, OP_SET_LOC0, OP_RETURN],
                ScalarScriptDraft::Bool(false),
            ),
            (
                &[OP_PUSH_TRUE, OP_SET_LOC0, OP_RETURN],
                ScalarScriptDraft::Bool(true),
            ),
            (
                &[
                    OP_PUSH_BIGINT_I32,
                    0x00,
                    0x00,
                    0x00,
                    0x00,
                    OP_SET_LOC0,
                    OP_RETURN,
                ],
                ScalarScriptDraft::BigIntI32(0),
            ),
            (
                &[
                    OP_PUSH_BIGINT_I32,
                    0xff,
                    0xff,
                    0xff,
                    0xff,
                    OP_SET_LOC0,
                    OP_RETURN,
                ],
                ScalarScriptDraft::BigIntI32(-1),
            ),
            (
                &[
                    OP_PUSH_BIGINT_I32,
                    0xff,
                    0xff,
                    0xff,
                    0x7f,
                    OP_SET_LOC0,
                    OP_RETURN,
                ],
                ScalarScriptDraft::BigIntI32(i32::MAX),
            ),
            (
                &[
                    OP_PUSH_BIGINT_I32,
                    0x01,
                    0x00,
                    0x00,
                    0x80,
                    OP_SET_LOC0,
                    OP_RETURN,
                ],
                ScalarScriptDraft::BigIntI32(-2_147_483_647),
            ),
            (
                &[
                    OP_PUSH_BIGINT_I32,
                    0x00,
                    0x00,
                    0x00,
                    0x80,
                    OP_SET_LOC0,
                    OP_RETURN,
                ],
                ScalarScriptDraft::BigIntI32(i32::MIN),
            ),
            (
                &[OP_PUSH_EMPTY_STRING, OP_SET_LOC0, OP_RETURN],
                ScalarScriptDraft::EmptyString,
            ),
        ];

        for (code, expected) in cases {
            assert_eq!(
                decode_trusted_scalar_script(&scalar_with_code(code)),
                Ok(expected.clone())
            );
        }
    }

    #[test]
    fn admits_the_complete_single_string_scalar_cohort() {
        const SHORT_INDEX_ZERO: &[u8] = &[OP_PUSH_CONST8, 0, OP_SET_LOC0, OP_RETURN];
        const WIDE_INDEX_ZERO: &[u8] = &[OP_PUSH_CONST, 0, 0, 0, 0, OP_SET_LOC0, OP_RETURN];

        let constant_cases: &[(bool, &[u16])] = &[
            (false, &[]),
            (false, &[u16::from(b'a')]),
            (false, &[0]),
            (false, &[0x00e9]),
            (true, &[0x0100]),
            (true, &[0xd83d, 0xde00]),
            (true, &[0xd800]),
            // Wide storage containing only Latin-1 remains a valid input even
            // though the runtime-independent DTO keeps only UTF-16 semantics.
            (true, &[u16::from(b'a')]),
        ];
        for &(wide, units) in constant_cases {
            let object = scalar_with_string_constant(SHORT_INDEX_ZERO, units, wide);
            assert_eq!(
                decode_trusted_scalar_script(&object),
                Ok(ScalarScriptDraft::ConstantString(ScalarStringDraft(
                    Box::from(units)
                )))
            );
        }
        assert_eq!(
            decode_trusted_scalar_script(&scalar_with_string_constant(
                WIDE_INDEX_ZERO,
                &[u16::from(b'4'), u16::from(b'2')],
                false,
            )),
            Ok(ScalarScriptDraft::ConstantString(ScalarStringDraft(
                Box::from([u16::from(b'4'), u16::from(b'2')])
            )))
        );

        let direct_atom_cases: &[(u32, ScalarScriptDraft)] = &[
            (
                47,
                ScalarScriptDraft::AtomString(ScalarStringDraft(Box::from([]))),
            ),
            (
                50,
                ScalarScriptDraft::AtomString(ScalarStringDraft(Box::from(
                    "length".encode_utf16().collect::<Vec<_>>(),
                ))),
            ),
            (0x8000_0000, ScalarScriptDraft::IntegerAtomString(0)),
            (0x8000_002a, ScalarScriptDraft::IntegerAtomString(42)),
            (
                0xffff_ffff,
                ScalarScriptDraft::IntegerAtomString(i32::MAX as u32),
            ),
        ];
        for (atom, expected) in direct_atom_cases {
            assert_eq!(
                decode_trusted_scalar_script(&scalar_with_atom_value(*atom)),
                Ok(expected.clone())
            );
        }

        let slot_cases: &[(&[u16], bool, ScalarScriptDraft)] = &[
            (
                &[u16::from(b'a')],
                false,
                ScalarScriptDraft::AtomString(ScalarStringDraft(Box::from([u16::from(b'a')]))),
            ),
            (
                &[
                    u16::from(b'l'),
                    u16::from(b'e'),
                    u16::from(b'n'),
                    u16::from(b'g'),
                    u16::from(b't'),
                    u16::from(b'h'),
                ],
                false,
                ScalarScriptDraft::AtomString(ScalarStringDraft(Box::from(
                    "length".encode_utf16().collect::<Vec<_>>(),
                ))),
            ),
            (
                &[u16::from(b'4'), u16::from(b'2')],
                false,
                ScalarScriptDraft::IntegerAtomString(42),
            ),
            (
                &[0x0100, 0, 0xd800],
                true,
                ScalarScriptDraft::AtomString(ScalarStringDraft(Box::from([0x0100, 0, 0xd800]))),
            ),
        ];
        for (units, wide, expected) in slot_cases {
            assert_eq!(
                decode_trusted_scalar_script(&scalar_with_atom_slot(units, *wide)),
                Ok(expected.clone())
            );
        }
    }

    #[test]
    fn string_scalar_frontier_preserves_atom_and_pool_provenance() {
        const SHORT_INDEX_ZERO: &[u8] = &[OP_PUSH_CONST8, 0, OP_SET_LOC0, OP_RETURN];
        let private = scalar_with_atom_value(229);
        let symbol = scalar_with_atom_value(230);
        let null = scalar_with_atom_value(0);
        let unused_slot = scalar_with_unused_atom_slot(50, &[u16::from(b'x')], false);
        let two_slots = scalar_with_two_atom_slots();
        for object in [private, symbol, null, unused_slot, two_slots] {
            assert!(matches!(
                decode_trusted_scalar_script(&object),
                Err(ScalarScriptReadError::Unadmitted(_))
            ));
        }

        let string_entry = string_constant_entry(&[u16::from(b'x')], false);
        let other_string_entry = string_constant_entry(&[u16::from(b'y')], false);
        let invalid_pool_shapes = [
            scalar_with_constants(
                &[OP_PUSH_CONST8, 1, OP_SET_LOC0, OP_RETURN],
                &[&string_entry],
            ),
            scalar_with_constants(SHORT_INDEX_ZERO, &[&string_entry, &other_string_entry]),
            scalar_with_constants(
                &[OP_PUSH_ATOM_VALUE, 50, 0, 0, 0, OP_SET_LOC0, OP_RETURN],
                &[&string_entry],
            ),
        ];
        for object in invalid_pool_shapes {
            assert!(matches!(
                decode_trusted_scalar_script(&object),
                Err(ScalarScriptReadError::Unadmitted(_))
            ));
        }
    }

    #[test]
    fn admits_only_exactly_paired_float64_constants() {
        const SHORT_INDEX_ZERO: &[u8] = &[OP_PUSH_CONST8, 0, OP_SET_LOC0, OP_RETURN];
        const WIDE_INDEX_ZERO: &[u8] = &[OP_PUSH_CONST, 0, 0, 0, 0, OP_SET_LOC0, OP_RETURN];
        let bits_cases = [
            0.5_f64.to_bits(),
            2_147_483_648_f64.to_bits(),
            1,
            f64::MAX.to_bits(),
            f64::INFINITY.to_bits(),
            f64::NEG_INFINITY.to_bits(),
            0.0_f64.to_bits(),
            (-0.0_f64).to_bits(),
            42.0_f64.to_bits(),
            0x7ff8_0000_0000_0042,
            0x7ff0_0000_0000_0042,
        ];

        for bits in bits_cases {
            assert_eq!(
                decode_trusted_scalar_script(&scalar_with_float_constant(SHORT_INDEX_ZERO, bits,)),
                Ok(ScalarScriptDraft::Float64Bits(bits))
            );
        }
        assert_eq!(
            decode_trusted_scalar_script(&scalar_with_float_constant(
                WIDE_INDEX_ZERO,
                0.5_f64.to_bits(),
            )),
            Ok(ScalarScriptDraft::Float64Bits(0.5_f64.to_bits()))
        );

        let float_entry = float_constant_entry(0.5_f64.to_bits());
        let other_float_entry = float_constant_entry(1.5_f64.to_bits());
        let invalid_pairs = [
            scalar_with_code(SHORT_INDEX_ZERO),
            scalar_with_constants(&[OP_PUSH_0, OP_SET_LOC0, OP_RETURN], &[&float_entry]),
            scalar_with_constants(
                &[OP_PUSH_CONST8, 1, OP_SET_LOC0, OP_RETURN],
                &[&float_entry],
            ),
            scalar_with_constants(
                &[OP_PUSH_CONST, 1, 0, 0, 0, OP_SET_LOC0, OP_RETURN],
                &[&float_entry],
            ),
            scalar_with_constants(SHORT_INDEX_ZERO, &[&float_entry, &other_float_entry]),
            scalar_with_constants(SHORT_INDEX_ZERO, &[&[0x05, 0x54]]),
        ];
        for object in invalid_pairs {
            assert!(matches!(
                decode_trusted_scalar_script(&object),
                Err(ScalarScriptReadError::Unadmitted(_))
            ));
        }

        let mut truncated_float = scalar_with_code(SHORT_INDEX_ZERO);
        truncated_float[14] = 1;
        truncated_float.extend_from_slice(&[0x06, 0, 0, 0, 0, 0, 0, 0]);
        assert!(matches!(
            decode_trusted_scalar_script(&truncated_float),
            Err(ScalarScriptReadError::Malformed(_))
        ));
    }

    #[test]
    fn admits_only_exactly_paired_normalized_bigint_constants() {
        const SHORT_INDEX_ZERO: &[u8] = &[OP_PUSH_CONST8, 0, OP_SET_LOC0, OP_RETURN];
        const WIDE_INDEX_ZERO: &[u8] = &[OP_PUSH_CONST, 0, 0, 0, 0, OP_SET_LOC0, OP_RETURN];
        let cases: &[(&[u8], &[u8])] = &[
            (&[], &[]),
            (&[0x01], &[0x01]),
            (&[0xff], &[0xff]),
            (
                &[0x00, 0x00, 0x00, 0x80, 0x00],
                &[0x00, 0x00, 0x00, 0x80, 0x00],
            ),
            (
                &[0xff, 0xff, 0xff, 0x7f, 0xff],
                &[0xff, 0xff, 0xff, 0x7f, 0xff],
            ),
            (
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
            ),
            (
                &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00],
                &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00],
            ),
            (
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff],
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xff],
            ),
            (
                &[
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x01,
                ],
                &[
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x01,
                ],
            ),
            // QuickJS-compatible mode accepts redundant sign extension and
            // the graph decoder normalizes it before scalar admission.
            (&[0x00], &[]),
            (&[0x01, 0x00], &[0x01]),
            (&[0xff, 0xff], &[0xff]),
        ];

        for (payload, expected) in cases {
            assert_eq!(
                decode_trusted_scalar_script(&scalar_with_bigint_constant(
                    SHORT_INDEX_ZERO,
                    payload,
                )),
                Ok(ScalarScriptDraft::BigIntBytes(Box::from(*expected)))
            );
        }
        assert_eq!(
            decode_trusted_scalar_script(&scalar_with_bigint_constant(
                WIDE_INDEX_ZERO,
                &[0x00, 0x00, 0x00, 0x80, 0x00],
            )),
            Ok(ScalarScriptDraft::BigIntBytes(Box::from([
                0x00, 0x00, 0x00, 0x80, 0x00,
            ])))
        );

        let bigint_entry = bigint_constant_entry(&[0x00, 0x00, 0x00, 0x80, 0x00]);
        let other_bigint_entry = bigint_constant_entry(&[0x01]);
        let invalid_pairs = [
            scalar_with_constants(&[OP_PUSH_0, OP_SET_LOC0, OP_RETURN], &[&bigint_entry]),
            scalar_with_constants(
                &[OP_PUSH_CONST8, 1, OP_SET_LOC0, OP_RETURN],
                &[&bigint_entry],
            ),
            scalar_with_constants(
                &[OP_PUSH_CONST, 1, 0, 0, 0, OP_SET_LOC0, OP_RETURN],
                &[&bigint_entry],
            ),
            scalar_with_constants(SHORT_INDEX_ZERO, &[&bigint_entry, &other_bigint_entry]),
        ];
        for object in invalid_pairs {
            assert!(matches!(
                decode_trusted_scalar_script(&object),
                Err(ScalarScriptReadError::Unadmitted(_))
            ));
        }
    }

    #[test]
    fn admits_only_exactly_paired_single_negated_bigints() {
        const SHORT_INDEX_ZERO_NEG: &[u8] = &[OP_PUSH_CONST8, 0, OP_NEG, OP_SET_LOC0, OP_RETURN];
        const WIDE_INDEX_ZERO_NEG: &[u8] =
            &[OP_PUSH_CONST, 0, 0, 0, 0, OP_NEG, OP_SET_LOC0, OP_RETURN];
        let cases: &[(&[u8], &[u8])] = &[
            (&[], &[]),
            (&[0x01], &[0x01]),
            (&[0xff], &[0xff]),
            (
                &[0x00, 0x00, 0x00, 0x80, 0x00],
                &[0x00, 0x00, 0x00, 0x80, 0x00],
            ),
            (
                &[0xff, 0xff, 0xff, 0x7f, 0xff],
                &[0xff, 0xff, 0xff, 0x7f, 0xff],
            ),
            (
                &[
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x01,
                ],
                &[
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x01,
                ],
            ),
            // Compatible mode normalizes redundant sign extension before the
            // draft crosses the archive/runtime boundary.
            (&[0x00], &[]),
            (&[0x01, 0x00], &[0x01]),
            (&[0xff, 0xff], &[0xff]),
        ];

        for (payload, expected) in cases {
            assert_eq!(
                decode_trusted_scalar_script(&scalar_with_bigint_constant(
                    SHORT_INDEX_ZERO_NEG,
                    payload,
                )),
                Ok(ScalarScriptDraft::NegatedBigIntBytes(Box::from(*expected)))
            );
        }
        assert_eq!(
            decode_trusted_scalar_script(&scalar_with_bigint_constant(
                WIDE_INDEX_ZERO_NEG,
                &[0x00, 0x00, 0x00, 0x80, 0x00],
            )),
            Ok(ScalarScriptDraft::NegatedBigIntBytes(Box::from([
                0x00, 0x00, 0x00, 0x80, 0x00,
            ])))
        );

        for value in [0_i32, 1, -1, i32::MAX, i32::MIN] {
            let mut code = vec![OP_PUSH_BIGINT_I32];
            code.extend_from_slice(&value.to_le_bytes());
            code.extend_from_slice(&[OP_NEG, OP_SET_LOC0, OP_RETURN]);
            assert_eq!(
                decode_trusted_scalar_script(&scalar_with_code(&code)),
                Ok(ScalarScriptDraft::NegatedBigIntI32(value))
            );
        }

        let bigint_entry = bigint_constant_entry(&[0x00, 0x00, 0x00, 0x80, 0x00]);
        let other_bigint_entry = bigint_constant_entry(&[0x01]);
        let float_entry = float_constant_entry(0.5_f64.to_bits());
        let invalid_shapes = [
            scalar_with_code(SHORT_INDEX_ZERO_NEG),
            scalar_with_constants(
                &[OP_PUSH_CONST8, 1, OP_NEG, OP_SET_LOC0, OP_RETURN],
                &[&bigint_entry],
            ),
            scalar_with_constants(SHORT_INDEX_ZERO_NEG, &[&bigint_entry, &other_bigint_entry]),
            scalar_with_constants(SHORT_INDEX_ZERO_NEG, &[&float_entry]),
            scalar_with_constants(SHORT_INDEX_ZERO_NEG, &[&[0x05, 0x54]]),
            scalar_with_constants(SHORT_INDEX_ZERO_NEG, &[&[0x07, 0x02, 0x78]]),
            scalar_with_constants(
                &[
                    OP_PUSH_BIGINT_I32,
                    1,
                    0,
                    0,
                    0,
                    OP_NEG,
                    OP_SET_LOC0,
                    OP_RETURN,
                ],
                &[&bigint_entry],
            ),
            scalar_with_code(&[OP_PUSH_0, OP_NEG, OP_SET_LOC0, OP_RETURN]),
            scalar_with_constants(
                &[OP_PUSH_CONST8, 0, OP_NEG, OP_NEG, OP_SET_LOC0, OP_RETURN],
                &[&bigint_entry],
            ),
        ];
        for object in invalid_shapes {
            assert!(matches!(
                decode_trusted_scalar_script(&object),
                Err(ScalarScriptReadError::Unadmitted(_))
            ));
        }
    }

    #[test]
    fn compatible_reader_accepts_non_minimal_fields_and_trailing_bytes() {
        let mut non_minimal = RETURN_42.to_vec();
        non_minimal.splice(8..9, [0x80, 0x00]);
        assert_eq!(
            decode_trusted_scalar_script(&non_minimal),
            Ok(ScalarScriptDraft::Int(42))
        );

        let mut trailing = RETURN_42.to_vec();
        trailing.extend_from_slice(&[0xde, 0xad]);
        assert_eq!(
            decode_trusted_scalar_script(&trailing),
            Ok(ScalarScriptDraft::Int(42))
        );
    }

    #[test]
    fn valid_but_unadmitted_roots_and_shapes_stay_unadmitted() {
        let primitive_root = [0x05, 0x00, 0x05, 0x54];
        assert!(matches!(
            decode_trusted_scalar_script(&primitive_root),
            Err(ScalarScriptReadError::Unadmitted(_))
        ));

        let mut unsupported_opcode = RETURN_42;
        unsupported_opcode[21] = 0xbd;
        assert!(matches!(
            decode_trusted_scalar_script(&unsupported_opcode),
            Err(ScalarScriptReadError::Unadmitted(_))
        ));

        assert!(matches!(
            decode_trusted_scalar_script(&scalar_with_code(&[0x08, OP_SET_LOC0, OP_RETURN])),
            Err(ScalarScriptReadError::Unadmitted(_))
        ));

        let mut unused_atom_slot = RETURN_42.to_vec();
        unused_atom_slot.splice(1..2, [0x01, 0x00]);
        assert!(matches!(
            decode_trusted_scalar_script(&unused_atom_slot),
            Err(ScalarScriptReadError::Unadmitted(_))
        ));

        let constant_pool_bigint_with_double_neg = [
            0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
            0x01, 0x06, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbd, 0x00, 0x8a, 0x8a, 0xcb, 0x28, 0x0a,
            0x05, 0x00, 0x00, 0x00, 0x80, 0x00,
        ];
        assert!(matches!(
            decode_trusted_scalar_script(&constant_pool_bigint_with_double_neg),
            Err(ScalarScriptReadError::Unadmitted(_))
        ));

        let mut unsupported_metadata = RETURN_42;
        unsupported_metadata[3] = 0x01;
        assert!(matches!(
            decode_trusted_scalar_script(&unsupported_metadata),
            Err(ScalarScriptReadError::Unadmitted(_))
        ));

        let mut non_eval_name = RETURN_42;
        // Pinned atom 85 (`<ret>`) has the same two-byte metadata spelling as
        // pinned atom 84 (`<eval>`), so this remains a valid BC5 image.
        non_eval_name[6] = 0xaa;
        assert!(matches!(
            decode_trusted_scalar_script(&non_eval_name),
            Err(ScalarScriptReadError::Unadmitted(_))
        ));

        let mut named_completion_local = RETURN_42;
        // Pinned atom 1 (`null`) is a valid one-byte metadata atom, but the
        // stripped completion local itself must carry atom zero.
        named_completion_local[17] = 0x02;
        assert!(matches!(
            decode_trusted_scalar_script(&named_completion_local),
            Err(ScalarScriptReadError::Unadmitted(_))
        ));

        let mut wrapping_scope_link = RETURN_42.to_vec();
        wrapping_scope_link.splice(18..19, [0x80, 0x80, 0x80, 0x80, 0x08]);
        assert!(matches!(
            decode_trusted_scalar_script(&wrapping_scope_link),
            Err(ScalarScriptReadError::Unadmitted(_))
        ));
    }

    #[test]
    fn preserves_authenticated_type_errors_for_invalid_data_parents() {
        let cases: [(&[u8], &str); 3] = [
            (
                &[
                    0x05, 0x00, 0x12, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01,
                    0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
                ],
                "cannot convert to object",
            ),
            (
                &[
                    0x05, 0x00, 0x11, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01,
                    0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
                ],
                "Number tag expected for date",
            ),
            (
                &[
                    0x05, 0x00, 0x0e, 0x02, 0x01, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00,
                    0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb,
                    0x2a, 0xcb, 0x28,
                ],
                "ArrayBuffer object expected",
            ),
        ];

        for (object, expected) in cases {
            assert_eq!(
                decode_trusted_scalar_script(object),
                Err(ScalarScriptReadError::Type(expected.into()))
            );
        }
    }

    #[test]
    fn preserves_release_pinned_data_reader_error_classes() {
        let cases: [(&[u8], ScalarScriptReadError); 7] = [
            (
                &[0x05, 0x00, 0x0f, 0x01, 0x00],
                ScalarScriptReadError::Type("invalid array buffer".into()),
            ),
            (
                &[0x05, 0x00, 0x0e, 0xff],
                ScalarScriptReadError::Type("invalid typed array".into()),
            ),
            (
                &[0x05, 0x00, 0x0e, 0x02, 0x00, 0x00, 0x01],
                ScalarScriptReadError::Type("ArrayBuffer object expected".into()),
            ),
            (
                &[0x05, 0x00, 0x12, 0x01],
                ScalarScriptReadError::Type("cannot convert to object".into()),
            ),
            (
                &[0x05, 0x00, 0x11, 0x01],
                ScalarScriptReadError::Type("Number tag expected for date".into()),
            ),
            (
                &[0x05, 0x00, 0x0e, 0x04, 0x00, 0x01, 0x0f, 0x00, 0x00],
                ScalarScriptReadError::Range("invalid offset".into()),
            ),
            (
                &[0x05, 0x00, 0x0e, 0x02, 0x01, 0x01, 0x0f, 0x01, 0x01, 0x00],
                ScalarScriptReadError::Range("invalid length".into()),
            ),
        ];

        for (object, expected) in cases {
            assert_eq!(decode_trusted_scalar_script(object), Err(expected));
        }
    }

    #[test]
    fn malformed_and_resource_failures_remain_distinct() {
        assert!(matches!(
            decode_trusted_scalar_script(&scalar_with_code(&[
                OP_PUSH_BIGINT_I32,
                0x00,
                0x00,
                0x00,
            ])),
            Err(ScalarScriptReadError::Malformed(_))
        ));
        assert_eq!(
            decode_trusted_scalar_script(&RETURN_42[..RETURN_42.len() - 1]),
            Err(ScalarScriptReadError::Malformed(
                "read after the end of the buffer".into()
            ))
        );
        assert_eq!(
            decode_trusted_scalar_script(&[0x05, 0x80, 0x80, 0x80, 0x80, 0x80]),
            Err(ScalarScriptReadError::Malformed(
                "read after the end of the buffer".into()
            ))
        );
        let mut invalid_atom = RETURN_42.to_vec();
        invalid_atom.splice(6..8, [0xe6, 0x03]);
        assert_eq!(
            decode_trusted_scalar_script(&invalid_atom),
            Err(ScalarScriptReadError::Malformed(
                "invalid atom index (pos=8)".into()
            ))
        );
        let mut negative_bytecode_length = RETURN_42.to_vec();
        negative_bytecode_length.splice(15..16, [0x80, 0x80, 0x80, 0x80, 0x08]);
        assert_eq!(
            decode_trusted_scalar_script(&negative_bytecode_length),
            Err(ScalarScriptReadError::JsInternal("out of memory".into()))
        );
        assert_eq!(
            decode_trusted_scalar_script(&[0x05, 0x01, 0x80, 0x80, 0x80, 0x80, 0x08]),
            Err(ScalarScriptReadError::JsInternal("string too long".into()))
        );
        assert_eq!(
            decode_trusted_scalar_script(&[0x05, 0x00, 0x13, 0x00]),
            Err(ScalarScriptReadError::Malformed(
                "invalid object reference (0 >= 0)".into()
            ))
        );

        let mut wrong_version = RETURN_42;
        wrong_version[0] = 4;
        assert!(matches!(
            decode_trusted_scalar_script(&wrong_version),
            Err(ScalarScriptReadError::Malformed(_))
        ));

        assert!(matches!(
            decode_trusted_scalar_script(&vec![0; MAX_INPUT_BYTES + 1]),
            Err(ScalarScriptReadError::Resource(_))
        ));
    }

    #[test]
    fn display_preserves_the_public_failure_class() {
        assert_eq!(
            ScalarScriptReadError::Malformed("bad".into()).to_string(),
            "malformed BC5 object: bad"
        );
        assert_eq!(
            ScalarScriptReadError::Type("bad value".into()).to_string(),
            "invalid BC5 value: bad value"
        );
        assert_eq!(
            ScalarScriptReadError::Range("bad range".into()).to_string(),
            "out-of-range BC5 value: bad range"
        );
        assert_eq!(
            ScalarScriptReadError::JsInternal("too long".into()).to_string(),
            "BC5 reader internal error: too long"
        );
        assert_eq!(
            ScalarScriptReadError::Unadmitted("shape".into()).to_string(),
            "BC5 object is not admitted as a trusted scalar script: shape"
        );
        assert_eq!(
            ScalarScriptReadError::Resource("budget".into()).to_string(),
            "BC5 scalar-script resource limit: budget"
        );
        assert_eq!(
            ScalarScriptReadError::Internal("invariant".into()).to_string(),
            "BC5 scalar-script internal failure: invariant"
        );
    }

    fn scalar_with_code(code: &[u8]) -> Vec<u8> {
        let mut object = RETURN_42.to_vec();
        object[15] = u8::try_from(code.len()).expect("test code length fits one-byte ULEB");
        object.splice(21.., code.iter().copied());
        object
    }

    fn scalar_with_float_constant(code: &[u8], bits: u64) -> Vec<u8> {
        let entry = float_constant_entry(bits);
        scalar_with_constants(code, &[&entry])
    }

    fn scalar_with_bigint_constant(code: &[u8], payload: &[u8]) -> Vec<u8> {
        let entry = bigint_constant_entry(payload);
        scalar_with_constants(code, &[&entry])
    }

    fn scalar_with_string_constant(code: &[u8], units: &[u16], wide: bool) -> Vec<u8> {
        let entry = string_constant_entry(units, wide);
        scalar_with_constants(code, &[&entry])
    }

    fn string_constant_entry(units: &[u16], wide: bool) -> Vec<u8> {
        let mut entry = vec![0x07];
        entry.extend(wire_string_entry(units, wide));
        entry
    }

    fn scalar_with_atom_value(atom: u32) -> Vec<u8> {
        let mut code = vec![OP_PUSH_ATOM_VALUE];
        code.extend_from_slice(&atom.to_le_bytes());
        code.extend_from_slice(&[OP_SET_LOC0, OP_RETURN]);
        scalar_with_code(&code)
    }

    fn scalar_with_atom_slot(units: &[u16], wide: bool) -> Vec<u8> {
        let mut object = scalar_with_atom_value(243);
        object[1] = 1;
        object.splice(2..2, wire_string_entry(units, wide));
        object
    }

    fn scalar_with_unused_atom_slot(atom: u32, units: &[u16], wide: bool) -> Vec<u8> {
        let mut object = scalar_with_atom_value(atom);
        object[1] = 1;
        object.splice(2..2, wire_string_entry(units, wide));
        object
    }

    fn scalar_with_two_atom_slots() -> Vec<u8> {
        let mut object = scalar_with_atom_value(243);
        object[1] = 2;
        let mut slots = wire_string_entry(&[u16::from(b'x')], false);
        slots.extend(wire_string_entry(&[u16::from(b'y')], false));
        object.splice(2..2, slots);
        object
    }

    fn wire_string_entry(units: &[u16], wide: bool) -> Vec<u8> {
        let length = u8::try_from(units.len()).expect("test String length fits one-byte ULEB");
        let mut entry = vec![(length << 1) | u8::from(wide)];
        if wide {
            for unit in units {
                entry.extend_from_slice(&unit.to_le_bytes());
            }
        } else {
            entry.extend(units.iter().map(|unit| {
                u8::try_from(*unit).expect("test narrow String contains only Latin-1")
            }));
        }
        entry
    }

    fn bigint_constant_entry(payload: &[u8]) -> Vec<u8> {
        let mut entry = vec![
            0x0a,
            u8::try_from(payload.len()).expect("test BigInt length fits one-byte ULEB"),
        ];
        entry.extend_from_slice(payload);
        entry
    }

    fn float_constant_entry(bits: u64) -> Vec<u8> {
        let mut entry = vec![0x06];
        entry.extend_from_slice(&bits.to_le_bytes());
        entry
    }

    fn scalar_with_constants(code: &[u8], constants: &[&[u8]]) -> Vec<u8> {
        let mut object = scalar_with_code(code);
        object[14] = u8::try_from(constants.len()).expect("test constant count fits one-byte ULEB");
        for constant in constants {
            object.extend_from_slice(constant);
        }
        object
    }
}
