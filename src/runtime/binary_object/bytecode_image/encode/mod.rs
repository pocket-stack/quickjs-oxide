//! Canonical writer for an authenticated, non-executable [`BytecodeImage`].
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
use super::budget::{BytecodeImageBudgetError, BytecodeImageLimits, ModuleBudgetError};
use super::model::BytecodeImage;

/// Explicit policy for one canonical whole-image write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct BytecodeImageEncodeOptions {
    allow_object_references: bool,
    max_output_bytes: usize,
    limits: BytecodeImageLimits,
}

impl BytecodeImageEncodeOptions {
    #[must_use]
    pub(in crate::runtime) const fn new(
        allow_object_references: bool,
        max_output_bytes: usize,
        limits: BytecodeImageLimits,
    ) -> Self {
        Self {
            allow_object_references,
            max_output_bytes,
            limits,
        }
    }
}

/// The positive QuickJS `int` slot occupied by one Module field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ModuleIntegerField {
    RequestCount,
    ExportCount,
    ExportVariableIndex,
    ExportRequestIndex,
    StarExportCount,
    StarExportRequestIndex,
    ImportCount,
    ImportVariableIndex,
    ImportRequestIndex,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum BytecodeImageEncodeError {
    Wire(WireError),
    Graph(GraphError),
    Budget(BytecodeImageBudgetError),
    Module(ModuleBudgetError),
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
    ForeignModule {
        module_index: u32,
    },
    FunctionPreorder {
        expected: u32,
        found: u32,
    },
    ModulePreorder {
        expected: u32,
        found: u32,
    },
    MissingFunctions {
        reachable: usize,
        function_count: usize,
    },
    MissingModules {
        reachable: usize,
        module_count: usize,
    },
    DuplicatePropertyKey {
        node: NodeId,
    },
    NonCanonicalBigInt,
    ArchivedBackingContextRequired {
        node: NodeId,
    },
    CircularReference {
        node: NodeId,
    },
    CircularFunction {
        function_index: u32,
    },
    CircularModule {
        module_index: u32,
    },
    ModuleIntegerOutOfRange {
        module_index: u32,
        field: ModuleIntegerField,
        value: u64,
    },
    InvalidModuleExport {
        module_index: u32,
        export_index: usize,
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

impl fmt::Display for BytecodeImageEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => fmt::Display::fmt(error, formatter),
            Self::Graph(error) => fmt::Display::fmt(error, formatter),
            Self::Budget(error) => fmt::Display::fmt(error, formatter),
            Self::Module(error) => fmt::Display::fmt(error, formatter),
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
            Self::ForeignModule { module_index } => write!(
                formatter,
                "module {module_index} does not belong to this authenticated image"
            ),
            Self::FunctionPreorder { expected, found } => write!(
                formatter,
                "function preorder expected first occurrence {expected}, found {found}"
            ),
            Self::ModulePreorder { expected, found } => write!(
                formatter,
                "module preorder expected first occurrence {expected}, found {found}"
            ),
            Self::MissingFunctions {
                reachable,
                function_count,
            } => write!(
                formatter,
                "bytecode image exposes {reachable} reachable records out of {function_count}"
            ),
            Self::MissingModules {
                reachable,
                module_count,
            } => write!(
                formatter,
                "bytecode image exposes {reachable} reachable modules out of {module_count}"
            ),
            Self::DuplicatePropertyKey { node } => write!(
                formatter,
                "ordinary node {} contains a duplicate semantic property key",
                node.zero_based()
            ),
            Self::NonCanonicalBigInt => {
                formatter.write_str("bytecode image contains a non-canonical BigInt payload")
            }
            Self::ArchivedBackingContextRequired { node } => write!(
                formatter,
                "node {} requires its inseparable archived SharedArrayBuffer backing context",
                node.zero_based()
            ),
            Self::CircularReference { node } => write!(
                formatter,
                "bytecode image contains a circular reference through node {}",
                node.zero_based()
            ),
            Self::CircularFunction { function_index } => write!(
                formatter,
                "bytecode image contains a recursive record cycle through function {function_index}"
            ),
            Self::CircularModule { module_index } => write!(
                formatter,
                "bytecode image contains a recursive record cycle through module {module_index}"
            ),
            Self::ModuleIntegerOutOfRange {
                module_index,
                field,
                value,
            } => write!(
                formatter,
                "module {module_index} {field:?} value {value} exceeds QuickJS's positive int range"
            ),
            Self::InvalidModuleExport {
                module_index,
                export_index,
            } => write!(
                formatter,
                "module {module_index} export {export_index} has an inconsistent binding"
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
                "canonical bytecode-image plan promised {planned} bytes but emitted {actual}"
            ),
            Self::EncodedLengthOverflow => {
                formatter.write_str("canonical bytecode-image length overflowed")
            }
            Self::AllocationFailed => {
                formatter.write_str("canonical bytecode-image writer allocation failed")
            }
        }
    }
}

impl std::error::Error for BytecodeImageEncodeError {}

impl From<WireError> for BytecodeImageEncodeError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<GraphError> for BytecodeImageEncodeError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

impl From<DataWriteStateError> for BytecodeImageEncodeError {
    fn from(error: DataWriteStateError) -> Self {
        match error {
            DataWriteStateError::Graph(error) => Self::Graph(error),
            DataWriteStateError::CircularReference { node } => Self::CircularReference { node },
            DataWriteStateError::AllocationFailed => Self::AllocationFailed,
        }
    }
}

impl From<BytecodeImageBudgetError> for BytecodeImageEncodeError {
    fn from(error: BytecodeImageBudgetError) -> Self {
        Self::Budget(error)
    }
}

impl From<ModuleBudgetError> for BytecodeImageEncodeError {
    fn from(error: ModuleBudgetError) -> Self {
        Self::Module(error)
    }
}

impl From<FunctionEnvelopeError> for BytecodeImageEncodeError {
    fn from(error: FunctionEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<CodeError> for BytecodeImageEncodeError {
    fn from(error: CodeError) -> Self {
        Self::Code(error)
    }
}

/// Canonically encode one complete, authenticated bytecode image.
pub(in crate::runtime) fn encode_bytecode_image(
    image: &BytecodeImage,
    options: BytecodeImageEncodeOptions,
) -> Result<Vec<u8>, BytecodeImageEncodeError> {
    emit::encode_authenticated(plan::authenticate_for_write(image, options)?)
}
