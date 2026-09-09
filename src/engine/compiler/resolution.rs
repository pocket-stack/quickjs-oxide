//! Resolve lexical names, declaration hoists, eval environments, and closure captures in the IR.

use super::{
    ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME, ArgumentsKind, BindingKind, BindingStorage, ClosureSource,
    ClosureVariable, ClosureVariableKind, ClosureVariableName, DynamicEnvironmentSource,
    EVAL_VARIABLE_OBJECT_LOCAL_NAME, Error, ErrorKind, EvalBinding, EvalBindingSource,
    EvalCallerVariableTarget, EvalDeclarationTarget, EvalDeclarationValue, EvalEnvironment,
    EvalKind, EvalScope, EvalScopeKind, EvalVariableEnvironment, EvalVariableSource, FunctionId,
    FunctionIr, FunctionKind, FunctionTree, HashMap, IdentifierAccess, IdentifierReferenceAccess,
    Instruction, IrConstant, IrHoistedFunction, IrOp, JsString, MAX_LOCAL_VARIABLES,
    PrivateFieldAccess, PseudoBinding, ScopeId, ScopeKind, SourceOffset, Span, SpannedIrOp, Value,
    WITH_OBJECT_LOCAL_NAME, WithObjectSource, binding_kind_from_closure_flags,
    binding_kinds_compatible, ensure_eval_visible_pseudo_bindings,
    find_or_create_own_pseudo_binding, install_pseudo_binding_prologues, module, private_reference,
    source_span, validate_scope_graph,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResolvedBinding {
    pub(super) storage: BindingStorage,
    pub(super) kind: BindingKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FunctionResolutionEvent {
    Enter(FunctionId),
    Resolve(FunctionId),
}

fn function_resolution_events(tree: &FunctionTree) -> Result<Vec<FunctionResolutionEvent>, Error> {
    if tree.functions.is_empty() {
        return Err(Error::internal("compiler produced no root function"));
    }
    let mut children = vec![Vec::new(); tree.functions.len()];
    for (function_id, function) in tree.functions.iter().enumerate().skip(1) {
        let parent = function
            .parent
            .ok_or_else(|| Error::internal("non-root function has no parent"))?
            .function;
        if parent >= function_id {
            return Err(Error::internal(
                "function parent must precede its child in the arena",
            ));
        }
        let siblings = children
            .get_mut(parent)
            .ok_or_else(|| Error::internal("function parent is out of bounds"))?;
        siblings.push(function_id);
    }
    if tree.functions[0].parent.is_some() {
        return Err(Error::internal("root function unexpectedly has a parent"));
    }

    let mut events = Vec::with_capacity(tree.functions.len().saturating_mul(2));
    let mut stack = vec![(0_usize, false)];
    while let Some((function_id, visited)) = stack.pop() {
        if visited {
            events.push(FunctionResolutionEvent::Resolve(function_id));
            continue;
        }
        events.push(FunctionResolutionEvent::Enter(function_id));
        stack.push((function_id, true));
        for &child in children[function_id].iter().rev() {
            stack.push((child, false));
        }
    }
    if events.len() != tree.functions.len().saturating_mul(2) {
        return Err(Error::internal("function arena is not one rooted tree"));
    }
    Ok(events)
}

pub(super) fn resolve_identifiers(tree: &mut FunctionTree) -> Result<(), Error> {
    #[derive(Clone, Copy)]
    enum UnresolvedAccess {
        Identifier(IdentifierAccess),
        IdentifierReference(IdentifierReferenceAccess),
        ImportMeta,
        PrivateField(PrivateFieldAccess),
    }

    install_eval_variable_objects(tree)?;
    validate_scope_graph(tree)?;
    seed_global_declarations(tree)?;
    seed_module_bindings(tree)?;
    module::resolve_module_exports(tree)?;
    // QuickJS enters each function by pre-populating its direct-eval closure
    // table, then creates children depth-first in source order, and only then
    // resolves the parent's ordinary identifiers. The entry event matters:
    // `get_closure_var` is first-slot-wins, so a descendant eval can establish
    // an ancestor relay before that ancestor's own bytecode is resolved.
    for event in function_resolution_events(tree)? {
        match event {
            FunctionResolutionEvent::Enter(function_id) => {
                link_eval_environments(tree, function_id)?;
            }
            FunctionResolutionEvent::Resolve(function_id) => {
                let unresolved = tree.functions[function_id]
                    .ops
                    .iter()
                    .enumerate()
                    .filter_map(|(index, operation)| match &operation.op {
                        IrOp::Identifier {
                            name,
                            span,
                            scope,
                            access,
                        } => Some((
                            index,
                            name.clone(),
                            *span,
                            *scope,
                            UnresolvedAccess::Identifier(*access),
                        )),
                        IrOp::IdentifierReference {
                            name,
                            span,
                            scope,
                            access,
                        } => Some((
                            index,
                            name.clone(),
                            *span,
                            *scope,
                            UnresolvedAccess::IdentifierReference(*access),
                        )),
                        IrOp::ImportMeta { span, scope } => Some((
                            index,
                            crate::engine::code::module::MODULE_IMPORT_META_BINDING_NAME.to_owned(),
                            *span,
                            *scope,
                            UnresolvedAccess::ImportMeta,
                        )),
                        IrOp::PrivateField {
                            name,
                            span,
                            scope,
                            access,
                        } => Some((
                            index,
                            name.clone(),
                            *span,
                            *scope,
                            UnresolvedAccess::PrivateField(*access),
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>();

                for (operation_index, name, span, scope, access) in unresolved {
                    let operation = match access {
                        UnresolvedAccess::Identifier(access) => {
                            resolve_identifier(tree, function_id, scope, &name, span, access)?
                        }
                        UnresolvedAccess::IdentifierReference(access) => {
                            resolve_identifier_reference(
                                tree,
                                function_id,
                                scope,
                                &name,
                                span,
                                access,
                            )?
                        }
                        UnresolvedAccess::ImportMeta => resolve_import_meta(tree, function_id)?,
                        UnresolvedAccess::PrivateField(access) => {
                            private_reference::resolve_private_field_operation(
                                tree,
                                function_id,
                                scope,
                                &name,
                                access,
                            )?
                        }
                    };
                    tree.functions[function_id].ops[operation_index].op = operation;
                }
                finalize_eval_closure_suffixes(tree, function_id)?;
            }
        }
    }
    install_global_function_hoists(tree)?;
    install_eval_declaration_hoists(tree)?;
    install_function_body_hoists(tree)?;
    // Every prepend pass runs after identifier linking. Install ordinary
    // pseudo activation cells after declaration hoists. A module's dual-entry
    // link guard is then wrapped around that authored-evaluation entry so a
    // link-time call never initializes `this`/direct-eval pseudo cells.
    install_pseudo_binding_prologues(tree)?;
    install_module_declaration_hoists(tree)?;
    validate_scope_graph(tree)
}

/// QuickJS `add_eval_variables` allocates one hidden `<var>` local before
/// resolving any identifier in sloppy authored function code which contains a
/// syntactic direct-eval site. Keeping this as a separate prepass is essential:
/// children authored before the eval call must resolve through the same object.
fn install_eval_variable_objects(tree: &mut FunctionTree) -> Result<(), Error> {
    for function in &mut tree.functions {
        let has_direct_eval = matches!(
            function.kind,
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
        ) && function
            .ops
            .iter()
            .any(|operation| matches!(operation.op, IrOp::EvalCall { .. }));
        let needs_objects = has_direct_eval && !function.strict;
        if !needs_objects {
            if function.eval_variable_object_local.is_some()
                || function.arg_eval_variable_object_local.is_some()
                || function.synthetic_parameter_arguments_local.is_some()
            {
                return Err(Error::internal(
                    "function retained unnecessary parameter-eval state",
                ));
            }
            continue;
        }
        if function.eval_variable_object_local.is_some()
            || function.arg_eval_variable_object_local.is_some()
            || function.synthetic_parameter_arguments_local.is_some()
        {
            return Err(Error::internal(
                "eval variable objects were installed more than once",
            ));
        }
        function.eval_variable_object_local = Some(allocate_hidden_eval_object(
            function,
            EVAL_VARIABLE_OBJECT_LOCAL_NAME,
            BindingKind::EvalVariableObject,
        )?);
        if function.parameter_scope.is_some() {
            function.arg_eval_variable_object_local = Some(allocate_hidden_eval_object(
                function,
                ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME,
                BindingKind::ArgEvalVariableObject,
            )?);
            install_parameter_eval_arguments(function)?;
        }
    }
    Ok(())
}

fn allocate_hidden_eval_object(
    function: &mut FunctionIr,
    name: &'static str,
    kind: BindingKind,
) -> Result<u16, Error> {
    if function.locals.len() >= MAX_LOCAL_VARIABLES {
        return Err(Error::new(
            ErrorKind::JsInternal,
            "too many local variables",
        ));
    }
    let index = u16::try_from(function.locals.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
    function.locals.push(name.to_owned());
    function.add_binding(
        function.var_scope,
        function.var_scope,
        name.to_owned(),
        BindingStorage::Local(index),
        kind,
        None,
    );
    Ok(index)
}

/// QuickJS gives sloppy ordinary functions with parameter direct eval two
/// bindings for one unmapped arguments object: the usual FunctionRoot binding
/// and, when no authored parameter shadows it, a Parameter-scoped lexical
/// alias. A named physical `arguments` parameter does not suppress the body
/// object on this path; a BindingPattern body local is reused and later
/// overwritten by the authenticated parameter copy.
fn install_parameter_eval_arguments(function: &mut FunctionIr) -> Result<(), Error> {
    if !matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method) {
        return Ok(());
    }
    let parameter_scope = function
        .parameter_scope
        .ok_or_else(|| Error::internal("parameter eval arguments has no parameter scope"))?;
    if function.arguments_local.is_none() {
        let reusable = function.scopes[function.var_scope.0]
            .bindings
            .iter()
            .rev()
            .filter_map(|binding| function.bindings.get(binding.0))
            .find_map(|binding| {
                (binding.name == "arguments"
                    && binding.kind == BindingKind::Normal
                    && matches!(binding.storage, BindingStorage::Local(_)))
                .then_some(binding.storage)
            });
        let local = match reusable {
            Some(BindingStorage::Local(local)) => local,
            Some(_) => unreachable!("reusable arguments storage is local"),
            None => {
                if function.locals.len() >= MAX_LOCAL_VARIABLES {
                    return Err(Error::new(
                        ErrorKind::JsInternal,
                        "too many local variables",
                    ));
                }
                let local = u16::try_from(function.locals.len())
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
                function.locals.push("arguments".to_owned());
                function.add_binding(
                    function.var_scope,
                    function.var_scope,
                    "arguments".to_owned(),
                    BindingStorage::Local(local),
                    BindingKind::Normal,
                    None,
                );
                local
            }
        };
        function.arguments_local = Some(local);
    }

    if function.scopes[parameter_scope.0]
        .bindings
        .iter()
        .any(|binding| function.bindings[binding.0].name == "arguments")
    {
        return Ok(());
    }
    if function.locals.len() >= MAX_LOCAL_VARIABLES {
        return Err(Error::new(
            ErrorKind::JsInternal,
            "too many local variables",
        ));
    }
    let local = u16::try_from(function.locals.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
    function.locals.push("arguments".to_owned());
    function.add_binding(
        parameter_scope,
        parameter_scope,
        "arguments".to_owned(),
        BindingStorage::Local(local),
        BindingKind::Lexical { is_const: false },
        None,
    );
    function.synthetic_parameter_arguments_local = Some(local);
    Ok(())
}

const fn eval_scope_kind(kind: ScopeKind) -> EvalScopeKind {
    match kind {
        ScopeKind::FunctionRoot => EvalScopeKind::FunctionRoot,
        ScopeKind::Parameter => EvalScopeKind::Parameter,
        ScopeKind::FunctionBody => EvalScopeKind::FunctionBody,
        ScopeKind::ProgramBody => EvalScopeKind::ProgramBody,
        ScopeKind::Block | ScopeKind::ClassPrivate => EvalScopeKind::Block,
        ScopeKind::If => EvalScopeKind::If,
        ScopeKind::For => EvalScopeKind::For,
        ScopeKind::Switch => EvalScopeKind::Switch,
        ScopeKind::Catch => EvalScopeKind::Catch,
        ScopeKind::With => EvalScopeKind::With,
    }
}

/// Publish the immutable scope chains for one function at its QuickJS
/// creation-entry event. Besides retaining names for later String compilation,
/// this deliberately pre-populates closure slots before children and ordinary
/// identifier resolution so source-order first-slot-wins behavior is stable.
fn link_eval_environments(tree: &mut FunctionTree, function_id: FunctionId) -> Result<(), Error> {
    let has_eval = tree.functions[function_id]
        .ops
        .iter()
        .any(|operation| matches!(operation.op, IrOp::EvalCall { .. }));
    if !has_eval {
        return Ok(());
    }

    // Lazy pseudo-bindings must exist before any descriptor snapshots a scope.
    // The nearest ordinary-function or method frame owns `arguments`;
    // named-expression self bindings from every enclosing function are also
    // visible even when no ordinary identifier opcode forced them first.
    ensure_eval_visible_pseudo_bindings(tree, function_id)?;

    let sites = tree.functions[function_id]
        .ops
        .iter()
        .enumerate()
        .filter_map(|(operation_index, operation)| match operation.op {
            IrOp::EvalCall {
                scope,
                environment: None,
                ..
            } => Some((operation_index, scope)),
            IrOp::EvalCall {
                environment: Some(_),
                ..
            } => None,
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut by_scope = HashMap::<ScopeId, u16>::new();
    let mut linked_sites = Vec::with_capacity(sites.len());
    for (operation_index, scope) in sites {
        let environment = if let Some(&environment) = by_scope.get(&scope) {
            environment
        } else {
            let environment = u16::try_from(tree.functions[function_id].eval_environments.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many eval environments"))?;
            let descriptor = link_eval_environment(tree, function_id, scope)?;
            tree.functions[function_id]
                .eval_environments
                .push(descriptor);
            by_scope.insert(scope, environment);
            environment
        };
        linked_sites.push((operation_index, environment));
    }
    for (operation_index, environment) in linked_sites {
        let Some(operation) = tree.functions[function_id].ops.get_mut(operation_index) else {
            return Err(Error::internal("eval operation moved while linking scopes"));
        };
        let IrOp::EvalCall {
            environment: linked,
            ..
        } = &mut operation.op
        else {
            return Err(Error::internal(
                "eval operation changed while linking scopes",
            ));
        };
        if linked.replace(environment).is_some() {
            return Err(Error::internal("eval operation was linked more than once"));
        }
    }
    Ok(())
}

/// QuickJS builds a caller bytecode's direct-eval closure prefix before
/// resolving ordinary identifiers, then `add_closure_variables` appends the
/// caller's final closure vector when eval source is compiled. Usually the
/// early scope walk already projected every such slot. One observable exception
/// is `arguments`: a closure created in a non-simple parameter initializer does
/// not see the owner's body arguments binding merely because it contains eval,
/// but an authored (or descendant-arrow) `arguments` reference can add that
/// relay later. Append that exact late slot without inventing another function
/// scope segment.
fn finalize_eval_closure_suffixes(
    tree: &mut FunctionTree,
    function_id: FunctionId,
) -> Result<(), Error> {
    if tree.functions[function_id].eval_environments.is_empty() {
        return Ok(());
    }

    let suffix = {
        let function = &tree.functions[function_id];
        function
            .closure_variables
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, descriptor)| {
                (!matches!(
                    descriptor.source,
                    ClosureSource::Global
                        | ClosureSource::GlobalDeclaration
                        | ClosureSource::ParentGlobal(_)
                ))
                .then_some((index, descriptor))
            })
            .map(|(index, descriptor)| {
                let name = match descriptor.name {
                    ClosureVariableName::Constant(name) => {
                        match function.constants.get(name as usize) {
                            Some(IrConstant::Primitive(Value::String(name))) => name.clone(),
                            _ => {
                                return Err(Error::internal(
                                    "eval-visible closure name is not a string constant",
                                ));
                            }
                        }
                    }
                    ClosureVariableName::None | ClosureVariableName::Atom(_) => {
                        return Err(Error::internal(
                            "eval-visible closure slot retained no compiler string name",
                        ));
                    }
                };
                let index = u16::try_from(index)
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
                Ok((index, descriptor, name))
            })
            .collect::<Result<Vec<_>, Error>>()?
    };

    for environment in &mut tree.functions[function_id].eval_environments {
        let used = environment
            .scopes
            .iter()
            .flat_map(|scope| scope.bindings.iter())
            .filter_map(|binding| match binding.source {
                EvalBindingSource::Closure(index) => Some(index),
                EvalBindingSource::Local(_) | EvalBindingSource::Argument(_) => None,
            })
            .collect::<Vec<_>>();
        let mut late = Vec::new();
        for &(index, descriptor, ref name) in &suffix {
            if used.contains(&index) {
                continue;
            }
            if descriptor.kind != ClosureVariableKind::Normal
                || descriptor.is_lexical
                || descriptor.is_const
                || name.to_utf8_lossy() != "arguments"
                || matches!(descriptor.source, ClosureSource::EvalEnvironment(_))
            {
                return Err(Error::internal(
                    "unexpected late direct-eval closure suffix binding",
                ));
            }
            late.push(EvalBinding {
                name: name.clone(),
                source: EvalBindingSource::Closure(index),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
                is_catch_parameter: false,
            });
        }
        if late.is_empty() {
            continue;
        }
        let anchor = environment
            .scopes
            .last_mut()
            .filter(|scope| {
                matches!(
                    scope.kind,
                    EvalScopeKind::FunctionRoot | EvalScopeKind::Parameter
                )
            })
            .ok_or_else(|| Error::internal("eval closure suffix has no outer function anchor"))?;
        let mut bindings = anchor.bindings.to_vec();
        bindings.extend(late);
        anchor.bindings = bindings.into_boxed_slice();
    }
    Ok(())
}

fn link_eval_environment(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    call_scope: ScopeId,
) -> Result<EvalEnvironment<JsString>, Error> {
    let caller_strict = tree.functions[consuming_function].strict;
    let caller_kind = tree.functions[consuming_function].kind;
    let super_call_allowed = tree.functions[consuming_function].super_call_allowed;
    let super_allowed = tree.functions[consuming_function].super_allowed;
    let current_parameter_phase = tree.functions[consuming_function]
        .parameter_scope
        .is_some_and(|parameter| {
            tree.functions[consuming_function].scope_is_within(call_scope, parameter)
        });
    let mut scope_path = Vec::<(FunctionId, ScopeId)>::new();
    let mut owner = consuming_function;
    let mut scope = call_scope;
    loop {
        loop {
            let ir_scope = tree.functions[owner]
                .scopes
                .get(scope.0)
                .ok_or_else(|| Error::internal("eval call scope is out of bounds"))?;
            scope_path.push((owner, scope));
            let Some(parent) = ir_scope.parent else {
                break;
            };
            scope = parent;
        }
        let Some(parent) = tree.functions[owner].parent else {
            break;
        };
        owner = parent.function;
        scope = parent.definition_scope;
    }

    let mut scopes = Vec::with_capacity(scope_path.len());
    let mut current_function_segment = None;
    for (owner, scope) in scope_path {
        let (kind, binding_snapshots, pattern_parameter_initialization) = {
            let function = &tree.functions[owner];
            let ir_scope = function
                .scopes
                .get(scope.0)
                .ok_or_else(|| Error::internal("eval scope path is out of bounds"))?;
            let mut ordered = ir_scope.bindings.iter().rev().copied().collect::<Vec<_>>();
            if ir_scope.kind == ScopeKind::FunctionRoot
                && function.eval_variable_object_local.is_some()
            {
                // Existing authored root bindings precede `<var>`, which in
                // turn precedes `<arg_var>`. Lazy pseudo-bindings are fallback
                // bindings after both dynamic objects.
                ordered.sort_by_key(|binding| {
                    let binding = &function.bindings[binding.0];
                    match binding.kind {
                        BindingKind::EvalVariableObject => 1_u8,
                        BindingKind::ArgEvalVariableObject => 2,
                        BindingKind::FunctionName { .. } => 3,
                        BindingKind::Normal
                            if matches!(
                                binding.storage,
                                BindingStorage::Local(index)
                                    if function.arguments_local == Some(index)
                                        && function
                                            .eval_variable_object_local
                                            .is_some_and(|object| index > object)
                            ) =>
                        {
                            3
                        }
                        BindingKind::Normal
                        | BindingKind::Lexical { .. }
                        | BindingKind::WithObject
                        | BindingKind::PrivateField { .. }
                        | BindingKind::PrivateMethod { .. }
                        | BindingKind::PrivateGetter { .. }
                        | BindingKind::PrivateSetter { .. }
                        | BindingKind::PrivateGetterSetter { .. } => 0,
                    }
                });
            }
            let mut bindings = ordered
                .into_iter()
                .map(|binding| {
                    function
                        .bindings
                        .get(binding.0)
                        .map(|binding| {
                            (
                                binding.name.clone(),
                                binding.storage,
                                binding.kind,
                                binding.is_catch_parameter,
                            )
                        })
                        .ok_or_else(|| Error::internal("eval binding is out of bounds"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if ir_scope.kind == ScopeKind::Parameter {
                // The Parameter scope is deliberately parentless, but QuickJS
                // projects a small set of activation-root pseudo bindings into
                // eval's argument-scope closure: `<arg_var>`, HomeObject,
                // new.target, this, and the private function-expression name.
                // Raw arguments, body declarations, the body arguments object,
                // and `<var>` remain excluded.
                let mut pseudo = function.scopes[function.var_scope.0]
                    .bindings
                    .iter()
                    .rev()
                    .filter_map(|binding| function.bindings.get(binding.0))
                    .filter(|binding| {
                        binding.kind == BindingKind::ArgEvalVariableObject
                            || matches!(binding.kind, BindingKind::FunctionName { .. })
                            || matches!(binding.storage, BindingStorage::Local(index)
                                if function.home_object_local == Some(index)
                                    || function.active_function_local == Some(index)
                                    || function.new_target_local == Some(index)
                                    || function.this_local == Some(index))
                    })
                    .map(|binding| {
                        (
                            binding.name.clone(),
                            binding.storage,
                            binding.kind,
                            binding.is_catch_parameter,
                        )
                    })
                    .collect::<Vec<_>>();
                pseudo.sort_by_key(|(_, _, kind, _)| {
                    u8::from(*kind != BindingKind::ArgEvalVariableObject)
                });
                bindings.extend(pseudo);
            }
            (
                ir_scope.kind,
                bindings,
                function.pattern_parameter_initialization,
            )
        };

        // A no-`=` BindingPattern executes directly in FunctionRoot before
        // the authored FunctionBody scope is entered. Eval descriptors retain
        // a stable Body -> Root segment for every function in the lexical
        // chain, including an arrow or nested closure created by a computed
        // key. Represent each such not-yet-visible body as one empty scope.
        if kind == ScopeKind::FunctionRoot
            && pattern_parameter_initialization
            && scopes
                .last()
                .is_none_or(|scope: &EvalScope<JsString>| scope.kind != EvalScopeKind::FunctionBody)
        {
            scopes.push(EvalScope {
                kind: EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            });
        }

        let scope_ordinal = u16::try_from(scopes.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many eval scopes"))?;
        if owner == consuming_function
            && matches!(kind, ScopeKind::FunctionRoot | ScopeKind::Parameter)
        {
            current_function_segment = Some(scope_ordinal);
        }

        let mut bindings = Vec::with_capacity(binding_snapshots.len());
        for (name, storage, binding_kind, is_catch_parameter) in binding_snapshots {
            // The synthetic eval root installs imported bindings in its own
            // parser root only for identifier resolution.  Their semantic
            // scope provenance is reconstructed from `eval_caller_profile`
            // below, so serializing them here would falsely turn caller catch
            // or block bindings into FunctionRoot bindings.
            if matches!(storage, BindingStorage::External(_)) {
                continue;
            }
            let module_import_view = binding_storage_is_module_import_view(tree, owner, storage)?;
            let (source, resolved_kind) = if owner == consuming_function {
                match storage {
                    BindingStorage::Argument(index) => {
                        (EvalBindingSource::Argument(index), binding_kind)
                    }
                    BindingStorage::Local(index) => (EvalBindingSource::Local(index), binding_kind),
                    BindingStorage::External(_) => unreachable!("filtered above"),
                    BindingStorage::Module(binding) => (
                        EvalBindingSource::Closure(module_binding_closure_index(tree, binding)?),
                        binding_kind,
                    ),
                    BindingStorage::Global => continue,
                }
            } else {
                if storage == BindingStorage::Global {
                    continue;
                }
                let (index, resolved_kind) = capture_binding_path(
                    tree,
                    owner,
                    consuming_function,
                    ResolvedBinding {
                        storage,
                        kind: binding_kind,
                    },
                    &name,
                    true,
                    true,
                )?;
                if !binding_kinds_compatible(resolved_kind, binding_kind)
                    && !matches!(
                        (binding_kind, resolved_kind),
                        (BindingKind::FunctionName { .. }, BindingKind::Normal)
                    )
                {
                    return Err(Error::internal(
                        "eval closure relay changed binding metadata",
                    ));
                }
                (EvalBindingSource::Closure(index), resolved_kind)
            };
            bindings.push(EvalBinding {
                name: JsString::try_from_utf8(&name)?,
                source,
                is_lexical: matches!(
                    resolved_kind,
                    BindingKind::Lexical { .. }
                        | BindingKind::PrivateField { .. }
                        | BindingKind::PrivateMethod { .. }
                        | BindingKind::PrivateGetter { .. }
                        | BindingKind::PrivateSetter { .. }
                        | BindingKind::PrivateGetterSetter { .. }
                ),
                is_const: matches!(
                    resolved_kind,
                    BindingKind::Lexical { is_const: true }
                        | BindingKind::FunctionName { is_const: true }
                        | BindingKind::PrivateField { .. }
                        | BindingKind::PrivateMethod { .. }
                        | BindingKind::PrivateGetter { .. }
                        | BindingKind::PrivateSetter { .. }
                        | BindingKind::PrivateGetterSetter { .. }
                ),
                kind: if module_import_view {
                    ClosureVariableKind::ModuleImportView
                } else {
                    closure_kind(resolved_kind)
                },
                is_catch_parameter,
            });
        }
        scopes.push(EvalScope {
            kind: eval_scope_kind(kind),
            bindings: bindings.into_boxed_slice(),
        });
    }

    // A direct eval root is a real frame, but its imported caller bindings do
    // not become declarations in that synthetic FunctionRoot.  Append the
    // exact original scope suffix (including empty scopes) and relay each
    // flattened external slot through any intervening eval-created function.
    let imported_profile = tree.functions[0].eval_caller_profile.clone();
    let imported_bindings = tree.functions[0].external_bindings.clone();
    let imported_target = match imported_profile.variable_target {
        EvalCallerVariableTarget::ExternalBinding(index) => Some(index),
        EvalCallerVariableTarget::Global | EvalCallerVariableTarget::StrictLocal => None,
    };
    let mut imported_variable_target = None;
    for (scope_index, kind) in imported_profile.scope_kinds.iter().copied().enumerate() {
        let scope_index = u16::try_from(scope_index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many eval scopes"))?;
        let snapshots = imported_bindings
            .iter()
            .enumerate()
            .filter(|(_, binding)| binding.scope == scope_index)
            .map(|(index, binding)| (index, binding.clone()))
            .collect::<Vec<_>>();
        let emitted_scope = u16::try_from(scopes.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many eval scopes"))?;
        let mut bindings = Vec::with_capacity(snapshots.len());
        for (external_index, binding) in snapshots {
            let external_index = u16::try_from(external_index)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
            let binding_kind =
                binding_kind_from_closure_flags(binding.kind, binding.is_lexical, binding.is_const)
                    .ok_or_else(|| {
                        Error::internal("imported eval binding flags are inconsistent")
                    })?;
            let source = if consuming_function == 0 {
                EvalBindingSource::Closure(external_index)
            } else {
                let name = binding.name.to_utf8_lossy();
                let (closure, relayed_kind) = capture_binding_path(
                    tree,
                    0,
                    consuming_function,
                    ResolvedBinding {
                        storage: BindingStorage::External(external_index),
                        kind: binding_kind,
                    },
                    &name,
                    true,
                    false,
                )?;
                if !binding_kinds_compatible(relayed_kind, binding_kind) {
                    return Err(Error::internal(
                        "imported eval binding relay changed metadata",
                    ));
                }
                EvalBindingSource::Closure(closure)
            };
            if imported_target == Some(external_index)
                && imported_variable_target
                    .replace((emitted_scope, source))
                    .is_some()
            {
                return Err(Error::internal(
                    "imported eval variable target was projected more than once",
                ));
            }
            bindings.push(EvalBinding {
                name: binding.name,
                source,
                is_lexical: binding.is_lexical,
                is_const: binding.is_const,
                kind: binding.kind,
                is_catch_parameter: binding.is_catch_parameter,
            });
        }
        scopes.push(EvalScope {
            kind,
            bindings: bindings.into_boxed_slice(),
        });
    }

    let current_function_segment = current_function_segment
        .ok_or_else(|| Error::internal("eval environment has no current function segment"))?;
    let variable_environment = match caller_kind {
        FunctionKind::Script => EvalVariableEnvironment::Global,
        FunctionKind::Module => EvalVariableEnvironment::StrictLocal(current_function_segment),
        FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow => {
            if caller_strict {
                EvalVariableEnvironment::StrictLocal(current_function_segment)
            } else {
                let local = if current_parameter_phase {
                    tree.functions[consuming_function]
                        .arg_eval_variable_object_local
                        .ok_or_else(|| {
                            Error::internal("parameter eval environment has no `<arg_var>` local")
                        })?
                } else {
                    tree.functions[consuming_function]
                        .eval_variable_object_local
                        .ok_or_else(|| {
                            Error::internal("body eval environment has no `<var>` local")
                        })?
                };
                EvalVariableEnvironment::VariableObject {
                    scope: current_function_segment,
                    source: EvalBindingSource::Local(local),
                }
            }
        }
        FunctionKind::Eval(_) if caller_strict => {
            EvalVariableEnvironment::StrictLocal(current_function_segment)
        }
        FunctionKind::Eval(EvalKind::Direct) => {
            match tree.functions[consuming_function]
                .eval_caller_profile
                .variable_target
            {
                EvalCallerVariableTarget::Global => EvalVariableEnvironment::Global,
                EvalCallerVariableTarget::ExternalBinding(_) => imported_variable_target
                    .map(|(scope, source)| EvalVariableEnvironment::VariableObject {
                        scope,
                        source,
                    })
                    .ok_or_else(|| {
                        Error::internal("sloppy eval root lost its imported variable target")
                    })?,
                EvalCallerVariableTarget::StrictLocal => {
                    return Err(Error::internal(
                        "sloppy eval root retained a strict-local variable target",
                    ));
                }
            }
        }
        FunctionKind::Eval(EvalKind::Indirect) => {
            if tree.functions[consuming_function]
                .eval_caller_profile
                .variable_target
                != EvalCallerVariableTarget::Global
            {
                return Err(Error::internal(
                    "indirect eval root retained a caller variable target",
                ));
            }
            EvalVariableEnvironment::Global
        }
        FunctionKind::Eval(EvalKind::None) => {
            return Err(Error::internal("eval root has no eval kind"));
        }
    };
    Ok(EvalEnvironment {
        scopes: scopes.into_boxed_slice(),
        variable_environment,
        caller_strict,
        super_call_allowed,
        super_allowed,
    })
}

/// QuickJS adds every Program declaration to the root GLOBAL_DECL list in
/// source order before child-first identifier resolution. Pre-seeding prevents
/// a child capture of a later name from changing declaration-check priority.
fn seed_global_declarations(tree: &mut FunctionTree) -> Result<(), Error> {
    let declarations = tree
        .functions
        .first()
        .ok_or_else(|| Error::internal("compiler produced no root function"))?
        .global_declarations
        .iter()
        .map(|declaration| {
            (
                declaration.name.clone(),
                declaration.is_lexical,
                declaration.is_const,
                declaration.function_constant.is_some(),
            )
        })
        .collect::<Vec<_>>();

    for (declaration_index, (name, is_lexical, is_const, is_function)) in
        declarations.into_iter().enumerate()
    {
        let name_index = ensure_string_constant(&mut tree.functions[0], &name)?;
        let closure_index = push_closure_variable(
            &mut tree.functions[0],
            ClosureVariable {
                source: ClosureSource::GlobalDeclaration,
                name: ClosureVariableName::Constant(name_index),
                is_lexical,
                is_const,
                kind: if is_function {
                    ClosureVariableKind::GlobalFunction
                } else {
                    ClosureVariableKind::Normal
                },
            },
        )?;
        tree.functions[0].global_declarations[declaration_index].closure_index =
            Some(closure_index);
    }
    Ok(())
}

/// Convert the parser's typed module bindings into root VarRef descriptors.
fn seed_module_bindings(tree: &mut FunctionTree) -> Result<(), Error> {
    let Some(module) = tree.module.as_ref() else {
        return Ok(());
    };
    let bindings = module
        .bindings
        .iter()
        .map(|binding| {
            (
                binding.name.clone(),
                binding.declaration,
                binding.import,
                binding.is_import_meta,
            )
        })
        .collect::<Vec<_>>();
    for (binding_index, (name, declaration, import, is_import_meta)) in
        bindings.into_iter().enumerate()
    {
        let name_index = ensure_string_constant(&mut tree.functions[0], &name)?;
        let (source, is_lexical, is_const, kind) = if is_import_meta {
            if declaration.is_some() || import.is_some() {
                return Err(Error::internal(
                    "import.meta binding acquired a declaration or import",
                ));
            }
            (
                ClosureSource::ModuleImportMeta,
                true,
                true,
                ClosureVariableKind::Normal,
            )
        } else {
            match import {
                Some(module::ModuleImportKind::Named) => (
                    if declaration.is_some() {
                        ClosureSource::ModuleImportCollision
                    } else {
                        ClosureSource::ModuleImport
                    },
                    true,
                    true,
                    ClosureVariableKind::ModuleImportView,
                ),
                Some(module::ModuleImportKind::Namespace) => (
                    if declaration.is_some() {
                        ClosureSource::ModuleImportCollision
                    } else {
                        ClosureSource::ModuleDeclaration
                    },
                    true,
                    true,
                    ClosureVariableKind::Normal,
                ),
                None => match declaration.ok_or_else(|| {
                    Error::internal("module binding has neither an import nor a declaration")
                })? {
                    module::ModuleDeclarationOrigin::Var
                    | module::ModuleDeclarationOrigin::Function { .. } => (
                        ClosureSource::ModuleDeclaration,
                        false,
                        false,
                        ClosureVariableKind::Normal,
                    ),
                    module::ModuleDeclarationOrigin::Lexical { is_const } => (
                        ClosureSource::ModuleDeclaration,
                        true,
                        is_const,
                        ClosureVariableKind::Normal,
                    ),
                },
            }
        };
        let closure_index = push_closure_variable(
            &mut tree.functions[0],
            ClosureVariable {
                source,
                name: ClosureVariableName::Constant(name_index),
                is_lexical,
                is_const,
                kind,
            },
        )?;
        tree.module
            .as_mut()
            .and_then(|module| module.bindings.get_mut(binding_index))
            .ok_or_else(|| Error::internal("module binding moved while seeding"))?
            .closure_index = Some(closure_index);
    }
    Ok(())
}

/// QuickJS emits every Program function initializer before authored body
/// bytecode. Each declaration retains its own child constant, while the raw
/// write resolves by name to the first same-name `GLOBAL_DECL` slot.
fn install_global_function_hoists(tree: &mut FunctionTree) -> Result<(), Error> {
    let declarations = tree
        .functions
        .first()
        .ok_or_else(|| Error::internal("compiler produced no root function"))?
        .global_declarations
        .iter()
        .filter_map(|declaration| {
            declaration
                .function_constant
                .map(|constant| (declaration.name.clone(), constant))
        })
        .collect::<Vec<_>>();
    if declarations.is_empty() {
        return Ok(());
    }

    let mut prefix = Vec::with_capacity(declarations.len().saturating_mul(2));
    for (name, constant) in declarations {
        let target = tree.functions[0]
            .global_declarations
            .iter()
            .find(|declaration| declaration.name == name)
            .and_then(|declaration| declaration.closure_index)
            .ok_or_else(|| Error::internal("global function hoist target was not seeded"))?;
        prefix.push(SpannedIrOp {
            op: IrOp::MakeClosure(constant),
            pc_site: None,
        });
        // PutVarInit is the declaration-time raw VarRef write. It is not an
        // ordinary assignment and intentionally bypasses TDZ, const, and
        // global-property setter fallback.
        prefix.push(SpannedIrOp {
            op: IrOp::Bytecode(Instruction::PutVarInit(target)),
            pc_site: None,
        });
    }
    prepend_hoist_prefix(&mut tree.functions[0], prefix)
}

/// QuickJS's module function has two entry modes. Link-time calls it with a
/// truthy `this` to initialize var/function cells and return immediately;
/// evaluation calls it with `undefined` and jumps directly to authored code.
fn install_module_declaration_hoists(tree: &mut FunctionTree) -> Result<(), Error> {
    let Some(module) = tree.module.as_ref() else {
        return Ok(());
    };
    let mut initializers = Vec::new();
    for binding_id in &module.declaration_order {
        let binding = module.binding(*binding_id)?;
        let declaration = binding.declaration.ok_or_else(|| {
            Error::internal("module declaration order referenced an import-only binding")
        })?;
        let value = match declaration {
            module::ModuleDeclarationOrigin::Var if binding.import.is_none() => Some(None),
            module::ModuleDeclarationOrigin::Function {
                constant,
                inferred_name,
            } => Some(Some((constant, inferred_name))),
            module::ModuleDeclarationOrigin::Var
            | module::ModuleDeclarationOrigin::Lexical { .. } => None,
        };
        if let Some(value) = value {
            initializers.push((binding.closure_index, value, binding.import.is_some()));
        }
    }
    let mut prefix = vec![
        SpannedIrOp {
            op: IrOp::Bytecode(Instruction::PushThis),
            pc_site: None,
        },
        SpannedIrOp {
            op: IrOp::Bytecode(Instruction::IfFalse(u32::MAX)),
            pc_site: None,
        },
    ];
    for (closure_index, initializer, import_collision) in initializers {
        let closure_index = closure_index
            .ok_or_else(|| Error::internal("module initializer closure was not seeded"))?;
        match initializer {
            None => prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::Undefined),
                pc_site: None,
            }),
            Some((constant, inferred_name)) => {
                prefix.push(SpannedIrOp {
                    op: IrOp::MakeClosure(constant),
                    pc_site: None,
                });
                if let Some(name) = inferred_name {
                    prefix.push(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::SetName(name)),
                        pc_site: None,
                    });
                }
            }
        }
        prefix.push(SpannedIrOp {
            op: IrOp::Bytecode(if import_collision {
                Instruction::InitializeModuleImportCollision(closure_index)
            } else {
                Instruction::PutVarRef(closure_index)
            }),
            pc_site: None,
        });
    }
    prefix.push(SpannedIrOp {
        op: IrOp::Bytecode(Instruction::Undefined),
        pc_site: None,
    });
    prefix.push(SpannedIrOp {
        op: IrOp::Bytecode(Instruction::Return),
        pc_site: None,
    });
    let body = u32::try_from(prefix.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    let Some(SpannedIrOp {
        op: IrOp::Bytecode(Instruction::IfFalse(target)),
        ..
    }) = prefix.get_mut(1)
    else {
        return Err(Error::internal("module link entry guard is malformed"));
    };
    *target = body;
    prepend_hoist_prefix(&mut tree.functions[0], prefix)
}

/// Install the source-ordered declaration prelude used by sloppy direct eval
/// in an ordinary caller. Unlike ordinary function hoists, every novel `var`
/// record is retained and writes `undefined`; this preserves QuickJS's
/// observable overwrite behavior across repeated eval invocations.
fn install_eval_declaration_hoists(tree: &mut FunctionTree) -> Result<(), Error> {
    let Some(function) = tree.functions.first_mut() else {
        return Err(Error::internal("compiler produced no root function"));
    };
    if function.eval_declarations_installed {
        return Err(Error::internal(
            "eval declaration hoists were installed more than once",
        ));
    }
    if function.eval_declarations.is_empty() && function.eval_redeclaration.is_none() {
        function.eval_declarations_installed = true;
        return Ok(());
    }
    if !matches!(function.kind, FunctionKind::Eval(EvalKind::Direct)) || function.strict {
        return Err(Error::internal(
            "dynamic eval declarations escaped sloppy direct eval",
        ));
    }

    let declarations = function.eval_declarations.clone();
    let mut prefix = Vec::with_capacity(
        declarations
            .len()
            .saturating_mul(2)
            .saturating_add(usize::from(function.eval_redeclaration.is_some())),
    );
    if let Some(name) = function.eval_redeclaration.clone() {
        let name = ensure_string_constant(function, &name)?;
        prefix.push(SpannedIrOp {
            op: IrOp::Bytecode(Instruction::ThrowRedeclaration(name)),
            pc_site: None,
        });
    }
    for declaration in declarations {
        match declaration.value {
            EvalDeclarationValue::Undefined => {
                if matches!(declaration.target, EvalDeclarationTarget::Dynamic(_)) {
                    prefix.push(SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::Undefined),
                        pc_site: None,
                    });
                }
            }
            EvalDeclarationValue::Function(constant) => prefix.push(SpannedIrOp {
                op: IrOp::MakeClosure(constant),
                pc_site: None,
            }),
        }

        let write = match declaration.target {
            EvalDeclarationTarget::Dynamic(source) => {
                let name = ensure_string_constant(function, &declaration.name)?;
                Some(IrOp::Bytecode(Instruction::DefineEvalVariable {
                    source,
                    name,
                }))
            }
            EvalDeclarationTarget::External { index, kind } => match declaration.value {
                EvalDeclarationValue::Undefined => None,
                EvalDeclarationValue::Function(_) => Some(closure_binding_operation(
                    function,
                    index,
                    kind,
                    IdentifierAccess::Put,
                    &declaration.name,
                )?),
            },
        };
        if let Some(write) = write {
            prefix.push(SpannedIrOp {
                op: write,
                pc_site: None,
            });
        }
    }
    prepend_hoist_prefix(function, prefix)?;
    function.eval_declarations_installed = true;
    Ok(())
}

/// QuickJS initializes the lazily selected arguments binding before storing
/// direct body function declarations into argument/root-local slots.
fn install_function_body_hoists(tree: &mut FunctionTree) -> Result<(), Error> {
    for function_id in 0..tree.functions.len() {
        if matches!(
            tree.functions[function_id].kind,
            FunctionKind::Script | FunctionKind::Module
        ) {
            continue;
        }
        let hoists = ordered_hoisted_functions(&tree.functions[function_id])?;
        let arguments_local = tree.functions[function_id].arguments_local;
        let eval_variable_object_local = tree.functions[function_id].eval_variable_object_local;
        let arg_eval_variable_object_local =
            tree.functions[function_id].arg_eval_variable_object_local;
        let synthetic_arguments_local =
            tree.functions[function_id].synthetic_parameter_arguments_local;
        let has_parameter_environment = tree.functions[function_id].parameter_scope.is_some();
        let has_pattern_parameters = tree.functions[function_id].pattern_parameter_initialization;
        let class_constructor = tree.functions[function_id].class_constructor;
        let derived_class_constructor = tree.functions[function_id].derived_class_constructor;
        let simple_rest = tree.functions[function_id]
            .rest_parameter
            .filter(|_| !has_parameter_environment && !has_pattern_parameters);
        let guarded_rest = simple_rest.filter(|_| class_constructor);
        let mut prefix = Vec::with_capacity(
            usize::from(arguments_local.is_some()) * 2
                + usize::from(synthetic_arguments_local.is_some()) * 2
                + usize::from(eval_variable_object_local.is_some()) * 2
                + usize::from(arg_eval_variable_object_local.is_some()) * 2
                + usize::from(
                    tree.functions[function_id].rest_parameter.is_some()
                        && !has_parameter_environment
                        && !has_pattern_parameters,
                ) * 2,
        );
        if let Some(local) = arguments_local {
            let kind = if tree.functions[function_id].strict
                || !tree.functions[function_id].has_simple_parameter_list
            {
                ArgumentsKind::Unmapped
            } else {
                ArgumentsKind::Mapped
            };
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::Arguments(kind)),
                pc_site: None,
            });
            if let Some(synthetic) = synthetic_arguments_local {
                prefix.push(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::Dup),
                    pc_site: None,
                });
                prefix.push(SpannedIrOp {
                    op: IrOp::Bytecode(Instruction::InitializeLocal(synthetic)),
                    pc_site: None,
                });
            }
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PutLocal(local)),
                pc_site: None,
            });
        }
        for local in [eval_variable_object_local, arg_eval_variable_object_local]
            .into_iter()
            .flatten()
        {
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::VariableEnvironment),
                pc_site: None,
            });
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PutLocal(local)),
                pc_site: None,
            });
        }
        if let Some(rest) = simple_rest.filter(|_| !class_constructor) {
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::Rest(rest)),
                pc_site: None,
            });
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PutArg(rest)),
                pc_site: None,
            });
        }
        let mut body_hoists = Vec::with_capacity(hoists.len().saturating_mul(2));
        for hoist in hoists {
            let binding = &tree.functions[function_id].bindings[hoist.binding.0];
            body_hoists.push(SpannedIrOp {
                op: IrOp::MakeClosure(hoist.constant),
                pc_site: None,
            });
            let instruction = match (binding.storage, binding.kind) {
                (BindingStorage::Argument(index), _) => Instruction::PutArg(index),
                (
                    BindingStorage::Local(_),
                    BindingKind::PrivateField { .. }
                    | BindingKind::PrivateMethod { .. }
                    | BindingKind::PrivateGetter { .. }
                    | BindingKind::PrivateSetter { .. }
                    | BindingKind::PrivateGetterSetter { .. },
                ) => {
                    return Err(Error::internal(
                        "private binding reached function hoist lowering",
                    ));
                }
                (BindingStorage::Local(index), BindingKind::Lexical { .. }) => {
                    Instruction::PutLocalCheck(index)
                }
                (BindingStorage::Local(index), _) => Instruction::PutLocal(index),
                (BindingStorage::External(_) | BindingStorage::Global, _) => {
                    return Err(Error::internal(
                        "ordinary function hoist targeted global storage",
                    ));
                }
                (BindingStorage::Module(_), _) => {
                    return Err(Error::internal(
                        "ordinary function hoist targeted module storage",
                    ));
                }
            };
            body_hoists.push(SpannedIrOp {
                op: IrOp::Bytecode(instruction),
                pc_site: None,
            });
        }
        let body_entry = tree.functions[function_id]
            .ops
            .iter()
            .position(|operation| {
                matches!(
                    operation.op,
                    IrOp::EnterScope(scope) if scope == tree.functions[function_id].body_scope
                )
            })
            .ok_or_else(|| Error::internal("function body has no scope entry"))?;
        insert_hoist_fragment(
            &mut tree.functions[function_id],
            body_entry + 1,
            body_hoists,
        )?;
        prepend_hoist_prefix(&mut tree.functions[function_id], prefix)?;
        if let Some(rest) = guarded_rest {
            let guard = tree.functions[function_id]
                .ops
                .iter()
                .position(|operation| {
                    matches!(operation.op, IrOp::Bytecode(Instruction::CheckCtor))
                })
                .ok_or_else(|| Error::internal("class rest constructor lost its call guard"))?;
            let after_guard = if derived_class_constructor {
                guard + 1
            } else {
                guard + 5
            };
            insert_hoist_fragment(
                &mut tree.functions[function_id],
                after_guard,
                vec![
                    SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::Rest(rest)),
                        pc_site: None,
                    },
                    SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::PutArg(rest)),
                        pc_site: None,
                    },
                ],
            )?;
        }
        tree.functions[function_id].function_hoists_installed = true;
    }
    Ok(())
}

