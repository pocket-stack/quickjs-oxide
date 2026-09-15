//! Binding identities, storage, and declaration records shared by parsing and resolution.

use super::scope::ScopeId;
use crate::engine::code::bytecode::EvalVariableSource;
use crate::engine::code::function::metadata::ClosureVariableKind;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::module;
use crate::engine::compiler::{EVAL_RET_LOCAL_NAME, FINALLY_EVAL_RET_LOCAL_NAME};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::engine::compiler) struct BindingId(pub(in crate::engine::compiler) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum SyntheticLocalKind {
    EvalCompletion,
    FinallySavedEvalCompletion,
}

impl SyntheticLocalKind {
    pub(in crate::engine::compiler) const fn name(self) -> &'static str {
        match self {
            Self::EvalCompletion => EVAL_RET_LOCAL_NAME,
            Self::FinallySavedEvalCompletion => FINALLY_EVAL_RET_LOCAL_NAME,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) struct SyntheticLocal {
    pub(in crate::engine::compiler) index: u16,
    pub(in crate::engine::compiler) kind: SyntheticLocalKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum BindingKind {
    Normal,
    Lexical {
        is_const: bool,
    },
    FunctionName {
        is_const: bool,
    },
    EvalVariableObject,
    ArgEvalVariableObject,
    WithObject,
    /// One class-private data-field identity. Staticness is declaration-only
    /// metadata used to diagnose conflicts; closure/runtime metadata needs only
    /// the authenticated `PrivateField` kind.
    PrivateField {
        is_static: bool,
    },
    /// One class-private synchronous method. Like private fields, staticness
    /// exists only while parsing the declaring class so same-spelling
    /// instance/static elements conflict before the typed closure metadata is
    /// published.
    PrivateMethod {
        is_static: bool,
    },
    /// Primary binding for a private getter. The cell contains the getter
    /// callable; a matching setter upgrades this binding to
    /// `PrivateGetterSetter` while retaining the same local identity.
    PrivateGetter {
        is_static: bool,
    },
    /// Private setter capability. The source-visible primary `#name` binding
    /// deliberately remains uninitialized for a setter-only declaration;
    /// the callable itself lives in the synthetic `#name<set>` binding with
    /// this same kind, matching pinned QuickJS.
    PrivateSetter {
        is_static: bool,
    },
    /// Primary binding for a paired private getter/setter. The getter lives in
    /// this cell and the setter lives in the sibling synthetic `<set>` cell.
    PrivateGetterSetter {
        is_static: bool,
    },
}

pub(in crate::engine::compiler) fn binding_kinds_compatible(
    left: BindingKind,
    right: BindingKind,
) -> bool {
    left == right
        || matches!(
            (left, right),
            (
                BindingKind::PrivateField { .. },
                BindingKind::PrivateField { .. }
            ) | (
                BindingKind::PrivateMethod { .. },
                BindingKind::PrivateMethod { .. }
            ) | (
                BindingKind::PrivateGetter { .. },
                BindingKind::PrivateGetter { .. }
            ) | (
                BindingKind::PrivateSetter { .. },
                BindingKind::PrivateSetter { .. }
            ) | (
                BindingKind::PrivateGetterSetter { .. },
                BindingKind::PrivateGetterSetter { .. }
            )
        )
}

pub(in crate::engine::compiler) const fn binding_kind_from_closure_flags(
    kind: ClosureVariableKind,
    is_lexical: bool,
    is_const: bool,
) -> Option<BindingKind> {
    match kind {
        ClosureVariableKind::Normal if is_lexical => Some(BindingKind::Lexical { is_const }),
        ClosureVariableKind::Normal if !is_const => Some(BindingKind::Normal),
        ClosureVariableKind::ModuleImportView if is_lexical && is_const => {
            Some(BindingKind::Lexical { is_const: true })
        }
        ClosureVariableKind::FunctionName if !is_lexical => {
            Some(BindingKind::FunctionName { is_const })
        }
        ClosureVariableKind::EvalVariableObject if !is_lexical && !is_const => {
            Some(BindingKind::EvalVariableObject)
        }
        ClosureVariableKind::ArgEvalVariableObject if !is_lexical && !is_const => {
            Some(BindingKind::ArgEvalVariableObject)
        }
        ClosureVariableKind::WithObject if !is_lexical && !is_const => {
            Some(BindingKind::WithObject)
        }
        ClosureVariableKind::PrivateField if is_lexical && is_const => {
            // Staticness is declaration-only conflict metadata. A closure or
            // direct-eval descriptor needs only the authenticated identity kind.
            Some(BindingKind::PrivateField { is_static: false })
        }
        ClosureVariableKind::PrivateMethod if is_lexical && is_const => {
            // Staticness is likewise declaration-only for method identities.
            Some(BindingKind::PrivateMethod { is_static: false })
        }
        ClosureVariableKind::PrivateGetter if is_lexical && is_const => {
            Some(BindingKind::PrivateGetter { is_static: false })
        }
        ClosureVariableKind::PrivateSetter if is_lexical && is_const => {
            Some(BindingKind::PrivateSetter { is_static: false })
        }
        ClosureVariableKind::PrivateGetterSetter if is_lexical && is_const => {
            Some(BindingKind::PrivateGetterSetter { is_static: false })
        }
        ClosureVariableKind::Normal
        | ClosureVariableKind::ModuleImportView
        | ClosureVariableKind::FunctionName
        | ClosureVariableKind::GlobalFunction
        | ClosureVariableKind::EvalVariableObject
        | ClosureVariableKind::ArgEvalVariableObject
        | ClosureVariableKind::WithObject
        | ClosureVariableKind::PrivateField
        | ClosureVariableKind::PrivateMethod
        | ClosureVariableKind::PrivateGetter
        | ClosureVariableKind::PrivateSetter
        | ClosureVariableKind::PrivateGetterSetter => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum BindingStorage {
    Argument(u16),
    Local(u16),
    /// Exact slot on a synthetic direct-eval root, supplied by the active
    /// caller environment rather than by a compiler-tree parent.
    External(u16),
    /// Exact binding in the synthetic module root. Its physical VarRef slot is
    /// assigned after the complete module declaration table is known.
    Module(module::ModuleBindingId),
    Global,
}

#[derive(Debug)]
pub(in crate::engine::compiler) struct IrBinding {
    pub(in crate::engine::compiler) name: String,
    pub(in crate::engine::compiler) storage_scope: ScopeId,
    /// Parse scope of the first declaration. QuickJS keeps this separately as
    /// the `scope_next` origin even for function-scoped `var` storage.
    pub(in crate::engine::compiler) declaration_scope: ScopeId,
    pub(in crate::engine::compiler) storage: BindingStorage,
    pub(in crate::engine::compiler) kind: BindingKind,
    /// Header-time marker for a block/switch FunctionDeclaration. The child
    /// constant is attached separately after its body parses successfully.
    pub(in crate::engine::compiler) is_scoped_function: bool,
    /// Generator declarations are lexical declarations and never receive the
    /// sloppy Annex B duplicate-function exception. Retain the grammar kind
    /// on the binding until the child constant is attached so a later
    /// declaration in the same block can apply QuickJS's early-error rule.
    pub(in crate::engine::compiler) is_scoped_generator: bool,
    /// Catch parameters behave as mutable lexicals for resolution/lifetime,
    /// but `var` of the same name is explicitly permitted and resolves its
    /// initializer through this nearer cell.
    pub(in crate::engine::compiler) is_catch_parameter: bool,
    pub(in crate::engine::compiler) declaration_span: Option<Span>,
}

#[derive(Debug)]
pub(in crate::engine::compiler) struct IrGlobalDeclaration {
    pub(in crate::engine::compiler) name: String,
    pub(in crate::engine::compiler) is_lexical: bool,
    pub(in crate::engine::compiler) is_const: bool,
    /// Child-function constant for a QuickJS
    /// `JS_VAR_GLOBAL_FUNCTION_DECL`. Ordinary `var` and lexical
    /// declarations have no declaration-time value.
    pub(in crate::engine::compiler) function_constant: Option<u32>,
    /// Exact root `GLOBAL_DECL` slot allocated during declaration seeding.
    pub(in crate::engine::compiler) closure_index: Option<u16>,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::engine::compiler) struct IrHoistedFunction {
    pub(in crate::engine::compiler) binding: BindingId,
    pub(in crate::engine::compiler) constant: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum EvalDeclarationTarget {
    /// A pre-existing caller cell which precedes the caller's `<var>` object.
    External { index: u16, kind: BindingKind },
    /// A novel name stored as a configurable property on the caller's `<var>`
    /// object. Repeated records deliberately redefine the property.
    Dynamic(EvalVariableSource),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum EvalDeclarationValue {
    Undefined,
    Function(u32),
}

#[derive(Clone, Debug)]
pub(in crate::engine::compiler) struct IrEvalDeclaration {
    pub(in crate::engine::compiler) name: String,
    pub(in crate::engine::compiler) target: EvalDeclarationTarget,
    pub(in crate::engine::compiler) value: EvalDeclarationValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum EvalDeclarationMode {
    Local,
    Global,
    Dynamic(EvalVariableSource),
}

#[derive(Clone, Copy, Debug)]
pub(in crate::engine::compiler) struct IrScopedFunction {
    pub(in crate::engine::compiler) binding: BindingId,
    pub(in crate::engine::compiler) constant: u32,
    pub(in crate::engine::compiler) annex_binding: Option<IrAnnexBinding>,
    pub(in crate::engine::compiler) authored_closure: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum IrAnnexBinding {
    Static(BindingId),
    Dynamic,
}

/// A sloppy labelled FunctionDeclaration in ProgramBody. QuickJS deliberately
/// skips the lexical declaration used by block/body Annex B forms, then writes
/// the authored closure through both the synthetic root var and the current
/// global environment at the declaration's source position.
#[derive(Clone, Copy, Debug)]
pub(in crate::engine::compiler) struct IrProgramAnnexFunction {
    pub(in crate::engine::compiler) binding: BindingId,
    pub(in crate::engine::compiler) constant: u32,
    pub(in crate::engine::compiler) authored_closure: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) struct ResolvedBinding {
    pub(in crate::engine::compiler) storage: BindingStorage,
    pub(in crate::engine::compiler) kind: BindingKind,
}
