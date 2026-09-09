//! Authenticate private binding metadata and private callable bytecode before heap publication.

use super::{
    AuthenticatedPrivateBindings, BytecodeConstant, ClassInitializerKind, ClosureSource,
    ClosureVariableKind, ClosureVariableName, ConstructorKind, EvalKind, FunctionBytecodeData,
    FunctionKind, HashSet, Heap, HeapError, Instruction, PrivateNameSource,
    PublishedPrivateBindingRole,
};

fn private_binding_flags_are_valid(
    kind: ClosureVariableKind,
    is_lexical: bool,
    is_const: bool,
) -> bool {
    kind.is_private() && is_lexical && is_const
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PublishedPrivateBindingInfo {
    kind: ClosureVariableKind,
    role: PublishedPrivateBindingRole,
}

fn validate_published_private_binding_metadata(
    bytecode: &FunctionBytecodeData,
) -> Result<Option<&AuthenticatedPrivateBindings>, HeapError> {
    let has_private_bindings = bytecode
        .local_definitions
        .iter()
        .any(|definition| definition.kind.is_private())
        || bytecode
            .closure_variables
            .iter()
            .any(|descriptor| descriptor.kind.is_private());
    let Some(authenticated) = bytecode.private_bindings.authenticated.as_ref() else {
        return if has_private_bindings {
            Err(HeapError::Invariant(
                "published private bindings are missing sealed role metadata",
            ))
        } else {
            Ok(None)
        };
    };
    if authenticated.locals.len() != bytecode.local_definitions.len()
        || authenticated.closures.len() != bytecode.closure_variables.len()
    {
        return Err(HeapError::Invariant(
            "published private binding role table has the wrong shape",
        ));
    }

    for (definition, binding) in bytecode
        .local_definitions
        .iter()
        .zip(authenticated.locals.iter())
    {
        match (definition.kind.is_private(), binding) {
            (false, None) => continue,
            (false, Some(_)) => {
                return Err(HeapError::Invariant(
                    "ordinary local carries a sealed private binding role",
                ));
            }
            (true, None) => {
                return Err(HeapError::Invariant(
                    "published private local is missing its sealed binding role",
                ));
            }
            (true, Some(binding)) => {
                if !private_binding_flags_are_valid(
                    definition.kind,
                    definition.is_lexical,
                    definition.is_const,
                ) || definition.is_parameter_initializer
                    || definition.name != Some(binding.name)
                    || (binding.role == PublishedPrivateBindingRole::SetterStorage
                        && definition.kind != ClosureVariableKind::PrivateSetter)
                {
                    return Err(HeapError::Invariant(
                        "published private-name local has invalid sealed metadata",
                    ));
                }
            }
        }
    }
    for (descriptor, binding) in bytecode
        .closure_variables
        .iter()
        .zip(authenticated.closures.iter())
    {
        match (descriptor.kind.is_private(), binding) {
            (false, None) => continue,
            (false, Some(_)) => {
                return Err(HeapError::Invariant(
                    "ordinary closure carries a sealed private binding role",
                ));
            }
            (true, None) => {
                return Err(HeapError::Invariant(
                    "published private closure is missing its sealed binding role",
                ));
            }
            (true, Some(binding)) => {
                if !private_binding_flags_are_valid(
                    descriptor.kind,
                    descriptor.is_lexical,
                    descriptor.is_const,
                ) || descriptor.name != ClosureVariableName::Atom(binding.name)
                    || !matches!(
                        descriptor.source,
                        ClosureSource::ParentLocal(_)
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::EvalEnvironment(_)
                    )
                    || binding.pair.is_some()
                    || (binding.role == PublishedPrivateBindingRole::SetterStorage
                        && descriptor.kind != ClosureVariableKind::PrivateSetter)
                {
                    return Err(HeapError::Invariant(
                        "published private-name closure has invalid sealed metadata",
                    ));
                }
            }
        }
    }

    for (index, (definition, binding)) in bytecode
        .local_definitions
        .iter()
        .zip(authenticated.locals.iter())
        .enumerate()
    {
        let Some(binding) = binding else {
            continue;
        };
        let pair_required = binding.role == PublishedPrivateBindingRole::SetterStorage
            || (binding.role == PublishedPrivateBindingRole::Primary
                && matches!(
                    definition.kind,
                    ClosureVariableKind::PrivateSetter | ClosureVariableKind::PrivateGetterSetter
                ));
        if pair_required != binding.pair.is_some() {
            return Err(HeapError::Invariant(
                "published private setter has malformed pair metadata",
            ));
        }
        let Some(pair) = binding.pair else {
            continue;
        };
        let pair_index = usize::from(pair);
        let Some(Some(other)) = authenticated.locals.get(pair_index) else {
            return Err(HeapError::Invariant(
                "published private setter pair is out of bounds",
            ));
        };
        let Some(other_definition) = bytecode.local_definitions.get(pair_index) else {
            return Err(HeapError::Invariant(
                "published private setter pair is out of bounds",
            ));
        };
        if pair_index == index
            || other.pair != u16::try_from(index).ok()
            || other.role == binding.role
            || binding.name == other.name
            || match binding.role {
                PublishedPrivateBindingRole::Primary => {
                    !matches!(
                        definition.kind,
                        ClosureVariableKind::PrivateSetter
                            | ClosureVariableKind::PrivateGetterSetter
                    ) || other_definition.kind != ClosureVariableKind::PrivateSetter
                }
                PublishedPrivateBindingRole::SetterStorage => {
                    definition.kind != ClosureVariableKind::PrivateSetter
                        || !matches!(
                            other_definition.kind,
                            ClosureVariableKind::PrivateSetter
                                | ClosureVariableKind::PrivateGetterSetter
                        )
                }
            }
        {
            return Err(HeapError::Invariant(
                "published private setter pair is not reciprocal",
            ));
        }
    }
    Ok(Some(authenticated))
}

fn validate_published_private_source(
    bytecode: &FunctionBytecodeData,
    source: PrivateNameSource,
) -> Result<PublishedPrivateBindingInfo, HeapError> {
    let authenticated =
        bytecode
            .private_bindings
            .authenticated
            .as_ref()
            .ok_or(HeapError::Invariant(
                "private bytecode source has no sealed binding role",
            ))?;
    let info = match source {
        PrivateNameSource::Local(index) => bytecode
            .local_definitions
            .get(usize::from(index))
            .zip(authenticated.locals.get(usize::from(index)))
            .and_then(|(definition, binding)| {
                binding.map(|binding| PublishedPrivateBindingInfo {
                    kind: definition.kind,
                    role: binding.role,
                })
            }),
        PrivateNameSource::Closure(index) => bytecode
            .closure_variables
            .get(usize::from(index))
            .zip(authenticated.closures.get(usize::from(index)))
            .and_then(|(descriptor, binding)| {
                binding.map(|binding| PublishedPrivateBindingInfo {
                    kind: descriptor.kind,
                    role: binding.role,
                })
            }),
    };
    info.ok_or(HeapError::Invariant(
        "private bytecode source is not an authenticated lexical binding",
    ))
}

fn explicit_private_control_flow_target(instruction: &Instruction) -> Option<usize> {
    let target = match instruction {
        Instruction::Goto(target)
        | Instruction::IfFalse(target)
        | Instruction::IfTrue(target)
        | Instruction::Catch(target)
        | Instruction::Gosub(target) => *target,
        _ => return None,
    };
    usize::try_from(target).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivateCallableInitializerKind {
    Method,
    Accessor,
}

fn validate_published_private_callable_initializer(
    heap: &Heap,
    bytecode: &FunctionBytecodeData,
    pc: usize,
    binding_index: u16,
    accessor_role: Option<PublishedPrivateBindingRole>,
    explicit_control_flow_targets: &HashSet<usize>,
    kind: PrivateCallableInitializerKind,
) -> Result<(), HeapError> {
    let adjacent_error = match kind {
        PrivateCallableInitializerKind::Method => {
            "private-method initializer did not consume an adjacent closure"
        }
        PrivateCallableInitializerKind::Accessor => {
            "private-accessor initializer did not consume an adjacent closure"
        }
    };
    let closure_pc = pc
        .checked_sub(1)
        .ok_or(HeapError::Invariant(adjacent_error))?;
    if explicit_control_flow_targets.contains(&closure_pc)
        || explicit_control_flow_targets.contains(&pc)
    {
        return Err(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method closure/initializer pair has a non-fallthrough entry"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor closure/initializer pair has a non-fallthrough entry"
            }
        }));
    }
    let Some(Instruction::FClosure(constant)) = bytecode.code.get(closure_pc) else {
        return Err(HeapError::Invariant(adjacent_error));
    };
    let child_id = usize::try_from(*constant)
        .ok()
        .and_then(|constant| bytecode.constants.get(constant))
        .and_then(|constant| match constant {
            BytecodeConstant::Function(child) => Some(*child),
            BytecodeConstant::Value(_) | BytecodeConstant::RegExp { .. } => None,
        })
        .ok_or(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method initializer did not reference child bytecode"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor initializer did not reference child bytecode"
            }
        }))?;
    let child = heap.function_bytecode(child_id).map_err(|_| {
        HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method initializer did not reference live child bytecode"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor initializer did not reference live child bytecode"
            }
        })
    })?;
    let callable_shape_valid = match kind {
        PrivateCallableInitializerKind::Method => matches!(
            (child.metadata.function_kind, child.metadata.has_prototype),
            (FunctionKind::Normal | FunctionKind::Async, false)
                | (FunctionKind::Generator | FunctionKind::AsyncGenerator, true)
        ),
        PrivateCallableInitializerKind::Accessor => {
            child.metadata.function_kind == FunctionKind::Normal && !child.metadata.has_prototype
        }
    };
    if !child.metadata.needs_home_object
        || !child.metadata.strict
        || child.metadata.eval_kind != EvalKind::None
        || !callable_shape_valid
        || child.metadata.constructor_kind != ConstructorKind::None
        || child.metadata.class_initializer_kind.is_some()
    {
        return Err(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method child has invalid HomeObject metadata"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor child has invalid HomeObject metadata"
            }
        }));
    }
    if bytecode
        .code
        .iter()
        .filter(|instruction| {
            matches!(instruction, Instruction::FClosure(candidate) if candidate == constant)
        })
        .count()
        != 1
    {
        return Err(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method child did not have one unique closure site"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor child did not have one unique closure site"
            }
        }));
    }

    if let Some(role) = accessor_role {
        let expected_arguments = match role {
            PublishedPrivateBindingRole::Primary => 0,
            PublishedPrivateBindingRole::SetterStorage => 1,
        };
        if child.metadata.argument_count != expected_arguments {
            return Err(HeapError::Invariant(
                "private-accessor child has invalid authored arity",
            ));
        }
        if child
            .func_name
            .as_ref()
            .is_some_and(|name| !name.is_empty())
        {
            return Err(HeapError::Invariant(
                "private-accessor child retained a non-empty intrinsic name",
            ));
        }
    }

    let mut scope_entries = bytecode
        .code
        .iter()
        .enumerate()
        .filter_map(|(entry_pc, instruction)| {
            matches!(instruction, Instruction::SetLocalUninitialized(index) if *index == binding_index)
                .then_some(entry_pc)
        });
    let scope_entry_pc = scope_entries
        .next()
        .ok_or(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method initializer has no lexical scope entry"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor initializer has no lexical scope entry"
            }
        }))?;
    if scope_entries.next().is_some() || scope_entry_pc >= closure_pc {
        return Err(HeapError::Invariant(match kind {
            PrivateCallableInitializerKind::Method => {
                "private-method initializer has an invalid lexical scope entry"
            }
            PrivateCallableInitializerKind::Accessor => {
                "private-accessor initializer has an invalid lexical scope entry"
            }
        }));
    }
    for (source_pc, instruction) in bytecode.code.iter().enumerate().skip(pc) {
        let Some(target_pc) = explicit_private_control_flow_target(instruction) else {
            continue;
        };
        if target_pc > scope_entry_pc && target_pc <= pc && source_pc >= pc {
            return Err(HeapError::Invariant(match kind {
                PrivateCallableInitializerKind::Method => {
                    "private-method initializer is reachable by a repeated-lifetime backedge"
                }
                PrivateCallableInitializerKind::Accessor => {
                    "private-accessor initializer is reachable by a repeated-lifetime backedge"
                }
            }));
        }
    }
    Ok(())
}

