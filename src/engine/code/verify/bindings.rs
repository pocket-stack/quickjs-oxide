//! Local and argument binding authentication, before closure provenance checks.
use super::pseudo_binding_entry;
use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::function::UnlinkedFunction;
use crate::engine::code::function::metadata::{ClosureVariableKind, EvalKind, FunctionKind};
use crate::engine::value::JsString;

pub(super) fn verify_metadata(
    function: &UnlinkedFunction,
    is_root: bool,
    arg_eval_variable_object_local: Option<u16>,
) -> Result<(), RuntimeError> {
    if function
        .metadata()
        .function_name_local
        .is_some_and(|index| index >= function.metadata().local_count)
    {
        return Err(RuntimeError::Engine(Error::internal(
            "function-name local is outside bytecode local slots",
        )));
    }
    for (local, message) in [
        (
            function.metadata().derived_this_local,
            "derived this local is outside bytecode local slots",
        ),
        (
            function.metadata().active_function_local,
            "active-function local is outside bytecode local slots",
        ),
    ] {
        if local.is_some_and(|index| index >= function.metadata().local_count) {
            return Err(RuntimeError::Engine(Error::internal(message)));
        }
    }
    if function
        .metadata()
        .eval_variable_object_local
        .is_some_and(|index| index >= function.metadata().local_count)
    {
        return Err(RuntimeError::Engine(Error::internal(
            "eval variable-object local is outside bytecode local slots",
        )));
    }
    if arg_eval_variable_object_local.is_some_and(|index| index >= function.metadata().local_count)
    {
        return Err(RuntimeError::Engine(Error::internal(
            "parameter eval variable-object local is outside bytecode local slots",
        )));
    }
    if function.metadata().eval_variable_object_local.is_some()
        && function.metadata().eval_variable_object_local == function.metadata().function_name_local
    {
        return Err(RuntimeError::Engine(Error::internal(
            "eval variable-object and function-name locals overlap",
        )));
    }
    if arg_eval_variable_object_local.is_some_and(|index| {
        Some(index) == function.metadata().eval_variable_object_local
            || Some(index) == function.metadata().function_name_local
    }) {
        return Err(RuntimeError::Engine(Error::internal(
            "parameter eval variable-object local overlaps another hidden local",
        )));
    }
    let private_locals = [
        function.metadata().function_name_local,
        function.metadata().eval_variable_object_local,
        arg_eval_variable_object_local,
        function.metadata().derived_this_local,
        function.metadata().active_function_local,
    ];
    for (index, local) in private_locals.iter().enumerate() {
        if local.is_some()
            && private_locals[..index]
                .iter()
                .any(|earlier| earlier == local)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "authenticated private locals overlap",
            )));
        }
    }
    // GetSuper is also used by derived-constructor `super()`, while the
    // get/put-value operations consume only ordinary stack values. The
    // hidden capability authenticated by this metadata is specifically
    // reading the active function object's HomeObject.
    let reads_home_object = function.code().iter().any(|instruction| {
        matches!(
            instruction,
            crate::engine::code::bytecode::Instruction::PushHomeObject
        )
    });
    if reads_home_object && !function.metadata().needs_home_object {
        return Err(RuntimeError::Engine(Error::internal(
            "super bytecode has no authenticated HomeObject metadata",
        )));
    }
    if function.metadata().eval_variable_object_local.is_some()
        && (is_root
            || function.metadata().strict
            || function.metadata().eval_kind != EvalKind::None
            || !matches!(
                function.metadata().function_kind,
                FunctionKind::Normal
                    | FunctionKind::Generator
                    | FunctionKind::Async
                    | FunctionKind::AsyncGenerator
            ))
    {
        return Err(RuntimeError::Engine(Error::internal(
            "eval variable-object local escaped a sloppy ordinary, generator, or async function",
        )));
    }
    if arg_eval_variable_object_local.is_some()
        && (function.metadata().strict
            || function.metadata().eval_kind != EvalKind::None
            || !matches!(
                function.metadata().function_kind,
                FunctionKind::Normal
                    | FunctionKind::Generator
                    | FunctionKind::Async
                    | FunctionKind::AsyncGenerator
            )
            || function.metadata().eval_variable_object_local.is_none())
    {
        return Err(RuntimeError::Engine(Error::internal(
            "parameter eval variable-object local escaped a sloppy eval-enabled function",
        )));
    }
    if is_root
        && function.metadata().eval_kind != EvalKind::None
        && function.metadata().function_name_local.is_some()
    {
        return Err(RuntimeError::Engine(Error::internal(
            "function-name local escaped into a synthetic eval root",
        )));
    }
    if function.metadata().function_name_local.is_some()
        && function.func_name().is_none_or(JsString::is_empty)
    {
        return Err(RuntimeError::Engine(Error::internal(
            "function-name local requires a non-empty intrinsic function name",
        )));
    }
    if function.argument_definitions().len() != usize::from(function.metadata().argument_count) {
        return Err(RuntimeError::Engine(Error::internal(
            "argument definition count does not match bytecode metadata",
        )));
    }
    if function.local_definitions().len() != usize::from(function.metadata().local_count) {
        return Err(RuntimeError::Engine(Error::internal(
            "local definition count does not match bytecode metadata",
        )));
    }
    Ok(())
}

