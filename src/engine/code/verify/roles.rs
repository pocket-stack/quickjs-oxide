//! Authenticate function roles against the actual publication entry point.
use super::RootPublication;
use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::UnlinkedFunction;
use crate::engine::code::function::metadata::{
    ClosureVariableKind, ConstructorKind, EvalKind, FunctionKind,
};

pub(super) fn verify(
    function: &UnlinkedFunction,
    is_root: bool,
    root_publication: RootPublication<'_>,
) -> Result<(), RuntimeError> {
    let expected_eval_kind = if is_root {
        match root_publication {
            RootPublication::Script
            | RootPublication::TrustedOrdinaryLeaf
            | RootPublication::Module(_) => EvalKind::None,
            RootPublication::Eval { kind, .. } => kind,
        }
    } else {
        EvalKind::None
    };
    if function.metadata().eval_kind != expected_eval_kind {
        return Err(RuntimeError::Engine(Error::internal(if is_root {
            "root bytecode eval kind disagrees with its publication entry point"
        } else {
            "non-root bytecode carried a synthetic eval kind"
        })));
    }
    if function.metadata().super_call_allowed && !function.metadata().super_allowed {
        return Err(RuntimeError::Engine(Error::internal(
            "bytecode permits super() without SuperProperty",
        )));
    }
    if is_root {
        match root_publication {
            RootPublication::Script => {
                if function.metadata().is_module {
                    return Err(RuntimeError::Engine(Error::internal(
                        "script root retained module authority",
                    )));
                }
                if function.metadata().super_call_allowed || function.metadata().super_allowed {
                    return Err(RuntimeError::Engine(Error::internal(
                        "script root retained a super capability",
                    )));
                }
            }
            RootPublication::TrustedOrdinaryLeaf => {
                let metadata = function.metadata();
                if metadata.is_module
                    || metadata.super_call_allowed
                    || metadata.super_allowed
                    || metadata.arguments_forbidden
                    || metadata.needs_home_object
                    || !metadata.strip_variable_debug
                    || metadata.function_kind != FunctionKind::Normal
                    || !metadata.has_prototype
                    || metadata.constructor_kind != ConstructorKind::Base
                    || metadata.function_name_local.is_some()
                    || metadata.derived_this_local.is_some()
                    || metadata.active_function_local.is_some()
                    || metadata.eval_variable_object_local.is_some()
                    || function.parameter_environment().is_some()
                    || !function.closure_variables().is_empty()
                    || !function.eval_environments().is_empty()
                    || function.func_name().is_some()
                    || function.debug().is_some()
                    || function.constants().iter().any(|constant| {
                        !constant.is_plain_primitive() && !constant.is_empty_atom_string()
                    })
                    || function
                        .argument_definitions()
                        .iter()
                        .chain(function.local_definitions())
                        .any(|definition| {
                            definition.name.is_some()
                                || definition.is_lexical
                                || definition.is_const
                                || definition.is_parameter_initializer
                                || definition.kind != ClosureVariableKind::Normal
                        })
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "trusted ordinary leaf metadata disagrees with its publication entry point",
                    )));
                }
            }
            RootPublication::Module(module) => {
                if !function.metadata().is_module
                    || !function.metadata().strict
                    || function.metadata().function_kind != FunctionKind::Async
                    || module.has_top_level_await()
                        != function
                            .code()
                            .iter()
                            .any(|instruction| matches!(instruction, Instruction::Await))
                    || function.metadata().super_call_allowed
                    || function.metadata().super_allowed
                    || function.metadata().arguments_forbidden
                    || !std::ptr::eq(function, module.function())
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "module root metadata disagrees with its publication entry point",
                    )));
                }
            }
            RootPublication::Eval {
                kind,
                expected_capabilities,
                ..
            } => {
                if (
                    function.metadata().super_call_allowed,
                    function.metadata().super_allowed,
                ) != (
                    expected_capabilities.super_call_allowed,
                    expected_capabilities.super_allowed,
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval root super capability disagrees with its caller",
                    )));
                }
                if kind == EvalKind::Indirect
                    && (function.metadata().super_call_allowed || function.metadata().super_allowed)
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "indirect eval root retained a super capability",
                    )));
                }
                if function.metadata().arguments_forbidden
                    != expected_capabilities.arguments_forbidden
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval root arguments capability disagrees with its caller",
                    )));
                }
                if kind == EvalKind::Indirect && function.metadata().arguments_forbidden {
                    return Err(RuntimeError::Engine(Error::internal(
                        "indirect eval root retained an arguments restriction",
                    )));
                }
                if function.metadata().is_module {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval root retained module authority",
                    )));
                }
            }
        }
    }
    if !is_root && function.metadata().is_module {
        return Err(RuntimeError::Engine(Error::internal(
            "nested function retained module authority",
        )));
    }
    if is_root
        && matches!(
            root_publication,
            RootPublication::Eval {
                kind: EvalKind::Direct,
                caller_strict: true,
                ..
            }
        )
        && !function.metadata().strict
    {
        return Err(RuntimeError::Engine(Error::internal(
            "direct eval root lost inherited caller strictness",
        )));
    }
    Ok(())
}
