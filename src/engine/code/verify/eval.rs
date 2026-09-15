//! Authentication of eval environments, source permissions and scope topology.
use super::{eval_variable_object_sentinel, unlinked_closure_name};
use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::bytecode::{
    DynamicEnvironmentSource, EvalVariableSource, WithObjectSource,
};
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariableKind, EvalCallerProfile, EvalCallerVariableTarget,
    EvalEnvironment, EvalKind, EvalRootBinding,
};
use crate::engine::code::function::{UnlinkedConstant, UnlinkedFunction};
use crate::engine::value::JsString;

#[derive(Clone, Copy)]
enum SuperPseudoRole {
    DerivedThis,
    ActiveFunction,
    NewTarget,
}

pub(super) fn verify_eval_super_pseudo_bindings(
    environment: &EvalEnvironment<JsString>,
    derived_this_local: Option<u16>,
    active_function_local: Option<u16>,
    new_target_local: Option<u16>,
    derived_this_origins: &[bool],
    active_function_origins: &[bool],
    new_target_origins: &[bool],
) -> Result<(), RuntimeError> {
    // Eval scopes are ordered from innermost to outermost. Only the first
    // binding for each private role can be resolved by the eval; same-named
    // entries in later ancestor function segments are shadowed and therefore
    // cannot grant authority.
    let mut seen_roles = [false; 3];
    for binding in environment
        .scopes
        .iter()
        .flat_map(|scope| scope.bindings.iter())
    {
        let role = if binding.is_lexical
            && !binding.is_const
            && binding.kind == ClosureVariableKind::Normal
            && !binding.is_catch_parameter
            && binding.name.utf16_units().eq("<this>".encode_utf16())
        {
            Some(SuperPseudoRole::DerivedThis)
        } else if !binding.is_lexical
            && !binding.is_const
            && binding.kind == ClosureVariableKind::Normal
            && !binding.is_catch_parameter
            && binding
                .name
                .utf16_units()
                .eq("<this_active_func>".encode_utf16())
        {
            Some(SuperPseudoRole::ActiveFunction)
        } else if !binding.is_lexical
            && !binding.is_const
            && binding.kind == ClosureVariableKind::Normal
            && !binding.is_catch_parameter
            && binding.name.utf16_units().eq("<new.target>".encode_utf16())
        {
            Some(SuperPseudoRole::NewTarget)
        } else {
            None
        };
        let Some(role) = role else {
            continue;
        };
        let role_index = match role {
            SuperPseudoRole::DerivedThis => 0,
            SuperPseudoRole::ActiveFunction => 1,
            SuperPseudoRole::NewTarget => 2,
        };
        if seen_roles[role_index] {
            continue;
        }
        seen_roles[role_index] = true;
        let authenticated = match (role, binding.source) {
            (
                SuperPseudoRole::DerivedThis,
                crate::engine::code::function::metadata::EvalBindingSource::Local(index),
            ) => derived_this_local == Some(index),
            (
                SuperPseudoRole::ActiveFunction,
                crate::engine::code::function::metadata::EvalBindingSource::Local(index),
            ) => active_function_local == Some(index),
            (
                SuperPseudoRole::NewTarget,
                crate::engine::code::function::metadata::EvalBindingSource::Local(index),
            ) => new_target_local == Some(index),
            (
                SuperPseudoRole::DerivedThis,
                crate::engine::code::function::metadata::EvalBindingSource::Closure(index),
            ) => derived_this_origins
                .get(usize::from(index))
                .copied()
                .unwrap_or(false),
            (
                SuperPseudoRole::ActiveFunction,
                crate::engine::code::function::metadata::EvalBindingSource::Closure(index),
            ) => active_function_origins
                .get(usize::from(index))
                .copied()
                .unwrap_or(false),
            (
                SuperPseudoRole::NewTarget,
                crate::engine::code::function::metadata::EvalBindingSource::Closure(index),
            ) => new_target_origins
                .get(usize::from(index))
                .copied()
                .unwrap_or(false),
            (_, crate::engine::code::function::metadata::EvalBindingSource::Argument(_)) => false,
        };
        if !authenticated {
            return Err(RuntimeError::Engine(Error::internal(
                "eval super pseudo binding did not originate from its authenticated source",
            )));
        }
    }
    if seen_roles != [true; 3] {
        return Err(RuntimeError::Engine(Error::internal(
            "eval super capability is missing an authenticated pseudo binding",
        )));
    }
    Ok(())
}

