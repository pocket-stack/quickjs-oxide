//! Narrow semantic admission for trusted scalar-script BC5 objects.
//!
//! This layer always completes the release-pinned whole-image read before it
//! considers execution eligibility. A successful result is an inert scalar
//! draft, not verified bytecode and not a runtime or heap identity.

use std::fmt;

use super::bytecode_image::{
    BytecodeImage, BytecodeImageError, BytecodeImageLimits, ImageAtomError, ModuleLimits,
    decode_bytecode_image_body,
};
use super::code::{CodeError, CodeLimits};
use super::function_envelope::{FunctionEnvelopeError, FunctionEnvelopeLimits};
use super::graph::decode::DecodeError;
use super::graph::model::{ArrayBufferLayoutError, GraphError, GraphLimits, TypedArrayLayoutError};
use super::wire::{ReaderMode, WireCursor, WireError, WireLimits};

const MAX_INPUT_BYTES: usize = 4096;

const OP_RETURN: u8 = 0x28;
const OP_PUSH_I8: u8 = 0xbb;
const OP_SET_LOC0: u8 = 0xcb;

/// Runtime-independent result of the first executable BC5 admission cohort.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ScalarScriptDraft {
    Int(i32),
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

/// Decode one complete pinned-QuickJS object and admit the branch-free i8
/// script shape. Compatibility mode is semantic: pinned QuickJS accepts
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
    if !image.atoms().is_empty() {
        return unadmitted("dynamic atom table is not empty");
    }
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
    if !function.constants().is_empty() {
        return unadmitted("function constant pool is not empty");
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
    if !native_payload.atom_relocations().is_empty() {
        return unadmitted("native payload contains atom relocations");
    }
    let [push, set_completion, return_value] = native_payload.instructions() else {
        return unadmitted("native payload is not the three-instruction scalar shape");
    };
    if push.offset() != 0
        || push.opcode().raw() != OP_PUSH_I8
        || set_completion.offset() != 2
        || set_completion.opcode().raw() != OP_SET_LOC0
        || return_value.offset() != 3
        || return_value.opcode().raw() != OP_RETURN
    {
        return unadmitted("native payload opcode sequence is outside the admitted shape");
    }
    let [push_opcode, immediate, set_opcode, return_opcode] = native_payload.as_bytes() else {
        return unadmitted("native payload width is outside the admitted shape");
    };
    if (*push_opcode, *set_opcode, *return_opcode) != (OP_PUSH_I8, OP_SET_LOC0, OP_RETURN) {
        return Err(ScalarScriptReadError::Internal(
            "instruction sidecars disagree with their owned native bytes".into(),
        ));
    }

    Ok(ScalarScriptDraft::Int(i32::from(*immediate as i8)))
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
    fn admits_the_pinned_return_42_shape_without_special_casing_42() {
        for (raw, expected) in [(0x2a, 42), (0x29, 41), (0xff, -1)] {
            let mut object = RETURN_42;
            object[22] = raw;
            assert_eq!(
                decode_trusted_scalar_script(&object),
                Ok(ScalarScriptDraft::Int(expected))
            );
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
}
