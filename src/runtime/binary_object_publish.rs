//! The sole executable bridge from the release-pinned BC5 archive reader.
//!
//! Binary-object decoding remains heap-independent. This module consumes only
//! the narrow scalar-script and ordinary-leaf DTOs, translates them to the
//! engine's typed compiler drafts, and enters reviewed verifier and
//! transactional publication paths.

use super::binary_object::{
    DetachedPrimitive, OrdinaryLeafApplyKind, OrdinaryLeafBinaryOp, OrdinaryLeafOp,
    OrdinaryLeafPredicateOp, OrdinaryLeafReadError, OrdinaryLeafStackOp, OrdinaryLeafUnaryOp,
    RootFunctionConstantSelector, ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp,
    ScalarValueDraft, decode_trusted_ordinary_leaf, decode_trusted_scalar_script,
};
use super::{Runtime, RuntimeError};
use crate::bigint::JsBigInt;
use crate::bytecode::{ApplyKind, Instruction};
use crate::error::{Error, ErrorKind};
use crate::function::{FunctionBytecodeRef, UnlinkedConstant, UnlinkedFunction};
use crate::heap::{ConstructorKind, ContextId, FunctionKind, FunctionMetadata};
use crate::object::CallableRef;
use crate::value::{JsString, Value};

impl Runtime {
    pub(super) fn read_trusted_ordinary_function_in_realm(
        &self,
        realm: ContextId,
        bytes: &[u8],
        root_constant_index: u32,
    ) -> Result<CallableRef, RuntimeError> {
        let draft = decode_trusted_ordinary_leaf(
            bytes,
            RootFunctionConstantSelector::from_zero_based(root_constant_index),
        )
        .map_err(map_ordinary_leaf_read_error)?;
        let (metadata, detached_constants, detached_code) = draft.into_parts();
        if !metadata.has_simple_parameter_list()
            || !metadata.has_prototype()
            || !metadata.allows_new_target()
            || !metadata.allows_arguments()
            || !metadata.strip_variable_debug()
        {
            return Err(RuntimeError::Engine(Error::internal(
                "trusted ordinary-leaf draft lost its admitted metadata capabilities",
            )));
        }

        let original_constant_count = detached_constants.len();
        let synthetic_constant_count = detached_code
            .iter()
            .filter(|operation| {
                matches!(
                    operation,
                    OrdinaryLeafOp::PushBigIntI32(_)
                        | OrdinaryLeafOp::PushEmptyString
                        | OrdinaryLeafOp::ThrowReadOnly(_)
                )
            })
            .count();
        let total_constant_count = original_constant_count
            .checked_add(synthetic_constant_count)
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "trusted ordinary-leaf constant count overflowed",
                ))
            })?;
        u32::try_from(total_constant_count).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "trusted ordinary-leaf constant count exceeded the typed index space",
            ))
        })?;

        let mut constants = Vec::new();
        constants
            .try_reserve_exact(detached_constants.len())
            .map_err(|_| {
                RuntimeError::Engine(Error::internal(
                    "could not allocate trusted ordinary-leaf constants",
                ))
            })?;
        for constant in detached_constants {
            constants.push(lower_detached_primitive(constant)?);
        }
        if synthetic_constant_count != 0 {
            constants
                .try_reserve_exact(synthetic_constant_count)
                .map_err(|_| {
                    RuntimeError::Engine(Error::internal(
                        "could not allocate trusted ordinary-leaf synthesized constants",
                    ))
                })?;
        }
        for operation in &detached_code {
            match operation {
                OrdinaryLeafOp::PushBigIntI32(value) => constants.push(lower_primitive_constant(
                    Value::BigInt(JsBigInt::from(*value)),
                )?),
                OrdinaryLeafOp::PushEmptyString => {
                    constants.push(UnlinkedConstant::atom_string(JsString::from_static("")))
                }
                OrdinaryLeafOp::ThrowReadOnly(name) => {
                    let value = JsString::try_from_utf16(name.clone().into_units().into_vec())
                        .map_err(|error| RuntimeError::Engine(error.into()))?;
                    constants.push(lower_primitive_constant(Value::String(value))?);
                }
                _ => {}
            }
        }

        let mut instructions = Vec::new();
        instructions
            .try_reserve_exact(detached_code.len())
            .map_err(|_| {
                RuntimeError::Engine(Error::internal(
                    "could not allocate trusted ordinary-leaf instructions",
                ))
            })?;
        let mut next_synthetic_index = u32::try_from(original_constant_count).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "trusted ordinary-leaf constant count exceeded the typed index space",
            ))
        })?;
        for operation in detached_code {
            instructions.push(lower_ordinary_leaf_op(
                operation,
                &mut next_synthetic_index,
            )?);
        }
        if next_synthetic_index as usize != total_constant_count {
            return Err(RuntimeError::Engine(Error::internal(
                "trusted ordinary-leaf synthesized constant plan drifted",
            )));
        }

        let function = UnlinkedFunction::new(
            instructions,
            constants,
            FunctionMetadata {
                argument_count: metadata.argument_count(),
                defined_argument_count: metadata.defined_argument_count(),
                local_count: metadata.local_count(),
                max_stack: metadata.max_stack(),
                strict: metadata.is_strict(),
                strip_variable_debug: metadata.strip_variable_debug(),
                function_kind: FunctionKind::Normal,
                has_prototype: metadata.has_prototype(),
                constructor_kind: ConstructorKind::Base,
                arguments_forbidden: !metadata.allows_arguments(),
                ..FunctionMetadata::default()
            },
        );

        super::bytecode_publish::verify_unlinked_ordinary_leaf(&function)
            .map_err(map_ordinary_leaf_verification_error)?;
        let bytecode = self.publish_verified_unlinked_function(realm, function)?;
        self.new_bytecode_closure(realm, &bytecode)
    }

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
            lower_primitive_constant(Value::Float(f64::from_bits(bits)))
                .map(LoweredScalar::Constant)
        }
        ScalarValueDraft::BigIntI32(value) => {
            lower_primitive_constant(Value::BigInt(JsBigInt::from(value)))
                .map(LoweredScalar::Constant)
        }
        ScalarValueDraft::BigIntBytes(bytes) => {
            lower_bigint_constant(&bytes).map(LoweredScalar::Constant)
        }
        ScalarValueDraft::EmptyString => Ok(LoweredScalar::AtomString(
            UnlinkedConstant::atom_string(JsString::from_static("")),
        )),
        ScalarValueDraft::ConstantString(value) => lower_scalar_string(value)
            .and_then(|value| lower_primitive_constant(Value::String(value)))
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
    lower_primitive_constant(Value::BigInt(decode_bigint_constant(bytes)?))
}

