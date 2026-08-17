//! The sole executable bridge from the release-pinned BC5 archive reader.
//!
//! Binary-object decoding remains heap-independent. This module consumes only
//! the narrow scalar-script DTO, translates it to the engine's typed compiler
//! draft, and enters the same verifier and transactional publication path as
//! source compilation.

use super::binary_object::{
    ScalarScriptDraft, ScalarScriptReadError, decode_trusted_scalar_script,
};
use super::{Runtime, RuntimeError};
use crate::bigint::JsBigInt;
use crate::bytecode::Instruction;
use crate::error::{Error, ErrorKind};
use crate::function::{FunctionBytecodeRef, UnlinkedConstant, UnlinkedFunction};
use crate::heap::{ContextId, FunctionMetadata};
use crate::value::{JsString, Value};

impl Runtime {
    pub(super) fn read_trusted_scalar_script_in_realm(
        &self,
        realm: ContextId,
        bytes: &[u8],
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let draft = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;
        let (push, constants) = lower_scalar_draft(draft)?;
        let function = UnlinkedFunction::new(
            vec![push, Instruction::SetLocal(0), Instruction::Return],
            constants,
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                strip_variable_debug: true,
                ..FunctionMetadata::default()
            },
        );

        // This is intentionally the ordinary compiler publication boundary.
        // It verifies the complete draft before allocating a bytecode node.
        self.publish_unlinked_function(realm, function)
    }
}

fn lower_scalar_draft(
    draft: ScalarScriptDraft,
) -> Result<(Instruction, Vec<UnlinkedConstant>), RuntimeError> {
    match draft {
        ScalarScriptDraft::Undefined => Ok((Instruction::Undefined, Vec::new())),
        ScalarScriptDraft::Null => Ok((Instruction::Null, Vec::new())),
        ScalarScriptDraft::Bool(false) => Ok((Instruction::PushFalse, Vec::new())),
        ScalarScriptDraft::Bool(true) => Ok((Instruction::PushTrue, Vec::new())),
        ScalarScriptDraft::Int(value) => Ok((Instruction::PushI32(value), Vec::new())),
        ScalarScriptDraft::BigIntI32(value) => {
            lower_scalar_constant(Value::BigInt(JsBigInt::from(value)))
        }
        ScalarScriptDraft::EmptyString => {
            lower_scalar_constant(Value::String(JsString::from_static("")))
        }
    }
}

fn lower_scalar_constant(
    value: Value,
) -> Result<(Instruction, Vec<UnlinkedConstant>), RuntimeError> {
    let constant = UnlinkedConstant::primitive(value).map_err(|error| {
        RuntimeError::Engine(Error::internal(format!(
            "trusted scalar draft produced an invalid primitive constant: {error}"
        )))
    })?;
    Ok((Instruction::PushConst(0), vec![constant]))
}

fn map_read_error(error: ScalarScriptReadError) -> RuntimeError {
    let (kind, message) = match error {
        ScalarScriptReadError::Malformed(message) => (ErrorKind::Syntax, message),
        ScalarScriptReadError::Type(message) => (ErrorKind::Type, message),
        ScalarScriptReadError::Range(message) => (ErrorKind::Range, message),
        ScalarScriptReadError::JsInternal(message) => (ErrorKind::JsInternal, message),
        ScalarScriptReadError::Unadmitted(message) => (
            ErrorKind::Unsupported,
            format!("trusted QuickJS scalar script is not admitted: {message}"),
        ),
        ScalarScriptReadError::Resource(message) => (
            ErrorKind::Unsupported,
            format!("trusted QuickJS scalar script exceeds its resource policy: {message}"),
        ),
        ScalarScriptReadError::Internal(message) => (ErrorKind::Internal, message),
    };
    Error::new(kind, message).into()
}
