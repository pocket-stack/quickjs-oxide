//! Canonical writer for an authenticated, non-executable [`FunctionImage`].
//!
//! The writer first builds a source-bound plan which validates traversal,
//! resource, atom, and code-sidecar invariants without exposing output. Only a
//! complete plan may allocate the final BC5 byte vector. Nothing in this
//! module materializes a runtime object or admits native QuickJS code to
//! execution.

mod emit;
mod plan;

use std::fmt;

use super::super::code::CodeError;
use super::super::function_envelope::FunctionEnvelopeError;
use super::super::graph::model::{
    ArrayBufferLayoutError, GraphError, NodeId, TypedArrayBackingError, TypedArrayLayoutError,
};
use super::super::graph::write_state::DataWriteStateError;
use super::super::wire::WireError;
use super::budget::{FunctionImageBudgetError, FunctionImageLimits};
use super::model::FunctionImage;

/// Explicit policy for one canonical whole-image write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FunctionImageEncodeOptions {
    allow_object_references: bool,
    max_output_bytes: usize,
    limits: FunctionImageLimits,
}

impl FunctionImageEncodeOptions {
    #[must_use]
    pub(in crate::runtime) const fn new(
        allow_object_references: bool,
        max_output_bytes: usize,
        limits: FunctionImageLimits,
    ) -> Self {
        Self {
            allow_object_references,
            max_output_bytes,
            limits,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum FunctionImageEncodeError {
    Wire(WireError),
    Graph(GraphError),
    Budget(FunctionImageBudgetError),
    Envelope(FunctionEnvelopeError),
    Code(CodeError),
    DynamicAtomOutOfRange {
        index: u32,
        atom_count: usize,
    },
    IntegerAtomOutOfRange {
        index: u32,
    },
    ForeignFunction {
        function_index: u32,
    },
    FunctionPreorder {
        expected: u32,
        found: u32,
    },
    MissingFunctions {
        reachable: usize,
        function_count: usize,
    },
    DuplicatePropertyKey {
        node: NodeId,
    },
    NonCanonicalBigInt,
    CircularReference {
        node: NodeId,
    },
    CircularFunction {
        function_index: u32,
    },
    InvalidArrayBuffer {
        node: NodeId,
        reason: ArrayBufferLayoutError,
    },
    InvalidTypedArrayBacking {
        node: NodeId,
        reason: TypedArrayBackingError,
    },
    InvalidTypedArray {
        node: NodeId,
        reason: TypedArrayLayoutError,
    },
    InvalidCodeSidecar {
        function_index: u32,
        offset: u32,
    },
    EncodedLengthMismatch {
        planned: usize,
        actual: usize,
    },
    EncodedLengthOverflow,
    AllocationFailed,
}

impl fmt::Display for FunctionImageEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => fmt::Display::fmt(error, formatter),
            Self::Graph(error) => fmt::Display::fmt(error, formatter),
            Self::Budget(error) => fmt::Display::fmt(error, formatter),
            Self::Envelope(error) => fmt::Display::fmt(error, formatter),
            Self::Code(error) => fmt::Display::fmt(error, formatter),
            Self::DynamicAtomOutOfRange { index, atom_count } => write!(
                formatter,
                "dynamic atom {index} does not belong to the image's {atom_count} atoms"
            ),
            Self::IntegerAtomOutOfRange { index } => write!(
                formatter,
                "integer atom {index} exceeds QuickJS's 31-bit tagged range"
            ),
            Self::ForeignFunction { function_index } => write!(
                formatter,
                "function {function_index} does not belong to this authenticated image"
            ),
            Self::FunctionPreorder { expected, found } => write!(
                formatter,
                "function preorder expected first occurrence {expected}, found {found}"
            ),
            Self::MissingFunctions {
                reachable,
                function_count,
            } => write!(
                formatter,
                "function image exposes {reachable} reachable records out of {function_count}"
            ),
            Self::DuplicatePropertyKey { node } => write!(
                formatter,
                "ordinary node {} contains a duplicate semantic property key",
                node.zero_based()
            ),
            Self::NonCanonicalBigInt => {
                formatter.write_str("function image contains a non-canonical BigInt payload")
            }
            Self::CircularReference { node } => write!(
                formatter,
                "function image contains a circular reference through node {}",
                node.zero_based()
            ),
            Self::CircularFunction { function_index } => write!(
                formatter,
                "function image contains a recursive record cycle through function {function_index}"
            ),
            Self::InvalidArrayBuffer { node, reason } => write!(
                formatter,
                "node {} contains an invalid ArrayBuffer layout: {reason}",
                node.zero_based()
            ),
            Self::InvalidTypedArrayBacking { node, reason } => write!(
                formatter,
                "node {} contains an invalid TypedArray backing: {reason}",
                node.zero_based()
            ),
            Self::InvalidTypedArray { node, reason } => write!(
                formatter,
                "node {} contains an invalid TypedArray layout: {reason}",
                node.zero_based()
            ),
            Self::InvalidCodeSidecar {
                function_index,
                offset,
            } => write!(
                formatter,
                "function {function_index} has an invalid code sidecar at byte {offset}"
            ),
            Self::EncodedLengthMismatch { planned, actual } => write!(
                formatter,
                "canonical function-image plan promised {planned} bytes but emitted {actual}"
            ),
            Self::EncodedLengthOverflow => {
                formatter.write_str("canonical function-image length overflowed")
            }
            Self::AllocationFailed => {
                formatter.write_str("canonical function-image writer allocation failed")
            }
        }
    }
}

impl std::error::Error for FunctionImageEncodeError {}

impl From<WireError> for FunctionImageEncodeError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<GraphError> for FunctionImageEncodeError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

impl From<DataWriteStateError> for FunctionImageEncodeError {
    fn from(error: DataWriteStateError) -> Self {
        match error {
            DataWriteStateError::Graph(error) => Self::Graph(error),
            DataWriteStateError::CircularReference { node } => Self::CircularReference { node },
            DataWriteStateError::AllocationFailed => Self::AllocationFailed,
        }
    }
}

impl From<FunctionImageBudgetError> for FunctionImageEncodeError {
    fn from(error: FunctionImageBudgetError) -> Self {
        Self::Budget(error)
    }
}

impl From<FunctionEnvelopeError> for FunctionImageEncodeError {
    fn from(error: FunctionEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<CodeError> for FunctionImageEncodeError {
    fn from(error: CodeError) -> Self {
        Self::Code(error)
    }
}

/// Canonically encode one complete, authenticated bytecode image.
pub(in crate::runtime) fn encode_function_image(
    image: &FunctionImage,
    options: FunctionImageEncodeOptions,
) -> Result<Vec<u8>, FunctionImageEncodeError> {
    emit::encode_authenticated(plan::authenticate_for_write(image, options)?)
}