pub(super) fn verify_definitions(
    function: &UnlinkedFunction,
    arg_eval_variable_object_local: Option<u16>,
) -> Result<(), RuntimeError> {
    for definition in function.argument_definitions() {
        if definition.kind != ClosureVariableKind::Normal
            || definition.is_lexical
            || definition.is_const
            || definition.is_parameter_initializer
        {
            return Err(RuntimeError::Engine(Error::internal(
                "argument definition is not an ordinary mutable binding",
            )));
        }
    }
    if let Some(layout) = function.parameter_environment() {
        let parameter_definitions = function
            .local_definitions()
            .iter()
            .take(usize::from(
                function.metadata().parameter_environment_local_count,
            ))
            .collect::<Vec<_>>();
        for (index, local) in parameter_definitions.iter().enumerate() {
            if local.kind != ClosureVariableKind::Normal
                || !local.is_lexical
                || local.is_const
                || local.is_parameter_initializer
                || local.name.is_none()
                || parameter_definitions[..index]
                    .iter()
                    .any(|earlier| earlier.name == local.name)
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "parameter environment cell definition is not authenticated",
                )));
            }
        }
        let mut mapped_arguments = vec![false; function.argument_definitions().len()];
        for cell in layout.argument_cells.iter() {
            mapped_arguments[usize::from(cell.argument)] = true;
            let argument = &function.argument_definitions()[usize::from(cell.argument)];
            let local = &function.local_definitions()[usize::from(cell.parameter_local)];
            if argument.name.is_none() || argument.name.as_ref() != local.name.as_ref() {
                return Err(RuntimeError::Engine(Error::internal(
                    "parameter argument cell name disagrees with its physical argument",
                )));
            }
        }
        if function
            .argument_definitions()
            .iter()
            .zip(mapped_arguments)
            .any(|(argument, mapped)| argument.name.is_some() != mapped)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "parameter argument-cell map is not one-to-one with named arguments",
            )));
        }
        for copy in layout.pattern_copies.iter() {
            let source = &function.local_definitions()[usize::from(copy.parameter_local)];
            let target = &function.local_definitions()[usize::from(copy.body_local)];
            if target.kind != ClosureVariableKind::Normal
                || target.is_lexical
                || target.is_const
                || source.is_parameter_initializer
                || target.is_parameter_initializer
                || source.name.as_ref() != target.name.as_ref()
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "parameter pattern copy definitions are not same-name lexical-to-root storage",
                )));
            }
        }
    }
    if function.parameter_environment().is_some() {
        let mut entry_pc = 0_usize;
        let mut pseudo_rank = 0_u8;
        let mut pseudo_targets = Vec::with_capacity(4);
        while let Some(
            [
                source,
                crate::engine::code::bytecode::Instruction::PutLocal(local),
            ],
        ) = function.code().get(entry_pc..entry_pc + 2)
        {
            let Some((rank, expected_name)) = pseudo_binding_entry(source) else {
                break;
            };
            if rank <= pseudo_rank || pseudo_targets.contains(local) {
                return Err(RuntimeError::Engine(Error::internal(
                    "parameter pseudo-binding prologue is malformed",
                )));
            }
            let definition = function
                .local_definitions()
                .get(usize::from(*local))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "parameter pseudo-binding local is out of bounds",
                    ))
                })?;
            if definition.kind != ClosureVariableKind::Normal
                || definition.is_lexical
                || definition.is_const
                || definition.is_parameter_initializer
                || definition
                    .name
                    .as_ref()
                    .is_none_or(|name| name.utf16_units().ne(expected_name.encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "parameter pseudo-binding definition is not authenticated",
                )));
            }
            pseudo_rank = rank;
            pseudo_targets.push(*local);
            entry_pc += 2;
        }
    }
    for (index, definition) in function.local_definitions().iter().enumerate() {
        let is_function_name = function.metadata().function_name_local == u16::try_from(index).ok();
        let is_derived_this = function.metadata().derived_this_local == u16::try_from(index).ok();
        let is_active_function =
            function.metadata().active_function_local == u16::try_from(index).ok();
        let is_eval_variable_object =
            function.metadata().eval_variable_object_local == u16::try_from(index).ok();
        let is_arg_eval_variable_object =
            arg_eval_variable_object_local == u16::try_from(index).ok();
        if is_function_name {
            if definition.kind != ClosureVariableKind::FunctionName
                || definition.is_lexical
                || definition.is_const != function.metadata().strict
                || definition.name.as_ref() != function.func_name()
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "function-name definition disagrees with bytecode metadata",
                )));
            }
        } else if is_derived_this {
            if definition.kind != ClosureVariableKind::Normal
                || !definition.is_lexical
                || definition.is_const
                || definition.is_parameter_initializer
                || definition
                    .name
                    .as_ref()
                    .is_none_or(|name| name.utf16_units().ne("<this>".encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "derived this definition disagrees with bytecode metadata",
                )));
            }
        } else if is_active_function {
            if definition.kind != ClosureVariableKind::Normal
                || definition.is_lexical
                || definition.is_const
                || definition.is_parameter_initializer
                || definition
                    .name
                    .as_ref()
                    .is_none_or(|name| name.utf16_units().ne("<this_active_func>".encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "active-function definition disagrees with bytecode metadata",
                )));
            }
        } else if is_eval_variable_object {
            if definition.kind != ClosureVariableKind::EvalVariableObject
                || definition.is_lexical
                || definition.is_const
                || definition
                    .name
                    .as_ref()
                    .is_none_or(|name| name.utf16_units().ne("<var>".encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval variable-object definition disagrees with bytecode metadata",
                )));
            }
        } else if is_arg_eval_variable_object {
            if definition.kind != ClosureVariableKind::ArgEvalVariableObject
                || definition.is_lexical
                || definition.is_const
                || definition
                    .name
                    .as_ref()
                    .is_none_or(|name| name.utf16_units().ne("<arg_var>".encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "parameter eval variable-object definition disagrees with its layout",
                )));
            }
        } else if definition.kind == ClosureVariableKind::WithObject {
            if function.metadata().strict
                || definition.is_lexical
                || definition.is_const
                || definition
                    .name
                    .as_ref()
                    .is_none_or(|name| name.utf16_units().ne("<with>".encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "strict or malformed bytecode contains a with-object local",
                )));
            }
        } else if definition.kind != ClosureVariableKind::Normal && !definition.kind.is_private() {
            return Err(RuntimeError::Engine(Error::internal(
                "ordinary local definition uses a non-local binding kind",
            )));
        } else if definition.is_const && !definition.is_lexical {
            return Err(RuntimeError::Engine(Error::internal(
                "a const local definition must also be lexical",
            )));
        }
    }
    if function.parameter_environment().is_none() {
        let mut variable_environment_pc = 0_usize;
        while function
            .code()
            .get(variable_environment_pc..variable_environment_pc + 2)
            .is_some_and(|pair| {
                matches!(pair, [source, crate::engine::code::bytecode::Instruction::PutLocal(_)]
                if pseudo_binding_entry(source).is_some())
            })
        {
            variable_environment_pc += 2;
        }
        if matches!(
            function.code().get(variable_environment_pc),
            Some(crate::engine::code::bytecode::Instruction::Arguments(_))
        ) && matches!(
            function.code().get(variable_environment_pc + 1),
            Some(crate::engine::code::bytecode::Instruction::PutLocal(_))
        ) {
            variable_environment_pc += 2;
        }
        match function.metadata().eval_variable_object_local {
            Some(index)
                if matches!(
                    function
                        .code()
                        .get(variable_environment_pc..variable_environment_pc + 2),
                    Some([
                        crate::engine::code::bytecode::Instruction::VariableEnvironment,
                        crate::engine::code::bytecode::Instruction::PutLocal(target),
                    ]) if *target == index
                ) && function.code().iter().enumerate().all(|(pc, instruction)| {
                    pc == variable_environment_pc
                        || !matches!(
                            instruction,
                            crate::engine::code::bytecode::Instruction::VariableEnvironment
                        )
                }) => {}
            Some(_) => {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval variable-object local has no exact entry prologue",
                )));
            }
            None if function.code().iter().any(|instruction| {
                matches!(
                    instruction,
                    crate::engine::code::bytecode::Instruction::VariableEnvironment
                )
            }) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "variable-environment opcode has no authenticated local",
                )));
            }
            None => {}
        }
    }
    Ok(())
}
