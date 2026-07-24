//! Publication-time authentication for class-private bytecode.
//!
//! Private names live in lexical cells, but they are not ECMAScript Values.
//! This verifier keeps ordinary local/VarRef instructions from turning those
//! cells into a public symbol channel and authenticates every typed private
//! operand against compiler-retained binding metadata.

use super::*;
use crate::bytecode::{Instruction, PrivateNameSource};
use crate::heap::{PublishedPrivateBinding, PublishedPrivateBindings, VariableDefinition};
use std::collections::HashMap;

fn is_private_source_name(name: &JsString) -> bool {
    name.utf16_units().next() == Some(u16::from(b'#'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivateBindingRole {
    Primary,
    SetterStorage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PrivateBindingInfo {
    kind: ClosureVariableKind,
    role: PrivateBindingRole,
}

/// Name-aware publication data retained across atom linking. The unlinked
/// verifier has exact source spellings, while the heap deliberately sees only
/// atoms, so setter-primary/storage roles and local pair identities must be
/// sealed at this boundary rather than reconstructed later from `kind`.
pub(in crate::runtime) struct PrivateBindingPublicationPlan {
    local_roles: Vec<Option<PrivateBindingRole>>,
    local_pairs: Vec<Option<u16>>,
    closure_roles: Vec<Option<PrivateBindingRole>>,
    has_private_bindings: bool,
}

fn setter_storage_base(name: &JsString) -> Option<Vec<u16>> {
    let units = name.utf16_units().collect::<Vec<_>>();
    let suffix = "<set>".encode_utf16().collect::<Vec<_>>();
    let base = units.strip_suffix(suffix.as_slice())?;
    (base.len() > 1 && base.first() == Some(&u16::from(b'#'))).then(|| base.to_vec())
}

fn private_binding_info(kind: ClosureVariableKind, name: &JsString) -> Option<PrivateBindingInfo> {
    if !is_private_source_name(name) {
        return None;
    }
    let role = if setter_storage_base(name).is_some() {
        PrivateBindingRole::SetterStorage
    } else {
        PrivateBindingRole::Primary
    };
    if role == PrivateBindingRole::SetterStorage && kind != ClosureVariableKind::PrivateSetter {
        return None;
    }
    Some(PrivateBindingInfo { kind, role })
}

fn private_setter_local_pairs(
    local_definitions: &[UnlinkedVariableDefinition],
) -> Result<Vec<Option<u16>>, RuntimeError> {
    let mut pairs = vec![None; local_definitions.len()];
    let mut unmatched_primaries = HashMap::<Vec<u16>, Vec<u16>>::new();
    for (index, definition) in local_definitions.iter().enumerate() {
        let Some(name) = definition.name.as_ref() else {
            continue;
        };
        let Some(info) = private_binding_info(definition.kind, name) else {
            continue;
        };
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "private-name local index exceeds bytecode range",
            ))
        })?;
        match info {
            PrivateBindingInfo {
                kind: ClosureVariableKind::PrivateSetter | ClosureVariableKind::PrivateGetterSetter,
                role: PrivateBindingRole::Primary,
            } => unmatched_primaries
                .entry(name.utf16_units().collect())
                .or_default()
                .push(index),
            PrivateBindingInfo {
                kind: ClosureVariableKind::PrivateSetter,
                role: PrivateBindingRole::SetterStorage,
            } => {
                let Some(base) = setter_storage_base(name) else {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private setter primary/storage cells are not paired",
                    )));
                };
                let Some(primary) = unmatched_primaries.get_mut(&base).and_then(Vec::pop) else {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private setter primary/storage cells are not paired",
                    )));
                };
                pairs[usize::from(primary)] = Some(index);
                pairs[usize::from(index)] = Some(primary);
            }
            _ => {}
        }
    }
    if unmatched_primaries
        .values()
        .any(|primaries| !primaries.is_empty())
    {
        return Err(RuntimeError::Engine(Error::internal(
            "private setter primary/storage cells are not paired",
        )));
    }
    Ok(pairs)
}

