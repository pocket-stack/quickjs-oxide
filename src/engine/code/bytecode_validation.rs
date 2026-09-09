//! Validate compiler-authored frame, parameter, and eval bytecode layouts before publication.

use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::*;
use std::collections::HashSet;

/// Compiler-authored frame pseudo bindings have one canonical QuickJS entry
/// order.  Keeping the rank in the heap layer lets every publication shape
/// (plain/default/rest/pattern parameters) authenticate the same ABI.
const fn pseudo_binding_entry_rank(instruction: &Instruction) -> Option<u8> {
    match instruction {
        Instruction::PushHomeObject => Some(1),
        Instruction::PushActiveFunction => Some(2),
        Instruction::PushNewTarget => Some(3),
        Instruction::PushThis => Some(4),
        _ => None,
    }
}

const fn is_derived_initialization_source(instruction: &Instruction) -> bool {
    matches!(
        instruction,
        Instruction::ConstructSuper(_)
            | Instruction::ApplySuper
            | Instruction::InitDerivedConstructor
    )
}

/// Whether pinned QuickJS copies `defined_arg_count` out of its parser record.
///
/// QuickJS gates that copy on `arg_count + var_count > 0`. Most Rust locals
/// correspond directly to QuickJS variables, but owning-function HomeObject,
/// active-function, `new.target`, and ordinary `this` reads use dedicated
/// bytecode here while QuickJS resolves each to a hidden variable. This
/// predicate keeps the observable empty terminal rest-BindingPattern `length`
/// quirk shared by lowering and both publication boundaries.
pub fn quickjs_copies_defined_argument_count(
    argument_count: usize,
    local_count: usize,
    code: &[Instruction],
) -> bool {
    argument_count != 0
        || local_count != 0
        || code
            .iter()
            .any(|instruction| pseudo_binding_entry_rank(instruction).is_some())
}

/// Authenticate the call-frame ABI encoded by formal-parameter bytecode.
///
/// This stays independent from compiler IR so both unlinked publication and
/// the final heap allocation boundary authenticate the same structural ABI.
/// The unlinked publisher separately authenticates source-level binding names.
/// A successful parameter environment returns the first body instruction so
/// that the unlinked boundary can authenticate segment-specific captures.
fn validate_class_constructor_guard(
    metadata: &FunctionMetadata,
    code: &[Instruction],
) -> Result<Option<usize>, &'static str> {
    let guard_pcs = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| matches!(instruction, Instruction::CheckCtor).then_some(pc))
        .collect::<Vec<_>>();
    let is_class_constructor = metadata.constructor_kind != ConstructorKind::None
        && metadata.strict
        && !metadata.has_prototype;
    let guard_pc = match (is_class_constructor, guard_pcs.as_slice()) {
        (true, [pc]) => *pc,
        (true, []) => return Err("class constructor has no constructor-call guard"),
        (true, _) => return Err("class constructor guard is not unique"),
        (false, []) => return Ok(None),
        (false, _) => return Err("non-class function contains a constructor-call guard"),
    };

    // The guard may follow only entry ABI work: authenticated pseudo-binding
    // materialization, lexical TDZ reset, arguments/eval objects and function
    // hoists. Parameter-specific validators below pin it to the exact slot
    // between that prologue and the first parameter selection skeleton.
    let mut depth = 0_usize;
    for instruction in &code[..guard_pc] {
        if !matches!(
            instruction,
            Instruction::PushHomeObject
                | Instruction::PushActiveFunction
                | Instruction::PushNewTarget
                | Instruction::PushThis
                | Instruction::FClosure(_)
                | Instruction::Arguments(_)
                | Instruction::VariableEnvironment
                | Instruction::Dup
                | Instruction::PutLocal(_)
                | Instruction::InitializeLocal(_)
                | Instruction::SetLocalUninitialized(_)
        ) {
            return Err("class constructor guard is not in the entry prologue");
        }
        let (popped, pushed) = instruction.stack_effect();
        depth = depth
            .checked_sub(popped)
            .ok_or("class constructor entry prologue has stack underflow")?
            .checked_add(pushed)
            .ok_or("class constructor entry prologue has stack overflow")?;
    }
    if depth != 0 {
        return Err("class constructor guard interrupts its entry prologue");
    }
    Ok(Some(guard_pc))
}

fn consume_class_constructor_guard(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    guard_pc: Option<usize>,
    expected_pc: usize,
) -> Result<usize, &'static str> {
    match guard_pc {
        Some(actual) if actual == expected_pc => {
            let after_guard = expected_pc
                .checked_add(1)
                .ok_or("class constructor guard position overflowed bytecode")?;
            if metadata.constructor_kind == ConstructorKind::Base {
                if !matches!(
                    code.get(after_guard..after_guard + 4),
                    Some([
                        Instruction::PushThis,
                        Instruction::PushActiveFunction,
                        Instruction::CallClassInstanceInitializer,
                        Instruction::Drop,
                    ])
                ) {
                    return Err("base class constructor has no exact field initializer hook");
                }
                after_guard
                    .checked_add(4)
                    .ok_or("class field initializer position overflowed bytecode")
            } else {
                Ok(after_guard)
            }
        }
        Some(_) => Err("class constructor guard is not at parameter entry"),
        None => Ok(expected_pc),
    }
}