pub(super) fn eval_variable_object_local_kind(
    function: &UnlinkedFunction,
    index: u16,
) -> Option<ClosureVariableKind> {
    if function.metadata().eval_variable_object_local == Some(index) {
        return Some(ClosureVariableKind::EvalVariableObject);
    }
    if function
        .parameter_environment()
        .and_then(|layout| layout.arg_eval_variable_object_local)
        == Some(index)
    {
        return Some(ClosureVariableKind::ArgEvalVariableObject);
    }
    None
}

pub(super) fn verify_eval_variable_source(
    function: &UnlinkedFunction,
    source: EvalVariableSource,
) -> Result<(), RuntimeError> {
    match source {
        EvalVariableSource::Local(index) => {
            let kind = eval_variable_object_local_kind(function, index);
            if kind.is_none()
                || function
                    .local_definitions()
                    .get(usize::from(index))
                    .is_none_or(|definition| {
                        Some(definition.kind) != kind
                            || definition.is_lexical
                            || definition.is_const
                            || eval_variable_object_sentinel(definition.kind).is_none_or(
                                |sentinel| {
                                    definition.name.as_ref().is_none_or(|name| {
                                        name.utf16_units().ne(sentinel.encode_utf16())
                                    })
                                },
                            )
                    })
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval variable opcode did not reference the authenticated local",
                )));
            }
        }
        EvalVariableSource::Closure(index) => {
            if function
                .closure_variables()
                .get(usize::from(index))
                .is_none_or(|descriptor| {
                    !descriptor.kind.is_eval_variable_object()
                        || descriptor.is_lexical
                        || descriptor.is_const
                        || !matches!(
                            descriptor.source,
                            ClosureSource::ParentLocal(_)
                                | ClosureSource::ParentClosure(_)
                                | ClosureSource::EvalEnvironment(_)
                        )
                })
                || function
                    .closure_variables()
                    .get(usize::from(index))
                    .is_some_and(|descriptor| {
                        eval_variable_object_sentinel(descriptor.kind).is_none_or(|sentinel| {
                            unlinked_closure_name(function, descriptor)
                                .ok()
                                .flatten()
                                .is_none_or(|name| name.utf16_units().ne(sentinel.encode_utf16()))
                        })
                    })
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval variable opcode did not reference an authenticated closure",
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn verify_with_object_source(
    function: &UnlinkedFunction,
    source: WithObjectSource,
) -> Result<(), RuntimeError> {
    match source {
        WithObjectSource::Local(index) => {
            let definition = function
                .local_definitions()
                .get(usize::from(index))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "with-object dynamic source local is out of bounds",
                    ))
                })?;
            if definition.kind != ClosureVariableKind::WithObject
                || definition.is_lexical
                || definition.is_const
                || definition
                    .name
                    .as_ref()
                    .is_none_or(|name| name.utf16_units().ne("<with>".encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "with-object dynamic source did not reference the authenticated local",
                )));
            }
        }
        WithObjectSource::Closure(index) => {
            let descriptor = function
                .closure_variables()
                .get(usize::from(index))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "with-object dynamic source closure is out of bounds",
                    ))
                })?;
            if descriptor.kind != ClosureVariableKind::WithObject
                || descriptor.is_lexical
                || descriptor.is_const
                || !matches!(
                    descriptor.source,
                    ClosureSource::ParentLocal(_)
                        | ClosureSource::ParentClosure(_)
                        | ClosureSource::EvalEnvironment(_)
                )
                || unlinked_closure_name(function, descriptor)?
                    .is_none_or(|name| name.utf16_units().ne("<with>".encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "with-object dynamic source did not reference an authenticated closure",
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn verify_dynamic_environment_source(
    function: &UnlinkedFunction,
    source: DynamicEnvironmentSource,
) -> Result<(), RuntimeError> {
    match source {
        DynamicEnvironmentSource::Eval(source) => verify_eval_variable_source(function, source),
        DynamicEnvironmentSource::With(source) => verify_with_object_source(function, source),
    }
}

pub(super) fn verify_unlinked_string_constant(
    function: &UnlinkedFunction,
    index: u32,
    diagnostic: &'static str,
) -> Result<(), RuntimeError> {
    let index = usize::try_from(index)
        .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
    if !matches!(
        function
            .constants()
            .get(index)
            .and_then(UnlinkedConstant::as_primitive),
        Some(crate::engine::value::PrimitiveValue::String(_))
    ) {
        return Err(RuntimeError::Engine(Error::internal(diagnostic)));
    }
    Ok(())
}

pub(super) fn verify_eval_scope_topology(
    environment: &EvalEnvironment<JsString>,
    function_depth: usize,
    synthetic_eval_tree: bool,
    module_root: bool,
    imported_segment_count: usize,
    imported_scope_start: usize,
) -> Result<usize, RuntimeError> {
    if environment.scopes.is_empty() {
        return Err(RuntimeError::Engine(Error::internal(
            "eval environment contains no scopes",
        )));
    }

    let mut segment_start = 0;
    let mut segment_count = 0;
    let mut first_function_anchor = None;
    while segment_start < environment.scopes.len() {
        let function_anchor = environment.scopes[segment_start..]
            .iter()
            .position(|scope| {
                matches!(
                    scope.kind,
                    crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot
                        | crate::engine::code::function::metadata::EvalScopeKind::Parameter
                )
            })
            .map(|offset| segment_start + offset)
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "eval scope segment contains no function anchor",
                ))
            })?;
        first_function_anchor.get_or_insert(function_anchor);

        let final_segment = function_anchor + 1 == environment.scopes.len();
        let synthetic_root_segment = synthetic_eval_tree && segment_count == function_depth;
        let imported_segment = synthetic_eval_tree && segment_start >= imported_scope_start;
        match environment.scopes[function_anchor].kind {
            crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot => {
                let expected_body = if final_segment || synthetic_root_segment {
                    crate::engine::code::function::metadata::EvalScopeKind::ProgramBody
                } else {
                    crate::engine::code::function::metadata::EvalScopeKind::FunctionBody
                };
                if function_anchor == segment_start
                    || (!imported_segment
                        && environment.scopes[function_anchor - 1].kind != expected_body)
                    || (imported_segment
                        && !matches!(
                            environment.scopes[function_anchor - 1].kind,
                            crate::engine::code::function::metadata::EvalScopeKind::FunctionBody
                                | crate::engine::code::function::metadata::EvalScopeKind::ProgramBody
                        ))
                {
                    return Err(RuntimeError::Engine(Error::internal(format!(
                        "eval scope segment {segment_count} has the wrong body scope {:?}, expected {expected_body:?} at function depth {function_depth}",
                        function_anchor
                            .checked_sub(1)
                            .and_then(|index| environment.scopes.get(index))
                            .map(|scope| scope.kind),
                    ))));
                }
            }
            crate::engine::code::function::metadata::EvalScopeKind::Parameter => {
                if synthetic_root_segment
                    || environment.scopes[segment_start..function_anchor]
                        .iter()
                        .any(|scope| {
                            matches!(
                                scope.kind,
                                crate::engine::code::function::metadata::EvalScopeKind::FunctionBody
                                    | crate::engine::code::function::metadata::EvalScopeKind::ProgramBody
                            )
                        })
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "parameter eval scope segment exposed a body scope",
                    )));
                }
            }
            _ => unreachable!("function anchor kind was selected above"),
        }
        let body_exclusive_end = if environment.scopes[function_anchor].kind
            == crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot
        {
            function_anchor.saturating_sub(1)
        } else {
            function_anchor
        };
        if environment.scopes[segment_start..body_exclusive_end]
            .iter()
            .any(|scope| {
                matches!(
                    scope.kind,
                    crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot
                        | crate::engine::code::function::metadata::EvalScopeKind::Parameter
                        | crate::engine::code::function::metadata::EvalScopeKind::FunctionBody
                        | crate::engine::code::function::metadata::EvalScopeKind::ProgramBody
                )
            })
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval scope segment contains a misplaced body scope",
            )));
        }

        for binding in environment.scopes[segment_start..=function_anchor]
            .iter()
            .flat_map(|scope| scope.bindings.iter())
        {
            let source_matches_segment = if segment_count == 0 && module_root {
                matches!(
                    binding.source,
                    crate::engine::code::function::metadata::EvalBindingSource::Local(_)
                        | crate::engine::code::function::metadata::EvalBindingSource::Closure(_)
                )
            } else if segment_count == 0 {
                matches!(
                    binding.source,
                    crate::engine::code::function::metadata::EvalBindingSource::Local(_)
                        | crate::engine::code::function::metadata::EvalBindingSource::Argument(_)
                )
            } else {
                matches!(
                    binding.source,
                    crate::engine::code::function::metadata::EvalBindingSource::Closure(_)
                )
            };
            if !source_matches_segment {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval binding source does not match its function scope segment",
                )));
            }
        }

        segment_count += 1;
        segment_start = function_anchor + 1;
    }
    let expected_segments = function_depth
        .checked_add(1)
        .and_then(|count| count.checked_add(imported_segment_count))
        .ok_or_else(|| {
            RuntimeError::Engine(Error::internal(
                "eval environment function depth overflowed",
            ))
        })?;
    if segment_count != expected_segments {
        return Err(RuntimeError::Engine(Error::internal(
            "eval environment segment count disagrees with its function-tree depth",
        )));
    }
    first_function_anchor.ok_or_else(|| {
        RuntimeError::Engine(Error::internal(
            "eval environment contains no function anchor scope",
        ))
    })
}