pub(super) fn ordered_hoisted_functions(
    function: &FunctionIr,
) -> Result<Vec<IrHoistedFunction>, Error> {
    let mut hoists = function.hoisted_functions.clone();
    for hoist in &hoists {
        let binding = function
            .bindings
            .get(hoist.binding.0)
            .ok_or_else(|| Error::internal("hoisted function binding is out of bounds"))?;
        if matches!(
            binding.storage,
            BindingStorage::External(_) | BindingStorage::Module(_) | BindingStorage::Global
        ) {
            return Err(Error::internal(
                "ordinary function hoist targeted global storage",
            ));
        }
    }
    hoists.sort_by_key(|hoist| match function.bindings[hoist.binding.0].storage {
        BindingStorage::Argument(index) => (0_u8, index),
        BindingStorage::Local(index) => (1_u8, index),
        BindingStorage::External(_) => unreachable!("validated above"),
        BindingStorage::Module(_) => unreachable!("validated above"),
        BindingStorage::Global => unreachable!("validated above"),
    });
    Ok(hoists)
}

pub(super) fn insert_hoist_fragment(
    function: &mut FunctionIr,
    at: usize,
    fragment: Vec<SpannedIrOp>,
) -> Result<(), Error> {
    if fragment.is_empty() {
        return Ok(());
    }
    if at > function.ops.len() {
        return Err(Error::internal("function hoist insertion is out of bounds"));
    }
    let shift = u32::try_from(fragment.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    for scoped in &mut function.scoped_functions {
        if scoped.authored_closure >= at {
            scoped.authored_closure = scoped
                .authored_closure
                .checked_add(fragment.len())
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        }
    }
    for annex in &mut function.program_annex_functions {
        if annex.authored_closure >= at {
            annex.authored_closure = annex
                .authored_closure
                .checked_add(fragment.len())
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        }
    }
    for operation in &mut function.ops {
        let target = match &mut operation.op {
            IrOp::Bytecode(
                Instruction::Goto(target)
                | Instruction::IfFalse(target)
                | Instruction::IfTrue(target)
                | Instruction::Catch(target)
                | Instruction::Gosub(target),
            ) => target,
            _ => continue,
        };
        // Forward edges use u32::MAX until their enclosing control construct
        // is complete. NamedEvaluation can insert a zero-effect SetName while
        // such an edge is still open; leave the sentinel for patch_jump.
        if *target != u32::MAX && usize::try_from(*target).is_ok_and(|target| target >= at) {
            *target = target
                .checked_add(shift)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        }
    }
    function.ops.splice(at..at, fragment);
    Ok(())
}

pub(super) fn prepend_hoist_prefix(
    function: &mut FunctionIr,
    mut prefix: Vec<SpannedIrOp>,
) -> Result<(), Error> {
    if prefix.is_empty() {
        return Ok(());
    }
    let shift = u32::try_from(prefix.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    for scoped in &mut function.scoped_functions {
        scoped.authored_closure = scoped
            .authored_closure
            .checked_add(prefix.len())
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    for annex in &mut function.program_annex_functions {
        annex.authored_closure = annex
            .authored_closure
            .checked_add(prefix.len())
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    for operation in &mut function.ops {
        let target = match &mut operation.op {
            IrOp::Bytecode(
                Instruction::Goto(target)
                | Instruction::IfFalse(target)
                | Instruction::IfTrue(target)
                | Instruction::Catch(target)
                | Instruction::Gosub(target),
            ) => target,
            _ => continue,
        };
        *target = target
            .checked_add(shift)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    prefix.append(&mut function.ops);
    function.ops = prefix;
    Ok(())
}

pub(super) fn apply_quickjs_late_throw_sites(
    code: &[Instruction],
    pc_sites: &mut [Option<SourceOffset>],
) -> Result<(), Error> {
    if code.len() != pc_sites.len() {
        return Err(Error::internal(
            "lowered instructions and source markers have different lengths",
        ));
    }
    // Maintenance invariant: every new label-bearing instruction or
    // resolve-labels peephole must update this projection and add a pinned
    // fault-stack oracle before that control-flow slice is enabled.
    let label_target = |instruction: &Instruction| -> Result<Option<usize>, Error> {
        let (Instruction::Goto(target)
        | Instruction::IfFalse(target)
        | Instruction::IfTrue(target)
        | Instruction::Catch(target)
        | Instruction::Gosub(target)) = instruction
        else {
            return Ok(None);
        };
        usize::try_from(*target)
            .map(Some)
            .map_err(|_| Error::internal("jump target did not fit usize"))
    };
    let branch_target = |instruction: &Instruction| -> Result<Option<usize>, Error> {
        let (Instruction::Goto(target)
        | Instruction::IfFalse(target)
        | Instruction::IfTrue(target)) = instruction
        else {
            return Ok(None);
        };
        usize::try_from(*target)
            .map(Some)
            .map_err(|_| Error::internal("jump target did not fit usize"))
    };

    // `resolve_scope_var` introduces terminal OP_throw_error only after
    // parsing. QuickJS then performs two relevant linear rewrites.
    // `resolve_variables` first drops source after parser-authored terminals,
    // updating label reference counts for jumps in that dead range.
    // `resolve_labels` recognizes every newly introduced throw as terminal and
    // repeats the walk with one shared, cumulatively updated reference table.
    // Project both passes once for the whole function: per-throw simulation is
    // not equivalent when an earlier throw removes a forward branch reference.
    let mut label_references = vec![0_usize; code.len()];
    let mut has_physical_label = vec![false; code.len()];
    for instruction in code {
        let Some(target) = label_target(instruction)? else {
            continue;
        };
        let references = label_references
            .get_mut(target)
            .ok_or_else(|| Error::internal("jump target is out of bounds"))?;
        *references = references
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "out of memory"))?;
        has_physical_label[target] = true;
    }

    let mut survives_first_pass = vec![false; code.len()];
    let mut marker_before_label = vec![None; code.len()];
    let mut index = 0_usize;
    while index < code.len() {
        survives_first_pass[index] = true;
        let parser_terminal = matches!(
            code[index],
            Instruction::Goto(_)
                | Instruction::Return
                | Instruction::ReturnUndefined
                | Instruction::Throw
                | Instruction::Ret
        );
        if !parser_terminal {
            index += 1;
            continue;
        }

        let mut dead_index = index + 1;
        let mut final_dead_marker = None;
        while dead_index < code.len() {
            // An upstream OP_label precedes the marker attached to our direct
            // target instruction. A still-referenced label ends this dead
            // range before that authored marker is observed.
            if label_references[dead_index] > 0 {
                break;
            }
            if pc_sites[dead_index].is_some() {
                final_dead_marker = pc_sites[dead_index];
            }
            if let Some(target) = label_target(&code[dead_index])? {
                label_references[target] = label_references[target]
                    .checked_sub(1)
                    .ok_or_else(|| Error::internal("jump label reference count underflow"))?;
            }
            dead_index += 1;
        }
        if dead_index == code.len() {
            break;
        }
        marker_before_label[dead_index] = final_dead_marker;
        index = dead_index;
    }

    let follow_jump_target =
        |initial_target: usize, references: &mut [usize]| -> Result<usize, Error> {
            let initial = initial_target;
            let initial_references = references
                .get_mut(initial)
                .ok_or_else(|| Error::internal("jump target is out of bounds"))?;
            *initial_references = initial_references
                .checked_sub(1)
                .ok_or_else(|| Error::internal("jump label reference count underflow"))?;

            let mut target = initial;
            let mut followed_ten_gotos = true;
            for _ in 0..10 {
                if !survives_first_pass
                    .get(target)
                    .copied()
                    .ok_or_else(|| Error::internal("jump target is out of bounds"))?
                {
                    return Err(Error::internal(
                        "jump target did not survive variable resolution",
                    ));
                }
                let Some(next_target) = branch_target(&code[target])? else {
                    followed_ten_gotos = false;
                    break;
                };
                if !matches!(code[target], Instruction::Goto(_)) {
                    followed_ten_gotos = false;
                    break;
                }
                target = next_target;
            }
            // Preserve QuickJS's cycle workaround after ten chained gotos.
            if followed_ten_gotos {
                target = initial;
            }
            let final_references = references
                .get_mut(target)
                .ok_or_else(|| Error::internal("jump target is out of bounds"))?;
            *final_references = final_references
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "out of memory"))?;
            Ok(target)
        };

    let mut current_site = None;
    let mut late_throw_sites = Vec::new();
    index = 0;
    while index < code.len() {
        if !survives_first_pass[index] {
            index += 1;
            continue;
        }
        if marker_before_label[index].is_some() {
            current_site = marker_before_label[index];
        }
        if pc_sites[index].is_some() {
            current_site = pc_sites[index];
        }

        // `resolve_labels` folds the same adjacent constant-condition forms
        // as `fold_quickjs_constant_branches`. A non-taken branch releases its
        // forward label before a following late throw is visited; a taken one
        // becomes a terminal Goto whose target reference remains live.
        let constant_truthy = match code[index] {
            Instruction::Undefined | Instruction::Null | Instruction::PushFalse => Some(false),
            Instruction::PushTrue => Some(true),
            Instruction::PushI32(value) => Some(value != 0),
            Instruction::PushAtomValueIndex(_) => Some(true),
            _ => None,
        };
        let mut folded_goto = false;
        let mut terminal_tail = index + 1;
        if let Some(truthy) = constant_truthy
            && let Some(conditional_index) = index.checked_add(1)
            && conditional_index < code.len()
            && survives_first_pass[conditional_index]
            // `code_match` skips source markers but never crosses a physical
            // OP_label, even after earlier rewrites reduce its refcount to
            // zero. Direct-target IR therefore needs an immutable label bit;
            // the mutable reference count alone is not an adjacency test.
            && !has_physical_label[conditional_index]
        {
            let branch = match code[conditional_index] {
                Instruction::IfFalse(target) => Some((false, target)),
                Instruction::IfTrue(target) => Some((true, target)),
                _ => None,
            };
            if let Some((branch_on_true, target)) = branch {
                if marker_before_label[conditional_index].is_some() {
                    current_site = marker_before_label[conditional_index];
                }
                if pc_sites[conditional_index].is_some() {
                    current_site = pc_sites[conditional_index];
                }
                let target = usize::try_from(target)
                    .map_err(|_| Error::internal("jump target did not fit usize"))?;
                terminal_tail = conditional_index + 1;
                if truthy == branch_on_true {
                    follow_jump_target(target, &mut label_references)?;
                    folded_goto = true;
                } else {
                    label_references[target] = label_references[target]
                        .checked_sub(1)
                        .ok_or_else(|| Error::internal("jump label reference count underflow"))?;
                    index = terminal_tail;
                    continue;
                }
            }
        }

        let mut followed_target = None;
        if !folded_goto && let Some(target) = branch_target(&code[index])? {
            followed_target = Some(follow_jump_target(target, &mut label_references)?);
        }

        // QuickJS also folds `if_x(l1); goto(l2); label(l1)` to the opposite
        // conditional targeting `l2`. The Goto is consumed, its existing l2
        // reference is reused by the conditional, and l1 loses the reference
        // transferred above. This must happen before either branch's late
        // readonly throw updates the shared label table.
        if !folded_goto
            && matches!(
                code[index],
                Instruction::IfFalse(_) | Instruction::IfTrue(_)
            )
            && let Some(effective_target) = followed_target
        {
            let mut goto_index = index + 1;
            while goto_index < code.len() && !survives_first_pass[goto_index] {
                goto_index += 1;
            }
            if goto_index < code.len()
                && !has_physical_label[goto_index]
                && matches!(code[goto_index], Instruction::Goto(_))
            {
                let mut after_goto = goto_index + 1;
                while after_goto < code.len() && !survives_first_pass[after_goto] {
                    after_goto += 1;
                }
                let has_effective_label = after_goto < code.len()
                    && ((has_physical_label[after_goto] && after_goto == effective_target)
                        || (matches!(code[after_goto], Instruction::Goto(_))
                            && branch_target(&code[after_goto])? == Some(effective_target)));
                if has_effective_label {
                    if pc_sites[goto_index].is_some() {
                        current_site = pc_sites[goto_index];
                    }
                    label_references[effective_target] = label_references[effective_target]
                        .checked_sub(1)
                        .ok_or_else(|| Error::internal("jump label reference count underflow"))?;
                    index = after_goto;
                    continue;
                }
            }
        }

        let terminal = folded_goto
            || matches!(
                code[index],
                Instruction::Goto(_)
                    | Instruction::Return
                    | Instruction::ReturnUndefined
                    | Instruction::Throw
                    | Instruction::Ret
                    | Instruction::ThrowReadOnly(_)
                    | Instruction::ThrowRedeclaration(_)
            );
        if !terminal {
            index += 1;
            continue;
        }

        let terminal_index = index;
        let mut dead_index = terminal_tail;
        while dead_index < code.len() {
            if !survives_first_pass[dead_index] {
                dead_index += 1;
                continue;
            }
            // The first pass emits its final removed marker before the label,
            // so the second pass observes it even when another live reference
            // makes that label the stopping point.
            if marker_before_label[dead_index].is_some() {
                current_site = marker_before_label[dead_index];
            }
            if label_references[dead_index] > 0 {
                break;
            }
            if pc_sites[dead_index].is_some() {
                current_site = pc_sites[dead_index];
            }
            if let Some(target) = label_target(&code[dead_index])? {
                label_references[target] = label_references[target]
                    .checked_sub(1)
                    .ok_or_else(|| Error::internal("jump label reference count underflow"))?;
            }
            dead_index += 1;
        }
        if matches!(
            code[terminal_index],
            Instruction::ThrowReadOnly(_) | Instruction::ThrowRedeclaration(_)
        ) {
            late_throw_sites.push((terminal_index, current_site));
        }
        index = dead_index;
    }

    for (index, site) in late_throw_sites {
        pc_sites[index] = site;
    }
    Ok(())
}

fn resolve_identifier(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    use_scope: ScopeId,
    name: &str,
    span: Span,
    access: IdentifierAccess,
) -> Result<IrOp, Error> {
    if access == IdentifierAccess::AnnexBPut {
        let binding = find_or_create_own_binding(tree, function_id, use_scope, name, span)?
            .ok_or_else(|| Error::internal("Annex B root binding was not registered"))?;
        if binding.kind != BindingKind::Normal
            && !matches!(binding.storage, BindingStorage::External(_))
        {
            return Err(Error::internal(
                "Annex B root write resolved to a non-ordinary binding",
            ));
        }
        if binding.storage == BindingStorage::Global {
            let closure_index = capture_global_path(tree, function_id, name)?;
            return Ok(IrOp::Bytecode(Instruction::PutVar(closure_index)));
        }
        if let BindingStorage::External(index) = binding.storage {
            return closure_binding_operation(
                &mut tree.functions[function_id],
                index,
                binding.kind,
                IdentifierAccess::Put,
                name,
            );
        }
        return binding_instruction(
            &mut tree.functions[function_id],
            binding,
            IdentifierAccess::Put,
            name,
        )
        .map(IrOp::Bytecode);
    }
    let path = resolve_identifier_path(tree, function_id, use_scope, name, span, access)?;
    wrap_dynamic_identifier(
        &mut tree.functions[function_id],
        name,
        access,
        path.sources,
        path.fallback,
    )
}

fn resolve_import_meta(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
) -> Result<IrOp, Error> {
    let binding = tree
        .module
        .as_ref()
        .ok_or_else(|| Error::internal("import.meta escaped module compilation"))?
        .bindings
        .iter()
        .enumerate()
        .find_map(|(index, binding)| {
            binding
                .is_import_meta
                .then_some(module::ModuleBindingId(index))
        })
        .ok_or_else(|| Error::internal("import.meta has no hidden module binding"))?;
    resolved_binding_operation(
        tree,
        0,
        consuming_function,
        ResolvedBinding {
            storage: BindingStorage::Module(binding),
            kind: BindingKind::Lexical { is_const: true },
        },
        IdentifierAccess::Get,
        crate::engine::code::module::MODULE_IMPORT_META_BINDING_NAME,
    )
}

#[derive(Debug)]
struct ResolvedIdentifierPath {
    sources: Vec<DynamicEnvironmentSource>,
    fallback: IrOp,
    fallback_readonly: bool,
}

fn resolve_identifier_reference(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    use_scope: ScopeId,
    name: &str,
    span: Span,
    access: IdentifierReferenceAccess,
) -> Result<IrOp, Error> {
    let fallback_access = match access {
        IdentifierReferenceAccess::Prepare | IdentifierReferenceAccess::Set => {
            IdentifierAccess::Set
        }
        IdentifierReferenceAccess::Get | IdentifierReferenceAccess::Call => IdentifierAccess::Get,
        IdentifierReferenceAccess::PostPut => IdentifierAccess::Put,
    };
    let path = resolve_identifier_path(tree, function_id, use_scope, name, span, fallback_access)?;
    let syntactic_with = has_authored_with_scope(tree, function_id, use_scope)?;
    let (sources, late_sources) = if syntactic_with {
        (path.sources, Vec::new())
    } else {
        (Vec::new(), path.sources)
    };
    let name = ensure_string_constant(&mut tree.functions[function_id], name)?;
    Ok(IrOp::DynamicIdentifierReference {
        name,
        access,
        sources: sources.into_boxed_slice(),
        late_sources: late_sources.into_boxed_slice(),
        fallback: Box::new(path.fallback),
        syntactic_with,
        fallback_readonly: path.fallback_readonly,
    })
}

fn has_authored_with_scope(
    tree: &FunctionTree,
    function_id: FunctionId,
    use_scope: ScopeId,
) -> Result<bool, Error> {
    let mut owner = function_id;
    let mut scope = use_scope;
    loop {
        loop {
            let current = tree.functions[owner]
                .scopes
                .get(scope.0)
                .ok_or_else(|| Error::internal("identifier use scope is out of bounds"))?;
            if current.kind == ScopeKind::With {
                return Ok(true);
            }
            let Some(parent) = current.parent else {
                break;
            };
            scope = parent;
        }
        let Some(parent) = tree.functions[owner].parent else {
            return Ok(false);
        };
        owner = parent.function;
        scope = parent.definition_scope;
    }
}

/// Resolve one identifier scope-by-scope.  An exact authored binding wins in
/// its scope; otherwise that scope's hidden `with` record is appended before
/// continuing outward. Synthetic eval roots replay their imported descriptors
/// in the original inner-to-outer order, including `<var>` and `<with>`.
fn resolve_identifier_path(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    use_scope: ScopeId,
    name: &str,
    span: Span,
    access: IdentifierAccess,
) -> Result<ResolvedIdentifierPath, Error> {
    let mut sources = Vec::new();
    let pseudo = PseudoBinding::from_name(name);
    if pseudo.is_some()
        && !matches!(
            access,
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined
        )
        && !(pseudo == Some(PseudoBinding::This)
            && access == IdentifierAccess::InitializeDerivedThis)
    {
        return Err(Error::internal(
            "pseudo binding received a non-read operation",
        ));
    }
    let mut owner = consuming_function;
    let mut scope = use_scope;
    loop {
        let terminal_scope_kind = loop {
            let (scope_kind, parent, exact, with_binding) = {
                let function = tree
                    .functions
                    .get(owner)
                    .ok_or_else(|| Error::internal("identifier owner is out of bounds"))?;
                let current = function
                    .scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("identifier use scope is out of bounds"))?;
                let exact = current.bindings.iter().rev().find_map(|binding| {
                    let binding = function.bindings.get(binding.0)?;
                    (binding.name == name
                        && (pseudo.is_some()
                            || !matches!(binding.storage, BindingStorage::External(_))))
                    .then_some(ResolvedBinding {
                        storage: binding.storage,
                        kind: binding.kind,
                    })
                });
                let with_binding = (pseudo.is_none() && current.kind == ScopeKind::With)
                    .then(|| {
                        current
                            .bindings
                            .iter()
                            .find_map(|binding| {
                                let binding = function.bindings.get(binding.0)?;
                                (binding.kind == BindingKind::WithObject).then_some(
                                    ResolvedBinding {
                                        storage: binding.storage,
                                        kind: binding.kind,
                                    },
                                )
                            })
                            .ok_or_else(|| {
                                Error::internal("with scope lost its hidden object binding")
                            })
                    })
                    .transpose()?;
                (current.kind, current.parent, exact, with_binding)
            };

            if let Some(binding) = exact {
                let fallback_readonly = binding_is_readonly(binding.kind);
                let fallback = resolved_binding_operation(
                    tree,
                    owner,
                    consuming_function,
                    binding,
                    access,
                    name,
                )?;
                return Ok(ResolvedIdentifierPath {
                    sources,
                    fallback,
                    fallback_readonly,
                });
            }

            if let Some(binding) = with_binding {
                push_dynamic_environment_source(
                    tree,
                    owner,
                    consuming_function,
                    binding,
                    WITH_OBJECT_LOCAL_NAME,
                    &mut sources,
                )?;
            }

            let Some(parent) = parent else {
                debug_assert!(matches!(
                    scope_kind,
                    ScopeKind::FunctionRoot | ScopeKind::Parameter
                ));
                break scope_kind;
            };
            scope = parent;
        };

        let own_binding = if let Some(pseudo) = pseudo {
            find_or_create_own_pseudo_binding(tree, owner, pseudo, span)?
        } else {
            // `arguments` and a private function-expression name are
            // logically rooted before the function's own eval variable
            // object. A sloppy delete of implicit `arguments` is false
            // without materializing it.
            if name == "arguments"
                && access == IdentifierAccess::Delete
                && matches!(
                    tree.functions[owner].kind,
                    FunctionKind::Ordinary | FunctionKind::Method
                )
            {
                return Ok(ResolvedIdentifierPath {
                    sources,
                    fallback: IrOp::Bytecode(Instruction::PushFalse),
                    fallback_readonly: false,
                });
            }
            if matches!(tree.functions[owner].kind, FunctionKind::Eval(_)) {
                None
            } else if terminal_scope_kind == ScopeKind::Parameter {
                find_or_create_parameter_special_binding(tree, owner, name, span)?
            } else {
                find_or_create_own_binding(tree, owner, ScopeId(0), name, span)?
            }
        };
        if let Some(binding) = own_binding {
            let fallback_readonly = binding_is_readonly(binding.kind);
            let fallback =
                resolved_binding_operation(tree, owner, consuming_function, binding, access, name)?;
            return Ok(ResolvedIdentifierPath {
                sources,
                fallback,
                fallback_readonly,
            });
        }

        if matches!(tree.functions[owner].kind, FunctionKind::Eval(_)) {
            if let Some(binding) =
                resolve_eval_external_chain(tree, owner, consuming_function, name, &mut sources)?
            {
                let fallback_readonly = binding_is_readonly(binding.kind);
                let fallback = resolved_binding_operation(
                    tree,
                    owner,
                    consuming_function,
                    binding,
                    access,
                    name,
                )?;
                return Ok(ResolvedIdentifierPath {
                    sources,
                    fallback,
                    fallback_readonly,
                });
            }
        } else if pseudo.is_none() {
            push_owned_eval_variable_sources(
                tree,
                owner,
                consuming_function,
                terminal_scope_kind,
                &mut sources,
            )?;
        }

        let Some(parent) = tree.functions[owner].parent else {
            break;
        };
        owner = parent.function;
        scope = parent.definition_scope;
    }

    if pseudo.is_some() {
        return Err(Error::internal(
            "pseudo binding escaped every authenticated function owner",
        ));
    }
    let closure_index = capture_global_path(tree, consuming_function, name)?;
    let fallback = match access {
        IdentifierAccess::Get => IrOp::Bytecode(Instruction::GetVar(closure_index)),
        IdentifierAccess::GetOrUndefined => IrOp::Bytecode(Instruction::GetVarUndef(closure_index)),
        IdentifierAccess::Delete => IrOp::Bytecode(Instruction::DeleteVar(closure_index)),
        IdentifierAccess::Initialize => {
            return Err(Error::internal(
                "lexical initializer did not resolve to its owning local",
            ));
        }
        IdentifierAccess::InitializeDerivedThis => {
            return Err(Error::internal(
                "derived this initializer escaped its authenticated binding",
            ));
        }
        IdentifierAccess::Put => IrOp::Bytecode(Instruction::PutVar(closure_index)),
        IdentifierAccess::AnnexBPut => {
            return Err(Error::internal(
                "Annex B write escaped its dedicated resolver path",
            ));
        }
        IdentifierAccess::Set => IrOp::GlobalSet(closure_index),
    };
    Ok(ResolvedIdentifierPath {
        sources,
        fallback,
        fallback_readonly: false,
    })
}