fn decode_bigint_constant(bytes: &[u8]) -> Result<JsBigInt, RuntimeError> {
    let (value, consumed) = JsBigInt::decode_bc5_signed_le(bytes, bytes.len(), bytes.len(), true)
        .map_err(|error| {
        RuntimeError::Engine(Error::internal(format!(
            "trusted binary-object draft contained invalid canonical BigInt bytes: {error:?}"
        )))
    })?;
    if consumed != bytes.len() {
        return Err(RuntimeError::Engine(Error::internal(
            "trusted scalar BigInt draft was not consumed exactly",
        )));
    }
    Ok(value)
}

fn lower_primitive_constant(value: Value) -> Result<UnlinkedConstant, RuntimeError> {
    UnlinkedConstant::primitive(value).map_err(|error| {
        RuntimeError::Engine(Error::internal(format!(
            "trusted binary-object draft produced an invalid primitive constant: {error}"
        )))
    })
}

fn lower_detached_primitive(constant: DetachedPrimitive) -> Result<UnlinkedConstant, RuntimeError> {
    let value = match constant {
        DetachedPrimitive::Undefined => Value::Undefined,
        DetachedPrimitive::Null => Value::Null,
        DetachedPrimitive::Bool(value) => Value::Bool(value),
        DetachedPrimitive::Int(value) => Value::Int(value),
        DetachedPrimitive::Float64Bits(bits) => Value::Float(f64::from_bits(bits)),
        DetachedPrimitive::String(units) => Value::String(
            JsString::try_from_utf16(units.into_vec())
                .map_err(|error| RuntimeError::Engine(error.into()))?,
        ),
        DetachedPrimitive::BigIntSignedLeCanonical(bytes) => {
            Value::BigInt(decode_bigint_constant(&bytes)?)
        }
    };
    lower_primitive_constant(value)
}

