//! Validation and iterative flattening for unlinked bytecode publication.

use super::*;

use crate::bytecode::{EvalVariableSource, MAX_LOCAL_SLOTS, verify_parts};
use crate::heap::{EvalBinding, EvalKind, EvalRootBinding, EvalScope};

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

fn verify_eval_scope_topology(
    environment: &EvalEnvironment<JsString>,
    function_depth: usize,
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
        let expected_body = if final_segment {
            crate::heap::EvalScopeKind::ProgramBody
        } else {
            crate::heap::EvalScopeKind::FunctionBody
        };
        if function_root == segment_start
            || environment.scopes[function_root - 1].kind != expected_body
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval scope segment has the wrong body scope",
            )));
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
    let expected_segments = function_depth.checked_add(1).ok_or_else(|| {
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

fn verify_eval_environments(
    function: &UnlinkedFunction,
    function_depth: usize,
    captured_locals: &mut [bool],
) -> Result<(), RuntimeError> {
    let is_root = function_depth == 0;
    for environment in function.eval_environments() {
        if environment.caller_strict != function.metadata().strict {
            return Err(RuntimeError::Engine(Error::internal(
                "eval environment strictness disagrees with bytecode metadata",
            )));
        }
        let first_function_root = verify_eval_scope_topology(environment, function_depth)?;
        match environment.variable_environment {
            crate::heap::EvalVariableEnvironment::Global => {
                if !is_root {
                    return Err(RuntimeError::Engine(Error::internal(
                        "non-root eval environment used the global variable environment",
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
                if is_root {
                    return Err(RuntimeError::Engine(Error::internal(
                        "root eval environment used a function variable scope",
                    )));
                }
                if usize::from(index) != first_function_root {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval variable environment did not reference its current function root scope",
                    )));
                }
            }
        }

        for scope in &environment.scopes {
            for binding in &scope.bindings {
                if binding.name.is_empty() {
                    return Err(RuntimeError::Engine(Error::internal(
                        "eval binding has an empty name",
                    )));
                }
                let is_catch_scope = scope.kind == crate::heap::EvalScopeKind::Catch;
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
pub(in crate::runtime) fn verify_unlinked_eval_tree(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
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
    for binding in expected_bindings {
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
        if binding.kind == ClosureVariableKind::GlobalFunction {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root imported a declaration-only global binding kind",
            )));
        }
    }
    verify_unlinked_tree_with_root(
        function,
        RootPublication::Eval {
            kind,
            caller_strict,
            expected_bindings,
        },
    )
}

fn verify_unlinked_tree_with_root(
    function: &UnlinkedFunction,
    root_publication: RootPublication<'_>,
) -> Result<(), RuntimeError> {
    let mut pending = vec![(function, 0_usize)];
    while let Some((function, function_depth)) = pending.pop() {
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
        if expected_eval_bindings.is_some() && !function.eval_environments().is_empty() {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root retained a nested direct-eval environment",
            )));
        }
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
            let requires_name = matches!(
                descriptor.kind,
                ClosureVariableKind::FunctionName | ClosureVariableKind::EvalVariableObject
            ) || matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration
                    | ClosureSource::Global
                    | ClosureSource::ParentGlobal(_)
                    | ClosureSource::EvalEnvironment(_)
            );
            let name = unlinked_closure_name(function, descriptor)?;
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
        verify_eval_environments(function, function_depth, &mut captured_locals)?;

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
                | crate::bytecode::Instruction::DefineField(index) => {
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
                        .is_some_and(|definition| definition.is_lexical) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "unchecked local opcode referenced a lexical definition",
                    )));
                }
                crate::bytecode::Instruction::SetLocalUninitialized(index)
                | crate::bytecode::Instruction::GetLocalCheck(index)
                | crate::bytecode::Instruction::InitializeLocal(index)
                | crate::bytecode::Instruction::PutLocalCheck(index)
                | crate::bytecode::Instruction::SetLocalCheck(index)
                | crate::bytecode::Instruction::CloseLocal(index)
                    if function
                        .local_definitions()
                        .get(usize::from(*index))
                        .is_some_and(|definition| !definition.is_lexical) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "checked lexical-local opcode referenced an ordinary definition",
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
                            descriptor.kind == ClosureVariableKind::EvalVariableObject
                        }) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "ordinary closure opcode referenced the private eval variable object",
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
                for descriptor in child.closure_variables() {
                    let flags = (descriptor.is_lexical, descriptor.is_const, descriptor.kind);
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
                            if flags
                                != (definition.is_lexical, definition.is_const, definition.kind)
                            {
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
                            verify_capture_flags(slot, flags)?;
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
                            if (parent.is_lexical, parent.is_const, parent.kind) != flags {
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
                            if (descriptor.is_lexical
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
                }
                let child_depth = function_depth.checked_add(1).ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "function-tree depth overflowed during publication",
                    ))
                })?;
                pending.push((child, child_depth));
            } else if constant.as_primitive().is_none() && constant.as_regexp().is_none() {
                return Err(RuntimeError::Invariant(
                    "unlinked constant did not contain exactly one payload",
                ));
            }
        }
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
