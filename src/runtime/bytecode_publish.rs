//! Validation and iterative flattening for unlinked bytecode publication.

use super::*;
use std::collections::HashSet;

use crate::bytecode::{
    DynamicEnvironmentSource, EvalVariableSource, MAX_LOCAL_SLOTS, WithObjectSource, verify_parts,
};
use crate::heap::{
    EvalBinding, EvalCallerProfile, EvalCallerVariableTarget, EvalKind, EvalRootBinding, EvalScope,
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
                if binding.is_catch_parameter != is_catch_scope
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
#[cfg_attr(not(test), allow(dead_code))]
pub(in crate::runtime) fn verify_unlinked_eval_tree(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
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
    )
}

pub(in crate::runtime) fn verify_unlinked_eval_tree_with_profile(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_profile: &EvalCallerProfile,
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
        if binding.is_catch_parameter != (scope_kind == crate::heap::EvalScopeKind::Catch)
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
    )];
    while let Some((
        function,
        function_depth,
        closure_origins,
        function_name_origins,
        function_id,
    )) = pending.pop()
    {
        let is_root = function_depth == 0;
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
        if function.metadata().defined_argument_count > function.metadata().argument_count {
            return Err(RuntimeError::Engine(Error::internal(
                "defined argument count exceeds function argument slots",
            )));
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
        for child in function
            .constants()
            .iter()
            .filter_map(UnlinkedConstant::as_child)
        {
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
                .filter_map(UnlinkedConstant::as_child)
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
        for constant in function.constants() {
            if let Some(child) = constant.as_child() {
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
                ));
            } else if constant.as_primitive().is_none() && constant.as_regexp().is_none() {
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
        EvalVariableEnvironment,
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

        verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &expected).unwrap();
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
        verify_unlinked_eval_tree_with_profile(&root, EvalKind::Direct, false, &expected, &profile)
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
            verify_unlinked_eval_tree(&script_root, EvalKind::Direct, false, &[])
                .unwrap_err()
                .to_string()
                .contains("publication entry point")
        );
        assert!(verify_unlinked_eval_tree(&script_root, EvalKind::None, false, &[]).is_err());
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
            verify_unlinked_eval_tree(&wrong_name, EvalKind::Direct, false, &expected[..1])
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
            verify_unlinked_eval_tree(&wrong_flags, EvalKind::Direct, false, &expected[..1])
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
            verify_unlinked_eval_tree(&wrong_kind, EvalKind::Direct, false, &expected[..1])
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
            verify_unlinked_eval_tree(&wrong_order, EvalKind::Direct, false, &expected)
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
            verify_unlinked_eval_tree(&missing, EvalKind::Direct, false, &expected)
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
        verify_unlinked_eval_tree(&direct_global, EvalKind::Direct, false, &[]).unwrap();

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
        verify_unlinked_eval_tree(&indirect_global, EvalKind::Indirect, false, &[]).unwrap();

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
                verify_unlinked_eval_tree(&strict, kind, caller_strict, &[])
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
            verify_unlinked_eval_tree(&lexical, EvalKind::Direct, false, &[])
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
        verify_unlinked_eval_tree(&imported, EvalKind::Direct, false, &expected).unwrap();

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
            verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &expected)
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
            verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &expected)
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
            verify_unlinked_eval_tree(&global_special, EvalKind::Direct, false, &[])
                .unwrap_err()
                .to_string()
                .contains("non-global binding metadata")
        );

        let root = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
        let mut forged_catch =
            eval_root_binding("caught", 0, false, false, ClosureVariableKind::Normal);
        forged_catch.is_catch_parameter = true;
        assert!(
            verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &[forged_catch])
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
            verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &[])
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
            verify_unlinked_eval_tree(&root, EvalKind::Direct, false, &[])
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
        verify_unlinked_eval_tree(&indirect, EvalKind::Indirect, false, &[]).unwrap();
        let caller_binding =
            eval_root_binding("caller", 0, false, false, ClosureVariableKind::Normal);
        assert!(
            verify_unlinked_eval_tree(&indirect, EvalKind::Indirect, false, &[caller_binding])
                .unwrap_err()
                .to_string()
                .contains("received caller bindings")
        );
        assert!(
            verify_unlinked_eval_tree(&indirect, EvalKind::Indirect, true, &[])
                .unwrap_err()
                .to_string()
                .contains("received caller strictness")
        );

        let sloppy_direct = eval_root_with_descriptors(EvalKind::Direct, Vec::new());
        assert!(
            verify_unlinked_eval_tree(&sloppy_direct, EvalKind::Direct, true, &[])
                .unwrap_err()
                .to_string()
                .contains("lost inherited caller strictness")
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
    fn eval_environment_authenticates_catch_parameter_provenance() {
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
        };
        verify_unlinked_tree(&script_with_child(lexical_local_function(
            environment.clone(),
            eval_code(0, false),
        )))
        .unwrap();

        let mut forged = environment;
        forged.scopes[0].bindings = vec![EvalBinding {
            is_catch_parameter: false,
            ..catch_binding
        }]
        .into_boxed_slice();
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