fn lower_ordinary_leaf_op(
    operation: OrdinaryLeafOp,
    next_synthetic_index: &mut u32,
) -> Result<Instruction, RuntimeError> {
    let instruction = match operation {
        OrdinaryLeafOp::Nop => Instruction::Nop,
        OrdinaryLeafOp::PushI32(value) => Instruction::PushI32(value),
        OrdinaryLeafOp::PushConst(index) => Instruction::PushConst(index),
        OrdinaryLeafOp::PushUndefined => Instruction::Undefined,
        OrdinaryLeafOp::PushNull => Instruction::Null,
        OrdinaryLeafOp::PushBool(false) => Instruction::PushFalse,
        OrdinaryLeafOp::PushBool(true) => Instruction::PushTrue,
        OrdinaryLeafOp::PushBigIntI32(_) | OrdinaryLeafOp::PushEmptyString => {
            let index = *next_synthetic_index;
            *next_synthetic_index = next_synthetic_index.checked_add(1).ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "trusted ordinary-leaf synthesized constant index overflowed",
                ))
            })?;
            Instruction::PushConst(index)
        }
        OrdinaryLeafOp::Stack(operation) => match operation {
            OrdinaryLeafStackOp::Drop => Instruction::Drop,
            OrdinaryLeafStackOp::Nip => Instruction::Nip,
            OrdinaryLeafStackOp::Dup => Instruction::Dup,
            OrdinaryLeafStackOp::Dup1 => Instruction::Dup1,
            OrdinaryLeafStackOp::Dup3 => Instruction::Dup3,
            OrdinaryLeafStackOp::Insert2 => Instruction::Insert2,
            OrdinaryLeafStackOp::Insert3 => Instruction::Insert3,
            OrdinaryLeafStackOp::Insert4 => Instruction::Insert4,
            OrdinaryLeafStackOp::Perm3 => Instruction::Perm3,
            OrdinaryLeafStackOp::Perm4 => Instruction::Perm4,
            OrdinaryLeafStackOp::Perm5 => Instruction::Perm5,
            OrdinaryLeafStackOp::Swap => Instruction::Swap,
            OrdinaryLeafStackOp::Rot4Left => Instruction::Rot4Left,
        },
        OrdinaryLeafOp::Unary(operation) => match operation {
            OrdinaryLeafUnaryOp::Neg => Instruction::Neg,
            OrdinaryLeafUnaryOp::Plus => Instruction::Plus,
            OrdinaryLeafUnaryOp::Dec => Instruction::Dec,
            OrdinaryLeafUnaryOp::Inc => Instruction::Inc,
            OrdinaryLeafUnaryOp::BitNot => Instruction::BitNot,
            OrdinaryLeafUnaryOp::LogicalNot => Instruction::Not,
            OrdinaryLeafUnaryOp::TypeOf => Instruction::TypeOf,
        },
        OrdinaryLeafOp::PostDec => Instruction::PostDec,
        OrdinaryLeafOp::PostInc => Instruction::PostInc,
        OrdinaryLeafOp::GetLocal(index) => Instruction::GetLocal(index),
        OrdinaryLeafOp::PutLocal(index) => Instruction::PutLocal(index),
        OrdinaryLeafOp::SetLocal(index) => Instruction::SetLocal(index),
        OrdinaryLeafOp::GetArgument(index) => Instruction::GetArg(index),
        OrdinaryLeafOp::PutArgument(index) => Instruction::PutArg(index),
        OrdinaryLeafOp::SetArgument(index) => Instruction::SetArg(index),
        OrdinaryLeafOp::Binary(operation) => match operation {
            OrdinaryLeafBinaryOp::Add => Instruction::Add,
            OrdinaryLeafBinaryOp::Sub => Instruction::Sub,
            OrdinaryLeafBinaryOp::Mul => Instruction::Mul,
            OrdinaryLeafBinaryOp::Div => Instruction::Div,
            OrdinaryLeafBinaryOp::Mod => Instruction::Mod,
            OrdinaryLeafBinaryOp::Pow => Instruction::Pow,
            OrdinaryLeafBinaryOp::Shl => Instruction::Shl,
            OrdinaryLeafBinaryOp::Sar => Instruction::Sar,
            OrdinaryLeafBinaryOp::Shr => Instruction::Shr,
            OrdinaryLeafBinaryOp::LessThan => Instruction::Lt,
            OrdinaryLeafBinaryOp::LessThanOrEqual => Instruction::Lte,
            OrdinaryLeafBinaryOp::GreaterThan => Instruction::Gt,
            OrdinaryLeafBinaryOp::GreaterThanOrEqual => Instruction::Gte,
            OrdinaryLeafBinaryOp::Equal => Instruction::Eq,
            OrdinaryLeafBinaryOp::NotEqual => Instruction::Neq,
            OrdinaryLeafBinaryOp::StrictEqual => Instruction::StrictEq,
            OrdinaryLeafBinaryOp::StrictNotEqual => Instruction::StrictNeq,
            OrdinaryLeafBinaryOp::BitAnd => Instruction::BitAnd,
            OrdinaryLeafBinaryOp::BitXor => Instruction::BitXor,
            OrdinaryLeafBinaryOp::BitOr => Instruction::BitOr,
        },
        OrdinaryLeafOp::Predicate(operation) => match operation {
            OrdinaryLeafPredicateOp::IsUndefinedOrNull => Instruction::IsUndefinedOrNull,
            OrdinaryLeafPredicateOp::IsUndefined => Instruction::IsUndefined,
            OrdinaryLeafPredicateOp::IsNull => Instruction::IsNull,
            OrdinaryLeafPredicateOp::TypeOfIsUndefined => Instruction::TypeOfIsUndefined,
            OrdinaryLeafPredicateOp::TypeOfIsFunction => Instruction::TypeOfIsFunction,
        },
        OrdinaryLeafOp::IfFalse(target) => Instruction::IfFalse(target),
        OrdinaryLeafOp::IfTrue(target) => Instruction::IfTrue(target),
        OrdinaryLeafOp::Goto(target) => Instruction::Goto(target),
        OrdinaryLeafOp::Call(argument_count) => Instruction::Call(argument_count),
        OrdinaryLeafOp::TailCall(argument_count) => Instruction::TailCall(argument_count),
        OrdinaryLeafOp::Construct(argument_count) => Instruction::Construct(argument_count),
        OrdinaryLeafOp::CallMethod(argument_count) => Instruction::CallMethod(argument_count),
        OrdinaryLeafOp::TailCallMethod(argument_count) => {
            Instruction::TailCallMethod(argument_count)
        }
        OrdinaryLeafOp::ArrayFrom(element_count) => Instruction::ArrayFrom(element_count),
        OrdinaryLeafOp::Apply(kind) => Instruction::Apply(match kind {
            OrdinaryLeafApplyKind::Call => ApplyKind::Call,
            OrdinaryLeafApplyKind::Construct => ApplyKind::Construct,
        }),
        OrdinaryLeafOp::Return => Instruction::Return,
        OrdinaryLeafOp::ReturnUndefined => Instruction::ReturnUndefined,
        OrdinaryLeafOp::Throw => Instruction::Throw,
        OrdinaryLeafOp::ThrowReadOnly(_) => {
            let index = *next_synthetic_index;
            *next_synthetic_index = next_synthetic_index.checked_add(1).ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "trusted ordinary-leaf synthesized constant index overflowed",
                ))
            })?;
            Instruction::ThrowReadOnly(index)
        }
    };
    Ok(instruction)
}

