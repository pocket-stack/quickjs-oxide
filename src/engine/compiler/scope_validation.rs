//! Validate the completed scope and binding graph before identifier resolution.

use super::{
    ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME, ArgumentsKind, BindingKind, BindingStorage,
    BytecodeFunctionKind, ClassInitializerKind, ClosureSource, ClosureVariableKind,
    ClosureVariableName, EVAL_RET_LOCAL_NAME, EVAL_VARIABLE_OBJECT_LOCAL_NAME, Error, ErrorKind,
    EvalCallerVariableTarget, EvalKind, EvalScopeKind, FINALLY_EVAL_RET_LOCAL_NAME, FunctionKind,
    FunctionTree, IdentifierAccess, Instruction, IrAnnexBinding, IrConstant, IrOp,
    ParameterDefaultSource, ParentLink, PseudoBinding, ScopeId, ScopeKind, SpannedIrOp,
    SyntheticLocalKind, THIS_LOCAL_NAME, Value, WITH_OBJECT_LOCAL_NAME,
    binding_kind_from_closure_flags, binding_kinds_compatible, function_owns_pseudo_binding,
    ordered_hoisted_functions,
};

pub(super) fn validate_scope_graph(tree: &FunctionTree) -> Result<(), Error> {
    for (function_id, function) in tree.functions.iter().enumerate() {
        if function.scopes.len() < 2
            || function.var_scope != ScopeId(0)
            || function.current_scope != function.body_scope
            || function.scopes[0].parent.is_some()
            || function.scopes[0].kind != ScopeKind::FunctionRoot
            || function.scopes[0].is_parameter_initializer
            || function.body_scope.0 >= function.scopes.len()
            || function.body_scope == function.var_scope
            || function.scopes[function.body_scope.0].is_parameter_initializer
        {
            return Err(Error::internal("function scope roots are malformed"));
        }
        let expected_body = if matches!(
            function.kind,
            FunctionKind::Script | FunctionKind::Module | FunctionKind::Eval(_)
        ) {
            ScopeKind::ProgramBody
        } else {
            ScopeKind::FunctionBody
        };
        if function.scopes[function.body_scope.0].kind != expected_body {
            return Err(Error::internal("function body scope kind is malformed"));
        }
        let initial_yields = function
            .ops
            .iter()
            .filter(|operation| matches!(operation.op, IrOp::Bytecode(Instruction::InitialYield)))
            .count();
        let suspension_ops = function
            .ops
            .iter()
            .filter(|operation| {
                matches!(
                    operation.op,
                    IrOp::Bytecode(
                        Instruction::Yield
                            | Instruction::YieldStar
                            | Instruction::AsyncYieldStar
                            | Instruction::IteratorStart
                            | Instruction::AsyncIteratorStart
                            | Instruction::IteratorNext
                            | Instruction::IteratorCall(_)
                            | Instruction::IteratorCheckObject
                            | Instruction::ThrowIteratorMissingThrow
                    )
                )
            })
            .count();
        let await_ops = function
            .ops
            .iter()
            .filter(|operation| matches!(operation.op, IrOp::Bytecode(Instruction::Await)))
            .count();
        match function.execution_kind {
            BytecodeFunctionKind::Normal
                if initial_yields == 0 && suspension_ops == 0 && await_ops == 0 => {}
            BytecodeFunctionKind::Generator
                if matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
                    && !function.class_constructor
                    && function.class_initializer_kind.is_none()
                    && function.in_function_body
                    && await_ops == 0
                    && initial_yields == 1 =>
            {
                let initial = function
                    .ops
                    .iter()
                    .position(|operation| {
                        matches!(operation.op, IrOp::Bytecode(Instruction::InitialYield))
                    })
                    .ok_or_else(|| Error::internal("generator lost its initial yield"))?;
                let body = function
                    .ops
                    .iter()
                    .position(|operation| {
                        matches!(operation.op, IrOp::EnterScope(scope) if scope == function.body_scope)
                    })
                    .ok_or_else(|| Error::internal("generator lost its body entry"))?;
                if initial >= body {
                    return Err(Error::internal(
                        "generator initial yield did not precede its body scope",
                    ));
                }
            }
            BytecodeFunctionKind::Async
                if matches!(
                    function.kind,
                    FunctionKind::Module
                        | FunctionKind::Ordinary
                        | FunctionKind::Method
                        | FunctionKind::Arrow
                ) && !function.class_constructor
                    && function.class_initializer_kind.is_none()
                    && function.in_function_body
                    && initial_yields == 0
                    && suspension_ops == 0 => {}
            BytecodeFunctionKind::AsyncGenerator
                if matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
                    && !function.class_constructor
                    && function.class_initializer_kind.is_none()
                    && function.in_function_body
                    && initial_yields == 1 =>
            {
                let initial = function
                    .ops
                    .iter()
                    .position(|operation| {
                        matches!(operation.op, IrOp::Bytecode(Instruction::InitialYield))
                    })
                    .ok_or_else(|| Error::internal("async generator lost its initial yield"))?;
                let body = function
                    .ops
                    .iter()
                    .position(|operation| {
                        matches!(operation.op, IrOp::EnterScope(scope) if scope == function.body_scope)
                    })
                    .ok_or_else(|| Error::internal("async generator lost its body entry"))?;
                if initial >= body {
                    return Err(Error::internal(
                        "async generator initial yield did not precede its body scope",
                    ));
                }
            }
            _ => {
                return Err(Error::internal(
                    "function execution kind and suspension bytecode disagree",
                ));
            }
        }
        match function.parameter_scope {
            Some(scope)
                if scope != function.var_scope
                    && scope != function.body_scope
                    && function.scopes.get(scope.0).is_some_and(|scope| {
                        scope.kind == ScopeKind::Parameter
                            && scope.parent.is_none()
                            && scope.is_parameter_initializer
                    })
                    && matches!(
                        function.kind,
                        FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                    ) => {}
            None => {}
            Some(_) => {
                return Err(Error::internal(
                    "parameter environment scope metadata is malformed",
                ));
            }
        }
        let has_parameter_environment = function.parameter_scope.is_some();
        let has_pattern_parameters = function.pattern_parameter_initialization;
        for (scope_index, scope) in function.scopes.iter().enumerate() {
            if scope_index == function.var_scope.0 || scope_index == function.body_scope.0 {
                continue;
            }
            if function.parameter_scope == Some(ScopeId(scope_index)) {
                if !scope.is_parameter_initializer {
                    return Err(Error::internal(
                        "parameter scope lost its initializer classification",
                    ));
                }
                continue;
            }
            let parent = scope
                .parent
                .ok_or_else(|| Error::internal("nested scope has no parent"))?;
            let parent_scope = function
                .scopes
                .get(parent.0)
                .ok_or_else(|| Error::internal("lexical scope parent is malformed"))?;
            let expected_initializer = parent_scope.is_parameter_initializer
                || (has_pattern_parameters && parent == function.var_scope);
            if scope.is_parameter_initializer != expected_initializer {
                return Err(Error::internal(
                    "nested scope parameter-initializer classification is malformed",
                ));
            }
        }
        if function.defined_argument_count
            > function
                .parameters
                .len()
                .saturating_add(usize::from(function.rest_pattern_start.is_some()))
            || (function.has_simple_parameter_list
                && (function.defined_argument_count != function.parameters.len()
                    || function.rest_parameter.is_some()
                    || function.rest_pattern_start.is_some()
                    || has_parameter_environment
                    || has_pattern_parameters
                    || function.parameters.iter().any(Option::is_none)))
            || function.rest_parameter.is_some_and(|rest| {
                usize::from(rest) + 1 != function.parameters.len()
                    || function.defined_argument_count > usize::from(rest)
                    || (!has_parameter_environment
                        && function.defined_argument_count != usize::from(rest))
                    || function.has_simple_parameter_list
                    || !matches!(
                        function.kind,
                        FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                    )
            })
            || function.rest_parameter.is_some() && function.rest_pattern_start.is_some()
            || function.rest_pattern_start.is_some_and(|start| {
                usize::from(start) != function.parameters.len()
                    || function.defined_argument_count > usize::from(start) + 1
                    || function.has_simple_parameter_list
                    || !has_pattern_parameters
            })
            || has_parameter_environment
                && (function.has_simple_parameter_list
                    || function.parameter_locals.len() != function.parameter_names.len()
                    || function
                        .parameter_local_reservation_count
                        .is_some_and(|reserved| reserved != function.parameter_locals.len()))
            || function.parameter_argument_locals.len() != function.parameters.len()
            || !has_parameter_environment
                && (!function.parameter_locals.is_empty()
                    || function.parameter_local_reservation_count.is_some()
                    || function
                        .parameter_argument_locals
                        .iter()
                        .any(Option::is_some)
                    || !function.parameter_pattern_bindings.is_empty()
                    || !function.parameter_default_sources.is_empty())
            || has_pattern_parameters
                && (function.has_simple_parameter_list
                    || (!function.parameters.iter().any(Option::is_none)
                        && function.rest_pattern_start.is_none()))
        {
            return Err(Error::internal("formal parameter metadata is malformed"));
        }
        let parameter_markers = function
            .ops
            .iter()
            .filter(|operation| matches!(operation.op, IrOp::ParameterInitializationEnd))
            .count();
        if parameter_markers != usize::from(has_pattern_parameters || has_parameter_environment) {
            return Err(Error::internal(
                "pattern parameter marker metadata is malformed",
            ));
        }
        if let Some(parameter_scope) = function.parameter_scope {
            let scope = &function.scopes[parameter_scope.0];
            let authored_cells = function.parameter_locals.len();
            let synthetic_arguments_matches = function
                .synthetic_parameter_arguments_local
                .is_none_or(|local| {
                    scope.bindings.last().is_some_and(|binding| {
                        function.bindings[binding.0].storage == BindingStorage::Local(local)
                    })
                });
            if scope.bindings.len()
                != authored_cells
                    .checked_add(usize::from(
                        function.synthetic_parameter_arguments_local.is_some(),
                    ))
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "too many local variables"))?
                || !synthetic_arguments_matches
            {
                return Err(Error::internal(
                    "parameter environment does not own every parameter cell",
                ));
            }
            for (cell, (&local, &binding_id)) in function
                .parameter_locals
                .iter()
                .zip(&scope.bindings[..authored_cells])
                .enumerate()
            {
                let binding = function
                    .bindings
                    .get(binding_id.0)
                    .ok_or_else(|| Error::internal("parameter binding is out of bounds"))?;
                if usize::from(local) != cell
                    || function.locals.get(cell).map(String::as_str) != Some(binding.name.as_str())
                    || binding.storage != BindingStorage::Local(local)
                    || binding.storage_scope != parameter_scope
                    || binding.declaration_scope != parameter_scope
                    || binding.kind != (BindingKind::Lexical { is_const: false })
                    || binding.is_catch_parameter
                {
                    return Err(Error::internal(
                        "parameter environment cell metadata is malformed",
                    ));
                }
            }
            for (argument, parameter) in function.parameters.iter().enumerate() {
                match (parameter, function.parameter_argument_locals[argument]) {
                    (Some(name), Some(local))
                        if function
                            .locals
                            .get(usize::from(local))
                            .is_some_and(|local_name| local_name == name) => {}
                    (None, None) => {}
                    _ => {
                        return Err(Error::internal(
                            "parameter argument-to-cell metadata is malformed",
                        ));
                    }
                }
            }
            let mut previous_default = None;
            for source in function.parameter_default_sources.iter().copied() {
                let formal = match source {
                    ParameterDefaultSource::Argument(argument)
                        if usize::from(argument) < function.parameters.len()
                            && function.rest_parameter != Some(argument) =>
                    {
                        argument
                    }
                    ParameterDefaultSource::RestPattern(start)
                        if function.rest_pattern_start == Some(start) =>
                    {
                        start
                    }
                    _ => {
                        return Err(Error::internal(
                            "parameter default source metadata is malformed",
                        ));
                    }
                };
                if previous_default.is_some_and(|previous| previous >= formal) {
                    return Err(Error::internal(
                        "parameter default sources are not in formal order",
                    ));
                }
                previous_default = Some(formal);
            }
            for pattern in &function.parameter_pattern_bindings {
                let Some(body_local) = pattern.body_local else {
                    return Err(Error::internal(
                        "parameter pattern body binding was not allocated",
                    ));
                };
                let parameter_binding = function.binding_in_scope(parameter_scope, &pattern.name);
                let body_binding = function.binding_in_scope(function.var_scope, &pattern.name);
                if parameter_binding.is_none_or(|binding| {
                    binding.storage != BindingStorage::Local(pattern.parameter_local)
                }) || body_binding.is_none_or(|binding| {
                    binding.storage != BindingStorage::Local(body_local)
                        || binding.kind != BindingKind::Normal
                }) || function.locals.get(usize::from(body_local)) != Some(&pattern.name)
                {
                    return Err(Error::internal(
                        "parameter pattern copy metadata is malformed",
                    ));
                }
            }
        }
        let rest_operations = function
            .ops
            .iter()
            .filter(|operation| matches!(operation.op, IrOp::Bytecode(Instruction::Rest(_))))
            .count();
        let expected_rest_operations = usize::from(
            function.rest_pattern_start.is_some()
                || function.rest_parameter.is_some()
                    && (has_parameter_environment
                        || has_pattern_parameters
                        || function.function_hoists_installed),
        );
        if rest_operations != expected_rest_operations {
            return Err(Error::internal(
                "rest parameter entry initialization is malformed",
            ));
        }
        if function.super_call_allowed && !function.super_allowed {
            return Err(Error::internal(
                "function permits super() without SuperProperty",
            ));
        }
        match function.kind {
            FunctionKind::Method
                if function.super_allowed
                    && function.super_call_allowed == function.derived_class_constructor => {}
            FunctionKind::Arrow => {
                let parent = function
                    .parent
                    .and_then(|parent| tree.functions.get(parent.function))
                    .ok_or_else(|| Error::internal("arrow function has no valid parent"))?;
                if (function.super_call_allowed, function.super_allowed)
                    != (parent.super_call_allowed, parent.super_allowed)
                {
                    return Err(Error::internal(
                        "arrow super capability disagrees with its parent",
                    ));
                }
            }
            FunctionKind::Eval(EvalKind::Direct) => {}
            FunctionKind::Script
            | FunctionKind::Module
            | FunctionKind::Ordinary
            | FunctionKind::Eval(EvalKind::Indirect)
                if !function.super_call_allowed && !function.super_allowed => {}
            FunctionKind::Eval(EvalKind::None) => {
                return Err(Error::internal("eval root has no eval kind"));
            }
            FunctionKind::Method
            | FunctionKind::Script
            | FunctionKind::Module
            | FunctionKind::Ordinary
            | FunctionKind::Eval(EvalKind::Indirect) => {
                return Err(Error::internal(
                    "function kind retained malformed super capability",
                ));
            }
        }
        if function.private_name_binding
            && (!matches!(function.kind, FunctionKind::Ordinary)
                || function.function_name.is_none())
        {
            return Err(Error::internal(
                "private function-name capability is malformed",
            ));
        }
        if function.class_private_brand
            && !matches!(
                function.class_initializer_kind,
                Some(ClassInitializerKind::InstanceFields | ClassInitializerKind::StaticElements)
            )
        {
            return Err(Error::internal(
                "private brand escaped an aggregate class initializer",
            ));
        }
        let check_ctor_count = function
            .ops
            .iter()
            .filter(|operation| matches!(operation.op, IrOp::Bytecode(Instruction::CheckCtor)))
            .count();
        if function.class_constructor {
            if !matches!(function.kind, FunctionKind::Method)
                || !function.strict
                || check_ctor_count != 1
                || function.derived_class_constructor != function.super_call_allowed
                || function.derived_class_constructor != function.active_function_local.is_some()
                || (function.derived_class_constructor && function.this_local.is_none())
            {
                return Err(Error::internal("class constructor metadata is malformed"));
            }
        } else if check_ctor_count != 0 || function.derived_class_constructor {
            return Err(Error::internal(
                "non-class function retained a constructor-call guard",
            ));
        }
        if matches!(function.kind, FunctionKind::Script | FunctionKind::Module)
            && (!function.hoisted_functions.is_empty() || function.function_hoists_installed)
        {
            return Err(Error::internal(
                "root script contains ordinary function-body hoists",
            ));
        }
        match function.kind {
            FunctionKind::Eval(EvalKind::Direct) => {
                if function.external_bindings.len() > function.closure_variables.len() {
                    return Err(Error::internal(
                        "direct eval external bindings exceed closure slots",
                    ));
                }
                if function.external_bindings.iter().any(|binding| {
                    let Some(&scope_kind) = function
                        .eval_caller_profile
                        .scope_kinds
                        .get(usize::from(binding.scope))
                    else {
                        return true;
                    };
                    (binding.is_catch_parameter && scope_kind != EvalScopeKind::Catch)
                        || (binding.kind == ClosureVariableKind::WithObject)
                            != (scope_kind == EvalScopeKind::With)
                }) || function
                    .eval_caller_profile
                    .scope_kinds
                    .iter()
                    .enumerate()
                    .any(|(scope, kind)| {
                        *kind == EvalScopeKind::With
                            && function
                                .external_bindings
                                .iter()
                                .filter(|binding| usize::from(binding.scope) == scope)
                                .count()
                                != 1
                    })
                {
                    return Err(Error::internal(
                        "direct eval bindings disagree with the caller scope profile",
                    ));
                }
                match function.eval_caller_profile.variable_target {
                    EvalCallerVariableTarget::Global => {}
                    EvalCallerVariableTarget::StrictLocal if function.strict => {}
                    EvalCallerVariableTarget::ExternalBinding(index)
                        if function
                            .external_bindings
                            .get(usize::from(index))
                            .is_some_and(|binding| {
                                matches!(
                                    binding.kind,
                                    ClosureVariableKind::EvalVariableObject
                                        | ClosureVariableKind::ArgEvalVariableObject
                                ) && !binding.is_lexical
                                    && !binding.is_const
                                    && !binding.is_catch_parameter
                            }) => {}
                    EvalCallerVariableTarget::StrictLocal
                    | EvalCallerVariableTarget::ExternalBinding(_) => {
                        return Err(Error::internal(
                            "direct eval caller variable target is malformed",
                        ));
                    }
                }
            }
            FunctionKind::Module => {
                if !function.external_bindings.is_empty()
                    || !function.eval_caller_profile.scope_kinds.is_empty()
                    || function.eval_caller_profile.variable_target
                        != EvalCallerVariableTarget::StrictLocal
                {
                    return Err(Error::internal(
                        "module root retained a non-strict eval variable target",
                    ));
                }
            }
            FunctionKind::Eval(EvalKind::Indirect) => {
                if !function.external_bindings.is_empty()
                    || !function.eval_caller_profile.scope_kinds.is_empty()
                    || function.eval_caller_profile.variable_target
                        != EvalCallerVariableTarget::Global
                {
                    return Err(Error::internal(
                        "indirect eval retained a caller environment",
                    ));
                }
            }
            FunctionKind::Eval(EvalKind::None) => {
                return Err(Error::internal("eval root has no eval kind"));
            }
            FunctionKind::Script
            | FunctionKind::Ordinary
            | FunctionKind::Method
            | FunctionKind::Arrow => {
                if !function.external_bindings.is_empty()
                    || !function.eval_caller_profile.scope_kinds.is_empty()
                    || function.eval_caller_profile.variable_target
                        != EvalCallerVariableTarget::Global
                {
                    return Err(Error::internal(
                        "non-eval function retained an eval caller environment",
                    ));
                }
            }
        }
        if let Some(index) = function.arguments_local {
            let matches_binding = function.bindings.iter().any(|binding| {
                binding.name == "arguments"
                    && binding.storage_scope == function.var_scope
                    && binding.kind == BindingKind::Normal
                    && binding.storage == BindingStorage::Local(index)
            });
            if !matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
                || usize::from(index) >= function.locals.len()
                || function.locals[usize::from(index)] != "arguments"
                || (function
                    .parameters
                    .iter()
                    .any(|parameter| parameter.as_deref() == Some("arguments"))
                    && !(function.parameter_scope.is_some()
                        && !function.strict
                        && function.eval_variable_object_local.is_some()))
                || !matches_binding
            {
                return Err(Error::internal(
                    "implicit arguments local metadata is malformed",
                ));
            }
        }
        for (pseudo, local) in [
            (PseudoBinding::HomeObject, function.home_object_local),
            (
                PseudoBinding::ActiveFunction,
                function.active_function_local,
            ),
            (PseudoBinding::This, function.this_local),
            (PseudoBinding::NewTarget, function.new_target_local),
        ] {
            let local_bindings = function
                .bindings
                .iter()
                .filter(|binding| {
                    binding.name == pseudo.name()
                        && matches!(binding.storage, BindingStorage::Local(_))
                })
                .collect::<Vec<_>>();
            match (local, local_bindings.as_slice()) {
                (Some(index), [binding])
                    if function_owns_pseudo_binding(function.kind, pseudo)
                        && usize::from(index) < function.locals.len()
                        && function.locals[usize::from(index)] == pseudo.name()
                        && binding.storage == BindingStorage::Local(index)
                        && binding.kind
                            == if pseudo == PseudoBinding::This
                                && function.derived_class_constructor
                            {
                                BindingKind::Lexical { is_const: false }
                            } else {
                                BindingKind::Normal
                            }
                        && binding.storage_scope == function.var_scope
                        && binding.declaration_scope == function.var_scope
                        && binding.declaration_span.is_none() =>
                {
                    let initialized = function
                        .ops
                        .windows(2)
                        .filter(|window| {
                            matches!(
                                (&window[0].op, &window[1].op, pseudo),
                                (
                                    IrOp::Bytecode(Instruction::PushHomeObject),
                                    IrOp::Bytecode(Instruction::PutLocal(target)),
                                    PseudoBinding::HomeObject,
                                ) if *target == index
                            ) || matches!(
                                (&window[0].op, &window[1].op, pseudo),
                                (
                                    IrOp::Bytecode(Instruction::PushActiveFunction),
                                    IrOp::Bytecode(Instruction::PutLocal(target)),
                                    PseudoBinding::ActiveFunction,
                                ) if *target == index
                            ) || matches!(
                                (&window[0].op, &window[1].op, pseudo),
                                (
                                    IrOp::Bytecode(Instruction::PushThis),
                                    IrOp::Bytecode(Instruction::PutLocal(target)),
                                    PseudoBinding::This,
                                ) if *target == index
                            ) || matches!(
                                (&window[0].op, &window[1].op, pseudo),
                                (
                                    IrOp::Bytecode(Instruction::PushNewTarget),
                                    IrOp::Bytecode(Instruction::PutLocal(target)),
                                    PseudoBinding::NewTarget,
                                ) if *target == index
                            )
                        })
                        .count();
                    // The scope graph is validated once before identifier
                    // linking/entry-prologue installation and again after
                    // all hoists are fixed. Derived constructors eagerly own
                    // their active-function and TDZ `this` cells, so the
                    // pre-link pass must require zero entry writes while the
                    // final pass requires every ordinary pseudo initializer.
                    let expected = usize::from(
                        function.pseudo_binding_prologues_installed
                            && !(pseudo == PseudoBinding::This
                                && function.derived_class_constructor),
                    );
                    if initialized != expected {
                        return Err(Error::internal(format!(
                            "{} pseudo local entry initialization is malformed: expected {expected}, found {initialized}",
                            pseudo.name(),
                        )));
                    }
                }
                (None, []) => {}
                _ => {
                    return Err(Error::internal(
                        "pseudo local binding metadata is malformed",
                    ));
                }
            }
        }
        if function.home_object_local.is_some()
            && (!function.needs_home_object || !function.super_allowed)
        {
            return Err(Error::internal(
                "HomeObject pseudo local lacks publication metadata",
            ));
        }
        if function.needs_home_object && !matches!(function.kind, FunctionKind::Method) {
            return Err(Error::internal(
                "non-method function retained HomeObject metadata",
            ));
        }
        let eval_variable_object_bindings = function
            .bindings
            .iter()
            .filter(|binding| {
                binding.kind == BindingKind::EvalVariableObject
                    && matches!(binding.storage, BindingStorage::Local(_))
            })
            .collect::<Vec<_>>();
        match (
            function.eval_variable_object_local,
            eval_variable_object_bindings.as_slice(),
        ) {
            (Some(index), [binding])
                if matches!(
                    function.kind,
                    FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                ) && !function.strict
                    && binding.name == EVAL_VARIABLE_OBJECT_LOCAL_NAME
                    && binding.storage == BindingStorage::Local(index)
                    && binding.storage_scope == function.var_scope
                    && binding.declaration_scope == function.var_scope
                    && binding.declaration_span.is_none()
                    && function
                        .ops
                        .iter()
                        .any(|operation| matches!(operation.op, IrOp::EvalCall { .. })) => {}
            (None, []) => {}
            _ => {
                return Err(Error::internal(
                    "eval variable object local metadata is malformed",
                ));
            }
        }
        let arg_eval_variable_object_bindings = function
            .bindings
            .iter()
            .filter(|binding| {
                binding.kind == BindingKind::ArgEvalVariableObject
                    && matches!(binding.storage, BindingStorage::Local(_))
            })
            .collect::<Vec<_>>();
        match (
            function.arg_eval_variable_object_local,
            arg_eval_variable_object_bindings.as_slice(),
        ) {
            (Some(index), [binding])
                if function.parameter_scope.is_some()
                    && function.eval_variable_object_local.is_some()
                    && !function.strict
                    && binding.name == ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME
                    && binding.storage == BindingStorage::Local(index)
                    && binding.storage_scope == function.var_scope
                    && binding.declaration_scope == function.var_scope
                    && binding.declaration_span.is_none() => {}
            (None, []) => {}
            _ => {
                return Err(Error::internal(
                    "argument eval variable object local metadata is malformed",
                ));
            }
        }
        let synthetic_arguments_bindings = function
            .bindings
            .iter()
            .filter(|binding| {
                matches!(binding.storage, BindingStorage::Local(index)
                    if function.synthetic_parameter_arguments_local == Some(index))
            })
            .collect::<Vec<_>>();
        match (
            function.synthetic_parameter_arguments_local,
            synthetic_arguments_bindings.as_slice(),
        ) {
            (Some(index), [binding])
                if matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
                    && !function.strict
                    && function.arg_eval_variable_object_local.is_some()
                    && function.parameter_scope == Some(binding.storage_scope)
                    && binding.declaration_scope == binding.storage_scope
                    && binding.name == "arguments"
                    && binding.kind == (BindingKind::Lexical { is_const: false })
                    && binding.declaration_span.is_none()
                    && function.locals.get(usize::from(index)).map(String::as_str)
                        == Some("arguments")
                    && !function
                        .parameter_names
                        .iter()
                        .any(|name| name == "arguments") => {}
            (None, []) => {}
            _ => {
                return Err(Error::internal(
                    "synthetic parameter arguments metadata is malformed",
                ));
            }
        }
        if function.strict
            && function.bindings.iter().any(|binding| {
                binding.kind == BindingKind::WithObject
                    && matches!(binding.storage, BindingStorage::Local(_))
            })
        {
            return Err(Error::internal(
                "strict function retained a local with object binding",
            ));
        }
        let mut seen_hoisted_bindings = vec![false; function.bindings.len()];
        for hoist in &function.hoisted_functions {
            let binding = function
                .bindings
                .get(hoist.binding.0)
                .ok_or_else(|| Error::internal("hoisted function binding is out of bounds"))?;
            if std::mem::replace(&mut seen_hoisted_bindings[hoist.binding.0], true)
                || binding.storage_scope != function.var_scope
                || binding.kind != BindingKind::Normal
                || !matches!(
                    binding.storage,
                    BindingStorage::Argument(_) | BindingStorage::Local(_)
                )
            {
                return Err(Error::internal(
                    "hoisted function has malformed frame binding metadata",
                ));
            }
            let constant = usize::try_from(hoist.constant)
                .map_err(|_| Error::internal("hoisted function constant is out of bounds"))?;
            let Some(IrConstant::Child(child)) = function.constants.get(constant) else {
                return Err(Error::internal(
                    "hoisted function does not reference child bytecode",
                ));
            };
            let child = tree
                .functions
                .get(*child)
                .ok_or_else(|| Error::internal("hoisted child function is out of bounds"))?;
            if child.function_name.as_deref() != Some(binding.name.as_str())
                || child.private_name_binding
            {
                return Err(Error::internal(
                    "hoisted child name metadata disagrees with its binding",
                ));
            }
        }
        if function.function_hoists_installed {
            let mut hoist_start = 0_usize;
            for (local, pseudo) in [
                (function.home_object_local, PseudoBinding::HomeObject),
                (
                    function.active_function_local,
                    PseudoBinding::ActiveFunction,
                ),
                (function.new_target_local, PseudoBinding::NewTarget),
                (
                    function
                        .this_local
                        .filter(|_| !function.derived_class_constructor),
                    PseudoBinding::This,
                ),
            ] {
                let Some(local) = local else {
                    continue;
                };
                let push_matches = match pseudo {
                    PseudoBinding::HomeObject => matches!(
                        function.ops.get(hoist_start),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::PushHomeObject),
                            pc_site: None,
                        })
                    ),
                    PseudoBinding::ActiveFunction => matches!(
                        function.ops.get(hoist_start),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::PushActiveFunction),
                            pc_site: None,
                        })
                    ),
                    PseudoBinding::NewTarget => matches!(
                        function.ops.get(hoist_start),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::PushNewTarget),
                            pc_site: None,
                        })
                    ),
                    PseudoBinding::This => matches!(
                        function.ops.get(hoist_start),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::PushThis),
                            pc_site: None,
                        })
                    ),
                };
                if !push_matches
                    || !matches!(
                        function.ops.get(hoist_start + 1),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::PutLocal(target)),
                            pc_site: None,
                        }) if *target == local
                    )
                {
                    return Err(Error::internal(
                        "installed pseudo-binding prologue is malformed",
                    ));
                }
                hoist_start += 2;
            }
            if let Some(local) = function.arguments_local {
                let expected_kind = if function.strict || !function.has_simple_parameter_list {
                    ArgumentsKind::Unmapped
                } else {
                    ArgumentsKind::Mapped
                };
                if !matches!(
                    function.ops.get(hoist_start),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::Arguments(kind)),
                        pc_site: None,
                    }) if *kind == expected_kind
                ) {
                    return Err(Error::internal(
                        "installed arguments-object prologue is malformed",
                    ));
                }
                hoist_start += 1;
                if let Some(synthetic) = function.synthetic_parameter_arguments_local {
                    if !matches!(
                        function.ops.get(hoist_start),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::Dup),
                            pc_site: None,
                        })
                    ) || !matches!(
                        function.ops.get(hoist_start + 1),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::InitializeLocal(target)),
                            pc_site: None,
                        }) if *target == synthetic
                    ) {
                        return Err(Error::internal(
                            "installed parameter arguments alias prologue is malformed",
                        ));
                    }
                    hoist_start += 2;
                }
                if !matches!(
                    function.ops.get(hoist_start),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::PutLocal(target)),
                        pc_site: None,
                    }) if *target == local
                ) {
                    return Err(Error::internal(
                        "installed arguments-object prologue is malformed",
                    ));
                }
                hoist_start += 1;
            }
            for local in [
                function.eval_variable_object_local,
                function.arg_eval_variable_object_local,
            ]
            .into_iter()
            .flatten()
            {
                if !matches!(
                    function.ops.get(hoist_start),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::VariableEnvironment),
                        pc_site: None,
                    })
                ) || !matches!(
                    function.ops.get(hoist_start + 1),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::PutLocal(target)),
                        pc_site: None,
                    }) if *target == local
                ) {
                    return Err(Error::internal(
                        "installed eval-variable-object prologue is malformed",
                    ));
                }
                hoist_start += 2;
            }
            if let Some(rest) = function.rest_parameter.filter(|_| {
                function.parameter_scope.is_none() && !function.pattern_parameter_initialization
            }) {
                if function.class_constructor {
                    if !matches!(
                        function.ops.get(hoist_start),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::CheckCtor),
                            pc_site: None,
                        })
                    ) {
                        return Err(Error::internal(
                            "class rest constructor guard escaped its entry prologue",
                        ));
                    }
                    hoist_start += 1;
                    if !function.derived_class_constructor {
                        if !matches!(
                            function.ops.get(hoist_start..hoist_start + 4),
                            Some([
                                SpannedIrOp {
                                    op: IrOp::Bytecode(Instruction::PushThis),
                                    pc_site: None,
                                },
                                SpannedIrOp {
                                    op: IrOp::Bytecode(Instruction::PushActiveFunction),
                                    pc_site: None,
                                },
                                SpannedIrOp {
                                    op: IrOp::Bytecode(Instruction::CallClassInstanceInitializer,),
                                    pc_site: None,
                                },
                                SpannedIrOp {
                                    op: IrOp::Bytecode(Instruction::Drop),
                                    pc_site: None,
                                },
                            ])
                        ) {
                            return Err(Error::internal(
                                "base class rest constructor lost its field initializer hook",
                            ));
                        }
                        hoist_start += 4;
                    }
                }
                if !matches!(
                    function.ops.get(hoist_start),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::Rest(target)),
                        pc_site: None,
                    }) if *target == rest
                ) || !matches!(
                    function.ops.get(hoist_start + 1),
                    Some(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::PutArg(target)),
                        pc_site: None,
                    }) if *target == rest
                ) {
                    return Err(Error::internal(
                        "installed rest parameter prologue is malformed",
                    ));
                }
            }
            let body_hoist_start = function
                .ops
                .iter()
                .position(|operation| {
                    matches!(
                        operation.op,
                        IrOp::EnterScope(scope) if scope == function.body_scope
                    )
                })
                .and_then(|entry| entry.checked_add(1))
                .ok_or_else(|| Error::internal("installed function has no body scope entry"))?;
            for (ordinal, hoist) in ordered_hoisted_functions(function)?.into_iter().enumerate() {
                let closure_pc = ordinal
                    .checked_mul(2)
                    .and_then(|pc| pc.checked_add(body_hoist_start))
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                if !matches!(
                    function.ops.get(closure_pc),
                    Some(SpannedIrOp {
                        op: IrOp::MakeClosure(constant),
                        pc_site: None,
                    }) if *constant == hoist.constant
                ) {
                    return Err(Error::internal(
                        "installed function hoist lost its child closure",
                    ));
                }
                let binding = &function.bindings[hoist.binding.0];
                let write_matches = match binding.storage {
                    BindingStorage::Argument(index) => matches!(
                        function.ops.get(closure_pc + 1),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::PutArg(target)),
                            pc_site: None,
                        }) if *target == index
                    ),
                    BindingStorage::Local(index) => match binding.kind {
                        BindingKind::Lexical { .. } => matches!(
                            function.ops.get(closure_pc + 1),
                            Some(SpannedIrOp {
                                op: IrOp::Bytecode(Instruction::PutLocalCheck(target)),
                                pc_site: None,
                            }) if *target == index
                        ),
                        _ => matches!(
                            function.ops.get(closure_pc + 1),
                            Some(SpannedIrOp {
                                op: IrOp::Bytecode(Instruction::PutLocal(target)),
                                pc_site: None,
                            }) if *target == index
                        ),
                    },
                    BindingStorage::Global => false,
                    BindingStorage::External(_) => false,
                    BindingStorage::Module(_) => false,
                };
                if !write_matches {
                    return Err(Error::internal(
                        "installed function hoist targeted the wrong frame slot",
                    ));
                }
            }
        }
        let mut seen_scoped_function_bindings = vec![false; function.bindings.len()];
        for scoped in &function.scoped_functions {
            let binding = function
                .bindings
                .get(scoped.binding.0)
                .ok_or_else(|| Error::internal("scoped function binding is out of bounds"))?;
            let scope_kind = function.scopes[binding.storage_scope.0].kind;
            if std::mem::replace(&mut seen_scoped_function_bindings[scoped.binding.0], true)
                || binding.storage_scope != binding.declaration_scope
                || binding.kind != (BindingKind::Lexical { is_const: false })
                || !binding.is_scoped_function
                || !matches!(binding.storage, BindingStorage::Local(_))
                || !(matches!(
                    scope_kind,
                    ScopeKind::Block | ScopeKind::If | ScopeKind::Switch | ScopeKind::FunctionBody
                ) || (matches!(scope_kind, ScopeKind::ProgramBody)
                    && matches!(function.kind, FunctionKind::Eval(_))
                    && binding.storage_scope == function.body_scope))
            {
                return Err(Error::internal(
                    "scoped function has malformed lexical binding metadata",
                ));
            }
            let constant = usize::try_from(scoped.constant)
                .map_err(|_| Error::internal("scoped function constant is out of bounds"))?;
            let Some(IrConstant::Child(child_id)) = function.constants.get(constant) else {
                return Err(Error::internal(
                    "scoped function does not reference child bytecode",
                ));
            };
            let child = tree
                .functions
                .get(*child_id)
                .ok_or_else(|| Error::internal("scoped child function is out of bounds"))?;
            if child.function_name.as_deref() != Some(binding.name.as_str())
                || (child.execution_kind != BytecodeFunctionKind::Normal)
                    != binding.is_scoped_generator
                || child.private_name_binding
                || child.parent
                    != Some(ParentLink {
                        function: function_id,
                        definition_scope: binding.storage_scope,
                    })
            {
                return Err(Error::internal(
                    "scoped child name or parent metadata disagrees with its binding",
                ));
            }
            if !matches!(
                function.ops.get(scoped.authored_closure),
                Some(SpannedIrOp {
                    op: IrOp::MakeClosure(found),
                    pc_site: None,
                }) if *found == scoped.constant
            ) {
                return Err(Error::internal(
                    "scoped function lost its authored closure allocation",
                ));
            }
            let drop_offset = if let Some(annex_binding) = scoped.annex_binding {
                if binding.is_scoped_generator {
                    return Err(Error::internal(
                        "scoped generator retained an Annex B outer binding",
                    ));
                }
                if function.strict
                    || !matches!(
                        function.ops.get(scoped.authored_closure + 1),
                        Some(SpannedIrOp {
                            op: IrOp::Bytecode(Instruction::Dup),
                            pc_site: None,
                        })
                    )
                {
                    return Err(Error::internal(
                        "Annex B function lost its duplicate outer value",
                    ));
                }
                let write = function.ops.get(scoped.authored_closure + 2);
                let (unresolved, resolved) = match annex_binding {
                    IrAnnexBinding::Static(annex_binding) => {
                        let annex = function.bindings.get(annex_binding.0).ok_or_else(|| {
                            Error::internal("Annex B function binding is out of bounds")
                        })?;
                        if annex.storage_scope != function.var_scope
                            || (annex.kind != BindingKind::Normal
                                && !matches!(annex.storage, BindingStorage::External(_)))
                            || annex.name != binding.name
                        {
                            return Err(Error::internal(
                                "Annex B function has malformed root binding metadata",
                            ));
                        }
                        let unresolved = matches!(
                            write,
                            Some(SpannedIrOp {
                                op: IrOp::Identifier {
                                    name,
                                    scope,
                                    access: IdentifierAccess::AnnexBPut,
                                    ..
                                },
                                pc_site: None,
                            }) if name == &binding.name && *scope == function.var_scope
                        );
                        let resolved = match annex.storage {
                            BindingStorage::Local(index) => matches!(
                                write,
                                Some(SpannedIrOp {
                                    op: IrOp::Bytecode(Instruction::PutLocal(target)),
                                    pc_site: None,
                                }) if *target == index
                            ),
                            BindingStorage::Global => matches!(
                                write,
                                Some(SpannedIrOp {
                                    op: IrOp::Bytecode(Instruction::PutVar(index)),
                                    pc_site: None,
                                }) if function.closure_variables.get(usize::from(*index)).is_some_and(
                                    |descriptor| {
                                        descriptor.source == ClosureSource::Global
                                            && match descriptor.name {
                                                ClosureVariableName::Constant(name) => matches!(
                                                    function.constants.get(name as usize),
                                                    Some(IrConstant::Primitive(Value::String(found)))
                                                        if found.to_utf8_lossy() == annex.name
                                                ),
                                                _ => false,
                                            }
                                    }
                                )
                            ),
                            BindingStorage::External(index) => matches!(
                                write,
                                Some(SpannedIrOp {
                                    op: IrOp::Bytecode(
                                        Instruction::PutVarRef(target)
                                            | Instruction::PutVarRefCheck(target)
                                    ),
                                    pc_site: None,
                                }) if *target == index
                            ),
                            BindingStorage::Argument(_) => false,
                            BindingStorage::Module(_) => false,
                        };
                        (unresolved, resolved)
                    }
                    IrAnnexBinding::Dynamic => {
                        let unresolved = matches!(
                            write,
                            Some(SpannedIrOp {
                                op: IrOp::Identifier {
                                    name,
                                    scope,
                                    access: IdentifierAccess::Put,
                                    ..
                                },
                                pc_site: None,
                            }) if name == &binding.name && *scope == function.var_scope
                        );
                        let resolved = matches!(
                            write,
                            Some(SpannedIrOp {
                                op: IrOp::DynamicIdentifier {
                                    access: IdentifierAccess::Put,
                                    ..
                                },
                                pc_site: None,
                            })
                        );
                        (unresolved, resolved)
                    }
                };
                if !unresolved && !resolved {
                    return Err(Error::internal(
                        "Annex B function targeted the wrong outer binding",
                    ));
                }
                3
            } else {
                1
            };
            if !matches!(
                function.ops.get(scoped.authored_closure + drop_offset),
                Some(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::Drop),
                    pc_site: None,
                })
            ) {
                return Err(Error::internal(
                    "scoped function lost its authored closure drop",
                ));
            }
        }
        if function
            .bindings
            .iter()
            .enumerate()
            .any(|(index, binding)| {
                (binding.is_scoped_function && !seen_scoped_function_bindings[index])
                    || (binding.is_scoped_generator && !binding.is_scoped_function)
            })
        {
            return Err(Error::internal(
                "scoped function binding has no child declaration record",
            ));
        }
        for annex in &function.program_annex_functions {
            if !matches!(function.kind, FunctionKind::Script) || function.strict {
                return Err(Error::internal(
                    "Program Annex B function escaped sloppy script code",
                ));
            }
            let binding = function
                .bindings
                .get(annex.binding.0)
                .ok_or_else(|| Error::internal("Program Annex B binding is out of bounds"))?;
            if binding.storage_scope != function.var_scope
                || binding.storage != BindingStorage::Global
                || binding.kind != BindingKind::Normal
            {
                return Err(Error::internal(
                    "Program Annex B function has malformed global binding metadata",
                ));
            }
            let constant = usize::try_from(annex.constant).map_err(|_| {
                Error::internal("Program Annex B function constant is out of bounds")
            })?;
            let Some(IrConstant::Child(child_id)) = function.constants.get(constant) else {
                return Err(Error::internal(
                    "Program Annex B function does not reference child bytecode",
                ));
            };
            let child = tree.functions.get(*child_id).ok_or_else(|| {
                Error::internal("Program Annex B child function is out of bounds")
            })?;
            if child.function_name.as_deref() != Some(binding.name.as_str())
                || child.private_name_binding
                || child.parent
                    != Some(ParentLink {
                        function: function_id,
                        definition_scope: function.body_scope,
                    })
            {
                return Err(Error::internal(
                    "Program Annex B child metadata disagrees with its binding",
                ));
            }
            if !matches!(
                function.ops.get(annex.authored_closure),
                Some(SpannedIrOp {
                    op: IrOp::MakeClosure(found),
                    pc_site: None,
                }) if *found == annex.constant
            ) || !matches!(
                function.ops.get(annex.authored_closure + 1),
                Some(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::Dup),
                    pc_site: None,
                })
            ) {
                return Err(Error::internal(
                    "Program Annex B function lost its authored closure allocation",
                ));
            }
            let outer_write = function.ops.get(annex.authored_closure + 2);
            let unresolved_outer = matches!(
                outer_write,
                Some(SpannedIrOp {
                    op: IrOp::Identifier {
                        name,
                        scope,
                        access: IdentifierAccess::AnnexBPut,
                        ..
                    },
                    pc_site: None,
                }) if name == &binding.name && *scope == function.var_scope
            );
            let resolved_outer = matches!(
                outer_write,
                Some(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::PutVar(index)),
                    pc_site: None,
                }) if function.closure_variables.get(usize::from(*index)).is_some_and(
                    |descriptor| {
                        descriptor.source == ClosureSource::Global
                            && match descriptor.name {
                                ClosureVariableName::Constant(name) => matches!(
                                    function.constants.get(name as usize),
                                    Some(IrConstant::Primitive(Value::String(found)))
                                        if found.to_utf8_lossy() == binding.name
                                ),
                                _ => false,
                            }
                    }
                )
            );
            if !unresolved_outer && !resolved_outer {
                return Err(Error::internal(
                    "Program Annex B function targeted the wrong root binding",
                ));
            }

            let current_write = function.ops.get(annex.authored_closure + 3);
            let unresolved_current = matches!(
                current_write,
                Some(SpannedIrOp {
                    op: IrOp::Identifier {
                        name,
                        scope,
                        access: IdentifierAccess::Put,
                        ..
                    },
                    pc_site: None,
                }) if name == &binding.name && *scope == function.body_scope
            );
            let resolved_current = matches!(
                current_write,
                Some(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::PutVar(index)),
                    pc_site: None,
                }) if function.closure_variables.get(usize::from(*index)).is_some_and(
                    |descriptor| {
                        descriptor.source == ClosureSource::GlobalDeclaration
                            && match descriptor.name {
                                ClosureVariableName::Constant(name) => matches!(
                                    function.constants.get(name as usize),
                                    Some(IrConstant::Primitive(Value::String(found)))
                                        if found.to_utf8_lossy() == binding.name
                                ),
                                _ => false,
                            }
                    }
                )
            );
            if !unresolved_current && !resolved_current {
                return Err(Error::internal(
                    "Program Annex B function lost its current-environment write",
                ));
            }
        }
        match function.kind {
            FunctionKind::Script | FunctionKind::Eval(_) => {
                for declaration in &function.global_declarations {
                    let binding_scope = if declaration.is_lexical {
                        function.body_scope
                    } else {
                        function.var_scope
                    };
                    let binding = function.scopes[binding_scope.0]
                        .bindings
                        .iter()
                        .rev()
                        .map(|binding| &function.bindings[binding.0])
                        .find(|binding| {
                            binding.name == declaration.name
                                && binding.storage == BindingStorage::Global
                        });
                    let expected_kind = if declaration.is_lexical {
                        BindingKind::Lexical {
                            is_const: declaration.is_const,
                        }
                    } else {
                        if declaration.is_const {
                            return Err(Error::internal(
                                "ordinary global declaration is marked const",
                            ));
                        }
                        BindingKind::Normal
                    };
                    let masked_program_lexical = declaration.is_lexical
                        && function.first_global_declaration_is_normal(&declaration.name);
                    if binding.is_none_or(|binding| {
                        binding.storage != BindingStorage::Global
                            || if masked_program_lexical {
                                !matches!(binding.kind, BindingKind::Lexical { .. })
                            } else {
                                binding.kind != expected_kind
                            }
                    }) {
                        return Err(Error::internal(
                            "global declaration has no matching binding identity",
                        ));
                    }
                    if let Some(constant) = declaration.function_constant {
                        if declaration.is_lexical || declaration.is_const {
                            return Err(Error::internal(
                                "global function declaration has lexical metadata",
                            ));
                        }
                        let constant = usize::try_from(constant).map_err(|_| {
                            Error::internal("global function constant is out of bounds")
                        })?;
                        if !matches!(function.constants.get(constant), Some(IrConstant::Child(_))) {
                            return Err(Error::internal(
                                "global function declaration does not reference child bytecode",
                            ));
                        }
                    }
                    if let Some(index) = declaration.closure_index {
                        let descriptor = function
                            .closure_variables
                            .get(usize::from(index))
                            .ok_or_else(|| {
                                Error::internal("global declaration closure is out of bounds")
                            })?;
                        let expected_kind = if declaration.function_constant.is_some() {
                            ClosureVariableKind::GlobalFunction
                        } else {
                            ClosureVariableKind::Normal
                        };
                        if descriptor.source != ClosureSource::GlobalDeclaration
                            || descriptor.is_lexical != declaration.is_lexical
                            || descriptor.is_const != declaration.is_const
                            || descriptor.kind != expected_kind
                        {
                            return Err(Error::internal(
                                "global declaration closure metadata disagrees",
                            ));
                        }
                    }
                }
            }
            FunctionKind::Module => {
                if !function.global_declarations.is_empty() {
                    return Err(Error::internal(
                        "module root retained realm-global declarations",
                    ));
                }
            }
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                if function.global_declarations.is_empty() => {}
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow => {
                return Err(Error::internal(
                    "ordinary function contains Program global declarations",
                ));
            }
        }
        if let Some(parent_link) = function.parent {
            let parent = tree
                .functions
                .get(parent_link.function)
                .ok_or_else(|| Error::internal("function parent is out of bounds"))?;
            if parent_link.definition_scope.0 >= parent.scopes.len() {
                return Err(Error::internal("child definition scope is out of bounds"));
            }
        }

        for (scope_index, scope) in function.scopes.iter().enumerate() {
            let valid_parameter_root = scope.kind == ScopeKind::Parameter
                && function.parameter_scope == Some(ScopeId(scope_index))
                && scope.parent.is_none();
            if scope_index > 0
                && !valid_parameter_root
                && scope.parent.is_none_or(|parent| parent.0 >= scope_index)
            {
                return Err(Error::internal("lexical scope parent is malformed"));
            }
        }

        let mut seen_bindings = vec![false; function.bindings.len()];
        let mut seen_arguments = vec![false; function.parameters.len()];
        let mut seen_locals = vec![false; function.locals.len()];
        let mut seen_external = vec![false; function.external_bindings.len()];
        for (index, external) in function.external_bindings.iter().enumerate() {
            let sentinel = match external.kind {
                ClosureVariableKind::EvalVariableObject => EVAL_VARIABLE_OBJECT_LOCAL_NAME,
                ClosureVariableKind::ArgEvalVariableObject => ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME,
                ClosureVariableKind::WithObject => WITH_OBJECT_LOCAL_NAME,
                ClosureVariableKind::Normal
                | ClosureVariableKind::ModuleImportView
                | ClosureVariableKind::FunctionName
                | ClosureVariableKind::GlobalFunction
                | ClosureVariableKind::PrivateField
                | ClosureVariableKind::PrivateMethod
                | ClosureVariableKind::PrivateGetter
                | ClosureVariableKind::PrivateSetter
                | ClosureVariableKind::PrivateGetterSetter => continue,
            };
            let descriptor = function
                .closure_variables
                .get(index)
                .ok_or_else(|| Error::internal("eval hidden object closure is out of bounds"))?;
            if external.is_lexical
                || external.is_const
                || external.is_catch_parameter
                || external.name.to_utf8_lossy() != sentinel
                || descriptor.source
                    != ClosureSource::EvalEnvironment(u16::try_from(index).map_err(|_| {
                        Error::new(ErrorKind::JsInternal, "too many closure variables")
                    })?)
                || descriptor.kind != external.kind
                || descriptor.is_lexical
                || descriptor.is_const
            {
                return Err(Error::internal(
                    "eval hidden object external metadata is malformed",
                ));
            }
        }
        for (scope_index, scope) in function.scopes.iter().enumerate() {
            if scope.kind == ScopeKind::With
                && (scope.bindings.len() != 1
                    || scope.bindings.first().is_none_or(|binding| {
                        function.bindings.get(binding.0).is_none_or(|binding| {
                            binding.kind != BindingKind::WithObject
                                || !matches!(binding.storage, BindingStorage::Local(_))
                        })
                    }))
            {
                return Err(Error::internal(
                    "with scope does not own exactly one hidden object binding",
                ));
            }
            for &binding_id in &scope.bindings {
                let binding = function
                    .bindings
                    .get(binding_id.0)
                    .ok_or_else(|| Error::internal("scope binding is out of bounds"))?;
                if std::mem::replace(&mut seen_bindings[binding_id.0], true) {
                    return Err(Error::internal(
                        "binding appears more than once in the scope graph",
                    ));
                }
                if binding.storage_scope != ScopeId(scope_index)
                    || binding.declaration_scope.0 >= function.scopes.len()
                {
                    return Err(Error::internal("binding scope metadata is malformed"));
                }
                let mut declaration_ancestor = Some(binding.declaration_scope);
                while let Some(scope) = declaration_ancestor {
                    if scope == binding.storage_scope {
                        break;
                    }
                    declaration_ancestor = function.scopes[scope.0].parent;
                }
                if declaration_ancestor.is_none() {
                    return Err(Error::internal(
                        "binding storage scope does not contain its declaration",
                    ));
                }
                if binding
                    .declaration_span
                    .is_some_and(|span| span.start.byte_offset > span.end.byte_offset)
                {
                    return Err(Error::internal("binding declaration span is malformed"));
                }
                match binding.storage {
                    BindingStorage::Argument(index) => {
                        let index = usize::from(index);
                        let parameter = function
                            .parameters
                            .get(index)
                            .and_then(Option::as_ref)
                            .ok_or_else(|| {
                                Error::internal("argument binding referenced an unnamed slot")
                            })?;
                        if binding.is_catch_parameter
                            || binding.storage_scope != function.var_scope
                            || binding.declaration_scope != function.var_scope
                            || binding.kind != BindingKind::Normal
                            || binding.name != *parameter
                        {
                            return Err(Error::internal("argument binding metadata is malformed"));
                        }
                        if std::mem::replace(&mut seen_arguments[index], true) {
                            return Err(Error::internal(
                                "argument slot has more than one binding identity",
                            ));
                        }
                    }
                    BindingStorage::Local(index) => {
                        let index = usize::from(index);
                        let local = function
                            .locals
                            .get(index)
                            .ok_or_else(|| Error::internal("local binding is out of bounds"))?;
                        if binding.name != *local {
                            return Err(Error::internal("local binding metadata is malformed"));
                        }
                        let catch_parameter_metadata = binding.is_catch_parameter
                            && binding.storage_scope == binding.declaration_scope
                            && function.scopes[binding.storage_scope.0].kind == ScopeKind::Catch
                            && binding.kind == (BindingKind::Lexical { is_const: false });
                        if binding.is_catch_parameter != catch_parameter_metadata {
                            return Err(Error::internal(
                                "catch parameter binding metadata is malformed",
                            ));
                        }
                        if binding.kind == BindingKind::WithObject
                            && (binding.name != WITH_OBJECT_LOCAL_NAME
                                || binding.storage_scope != binding.declaration_scope
                                || function.scopes[binding.storage_scope.0].kind != ScopeKind::With
                                || binding.is_catch_parameter)
                        {
                            return Err(Error::internal(
                                "with object local binding metadata is malformed",
                            ));
                        }
                        if matches!(
                            binding.kind,
                            BindingKind::PrivateField { .. }
                                | BindingKind::PrivateMethod { .. }
                                | BindingKind::PrivateGetter { .. }
                                | BindingKind::PrivateSetter { .. }
                                | BindingKind::PrivateGetterSetter { .. }
                        ) && (!binding.name.starts_with('#')
                            || binding.name.len() == 1
                            || binding.storage_scope != binding.declaration_scope
                            || function.scopes[binding.storage_scope.0].kind
                                != ScopeKind::ClassPrivate
                            || binding.is_catch_parameter)
                        {
                            return Err(Error::internal(
                                "private-field local binding metadata is malformed",
                            ));
                        }
                        if matches!(binding.kind, BindingKind::Lexical { .. }) {
                            let scope_kind = function.scopes[binding.storage_scope.0].kind;
                            let supported_scope = matches!(
                                scope_kind,
                                ScopeKind::Parameter
                                    | ScopeKind::Block
                                    | ScopeKind::ClassPrivate
                                    | ScopeKind::If
                                    | ScopeKind::For
                                    | ScopeKind::Switch
                                    | ScopeKind::Catch
                            ) || (matches!(
                                scope_kind,
                                ScopeKind::FunctionBody
                            ) && matches!(
                                function.kind,
                                FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                            ) && binding.storage_scope
                                == function.body_scope)
                                || (scope_kind == ScopeKind::ProgramBody
                                    && matches!(function.kind, FunctionKind::Eval(_))
                                    && binding.storage_scope == function.body_scope);
                            let supported_scope = supported_scope
                                || (scope_kind == ScopeKind::FunctionRoot
                                    && function.derived_class_constructor
                                    && binding.name == THIS_LOCAL_NAME
                                    && binding.kind == (BindingKind::Lexical { is_const: false })
                                    && binding.storage_scope == function.var_scope);
                            if binding.storage_scope != binding.declaration_scope
                                || !supported_scope
                            {
                                return Err(Error::internal(
                                    "lexical binding scope metadata is malformed",
                                ));
                            }
                        }
                        if std::mem::replace(&mut seen_locals[index], true) {
                            return Err(Error::internal(
                                "local slot has more than one binding identity",
                            ));
                        }
                    }
                    BindingStorage::Global => {
                        let valid_lexical = matches!(binding.kind, BindingKind::Lexical { .. })
                            && binding.storage_scope == function.body_scope
                            && binding.declaration_scope == function.body_scope
                            && function.scopes[binding.storage_scope.0].kind
                                == ScopeKind::ProgramBody;
                        let valid_var = binding.kind == BindingKind::Normal
                            && binding.storage_scope == function.var_scope;
                        if binding.is_catch_parameter
                            || !(matches!(function.kind, FunctionKind::Script)
                                || matches!(function.kind, FunctionKind::Eval(_)) && valid_var)
                            || (!valid_lexical && !valid_var)
                        {
                            return Err(Error::internal("global binding metadata is malformed"));
                        }
                    }
                    BindingStorage::External(index) => {
                        let external_index = usize::from(index);
                        let external =
                            function
                                .external_bindings
                                .get(external_index)
                                .ok_or_else(|| {
                                    Error::internal("eval external binding is out of bounds")
                                })?;
                        if std::mem::replace(&mut seen_external[external_index], true) {
                            return Err(Error::internal(
                                "eval external slot has more than one binding identity",
                            ));
                        }
                        let expected_kind = binding_kind_from_closure_flags(
                            external.kind,
                            external.is_lexical,
                            external.is_const,
                        )
                        .ok_or_else(|| {
                            Error::internal("eval external binding flags are inconsistent")
                        })?;
                        if !matches!(function.kind, FunctionKind::Eval(EvalKind::Direct))
                            || function.parent.is_some()
                            || binding.storage_scope != function.var_scope
                            || binding.declaration_scope != function.var_scope
                            || binding.declaration_span.is_some()
                            || binding.is_catch_parameter != external.is_catch_parameter
                            || binding.name != external.name.to_utf8_lossy()
                            || !binding_kinds_compatible(binding.kind, expected_kind)
                        {
                            return Err(Error::internal(
                                "eval external binding metadata is malformed",
                            ));
                        }
                        let descriptor = function
                            .closure_variables
                            .get(external_index)
                            .ok_or_else(|| {
                                Error::internal("eval external closure is out of bounds")
                            })?;
                        if descriptor.source != ClosureSource::EvalEnvironment(index)
                            || descriptor.is_lexical != external.is_lexical
                            || descriptor.is_const != external.is_const
                            || descriptor.kind != external.kind
                            || !matches!(
                                descriptor.name,
                                ClosureVariableName::Constant(name)
                                    if matches!(
                                        function.constants.get(name as usize),
                                        Some(IrConstant::Primitive(Value::String(found)))
                                            if found == &external.name
                                    )
                            )
                        {
                            return Err(Error::internal(
                                "eval external closure metadata disagrees",
                            ));
                        }
                    }
                    BindingStorage::Module(binding_id) => {
                        if !matches!(function.kind, FunctionKind::Module)
                            || function.parent.is_some()
                            || binding.is_catch_parameter
                        {
                            return Err(Error::internal(
                                "module binding metadata escaped the module root",
                            ));
                        }
                        let module_binding = tree
                            .module
                            .as_ref()
                            .ok_or_else(|| Error::internal("module binding has no record"))?
                            .binding(binding_id)?;
                        if module_binding.name != binding.name {
                            return Err(Error::internal("module binding metadata disagrees"));
                        }
                    }
                }
            }
        }
        if seen_bindings.iter().any(|seen| !seen) {
            return Err(Error::internal("binding is missing from the scope graph"));
        }
        if seen_arguments
            .iter()
            .zip(&function.parameters)
            .any(|(seen, parameter)| !*seen && parameter.is_some())
        {
            return Err(Error::internal(
                "argument slot is missing its binding identity",
            ));
        }
        if seen_arguments
            .iter()
            .zip(&function.parameters)
            .any(|(seen, parameter)| *seen && parameter.is_none())
        {
            return Err(Error::internal(
                "anonymous argument slot retained a binding identity",
            ));
        }
        if seen_external.iter().any(|seen| !seen) {
            return Err(Error::internal(
                "eval external slot is missing its binding identity",
            ));
        }
        let eval_ret_index = function.eval_ret_local.map(usize::from);
        let mut seen_synthetic = vec![false; function.locals.len()];
        let mut synthetic_eval_ret = None;
        for synthetic in &function.synthetic_locals {
            let index = usize::from(synthetic.index);
            let name = function
                .locals
                .get(index)
                .ok_or_else(|| Error::internal("synthetic local is out of bounds"))?;
            if std::mem::replace(
                seen_synthetic
                    .get_mut(index)
                    .ok_or_else(|| Error::internal("synthetic local is out of bounds"))?,
                true,
            ) || name != synthetic.kind.name()
            {
                return Err(Error::internal("synthetic local metadata is malformed"));
            }
            match synthetic.kind {
                SyntheticLocalKind::EvalCompletion => {
                    if synthetic_eval_ret.replace(index).is_some() {
                        return Err(Error::internal(
                            "eval completion slot metadata is malformed",
                        ));
                    }
                }
                SyntheticLocalKind::FinallySavedEvalCompletion
                    if matches!(function.kind, FunctionKind::Script | FunctionKind::Eval(_)) => {}
                SyntheticLocalKind::FinallySavedEvalCompletion => {
                    return Err(Error::internal(
                        "ordinary function contains a finally eval-completion save slot",
                    ));
                }
            }
        }
        match function.kind {
            FunctionKind::Script | FunctionKind::Eval(_)
                if eval_ret_index == Some(0)
                    && synthetic_eval_ret == eval_ret_index
                    && function
                        .locals
                        .first()
                        .is_some_and(|name| name == EVAL_RET_LOCAL_NAME) => {}
            FunctionKind::Module
            | FunctionKind::Ordinary
            | FunctionKind::Method
            | FunctionKind::Arrow
                if eval_ret_index.is_none() && synthetic_eval_ret.is_none() => {}
            _ => {
                return Err(Error::internal(
                    "eval completion slot metadata is malformed",
                ));
            }
        }
        if function.bindings.iter().any(|binding| {
            matches!(
                binding.name.as_str(),
                EVAL_RET_LOCAL_NAME | FINALLY_EVAL_RET_LOCAL_NAME
            )
        }) || function.locals.iter().enumerate().any(|(index, name)| {
            matches!(
                name.as_str(),
                EVAL_RET_LOCAL_NAME | FINALLY_EVAL_RET_LOCAL_NAME
            ) && !seen_synthetic[index]
        }) {
            return Err(Error::internal(
                "synthetic local leaked into source binding lookup",
            ));
        }
        for (index, seen) in seen_locals.into_iter().enumerate() {
            if seen_synthetic[index] {
                if seen {
                    return Err(Error::internal(
                        "synthetic local has a source binding identity",
                    ));
                }
            } else if !seen {
                return Err(Error::internal(
                    "local slot is missing its binding identity",
                ));
            }
        }

        let mut scope_entries = vec![0_usize; function.scopes.len()];
        let mut catch_scope_preparations = vec![0_usize; function.scopes.len()];
        let mut scope_leaves = vec![0_usize; function.scopes.len()];
        for operation in &function.ops {
            let (scope, counts) = match operation.op {
                IrOp::EnterScope(scope) => (scope, &mut scope_entries),
                IrOp::PrepareCatchScope(scope) => (scope, &mut catch_scope_preparations),
                IrOp::LeaveScope(scope) => (scope, &mut scope_leaves),
                _ => continue,
            };
            let count = counts
                .get_mut(scope.0)
                .ok_or_else(|| Error::internal("scope lifecycle target is out of bounds"))?;
            *count = count
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "out of memory"))?;
        }
        for (scope_index, scope) in function.scopes.iter().enumerate() {
            let entries = scope_entries[scope_index];
            let catch_preparations = catch_scope_preparations[scope_index];
            let leaves = scope_leaves[scope_index];
            match scope.kind {
                ScopeKind::Catch if catch_preparations == 1 => {}
                ScopeKind::Catch => {
                    return Err(Error::internal(
                        "catch scope preparation metadata is malformed",
                    ));
                }
                _ if catch_preparations == 0 => {}
                _ => {
                    return Err(Error::internal(
                        "non-catch scope has catch preparation metadata",
                    ));
                }
            }
            if scope_index == function.var_scope.0 {
                if entries != 0 || leaves != 0 {
                    return Err(Error::internal(
                        "function root unexpectedly has scope lifecycle operations",
                    ));
                }
            } else if scope_index == function.body_scope.0 {
                match function.kind {
                    FunctionKind::Script | FunctionKind::Module if entries == 0 && leaves == 0 => {}
                    FunctionKind::Ordinary
                    | FunctionKind::Method
                    | FunctionKind::Arrow
                    | FunctionKind::Eval(_)
                        if entries == 1
                            && leaves == 0
                            && function.ops.iter().any(|operation| {
                                matches!(
                                    operation.op,
                                    IrOp::EnterScope(body) if body == function.body_scope
                                )
                            }) => {}
                    _ => {
                        return Err(Error::internal(
                            "function body scope lifecycle metadata is malformed",
                        ));
                    }
                }
            } else if function.parameter_scope == Some(ScopeId(scope_index)) {
                if scope.kind != ScopeKind::Parameter
                    || scope.parent.is_some()
                    || entries != 1
                    || leaves != 1
                {
                    return Err(Error::internal(
                        "parameter scope lifecycle metadata is malformed",
                    ));
                }
            } else if entries != 1 || leaves == 0 || scope.parent.is_none() {
                return Err(Error::internal(
                    "nested scope lifecycle metadata is malformed",
                ));
            }
        }

        let function_name_bindings = function
            .bindings
            .iter()
            .filter(|binding| matches!(binding.kind, BindingKind::FunctionName { .. }))
            .collect::<Vec<_>>();
        match function.kind {
            FunctionKind::Ordinary => match (
                function.function_name_local,
                function_name_bindings.as_slice(),
            ) {
                (Some(index), [binding])
                    if function.private_name_binding
                        && binding.storage == BindingStorage::Local(index)
                        && binding.storage_scope == function.var_scope
                        && binding.declaration_scope == function.var_scope
                        && function.function_name.as_deref() == Some(binding.name.as_str())
                        && binding.kind
                            == (BindingKind::FunctionName {
                                is_const: function.strict,
                            }) => {}
                // Private function names are materialized lazily when the
                // body references them or an eval site makes them visible.
                (None, []) => {}
                _ => return Err(Error::internal("function-name binding metadata disagrees")),
            },
            FunctionKind::Method | FunctionKind::Arrow
                if function.function_name_local.is_none()
                    && !function.private_name_binding
                    && function_name_bindings.is_empty() => {}
            FunctionKind::Method | FunctionKind::Arrow => {
                return Err(Error::internal(
                    "unnamed function retained private function-name metadata",
                ));
            }
            FunctionKind::Eval(EvalKind::Direct)
                if function.function_name_local.is_none()
                    && !function.private_name_binding
                    && function_name_bindings
                        .iter()
                        .all(|binding| matches!(binding.storage, BindingStorage::External(_))) => {}
            FunctionKind::Script
            | FunctionKind::Module
            | FunctionKind::Eval(EvalKind::Indirect)
                if function.function_name_local.is_none()
                    && !function.private_name_binding
                    && function_name_bindings.is_empty() => {}
            FunctionKind::Eval(EvalKind::None) => {
                return Err(Error::internal("eval root has no eval kind"));
            }
            FunctionKind::Script | FunctionKind::Module | FunctionKind::Eval(_) => {
                return Err(Error::internal("function-name binding metadata disagrees"));
            }
        }
        if let Some(function_name_local) = function.function_name_local {
            let root_bindings = &function.scopes[function.var_scope.0].bindings;
            let Some(function_name_position) = root_bindings.iter().position(|binding| {
                function.bindings[binding.0].storage == BindingStorage::Local(function_name_local)
                    && matches!(
                        function.bindings[binding.0].kind,
                        BindingKind::FunctionName { .. }
                    )
            }) else {
                return Err(Error::internal(
                    "function-name binding is missing from the function root",
                ));
            };
            let function_name = function.function_name.as_deref().ok_or_else(|| {
                Error::internal("function-name local has no intrinsic function name")
            })?;
            if root_bindings[..function_name_position]
                .iter()
                .any(|binding| function.bindings[binding.0].name == function_name)
            {
                return Err(Error::internal(
                    "private function name does not precede same-named body bindings",
                ));
            }
        }
        let root_kind = matches!(
            function.kind,
            FunctionKind::Script | FunctionKind::Module | FunctionKind::Eval(_)
        );
        if (function_id == 0) != function.parent.is_none() || (function_id == 0) != root_kind {
            return Err(Error::internal("function topology is malformed"));
        }
    }
    Ok(())
}
