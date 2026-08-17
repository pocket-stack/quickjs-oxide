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
        let (instructions, constants) = match lower_scalar_draft(draft)? {
            LoweredScalar::Direct(push) => (
                vec![push, Instruction::SetLocal(0), Instruction::Return],
                Vec::new(),
            ),
            LoweredScalar::Constant(constant) => (
                vec![
                    Instruction::PushConst(0),
                    Instruction::SetLocal(0),
                    Instruction::Return,
                ],
                vec![constant],
            ),
            LoweredScalar::NegatedBigInt(constant) => (
                vec![
                    Instruction::PushConst(0),
                    Instruction::Neg,
                    Instruction::SetLocal(0),
                    Instruction::Return,
                ],
                vec![constant],
            ),
        };
        let function = UnlinkedFunction::new(
            instructions,
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

enum LoweredScalar {
    Direct(Instruction),
    Constant(UnlinkedConstant),
    NegatedBigInt(UnlinkedConstant),
}

fn lower_scalar_draft(draft: ScalarScriptDraft) -> Result<LoweredScalar, RuntimeError> {
    match draft {
        ScalarScriptDraft::Undefined => Ok(LoweredScalar::Direct(Instruction::Undefined)),
        ScalarScriptDraft::Null => Ok(LoweredScalar::Direct(Instruction::Null)),
        ScalarScriptDraft::Bool(false) => Ok(LoweredScalar::Direct(Instruction::PushFalse)),
        ScalarScriptDraft::Bool(true) => Ok(LoweredScalar::Direct(Instruction::PushTrue)),
        ScalarScriptDraft::Int(value) => Ok(LoweredScalar::Direct(Instruction::PushI32(value))),
        ScalarScriptDraft::Float64Bits(bits) => {
            lower_scalar_constant(Value::Float(f64::from_bits(bits))).map(LoweredScalar::Constant)
        }
        ScalarScriptDraft::BigIntI32(value) => {
            lower_scalar_constant(Value::BigInt(JsBigInt::from(value))).map(LoweredScalar::Constant)
        }
        ScalarScriptDraft::BigIntBytes(bytes) => {
            lower_bigint_constant(&bytes).map(LoweredScalar::Constant)
        }
        ScalarScriptDraft::NegatedBigIntI32(value) => lower_negated_bigint(JsBigInt::from(value)),
        ScalarScriptDraft::NegatedBigIntBytes(bytes) => {
            lower_negated_bigint(decode_bigint_constant(&bytes)?)
        }
        ScalarScriptDraft::EmptyString => {
            lower_scalar_constant(Value::String(JsString::from_static("")))
                .map(LoweredScalar::Constant)
        }
    }
}

fn lower_bigint_constant(bytes: &[u8]) -> Result<UnlinkedConstant, RuntimeError> {
    lower_scalar_constant(Value::BigInt(decode_bigint_constant(bytes)?))
}

fn decode_bigint_constant(bytes: &[u8]) -> Result<JsBigInt, RuntimeError> {
    let (value, consumed) = JsBigInt::decode_bc5_signed_le(bytes, bytes.len(), bytes.len(), true)
        .map_err(|error| {
        RuntimeError::Engine(Error::internal(format!(
            "trusted scalar draft contained invalid canonical BigInt bytes: {error:?}"
        )))
    })?;
    if consumed != bytes.len() {
        return Err(RuntimeError::Engine(Error::internal(
            "trusted scalar BigInt draft was not consumed exactly",
        )));
    }
    Ok(value)
}

fn lower_negated_bigint(value: JsBigInt) -> Result<LoweredScalar, RuntimeError> {
    lower_scalar_constant(Value::BigInt(value)).map(LoweredScalar::NegatedBigInt)
}

fn lower_scalar_constant(value: Value) -> Result<UnlinkedConstant, RuntimeError> {
    UnlinkedConstant::primitive(value).map_err(|error| {
        RuntimeError::Engine(Error::internal(format!(
            "trusted scalar draft produced an invalid primitive constant: {error}"
        )))
    })
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
