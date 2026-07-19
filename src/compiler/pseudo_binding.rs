use super::{
    BindingKind, BindingStorage, FunctionId, FunctionKind, FunctionTree, IrOp, MAX_LOCAL_VARIABLES,
    ResolvedBinding, ScopeId, SpannedIrOp, find_or_create_own_binding, prepend_hoist_prefix,
    source_span,
};
use crate::bytecode::Instruction;
use crate::error::{Error, ErrorKind};
use crate::heap::EvalKind;
use crate::lexer::Span;

// QuickJS `JS_ATOM_this`, `JS_ATOM_new_target`, and `JS_ATOM_home_object`
// pseudo variables. Arrow functions never own these bindings: the resolver
// lazily creates the local in the nearest authenticated owner and relays it
// through ordinary closure slots. Source text cannot spell any identity as an
// IdentifierName.
pub(super) const THIS_LOCAL_NAME: &str = "<this>";
pub(super) const NEW_TARGET_LOCAL_NAME: &str = "<new.target>";
pub(super) const HOME_OBJECT_LOCAL_NAME: &str = "<home_object>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PseudoBinding {
    HomeObject,
    This,
    NewTarget,
}

impl PseudoBinding {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::HomeObject => HOME_OBJECT_LOCAL_NAME,
            Self::This => THIS_LOCAL_NAME,
            Self::NewTarget => NEW_TARGET_LOCAL_NAME,
        }
    }

    pub(super) fn from_name(name: &str) -> Option<Self> {
        match name {
            HOME_OBJECT_LOCAL_NAME => Some(Self::HomeObject),
            THIS_LOCAL_NAME => Some(Self::This),
            NEW_TARGET_LOCAL_NAME => Some(Self::NewTarget),
            _ => None,
        }
    }
}

pub(super) fn ensure_eval_visible_pseudo_bindings(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
) -> Result<(), Error> {
    let span = tree.functions[consuming_function].source.span;
    if !ensure_pseudo_binding_path(tree, consuming_function, PseudoBinding::This, span)? {
        return Err(Error::internal(
            "direct eval environment has no authenticated this binding",
        ));
    }
    if function_allows_new_target(tree, consuming_function)
        && !ensure_pseudo_binding_path(tree, consuming_function, PseudoBinding::NewTarget, span)?
    {
        return Err(Error::internal(
            "direct eval environment lost its new.target capability",
        ));
    }

    // Arrow functions do not own `arguments`. Force the lazy binding in the
    // nearest ordinary-function or method owner so the eval descriptor can
    // relay it through the same closure chain as an authored arrow reference.
    let mut arguments_owner = Some(consuming_function);
    while let Some(function_id) = arguments_owner {
        let function = &tree.functions[function_id];
        if matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method) {
            let span = function.source.span;
            find_or_create_own_binding(tree, function_id, ScopeId(0), "arguments", span)?;
            break;
        }
        if matches!(function.kind, FunctionKind::Eval(EvalKind::Direct))
            && function
                .binding_from_scope(function.var_scope, "arguments")
                .is_some()
        {
            break;
        }
        arguments_owner = function.parent.map(|parent| parent.function);
    }

    let mut cursor = Some(consuming_function);
    while let Some(function_id) = cursor {
        let (name, span, parent) = {
            let function = &tree.functions[function_id];
            (
                if function.private_name_binding {
                    function.function_name.clone()
                } else {
                    None
                },
                function.source.span,
                function.parent,
            )
        };
        if let Some(name) = name {
            find_or_create_own_binding(tree, function_id, ScopeId(0), &name, span)?;
        }
        cursor = parent.map(|parent| parent.function);
    }
    Ok(())
}

fn function_allows_new_target(tree: &FunctionTree, mut function_id: FunctionId) -> bool {
    loop {
        let function = &tree.functions[function_id];
        match function.kind {
            FunctionKind::Ordinary | FunctionKind::Method => return true,
            FunctionKind::Script | FunctionKind::Eval(EvalKind::Indirect) => return false,
            FunctionKind::Eval(EvalKind::Direct) => {
                return function
                    .binding_from_scope(function.var_scope, NEW_TARGET_LOCAL_NAME)
                    .is_some();
            }
            FunctionKind::Eval(EvalKind::None) => return false,
            FunctionKind::Arrow => {
                let Some(parent) = function.parent else {
                    return false;
                };
                function_id = parent.function;
            }
        }
    }
}

