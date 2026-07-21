//! Validation and iterative flattening for unlinked bytecode publication.

use super::*;
use std::collections::HashSet;

use crate::bytecode::{
    DynamicEnvironmentSource, EvalVariableSource, MAX_LOCAL_SLOTS, WithObjectSource, verify_parts,
};
use crate::heap::{
    EvalBinding, EvalCallerProfile, EvalCallerVariableTarget, EvalKind, EvalRootBinding, EvalScope,
    validate_parameter_bytecode_layout, validate_pattern_parameter_bytecode_layout,
};

/// Intern every semantically retained direct-eval binding name while keeping
/// the parent publication routine's atom transaction authoritative. The
/// caller releases `auxiliary_atoms` on any later failure and transfers the
/// complete list to the bytecode node on success.
pub(super) fn link_eval_environments(
    state: &mut RuntimeState,
    environments: Vec<EvalEnvironment<JsString>>,
    auxiliary_atoms: &mut Vec<Atom>,
) -> Result<Vec<EvalEnvironment<Atom>>, RuntimeError> {
    let mut linked_environments = Vec::with_capacity(environments.len());
    for environment in environments {
        let mut linked_scopes = Vec::with_capacity(environment.scopes.len());
        for scope in environment.scopes {
            let mut linked_bindings = Vec::with_capacity(scope.bindings.len());
            for binding in scope.bindings {
                let name = state.atoms.intern_property_key_js_string(&binding.name)?;
                auxiliary_atoms.push(name);
                linked_bindings.push(EvalBinding {
                    name,
                    source: binding.source,
                    is_lexical: binding.is_lexical,
                    is_const: binding.is_const,
                    kind: binding.kind,
                    is_catch_parameter: binding.is_catch_parameter,
                });
            }
            linked_scopes.push(EvalScope {
                kind: scope.kind,
                bindings: linked_bindings.into_boxed_slice(),
            });
        }
        linked_environments.push(EvalEnvironment {
            scopes: linked_scopes.into_boxed_slice(),
            variable_environment: environment.variable_environment,
            caller_strict: environment.caller_strict,
            super_call_allowed: environment.super_call_allowed,
            super_allowed: environment.super_allowed,
        });
    }
    Ok(linked_environments)
}

fn unlinked_closure_name<'a>(
    function: &'a UnlinkedFunction,
    descriptor: &ClosureVariable,
) -> Result<Option<&'a JsString>, RuntimeError> {
    match descriptor.name {
        ClosureVariableName::None => Ok(None),
        ClosureVariableName::Constant(index) => {
            let name = usize::try_from(index)
                .ok()
                .and_then(|index| function.constants().get(index))
                .and_then(UnlinkedConstant::as_primitive);
            let Some(Value::String(name)) = name else {
                return Err(RuntimeError::Engine(Error::internal(
                    "closure descriptor referenced a non-string name constant",
                )));
            };
            Ok(Some(name))
        }
        ClosureVariableName::Atom(_) => Err(RuntimeError::Engine(Error::internal(
            "unlinked closure descriptor already contained a runtime atom",
        ))),
    }
}

fn verify_eval_variable_source(
    function: &UnlinkedFunction,
    source: EvalVariableSource,
) -> Result<(), RuntimeError> {
    match source {
        EvalVariableSource::Local(index) => {
            if function.metadata().eval_variable_object_local != Some(index)
                || function
                    .local_definitions()
                    .get(usize::from(index))
                    .is_none_or(|definition| {
                        definition.kind != ClosureVariableKind::EvalVariableObject
                            || definition.is_lexical
                            || definition.is_const
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
                    descriptor.kind != ClosureVariableKind::EvalVariableObject
                        || descriptor.is_lexical
                        || descriptor.is_const
                        || !matches!(
                            descriptor.source,
                            ClosureSource::ParentLocal(_)
                                | ClosureSource::ParentClosure(_)
                                | ClosureSource::EvalEnvironment(_)
                        )
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

fn verify_with_object_source(
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

fn verify_dynamic_environment_source(
    function: &UnlinkedFunction,
    source: DynamicEnvironmentSource,
) -> Result<(), RuntimeError> {
    match source {
        DynamicEnvironmentSource::Eval(source) => verify_eval_variable_source(function, source),
        DynamicEnvironmentSource::With(source) => verify_with_object_source(function, source),
    }
}

fn verify_unlinked_string_constant(
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
        Some(Value::String(_))
    ) {
        return Err(RuntimeError::Engine(Error::internal(diagnostic)));
    }
    Ok(())
}

fn verify_eval_scope_topology(
    environment: &EvalEnvironment<JsString>,
    function_depth: usize,
    synthetic_eval_tree: bool,
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
    let mut first_function_root = None;
    while segment_start < environment.scopes.len() {
        let function_root = environment.scopes[segment_start..]
            .iter()
            .position(|scope| scope.kind == crate::heap::EvalScopeKind::FunctionRoot)
            .map(|offset| segment_start + offset)
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "eval scope segment contains no function root",
                ))
            })?;
        first_function_root.get_or_insert(function_root);

        let final_segment = function_root + 1 == environment.scopes.len();
        let synthetic_root_segment = synthetic_eval_tree && segment_count == function_depth;
        let imported_segment = synthetic_eval_tree
            && function_root
                .checked_sub(1)
                .is_some_and(|body_scope| body_scope >= imported_scope_start);
        let expected_body = if final_segment || synthetic_root_segment {
            crate::heap::EvalScopeKind::ProgramBody
        } else {
            crate::heap::EvalScopeKind::FunctionBody
        };
        if function_root == segment_start
            || (!imported_segment && environment.scopes[function_root - 1].kind != expected_body)
            || (imported_segment
                && !matches!(
                    environment.scopes[function_root - 1].kind,
                    crate::heap::EvalScopeKind::FunctionBody
                        | crate::heap::EvalScopeKind::ProgramBody
                ))
        {
            return Err(RuntimeError::Engine(Error::internal(format!(
                "eval scope segment {segment_count} has the wrong body scope {:?}, expected {expected_body:?} at function depth {function_depth}",
                function_root
                    .checked_sub(1)
                    .and_then(|index| environment.scopes.get(index))
                    .map(|scope| scope.kind),
            ))));
        }
        if environment.scopes[segment_start..function_root - 1]
            .iter()
            .any(|scope| {
                matches!(
                    scope.kind,
                    crate::heap::EvalScopeKind::FunctionRoot
                        | crate::heap::EvalScopeKind::FunctionBody
                        | crate::heap::EvalScopeKind::ProgramBody
                )
            })
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval scope segment contains a misplaced body scope",
            )));
        }

        for binding in environment.scopes[segment_start..=function_root]
            .iter()
            .flat_map(|scope| scope.bindings.iter())
        {
            let source_matches_segment = if segment_count == 0 {
                matches!(
                    binding.source,
                    crate::heap::EvalBindingSource::Local(_)
                        | crate::heap::EvalBindingSource::Argument(_)
                )
            } else {
                matches!(binding.source, crate::heap::EvalBindingSource::Closure(_))
            };
            if !source_matches_segment {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval binding source does not match its function scope segment",
                )));
            }
        }

        segment_count += 1;
        segment_start = function_root + 1;
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
    first_function_root.ok_or_else(|| {
        RuntimeError::Engine(Error::internal(
            "eval environment contains no function root scope",
        ))
    })
}

fn verify_eval_imported_suffix(
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
            let crate::heap::EvalBindingSource::Closure(closure) = actual.source else {
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
            .filter(|kind| **kind == crate::heap::EvalScopeKind::FunctionRoot)
            .count(),
        suffix_start,
    ))
}

fn verify_eval_environments(
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
        let first_function_root = verify_eval_scope_topology(
            environment,
            function_depth,
            synthetic_eval_tree,
            imported_segment_count,
            imported_scope_start,
        )?;
        if synthetic_eval_root {
            let expected_profile = expected_profile.ok_or_else(|| {
                RuntimeError::Engine(Error::internal("synthetic eval root has no caller profile"))
            })?;
            let variable_target_matches = if function.metadata().strict {
                environment.variable_environment
                    == crate::heap::EvalVariableEnvironment::Scope(
                        u16::try_from(first_function_root).map_err(|_| {
                            RuntimeError::Engine(Error::internal(
                                "eval function root index exceeds bytecode range",
                            ))
                        })?,
                    )
            } else {
                match (
                    expected_profile.variable_target,
                    environment.variable_environment,
                ) {
                    (
                        EvalCallerVariableTarget::Global,
                        crate::heap::EvalVariableEnvironment::Global,
                    ) => true,
                    (
                        EvalCallerVariableTarget::ExternalBinding(expected),
                        crate::heap::EvalVariableEnvironment::Closure(actual),
                    ) => {
                        closure_origins.get(usize::from(actual)).copied().flatten()
                            == Some(expected)
                    }
                    (EvalCallerVariableTarget::StrictLocal, _) => false,
                    _ => false,
                }
            };
            if !variable_target_matches {
                return Err(RuntimeError::Engine(Error::internal(
                    "nested eval variable target disagrees with its caller profile",
                )));
            }
        }
        match environment.variable_environment {
            crate::heap::EvalVariableEnvironment::Global => {
                if !is_root {
                    return Err(RuntimeError::Engine(Error::internal(
                        "non-root eval environment used the global variable environment",
                    )));
                }
                if synthetic_eval_root && function.metadata().strict {
                    return Err(RuntimeError::Engine(Error::internal(
                        "strict eval root used the global variable environment",
                    )));
                }
                if environment.scopes[..first_function_root]
                    .iter()
                    .find(|scope| {
                        matches!(
                            scope.kind,
                            crate::heap::EvalScopeKind::FunctionBody
                                | crate::heap::EvalScopeKind::ProgramBody
                        )
                    })
                    .is_none_or(|scope| scope.kind != crate::heap::EvalScopeKind::ProgramBody)
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "global eval variable environment has no current Program body scope",
                    )));
                }
            }
            crate::heap::EvalVariableEnvironment::Scope(index) => {
                if is_root && (!synthetic_eval_root || !function.metadata().strict) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "root eval environment used an invalid local variable scope",
                    )));
                }
                if usize::from(index) != first_function_root {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval variable environment did not reference its current function root scope",
                    )));
                }
            }
            crate::heap::EvalVariableEnvironment::Closure(index) => {
                if !synthetic_eval_root
                    || function.metadata().strict
                    || function.metadata().eval_kind != EvalKind::Direct
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "non-sloppy-direct eval environment used a closure variable environment",
                    )));
                }
                let descriptor = function
                    .closure_variables()
                    .get(usize::from(index))
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "eval closure variable environment is out of bounds",
                        ))
                    })?;
                if descriptor.source != ClosureSource::EvalEnvironment(index)
                    || descriptor.kind != ClosureVariableKind::EvalVariableObject
                    || descriptor.is_lexical
                    || descriptor.is_const
                    || !environment.scopes.iter().any(|scope| {
                        scope.bindings.iter().any(|binding| {
                            binding.source == crate::heap::EvalBindingSource::Closure(index)
                                && binding.kind == ClosureVariableKind::EvalVariableObject
                                && !binding.is_lexical
                                && !binding.is_const
                                && !binding.is_catch_parameter
                        })
                    })
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval closure variable environment is not authenticated",
                    )));
                }
            }
        }

        for scope in &environment.scopes {
            if scope.kind == crate::heap::EvalScopeKind::With && scope.bindings.len() != 1 {
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
                let is_catch_scope = scope.kind == crate::heap::EvalScopeKind::Catch;
                let is_with_scope = scope.kind == crate::heap::EvalScopeKind::With;
                if (binding.is_catch_parameter && !is_catch_scope)
                    || (binding.is_catch_parameter
                        && (!binding.is_lexical
                            || binding.is_const
                            || binding.kind != ClosureVariableKind::Normal))
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
                                crate::heap::EvalBindingSource::Argument(_)
                            )))
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval with-object binding metadata disagrees with its scope",
                    )));
                }
                let expected = match binding.source {
                    crate::heap::EvalBindingSource::Local(index) => {
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
                    crate::heap::EvalBindingSource::Argument(index) => {
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
                    crate::heap::EvalBindingSource::Closure(index) => {
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

#[derive(Clone, Copy)]
enum RootPublication<'a> {
    Script,
    Eval {
        kind: EvalKind,
        caller_strict: bool,
        expected_bindings: &'a [EvalRootBinding<JsString>],
        expected_profile: &'a EvalCallerProfile,
        expected_super_call_allowed: bool,
        expected_super_allowed: bool,
    },
}

pub(super) fn verify_unlinked_tree(function: &UnlinkedFunction) -> Result<(), RuntimeError> {
    verify_unlinked_tree_with_root(function, RootPublication::Script)
}

/// Authenticate a synthetic eval root against the exact caller bindings that
/// will instantiate its `EvalEnvironment` closure slots. This entry point is
/// deliberately separate from ordinary script publication: accepting these
/// sources without the live caller descriptor would make a forged bytecode
/// root indistinguishable from a compiler-produced direct eval.
#[cfg(test)]
pub(in crate::runtime) fn verify_unlinked_eval_tree(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_super_call_allowed: bool,
    expected_super_allowed: bool,
) -> Result<(), RuntimeError> {
    let scope_count = expected_bindings
        .iter()
        .map(|binding| usize::from(binding.scope) + 1)
        .max()
        .unwrap_or(0);
    let mut scope_kinds = vec![crate::heap::EvalScopeKind::FunctionRoot; scope_count];
    for binding in expected_bindings {
        if binding.kind == ClosureVariableKind::WithObject {
            scope_kinds[usize::from(binding.scope)] = crate::heap::EvalScopeKind::With;
        } else if binding.is_catch_parameter {
            scope_kinds[usize::from(binding.scope)] = crate::heap::EvalScopeKind::Catch;
        }
    }
    let variable_target = if caller_strict {
        EvalCallerVariableTarget::StrictLocal
    } else {
        expected_bindings
            .iter()
            .position(|binding| binding.kind == ClosureVariableKind::EvalVariableObject)
            .and_then(|index| u16::try_from(index).ok())
            .map(EvalCallerVariableTarget::ExternalBinding)
            .unwrap_or(EvalCallerVariableTarget::Global)
    };
    verify_unlinked_eval_tree_with_profile(
        function,
        kind,
        caller_strict,
        expected_bindings,
        &EvalCallerProfile {
            scope_kinds: scope_kinds.into_boxed_slice(),
            variable_target,
        },
        expected_super_call_allowed,
        expected_super_allowed,
    )
}

pub(in crate::runtime) fn verify_unlinked_eval_tree_with_profile(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_profile: &EvalCallerProfile,
    expected_super_call_allowed: bool,
    expected_super_allowed: bool,
) -> Result<(), RuntimeError> {
    if kind == EvalKind::None {
        return Err(RuntimeError::Engine(Error::internal(
            "eval publication requires a direct or indirect eval kind",
        )));
    }
    if kind == EvalKind::Indirect && !expected_bindings.is_empty() {
        return Err(RuntimeError::Engine(Error::internal(
            "indirect eval publication received caller bindings",
        )));
    }
    if kind == EvalKind::Indirect && caller_strict {
        return Err(RuntimeError::Engine(Error::internal(
            "indirect eval publication received caller strictness",
        )));
    }
    if expected_super_call_allowed && !expected_super_allowed {
        return Err(RuntimeError::Engine(Error::internal(
            "eval publication permits super() without SuperProperty",
        )));
    }
    if kind == EvalKind::Indirect && (expected_super_call_allowed || expected_super_allowed) {
        return Err(RuntimeError::Engine(Error::internal(
            "indirect eval publication received a super capability",
        )));
    }
    if kind == EvalKind::Indirect
        && (!expected_profile.scope_kinds.is_empty()
            || expected_profile.variable_target != EvalCallerVariableTarget::Global)
    {
        return Err(RuntimeError::Engine(Error::internal(
            "indirect eval publication received a caller scope profile",
        )));
    }
    for binding in expected_bindings {
        let Some(&scope_kind) = expected_profile.scope_kinds.get(usize::from(binding.scope)) else {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root binding disagrees with its caller scope profile",
            )));
        };
        if (binding.is_catch_parameter && scope_kind != crate::heap::EvalScopeKind::Catch)
            || (binding.kind == ClosureVariableKind::WithObject)
                != (scope_kind == crate::heap::EvalScopeKind::With)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root binding disagrees with its caller scope profile",
            )));
        }
        if binding.name.is_empty() {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root binding has an empty name",
            )));
        }
        if binding.is_catch_parameter
            && (!binding.is_lexical
                || binding.is_const
                || binding.kind != ClosureVariableKind::Normal)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root catch binding has invalid binding metadata",
            )));
        }
        if binding.kind == ClosureVariableKind::EvalVariableObject
            && (binding.is_lexical || binding.is_const || binding.is_catch_parameter)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root variable-object binding has invalid binding metadata",
            )));
        }
        if binding.kind == ClosureVariableKind::WithObject
            && (binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || binding.name.utf16_units().ne("<with>".encode_utf16()))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root with-object binding has invalid binding metadata",
            )));
        }
        if binding.kind == ClosureVariableKind::GlobalFunction {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root imported a declaration-only global binding kind",
            )));
        }
    }
    if expected_profile
        .scope_kinds
        .iter()
        .enumerate()
        .any(|(scope, kind)| {
            *kind == crate::heap::EvalScopeKind::With
                && expected_bindings
                    .iter()
                    .filter(|binding| usize::from(binding.scope) == scope)
                    .count()
                    != 1
        })
    {
        return Err(RuntimeError::Engine(Error::internal(
            "eval root with scope does not contain exactly one object binding",
        )));
    }
    let has_variable_object = expected_bindings
        .iter()
        .any(|binding| binding.kind == ClosureVariableKind::EvalVariableObject);
    match (caller_strict, expected_profile.variable_target) {
        (false, EvalCallerVariableTarget::Global) if !has_variable_object => {}
        (true, EvalCallerVariableTarget::StrictLocal) if kind == EvalKind::Direct => {}
        (false, EvalCallerVariableTarget::ExternalBinding(index))
            if expected_bindings
                .get(usize::from(index))
                .is_some_and(|binding| {
                    binding.kind == ClosureVariableKind::EvalVariableObject
                        && !binding.is_lexical
                        && !binding.is_const
                        && !binding.is_catch_parameter
                }) => {}
        _ => {
            return Err(RuntimeError::Engine(Error::internal(
                "eval caller variable target is not authenticated",
            )));
        }
    }
    verify_unlinked_tree_with_root(
        function,
        RootPublication::Eval {
            kind,
            caller_strict,
            expected_bindings,
            expected_profile,
            expected_super_call_allowed,
            expected_super_allowed,
        },
    )
}

