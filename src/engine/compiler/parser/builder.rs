//! Own the single IR allocation during parsing and consume temporary state at finish.
use crate::engine::compiler::model::ir::function::FunctionIr;
use crate::engine::compiler::model::ir::function::FunctionIrOptions;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::model::ir::function::FunctionSourceInfo;
use crate::engine::compiler::model::ir::function::ParentLink;

use super::context::FunctionParseContext;
use crate::engine::api::error::Error;

use std::ops::{Deref, DerefMut};

#[derive(Debug)]
pub(in crate::engine::compiler) struct FunctionBuilder {
    pub(in crate::engine::compiler) ir: FunctionIr,
    pub(in crate::engine::compiler) context: FunctionParseContext,
}

impl FunctionBuilder {
    pub(in crate::engine::compiler) fn new(
        parent: Option<ParentLink>,
        kind: FunctionKind,
        source: FunctionSourceInfo,
        options: FunctionIrOptions,
    ) -> Result<Self, Error> {
        let ir = FunctionIr::new(parent, kind, source, options)?;
        let context = FunctionParseContext::new(ir.body_scope);
        Ok(Self { ir, context })
    }

    /// Move the existing operations and binding arrays; no IR is cloned.
    /// Unclosed parser scopes/controls must never reach name resolution.
    pub(in crate::engine::compiler) fn finish(mut self) -> Result<FunctionIr, Error> {
        if self.context.current_scope != self.ir.body_scope {
            return Err(Error::internal("function scope roots are malformed"));
        }
        if !self.context.break_controls.is_empty() {
            return Err(Error::internal("function parser controls are not closed"));
        }
        self.ir.body_parsed = self.context.in_function_body;
        Ok(self.ir)
    }
}

// Construction algorithms share the same IR, while parser state is always
// explicit through `context`. These borrows cannot outlive the builder.
impl Deref for FunctionBuilder {
    type Target = FunctionIr;
    fn deref(&self) -> &FunctionIr {
        &self.ir
    }
}
impl DerefMut for FunctionBuilder {
    fn deref_mut(&mut self) -> &mut FunctionIr {
        &mut self.ir
    }
}