pub(in crate::runtime) fn prepare_private_binding_publication(
    local_definitions: &[UnlinkedVariableDefinition],
    closure_variables: &[ClosureVariable],
    constants: &[BytecodeConstant],
) -> Result<PrivateBindingPublicationPlan, RuntimeError> {
    let mut local_roles = vec![None; local_definitions.len()];
    let local_pairs = private_setter_local_pairs(local_definitions)?;
    let mut has_private_bindings = false;

    for (index, definition) in local_definitions.iter().enumerate() {
        if !definition.kind.is_private() {
            continue;
        }
        has_private_bindings = true;
        let name = definition.name.as_ref().ok_or(RuntimeError::Invariant(
            "verified private local lost its source name before publication",
        ))?;
        let info = private_binding_info(definition.kind, name).ok_or(RuntimeError::Invariant(
            "verified private local lost its authenticated role before publication",
        ))?;
        local_roles[index] = Some(info.role);
    }

    let mut closure_roles = vec![None; closure_variables.len()];
    for (index, descriptor) in closure_variables.iter().enumerate() {
        if !descriptor.kind.is_private() {
            continue;
        }
        has_private_bindings = true;
        let ClosureVariableName::Constant(constant) = descriptor.name else {
            return Err(RuntimeError::Invariant(
                "verified private closure lost its unlinked source name",
            ));
        };
        let name = usize::try_from(constant)
            .ok()
            .and_then(|constant| constants.get(constant))
            .and_then(|constant| match constant {
                BytecodeConstant::Value(RawValue::String(name)) => Some(name),
                BytecodeConstant::Value(_)
                | BytecodeConstant::RegExp { .. }
                | BytecodeConstant::Function(_) => None,
            })
            .ok_or(RuntimeError::Invariant(
                "verified private closure source name was not a string constant",
            ))?;
        let info = private_binding_info(descriptor.kind, name).ok_or(RuntimeError::Invariant(
            "verified private closure lost its authenticated role before publication",
        ))?;
        closure_roles[index] = Some(info.role);
    }

    Ok(PrivateBindingPublicationPlan {
        local_roles,
        local_pairs,
        closure_roles,
        has_private_bindings,
    })
}