/// Authenticate the frame layout and opcode authority used by derived class
/// constructors and by arrows/direct eval which relay their one-shot `this`
/// binding. This validation runs again at final heap allocation so dead code
/// and hand-authored unlinked bytecode cannot smuggle constructor-only state
/// transitions into an ordinary function.
pub fn validate_derived_constructor_bytecode_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    lexical_locals: &[bool],
    const_locals: &[bool],
    closure_variables: &[ClosureVariable],
) -> Result<(), &'static str> {
    if lexical_locals.len() != usize::from(metadata.local_count)
        || const_locals.len() != usize::from(metadata.local_count)
    {
        return Err("derived constructor local classification has the wrong length");
    }

    let derived = metadata.constructor_kind == ConstructorKind::Derived;
    let (this_local, active_local) = match (
        derived,
        metadata.derived_this_local,
        metadata.active_function_local,
    ) {
        (true, Some(this), Some(active))
            if this < metadata.local_count
                && active < metadata.local_count
                && this != active
                && metadata.strict
                && !metadata.has_prototype
                && metadata.super_call_allowed
                && metadata.super_allowed
                && lexical_locals[usize::from(this)]
                && !const_locals[usize::from(this)]
                && !lexical_locals[usize::from(active)]
                && !const_locals[usize::from(active)] =>
        {
            (Some(this), Some(active))
        }
        (true, _, _) => return Err("derived constructor metadata is malformed"),
        (false, None, None) => (None, None),
        (false, _, _) => return Err("derived constructor locals escaped their constructor"),
    };

    let mut entry_pc = 0_usize;
    let mut pseudo_rank = 0_u8;
    let mut pseudo_targets = Vec::with_capacity(4);
    let mut active_initialized_at_entry = false;
    while let Some([source, Instruction::PutLocal(local)]) = code.get(entry_pc..entry_pc + 2) {
        let Some(rank) = pseudo_binding_entry_rank(source) else {
            break;
        };
        if rank <= pseudo_rank || *local >= metadata.local_count || pseudo_targets.contains(local) {
            return Err("pseudo-binding entry prologue is malformed");
        }
        if derived && matches!(source, Instruction::PushThis) {
            return Err("derived constructor contains an ordinary this prologue");
        }
        if matches!(source, Instruction::PushActiveFunction) {
            if active_local != Some(*local) {
                return Err("active-function opcode targets another entry local");
            }
            active_initialized_at_entry = true;
        }
        pseudo_rank = rank;
        pseudo_targets.push(*local);
        entry_pc += 2;
    }

    let mut active_initializations = 0_usize;
    let mut default_initializers = 0_usize;
    let explicit_targets = code
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target) => usize::try_from(*target).ok(),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let base_initializer_hook = if metadata.constructor_kind == ConstructorKind::Base
        && metadata.strict
        && !metadata.has_prototype
    {
        let Some(guard_pc) = validate_class_constructor_guard(metadata, code)? else {
            return Err("base class constructor has no constructor-call guard");
        };
        let call_pc = guard_pc
            .checked_add(3)
            .ok_or("class field initializer position overflowed bytecode")?;
        if !matches!(
            code.get(guard_pc..guard_pc + 5),
            Some([
                Instruction::CheckCtor,
                Instruction::PushThis,
                Instruction::PushActiveFunction,
                Instruction::CallClassInstanceInitializer,
                Instruction::Drop,
            ])
        ) {
            return Err("base class constructor has no exact field initializer hook");
        }
        if (guard_pc..=guard_pc + 4).any(|pc| explicit_targets.contains(&pc)) {
            return Err("base class initializer protocol has a non-fallthrough entry");
        }
        Some(call_pc)
    } else {
        None
    };
    for (pc, instruction) in code.iter().enumerate() {
        match instruction {
            Instruction::MarkSuperCall
            | Instruction::ConstructSuper(_)
            | Instruction::ApplySuper
                if !metadata.super_call_allowed || !metadata.super_allowed =>
            {
                return Err("typed super-call opcode has no inherited super authority");
            }
            Instruction::PushActiveFunction => {
                let base_field_hook = base_initializer_hook == pc.checked_add(1);
                if base_field_hook {
                    continue;
                }
                let Some(active) = active_local else {
                    return Err("active-function opcode escaped a derived constructor");
                };
                if !matches!(code.get(pc + 1), Some(Instruction::PutLocal(local)) if *local == active)
                {
                    return Err("active-function opcode has no authenticated local store");
                }
                active_initializations += 1;
            }
            Instruction::InitDerivedConstructor => {
                if !derived {
                    return Err("default-derived initializer escaped a derived constructor");
                }
                default_initializers += 1;
            }
            Instruction::InitializeDerivedLocal(local) => {
                if Some(*local) != this_local {
                    return Err("derived local initializer targets another local");
                }
                if pc < 2
                    || !matches!(code.get(pc - 1), Some(Instruction::Dup))
                    || code
                        .get(pc - 2)
                        .is_none_or(|source| !is_derived_initialization_source(source))
                {
                    return Err("derived local initializer has no constructor result");
                }
                if explicit_targets.contains(&(pc - 1)) || explicit_targets.contains(&pc) {
                    return Err("derived local initializer protocol has a non-fallthrough entry");
                }
            }
            Instruction::InitializeDerivedVarRef(index) => {
                if derived || !metadata.super_call_allowed || !metadata.super_allowed {
                    return Err("captured derived initializer has no inherited super authority");
                }
                let Some(descriptor) = closure_variables.get(usize::from(*index)) else {
                    return Err("captured derived initializer is outside closure slots");
                };
                if descriptor.kind != ClosureVariableKind::Normal
                    || !descriptor.is_lexical
                    || descriptor.is_const
                {
                    return Err("captured derived initializer targets a non-mutable lexical cell");
                }
                if pc < 2
                    || !matches!(code.get(pc - 1), Some(Instruction::Dup))
                    || code
                        .get(pc - 2)
                        .is_none_or(|source| !is_derived_initialization_source(source))
                {
                    return Err("captured derived initializer has no constructor result");
                }
                if explicit_targets.contains(&(pc - 1)) || explicit_targets.contains(&pc) {
                    return Err(
                        "captured derived initializer protocol has a non-fallthrough entry",
                    );
                }
            }
            Instruction::CallClassInstanceInitializer => {
                let valid_base = base_initializer_hook == Some(pc);
                let valid_derived_local = derived
                    && pc >= 2
                    && matches!(
                        code.get(pc - 2),
                        Some(Instruction::InitializeDerivedLocal(local)) if Some(*local) == this_local
                    )
                    && matches!(
                        code.get(pc - 1),
                        Some(Instruction::GetLocal(local)) if Some(*local) == active_local
                    );
                let valid_relay = !derived
                    && metadata.super_call_allowed
                    && metadata.super_allowed
                    && pc >= 2
                    && matches!(
                        (code.get(pc - 2), code.get(pc - 1)),
                        (
                            Some(Instruction::InitializeDerivedVarRef(initialized)),
                            Some(Instruction::GetVarRef(read)),
                        ) if initialized != read
                            && closure_variables.get(usize::from(*read)).is_some_and(
                                |descriptor| descriptor.kind == ClosureVariableKind::Normal
                                    && !descriptor.is_lexical
                                    && !descriptor.is_const
                            )
                    );
                if !valid_base && !valid_derived_local && !valid_relay {
                    return Err("class instance initializer call has no authenticated receiver");
                }
                if explicit_targets.contains(&pc)
                    || explicit_targets.contains(&(pc.saturating_sub(1)))
                {
                    return Err("class instance initializer hook has a non-fallthrough entry");
                }
            }
            Instruction::ReturnDerived(local) => {
                if Some(*local) != this_local {
                    return Err("derived return targets another local");
                }
            }
            Instruction::TailCall(_)
            | Instruction::TailCallMethod(_)
            | Instruction::Return
            | Instruction::ReturnUndefined
                if derived =>
            {
                return Err("derived constructor contains an ordinary return");
            }
            _ => {}
        }

        let local = match instruction {
            Instruction::GetLocal(local)
            | Instruction::PutLocal(local)
            | Instruction::SetLocal(local)
            | Instruction::SetLocalUninitialized(local)
            | Instruction::GetLocalCheck(local)
            | Instruction::InitializeLocal(local)
            | Instruction::InitializeDerivedLocal(local)
            | Instruction::PutLocalCheck(local)
            | Instruction::SetLocalCheck(local)
            | Instruction::CloseLocal(local)
            | Instruction::ReturnDerived(local) => Some(*local),
            _ => None,
        };
        if active_local.is_some() && local == active_local {
            let authenticated_store = matches!(instruction, Instruction::PutLocal(local)
                if pc > 0
                    && Some(*local) == active_local
                    && matches!(code.get(pc - 1), Some(Instruction::PushActiveFunction)));
            if !authenticated_store && !matches!(instruction, Instruction::GetLocal(_)) {
                return Err("active-function local has an unauthenticated access");
            }
        }
        if this_local.is_some()
            && local == this_local
            && !matches!(
                instruction,
                Instruction::GetLocalCheck(_)
                    | Instruction::InitializeDerivedLocal(_)
                    | Instruction::ReturnDerived(_)
            )
        {
            return Err("derived this local has an unauthenticated access");
        }
    }

    if derived && active_initializations != 1 {
        return Err("derived constructor active-function initialization is not unique");
    }
    if derived && !active_initialized_at_entry {
        return Err("derived constructor active-function initialization is not at entry");
    }
    if default_initializers > 1 {
        return Err("derived constructor has more than one default initializer");
    }
    if default_initializers == 1 {
        let (Some(this), Some(active)) = (this_local, active_local) else {
            return Err("default-derived constructor lost its authenticated locals");
        };
        let exact_metadata = metadata.argument_count == 0
            && metadata.defined_argument_count == 0
            && metadata.rest_parameter.is_none()
            && metadata.rest_pattern_start.is_none()
            && metadata.parameter_environment_local_count == 0
            && metadata.pattern_argument_count == 0
            && metadata.parameter_pattern_end.is_none()
            && metadata.local_count == 2
            && metadata.function_name_local.is_none()
            && metadata.eval_variable_object_local.is_none()
            && metadata.closure_count == 0
            && !metadata.needs_home_object
            && metadata.eval_kind == EvalKind::None
            && metadata.function_kind == FunctionKind::Normal;
        let exact_code = matches!(
            code,
            [
                Instruction::PushActiveFunction,
                Instruction::PutLocal(active_target),
                Instruction::CheckCtor,
                Instruction::InitDerivedConstructor,
                Instruction::Dup,
                Instruction::InitializeDerivedLocal(this_target),
                Instruction::GetLocal(active_read),
                Instruction::CallClassInstanceInitializer,
                Instruction::ReturnDerived(return_target),
            ] if *active_target == active
                && *this_target == this
                && *active_read == active
                && *return_target == this
        );
        if !exact_metadata || !exact_code {
            return Err("default-derived constructor has no exact synthesized shape");
        }
    }
    Ok(())
}

/// Authenticate the non-visible function roles used by class element
/// initialization.  These functions execute as ordinary synchronous frames,
/// but they are never ordinary authored callables: no constructor/parameter or
/// eval ABI may leak into them, `super` property access is permitted, and
/// `super()` is not.
pub fn validate_class_initializer_bytecode_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
) -> Result<(), &'static str> {
    if metadata.class_private_brand
        && !matches!(
            metadata.class_initializer_kind,
            Some(ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements)
        )
    {
        return Err("private brand metadata escaped a class initializer");
    }
    let Some(_) = metadata.class_initializer_kind else {
        return Ok(());
    };
    if metadata.argument_count != 0
        || metadata.defined_argument_count != 0
        || metadata.rest_parameter.is_some()
        || metadata.rest_pattern_start.is_some()
        || metadata.parameter_environment_local_count != 0
        || metadata.pattern_argument_count != 0
        || metadata.parameter_pattern_end.is_some()
        || metadata.constructor_kind != ConstructorKind::None
        || metadata.has_prototype
        || !metadata.strict
        || metadata.super_call_allowed
        || !metadata.super_allowed
        || metadata.eval_kind != EvalKind::None
        || metadata.function_kind != FunctionKind::Normal
        || !metadata.arguments_forbidden
        || !metadata.needs_home_object
    {
        return Err("class initializer function metadata is malformed");
    }
    if code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::CheckCtor
                | Instruction::InitDerivedConstructor
                | Instruction::MarkSuperCall
                | Instruction::ConstructSuper(_)
                | Instruction::ApplySuper
                | Instruction::InitializeDerivedLocal(_)
                | Instruction::InitializeDerivedVarRef(_)
                | Instruction::ReturnDerived(_)
        )
    }) {
        return Err("constructor-only bytecode escaped into a class initializer");
    }
    if metadata.arguments_forbidden
        && code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Arguments(_)))
    {
        return Err("arguments object escaped into an arguments-forbidden function");
    }
    Ok(())
}

