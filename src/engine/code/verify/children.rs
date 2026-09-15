//! Authenticate child capture sources and enqueue the same iterative tree walk.
use super::{
    PublicationClosureOrigins, PublicationWorkItem, function_name_view_matches_origin,
    is_erased_function_name_view, unlinked_closure_name, verify_capture_flags,
};
use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::function::UnlinkedFunction;
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariableKind, ConstructorKind,
};
use std::collections::{HashMap, HashSet};

pub(super) struct ChildPublicationInputs<'a, 's> {
    pub function: &'a UnlinkedFunction,
    pub function_id: usize,
    pub authenticated_active_function_local: Option<u16>,
    pub authenticated_new_target_local: Option<u16>,
    pub function_depth: usize,
    pub child_closure_pcs: &'s [Vec<usize>],
    pub parameter_body_pc: Option<usize>,
    pub pattern_body_pc: Option<usize>,
    pub parameter_initializer_capture_locals: Option<&'s [bool]>,
    pub function_name_origins: &'s [Option<bool>],
    pub derived_this_origins: &'s [bool],
    pub active_function_origins: &'s [bool],
    pub new_target_origins: &'s [bool],
    pub closure_origins: &'s [Option<u16>],
}

pub(super) struct ChildPublicationQueue<'a, 's> {
    pub pending: &'s mut Vec<PublicationWorkItem<'a>>,
    pub next_function_id: &'s mut usize,
    pub erased_parent_by_child: &'s mut HashMap<(usize, u16), (usize, u16)>,
}

pub(super) fn verify_and_enqueue<'a>(
    inputs: ChildPublicationInputs<'a, '_>,
    queue: ChildPublicationQueue<'a, '_>,
) -> Result<(), RuntimeError> {
    let ChildPublicationInputs {
        function,
        function_id,
        authenticated_active_function_local,
        authenticated_new_target_local,
        function_depth,
        child_closure_pcs,
        parameter_body_pc,
        pattern_body_pc,
        parameter_initializer_capture_locals,
        function_name_origins,
        derived_this_origins,
        active_function_origins,
        new_target_origins,
        closure_origins,
    } = inputs;
    let ChildPublicationQueue {
        pending,
        next_function_id,
        erased_parent_by_child,
    } = queue;
    let mut local_flags = vec![None; usize::from(function.metadata().local_count)];
    let mut argument_flags = vec![None; usize::from(function.metadata().argument_count)];
    for (constant_index, constant) in function.constants().iter().enumerate() {
        if let Some(child) = constant.as_child() {
            let closure_pcs = &child_closure_pcs[constant_index];
            let child_id = *next_function_id;
            *next_function_id = next_function_id.checked_add(1).ok_or_else(|| {
                RuntimeError::Engine(Error::internal("function publication identity overflowed"))
            })?;
            let mut child_function_name_origins =
                Vec::with_capacity(child.closure_variables().len());
            let mut child_derived_this_origins =
                Vec::with_capacity(child.closure_variables().len());
            let mut child_active_function_origins =
                Vec::with_capacity(child.closure_variables().len());
            let mut child_new_target_origins = Vec::with_capacity(child.closure_variables().len());
            let mut child_physical_sources = HashSet::new();
            for (descriptor_index, descriptor) in child.closure_variables().iter().enumerate() {
                if let (Some(body_pc), Some(initializer_capture_locals)) =
                    (parameter_body_pc, &parameter_initializer_capture_locals)
                {
                    let instantiated_in_initializer = closure_pcs.iter().any(|pc| *pc < body_pc);
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
                                    < function.metadata().parameter_environment_local_count
                                    || function.parameter_environment().is_some_and(
                                        |layout| layout.synthetic_arguments_local == Some(index),
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
                                    >= function.metadata().parameter_environment_local_count
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
                        let slot = local_flags.get_mut(usize::from(index)).ok_or_else(|| {
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
                        derived_this_origin = function.metadata().derived_this_local == Some(index);
                        active_function_origin = authenticated_active_function_local == Some(index);
                        new_target_origin = authenticated_new_target_local == Some(index);
                    }
                    ClosureSource::ParentArgument(index) => {
                        let slot = argument_flags.get_mut(usize::from(index)).ok_or_else(|| {
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
                        if flags != (definition.is_lexical, definition.is_const, definition.kind) {
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
                                .insert((child_id, descriptor_index), (function_id, parent_index))
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
            pending.push(PublicationWorkItem {
                function: child,
                depth: child_depth,
                origins: PublicationClosureOrigins {
                    eval_bindings: child_origins,
                    function_names: child_function_name_origins,
                    derived_this: child_derived_this_origins,
                    active_function: child_active_function_origins,
                    new_target: child_new_target_origins,
                },
                id: child_id,
            });
        } else if constant.as_primitive().is_none()
            && constant.as_regexp().is_none()
            && constant.as_template_object().is_none()
        {
            return Err(RuntimeError::Invariant(
                "unlinked constant did not contain exactly one payload",
            ));
        }
    }
    Ok(())
}