fn verify_unlinked_tree_with_root(
    function: &UnlinkedFunction,
    root_publication: RootPublication<'_>,
) -> Result<(), RuntimeError> {
    let (synthetic_eval_tree, tree_expected_bindings, tree_expected_profile) =
        match root_publication {
            RootPublication::Script => (false, &[][..], None),
            RootPublication::Eval {
                expected_bindings,
                expected_profile,
                ..
            } => (true, expected_bindings, Some(expected_profile)),
        };
    let root_origins = function
        .closure_variables()
        .iter()
        .map(|descriptor| match descriptor.source {
            ClosureSource::EvalEnvironment(index) => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Synthetic Eval roots authenticate imported descriptors directly and do
    // not expose the ordinary-function `add_eval_variables` metadata quirk.
    let root_function_name_origins = vec![None; function.closure_variables().len()];
    let mut next_function_id = 1_usize;
    let mut erased_function_name_slots = HashSet::<(usize, u16)>::new();
    let mut eval_consumed_erased_slots = HashSet::<(usize, u16)>::new();
    let mut erased_parent_by_child = HashMap::<(usize, u16), (usize, u16)>::new();
    let mut pending = vec![(
        function,
        0_usize,
        root_origins,
        root_function_name_origins,
        0_usize,
        false,
    )];
    while let Some((
        function,
        function_depth,
        closure_origins,
        function_name_origins,
        function_id,
        inherited_parameter_environment,
    )) = pending.pop()
    {
        let is_root = function_depth == 0;
        let under_parameter_environment =
            inherited_parameter_environment || function.parameter_environment().is_some();
        if under_parameter_environment
            && (function.metadata().eval_variable_object_local.is_some()
                || !function.eval_environments().is_empty()
                || function.code().iter().any(|instruction| {
                    matches!(instruction, crate::bytecode::Instruction::Eval { .. })
                }))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "direct eval in or below a parameter environment is not supported",
            )));
        }
        let expected_eval_kind = if is_root {
            match root_publication {
                RootPublication::Script => EvalKind::None,
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
                    if function.metadata().super_call_allowed || function.metadata().super_allowed {
                        return Err(RuntimeError::Engine(Error::internal(
                            "script root retained a super capability",
                        )));
                    }
                }
                RootPublication::Eval {
                    kind,
                    expected_super_call_allowed,
                    expected_super_allowed,
                    ..
                } => {
                    if (
                        function.metadata().super_call_allowed,
                        function.metadata().super_allowed,
                    ) != (expected_super_call_allowed, expected_super_allowed)
                    {
                        return Err(RuntimeError::Engine(Error::internal(
                            "eval root super capability disagrees with its caller",
                        )));
                    }
                    if kind == EvalKind::Indirect
                        && (function.metadata().super_call_allowed
                            || function.metadata().super_allowed)
                    {
                        return Err(RuntimeError::Engine(Error::internal(
                            "indirect eval root retained a super capability",
                        )));
                    }
                }
            }
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
        let expected_eval_bindings = if is_root {
            match root_publication {
                RootPublication::Script => None,
                RootPublication::Eval {
                    expected_bindings, ..
                } => Some(expected_bindings),
            }
        } else {
            None
        };
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
        let parameter_body_pc = validate_parameter_bytecode_layout(
            function.metadata(),
            function.code(),
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
        let parameter_initializer_capture_locals = parameter_body_pc.map(|_| {
            let mut allowed = vec![false; usize::from(function.metadata().local_count)];
            allowed[..usize::from(function.metadata().parameter_environment_local_count)]
                .fill(true);
            if let Some(local) = function.metadata().function_name_local
                && let Some(allowed) = allowed.get_mut(usize::from(local))
            {
                *allowed = true;
            }

            let mut entry_pc = 0_usize;
            if matches!(
                function.code().first(),
                Some(crate::bytecode::Instruction::Arguments(_))
            ) {
                if let Some(crate::bytecode::Instruction::PutLocal(local)) = function.code().get(1)
                    && let Some(allowed) = allowed.get_mut(usize::from(*local))
                {
                    *allowed = true;
                }
                entry_pc = 2;
            }
            while let Some([source, crate::bytecode::Instruction::PutLocal(local)]) =
                function.code().get(entry_pc..entry_pc + 2)
            {
                if !matches!(
                    source,
                    crate::bytecode::Instruction::PushHomeObject
                        | crate::bytecode::Instruction::PushNewTarget
                        | crate::bytecode::Instruction::PushThis
                ) {
                    break;
                }
                if let Some(allowed) = allowed.get_mut(usize::from(*local)) {
                    *allowed = true;
                }
                entry_pc += 2;
            }
            allowed
        });
        if (function.metadata().rest_parameter.is_some()
            || function.metadata().rest_pattern_start.is_some()
            || function.parameter_environment().is_some()
            || function.metadata().parameter_pattern_end.is_some())
            && let Some((pc, _)) = function.code().iter().enumerate().find(|(_, instruction)| {
                matches!(instruction, crate::bytecode::Instruction::Arguments(_))
            })
        {
            let Some(crate::bytecode::Instruction::PutLocal(local)) = function.code().get(pc + 1)
            else {
                return Err(RuntimeError::Engine(Error::internal(
                    "formal parameter arguments object has no entry binding",
                )));
            };
            let definition = function
                .local_definitions()
                .get(usize::from(*local))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "formal parameter arguments binding is out of bounds",
                    ))
                })?;
            if definition.kind != ClosureVariableKind::Normal
                || definition.is_lexical
                || definition.is_const
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
        if function
            .metadata()
            .function_name_local
            .is_some_and(|index| index >= function.metadata().local_count)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "function-name local is outside bytecode local slots",
            )));
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
        if function.metadata().eval_variable_object_local.is_some()
            && function.metadata().eval_variable_object_local
                == function.metadata().function_name_local
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval variable-object and function-name locals overlap",
            )));
        }
        // GetSuper is also used by derived-constructor `super()`, while the
        // get/put-value operations consume only ordinary stack values. The
        // hidden capability authenticated by this metadata is specifically
        // reading the active function object's HomeObject.
        let reads_home_object = function
            .code()
            .iter()
            .any(|instruction| matches!(instruction, crate::bytecode::Instruction::PushHomeObject));
        if reads_home_object && !function.metadata().needs_home_object {
            return Err(RuntimeError::Engine(Error::internal(
                "super bytecode has no authenticated HomeObject metadata",
            )));
        }
        if function.metadata().eval_variable_object_local.is_some()
            && (is_root
                || function.metadata().strict
                || function.metadata().eval_kind != EvalKind::None
                || function.metadata().function_kind != FunctionKind::Normal)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval variable-object local escaped a sloppy ordinary function",
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
        if function.argument_definitions().len() != usize::from(function.metadata().argument_count)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "argument definition count does not match bytecode metadata",
            )));
        }
        if function.local_definitions().len() != usize::from(function.metadata().local_count) {
            return Err(RuntimeError::Engine(Error::internal(
                "local definition count does not match bytecode metadata",
            )));
        }
        let unnamed_arguments = function
            .argument_definitions()
            .iter()
            .map(|definition| definition.name.is_none())
            .collect::<Vec<_>>();
        let lexical_locals = function
            .local_definitions()
            .iter()
            .map(|definition| definition.is_lexical)
            .collect::<Vec<_>>();
        validate_pattern_parameter_bytecode_layout(
            function.metadata(),
            function.code(),
            &unnamed_arguments,
            &lexical_locals,
            function.parameter_environment(),
        )
        .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
        for definition in function.argument_definitions() {
            if definition.kind != ClosureVariableKind::Normal
                || definition.is_lexical
                || definition.is_const
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
                    || source.name.as_ref() != target.name.as_ref()
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "parameter pattern copy definitions are not same-name lexical-to-root storage",
                    )));
                }
            }
        }
        if function.parameter_environment().is_some() {
            let mut entry_pc = usize::from(matches!(
                function.code().first(),
                Some(crate::bytecode::Instruction::Arguments(_))
            )) * 2;
            while let Some([source, crate::bytecode::Instruction::PutLocal(local)]) =
                function.code().get(entry_pc..entry_pc + 2)
            {
                let expected_name = match source {
                    crate::bytecode::Instruction::PushHomeObject => "<home_object>",
                    crate::bytecode::Instruction::PushNewTarget => "<new.target>",
                    crate::bytecode::Instruction::PushThis => "<this>",
                    _ => break,
                };
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
                    || definition
                        .name
                        .as_ref()
                        .is_none_or(|name| name.utf16_units().ne(expected_name.encode_utf16()))
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "parameter pseudo-binding definition is not authenticated",
                    )));
                }
                entry_pc += 2;
            }
        }
        for (index, definition) in function.local_definitions().iter().enumerate() {
            let is_function_name =
                function.metadata().function_name_local == u16::try_from(index).ok();
            let is_eval_variable_object =
                function.metadata().eval_variable_object_local == u16::try_from(index).ok();
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
            } else if definition.kind != ClosureVariableKind::Normal {
                return Err(RuntimeError::Engine(Error::internal(
                    "ordinary local definition uses a non-local binding kind",
                )));
            } else if definition.is_const && !definition.is_lexical {
                return Err(RuntimeError::Engine(Error::internal(
                    "a const local definition must also be lexical",
                )));
            }
        }
        match function.metadata().eval_variable_object_local {
            Some(index)
                if matches!(
                    function.code(),
                    [
                        crate::bytecode::Instruction::VariableEnvironment,
                        crate::bytecode::Instruction::PutLocal(target),
                        ..
                    ] if *target == index
                ) && !function.code()[2..].iter().any(|instruction| {
                    matches!(
                        instruction,
                        crate::bytecode::Instruction::VariableEnvironment
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
                    crate::bytecode::Instruction::VariableEnvironment
                )
            }) =>
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "variable-environment opcode has no authenticated local",
                )));
            }
            None => {}
        }
        if function.closure_variables().len() != usize::from(function.metadata().closure_count) {
            return Err(RuntimeError::Engine(Error::internal(
                "function closure descriptor count does not match bytecode metadata",
            )));
        }
        verify_unlinked_debug(function)?;
        if function.closure_variables().iter().any(|descriptor| {
            descriptor.is_const
                && !descriptor.is_lexical
                && descriptor.kind != ClosureVariableKind::FunctionName
        }) {
            return Err(RuntimeError::Engine(Error::internal(
                "a const closure descriptor must also be lexical",
            )));
        }
        if function_name_origins.len() != function.closure_variables().len() {
            return Err(RuntimeError::Invariant(
                "function-name provenance count disagrees with closure descriptors",
            ));
        }
        for (index, (descriptor, origin)) in function
            .closure_variables()
            .iter()
            .zip(&function_name_origins)
            .enumerate()
        {
            let Some(is_const) = origin else {
                continue;
            };
            if !function_name_view_matches_origin(*descriptor, *is_const) {
                return Err(RuntimeError::Engine(Error::internal(
                    "closure descriptor lost its ordinary FunctionName provenance",
                )));
            }
            if is_erased_function_name_view(*descriptor) {
                let index = u16::try_from(index).map_err(|_| {
                    RuntimeError::Engine(Error::internal(
                        "closure descriptor index exceeds bytecode range",
                    ))
                })?;
                erased_function_name_slots.insert((function_id, index));
            }
        }
        let mut global_declaration_names = HashMap::new();
        let mut first_global_declaration_indices = HashMap::new();
        let mut global_function_declarations = Vec::new();
        let mut verified_eval_binding_count = 0_usize;
        let eval_allows_global_declarations = is_root
            && match root_publication {
                RootPublication::Script => false,
                RootPublication::Eval {
                    kind,
                    caller_strict,
                    expected_bindings,
                    ..
                } => {
                    !function.metadata().strict
                        && !caller_strict
                        && (kind == EvalKind::Indirect
                            || (kind == EvalKind::Direct
                                && !expected_bindings.iter().any(|binding| {
                                    binding.kind == ClosureVariableKind::EvalVariableObject
                                })))
                }
            };
        for (descriptor_index, descriptor) in function.closure_variables().iter().enumerate() {
            if descriptor.kind == ClosureVariableKind::GlobalFunction
                && (descriptor.is_lexical || descriptor.is_const)
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "global function declaration descriptor has lexical metadata",
                )));
            }
            if (descriptor.source == ClosureSource::GlobalDeclaration
                && !matches!(
                    descriptor.kind,
                    ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
                ))
                || (descriptor.source == ClosureSource::Global
                    && descriptor.kind != ClosureVariableKind::Normal)
                || (matches!(descriptor.source, ClosureSource::ParentGlobal(_))
                    && !matches!(
                        descriptor.kind,
                        ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
                    ))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "global declaration descriptor has non-global binding metadata",
                )));
            }
            if descriptor.kind == ClosureVariableKind::GlobalFunction
                && !matches!(
                    descriptor.source,
                    ClosureSource::GlobalDeclaration | ClosureSource::ParentGlobal(_)
                )
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "global function binding kind escaped a declaration relay",
                )));
            }
            if descriptor.kind == ClosureVariableKind::EvalVariableObject
                && (descriptor.is_lexical
                    || descriptor.is_const
                    || !matches!(
                        descriptor.source,
                        ClosureSource::ParentLocal(_)
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::EvalEnvironment(_)
                    ))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval variable-object descriptor has invalid binding metadata",
                )));
            }
            if descriptor.kind == ClosureVariableKind::WithObject
                && (descriptor.is_lexical
                    || descriptor.is_const
                    || !matches!(
                        descriptor.source,
                        ClosureSource::ParentLocal(_)
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::EvalEnvironment(_)
                    ))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "with-object descriptor has invalid binding metadata",
                )));
            }
            let requires_name = matches!(
                descriptor.kind,
                ClosureVariableKind::FunctionName
                    | ClosureVariableKind::EvalVariableObject
                    | ClosureVariableKind::WithObject
            ) || matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration
                    | ClosureSource::Global
                    | ClosureSource::ParentGlobal(_)
                    | ClosureSource::EvalEnvironment(_)
            );
            let name = unlinked_closure_name(function, descriptor)?;
            if descriptor.kind == ClosureVariableKind::WithObject
                && name.is_none_or(|name| name.utf16_units().ne("<with>".encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "with-object descriptor lost its sentinel name",
                )));
            }
            if descriptor.source == ClosureSource::GlobalDeclaration {
                let name = name.ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "global declaration descriptor has no name",
                    ))
                })?;
                let key = name.utf16_units().collect::<Vec<_>>();
                first_global_declaration_indices
                    .entry(key.clone())
                    .or_insert(descriptor_index);
                if descriptor.kind == ClosureVariableKind::GlobalFunction {
                    global_function_declarations.push(key.clone());
                }
                match global_declaration_names.entry(key) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert((descriptor.is_lexical, descriptor.is_lexical));
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let (first_is_lexical, seen_lexical) = *entry.get();
                        if first_is_lexical
                            && seen_lexical
                            && (descriptor.is_lexical
                                || descriptor.kind != ClosureVariableKind::GlobalFunction)
                        {
                            return Err(RuntimeError::Engine(Error::internal(
                                "duplicate lexical global declaration descriptor name",
                            )));
                        }
                        // A first sloppy Annex B normal record masks every
                        // later same-name declaration in QuickJS's global
                        // conflict lookup, including repeated lexical and var
                        // records. A first lexical remains restricted to the
                        // pinned direct Program-function exception.
                        if descriptor.is_lexical {
                            entry.get_mut().1 = true;
                        }
                    }
                }
            }
            // Direct eval retains ordinary local/argument/closure names as
            // semantic metadata. Global descriptors already require names;
            // local relay names remain optional outside an eval-visible path.
            let allows_name = requires_name
                || descriptor.is_lexical
                || matches!(
                    descriptor.source,
                    ClosureSource::ParentLocal(_)
                        | ClosureSource::ParentArgument(_)
                        | ClosureSource::ParentClosure(_)
                );
            if (requires_name && name.is_none()) || (!allows_name && name.is_some()) {
                return Err(RuntimeError::Engine(Error::internal(
                    "closure descriptor name does not match its binding kind",
                )));
            }
            if let Some(expected_bindings) = expected_eval_bindings
                && descriptor_index < expected_bindings.len()
                && descriptor.source
                    != ClosureSource::EvalEnvironment(u16::try_from(descriptor_index).map_err(
                        |_| {
                            RuntimeError::Engine(Error::internal(
                                "eval environment closure prefix exceeds bytecode range",
                            ))
                        },
                    )?)
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval environment closure descriptors are not an exact prefix",
                )));
            }
            if is_root {
                match root_publication {
                    RootPublication::Script => {
                        if !matches!(
                            descriptor.source,
                            ClosureSource::GlobalDeclaration | ClosureSource::Global
                        ) {
                            return Err(RuntimeError::Engine(Error::internal(
                                "root bytecode closure descriptor did not use Global",
                            )));
                        }
                    }
                    RootPublication::Eval { .. } => match descriptor.source {
                        ClosureSource::Global | ClosureSource::EvalEnvironment(_) => {}
                        ClosureSource::GlobalDeclaration
                            if eval_allows_global_declarations
                                && !descriptor.is_lexical
                                && !descriptor.is_const
                                && matches!(
                                    descriptor.kind,
                                    ClosureVariableKind::Normal
                                        | ClosureVariableKind::GlobalFunction
                                ) => {}
                        ClosureSource::GlobalDeclaration => {
                            return Err(RuntimeError::Engine(Error::internal(
                                "eval root contained an illegal global declaration descriptor",
                            )));
                        }
                        _ => {
                            return Err(RuntimeError::Engine(Error::internal(
                                "eval root closure descriptor used a non-root source",
                            )));
                        }
                    },
                }
            } else {
                match descriptor.source {
                    ClosureSource::GlobalDeclaration | ClosureSource::Global => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "only root bytecode may resolve a global closure binding",
                        )));
                    }
                    ClosureSource::EvalEnvironment(_) => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "only a verified eval root may use an eval environment binding",
                        )));
                    }
                    _ => {}
                }
            }
            if let ClosureSource::EvalEnvironment(index) = descriptor.source {
                let expected_bindings = expected_eval_bindings.ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "eval environment binding escaped specialized eval publication",
                    ))
                })?;
                let index = usize::from(index);
                if index != descriptor_index || index != verified_eval_binding_count {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval environment closure descriptors are not in caller binding order",
                    )));
                }
                let expected = expected_bindings.get(index).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "eval environment closure descriptor exceeds caller binding count",
                    ))
                })?;
                if name != Some(&expected.name) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval environment closure name disagrees with the caller binding",
                    )));
                }
                if (descriptor.is_lexical, descriptor.is_const, descriptor.kind)
                    != (expected.is_lexical, expected.is_const, expected.kind)
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval environment closure flags disagree with the caller binding",
                    )));
                }
                verified_eval_binding_count += 1;
            }
            if matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration
                    | ClosureSource::Global
                    | ClosureSource::ParentGlobal(_)
            ) && !matches!(
                descriptor.kind,
                ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
            ) {
                return Err(RuntimeError::Engine(Error::internal(
                    "global closure descriptor has a non-global binding kind",
                )));
            }
        }
        if let Some(expected_bindings) = expected_eval_bindings {
            if verified_eval_binding_count != expected_bindings.len() {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval environment closure descriptor count disagrees with caller bindings",
                )));
            }
        }
        let mut global_function_initializer_pcs = HashMap::new();
        let global_function_prologue_offset = usize::from(matches!(
            function.code().first(),
            Some(crate::bytecode::Instruction::ThrowRedeclaration(_))
        ));
        for (ordinal, name) in global_function_declarations.iter().enumerate() {
            let closure_pc = ordinal
                .checked_mul(2)
                .and_then(|pc| pc.checked_add(global_function_prologue_offset))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal("global function prologue is too large"))
                })?;
            let initializer_pc = closure_pc + 1;
            let Some(crate::bytecode::Instruction::FClosure(constant)) =
                function.code().get(closure_pc)
            else {
                return Err(RuntimeError::Engine(Error::internal(
                    "global function declaration has no hoisted closure",
                )));
            };
            let constant = usize::try_from(*constant)
                .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
            let child = function
                .constants()
                .get(constant)
                .and_then(UnlinkedConstant::as_child)
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "global function hoist did not reference child bytecode",
                    ))
                })?;
            if child
                .func_name()
                .is_none_or(|child_name| child_name.utf16_units().ne(name.iter().copied()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "global function hoist name disagrees with its declaration",
                )));
            }
            let expected_target = *first_global_declaration_indices.get(name).ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "global function declaration has no first-name slot",
                ))
            })?;
            let expected_target = u16::try_from(expected_target).map_err(|_| {
                RuntimeError::Engine(Error::internal(
                    "global function initializer target is out of bounds",
                ))
            })?;
            if !matches!(
                function.code().get(initializer_pc),
                Some(crate::bytecode::Instruction::PutVarInit(target))
                    if *target == expected_target
            ) {
                return Err(RuntimeError::Engine(Error::internal(
                    "global function initializer did not target its first-name slot",
                )));
            }
            global_function_initializer_pcs.insert(initializer_pc, expected_target);
        }
        let masked_lexical_initializer_targets = global_declaration_names
            .iter()
            .filter_map(|(name, &(first_is_lexical, seen_lexical))| {
                (!first_is_lexical && seen_lexical)
                    .then(|| first_global_declaration_indices.get(name).copied())
                    .flatten()
            })
            .collect::<Vec<_>>();
        verify_parts(
            function.code(),
            function.constants().len(),
            function.metadata().max_stack,
        )?;

        let mut child_closure_pcs = vec![Vec::new(); function.constants().len()];
        for (pc, instruction) in function.code().iter().enumerate() {
            let crate::bytecode::Instruction::FClosure(index) = instruction else {
                continue;
            };
            let Ok(index) = usize::try_from(*index) else {
                continue;
            };
            if function
                .constants()
                .get(index)
                .is_some_and(|constant| constant.as_child().is_some())
            {
                child_closure_pcs[index].push(pc);
            }
        }

        let mut referenced_eval_environments = vec![false; function.eval_environments().len()];
        for instruction in function.code() {
            let crate::bytecode::Instruction::Eval { environment, .. } = instruction else {
                continue;
            };
            let referenced = referenced_eval_environments
                .get_mut(usize::from(*environment))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "Eval bytecode environment operand is out of bounds",
                    ))
                })?;
            *referenced = true;
        }
        if referenced_eval_environments
            .iter()
            .any(|referenced| !referenced)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval environment descriptor is not referenced by bytecode",
            )));
        }

        let mut captured_locals = vec![false; usize::from(function.metadata().local_count)];
        for (constant_index, child) in function
            .constants()
            .iter()
            .enumerate()
            .filter_map(|(index, constant)| constant.as_child().map(|child| (index, child)))
        {
            if child_closure_pcs[constant_index].is_empty() {
                continue;
            }
            for descriptor in child.closure_variables() {
                if let ClosureSource::ParentLocal(index) = descriptor.source {
                    if let Some(captured) = captured_locals.get_mut(usize::from(index)) {
                        *captured = true;
                    }
                }
            }
        }
        verify_eval_environments(
            function,
            function_depth,
            &mut captured_locals,
            synthetic_eval_tree,
            &closure_origins,
            tree_expected_bindings,
            tree_expected_profile,
        )?;
        for binding in function
            .eval_environments()
            .iter()
            .flat_map(|environment| environment.scopes.iter())
            .flat_map(|scope| scope.bindings.iter())
        {
            let crate::heap::EvalBindingSource::Closure(index) = binding.source else {
                continue;
            };
            if function_name_origins
                .get(usize::from(index))
                .is_some_and(Option::is_some)
                && function
                    .closure_variables()
                    .get(usize::from(index))
                    .is_some_and(|descriptor| is_erased_function_name_view(*descriptor))
                && !binding.is_lexical
                && !binding.is_const
                && binding.kind == ClosureVariableKind::Normal
            {
                eval_consumed_erased_slots.insert((function_id, index));
            }
        }
        for (index, (descriptor, origin)) in function
            .closure_variables()
            .iter()
            .zip(&function_name_origins)
            .enumerate()
        {
            if origin.is_none() || !is_erased_function_name_view(*descriptor) {
                continue;
            }
            let index = u16::try_from(index).map_err(|_| {
                RuntimeError::Engine(Error::internal(
                    "closure descriptor index exceeds bytecode range",
                ))
            })?;
            if eval_consumed_erased_slots.contains(&(function_id, index)) {
                // A function's own eval prepass runs before every child.
                continue;
            }
            let first_child_view = function
                .constants()
                .iter()
                .enumerate()
                .filter(|(constant_index, _)| !child_closure_pcs[*constant_index].is_empty())
                .filter_map(|(_, constant)| constant.as_child())
                .find_map(|child| {
                    child
                        .closure_variables()
                        .iter()
                        .find(|candidate| candidate.source == ClosureSource::ParentClosure(index))
                        .copied()
                });
            if first_child_view.is_none_or(|view| !is_erased_function_name_view(view)) {
                return Err(RuntimeError::Engine(Error::internal(
                    "erased FunctionName closure was not the first source request",
                )));
            }
        }

        for (pc, instruction) in function.code().iter().enumerate() {
            if let Some((source, name)) = match instruction {
                crate::bytecode::Instruction::HasEvalVariable { source, name }
                | crate::bytecode::Instruction::GetEvalVariable { source, name }
                | crate::bytecode::Instruction::PutEvalVariable { source, name }
                | crate::bytecode::Instruction::DeleteEvalVariable { source, name }
                | crate::bytecode::Instruction::DefineEvalVariable { source, name } => {
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
                    Some(Value::String(_))
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval variable opcode referenced a non-string name constant",
                    )));
                }
            }
            if let Some((source, name)) = match instruction {
                crate::bytecode::Instruction::HasDynamicBinding { source, name }
                | crate::bytecode::Instruction::GetDynamicBinding { source, name }
                | crate::bytecode::Instruction::PutDynamicBinding { source, name }
                | crate::bytecode::Instruction::DeleteDynamicBinding { source, name } => {
                    Some((*source, Some(*name)))
                }
                crate::bytecode::Instruction::DynamicEnvironmentObject(source) => {
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
                crate::bytecode::Instruction::GetRefValue(name)
                | crate::bytecode::Instruction::GetRefValueUndef(name)
                | crate::bytecode::Instruction::PutRefValue(name) => Some(*name),
                _ => None,
            } {
                verify_unlinked_string_constant(
                    function,
                    name,
                    "reference opcode referenced a non-string name constant",
                )?;
            }
            match instruction {
                crate::bytecode::Instruction::PushConst(index) => {
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
                crate::bytecode::Instruction::RegExp(index) => {
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
                crate::bytecode::Instruction::FClosure(index) => {
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
                crate::bytecode::Instruction::SetName(index)
                | crate::bytecode::Instruction::ThrowReadOnly(index)
                | crate::bytecode::Instruction::ThrowRedeclaration(index)
                | crate::bytecode::Instruction::GetField(index)
                | crate::bytecode::Instruction::GetField2(index)
                | crate::bytecode::Instruction::PutField(index)
                | crate::bytecode::Instruction::DefineField(index)
                | crate::bytecode::Instruction::DefineMethod { key: index, .. } => {
                    let index = usize::try_from(*index)
                        .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
                    let constant = function.constants().get(index).ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "string-key constant index is out of bounds",
                        ))
                    })?;
                    if !matches!(constant.as_primitive(), Some(Value::String(_))) {
                        return Err(RuntimeError::Engine(Error::internal(
                            "string-key opcode referenced a non-string constant",
                        )));
                    }
                }
                crate::bytecode::Instruction::PutLocal(index)
                | crate::bytecode::Instruction::SetLocal(index)
                    if function.metadata().function_name_local == Some(*index) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "bytecode directly writes its private function-name local",
                    )));
                }
                crate::bytecode::Instruction::GetLocal(index)
                | crate::bytecode::Instruction::PutLocal(index)
                | crate::bytecode::Instruction::SetLocal(index)
                    if function.metadata().eval_variable_object_local == Some(*index)
                        && !matches!(
                            (pc, instruction),
                            (1, crate::bytecode::Instruction::PutLocal(_))
                        ) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "ordinary local opcode referenced the private eval variable object",
                    )));
                }
                crate::bytecode::Instruction::GetLocal(index)
                | crate::bytecode::Instruction::PutLocal(index)
                | crate::bytecode::Instruction::SetLocal(index)
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
                crate::bytecode::Instruction::GetLocal(index)
                | crate::bytecode::Instruction::PutLocal(index)
                | crate::bytecode::Instruction::SetLocal(index)
                    if function
                        .local_definitions()
                        .get(usize::from(*index))
                        .is_some_and(|definition| definition.is_lexical) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "unchecked local opcode referenced a lexical definition",
                    )));
                }
                crate::bytecode::Instruction::SetLocalUninitialized(index)
                | crate::bytecode::Instruction::GetLocalCheck(index)
                | crate::bytecode::Instruction::PutLocalCheck(index)
                | crate::bytecode::Instruction::SetLocalCheck(index)
                    if function
                        .local_definitions()
                        .get(usize::from(*index))
                        .is_some_and(|definition| !definition.is_lexical) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "checked lexical-local opcode referenced an ordinary definition",
                    )));
                }
                crate::bytecode::Instruction::InitializeLocal(index)
                | crate::bytecode::Instruction::CloseLocal(index)
                    if function
                        .local_definitions()
                        .get(usize::from(*index))
                        .is_some_and(|definition| {
                            !definition.is_lexical
                                && definition.kind != ClosureVariableKind::WithObject
                        }) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "lifetime opcode referenced an ordinary local definition",
                    )));
                }
                crate::bytecode::Instruction::PutLocalCheck(index)
                | crate::bytecode::Instruction::SetLocalCheck(index)
                    if function
                        .local_definitions()
                        .get(usize::from(*index))
                        .is_some_and(|definition| definition.is_const) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "mutable lexical-local write bypassed a const definition",
                    )));
                }
                crate::bytecode::Instruction::CloseLocal(index)
                    if captured_locals
                        .get(usize::from(*index))
                        .is_some_and(|captured| !captured) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "CloseLocal referenced a local which no child captures",
                    )));
                }
                crate::bytecode::Instruction::PutVarRef(index)
                | crate::bytecode::Instruction::SetVarRef(index)
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
                crate::bytecode::Instruction::GetVarRef(index)
                | crate::bytecode::Instruction::PutVarRef(index)
                | crate::bytecode::Instruction::SetVarRef(index)
                | crate::bytecode::Instruction::GetVarRefCheck(index)
                | crate::bytecode::Instruction::PutVarRefCheck(index)
                    if function
                        .closure_variables()
                        .get(usize::from(*index))
                        .is_some_and(|descriptor| {
                            matches!(
                                descriptor.kind,
                                ClosureVariableKind::EvalVariableObject
                                    | ClosureVariableKind::WithObject
                            )
                        }) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "ordinary closure opcode referenced a hidden object binding",
                    )));
                }
                crate::bytecode::Instruction::GetVarRef(index)
                | crate::bytecode::Instruction::PutVarRef(index)
                | crate::bytecode::Instruction::SetVarRef(index)
                | crate::bytecode::Instruction::GetVarRefCheck(index)
                | crate::bytecode::Instruction::PutVarRefCheck(index)
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
                crate::bytecode::Instruction::GetVarRef(index)
                | crate::bytecode::Instruction::PutVarRef(index)
                | crate::bytecode::Instruction::SetVarRef(index)
                    if function
                        .closure_variables()
                        .get(usize::from(*index))
                        .is_some_and(|descriptor| descriptor.is_lexical) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "unchecked closure opcode referenced a lexical binding",
                    )));
                }
                crate::bytecode::Instruction::GetVarRefCheck(index)
                | crate::bytecode::Instruction::PutVarRefCheck(index)
                    if function
                        .closure_variables()
                        .get(usize::from(*index))
                        .is_some_and(|descriptor| !descriptor.is_lexical) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "checked closure opcode referenced an ordinary binding",
                    )));
                }
                crate::bytecode::Instruction::PutVarRefCheck(index)
                    if function
                        .closure_variables()
                        .get(usize::from(*index))
                        .is_some_and(|descriptor| descriptor.is_const) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "mutable checked closure write bypassed a const binding",
                    )));
                }
                crate::bytecode::Instruction::GetVar(index)
                | crate::bytecode::Instruction::GetVarUndef(index)
                | crate::bytecode::Instruction::DeleteVar(index)
                | crate::bytecode::Instruction::PutVar(index)
                | crate::bytecode::Instruction::PutVarInit(index)
                | crate::bytecode::Instruction::GlobalReference(index)
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
                crate::bytecode::Instruction::PutVarInit(index)
                    if function
                        .closure_variables()
                        .get(usize::from(*index))
                        .is_some_and(|descriptor| {
                            !descriptor.is_lexical
                                && global_function_initializer_pcs.get(&pc) != Some(index)
                                && !masked_lexical_initializer_targets
                                    .contains(&usize::from(*index))
                        }) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "global initializer referenced an ordinary non-function descriptor",
                    )));
                }
                crate::bytecode::Instruction::GetLocal(index)
                | crate::bytecode::Instruction::PutLocal(index)
                | crate::bytecode::Instruction::SetLocal(index)
                | crate::bytecode::Instruction::SetLocalUninitialized(index)
                | crate::bytecode::Instruction::GetLocalCheck(index)
                | crate::bytecode::Instruction::InitializeLocal(index)
                | crate::bytecode::Instruction::PutLocalCheck(index)
                | crate::bytecode::Instruction::SetLocalCheck(index)
                | crate::bytecode::Instruction::CloseLocal(index)
                    if *index >= function.metadata().local_count =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "local bytecode operand is out of bounds",
                    )));
                }
                crate::bytecode::Instruction::GetArg(index)
                | crate::bytecode::Instruction::PutArg(index)
                | crate::bytecode::Instruction::SetArg(index)
                    if *index >= function.metadata().argument_count =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "argument bytecode operand is out of bounds",
                    )));
                }
                crate::bytecode::Instruction::Rest(start)
                    if *start > function.metadata().argument_count =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "rest bytecode operand is out of bounds",
                    )));
                }
                crate::bytecode::Instruction::GetVarRef(index)
                | crate::bytecode::Instruction::PutVarRef(index)
                | crate::bytecode::Instruction::SetVarRef(index)
                | crate::bytecode::Instruction::GetVarRefCheck(index)
                | crate::bytecode::Instruction::PutVarRefCheck(index)
                | crate::bytecode::Instruction::GetVar(index)
                | crate::bytecode::Instruction::GetVarUndef(index)
                | crate::bytecode::Instruction::DeleteVar(index)
                | crate::bytecode::Instruction::PutVar(index)
                | crate::bytecode::Instruction::PutVarInit(index)
                | crate::bytecode::Instruction::GlobalReference(index)
                    if *index >= function.metadata().closure_count =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "closure variable bytecode operand is out of bounds",
                    )));
                }
                _ => {}
            }
        }
        let mut local_flags = vec![None; usize::from(function.metadata().local_count)];
        let mut argument_flags = vec![None; usize::from(function.metadata().argument_count)];
        for (constant_index, constant) in function.constants().iter().enumerate() {
            if let Some(child) = constant.as_child() {
                let closure_pcs = &child_closure_pcs[constant_index];
                let child_id = next_function_id;
                next_function_id = next_function_id.checked_add(1).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "function publication identity overflowed",
                    ))
                })?;
                let mut child_function_name_origins =
                    Vec::with_capacity(child.closure_variables().len());
                let mut child_physical_sources = HashSet::new();
                for (descriptor_index, descriptor) in child.closure_variables().iter().enumerate() {
                    if let (Some(body_pc), Some(initializer_capture_locals)) =
                        (parameter_body_pc, &parameter_initializer_capture_locals)
                    {
                        let instantiated_in_initializer =
                            closure_pcs.iter().any(|pc| *pc < body_pc);
                        let instantiated_in_body = closure_pcs.iter().any(|pc| *pc >= body_pc);
                        match descriptor.source {
                            ClosureSource::ParentArgument(_) if instantiated_in_initializer => {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "parameter initializer closure captured a raw argument slot",
                                )));
                            }
                            ClosureSource::ParentLocal(index)
                                if instantiated_in_body
                                    && index
                                        < function.metadata().parameter_environment_local_count =>
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "function body closure captured a parameter-environment cell",
                                )));
                            }
                            ClosureSource::ParentLocal(index)
                                if instantiated_in_initializer
                                    && !initializer_capture_locals
                                        .get(usize::from(index))
                                        .copied()
                                        .unwrap_or(false) =>
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "parameter initializer closure captured a body-only local",
                                )));
                            }
                            _ => {}
                        }
                    }
                    if let Some(body_pc) = pattern_body_pc {
                        let instantiated_in_pattern = closure_pcs.iter().any(|pc| *pc < body_pc);
                        match descriptor.source {
                            ClosureSource::ParentArgument(index)
                                if function
                                    .argument_definitions()
                                    .get(usize::from(index))
                                    .is_some_and(|definition| definition.name.is_none()) =>
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "child closure captured an anonymous pattern argument slot",
                                )));
                            }
                            ClosureSource::ParentLocal(index)
                                if instantiated_in_pattern
                                    && index
                                        >= function
                                            .metadata()
                                            .parameter_environment_local_count
                                    && function
                                        .local_definitions()
                                        .get(usize::from(index))
                                        .is_some_and(|definition| definition.is_lexical) =>
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "pattern initializer closure captured a body lexical local",
                                )));
                            }
                            _ => {}
                        }
                    }
                    if matches!(
                        descriptor.source,
                        ClosureSource::ParentLocal(_)
                            | ClosureSource::ParentArgument(_)
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::ParentGlobal(_)
                    ) && !child_physical_sources.insert(descriptor.source)
                    {
                        return Err(RuntimeError::Engine(Error::internal(
                            "child closure table duplicated one physical parent source",
                        )));
                    }
                    let flags = (descriptor.is_lexical, descriptor.is_const, descriptor.kind);
                    let mut function_name_origin = None;
                    match descriptor.source {
                        ClosureSource::ParentLocal(index) => {
                            let slot =
                                local_flags.get_mut(usize::from(index)).ok_or_else(|| {
                                    RuntimeError::Engine(Error::internal(
                                        "child closure descriptor source is out of parent bounds",
                                    ))
                                })?;
                            let definition = function
                                .local_definitions()
                                .get(usize::from(index))
                                .ok_or_else(|| {
                                RuntimeError::Engine(Error::internal(
                                    "child closure descriptor source is out of parent definitions",
                                ))
                            })?;
                            let definition_flags =
                                (definition.is_lexical, definition.is_const, definition.kind);
                            function_name_origin = (definition.kind
                                == ClosureVariableKind::FunctionName)
                                .then_some(definition.is_const);
                            let flags_match = function_name_origin.is_some_and(|is_const| {
                                function_name_view_matches_origin(*descriptor, is_const)
                            }) || flags == definition_flags;
                            if !flags_match {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "child closure descriptor flags disagree with its parent local definition",
                                )));
                            }
                            let descriptor_name = unlinked_closure_name(child, descriptor)?;
                            if (definition.is_lexical
                                || matches!(
                                    definition.kind,
                                    ClosureVariableKind::FunctionName
                                        | ClosureVariableKind::EvalVariableObject
                                )
                                || descriptor_name.is_some())
                                && descriptor_name != definition.name.as_ref()
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "child closure descriptor name disagrees with its parent local definition",
                                )));
                            }
                            // Siblings may legitimately observe either the
                            // original FunctionName flags or QuickJS's
                            // direct-eval-child erasure. Compare their common
                            // authenticated parent definition, not that
                            // observable child-local representation quirk.
                            verify_capture_flags(slot, definition_flags)?;
                        }
                        ClosureSource::ParentArgument(index) => {
                            let slot =
                                argument_flags.get_mut(usize::from(index)).ok_or_else(|| {
                                    RuntimeError::Engine(Error::internal(
                                        "child closure descriptor source is out of parent bounds",
                                    ))
                                })?;
                            let definition = function
                                .argument_definitions()
                                .get(usize::from(index))
                                .ok_or_else(|| {
                                    RuntimeError::Engine(Error::internal(
                                        "child closure descriptor source is out of parent definitions",
                                    ))
                                })?;
                            if flags
                                != (definition.is_lexical, definition.is_const, definition.kind)
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "child closure descriptor flags disagree with its parent argument definition",
                                )));
                            }
                            let descriptor_name = unlinked_closure_name(child, descriptor)?;
                            if (definition.is_lexical || descriptor_name.is_some())
                                && descriptor_name != definition.name.as_ref()
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "child closure descriptor name disagrees with its parent argument definition",
                                )));
                            }
                            verify_capture_flags(slot, flags)?;
                        }
                        ClosureSource::ParentClosure(index) => {
                            let parent = function
                                .closure_variables()
                                .get(usize::from(index))
                                .ok_or_else(|| {
                                    RuntimeError::Engine(Error::internal(
                                        "child closure descriptor source is out of parent bounds",
                                    ))
                                })?;
                            let parent_flags = (parent.is_lexical, parent.is_const, parent.kind);
                            function_name_origin = function_name_origins
                                .get(usize::from(index))
                                .copied()
                                .flatten();
                            let flags_match = function_name_origin.is_some_and(|is_const| {
                                function_name_view_matches_origin(*descriptor, is_const)
                            }) || parent_flags == flags;
                            if !flags_match {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "transitive closure descriptor flags do not match the parent slot",
                                )));
                            }
                            if matches!(
                                parent.source,
                                ClosureSource::GlobalDeclaration
                                    | ClosureSource::Global
                                    | ClosureSource::ParentGlobal(_)
                            ) {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "local closure relay referenced a global parent slot",
                                )));
                            }
                            let descriptor_name = unlinked_closure_name(child, descriptor)?;
                            if (function_name_origin.is_some()
                                || descriptor.is_lexical
                                || matches!(
                                    descriptor.kind,
                                    ClosureVariableKind::FunctionName
                                        | ClosureVariableKind::EvalVariableObject
                                )
                                || descriptor_name.is_some())
                                && descriptor_name != unlinked_closure_name(function, parent)?
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "transitive closure relay changed its lexical binding name",
                                )));
                            }
                        }
                        ClosureSource::GlobalDeclaration => {
                            if !matches!(
                                descriptor.kind,
                                ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
                            ) {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "global closure descriptor has a non-global binding kind",
                                )));
                            }
                        }
                        ClosureSource::Global => {
                            if descriptor.kind != ClosureVariableKind::Normal {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "resolved global has a declaration-only binding kind",
                                )));
                            }
                        }
                        ClosureSource::ParentGlobal(index) => {
                            let parent = function
                                .closure_variables()
                                .get(usize::from(index))
                                .ok_or_else(|| {
                                    RuntimeError::Engine(Error::internal(
                                        "child global relay source is out of parent bounds",
                                    ))
                                })?;
                            if !matches!(
                                parent.source,
                                ClosureSource::GlobalDeclaration
                                    | ClosureSource::Global
                                    | ClosureSource::ParentGlobal(_)
                            ) || (parent.is_lexical, parent.is_const, parent.kind) != flags
                                || unlinked_closure_name(child, descriptor)?
                                    != unlinked_closure_name(function, parent)?
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "parent global relay descriptor disagrees with the parent slot",
                                )));
                            }
                        }
                        ClosureSource::EvalEnvironment(_) => {
                            return Err(RuntimeError::Engine(Error::internal(
                                "child bytecode directly referenced an eval environment binding",
                            )));
                        }
                    }
                    let descriptor_index = u16::try_from(descriptor_index).map_err(|_| {
                        RuntimeError::Engine(Error::internal(
                            "child closure descriptor index exceeds bytecode range",
                        ))
                    })?;
                    if function_name_origin.is_some() && is_erased_function_name_view(*descriptor) {
                        if let ClosureSource::ParentClosure(parent_index) = descriptor.source {
                            let parent_is_erased = function_name_origins
                                .get(usize::from(parent_index))
                                .is_some_and(Option::is_some)
                                && function
                                    .closure_variables()
                                    .get(usize::from(parent_index))
                                    .is_some_and(|parent| is_erased_function_name_view(*parent));
                            if parent_is_erased
                                && erased_parent_by_child
                                    .insert(
                                        (child_id, descriptor_index),
                                        (function_id, parent_index),
                                    )
                                    .is_some()
                            {
                                return Err(RuntimeError::Invariant(
                                    "erased FunctionName slot acquired two parents",
                                ));
                            }
                        }
                    }
                    child_function_name_origins.push(function_name_origin);
                }
                let child_depth = function_depth.checked_add(1).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "function-tree depth overflowed during publication",
                    ))
                })?;
                let child_origins = child
                    .closure_variables()
                    .iter()
                    .map(|descriptor| match descriptor.source {
                        ClosureSource::ParentClosure(index) => {
                            closure_origins.get(usize::from(index)).copied().flatten()
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                pending.push((
                    child,
                    child_depth,
                    child_origins,
                    child_function_name_origins,
                    child_id,
                    under_parameter_environment,
                ));
            } else if constant.as_primitive().is_none()
                && constant.as_regexp().is_none()
                && constant.as_template_object().is_none()
            {
                return Err(RuntimeError::Invariant(
                    "unlinked constant did not contain exactly one payload",
                ));
            }
        }
    }
    let mut authenticated_erased_slots = HashSet::new();
    let mut lineage = eval_consumed_erased_slots.into_iter().collect::<Vec<_>>();
    while let Some(slot) = lineage.pop() {
        if !authenticated_erased_slots.insert(slot) {
            continue;
        }
        if let Some(parent) = erased_parent_by_child.get(&slot).copied() {
            lineage.push(parent);
        }
    }
    if erased_function_name_slots
        .iter()
        .any(|slot| !authenticated_erased_slots.contains(slot))
    {
        return Err(RuntimeError::Engine(Error::internal(
            "erased FunctionName closure has no direct-eval lineage",
        )));
    }
    Ok(())
}