const fn binding_is_readonly(kind: BindingKind) -> bool {
    matches!(
        kind,
        BindingKind::Lexical { is_const: true }
            | BindingKind::FunctionName { is_const: true }
            | BindingKind::PrivateField { .. }
            | BindingKind::PrivateMethod { .. }
            | BindingKind::PrivateGetter { .. }
            | BindingKind::PrivateSetter { .. }
            | BindingKind::PrivateGetterSetter { .. }
    )
}

fn resolved_binding_operation(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    binding: ResolvedBinding,
    access: IdentifierAccess,
    name: &str,
) -> Result<IrOp, Error> {
    if binding.storage == BindingStorage::Global {
        return global_declaration_operation(tree, consuming_function, binding.kind, access, name);
    }
    if let BindingStorage::Module(binding_id) = binding.storage
        && defining_function == consuming_function
    {
        let index = module_binding_closure_index(tree, binding_id)?;
        let import_lexical_collision = tree
            .module
            .as_ref()
            .and_then(|module| module.binding(binding_id).ok())
            .is_some_and(|binding| {
                binding.import.is_some()
                    && matches!(
                        binding.declaration,
                        Some(module::ModuleDeclarationOrigin::Lexical { .. })
                    )
            });
        return module_binding_operation(
            &mut tree.functions[consuming_function],
            index,
            binding.kind,
            access,
            name,
            import_lexical_collision,
        );
    }
    if defining_function == consuming_function {
        if let BindingStorage::External(index) = binding.storage {
            return closure_binding_operation(
                &mut tree.functions[consuming_function],
                index,
                binding.kind,
                access,
                name,
            );
        }
        return binding_instruction(
            &mut tree.functions[consuming_function],
            binding,
            access,
            name,
        )
        .map(IrOp::Bytecode);
    }
    let (closure_index, kind) = capture_binding_path(
        tree,
        defining_function,
        consuming_function,
        binding,
        name,
        false,
        false,
    )?;
    closure_binding_operation(
        &mut tree.functions[consuming_function],
        closure_index,
        kind,
        access,
        name,
    )
}