pub fn validate_parameter_bytecode_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    parameter_initializer_locals: &[bool],
    parameter_environment: Option<&ParameterEnvironmentLayout>,
) -> Result<Option<usize>, &'static str> {
    if parameter_initializer_locals.len() != usize::from(metadata.local_count) {
        return Err("parameter-initializer local classification has the wrong length");
    }
    let class_constructor_guard = validate_class_constructor_guard(metadata, code)?;
    if metadata.rest_parameter.is_some() && metadata.rest_pattern_start.is_some() {
        return Err("identifier rest and rest BindingPattern metadata overlap");
    }
    let maximum_defined_arguments = metadata
        .argument_count
        .checked_add(u16::from(metadata.rest_pattern_start.is_some()))
        .ok_or("defined argument count overflowed function argument slots")?;
    if metadata.defined_argument_count > maximum_defined_arguments {
        return Err("defined argument count exceeds function argument slots");
    }
    if metadata
        .rest_pattern_start
        .is_some_and(|start| start != metadata.argument_count)
    {
        return Err("rest BindingPattern metadata disagrees with argument slots");
    }
    if metadata.pattern_argument_count > metadata.argument_count {
        return Err("pattern argument count exceeds function argument slots");
    }
    let has_pattern_parameters =
        metadata.pattern_argument_count != 0 || metadata.rest_pattern_start.is_some();
    if let Some(end) = metadata.parameter_pattern_end {
        if !has_pattern_parameters {
            return Err("parameter initialization marker has no BindingPattern");
        }
        let end = usize::try_from(end)
            .map_err(|_| "parameter BindingPattern marker is outside bytecode")?;
        if !matches!(code.get(end), Some(Instruction::Nop)) {
            return Err("parameter BindingPattern marker is outside bytecode");
        }
    } else if has_pattern_parameters {
        return Err("parameter BindingPattern has no initialization marker");
    }

    let parameter_locals = metadata.parameter_environment_local_count;
    let mut explicit_body_pc = None;
    if let Some(layout) = parameter_environment {
        explicit_body_pc = validate_explicit_parameter_environment_layout(
            metadata,
            code,
            layout,
            class_constructor_guard,
        )?;
        let has_extended_parameter_entry = metadata.eval_variable_object_local.is_some()
            || layout.synthetic_arguments_local.is_some()
            || layout.arg_eval_variable_object_local.is_some()
            || code
                .iter()
                .any(|instruction| instruction.eval_environment().is_some());
        if has_pattern_parameters || parameter_locals == 0 || has_extended_parameter_entry {
            return Ok(explicit_body_pc);
        }
    }
    if parameter_environment.is_none() && parameter_locals != 0 {
        return Err("parameter-environment cells have no immutable layout");
    }
    if parameter_locals == 0 {
        if metadata.parameter_pattern_end.is_some() {
            return match (metadata.rest_parameter, metadata.rest_pattern_start) {
                (Some(rest), None)
                    if rest.checked_add(1) == Some(metadata.argument_count)
                        && metadata.defined_argument_count == rest =>
                {
                    Ok(None)
                }
                (Some(_), None) => Err("rest parameter metadata disagrees with argument slots"),
                (None, Some(start))
                    if metadata.defined_argument_count
                        == if quickjs_copies_defined_argument_count(
                            usize::from(metadata.argument_count),
                            usize::from(metadata.local_count),
                            code,
                        ) {
                            start.saturating_add(1)
                        } else {
                            0
                        } =>
                {
                    Ok(None)
                }
                (None, Some(_)) => {
                    Err("rest BindingPattern metadata disagrees with function length")
                }
                (None, None) if metadata.defined_argument_count == metadata.argument_count => {
                    Ok(None)
                }
                (None, None) => Err("default parameter metadata has no parameter environment"),
                (Some(_), Some(_)) => {
                    Err("identifier rest and rest BindingPattern metadata overlap")
                }
            };
        }
        return match metadata.rest_parameter {
            Some(rest)
                if rest.checked_add(1) == Some(metadata.argument_count)
                    && metadata.defined_argument_count == rest =>
            {
                let mut rest_pc = 0_usize;
                let mut pseudo_rank = 0_u8;
                let mut entry_targets = Vec::with_capacity(6);
                while let Some([source, Instruction::PutLocal(local)]) =
                    code.get(rest_pc..rest_pc + 2)
                {
                    let Some(rank) = pseudo_binding_entry_rank(source) else {
                        break;
                    };
                    if rank <= pseudo_rank
                        || *local >= metadata.local_count
                        || metadata.eval_variable_object_local == Some(*local)
                        || metadata.function_name_local == Some(*local)
                        || entry_targets.contains(local)
                    {
                        return Err("rest parameter contains a malformed pseudo-binding prologue");
                    }
                    pseudo_rank = rank;
                    entry_targets.push(*local);
                    rest_pc += 2;
                }
                let arguments = code
                    .iter()
                    .enumerate()
                    .filter_map(|(pc, instruction)| match instruction {
                        Instruction::Arguments(kind) => Some((pc, *kind)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                match arguments.as_slice() {
                    [] => {}
                    [(pc, crate::engine::code::bytecode::ArgumentsKind::Unmapped)]
                        if *pc == rest_pc =>
                    {
                        let Some(Instruction::PutLocal(local)) = code.get(rest_pc + 1) else {
                            return Err("rest parameter arguments object has no entry binding");
                        };
                        if *local >= metadata.local_count
                            || metadata.eval_variable_object_local == Some(*local)
                            || metadata.function_name_local == Some(*local)
                            || entry_targets.contains(local)
                        {
                            return Err("rest parameter arguments object has no entry binding");
                        }
                        entry_targets.push(*local);
                        rest_pc += 2;
                    }
                    _ => return Err("rest parameter contains a malformed arguments prologue"),
                }
                match metadata.eval_variable_object_local {
                    Some(local)
                        if local < metadata.local_count
                            && metadata.function_name_local != Some(local)
                            && !entry_targets.contains(&local)
                            && matches!(
                                code.get(rest_pc..rest_pc + 2),
                                Some([
                                    Instruction::VariableEnvironment,
                                    Instruction::PutLocal(target),
                                ]) if *target == local
                            ) =>
                    {
                        rest_pc += 2;
                    }
                    Some(_) => {
                        return Err("eval variable-object local has no exact entry prologue");
                    }
                    None => {}
                }
                if code
                    .iter()
                    .filter(|instruction| matches!(instruction, Instruction::VariableEnvironment))
                    .count()
                    != usize::from(metadata.eval_variable_object_local.is_some())
                {
                    return Err("variable-environment opcode has no authenticated local");
                }
                rest_pc = consume_class_constructor_guard(
                    metadata,
                    code,
                    class_constructor_guard,
                    rest_pc,
                )?;
                if !matches!(
                    code.get(rest_pc..rest_pc + 2),
                    Some([Instruction::Rest(start), Instruction::PutArg(target)])
                        if *start == rest && *target == rest
                ) || code.iter().enumerate().any(|(pc, instruction)| {
                    pc != rest_pc && matches!(instruction, Instruction::Rest(_))
                }) {
                    return Err("rest parameter has no exact entry initialization");
                }
                Ok(None)
            }
            Some(_) => Err("rest parameter metadata disagrees with argument slots"),
            None if metadata.defined_argument_count != metadata.argument_count => {
                Err("default parameter metadata has no parameter environment")
            }
            None if code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Rest(_))) =>
            {
                Err("rest opcode has no authenticated parameter metadata")
            }
            None => Ok(None),
        };
    }

    if metadata.pattern_argument_count != 0
        || metadata.parameter_pattern_end.is_some()
        || metadata.rest_pattern_start.is_some()
        || parameter_locals != metadata.argument_count
        || parameter_locals > metadata.local_count
        || metadata.defined_argument_count >= metadata.argument_count
        || metadata.rest_parameter.is_some_and(|rest| {
            rest.checked_add(1) != Some(metadata.argument_count)
                || metadata.defined_argument_count >= rest
        })
    {
        return Err("parameter environment metadata disagrees with function slots");
    }
    if metadata.eval_variable_object_local.is_some()
        || code
            .iter()
            .any(|instruction| instruction.eval_environment().is_some())
    {
        return Err("direct eval is not supported in a parameter environment");
    }

    let mut entry_pc = 0_usize;
    // QuickJS materializes owned HomeObject/active-function/new.target/this
    // cells before entering the argument scope when parameter code needs one.
    // The compiler emits the same fixed-order pairs before the arguments
    // object and the parameter TDZ reset; every target must remain outside the
    // leading parameter-local range.
    let mut pseudo_rank = 0_u8;
    let mut pseudo_targets = Vec::with_capacity(4);
    while let Some([source, Instruction::PutLocal(local)]) = code.get(entry_pc..entry_pc + 2) {
        let Some(rank) = pseudo_binding_entry_rank(source) else {
            break;
        };
        if rank <= pseudo_rank
            || *local < parameter_locals
            || *local >= metadata.local_count
            || metadata.function_name_local == Some(*local)
            || pseudo_targets.contains(local)
        {
            return Err("parameter environment contains a malformed pseudo-binding prologue");
        }
        pseudo_rank = rank;
        pseudo_targets.push(*local);
        entry_pc += 2;
    }

    let arguments = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| match instruction {
            Instruction::Arguments(kind) => Some((pc, *kind)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let arguments_local = match arguments.as_slice() {
        [] => None,
        [(pc, crate::engine::code::bytecode::ArgumentsKind::Unmapped)] if *pc == entry_pc => {
            let Some(Instruction::PutLocal(local)) = code.get(entry_pc + 1) else {
                return Err("parameter environment arguments object has no entry binding");
            };
            if *local < parameter_locals
                || *local >= metadata.local_count
                || metadata.function_name_local == Some(*local)
                || pseudo_targets.contains(local)
            {
                return Err("parameter environment arguments object has no entry binding");
            }
            entry_pc += 2;
            Some(*local)
        }
        _ => return Err("parameter environment contains a malformed arguments prologue"),
    };

    let parameter_count = usize::from(parameter_locals);
    for (offset, local) in (0..parameter_locals).rev().enumerate() {
        if !matches!(
            code.get(entry_pc + offset),
            Some(Instruction::SetLocalUninitialized(target)) if *target == local
        ) {
            return Err("parameter environment has no exact TDZ entry initialization");
        }
    }
    let parameter_body_pc = consume_class_constructor_guard(
        metadata,
        code,
        class_constructor_guard,
        entry_pc + parameter_count,
    )?;
    let mut initializer_pcs = vec![None; parameter_count];
    for (pc, instruction) in code.iter().enumerate() {
        let Instruction::InitializeLocal(target) = instruction else {
            continue;
        };
        let target = usize::from(*target);
        if target >= parameter_count {
            continue;
        }
        if initializer_pcs[target].replace(pc).is_some() {
            return Err("parameter cell does not have one exact initializer");
        }
    }
    let mut previous_initializer = None;
    for initializer in &initializer_pcs {
        let Some(pc) = *initializer else {
            return Err("parameter cell does not have one exact initializer");
        };
        if pc < parameter_body_pc || previous_initializer.is_some_and(|previous| previous >= pc) {
            return Err("parameter cells are not initialized left to right");
        }
        previous_initializer = Some(pc);
    }
    let initializer_pcs = initializer_pcs
        .into_iter()
        .map(|pc| pc.expect("parameter initializer presence checked above"))
        .collect::<Vec<_>>();
    if code.iter().enumerate().any(|(pc, instruction)| {
        matches!(instruction, Instruction::SetLocalUninitialized(local) if *local < parameter_locals)
            && !(entry_pc..parameter_body_pc).contains(&pc)
    }) {
        return Err("parameter cell has an unauthenticated TDZ reset");
    }

    // Consume the compiler's contiguous argument-initialization ABI. The
    // public `defined_argument_count` identifies the first default, while
    // later slots are distinguished by their exact plain/default skeleton.
    // This authenticates the branch which skips an initializer as well as the
    // raw argument-slot synchronization on the default path.
    let mut parameter_pc = parameter_body_pc;
    let mut authenticated_rest_pc = None;
    for (local, &initializer_pc) in (0..parameter_locals).zip(&initializer_pcs) {
        if metadata.rest_parameter == Some(local) {
            if !matches!(
                code.get(parameter_pc..parameter_pc + 4),
                Some([
                    Instruction::Rest(start),
                    Instruction::Dup,
                    Instruction::PutArg(argument),
                    Instruction::InitializeLocal(target),
                ]) if *start == local && *argument == local && *target == local
            ) || initializer_pc != parameter_pc + 3
            {
                return Err("parameter-environment rest parameter has no exact initialization");
            }
            authenticated_rest_pc = Some(parameter_pc);
            parameter_pc += 4;
            continue;
        }

        let plain = matches!(
            code.get(parameter_pc..parameter_pc + 2),
            Some([Instruction::GetArg(argument), Instruction::InitializeLocal(target)])
                if *argument == local && *target == local
        ) && initializer_pc == parameter_pc + 1;
        if plain {
            if local == metadata.defined_argument_count {
                return Err("first default parameter has no default entry initialization");
            }
            parameter_pc += 2;
            continue;
        }
        if local < metadata.defined_argument_count {
            return Err("leading plain parameter has no exact entry initialization");
        }

        let Some(default_header_end) = parameter_pc.checked_add(6) else {
            return Err("default parameter entry initialization overflowed bytecode");
        };
        if !matches!(
            code.get(parameter_pc..default_header_end),
            Some([
                Instruction::GetArg(argument),
                Instruction::Dup,
                Instruction::Undefined,
                Instruction::StrictEq,
                Instruction::IfFalse(target),
                Instruction::Drop,
            ]) if *argument == local && usize::try_from(*target).ok() == Some(initializer_pc)
        ) {
            return Err("default parameter has no exact selection branch");
        }
        let Some(sync_pc) = initializer_pc.checked_sub(2) else {
            return Err("default parameter has no exact argument synchronization");
        };
        if sync_pc <= default_header_end
            || !matches!(
                code.get(sync_pc..initializer_pc + 1),
                Some([
                    Instruction::Dup,
                    Instruction::PutArg(argument),
                    Instruction::InitializeLocal(target),
                ]) if *argument == local && *target == local
            )
        {
            return Err("default parameter has no exact argument synchronization");
        }
        for instruction in &code[default_header_end..sync_pc] {
            if matches!(
                instruction,
                Instruction::GetArg(_)
                    | Instruction::PutArg(_)
                    | Instruction::SetArg(_)
                    | Instruction::Rest(_)
            ) {
                return Err("default parameter initializer bypasses parameter cells");
            }
            let unauthenticated_local_access = match instruction {
                Instruction::GetLocal(target) => {
                    *target < parameter_locals
                        || (arguments_local != Some(*target)
                            && metadata.function_name_local != Some(*target)
                            && !pseudo_targets.contains(target))
                }
                Instruction::PutLocal(target) | Instruction::SetLocal(target) => {
                    arguments_local != Some(*target)
                }
                Instruction::GetLocalCheck(target) => {
                    *target >= parameter_locals
                        && metadata.derived_this_local != Some(*target)
                        && !parameter_initializer_locals
                            .get(usize::from(*target))
                            .copied()
                            .unwrap_or(false)
                }
                Instruction::PutLocalCheck(target) | Instruction::SetLocalCheck(target) => {
                    *target >= parameter_locals
                        && !parameter_initializer_locals
                            .get(usize::from(*target))
                            .copied()
                            .unwrap_or(false)
                }
                Instruction::InitializeDerivedLocal(target) => {
                    metadata.derived_this_local != Some(*target)
                }
                // Closure provenance and inherited super authority are
                // authenticated by `validate_derived_constructor_bytecode_layout`.
                Instruction::InitializeDerivedVarRef(_) => false,
                Instruction::SetLocalUninitialized(_)
                | Instruction::InitializeLocal(_)
                | Instruction::CloseLocal(_) => match instruction {
                    Instruction::SetLocalUninitialized(target)
                    | Instruction::InitializeLocal(target)
                    | Instruction::CloseLocal(target) => !parameter_initializer_locals
                        .get(usize::from(*target))
                        .copied()
                        .unwrap_or(false),
                    _ => unreachable!("matched parameter-initializer lifecycle opcode"),
                },
                _ => false,
            };
            if unauthenticated_local_access {
                return Err("default parameter initializer has an unauthenticated local access");
            }
            let target = match instruction {
                Instruction::Goto(target)
                | Instruction::IfFalse(target)
                | Instruction::IfTrue(target)
                | Instruction::Catch(target)
                | Instruction::Gosub(target) => Some(*target),
                _ => None,
            };
            if target.is_some_and(|target| {
                usize::try_from(target)
                    .ok()
                    .is_none_or(|target| !(default_header_end..=sync_pc).contains(&target))
            }) {
                return Err("default parameter initializer escaped its entry segment");
            }
        }
        parameter_pc = initializer_pc + 1;
    }

    let rest_pcs = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| matches!(instruction, Instruction::Rest(_)).then_some(pc))
        .collect::<Vec<_>>();
    match (metadata.rest_parameter, authenticated_rest_pc) {
        (Some(_), Some(expected)) if rest_pcs.as_slice() == [expected] => {}
        (Some(_), _) => {
            return Err("parameter-environment rest parameter is not unique");
        }
        (None, None) if rest_pcs.is_empty() => {}
        (None, None) => {
            return Err("rest opcode has no authenticated parameter metadata");
        }
        (None, Some(_)) => {
            return Err("rest opcode has no authenticated parameter metadata");
        }
    }
    let mut body_pc = parameter_pc;
    if let Some(layout) = parameter_environment {
        if usize::try_from(layout.initialization_end).ok() != Some(body_pc)
            || !matches!(code.get(body_pc), Some(Instruction::Nop))
        {
            return Err("parameter environment marker does not follow exact initialization");
        }
        body_pc += 1;
    } else if explicit_body_pc.is_some() {
        return Err("parameter environment lost its immutable layout");
    }
    let mut closed_parameter_cells = vec![false; parameter_count];
    while let Some(Instruction::CloseLocal(target)) = code.get(body_pc) {
        let target = usize::from(*target);
        if target >= parameter_count {
            break;
        }
        if std::mem::replace(&mut closed_parameter_cells[target], true) {
            return Err("parameter cell is closed more than once");
        }
        body_pc += 1;
    }
    if code[body_pc..].iter().any(|instruction| {
        let target = match instruction {
            Instruction::GetLocal(target)
            | Instruction::PutLocal(target)
            | Instruction::SetLocal(target)
            | Instruction::SetLocalUninitialized(target)
            | Instruction::GetLocalCheck(target)
            | Instruction::InitializeLocal(target)
            | Instruction::PutLocalCheck(target)
            | Instruction::SetLocalCheck(target)
            | Instruction::CloseLocal(target) => Some(*target),
            _ => None,
        };
        target.is_some_and(|target| target < parameter_locals)
    }) {
        return Err("function body accesses a parameter-environment cell");
    }
    if code.iter().skip(body_pc).any(|instruction| {
        let target = match instruction {
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target) => Some(*target),
            _ => None,
        };
        target.is_some_and(|target| {
            usize::try_from(target)
                .ok()
                .is_some_and(|target| target < body_pc)
        })
    }) {
        return Err("function body jumps back into parameter initialization");
    }
    Ok(Some(body_pc))
}

