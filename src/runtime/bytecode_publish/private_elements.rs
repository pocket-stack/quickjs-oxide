//! Publication-time authentication for class-private bytecode.
//!
//! Private names live in lexical cells, but they are not ECMAScript Values.
//! This verifier keeps ordinary local/VarRef instructions from turning those
//! cells into a public symbol channel and authenticates every typed private
//! operand against compiler-retained binding metadata.

use super::*;
use crate::bytecode::{Instruction, PrivateNameSource};

fn is_private_source_name(name: &JsString) -> bool {
    name.utf16_units().next() == Some(u16::from(b'#'))
}

fn verify_private_local(function: &UnlinkedFunction, index: u16) -> Result<(), RuntimeError> {
    let definition = function
        .local_definitions()
        .get(usize::from(index))
        .ok_or_else(|| {
            RuntimeError::Engine(Error::internal(
                "private-name local operand is out of bounds",
            ))
        })?;
    if definition.kind != ClosureVariableKind::PrivateField
        || !definition.is_lexical
        || !definition.is_const
        || definition.is_parameter_initializer
        || definition
            .name
            .as_ref()
            .is_none_or(|name| !is_private_source_name(name))
    {
        return Err(RuntimeError::Engine(Error::internal(
            "private-name local is not an authenticated immutable lexical binding",
        )));
    }
    Ok(())
}

fn verify_private_closure(function: &UnlinkedFunction, index: u16) -> Result<(), RuntimeError> {
    let descriptor = function
        .closure_variables()
        .get(usize::from(index))
        .ok_or_else(|| {
            RuntimeError::Engine(Error::internal(
                "private-name closure operand is out of bounds",
            ))
        })?;
    let valid_source = matches!(
        descriptor.source,
        ClosureSource::ParentLocal(_)
            | ClosureSource::ParentClosure(_)
            | ClosureSource::EvalEnvironment(_)
    );
    if descriptor.kind != ClosureVariableKind::PrivateField
        || !descriptor.is_lexical
        || !descriptor.is_const
        || !valid_source
        || unlinked_closure_name(function, descriptor)?
            .is_none_or(|name| !is_private_source_name(name))
    {
        return Err(RuntimeError::Engine(Error::internal(
            "private-name closure is not an authenticated immutable lexical binding",
        )));
    }
    Ok(())
}

fn verify_private_source(
    function: &UnlinkedFunction,
    source: PrivateNameSource,
) -> Result<(), RuntimeError> {
    match source {
        PrivateNameSource::Local(index) => verify_private_local(function, index),
        PrivateNameSource::Closure(index) => verify_private_closure(function, index),
    }
}

fn ordinary_local_operand(instruction: &Instruction) -> Option<u16> {
    match instruction {
        Instruction::GetLocal(index)
        | Instruction::PutLocal(index)
        | Instruction::SetLocal(index)
        | Instruction::GetLocalCheck(index)
        | Instruction::InitializeLocal(index)
        | Instruction::InitializeDerivedLocal(index)
        | Instruction::PutLocalCheck(index)
        | Instruction::SetLocalCheck(index)
        | Instruction::ReturnDerived(index) => Some(*index),
        _ => None,
    }
}

fn ordinary_closure_operand(instruction: &Instruction) -> Option<u16> {
    match instruction {
        Instruction::GetVarRef(index)
        | Instruction::PutVarRef(index)
        | Instruction::SetVarRef(index)
        | Instruction::GetVarRefCheck(index)
        | Instruction::PutVarRefCheck(index)
        | Instruction::InitializeDerivedVarRef(index)
        | Instruction::GetVar(index)
        | Instruction::GetVarUndef(index)
        | Instruction::DeleteVar(index)
        | Instruction::PutVar(index)
        | Instruction::PutVarInit(index)
        | Instruction::GlobalReference(index) => Some(*index),
        _ => None,
    }
}

