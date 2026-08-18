//! Validation and iterative flattening for unlinked bytecode publication.

mod module_initializer_flow;
mod private_elements;

pub(super) use private_elements::prepare_private_binding_publication;

use super::*;
use std::collections::HashSet;

use crate::bytecode::{
    DynamicEnvironmentSource, EvalVariableSource, Instruction, MAX_LOCAL_SLOTS, WithObjectSource,
    verify_parts,
};
use crate::heap::{
    EvalBinding, EvalCallerProfile, EvalCallerVariableTarget, EvalEnvironmentPhaseContext,
    EvalKind, EvalRootBinding, EvalScope, parameter_initializer_visible_locals,
    validate_class_initializer_bytecode_layout, validate_derived_constructor_bytecode_layout,
    validate_eval_environment_phase_layout, validate_parameter_bytecode_layout,
    validate_parameter_initializer_scope_layout, validate_pattern_parameter_bytecode_layout,
};
use crate::module::{
    ModuleExportTarget, ModuleImportCollisionDeclaration, ModuleImportName,
    ModuleLinkInitializerValue, UnlinkedModule,
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

const fn eval_variable_object_sentinel(kind: ClosureVariableKind) -> Option<&'static str> {
    match kind {
        ClosureVariableKind::EvalVariableObject => Some("<var>"),
        ClosureVariableKind::ArgEvalVariableObject => Some("<arg_var>"),
        ClosureVariableKind::Normal
        | ClosureVariableKind::ModuleImportView
        | ClosureVariableKind::FunctionName
        | ClosureVariableKind::GlobalFunction
        | ClosureVariableKind::WithObject
        | ClosureVariableKind::PrivateField
        | ClosureVariableKind::PrivateMethod
        | ClosureVariableKind::PrivateGetter
        | ClosureVariableKind::PrivateSetter
        | ClosureVariableKind::PrivateGetterSetter => None,
    }
}

/// Canonical compiler-authored pseudo-binding entry order. The unlinked
/// publisher repeats the heap boundary's structural check while additionally
/// authenticating the source-only sentinel names.
const fn pseudo_binding_entry(
    instruction: &crate::bytecode::Instruction,
) -> Option<(u8, &'static str)> {
    match instruction {
        crate::bytecode::Instruction::PushHomeObject => Some((1, "<home_object>")),
        crate::bytecode::Instruction::PushActiveFunction => Some((2, "<this_active_func>")),
        crate::bytecode::Instruction::PushNewTarget => Some((3, "<new.target>")),
        crate::bytecode::Instruction::PushThis => Some((4, "<this>")),
        _ => None,
    }
}

fn eval_root_binding_is_derived_this(binding: &EvalRootBinding<JsString>) -> bool {
    binding.is_lexical
        && !binding.is_const
        && binding.kind == ClosureVariableKind::Normal
        && !binding.is_catch_parameter
        && binding.name.utf16_units().eq("<this>".encode_utf16())
}

fn eval_root_binding_is_super_pseudo(
    binding: &EvalRootBinding<JsString>,
    expected_name: &'static str,
) -> bool {
    !binding.is_lexical
        && !binding.is_const
        && binding.kind == ClosureVariableKind::Normal
        && !binding.is_catch_parameter
        && binding.name.utf16_units().eq(expected_name.encode_utf16())
}

fn class_initializer_bridge_kind(
    instruction: &crate::bytecode::Instruction,
) -> Option<ClassInitializerKind> {
    match instruction {
        crate::bytecode::Instruction::InstallClassInstanceInitializer => {
            Some(ClassInitializerKind::InstanceFields)
        }
        crate::bytecode::Instruction::RunClassStaticInitializer => {
            Some(ClassInitializerKind::StaticElements)
        }
        crate::bytecode::Instruction::CallClassStaticBlock => {
            Some(ClassInitializerKind::StaticBlock)
        }
        _ => None,
    }
}

fn explicit_control_flow_target(instruction: &crate::bytecode::Instruction) -> Option<usize> {
    let target = match instruction {
        crate::bytecode::Instruction::Goto(target)
        | crate::bytecode::Instruction::IfFalse(target)
        | crate::bytecode::Instruction::IfTrue(target)
        | crate::bytecode::Instruction::Catch(target)
        | crate::bytecode::Instruction::Gosub(target) => *target,
        _ => return None,
    };
    usize::try_from(target).ok()
}

fn validate_class_initializer_publication_edges(
    function: &UnlinkedFunction,
    child_closure_pcs: &[Vec<usize>],
    explicit_control_flow_targets: &HashSet<usize>,
) -> Result<(), RuntimeError> {
    // Every privileged bridge must consume the closure created by the
    // immediately preceding FClosure. Stack verification alone cannot prove
    // that the operand is the compiler-authored hidden child rather than an
    // unrelated callable supplied by forged bytecode.
    for (bridge_pc, instruction) in function.code().iter().enumerate() {
        let Some(expected_kind) = class_initializer_bridge_kind(instruction) else {
            continue;
        };
        // Install/Run may occur in any authored or initializer function because
        // a nested class expression is legal in all of those contexts. Their
        // authority therefore comes from the adjacent typed child. A static
        // block call is different: it is emitted only inside its aggregate.
        if expected_kind == ClassInitializerKind::StaticBlock
            && function.metadata().class_initializer_kind
                != Some(ClassInitializerKind::StaticElements)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "class static block call escaped its static-elements parent",
            )));
        }
        let closure_pc = bridge_pc.checked_sub(1).ok_or_else(|| {
            RuntimeError::Engine(Error::internal(
                "class initializer bridge did not consume an adjacent child closure",
            ))
        })?;
        if explicit_control_flow_targets.contains(&closure_pc)
            || explicit_control_flow_targets.contains(&bridge_pc)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "class initializer closure/bridge pair has a non-fallthrough entry",
            )));
        }
        if expected_kind == ClassInitializerKind::StaticBlock
            && function
                .code()
                .iter()
                .enumerate()
                .any(|(source_pc, instruction)| {
                    source_pc > bridge_pc
                        && explicit_control_flow_target(instruction)
                            .is_some_and(|target_pc| target_pc <= closure_pc)
                })
        {
            return Err(RuntimeError::Engine(Error::internal(
                "class static block closure/bridge pair is reentrant",
            )));
        }
        let Some(crate::bytecode::Instruction::FClosure(constant)) =
            function.code().get(closure_pc)
        else {
            return Err(RuntimeError::Engine(Error::internal(
                "class initializer bridge did not consume an adjacent child closure",
            )));
        };
        let child = usize::try_from(*constant)
            .ok()
            .and_then(|constant| function.constants().get(constant))
            .and_then(UnlinkedConstant::as_child)
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "class initializer bridge did not reference child bytecode",
                ))
            })?;
        if child.metadata().class_initializer_kind != Some(expected_kind) {
            return Err(RuntimeError::Engine(Error::internal(
                "class initializer bridge consumed a child with the wrong role",
            )));
        }
    }

    // Conversely, a hidden initializer child is valid only at one creation
    // site and that site must immediately enter its matching bridge. This
    // prevents FClosure/Return, FClosure/Drop, and repeated FClosure sites from
    // exposing or reusing the internal callable.
    for (constant_index, constant) in function.constants().iter().enumerate() {
        let Some(child) = constant.as_child() else {
            continue;
        };
        let Some(expected_kind) = child.metadata().class_initializer_kind else {
            continue;
        };
        let [closure_pc] = child_closure_pcs[constant_index].as_slice() else {
            return Err(RuntimeError::Engine(Error::internal(
                "class initializer child did not have one unique closure site",
            )));
        };
        if expected_kind == ClassInitializerKind::StaticBlock
            && function.metadata().class_initializer_kind
                != Some(ClassInitializerKind::StaticElements)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "class static block child escaped its static-elements parent",
            )));
        }
        let bridge = closure_pc
            .checked_add(1)
            .and_then(|bridge_pc| function.code().get(bridge_pc));
        if bridge.and_then(class_initializer_bridge_kind) != Some(expected_kind) {
            return Err(RuntimeError::Engine(Error::internal(
                "class initializer child escaped its matching bridge",
            )));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum SuperPseudoRole {
    DerivedThis,
    ActiveFunction,
    NewTarget,
}

fn verify_eval_super_pseudo_bindings(
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
            (SuperPseudoRole::DerivedThis, crate::heap::EvalBindingSource::Local(index)) => {
                derived_this_local == Some(index)
            }
            (SuperPseudoRole::ActiveFunction, crate::heap::EvalBindingSource::Local(index)) => {
                active_function_local == Some(index)
            }
            (SuperPseudoRole::NewTarget, crate::heap::EvalBindingSource::Local(index)) => {
                new_target_local == Some(index)
            }
            (SuperPseudoRole::DerivedThis, crate::heap::EvalBindingSource::Closure(index)) => {
                derived_this_origins
                    .get(usize::from(index))
                    .copied()
                    .unwrap_or(false)
            }
            (SuperPseudoRole::ActiveFunction, crate::heap::EvalBindingSource::Closure(index)) => {
                active_function_origins
                    .get(usize::from(index))
                    .copied()
                    .unwrap_or(false)
            }
            (SuperPseudoRole::NewTarget, crate::heap::EvalBindingSource::Closure(index)) => {
                new_target_origins
                    .get(usize::from(index))
                    .copied()
                    .unwrap_or(false)
            }
            (_, crate::heap::EvalBindingSource::Argument(_)) => false,
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

fn eval_variable_object_local_kind(
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

fn verify_eval_variable_source(
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
                    crate::heap::EvalScopeKind::FunctionRoot
                        | crate::heap::EvalScopeKind::Parameter
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
            crate::heap::EvalScopeKind::FunctionRoot => {
                let expected_body = if final_segment || synthetic_root_segment {
                    crate::heap::EvalScopeKind::ProgramBody
                } else {
                    crate::heap::EvalScopeKind::FunctionBody
                };
                if function_anchor == segment_start
                    || (!imported_segment
                        && environment.scopes[function_anchor - 1].kind != expected_body)
                    || (imported_segment
                        && !matches!(
                            environment.scopes[function_anchor - 1].kind,
                            crate::heap::EvalScopeKind::FunctionBody
                                | crate::heap::EvalScopeKind::ProgramBody
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
            crate::heap::EvalScopeKind::Parameter => {
                if synthetic_root_segment
                    || environment.scopes[segment_start..function_anchor]
                        .iter()
                        .any(|scope| {
                            matches!(
                                scope.kind,
                                crate::heap::EvalScopeKind::FunctionBody
                                    | crate::heap::EvalScopeKind::ProgramBody
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
            == crate::heap::EvalScopeKind::FunctionRoot
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
                    crate::heap::EvalScopeKind::FunctionRoot
                        | crate::heap::EvalScopeKind::Parameter
                        | crate::heap::EvalScopeKind::FunctionBody
                        | crate::heap::EvalScopeKind::ProgramBody
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
                    crate::heap::EvalBindingSource::Local(_)
                        | crate::heap::EvalBindingSource::Closure(_)
                )
            } else if segment_count == 0 {
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
            .filter(|kind| {
                matches!(
                    **kind,
                    crate::heap::EvalScopeKind::FunctionRoot
                        | crate::heap::EvalScopeKind::Parameter
                )
            })
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
            crate::heap::EvalVariableEnvironment::Global => {
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
            crate::heap::EvalVariableEnvironment::StrictLocal(index) => {
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
                    crate::heap::EvalScopeKind::FunctionRoot
                        | crate::heap::EvalScopeKind::Parameter
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "strict eval variable environment selected a non-function scope",
                    )));
                }
            }
            crate::heap::EvalVariableEnvironment::VariableObject { scope, source } => {
                if environment.caller_strict {
                    return Err(RuntimeError::Engine(Error::internal(
                        "strict eval environment selected a variable object",
                    )));
                }
                let target_matches_function_segment = if synthetic_eval_root {
                    function.metadata().eval_kind == EvalKind::Direct
                        && usize::from(scope) >= imported_scope_start
                        && matches!(source, crate::heap::EvalBindingSource::Closure(_))
                } else {
                    scope == first_function_anchor
                        && matches!(source, crate::heap::EvalBindingSource::Local(_))
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
                    crate::heap::EvalScopeKind::FunctionRoot => {
                        ClosureVariableKind::EvalVariableObject
                    }
                    crate::heap::EvalScopeKind::Parameter => {
                        ClosureVariableKind::ArgEvalVariableObject
                    }
                    _ => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "eval variable object selected a non-function scope",
                        )));
                    }
                };
                if matches!(source, crate::heap::EvalBindingSource::Argument(_))
                    || target_scope
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
                    crate::heap::EvalBindingSource::Local(index)
                        if eval_variable_object_local_kind(function, index)
                            == Some(expected_kind) => {}
                    crate::heap::EvalBindingSource::Closure(index) => {
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
                    crate::heap::EvalBindingSource::Local(_)
                    | crate::heap::EvalBindingSource::Argument(_) => {
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
                    crate::heap::EvalVariableEnvironment::StrictLocal(actual)
                        if actual == first_function_anchor
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
                        crate::heap::EvalVariableEnvironment::VariableObject {
                            source: crate::heap::EvalBindingSource::Closure(actual),
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
                                crate::heap::EvalBindingSource::Argument(_)
                            )))
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval with-object binding metadata disagrees with its scope",
                    )));
                }
                if binding.kind.is_eval_variable_object() {
                    let role_allowed = match scope.kind {
                        crate::heap::EvalScopeKind::FunctionRoot => true,
                        crate::heap::EvalScopeKind::Parameter => {
                            binding.kind == ClosureVariableKind::ArgEvalVariableObject
                        }
                        _ => false,
                    };
                    if !role_allowed
                        || binding.is_lexical
                        || binding.is_const
                        || binding.is_catch_parameter
                        || matches!(binding.source, crate::heap::EvalBindingSource::Argument(_))
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
    TrustedOrdinaryLeaf,
    Module(&'a UnlinkedModule),
    Eval {
        kind: EvalKind,
        caller_strict: bool,
        expected_bindings: &'a [EvalRootBinding<JsString>],
        expected_profile: &'a EvalCallerProfile,
        expected_capabilities: EvalPublicationCapabilities,
    },
}

#[derive(Clone, Copy)]
pub(in crate::runtime) struct EvalPublicationCapabilities {
    pub super_call_allowed: bool,
    pub super_allowed: bool,
    pub arguments_forbidden: bool,
}

pub(super) fn verify_unlinked_tree(function: &UnlinkedFunction) -> Result<(), RuntimeError> {
    verify_unlinked_tree_with_root(function, RootPublication::Script)
}

/// Authenticate one detached ordinary callable translated from trusted BC5.
///
/// Unlike a Script root, this role may own arguments and ordinary locals. It
/// remains a standalone leaf: no module, eval, class, super, HomeObject, or
/// closure authority may be smuggled through the specialized publication
/// entry point.
pub(in crate::runtime) fn verify_unlinked_ordinary_leaf(
    function: &UnlinkedFunction,
) -> Result<(), RuntimeError> {
    verify_unlinked_tree_with_root(function, RootPublication::TrustedOrdinaryLeaf)
}

/// Authenticate a compiler-owned module record and the distinct bytecode ABI
/// used to link and evaluate it. Ordinary script publication never accepts
/// this root shape.
pub(in crate::runtime) fn verify_unlinked_module_tree(
    module: &UnlinkedModule,
) -> Result<(), RuntimeError> {
    verify_unlinked_module_tables(module)?;
    verify_module_link_entry(module)?;
    verify_unlinked_tree_with_root(module.function(), RootPublication::Module(module))
}

fn verify_unlinked_module_tables(module: &UnlinkedModule) -> Result<(), RuntimeError> {
    let function = module.function();
    let descriptors = function.closure_variables();
    let request_count = module.requested_modules().len();
    let request_exists = |index: crate::module::ModuleRequestIndex| {
        usize::try_from(index.0)
            .ok()
            .is_some_and(|index| index < request_count)
    };

    let mut imported_slots = HashSet::new();
    let mut namespace_slots = HashSet::new();
    let mut import_meta_slot = None;
    for (index, descriptor) in descriptors.iter().enumerate() {
        if descriptor.source != ClosureSource::ModuleImportMeta {
            continue;
        }
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "import.meta descriptor index exceeds bytecode range",
            ))
        })?;
        if import_meta_slot.replace(index).is_some() {
            return Err(RuntimeError::Engine(Error::internal(
                "module contains more than one import.meta binding",
            )));
        }
        if !descriptor.is_lexical
            || !descriptor.is_const
            || descriptor.kind != ClosureVariableKind::Normal
            || unlinked_closure_name(function, descriptor)?
                != Some(&JsString::from_static(
                    crate::module::MODULE_IMPORT_META_BINDING_NAME,
                ))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "import.meta descriptor has invalid binding metadata",
            )));
        }
    }
    for import in module.imports() {
        if !request_exists(import.request) {
            return Err(RuntimeError::Engine(Error::internal(
                "module import referenced a missing request",
            )));
        }
        if !imported_slots.insert(import.closure_index) {
            return Err(RuntimeError::Engine(Error::internal(
                "module import table reused a closure slot",
            )));
        }
        let is_namespace = matches!(&import.import_name, ModuleImportName::Namespace);
        if is_namespace {
            namespace_slots.insert(import.closure_index);
        }
        let is_collision = module
            .import_collisions()
            .iter()
            .any(|collision| collision.closure_index == import.closure_index);
        let descriptor = descriptors
            .get(usize::from(import.closure_index))
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "module import closure index is out of bounds",
                ))
            })?;
        let expected_source = if is_collision {
            ClosureSource::ModuleImportCollision
        } else if is_namespace {
            ClosureSource::ModuleDeclaration
        } else {
            ClosureSource::ModuleImport
        };
        let expected_kind = if is_namespace {
            ClosureVariableKind::Normal
        } else {
            ClosureVariableKind::ModuleImportView
        };
        if descriptor.source != expected_source
            || !descriptor.is_lexical
            || !descriptor.is_const
            || descriptor.kind != expected_kind
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module import table disagrees with its closure descriptor",
            )));
        }
    }

    for (index, descriptor) in descriptors.iter().enumerate() {
        if !matches!(
            descriptor.source,
            ClosureSource::ModuleImport | ClosureSource::ModuleImportCollision
        ) {
            continue;
        }
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "module import descriptor index exceeds bytecode range",
            ))
        })?;
        if !imported_slots.contains(&index) {
            return Err(RuntimeError::Engine(Error::internal(
                "module import descriptor has no import-table entry",
            )));
        }
        if descriptor.source == ClosureSource::ModuleImportCollision
            && !module
                .import_collisions()
                .iter()
                .any(|collision| collision.closure_index == index)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module import collision descriptor has no collision ledger entry",
            )));
        }
    }

    let mut collision_declarations = vec![None; descriptors.len()];
    for collision in module.import_collisions() {
        if !imported_slots.contains(&collision.closure_index) {
            return Err(RuntimeError::Engine(Error::internal(
                "module import collision has no import-table entry",
            )));
        }
        let declaration = collision_declarations
            .get_mut(usize::from(collision.closure_index))
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "module import collision closure index is out of bounds",
                ))
            })?;
        if declaration.replace(collision.declaration).is_some() {
            return Err(RuntimeError::Engine(Error::internal(
                "module import collision ledger reused a closure slot",
            )));
        }
    }

    let expected_declaration_slots = descriptors
        .iter()
        .enumerate()
        .filter_map(|(index, descriptor)| {
            let index = u16::try_from(index).ok()?;
            (descriptor.source == ClosureSource::ModuleImportCollision
                || descriptor.source == ClosureSource::ModuleDeclaration
                    && (!descriptor.is_lexical || !namespace_slots.contains(&index)))
            .then_some(index)
        })
        .collect::<HashSet<_>>();
    let mut seen_declarations = HashSet::with_capacity(module.declaration_order().len());
    for closure_index in module.declaration_order() {
        if !expected_declaration_slots.contains(closure_index)
            || !seen_declarations.insert(*closure_index)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module declaration-order ledger disagrees with descriptors",
            )));
        }
    }
    if seen_declarations != expected_declaration_slots {
        return Err(RuntimeError::Engine(Error::internal(
            "module declaration-order ledger omitted a declaration",
        )));
    }
    let ordered_collision_slots = module.declaration_order().iter().copied().filter(|index| {
        descriptors
            .get(usize::from(*index))
            .is_some_and(|descriptor| descriptor.source == ClosureSource::ModuleImportCollision)
    });
    if !module
        .import_collisions()
        .iter()
        .map(|collision| collision.closure_index)
        .eq(ordered_collision_slots)
    {
        return Err(RuntimeError::Engine(Error::internal(
            "module import collision ledger is not in declaration order",
        )));
    }

    let mut lexical_initializer_counts = vec![0_u8; descriptors.len()];
    let mut collision_initializer_counts = vec![0_u8; descriptors.len()];
    for instruction in function.code() {
        let (index, counts, message) = match instruction {
            crate::bytecode::Instruction::InitializeVarRef(index) => (
                index,
                &mut lexical_initializer_counts,
                "module lexical initializer count overflowed",
            ),
            crate::bytecode::Instruction::InitializeModuleImportCollision(index) => (
                index,
                &mut collision_initializer_counts,
                "module import collision initializer count overflowed",
            ),
            _ => continue,
        };
        let count = counts.get_mut(usize::from(*index)).ok_or_else(|| {
            RuntimeError::Engine(Error::internal(
                "module initializer is outside closure slots",
            ))
        })?;
        *count = count
            .checked_add(1)
            .ok_or_else(|| RuntimeError::Engine(Error::internal(message)))?;
    }
    for (index, (descriptor, count)) in descriptors
        .iter()
        .zip(lexical_initializer_counts)
        .enumerate()
    {
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "module declaration descriptor index exceeds bytecode range",
            ))
        })?;
        let expected = u8::from(
            descriptor.source == ClosureSource::ModuleDeclaration
                && descriptor.is_lexical
                && !namespace_slots.contains(&index),
        );
        if count != expected {
            return Err(RuntimeError::Engine(Error::internal(
                "module lexical initializer count disagrees with declarations",
            )));
        }
    }
    for (declaration, count) in collision_declarations
        .iter()
        .copied()
        .zip(collision_initializer_counts)
    {
        let expected = match declaration {
            Some(
                ModuleImportCollisionDeclaration::Lexical
                | ModuleImportCollisionDeclaration::Function,
            ) => 1,
            Some(ModuleImportCollisionDeclaration::Var) | None => 0,
        };
        if count != expected {
            return Err(RuntimeError::Engine(Error::internal(
                "module import collision initializer count disagrees with its ledger",
            )));
        }
    }

    let mut exported_names = Vec::with_capacity(module.exports().len());
    for export in module.exports() {
        if exported_names
            .iter()
            .any(|name| name == &export.export_name)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module export table contains a duplicate exported name",
            )));
        }
        exported_names.push(export.export_name.clone());
        match &export.target {
            ModuleExportTarget::Local { closure_index } => {
                let descriptor = descriptors
                    .get(usize::from(*closure_index))
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "local module export closure index is out of bounds",
                        ))
                    })?;
                if !matches!(
                    descriptor.source,
                    ClosureSource::ModuleDeclaration
                        | ClosureSource::ModuleImport
                        | ClosureSource::ModuleImportCollision
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "local module export referenced a non-module binding",
                    )));
                }
            }
            ModuleExportTarget::Indirect {
                request,
                import_name: _,
            } => {
                if !request_exists(*request) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "indirect module export referenced a missing request",
                    )));
                }
            }
        }
    }
    for export in module.star_exports() {
        if !request_exists(export.request) {
            return Err(RuntimeError::Engine(Error::internal(
                "star module export referenced a missing request",
            )));
        }
    }
    Ok(())
}