use crate::engine::api::error::ErrorKind;
use crate::engine::code::bytecode::Instruction;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::model::ir::IdentifierAccess;
use crate::engine::compiler::model::ir::IdentifierReferenceAccess;
use crate::engine::compiler::model::ir::IrConstant;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::SpannedIrOp;
use crate::engine::compiler::model::scope::ScopeId;
use crate::engine::compiler::parser::context::AnonymousFunctionDefinition;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::diagnostics::source_offset;
use crate::engine::compiler::relocation::insert_hoist_fragment;
use crate::engine::value::JsString;
use crate::engine::value::PrimitiveValue as Value;
use crate::source::SourceOffset;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn emit_value(&mut self, value: Value) -> Result<(), Error> {
        self.emit_value_with_site(value, None)
    }

    pub(in crate::engine::compiler) fn emit_atom_string(
        &mut self,
        value: JsString,
    ) -> Result<(), Error> {
        let index = self.add_constant(IrConstant::AtomString(value))?;
        self.emit(IrOp::PushConstant(index)).map(|_| ())
    }

    pub(in crate::engine::compiler) fn emit_value_with_site(
        &mut self,
        value: Value,
        site: Option<SourceOffset>,
    ) -> Result<(), Error> {
        if let Value::Int(value) = value {
            return self
                .emit_with_site(IrOp::Bytecode(Instruction::PushI32(value)), site)
                .map(|_| ());
        }
        let index = self.add_constant(IrConstant::Primitive(value))?;
        self.emit_with_site(IrOp::PushConstant(index), site)
            .map(|_| ())
    }

    pub(in crate::engine::compiler) fn add_constant(
        &mut self,
        constant: IrConstant,
    ) -> Result<u32, Error> {
        self.current_ir_mut().append_constant(constant)
    }

    pub(in crate::engine::compiler) fn emit_instruction(
        &mut self,
        instruction: Instruction,
    ) -> Result<usize, Error> {
        self.emit(IrOp::Bytecode(instruction))
    }

    pub(in crate::engine::compiler) fn take_anonymous_function_definition(
        &mut self,
    ) -> Option<AnonymousFunctionDefinition> {
        self.anonymous_function_definition.take()
    }

    pub(in crate::engine::compiler) fn emit_anonymous_set_name(
        &mut self,
        definition: AnonymousFunctionDefinition,
        instruction: Instruction,
    ) -> Result<usize, Error> {
        if !matches!(
            instruction,
            Instruction::SetName(_) | Instruction::SetNameComputed
        ) {
            return Err(Error::internal(
                "anonymous definition received a non-naming instruction",
            ));
        }
        match definition {
            AnonymousFunctionDefinition::Function => self.emit_instruction(instruction),
            AnonymousFunctionDefinition::Class {
                owner,
                static_initializer_start: Some(at),
            } => {
                if owner != self.current_function {
                    return Err(Error::internal(
                        "anonymous class static initializer changed defining function",
                    ));
                }
                insert_hoist_fragment(
                    self.current_ir_mut(),
                    at,
                    vec![SpannedIrOp {
                        op: IrOp::Bytecode(instruction),
                        pc_site: None,
                    }],
                )?;
                Ok(at)
            }
            AnonymousFunctionDefinition::Class {
                static_initializer_start: None,
                ..
            } => self.emit_instruction(instruction),
        }
    }

    pub(in crate::engine::compiler) fn emit_instruction_at(
        &mut self,
        instruction: Instruction,
        site: SourceOffset,
    ) -> Result<usize, Error> {
        self.emit_at(IrOp::Bytecode(instruction), site)
    }

    pub(in crate::engine::compiler) fn emit_identifier(
        &mut self,
        name: String,
        span: Span,
        access: IdentifierAccess,
    ) -> Result<usize, Error> {
        self.emit_identifier_at(name, span, access, source_offset(span)?)
    }

    pub(in crate::engine::compiler) fn emit_identifier_at(
        &mut self,
        name: String,
        span: Span,
        access: IdentifierAccess,
        pc_site: SourceOffset,
    ) -> Result<usize, Error> {
        let scope = self.current_ir().context.current_scope;
        self.emit_at(
            IrOp::Identifier {
                name,
                span,
                scope,
                access,
            },
            pc_site,
        )
    }

    pub(in crate::engine::compiler) fn emit_identifier_inherited(
        &mut self,
        name: String,
        span: Span,
        scope: ScopeId,
        access: IdentifierAccess,
    ) -> Result<usize, Error> {
        self.emit(IrOp::Identifier {
            name,
            span,
            scope,
            access,
        })
    }

    pub(in crate::engine::compiler) fn emit_identifier_reference_inherited(
        &mut self,
        name: String,
        span: Span,
        scope: ScopeId,
        access: IdentifierReferenceAccess,
    ) -> Result<usize, Error> {
        self.emit(IrOp::IdentifierReference {
            name,
            span,
            scope,
            access,
        })
    }

    /// Materialize an `emit_source_pos` which appeared before an expression's
    /// first real opcode. If the RHS emitted its own marker before that same
    /// opcode, QuickJS's later `OP_line_num` wins and the pending marker is
    /// intentionally discarded.
    pub(in crate::engine::compiler) fn inherit_source_marker_at(
        &mut self,
        first_operation: usize,
        marker: SourceOffset,
    ) -> Result<(), Error> {
        let operation = self
            .current_ir_mut()
            .ops
            .get_mut(first_operation)
            .ok_or_else(|| Error::internal("assignment RHS emitted no operation"))?;
        if operation.pc_site.is_none() {
            operation.pc_site = Some(marker);
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn emit(&mut self, operation: IrOp) -> Result<usize, Error> {
        self.emit_with_site(operation, None)
    }

    pub(in crate::engine::compiler) fn emit_at(
        &mut self,
        operation: IrOp,
        site: SourceOffset,
    ) -> Result<usize, Error> {
        self.emit_with_site(operation, Some(site))
    }

    pub(in crate::engine::compiler) fn emit_with_site(
        &mut self,
        operation: IrOp,
        pc_site: Option<SourceOffset>,
    ) -> Result<usize, Error> {
        let (popped, pushed) = operation.stack_effect();
        let function = self.current_ir_mut();
        function.context.last_member_reference = None;
        function.context.last_identifier_reference = None;
        function.context.last_optional_chain = None;
        function.context.stack_depth = function
            .context
            .stack_depth
            .checked_sub(popped)
            .ok_or_else(|| Error::internal("compiler produced a stack underflow"))?;
        function.context.stack_depth = function
            .context
            .stack_depth
            .checked_add(pushed)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        let index = function.ops.len();
        function.ops.push(SpannedIrOp {
            op: operation,
            pc_site,
        });
        Ok(index)
    }

    pub(in crate::engine::compiler) fn patch_jump(
        &mut self,
        instruction_index: usize,
        target: usize,
    ) -> Result<(), Error> {
        let target = u32::try_from(target)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?;
        let operation = self
            .current_ir_mut()
            .ops
            .get_mut(instruction_index)
            .ok_or_else(|| Error::internal("missing jump instruction"))?;
        match &mut operation.op {
            IrOp::Bytecode(
                Instruction::IfFalse(value)
                | Instruction::IfTrue(value)
                | Instruction::Goto(value)
                | Instruction::Catch(value)
                | Instruction::Gosub(value),
            ) => {
                *value = target;
                Ok(())
            }
            _ => Err(Error::internal("attempted to patch a non-jump instruction")),
        }
    }

    pub(in crate::engine::compiler) fn current_ir(&self) -> &FunctionBuilder {
        &self.functions[self.current_function]
    }

    pub(in crate::engine::compiler) fn current_ir_mut(&mut self) -> &mut FunctionBuilder {
        &mut self.functions[self.current_function]
    }
}

#[cfg(test)]
mod tests {
    use super::{FunctionBuilder, FunctionParseContext};
    use crate::engine::compiler::model::scope::ScopeId;
    use crate::engine::compiler::parser::context::{BreakControlContext, BreakControlKind, Parser};
    use crate::engine::value::JsString;

    fn parsed_builder() -> FunctionBuilder {
        let mut tree = Parser::parse("0;", JsString::from_static("<finish-contract>")).unwrap();
        let ir = tree.functions.remove(0);
        let context = FunctionParseContext::new(ir.body_scope);
        FunctionBuilder { ir, context }
    }

    #[test]
    fn unfinished_scope_cannot_enter_resolution() {
        let mut builder = parsed_builder();
        builder.context.current_scope = ScopeId(0);
        assert_eq!(
            builder.finish().unwrap_err().message(),
            "function scope roots are malformed"
        );
    }

    #[test]
    fn pending_abrupt_targets_cannot_enter_resolution() {
        let mut builder = parsed_builder();
        builder.context.break_controls.push(BreakControlContext {
            kind: BreakControlKind::Loop,
            label_name: None,
            scope: builder.ir.body_scope,
            entry_depth: 0,
            drop_count: 0,
            break_jumps: vec![0],
            continue_jumps: Vec::new(),
            finally_gosubs: Vec::new(),
        });
        assert_eq!(
            builder.finish().unwrap_err().message(),
            "function parser controls are not closed"
        );
    }
}
