//! Formal parameter layout authentication before binding and flow checks.
use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::bytecode::MAX_LOCAL_SLOTS;
use crate::engine::code::bytecode_validation::{
    parameter_initializer_visible_locals, validate_parameter_bytecode_layout,
};
use crate::engine::code::function::UnlinkedFunction;
use crate::engine::code::function::metadata::ClosureVariableKind;

pub(super) struct ParameterPublicationLayout {
    pub parameter_initializer_locals: Vec<bool>,
    pub parameter_body_pc: Option<usize>,
    pub pattern_body_pc: Option<usize>,
    pub parameter_initializer_capture_locals: Option<Vec<bool>>,
}

pub(super) fn verify(
    function: &UnlinkedFunction,
    is_root: bool,
) -> Result<ParameterPublicationLayout, RuntimeError> {
    if function.metadata().local_count > MAX_LOCAL_SLOTS {
        return Err(RuntimeError::Engine(Error::internal(
            "bytecode local count exceeds QuickJS JS_MAX_LOCAL_VARS",
        )));
    }
    if is_root
        && (function.metadata().rest_parameter.is_some()
            || function.metadata().rest_pattern_start.is_some())
    {
        return Err(RuntimeError::Engine(Error::internal(
            "rest parameter metadata disagrees with argument slots",
        )));
    }
    if is_root && function.parameter_environment().is_some() {
        return Err(RuntimeError::Engine(Error::internal(
            "synthetic root contains parameter-environment metadata",
        )));
    }
    if is_root
        && (function.metadata().pattern_argument_count != 0
            || function.metadata().parameter_pattern_end.is_some())
    {
        return Err(RuntimeError::Engine(Error::internal(
            "synthetic root contains formal-parameter metadata",
        )));
    }
    let parameter_initializer_locals = function
        .local_definitions()
        .iter()
        .map(|definition| definition.is_parameter_initializer)
        .collect::<Vec<_>>();
    let parameter_body_pc = validate_parameter_bytecode_layout(
        function.metadata(),
        function.code(),
        &parameter_initializer_locals,
        function.parameter_environment(),
    )
    .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
    let pattern_body_pc = function
        .metadata()
        .parameter_pattern_end
        .map(|marker| {
            usize::try_from(marker)
                .ok()
                .and_then(|marker| marker.checked_add(1))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "parameter BindingPattern marker is outside bytecode",
                    ))
                })
        })
        .transpose()?;
    let parameter_initializer_capture_locals = parameter_initializer_visible_locals(
        function.metadata(),
        function.code(),
        parameter_body_pc,
        &parameter_initializer_locals,
        function.parameter_environment(),
    )
    .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
    if (function.metadata().rest_parameter.is_some()
        || function.metadata().rest_pattern_start.is_some()
        || function.parameter_environment().is_some()
        || function.metadata().parameter_pattern_end.is_some())
        && let Some((pc, _)) = function.code().iter().enumerate().find(|(_, instruction)| {
            matches!(
                instruction,
                crate::engine::code::bytecode::Instruction::Arguments(_)
            )
        })
    {
        let local = if let Some(synthetic) = function
            .parameter_environment()
            .and_then(|layout| layout.synthetic_arguments_local)
        {
            let Some(
                [
                    crate::engine::code::bytecode::Instruction::Arguments(_),
                    crate::engine::code::bytecode::Instruction::Dup,
                    crate::engine::code::bytecode::Instruction::InitializeLocal(target),
                    crate::engine::code::bytecode::Instruction::PutLocal(body),
                ],
            ) = function.code().get(pc..pc + 4)
            else {
                return Err(RuntimeError::Engine(Error::internal(
                    "parameter arguments object has no exact dual binding",
                )));
            };
            if *target != synthetic {
                return Err(RuntimeError::Engine(Error::internal(
                    "parameter arguments object initialized the wrong synthetic cell",
                )));
            }
            let synthetic_definition = function
                .local_definitions()
                .get(usize::from(synthetic))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "synthetic parameter arguments binding is out of bounds",
                    ))
                })?;
            if synthetic_definition.kind != ClosureVariableKind::Normal
                || !synthetic_definition.is_lexical
                || synthetic_definition.is_const
                || synthetic_definition
                    .name
                    .as_ref()
                    .is_none_or(|name| name.utf16_units().ne("arguments".encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "synthetic parameter arguments binding is not authenticated",
                )));
            }
            *body
        } else {
            let Some(crate::engine::code::bytecode::Instruction::PutLocal(local)) =
                function.code().get(pc + 1)
            else {
                return Err(RuntimeError::Engine(Error::internal(
                    "formal parameter arguments object has no entry binding",
                )));
            };
            *local
        };
        let definition = function
            .local_definitions()
            .get(usize::from(local))
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "formal parameter arguments binding is out of bounds",
                ))
            })?;
        if definition.kind != ClosureVariableKind::Normal
            || definition.is_lexical
            || definition.is_const
            || definition.is_parameter_initializer
            || definition
                .name
                .as_ref()
                .is_none_or(|name| name.utf16_units().ne("arguments".encode_utf16()))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "formal parameter arguments binding is not authenticated",
            )));
        }
    }
    Ok(ParameterPublicationLayout {
        parameter_initializer_locals,
        parameter_body_pc,
        pattern_body_pc,
        parameter_initializer_capture_locals,
    })
}