fn module_binding_closure_index(
    tree: &FunctionTree,
    binding: module::ModuleBindingId,
) -> Result<u16, Error> {
    tree.module
        .as_ref()
        .ok_or_else(|| Error::internal("module binding has no module record"))?
        .binding(binding)?
        .closure_index
        .ok_or_else(|| Error::internal("module binding closure was not seeded"))
}

fn binding_storage_is_module_import_view(
    tree: &FunctionTree,
    defining_function: FunctionId,
    storage: BindingStorage,
) -> Result<bool, Error> {
    let kind = match storage {
        BindingStorage::Module(binding) => {
            let index = module_binding_closure_index(tree, binding)?;
            tree.functions
                .first()
                .and_then(|function| function.closure_variables.get(usize::from(index)))
                .map(|descriptor| descriptor.kind)
                .ok_or_else(|| Error::internal("module binding descriptor is missing"))?
        }
        BindingStorage::External(index) => tree
            .functions
            .get(defining_function)
            .and_then(|function| function.external_bindings.get(usize::from(index)))
            .map(|binding| binding.kind)
            .ok_or_else(|| Error::internal("eval external binding is missing"))?,
        BindingStorage::Argument(_) | BindingStorage::Local(_) | BindingStorage::Global => {
            return Ok(false);
        }
    };
    Ok(kind == ClosureVariableKind::ModuleImportView)
}

