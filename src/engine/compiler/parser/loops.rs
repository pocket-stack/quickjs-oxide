//! For, while and do iteration grammar.

use crate::engine::api::error::Error;
use crate::engine::api::error::ErrorKind;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::FunctionKind as BytecodeFunctionKind;
use crate::engine::compiler::lexer::Keyword;
use crate::engine::compiler::lexer::Punctuator;
use crate::engine::compiler::lexer::TokenKind;
use crate::engine::compiler::model::ir::IdentifierAccess;
use crate::engine::compiler::model::ir::IdentifierReferenceAccess;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::PrivateFieldAccess;
use crate::engine::compiler::model::ir::SpannedIrOp;
use crate::engine::compiler::model::scope::ScopeId;
use crate::engine::compiler::model::scope::ScopeKind;
use crate::engine::compiler::parser::context::BreakControlKind;
use crate::engine::compiler::parser::context::ForAssignmentDeclaration;
use crate::engine::compiler::parser::context::ForAssignmentTargetInfo;
use crate::engine::compiler::parser::context::ForIterationKind;
use crate::engine::compiler::parser::context::IdentifierReference;
use crate::engine::compiler::parser::context::InMode;
use crate::engine::compiler::parser::context::MemberReference;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::context::StatementCompletion;
use crate::engine::compiler::parser::context::StatementPosition;
use crate::engine::compiler::parser::diagnostics::IdentifierContext;
use crate::engine::compiler::parser::diagnostics::source_offset;
use crate::engine::compiler::parser::diagnostics::source_span;
use crate::engine::compiler::parser::diagnostics::validate_identifier_reservation;
use crate::engine::compiler::relocation::relocate_ir_fragment;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn parse_while_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: Option<String>,
    ) -> Result<(), Error> {
        let entry_depth = self.current_ir().context.stack_depth;
        self.push_loop_control(entry_depth, label_name);
        self.advance()?;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }

        let condition_target = self.current_ir().ops.len();
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;
        let false_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        self.require_stack_depth(entry_depth, "while condition")?;

        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.require_stack_depth(entry_depth, "while body")?;
        self.emit_instruction(Instruction::Goto(
            u32::try_from(condition_target)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
        ))?;

        let break_target = self.current_ir().ops.len();
        let control = self.pop_break_control()?;
        self.patch_jump(false_jump, break_target)?;
        for jump in control.continue_jumps {
            self.patch_jump(jump, condition_target)?;
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.finish_control_statement();
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_do_while_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: Option<String>,
    ) -> Result<(), Error> {
        let entry_depth = self.current_ir().context.stack_depth;
        self.push_loop_control(entry_depth, label_name);
        self.advance()?;

        // QuickJS targets the reset itself, so every entered iteration starts
        // with an undefined eval completion. A continue instead targets the
        // condition below and does not repeat this reset prematurely.
        let body_target = self.current_ir().ops.len();
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.require_stack_depth(entry_depth, "do-while body")?;

        let condition_target = self.current_ir().ops.len();
        if !matches!(self.current().kind, TokenKind::Keyword(Keyword::While)) {
            // `js_parse_expect(TOK_WHILE)` formats the non-ASCII token through
            // `%c`; preserve the pinned release's observable replacement-char
            // diagnostic, including its missing closing quote.
            return Err(self.syntax_here("expecting '�"));
        }
        self.advance()?;
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;
        // Unlike an ordinary statement terminator, the trailing semicolon is
        // unconditionally optional, even before a same-line expression.
        self.consume_punctuator(Punctuator::Semicolon)?;
        self.emit_instruction(Instruction::IfTrue(
            u32::try_from(body_target)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
        ))?;
        self.require_stack_depth(entry_depth, "do-while condition")?;

        let break_target = self.current_ir().ops.len();
        let control = self.pop_break_control()?;
        for jump in control.continue_jumps {
            self.patch_jump(jump, condition_target)?;
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.finish_control_statement();
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_for_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: Option<String>,
    ) -> Result<(), Error> {
        let entry_depth = self.current_ir().context.stack_depth;
        self.advance()?;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        // QuickJS recognizes this contextual form only when the lexer has
        // classified `await` as the real keyword. In an ordinary function or
        // script the same source spelling remains an Identifier, so the
        // ordinary `for (` expectation reports the syntax error instead.
        let is_for_await = matches!(self.current().kind, TokenKind::Keyword(Keyword::Await));
        if is_for_await {
            if !matches!(
                self.current_ir().execution_kind,
                BytecodeFunctionKind::Async | BytecodeFunctionKind::AsyncGenerator
            ) {
                return Err(self.syntax_here("for await is only valid in asynchronous functions"));
            }
            self.advance()?;
        }
        // `for await` always enters the for-in/of parser. A semicolon or `in`
        // is rejected there rather than being reinterpreted as a classic for
        // head.
        let classic_head = !is_for_await && self.for_head_has_top_level_semicolon();
        self.expect_punctuator(Punctuator::LeftParen)?;
        let outer_scope = self.current_ir().context.current_scope;
        let scope = self.push_scope(ScopeKind::For);

        if !classic_head {
            return self.parse_for_in_of_statement(
                completion,
                label_name,
                entry_depth,
                outer_scope,
                scope,
                is_for_await,
            );
        }

        // QuickJS parses the classic initializer with PF_IN_ACCEPTED clear.
        // Keep that mode explicit even while the AllowIn operator itself
        // remains a later runtime slice.
        if !self.is_punctuator(Punctuator::Semicolon) {
            if self.lexical_declaration_ahead(true)? {
                self.parse_lexical_declarations_with_in(InMode::Disallow)?;
            } else if matches!(self.current().kind, TokenKind::Keyword(Keyword::Var)) {
                self.advance()?;
                self.parse_var_declarations_with_in(InMode::Disallow)?;
            } else {
                self.parse_expression_no_in()?;
                self.emit_instruction(Instruction::Drop)?;
            }
            self.require_stack_depth(entry_depth, "for initializer")?;
            // Detach any initializer capture before the first test while the
            // initialized value remains in the local slot for the iteration.
            self.emit_scope_closures(scope, outer_scope)?;
        }
        self.expect_punctuator(Punctuator::Semicolon)?;

        self.push_loop_control(entry_depth, label_name);
        let test_target = if self.is_punctuator(Punctuator::Semicolon) {
            None
        } else {
            let target = self.current_ir().ops.len();
            self.parse_expression()?;
            let false_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
            self.require_stack_depth(entry_depth, "for test")?;
            Some((target, false_jump))
        };
        self.expect_punctuator(Punctuator::Semicolon)?;

        let mut body_skip = None;
        let mut moved_update = None;
        let unmoved_continue_target = if self.is_punctuator(Punctuator::RightParen) {
            test_target.map(|(target, _)| target)
        } else {
            body_skip = Some(self.emit_instruction(Instruction::Goto(u32::MAX))?);
            let update_start = self.current_ir().ops.len();
            self.parse_expression()?;
            self.emit_instruction(Instruction::Drop)?;
            self.require_stack_depth(entry_depth, "for update")?;
            if let Some((test_target, _)) = test_target {
                self.emit_instruction(Instruction::Goto(
                    u32::try_from(test_target)
                        .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
                ))?;
            }
            if test_target.is_some() {
                // QuickJS's OPTIMIZE path moves the complete update chunk
                // after the body. Preserve empty Nop slots at its source
                // position and relocate only fragment-internal targets; the
                // backedge to the earlier test remains external.
                let update_end = self.current_ir().ops.len();
                let fragment = self.current_ir_mut().ops.split_off(update_start);
                for _ in 0..fragment.len() {
                    self.emit_instruction(Instruction::Nop)?;
                }
                moved_update = Some((update_start, update_end, fragment));
                None
            } else {
                Some(update_start)
            }
        };
        self.expect_punctuator(Punctuator::RightParen)?;

        let body_target = self.current_ir().ops.len();
        if let Some(body_skip) = body_skip {
            self.patch_jump(body_skip, body_target)?;
        }
        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.require_stack_depth(entry_depth, "for body")?;
        // Normal fallthrough closes the current head-binding cell before the
        // update creates the next iteration's cell. A `continue` targets the
        // update/test below and intentionally skips this close, preserving the
        // pinned release's observable `XXX: check continue` behavior.
        self.emit_scope_closures(scope, outer_scope)?;
        let continue_target = if let Some((old_start, old_end, mut fragment)) = moved_update {
            let target = self.current_ir().ops.len();
            relocate_ir_fragment(&mut fragment, old_start..old_end, target)?;
            self.current_ir_mut().ops.extend(fragment);
            target
        } else {
            let target = unmoved_continue_target.unwrap_or(body_target);
            self.emit_instruction(Instruction::Goto(
                u32::try_from(target)
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
            ))?;
            target
        };

        let break_target = self.current_ir().ops.len();
        let control = self.pop_break_control()?;
        if let Some((_, false_jump)) = test_target {
            self.patch_jump(false_jump, break_target)?;
        }
        for jump in control.continue_jumps {
            self.patch_jump(jump, continue_target)?;
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.finish_control_statement();
        self.pop_scope(scope)?;
        Ok(())
    }

    /// Lower QuickJS `js_parse_for_in_of`. The assignment fragment is emitted
    /// before the enumerated expression and skipped on first entry, just as
    /// upstream does; each `done == false` edge jumps back with the yielded
    /// value above the retained for-in object or three-slot iterator record.
    pub(in crate::engine::compiler) fn parse_for_in_of_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: Option<String>,
        entry_depth: usize,
        outer_scope: ScopeId,
        scope: ScopeId,
        is_for_await: bool,
    ) -> Result<(), Error> {
        let iteration_hint = self
            .for_iteration_kind_ahead()
            .ok_or_else(|| self.syntax_here("expected 'of' or 'in' in for control expression"))?;
        if is_for_await && iteration_hint != ForIterationKind::Of {
            return Err(self.syntax_here("'for await' loop should be used with 'of'"));
        }
        let retained_slots = match iteration_hint {
            ForIterationKind::In => 1,
            ForIterationKind::Of => 3,
        };

        let expression_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let assignment_target = self.current_ir().ops.len();

        // ForInNext supplies `enum, value`; ForOfNext supplies the retained
        // three-slot iterator record plus `value` on this edge.
        self.current_ir_mut().context.stack_depth = entry_depth
            .checked_add(retained_slots + 1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        if iteration_hint == ForIterationKind::Of {
            self.push_for_of_assignment_fragment_control(entry_depth)?;
        }
        let target = self.parse_for_iteration_assignment_target(iteration_hint, is_for_await)?;
        self.require_stack_depth(entry_depth + retained_slots, "for-in/of assignment target")?;
        if iteration_hint == ForIterationKind::Of {
            self.pop_for_of_assignment_fragment_control(entry_depth)?;
        }
        let body_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;

        let expression_target = self.current_ir().ops.len();
        self.patch_jump(expression_jump, expression_target)?;
        self.current_ir_mut().context.stack_depth = entry_depth;

        let has_initializer = if self.consume_punctuator(Punctuator::Equal)? {
            match iteration_hint {
                ForIterationKind::In => {
                    self.with_in_mode(InMode::Disallow, Self::parse_assignment)?;
                }
                ForIterationKind::Of => self.parse_assignment_allow_in()?,
            }
            if iteration_hint == ForIterationKind::In
                && target.declaration == ForAssignmentDeclaration::Var
                && !self.current_ir().strict
                && !target.is_destructuring
            {
                let initializer = target
                    .var_initializer
                    .as_ref()
                    .ok_or_else(|| Error::internal("for-in var initializer lost its binding"))?;
                self.emit_identifier_inherited(
                    initializer.name.clone(),
                    initializer.span,
                    initializer.scope,
                    IdentifierAccess::Put,
                )?;
            } else {
                self.emit_instruction(Instruction::Drop)?;
            }
            true
        } else {
            false
        };

        let iteration_kind = if self.is_for_of_keyword() {
            ForIterationKind::Of
        } else if matches!(self.current().kind, TokenKind::Keyword(Keyword::In)) {
            ForIterationKind::In
        } else {
            return Err(self.syntax_here("expected 'of' or 'in' in for control expression"));
        };
        if iteration_kind != iteration_hint {
            return Err(Error::internal("for-in/of delimiter probe drifted"));
        }
        if is_for_await && iteration_kind != ForIterationKind::Of {
            return Err(self.syntax_here("'for await' loop should be used with 'of'"));
        }
        if has_initializer
            && (iteration_kind == ForIterationKind::Of
                || target.declaration != ForAssignmentDeclaration::Var
                || self.current_ir().strict
                || target.is_destructuring)
        {
            return Err(self.syntax_here(format!(
                "a declaration in the head of a for-{} loop can't have an initializer",
                if iteration_kind == ForIterationKind::Of {
                    "of"
                } else {
                    "in"
                }
            )));
        }

        // After contextual `of`, a slash starts the right-hand side's RegExp
        // lexical goal. The literal itself remains an explicit frontier, but
        // it must not drift into the division-token diagnostic.
        self.advance_expression_start()?;
        if iteration_kind == ForIterationKind::Of {
            // For-of consumes exactly one AssignmentExpression.
            self.parse_assignment_allow_in()?;
        } else {
            // QuickJS deliberately accepts a full comma Expression for-in.
            self.parse_expression()?;
        }
        self.emit_scope_closures(scope, outer_scope)?;
        self.emit_instruction(match (iteration_kind, is_for_await) {
            (ForIterationKind::In, false) => Instruction::ForInStart,
            (ForIterationKind::Of, false) => Instruction::ForOfStart,
            (ForIterationKind::Of, true) => Instruction::ForAwaitOfStart,
            (ForIterationKind::In, true) => {
                return Err(Error::internal("for-await retained a for-in iterator"));
            }
        })?;
        let record_depth = entry_depth + retained_slots;
        self.require_stack_depth(record_depth, "for-in/of iterator start")?;
        let next_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        self.expect_punctuator(Punctuator::RightParen)?;

        let body_target = self.current_ir().ops.len();
        self.patch_jump(body_jump, body_target)?;
        self.current_ir_mut().context.stack_depth = record_depth;
        match iteration_kind {
            ForIterationKind::In => {
                self.push_for_in_control(entry_depth, label_name, outer_scope)?;
            }
            ForIterationKind::Of => {
                self.push_for_of_control(entry_depth, label_name, outer_scope)?;
            }
        }
        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.require_stack_depth(record_depth, "for-in/of body")?;
        self.emit_scope_closures(scope, outer_scope)?;

        let next_target = self.current_ir().ops.len();
        self.patch_jump(next_jump, next_target)?;
        match (iteration_kind, is_for_await) {
            (ForIterationKind::In, false) => {
                self.emit_instruction(Instruction::ForInNext)?;
            }
            (ForIterationKind::Of, false) => {
                self.emit_instruction(Instruction::ForOfNext(0))?;
            }
            (ForIterationKind::Of, true) => {
                self.emit_instruction(Instruction::ForAwaitOfNext)?;
                self.emit_instruction(Instruction::Await)?;
                self.emit_instruction(Instruction::IteratorGetValueDone)?;
            }
            (ForIterationKind::In, true) => {
                return Err(Error::internal("for-await advanced a for-in iterator"));
            }
        }
        let assignment_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        self.patch_jump(assignment_jump, assignment_target)?;

        // A completed enumeration contributes undefined above its retained
        // record. Natural exhaustion removes it and shares the break tail.
        self.emit_instruction(Instruction::Drop)?;
        let break_target = self.current_ir().ops.len();
        match iteration_kind {
            ForIterationKind::In => {
                self.emit_instruction(Instruction::Drop)?;
            }
            ForIterationKind::Of => {
                self.emit_instruction(Instruction::IteratorClose)?;
            }
        }
        self.require_stack_depth(entry_depth, "for-in/of close")?;

        let control = self.pop_break_control()?;
        let expected_control = match iteration_kind {
            ForIterationKind::In => (BreakControlKind::ForIn, 1),
            ForIterationKind::Of => (BreakControlKind::ForOf, 3),
        };
        if (control.kind, control.drop_count) != expected_control {
            return Err(Error::internal("for-in/of control stack is unbalanced"));
        }
        for jump in control.continue_jumps {
            self.patch_jump(jump, next_target)?;
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.finish_control_statement();
        self.pop_scope(scope)?;
        Ok(())
    }

    /// Parse and consume the yielded value for one supported for-in/of head.
    /// Declarations bind in the enumeration scope; ordinary references keep
    /// their base/key evaluation in the per-iteration assignment fragment.
    pub(in crate::engine::compiler) fn parse_for_iteration_assignment_target(
        &mut self,
        iteration_kind: ForIterationKind,
        is_for_await: bool,
    ) -> Result<ForAssignmentTargetInfo, Error> {
        if self.for_array_assignment_pattern_ahead(iteration_kind) {
            return self.parse_for_array_assignment_pattern(iteration_kind);
        }
        if self.for_object_assignment_pattern_ahead(iteration_kind) {
            return self.parse_for_object_assignment_pattern(iteration_kind);
        }

        if self.lexical_declaration_ahead(true)? {
            let is_const = matches!(self.current().kind, TokenKind::Keyword(Keyword::Const));
            self.advance()?;
            if self.is_punctuator(Punctuator::LeftBracket) {
                return self.parse_for_array_binding_pattern(
                    iteration_kind,
                    ForAssignmentDeclaration::Lexical,
                    is_const,
                );
            }
            if matches!(
                self.current().kind,
                TokenKind::Punctuator(Punctuator::LeftBrace)
            ) {
                return self.parse_for_object_binding_pattern(
                    iteration_kind,
                    ForAssignmentDeclaration::Lexical,
                    is_const,
                );
            }
            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind else {
                return Err(self.syntax_here("variable name expected"));
            };
            validate_identifier_reservation(
                &identifier,
                token.span,
                self.current_ir().strict,
                IdentifierContext::Variable,
            )?;
            if identifier.value == "let" {
                return Err(Error::syntax(
                    "'let' is not a valid lexical identifier",
                    source_span(token.span),
                ));
            }
            let name = identifier.value;
            let strict = self.current_ir().strict;
            self.advance()?;
            if strict && matches!(name.as_str(), "eval" | "arguments") {
                return Err(Error::syntax(
                    "invalid variable name in strict mode",
                    source_span(self.current().span),
                ));
            }
            self.register_lexical_binding(&name, token.span, self.current().span, is_const, false)?;
            self.emit_identifier_at(
                name,
                token.span,
                IdentifierAccess::Initialize,
                source_offset(token.span)?,
            )?;
            return Ok(ForAssignmentTargetInfo {
                declaration: ForAssignmentDeclaration::Lexical,
                var_initializer: None,
                is_destructuring: false,
            });
        }

        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Var)) {
            self.advance()?;
            if self.is_punctuator(Punctuator::LeftBracket) {
                return self.parse_for_array_binding_pattern(
                    iteration_kind,
                    ForAssignmentDeclaration::Var,
                    false,
                );
            }
            if matches!(
                self.current().kind,
                TokenKind::Punctuator(Punctuator::LeftBrace)
            ) {
                return self.parse_for_object_binding_pattern(
                    iteration_kind,
                    ForAssignmentDeclaration::Var,
                    false,
                );
            }
            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind else {
                return Err(self.syntax_here("variable name expected"));
            };
            validate_identifier_reservation(
                &identifier,
                token.span,
                self.current_ir().strict,
                IdentifierContext::Variable,
            )?;
            let strict = self.current_ir().strict;
            let name = identifier.value;
            self.advance()?;
            if strict && matches!(name.as_str(), "eval" | "arguments") {
                return Err(Error::syntax(
                    "invalid variable name in strict mode",
                    source_span(self.current().span),
                ));
            }
            self.register_var_binding(&name, token.span, self.current().span)?;
            let initializer = IdentifierReference {
                name: name.clone(),
                span: token.span,
                scope: self.current_ir().context.current_scope,
                object_environment: false,
            };
            self.emit_identifier_at(
                name,
                token.span,
                IdentifierAccess::Put,
                source_offset(token.span)?,
            )?;
            return Ok(ForAssignmentTargetInfo {
                declaration: ForAssignmentDeclaration::Var,
                var_initializer: Some(initializer),
                is_destructuring: false,
            });
        }

        let async_span = match &self.current().kind {
            TokenKind::Identifier(identifier)
                if identifier.value == "async" && !identifier.has_escape =>
            {
                Some(self.current().span)
            }
            _ => None,
        };
        // Mirror QuickJS's pre-LHS `token_is_pseudo_keyword(async)` plus
        // `peek_token(FALSE) == TOK_OF` ambiguity guard. Looking ahead from
        // the raw token boundary rejects only the bare `async of` pair;
        // `async.value` and `async[key]` proceed through ordinary member-LHS
        // parsing.
        if let Some(async_span) = async_span
            && !is_for_await
            && self.next_token_is_for_of_keyword()
        {
            return Err(Error::syntax(
                "'for of' expression cannot start with 'async'",
                source_span(async_span),
            ));
        }
        self.parse_left_hand_side_expression()?;
        if self.current_ir().context.last_optional_chain.is_some() {
            return Err(self.syntax_here("invalid for in/of left hand-side"));
        }
        if let Some(target) = self.take_tail_identifier_reference()? {
            self.validate_identifier_assignment_target(&target)?;
            if target.object_environment {
                let function = self.current_ir_mut();
                let Some(SpannedIrOp {
                    op:
                        IrOp::IdentifierReference {
                            access: IdentifierReferenceAccess::Prepare,
                            ..
                        },
                    ..
                }) = function.ops.pop()
                else {
                    return Err(Error::internal(
                        "for-in/of identifier target lost its prepared Reference",
                    ));
                };
                function.context.stack_depth =
                    function.context.stack_depth.checked_sub(1).ok_or_else(|| {
                        Error::internal("for-in/of identifier Reference underflowed the stack")
                    })?;
            }
            self.emit_identifier_inherited(
                target.name,
                target.span,
                target.scope,
                IdentifierAccess::Put,
            )?;
            return Ok(ForAssignmentTargetInfo {
                declaration: ForAssignmentDeclaration::Assignment,
                var_initializer: None,
                is_destructuring: false,
            });
        }
        let Some(target) = self.take_tail_member_reference()? else {
            return Err(self.syntax_here("invalid for in/of left hand-side"));
        };
        self.emit_for_of_member_put(target)?;
        Ok(ForAssignmentTargetInfo {
            declaration: ForAssignmentDeclaration::Assignment,
            var_initializer: None,
            is_destructuring: false,
        })
    }

    /// Reorder `value, base[, key]` into the ordinary property-write layout
    /// without introducing a forgeable temporary. `Insert2; Drop` is the
    /// existing typed bytecode's two-value swap; `Perm3` first rotates the
    /// computed form into position.
    pub(in crate::engine::compiler) fn emit_for_of_member_put(
        &mut self,
        target: MemberReference,
    ) -> Result<(), Error> {
        match target {
            MemberReference::Field { key, site } => {
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction_at(Instruction::PutField(key), site)?;
            }
            MemberReference::Computed { site } => {
                self.emit_instruction(Instruction::Perm3)?;
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction_at(Instruction::PutArrayEl, site)?;
            }
            MemberReference::Super { site } => {
                self.emit_instruction(Instruction::Rot4Left)?;
                self.emit_instruction_at(Instruction::PutSuperValue, site)?;
            }
            MemberReference::Private {
                name,
                span,
                scope,
                site,
            } => {
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_private_field_operation(
                    name,
                    span,
                    scope,
                    PrivateFieldAccess::Put,
                    site,
                )?;
            }
        }
        Ok(())
    }
}