fn ordinary_private_local_operand(instruction: &Instruction) -> Option<u16> {
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

fn ordinary_private_closure_operand(instruction: &Instruction) -> Option<u16> {
    match instruction {
        Instruction::GetVarRef(index)
        | Instruction::PutVarRef(index)
        | Instruction::SetVarRef(index)
        | Instruction::GetVarRefCheck(index)
        | Instruction::PutVarRefCheck(index)
        | Instruction::InitializeVarRef(index)
        | Instruction::InitializeModuleImportCollision(index)
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

pub(super) fn validate_published_private_elements(
    heap: &Heap,
    bytecode: &FunctionBytecodeData,
) -> Result<(), HeapError> {
    let mut initialization_counts = vec![0_u8; bytecode.local_definitions.len()];
    let mut scope_entry_counts = vec![0_u8; bytecode.local_definitions.len()];
    let _ = validate_published_private_binding_metadata(bytecode)?;
    let explicit_control_flow_targets = bytecode
        .code
        .iter()
        .filter_map(explicit_private_control_flow_target)
        .collect::<HashSet<_>>();

    for (pc, instruction) in bytecode.code.iter().enumerate() {
        if let Some(index) = ordinary_private_local_operand(instruction)
            && bytecode
                .local_definitions
                .get(usize::from(index))
                .is_some_and(|definition| definition.kind.is_private())
        {
            return Err(HeapError::Invariant(
                "ordinary local bytecode references a private-name binding",
            ));
        }
        if let Some(index) = ordinary_private_closure_operand(instruction)
            && bytecode
                .closure_variables
                .get(usize::from(index))
                .is_some_and(|descriptor| descriptor.kind.is_private())
        {
            return Err(HeapError::Invariant(
                "ordinary closure bytecode references a private-name binding",
            ));
        }

        match *instruction {
            Instruction::SetLocalUninitialized(index)
                if bytecode
                    .local_definitions
                    .get(usize::from(index))
                    .is_some_and(|definition| definition.kind.is_private()) =>
            {
                let count =
                    scope_entry_counts
                        .get_mut(usize::from(index))
                        .ok_or(HeapError::Invariant(
                            "private-name scope-entry local is out of bounds",
                        ))?;
                *count = count.checked_add(1).ok_or(HeapError::Invariant(
                    "private-name scope-entry count overflowed",
                ))?;
            }
            Instruction::InitializePrivateName(index) => {
                if validate_published_private_source(bytecode, PrivateNameSource::Local(index))?
                    != (PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateField,
                        role: PublishedPrivateBindingRole::Primary,
                    })
                {
                    return Err(HeapError::Invariant(
                        "private-name initializer referenced a non-field binding",
                    ));
                }
                let count = initialization_counts.get_mut(usize::from(index)).ok_or(
                    HeapError::Invariant("private-name initializer local is out of bounds"),
                )?;
                *count = count.checked_add(1).ok_or(HeapError::Invariant(
                    "private-name initializer count overflowed",
                ))?;
            }
            Instruction::InitializePrivateMethod(index) => {
                if validate_published_private_source(bytecode, PrivateNameSource::Local(index))?
                    != (PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateMethod,
                        role: PublishedPrivateBindingRole::Primary,
                    })
                {
                    return Err(HeapError::Invariant(
                        "private-method initializer referenced a non-method binding",
                    ));
                }
                validate_published_private_callable_initializer(
                    heap,
                    bytecode,
                    pc,
                    index,
                    None,
                    &explicit_control_flow_targets,
                    PrivateCallableInitializerKind::Method,
                )?;
                let count = initialization_counts.get_mut(usize::from(index)).ok_or(
                    HeapError::Invariant("private-method initializer local is out of bounds"),
                )?;
                *count = count.checked_add(1).ok_or(HeapError::Invariant(
                    "private-method initializer count overflowed",
                ))?;
            }
            Instruction::InitializePrivateAccessor(index) => {
                let binding =
                    validate_published_private_source(bytecode, PrivateNameSource::Local(index))?;
                if !matches!(
                    binding,
                    PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateGetter
                            | ClosureVariableKind::PrivateGetterSetter,
                        role: PublishedPrivateBindingRole::Primary,
                    } | PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateSetter,
                        role: PublishedPrivateBindingRole::SetterStorage,
                    }
                ) {
                    return Err(HeapError::Invariant(
                        "private-accessor initializer referenced an incompatible binding",
                    ));
                }
                validate_published_private_callable_initializer(
                    heap,
                    bytecode,
                    pc,
                    index,
                    Some(binding.role),
                    &explicit_control_flow_targets,
                    PrivateCallableInitializerKind::Accessor,
                )?;
                let count = initialization_counts.get_mut(usize::from(index)).ok_or(
                    HeapError::Invariant("private-accessor initializer local is out of bounds"),
                )?;
                *count = count.checked_add(1).ok_or(HeapError::Invariant(
                    "private-accessor initializer count overflowed",
                ))?;
            }
            Instruction::GetPrivateField(source) | Instruction::GetPrivateField2(source) => {
                let binding = validate_published_private_source(bytecode, source)?;
                if binding.role != PublishedPrivateBindingRole::Primary
                    || !matches!(
                        binding.kind,
                        ClosureVariableKind::PrivateField
                            | ClosureVariableKind::PrivateMethod
                            | ClosureVariableKind::PrivateGetter
                            | ClosureVariableKind::PrivateGetterSetter
                    )
                {
                    return Err(HeapError::Invariant(
                        "private get referenced an incompatible binding",
                    ));
                }
            }
            Instruction::PutPrivateField(source) => {
                let binding = validate_published_private_source(bytecode, source)?;
                if !matches!(
                    binding,
                    PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateField,
                        role: PublishedPrivateBindingRole::Primary,
                    } | PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateSetter,
                        role: PublishedPrivateBindingRole::SetterStorage,
                    }
                ) {
                    return Err(HeapError::Invariant(
                        "private put referenced an incompatible binding",
                    ));
                }
            }
            Instruction::PrivateIn(source) => {
                if validate_published_private_source(bytecode, source)?.role
                    != PublishedPrivateBindingRole::Primary
                {
                    return Err(HeapError::Invariant(
                        "private-in referenced a synthetic setter binding",
                    ));
                }
            }
            Instruction::DefinePrivateField(source) => {
                if validate_published_private_source(bytecode, source)?
                    != (PublishedPrivateBindingInfo {
                        kind: ClosureVariableKind::PrivateField,
                        role: PublishedPrivateBindingRole::Primary,
                    })
                {
                    return Err(HeapError::Invariant(
                        "private-field definition referenced a non-field binding",
                    ));
                }
                if !matches!(
                    bytecode.metadata.class_initializer_kind,
                    Some(
                        ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements
                    )
                ) {
                    return Err(HeapError::Invariant(
                        "private-field definition escaped a class initializer",
                    ));
                }
            }
            _ => {}
        }
    }

    let authenticated = bytecode.private_bindings.authenticated.as_ref();
    for (index, definition) in bytecode.local_definitions.iter().enumerate() {
        if !definition.kind.is_private() {
            continue;
        }
        let binding = authenticated
            .and_then(|authenticated| authenticated.locals.get(index))
            .and_then(Option::as_ref)
            .ok_or(HeapError::Invariant(
                "published private local lost its sealed binding role",
            ))?;
        let expected_initializers = usize::from(
            !(definition.kind == ClosureVariableKind::PrivateSetter
                && binding.role == PublishedPrivateBindingRole::Primary),
        );
        if usize::from(initialization_counts[index]) != expected_initializers {
            return Err(HeapError::Invariant(
                match (definition.kind, binding.role) {
                    (ClosureVariableKind::PrivateField, PublishedPrivateBindingRole::Primary) => {
                        "private-name local does not have exactly one lexical initializer"
                    }
                    (ClosureVariableKind::PrivateSetter, PublishedPrivateBindingRole::Primary) => {
                        "private-setter primary local must remain uninitialized"
                    }
                    (
                        ClosureVariableKind::PrivateGetter
                        | ClosureVariableKind::PrivateSetter
                        | ClosureVariableKind::PrivateGetterSetter,
                        _,
                    ) => "private-accessor local does not have its required typed initializer",
                    _ => "private-method local does not have exactly one typed initializer",
                },
            ));
        }
        if scope_entry_counts[index] != 1 {
            return Err(HeapError::Invariant(
                if definition.kind == ClosureVariableKind::PrivateField {
                    "private-name local does not have exactly one lexical scope entry"
                } else {
                    "private-method local does not have exactly one lexical scope entry"
                },
            ));
        }
    }

    let has_private_method_initializer = bytecode
        .code
        .iter()
        .any(|instruction| matches!(instruction, Instruction::InitializePrivateMethod(_)));
    let has_private_accessor_initializer = bytecode
        .code
        .iter()
        .any(|instruction| matches!(instruction, Instruction::InitializePrivateAccessor(_)));
    let mut has_private_brand_child = false;
    for constant in bytecode.constants.iter() {
        let BytecodeConstant::Function(child) = constant else {
            continue;
        };
        let child = heap.function_bytecode(*child).map_err(|_| {
            HeapError::Invariant("private declaration referenced non-live child bytecode")
        })?;
        has_private_brand_child |= child.metadata.class_private_brand
            && matches!(
                child.metadata.class_initializer_kind,
                Some(ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements)
            );
    }
    if (has_private_method_initializer || has_private_accessor_initializer)
        != has_private_brand_child
    {
        return Err(HeapError::Invariant(if has_private_accessor_initializer {
            "private-callable declarations disagree with class brand initializer metadata"
        } else {
            "private-method declarations disagree with class brand initializer metadata"
        }));
    }
    Ok(())
}