fn module_binding_operation(
    function: &mut FunctionIr,
    index: u16,
    kind: BindingKind,
    access: IdentifierAccess,
    name: &str,
    import_lexical_collision: bool,
) -> Result<IrOp, Error> {
    if access == IdentifierAccess::Initialize {
        if import_lexical_collision {
            return Ok(IrOp::Bytecode(
                Instruction::InitializeModuleImportCollision(index),
            ));
        }
        return match kind {
            BindingKind::Lexical { .. } => Ok(IrOp::Bytecode(Instruction::InitializeVarRef(index))),
            _ => Err(Error::internal(
                "ordinary module binding used lexical initialization",
            )),
        };
    }
    closure_binding_operation(function, index, kind, access, name)
}

fn push_owned_eval_variable_sources(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    terminal_scope_kind: ScopeKind,
    sources: &mut Vec<DynamicEnvironmentSource>,
) -> Result<(), Error> {
    let (body, parameter) = {
        let function = &tree.functions[defining_function];
        (
            function.eval_variable_object_local,
            function.arg_eval_variable_object_local,
        )
    };
    if terminal_scope_kind != ScopeKind::Parameter
        && let Some(index) = body
    {
        push_dynamic_environment_source(
            tree,
            defining_function,
            consuming_function,
            ResolvedBinding {
                storage: BindingStorage::Local(index),
                kind: BindingKind::EvalVariableObject,
            },
            EVAL_VARIABLE_OBJECT_LOCAL_NAME,
            sources,
        )?;
    }
    if let Some(index) = parameter {
        push_dynamic_environment_source(
            tree,
            defining_function,
            consuming_function,
            ResolvedBinding {
                storage: BindingStorage::Local(index),
                kind: BindingKind::ArgEvalVariableObject,
            },
            ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME,
            sources,
        )?;
    }
    Ok(())
}

