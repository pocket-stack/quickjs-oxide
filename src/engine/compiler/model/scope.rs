//! Lexical scope identities and declaration-order indexes shared by compilation stages.

use super::bindings::BindingId;
use std::collections::HashMap;

/// Function-local lexical scope identity. QuickJS carries the corresponding
/// `scope_level` beside every unresolved scope opcode; keeping it typed avoids
/// accidentally resolving a child use from the parent's final parse scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::engine::compiler) struct ScopeId(pub(in crate::engine::compiler) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum ScopeKind {
    FunctionRoot,
    /// Declarative environment used while evaluating non-simple formal
    /// parameters. It is deliberately parentless: parameter initializers may
    /// see parameter cells and selected function pseudo-bindings, but never
    /// declarations from the authored body variable environment.
    Parameter,
    FunctionBody,
    ProgramBody,
    Block,
    /// QuickJS's class-body scope. It owns private-name declarations plus the
    /// compiler-only lexical cells used to retain computed field keys. It is
    /// separate from the class-name scope so heritage evaluation cannot observe
    /// names declared later in the body.
    ClassPrivate,
    If,
    For,
    Switch,
    Catch,
    With,
}

#[derive(Debug)]
pub(in crate::engine::compiler) struct IrScope {
    pub(in crate::engine::compiler) parent: Option<ScopeId>,
    pub(in crate::engine::compiler) kind: ScopeKind,
    /// This nested lexical scope is entered and left while formal-parameter
    /// initialization is still running.  It is distinct from the Parameter
    /// scope itself: its bindings may be captured by closures created in an
    /// initializer, but must never be mistaken for authored body lexicals.
    pub(in crate::engine::compiler) is_parameter_initializer: bool,
    pub(in crate::engine::compiler) bindings: Vec<BindingId>,
    /// Last binding in declaration order for each name. The ordered list remains
    /// authoritative for validation, lowering and observable declaration order.
    pub(in crate::engine::compiler) bindings_by_name: HashMap<String, BindingId>,
}

impl IrScope {
    pub(in crate::engine::compiler) fn binding_named(&self, name: &str) -> Option<BindingId> {
        self.bindings_by_name.get(name).copied()
    }
}