fn validate_explicit_parameter_environment_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    layout: &ParameterEnvironmentLayout,
    class_constructor_guard: Option<usize>,
) -> Result<Option<usize>, &'static str> {
    let parameter_locals = metadata.parameter_environment_local_count;
    if parameter_locals > metadata.local_count {
        return Err("parameter environment exceeds function local slots");
    }
    let marker = usize::try_from(layout.initialization_end)
        .map_err(|_| "parameter environment marker is outside bytecode")?;
    if !matches!(code.get(marker), Some(Instruction::Nop)) {
        return Err("parameter environment marker is outside bytecode");
    }
    if code[..marker].iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::InitDerivedConstructor | Instruction::ReturnDerived(_)
        )
    }) {
        return Err("constructor completion protocol escaped into parameter initialization");
    }
    let has_pattern = metadata.pattern_argument_count != 0 || metadata.rest_pattern_start.is_some();
    match (has_pattern, metadata.parameter_pattern_end) {
        (true, Some(pattern_end)) if pattern_end == layout.initialization_end => {}
        (true, _) => return Err("parameter BindingPattern marker disagrees with its environment"),
        (false, None) => {}
        (false, Some(_)) => return Err("parameter marker has no BindingPattern"),
    }

    let expected_cells = layout
        .argument_cells
        .len()
        .checked_add(layout.pattern_copies.len())
        .ok_or("parameter environment cell count overflowed")?;
    if expected_cells != usize::from(parameter_locals) {
        return Err("parameter environment layout does not cover every cell");
    }
    let mut cell_roles = vec![false; usize::from(parameter_locals)];
    let mut arguments = vec![false; usize::from(metadata.argument_count)];
    let mut defaulted_arguments = vec![false; usize::from(metadata.argument_count)];
    let mut rest_pattern_defaulted = false;
    let mut previous_default = None;
    for source in layout.default_sources.iter().copied() {
        let formal = match source {
            ParameterDefaultSource::Argument(argument) => {
                let argument_index = usize::from(argument);
                if argument_index >= defaulted_arguments.len()
                    || std::mem::replace(&mut defaulted_arguments[argument_index], true)
                    || metadata.rest_parameter == Some(argument)
                {
                    return Err("parameter default source overlaps or is out of bounds");
                }
                argument
            }
            ParameterDefaultSource::RestPattern(start)
                if metadata.rest_pattern_start == Some(start) && !rest_pattern_defaulted =>
            {
                rest_pattern_defaulted = true;
                start
            }
            ParameterDefaultSource::RestPattern(_) => {
                return Err("parameter default source disagrees with rest BindingPattern");
            }
        };
        if previous_default.is_some_and(|previous| previous >= formal) {
            return Err("parameter default sources are not in formal order");
        }
        previous_default = Some(formal);
    }
    let expected_defined_arguments = if let Some(first_default) =
        layout.default_sources.first().map(|source| match source {
            ParameterDefaultSource::Argument(argument)
            | ParameterDefaultSource::RestPattern(argument) => *argument,
        }) {
        first_default
    } else {
        match (metadata.rest_parameter, metadata.rest_pattern_start) {
            (Some(rest), None) => rest,
            (None, Some(start))
                if quickjs_copies_defined_argument_count(
                    usize::from(metadata.argument_count),
                    usize::from(metadata.local_count),
                    code,
                ) =>
            {
                start
                    .checked_add(1)
                    .ok_or("defined argument count overflowed function argument slots")?
            }
            (None, Some(_)) => 0,
            (None, None) => metadata.argument_count,
            (Some(_), Some(_)) => {
                return Err("identifier rest and rest BindingPattern metadata overlap");
            }
        }
    };
    if metadata.defined_argument_count != expected_defined_arguments {
        return Err("parameter default sources disagree with function length");
    }
    let mut body_targets = vec![false; usize::from(metadata.local_count)];
    for cell in layout.argument_cells.iter() {
        let local = usize::from(cell.parameter_local);
        let argument = usize::from(cell.argument);
        if local >= cell_roles.len()
            || std::mem::replace(&mut cell_roles[local], true)
            || argument >= arguments.len()
            || std::mem::replace(&mut arguments[argument], true)
        {
            return Err("parameter argument cell mapping overlaps or is out of bounds");
        }
        match cell.body {
            ParameterBodyStorage::Argument(body) if body == cell.argument => {}
            ParameterBodyStorage::Argument(_) => {
                return Err("parameter body argument mapping changed physical slots");
            }
            ParameterBodyStorage::Local(_) => {
                return Err("parameter body local storage requires direct-eval support");
            }
        }
    }
    for copy in layout.pattern_copies.iter() {
        let source = usize::from(copy.parameter_local);
        if source >= cell_roles.len() || std::mem::replace(&mut cell_roles[source], true) {
            return Err("parameter pattern copy source overlaps or is out of bounds");
        }
        let target = usize::from(copy.body_local);
        if copy.body_local < parameter_locals
            || target >= body_targets.len()
            || std::mem::replace(&mut body_targets[target], true)
        {
            return Err("parameter pattern copy target overlaps or is out of bounds");
        }
    }
    if let Some(local) = layout.synthetic_arguments_local {
        let local = usize::from(local);
        if local < cell_roles.len()
            || local >= usize::from(metadata.local_count)
            || metadata.eval_variable_object_local == u16::try_from(local).ok()
            || metadata.function_name_local == u16::try_from(local).ok()
            || body_targets.get(local).copied().unwrap_or(false)
        {
            return Err("synthetic parameter arguments cell overlaps or is out of bounds");
        }
    }
    if let Some(local) = layout.arg_eval_variable_object_local {
        let local = usize::from(local);
        if local < cell_roles.len()
            || local >= usize::from(metadata.local_count)
            || layout.synthetic_arguments_local == u16::try_from(local).ok()
            || metadata.eval_variable_object_local == u16::try_from(local).ok()
            || metadata.function_name_local == u16::try_from(local).ok()
            || body_targets.get(local).copied().unwrap_or(false)
        {
            return Err("parameter eval variable-object local overlaps or is out of bounds");
        }
        if metadata.strict || metadata.eval_variable_object_local.is_none() {
            return Err("parameter eval variable object escaped a sloppy eval-enabled function");
        }
    }
    if cell_roles.iter().any(|covered| !covered) {
        return Err("parameter environment layout left an untyped cell");
    }

    let mut entry_pc = 0_usize;
    let mut pseudo_rank = 0_u8;
    let mut pseudo_targets = Vec::with_capacity(4);
    while let Some([source, Instruction::PutLocal(local)]) = code.get(entry_pc..entry_pc + 2) {
        let Some(rank) = pseudo_binding_entry_rank(source) else {
            break;
        };
        if rank <= pseudo_rank
            || *local < parameter_locals
            || *local >= metadata.local_count
            || pseudo_targets.contains(local)
            || metadata.eval_variable_object_local == Some(*local)
            || layout.synthetic_arguments_local == Some(*local)
            || layout.arg_eval_variable_object_local == Some(*local)
            || metadata.function_name_local == Some(*local)
            || body_targets
                .get(usize::from(*local))
                .copied()
                .unwrap_or(false)
        {
            return Err("parameter environment contains a malformed pseudo-binding prologue");
        }
        pseudo_rank = rank;
        pseudo_targets.push(*local);
        entry_pc += 2;
    }

    let arguments_prologues = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| match instruction {
            Instruction::Arguments(kind) => Some((pc, *kind)),
            _ => None,
        })
        .collect::<Vec<_>>();
    match layout.synthetic_arguments_local {
        None => match arguments_prologues.as_slice() {
            [] => {}
            [(pc, crate::engine::code::bytecode::ArgumentsKind::Unmapped)] => {
                let Some(Instruction::PutLocal(local)) = code.get(entry_pc + 1) else {
                    return Err("parameter environment arguments object has no entry binding");
                };
                if *pc != entry_pc {
                    return Err("parameter environment arguments object is not at function entry");
                }
                if *local < parameter_locals
                    || *local >= metadata.local_count
                    || metadata.eval_variable_object_local == Some(*local)
                    || layout.arg_eval_variable_object_local == Some(*local)
                    || metadata.function_name_local == Some(*local)
                    || pseudo_targets.contains(local)
                {
                    return Err("parameter environment arguments object has no entry binding");
                }
                entry_pc += 2;
            }
            _ => return Err("parameter environment contains a malformed arguments prologue"),
        },
        Some(synthetic) => {
            if !matches!(
                arguments_prologues.as_slice(),
                [(pc, crate::engine::code::bytecode::ArgumentsKind::Unmapped)] if *pc == entry_pc
            ) || !matches!(
                code.get(entry_pc..entry_pc + 4),
                Some([
                    Instruction::Arguments(crate::engine::code::bytecode::ArgumentsKind::Unmapped),
                    Instruction::Dup,
                    Instruction::InitializeLocal(target),
                    Instruction::PutLocal(body),
                ]) if *target == synthetic
                    && *body >= parameter_locals
                    && *body < metadata.local_count
                    && *body != synthetic
                    && metadata.eval_variable_object_local != Some(*body)
                    && layout.arg_eval_variable_object_local != Some(*body)
                    && metadata.function_name_local != Some(*body)
                    && !pseudo_targets.contains(body)
            ) {
                return Err("parameter environment contains a malformed arguments prologue");
            }
            entry_pc += 4;
        }
    }

    let mut variable_environment_targets = Vec::with_capacity(2);
    for expected in [
        metadata.eval_variable_object_local,
        layout.arg_eval_variable_object_local,
    ]
    .into_iter()
    .flatten()
    {
        if !matches!(
            code.get(entry_pc..entry_pc + 2),
            Some([Instruction::VariableEnvironment, Instruction::PutLocal(target)])
                if *target == expected
        ) {
            return Err("eval variable-object local has no exact entry prologue");
        }
        variable_environment_targets.push(expected);
        entry_pc += 2;
    }
    if code
        .iter()
        .filter(|instruction| matches!(instruction, Instruction::VariableEnvironment))
        .count()
        != variable_environment_targets.len()
    {
        return Err("variable-environment opcode has no authenticated local");
    }

    for (offset, local) in (0..parameter_locals).rev().enumerate() {
        if !matches!(
            code.get(entry_pc + offset),
            Some(Instruction::SetLocalUninitialized(target)) if *target == local
        ) {
            return Err("parameter environment has no exact TDZ entry initialization");
        }
    }
    let initialization_pc = consume_class_constructor_guard(
        metadata,
        code,
        class_constructor_guard,
        entry_pc + usize::from(parameter_locals),
    )?;
    if initialization_pc > marker {
        return Err("parameter environment marker precedes its TDZ prologue");
    }

    let mut initializer_pcs = vec![None; usize::from(parameter_locals)];
    for (pc, instruction) in code.iter().enumerate() {
        let Instruction::InitializeLocal(local) = instruction else {
            continue;
        };
        let local = usize::from(*local);
        if local >= initializer_pcs.len() {
            continue;
        }
        if pc < initialization_pc || pc >= marker || initializer_pcs[local].replace(pc).is_some() {
            return Err("parameter cell does not have one exact initializer");
        }
    }
    let mut previous = None;
    for &initializer in &initializer_pcs {
        let Some(initializer) = initializer else {
            return Err("parameter cell does not have one exact initializer");
        };
        if previous.is_some_and(|previous| previous >= initializer) {
            return Err("parameter cells are not initialized in BoundName order");
        }
        previous = Some(initializer);
    }
    if code.iter().enumerate().any(|(pc, instruction)| {
        matches!(instruction, Instruction::SetLocalUninitialized(local) if *local < parameter_locals)
            && !(entry_pc..initialization_pc).contains(&pc)
    }) {
        return Err("parameter cell has an unauthenticated TDZ reset");
    }

    let copy_start = marker
        .checked_sub(layout.pattern_copies.len().saturating_mul(2))
        .ok_or("parameter copy phase begins outside bytecode")?;
    for (offset, copy) in layout.pattern_copies.iter().rev().enumerate() {
        let pc = copy_start + offset * 2;
        if !matches!(
            code.get(pc..pc + 2),
            Some([Instruction::GetLocalCheck(source), Instruction::PutLocal(target)])
                if *source == copy.parameter_local && *target == copy.body_local
        ) {
            return Err("parameter pattern copy phase disagrees with immutable layout");
        }
    }
    if copy_start < initialization_pc {
        return Err("parameter pattern copy phase overlaps the TDZ prologue");
    }

    for cell in layout.argument_cells.iter() {
        let argument = cell.argument;
        let argument_index = usize::from(argument);
        let parameter_local = usize::from(cell.parameter_local);
        let initializer_pc = initializer_pcs[parameter_local]
            .ok_or("parameter argument cell has no exact initialization")?;
        let raw_reads = code[..marker]
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::GetArg(source) if *source == argument)
                    .then_some(pc)
            })
            .collect::<Vec<_>>();
        let raw_writes = code[..marker]
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::PutArg(target) | Instruction::SetArg(target) if *target == argument)
                    .then_some(pc)
            })
            .collect::<Vec<_>>();

        if metadata.rest_parameter == Some(argument) {
            if defaulted_arguments[argument_index]
                || !raw_reads.is_empty()
                || raw_writes.as_slice() != [initializer_pc.saturating_sub(1)]
                || initializer_pc < 3
                || !matches!(
                    code.get(initializer_pc - 3..=initializer_pc),
                    Some([
                        Instruction::Rest(start),
                        Instruction::Dup,
                        Instruction::PutArg(target),
                        Instruction::InitializeLocal(local),
                    ]) if *start == argument && *target == argument && usize::from(*local) == parameter_local
                )
            {
                return Err("parameter rest cell has no exact initialization");
            }
            continue;
        }

        if defaulted_arguments[argument_index] {
            let Some(&source_pc) = raw_reads.first().filter(|_| raw_reads.len() == 1) else {
                return Err("parameter default cell has no exact argument selection");
            };
            if raw_writes.as_slice() != [initializer_pc.saturating_sub(1)]
                || initializer_pc < 2
                || !matches!(
                    code.get(source_pc..source_pc + 5),
                    Some([
                        Instruction::GetArg(source),
                        Instruction::Dup,
                        Instruction::Undefined,
                        Instruction::StrictEq,
                        Instruction::IfFalse(target),
                    ]) if *source == argument && usize::try_from(*target).ok() == Some(initializer_pc)
                )
                || !matches!(
                    code.get(initializer_pc - 2..=initializer_pc),
                    Some([
                        Instruction::Dup,
                        Instruction::PutArg(target),
                        Instruction::InitializeLocal(local),
                    ]) if *target == argument && usize::from(*local) == parameter_local
                )
            {
                return Err("parameter default cell has no exact argument selection");
            }
        } else if raw_reads.as_slice() != [initializer_pc.saturating_sub(1)]
            || !raw_writes.is_empty()
            || initializer_pc == 0
            || !matches!(
                code.get(initializer_pc - 1..=initializer_pc),
                Some([
                    Instruction::GetArg(source),
                    Instruction::InitializeLocal(local),
                ]) if *source == argument && usize::from(*local) == parameter_local
            )
        {
            return Err("plain parameter argument cell has no exact initialization");
        }
    }

    if has_pattern {
        if arguments.iter().filter(|mapped| !**mapped).count()
            != usize::from(metadata.pattern_argument_count)
        {
            return Err("parameter argument-cell map disagrees with BindingPattern slots");
        }
        let validate_pattern_default = |source_pc: usize,
                                        expected: bool|
         -> Result<(), &'static str> {
            let initializer_pc = match code.get(source_pc + 1..source_pc + 5) {
                Some(
                    [
                        Instruction::Dup,
                        Instruction::Undefined,
                        Instruction::StrictEq,
                        Instruction::IfTrue(target),
                    ],
                ) => usize::try_from(*target).ok(),
                _ => None,
            };
            if !expected {
                return if initializer_pc.is_some() {
                    Err("BindingPattern has an unauthenticated top-level initializer")
                } else {
                    Ok(())
                };
            }
            let Some(initializer_pc) = initializer_pc else {
                return Err("BindingPattern default has no exact argument selection");
            };
            let assignment_pc = source_pc + 5;
            let Some(Instruction::Goto(done_pc)) =
                initializer_pc.checked_sub(1).and_then(|pc| code.get(pc))
            else {
                return Err("BindingPattern default has no exact argument selection");
            };
            let Some(done_pc) = usize::try_from(*done_pc).ok() else {
                return Err("BindingPattern default has no exact argument selection");
            };
            if initializer_pc <= assignment_pc
                || initializer_pc >= copy_start
                || !matches!(code.get(initializer_pc), Some(Instruction::Drop))
                || done_pc <= initializer_pc
                || done_pc > copy_start
                || !matches!(
                    done_pc.checked_sub(1).and_then(|pc| code.get(pc)),
                    Some(Instruction::Goto(target)) if usize::try_from(*target).ok() == Some(assignment_pc)
                )
            {
                return Err("BindingPattern default has no exact argument selection");
            }
            Ok(())
        };
        for (argument, mapped) in arguments.iter().copied().enumerate() {
            if mapped {
                continue;
            }
            let argument = u16::try_from(argument)
                .map_err(|_| "parameter BindingPattern argument is out of bounds")?;
            let mut sources =
                code[..copy_start]
                    .iter()
                    .enumerate()
                    .filter_map(|(pc, instruction)| {
                        matches!(instruction, Instruction::GetArg(source) if *source == argument)
                            .then_some(pc)
                    });
            let Some(source_pc) = sources.next() else {
                return Err("parameter BindingPattern has no exact raw argument source");
            };
            if sources.next().is_some()
            || code[..marker].iter().any(|instruction| {
                matches!(instruction, Instruction::PutArg(target) | Instruction::SetArg(target) if *target == argument)
            })
        {
            return Err("parameter BindingPattern bypasses its anonymous argument slot");
        }
            validate_pattern_default(source_pc, defaulted_arguments[usize::from(argument)])?;
        }
        if let Some(start) = metadata.rest_pattern_start {
            let mut sources =
                code[..copy_start]
                    .iter()
                    .enumerate()
                    .filter_map(|(pc, instruction)| {
                        matches!(instruction, Instruction::Rest(source) if *source == start)
                            .then_some(pc)
                    });
            let Some(source_pc) = sources.next() else {
                return Err("rest BindingPattern has no exact raw argument source");
            };
            if sources.next().is_some() {
                return Err("rest BindingPattern bypasses its raw argument source");
            }
            validate_pattern_default(source_pc, rest_pattern_defaulted)?;
        }
    }

    let rest_pcs = code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| match instruction {
            Instruction::Rest(start) => Some((pc, *start)),
            _ => None,
        })
        .collect::<Vec<_>>();
    match (metadata.rest_parameter, metadata.rest_pattern_start) {
        (Some(rest), None) if matches!(rest_pcs.as_slice(), [(pc, start)] if *pc < marker && *start == rest) =>
            {}
        (None, Some(rest)) if matches!(rest_pcs.as_slice(), [(pc, start)] if *pc < marker && *start == rest) =>
            {}
        (None, None) if rest_pcs.is_empty() => {}
        (Some(_), Some(_)) => {
            return Err("identifier rest and rest BindingPattern metadata overlap");
        }
        _ => return Err("parameter environment rest opcode disagrees with metadata"),
    }

    for (pc, instruction) in code.iter().enumerate() {
        let target = match instruction {
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target) => usize::try_from(*target).ok(),
            _ => None,
        };
        if target.is_some_and(|target| {
            (pc < copy_start && target > copy_start) || (pc > marker && target <= marker)
        }) {
            return Err("bytecode crosses the parameter environment boundary");
        }
    }

    let mut body_pc = marker + 1;
    let mut parameter_owned = vec![false; usize::from(metadata.local_count)];
    parameter_owned[..usize::from(parameter_locals)].fill(true);
    if let Some(local) = layout.synthetic_arguments_local {
        parameter_owned[usize::from(local)] = true;
    }
    let mut closed = vec![false; usize::from(metadata.local_count)];
    while let Some(Instruction::CloseLocal(local)) = code.get(body_pc) {
        let local = usize::from(*local);
        if !parameter_owned.get(local).copied().unwrap_or(false) {
            break;
        }
        if std::mem::replace(&mut closed[local], true) {
            return Err("parameter cell is closed more than once");
        }
        body_pc += 1;
    }
    if code[body_pc..].iter().any(|instruction| {
        let local = match instruction {
            Instruction::GetLocal(local)
            | Instruction::PutLocal(local)
            | Instruction::SetLocal(local)
            | Instruction::SetLocalUninitialized(local)
            | Instruction::GetLocalCheck(local)
            | Instruction::InitializeLocal(local)
            | Instruction::PutLocalCheck(local)
            | Instruction::SetLocalCheck(local)
            | Instruction::CloseLocal(local) => Some(*local),
            _ => None,
        };
        local.is_some_and(|local| {
            parameter_owned
                .get(usize::from(local))
                .copied()
                .unwrap_or(false)
        })
    }) {
        return Err("function body accesses a parameter-environment cell");
    }
    Ok(Some(body_pc))
}