fn push_dynamic_environment_source(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    binding: ResolvedBinding,
    sentinel: &str,
    sources: &mut Vec<DynamicEnvironmentSource>,
) -> Result<(), Error> {
    let storage = if defining_function == consuming_function {
        binding.storage
    } else {
        let (closure, kind) = capture_binding_path(
            tree,
            defining_function,
            consuming_function,
            binding,
            sentinel,
            true,
            false,
        )?;
        if kind != binding.kind {
            return Err(Error::internal(
                "dynamic environment closure relay changed kind",
            ));
        }
        BindingStorage::External(closure)
    };
    let source = match (binding.kind, storage) {
        (
            BindingKind::EvalVariableObject | BindingKind::ArgEvalVariableObject,
            BindingStorage::Local(index),
        ) => DynamicEnvironmentSource::Eval(EvalVariableSource::Local(index)),
        (
            BindingKind::EvalVariableObject | BindingKind::ArgEvalVariableObject,
            BindingStorage::External(index),
        ) => DynamicEnvironmentSource::Eval(EvalVariableSource::Closure(index)),
        (BindingKind::WithObject, BindingStorage::Local(index)) => {
            DynamicEnvironmentSource::With(WithObjectSource::Local(index))
        }
        (BindingKind::WithObject, BindingStorage::External(index)) => {
            DynamicEnvironmentSource::With(WithObjectSource::Closure(index))
        }
        (
            BindingKind::Normal
            | BindingKind::Lexical { .. }
            | BindingKind::FunctionName { .. }
            | BindingKind::PrivateField { .. }
            | BindingKind::PrivateMethod { .. }
            | BindingKind::PrivateGetter { .. }
            | BindingKind::PrivateSetter { .. }
            | BindingKind::PrivateGetterSetter { .. },
            _,
        )
        | (_, BindingStorage::Argument(_) | BindingStorage::Module(_) | BindingStorage::Global) => {
            return Err(Error::internal(
                "ordinary binding reached dynamic environment selection",
            ));
        }
    };
    sources.push(source);
    Ok(())
}

fn resolve_eval_external_chain(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    name: &str,
    sources: &mut Vec<DynamicEnvironmentSource>,
) -> Result<Option<ResolvedBinding>, Error> {
    let external = tree.functions[defining_function].external_bindings.clone();
    for (index, binding) in external.into_iter().enumerate() {
        let index = u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
        if matches!(
            binding.kind,
            ClosureVariableKind::EvalVariableObject
                | ClosureVariableKind::ArgEvalVariableObject
                | ClosureVariableKind::WithObject
        ) {
            let (kind, sentinel) = match binding.kind {
                ClosureVariableKind::EvalVariableObject => (
                    BindingKind::EvalVariableObject,
                    EVAL_VARIABLE_OBJECT_LOCAL_NAME,
                ),
                ClosureVariableKind::ArgEvalVariableObject => (
                    BindingKind::ArgEvalVariableObject,
                    ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME,
                ),
                ClosureVariableKind::WithObject => {
                    (BindingKind::WithObject, WITH_OBJECT_LOCAL_NAME)
                }
                _ => unreachable!(),
            };
            push_dynamic_environment_source(
                tree,
                defining_function,
                consuming_function,
                ResolvedBinding {
                    storage: BindingStorage::External(index),
                    kind,
                },
                sentinel,
                sources,
            )?;
            continue;
        }
        if binding.name.to_utf8_lossy() != name {
            continue;
        }
        let kind =
            binding_kind_from_closure_flags(binding.kind, binding.is_lexical, binding.is_const)
                .ok_or_else(|| Error::internal("eval caller binding flags are inconsistent"))?;
        return Ok(Some(ResolvedBinding {
            storage: BindingStorage::External(index),
            kind,
        }));
    }
    Ok(None)
}

fn wrap_dynamic_identifier(
    function: &mut FunctionIr,
    name: &str,
    access: IdentifierAccess,
    sources: Vec<DynamicEnvironmentSource>,
    fallback: IrOp,
) -> Result<IrOp, Error> {
    if sources.is_empty() {
        return Ok(fallback);
    }
    if matches!(
        access,
        IdentifierAccess::Initialize
            | IdentifierAccess::InitializeDerivedThis
            | IdentifierAccess::AnnexBPut
    ) {
        return Err(Error::internal(
            "declaration-only identifier access crossed a dynamic environment",
        ));
    }
    let name = ensure_string_constant(function, name)?;
    Ok(IrOp::DynamicIdentifier {
        name,
        access,
        sources: sources.into_boxed_slice(),
        fallback: Box::new(fallback),
    })
}

