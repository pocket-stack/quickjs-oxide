use super::{
    BytecodeFunctionKind, Error, IrOp, Parser, Punctuator, SpannedIrOp, TokenKind,
    insert_hoist_fragment, source_offset,
};
use crate::bytecode::{Instruction, IteratorCallKind};
use crate::lexer::Keyword;
use crate::value::{JsString, Value};

impl<'source> Parser<'source> {
    /// Place QuickJS's initial suspension after every parameter initializer
    /// and immediately before the authored body scope. Later entry-prologue
    /// insertion stays before this point, while body function hoists stay
    /// after it.
    pub(super) fn insert_generator_initial_yield(&mut self) -> Result<(), Error> {
        let function = self.current_ir();
        if function.execution_kind != BytecodeFunctionKind::Generator
            || function.in_function_body
            || function.stack_depth != 0
        {
            return Err(Error::internal(
                "generator initial yield was inserted in an invalid phase",
            ));
        }
        if function
            .ops
            .iter()
            .any(|operation| matches!(operation.op, IrOp::Bytecode(Instruction::InitialYield)))
        {
            return Err(Error::internal(
                "generator contains more than one initial yield",
            ));
        }
        let body_scope = function.body_scope;
        let body_entry = function
            .ops
            .iter()
            .position(
                |operation| matches!(operation.op, IrOp::EnterScope(scope) if scope == body_scope),
            )
            .ok_or_else(|| Error::internal("generator body has no scope entry"))?;
        insert_hoist_fragment(
            self.current_ir_mut(),
            body_entry,
            vec![SpannedIrOp {
                op: IrOp::Bytecode(Instruction::InitialYield),
                pc_site: None,
            }],
        )
    }

    /// Parse YieldExpression at AssignmentExpression precedence. `Yield` and
    /// `YieldStar` share the resumable ABI: the suspension consumes one value,
    /// then resumption produces the injected value plus an authenticated
    /// next/return discriminator. The true branch enters the ordinary return
    /// unwinder, so authored `finally` clauses remain observable.
    pub(super) fn parse_yield_expression(&mut self) -> Result<(), Error> {
        let yield_span = self.current().span;
        if !matches!(self.current().kind, TokenKind::Keyword(Keyword::Yield)) {
            return Err(Error::internal("yield parser did not start at yield"));
        }
        if self.current_ir().execution_kind != BytecodeFunctionKind::Generator {
            return Err(self.syntax_here("unexpected 'yield' keyword"));
        }
        if !self.current_ir().in_function_body {
            return Err(self.syntax_here("yield in default expression"));
        }

        self.advance()?;
        let line_terminator = self.current().line_terminator_before;
        let delegated = !line_terminator && self.is_punctuator(Punctuator::Multiply);
        if delegated {
            self.advance_expression_start()?;
            self.parse_assignment()?;
        } else if line_terminator || Self::yield_has_no_operand(&self.current().kind) {
            self.emit_instruction(Instruction::Undefined)?;
        } else {
            self.parse_assignment()?;
        }
        self.anonymous_function_definition = None;

        if delegated {
            self.lower_yield_star(yield_span)?;
            self.anonymous_function_definition = None;
            self.current_ir_mut().last_member_reference = None;
            self.current_ir_mut().last_identifier_reference = None;
            return Ok(());
        }

        self.emit_instruction_at(Instruction::Yield, source_offset(yield_span)?)?;

        // QuickJS's driver injects a false discriminator for `.next()` and a
        // true discriminator for `.return(value)`. `.throw(value)` resumes via
        // the VM's pending-exception path and never reaches this branch.
        let next = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        let resumed_depth = self.current_ir().stack_depth;
        self.emit_return_completion(yield_span)?;
        let next_target = self.current_ir().ops.len();
        self.patch_jump(next, next_target)?;
        self.current_ir_mut().stack_depth = resumed_depth;
        self.current_ir_mut().last_member_reference = None;
        self.current_ir_mut().last_identifier_reference = None;
        Ok(())
    }