pub(super) fn verify_unlinked(function: &UnlinkedFunction) -> Result<(), RuntimeError> {
    let mut initialization_counts = vec![0_u8; function.local_definitions().len()];
    let mut scope_entry_counts = vec![0_u8; function.local_definitions().len()];

    for (index, definition) in function.local_definitions().iter().enumerate() {
        if definition.kind != ClosureVariableKind::PrivateField {
            continue;
        }
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "private-name local index exceeds bytecode range",
            ))
        })?;
        verify_private_local(function, index)?;
    }
    for (index, descriptor) in function.closure_variables().iter().enumerate() {
        if descriptor.kind != ClosureVariableKind::PrivateField {
            continue;
        }
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "private-name closure index exceeds bytecode range",
            ))
        })?;
        verify_private_closure(function, index)?;
    }

    for instruction in function.code() {
        if let Some(index) = ordinary_local_operand(instruction)
            && function
                .local_definitions()
                .get(usize::from(index))
                .is_some_and(|definition| definition.kind.is_private())
        {
            return Err(RuntimeError::Engine(Error::internal(
                "ordinary local bytecode referenced a private-name binding",
            )));
        }
        if let Some(index) = ordinary_closure_operand(instruction)
            && function
                .closure_variables()
                .get(usize::from(index))
                .is_some_and(|descriptor| descriptor.kind.is_private())
        {
            return Err(RuntimeError::Engine(Error::internal(
                "ordinary closure bytecode referenced a private-name binding",
            )));
        }

        match *instruction {
            Instruction::SetLocalUninitialized(index)
                if function
                    .local_definitions()
                    .get(usize::from(index))
                    .is_some_and(|definition| definition.kind.is_private()) =>
            {
                let count = scope_entry_counts
                    .get_mut(usize::from(index))
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "private-name scope-entry local is out of bounds",
                        ))
                    })?;
                *count = count.checked_add(1).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "private-name scope-entry count overflowed",
                    ))
                })?;
            }
            Instruction::InitializePrivateName(index) => {
                verify_private_local(function, index)?;
                let count = initialization_counts
                    .get_mut(usize::from(index))
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "private-name initializer local is out of bounds",
                        ))
                    })?;
                *count = count.checked_add(1).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "private-name initializer count overflowed",
                    ))
                })?;
            }
            Instruction::GetPrivateField(source)
            | Instruction::GetPrivateField2(source)
            | Instruction::PutPrivateField(source)
            | Instruction::PrivateIn(source) => verify_private_source(function, source)?,
            Instruction::DefinePrivateField(source) => {
                verify_private_source(function, source)?;
                if !matches!(
                    function.metadata().class_initializer_kind,
                    Some(
                        ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements
                    )
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private-field definition escaped a class initializer",
                    )));
                }
            }
            _ => {}
        }
    }

    for (index, definition) in function.local_definitions().iter().enumerate() {
        if definition.kind == ClosureVariableKind::PrivateField && initialization_counts[index] != 1
        {
            return Err(RuntimeError::Engine(Error::internal(
                "private-name local does not have exactly one lexical initializer",
            )));
        }
        if definition.kind == ClosureVariableKind::PrivateField && scope_entry_counts[index] != 1 {
            return Err(RuntimeError::Engine(Error::internal(
                "private-name local does not have exactly one lexical scope entry",
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_local_function(code: Vec<Instruction>) -> UnlinkedFunction {
        let metadata = FunctionMetadata {
            local_count: 1,
            max_stack: 2,
            ..FunctionMetadata::default()
        };
        UnlinkedFunction::new(code, Vec::new(), metadata).with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition {
                name: Some(JsString::from_static("#field")),
                is_lexical: true,
                is_const: true,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::PrivateField,
            }],
        )
    }

    #[test]
    fn private_local_allows_only_one_typed_initializer_and_lifecycle_ops() {
        let valid = private_local_function(vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::InitializePrivateName(0),
            Instruction::CloseLocal(0),
        ]);
        assert!(verify_unlinked(&valid).is_ok());

        let missing = private_local_function(vec![Instruction::SetLocalUninitialized(0)]);
        assert!(matches!(
            verify_unlinked(&missing),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "private-name local does not have exactly one lexical initializer"
        ));

        let duplicate = private_local_function(vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::InitializePrivateName(0),
            Instruction::InitializePrivateName(0),
        ]);
        assert!(matches!(
            verify_unlinked(&duplicate),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "private-name local does not have exactly one lexical initializer"
        ));

        let missing_scope_entry =
            private_local_function(vec![Instruction::InitializePrivateName(0)]);
        assert!(matches!(
            verify_unlinked(&missing_scope_entry),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "private-name local does not have exactly one lexical scope entry"
        ));
    }

    #[test]
    fn private_local_rejects_ordinary_value_reads() {
        let forged = private_local_function(vec![
            Instruction::InitializePrivateName(0),
            Instruction::GetLocalCheck(0),
        ]);
        assert!(matches!(
            verify_unlinked(&forged),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "ordinary local bytecode referenced a private-name binding"
        ));
    }

    #[test]
    fn private_definition_requires_a_class_initializer_role() {
        let constants = vec![
            UnlinkedConstant::primitive(Value::String(JsString::from_static("#field"))).unwrap(),
        ];
        let descriptor = ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Constant(0),
            is_lexical: true,
            is_const: true,
            kind: ClosureVariableKind::PrivateField,
        };
        let metadata = FunctionMetadata {
            closure_count: 1,
            ..FunctionMetadata::default()
        };
        let forged = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::DefinePrivateField(PrivateNameSource::Closure(
                0,
            ))],
            constants,
            metadata,
            vec![descriptor],
        );
        assert!(matches!(
            verify_unlinked(&forged),
            Err(RuntimeError::Engine(ref error))
                if error.message() == "private-field definition escaped a class initializer"
        ));
    }
}