fn map_ordinary_leaf_verification_error(error: RuntimeError) -> RuntimeError {
    match error {
        RuntimeError::Engine(error) => RuntimeError::Engine(Error::new(
            ErrorKind::Unsupported,
            format!("trusted QuickJS ordinary leaf is not admitted by typed verification: {error}"),
        )),
        other => other,
    }
}

fn map_ordinary_leaf_read_error(error: OrdinaryLeafReadError) -> RuntimeError {
    let (kind, message) = match error {
        OrdinaryLeafReadError::Malformed(message) => (ErrorKind::Syntax, message),
        OrdinaryLeafReadError::Type(message) => (ErrorKind::Type, message),
        OrdinaryLeafReadError::Range(message) => (ErrorKind::Range, message),
        OrdinaryLeafReadError::JsInternal(message) => (ErrorKind::JsInternal, message),
        OrdinaryLeafReadError::Unadmitted(message) => (
            ErrorKind::Unsupported,
            format!("trusted QuickJS ordinary leaf is not admitted: {message}"),
        ),
        OrdinaryLeafReadError::Resource(message) => (
            ErrorKind::Unsupported,
            format!("trusted QuickJS ordinary leaf exceeds its resource policy: {message}"),
        ),
        OrdinaryLeafReadError::Internal(message) => (ErrorKind::Internal, message),
    };
    Error::new(kind, message).into()
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

    #[test]
    fn ordinary_leaf_draft_ops_lower_one_for_one_without_reordering() {
        let lower = |operation| {
            let mut next_synthetic_index = 7;
            lower_ordinary_leaf_op(operation, &mut next_synthetic_index).unwrap()
        };
        assert!(matches!(lower(OrdinaryLeafOp::Nop), Instruction::Nop));
        assert!(matches!(
            lower(OrdinaryLeafOp::PushI32(-7)),
            Instruction::PushI32(-7)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::PushConst(2)),
            Instruction::PushConst(2)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::GetLocal(3)),
            Instruction::GetLocal(3)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::PutLocal(3)),
            Instruction::PutLocal(3)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::SetLocal(3)),
            Instruction::SetLocal(3)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::GetArgument(4)),
            Instruction::GetArg(4)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::PutArgument(4)),
            Instruction::PutArg(4)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::SetArgument(4)),
            Instruction::SetArg(4)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::Binary(OrdinaryLeafBinaryOp::StrictEqual)),
            Instruction::StrictEq
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::Predicate(
                OrdinaryLeafPredicateOp::TypeOfIsUndefined
            )),
            Instruction::TypeOfIsUndefined
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::IfFalse(11)),
            Instruction::IfFalse(11)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::IfTrue(12)),
            Instruction::IfTrue(12)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::Goto(13)),
            Instruction::Goto(13)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::ReturnUndefined),
            Instruction::ReturnUndefined
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::PushEmptyString),
            Instruction::PushConst(7)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::PushBigIntI32(42)),
            Instruction::PushConst(7)
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::Stack(OrdinaryLeafStackOp::Rot4Left)),
            Instruction::Rot4Left
        ));
        assert!(matches!(
            lower(OrdinaryLeafOp::Unary(OrdinaryLeafUnaryOp::TypeOf)),
            Instruction::TypeOf
        ));
        assert!(matches!(lower(OrdinaryLeafOp::Return), Instruction::Return));
        assert!(matches!(lower(OrdinaryLeafOp::Throw), Instruction::Throw));
    }

    #[test]
    fn ordinary_plain_call_publishes_the_unchanged_argument_count() {
        for argument_count in 0..=4 {
            let mut next_synthetic_index = 0;
            assert!(matches!(
                lower_ordinary_leaf_op(
                    OrdinaryLeafOp::Call(argument_count),
                    &mut next_synthetic_index,
                ),
                Ok(Instruction::Call(actual)) if actual == argument_count
            ));
            assert_eq!(next_synthetic_index, 0);
        }
    }

    #[test]
    fn ordinary_non_tail_invocation_publishes_the_unchanged_operand() {
        for operation in [
            OrdinaryLeafOp::Construct(65_535),
            OrdinaryLeafOp::CallMethod(65_535),
            OrdinaryLeafOp::ArrayFrom(65_535),
        ] {
            let expected = match operation {
                OrdinaryLeafOp::Construct(_) => 0,
                OrdinaryLeafOp::CallMethod(_) => 1,
                OrdinaryLeafOp::ArrayFrom(_) => 2,
                _ => unreachable!("test matrix contains only invocation operations"),
            };
            let mut next_synthetic_index = 0;
            let actual = lower_ordinary_leaf_op(operation, &mut next_synthetic_index).unwrap();
            assert!(matches!(
                (expected, actual),
                (0, Instruction::Construct(65_535))
                    | (1, Instruction::CallMethod(65_535))
                    | (2, Instruction::ArrayFrom(65_535))
            ));
            assert_eq!(next_synthetic_index, 0);
        }
    }

    #[test]
    fn ordinary_tail_invocation_publishes_one_for_one_with_the_unchanged_operand() {
        for (operation, expected_method) in [
            (OrdinaryLeafOp::TailCall(u16::MAX), false),
            (OrdinaryLeafOp::TailCallMethod(u16::MAX), true),
        ] {
            let mut next_synthetic_index = 0;
            let actual = lower_ordinary_leaf_op(operation, &mut next_synthetic_index).unwrap();
            assert!(matches!(
                (expected_method, actual),
                (false, Instruction::TailCall(u16::MAX))
                    | (true, Instruction::TailCallMethod(u16::MAX))
            ));
            assert_eq!(next_synthetic_index, 0);
        }
    }

    #[test]
    fn ordinary_apply_publishes_the_typed_kind() {
        for (operation, expected) in [
            (
                OrdinaryLeafOp::Apply(OrdinaryLeafApplyKind::Call),
                ApplyKind::Call,
            ),
            (
                OrdinaryLeafOp::Apply(OrdinaryLeafApplyKind::Construct),
                ApplyKind::Construct,
            ),
        ] {
            let mut next_synthetic_index = 0;
            assert!(matches!(
                lower_ordinary_leaf_op(operation, &mut next_synthetic_index),
                Ok(Instruction::Apply(actual)) if actual == expected
            ));
            assert_eq!(next_synthetic_index, 0);
        }
    }

    fn scalar_with_code(code: &[u8]) -> Vec<u8> {
        let mut object = RETURN_42.to_vec();
        object[15] = u8::try_from(code.len()).expect("test code length fits one-byte ULEB");
        object.splice(21.., code.iter().copied());
        object
    }
}