fn verify_unlinked_debug(function: &UnlinkedFunction) -> Result<(), RuntimeError> {
    let Some(debug) = function.debug() else {
        return Ok(());
    };
    if debug
        .source
        .as_deref()
        .is_some_and(|source| std::str::from_utf8(source).is_err())
    {
        return Err(RuntimeError::Engine(Error::internal(
            "bytecode debug source is not valid UTF-8",
        )));
    }
    let Some(table) = &debug.pc2line else {
        return Ok(());
    };
    if table.definition.line == u32::MAX || table.definition.column == u32::MAX {
        return Err(RuntimeError::Engine(Error::internal(
            "bytecode debug definition position cannot be represented one-based",
        )));
    }
    let mut previous_pc = None;
    for entry in &table.entries {
        if usize::try_from(entry.pc)
            .ok()
            .is_none_or(|pc| pc >= function.code().len())
        {
            return Err(RuntimeError::Engine(Error::internal(
                "bytecode debug PC is outside the instruction stream",
            )));
        }
        if previous_pc.is_some_and(|previous| entry.pc < previous) {
            return Err(RuntimeError::Engine(Error::internal(
                "bytecode debug PCs are not ordered",
            )));
        }
        if entry.position.line == u32::MAX || entry.position.column == u32::MAX {
            return Err(RuntimeError::Engine(Error::internal(
                "bytecode debug position cannot be represented one-based",
            )));
        }
        previous_pc = Some(entry.pc);
    }
    Ok(())
}