fn verify_module_link_entry(module: &UnlinkedModule) -> Result<(), RuntimeError> {
    let function = module.function();
    let code = function.code();
    if !matches!(
        code.get(code.len().saturating_sub(2)..),
        Some([
            crate::bytecode::Instruction::Undefined,
            crate::bytecode::Instruction::Return
        ])
    ) {
        return Err(RuntimeError::Engine(Error::internal(
            "module body has no canonical undefined return",
        )));
    }
    let Some(
        [
            crate::bytecode::Instruction::PushThis,
            crate::bytecode::Instruction::IfFalse(body),
        ],
    ) = code.get(0..2)
    else {
        return Err(RuntimeError::Engine(Error::internal(
            "module bytecode has no link-entry guard",
        )));
    };
    let body = usize::try_from(*body).map_err(|_| {
        RuntimeError::Engine(Error::internal(
            "module link-entry target is outside bytecode",
        ))
    })?;
    if body < 4 || body >= code.len() {
        return Err(RuntimeError::Engine(Error::internal(
            "module link-entry target is outside bytecode",
        )));
    }
    if !matches!(
        code.get(body - 2..body),
        Some([
            crate::bytecode::Instruction::Undefined,
            crate::bytecode::Instruction::Return
        ])
    ) {
        return Err(RuntimeError::Engine(Error::internal(
            "module link entry has no undefined return",
        )));
    }
    for (pc, instruction) in code.iter().enumerate() {
        let Some(target) = explicit_control_flow_target(instruction) else {
            continue;
        };
        if pc >= body && target < body {
            return Err(RuntimeError::Engine(Error::internal(
                "module evaluation control flow entered the link phase",
            )));
        }
        if pc < body
            && (pc != 1
                || !matches!(instruction, crate::bytecode::Instruction::IfFalse(_))
                || target != body)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module link phase contains non-canonical control flow",
            )));
        }
    }
    // Establish one stack-valid unwind/Gosub shape per PC before the
    // module-specific summary analysis projects and follows that shape.
    verify_parts(
        function.code(),
        function.constants().len(),
        function.metadata().max_stack,
    )?;
    module_initializer_flow::verify(module, body)?;

    let function_collision_slots = module
        .import_collisions()
        .iter()
        .filter_map(|collision| {
            (collision.declaration == ModuleImportCollisionDeclaration::Function)
                .then_some(collision.closure_index)
        })
        .collect::<HashSet<_>>();
    let expected_slots = function.closure_variables();
    let expected_slots = module
        .declaration_order()
        .iter()
        .copied()
        .filter(|index| {
            let descriptor = expected_slots.get(usize::from(*index));
            let Some(descriptor) = descriptor else {
                return false;
            };
            descriptor.source == ClosureSource::ModuleDeclaration && !descriptor.is_lexical
                || function_collision_slots.contains(index)
        })
        .collect::<Vec<_>>();
    let initializers = module.link_initializers();
    if initializers.len() != expected_slots.len() {
        return Err(RuntimeError::Engine(Error::internal(
            "module link initializer ledger disagrees with declarations",
        )));
    }
    let mut seen_slots = HashSet::with_capacity(initializers.len());
    for initializer in initializers {
        if !seen_slots.insert(initializer.closure_index) {
            return Err(RuntimeError::Engine(Error::internal(
                "module link initializer ledger reused a closure slot",
            )));
        }
        let descriptor = function
            .closure_variables()
            .get(usize::from(initializer.closure_index))
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "module link initializer closure index is out of bounds",
                ))
            })?;
        let collision = function_collision_slots.contains(&initializer.closure_index);
        let ordinary_declaration = descriptor.source == ClosureSource::ModuleDeclaration
            && !descriptor.is_lexical
            && !descriptor.is_const
            && descriptor.kind == ClosureVariableKind::Normal;
        if !collision && !ordinary_declaration {
            return Err(RuntimeError::Engine(Error::internal(
                "module link initializer disagrees with its closure descriptor",
            )));
        }
    }
    if !initializers
        .iter()
        .map(|initializer| initializer.closure_index)
        .eq(expected_slots.iter().copied())
    {
        return Err(RuntimeError::Engine(Error::internal(
            "module link initializer ledger is not in declaration order",
        )));
    }

    let initializer_code = &code[2..body - 2];
    let mut cursor = 0;
    for initializer in initializers {
        let value_is_exact = match initializer.value {
            ModuleLinkInitializerValue::Undefined => {
                let matches = matches!(
                    initializer_code.get(cursor),
                    Some(crate::bytecode::Instruction::Undefined)
                ) && !function_collision_slots.contains(&initializer.closure_index);
                cursor = cursor.saturating_add(1);
                matches
            }
            ModuleLinkInitializerValue::Function {
                constant,
                inferred_name,
            } => {
                let child = usize::try_from(constant)
                    .ok()
                    .and_then(|index| function.constants().get(index))
                    .and_then(UnlinkedConstant::as_child)
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "module function initializer referenced non-function bytecode",
                        ))
                    })?;
                let descriptor = function
                    .closure_variables()
                    .get(usize::from(initializer.closure_index))
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "module function initializer descriptor is out of bounds",
                        ))
                    })?;
                let descriptor_name =
                    unlinked_closure_name(function, descriptor)?.ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "module function initializer descriptor has no name",
                        ))
                    })?;
                let declaration_child_is_exact =
                    child.metadata().strict && child.metadata().function_name_local.is_none();
                let binding_is_exact = declaration_child_is_exact
                    && if inferred_name.is_some() {
                        let mut local_exports = module.exports().iter().filter(|export| {
                            matches!(
                                &export.target,
                                ModuleExportTarget::Local { closure_index }
                                    if *closure_index == initializer.closure_index
                            )
                        });
                        local_exports.next().is_some_and(|export| {
                            export.export_name == JsString::from_static("default")
                        }) && local_exports.next().is_none()
                            && descriptor_name
                                == &JsString::from_static(
                                    crate::module::MODULE_DEFAULT_BINDING_NAME,
                                )
                            && child.func_name().is_none()
                    } else {
                        descriptor_name
                            != &JsString::from_static(crate::module::MODULE_DEFAULT_BINDING_NAME)
                            && child.func_name() == Some(descriptor_name)
                    };
                let matches = matches!(
                    initializer_code.get(cursor),
                    Some(crate::bytecode::Instruction::FClosure(actual))
                        if *actual == constant
                );
                cursor = cursor.saturating_add(1);
                if let Some(name) = inferred_name {
                    let name_is_default = usize::try_from(name)
                        .ok()
                        .and_then(|index| function.constants().get(index))
                        .and_then(|constant| constant.as_primitive())
                        .is_some_and(|value| {
                            value == &Value::String(JsString::from_static("default"))
                        });
                    let set_name_is_exact = matches!(
                        initializer_code.get(cursor),
                        Some(crate::bytecode::Instruction::SetName(actual)) if *actual == name
                    );
                    cursor = cursor.saturating_add(1);
                    matches && binding_is_exact && name_is_default && set_name_is_exact
                } else {
                    matches && binding_is_exact
                }
            }
        };
        let target_is_exact = if function_collision_slots.contains(&initializer.closure_index) {
            matches!(
                initializer_code.get(cursor),
                Some(crate::bytecode::Instruction::InitializeModuleImportCollision(slot))
                    if *slot == initializer.closure_index
            )
        } else {
            matches!(
                initializer_code.get(cursor),
                Some(crate::bytecode::Instruction::PutVarRef(slot))
                    if *slot == initializer.closure_index
            )
        };
        cursor = cursor.saturating_add(1);
        if !value_is_exact || !target_is_exact {
            return Err(RuntimeError::Engine(Error::internal(
                "module link entry initializer is not canonical",
            )));
        }
    }
    if cursor != initializer_code.len() {
        return Err(RuntimeError::Engine(Error::internal(
            "module link entry initializer count disagrees with declarations",
        )));
    }
    Ok(())
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
    for (scope, scope_kind) in scope_kinds.iter_mut().enumerate() {
        let has_parameter_object = expected_bindings.iter().any(|binding| {
            usize::from(binding.scope) == scope
                && binding.kind == ClosureVariableKind::ArgEvalVariableObject
        });
        let has_body_object = expected_bindings.iter().any(|binding| {
            usize::from(binding.scope) == scope
                && binding.kind == ClosureVariableKind::EvalVariableObject
        });
        if has_parameter_object && !has_body_object {
            *scope_kind = crate::heap::EvalScopeKind::Parameter;
        }
    }
    let variable_target = if caller_strict {
        EvalCallerVariableTarget::StrictLocal
    } else {
        expected_bindings
            .iter()
            .position(|binding| {
                matches!(
                    binding.kind,
                    ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::ArgEvalVariableObject
                )
            })
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

#[cfg(test)]
pub(in crate::runtime) fn verify_unlinked_eval_tree_with_profile(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_profile: &EvalCallerProfile,
    expected_super_call_allowed: bool,
    expected_super_allowed: bool,
) -> Result<(), RuntimeError> {
    verify_unlinked_eval_tree_with_profile_and_arguments(
        function,
        kind,
        caller_strict,
        expected_bindings,
        expected_profile,
        EvalPublicationCapabilities {
            super_call_allowed: expected_super_call_allowed,
            super_allowed: expected_super_allowed,
            arguments_forbidden: false,
        },
    )
}

pub(in crate::runtime) fn verify_unlinked_eval_tree_with_profile_and_arguments(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_profile: &EvalCallerProfile,
    expected_capabilities: EvalPublicationCapabilities,
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
    if expected_capabilities.super_call_allowed && !expected_capabilities.super_allowed {
        return Err(RuntimeError::Engine(Error::internal(
            "eval publication permits super() without SuperProperty",
        )));
    }
    if kind == EvalKind::Indirect
        && (expected_capabilities.super_call_allowed || expected_capabilities.super_allowed)
    {
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
        if binding.kind.is_eval_variable_object() {
            let role_allowed = match scope_kind {
                crate::heap::EvalScopeKind::FunctionRoot => true,
                crate::heap::EvalScopeKind::Parameter => {
                    binding.kind == ClosureVariableKind::ArgEvalVariableObject
                }
                _ => false,
            };
            if !role_allowed
                || binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || eval_variable_object_sentinel(binding.kind)
                    .is_none_or(|sentinel| binding.name.utf16_units().ne(sentinel.encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval root variable-object binding has invalid binding metadata",
                )));
            }
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
        .any(|binding| binding.kind.is_eval_variable_object());
    match (caller_strict, expected_profile.variable_target) {
        (false, EvalCallerVariableTarget::Global) if !has_variable_object => {}
        (true, EvalCallerVariableTarget::StrictLocal) if kind == EvalKind::Direct => {}
        (false, EvalCallerVariableTarget::ExternalBinding(index))
            if expected_bindings
                .get(usize::from(index))
                .is_some_and(|binding| {
                    let target_role_matches = expected_profile
                        .scope_kinds
                        .get(usize::from(binding.scope))
                        .is_some_and(|scope_kind| match scope_kind {
                            crate::heap::EvalScopeKind::FunctionRoot => {
                                binding.kind == ClosureVariableKind::EvalVariableObject
                            }
                            crate::heap::EvalScopeKind::Parameter => {
                                binding.kind == ClosureVariableKind::ArgEvalVariableObject
                            }
                            _ => false,
                        });
                    target_role_matches
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
            expected_capabilities,
        },
    )
}

fn verify_unlinked_tree_with_root(
    function: &UnlinkedFunction,
    root_publication: RootPublication<'_>,
) -> Result<(), RuntimeError> {
    let (synthetic_eval_tree, tree_expected_bindings, tree_expected_profile) =
        match root_publication {
            RootPublication::Script
            | RootPublication::TrustedOrdinaryLeaf
            | RootPublication::Module(_) => (false, &[][..], None),
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
    // `expected_bindings` preserves the caller's inner-to-outer resolution
    // order. A same-shaped pseudo binding in an outer function segment is
    // shadowed and must not gain capability merely by matching the sentinel.
    let (root_derived_this_origin, root_active_function_origin, root_new_target_origin) =
        match root_publication {
            RootPublication::Eval {
                kind: EvalKind::Direct,
                expected_bindings,
                expected_capabilities,
                ..
            } if expected_capabilities.super_call_allowed => (
                expected_bindings
                    .iter()
                    .position(eval_root_binding_is_derived_this),
                expected_bindings.iter().position(|binding| {
                    eval_root_binding_is_super_pseudo(binding, "<this_active_func>")
                }),
                expected_bindings
                    .iter()
                    .position(|binding| eval_root_binding_is_super_pseudo(binding, "<new.target>")),
            ),
            _ => (None, None, None),
        };
    let root_derived_this_origins = function
        .closure_variables()
        .iter()
        .map(|descriptor| match descriptor.source {
            ClosureSource::EvalEnvironment(index) => {
                root_derived_this_origin == Some(usize::from(index))
            }
            _ => false,
        })
        .collect::<Vec<_>>();
    let root_active_function_origins = function
        .closure_variables()
        .iter()
        .map(|descriptor| match descriptor.source {
            ClosureSource::EvalEnvironment(index) => {
                root_active_function_origin == Some(usize::from(index))
            }
            _ => false,
        })
        .collect::<Vec<_>>();
    let root_new_target_origins = function
        .closure_variables()
        .iter()
        .map(|descriptor| match descriptor.source {
            ClosureSource::EvalEnvironment(index) => {
                root_new_target_origin == Some(usize::from(index))
            }
            _ => false,
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
        root_derived_this_origins,
        root_active_function_origins,
        root_new_target_origins,
        0_usize,
    )];
    while let Some((
        function,
        function_depth,
        closure_origins,
        function_name_origins,
        derived_this_origins,
        active_function_origins,
        new_target_origins,
        function_id,
    )) = pending.pop()
    {
        let is_root = function_depth == 0;
        if is_root && function.metadata().class_initializer_kind.is_some() {
            return Err(RuntimeError::Engine(Error::internal(
                "class initializer bytecode escaped the class publication tree",
            )));
        }
        let arg_eval_variable_object_local = function
            .parameter_environment()
            .and_then(|layout| layout.arg_eval_variable_object_local);
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
                        || function
                            .constants()
                            .iter()
                            .any(|constant| !constant.is_plain_primitive())
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
                        && (function.metadata().super_call_allowed
                            || function.metadata().super_allowed)
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
        let expected_eval_bindings = if is_root {
            match root_publication {
                RootPublication::Script
                | RootPublication::TrustedOrdinaryLeaf
                | RootPublication::Module(_) => None,
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
                matches!(instruction, crate::bytecode::Instruction::Arguments(_))
            })
        {
            let local = if let Some(synthetic) = function
                .parameter_environment()
                .and_then(|layout| layout.synthetic_arguments_local)
            {
                let Some(
                    [
                        crate::bytecode::Instruction::Arguments(_),
                        crate::bytecode::Instruction::Dup,
                        crate::bytecode::Instruction::InitializeLocal(target),
                        crate::bytecode::Instruction::PutLocal(body),
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
                let Some(crate::bytecode::Instruction::PutLocal(local)) =
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
        if arg_eval_variable_object_local
            .is_some_and(|index| index >= function.metadata().local_count)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "parameter eval variable-object local is outside bytecode local slots",
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
        private_elements::verify_unlinked(function)?;
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
        let const_locals = function
            .local_definitions()
            .iter()
            .map(|definition| definition.is_const)
            .collect::<Vec<_>>();
        validate_derived_constructor_bytecode_layout(
            function.metadata(),
            function.code(),
            &lexical_locals,
            &const_locals,
            function.closure_variables(),
        )
        .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
        validate_class_initializer_bytecode_layout(function.metadata(), function.code())
            .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
        validate_parameter_initializer_scope_layout(
            function.metadata(),
            function.code(),
            parameter_body_pc.or(pattern_body_pc),
            &lexical_locals,
            &parameter_initializer_locals,
        )
        .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
        validate_pattern_parameter_bytecode_layout(
            function.metadata(),
            function.code(),
            &unnamed_arguments,
            &lexical_locals,
            &parameter_initializer_locals,
            function.parameter_environment(),
        )
        .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
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
            while let Some([source, crate::bytecode::Instruction::PutLocal(local)]) =
                function.code().get(entry_pc..entry_pc + 2)
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
            let is_function_name =
                function.metadata().function_name_local == u16::try_from(index).ok();
            let is_derived_this =
                function.metadata().derived_this_local == u16::try_from(index).ok();
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
                    || definition.name.as_ref().is_none_or(|name| {
                        name.utf16_units().ne("<this_active_func>".encode_utf16())
                    })
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
            } else if definition.kind != ClosureVariableKind::Normal
                && !definition.kind.is_private()
            {
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
                    matches!(pair, [source, crate::bytecode::Instruction::PutLocal(_)]
                        if pseudo_binding_entry(source).is_some())
                })
            {
                variable_environment_pc += 2;
            }
            if matches!(
                function.code().get(variable_environment_pc),
                Some(crate::bytecode::Instruction::Arguments(_))
            ) && matches!(
                function.code().get(variable_environment_pc + 1),
                Some(crate::bytecode::Instruction::PutLocal(_))
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
                            crate::bytecode::Instruction::VariableEnvironment,
                            crate::bytecode::Instruction::PutLocal(target),
                        ]) if *target == index
                    ) && function.code().iter().enumerate().all(|(pc, instruction)| {
                        pc == variable_environment_pc
                            || !matches!(
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
        }
        if function.closure_variables().len() != usize::from(function.metadata().closure_count) {
            return Err(RuntimeError::Engine(Error::internal(
                "function closure descriptor count does not match bytecode metadata",
            )));
        }
        if derived_this_origins.len() != function.closure_variables().len() {
            return Err(RuntimeError::Invariant(
                "derived-this provenance count disagrees with closure descriptors",
            ));
        }
        if active_function_origins.len() != function.closure_variables().len()
            || new_target_origins.len() != function.closure_variables().len()
        {
            return Err(RuntimeError::Invariant(
                "super-call provenance count disagrees with closure descriptors",
            ));
        }
        for instruction in function.code() {
            let crate::bytecode::Instruction::InitializeDerivedVarRef(index) = instruction else {
                continue;
            };
            let descriptor = function
                .closure_variables()
                .get(usize::from(*index))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "captured derived initializer is outside closure slots",
                    ))
                })?;
            let name = unlinked_closure_name(function, descriptor)?;
            if name.is_none_or(|name| name.utf16_units().ne("<this>".encode_utf16())) {
                return Err(RuntimeError::Engine(Error::internal(
                    "captured derived initializer lost its this-binding provenance",
                )));
            }
            if !derived_this_origins
                .get(usize::from(*index))
                .copied()
                .unwrap_or(false)
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "captured derived initializer did not originate from derived this",
                )));
            }
        }
        for instruction in function.code() {
            let crate::bytecode::Instruction::InitializeVarRef(index) = instruction else {
                continue;
            };
            if !is_root || !matches!(root_publication, RootPublication::Module(_)) {
                return Err(RuntimeError::Engine(Error::internal(
                    "module lexical initializer escaped a module root",
                )));
            }
            let descriptor = function
                .closure_variables()
                .get(usize::from(*index))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "module lexical initializer is outside closure slots",
                    ))
                })?;
            if descriptor.source != ClosureSource::ModuleDeclaration
                || !descriptor.is_lexical
                || descriptor.kind != ClosureVariableKind::Normal
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "module lexical initializer targeted a non-declaration binding",
                )));
            }
        }
        for instruction in function.code() {
            let crate::bytecode::Instruction::InitializeModuleImportCollision(index) = instruction
            else {
                continue;
            };
            let RootPublication::Module(module) = root_publication else {
                return Err(RuntimeError::Engine(Error::internal(
                    "module import collision initializer escaped a module root",
                )));
            };
            if !is_root
                || !module.import_collisions().iter().any(|collision| {
                    collision.closure_index == *index
                        && matches!(
                            collision.declaration,
                            ModuleImportCollisionDeclaration::Lexical
                                | ModuleImportCollisionDeclaration::Function
                        )
                })
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "module import collision initializer has no matching ledger entry",
                )));
            }
            let descriptor = function
                .closure_variables()
                .get(usize::from(*index))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "module import collision initializer is outside closure slots",
                    ))
                })?;
            if descriptor.source != ClosureSource::ModuleImportCollision
                || !descriptor.is_lexical
                || !descriptor.is_const
                || !matches!(
                    descriptor.kind,
                    ClosureVariableKind::Normal | ClosureVariableKind::ModuleImportView
                )
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "module import collision initializer targeted a non-import binding",
                )));
            }
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
                RootPublication::Script
                | RootPublication::TrustedOrdinaryLeaf
                | RootPublication::Module(_) => false,
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
                                && !expected_bindings
                                    .iter()
                                    .any(|binding| binding.kind.is_eval_variable_object())))
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
            if descriptor.kind.is_eval_variable_object()
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
                    | ClosureVariableKind::ArgEvalVariableObject
                    | ClosureVariableKind::WithObject
                    | ClosureVariableKind::PrivateField
                    | ClosureVariableKind::PrivateMethod
                    | ClosureVariableKind::PrivateGetter
                    | ClosureVariableKind::PrivateSetter
                    | ClosureVariableKind::PrivateGetterSetter
            ) || matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration
                    | ClosureSource::Global
                    | ClosureSource::ParentGlobal(_)
                    | ClosureSource::EvalEnvironment(_)
                    | ClosureSource::ModuleDeclaration
                    | ClosureSource::ModuleImport
                    | ClosureSource::ModuleImportCollision
                    | ClosureSource::ModuleImportMeta
            );
            let name = unlinked_closure_name(function, descriptor)?;
            if descriptor.kind.is_eval_variable_object()
                && eval_variable_object_sentinel(descriptor.kind).is_none_or(|sentinel| {
                    name.is_none_or(|name| name.utf16_units().ne(sentinel.encode_utf16()))
                })
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval variable-object descriptor lost its role sentinel",
                )));
            }
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
                    RootPublication::TrustedOrdinaryLeaf => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "trusted ordinary leaf retained a closure descriptor",
                        )));
                    }
                    RootPublication::Module(_) => match descriptor.source {
                        ClosureSource::ModuleDeclaration => {
                            if descriptor.kind != ClosureVariableKind::Normal
                                || (descriptor.is_const && !descriptor.is_lexical)
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "module declaration descriptor has invalid binding metadata",
                                )));
                            }
                        }
                        ClosureSource::ModuleImport => {
                            if descriptor.kind != ClosureVariableKind::ModuleImportView
                                || !descriptor.is_lexical
                                || !descriptor.is_const
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "module import descriptor has invalid binding metadata",
                                )));
                            }
                        }
                        ClosureSource::ModuleImportCollision => {
                            if !descriptor.is_lexical
                                || !descriptor.is_const
                                || !matches!(
                                    descriptor.kind,
                                    ClosureVariableKind::Normal
                                        | ClosureVariableKind::ModuleImportView
                                )
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "module import collision descriptor has invalid binding metadata",
                                )));
                            }
                        }
                        ClosureSource::ModuleImportMeta => {
                            if descriptor.kind != ClosureVariableKind::Normal
                                || !descriptor.is_lexical
                                || !descriptor.is_const
                                || name
                                    != Some(&JsString::from_static(
                                        crate::module::MODULE_IMPORT_META_BINDING_NAME,
                                    ))
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "import.meta descriptor has invalid binding metadata",
                                )));
                            }
                        }
                        ClosureSource::Global => {
                            if descriptor.kind != ClosureVariableKind::Normal
                                || descriptor.is_lexical
                                || descriptor.is_const
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "module global descriptor has invalid binding metadata",
                                )));
                            }
                        }
                        _ => {
                            return Err(RuntimeError::Engine(Error::internal(
                                "module root closure descriptor used a non-module source",
                            )));
                        }
                    },
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
                    ClosureSource::ModuleDeclaration
                    | ClosureSource::ModuleImport
                    | ClosureSource::ModuleImportCollision
                    | ClosureSource::ModuleImportMeta => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "only a verified module root may own a module binding",
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
        let mut global_function_prologue_offset = 0_usize;
        let mut pseudo_rank = 0_u8;
        let mut pseudo_targets = Vec::with_capacity(4);
        let mut authenticated_active_function_local = None;
        let mut authenticated_new_target_local = None;
        while let Some([source, crate::bytecode::Instruction::PutLocal(local)]) = function
            .code()
            .get(global_function_prologue_offset..global_function_prologue_offset + 2)
        {
            let Some((rank, expected_name)) = pseudo_binding_entry(source) else {
                break;
            };
            let Some(definition) = function.local_definitions().get(usize::from(*local)) else {
                break;
            };
            let is_pseudo_binding = definition.kind == ClosureVariableKind::Normal
                && !definition.is_lexical
                && !definition.is_const
                && definition
                    .name
                    .as_ref()
                    .is_some_and(|name| name.utf16_units().eq(expected_name.encode_utf16()));
            if !is_pseudo_binding {
                break;
            }
            if rank <= pseudo_rank || pseudo_targets.contains(local) {
                return Err(RuntimeError::Engine(Error::internal(
                    "global function pseudo-binding prologue is malformed",
                )));
            }
            pseudo_rank = rank;
            pseudo_targets.push(*local);
            match source {
                crate::bytecode::Instruction::PushActiveFunction => {
                    authenticated_active_function_local = Some(*local);
                }
                crate::bytecode::Instruction::PushNewTarget => {
                    authenticated_new_target_local = Some(*local);
                }
                _ => {}
            }
            global_function_prologue_offset += 2;
        }
        let explicit_control_flow_targets = function
            .code()
            .iter()
            .filter_map(explicit_control_flow_target)
            .collect::<HashSet<_>>();
        for (pc, instruction) in function.code().iter().enumerate() {
            if matches!(
                instruction,
                crate::bytecode::Instruction::CallClassInstanceInitializer
            ) && let Some(crate::bytecode::Instruction::GetVarRef(index)) = pc
                .checked_sub(1)
                .and_then(|read_pc| function.code().get(read_pc))
                && !active_function_origins
                    .get(usize::from(*index))
                    .copied()
                    .unwrap_or(false)
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "class instance initializer relay did not read the authenticated active function",
                )));
            }
            if !matches!(instruction, crate::bytecode::Instruction::MarkSuperCall) {
                continue;
            }
            let Some(active_pc) = pc.checked_sub(3) else {
                return Err(RuntimeError::Engine(Error::internal(
                    "super-call marker has no authenticated operand reads",
                )));
            };
            if explicit_control_flow_targets.contains(&(pc - 2))
                || explicit_control_flow_targets.contains(&(pc - 1))
                || explicit_control_flow_targets.contains(&pc)
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "super-call operand protocol has a non-fallthrough entry",
                )));
            }
            if !matches!(
                function.code().get(pc - 2),
                Some(crate::bytecode::Instruction::GetSuper)
            ) {
                return Err(RuntimeError::Engine(Error::internal(
                    "super-call marker is not preceded by GetSuper",
                )));
            }
            let active_read_is_authenticated = match function.code().get(active_pc) {
                Some(crate::bytecode::Instruction::GetLocal(index)) => {
                    authenticated_active_function_local == Some(*index)
                }
                Some(crate::bytecode::Instruction::GetVarRef(index)) => active_function_origins
                    .get(usize::from(*index))
                    .copied()
                    .unwrap_or(false),
                _ => false,
            };
            if !active_read_is_authenticated {
                return Err(RuntimeError::Engine(Error::internal(
                    "super-call marker did not read the authenticated active function",
                )));
            }
            let new_target_read_is_authenticated = match function.code().get(pc - 1) {
                Some(crate::bytecode::Instruction::GetLocal(index)) => {
                    authenticated_new_target_local == Some(*index)
                }
                Some(crate::bytecode::Instruction::GetVarRef(index)) => new_target_origins
                    .get(usize::from(*index))
                    .copied()
                    .unwrap_or(false),
                _ => false,
            };
            if !new_target_read_is_authenticated {
                return Err(RuntimeError::Engine(Error::internal(
                    "super-call marker did not read the authenticated new.target",
                )));
            }
        }
        global_function_prologue_offset += usize::from(matches!(
            function.code().get(global_function_prologue_offset),
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
        validate_class_initializer_publication_edges(
            function,
            &child_closure_pcs,
            &explicit_control_flow_targets,
        )?;

        validate_eval_environment_phase_layout(
            function.eval_environments(),
            EvalEnvironmentPhaseContext {
                metadata: function.metadata(),
                code: function.code(),
                parameter_body_pc,
                pattern_body_pc,
                lexical_locals: &lexical_locals,
                parameter_initializer_locals: &parameter_initializer_locals,
                parameter_initializer_visible_locals: parameter_initializer_capture_locals
                    .as_deref(),
                parameter_environment: function.parameter_environment(),
            },
        )
        .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;

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
        for environment in function
            .eval_environments()
            .iter()
            // Ordinary direct eval also carries compiler-private `<this>` and
            // `new.target` spellings. They become derived-constructor
            // capabilities only when this exact call site inherited super()
            // authority; super-property-only object methods must not be
            // mistaken for derived constructors.
            .filter(|environment| environment.super_call_allowed)
        {
            verify_eval_super_pseudo_bindings(
                environment,
                function.metadata().derived_this_local,
                authenticated_active_function_local,
                authenticated_new_target_local,
                &derived_this_origins,
                &active_function_origins,
                &new_target_origins,
            )?;
        }
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
                | crate::bytecode::Instruction::DefineMethod { key: index, .. }
                | crate::bytecode::Instruction::DefineClass { name: index, .. } => {
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
                    if eval_variable_object_local_kind(function, *index).is_some()
                        && !(matches!(instruction, crate::bytecode::Instruction::PutLocal(_))
                            && pc.checked_sub(1).is_some_and(|previous| {
                                matches!(
                                    function.code().get(previous),
                                    Some(crate::bytecode::Instruction::VariableEnvironment)
                                )
                            })) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "ordinary local opcode referenced a private eval variable object",
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
                                    | ClosureVariableKind::ArgEvalVariableObject
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
                | crate::bytecode::Instruction::InitializeVarRef(index)
                | crate::bytecode::Instruction::InitializeModuleImportCollision(index)
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
                let mut child_derived_this_origins =
                    Vec::with_capacity(child.closure_variables().len());
                let mut child_active_function_origins =
                    Vec::with_capacity(child.closure_variables().len());
                let mut child_new_target_origins =
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
                                    && (index
                                        < function
                                            .metadata()
                                            .parameter_environment_local_count
                                        || function.parameter_environment().is_some_and(
                                            |layout| {
                                                layout.synthetic_arguments_local == Some(index)
                                            },
                                        )
                                        || function
                                            .local_definitions()
                                            .get(usize::from(index))
                                            .is_some_and(|definition| {
                                                definition.is_parameter_initializer
                                            })) =>
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "function body closure captured a parameter-initializer cell",
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
                        let instantiated_in_body = closure_pcs.iter().any(|pc| *pc >= body_pc);
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
                                    && function.parameter_environment().is_none_or(|layout| {
                                        layout.synthetic_arguments_local != Some(index)
                                    })
                                    && function
                                        .local_definitions()
                                        .get(usize::from(index))
                                        .is_some_and(|definition| {
                                            definition.is_lexical
                                                && !definition.is_parameter_initializer
                                        }) =>
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "pattern initializer closure captured a body lexical local",
                                )));
                            }
                            ClosureSource::ParentLocal(index)
                                if instantiated_in_body
                                    && function
                                        .local_definitions()
                                        .get(usize::from(index))
                                        .is_some_and(|definition| {
                                            definition.is_parameter_initializer
                                        }) =>
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "function body closure captured a parameter-initializer local",
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
                    let mut derived_this_origin = false;
                    let mut active_function_origin = false;
                    let mut new_target_origin = false;
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
                                        | ClosureVariableKind::ArgEvalVariableObject
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
                            derived_this_origin =
                                function.metadata().derived_this_local == Some(index);
                            active_function_origin =
                                authenticated_active_function_local == Some(index);
                            new_target_origin = authenticated_new_target_local == Some(index);
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
                                        | ClosureVariableKind::ArgEvalVariableObject
                                )
                                || descriptor_name.is_some())
                                && descriptor_name != unlinked_closure_name(function, parent)?
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "transitive closure relay changed its lexical binding name",
                                )));
                            }
                            derived_this_origin = derived_this_origins
                                .get(usize::from(index))
                                .copied()
                                .unwrap_or(false);
                            active_function_origin = active_function_origins
                                .get(usize::from(index))
                                .copied()
                                .unwrap_or(false);
                            new_target_origin = new_target_origins
                                .get(usize::from(index))
                                .copied()
                                .unwrap_or(false);
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
                        ClosureSource::ModuleDeclaration
                        | ClosureSource::ModuleImport
                        | ClosureSource::ModuleImportCollision
                        | ClosureSource::ModuleImportMeta => {
                            return Err(RuntimeError::Engine(Error::internal(
                                "child bytecode directly referenced a module-root descriptor",
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
                    child_derived_this_origins.push(derived_this_origin);
                    child_active_function_origins.push(active_function_origin);
                    child_new_target_origins.push(new_target_origin);
                }
                if child.metadata().super_call_allowed
                    && child.metadata().constructor_kind != ConstructorKind::Derived
                    && (!function.metadata().super_call_allowed || child.metadata().has_prototype)
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "inherited super call has no parent authority",
                    )));
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
                    child_derived_this_origins,
                    child_active_function_origins,
                    child_new_target_origins,
                    child_id,
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
    use crate::compiler::compile_unlinked_module_with_filename;
    use crate::debug::DebugInfoMode;
    use crate::function::{
        UnlinkedFunctionDebug, UnlinkedFunctionParts, UnlinkedVariableDefinition,
    };
    use crate::heap::{
        EvalBinding, EvalBindingSource, EvalEnvironment, EvalScope, EvalScopeKind,
        EvalVariableEnvironment, ParameterArgumentCell, ParameterBodyStorage,
        ParameterDefaultSource, ParameterPatternCopy,
    };
    use crate::module::{ModuleLinkInitializer, ModuleLinkInitializerValue, UnlinkedModuleTables};

    fn trusted_ordinary_leaf(metadata: FunctionMetadata) -> UnlinkedFunction {
        UnlinkedFunction::new(
            vec![Instruction::GetArg(0), Instruction::Return],
            Vec::new(),
            metadata,
        )
    }

    #[test]
    fn trusted_ordinary_leaf_has_a_distinct_root_publication_role() {
        let metadata = FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            max_stack: 1,
            strip_variable_debug: true,
            function_kind: FunctionKind::Normal,
            has_prototype: true,
            constructor_kind: ConstructorKind::Base,
            ..FunctionMetadata::default()
        };
        verify_unlinked_ordinary_leaf(&trusted_ordinary_leaf(metadata)).unwrap();

        for forged in [
            FunctionMetadata {
                super_allowed: true,
                ..metadata
            },
            FunctionMetadata {
                arguments_forbidden: true,
                ..metadata
            },
            FunctionMetadata {
                has_prototype: false,
                ..metadata
            },
            FunctionMetadata {
                constructor_kind: ConstructorKind::None,
                ..metadata
            },
            FunctionMetadata {
                strip_variable_debug: false,
                ..metadata
            },
        ] {
            assert!(
                verify_unlinked_ordinary_leaf(&trusted_ordinary_leaf(forged))
                    .unwrap_err()
                    .to_string()
                    .contains(
                        "trusted ordinary leaf metadata disagrees with its publication entry point"
                    )
            );
        }

        let child = trusted_ordinary_leaf(metadata);
        let with_child = UnlinkedFunction::new(
            vec![Instruction::GetArg(0), Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            metadata,
        );
        assert!(verify_unlinked_ordinary_leaf(&with_child).is_err());

        let with_atom_string = UnlinkedFunction::new(
            vec![Instruction::PushConst(0), Instruction::Return],
            vec![UnlinkedConstant::atom_string(JsString::from_static("atom"))],
            metadata,
        );
        assert!(verify_unlinked_ordinary_leaf(&with_atom_string).is_err());

        let named_argument = trusted_ordinary_leaf(metadata).with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(Some(
                JsString::from_static("argument"),
            ))],
            Vec::new(),
        );
        assert!(verify_unlinked_ordinary_leaf(&named_argument).is_err());

        let lexical_argument = trusted_ordinary_leaf(metadata).with_variable_definitions(
            vec![UnlinkedVariableDefinition::lexical(None, false)],
            Vec::new(),
        );
        assert!(verify_unlinked_ordinary_leaf(&lexical_argument).is_err());

        let parameter_environment = trusted_ordinary_leaf(metadata).with_parameter_environment(
            Some(ParameterEnvironmentLayout {
                initialization_end: 0,
                argument_cells: Box::new([]),
                pattern_copies: Box::new([]),
                default_sources: Box::new([]),
                synthetic_arguments_local: None,
                arg_eval_variable_object_local: None,
            }),
        );
        assert!(verify_unlinked_ordinary_leaf(&parameter_environment).is_err());

        let with_debug = trusted_ordinary_leaf(metadata).with_debug(UnlinkedFunctionDebug {
            filename: JsString::from_static("ordinary-leaf.js"),
            pc2line: None,
            source: None,
        });
        assert!(verify_unlinked_ordinary_leaf(&with_debug).is_err());
    }

    fn module_with_link_initializers(
        module: UnlinkedModule,
        link_initializers: Vec<ModuleLinkInitializer>,
    ) -> UnlinkedModule {
        let parts = module.into_parts();
        UnlinkedModule::new(
            parts.name,
            parts.function,
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers,
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        )
    }

    fn module_with_import_collisions(
        module: UnlinkedModule,
        import_collisions: Vec<crate::module::ModuleImportCollision>,
    ) -> UnlinkedModule {
        let parts = module.into_parts();
        UnlinkedModule::new(
            parts.name,
            parts.function,
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions,
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        )
    }

    fn module_with_code(
        module: UnlinkedModule,
        code: Vec<Instruction>,
        max_stack: u16,
    ) -> UnlinkedModule {
        let parts = module.into_parts();
        let mut function_parts = parts.function.into_parts();
        function_parts.code = code;
        function_parts.metadata.max_stack = max_stack;
        UnlinkedModule::new(
            parts.name,
            function_from_parts(function_parts),
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        )
    }

    fn module_with_mutated_function(
        module: UnlinkedModule,
        mutate: impl FnOnce(&mut UnlinkedFunctionParts),
    ) -> UnlinkedModule {
        let parts = module.into_parts();
        let mut function_parts = parts.function.into_parts();
        mutate(&mut function_parts);
        UnlinkedModule::new(
            parts.name,
            function_from_parts(function_parts),
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        )
    }

    fn module_with_top_level_await_flag(
        module: UnlinkedModule,
        has_top_level_await: bool,
    ) -> UnlinkedModule {
        let parts = module.into_parts();
        UnlinkedModule::new(
            parts.name,
            parts.function,
            has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        )
    }

    fn function_from_parts(parts: UnlinkedFunctionParts) -> UnlinkedFunction {
        let UnlinkedFunctionParts {
            code,
            constants,
            metadata,
            parameter_environment,
            func_name,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments,
            debug,
        } = parts;
        let mut function = UnlinkedFunction::new_with_closure_variables(
            code,
            constants,
            metadata,
            closure_variables,
        )
        .with_parameter_environment(parameter_environment)
        .with_name(func_name)
        .with_variable_definitions(argument_definitions, local_definitions)
        .with_eval_environments(eval_environments);
        if let Some(debug) = debug {
            function = function.with_debug(debug);
        }
        function
    }

    #[test]
    fn module_root_rejects_forged_top_level_await_flag_in_both_directions() {
        for (source, forged_has_top_level_await) in [("0;", true), ("await 0;", false)] {
            let module = compile_unlinked_module_with_filename(
                source,
                "forged-top-level-await-flag.mjs",
                DebugInfoMode::StripDebug,
            )
            .unwrap();
            let forged = module_with_top_level_await_flag(module, forged_has_top_level_await);
            let error = verify_unlinked_module_tree(&forged).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("module root metadata disagrees with its publication entry point"),
                "{source:?}: {error}"
            );
        }
    }

    #[test]
    fn module_link_initializer_ledger_rejects_reordered_slots() {
        let module = compile_unlinked_module_with_filename(
            "var first; function second() {}",
            "forged-ledger-order.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        verify_unlinked_module_tree(&module).unwrap();

        let mut initializers = module.link_initializers().to_vec();
        assert_eq!(initializers.len(), 2);
        initializers.swap(0, 1);
        let forged = module_with_link_initializers(module, initializers);
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link initializer ledger is not in declaration order")
        );
    }

    #[test]
    fn module_link_initializer_ledger_rejects_value_mismatch() {
        let module = compile_unlinked_module_with_filename(
            "var first; function second() {}",
            "forged-ledger-value.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        verify_unlinked_module_tree(&module).unwrap();

        let mut initializers = module.link_initializers().to_vec();
        let function = initializers
            .iter_mut()
            .find(|initializer| {
                matches!(
                    initializer.value,
                    ModuleLinkInitializerValue::Function { .. }
                )
            })
            .expect("module function declaration has a link initializer");
        function.value = ModuleLinkInitializerValue::Undefined;
        let forged = module_with_link_initializers(module, initializers);
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link entry initializer is not canonical")
        );
    }

    #[test]
    fn module_import_collision_requires_an_exact_unique_ledger() {
        let module = compile_unlinked_module_with_filename(
            "import { value } from './dependency.js'; let value = 1;",
            "forged-collision-ledger.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        verify_unlinked_module_tree(&module).unwrap();

        let missing = module_with_import_collisions(module, Vec::new());
        let error = verify_unlinked_module_tree(&missing).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module import table disagrees with its closure descriptor")
        );

        let module = compile_unlinked_module_with_filename(
            "import { value } from './dependency.js'; let value = 1;",
            "forged-duplicate-collision-ledger.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let collision = module.import_collisions()[0];
        let duplicate = module_with_import_collisions(module, vec![collision, collision]);
        let error = verify_unlinked_module_tree(&duplicate).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module import collision ledger reused a closure slot")
        );
    }

    #[test]
    fn module_import_meta_descriptor_is_unique_and_exact() {
        let compile = || {
            compile_unlinked_module_with_filename(
                "globalThis.meta = import.meta;",
                "forged-import-meta.mjs",
                DebugInfoMode::StripDebug,
            )
            .unwrap()
        };

        let malformed = module_with_mutated_function(compile(), |parts| {
            let descriptor = parts
                .closure_variables
                .iter_mut()
                .find(|descriptor| descriptor.source == ClosureSource::ModuleImportMeta)
                .unwrap();
            descriptor.is_const = false;
        });
        let error = verify_unlinked_module_tree(&malformed).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("import.meta descriptor has invalid binding metadata"),
            "{error}"
        );

        let unnamed = module_with_mutated_function(compile(), |parts| {
            let descriptor = parts
                .closure_variables
                .iter_mut()
                .find(|descriptor| descriptor.source == ClosureSource::ModuleImportMeta)
                .unwrap();
            descriptor.name = ClosureVariableName::None;
        });
        let error = verify_unlinked_module_tree(&unnamed).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("import.meta descriptor has invalid binding metadata"),
            "{error}"
        );

        let duplicate = module_with_mutated_function(compile(), |parts| {
            let descriptor = parts
                .closure_variables
                .iter()
                .find(|descriptor| descriptor.source == ClosureSource::ModuleImportMeta)
                .copied()
                .unwrap();
            parts.closure_variables.push(descriptor);
            parts.metadata.closure_count += 1;
        });
        let error = verify_unlinked_module_tree(&duplicate).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module contains more than one import.meta binding"),
            "{error}"
        );
    }

    #[test]
    fn module_evaluation_cannot_reenter_link_phase() {
        let module = compile_unlinked_module_with_filename(
            "import { value } from './dependency.js'; let value = 1;",
            "forged-collision-control-flow.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let parts = module.into_parts();
        let mut function_parts = parts.function.into_parts();
        let insertion = function_parts.code.len() - 2;
        function_parts.code.insert(insertion, Instruction::Goto(2));
        let forged = UnlinkedModule::new(
            parts.name,
            function_from_parts(function_parts),
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        );
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module evaluation control flow entered the link phase"),
            "{error}"
        );
    }

    #[test]
    fn module_initializer_flow_rejects_stack_valid_skips_repeats_and_shared_gosubs() {
        let cases = [
            (
                "forward-skip",
                vec![
                    Instruction::PushThis,
                    Instruction::IfFalse(4),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::PushFalse,
                    Instruction::IfFalse(9),
                    Instruction::PushI32(1),
                    Instruction::InitializeModuleImportCollision(0),
                    Instruction::Goto(9),
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                1,
            ),
            (
                "back-edge-repeat",
                vec![
                    Instruction::PushThis,
                    Instruction::IfFalse(4),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::PushI32(1),
                    Instruction::InitializeModuleImportCollision(0),
                    Instruction::PushI32(2),
                    Instruction::Goto(5),
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                1,
            ),
            (
                "shared-gosub-repeat",
                vec![
                    Instruction::PushThis,
                    Instruction::IfFalse(4),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::Gosub(7),
                    Instruction::Gosub(7),
                    Instruction::Goto(10),
                    Instruction::PushI32(1),
                    Instruction::InitializeModuleImportCollision(0),
                    Instruction::Ret,
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                2,
            ),
            (
                "caught-call-skip",
                vec![
                    Instruction::PushThis,
                    Instruction::IfFalse(4),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::Catch(13),
                    Instruction::Undefined,
                    Instruction::Call(0),
                    Instruction::Drop,
                    Instruction::PushI32(1),
                    Instruction::InitializeModuleImportCollision(0),
                    Instruction::DropCatch,
                    Instruction::Goto(14),
                    Instruction::Nop,
                    Instruction::Return,
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                2,
            ),
        ];

        for (label, code, max_stack) in cases {
            let module = compile_unlinked_module_with_filename(
                "import { value } from './dependency.js'; let value = 1;",
                "forged-stack-valid-initializer-flow.mjs",
                DebugInfoMode::StripDebug,
            )
            .unwrap();
            let forged = module_with_code(module, code, max_stack);
            verify_parts(
                forged.function().code(),
                forged.function().constants().len(),
                forged.function().metadata().max_stack,
            )
            .unwrap_or_else(|error| panic!("{label}: generic verifier rejected fixture: {error}"));
            let error = verify_unlinked_module_tree(&forged).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("module lexical initializer is not a one-shot control-flow cut"),
                "{label}: {error}"
            );
        }
    }

    #[test]
    fn module_initializer_flow_tracks_cleanup_discarded_gosub_returns() {
        let cases = [
            (
                "nip-catch",
                vec![
                    Instruction::PushThis,
                    Instruction::IfFalse(4),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::Gosub(7),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::Catch(18),
                    Instruction::Gosub(13),
                    Instruction::PushI32(1),
                    Instruction::InitializeModuleImportCollision(0),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::Undefined,
                    Instruction::NipCatch,
                    Instruction::Gosub(20),
                    Instruction::Drop,
                    Instruction::Ret,
                    Instruction::Throw,
                    Instruction::Nop,
                    Instruction::Ret,
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                4,
            ),
            (
                "iterator-drop-preserve",
                vec![
                    Instruction::PushThis,
                    Instruction::IfFalse(4),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::Gosub(7),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::ArrayFrom(0),
                    Instruction::ForOfStart,
                    Instruction::Gosub(14),
                    Instruction::PushI32(1),
                    Instruction::InitializeModuleImportCollision(0),
                    Instruction::Undefined,
                    Instruction::Throw,
                    Instruction::Undefined,
                    Instruction::IteratorDropPreserve,
                    Instruction::Gosub(20),
                    Instruction::Drop,
                    Instruction::Ret,
                    Instruction::Nop,
                    Instruction::Ret,
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                6,
            ),
            (
                "return-derived-throw",
                vec![
                    Instruction::PushThis,
                    Instruction::IfFalse(4),
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::PushI32(1),
                    Instruction::InitializeModuleImportCollision(0),
                    Instruction::Catch(13),
                    Instruction::Catch(10),
                    Instruction::Undefined,
                    Instruction::Throw,
                    Instruction::ReturnDerived(0),
                    Instruction::Nop,
                    Instruction::Nop,
                    Instruction::Goto(5),
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                3,
            ),
        ];

        for (label, code, max_stack) in cases {
            let module = compile_unlinked_module_with_filename(
                "import { value } from './dependency.js'; let value = 1;",
                "forged-cleanup-initializer-flow.mjs",
                DebugInfoMode::StripDebug,
            )
            .unwrap();
            let forged = module_with_code(module, code, max_stack);
            verify_parts(
                forged.function().code(),
                forged.function().constants().len(),
                forged.function().metadata().max_stack,
            )
            .unwrap_or_else(|error| panic!("{label}: generic verifier rejected fixture: {error}"));
            let error = verify_unlinked_module_tree(&forged).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("module lexical initializer is not a one-shot control-flow cut"),
                "{label}: {error}"
            );
        }
    }

    #[test]
    fn module_lexical_initializers_allow_expression_joins() {
        for import in [
            "",
            "import value from './dependency.js';",
            "import { value } from './dependency.js';",
            "import * as value from './dependency.js';",
        ] {
            for expression in [
                "globalThis.flag ? 1 : 2",
                "globalThis.flag && 42",
                "globalThis.flag || 42",
                "globalThis.flag ?? 42",
            ] {
                let source = format!("{import} let value = {expression};");
                let module = compile_unlinked_module_with_filename(
                    &source,
                    "module-lexical-expression-join.mjs",
                    DebugInfoMode::StripDebug,
                )
                .unwrap_or_else(|error| panic!("{source}: {error}"));
                let initializer_pc = module
                    .function()
                    .code()
                    .iter()
                    .position(|instruction| {
                        matches!(
                            instruction,
                            Instruction::InitializeVarRef(_)
                                | Instruction::InitializeModuleImportCollision(_)
                        )
                    })
                    .unwrap_or_else(|| panic!("{source}: lexical initializer disappeared"));
                assert!(
                    module.function().code()[..initializer_pc]
                        .iter()
                        .any(|instruction| {
                            explicit_control_flow_target(instruction) == Some(initializer_pc)
                        }),
                    "{source}: expression lost its initializer join edge"
                );
                verify_unlinked_module_tree(&module)
                    .unwrap_or_else(|error| panic!("{source}: {error}"));
            }
        }
    }

    #[test]
    fn module_import_collision_destructuring_initializers_publish() {
        for source in [
            "import { value } from './dependency.js'; let [value] = [1];",
            "import { value } from './dependency.js'; const [value] = [1];",
            "import { value } from './dependency.js';\
             let { answer: value } = { answer: 1 };",
            "import { value } from './dependency.js';\
             const { answer: value } = { answer: 1 };",
            "import { first, second } from './dependency.js';\
             let [first, second] = [1, 2];",
            "import { first, second } from './dependency.js';\
             const { first, nested: { second } } =\
                 { first: 1, nested: { second: 2 } };",
            "import { value } from './dependency.js';\
             let [[value] = [1]] = [];",
            "import { first, second } from './dependency.js';\
             let [[first] = [1], second] = [[], 2];",
            "export let [first, ...second] = [1, 2, 3];",
        ] {
            let module = compile_unlinked_module_with_filename(
                source,
                "module-import-collision-destructuring.mjs",
                DebugInfoMode::StripDebug,
            )
            .unwrap_or_else(|error| panic!("{source}: {error}"));
            verify_unlinked_module_tree(&module).unwrap_or_else(|error| {
                panic!(
                    "{source}: {error}\nbytecode: {:#?}",
                    module.function().code()
                )
            });
        }
    }

    #[test]
    fn module_initializer_flow_handles_many_shared_finally_entries() {
        let mut source = String::from("outer: { try {");
        for _ in 0..384 {
            source.push_str("if (globalThis.flag) break outer;");
        }
        source.push_str("} finally {");
        for _ in 0..384 {
            source.push_str("globalThis.observed;");
        }
        source.push_str("} } export let answer = 42;");

        let module = compile_unlinked_module_with_filename(
            &source,
            "module-many-finally-entries.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        verify_unlinked_module_tree(&module).unwrap();
    }

    #[test]
    fn module_initializer_flow_summarizes_nested_shared_gosubs() {
        const DEPTH: usize = 48;

        let mut code = vec![
            Instruction::PushThis,
            Instruction::IfFalse(4),
            Instruction::Undefined,
            Instruction::Return,
            Instruction::Gosub(0),
            Instruction::Goto(0),
        ];
        let mut layers = Vec::with_capacity(DEPTH);
        for _ in 0..DEPTH {
            let start = code.len();
            layers.push(start);
            code.extend([
                Instruction::PushTrue,
                Instruction::IfFalse(u32::try_from(start + 4).unwrap()),
                Instruction::Gosub(0),
                Instruction::Goto(u32::try_from(start + 5).unwrap()),
                Instruction::Gosub(0),
                Instruction::Ret,
            ]);
        }
        let leaf = code.len();
        code.extend([
            Instruction::PushI32(1),
            Instruction::InitializeModuleImportCollision(0),
            Instruction::Ret,
        ]);
        let end = code.len();
        code.extend([Instruction::Undefined, Instruction::Return]);

        code[4] = Instruction::Gosub(u32::try_from(layers[0]).unwrap());
        code[5] = Instruction::Goto(u32::try_from(end).unwrap());
        for (index, start) in layers.iter().copied().enumerate() {
            let next = layers.get(index + 1).copied().unwrap_or(leaf);
            code[start + 2] = Instruction::Gosub(u32::try_from(next).unwrap());
            code[start + 4] = Instruction::Gosub(u32::try_from(next).unwrap());
        }

        let module = compile_unlinked_module_with_filename(
            "import { value } from './dependency.js'; let value = 1;",
            "nested-shared-gosub-initializer-flow.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let max_stack = u16::try_from(DEPTH + 2).unwrap();
        let forged = module_with_code(module, code, max_stack);
        verify_parts(
            forged.function().code(),
            forged.function().constants().len(),
            forged.function().metadata().max_stack,
        )
        .unwrap();
        verify_unlinked_module_tree(&forged).unwrap();
    }

    #[test]
    fn module_initializer_flow_does_not_invent_throws_for_pure_bytecode() {
        let module = compile_unlinked_module_with_filename(
            "import { value } from './dependency.js'; let value = 1;",
            "pure-catch-initializer-flow.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let forged = module_with_code(
            module,
            vec![
                Instruction::PushThis,
                Instruction::IfFalse(4),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Catch(9),
                Instruction::PushI32(1),
                Instruction::InitializeModuleImportCollision(0),
                Instruction::DropCatch,
                Instruction::Goto(10),
                Instruction::Return,
                Instruction::Undefined,
                Instruction::Return,
            ],
            2,
        );
        verify_parts(
            forged.function().code(),
            forged.function().constants().len(),
            forged.function().metadata().max_stack,
        )
        .unwrap();
        verify_unlinked_module_tree(&forged).unwrap();
    }

    #[test]
    fn module_function_initializer_requires_a_strict_declaration_child() {
        for private_name in [false, true] {
            let module = compile_unlinked_module_with_filename(
                "export function value() {}",
                "forged-function-child-metadata.mjs",
                DebugInfoMode::StripDebug,
            )
            .unwrap();
            let constant = match module.link_initializers()[0].value {
                ModuleLinkInitializerValue::Function { constant, .. } => constant,
                _ => panic!("module function lost its initializer"),
            };
            let parts = module.into_parts();
            let mut function_parts = parts.function.into_parts();
            let original = std::mem::replace(
                &mut function_parts.constants[constant as usize],
                UnlinkedConstant::primitive(Value::Undefined).unwrap(),
            );
            let (_, _, child) = original.into_parts();
            let mut child_parts = child
                .expect("module function stopped referencing a child")
                .into_parts();
            if private_name {
                child_parts.metadata.function_name_local = Some(0);
            } else {
                child_parts.metadata.strict = false;
            }
            function_parts.constants[constant as usize] =
                UnlinkedConstant::child(function_from_parts(child_parts));
            let forged = UnlinkedModule::new(
                parts.name,
                function_from_parts(function_parts),
                parts.has_top_level_await,
                UnlinkedModuleTables {
                    declaration_order: parts.declaration_order.into_vec(),
                    link_initializers: parts.link_initializers.into_vec(),
                    import_collisions: parts.import_collisions.into_vec(),
                    requested_modules: parts.requested_modules.into_vec(),
                    imports: parts.imports.into_vec(),
                    exports: parts.exports.into_vec(),
                    star_exports: parts.star_exports.into_vec(),
                },
            );
            let error = verify_unlinked_module_tree(&forged).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("module link entry initializer is not canonical"),
                "{error}"
            );
        }
    }

    #[test]
    fn anonymous_default_function_initializer_requires_exact_name_inference() {
        let module = compile_unlinked_module_with_filename(
            "export default function () {}",
            "forged-default-function-name.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        verify_unlinked_module_tree(&module).unwrap();

        let mut initializers = module.link_initializers().to_vec();
        let [initializer] = initializers.as_mut_slice() else {
            panic!("anonymous default function lost its initializer");
        };
        let ModuleLinkInitializerValue::Function { inferred_name, .. } = &mut initializer.value
        else {
            panic!("anonymous default function initializer changed kind");
        };
        assert!(inferred_name.take().is_some());
        let forged = module_with_link_initializers(module, initializers);
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link entry initializer is not canonical")
        );
    }

    #[test]
    fn anonymous_default_function_initializer_rejects_a_coordinated_wrong_name() {
        let module = compile_unlinked_module_with_filename(
            "export default function () {}",
            "forged-default-function-wrong-name.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let name = match module.link_initializers()[0].value {
            ModuleLinkInitializerValue::Function {
                inferred_name: Some(name),
                ..
            } => name,
            _ => panic!("anonymous default function lost its inferred name"),
        };
        let parts = module.into_parts();
        let mut function_parts = parts.function.into_parts();
        function_parts.constants[name as usize] =
            UnlinkedConstant::primitive(Value::String(JsString::from_static("not-default")))
                .unwrap();
        let forged = UnlinkedModule::new(
            parts.name,
            function_from_parts(function_parts),
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        );
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link entry initializer is not canonical")
        );
    }

    #[test]
    fn named_function_initializer_cannot_claim_default_name_inference() {
        let module = compile_unlinked_module_with_filename(
            "'default'; export default function named() {}",
            "forged-named-default-function.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let name = module
            .function()
            .constants()
            .iter()
            .position(|constant| {
                constant.as_primitive() == Some(&Value::String(JsString::from_static("default")))
            })
            .and_then(|index| u32::try_from(index).ok())
            .expect("root default String constant is missing");
        let mut initializers = module.link_initializers().to_vec();
        let [initializer] = initializers.as_mut_slice() else {
            panic!("named default function lost its initializer");
        };
        let ModuleLinkInitializerValue::Function { inferred_name, .. } = &mut initializer.value
        else {
            panic!("named default function initializer changed kind");
        };
        assert_eq!(*inferred_name, None);
        *inferred_name = Some(name);
        let forged = module_with_link_initializers(module, initializers);
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link entry initializer is not canonical")
        );
    }

    #[test]
    fn anonymous_default_function_initializer_authenticates_its_private_export_cell() {
        let module = compile_unlinked_module_with_filename(
            "export default function () {}",
            "forged-default-function-export.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let parts = module.into_parts();
        let mut exports = parts.exports.into_vec();
        exports[0].export_name = JsString::from_static("forged");
        let forged = UnlinkedModule::new(
            parts.name,
            parts.function,
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports,
                star_exports: parts.star_exports.into_vec(),
            },
        );
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link entry initializer is not canonical")
        );
    }

    #[test]
    fn anonymous_default_function_initializer_authenticates_its_private_descriptor() {
        let module = compile_unlinked_module_with_filename(
            "export default function () {}",
            "forged-default-function-descriptor.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let parts = module.into_parts();
        let mut function_parts = parts.function.into_parts();
        let forged_name = u32::try_from(function_parts.constants.len()).unwrap();
        function_parts.constants.push(
            UnlinkedConstant::primitive(Value::String(JsString::from_static("forged"))).unwrap(),
        );
        function_parts.closure_variables[0].name = ClosureVariableName::Constant(forged_name);
        let forged = UnlinkedModule::new(
            parts.name,
            function_from_parts(function_parts),
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        );
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link entry initializer is not canonical")
        );
    }

    #[test]
    fn anonymous_default_function_initializer_authenticates_its_anonymous_child() {
        let module = compile_unlinked_module_with_filename(
            "export default function () {}",
            "forged-default-function-child.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let constant = match module.link_initializers()[0].value {
            ModuleLinkInitializerValue::Function { constant, .. } => constant,
            _ => panic!("anonymous default function lost its initializer"),
        };
        let parts = module.into_parts();
        let mut function_parts = parts.function.into_parts();
        let original = std::mem::replace(
            &mut function_parts.constants[constant as usize],
            UnlinkedConstant::primitive(Value::Undefined).unwrap(),
        );
        let (_, _, child) = original.into_parts();
        function_parts.constants[constant as usize] = UnlinkedConstant::child(
            child
                .expect("default function initializer stopped referencing a child")
                .with_name(Some(JsString::from_static("forged"))),
        );
        let forged = UnlinkedModule::new(
            parts.name,
            function_from_parts(function_parts),
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        );
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link entry initializer is not canonical")
        );
    }

    #[test]
    fn named_default_function_initializer_cannot_impersonate_the_private_default_cell() {
        let module = compile_unlinked_module_with_filename(
            "export default function named() {}",
            "forged-named-default-private-cell.mjs",
            DebugInfoMode::StripDebug,
        )
        .unwrap();
        let (constant, closure_index) = match module.link_initializers()[0] {
            ModuleLinkInitializer {
                closure_index,
                value:
                    ModuleLinkInitializerValue::Function {
                        constant,
                        inferred_name: None,
                    },
            } => (constant, closure_index),
            _ => panic!("named default function lost its canonical initializer"),
        };
        let parts = module.into_parts();
        let mut function_parts = parts.function.into_parts();
        let original = std::mem::replace(
            &mut function_parts.constants[constant as usize],
            UnlinkedConstant::primitive(Value::Undefined).unwrap(),
        );
        let (_, _, child) = original.into_parts();
        let private_name = JsString::from_static(crate::module::MODULE_DEFAULT_BINDING_NAME);
        function_parts.constants[constant as usize] = UnlinkedConstant::child(
            child
                .expect("named default function stopped referencing a child")
                .with_name(Some(private_name.clone())),
        );
        let private_name_index = u32::try_from(function_parts.constants.len()).unwrap();
        function_parts
            .constants
            .push(UnlinkedConstant::primitive(Value::String(private_name)).unwrap());
        function_parts.closure_variables[usize::from(closure_index)].name =
            ClosureVariableName::Constant(private_name_index);
        let forged = UnlinkedModule::new(
            parts.name,
            function_from_parts(function_parts),
            parts.has_top_level_await,
            UnlinkedModuleTables {
                declaration_order: parts.declaration_order.into_vec(),
                link_initializers: parts.link_initializers.into_vec(),
                import_collisions: parts.import_collisions.into_vec(),
                requested_modules: parts.requested_modules.into_vec(),
                imports: parts.imports.into_vec(),
                exports: parts.exports.into_vec(),
                star_exports: parts.star_exports.into_vec(),
            },
        );
        let error = verify_unlinked_module_tree(&forged).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module link entry initializer is not canonical")
        );
    }

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
            variable_environment: EvalVariableEnvironment::StrictLocal(1),
            caller_strict: true,
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
                strict: true,
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
                    bindings: vec![EvalBinding {
                        name: JsString::from_static("<var>"),
                        source: EvalBindingSource::Local(1),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::EvalVariableObject,
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
            variable_environment: EvalVariableEnvironment::VariableObject {
                scope: 2,
                source: EvalBindingSource::Local(1),
            },
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        let mut code = vec![Instruction::VariableEnvironment, Instruction::PutLocal(1)];
        code.extend(eval_code(0, false));
        UnlinkedFunction::new(
            code,
            Vec::new(),
            FunctionMetadata {
                local_count: 2,
                eval_variable_object_local: Some(1),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![
                UnlinkedVariableDefinition {
                    name: Some(JsString::from_static(definition_name)),
                    is_lexical: false,
                    is_const: false,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::WithObject,
                },
                UnlinkedVariableDefinition {
                    name: Some(JsString::from_static("<var>")),
                    is_lexical: false,
                    is_const: false,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::EvalVariableObject,
                },
            ],
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

    fn empty_class_initializer(kind: ClassInitializerKind) -> UnlinkedFunction {
        UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                super_allowed: true,
                arguments_forbidden: true,
                needs_home_object: true,
                class_initializer_kind: Some(kind),
                ..FunctionMetadata::default()
            },
        )
    }

    fn script_installing_instance_initializer(child: UnlinkedFunction) -> UnlinkedFunction {
        UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::FClosure(0),
                Instruction::InstallClassInstanceInitializer,
                Instruction::Drop,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                max_stack: 3,
                ..FunctionMetadata::default()
            },
        )
    }

    fn script_running_static_initializer(child: UnlinkedFunction) -> UnlinkedFunction {
        UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::FClosure(0),
                Instruction::RunClassStaticInitializer,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                max_stack: 2,
                ..FunctionMetadata::default()
            },
        )
    }

    #[test]
    fn class_initializer_children_require_unique_matching_bridge_consumption() {
        verify_unlinked_tree(&script_installing_instance_initializer(
            empty_class_initializer(ClassInitializerKind::InstanceFields),
        ))
        .unwrap();

        let static_elements = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::CallClassStaticBlock,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::StaticBlock,
            ))],
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                super_allowed: true,
                arguments_forbidden: true,
                needs_home_object: true,
                class_initializer_kind: Some(ClassInitializerKind::StaticElements),
                ..FunctionMetadata::default()
            },
        );
        verify_unlinked_tree(&script_running_static_initializer(static_elements)).unwrap();

        let escaped = UnlinkedFunction::new(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::InstanceFields,
            ))],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        let error = verify_unlinked_tree(&escaped).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class initializer child escaped its matching bridge"),
            "{error}"
        );

        let repeated = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::FClosure(0),
                Instruction::InstallClassInstanceInitializer,
                Instruction::Drop,
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::FClosure(0),
                Instruction::InstallClassInstanceInitializer,
                Instruction::Drop,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::InstanceFields,
            ))],
            FunctionMetadata {
                max_stack: 3,
                ..FunctionMetadata::default()
            },
        );
        let error = verify_unlinked_tree(&repeated).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class initializer child did not have one unique closure site"),
            "{error}"
        );
    }

    #[test]
    fn class_initializer_bridges_reject_forged_roles_and_parents() {
        let wrong_role = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::FClosure(0),
                Instruction::RunClassStaticInitializer,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::InstanceFields,
            ))],
            FunctionMetadata {
                max_stack: 2,
                ..FunctionMetadata::default()
            },
        );
        let error = verify_unlinked_tree(&wrong_role).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class initializer bridge consumed a child with the wrong role"),
            "{error}"
        );

        let missing_child = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::InstallClassInstanceInitializer,
                Instruction::Drop,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                max_stack: 3,
                ..FunctionMetadata::default()
            },
        );
        let error = verify_unlinked_tree(&missing_child).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class initializer bridge did not consume an adjacent child closure"),
            "{error}"
        );

        let injected_bridge_operand = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::PushTrue,
                Instruction::Dup,
                Instruction::IfFalse(7),
                Instruction::Drop,
                Instruction::FClosure(0),
                Instruction::InstallClassInstanceInitializer,
                Instruction::Drop,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::InstanceFields,
            ))],
            FunctionMetadata {
                max_stack: 4,
                ..FunctionMetadata::default()
            },
        );
        let error = verify_unlinked_tree(&injected_bridge_operand).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class initializer closure/bridge pair has a non-fallthrough entry"),
            "{error}"
        );

        let wrong_static_parent = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::CallClassStaticBlock,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::StaticBlock,
            ))],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        let error = verify_unlinked_tree(&wrong_static_parent).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class static block call escaped its static-elements parent"),
            "{error}"
        );

        let root_initializer = empty_class_initializer(ClassInitializerKind::InstanceFields);
        let error = verify_unlinked_tree(&root_initializer).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class initializer bytecode escaped the class publication tree"),
            "{error}"
        );
    }

    #[test]
    fn class_initializer_pairs_reject_direct_control_flow_entry() {
        let instance = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::Goto(3),
                Instruction::FClosure(0),
                Instruction::InstallClassInstanceInitializer,
                Instruction::Drop,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::InstanceFields,
            ))],
            FunctionMetadata {
                max_stack: 3,
                ..FunctionMetadata::default()
            },
        );
        let error = verify_unlinked_tree(&instance).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class initializer closure/bridge pair has a non-fallthrough entry"),
            "{error}"
        );

        let static_initializer = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Goto(2),
                Instruction::FClosure(0),
                Instruction::RunClassStaticInitializer,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::StaticElements,
            ))],
            FunctionMetadata {
                max_stack: 2,
                ..FunctionMetadata::default()
            },
        );
        let error = verify_unlinked_tree(&static_initializer).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class initializer closure/bridge pair has a non-fallthrough entry"),
            "{error}"
        );

        let static_elements = UnlinkedFunction::new(
            vec![
                Instruction::Goto(1),
                Instruction::FClosure(0),
                Instruction::CallClassStaticBlock,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::StaticBlock,
            ))],
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                super_allowed: true,
                arguments_forbidden: true,
                needs_home_object: true,
                class_initializer_kind: Some(ClassInitializerKind::StaticElements),
                ..FunctionMetadata::default()
            },
        );
        let error =
            verify_unlinked_tree(&script_running_static_initializer(static_elements)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class initializer closure/bridge pair has a non-fallthrough entry"),
            "{error}"
        );
    }

    #[test]
    fn static_block_pair_rejects_a_crossing_backedge() {
        let static_elements = UnlinkedFunction::new(
            vec![
                Instruction::Nop,
                Instruction::FClosure(0),
                Instruction::CallClassStaticBlock,
                Instruction::PushFalse,
                Instruction::IfFalse(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(empty_class_initializer(
                ClassInitializerKind::StaticBlock,
            ))],
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                super_allowed: true,
                arguments_forbidden: true,
                needs_home_object: true,
                class_initializer_kind: Some(ClassInitializerKind::StaticElements),
                ..FunctionMetadata::default()
            },
        );
        let error =
            verify_unlinked_tree(&script_running_static_initializer(static_elements)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("class static block closure/bridge pair is reentrant"),
            "{error}"
        );
    }

    #[test]
    fn authored_class_initializers_inside_a_loop_remain_publishable() {
        let function = crate::compiler::compile_unlinked_script(
            "while (again) { class C { field = 1; static value = 2; static {} } }",
        )
        .unwrap();
        assert!(
            function
                .code()
                .iter()
                .enumerate()
                .any(|(source_pc, instruction)| {
                    explicit_control_flow_target(instruction)
                        .is_some_and(|target_pc| source_pc > target_pc)
                })
        );
        verify_unlinked_tree(&function).unwrap();
    }

    fn derived_this_initializer(
        this_source: ClosureSource,
        active_function_source: ClosureSource,
        new_target_source: ClosureSource,
    ) -> UnlinkedFunction {
        derived_this_initializer_with_code(
            this_source,
            active_function_source,
            new_target_source,
            vec![
                Instruction::GetVarRef(1),
                Instruction::GetSuper,
                Instruction::GetVarRef(2),
                Instruction::MarkSuperCall,
                Instruction::ConstructSuper(0),
                Instruction::Dup,
                Instruction::InitializeDerivedVarRef(0),
                Instruction::Return,
            ],
            2,
        )
    }

    fn derived_this_initializer_with_code(
        this_source: ClosureSource,
        active_function_source: ClosureSource,
        new_target_source: ClosureSource,
        code: Vec<Instruction>,
        max_stack: u16,
    ) -> UnlinkedFunction {
        UnlinkedFunction::new_with_closure_variables(
            code,
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<this>")))
                    .unwrap(),
                UnlinkedConstant::primitive(Value::String(JsString::from_static(
                    "<this_active_func>",
                )))
                .unwrap(),
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<new.target>")))
                    .unwrap(),
            ],
            FunctionMetadata {
                closure_count: 3,
                max_stack,
                strict: true,
                super_call_allowed: true,
                super_allowed: true,
                ..FunctionMetadata::default()
            },
            vec![
                ClosureVariable {
                    source: this_source,
                    name: ClosureVariableName::Constant(0),
                    is_lexical: true,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
                ClosureVariable {
                    source: active_function_source,
                    name: ClosureVariableName::Constant(1),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
                ClosureVariable {
                    source: new_target_source,
                    name: ClosureVariableName::Constant(2),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
            ],
        )
    }

    fn derived_parent(child: UnlinkedFunction) -> UnlinkedFunction {
        UnlinkedFunction::new(
            vec![
                Instruction::PushActiveFunction,
                Instruction::PutLocal(1),
                Instruction::PushNewTarget,
                Instruction::PutLocal(2),
                Instruction::CheckCtor,
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::ReturnDerived(0),
            ],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                local_count: 6,
                derived_this_local: Some(0),
                active_function_local: Some(1),
                max_stack: 1,
                strict: true,
                super_call_allowed: true,
                super_allowed: true,
                constructor_kind: ConstructorKind::Derived,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![
                UnlinkedVariableDefinition::lexical(Some(JsString::from_static("<this>")), false),
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static(
                    "<this_active_func>",
                ))),
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<new.target>"))),
                // A hand-authored unlinked tree can copy the sentinel spelling
                // onto an unrelated lexical; provenance must not follow it.
                UnlinkedVariableDefinition::lexical(Some(JsString::from_static("<this>")), false),
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static(
                    "<this_active_func>",
                ))),
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("<new.target>"))),
            ],
        )
    }

    #[test]
    fn derived_this_initializer_requires_authenticated_parent_lineage() {
        let no_capture_arrow = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                super_call_allowed: true,
                super_allowed: true,
                ..FunctionMetadata::default()
            },
        );
        verify_unlinked_tree(&script_with_child(derived_parent(no_capture_arrow))).unwrap();

        verify_unlinked_tree(&script_with_child(derived_parent(
            derived_this_initializer(
                ClosureSource::ParentLocal(0),
                ClosureSource::ParentLocal(1),
                ClosureSource::ParentLocal(2),
            ),
        )))
        .unwrap();

        let injected_pair = derived_this_initializer_with_code(
            ClosureSource::ParentLocal(0),
            ClosureSource::ParentLocal(1),
            ClosureSource::ParentLocal(2),
            vec![
                Instruction::PushTrue,
                Instruction::IfFalse(10),
                Instruction::GetVarRef(1),
                Instruction::GetSuper,
                Instruction::GetVarRef(2),
                Instruction::MarkSuperCall,
                Instruction::ConstructSuper(0),
                Instruction::Dup,
                Instruction::InitializeDerivedVarRef(0),
                Instruction::Return,
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::Goto(5),
            ],
            2,
        );
        assert!(
            verify_unlinked_tree(&script_with_child(derived_parent(injected_pair)))
                .unwrap_err()
                .to_string()
                .contains("super-call operand protocol has a non-fallthrough entry")
        );

        let forged = script_with_child(derived_parent(derived_this_initializer(
            ClosureSource::ParentLocal(3),
            ClosureSource::ParentLocal(1),
            ClosureSource::ParentLocal(2),
        )));
        assert!(
            verify_unlinked_tree(&forged)
                .unwrap_err()
                .to_string()
                .contains("did not originate from derived this")
        );

        let forged_active = script_with_child(derived_parent(derived_this_initializer(
            ClosureSource::ParentLocal(0),
            ClosureSource::ParentLocal(4),
            ClosureSource::ParentLocal(2),
        )));
        assert!(
            verify_unlinked_tree(&forged_active)
                .unwrap_err()
                .to_string()
                .contains("did not read the authenticated active function")
        );

        let forged_new_target = script_with_child(derived_parent(derived_this_initializer(
            ClosureSource::ParentLocal(0),
            ClosureSource::ParentLocal(1),
            ClosureSource::ParentLocal(5),
        )));
        assert!(
            verify_unlinked_tree(&forged_new_target)
                .unwrap_err()
                .to_string()
                .contains("did not read the authenticated new.target")
        );

        let relay = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::FClosure(3),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<this>")))
                    .unwrap(),
                UnlinkedConstant::primitive(Value::String(JsString::from_static(
                    "<this_active_func>",
                )))
                .unwrap(),
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<new.target>")))
                    .unwrap(),
                UnlinkedConstant::child(derived_this_initializer(
                    ClosureSource::ParentClosure(0),
                    ClosureSource::ParentClosure(1),
                    ClosureSource::ParentClosure(2),
                )),
            ],
            FunctionMetadata {
                closure_count: 3,
                max_stack: 1,
                strict: true,
                super_call_allowed: true,
                super_allowed: true,
                ..FunctionMetadata::default()
            },
            vec![
                ClosureVariable {
                    source: ClosureSource::ParentLocal(0),
                    name: ClosureVariableName::Constant(0),
                    is_lexical: true,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
                ClosureVariable {
                    source: ClosureSource::ParentLocal(1),
                    name: ClosureVariableName::Constant(1),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
                ClosureVariable {
                    source: ClosureSource::ParentLocal(2),
                    name: ClosureVariableName::Constant(2),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
            ],
        );
        verify_unlinked_tree(&script_with_child(derived_parent(relay))).unwrap();

        let eval_root = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::GetVarRef(1),
                Instruction::GetSuper,
                Instruction::GetVarRef(2),
                Instruction::MarkSuperCall,
                Instruction::ConstructSuper(0),
                Instruction::Dup,
                Instruction::InitializeDerivedVarRef(0),
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<this>")))
                    .unwrap(),
                UnlinkedConstant::primitive(Value::String(JsString::from_static(
                    "<this_active_func>",
                )))
                .unwrap(),
                UnlinkedConstant::primitive(Value::String(JsString::from_static("<new.target>")))
                    .unwrap(),
            ],
            FunctionMetadata {
                closure_count: 3,
                max_stack: 2,
                strict: true,
                super_call_allowed: true,
                super_allowed: true,
                eval_kind: EvalKind::Direct,
                ..FunctionMetadata::default()
            },
            vec![
                ClosureVariable {
                    source: ClosureSource::EvalEnvironment(0),
                    name: ClosureVariableName::Constant(0),
                    is_lexical: true,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
                ClosureVariable {
                    source: ClosureSource::EvalEnvironment(1),
                    name: ClosureVariableName::Constant(1),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
                ClosureVariable {
                    source: ClosureSource::EvalEnvironment(2),
                    name: ClosureVariableName::Constant(2),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                },
            ],
        );
        let derived_binding = EvalRootBinding {
            name: JsString::from_static("<this>"),
            scope: 0,
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        };
        let active_function_binding = EvalRootBinding {
            name: JsString::from_static("<this_active_func>"),
            scope: 0,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        };
        let new_target_binding = EvalRootBinding {
            name: JsString::from_static("<new.target>"),
            scope: 0,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        };
        let eval_bindings = [
            derived_binding.clone(),
            active_function_binding.clone(),
            new_target_binding.clone(),
        ];
        verify_unlinked_eval_tree(
            &eval_root,
            EvalKind::Direct,
            true,
            &eval_bindings,
            true,
            true,
        )
        .unwrap();

        let ordinary_binding = EvalRootBinding {
            is_lexical: false,
            ..derived_binding
        };
        let ordinary_bindings = [
            ordinary_binding,
            active_function_binding,
            new_target_binding,
        ];
        assert!(
            verify_unlinked_eval_tree(
                &eval_root,
                EvalKind::Direct,
                true,
                &ordinary_bindings,
                true,
                true,
            )
            .unwrap_err()
            .to_string()
            .contains("did not originate from derived this")
        );
    }

    #[test]
    fn class_instance_initializer_relay_requires_active_function_provenance() {
        let relay = |active_read| {
            UnlinkedFunction::new_with_closure_variables(
                vec![
                    Instruction::GetVarRef(1),
                    Instruction::GetSuper,
                    Instruction::GetVarRef(2),
                    Instruction::MarkSuperCall,
                    Instruction::ConstructSuper(0),
                    Instruction::Dup,
                    Instruction::InitializeDerivedVarRef(0),
                    Instruction::GetVarRef(active_read),
                    Instruction::CallClassInstanceInitializer,
                    Instruction::Return,
                ],
                vec![
                    UnlinkedConstant::primitive(Value::String(JsString::from_static("<this>")))
                        .unwrap(),
                    UnlinkedConstant::primitive(Value::String(JsString::from_static(
                        "<this_active_func>",
                    )))
                    .unwrap(),
                    UnlinkedConstant::primitive(Value::String(JsString::from_static(
                        "<new.target>",
                    )))
                    .unwrap(),
                ],
                FunctionMetadata {
                    closure_count: 4,
                    max_stack: 2,
                    strict: true,
                    super_call_allowed: true,
                    super_allowed: true,
                    ..FunctionMetadata::default()
                },
                vec![
                    ClosureVariable {
                        source: ClosureSource::ParentLocal(0),
                        name: ClosureVariableName::Constant(0),
                        is_lexical: true,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                    },
                    ClosureVariable {
                        source: ClosureSource::ParentLocal(1),
                        name: ClosureVariableName::Constant(1),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                    },
                    ClosureVariable {
                        source: ClosureSource::ParentLocal(2),
                        name: ClosureVariableName::Constant(2),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                    },
                    // This slot deliberately copies the sentinel spelling but
                    // originates from an unauthenticated ordinary parent local.
                    ClosureVariable {
                        source: ClosureSource::ParentLocal(4),
                        name: ClosureVariableName::Constant(1),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                    },
                ],
            )
        };

        verify_unlinked_tree(&script_with_child(derived_parent(relay(1)))).unwrap();

        let error = verify_unlinked_tree(&script_with_child(derived_parent(relay(3)))).unwrap_err();
        assert!(
            error.to_string().contains(
                "class instance initializer relay did not read the authenticated active function"
            ),
            "{error}"
        );
    }

    #[test]
    fn eval_root_rejects_shadowed_super_pseudo_binding_origins() {
        let root = |derived_this: u16, active_function: u16, new_target: u16| {
            let names = [
                ("<this>", true),
                ("<this_active_func>", false),
                ("<new.target>", false),
                ("<this>", true),
                ("<this_active_func>", false),
                ("<new.target>", false),
            ];
            let constants = names
                .iter()
                .map(|(name, _)| {
                    UnlinkedConstant::primitive(Value::String(JsString::from_static(name))).unwrap()
                })
                .collect::<Vec<_>>();
            let closure_variables = names
                .iter()
                .enumerate()
                .map(|(index, (_, is_lexical))| ClosureVariable {
                    source: ClosureSource::EvalEnvironment(
                        u16::try_from(index).expect("test closure index fits u16"),
                    ),
                    name: ClosureVariableName::Constant(
                        u32::try_from(index).expect("test constant index fits u32"),
                    ),
                    is_lexical: *is_lexical,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                })
                .collect::<Vec<_>>();
            UnlinkedFunction::new_with_closure_variables(
                vec![
                    Instruction::GetVarRef(active_function),
                    Instruction::GetSuper,
                    Instruction::GetVarRef(new_target),
                    Instruction::MarkSuperCall,
                    Instruction::ConstructSuper(0),
                    Instruction::Dup,
                    Instruction::InitializeDerivedVarRef(derived_this),
                    Instruction::Return,
                ],
                constants,
                FunctionMetadata {
                    closure_count: 6,
                    max_stack: 2,
                    strict: true,
                    super_call_allowed: true,
                    super_allowed: true,
                    eval_kind: EvalKind::Direct,
                    ..FunctionMetadata::default()
                },
                closure_variables,
            )
        };
        let expected_bindings = [
            eval_root_binding("<this>", 0, true, false, ClosureVariableKind::Normal),
            eval_root_binding(
                "<this_active_func>",
                0,
                false,
                false,
                ClosureVariableKind::Normal,
            ),
            eval_root_binding("<new.target>", 0, false, false, ClosureVariableKind::Normal),
            eval_root_binding("<this>", 1, true, false, ClosureVariableKind::Normal),
            eval_root_binding(
                "<this_active_func>",
                1,
                false,
                false,
                ClosureVariableKind::Normal,
            ),
            eval_root_binding("<new.target>", 1, false, false, ClosureVariableKind::Normal),
        ];

        verify_unlinked_eval_tree(
            &root(0, 1, 2),
            EvalKind::Direct,
            true,
            &expected_bindings,
            true,
            true,
        )
        .unwrap();

        for ((derived_this, active_function, new_target), expected_error) in [
            (
                (3, 1, 2),
                "captured derived initializer did not originate from derived this",
            ),
            (
                (0, 4, 2),
                "super-call marker did not read the authenticated active function",
            ),
            (
                (0, 1, 5),
                "super-call marker did not read the authenticated new.target",
            ),
        ] {
            let error = verify_unlinked_eval_tree(
                &root(derived_this, active_function, new_target),
                EvalKind::Direct,
                true,
                &expected_bindings,
                true,
                true,
            )
            .unwrap_err();
            assert!(error.to_string().contains(expected_error), "{error}");
        }
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
        let names = ["<var>", "<var>"];
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
            variable_environment: EvalVariableEnvironment::VariableObject {
                scope: 3,
                source: EvalBindingSource::Closure(variable_target),
            },
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
                "<var>",
                1,
                false,
                false,
                ClosureVariableKind::EvalVariableObject,
            ),
            eval_root_binding(
                "<var>",
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
    fn eval_root_authenticates_the_parameter_variable_object_role_and_target() {
        let expected = [eval_root_binding(
            "<arg_var>",
            0,
            false,
            false,
            ClosureVariableKind::ArgEvalVariableObject,
        )];
        let imported = eval_root_with_descriptors(
            EvalKind::Direct,
            vec![(
                ClosureSource::EvalEnvironment(0),
                "<arg_var>",
                false,
                false,
                ClosureVariableKind::ArgEvalVariableObject,
            )],
        );
        verify_unlinked_eval_tree(&imported, EvalKind::Direct, false, &expected, false, false)
            .unwrap();

        let wrong_role = EvalCallerProfile {
            scope_kinds: vec![EvalScopeKind::FunctionRoot].into_boxed_slice(),
            variable_target: EvalCallerVariableTarget::ExternalBinding(0),
        };
        assert!(
            verify_unlinked_eval_tree_with_profile(
                &imported,
                EvalKind::Direct,
                false,
                &expected,
                &wrong_role,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("variable target is not authenticated")
        );

        let wrong_sentinel = [eval_root_binding(
            "<var>",
            0,
            false,
            false,
            ClosureVariableKind::ArgEvalVariableObject,
        )];
        assert!(
            verify_unlinked_eval_tree(
                &imported,
                EvalKind::Direct,
                false,
                &wrong_sentinel,
                false,
                false,
            )
            .unwrap_err()
            .to_string()
            .contains("invalid binding metadata")
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

        let synthetic_metadata = FunctionMetadata {
            argument_count: 1,
            defined_argument_count: 1,
            pattern_argument_count: 1,
            parameter_pattern_end: Some(9),
            parameter_environment_local_count: 1,
            local_count: 4,
            max_stack: 2,
            ..FunctionMetadata::default()
        };
        let synthetic_layout = ParameterEnvironmentLayout {
            initialization_end: 9,
            argument_cells: Box::new([]),
            pattern_copies: vec![ParameterPatternCopy {
                parameter_local: 0,
                body_local: 2,
            }]
            .into_boxed_slice(),
            default_sources: Box::new([]),
            synthetic_arguments_local: Some(1),
            arg_eval_variable_object_local: None,
        };
        let synthetic_code = || {
            vec![
                Instruction::Arguments(crate::bytecode::ArgumentsKind::Unmapped),
                Instruction::Dup,
                Instruction::InitializeLocal(1),
                Instruction::PutLocal(3),
                Instruction::SetLocalUninitialized(0),
                Instruction::GetArg(0),
                Instruction::InitializeLocal(0),
                Instruction::GetLocalCheck(0),
                Instruction::PutLocal(2),
                Instruction::Nop,
                Instruction::Undefined,
                Instruction::Return,
            ]
        };
        let unnamed_arguments = [true];
        let lexical_locals = [true, true, false, false];
        let parameter_initializer_locals = [false; 4];
        validate_pattern_parameter_bytecode_layout(
            &synthetic_metadata,
            &synthetic_code(),
            &unnamed_arguments,
            &lexical_locals,
            &parameter_initializer_locals,
            Some(&synthetic_layout),
        )
        .unwrap();
        let mut checked_read = synthetic_code();
        checked_read[8] = Instruction::GetLocalCheck(1);
        validate_pattern_parameter_bytecode_layout(
            &synthetic_metadata,
            &checked_read,
            &unnamed_arguments,
            &lexical_locals,
            &parameter_initializer_locals,
            Some(&synthetic_layout),
        )
        .unwrap();
        for forged in [
            Instruction::SetLocalUninitialized(1),
            Instruction::CloseLocal(1),
            Instruction::InitializeLocal(1),
        ] {
            let mut code = synthetic_code();
            code[8] = forged;
            assert!(
                validate_pattern_parameter_bytecode_layout(
                    &synthetic_metadata,
                    &code,
                    &unnamed_arguments,
                    &lexical_locals,
                    &parameter_initializer_locals,
                    Some(&synthetic_layout),
                )
                .unwrap_err()
                .contains("body lexical local")
            );
        }

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

        for (code, body_pc) in [
            (
                vec![
                    Instruction::Undefined,
                    Instruction::InitializeLocal(0),
                    Instruction::Nop,
                ],
                3,
            ),
            (
                vec![Instruction::SetLocalUninitialized(0), Instruction::Nop],
                2,
            ),
        ] {
            assert_eq!(
                validate_parameter_initializer_scope_layout(
                    &FunctionMetadata {
                        local_count: 1,
                        ..FunctionMetadata::default()
                    },
                    &code,
                    Some(body_pc),
                    &[true],
                    &[true],
                ),
                Err("parameter-initializer local has no exact pre-boundary TDZ lifecycle")
            );
        }

        let initializer_capture = UnlinkedFunction::new(
            vec![
                Instruction::GetArg(0),
                Instruction::Drop,
                Instruction::SetLocalUninitialized(1),
                Instruction::Undefined,
                Instruction::InitializeLocal(1),
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::CloseLocal(1),
                Instruction::Nop,
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(capture_child(
                ClosureSource::ParentLocal(1),
                Some("initializer"),
                true,
            ))],
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                pattern_argument_count: 1,
                parameter_pattern_end: Some(8),
                local_count: 2,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(None)],
            vec![
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
                UnlinkedVariableDefinition::lexical(
                    Some(JsString::from_static("initializer")),
                    false,
                )
                .with_parameter_initializer(true),
            ],
        );
        let error = verify_unlinked_tree(&script_with_child(initializer_capture)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("function body closure captured a parameter-initializer local"),
            "{error}"
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

        let mut missing_default_only_argument = default_only_code();
        missing_default_only_argument[1] = Instruction::Undefined;
        assert!(
            verify_unlinked_tree(&script_with_child(make_default_only(
                missing_default_only_argument,
                Vec::new(),
                default_only_metadata(),
            )))
            .unwrap_err()
            .to_string()
            .contains("default cell has no exact argument selection")
        );

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
            variable_environment: EvalVariableEnvironment::StrictLocal(1),
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        };
        let plain_eval_descendant = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
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
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![eval_descendant_environment]);
        let parameter_parent = make_default_only(
            default_only_code(),
            vec![UnlinkedConstant::child(eval_descendant)],
            default_only_metadata(),
        );
        verify_unlinked_tree(&script_with_child(parameter_parent)).unwrap();

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
        let error = verify_unlinked_tree(&script_with_child(make(wrong_rest_target, metadata())))
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("parameter default cell has no exact argument selection"),
            "{error}",
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
                .contains("parameter default cell has no exact argument selection")
        );

        let mut wrong_default_target = code();
        wrong_default_target[6] = Instruction::IfFalse(12);
        assert!(
            verify_unlinked_tree(&script_with_child(make(wrong_default_target, metadata())))
                .unwrap_err()
                .to_string()
                .contains("parameter default cell has no exact argument selection")
        );

        let mut missing_default_sync = code();
        missing_default_sync[10] = Instruction::Drop;
        assert!(
            verify_unlinked_tree(&script_with_child(make(missing_default_sync, metadata())))
                .unwrap_err()
                .to_string()
                .contains("parameter default cell has no exact argument selection")
        );

        let mut wrong_default_argument = code();
        wrong_default_argument[10] = Instruction::PutArg(1);
        assert!(
            verify_unlinked_tree(&script_with_child(make(wrong_default_argument, metadata())))
                .unwrap_err()
                .to_string()
                .contains("parameter default cell has no exact argument selection")
        );

        let mut raw_argument_rhs = code();
        raw_argument_rhs[8] = Instruction::GetArg(1);
        assert!(
            verify_unlinked_tree(&script_with_child(make(raw_argument_rhs, metadata())))
                .unwrap_err()
                .to_string()
                .contains("parameter rest cell has no exact initialization")
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
            .contains("body closure captured a parameter-initializer cell")
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
                .contains("environment operand is out of bounds")
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
    fn eval_environments_cannot_cross_a_binding_pattern_body_boundary() {
        let initializer_name = JsString::from_static("initializer");
        let initializer_binding = EvalBinding {
            name: initializer_name.clone(),
            source: EvalBindingSource::Local(1),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        };
        let body_eval = UnlinkedFunction::new(
            vec![
                Instruction::GetArg(0),
                Instruction::Drop,
                Instruction::SetLocalUninitialized(1),
                Instruction::Undefined,
                Instruction::InitializeLocal(1),
                Instruction::CloseLocal(1),
                Instruction::Nop,
                Instruction::Undefined,
                Instruction::Eval {
                    argument_count: 0,
                    environment: 0,
                },
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                pattern_argument_count: 1,
                parameter_pattern_end: Some(6),
                local_count: 2,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(None)],
            vec![
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
                UnlinkedVariableDefinition::lexical(Some(initializer_name), false)
                    .with_parameter_initializer(true),
            ],
        )
        .with_eval_environments(vec![ordinary_environment(Some(initializer_binding))]);
        let error = verify_unlinked_tree(&script_with_child(body_eval)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("function body eval captured a parameter-initializer local"),
            "{error}"
        );

        let body_name = JsString::from_static("body");
        let body_binding = EvalBinding {
            name: body_name.clone(),
            source: EvalBindingSource::Local(1),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
            is_catch_parameter: false,
        };
        let initializer_apply_eval = UnlinkedFunction::new(
            vec![
                Instruction::GetArg(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::ApplyEval { environment: 0 },
                Instruction::Drop,
                Instruction::Nop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                pattern_argument_count: 1,
                parameter_pattern_end: Some(6),
                local_count: 2,
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            vec![UnlinkedVariableDefinition::ordinary(None)],
            vec![
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("value"))),
                UnlinkedVariableDefinition::lexical(Some(body_name), false),
            ],
        )
        .with_eval_environments(vec![ordinary_environment(Some(body_binding))]);
        let error = verify_unlinked_tree(&script_with_child(initializer_apply_eval)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("pattern initializer eval captured a body lexical local"),
            "{error}"
        );
    }

    #[test]
    fn argument_definition_cannot_claim_parameter_initializer_provenance() {
        let function = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            vec![
                UnlinkedVariableDefinition::ordinary(Some(JsString::from_static("argument")))
                    .with_parameter_initializer(true),
            ],
            Vec::new(),
        );

        let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("argument definition is not an ordinary mutable binding"),
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
                strict: true,
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
                strict: true,
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

        let spread = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::ApplyEval { environment: 1 },
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![ordinary_environment(None)]);
        let error = verify_unlinked_tree(&script_with_child(spread)).unwrap_err();
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
            variable_environment: EvalVariableEnvironment::StrictLocal(2),
            caller_strict: true,
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
    fn eval_variable_object_target_cannot_forge_an_ancestor_function_anchor() {
        let variable_name = JsString::from_static("<var>");
        let current_binding = EvalBinding {
            name: variable_name.clone(),
            source: EvalBindingSource::Local(0),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::EvalVariableObject,
            is_catch_parameter: false,
        };
        let ancestor_binding = EvalBinding {
            source: EvalBindingSource::Closure(0),
            ..current_binding.clone()
        };
        let environment = EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: vec![current_binding].into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: vec![ancestor_binding].into_boxed_slice(),
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
            variable_environment: EvalVariableEnvironment::VariableObject {
                scope: 3,
                source: EvalBindingSource::Closure(0),
            },
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        let mut inner_code = vec![Instruction::VariableEnvironment, Instruction::PutLocal(0)];
        inner_code.extend(eval_code(0, false));
        let inner = UnlinkedFunction::new_with_closure_variables(
            inner_code,
            vec![UnlinkedConstant::primitive(Value::String(variable_name.clone())).unwrap()],
            FunctionMetadata {
                local_count: 1,
                closure_count: 1,
                eval_variable_object_local: Some(0),
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
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition {
                name: Some(variable_name.clone()),
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::EvalVariableObject,
            }],
        )
        .with_eval_environments(vec![environment]);
        let outer = UnlinkedFunction::new(
            vec![
                Instruction::VariableEnvironment,
                Instruction::PutLocal(0),
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(inner)],
            FunctionMetadata {
                local_count: 1,
                eval_variable_object_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition {
                name: Some(variable_name),
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::EvalVariableObject,
            }],
        );

        let error = verify_unlinked_tree(&script_with_child(outer)).unwrap_err();
        assert!(
            error.to_string().contains("wrong current function segment"),
            "{error}"
        );
    }

    #[test]
    fn eval_variable_object_target_cannot_forge_an_ancestor_parameter_anchor() {
        let body_name = JsString::from_static("<var>");
        let parameter_name = JsString::from_static("<arg_var>");
        let environment = EvalEnvironment {
            scopes: vec![
                EvalScope {
                    kind: EvalScopeKind::FunctionBody,
                    bindings: Box::new([]),
                },
                EvalScope {
                    kind: EvalScopeKind::FunctionRoot,
                    bindings: vec![EvalBinding {
                        name: body_name.clone(),
                        source: EvalBindingSource::Local(0),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::EvalVariableObject,
                        is_catch_parameter: false,
                    }]
                    .into_boxed_slice(),
                },
                EvalScope {
                    kind: EvalScopeKind::Parameter,
                    bindings: vec![EvalBinding {
                        name: parameter_name.clone(),
                        source: EvalBindingSource::Closure(0),
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::ArgEvalVariableObject,
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
            variable_environment: EvalVariableEnvironment::VariableObject {
                scope: 2,
                source: EvalBindingSource::Closure(0),
            },
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };
        let mut inner_code = vec![Instruction::VariableEnvironment, Instruction::PutLocal(0)];
        inner_code.extend(eval_code(0, false));
        let inner = UnlinkedFunction::new_with_closure_variables(
            inner_code,
            vec![UnlinkedConstant::primitive(Value::String(parameter_name.clone())).unwrap()],
            FunctionMetadata {
                local_count: 1,
                closure_count: 1,
                eval_variable_object_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(1),
                name: ClosureVariableName::Constant(0),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::ArgEvalVariableObject,
            }],
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition {
                name: Some(body_name.clone()),
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::EvalVariableObject,
            }],
        )
        .with_eval_environments(vec![environment]);
        let outer = UnlinkedFunction::new(
            vec![
                Instruction::VariableEnvironment,
                Instruction::PutLocal(0),
                Instruction::VariableEnvironment,
                Instruction::PutLocal(1),
                Instruction::Nop,
                Instruction::FClosure(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(inner)],
            FunctionMetadata {
                local_count: 2,
                eval_variable_object_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_parameter_environment(Some(ParameterEnvironmentLayout {
            initialization_end: 4,
            argument_cells: Box::new([]),
            pattern_copies: Box::new([]),
            default_sources: Box::new([]),
            synthetic_arguments_local: None,
            arg_eval_variable_object_local: Some(1),
        }))
        .with_variable_definitions(
            Vec::new(),
            vec![
                UnlinkedVariableDefinition {
                    name: Some(body_name),
                    is_lexical: false,
                    is_const: false,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::EvalVariableObject,
                },
                UnlinkedVariableDefinition {
                    name: Some(parameter_name),
                    is_lexical: false,
                    is_const: false,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::ArgEvalVariableObject,
                },
            ],
        );

        let error = verify_unlinked_tree(&script_with_child(outer)).unwrap_err();
        assert!(
            error.to_string().contains("wrong current function segment"),
            "{error}"
        );
    }

    #[test]
    fn strict_script_global_eval_anchor_does_not_leak_to_strict_functions() {
        let strict_script = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
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
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        }]);
        verify_unlinked_tree(&strict_script).unwrap();

        let mut non_canonical_script_environment = strict_script.eval_environments()[0].clone();
        non_canonical_script_environment.variable_environment =
            EvalVariableEnvironment::StrictLocal(1);
        let non_canonical_script = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![non_canonical_script_environment.clone()]);
        let error = verify_unlinked_tree(&non_canonical_script).unwrap_err();
        assert!(
            error.to_string().contains("non-canonical strict-local"),
            "{error}"
        );

        let synthetic_strict_eval = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                eval_kind: EvalKind::Direct,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![non_canonical_script_environment]);
        verify_unlinked_eval_tree(
            &synthetic_strict_eval,
            EvalKind::Direct,
            false,
            &[],
            false,
            false,
        )
        .unwrap();

        let mut forged_function = ordinary_environment(None);
        forged_function.variable_environment = EvalVariableEnvironment::Global;
        let function = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![forged_function]);
        let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("escaped an authored Script root"),
            "{error}"
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
                strict: true,
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
                strict: true,
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
                strict: true,
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
            variable_environment: EvalVariableEnvironment::StrictLocal(1),
            caller_strict: true,
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
                strict: true,
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
            variable_environment: EvalVariableEnvironment::StrictLocal(1),
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        };
        let eval_child = UnlinkedFunction::new_with_closure_variables(
            eval_code(0, false),
            vec![UnlinkedConstant::primitive(Value::String(name.clone())).unwrap()],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                strict: true,
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
        environment.variable_environment = EvalVariableEnvironment::StrictLocal(0);
        let function = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![environment]);

        let error = verify_unlinked_tree(&script_with_child(function)).unwrap_err();
        assert!(error.to_string().contains("wrong function anchor"));
    }

    #[test]
    fn eval_variable_environment_matches_publication_tree_topology() {
        let root_with_function_scope = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
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
            variable_environment: EvalVariableEnvironment::StrictLocal(1),
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        };
        let function = UnlinkedFunction::new(
            eval_code(0, false),
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .with_eval_environments(vec![environment]);

        verify_unlinked_tree(&script_with_child(function)).unwrap();
    }
}
