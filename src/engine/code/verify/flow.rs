//! Entry protocols, branch targets and class initializer edges.
use super::{closures::GlobalDeclarations, pseudo_binding_entry};
use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::bytecode::verify_parts;
use crate::engine::code::function::metadata::{ClassInitializerKind, ClosureVariableKind};
use crate::engine::code::function::{UnlinkedConstant, UnlinkedFunction};
use std::collections::{HashMap, HashSet};

pub(super) struct PublicationFlow {
    pub global_function_initializer_pcs: HashMap<usize, u16>,
    pub authenticated_active_function_local: Option<u16>,
    pub authenticated_new_target_local: Option<u16>,
    pub masked_lexical_initializer_targets: Vec<usize>,
    pub child_closure_pcs: Vec<Vec<usize>>,
}

pub(super) fn verify(
    function: &UnlinkedFunction,
    declarations: GlobalDeclarations,
    active_function_origins: &[bool],
    new_target_origins: &[bool],
) -> Result<PublicationFlow, RuntimeError> {
    let GlobalDeclarations {
        global_declaration_names,
        first_global_declaration_indices,
        global_function_declarations,
    } = declarations;
    let mut global_function_initializer_pcs = HashMap::new();
    let mut global_function_prologue_offset = 0_usize;
    let mut pseudo_rank = 0_u8;
    let mut pseudo_targets = Vec::with_capacity(4);
    let mut authenticated_active_function_local = None;
    let mut authenticated_new_target_local = None;
    while let Some(
        [
            source,
            crate::engine::code::bytecode::Instruction::PutLocal(local),
        ],
    ) = function
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
            crate::engine::code::bytecode::Instruction::PushActiveFunction => {
                authenticated_active_function_local = Some(*local);
            }
            crate::engine::code::bytecode::Instruction::PushNewTarget => {
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
            crate::engine::code::bytecode::Instruction::CallClassInstanceInitializer
        ) && let Some(crate::engine::code::bytecode::Instruction::GetVarRef(index)) = pc
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
        if !matches!(
            instruction,
            crate::engine::code::bytecode::Instruction::MarkSuperCall
        ) {
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
            Some(crate::engine::code::bytecode::Instruction::GetSuper)
        ) {
            return Err(RuntimeError::Engine(Error::internal(
                "super-call marker is not preceded by GetSuper",
            )));
        }
        let active_read_is_authenticated = match function.code().get(active_pc) {
            Some(crate::engine::code::bytecode::Instruction::GetLocal(index)) => {
                authenticated_active_function_local == Some(*index)
            }
            Some(crate::engine::code::bytecode::Instruction::GetVarRef(index)) => {
                active_function_origins
                    .get(usize::from(*index))
                    .copied()
                    .unwrap_or(false)
            }
            _ => false,
        };
        if !active_read_is_authenticated {
            return Err(RuntimeError::Engine(Error::internal(
                "super-call marker did not read the authenticated active function",
            )));
        }
        let new_target_read_is_authenticated = match function.code().get(pc - 1) {
            Some(crate::engine::code::bytecode::Instruction::GetLocal(index)) => {
                authenticated_new_target_local == Some(*index)
            }
            Some(crate::engine::code::bytecode::Instruction::GetVarRef(index)) => {
                new_target_origins
                    .get(usize::from(*index))
                    .copied()
                    .unwrap_or(false)
            }
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
        Some(crate::engine::code::bytecode::Instruction::ThrowRedeclaration(_))
    ));
    for (ordinal, name) in global_function_declarations.iter().enumerate() {
        let closure_pc = ordinal
            .checked_mul(2)
            .and_then(|pc| pc.checked_add(global_function_prologue_offset))
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal("global function prologue is too large"))
            })?;
        let initializer_pc = closure_pc + 1;
        let Some(crate::engine::code::bytecode::Instruction::FClosure(constant)) =
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
            Some(crate::engine::code::bytecode::Instruction::PutVarInit(target))
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
        let crate::engine::code::bytecode::Instruction::FClosure(index) = instruction else {
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

    Ok(PublicationFlow {
        global_function_initializer_pcs,
        authenticated_active_function_local,
        authenticated_new_target_local,
        masked_lexical_initializer_targets,
        child_closure_pcs,
    })
}

fn class_initializer_bridge_kind(
    instruction: &crate::engine::code::bytecode::Instruction,
) -> Option<ClassInitializerKind> {
    match instruction {
        crate::engine::code::bytecode::Instruction::InstallClassInstanceInitializer => {
            Some(ClassInitializerKind::InstanceFields)
        }
        crate::engine::code::bytecode::Instruction::RunClassStaticInitializer => {
            Some(ClassInitializerKind::StaticElements)
        }
        crate::engine::code::bytecode::Instruction::CallClassStaticBlock => {
            Some(ClassInitializerKind::StaticBlock)
        }
        _ => None,
    }
}

pub(super) fn explicit_control_flow_target(
    instruction: &crate::engine::code::bytecode::Instruction,
) -> Option<usize> {
    let target = instruction.control_effect().target()?;
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
        let Some(crate::engine::code::bytecode::Instruction::FClosure(constant)) =
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