/// Authenticate the source-only argument slots and entry segment used by a
/// formal-parameter BindingPattern.
///
/// QuickJS gives an ordinary BindingPattern one anonymous physical argument
/// slot, while a terminal rest BindingPattern owns no slot at all. The public
/// argument definitions are therefore required to agree with the bytecode
/// segment instead of treating a missing argument name as harmless debug
/// metadata. `parameter_pattern_end` is the compiler-authored boundary: entry
/// destructuring may branch within it, but the function body cannot re-enter
/// it and cannot read an anonymous raw argument after destructuring.
pub fn validate_pattern_parameter_bytecode_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    unnamed_arguments: &[bool],
    lexical_locals: &[bool],
    parameter_initializer_locals: &[bool],
    parameter_environment: Option<&ParameterEnvironmentLayout>,
) -> Result<(), &'static str> {
    if unnamed_arguments.len() != usize::from(metadata.argument_count) {
        return Err("argument definition count does not match bytecode metadata");
    }
    if lexical_locals.len() != usize::from(metadata.local_count) {
        return Err("local definition count does not match bytecode metadata");
    }
    if parameter_initializer_locals.len() != usize::from(metadata.local_count) {
        return Err("parameter-initializer local classification has the wrong length");
    }
    if parameter_initializer_locals
        .iter()
        .enumerate()
        .any(|(index, initializer)| {
            *initializer
                && (index < usize::from(metadata.parameter_environment_local_count)
                    || !lexical_locals[index])
        })
    {
        return Err("parameter-initializer classification names a non-nested lexical local");
    }

    let has_pattern = metadata.pattern_argument_count != 0 || metadata.rest_pattern_start.is_some();
    if !has_pattern {
        return if metadata.parameter_pattern_end.is_some() {
            Err("parameter initialization marker has no BindingPattern")
        } else {
            Ok(())
        };
    }
    if unnamed_arguments.iter().filter(|unnamed| **unnamed).count()
        != usize::from(metadata.pattern_argument_count)
    {
        return Err("pattern argument definitions disagree with bytecode metadata");
    }
    let Some(marker) = metadata.parameter_pattern_end else {
        return Err("parameter BindingPattern has no initialization marker");
    };
    let marker = usize::try_from(marker)
        .map_err(|_| "parameter BindingPattern marker is outside bytecode")?;
    if !matches!(code.get(marker), Some(Instruction::Nop)) {
        return Err("parameter BindingPattern marker is outside bytecode");
    }
    if code[..marker].iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::InitDerivedConstructor | Instruction::ReturnDerived(_)
        )
    }) {
        return Err("constructor completion protocol escaped into parameter initialization");
    }

    if let Some(rest) = metadata.rest_parameter {
        if unnamed_arguments
            .get(usize::from(rest))
            .copied()
            .unwrap_or(true)
        {
            return Err("identifier rest parameter has no named argument slot");
        }
    }

    let synthetic_arguments_local =
        parameter_environment.and_then(|layout| layout.synthetic_arguments_local);
    let mut synthetic_arguments_initialization_pc = None;
    let mut arguments_pcs =
        code.iter()
            .enumerate()
            .filter_map(|(pc, instruction)| match instruction {
                Instruction::Arguments(kind) => Some((pc, *kind)),
                _ => None,
            });
    let mut expected_pc = 0_usize;
    let mut pseudo_rank = 0_u8;
    let mut pseudo_targets = Vec::with_capacity(4);
    while let Some([source, Instruction::PutLocal(local)]) = code.get(expected_pc..expected_pc + 2)
    {
        let Some(rank) = pseudo_binding_entry_rank(source) else {
            break;
        };
        if rank <= pseudo_rank || *local >= metadata.local_count || pseudo_targets.contains(local) {
            return Err("parameter BindingPattern contains a malformed pseudo-binding prologue");
        }
        pseudo_rank = rank;
        pseudo_targets.push(*local);
        expected_pc += 2;
    }
    if let Some((pc, kind)) = arguments_pcs.next() {
        let arguments_shape_matches = match synthetic_arguments_local {
            Some(synthetic) => matches!(
                code.get(pc..pc + 4),
                Some([
                    Instruction::Arguments(crate::engine::code::bytecode::ArgumentsKind::Unmapped),
                    Instruction::Dup,
                    Instruction::InitializeLocal(target),
                    Instruction::PutLocal(body),
                ]) if *target == synthetic && *body < metadata.local_count
            ),
            None => matches!(
                code.get(pc + 1),
                Some(Instruction::PutLocal(local)) if *local < metadata.local_count
            ),
        };
        if arguments_pcs.next().is_some()
            || pc != expected_pc
            || pc >= marker
            || kind != crate::engine::code::bytecode::ArgumentsKind::Unmapped
            || !arguments_shape_matches
        {
            return Err("parameter BindingPattern contains a malformed arguments prologue");
        }
        if synthetic_arguments_local.is_some() {
            synthetic_arguments_initialization_pc = Some(pc + 2);
        }
    }

    let expected_unnamed_reads = unnamed_arguments
        .iter()
        .enumerate()
        .filter_map(|(argument, unnamed)| unnamed.then_some(argument))
        .collect::<Vec<_>>();
    let mut unnamed_reads = Vec::with_capacity(expected_unnamed_reads.len());
    let mut rest_operations = Vec::new();
    for (pc, instruction) in code.iter().enumerate() {
        let local = match instruction {
            Instruction::GetLocal(local)
            | Instruction::PutLocal(local)
            | Instruction::SetLocal(local)
            | Instruction::SetLocalUninitialized(local)
            | Instruction::GetLocalCheck(local)
            | Instruction::InitializeLocal(local)
            | Instruction::InitializeDerivedLocal(local)
            | Instruction::PutLocalCheck(local)
            | Instruction::SetLocalCheck(local)
            | Instruction::CloseLocal(local) => Some(*local),
            _ => None,
        };
        let is_synthetic_arguments_access = match instruction {
            Instruction::InitializeLocal(local) => {
                synthetic_arguments_local == Some(*local)
                    && synthetic_arguments_initialization_pc == Some(pc)
            }
            Instruction::GetLocalCheck(local)
            | Instruction::PutLocalCheck(local)
            | Instruction::SetLocalCheck(local) => synthetic_arguments_local == Some(*local),
            _ => false,
        };
        if pc < marker
            && !is_synthetic_arguments_access
            && local.is_some_and(|local| {
                local >= metadata.parameter_environment_local_count
                    && metadata.derived_this_local != Some(local)
                    && lexical_locals
                        .get(usize::from(local))
                        .copied()
                        .unwrap_or(false)
                    && !parameter_initializer_locals
                        .get(usize::from(local))
                        .copied()
                        .unwrap_or(false)
            })
        {
            return Err("parameter BindingPattern bytecode accessed a body lexical local");
        }

        match instruction {
            Instruction::GetArg(argument)
                if unnamed_arguments
                    .get(usize::from(*argument))
                    .copied()
                    .unwrap_or(false) =>
            {
                if pc >= marker {
                    return Err("function body reads an anonymous pattern argument slot");
                }
                unnamed_reads.push(usize::from(*argument));
            }
            Instruction::PutArg(argument) | Instruction::SetArg(argument)
                if unnamed_arguments
                    .get(usize::from(*argument))
                    .copied()
                    .unwrap_or(false) =>
            {
                return Err("bytecode writes an anonymous pattern argument slot");
            }
            Instruction::Rest(start) => rest_operations.push((pc, *start)),
            _ => {}
        }

        let target = match instruction {
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target) => Some(*target),
            _ => None,
        };
        if let Some(target) = target {
            let target = usize::try_from(target)
                .map_err(|_| "parameter BindingPattern jump target is outside bytecode")?;
            if pc < marker && target > marker {
                return Err("parameter BindingPattern escaped its initialization segment");
            }
            if pc > marker && target <= marker {
                return Err("function body jumps back into pattern initialization");
            }
        }
    }
    if unnamed_reads != expected_unnamed_reads {
        return Err("anonymous pattern arguments do not have exact entry reads");
    }

    match (metadata.rest_parameter, metadata.rest_pattern_start) {
        (Some(rest), None) => {
            let valid = matches!(rest_operations.as_slice(), [(pc, start)] if {
                *pc < marker
                    && *start == rest
                    && match parameter_environment {
                        Some(layout) => layout
                            .argument_cells
                            .iter()
                            .find(|cell| cell.argument == rest)
                            .is_some_and(|cell| {
                                matches!(
                                    code.get(*pc + 1..*pc + 4),
                                    Some([
                                        Instruction::Dup,
                                        Instruction::PutArg(target),
                                        Instruction::InitializeLocal(local),
                                    ]) if *target == rest && *local == cell.parameter_local
                                )
                            }),
                        None => matches!(
                            code.get(*pc + 1),
                            Some(Instruction::PutArg(target)) if *target == rest
                        ),
                    }
            });
            if valid {
                Ok(())
            } else {
                Err("identifier rest parameter has no exact pattern-segment entry")
            }
        }
        (None, Some(rest))
            if matches!(
                rest_operations.as_slice(),
                [(pc, start)] if *pc < marker && *start == rest
            ) =>
        {
            Ok(())
        }

        (None, Some(_)) => Err("rest BindingPattern has no exact entry initialization"),
        (None, None) if rest_operations.is_empty() => Ok(()),
        (None, None) => Err("rest opcode has no authenticated parameter metadata"),
        (Some(_), Some(_)) => Err("identifier rest and rest BindingPattern metadata overlap"),
    }
}