pub(super) fn verify_eval_imported_suffix(
    environment: &EvalEnvironment<JsString>,
    closure_origins: &[Option<u16>],
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_profile: &EvalCallerProfile,
) -> Result<(usize, usize), RuntimeError> {
    let suffix_start = environment
        .scopes
        .len()
        .checked_sub(expected_profile.scope_kinds.len())
        .ok_or_else(|| {
            RuntimeError::Engine(Error::internal(
                "eval environment is shorter than its imported caller profile",
            ))
        })?;
    for (scope_index, (&expected_kind, actual_scope)) in expected_profile
        .scope_kinds
        .iter()
        .zip(&environment.scopes[suffix_start..])
        .enumerate()
    {
        if actual_scope.kind != expected_kind {
            return Err(RuntimeError::Engine(Error::internal(
                "eval imported scope kind disagrees with its caller profile",
            )));
        }
        let scope_index = u16::try_from(scope_index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "eval imported scope index exceeds bytecode range",
            ))
        })?;
        let expected = expected_bindings
            .iter()
            .enumerate()
            .filter(|(_, binding)| binding.scope == scope_index)
            .collect::<Vec<_>>();
        if actual_scope.bindings.len() != expected.len() {
            return Err(RuntimeError::Engine(Error::internal(
                "eval imported scope binding count disagrees with its caller profile",
            )));
        }
        for (actual, (expected_index, expected)) in actual_scope.bindings.iter().zip(expected) {
            if actual.name != expected.name
                || actual.is_lexical != expected.is_lexical
                || actual.is_const != expected.is_const
                || actual.kind != expected.kind
                || actual.is_catch_parameter != expected.is_catch_parameter
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval imported binding metadata disagrees with its caller profile",
                )));
            }
            let crate::engine::code::function::metadata::EvalBindingSource::Closure(closure) =
                actual.source
            else {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval imported binding did not use a closure relay",
                )));
            };
            let expected_index = u16::try_from(expected_index).map_err(|_| {
                RuntimeError::Engine(Error::internal(
                    "eval imported binding index exceeds bytecode range",
                ))
            })?;
            if closure_origins.get(usize::from(closure)).copied().flatten() != Some(expected_index)
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval imported binding closure has the wrong caller origin",
                )));
            }
        }
    }
    Ok((
        expected_profile
            .scope_kinds
            .iter()
            .filter(|kind| {
                matches!(
                    **kind,
                    crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot
                        | crate::engine::code::function::metadata::EvalScopeKind::Parameter
                )
            })
            .count(),
        suffix_start,
    ))
}