fn global_declaration_operation(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    kind: BindingKind,
    access: IdentifierAccess,
    name: &str,
) -> Result<IrOp, Error> {
    let binding_is_lexical = match kind {
        BindingKind::Normal => false,
        BindingKind::Lexical { .. } => true,
        BindingKind::FunctionName { .. } => {
            return Err(Error::internal(
                "global declaration has function-name binding metadata",
            ));
        }
        BindingKind::EvalVariableObject | BindingKind::ArgEvalVariableObject => {
            return Err(Error::internal(
                "eval variable object reached global declaration resolution",
            ));
        }
        BindingKind::WithObject => {
            return Err(Error::internal(
                "with object reached global declaration resolution",
            ));
        }
        BindingKind::PrivateField { .. }
        | BindingKind::PrivateMethod { .. }
        | BindingKind::PrivateGetter { .. }
        | BindingKind::PrivateSetter { .. }
        | BindingKind::PrivateGetterSetter { .. } => {
            return Err(Error::internal(
                "private binding reached global declaration resolution",
            ));
        }
    };
    // `resolve_scope_var` scans QuickJS's ordered GLOBAL_DECL list by name and
    // therefore resolves through the first matching descriptor, even when a
    // preceding Annex B normal record has masked a later Program lexical. The
    // two descriptors share the lexical VarRef after instantiation, but the
    // first descriptor's non-lexical flag remains observable: an uninitialized
    // read falls back to the replacement global-object property instead of
    // throwing a lexical TDZ error. Writes still inspect the shared VarRef's
    // lexical/const metadata in the VM.
    let descriptor_is_lexical =
        binding_is_lexical && !tree.functions[0].first_global_declaration_is_normal(name);
    let closure_index =
        capture_global_declaration_path(tree, consuming_function, name, descriptor_is_lexical)?;
    Ok(match access {
        IdentifierAccess::Get => IrOp::Bytecode(Instruction::GetVar(closure_index)),
        IdentifierAccess::GetOrUndefined => IrOp::Bytecode(Instruction::GetVarUndef(closure_index)),
        IdentifierAccess::Delete => IrOp::Bytecode(Instruction::DeleteVar(closure_index)),
        IdentifierAccess::Initialize if binding_is_lexical => {
            IrOp::Bytecode(Instruction::PutVarInit(closure_index))
        }
        IdentifierAccess::Initialize => {
            return Err(Error::internal(
                "ordinary global declaration used lexical initialization",
            ));
        }
        IdentifierAccess::InitializeDerivedThis => {
            return Err(Error::internal(
                "derived this initializer resolved to a global binding",
            ));
        }
        IdentifierAccess::Put => IrOp::Bytecode(Instruction::PutVar(closure_index)),
        IdentifierAccess::AnnexBPut => {
            return Err(Error::internal(
                "Annex B write reached declaration-bound global resolution",
            ));
        }
        IdentifierAccess::Set => IrOp::GlobalSet(closure_index),
    })
}

/// Install QuickJS's `GLOBAL_DECL -> PARENT_GLOBAL` chain for a Program
/// binding. The root descriptor triggers declaration instantiation;
/// descendants reuse that exact VarRef without repeating the definition.
fn capture_global_declaration_path(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    name: &str,
    is_lexical: bool,
) -> Result<u16, Error> {
    let mut path = Vec::new();
    let mut cursor = Some(consuming_function);
    while let Some(function_id) = cursor {
        path.push(function_id);
        cursor = tree.functions[function_id]
            .parent
            .map(|parent| parent.function);
    }
    path.reverse();

    let (root_index, root_descriptor) = tree.functions[0]
        .global_declarations
        .iter()
        .find(|declaration| declaration.name == name && declaration.is_lexical == is_lexical)
        .and_then(|declaration| declaration.closure_index)
        .and_then(|index| {
            tree.functions[0]
                .closure_variables
                .get(usize::from(index))
                .copied()
                .map(|descriptor| (index, descriptor))
        })
        .ok_or_else(|| Error::internal("global declaration closure was not seeded"))?;
    let mut source = ClosureSource::ParentGlobal(root_index);
    let mut final_index = Some(root_index);
    for function_id in path.into_iter().skip(1) {
        let name_index = ensure_string_constant(&mut tree.functions[function_id], name)?;
        let descriptor = ClosureVariable {
            source,
            name: ClosureVariableName::Constant(name_index),
            is_lexical: root_descriptor.is_lexical,
            is_const: root_descriptor.is_const,
            kind: root_descriptor.kind,
        };
        let index = ensure_closure_variable(&mut tree.functions[function_id], descriptor)?;
        source = ClosureSource::ParentGlobal(index);
        final_index = Some(index);
    }
    final_index.ok_or_else(|| Error::internal("global declaration closure path was empty"))
}

/// Install the same Global -> ParentGlobal relay chain QuickJS creates while
/// resolving an otherwise-unbound identifier. Every function owns its exact
/// name atom after publication; descendants share the root VarRef identity.
fn capture_global_path(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    name: &str,
) -> Result<u16, Error> {
    let mut path = Vec::new();
    let mut cursor = Some(consuming_function);
    while let Some(function_id) = cursor {
        path.push(function_id);
        cursor = tree.functions[function_id]
            .parent
            .map(|parent| parent.function);
    }
    path.reverse();

    let mut source = ClosureSource::Global;
    let mut final_index = None;
    for function_id in path {
        let name_index = ensure_string_constant(&mut tree.functions[function_id], name)?;
        let descriptor = ClosureVariable {
            source,
            name: ClosureVariableName::Constant(name_index),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        };
        let index = ensure_closure_variable(&mut tree.functions[function_id], descriptor)?;
        source = ClosureSource::ParentGlobal(index);
        final_index = Some(index);
    }
    final_index.ok_or_else(|| Error::internal("global closure path was empty"))
}

