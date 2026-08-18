//! The sole executable bridge from the release-pinned BC5 archive reader.
//!
//! Binary-object decoding remains heap-independent. This module consumes only
//! the narrow scalar-script DTO, translates it to the engine's typed compiler
//! draft, and enters the same verifier and transactional publication path as
//! source compilation.

use super::binary_object::{
    ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft,
    decode_trusted_scalar_script,
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
        let (value, unary_ops) = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;
        let (push, constants) = match lower_scalar_value(value)? {
            LoweredScalar::Direct(push) => (push, Vec::new()),
            LoweredScalar::Constant(constant) | LoweredScalar::AtomString(constant) => {
                (Instruction::PushConst(0), vec![constant])
            }
            LoweredScalar::IntegerAtomString(value) => {
                (Instruction::PushAtomValueIndex(value), Vec::new())
            }
        };
        let instruction_capacity = unary_ops.len().checked_add(3).ok_or_else(|| {
            RuntimeError::Engine(Error::internal(
                "trusted scalar instruction count overflowed",
            ))
        })?;
        let mut instructions = Vec::new();
        instructions
            .try_reserve_exact(instruction_capacity)
            .map_err(|_| {
                RuntimeError::Engine(Error::internal(
                    "could not allocate trusted scalar instruction draft",
                ))
            })?;
        instructions.push(push);
        for operation in unary_ops {
            instructions.push(match operation {
                ScalarUnaryOp::Neg => Instruction::Neg,
                ScalarUnaryOp::Plus => Instruction::Plus,
                ScalarUnaryOp::Dec => Instruction::Dec,
                ScalarUnaryOp::Inc => Instruction::Inc,
                ScalarUnaryOp::BitNot => Instruction::BitNot,
                ScalarUnaryOp::LogicalNot => Instruction::Not,
                ScalarUnaryOp::TypeOf => Instruction::TypeOf,
            });
        }
        instructions.push(Instruction::SetLocal(0));
        instructions.push(Instruction::Return);
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
    AtomString(UnlinkedConstant),
    IntegerAtomString(u32),
}

fn lower_scalar_value(value: ScalarValueDraft) -> Result<LoweredScalar, RuntimeError> {
    match value {
        ScalarValueDraft::Undefined => Ok(LoweredScalar::Direct(Instruction::Undefined)),
        ScalarValueDraft::Null => Ok(LoweredScalar::Direct(Instruction::Null)),
        ScalarValueDraft::Bool(false) => Ok(LoweredScalar::Direct(Instruction::PushFalse)),
        ScalarValueDraft::Bool(true) => Ok(LoweredScalar::Direct(Instruction::PushTrue)),
        ScalarValueDraft::Int(value) => Ok(LoweredScalar::Direct(Instruction::PushI32(value))),
        ScalarValueDraft::Float64Bits(bits) => {
            lower_scalar_constant(Value::Float(f64::from_bits(bits))).map(LoweredScalar::Constant)
        }
        ScalarValueDraft::BigIntI32(value) => {
            lower_scalar_constant(Value::BigInt(JsBigInt::from(value))).map(LoweredScalar::Constant)
        }
        ScalarValueDraft::BigIntBytes(bytes) => {
            lower_bigint_constant(&bytes).map(LoweredScalar::Constant)
        }
        ScalarValueDraft::EmptyString => Ok(LoweredScalar::AtomString(
            UnlinkedConstant::atom_string(JsString::from_static("")),
        )),
        ScalarValueDraft::ConstantString(value) => lower_scalar_string(value)
            .and_then(|value| lower_scalar_constant(Value::String(value)))
            .map(LoweredScalar::Constant),
        ScalarValueDraft::AtomString(value) => Ok(LoweredScalar::AtomString(
            UnlinkedConstant::atom_string(lower_scalar_string(value)?),
        )),
        ScalarValueDraft::IntegerAtomString(value) => Ok(LoweredScalar::IntegerAtomString(value)),
    }
}

fn lower_scalar_string(value: ScalarStringDraft) -> Result<JsString, RuntimeError> {
    JsString::try_from_utf16(value.into_units()).map_err(|error| RuntimeError::Engine(error.into()))
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

#[cfg(test)]
mod tests {
    use super::*;

    const RETURN_42: [u8; 25] = [
        0x05, 0x00, 0x0c, 0x00, 0x02, 0x00, 0xa8, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0xbb, 0x2a, 0xcb, 0x28,
    ];

    #[test]
    fn publisher_emits_the_authenticated_unary_chain_in_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let image = scalar_with_code(&[
            0xbb, 42, 0x8a, 0x8b, 0x8c, 0x8d, 0x93, 0x94, 0x95, 0xcb, 0x28,
        ]);

        let function = context.read_trusted_scalar_script(&image).unwrap();
        let snapshot = runtime.snapshot_function_bytecode(&function).unwrap();
        assert!(matches!(
            snapshot.code.as_ref(),
            [
                Instruction::PushI32(42),
                Instruction::Neg,
                Instruction::Plus,
                Instruction::Dec,
                Instruction::Inc,
                Instruction::BitNot,
                Instruction::Not,
                Instruction::TypeOf,
                Instruction::SetLocal(0),
                Instruction::Return,
            ]
        ));
        assert!(snapshot.constants.is_empty());
        drop(snapshot);

        assert_eq!(
            context.execute(&function).unwrap(),
            Value::String(JsString::from_static("boolean"))
        );
    }

    #[test]
    fn bigint_unary_plus_publishes_and_throws_only_when_executed() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let baseline = runtime.heap_counts().function_bytecode_nodes;
        let image = scalar_with_code(&[0xb0, 1, 0, 0, 0, 0x8b, 0xcb, 0x28]);

        let function = context.read_trusted_scalar_script(&image).unwrap();
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, baseline + 1);
        let snapshot = runtime.snapshot_function_bytecode(&function).unwrap();
        assert!(matches!(
            snapshot.code.as_ref(),
            [
                Instruction::PushConst(0),
                Instruction::Plus,
                Instruction::SetLocal(0),
                Instruction::Return,
            ]
        ));
        assert_eq!(snapshot.constants.len(), 1);
        drop(snapshot);

        assert_eq!(context.execute(&function), Err(RuntimeError::Exception));
        assert!(context.has_exception());
    }

    fn scalar_with_code(code: &[u8]) -> Vec<u8> {
        let mut object = RETURN_42.to_vec();
        object[15] = u8::try_from(code.len()).expect("test code length fits one-byte ULEB");
        object.splice(21.., code.iter().copied());
        object
    }
}