/// Authenticate lexical locals whose complete scope lifetime belongs to the
/// formal-parameter initializer segment.  This is deliberately independent
/// from the leading Parameter Environment cells: a nested class-name scope,
/// for example, may be captured by a computed method key before the body
/// boundary, but authored body bytecode must never access that local.
pub fn validate_parameter_initializer_scope_layout(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    parameter_body_pc: Option<usize>,
    lexical_locals: &[bool],
    parameter_initializer_locals: &[bool],
) -> Result<(), &'static str> {
    let local_count = usize::from(metadata.local_count);
    if lexical_locals.len() != local_count || parameter_initializer_locals.len() != local_count {
        return Err("parameter-initializer local classification has the wrong length");
    }
    let any_initializer_local = parameter_initializer_locals.iter().any(|value| *value);
    let Some(body_pc) = parameter_body_pc else {
        return if any_initializer_local {
            Err("parameter-initializer local has no parameter/body boundary")
        } else {
            Ok(())
        };
    };
    if body_pc > code.len() {
        return Err("parameter/body boundary is outside bytecode");
    }

    for (index, is_initializer) in parameter_initializer_locals.iter().copied().enumerate() {
        if !is_initializer {
            continue;
        }
        if index < usize::from(metadata.parameter_environment_local_count) || !lexical_locals[index]
        {
            return Err("parameter-initializer classification names a non-nested lexical local");
        }
        let index = u16::try_from(index)
            .map_err(|_| "parameter-initializer local index is outside bytecode range")?;
        let references = |instruction: &Instruction| {
            matches!(
                instruction,
                Instruction::GetLocal(local)
                    | Instruction::PutLocal(local)
                    | Instruction::SetLocal(local)
                    | Instruction::SetLocalUninitialized(local)
                    | Instruction::GetLocalCheck(local)
                    | Instruction::InitializeLocal(local)
                    | Instruction::PutLocalCheck(local)
                    | Instruction::SetLocalCheck(local)
                    | Instruction::CloseLocal(local)
                    if *local == index
            )
        };
        if !code[..body_pc].iter().any(references) {
            return Err("parameter-initializer local has no initializer-segment lifetime");
        }
        let reset_pc = code[..body_pc]
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::SetLocalUninitialized(local) if *local == index)
                    .then_some(pc)
            })
            .collect::<Vec<_>>();
        let initialize_pc = code[..body_pc]
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::InitializeLocal(local) if *local == index)
                    .then_some(pc)
            })
            .collect::<Vec<_>>();
        if !matches!((reset_pc.as_slice(), initialize_pc.as_slice()), ([reset], [initialize]) if reset < initialize)
        {
            return Err("parameter-initializer local has no exact pre-boundary TDZ lifecycle");
        }
        if code[body_pc..].iter().any(references) {
            return Err("function body accesses a parameter-initializer local");
        }
    }
    Ok(())
}