pub(super) fn ensure_string_constant(function: &mut FunctionIr, name: &str) -> Result<u32, Error> {
    let name = JsString::try_from_utf8(name)?;
    if let Some(index) = function.constants.iter().position(
        |constant| matches!(constant, IrConstant::Primitive(Value::String(value)) if value == &name),
    ) {
        return u32::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"));
    }
    let index = u32::try_from(function.constants.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?;
    function
        .constants
        .push(IrConstant::Primitive(Value::String(name)));
    Ok(index)
}

/// A parentless parameter environment is a deliberate visibility barrier, not
/// a second path into the body FunctionRoot. Only bindings which ECMAScript
/// establishes before parameter evaluation may cross it.
fn find_or_create_parameter_special_binding(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    name: &str,
    span: Span,
) -> Result<Option<ResolvedBinding>, Error> {
    let function = tree
        .functions
        .get(function_id)
        .ok_or_else(|| Error::internal("parameter binding owner is out of bounds"))?;
    let arguments = name == "arguments"
        && matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
        && !function
            .parameters
            .iter()
            .any(|parameter| parameter.as_deref() == Some(name));
    let private_name =
        function.private_name_binding && function.function_name.as_deref() == Some(name);
    if arguments {
        find_or_create_own_binding(tree, function_id, ScopeId(0), name, span)
    } else if private_name {
        find_or_create_private_function_name_binding(tree, function_id, name, span)
    } else {
        Ok(None)
    }
}

/// Materialize a named function expression's private self binding without
/// consulting same-named body declarations. The private name is outside the
/// function body environment: a parameter initializer must see it, while an
/// authored body `var`/function of the same name must continue to shadow it.
fn find_or_create_private_function_name_binding(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    name: &str,
    span: Span,
) -> Result<Option<ResolvedBinding>, Error> {
    let function = tree
        .functions
        .get(function_id)
        .ok_or_else(|| Error::internal("function-name binding owner is out of bounds"))?;
    if !function.private_name_binding || function.function_name.as_deref() != Some(name) {
        return Ok(None);
    }
    if let Some(index) = function.function_name_local {
        let kind = BindingKind::FunctionName {
            is_const: function.strict,
        };
        if function.bindings.iter().any(|binding| {
            binding.name == name
                && binding.storage == BindingStorage::Local(index)
                && binding.kind == kind
        }) {
            return Ok(Some(ResolvedBinding {
                storage: BindingStorage::Local(index),
                kind,
            }));
        }
        return Err(Error::internal(
            "function-name local is missing its private binding",
        ));
    }

    let function = &mut tree.functions[function_id];
    if function.locals.len() >= MAX_LOCAL_VARIABLES {
        return Err(
            Error::new(ErrorKind::JsInternal, "too many local variables")
                .with_span(source_span(span)),
        );
    }
    let index = u16::try_from(function.locals.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
    let kind = BindingKind::FunctionName {
        is_const: function.strict,
    };
    let root = function.var_scope;
    let insertion = function.scopes[root.0]
        .bindings
        .iter()
        .position(|binding| function.bindings[binding.0].name == name)
        .unwrap_or(function.scopes[root.0].bindings.len());
    function.locals.push(name.to_owned());
    function.function_name_local = Some(index);
    let binding = function.add_binding(
        root,
        root,
        name.to_owned(),
        BindingStorage::Local(index),
        kind,
        None,
    );
    let appended = function.scopes[root.0]
        .bindings
        .pop()
        .ok_or_else(|| Error::internal("function-name binding was not appended"))?;
    if appended != binding {
        return Err(Error::internal(
            "function-name binding append order is malformed",
        ));
    }
    function.scopes[root.0].bindings.insert(insertion, binding);
    Ok(Some(ResolvedBinding {
        storage: BindingStorage::Local(index),
        kind,
    }))
}

pub(super) fn find_or_create_own_binding(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    start_scope: ScopeId,
    name: &str,
    span: Span,
) -> Result<Option<ResolvedBinding>, Error> {
    let function = &tree.functions[function_id];
    if start_scope.0 >= function.scopes.len() {
        return Err(Error::internal("identifier use scope is out of bounds"));
    }
    if let Some(binding) = function.binding_from_scope(start_scope, name) {
        return Ok(Some(binding));
    }
    if name == "arguments" && function.arguments_forbidden {
        // QuickJS's parser rejects true IdentifierReferences earlier, but its
        // object-shorthand path deliberately falls through this synthetic
        // initializer frame and may capture an enclosing arguments binding.
        return Ok(None);
    }
    if name == "arguments" && matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
    {
        let function = &mut tree.functions[function_id];
        if function.arguments_local.is_some() {
            return Err(Error::internal(
                "implicit arguments local is missing its root binding",
            ));
        }
        if function.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(
                Error::new(ErrorKind::JsInternal, "too many local variables")
                    .with_span(source_span(span)),
            );
        }
        let index = u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        function.locals.push(name.to_owned());
        function.arguments_local = Some(index);
        function.add_binding(
            function.var_scope,
            function.var_scope,
            name.to_owned(),
            BindingStorage::Local(index),
            BindingKind::Normal,
            None,
        );
        return Ok(Some(ResolvedBinding {
            storage: BindingStorage::Local(index),
            kind: BindingKind::Normal,
        }));
    }
    find_or_create_private_function_name_binding(tree, function_id, name, span)
}

fn binding_instruction(
    function: &mut FunctionIr,
    binding: ResolvedBinding,
    access: IdentifierAccess,
    name: &str,
) -> Result<Instruction, Error> {
    match (binding.storage, binding.kind, access) {
        (BindingStorage::Module(_), _, _) => Err(Error::internal(
            "module binding reached local binding instruction selection",
        )),
        (BindingStorage::Global, _, _) => Err(Error::internal(
            "global binding reached local binding instruction selection",
        )),
        (BindingStorage::External(_), _, _) => Err(Error::internal(
            "eval external binding reached local instruction selection",
        )),
        (
            _,
            BindingKind::PrivateField { .. }
            | BindingKind::PrivateMethod { .. }
            | BindingKind::PrivateGetter { .. }
            | BindingKind::PrivateSetter { .. }
            | BindingKind::PrivateGetterSetter { .. },
            _,
        ) => Err(Error::internal(
            "private binding reached ordinary identifier instruction selection",
        )),
        (
            BindingStorage::Local(_),
            BindingKind::EvalVariableObject
            | BindingKind::ArgEvalVariableObject
            | BindingKind::WithObject,
            _,
        ) => Err(Error::internal(
            "hidden object binding reached source binding instruction selection",
        )),
        (
            BindingStorage::Argument(index),
            _,
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(Instruction::GetArg(index)),
        (BindingStorage::Argument(_) | BindingStorage::Local(_), _, IdentifierAccess::Delete) => {
            Ok(Instruction::PushFalse)
        }
        (
            BindingStorage::Argument(_),
            _,
            IdentifierAccess::Initialize | IdentifierAccess::InitializeDerivedThis,
        ) => Err(Error::internal(
            "lexical initializer resolved to an argument binding",
        )),
        (BindingStorage::Argument(index), _, IdentifierAccess::Put) => {
            Ok(Instruction::PutArg(index))
        }
        (BindingStorage::Argument(index), _, IdentifierAccess::Set) => {
            Ok(Instruction::SetArg(index))
        }
        (
            BindingStorage::Local(index),
            BindingKind::Normal | BindingKind::FunctionName { .. },
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(Instruction::GetLocal(index)),
        (
            BindingStorage::Local(index),
            BindingKind::Lexical { .. },
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(Instruction::GetLocalCheck(index)),
        (
            BindingStorage::Local(index),
            BindingKind::Lexical { .. },
            IdentifierAccess::Initialize,
        ) => Ok(Instruction::InitializeLocal(index)),
        (
            BindingStorage::Local(index),
            BindingKind::Lexical { is_const: false },
            IdentifierAccess::InitializeDerivedThis,
        ) => Ok(Instruction::InitializeDerivedLocal(index)),
        (
            BindingStorage::Local(_),
            BindingKind::Normal
            | BindingKind::FunctionName { .. }
            | BindingKind::Lexical { is_const: true },
            IdentifierAccess::InitializeDerivedThis,
        ) => Err(Error::internal(
            "derived this initializer resolved to a non-mutable lexical local",
        )),
        (
            BindingStorage::Local(_),
            BindingKind::Normal | BindingKind::FunctionName { .. },
            IdentifierAccess::Initialize,
        ) => Err(Error::internal(
            "lexical initializer resolved to an ordinary local",
        )),
        (BindingStorage::Local(index), BindingKind::Normal, IdentifierAccess::Put) => {
            Ok(Instruction::PutLocal(index))
        }
        (BindingStorage::Local(index), BindingKind::Normal, IdentifierAccess::Set) => {
            Ok(Instruction::SetLocal(index))
        }
        (
            BindingStorage::Local(index),
            BindingKind::Lexical { is_const: false },
            IdentifierAccess::Put,
        ) => Ok(Instruction::PutLocalCheck(index)),
        (
            BindingStorage::Local(index),
            BindingKind::Lexical { is_const: false },
            IdentifierAccess::Set,
        ) => Ok(Instruction::SetLocalCheck(index)),
        (
            BindingStorage::Local(_),
            BindingKind::Lexical { is_const: true },
            IdentifierAccess::Put | IdentifierAccess::Set,
        ) => {
            let name = ensure_string_constant(function, name)?;
            Ok(Instruction::ThrowReadOnly(name))
        }
        (
            BindingStorage::Local(_),
            BindingKind::FunctionName { is_const },
            IdentifierAccess::Put | IdentifierAccess::Set,
        ) => function_name_write_instruction(function, name, is_const, access),
        (_, _, IdentifierAccess::AnnexBPut) => Err(Error::internal(
            "Annex B write reached ordinary binding instruction selection",
        )),
    }
}

fn closure_binding_operation(
    function: &mut FunctionIr,
    index: u16,
    kind: BindingKind,
    access: IdentifierAccess,
    name: &str,
) -> Result<IrOp, Error> {
    match (kind, access) {
        (
            BindingKind::EvalVariableObject
            | BindingKind::ArgEvalVariableObject
            | BindingKind::WithObject,
            _,
        ) => Err(Error::internal(
            "hidden object binding reached source closure operation selection",
        )),
        (
            BindingKind::PrivateField { .. }
            | BindingKind::PrivateMethod { .. }
            | BindingKind::PrivateGetter { .. }
            | BindingKind::PrivateSetter { .. }
            | BindingKind::PrivateGetterSetter { .. },
            _,
        ) => Err(Error::internal(
            "private binding reached ordinary identifier closure selection",
        )),
        (
            BindingKind::Normal | BindingKind::FunctionName { .. },
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined,
        ) => Ok(IrOp::Bytecode(Instruction::GetVarRef(index))),
        (BindingKind::Lexical { .. }, IdentifierAccess::Get | IdentifierAccess::GetOrUndefined) => {
            Ok(IrOp::Bytecode(Instruction::GetVarRefCheck(index)))
        }
        (_, IdentifierAccess::Delete) => Ok(IrOp::Bytecode(Instruction::PushFalse)),
        (_, IdentifierAccess::Initialize) => Err(Error::internal(
            "lexical initializer crossed a function boundary",
        )),
        (BindingKind::Lexical { is_const: false }, IdentifierAccess::InitializeDerivedThis) => {
            Ok(IrOp::Bytecode(Instruction::InitializeDerivedVarRef(index)))
        }
        (_, IdentifierAccess::InitializeDerivedThis) => Err(Error::internal(
            "derived this initializer crossed a non-mutable lexical binding",
        )),
        (BindingKind::Normal, IdentifierAccess::Put) => {
            Ok(IrOp::Bytecode(Instruction::PutVarRef(index)))
        }
        (BindingKind::Normal, IdentifierAccess::Set) => {
            Ok(IrOp::Bytecode(Instruction::SetVarRef(index)))
        }
        (BindingKind::Lexical { is_const: false }, IdentifierAccess::Put) => {
            Ok(IrOp::Bytecode(Instruction::PutVarRefCheck(index)))
        }
        (BindingKind::Lexical { is_const: false }, IdentifierAccess::Set) => {
            Ok(IrOp::CapturedLexicalSet(index))
        }
        (
            BindingKind::Lexical { is_const: true },
            IdentifierAccess::Put | IdentifierAccess::Set,
        ) => {
            let name = ensure_string_constant(function, name)?;
            Ok(IrOp::Bytecode(Instruction::ThrowReadOnly(name)))
        }
        (BindingKind::FunctionName { is_const }, IdentifierAccess::Put | IdentifierAccess::Set) => {
            function_name_write_instruction(function, name, is_const, access).map(IrOp::Bytecode)
        }
        (_, IdentifierAccess::AnnexBPut) => {
            Err(Error::internal("Annex B write crossed a function boundary"))
        }
    }
}

fn function_name_write_instruction(
    function: &mut FunctionIr,
    name: &str,
    is_const: bool,
    access: IdentifierAccess,
) -> Result<Instruction, Error> {
    if is_const {
        let name = ensure_string_constant(function, name)?;
        return Ok(Instruction::ThrowReadOnly(name));
    }
    Ok(match access {
        IdentifierAccess::Put => Instruction::Drop,
        IdentifierAccess::Set => Instruction::Nop,
        IdentifierAccess::Get
        | IdentifierAccess::GetOrUndefined
        | IdentifierAccess::Delete
        | IdentifierAccess::Initialize
        | IdentifierAccess::InitializeDerivedThis
        | IdentifierAccess::AnnexBPut => {
            return Err(Error::internal(
                "function-name write received a read access",
            ));
        }
    })
}

const fn closure_kind(kind: BindingKind) -> ClosureVariableKind {
    match kind {
        BindingKind::Normal | BindingKind::Lexical { .. } => ClosureVariableKind::Normal,
        BindingKind::FunctionName { .. } => ClosureVariableKind::FunctionName,
        BindingKind::EvalVariableObject => ClosureVariableKind::EvalVariableObject,
        BindingKind::ArgEvalVariableObject => ClosureVariableKind::ArgEvalVariableObject,
        BindingKind::WithObject => ClosureVariableKind::WithObject,
        BindingKind::PrivateField { .. } => ClosureVariableKind::PrivateField,
        BindingKind::PrivateMethod { .. } => ClosureVariableKind::PrivateMethod,
        BindingKind::PrivateGetter { .. } => ClosureVariableKind::PrivateGetter,
        BindingKind::PrivateSetter { .. } => ClosureVariableKind::PrivateSetter,
        BindingKind::PrivateGetterSetter { .. } => ClosureVariableKind::PrivateGetterSetter,
    }
}

pub(super) fn capture_binding_path(
    tree: &mut FunctionTree,
    defining_function: FunctionId,
    consuming_function: FunctionId,
    binding: ResolvedBinding,
    name: &str,
    retain_name: bool,
    erase_function_name: bool,
) -> Result<(u16, BindingKind), Error> {
    let module_import_view =
        binding_storage_is_module_import_view(tree, defining_function, binding.storage)?;
    let mut path = Vec::new();
    let mut cursor = consuming_function;
    while cursor != defining_function {
        path.push(cursor);
        cursor = tree.functions[cursor]
            .parent
            .ok_or_else(|| Error::internal("closure binding owner is not an ancestor"))?
            .function;
    }
    path.reverse();

    // QuickJS's `add_eval_variables` requests ordinary metadata for an
    // ancestor's unscoped FunctionName. `get_closure_var` recursively carries
    // that request unchanged but de-duplicates each hop solely by physical
    // source, so the first descriptor already occupying a slot wins. A later
    // plain descendant may therefore restore FunctionName on its own final
    // descriptor while relaying through an erased parent view. Synthetic Eval
    // roots use a different copy branch and never request this erasure.
    let may_lose_function_name = matches!(binding.kind, BindingKind::FunctionName { .. })
        && matches!(
            tree.functions[defining_function].kind,
            FunctionKind::Ordinary
        );
    let original_kind = binding.kind;
    let requested_kind = if erase_function_name && may_lose_function_name {
        BindingKind::Normal
    } else {
        original_kind
    };
    let mut source = match binding.storage {
        BindingStorage::Argument(index) => ClosureSource::ParentArgument(index),
        BindingStorage::Local(index) => ClosureSource::ParentLocal(index),
        BindingStorage::External(index) => ClosureSource::ParentClosure(index),
        BindingStorage::Module(binding) => {
            ClosureSource::ParentClosure(module_binding_closure_index(tree, binding)?)
        }
        BindingStorage::Global => {
            return Err(Error::internal(
                "global binding reached local closure capture",
            ));
        }
    };
    let mut final_index = None;
    let mut final_kind = None;
    for function_id in path {
        let function = &mut tree.functions[function_id];
        let descriptor_name = if retain_name
            || !function.eval_environments.is_empty()
            || matches!(
                original_kind,
                BindingKind::Lexical { .. }
                    | BindingKind::FunctionName { .. }
                    | BindingKind::EvalVariableObject
                    | BindingKind::ArgEvalVariableObject
                    | BindingKind::WithObject
                    | BindingKind::PrivateField { .. }
                    | BindingKind::PrivateMethod { .. }
                    | BindingKind::PrivateGetter { .. }
                    | BindingKind::PrivateSetter { .. }
                    | BindingKind::PrivateGetterSetter { .. }
            ) {
            ClosureVariableName::Constant(ensure_string_constant(function, name)?)
        } else {
            ClosureVariableName::None
        };
        let descriptor = ClosureVariable {
            source,
            name: descriptor_name,
            is_lexical: matches!(
                requested_kind,
                BindingKind::Lexical { .. }
                    | BindingKind::PrivateField { .. }
                    | BindingKind::PrivateMethod { .. }
                    | BindingKind::PrivateGetter { .. }
                    | BindingKind::PrivateSetter { .. }
                    | BindingKind::PrivateGetterSetter { .. }
            ),
            is_const: matches!(
                requested_kind,
                BindingKind::Lexical { is_const: true }
                    | BindingKind::FunctionName { is_const: true }
                    | BindingKind::PrivateField { .. }
                    | BindingKind::PrivateMethod { .. }
                    | BindingKind::PrivateGetter { .. }
                    | BindingKind::PrivateSetter { .. }
                    | BindingKind::PrivateGetterSetter { .. }
            ),
            kind: if module_import_view {
                ClosureVariableKind::ModuleImportView
            } else {
                closure_kind(requested_kind)
            },
        };
        let (index, actual_kind) =
            ensure_captured_closure_variable(function, descriptor, requested_kind)?;
        source = ClosureSource::ParentClosure(index);
        final_index = Some(index);
        final_kind = Some(actual_kind);
    }
    final_index
        .zip(final_kind)
        .ok_or_else(|| Error::internal("closure path did not cross a function boundary"))
}

/// Mirror QuickJS `get_closure_var`: the physical parent source, rather than
/// the requested flags, identifies a closure slot. The first request wins its
/// observable metadata. Later eval linking may still upgrade an omitted Rust
/// name because the source compiler always retained the corresponding atom.
fn ensure_captured_closure_variable(
    function: &mut FunctionIr,
    descriptor: ClosureVariable,
    requested_kind: BindingKind,
) -> Result<(u16, BindingKind), Error> {
    if let Some((index, candidate)) = function
        .closure_variables
        .iter_mut()
        .enumerate()
        .find(|(_, candidate)| candidate.source == descriptor.source)
    {
        let actual_kind = binding_kind_from_closure_descriptor(*candidate)?;
        let function_name_erasure = matches!(
            (actual_kind, requested_kind),
            (BindingKind::FunctionName { .. }, BindingKind::Normal)
                | (BindingKind::Normal, BindingKind::FunctionName { .. })
        );
        if !binding_kinds_compatible(actual_kind, requested_kind) && !function_name_erasure {
            return Err(Error::internal(
                "closure storage source has conflicting binding metadata",
            ));
        }
        match (candidate.name, descriptor.name) {
            (ClosureVariableName::None, ClosureVariableName::Constant(_)) => {
                candidate.name = descriptor.name;
            }
            (ClosureVariableName::Constant(left), ClosureVariableName::Constant(right))
                if left != right =>
            {
                return Err(Error::internal(
                    "closure storage source has conflicting binding names",
                ));
            }
            _ => {}
        }
        let index = u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
        return Ok((index, actual_kind));
    }
    let index = push_closure_variable(function, descriptor)?;
    Ok((index, requested_kind))
}

fn binding_kind_from_closure_descriptor(descriptor: ClosureVariable) -> Result<BindingKind, Error> {
    binding_kind_from_closure_flags(descriptor.kind, descriptor.is_lexical, descriptor.is_const)
        .ok_or_else(|| {
            Error::internal("captured closure descriptor has inconsistent binding metadata")
        })
}

pub(super) fn ensure_closure_variable(
    function: &mut FunctionIr,
    descriptor: ClosureVariable,
) -> Result<u16, Error> {
    if let Some((index, candidate)) = function
        .closure_variables
        .iter()
        .enumerate()
        .find(|(_, candidate)| same_closure_storage(candidate, &descriptor))
    {
        if *candidate != descriptor {
            return Err(Error::internal(
                "closure storage source has conflicting binding metadata",
            ));
        }
        return u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"));
    }
    push_closure_variable(function, descriptor)
}

pub(super) fn push_closure_variable(
    function: &mut FunctionIr,
    descriptor: ClosureVariable,
) -> Result<u16, Error> {
    if function.closure_variables.len() >= MAX_LOCAL_VARIABLES {
        return Err(Error::new(
            ErrorKind::JsInternal,
            "too many closure variables",
        ));
    }
    let index = u16::try_from(function.closure_variables.len())
        .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
    function.closure_variables.push(descriptor);
    Ok(index)
}

fn same_closure_storage(left: &ClosureVariable, right: &ClosureVariable) -> bool {
    match (left.source, right.source) {
        (ClosureSource::Global, ClosureSource::Global) => left.name == right.name,
        (ClosureSource::GlobalDeclaration, ClosureSource::GlobalDeclaration) => {
            left.name == right.name
        }
        (left, right) => left == right,
    }
}