    /// Lower the synchronous `yield*` protocol using the same retained
    /// `iterator, next, placeholder, value/result` record as QuickJS. Only the
    /// `YieldStar` suspension itself crosses the generator driver; every
    /// observable iterator operation remains explicit verified bytecode.
    fn lower_yield_star(&mut self, yield_span: super::Span) -> Result<(), Error> {
        let base_depth = self
            .current_ir()
            .stack_depth
            .checked_sub(1)
            .ok_or_else(|| Error::internal("yield* has no delegate operand"))?;
        let done = self.add_constant(super::IrConstant::Primitive(Value::String(
            JsString::from_static("done"),
        )))?;
        let value = self.add_constant(super::IrConstant::Primitive(Value::String(
            JsString::from_static("value"),
        )))?;

        self.emit_instruction(Instruction::IteratorStart)?;
        // IteratorStart already retains QuickJS's ordinary undefined
        // placeholder in lieu of a catch marker. This second undefined is the
        // initial argument passed to the delegate's cached `next` method.
        self.emit_instruction(Instruction::Undefined)?;
        let loop_target = self.current_ir().ops.len();
        self.emit_instruction(Instruction::IteratorNext)?;
        self.emit_instruction(Instruction::IteratorCheckObject)?;
        self.emit_instruction(Instruction::GetField2(done))?;
        let initial_done = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;

        let yield_target = self.current_ir().ops.len();
        self.emit_instruction_at(Instruction::YieldStar, source_offset(yield_span)?)?;
        self.emit_instruction(Instruction::Dup)?;
        let return_resume = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
        self.emit_instruction(Instruction::Drop)?;
        let next_loop = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        self.patch_jump(next_loop, loop_target)?;

        // `.return(value)` and `.throw(value)` both resume with a non-zero
        // discriminator. QuickJS uses 2 for throw; ordinary return uses 1.
        self.current_ir_mut().stack_depth = base_depth + 5;
        let return_target = self.current_ir().ops.len();
        self.patch_jump(return_resume, return_target)?;
        self.emit_instruction(Instruction::PushI32(2))?;
        self.emit_instruction(Instruction::StrictEq)?;
        let throw_resume = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;

        self.emit_instruction(Instruction::IteratorCall(IteratorCallKind::ReturnWithValue))?;
        let missing_return = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
        self.emit_instruction(Instruction::IteratorCheckObject)?;
        self.emit_instruction(Instruction::GetField2(done))?;
        let return_not_done = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        self.patch_jump(return_not_done, yield_target)?;
        self.emit_instruction(Instruction::GetField(value))?;
        let return_value = self.current_ir().ops.len();
        self.patch_jump(missing_return, return_value)?;
        for _ in 0..3 {
            self.emit_instruction(Instruction::Nip)?;
        }
        self.emit_return_completion(yield_span)?;

        self.current_ir_mut().stack_depth = base_depth + 4;
        let throw_target = self.current_ir().ops.len();
        self.patch_jump(throw_resume, throw_target)?;
        self.emit_instruction(Instruction::IteratorCall(IteratorCallKind::ThrowWithValue))?;
        let missing_throw = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
        self.emit_instruction(Instruction::IteratorCheckObject)?;
        self.emit_instruction(Instruction::GetField2(done))?;
        let throw_not_done = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        self.patch_jump(throw_not_done, yield_target)?;
        let throw_done = self.emit_instruction(Instruction::Goto(u32::MAX))?;

        let missing_throw_target = self.current_ir().ops.len();
        self.patch_jump(missing_throw, missing_throw_target)?;
        self.emit_instruction(Instruction::IteratorCall(
            IteratorCallKind::ReturnWithoutValue,
        ))?;
        let close_missing = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
        let iterator_throw = self.current_ir().ops.len();
        self.patch_jump(close_missing, iterator_throw)?;
        self.emit_instruction(Instruction::ThrowIteratorMissingThrow)?;

        let end_target = self.current_ir().ops.len();
        self.patch_jump(initial_done, end_target)?;
        self.patch_jump(throw_done, end_target)?;
        self.current_ir_mut().stack_depth = base_depth + 4;
        self.emit_instruction(Instruction::GetField(value))?;
        for _ in 0..3 {
            self.emit_instruction(Instruction::Nip)?;
        }
        Ok(())
    }

    fn yield_has_no_operand(kind: &TokenKind<'_>) -> bool {
        matches!(
            kind,
            TokenKind::Punctuator(
                Punctuator::Semicolon
                    | Punctuator::RightParen
                    | Punctuator::RightBracket
                    | Punctuator::RightBrace
                    | Punctuator::Comma
                    | Punctuator::Colon
            )
        )
    }
}
