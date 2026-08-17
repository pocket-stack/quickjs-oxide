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
use crate::bytecode::Instruction;
use crate::error::{Error, ErrorKind};
use crate::function::{FunctionBytecodeRef, UnlinkedFunction};
use crate::heap::{ContextId, FunctionMetadata};

impl Runtime {
    pub(super) fn read_trusted_scalar_script_in_realm(
        &self,
        realm: ContextId,
        bytes: &[u8],
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let draft = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;
        let function = match draft {
            ScalarScriptDraft::Int(value) => UnlinkedFunction::new(
                vec![
                    Instruction::PushI32(value),
                    Instruction::SetLocal(0),
                    Instruction::Return,
                ],
                Vec::new(),
                FunctionMetadata {
                    local_count: 1,
                    max_stack: 1,
                    strip_variable_debug: true,
                    ..FunctionMetadata::default()
                },
            ),
        };

        // This is intentionally the ordinary compiler publication boundary.
        // It verifies the complete draft before allocating a bytecode node.
        self.publish_unlinked_function(realm, function)
    }
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