fn function_name_view_matches_origin(descriptor: ClosureVariable, origin_is_const: bool) -> bool {
    (descriptor.is_lexical, descriptor.is_const, descriptor.kind)
        == (false, origin_is_const, ClosureVariableKind::FunctionName)
        || is_erased_function_name_view(descriptor)
}

fn is_erased_function_name_view(descriptor: ClosureVariable) -> bool {
    (descriptor.is_lexical, descriptor.is_const, descriptor.kind)
        == (false, false, ClosureVariableKind::Normal)
}

fn verify_capture_flags(
    previous: &mut Option<(bool, bool, ClosureVariableKind)>,
    current: (bool, bool, ClosureVariableKind),
) -> Result<(), RuntimeError> {
    if previous.is_some_and(|previous| previous != current) {
        return Err(RuntimeError::Engine(Error::internal(
            "sibling closure descriptors disagree about one parent binding",
        )));
    }
    *previous = Some(current);
    Ok(())
}

pub(super) fn flatten_unlinked_tree(
    function: UnlinkedFunction,
) -> Result<Vec<FlatFunction>, RuntimeError> {
    let mut frames = vec![FlattenFrame::new(function)];
    let mut functions = Vec::new();

    loop {
        let next = frames
            .last_mut()
            .ok_or(RuntimeError::Invariant(
                "unlinked function flattening lost its root frame",
            ))?
            .remaining
            .next();
        if let Some(constant) = next {
            let constant = match constant.into_template_object() {
                Ok((cooked, raw)) => {
                    frames
                        .last_mut()
                        .expect("flatten frame remains present")
                        .constants
                        .push(FlatConstant::TemplateObject { cooked, raw });
                    continue;
                }
                Err(constant) => constant,
            };
            let constant = match constant.into_regexp() {
                Ok((pattern, program)) => {
                    frames
                        .last_mut()
                        .expect("flatten frame remains present")
                        .constants
                        .push(FlatConstant::RegExp { pattern, program });
                    continue;
                }
                Err(constant) => constant,
            };
            let (primitive, atom_string, child) = constant.into_parts();
            match (primitive, atom_string, child) {
                (Some(Value::String(value)), true, None) => frames
                    .last_mut()
                    .expect("flatten frame remains present")
                    .constants
                    .push(FlatConstant::AtomString(value)),
                (Some(value), false, None) => frames
                    .last_mut()
                    .expect("flatten frame remains present")
                    .constants
                    .push(FlatConstant::Value(raw_unlinked_primitive(value)?)),
                (None, false, Some(child)) => frames.push(FlattenFrame::new(child)),
                (None, _, None)
                | (Some(_), true, None)
                | (Some(_), _, Some(_))
                | (None, true, Some(_)) => {
                    return Err(RuntimeError::Invariant(
                        "unlinked constant did not contain exactly one payload",
                    ));
                }
            }
            continue;
        }

        let frame = frames.pop().ok_or(RuntimeError::Invariant(
            "unlinked function flattening lost a completed frame",
        ))?;
        let index = functions.len();
        functions.push(FlatFunction {
            code: frame.code,
            constants: frame.constants,
            metadata: frame.metadata,
            parameter_environment: frame.parameter_environment,
            func_name: frame.func_name,
            argument_definitions: frame.argument_definitions,
            local_definitions: frame.local_definitions,
            closure_variables: frame.closure_variables,
            eval_environments: frame.eval_environments,
            debug: frame.debug,
        });
        if let Some(parent) = frames.last_mut() {
            parent.constants.push(FlatConstant::Child(index));
        } else {
            return Ok(functions);
        }
    }
}

