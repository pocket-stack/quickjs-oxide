//! Publication operand permissions and ranges, in first-error order.
use super::eval::{
    eval_variable_object_local_kind, verify_dynamic_environment_source,
    verify_eval_variable_source, verify_unlinked_string_constant,
};
use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariableKind, ClosureVariableName,
};
use crate::engine::code::function::{UnlinkedConstant, UnlinkedFunction};
use std::collections::HashMap;

pub(super) fn verify(
    function: &UnlinkedFunction,
    captured_locals: &[bool],
    global_function_initializer_pcs: &HashMap<usize, u16>,
    masked_lexical_initializer_targets: &[usize],
) -> Result<(), RuntimeError> {
    for (pc, instruction) in function.code().iter().enumerate() {
        if let Some((source, name)) = match instruction {
            crate::engine::code::bytecode::Instruction::HasEvalVariable { source, name }
            | crate::engine::code::bytecode::Instruction::GetEvalVariable { source, name }
            | crate::engine::code::bytecode::Instruction::PutEvalVariable { source, name }
            | crate::engine::code::bytecode::Instruction::DeleteEvalVariable { source, name }
            | crate::engine::code::bytecode::Instruction::DefineEvalVariable { source, name } => {
                Some((*source, *name))
            }
            _ => None,
        } {
            verify_eval_variable_source(function, source)?;
            let name = usize::try_from(name)
                .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
            if !matches!(
                function
                    .constants()
                    .get(name)
                    .and_then(UnlinkedConstant::as_primitive),
                Some(crate::engine::value::PrimitiveValue::String(_))
            ) {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval variable opcode referenced a non-string name constant",
                )));
            }
        }
        if let Some((source, name)) = match instruction {
            crate::engine::code::bytecode::Instruction::HasDynamicBinding { source, name }
            | crate::engine::code::bytecode::Instruction::GetDynamicBinding { source, name }
            | crate::engine::code::bytecode::Instruction::PutDynamicBinding { source, name }
            | crate::engine::code::bytecode::Instruction::DeleteDynamicBinding { source, name } => {
                Some((*source, Some(*name)))
            }
            crate::engine::code::bytecode::Instruction::DynamicEnvironmentObject(source) => {
                Some((*source, None))
            }
            _ => None,
        } {
            verify_dynamic_environment_source(function, source)?;
            if let Some(name) = name {
                verify_unlinked_string_constant(
                    function,
                    name,
                    "dynamic binding opcode referenced a non-string name constant",
                )?;
            }
        }
        if let Some(name) = match instruction {
            crate::engine::code::bytecode::Instruction::GetRefValue(name)
            | crate::engine::code::bytecode::Instruction::GetRefValueUndef(name)
            | crate::engine::code::bytecode::Instruction::PutRefValue(name) => Some(*name),
            _ => None,
        } {
            verify_unlinked_string_constant(
                function,
                name,
                "reference opcode referenced a non-string name constant",
            )?;
        }
        match instruction {
            crate::engine::code::bytecode::Instruction::PushConst(index) => {
                let index = usize::try_from(*index)
                    .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
                let constant = function.constants().get(index).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal("constant index is out of bounds"))
                })?;
                if constant.as_child().is_some() {
                    return Err(RuntimeError::Engine(Error::internal(
                        "value-constant opcode referenced child function bytecode",
                    )));
                }
                if constant.as_regexp().is_some() {
                    return Err(RuntimeError::Engine(Error::internal(
                        "value-constant opcode referenced a RegExp literal constant",
                    )));
                }
            }
            crate::engine::code::bytecode::Instruction::RegExp(index) => {
                let index = usize::try_from(*index)
                    .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
                let constant = function.constants().get(index).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal("constant index is out of bounds"))
                })?;
                if constant.as_regexp().is_none() {
                    return Err(RuntimeError::Engine(Error::internal(
                        "RegExp opcode referenced a non-RegExp constant",
                    )));
                }
            }
            crate::engine::code::bytecode::Instruction::FClosure(index) => {
                let index = usize::try_from(*index)
                    .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
                let constant = function.constants().get(index).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal("constant index is out of bounds"))
                })?;
                if constant.as_child().is_none() {
                    return Err(RuntimeError::Engine(Error::internal(
                        "function-closure opcode referenced a value constant",
                    )));
                }
            }
            crate::engine::code::bytecode::Instruction::SetName(index)
            | crate::engine::code::bytecode::Instruction::ThrowReadOnly(index)
            | crate::engine::code::bytecode::Instruction::ThrowRedeclaration(index)
            | crate::engine::code::bytecode::Instruction::GetField(index)
            | crate::engine::code::bytecode::Instruction::GetField2(index)
            | crate::engine::code::bytecode::Instruction::PutField(index)
            | crate::engine::code::bytecode::Instruction::DefineField(index)
            | crate::engine::code::bytecode::Instruction::DefineMethod { key: index, .. }
            | crate::engine::code::bytecode::Instruction::DefineClass { name: index, .. } => {
                let index = usize::try_from(*index)
                    .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
                let constant = function.constants().get(index).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "string-key constant index is out of bounds",
                    ))
                })?;
                if !matches!(
                    constant.as_primitive(),
                    Some(crate::engine::value::PrimitiveValue::String(_))
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "string-key opcode referenced a non-string constant",
                    )));
                }
            }
            crate::engine::code::bytecode::Instruction::PutLocal(index)
            | crate::engine::code::bytecode::Instruction::SetLocal(index)
                if function.metadata().function_name_local == Some(*index) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "bytecode directly writes its private function-name local",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetLocal(index)
            | crate::engine::code::bytecode::Instruction::PutLocal(index)
            | crate::engine::code::bytecode::Instruction::SetLocal(index)
                if eval_variable_object_local_kind(function, *index).is_some()
                    && !(matches!(
                        instruction,
                        crate::engine::code::bytecode::Instruction::PutLocal(_)
                    ) && pc.checked_sub(1).is_some_and(|previous| {
                        matches!(
                            function.code().get(previous),
                            Some(crate::engine::code::bytecode::Instruction::VariableEnvironment)
                        )
                    })) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "ordinary local opcode referenced a private eval variable object",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetLocal(index)
            | crate::engine::code::bytecode::Instruction::PutLocal(index)
            | crate::engine::code::bytecode::Instruction::SetLocal(index)
                if function
                    .local_definitions()
                    .get(usize::from(*index))
                    .is_some_and(|definition| {
                        definition.kind == ClosureVariableKind::WithObject
                    }) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "ordinary local opcode referenced a private with object",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetLocal(index)
            | crate::engine::code::bytecode::Instruction::PutLocal(index)
            | crate::engine::code::bytecode::Instruction::SetLocal(index)
                if function
                    .local_definitions()
                    .get(usize::from(*index))
                    .is_some_and(|definition| definition.is_lexical) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "unchecked local opcode referenced a lexical definition",
                )));
            }
            crate::engine::code::bytecode::Instruction::SetLocalUninitialized(index)
            | crate::engine::code::bytecode::Instruction::GetLocalCheck(index)
            | crate::engine::code::bytecode::Instruction::PutLocalCheck(index)
            | crate::engine::code::bytecode::Instruction::SetLocalCheck(index)
                if function
                    .local_definitions()
                    .get(usize::from(*index))
                    .is_some_and(|definition| !definition.is_lexical) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "checked lexical-local opcode referenced an ordinary definition",
                )));
            }
            crate::engine::code::bytecode::Instruction::InitializeLocal(index)
            | crate::engine::code::bytecode::Instruction::CloseLocal(index)
                if function
                    .local_definitions()
                    .get(usize::from(*index))
                    .is_some_and(|definition| {
                        !definition.is_lexical && definition.kind != ClosureVariableKind::WithObject
                    }) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "lifetime opcode referenced an ordinary local definition",
                )));
            }
            crate::engine::code::bytecode::Instruction::PutLocalCheck(index)
            | crate::engine::code::bytecode::Instruction::SetLocalCheck(index)
                if function
                    .local_definitions()
                    .get(usize::from(*index))
                    .is_some_and(|definition| definition.is_const) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "mutable lexical-local write bypassed a const definition",
                )));
            }
            crate::engine::code::bytecode::Instruction::CloseLocal(index)
                if captured_locals
                    .get(usize::from(*index))
                    .is_some_and(|captured| !captured) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "CloseLocal referenced a local which no child captures",
                )));
            }
            crate::engine::code::bytecode::Instruction::PutVarRef(index)
            | crate::engine::code::bytecode::Instruction::SetVarRef(index)
                if function
                    .closure_variables()
                    .get(usize::from(*index))
                    .is_some_and(|descriptor| {
                        descriptor.kind == ClosureVariableKind::FunctionName
                    }) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "bytecode directly writes a private function-name closure",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetVarRef(index)
            | crate::engine::code::bytecode::Instruction::PutVarRef(index)
            | crate::engine::code::bytecode::Instruction::SetVarRef(index)
            | crate::engine::code::bytecode::Instruction::GetVarRefCheck(index)
            | crate::engine::code::bytecode::Instruction::PutVarRefCheck(index)
                if function
                    .closure_variables()
                    .get(usize::from(*index))
                    .is_some_and(|descriptor| {
                        matches!(
                            descriptor.kind,
                            ClosureVariableKind::EvalVariableObject
                                | ClosureVariableKind::ArgEvalVariableObject
                                | ClosureVariableKind::WithObject
                        )
                    }) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "ordinary closure opcode referenced a hidden object binding",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetVarRef(index)
            | crate::engine::code::bytecode::Instruction::PutVarRef(index)
            | crate::engine::code::bytecode::Instruction::SetVarRef(index)
            | crate::engine::code::bytecode::Instruction::GetVarRefCheck(index)
            | crate::engine::code::bytecode::Instruction::PutVarRefCheck(index)
                if function
                    .closure_variables()
                    .get(usize::from(*index))
                    .is_some_and(|descriptor| {
                        matches!(
                            descriptor.source,
                            ClosureSource::GlobalDeclaration
                                | ClosureSource::Global
                                | ClosureSource::ParentGlobal(_)
                        )
                    }) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "lexical closure opcode referenced a global closure descriptor",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetVarRef(index)
            | crate::engine::code::bytecode::Instruction::PutVarRef(index)
            | crate::engine::code::bytecode::Instruction::SetVarRef(index)
                if function
                    .closure_variables()
                    .get(usize::from(*index))
                    .is_some_and(|descriptor| descriptor.is_lexical) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "unchecked closure opcode referenced a lexical binding",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetVarRefCheck(index)
            | crate::engine::code::bytecode::Instruction::PutVarRefCheck(index)
                if function
                    .closure_variables()
                    .get(usize::from(*index))
                    .is_some_and(|descriptor| !descriptor.is_lexical) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "checked closure opcode referenced an ordinary binding",
                )));
            }
            crate::engine::code::bytecode::Instruction::PutVarRefCheck(index)
                if function
                    .closure_variables()
                    .get(usize::from(*index))
                    .is_some_and(|descriptor| descriptor.is_const) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "mutable checked closure write bypassed a const binding",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetVar(index)
            | crate::engine::code::bytecode::Instruction::GetVarUndef(index)
            | crate::engine::code::bytecode::Instruction::DeleteVar(index)
            | crate::engine::code::bytecode::Instruction::PutVar(index)
            | crate::engine::code::bytecode::Instruction::PutVarInit(index)
            | crate::engine::code::bytecode::Instruction::GlobalReference(index)
                if function
                    .closure_variables()
                    .get(usize::from(*index))
                    .is_some_and(|descriptor| {
                        !matches!(
                            descriptor.source,
                            ClosureSource::GlobalDeclaration
                                | ClosureSource::Global
                                | ClosureSource::ParentGlobal(_)
                        ) || !matches!(descriptor.name, ClosureVariableName::Constant(_))
                    }) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "global closure opcode referenced a non-global closure descriptor",
                )));
            }
            crate::engine::code::bytecode::Instruction::PutVarInit(index)
                if function
                    .closure_variables()
                    .get(usize::from(*index))
                    .is_some_and(|descriptor| {
                        !descriptor.is_lexical
                            && global_function_initializer_pcs.get(&pc) != Some(index)
                            && !masked_lexical_initializer_targets.contains(&usize::from(*index))
                    }) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "global initializer referenced an ordinary non-function descriptor",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetLocal(index)
            | crate::engine::code::bytecode::Instruction::PutLocal(index)
            | crate::engine::code::bytecode::Instruction::SetLocal(index)
            | crate::engine::code::bytecode::Instruction::SetLocalUninitialized(index)
            | crate::engine::code::bytecode::Instruction::GetLocalCheck(index)
            | crate::engine::code::bytecode::Instruction::InitializeLocal(index)
            | crate::engine::code::bytecode::Instruction::PutLocalCheck(index)
            | crate::engine::code::bytecode::Instruction::SetLocalCheck(index)
            | crate::engine::code::bytecode::Instruction::CloseLocal(index)
                if *index >= function.metadata().local_count =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "local bytecode operand is out of bounds",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetArg(index)
            | crate::engine::code::bytecode::Instruction::PutArg(index)
            | crate::engine::code::bytecode::Instruction::SetArg(index)
                if *index >= function.metadata().argument_count =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "argument bytecode operand is out of bounds",
                )));
            }
            crate::engine::code::bytecode::Instruction::Rest(start)
                if *start > function.metadata().argument_count =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "rest bytecode operand is out of bounds",
                )));
            }
            crate::engine::code::bytecode::Instruction::GetVarRef(index)
            | crate::engine::code::bytecode::Instruction::PutVarRef(index)
            | crate::engine::code::bytecode::Instruction::SetVarRef(index)
            | crate::engine::code::bytecode::Instruction::GetVarRefCheck(index)
            | crate::engine::code::bytecode::Instruction::PutVarRefCheck(index)
            | crate::engine::code::bytecode::Instruction::InitializeVarRef(index)
            | crate::engine::code::bytecode::Instruction::InitializeModuleImportCollision(index)
            | crate::engine::code::bytecode::Instruction::GetVar(index)
            | crate::engine::code::bytecode::Instruction::GetVarUndef(index)
            | crate::engine::code::bytecode::Instruction::DeleteVar(index)
            | crate::engine::code::bytecode::Instruction::PutVar(index)
            | crate::engine::code::bytecode::Instruction::PutVarInit(index)
            | crate::engine::code::bytecode::Instruction::GlobalReference(index)
                if *index >= function.metadata().closure_count =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "closure variable bytecode operand is out of bounds",
                )));
            }
            _ => {}
        }
    }
    Ok(())
}