/// Build the exact local set which compiler-authored code may expose to a
/// direct eval while an explicit Parameter Environment is active. This is the
/// eval counterpart of the child-closure capture allowlist: leading parameter
/// cells, nested initializer lexicals, and authenticated entry pseudo-bindings
/// are visible, while authored body storage is not.
pub fn parameter_initializer_visible_locals(
    metadata: &FunctionMetadata,
    code: &[Instruction],
    parameter_body_pc: Option<usize>,
    parameter_initializer_locals: &[bool],
    parameter_environment: Option<&ParameterEnvironmentLayout>,
) -> Result<Option<Vec<bool>>, &'static str> {
    if parameter_initializer_locals.len() != usize::from(metadata.local_count) {
        return Err("parameter-initializer local classification has the wrong length");
    }
    let Some(_) = parameter_body_pc else {
        return Ok(None);
    };
    let layout = parameter_environment.ok_or("parameter boundary has no immutable layout")?;
    let mut allowed = vec![false; usize::from(metadata.local_count)];
    allowed
        .get_mut(..usize::from(metadata.parameter_environment_local_count))
        .ok_or("parameter environment exceeds function local slots")?
        .fill(true);
    for (allowed, is_initializer) in allowed.iter_mut().zip(parameter_initializer_locals) {
        *allowed |= *is_initializer;
    }
    if let Some(local) = metadata.function_name_local {
        *allowed
            .get_mut(usize::from(local))
            .ok_or("function-name local is outside bytecode local slots")? = true;
    }
    if let Some(local) = layout.arg_eval_variable_object_local {
        *allowed
            .get_mut(usize::from(local))
            .ok_or("parameter eval variable-object local is outside bytecode local slots")? = true;
    }
    if let Some(local) = metadata.derived_this_local {
        *allowed
            .get_mut(usize::from(local))
            .ok_or("derived this local is outside bytecode local slots")? = true;
    }

    let mut entry_pc = 0_usize;
    while let Some([source, Instruction::PutLocal(local)]) = code.get(entry_pc..entry_pc + 2) {
        if pseudo_binding_entry_rank(source).is_none() {
            break;
        }
        *allowed
            .get_mut(usize::from(*local))
            .ok_or("parameter pseudo-binding local is out of bounds")? = true;
        entry_pc += 2;
    }
    if let Some(synthetic) = layout.synthetic_arguments_local
        && matches!(
            code.get(entry_pc..entry_pc + 4),
            Some([
                Instruction::Arguments(_),
                Instruction::Dup,
                Instruction::InitializeLocal(target),
                Instruction::PutLocal(_),
            ]) if *target == synthetic
        )
    {
        *allowed
            .get_mut(usize::from(synthetic))
            .ok_or("synthetic parameter arguments local is out of bounds")? = true;
    } else if matches!(code.get(entry_pc), Some(Instruction::Arguments(_)))
        && let Some(Instruction::PutLocal(local)) = code.get(entry_pc + 1)
    {
        *allowed
            .get_mut(usize::from(*local))
            .ok_or("parameter arguments local is out of bounds")? = true;
    }
    Ok(Some(allowed))
}