pub(super) fn verify_eval_environments(
    function: &UnlinkedFunction,
    function_depth: usize,
    captured_locals: &mut [bool],
    synthetic_eval_tree: bool,
    closure_origins: &[Option<u16>],
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_profile: Option<&EvalCallerProfile>,
) -> Result<(), RuntimeError> {
    let is_root = function_depth == 0;
    let synthetic_eval_root = is_root && function.metadata().eval_kind != EvalKind::None;
    for environment in function.eval_environments() {
        if environment.caller_strict != function.metadata().strict {
            return Err(RuntimeError::Engine(Error::internal(
                "eval environment strictness disagrees with bytecode metadata",
            )));
        }
        if environment.super_call_allowed && !environment.super_allowed {
            return Err(RuntimeError::Engine(Error::internal(
                "eval environment permits super() without SuperProperty",
            )));
        }
        if (environment.super_call_allowed, environment.super_allowed)
            != (
                function.metadata().super_call_allowed,
                function.metadata().super_allowed,
            )
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval environment super capability disagrees with bytecode metadata",
            )));
        }
        let (imported_segment_count, imported_scope_start) =
            if let Some(expected_profile) = expected_profile {
                verify_eval_imported_suffix(
                    environment,
                    closure_origins,
                    expected_bindings,
                    expected_profile,
                )?
            } else {
                (0, environment.scopes.len())
            };
        let first_function_anchor = verify_eval_scope_topology(
            environment,
            function_depth,
            synthetic_eval_tree,
            function.metadata().is_module,
            imported_segment_count,
            imported_scope_start,
        )?;
        let first_function_anchor = u16::try_from(first_function_anchor).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "eval function anchor index exceeds bytecode range",
            ))
        })?;
        match environment.variable_environment {
            crate::engine::code::function::metadata::EvalVariableEnvironment::Global => {
                // Authored Script code always resolves its caller variable
                // environment through the global Program segment, even when
                // the Script itself is strict. A synthetic strict eval root
                // must instead own a StrictLocal destination.
                if function.metadata().is_module
                    || !is_root
                    || (environment.caller_strict
                        && function.metadata().eval_kind != EvalKind::None)
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "global eval variable environment escaped an authored Script root",
                    )));
                }
                if environment.scopes[..usize::from(first_function_anchor)]
                    .iter()
                    .find(|scope| {
                        matches!(
                            scope.kind,
                            crate::engine::code::function::metadata::EvalScopeKind::FunctionBody
                                | crate::engine::code::function::metadata::EvalScopeKind::ProgramBody
                        )
                    })
                    .is_none_or(|scope| {
                        scope.kind != crate::engine::code::function::metadata::EvalScopeKind::ProgramBody
                    })
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "global eval variable environment has no current Program body scope",
                    )));
                }
            }
            crate::engine::code::function::metadata::EvalVariableEnvironment::StrictLocal(
                index,
            ) => {
                if !environment.caller_strict || index != first_function_anchor {
                    return Err(RuntimeError::Engine(Error::internal(
                        "strict eval variable environment has the wrong function anchor",
                    )));
                }
                if is_root
                    && function.metadata().eval_kind == EvalKind::None
                    && !function.metadata().is_module
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "authored Script eval environment used a non-canonical strict-local target",
                    )));
                }
                let anchor = &environment.scopes[usize::from(index)];
                if !matches!(
                    anchor.kind,
                    crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot
                        | crate::engine::code::function::metadata::EvalScopeKind::Parameter
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "strict eval variable environment selected a non-function scope",
                    )));
                }
            }
            crate::engine::code::function::metadata::EvalVariableEnvironment::VariableObject {
                scope,
                source,
            } => {
                if environment.caller_strict {
                    return Err(RuntimeError::Engine(Error::internal(
                        "strict eval environment selected a variable object",
                    )));
                }
                let target_matches_function_segment = if synthetic_eval_root {
                    function.metadata().eval_kind == EvalKind::Direct
                        && usize::from(scope) >= imported_scope_start
                        && matches!(
                            source,
                            crate::engine::code::function::metadata::EvalBindingSource::Closure(_)
                        )
                } else {
                    scope == first_function_anchor
                        && matches!(
                            source,
                            crate::engine::code::function::metadata::EvalBindingSource::Local(_)
                        )
                };
                if !target_matches_function_segment {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval variable object selected the wrong current function segment",
                    )));
                }
                let target_scope = environment.scopes.get(usize::from(scope)).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "eval variable-object scope is out of bounds",
                    ))
                })?;
                let expected_kind = match target_scope.kind {
                    crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot => {
                        ClosureVariableKind::EvalVariableObject
                    }
                    crate::engine::code::function::metadata::EvalScopeKind::Parameter => {
                        ClosureVariableKind::ArgEvalVariableObject
                    }
                    _ => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "eval variable object selected a non-function scope",
                        )));
                    }
                };
                if matches!(
                    source,
                    crate::engine::code::function::metadata::EvalBindingSource::Argument(_)
                ) || target_scope
                    .bindings
                    .iter()
                    .filter(|binding| {
                        binding.source == source
                            && binding.kind == expected_kind
                            && !binding.is_lexical
                            && !binding.is_const
                            && !binding.is_catch_parameter
                    })
                    .count()
                    != 1
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval variable-object target is not exact",
                    )));
                }
                match source {
                    crate::engine::code::function::metadata::EvalBindingSource::Local(index)
                        if eval_variable_object_local_kind(function, index)
                            == Some(expected_kind) => {}
                    crate::engine::code::function::metadata::EvalBindingSource::Closure(index) => {
                        let descriptor = function
                            .closure_variables()
                            .get(usize::from(index))
                            .ok_or_else(|| {
                                RuntimeError::Engine(Error::internal(
                                    "eval variable-object closure is out of bounds",
                                ))
                            })?;
                        if descriptor.kind != expected_kind
                            || descriptor.is_lexical
                            || descriptor.is_const
                        {
                            return Err(RuntimeError::Engine(Error::internal(
                                "eval variable-object closure role is not authenticated",
                            )));
                        }
                    }
                    crate::engine::code::function::metadata::EvalBindingSource::Local(_)
                    | crate::engine::code::function::metadata::EvalBindingSource::Argument(_) => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "eval variable-object source is not authenticated",
                        )));
                    }
                }
            }
        }

        if synthetic_eval_root {
            let expected_profile = expected_profile.ok_or_else(|| {
                RuntimeError::Engine(Error::internal("synthetic eval root has no caller profile"))
            })?;
            let variable_target_matches = if function.metadata().strict {
                matches!(
                    environment.variable_environment,
                    crate::engine::code::function::metadata::EvalVariableEnvironment::StrictLocal(actual)
                        if actual == first_function_anchor
                )
            } else {
                match (
                    expected_profile.variable_target,
                    environment.variable_environment,
                ) {
                    (
                        EvalCallerVariableTarget::Global,
                        crate::engine::code::function::metadata::EvalVariableEnvironment::Global,
                    ) => true,
                    (
                        EvalCallerVariableTarget::ExternalBinding(expected),
                        crate::engine::code::function::metadata::EvalVariableEnvironment::VariableObject {
                            source: crate::engine::code::function::metadata::EvalBindingSource::Closure(actual),
                            ..
                        },
                    ) => {
                        closure_origins.get(usize::from(actual)).copied().flatten()
                            == Some(expected)
                    }
                    _ => false,
                }
            };
            if !variable_target_matches {
                return Err(RuntimeError::Engine(Error::internal(
                    "nested eval variable target disagrees with its caller profile",
                )));
            }
        }

        for scope in &environment.scopes {
            if scope.kind == crate::engine::code::function::metadata::EvalScopeKind::With
                && scope.bindings.len() != 1
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval with scope does not contain exactly one object binding",
                )));
            }
            for binding in &scope.bindings {
                if binding.name.is_empty() {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval binding has an empty name",
                    )));
                }
                let is_catch_scope =
                    scope.kind == crate::engine::code::function::metadata::EvalScopeKind::Catch;
                let is_with_scope =
                    scope.kind == crate::engine::code::function::metadata::EvalScopeKind::With;
                if binding.is_catch_parameter
                    && (!is_catch_scope
                        || !binding.is_lexical
                        || binding.is_const
                        || binding.kind != ClosureVariableKind::Normal)
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval catch binding metadata disagrees with its scope",
                    )));
                }
                if (binding.kind == ClosureVariableKind::WithObject) != is_with_scope
                    || (binding.kind == ClosureVariableKind::WithObject
                        && (binding.is_lexical
                            || binding.is_const
                            || binding.is_catch_parameter
                            || binding.name.utf16_units().ne("<with>".encode_utf16())
                            || matches!(
                                binding.source,
                                crate::engine::code::function::metadata::EvalBindingSource::Argument(_)
                            )))
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval with-object binding metadata disagrees with its scope",
                    )));
                }
                if binding.kind.is_eval_variable_object() {
                    let role_allowed = match scope.kind {
                        crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot => {
                            true
                        }
                        crate::engine::code::function::metadata::EvalScopeKind::Parameter => {
                            binding.kind == ClosureVariableKind::ArgEvalVariableObject
                        }
                        _ => false,
                    };
                    if !role_allowed
                        || binding.is_lexical
                        || binding.is_const
                        || binding.is_catch_parameter
                        || matches!(
                            binding.source,
                            crate::engine::code::function::metadata::EvalBindingSource::Argument(_)
                        )
                        || eval_variable_object_sentinel(binding.kind).is_none_or(|sentinel| {
                            binding.name.utf16_units().ne(sentinel.encode_utf16())
                        })
                    {
                        return Err(RuntimeError::Engine(Error::internal(
                            "eval variable-object binding metadata disagrees with its scope",
                        )));
                    }
                }
                let expected = match binding.source {
                    crate::engine::code::function::metadata::EvalBindingSource::Local(index) => {
                        let definition = function
                            .local_definitions()
                            .get(usize::from(index))
                            .ok_or_else(|| {
                                RuntimeError::Engine(Error::internal(
                                    "eval binding local source is out of bounds",
                                ))
                            })?;
                        if definition.name.as_ref() != Some(&binding.name) {
                            return Err(RuntimeError::Engine(Error::internal(
                                "eval binding name disagrees with its local definition",
                            )));
                        }
                        let captured =
                            captured_locals.get_mut(usize::from(index)).ok_or_else(|| {
                                RuntimeError::Engine(Error::internal(
                                    "eval binding local source is out of bounds",
                                ))
                            })?;
                        *captured = true;
                        (definition.is_lexical, definition.is_const, definition.kind)
                    }
                    crate::engine::code::function::metadata::EvalBindingSource::Argument(index) => {
                        let definition = function
                            .argument_definitions()
                            .get(usize::from(index))
                            .ok_or_else(|| {
                                RuntimeError::Engine(Error::internal(
                                    "eval binding argument source is out of bounds",
                                ))
                            })?;
                        if definition.name.as_ref() != Some(&binding.name) {
                            return Err(RuntimeError::Engine(Error::internal(
                                "eval binding name disagrees with its argument definition",
                            )));
                        }
                        (definition.is_lexical, definition.is_const, definition.kind)
                    }
                    crate::engine::code::function::metadata::EvalBindingSource::Closure(index) => {
                        let descriptor = function
                            .closure_variables()
                            .get(usize::from(index))
                            .ok_or_else(|| {
                                RuntimeError::Engine(Error::internal(
                                    "eval binding closure source is out of bounds",
                                ))
                            })?;
                        if matches!(
                            descriptor.source,
                            ClosureSource::GlobalDeclaration
                                | ClosureSource::Global
                                | ClosureSource::ParentGlobal(_)
                        ) {
                            return Err(RuntimeError::Engine(Error::internal(
                                "eval binding referenced a global closure descriptor",
                            )));
                        }
                        let descriptor_name = unlinked_closure_name(function, descriptor)?;
                        if descriptor_name != Some(&binding.name) {
                            return Err(RuntimeError::Engine(Error::internal(
                                "eval binding name disagrees with its closure descriptor",
                            )));
                        }
                        (descriptor.is_lexical, descriptor.is_const, descriptor.kind)
                    }
                };
                if expected != (binding.is_lexical, binding.is_const, binding.kind) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval binding flags disagree with its source definition",
                    )));
                }
            }
        }
    }
    Ok(())
}