impl PrivateBindingPublicationPlan {
    pub(in crate::runtime) fn authenticate(
        self,
        local_definitions: &[VariableDefinition],
        closure_variables: &[ClosureVariable],
    ) -> Result<PublishedPrivateBindings, RuntimeError> {
        if self.local_roles.len() != local_definitions.len()
            || self.local_pairs.len() != local_definitions.len()
            || self.closure_roles.len() != closure_variables.len()
        {
            return Err(RuntimeError::Invariant(
                "private binding publication plan no longer matches linked metadata",
            ));
        }
        if !self.has_private_bindings {
            return Ok(PublishedPrivateBindings::none());
        }

        let locals = self
            .local_roles
            .into_iter()
            .zip(self.local_pairs)
            .zip(local_definitions)
            .map(|((role, pair), definition)| {
                role.map(|role| {
                    let name = definition.name.ok_or(RuntimeError::Invariant(
                        "linked private local lost its atom name",
                    ))?;
                    Ok(match role {
                        PrivateBindingRole::Primary => PublishedPrivateBinding::primary(name, pair),
                        PrivateBindingRole::SetterStorage => {
                            PublishedPrivateBinding::setter_storage(name, pair)
                        }
                    })
                })
                .transpose()
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let closures = self
            .closure_roles
            .into_iter()
            .zip(closure_variables)
            .map(|(role, descriptor)| {
                role.map(|role| {
                    let ClosureVariableName::Atom(name) = descriptor.name else {
                        return Err(RuntimeError::Invariant(
                            "linked private closure lost its atom name",
                        ));
                    };
                    Ok(match role {
                        PrivateBindingRole::Primary => PublishedPrivateBinding::primary(name, None),
                        PrivateBindingRole::SetterStorage => {
                            PublishedPrivateBinding::setter_storage(name, None)
                        }
                    })
                })
                .transpose()
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;

        Ok(PublishedPrivateBindings::authenticated(locals, closures))
    }
}

fn verify_private_local(
    function: &UnlinkedFunction,
    index: u16,
) -> Result<PrivateBindingInfo, RuntimeError> {
    let definition = function
        .local_definitions()
        .get(usize::from(index))
        .ok_or_else(|| {
            RuntimeError::Engine(Error::internal(
                "private-name local operand is out of bounds",
            ))
        })?;
    let binding = definition
        .name
        .as_ref()
        .and_then(|name| private_binding_info(definition.kind, name));
    if !matches!(
        definition.kind,
        ClosureVariableKind::PrivateField
            | ClosureVariableKind::PrivateMethod
            | ClosureVariableKind::PrivateGetter
            | ClosureVariableKind::PrivateSetter
            | ClosureVariableKind::PrivateGetterSetter
    ) || !definition.is_lexical
        || !definition.is_const
        || definition.is_parameter_initializer
        || binding.is_none()
    {
        return Err(RuntimeError::Engine(Error::internal(
            "private-name local is not an authenticated immutable lexical binding",
        )));
    }
    binding.ok_or_else(|| {
        RuntimeError::Engine(Error::internal(
            "private-name local lost its authenticated binding role",
        ))
    })
}

fn verify_private_closure(
    function: &UnlinkedFunction,
    index: u16,
) -> Result<PrivateBindingInfo, RuntimeError> {
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
    let binding = unlinked_closure_name(function, descriptor)?
        .and_then(|name| private_binding_info(descriptor.kind, name));
    if !matches!(
        descriptor.kind,
        ClosureVariableKind::PrivateField
            | ClosureVariableKind::PrivateMethod
            | ClosureVariableKind::PrivateGetter
            | ClosureVariableKind::PrivateSetter
            | ClosureVariableKind::PrivateGetterSetter
    ) || !descriptor.is_lexical
        || !descriptor.is_const
        || !valid_source
        || binding.is_none()
    {
        return Err(RuntimeError::Engine(Error::internal(
            "private-name closure is not an authenticated immutable lexical binding",
        )));
    }
    binding.ok_or_else(|| {
        RuntimeError::Engine(Error::internal(
            "private-name closure lost its authenticated binding role",
        ))
    })
}

fn verify_private_source(
    function: &UnlinkedFunction,
    source: PrivateNameSource,
) -> Result<PrivateBindingInfo, RuntimeError> {
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

fn verify_private_callable_initializer(
    function: &UnlinkedFunction,
    pc: usize,
    binding_index: u16,
    accessor_role: Option<PrivateBindingRole>,
    explicit_control_flow_targets: &HashSet<usize>,
    label: &'static str,
) -> Result<(), RuntimeError> {
    let closure_pc = pc.checked_sub(1).ok_or_else(|| {
        RuntimeError::Engine(Error::internal(format!(
            "{label} initializer did not consume an adjacent closure"
        )))
    })?;
    if explicit_control_flow_targets.contains(&closure_pc)
        || explicit_control_flow_targets.contains(&pc)
    {
        return Err(RuntimeError::Engine(Error::internal(format!(
            "{label} closure/initializer pair has a non-fallthrough entry"
        ))));
    }
    let Some(Instruction::FClosure(constant)) = function.code().get(closure_pc) else {
        return Err(RuntimeError::Engine(Error::internal(format!(
            "{label} initializer did not consume an adjacent closure"
        ))));
    };
    let child = usize::try_from(*constant)
        .ok()
        .and_then(|constant| function.constants().get(constant))
        .and_then(UnlinkedConstant::as_child)
        .ok_or_else(|| {
            RuntimeError::Engine(Error::internal(format!(
                "{label} initializer did not reference child bytecode"
            )))
        })?;
    let callable_shape_valid = match accessor_role {
        None => matches!(
            (
                child.metadata().function_kind,
                child.metadata().has_prototype
            ),
            (FunctionKind::Normal | FunctionKind::Async, false) | (FunctionKind::Generator, true)
        ),
        Some(_) => {
            child.metadata().function_kind == FunctionKind::Normal
                && !child.metadata().has_prototype
        }
    };
    if !child.metadata().needs_home_object
        || !child.metadata().strict
        || child.metadata().eval_kind != EvalKind::None
        || !callable_shape_valid
        || child.metadata().constructor_kind != ConstructorKind::None
        || child.metadata().class_initializer_kind.is_some()
    {
        return Err(RuntimeError::Engine(Error::internal(format!(
            "{label} child has invalid HomeObject metadata"
        ))));
    }
    if function
        .code()
        .iter()
        .filter(|instruction| {
            matches!(instruction, Instruction::FClosure(candidate) if candidate == constant)
        })
        .count()
        != 1
    {
        return Err(RuntimeError::Engine(Error::internal(format!(
            "{label} child did not have one unique closure site"
        ))));
    }

    if let Some(role) = accessor_role {
        let expected_arguments = match role {
            PrivateBindingRole::Primary => 0,
            PrivateBindingRole::SetterStorage => 1,
        };
        if child.metadata().argument_count != expected_arguments {
            return Err(RuntimeError::Engine(Error::internal(
                "private-accessor child has invalid authored arity",
            )));
        }
        if child.func_name().is_some_and(|name| !name.is_empty()) {
            return Err(RuntimeError::Engine(Error::internal(
                "private-accessor child retained a non-empty intrinsic name",
            )));
        }
    }

    let mut scope_entries = function
        .code()
        .iter()
        .enumerate()
        .filter_map(|(entry_pc, instruction)| {
            matches!(instruction, Instruction::SetLocalUninitialized(index) if *index == binding_index)
                .then_some(entry_pc)
        });
    let scope_entry_pc = scope_entries.next().ok_or_else(|| {
        RuntimeError::Engine(Error::internal(format!(
            "{label} initializer has no lexical scope entry"
        )))
    })?;
    if scope_entries.next().is_some() || scope_entry_pc >= closure_pc {
        return Err(RuntimeError::Engine(Error::internal(format!(
            "{label} initializer has an invalid lexical scope entry"
        ))));
    }
    for (source_pc, instruction) in function.code().iter().enumerate().skip(pc) {
        let Some(target_pc) = explicit_control_flow_target(instruction) else {
            continue;
        };
        if target_pc > scope_entry_pc && target_pc <= pc && source_pc >= pc {
            return Err(RuntimeError::Engine(Error::internal(format!(
                "{label} initializer is reachable by a repeated-lifetime backedge"
            ))));
        }
    }
    Ok(())
}

pub(super) fn verify_unlinked(function: &UnlinkedFunction) -> Result<(), RuntimeError> {
    let mut initialization_counts = vec![0_u8; function.local_definitions().len()];
    let mut scope_entry_counts = vec![0_u8; function.local_definitions().len()];
    let explicit_control_flow_targets = function
        .code()
        .iter()
        .filter_map(explicit_control_flow_target)
        .collect::<HashSet<_>>();

    for (index, definition) in function.local_definitions().iter().enumerate() {
        if !definition.kind.is_private() {
            continue;
        }
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "private-name local index exceeds bytecode range",
            ))
        })?;
        let _ = verify_private_local(function, index)?;
    }
    for (index, descriptor) in function.closure_variables().iter().enumerate() {
        if !descriptor.kind.is_private() {
            continue;
        }
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "private-name closure index exceeds bytecode range",
            ))
        })?;
        let _ = verify_private_closure(function, index)?;
    }

    for (pc, instruction) in function.code().iter().enumerate() {
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
                if verify_private_local(function, index)?.kind != ClosureVariableKind::PrivateField
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private-name initializer referenced a non-field binding",
                    )));
                }
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
            Instruction::InitializePrivateMethod(index) => {
                if verify_private_local(function, index)?.kind != ClosureVariableKind::PrivateMethod
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private-method initializer referenced a non-method binding",
                    )));
                }
                verify_private_callable_initializer(
                    function,
                    pc,
                    index,
                    None,
                    &explicit_control_flow_targets,
                    "private-method",
                )?;
                let count = initialization_counts
                    .get_mut(usize::from(index))
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "private-method initializer local is out of bounds",
                        ))
                    })?;
                *count = count.checked_add(1).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "private-method initializer count overflowed",
                    ))
                })?;
            }
            Instruction::InitializePrivateAccessor(index) => {
                let binding = verify_private_local(function, index)?;
                if !matches!(
                    binding,
                    PrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateGetter
                            | ClosureVariableKind::PrivateGetterSetter,
                        role: PrivateBindingRole::Primary,
                    } | PrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateSetter,
                        role: PrivateBindingRole::SetterStorage,
                    }
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private-accessor initializer referenced an incompatible binding",
                    )));
                }
                verify_private_callable_initializer(
                    function,
                    pc,
                    index,
                    Some(binding.role),
                    &explicit_control_flow_targets,
                    "private-accessor",
                )?;
                let count = initialization_counts
                    .get_mut(usize::from(index))
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "private-accessor initializer local is out of bounds",
                        ))
                    })?;
                *count = count.checked_add(1).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "private-accessor initializer count overflowed",
                    ))
                })?;
            }
            Instruction::GetPrivateField(source) | Instruction::GetPrivateField2(source) => {
                let binding = verify_private_source(function, source)?;
                if binding.role != PrivateBindingRole::Primary
                    || !matches!(
                        binding.kind,
                        ClosureVariableKind::PrivateField
                            | ClosureVariableKind::PrivateMethod
                            | ClosureVariableKind::PrivateGetter
                            | ClosureVariableKind::PrivateGetterSetter
                    )
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private get referenced an incompatible binding",
                    )));
                }
            }
            Instruction::PutPrivateField(source) => {
                let binding = verify_private_source(function, source)?;
                if !matches!(
                    binding,
                    PrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateField,
                        role: PrivateBindingRole::Primary,
                    } | PrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateSetter,
                        role: PrivateBindingRole::SetterStorage,
                    }
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private put referenced an incompatible binding",
                    )));
                }
            }
            Instruction::PrivateIn(source) => {
                if verify_private_source(function, source)?.role != PrivateBindingRole::Primary {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private-in referenced a synthetic setter binding",
                    )));
                }
            }
            Instruction::DefinePrivateField(source) => {
                if verify_private_source(function, source)?
                    != (PrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateField,
                        role: PrivateBindingRole::Primary,
                    })
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "private-field definition referenced a non-field binding",
                    )));
                }
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
        if !definition.kind.is_private() {
            continue;
        }
        let info = verify_private_local(
            function,
            u16::try_from(index).map_err(|_| {
                RuntimeError::Engine(Error::internal(
                    "private-name local index exceeds bytecode range",
                ))
            })?,
        )?;
        let expected_initializers = match info {
            PrivateBindingInfo {
                kind: ClosureVariableKind::PrivateSetter,
                role: PrivateBindingRole::Primary,
            } => 0,
            _ => 1,
        };
        if initialization_counts[index] != expected_initializers {
            let message = match info {
                PrivateBindingInfo {
                    kind: ClosureVariableKind::PrivateField,
                    ..
                } => "private-name local does not have exactly one lexical initializer",
                PrivateBindingInfo {
                    kind: ClosureVariableKind::PrivateSetter,
                    role: PrivateBindingRole::Primary,
                } => "private-setter primary local must remain uninitialized",
                PrivateBindingInfo {
                    kind:
                        ClosureVariableKind::PrivateGetter
                        | ClosureVariableKind::PrivateSetter
                        | ClosureVariableKind::PrivateGetterSetter,
                    ..
                } => "private-accessor local does not have its required typed initializer",
                _ => "private-method local does not have exactly one typed initializer",
            };
            return Err(RuntimeError::Engine(Error::internal(message)));
        }
        if definition.kind.is_private() && scope_entry_counts[index] != 1 {
            return Err(RuntimeError::Engine(Error::internal(
                if definition.kind == ClosureVariableKind::PrivateField {
                    "private-name local does not have exactly one lexical scope entry"
                } else {
                    "private-method local does not have exactly one lexical scope entry"
                },
            )));
        }
    }
    let _ = private_setter_local_pairs(function.local_definitions())?;

    if function.metadata().class_private_brand
        && !matches!(
            function.metadata().class_initializer_kind,
            Some(ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements)
        )
    {
        return Err(RuntimeError::Engine(Error::internal(
            "private brand metadata escaped a class initializer",
        )));
    }

    let has_private_method_initializer = function
        .code()
        .iter()
        .any(|instruction| matches!(instruction, Instruction::InitializePrivateMethod(_)));
    let has_private_accessor_initializer = function
        .code()
        .iter()
        .any(|instruction| matches!(instruction, Instruction::InitializePrivateAccessor(_)));
    let has_private_brand_child = function.constants().iter().any(|constant| {
        constant.as_child().is_some_and(|child| {
            child.metadata().class_private_brand
                && matches!(
                    child.metadata().class_initializer_kind,
                    Some(
                        ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements
                    )
                )
        })
    });
    if (has_private_method_initializer || has_private_accessor_initializer)
        != has_private_brand_child
    {
        return Err(RuntimeError::Engine(Error::internal(
            if has_private_accessor_initializer {
                "private-callable declarations disagree with class brand initializer metadata"
            } else {
                "private-method declarations disagree with class brand initializer metadata"
            },
        )));
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

    fn private_callable_child(
        function_kind: FunctionKind,
        has_prototype: bool,
    ) -> UnlinkedFunction {
        let code = if function_kind == FunctionKind::Generator {
            vec![
                Instruction::InitialYield,
                Instruction::Undefined,
                Instruction::Return,
            ]
        } else {
            vec![Instruction::Undefined, Instruction::Return]
        };
        UnlinkedFunction::new(
            code,
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                needs_home_object: true,
                function_kind,
                has_prototype,
                ..FunctionMetadata::default()
            },
        )
    }

    fn private_method_function_with_shape(
        code: Vec<Instruction>,
        include_brand: bool,
        function_kind: FunctionKind,
        has_prototype: bool,
    ) -> UnlinkedFunction {
        let method = private_callable_child(function_kind, has_prototype);
        let mut constants = vec![UnlinkedConstant::child(method)];
        if include_brand {
            constants.push(UnlinkedConstant::child(UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    needs_home_object: true,
                    class_initializer_kind: Some(ClassInitializerKind::InstanceFields),
                    class_private_brand: true,
                    ..FunctionMetadata::default()
                },
            )));
        }
        UnlinkedFunction::new(
            code,
            constants,
            FunctionMetadata {
                local_count: 1,
                max_stack: 2,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(
            Vec::new(),
            vec![UnlinkedVariableDefinition {
                name: Some(JsString::from_static("#method")),
                is_lexical: true,
                is_const: true,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::PrivateMethod,
            }],
        )
    }

    fn private_method_function(code: Vec<Instruction>, include_brand: bool) -> UnlinkedFunction {
        private_method_function_with_shape(code, include_brand, FunctionKind::Normal, false)
    }

    fn private_accessor_definition(
        name: &'static str,
        kind: ClosureVariableKind,
    ) -> UnlinkedVariableDefinition {
        UnlinkedVariableDefinition {
            name: Some(JsString::from_static(name)),
            is_lexical: true,
            is_const: true,
            is_parameter_initializer: false,
            kind,
        }
    }

    fn private_accessor_function(
        code: Vec<Instruction>,
        definitions: Vec<UnlinkedVariableDefinition>,
        accessor_children: &[(u16, u16, Option<&'static str>)],
        include_brand: bool,
    ) -> UnlinkedFunction {
        let mut constants = accessor_children
            .iter()
            .map(|&(argument_count, defined_argument_count, name)| {
                let child = UnlinkedFunction::new(
                    vec![Instruction::Undefined, Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        argument_count,
                        defined_argument_count,
                        max_stack: 1,
                        strict: true,
                        needs_home_object: true,
                        ..FunctionMetadata::default()
                    },
                )
                .with_name(name.map(JsString::from_static));
                UnlinkedConstant::child(child)
            })
            .collect::<Vec<_>>();
        if include_brand {
            constants.push(UnlinkedConstant::child(UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    max_stack: 1,
                    strict: true,
                    needs_home_object: true,
                    class_initializer_kind: Some(ClassInitializerKind::InstanceFields),
                    class_private_brand: true,
                    ..FunctionMetadata::default()
                },
            )));
        }
        let local_count = u16::try_from(definitions.len()).unwrap();
        UnlinkedFunction::new(
            code,
            constants,
            FunctionMetadata {
                local_count,
                max_stack: 3,
                ..FunctionMetadata::default()
            },
        )
        .with_variable_definitions(Vec::new(), definitions)
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
    fn private_method_requires_one_adjacent_home_object_closure_and_brand_child() {
        let valid = private_method_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::FClosure(0),
                Instruction::InitializePrivateMethod(0),
                Instruction::CloseLocal(0),
            ],
            true,
        );
        assert!(verify_unlinked(&valid).is_ok());

        let missing_closure = private_method_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::InitializePrivateMethod(0),
                Instruction::CloseLocal(0),
            ],
            true,
        );
        assert!(matches!(
            verify_unlinked(&missing_closure),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "private-method initializer did not consume an adjacent closure"
        ));

        let missing_brand = private_method_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::FClosure(0),
                Instruction::InitializePrivateMethod(0),
                Instruction::CloseLocal(0),
            ],
            false,
        );
        assert!(matches!(
            verify_unlinked(&missing_brand),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "private-method declarations disagree with class brand initializer metadata"
        ));

        for target in [2, 3] {
            let non_fallthrough = private_method_function(
                vec![
                    Instruction::SetLocalUninitialized(0),
                    Instruction::Goto(target),
                    Instruction::FClosure(0),
                    Instruction::InitializePrivateMethod(0),
                    Instruction::CloseLocal(0),
                ],
                true,
            );
            assert!(matches!(
                verify_unlinked(&non_fallthrough),
                Err(RuntimeError::Engine(ref error))
                if error.message()
                        == "private-method closure/initializer pair has a non-fallthrough entry"
            ));
        }

        let repeated_lifetime = private_method_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::Nop,
                Instruction::FClosure(0),
                Instruction::InitializePrivateMethod(0),
                Instruction::Goto(1),
            ],
            true,
        );
        assert!(matches!(
            verify_unlinked(&repeated_lifetime),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "private-method initializer is reachable by a repeated-lifetime backedge"
        ));

        let new_lifetime = private_method_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::FClosure(0),
                Instruction::InitializePrivateMethod(0),
                Instruction::Goto(0),
            ],
            true,
        );
        assert!(verify_unlinked(&new_lifetime).is_ok());
    }

    #[test]
    fn private_callable_initializer_accepts_async_and_generator_method_shapes() {
        let code = vec![
            Instruction::SetLocalUninitialized(0),
            Instruction::FClosure(0),
            Instruction::InitializePrivateMethod(0),
            Instruction::CloseLocal(0),
        ];
        for (function_kind, has_prototype, accepted) in [
            (FunctionKind::Normal, false, true),
            (FunctionKind::Generator, true, true),
            (FunctionKind::Normal, true, false),
            (FunctionKind::Generator, false, false),
            (FunctionKind::Async, false, true),
            (FunctionKind::AsyncGenerator, true, false),
        ] {
            let function = private_method_function_with_shape(
                code.clone(),
                true,
                function_kind,
                has_prototype,
            );
            assert_eq!(
                verify_private_callable_initializer(
                    &function,
                    2,
                    0,
                    None,
                    &HashSet::new(),
                    "private-method",
                )
                .is_ok(),
                accepted,
                "{function_kind:?}/{has_prototype}"
            );
        }

        let generator =
            private_method_function_with_shape(code, true, FunctionKind::Generator, true);
        assert!(
            verify_private_callable_initializer(
                &generator,
                2,
                0,
                Some(PrivateBindingRole::Primary),
                &HashSet::new(),
                "private-accessor",
            )
            .is_err()
        );
        assert!(verify_unlinked(&generator).is_ok());
    }

    #[test]
    fn private_accessors_authenticate_primary_and_synthetic_cell_lifecycles() {
        let getter = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
                Instruction::CloseLocal(0),
            ],
            vec![private_accessor_definition(
                "#value",
                ClosureVariableKind::PrivateGetter,
            )],
            &[(0, 0, None)],
            true,
        );
        assert!(verify_unlinked(&getter).is_ok());

        let setter = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::SetLocalUninitialized(1),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(1),
                Instruction::CloseLocal(1),
                Instruction::CloseLocal(0),
            ],
            vec![
                private_accessor_definition("#value", ClosureVariableKind::PrivateSetter),
                private_accessor_definition("#value<set>", ClosureVariableKind::PrivateSetter),
            ],
            &[(1, 1, None)],
            true,
        );
        assert!(verify_unlinked(&setter).is_ok());

        let pair = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::SetLocalUninitialized(1),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
                Instruction::FClosure(1),
                Instruction::InitializePrivateAccessor(1),
                Instruction::CloseLocal(1),
                Instruction::CloseLocal(0),
            ],
            vec![
                private_accessor_definition("#value", ClosureVariableKind::PrivateGetterSetter),
                private_accessor_definition("#value<set>", ClosureVariableKind::PrivateSetter),
            ],
            &[(0, 0, None), (1, 1, None)],
            true,
        );
        assert!(verify_unlinked(&pair).is_ok());
    }

    #[test]
    fn private_accessors_require_quickjs_authored_arity_and_an_empty_intrinsic_name() {
        let getter_with_parameter = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
            ],
            vec![private_accessor_definition(
                "#value",
                ClosureVariableKind::PrivateGetter,
            )],
            &[(1, 1, None)],
            true,
        );
        assert!(matches!(
            verify_unlinked(&getter_with_parameter),
            Err(RuntimeError::Engine(ref error))
                if error.message() == "private-accessor child has invalid authored arity"
        ));

        let setter_without_parameter = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::SetLocalUninitialized(1),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(1),
            ],
            vec![
                private_accessor_definition("#value", ClosureVariableKind::PrivateSetter),
                private_accessor_definition("#value<set>", ClosureVariableKind::PrivateSetter),
            ],
            &[(0, 0, None)],
            true,
        );
        assert!(matches!(
            verify_unlinked(&setter_without_parameter),
            Err(RuntimeError::Engine(ref error))
                if error.message() == "private-accessor child has invalid authored arity"
        ));

        // QuickJS retains one authored setter parameter even when the public
        // `length`/defined count is zero because that parameter has a default
        // or BindingPattern. An explicitly empty intrinsic name is equivalent
        // to no retained name and must remain valid.
        let default_or_pattern_setter = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::SetLocalUninitialized(1),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(1),
            ],
            vec![
                private_accessor_definition("#value", ClosureVariableKind::PrivateSetter),
                private_accessor_definition("#value<set>", ClosureVariableKind::PrivateSetter),
            ],
            &[(1, 0, Some(""))],
            true,
        );
        assert!(verify_unlinked(&default_or_pattern_setter).is_ok());

        let named_getter = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
            ],
            vec![private_accessor_definition(
                "#value",
                ClosureVariableKind::PrivateGetter,
            )],
            &[(0, 0, Some("forged"))],
            true,
        );
        assert!(matches!(
            verify_unlinked(&named_getter),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "private-accessor child retained a non-empty intrinsic name"
        ));
    }

    #[test]
    fn private_accessor_initializer_rejects_mid_lifetime_backedges() {
        let forged = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::Nop,
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
                Instruction::Goto(1),
            ],
            vec![private_accessor_definition(
                "#value",
                ClosureVariableKind::PrivateGetter,
            )],
            &[(0, 0, None)],
            true,
        );
        assert!(matches!(
            verify_unlinked(&forged),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "private-accessor initializer is reachable by a repeated-lifetime backedge"
        ));

        let new_lifetime = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
                Instruction::Goto(0),
            ],
            vec![private_accessor_definition(
                "#value",
                ClosureVariableKind::PrivateGetter,
            )],
            &[(0, 0, None)],
            true,
        );
        assert!(verify_unlinked(&new_lifetime).is_ok());
    }

    #[test]
    fn private_accessors_reject_forged_setter_roles_and_unpaired_cells() {
        let initialized_primary = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
            ],
            vec![private_accessor_definition(
                "#value",
                ClosureVariableKind::PrivateSetter,
            )],
            &[(1, 1, None)],
            true,
        );
        assert!(matches!(
            verify_unlinked(&initialized_primary),
            Err(RuntimeError::Engine(ref error))
                if error.message()
                    == "private-accessor initializer referenced an incompatible binding"
        ));

        let synthetic_in = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::SetLocalUninitialized(1),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(1),
                Instruction::PrivateIn(PrivateNameSource::Local(1)),
            ],
            vec![
                private_accessor_definition("#value", ClosureVariableKind::PrivateSetter),
                private_accessor_definition("#value<set>", ClosureVariableKind::PrivateSetter),
            ],
            &[(1, 1, None)],
            true,
        );
        assert!(matches!(
            verify_unlinked(&synthetic_in),
            Err(RuntimeError::Engine(ref error))
                if error.message() == "private-in referenced a synthetic setter binding"
        ));

        let unpaired = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::CloseLocal(0),
            ],
            vec![private_accessor_definition(
                "#value",
                ClosureVariableKind::PrivateSetter,
            )],
            &[],
            false,
        );
        assert!(matches!(
            verify_unlinked(&unpaired),
            Err(RuntimeError::Engine(ref error))
                if error.message() == "private setter primary/storage cells are not paired"
        ));

        let storage_before_primary = private_accessor_function(
            vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::SetLocalUninitialized(1),
                Instruction::FClosure(0),
                Instruction::InitializePrivateAccessor(0),
            ],
            vec![
                private_accessor_definition("#value<set>", ClosureVariableKind::PrivateSetter),
                private_accessor_definition("#value", ClosureVariableKind::PrivateSetter),
            ],
            &[(1, 1, None)],
            true,
        );
        assert!(matches!(
            verify_unlinked(&storage_before_primary),
            Err(RuntimeError::Engine(ref error))
                if error.message() == "private setter primary/storage cells are not paired"
        ));

        let nested_same_name = vec![
            private_accessor_definition("#value", ClosureVariableKind::PrivateSetter),
            private_accessor_definition("#value", ClosureVariableKind::PrivateGetterSetter),
            private_accessor_definition("#value<set>", ClosureVariableKind::PrivateSetter),
            private_accessor_definition("#value<set>", ClosureVariableKind::PrivateSetter),
        ];
        assert_eq!(
            private_setter_local_pairs(&nested_same_name).unwrap(),
            vec![Some(3), Some(2), Some(1), Some(0)]
        );
    }

    #[test]
    fn private_brand_metadata_is_limited_to_aggregate_class_initializers() {
        let forged = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                class_private_brand: true,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            verify_unlinked(&forged),
            Err(RuntimeError::Engine(ref error))
                if error.message() == "private brand metadata escaped a class initializer"
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