/// Bind every immutable direct-eval descriptor to the bytecode phase which
/// references it. Descriptor topology and source metadata alone are
/// insufficient: a forged body-side `Eval` could otherwise reuse a Parameter
/// or BindingPattern initializer descriptor after its lexical scope ended.
pub struct EvalEnvironmentPhaseContext<'a> {
    pub metadata: &'a FunctionMetadata,
    pub code: &'a [Instruction],
    pub parameter_body_pc: Option<usize>,
    pub pattern_body_pc: Option<usize>,
    pub lexical_locals: &'a [bool],
    pub parameter_initializer_locals: &'a [bool],
    pub parameter_initializer_visible_locals: Option<&'a [bool]>,
    pub parameter_environment: Option<&'a ParameterEnvironmentLayout>,
}

pub fn validate_eval_environment_phase_layout<Name>(
    environments: &[EvalEnvironment<Name>],
    context: EvalEnvironmentPhaseContext<'_>,
) -> Result<(), &'static str> {
    let EvalEnvironmentPhaseContext {
        metadata,
        code,
        parameter_body_pc,
        pattern_body_pc,
        lexical_locals,
        parameter_initializer_locals,
        parameter_initializer_visible_locals,
        parameter_environment,
    } = context;
    let local_count = usize::from(metadata.local_count);
    if lexical_locals.len() != local_count
        || parameter_initializer_locals.len() != local_count
        || parameter_initializer_visible_locals.is_some_and(|visible| visible.len() != local_count)
    {
        return Err("eval parameter-phase local classification has the wrong length");
    }
    if parameter_body_pc.is_some() != parameter_initializer_visible_locals.is_some() {
        return Err("eval parameter-phase allowlist disagrees with its boundary");
    }

    let mut environment_pcs = vec![Vec::new(); environments.len()];
    for (pc, instruction) in code.iter().enumerate() {
        let Some(environment) = instruction.eval_environment() else {
            continue;
        };
        environment_pcs
            .get_mut(usize::from(environment))
            .ok_or("Eval bytecode environment operand is out of bounds")?
            .push(pc);
    }
    if environment_pcs.iter().any(Vec::is_empty) {
        return Err("eval environment descriptor is not referenced by bytecode");
    }

    let Some(body_pc) = parameter_body_pc.or(pattern_body_pc) else {
        return Ok(());
    };
    let explicit_parameter_environment = parameter_body_pc.is_some();
    let synthetic_arguments_local =
        parameter_environment.and_then(|layout| layout.synthetic_arguments_local);

    for (environment, pcs) in environments.iter().zip(environment_pcs) {
        let referenced_in_initializer = pcs.iter().any(|pc| *pc < body_pc);
        let referenced_in_body = pcs.iter().any(|pc| *pc >= body_pc);

        if explicit_parameter_environment {
            let current_anchor = environment
                .scopes
                .iter()
                .find(|scope| {
                    matches!(
                        scope.kind,
                        EvalScopeKind::FunctionRoot | EvalScopeKind::Parameter
                    )
                })
                .map(|scope| scope.kind)
                .ok_or("eval environment contains no current function anchor")?;
            if referenced_in_initializer && current_anchor != EvalScopeKind::Parameter {
                return Err("parameter initializer eval used a body environment descriptor");
            }
            if referenced_in_body && current_anchor != EvalScopeKind::FunctionRoot {
                return Err("function body eval used a parameter environment descriptor");
            }
        }

        for binding in environment
            .scopes
            .iter()
            .flat_map(|scope| scope.bindings.iter())
        {
            match binding.source {
                EvalBindingSource::Argument(_)
                    if referenced_in_initializer && explicit_parameter_environment =>
                {
                    return Err("parameter initializer eval captured a raw argument slot");
                }
                EvalBindingSource::Local(index) => {
                    let index_usize = usize::from(index);
                    let is_lexical = *lexical_locals
                        .get(index_usize)
                        .ok_or("eval binding local source is out of bounds")?;
                    let is_parameter_initializer =
                        *parameter_initializer_locals
                            .get(index_usize)
                            .ok_or("eval binding local source is out of bounds")?;
                    if referenced_in_initializer {
                        if let Some(visible) = parameter_initializer_visible_locals {
                            if !visible[index_usize] {
                                return Err(
                                    "parameter initializer eval captured a body-only local",
                                );
                            }
                        } else if pattern_body_pc.is_some()
                            && index >= metadata.parameter_environment_local_count
                            && synthetic_arguments_local != Some(index)
                            && is_lexical
                            && !is_parameter_initializer
                        {
                            return Err("pattern initializer eval captured a body lexical local");
                        }
                    }
                    // A body-side direct eval may legitimately retain the
                    // function's parameter cells and `<arg_var>` chain: those
                    // bindings remain part of name resolution after parameter
                    // initialization. Only a nested lexical whose complete
                    // lifetime ended at the boundary is forbidden here.
                    if referenced_in_body && is_parameter_initializer {
                        return Err("function body eval captured a parameter-initializer local");
                    }
                }
                EvalBindingSource::Argument(_) | EvalBindingSource::Closure(_) => {}
            }
        }
    }
    Ok(())
}
