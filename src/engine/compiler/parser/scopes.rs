//! Parser scope entry, exit and closure emission.

use crate::engine::api::error::Error;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::SpannedIrOp;
use crate::engine::compiler::model::scope::IrScope;
use crate::engine::compiler::model::scope::ScopeId;
use crate::engine::compiler::model::scope::ScopeKind;
use crate::engine::compiler::parser::context::Parser;
use std::collections::HashMap;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn push_scope(&mut self, kind: ScopeKind) -> ScopeId {
        let function = self.current_ir_mut();
        let parent = function.context.current_scope;
        let is_parameter_initializer = function.scopes[parent.0].is_parameter_initializer
            || (function.pattern_parameter_initialization && parent == function.ir.var_scope);
        let scope = ScopeId(function.scopes.len());
        function.scopes.push(IrScope {
            parent: Some(parent),
            kind,
            is_parameter_initializer,
            bindings: Vec::new(),
            bindings_by_name: HashMap::new(),
        });
        function.ops.push(SpannedIrOp {
            op: IrOp::EnterScope(scope),
            pc_site: None,
        });
        function.context.current_scope = scope;
        scope
    }

    pub(in crate::engine::compiler) fn pop_scope(
        &mut self,
        expected: ScopeId,
    ) -> Result<(), Error> {
        let function = self.current_ir_mut();
        if function.context.current_scope != expected {
            return Err(Error::internal("parser scope stack is unbalanced"));
        }
        function.ops.push(SpannedIrOp {
            op: IrOp::LeaveScope(expected),
            pc_site: None,
        });
        function.context.current_scope = function.scopes[expected.0]
            .parent
            .ok_or_else(|| Error::internal("cannot pop a function root scope"))?;
        Ok(())
    }

    /// Emit the runtime lexical exits which QuickJS's `close_scopes` inserts
    /// on an abrupt break/continue edge. Parser scope state is intentionally
    /// unchanged because parsing continues along the unreachable linear path.
    pub(in crate::engine::compiler) fn emit_scope_closures(
        &mut self,
        mut scope: ScopeId,
        stop: ScopeId,
    ) -> Result<(), Error> {
        while scope != stop {
            let parent = self
                .current_ir()
                .scopes
                .get(scope.0)
                .and_then(|scope| scope.parent)
                .ok_or_else(|| Error::internal("abrupt scope target is not an ancestor"))?;
            self.current_ir_mut().ops.push(SpannedIrOp {
                op: IrOp::LeaveScope(scope),
                pc_site: None,
            });
            scope = parent;
        }
        Ok(())
    }
}