fn ensure_pseudo_binding_path(
    tree: &mut FunctionTree,
    mut function_id: FunctionId,
    pseudo: PseudoBinding,
    span: Span,
) -> Result<bool, Error> {
    loop {
        if find_or_create_own_pseudo_binding(tree, function_id, pseudo, span)?.is_some() {
            return Ok(true);
        }
        let Some(parent) = tree.functions[function_id].parent else {
            return Ok(false);
        };
        function_id = parent.function;
    }
}

/// Initialize QuickJS's lazily selected `home_object`, `new.target`, and
/// `this` pseudo locals before authored body code can publish or invoke
/// descendant closures. The order mirrors QuickJS `resolve_labels`.
pub(super) fn install_pseudo_binding_prologues(tree: &mut FunctionTree) -> Result<(), Error> {
    for function in &mut tree.functions {
        let mut prefix = Vec::with_capacity(
            usize::from(function.home_object_local.is_some()) * 2
                + usize::from(function.new_target_local.is_some()) * 2
                + usize::from(function.this_local.is_some()) * 2,
        );
        if let Some(local) = function.home_object_local {
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PushHomeObject),
                pc_site: None,
            });
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PutLocal(local)),
                pc_site: None,
            });
        }
        if let Some(local) = function.new_target_local {
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PushNewTarget),
                pc_site: None,
            });
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PutLocal(local)),
                pc_site: None,
            });
        }
        if let Some(local) = function.this_local {
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PushThis),
                pc_site: None,
            });
            prefix.push(SpannedIrOp {
                op: IrOp::Bytecode(Instruction::PutLocal(local)),
                pc_site: None,
            });
        }
        if !prefix.is_empty() {
            prepend_hoist_prefix(function, prefix)?;
        }
    }
    Ok(())
}

pub(super) const fn function_owns_pseudo_binding(
    kind: FunctionKind,
    pseudo: PseudoBinding,
) -> bool {
    match (kind, pseudo) {
        (FunctionKind::Method, PseudoBinding::HomeObject) => true,
        (
            FunctionKind::Script
            | FunctionKind::Ordinary
            | FunctionKind::Method
            | FunctionKind::Eval(EvalKind::Indirect),
            PseudoBinding::This,
        ) => true,
        (FunctionKind::Ordinary | FunctionKind::Method, PseudoBinding::NewTarget) => true,
        (
            FunctionKind::Script
            | FunctionKind::Ordinary
            | FunctionKind::Arrow
            | FunctionKind::Eval(_),
            PseudoBinding::HomeObject,
        ) => false,
        (
            FunctionKind::Arrow
            | FunctionKind::Eval(EvalKind::Direct)
            | FunctionKind::Eval(EvalKind::None),
            _,
        )
        | (
            FunctionKind::Script | FunctionKind::Eval(EvalKind::Indirect),
            PseudoBinding::NewTarget,
        ) => false,
    }
}

/// QuickJS `resolve_pseudo_var`: materialize a hidden frame local only in a
/// function which owns the corresponding binding. Arrow functions and direct
/// eval roots instead continue through their parent/imported closure chain.
pub(super) fn find_or_create_own_pseudo_binding(
    tree: &mut FunctionTree,
    function_id: FunctionId,
    pseudo: PseudoBinding,
    span: Span,
) -> Result<Option<ResolvedBinding>, Error> {
    let name = pseudo.name();
    let function = tree
        .functions
        .get(function_id)
        .ok_or_else(|| Error::internal("pseudo-binding owner is out of bounds"))?;
    if let Some(binding) = function.binding_from_scope(function.var_scope, name) {
        if binding.kind != BindingKind::Normal {
            return Err(Error::internal(
                "pseudo binding has non-ordinary binding metadata",
            ));
        }
        return Ok(Some(binding));
    }
    if !function_owns_pseudo_binding(function.kind, pseudo) {
        return Ok(None);
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
    let slot = match pseudo {
        PseudoBinding::HomeObject => {
            function.needs_home_object = true;
            &mut function.home_object_local
        }
        PseudoBinding::This => &mut function.this_local,
        PseudoBinding::NewTarget => &mut function.new_target_local,
    };
    if slot.replace(index).is_some() {
        return Err(Error::internal(
            "pseudo local metadata was allocated more than once",
        ));
    }
    function.locals.push(name.to_owned());
    function.add_binding(
        function.var_scope,
        function.var_scope,
        name.to_owned(),
        BindingStorage::Local(index),
        BindingKind::Normal,
        None,
    );
    Ok(Some(ResolvedBinding {
        storage: BindingStorage::Local(index),
        kind: BindingKind::Normal,
    }))
}