fn raw_unlinked_primitive(value: Value) -> Result<RawValue, RuntimeError> {
    match value {
        Value::Undefined => Ok(RawValue::Undefined),
        Value::Null => Ok(RawValue::Null),
        Value::Bool(value) => Ok(RawValue::Bool(value)),
        Value::Int(value) => Ok(RawValue::Int(value)),
        Value::Float(value) => Ok(RawValue::Float(value)),
        Value::BigInt(value) => Ok(RawValue::BigInt(value)),
        Value::String(value) => Ok(RawValue::String(value)),
        Value::Object(_) | Value::Symbol(_) => Err(RuntimeError::Invariant(
            "runtime-bound value escaped the unlinked constant invariant",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instruction;
    use crate::heap::{
        EvalBinding, EvalBindingSource, EvalEnvironment, EvalScope, EvalScopeKind,
        EvalVariableEnvironment, ParameterArgumentCell, ParameterBodyStorage,
        ParameterDefaultSource, ParameterPatternCopy,
    };

    fn ordinary_environment(binding: Option<EvalBinding<JsString>>) -> EvalEnvironment<JsString> {
        EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: binding.into_iter().collect::<Vec<_>>().into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Scope(1),
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        }
    }

    fn eval_code(environment: u16, close_local: bool) -> Vec<Instruction> {
        let mut code = vec![
            Instruction::Undefined,
            Instruction::Eval {
                argument_count: 0,
                environment,
            },
        ];
        if close_local {
            code.extend([
                Instruction::Drop,
                Instruction::CloseLocal(0),
                Instruction::Undefined,
            ]);
        }
        code.push(Instruction::Return);
        code
    }

    fn lexical_local_function(
        environment: EvalEnvironment<JsString>,
        code: Vec<Instruction>,
    ) -> UnlinkedFunction {
        UnlinkedFunction::new(
            code,
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(
                Some(JsString::from_static("binding")),
                false,
            )],
        )
        .with_eval_environments(vec![environment])
    }

    fn local_with_environment(
        binding: EvalBinding<JsString>,
        definition_name: &'static str,
    ) -> UnlinkedFunction {
        let environment = EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::With,
                    bindings: vec![binding].into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Scope(2),
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition {
                name: Some(JsString::from_static(definition_name)),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::WithObject,
            }],
        )
        .with_eval_environments(vec![environment])
    }

    fn captured_with_object_function(strict: bool) -> UnlinkedFunction {
        let relay = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::Undefined, Instruction::Return],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<with>")))
                    .unwrap(),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::WithObject,
            }],
        );
        UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::InitializeLocal(0),
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::CloseLocal(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(relay)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                strict,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(Vec::new(), vec![UnlinkedVariableDefinition::with_object()])
    }

    fn script_with_child(child: UnlinkedFunction) -> UnlinkedFunction {
        UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
    }

    fn eval_root_binding(
        name: &'static str,
        scope: u16,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> EvalRootBinding<JsString> {
        EvalRootBinding {
            name: JsString::from_static(name),
            scope,
            is_lexical,
            is_const,
            kind,
            is_catch_parameter: false,
        }
    }

    fn eval_root_with_descriptors(
        eval_kind: EvalKind,
        descriptors: Vec<(ClosureSource, &'static str, bool, bool, ClosureVariableKind)>,
    ) -> UnlinkedFunction {
        eval_root_with_descriptors_and_strict(eval_kind, false, descriptors)
    }

    fn eval_root_with_descriptors_and_strict(
        eval_kind: EvalKind,
        strict: bool,
        descriptors: Vec<(ClosureSource, &'static str, bool, bool, ClosureVariableKind)>,
    ) -> UnlinkedFunction {
        let constants = descriptors
            .iter()
            .map(|(_, name, _, _, _)| {
                UnlinkedConstant::primitive(Value::String(JsString::from_static(name))).unwrap()
            })
            .collect();
        let closure_variables = descriptors
            .into_iter()
            .enumerate()
            .map(
                |(index, (source, _, is_lexical, is_const, kind))| ClosureVariable {
                    source,
                    name: ClosureVariableName::Constant(
                        u32::try_from(index).expect("test descriptor index fits u32"),
                    ),
                    is_lexical,
                    is_const,
                    kind,
                },
            )
            .collect::<Vec<_>>();
        UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::Undefined, Instruction::Return],
            constants,
            FunctionMetadata {
                closure_count: u16::try_from(closure_variables.len())
                    .expect("test descriptor count fits u16"),
                max_stack: 1,
                strict,
                eval_kind,
                ..FunctionMetadata::default()
            },
            closure_variables,
        )
    }

    fn recursive_eval_root(
        imported_body_kind: EvalScopeKind,
        binding_sources: [u16; 2],
        variable_target: u16,
    ) -> UnlinkedFunction {
        let names = ["<var:outer>", "<var:inner>"];
        let constants = names
            .iter()
            .map(|name| {
                UnlinkedConstant::primitive(Value::String(JsString::from_static(name))).unwrap()
            })
            .collect::<Vec<_>>();
        let descriptors = names
            .iter()
            .enumerate()
            .map(|(index, _)| ClosureVariable {
                source: ClosureSource::EvalEnvironment(
                    u16::try_from(index).expect("test closure index fits u16"),
                ),
                name: ClosureVariableName::Constant(
                    u32::try_from(index).expect("test constant index fits u32"),
                ),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::EvalVariableObject,
            })
            .collect::<Vec<_>>();
        let imported_bindings = names
            .iter()
            .zip(binding_sources)
            .map(|(name, source)| EvalBinding {
                name: JsString::from_static(name),
                source: EvalBindingSource::Closure(source),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::EvalVariableObject,
                is_catch_parameter: false,
            })
            .collect::<Vec<_>>();
        UnlinkedFunction::new_with_closure_variables(
            eval_code(0, false),
            constants,
            FunctionMetadata {
                closure_count: 2,
                max_stack: 1,
                eval_kind: EvalKind::Direct,
                ..FunctionMetadata::default()
            },
            descriptors,
        )
        .with_eval_environments(vec![EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: imported_body_kind,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: imported_bindings.into_boxed_slice(),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Closure(variable_target),
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        }])
    }

    #[test]
    fn eval_root_authenticates_ordered_caller_bindings_and_globals() {
        let expected = [
            eval_root_binding("outer", 3, false, false, ClosureVariableKind::Normal),
            eval_root_binding("inner", 0, true, true, ClosureVariableKind::Normal),
        ];
        let root = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![
                (
                    ClosureSource::EvalEnvironment(0),
                    "outer",
                    false,
                    false,
                    ClosureVariableKind::Normal,
                ),
                (
                    ClosureSource::EvalEnvironment(1),
                    "inner",
                    true,
                    true,
                    ClosureVariableKind::Normal,
                ),
                (
                    ClosureSource::Global,
                    "globalName",
                    false,
                    false,
                    ClosureVariableKind::Normal,
                ),
            ],
        );

        verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &expected, false, false).unwrap();
    }

    #[test]
    fn eval_root_authenticates_with_object_metadata_and_keeps_it_out_of_variable_targets() {
        let expected = [eval_root_binding(
            "<with>",
            0,
            false,
            false,
            ClosureVariableKind::WithObject,
        )];
        let root = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::EvalEnvironment(0),
                "<with>",
                false,
                false,
                ClosureVariableKind::WithObject,
            )],
        );
        let profile = EvalCallerProfile {
            scope_kinds: vec![EvalScopeKind::With].into_boxed_slice(),
            variable_target: EvalCallerVariableTarget::Global,
        };
        verify_unlinked_eval_tree_with_profile(
            &root,
            EvalKind::Direct,
            false,
            &expected,
            &profile,
            false,
            false,
        )
        .unwrap();

        let forged_target = EvalCallerProfile {
            scope_kinds: vec![EvalScopeKind::With].into_boxed_slice(),
            variable_target: EvalCallerVariableTarget::ExternalBinding(0),
        };
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &root,
                EvalKind::Direct,
                false,
                &expected,
                &forged_target,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("variable target is not authenticated")
        );

        let wrong_name = [eval_root_binding(
            "ordinary",
            0,
            false,
            false,
            ClosureVariableKind::WithObject,
        )];
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &root,
                EvalKind::Direct,
                false,
                &wrong_name,
                &profile,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("with-object binding has invalid")
        );
    }

    #[test]
    fn eval_root_rejects_incoherent_caller_variable_profiles() {
        let strict = eval_root_with_descriptors_and_strict(EvalKind::Direct, true, Vec::new());
        let global_profile = EvalCallerProfile {
            scope_kinds: Box::new([]),
            variable_target: EvalCallerVariableTarget::Global,
        };
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &strict,
                EvalKind::Direct,
                true,
                &[],
                &global_profile,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("variable target is not authenticated")
        );

        let expected = [eval_root_binding(
            "<var>",
            0,
            false,
            false,
            ClosureVariableKind::EvalVariableObject,
        )];
        let imported = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::EvalEnvironment(0),
                "<var>",
                false,
                false,
                ClosureVariableKind::EvalVariableObject,
            )],
        );
        let forged_global = EvalCallerProfile {
            scope_kinds: vec![EvalScopeKind::FunctionRoot].into_boxed_slice(),
            variable_target: EvalCallerVariableTarget::Global,
        };
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &imported,
                EvalKind::Direct,
                false,
                &expected,
                &forged_global,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("variable target is not authenticated")
        );
    }

    #[test]
    fn recursive_eval_root_authenticates_imported_suffix_origins_and_target() {
        let expected = [
            eval_root_binding(
                "<var:outer>",
                1,
                false,
                false,
                ClosureVariableKind::EvalVariableObject,
            ),
            eval_root_binding(
                "<var:inner>",
                1,
                false,
                false,
                ClosureVariableKind::EvalVariableObject,
            ),
        ];
        let profile = EvalCallerProfile {
            scope_kinds: vec![EvalScopeKind::FunctionBody, EvalScopeKind::FunctionRoot]
                .into_boxed_slice(),
            variable_target: EvalCallerVariableTarget::ExternalBinding(1),
        };
        verify_unlinked_eval_tree_with_profile(
            &recursive_eval_root(EvalScopeKind::FunctionBody, [0, 1], 1),
            EvalKind::Direct,
            false,
            &expected,
            &profile,
            false,
            false,
        )
        .unwrap();

        let wrong_kind = recursive_eval_root(EvalScopeKind::Catch, [0, 1], 1);
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &wrong_kind,
                EvalKind::Direct,
                false,
                &expected,
                &profile,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("imported scope kind")
        );

        let wrong_origin = recursive_eval_root(EvalScopeKind::FunctionBody, [1, 0], 1);
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &wrong_origin,
                EvalKind::Direct,
                false,
                &expected,
                &profile,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("wrong caller origin")
        );

        let wrong_target = recursive_eval_root(EvalScopeKind::FunctionBody, [0, 1], 0);
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &wrong_target,
                EvalKind::Direct,
                false,
                &expected,
                &profile,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("variable target disagrees")
        );
    }

    #[test]
    fn script_and_eval_root_publication_are_not_interchangeable() {
        let eval_root = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
        assert!(
            verify_unlinked_tree(&eval_root)
                .unwrap_err()
                .to_string()
                .contains("publication entry point")
        );

        let script_root = eval_root_with_descriptors(EvalKind::None, Vec::new());
        assert!(
            verify_unlinked_eval_tree(&script_root, EvalKind::Direct, false, &[], false, false)
                .unwrap_err()
                .to_string()
                .contains("publication entry point")
        );
        assert!(
            verify_unlinked_eval_tree(&script_root, EvalKind::None, false, &[], false, false)
                .is_err()
        );
    }

    #[test]
    fn eval_root_rejects_caller_binding_spoofs() {
        let expected = [
            eval_root_binding("outer", 1, false, false, ClosureVariableKind::Normal),
            eval_root_binding("inner", 0, true, false, ClosureVariableKind::Normal),
        ];

        let wrong_name = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::EvalEnvironment(0),
                "spoofed",
                false,
                false,
                ClosureVariableKind::Normal,
            )],
        );
        assert!(
            verify_unlinked_eval_tree(
                &wrong_name,
                EvalKind::Direct,
                false,
                &expected[..1],
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("name disagrees")
        );

        let wrong_flags = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::EvalEnvironment(0),
                "outer",
                true,
                false,
                ClosureVariableKind::Normal,
            )],
        );
        assert!(
            verify_unlinked_eval_tree(
                &wrong_flags,
                EvalKind::Direct,
                false,
                &expected[..1],
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("flags disagree")
        );

        let wrong_kind = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::EvalEnvironment(0),
                "outer",
                false,
                false,
                ClosureVariableKind::FunctionName,
            )],
        );
        assert!(
            verify_unlinked_eval_tree(
                &wrong_kind,
                EvalKind::Direct,
                false,
                &expected[..1],
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("flags disagree")
        );

        let wrong_order = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![
                (
                    ClosureSource::EvalEnvironment(1),
                    "inner",
                    true,
                    false,
                    ClosureVariableKind::Normal,
                ),
                (
                    ClosureSource::EvalEnvironment(0),
                    "outer",
                    false,
                    false,
                    ClosureVariableKind::Normal,
                ),
            ],
        );
        assert!(
            verify_unlinked_eval_tree(
                &wrong_order,
                EvalKind::Direct,
                false,
                &expected,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("exact prefix")
        );

        let missing = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::EvalEnvironment(0),
                "outer",
                false,
                false,
                ClosureVariableKind::Normal,
            )],
        );
        assert!(
            verify_unlinked_eval_tree(&missing, EvalKind::Direct, false, &expected, false, false,)
                .unwrap_err()
                .to_string()
                .contains("descriptor count")
        );
    }

    #[test]
    fn eval_root_allows_only_sloppy_global_variable_environment_declarations() {
        let direct_global = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::GlobalDeclaration,
                "declared",
                false,
                false,
                ClosureVariableKind::Normal,
            )],
        );
        verify_unlinked_eval_tree(&direct_global, EvalKind::Direct, false, &[], false, false)
            .unwrap();

        let indirect_global = eval_root_with_descriptors(
            EvalKind::Indirect,
            vec![(
                ClosureSource::GlobalDeclaration,
                "declared",
                false,
                false,
                ClosureVariableKind::Normal,
            )],
        );
        verify_unlinked_eval_tree(
            &indirect_global,
            EvalKind::Indirect,
            false,
            &[],
            false,
            false,
        )
        .unwrap();

        for (kind, caller_strict) in [
            (EvalKind::Direct, false),
            (EvalKind::Direct, true),
            (EvalKind::Indirect, false),
        ] {
            let strict = eval_root_with_descriptors_and_strict(
                kind,
                true,
                vec![(
                    ClosureSource::GlobalDeclaration,
                    "declared",
                    false,
                    false,
                    ClosureVariableKind::Normal,
                )],
            );
            assert!(
                verify_unlinked_eval_tree(&strict, kind, caller_strict, &[], false, false)
                    .unwrap_err()
                    .to_string()
                    .contains("illegal global declaration")
            );
        }

        let lexical = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::GlobalDeclaration,
                "lexical",
                true,
                false,
                ClosureVariableKind::Normal,
            )],
        );
        assert!(
            verify_unlinked_eval_tree(&lexical, EvalKind::Direct, false, &[], false, false)
                .unwrap_err()
                .to_string()
                .contains("illegal global declaration")
        );
    }

    #[test]
    fn eval_root_rejects_global_declarations_with_a_hidden_variable_object() {
        let expected = [eval_root_binding(
            "<var>",
            0,
            false,
            false,
            ClosureVariableKind::EvalVariableObject,
        )];
        let imported = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::HasEvalVariable {
                    source: crate::bytecode::EvalVariableSource::Closure(0),
                    name: 0,
                },
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<var>"))).unwrap(),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                eval_kind: EvalKind::Direct,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::EvalEnvironment(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::EvalVariableObject,
            }],
        );
        verify_unlinked_eval_tree(&imported, EvalKind::Direct, false, &expected, false, false)
            .unwrap();

        let root = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![
                (
                    ClosureSource::EvalEnvironment(0),
                    "<var>",
                    false,
                    false,
                    ClosureVariableKind::EvalVariableObject,
                ),
                (
                    ClosureSource::GlobalDeclaration,
                    "declared",
                    false,
                    false,
                    ClosureVariableKind::Normal,
                ),
            ],
        );
        assert!(
            verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &expected, false, false,)
                .unwrap_err()
                .to_string()
                .contains("illegal global declaration")
        );
    }

    #[test]
    fn eval_root_requires_external_bindings_to_be_an_exact_prefix() {
        let expected = [eval_root_binding(
            "caller",
            0,
            false,
            false,
            ClosureVariableKind::Normal,
        )];
        let root = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![
                (
                    ClosureSource::Global,
                    "global",
                    false,
                    false,
                    ClosureVariableKind::Normal,
                ),
                (
                    ClosureSource::EvalEnvironment(0),
                    "caller",
                    false,
                    false,
                    ClosureVariableKind::Normal,
                ),
            ],
        );
        assert!(
            verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &expected, false, false,)
                .unwrap_err()
                .to_string()
                .contains("exact prefix")
        );
    }

    #[test]
    fn eval_root_rejects_forged_special_and_catch_binding_metadata() {
        let global_special = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::GlobalDeclaration,
                "<var>",
                false,
                false,
                ClosureVariableKind::EvalVariableObject,
            )],
        );
        assert!(
            verify_unlinked_eval_tree(&global_special, EvalKind::Direct, false, &[], false, false,)
                .unwrap_err()
                .to_string()
                .contains("non-global binding metadata")
        );

        let root = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
        let mut forged_catch =
            eval_root_binding("caught", 0, false, false, ClosureVariableKind::Normal);
        forged_catch.is_catch_parameter = true;
        assert!(
            verify_unlinked_eval_tree(
                &root,
                EvalKind::Direct,
                false,
                &[forged_catch],
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("catch binding")
        );
    }

    #[test]
    fn eval_root_rejects_child_eval_sources() {
        let child = eval_root_with_descriptors(
            EvalKind::None,
            vec![(
                ClosureSource::EvalEnvironment(0),
                "escaped",
                false,
                false,
                ClosureVariableKind::Normal,
            )],
        );
        let root = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                max_stack: 1,
                eval_kind: EvalKind::Direct,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &[], false, false)
                .unwrap_err()
                .to_string()
                .contains("child bytecode directly referenced")
        );
    }

    #[test]
    fn eval_variable_object_local_and_dynamic_sources_are_authenticated() {
        let special_source = crate::bytecode::EvalVariableSource::Local(0);
        let valid = UnlinkedFunction::new(
            vec![
                Instruction::VariableEnvironment,
                Instruction::PutLocal(0),
                Instruction::HasEvalVariable {
                    source: special_source,
                    name: 0,
                },
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("created")))
                    .unwrap(),
            ],
            FunctionMetadata {
                local_count: 1,
                eval_variable_object_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        verify_unlinked_tree(&script_with_child(valid)).unwrap();

        let no_prologue = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                eval_variable_object_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(no_prologue))
                .unwrap_err()
                .to_string()
                .contains("exact entry prologue")
        );

        let exposed = UnlinkedFunction::new(
            vec![
                Instruction::VariableEnvironment,
                Instruction::PutLocal(0),
                Instruction::GetLocal(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                eval_variable_object_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(exposed))
                .unwrap_err()
                .to_string()
                .contains("private eval variable object")
        );

        let forged_source = UnlinkedFunction::new(
            vec![
                Instruction::HasEvalVariable {
                    source: special_source,
                    name: 0,
                },
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("created")))
                    .unwrap(),
            ],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(forged_source))
                .unwrap_err()
                .to_string()
                .contains("authenticated local")
        );
    }

    #[test]
    fn dynamic_sources_and_reference_names_are_authenticated() {
        let forged_with = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::DynamicEnvironmentObject(DynamicEnvironmentSource::With(
                    WithObjectSource::Local(0),
                )),
            ],
            Vec::new(),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(forged_with))
                .unwrap_err()
                .to_string()
                .contains("authenticated local")
        );

        let out_of_bounds = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::DynamicEnvironmentObject(DynamicEnvironmentSource::With(
                    WithObjectSource::Closure(0),
                )),
            ],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(out_of_bounds))
                .unwrap_err()
                .to_string()
                .contains("source closure is out of bounds")
        );

        for instruction in [
            Instruction::GetRefValue(0),
            Instruction::GetRefValueUndef(0),
            Instruction::PutRefValue(0),
        ] {
            let non_string_name = UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return, instruction],
                vec![UnlinkedConstant::primitive(Value::Int(0)).unwrap()],
                FunctionMetadata {
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            );
            assert!(
                verify_unlinked_tree(&script_with_child(non_string_name))
                    .unwrap_err()
                    .to_string()
                    .contains("reference opcode referenced a non-string name constant")
            );
        }
    }

    #[test]
    fn identifier_rest_metadata_authenticates_its_exact_entry_pair() {
        let valid = UnlinkedFunction::new(
            vec![
                Instruction::Rest(1),
                Instruction::PutArg(1),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 2,
                defined_argument_count: 1,
                rest_parameter: Some(1),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        verify_unlinked_tree(&script_with_child(valid)).unwrap();

        let unauthenticated = UnlinkedFunction::new(
            vec![
                Instruction::Rest(1),
                Instruction::PutArg(1),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 2,
                defined_argument_count: 2,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(unauthenticated))
                .unwrap_err()
                .to_string()
                .contains("no authenticated parameter metadata")
        );

        let misplaced = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Drop,
                Instruction::Rest(1),
                Instruction::PutArg(1),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 2,
                defined_argument_count: 1,
                rest_parameter: Some(1),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(misplaced))
                .unwrap_err()
                .to_string()
                .contains("no exact entry initialization")
        );

        let wrong_target = UnlinkedFunction::new(
            vec![
                Instruction::Rest(1),
                Instruction::PutArg(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 2,
                defined_argument_count: 1,
                rest_parameter: Some(1),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(wrong_target))
                .unwrap_err()
                .to_string()
                .contains("no exact entry initialization")
        );

        let duplicate = UnlinkedFunction::new(
            vec![
                Instruction::Rest(1),
                Instruction::PutArg(1),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Rest(1),
                Instruction::PutArg(1),
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 2,
                defined_argument_count: 1,
                rest_parameter: Some(1),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(duplicate))
                .unwrap_err()
                .to_string()
                .contains("no exact entry initialization")
        );

        let mapped_arguments = UnlinkedFunction::new(
            vec![
                Instruction::Arguments(crate::bytecode::ArgumentsKind::Mapped),
                Instruction::PutLocal(0),
                Instruction::Rest(1),
                Instruction::PutArg(1),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 2,
                defined_argument_count: 1,
                rest_parameter: Some(1),
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(mapped_arguments))
                .unwrap_err()
                .to_string()
                .contains("malformed arguments prologue")
        );

        let forged_arguments_local = UnlinkedFunction::new(
            vec![
                Instruction::Arguments(crate::bytecode::ArgumentsKind::Unmapped),
                Instruction::PutLocal(0),
                Instruction::Rest(1),
                Instruction::PutArg(1),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 2,
                defined_argument_count: 1,
                rest_parameter: Some(1),
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(forged_arguments_local))
                .unwrap_err()
                .to_string()
                .contains("arguments binding is not authenticated")
        );

        let malformed_metadata = UnlinkedFunction::new(
            vec![
                Instruction::Rest(1),
                Instruction::PutArg(1),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                rest_parameter: Some(1),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(malformed_metadata))
                .unwrap_err()
                .to_string()
                .contains("metadata disagrees with argument slots")
        );

        let root_rest = UnlinkedFunction::new(
            vec![
                Instruction::Rest(1),
                Instruction::PutArg(1),
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 2,
                defined_argument_count: 1,
                rest_parameter: Some(1),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&root_rest)
                .unwrap_err()
                .to_string()
                .contains("metadata disagrees with argument slots")
        );
    }

    #[test]
    fn parameter_binding_pattern_metadata_authenticates_anonymous_entry_segments() {
        let ordinary_metadata = || FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            pattern_argument_count: 1,
            parameter_pattern_end: Some(2),
            local_count: 1,
            max_stack: 1,
            ..FunctionMetadata::default()
        };
        let ordinary_code = || {
            vec![
                Instruction::GetArg(0),
                Instruction::PutLocal(0),
                Instruction::Nop,
                Instruction::GetLocal(0),
                Instruction::Return,
            ]
        };
        let make_ordinary = |code, metadata| {
            UnlinkedFunction::new(code, Vec::new(), metadata).with_variable_definitions(
                vec![UnlinkedVariableDefinition::ordinary(None)],
                vec![UnlinkedVariableDefinition::ordinary(Some(
                    JsString::from_static("value"),
                ))],
            )
        };
        verify_unlinked_tree(&script_with_child(make_ordinary(
            ordinary_code(),
            ordinary_metadata(),
        )))
        .unwrap();

        let capture_child = |source, name: Option<&'static str>, is_lexical| {
            let constants = name
                .map(|name| {
                    vec![
                        UnlinkedConstant::primitive(Value::String(JsString::from_static(name)))
                            .unwrap(),
                    ]
                })
                .unwrap_or_default();
            UnlinkedFunction::new_with_closure_variables(
                vec![
                    if is_lexical {
                        Instruction::GetVarRefCheck(0)
                    } else {
                        Instruction::GetVarRef(0)
                    },
                    Instruction::Return,
                ],
                constants,
                FunctionMetadata {
                    closure_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
                vec![ClosureVariable {
                    source,
                    name: if name.is_some() {
                        ClosureVariableName::Constant(0)
                    } else {
                        ClosureVariableName::None
                    },
                    is_lexical,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                }],
            )
        };
        let pattern_closure_code = || {
            vec![
                Instruction::GetArg(0),
                Instruction::PutLocal(0),
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Nop,
                Instruction::GetLocal(0),
                Instruction::Return,
            ]
        };
        let mut closure_metadata = ordinary_metadata();
        closure_metadata.parameter_pattern_end = Some(4);

        let valid_root_capture = UnlinkedFunction::new(
            pattern_closure_code(),
            vec![UnlinkedConstant::child(capture_child(
                ClosureSource::ParentLocal(0),
                Some("value"),
                false,
            ))],
            closure_metadata,
        )
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(None)],
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("value"),
            ))],
        );
        verify_unlinked_tree(&script_with_child(valid_root_capture)).unwrap();

        let anonymous_argument_capture = UnlinkedFunction::new(
            pattern_closure_code(),
            vec![UnlinkedConstant::child(capture_child(
                ClosureSource::ParentArgument(0),
                None,
                false,
            ))],
            closure_metadata,
        )
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(None)],
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("value"),
            ))],
        );
        assert!(
            verify_unlinked_tree(&script_with_child(anonymous_argument_capture))
                .unwrap_err()
                .to_string()
                .contains("anonymous pattern argument slot")
        );

        let body_lexical_capture = UnlinkedFunction::new(
            pattern_closure_code(),
            vec![UnlinkedConstant::child(capture_child(
                ClosureSource::ParentLocal(1),
                Some("body"),
                true,
            ))],
            FunctionMetadata {
                local_count: 2,
                ..closure_metadata
            },
        )
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(None)],
            vec![
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
                UnlinkedVariableDefinition::lexical(Some(JsString::from_static("body")), false),
            ],
        );
        assert!(
            verify_unlinked_tree(&script_with_child(body_lexical_capture))
                .unwrap_err()
                .to_string()
                .contains("pattern initializer closure captured a body lexical local")
        );

        let direct_body_lexical_access = UnlinkedFunction::new(
            vec![
                Instruction::GetArg(0),
                Instruction::Drop,
                Instruction::GetLocalCheck(1),
                Instruction::Drop,
                Instruction::Nop,
                Instruction::GetLocal(0),
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                local_count: 2,
                parameter_pattern_end: Some(4),
                ..ordinary_metadata()
            },
        )
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(None)],
            vec![
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
                UnlinkedVariableDefinition::lexical(Some(JsString::from_static("body")), false),
            ],
        );
        assert!(
            verify_unlinked_tree(&script_with_child(direct_body_lexical_access))
                .unwrap_err()
                .to_string()
                .contains("accessed a body lexical local")
        );

        let rest_pattern = UnlinkedFunction::new(
            vec![
                Instruction::Rest(0),
                Instruction::PutLocal(0),
                Instruction::Nop,
                Instruction::GetLocal(0),
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 0,
                defined_argument_count: 1,
                rest_pattern_start: Some(0),
                parameter_pattern_end: Some(2),
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("value"),
            ))],
        );
        verify_unlinked_tree(&script_with_child(rest_pattern)).unwrap();

        let empty_rest_metadata = |defined_argument_count| FunctionMetadata {
            defined_argument_count,
            rest_pattern_start: Some(0),
            parameter_pattern_end: Some(2),
            max_stack: 1,
            ..FunctionMetadata::default()
        };
        let empty_rest_code = |body_value| {
            vec![
                Instruction::Rest(0),
                Instruction::Drop,
                Instruction::Nop,
                body_value,
                Instruction::Return,
            ]
        };
        verify_unlinked_tree(&script_with_child(UnlinkedFunction::new(
            empty_rest_code(Instruction::Undefined),
            Vec::new(),
            empty_rest_metadata(0),
        )))
        .unwrap();
        for pseudo_read in [
            Instruction::PushHomeObject,
            Instruction::PushThis,
            Instruction::PushNewTarget,
        ] {
            let mut metadata = empty_rest_metadata(1);
            metadata.needs_home_object = matches!(&pseudo_read, Instruction::PushHomeObject);
            verify_unlinked_tree(&script_with_child(UnlinkedFunction::new(
                empty_rest_code(pseudo_read),
                Vec::new(),
                metadata,
            )))
            .unwrap();
        }
        assert!(
            verify_unlinked_tree(&script_with_child(UnlinkedFunction::new(
                empty_rest_code(Instruction::Undefined),
                Vec::new(),
                empty_rest_metadata(1),
            )))
            .unwrap_err()
            .to_string()
            .contains("metadata disagrees with function length")
        );
        assert!(
            verify_unlinked_tree(&script_with_child(UnlinkedFunction::new(
                empty_rest_code(Instruction::PushThis),
                Vec::new(),
                empty_rest_metadata(0),
            )))
            .unwrap_err()
            .to_string()
            .contains("metadata disagrees with function length")
        );

        let mut missing_marker = ordinary_metadata();
        missing_marker.parameter_pattern_end = None;
        assert!(
            verify_unlinked_tree(&script_with_child(make_ordinary(
                ordinary_code(),
                missing_marker,
            )))
            .unwrap_err()
            .to_string()
            .contains("no initialization marker")
        );

        let named_slot = UnlinkedFunction::new(ordinary_code(), Vec::new(), ordinary_metadata())
            .with_variable_definitions(
                vec![UnlinkedVariableDefinition::ordinary(Some(
                    JsString::from_static("forged"),
                ))],
                vec![UnlinkedVariableDefinition::ordinary(Some(
                    JsString::from_static("value"),
                ))],
            );
        assert!(
            verify_unlinked_tree(&script_with_child(named_slot))
                .unwrap_err()
                .to_string()
                .contains("definitions disagree with bytecode metadata")
        );

        let mut missing_read = ordinary_code();
        missing_read[0] = Instruction::Undefined;
        assert!(
            verify_unlinked_tree(&script_with_child(make_ordinary(
                missing_read,
                ordinary_metadata(),
            )))
            .unwrap_err()
            .to_string()
            .contains("exact entry reads")
        );

        let mut body_read = ordinary_code();
        body_read.insert(3, Instruction::GetArg(0));
        body_read.insert(4, Instruction::Drop);
        assert!(
            verify_unlinked_tree(&script_with_child(make_ordinary(
                body_read,
                ordinary_metadata(),
            )))
            .unwrap_err()
            .to_string()
            .contains("function body reads")
        );

        let mut anonymous_write = ordinary_code();
        anonymous_write.insert(2, Instruction::Undefined);
        anonymous_write.insert(3, Instruction::PutArg(0));
        let mut write_metadata = ordinary_metadata();
        write_metadata.parameter_pattern_end = Some(4);
        assert!(
            verify_unlinked_tree(&script_with_child(make_ordinary(
                anonymous_write,
                write_metadata,
            )))
            .unwrap_err()
            .to_string()
            .contains("writes an anonymous")
        );

        let escaped_segment = vec![
            Instruction::GetArg(0),
            Instruction::Drop,
            Instruction::Goto(4),
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ];
        let mut escaped_metadata = ordinary_metadata();
        escaped_metadata.parameter_pattern_end = Some(3);
        assert!(
            verify_unlinked_tree(&script_with_child(make_ordinary(
                escaped_segment,
                escaped_metadata,
            )))
            .unwrap_err()
            .to_string()
            .contains("escaped its initialization segment")
        );

        let body_reentry = vec![
            Instruction::GetArg(0),
            Instruction::Drop,
            Instruction::Nop,
            Instruction::Goto(2),
        ];
        assert!(
            verify_unlinked_tree(&script_with_child(make_ordinary(
                body_reentry,
                ordinary_metadata(),
            )))
            .unwrap_err()
            .to_string()
            .contains("jumps back into pattern initialization")
        );

        let mapped_arguments = UnlinkedFunction::new(
            vec![
                Instruction::Arguments(crate::bytecode::ArgumentsKind::Mapped),
                Instruction::PutLocal(0),
                Instruction::GetArg(0),
                Instruction::PutLocal(1),
                Instruction::Nop,
                Instruction::GetLocal(1),
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                pattern_argument_count: 1,
                parameter_pattern_end: Some(4),
                local_count: 2,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(None)],
            vec![
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("arguments"))),
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
            ],
        );
        assert!(
            verify_unlinked_tree(&script_with_child(mapped_arguments))
                .unwrap_err()
                .to_string()
                .contains("malformed arguments prologue")
        );

        assert!(
            verify_unlinked_tree(&make_ordinary(ordinary_code(), ordinary_metadata()))
                .unwrap_err()
                .to_string()
                .contains("synthetic root contains formal-parameter metadata")
        );
    }

    #[test]
    fn identifier_default_metadata_authenticates_parameter_environment_layout() {
        let parameter_environment = |code: &[Instruction], metadata: FunctionMetadata| {
            let initialization_end = code
                .iter()
                .position(|instruction| matches!(instruction, Instruction::Nop))
                .and_then(|pc| u32::try_from(pc).ok())
                .expect("parameter fixture has an initialization marker");
            ParameterEnvironmentLayout {
                initialization_end,
                argument_cells: (0..metadata.argument_count)
                    .map(|argument| ParameterArgumentCell {
                        argument,
                        parameter_local: argument,
                        body: ParameterBodyStorage::Argument(argument),
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                pattern_copies: Box::new([]),
                default_sources: (0..metadata.argument_count)
                    .filter(|argument| *argument >= metadata.defined_argument_count)
                    .filter(|argument| metadata.rest_parameter != Some(*argument))
                    .map(ParameterDefaultSource::Argument)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                synthetic_arguments_local: None,
                arg_eval_variable_object_local: None,
            }
        };
        let metadata = || FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 0,
            rest_parameter: Some(1),
            parameter_environment_local_count: 2,
            local_count: 2,
            max_stack: 3,
            ..FunctionMetadata::default()
        };
        let code = || {
            vec![
                Instruction::SetLocalUninitialized(1),
                Instruction::SetLocalUninitialized(0),
                Instruction::GetArg(0),
                Instruction::Dup,
                Instruction::Undefined,
                Instruction::StrictEq,
                Instruction::IfFalse(11),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Dup,
                Instruction::PutArg(0),
                Instruction::InitializeLocal(0),
                Instruction::Rest(1),
                Instruction::Dup,
                Instruction::PutArg(1),
                Instruction::InitializeLocal(1),
                Instruction::Nop,
                Instruction::Undefined,
                Instruction::Return,
            ]
        };
        let make_with_constants = |code: Vec<Instruction>,
                                   constants: Vec<UnlinkedConstant>,
                                   metadata: FunctionMetadata| {
            let layout = (metadata.parameter_environment_local_count != 0)
                .then(|| parameter_environment(&code, metadata));
            UnlinkedFunction::new(code, constants, metadata)
                .with_parameter_environment(layout)
                .with_variable_definitions(
                    vec![
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("rest"))),
                    ],
                    vec![
                        UnlinkedVariableDefinition::lexical(
                            Some(JsString::from_static("a")),
                            false,
                        ),
                        UnlinkedVariableDefinition::lexical(
                            Some(JsString::from_static("rest")),
                            false,
                        ),
                    ],
                )
        };
        let make = |code, metadata| make_with_constants(code, Vec::new(), metadata);
        let make_with_body_local = |code: Vec<Instruction>, metadata: FunctionMetadata| {
            let layout = parameter_environment(&code, metadata);
            UnlinkedFunction::new(code, Vec::new(), metadata)
                .with_parameter_environment(Some(layout))
                .with_variable_definitions(
                    vec![
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("rest"))),
                    ],
                    vec![
                        UnlinkedVariableDefinition::lexical(
                            Some(JsString::from_static("a")),
                            false,
                        ),
                        UnlinkedVariableDefinition::lexical(
                            Some(JsString::from_static("rest")),
                            false,
                        ),
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("body"))),
                    ],
                )
        };
        let capture_child = |source, is_lexical| {
            UnlinkedFunction::new_with_closure_variables(
                vec![
                    if is_lexical {
                        Instruction::GetVarRefCheck(0)
                    } else {
                        Instruction::GetVarRef(0)
                    },
                    Instruction::Return,
                ],
                vec![
                    UnlinkedConstant::primitive(Value::String(JsString::from_static("a"))).unwrap(),
                ],
                FunctionMetadata {
                    closure_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
                vec![ClosureVariable {
                    source,
                    name: ClosureVariableName::Constant(0),
                    is_lexical,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                }],
            )
        };

        verify_unlinked_tree(&script_with_child(make(code(), metadata()))).unwrap();

        let default_only_metadata = || FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 0,
            parameter_environment_local_count: 1,
            local_count: 1,
            max_stack: 3,
            ..FunctionMetadata::default()
        };
        let default_only_code = || {
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::GetArg(0),
                Instruction::Dup,
                Instruction::Undefined,
                Instruction::StrictEq,
                Instruction::IfFalse(10),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Dup,
                Instruction::PutArg(0),
                Instruction::InitializeLocal(0),
                Instruction::Nop,
                Instruction::Undefined,
                Instruction::Return,
            ]
        };
        let make_default_only = |code: Vec<Instruction>,
                                 constants: Vec<UnlinkedConstant>,
                                 metadata: FunctionMetadata| {
            let layout = (metadata.parameter_environment_local_count != 0)
                .then(|| parameter_environment(&code, metadata));
            UnlinkedFunction::new(code, constants, metadata)
                .with_parameter_environment(layout)
                .with_variable_definitions(
                    vec![UnlinkedVariableDefinition::ordinary(Some(
                        JsString::from_static("value"),
                    ))],
                    vec![UnlinkedVariableDefinition::lexical(
                        Some(JsString::from_static("value")),
                        false,
                    )],
                )
        };
        verify_unlinked_tree(&script_with_child(make_default_only(
            default_only_code(),
            Vec::new(),
            default_only_metadata(),
        )))
        .unwrap();

        let mut missing_default_only_environment = default_only_metadata();
        missing_default_only_environment.parameter_environment_local_count = 0;
        assert!(
            verify_unlinked_tree(&script_with_child(make_default_only(
                default_only_code(),
                Vec::new(),
                missing_default_only_environment,
            )))
            .unwrap_err()
            .to_string()
            .contains("default parameter metadata has no parameter environment")
        );

        let eval_descendant_environment = EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Scope(1),
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        let plain_eval_descendant = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![eval_descendant_environment.clone()]);
        let plain_parent = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            vec![UnlinkedConstant::child(plain_eval_descendant)],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        verify_unlinked_tree(&script_with_child(plain_parent)).unwrap();

        let eval_descendant = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![eval_descendant_environment]);
        let parameter_parent = make_default_only(
            default_only_code(),
            vec![UnlinkedConstant::child(eval_descendant)],
            default_only_metadata(),
        );
        assert!(
            verify_unlinked_tree(&script_with_child(parameter_parent))
                .unwrap_err()
                .to_string()
                .contains("below a parameter environment")
        );

        let mut missing_environment = metadata();
        missing_environment.parameter_environment_local_count = 0;
        assert!(
            verify_unlinked_tree(&script_with_child(make(code(), missing_environment)))
                .unwrap_err()
                .to_string()
                .contains("metadata disagrees with argument slots")
        );

        let mut wrong_tdz = code();
        wrong_tdz.swap(0, 1);
        assert!(
            verify_unlinked_tree(&script_with_child(make(wrong_tdz, metadata())))
                .unwrap_err()
                .to_string()
                .contains("exact TDZ entry initialization")
        );

        let mut forged_pseudo_code = code();
        forged_pseudo_code.insert(0, Instruction::PushThis);
        forged_pseudo_code.insert(1, Instruction::PutLocal(2));
        forged_pseudo_code[8] = Instruction::IfFalse(13);
        let mut forged_pseudo_metadata = metadata();
        forged_pseudo_metadata.local_count = 3;
        let forged_pseudo_layout =
            parameter_environment(&forged_pseudo_code, forged_pseudo_metadata);
        let forged_pseudo =
            UnlinkedFunction::new(forged_pseudo_code, Vec::new(), forged_pseudo_metadata)
                .with_parameter_environment(Some(forged_pseudo_layout))
                .with_variable_definitions(
                    vec![
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("rest"))),
                    ],
                    vec![
                        UnlinkedVariableDefinition::lexical(
                            Some(JsString::from_static("a")),
                            false,
                        ),
                        UnlinkedVariableDefinition::lexical(
                            Some(JsString::from_static("rest")),
                            false,
                        ),
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("forged"))),
                    ],
                );
        assert!(
            verify_unlinked_tree(&script_with_child(forged_pseudo))
                .unwrap_err()
                .to_string()
                .contains("pseudo-binding definition is not authenticated")
        );

        let mut pseudo_write_code = code();
        pseudo_write_code.insert(0, Instruction::PushThis);
        pseudo_write_code.insert(1, Instruction::PutLocal(2));
        pseudo_write_code[8] = Instruction::IfFalse(14);
        pseudo_write_code.insert(11, Instruction::SetLocal(2));
        let mut pseudo_write_metadata = metadata();
        pseudo_write_metadata.local_count = 3;
        let pseudo_write_layout = parameter_environment(&pseudo_write_code, pseudo_write_metadata);
        let pseudo_write =
            UnlinkedFunction::new(pseudo_write_code, Vec::new(), pseudo_write_metadata)
                .with_parameter_environment(Some(pseudo_write_layout))
                .with_variable_definitions(
                    vec![
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("rest"))),
                    ],
                    vec![
                        UnlinkedVariableDefinition::lexical(
                            Some(JsString::from_static("a")),
                            false,
                        ),
                        UnlinkedVariableDefinition::lexical(
                            Some(JsString::from_static("rest")),
                            false,
                        ),
                        UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<this>"))),
                    ],
                );
        assert!(
            verify_unlinked_tree(&script_with_child(pseudo_write))
                .unwrap_err()
                .to_string()
                .contains("initializer has an unauthenticated local access")
        );

        let mut wrong_rest_target = code();
        wrong_rest_target[14] = Instruction::PutArg(0);
        assert!(
            verify_unlinked_tree(&script_with_child(make(wrong_rest_target, metadata())))
                .unwrap_err()
                .to_string()
                .contains("rest parameter has no exact initialization")
        );

        let missing_default = vec![
            Instruction::SetLocalUninitialized(1),
            Instruction::SetLocalUninitialized(0),
            Instruction::GetArg(0),
            Instruction::InitializeLocal(0),
            Instruction::Rest(1),
            Instruction::Dup,
            Instruction::PutArg(1),
            Instruction::InitializeLocal(1),
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ];
        assert!(
            verify_unlinked_tree(&script_with_child(make(missing_default, metadata())))
                .unwrap_err()
                .to_string()
                .contains("first default parameter has no default entry initialization")
        );

        let mut wrong_default_target = code();
        wrong_default_target[6] = Instruction::IfFalse(12);
        assert!(
            verify_unlinked_tree(&script_with_child(make(wrong_default_target, metadata())))
                .unwrap_err()
                .to_string()
                .contains("default parameter has no exact selection branch")
        );

        let mut missing_default_sync = code();
        missing_default_sync[10] = Instruction::Drop;
        assert!(
            verify_unlinked_tree(&script_with_child(make(missing_default_sync, metadata())))
                .unwrap_err()
                .to_string()
                .contains("default parameter has no exact argument synchronization")
        );

        let mut wrong_default_argument = code();
        wrong_default_argument[10] = Instruction::PutArg(1);
        assert!(
            verify_unlinked_tree(&script_with_child(make(wrong_default_argument, metadata())))
                .unwrap_err()
                .to_string()
                .contains("default parameter has no exact argument synchronization")
        );

        let mut raw_argument_rhs = code();
        raw_argument_rhs[8] = Instruction::GetArg(1);
        assert!(
            verify_unlinked_tree(&script_with_child(make(raw_argument_rhs, metadata())))
                .unwrap_err()
                .to_string()
                .contains("initializer bypasses parameter cells")
        );

        let mut unchecked_parameter_rhs = code();
        unchecked_parameter_rhs[8] = Instruction::GetLocal(0);
        assert!(
            verify_unlinked_tree(&script_with_child(make(
                unchecked_parameter_rhs,
                metadata()
            )))
            .unwrap_err()
            .to_string()
            .contains("initializer has an unauthenticated local access")
        );

        let mut body_local_rhs = code();
        body_local_rhs[8] = Instruction::GetLocal(2);
        let mut body_local_metadata = metadata();
        body_local_metadata.local_count = 3;
        assert!(
            verify_unlinked_tree(&script_with_child(make_with_body_local(
                body_local_rhs,
                body_local_metadata,
            )))
            .unwrap_err()
            .to_string()
            .contains("initializer has an unauthenticated local access")
        );

        let mut initializer_parameter_capture = code();
        initializer_parameter_capture[8] = Instruction::FClosure(0);
        initializer_parameter_capture.insert(17, Instruction::CloseLocal(0));
        verify_unlinked_tree(&script_with_child(make_with_constants(
            initializer_parameter_capture,
            vec![UnlinkedConstant::child(capture_child(
                ClosureSource::ParentLocal(0),
                true,
            ))],
            metadata(),
        )))
        .unwrap();

        let mut initializer_raw_argument_capture = code();
        initializer_raw_argument_capture[8] = Instruction::FClosure(0);
        assert!(
            verify_unlinked_tree(&script_with_child(make_with_constants(
                initializer_raw_argument_capture,
                vec![UnlinkedConstant::child(capture_child(
                    ClosureSource::ParentArgument(0),
                    false,
                ))],
                metadata(),
            )))
            .unwrap_err()
            .to_string()
            .contains("initializer closure captured a raw argument slot")
        );

        let mut body_raw_argument_capture = code();
        body_raw_argument_capture.insert(17, Instruction::FClosure(0));
        body_raw_argument_capture.insert(18, Instruction::Drop);
        verify_unlinked_tree(&script_with_child(make_with_constants(
            body_raw_argument_capture,
            vec![UnlinkedConstant::child(capture_child(
                ClosureSource::ParentArgument(0),
                false,
            ))],
            metadata(),
        )))
        .unwrap();

        let mut body_parameter_capture = code();
        body_parameter_capture.insert(17, Instruction::CloseLocal(0));
        body_parameter_capture.insert(18, Instruction::FClosure(0));
        body_parameter_capture.insert(19, Instruction::Drop);
        assert!(
            verify_unlinked_tree(&script_with_child(make_with_constants(
                body_parameter_capture,
                vec![UnlinkedConstant::child(capture_child(
                    ClosureSource::ParentLocal(0),
                    true,
                ))],
                metadata(),
            )))
            .unwrap_err()
            .to_string()
            .contains("body closure captured a parameter-environment cell")
        );

        let mut body_parameter_cell = code();
        body_parameter_cell.insert(17, Instruction::GetLocalCheck(0));
        assert!(
            verify_unlinked_tree(&script_with_child(make(body_parameter_cell, metadata())))
                .unwrap_err()
                .to_string()
                .contains("function body accesses a parameter-environment cell")
        );

        let mut own_eval = code();
        own_eval.insert(
            own_eval.len() - 1,
            Instruction::Eval {
                argument_count: 0,
                environment: 0,
            },
        );
        assert!(
            verify_unlinked_tree(&script_with_child(make(own_eval, metadata())))
                .unwrap_err()
                .to_string()
                .contains("direct eval")
        );

        let ordinary_code = code();
        let ordinary_metadata = metadata();
        let ordinary_layout = parameter_environment(&ordinary_code, ordinary_metadata);
        let ordinary_locals = UnlinkedFunction::new(ordinary_code, Vec::new(), ordinary_metadata)
            .with_parameter_environment(Some(ordinary_layout));
        assert!(
            verify_unlinked_tree(&script_with_child(ordinary_locals))
                .unwrap_err()
                .to_string()
                .contains("parameter environment cell definition is not authenticated")
        );

        let mut root_metadata = metadata();
        root_metadata.rest_parameter = None;
        let error = verify_unlinked_tree(&make(code(), root_metadata)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("synthetic root contains parameter-environment metadata"),
            "{error}"
        );
    }

    #[test]
    fn mixed_parameter_environment_metadata_authenticates_cells_copies_and_defaults() {
        let metadata = |marker| FunctionMetadata {
            argument_count: 2,
            defined_argument_count: 2,
            parameter_environment_local_count: 2,
            pattern_argument_count: 1,
            parameter_pattern_end: Some(marker),
            local_count: 3,
            max_stack: 1,
            ..FunctionMetadata::default()
        };
        let layout = |marker| ParameterEnvironmentLayout {
            initialization_end: marker,
            argument_cells: vec![ParameterArgumentCell {
                argument: 0,
                parameter_local: 0,
                body: ParameterBodyStorage::Argument(0),
            }]
            .into_boxed_slice(),
            pattern_copies: vec![ParameterPatternCopy {
                parameter_local: 1,
                body_local: 2,
            }]
            .into_boxed_slice(),
            default_sources: Box::new([]),
            synthetic_arguments_local: None,
            arg_eval_variable_object_local: None,
        };
        let code = || {
            vec![
                Instruction::SetLocalUninitialized(1),
                Instruction::SetLocalUninitialized(0),
                Instruction::GetArg(0),
                Instruction::InitializeLocal(0),
                Instruction::GetArg(1),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::InitializeLocal(1),
                Instruction::GetLocalCheck(1),
                Instruction::PutLocal(2),
                Instruction::Nop,
                Instruction::GetLocal(2),
                Instruction::Return,
            ]
        };
        let definitions = |function: UnlinkedFunction, target_name, target_lexical| {
            function.with_variable_definitions(
                vec![
                    UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("a"))),
                    UnlinkedVariableDefinition::ordinary(None),
                ],
                vec![
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("a")), false),
                    UnlinkedVariableDefinition::lexical(Some(JsString::from_static("b")), false),
                    if target_lexical {
                        UnlinkedVariableDefinition::lexical(Some(target_name), false)
                    } else {
                        UnlinkedVariableDefinition::ordinary(Some(target_name))
                    },
                ],
            )
        };
        let make = |code, metadata, layout| {
            definitions(
                UnlinkedFunction::new(code, Vec::new(), metadata)
                    .with_parameter_environment(layout),
                JsString::from_static("b"),
                false,
            )
        };

        verify_unlinked_tree(&script_with_child(make(
            code(),
            metadata(10),
            Some(layout(10)),
        )))
        .unwrap();

        let error =
            verify_unlinked_tree(&script_with_child(make(code(), metadata(10), None))).unwrap_err();
        assert!(error.to_string().contains("immutable layout"), "{error}");

        let mut wrong_copy_source = layout(10);
        wrong_copy_source.pattern_copies[0].parameter_local = 0;
        let error = verify_unlinked_tree(&script_with_child(make(
            code(),
            metadata(10),
            Some(wrong_copy_source),
        )))
        .unwrap_err();
        assert!(
            error.to_string().contains("copy source overlaps"),
            "{error}"
        );

        let mut wrong_tdz = code();
        wrong_tdz.swap(0, 1);
        let error = verify_unlinked_tree(&script_with_child(make(
            wrong_tdz,
            metadata(10),
            Some(layout(10)),
        )))
        .unwrap_err();
        assert!(error.to_string().contains("exact TDZ"), "{error}");

        let mut duplicate_initializer = code();
        duplicate_initializer[7] = Instruction::InitializeLocal(0);
        let error = verify_unlinked_tree(&script_with_child(make(
            duplicate_initializer,
            metadata(10),
            Some(layout(10)),
        )))
        .unwrap_err();
        assert!(
            error.to_string().contains("one exact initializer"),
            "{error}"
        );

        let mut wrong_copy_code = code();
        wrong_copy_code[8] = Instruction::GetLocalCheck(0);
        let error = verify_unlinked_tree(&script_with_child(make(
            wrong_copy_code,
            metadata(10),
            Some(layout(10)),
        )))
        .unwrap_err();
        assert!(error.to_string().contains("copy phase"), "{error}");

        let mut body_cell_read = code();
        body_cell_read.insert(11, Instruction::GetLocalCheck(1));
        body_cell_read.insert(12, Instruction::Drop);
        let error = verify_unlinked_tree(&script_with_child(make(
            body_cell_read,
            metadata(10),
            Some(layout(10)),
        )))
        .unwrap_err();
        assert!(error.to_string().contains("body accesses"), "{error}");

        let mut bypassed_plain_cell = code();
        bypassed_plain_cell[2] = Instruction::Undefined;
        let error = verify_unlinked_tree(&script_with_child(make(
            bypassed_plain_cell,
            metadata(10),
            Some(layout(10)),
        )))
        .unwrap_err();
        assert!(
            error.to_string().contains("plain parameter argument cell"),
            "{error}"
        );

        let mut raw_named_bypass = code();
        raw_named_bypass.insert(8, Instruction::GetArg(0));
        raw_named_bypass.insert(9, Instruction::Drop);
        let error = verify_unlinked_tree(&script_with_child(make(
            raw_named_bypass,
            metadata(12),
            Some(layout(12)),
        )))
        .unwrap_err();
        assert!(
            error.to_string().contains("plain parameter argument cell"),
            "{error}"
        );

        let wrong_target_definition = definitions(
            UnlinkedFunction::new(code(), Vec::new(), metadata(10))
                .with_parameter_environment(Some(layout(10))),
            JsString::from_static("not_b"),
            false,
        );
        let error = verify_unlinked_tree(&script_with_child(wrong_target_definition)).unwrap_err();
        assert!(error.to_string().contains("same-name"), "{error}");

        let lexical_target_definition = definitions(
            UnlinkedFunction::new(code(), Vec::new(), metadata(10))
                .with_parameter_environment(Some(layout(10))),
            JsString::from_static("b"),
            true,
        );
        let error =
            verify_unlinked_tree(&script_with_child(lexical_target_definition)).unwrap_err();
        assert!(error.to_string().contains("body lexical local"), "{error}");

        let default_metadata = FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 0,
            pattern_argument_count: 1,
            parameter_pattern_end: Some(10),
            max_stack: 3,
            ..FunctionMetadata::default()
        };
        let default_layout = ParameterEnvironmentLayout {
            initialization_end: 10,
            argument_cells: Box::new([]),
            pattern_copies: Box::new([]),
            default_sources: vec![ParameterDefaultSource::Argument(0)].into_boxed_slice(),
            synthetic_arguments_local: None,
            arg_eval_variable_object_local: None,
        };
        let default_code = vec![
            Instruction::GetArg(0),
            Instruction::Dup,
            Instruction::Undefined,
            Instruction::StrictEq,
            Instruction::IfTrue(7),
            Instruction::Drop,
            Instruction::Goto(10),
            Instruction::Drop,
            Instruction::Undefined,
            Instruction::Goto(5),
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ];
        let make_default = |code, metadata, layout| {
            UnlinkedFunction::new(code, Vec::new(), metadata)
                .with_parameter_environment(Some(layout))
                .with_variable_definitions(
                    vec![UnlinkedVariableDefinition::ordinary(None)],
                    Vec::new(),
                )
        };
        verify_unlinked_tree(&script_with_child(make_default(
            default_code,
            default_metadata,
            default_layout.clone(),
        )))
        .unwrap();

        let missing_selection = vec![
            Instruction::GetArg(0),
            Instruction::Drop,
            Instruction::Nop,
            Instruction::Undefined,
            Instruction::Return,
        ];
        let missing_selection_metadata = FunctionMetadata {
            parameter_pattern_end: Some(2),
            max_stack: 1,
            ..default_metadata
        };
        let mut missing_selection_layout = default_layout;
        missing_selection_layout.initialization_end = 2;
        let error = verify_unlinked_tree(&script_with_child(make_default(
            missing_selection,
            missing_selection_metadata,
            missing_selection_layout,
        )))
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("default has no exact argument selection"),
            "{error}"
        );
    }

    #[test]
    fn global_reference_operand_accepts_only_named_global_closures() {
        let name = JsString::from_static("globalName");
        let valid = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::GlobalReference(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::Global,
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        verify_unlinked_tree(&valid).unwrap();

        let local_child = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::GlobalReference(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        let local_parent = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(local_child)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::ordinary(Some(name))],
        );
        assert!(
            verify_unlinked_tree(&local_parent)
                .unwrap_err()
                .to_string()
                .contains("global closure opcode referenced a non-global closure descriptor")
        );

        let out_of_bounds = UnlinkedFunction::new(
            vec![Instruction::GlobalReference(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&out_of_bounds)
                .unwrap_err()
                .to_string()
                .contains("closure variable bytecode operand is out of bounds")
        );
    }

    #[test]
    fn eval_variable_object_can_relay_only_through_a_special_closure() {
        let name = JsString::from_static("<var>");
        let child = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::HasEvalVariable {
                    source: crate::bytecode::EvalVariableSource::Closure(0),
                    name: 0,
                },
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::EvalVariableObject,
            }],
        );
        let parent = UnlinkedFunction::new(
            vec![
                Instruction::VariableEnvironment,
                Instruction::PutLocal(0),
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                local_count: 1,
                eval_variable_object_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        verify_unlinked_tree(&script_with_child(parent)).unwrap();
    }

    #[test]
    fn eval_kind_is_root_only_and_indirect_eval_has_no_caller_bindings() {
        let child = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
        let root = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                max_stack: 1,
                eval_kind: EvalKind::Direct,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &[], false, false)
                .unwrap_err()
                .to_string()
                .contains("non-root bytecode")
        );

        let indirect = eval_root_with_descriptors(
            EvalKind::Indirect,
            vec![(
                ClosureSource::Global,
                "globalName",
                false,
                false,
                ClosureVariableKind::Normal,
            )],
        );
        verify_unlinked_eval_tree(&indirect, EvalKind::Indirect, false, &[], false, false).unwrap();
        let caller_binding =
            eval_root_binding("caller", 0, false, false, ClosureVariableKind::Normal);
        assert!(
            verify_unlinked_eval_tree(
                &indirect,
                EvalKind::Indirect,
                false,
                &[caller_binding],
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("received caller bindings")
        );
        assert!(
            verify_unlinked_eval_tree(&indirect, EvalKind::Indirect, true, &[], false, false)
                .unwrap_err()
                .to_string()
                .contains("received caller strictness")
        );

        let sloppy_direct = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
        assert!(
            verify_unlinked_eval_tree(&sloppy_direct, EvalKind::Direct, true, &[], false, false)
                .unwrap_err()
                .to_string()
                .contains("lost inherited caller strictness")
        );
    }

    #[test]
    fn eval_super_capabilities_are_authenticated_at_publication() {
        let malformed = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                super_call_allowed: true,
                super_allowed: false,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            verify_unlinked_tree(&malformed)
                .unwrap_err()
                .to_string()
                .contains("without SuperProperty")
        );

        let mut environment = ordinary_environment(None);
        environment.super_allowed = true;
        let caller = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![environment]);
        assert!(
            verify_unlinked_tree(&script_with_child(caller))
                .unwrap_err()
                .to_string()
                .contains("super capability disagrees with bytecode metadata")
        );

        let profile = EvalCallerProfile {
            scope_kinds: Box::new([]),
            variable_target: EvalCallerVariableTarget::Global,
        };
        let capability_root = |kind| {
            UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    eval_kind: kind,
                    super_allowed: true,
                    ..FunctionMetadata::default()
                },
            )
        };

        let direct = capability_root(EvalKind::Direct);
        verify_unlinked_eval_tree_with_profile(
            &direct,
            EvalKind::Direct,
            false,
            &[],
            &profile,
            false,
            true,
        )
        .unwrap();
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &direct,
                EvalKind::Direct,
                false,
                &[],
                &profile,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("eval root super capability disagrees with its caller")
        );

        let indirect = capability_root(EvalKind::Indirect);
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &indirect,
                EvalKind::Indirect,
                false,
                &[],
                &profile,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("eval root super capability disagrees with its caller")
        );
    }

    #[test]
    fn eval_local_sources_count_as_captures_for_close_local() {
        let environment = ordinary_environment(Some(EvalBinding {
            name: JsString::from_static("binding"),
            source: EvalBindingSource::Local(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        }));
        let function = lexical_local_function(environment, eval_code(0, true));

        verify_unlinked_tree(&script_with_child(function)).unwrap();
    }

    #[test]
    fn eval_instruction_rejects_an_out_of_bounds_environment() {
        let function = UnlinkedFunction::new(
            eval_code(1, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![ordinary_environment(None)]);

        let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("environment operand is out of bounds")
        );
    }

    #[test]
    fn unreferenced_eval_environment_cannot_manufacture_a_local_capture() {
        let environment = ordinary_environment(Some(EvalBinding {
            name: JsString::from_static("binding"),
            source: EvalBindingSource::Local(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        }));
        let function = lexical_local_function(
            environment,
            vec![
                Instruction::CloseLocal(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
        );

        let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
        assert!(error.to_string().contains("not referenced by bytecode"));
    }

    #[test]
    fn eval_environment_rejects_malformed_binding_metadata() {
        let cases = [
            EvalBinding {
                name: JsString::from_static("binding"),
                source: EvalBindingSource::Local(1),
                is_lexical: true,
                is_const: false,
                kind: ClosureVariableKind::Normal,
                is_catch_parameter: false,
            },
            EvalBinding {
                name: JsString::from_static("wrong"),
                source: EvalBindingSource::Local(0),
                is_lexical: true,
                is_const: false,
                kind: ClosureVariableKind::Normal,
                is_catch_parameter: false,
            },
            EvalBinding {
                name: JsString::from_static("binding"),
                source: EvalBindingSource::Local(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
                is_catch_parameter: false,
            },
        ];

        for binding in cases {
            let function =
                lexical_local_function(ordinary_environment(Some(binding)), eval_code(0, false));
            assert!(verify_unlinked_tree(&script_with_child(function)).is_err());
        }
    }

    #[test]
    fn eval_environment_authenticates_local_with_object_metadata() {
        let binding = EvalBinding {
            name: JsString::from_static("<with>"),
            source: EvalBindingSource::Local(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::WithObject,
            is_catch_parameter: false,
        };
        verify_unlinked_tree(&script_with_child(local_with_environment(
            binding.clone(),
            "<with>",
        )))
        .unwrap();

        assert!(
            verify_unlinked_tree(&script_with_child(local_with_environment(
                binding.clone(),
                "ordinary",
            )))
            .unwrap_err()
            .to_string()
            .contains("with-object local")
        );

        let mut lexical = binding.clone();
        lexical.is_lexical = true;
        assert!(
            verify_unlinked_tree(&script_with_child(local_with_environment(
                lexical, "<with>",
            )))
            .unwrap_err()
            .to_string()
            .contains("with-object binding metadata")
        );

        let mut argument = binding;
        argument.source = EvalBindingSource::Argument(0);
        assert!(
            verify_unlinked_tree(&script_with_child(local_with_environment(
                argument, "<with>",
            )))
            .unwrap_err()
            .to_string()
            .contains("with-object binding metadata")
        );
    }

    #[test]
    fn captured_with_object_uses_only_initialize_and_close_local_lifecycle_ops() {
        verify_unlinked_tree(&script_with_child(captured_with_object_function(false))).unwrap();

        for instruction in [
            Instruction::GetLocal(0),
            Instruction::PutLocal(0),
            Instruction::SetLocal(0),
        ] {
            let code = match instruction {
                Instruction::GetLocal(_) => vec![instruction, Instruction::Return],
                Instruction::PutLocal(_) => vec![
                    Instruction::Undefined,
                    instruction,
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                Instruction::SetLocal(_) => {
                    vec![Instruction::Undefined, instruction, Instruction::Return]
                }
                _ => unreachable!(),
            };
            let forged = UnlinkedFunction::new(
                code,
                Vec::new(),
                FunctionMetadata {
                    local_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            )
            .with_variable_definitions(Vec::new(), vec![UnlinkedVariableDefinition::with_object()]);
            assert!(
                verify_unlinked_tree(&script_with_child(forged))
                    .unwrap_err()
                    .to_string()
                    .contains("ordinary local opcode referenced a private with object")
            );
        }
    }

    #[test]
    fn strict_bytecode_cannot_publish_a_local_with_object() {
        assert!(
            verify_unlinked_tree(&script_with_child(captured_with_object_function(true)))
                .unwrap_err()
                .to_string()
                .contains("strict or malformed bytecode contains a with-object local")
        );
    }

    #[test]
    fn with_object_closure_descriptor_rejects_non_local_sources() {
        for (source, diagnostic) in [
            (ClosureSource::ParentArgument(0), "with-object descriptor"),
            (ClosureSource::Global, "non-global binding metadata"),
        ] {
            let child = UnlinkedFunction::new_with_closure_variables(
                vec![Instruction::Undefined, Instruction::Return],
                vec![
                    UnlinkedConstant::primitive(Value::String(JsString::from_static("<with>")))
                        .unwrap(),
                ],
                FunctionMetadata {
                    closure_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
                vec![ClosureVariable {
                    source,
                    name: ClosureVariableName::Constant(0),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::WithObject,
                }],
            );
            assert!(
                verify_unlinked_tree(&child)
                    .unwrap_err()
                    .to_string()
                    .contains(diagnostic)
            );
        }
    }

    #[test]
    fn eval_environment_authenticates_simple_and_pattern_catch_provenance() {
        let catch_binding = EvalBinding {
            name: JsString::from_static("binding"),
            source: EvalBindingSource::Local(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: true,
        };
        let environment = EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::Catch,
                    bindings: vec![catch_binding.clone()].into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Scope(2),
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        verify_unlinked_tree(&script_with_child(lexical_local_function(
            environment.clone(),
            eval_code(0, false),
        )))
        .unwrap();

        let mut pattern_environment = environment.clone();
        pattern_environment.scopes[0].bindings = vec![EvalBinding {
            is_catch_parameter: false,
            ..catch_binding.clone()
        }]
        .into_boxed_slice();
        verify_unlinked_tree(&script_with_child(lexical_local_function(
            pattern_environment,
            eval_code(0, false),
        )))
        .unwrap();

        let mut forged = environment;
        forged.scopes[0].bindings = Box::new([]);
        forged.scopes[1].bindings = vec![catch_binding].into_boxed_slice();
        assert!(
            verify_unlinked_tree(&script_with_child(lexical_local_function(
                forged,
                eval_code(0, false),
            )))
            .unwrap_err()
            .to_string()
            .contains("catch binding metadata")
        );
    }

    #[test]
    fn eval_environment_rejects_sources_from_the_wrong_function_segment() {
        let binding = EvalBinding {
            name: JsString::from_static("binding"),
            source: EvalBindingSource::Local(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        };
        let mut ancestor_local = ordinary_environment(None);
        ancestor_local.scopes[2].bindings = vec![binding.clone()].into_boxed_slice();
        let function = lexical_local_function(ancestor_local, eval_code(0, false));
        assert!(
            verify_unlinked_tree(&script_with_child(function))
                .unwrap_err()
                .to_string()
                .contains("function scope segment")
        );

        let mut current_closure = ordinary_environment(None);
        current_closure.scopes[0].bindings = vec![EvalBinding {
            source: EvalBindingSource::Closure(0),
            ..binding
        }]
        .into_boxed_slice();
        let function = lexical_local_function(current_closure, eval_code(0, false));
        assert!(
            verify_unlinked_tree(&script_with_child(function))
                .unwrap_err()
                .to_string()
                .contains("function scope segment")
        );
    }

    #[test]
    fn eval_environment_rejects_malformed_function_segments() {
        let mut environment = ordinary_environment(None);
        environment.scopes[2].kind = EvalScopeKind::FunctionBody;
        let function = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![environment]);
        assert!(
            verify_unlinked_tree(&script_with_child(function))
                .unwrap_err()
                .to_string()
                .contains("wrong body scope")
        );

        let mut missing_body = ordinary_environment(None);
        missing_body.scopes[0].kind = EvalScopeKind::FunctionRoot;
        let function = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![missing_body]);
        assert!(
            verify_unlinked_tree(&script_with_child(function))
                .unwrap_err()
                .to_string()
                .contains("wrong body scope")
        );
    }

    #[test]
    fn eval_environment_rejects_global_and_nameless_lexical_closure_sources() {
        let global_name = JsString::from_static("globalName");
        let global = UnlinkedFunction::new_with_closure_variables(
            eval_code(0, false),
            vec![UnlinkedConstant::primitive(Value::String(global_name.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::Global,
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
        .with_eval_environments(vec![EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: vec![EvalBinding {
                        name: global_name,
                        source: EvalBindingSource::Closure(0),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                        is_catch_parameter: false,
                    }]
                    .into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Global,
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        }]);
        assert!(
            verify_unlinked_tree(&global)
                .unwrap_err()
                .to_string()
                .contains("function scope segment")
        );

        let mut environment = ordinary_environment(None);
        environment.scopes[2].bindings = vec![EvalBinding {
            name: JsString::from_static("outerLexical"),
            source: EvalBindingSource::Closure(0),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        }]
        .into_boxed_slice();
        let child = UnlinkedFunction::new_with_closure_variables(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::None,
                is_lexical: true,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
        .with_eval_environments(vec![environment]);
        let parent = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::lexical(None, false)],
        );
        assert!(
            verify_unlinked_tree(&parent)
                .unwrap_err()
                .to_string()
                .contains("closure descriptor")
        );
    }

    #[test]
    fn eval_environment_authenticates_ordinary_closure_names_to_parent_definitions() {
        let spoofed = JsString::from_static("spoofed");
        let mut environment = ordinary_environment(None);
        environment.scopes[2].bindings = vec![EvalBinding {
            name: spoofed.clone(),
            source: EvalBindingSource::Closure(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        }]
        .into_boxed_slice();
        let child = UnlinkedFunction::new_with_closure_variables(
            eval_code(0, false),
            vec![UnlinkedConstant::primitive(Value::String(spoofed.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
        .with_eval_environments(vec![environment]);
        let parent = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("real"),
            ))],
        );
        assert!(
            verify_unlinked_tree(&parent)
                .unwrap_err()
                .to_string()
                .contains("parent local definition")
        );
    }

    #[test]
    fn function_name_metadata_erasure_requires_a_direct_eval_child() {
        let child = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        let named_parent = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                local_count: 1,
                function_name_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_name(Some(JsString::from_static("named")));
        let script = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(named_parent)],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );

        assert!(
            verify_unlinked_tree(&script)
                .unwrap_err()
                .to_string()
                .contains("parent local definition"),
            "a child without direct eval erased FunctionName metadata",
        );
    }

    #[test]
    fn erased_function_name_view_cannot_stick_into_a_plain_descendant() {
        let name = JsString::from_static("named");
        let leaf = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentClosure(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        let environment = EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: vec![EvalBinding {
                        name: name.clone(),
                        source: EvalBindingSource::Closure(0),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                        is_catch_parameter: false,
                    }]
                    .into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Scope(1),
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        let middle = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::Undefined,
                Instruction::Eval {
                    argument_count: 0,
                    environment: 0,
                },
                Instruction::Drop,
                Instruction::FClosure(1),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(name.clone())).unwrap(),
                UnlinkedConstant::child(leaf),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
        .with_eval_environments(vec![environment]);
        let named_parent = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(middle)],
            FunctionMetadata {
                local_count: 1,
                function_name_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_name(Some(name));
        let script = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(named_parent)],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );

        assert!(
            verify_unlinked_tree(&script)
                .unwrap_err()
                .to_string()
                .contains("not the first source request"),
            "a plain descendant retained an erased FunctionName view",
        );
    }

    #[test]
    fn later_eval_child_cannot_rewrite_an_earlier_function_name_request() {
        let name = JsString::from_static("named");
        let plain_child = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentClosure(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::FunctionName,
            }],
        );
        let eval_environment = EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: vec![EvalBinding {
                        name: name.clone(),
                        source: EvalBindingSource::Closure(0),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                        is_catch_parameter: false,
                    }]
                    .into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Scope(1),
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        let eval_child = UnlinkedFunction::new_with_closure_variables(
            eval_code(0, false),
            vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentClosure(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
        .with_eval_environments(vec![eval_environment]);
        let middle = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::FClosure(1),
                Instruction::Drop,
                Instruction::FClosure(2),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(name.clone())).unwrap(),
                UnlinkedConstant::child(plain_child),
                UnlinkedConstant::child(eval_child),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        let named_parent = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(middle)],
            FunctionMetadata {
                local_count: 1,
                function_name_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_name(Some(name));
        let script = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(named_parent)],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );

        assert!(
            verify_unlinked_tree(&script)
                .unwrap_err()
                .to_string()
                .contains("not the first source request"),
            "a later eval child authenticated an impossible parent Normal view",
        );
    }

    #[test]
    fn eval_environment_rejects_a_local_relay_of_a_global_parent_slot() {
        let name = JsString::from_static("globalName");
        let mut environment = ordinary_environment(None);
        environment.scopes[2].bindings = vec![EvalBinding {
            name: name.clone(),
            source: EvalBindingSource::Closure(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        }]
        .into_boxed_slice();
        let child = UnlinkedFunction::new_with_closure_variables(
            eval_code(0, false),
            vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentClosure(0),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
        .with_eval_environments(vec![environment]);
        let root = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::FClosure(1),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(name)).unwrap(),
                UnlinkedConstant::child(child),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::Global,
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        assert!(
            verify_unlinked_tree(&root)
                .unwrap_err()
                .to_string()
                .contains("global parent slot")
        );
    }

    #[test]
    fn eval_variable_scope_must_select_the_current_function_root() {
        let mut environment = ordinary_environment(None);
        environment.variable_environment = EvalVariableEnvironment::Scope(0);
        let function = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![environment]);

        let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
        assert!(error.to_string().contains("current function root scope"));
    }

    #[test]
    fn eval_variable_environment_matches_publication_tree_topology() {
        let root_with_function_scope = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![ordinary_environment(None)]);
        assert!(
            verify_unlinked_tree(&root_with_function_scope)
                .unwrap_err()
                .to_string()
                .contains("segment count")
        );

        let child_with_global = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Global,
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        }]);
        assert!(
            verify_unlinked_tree(&script_with_child(child_with_global))
                .unwrap_err()
                .to_string()
                .contains("segment count")
        );
    }

    #[test]
    fn nested_eval_environment_may_contain_ancestor_function_roots() {
        let environment = EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Scope(1),
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        let function = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![environment]);

        verify_unlinked_tree(&script_with_child(function)).unwrap();
    }
}
