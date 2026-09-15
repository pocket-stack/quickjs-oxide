//! Statement and declaration-list grammar.

use crate::engine::api::error::Error;
use crate::engine::api::error::ErrorKind;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::ClassInitializerKind;
use crate::engine::compiler::MAX_LOCAL_VARIABLES;
use crate::engine::compiler::WITH_OBJECT_LOCAL_NAME;
use crate::engine::compiler::lexer::Keyword;
use crate::engine::compiler::lexer::Punctuator;
use crate::engine::compiler::lexer::TokenKind;
use crate::engine::compiler::model::bindings::BindingKind;
use crate::engine::compiler::model::bindings::BindingStorage;
use crate::engine::compiler::model::bindings::SyntheticLocalKind;
use crate::engine::compiler::model::ir::IdentifierAccess;
use crate::engine::compiler::model::ir::IdentifierReferenceAccess;
use crate::engine::compiler::model::ir::IrConstant;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::SpannedIrOp;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::model::scope::ScopeKind;
use crate::engine::compiler::parser::context::BreakControlKind;
use crate::engine::compiler::parser::context::ForAssignmentDeclaration;
use crate::engine::compiler::parser::context::InMode;
use crate::engine::compiler::parser::context::ModuleDeclarationExport;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::context::StatementCompletion;
use crate::engine::compiler::parser::context::StatementPosition;
use crate::engine::compiler::parser::diagnostics::IdentifierContext;
use crate::engine::compiler::parser::diagnostics::source_offset;
use crate::engine::compiler::parser::diagnostics::source_span;
use crate::engine::compiler::parser::diagnostics::validate_identifier_reservation;
use crate::engine::value::JsString;
use crate::engine::value::PrimitiveValue as Value;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn parse_script_body(&mut self) -> Result<(), Error> {
        while !self.at_eof() {
            self.parse_statement_or_decl(
                StatementCompletion::Eval,
                StatementPosition::ProgramBody,
            )?;
        }

        self.emit_instruction(Instruction::GetLocal(self.eval_ret_local()?))?;
        self.emit_instruction(Instruction::Return)?;
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_function_body(&mut self) -> Result<(), Error> {
        while !self.is_punctuator(Punctuator::RightBrace) {
            if self.at_eof() {
                return Err(self.syntax_here("unterminated function body"));
            }
            self.parse_statement_or_decl(
                StatementCompletion::Discard,
                StatementPosition::FunctionBody,
            )?;
        }

        // QuickJS ends function bytecode with `return_undef`. It may
        // be unreachable after an explicit return, but keeps fallthrough
        // behavior structural and gives every function a terminal opcode.
        self.emit_instruction(Instruction::Undefined)?;
        if self.current_ir().derived_class_constructor {
            let this = self
                .current_ir()
                .this_local
                .ok_or_else(|| Error::internal("derived constructor has no this binding"))?;
            self.emit_instruction(Instruction::ReturnDerived(this))?;
        } else {
            self.emit_instruction(Instruction::Return)?;
        }
        Ok(())
    }

    /// QuickJS funnels program elements, function bodies, block bodies and
    /// single-statement branches through `js_parse_statement_or_decl`. Keep
    /// the same spine so completion handling, ASI and later declaration masks
    /// have one parser boundary instead of diverging script/function loops.
    pub(in crate::engine::compiler) fn parse_statement_or_decl(
        &mut self,
        completion: StatementCompletion,
        position: StatementPosition,
    ) -> Result<(), Error> {
        if self.consume_punctuator(Punctuator::Semicolon)? {
            return Ok(());
        }

        if self.lexical_declaration_ahead(position.allows_other_declaration())? {
            return match position {
                StatementPosition::FunctionBody => self.parse_lexical_statement(),
                StatementPosition::ProgramBody => self.parse_lexical_statement(),
                StatementPosition::NestedList => self.parse_lexical_statement(),
                StatementPosition::AnnexBIfArm
                | StatementPosition::AnnexBLabelBody
                | StatementPosition::Single => Err(self
                    .syntax_here("lexical declarations can't appear in single-statement context")),
            };
        }

        let annex_b_function_allowed = matches!(
            position,
            StatementPosition::AnnexBIfArm | StatementPosition::AnnexBLabelBody
        );
        if !position.allows_other_declaration()
            && self.restricted_function_declaration_ahead(annex_b_function_allowed)?
        {
            return Err(
                self.syntax_here("function declarations can't appear in single-statement context")
            );
        }

        if let Some(label_name) = self.label_ahead() {
            return self.parse_labeled_statement(completion, label_name, position);
        }

        match self.current().kind {
            TokenKind::Punctuator(Punctuator::LeftBrace) => self.parse_block_statement(completion),
            TokenKind::Keyword(Keyword::If) => self.parse_if_statement(completion),
            TokenKind::Keyword(Keyword::While) => self.parse_while_statement(completion, None),
            TokenKind::Keyword(Keyword::Do) => self.parse_do_while_statement(completion, None),
            TokenKind::Keyword(Keyword::For) => self.parse_for_statement(completion, None),
            TokenKind::Keyword(Keyword::Switch) => self.parse_switch_statement(completion),
            TokenKind::Keyword(Keyword::Try) => self.parse_try_statement(completion),
            TokenKind::Keyword(Keyword::With) => self.parse_with_statement(completion),
            TokenKind::Keyword(Keyword::Break) => self.parse_loop_jump_statement(false),
            TokenKind::Keyword(Keyword::Continue) => self.parse_loop_jump_statement(true),
            TokenKind::Keyword(Keyword::Function) => {
                self.parse_hoistable_function_declaration(position)
            }
            TokenKind::Identifier(_) if self.async_function_ahead() => {
                self.parse_hoistable_function_declaration(position)
            }
            TokenKind::Keyword(Keyword::Class) => {
                if position.allows_other_declaration() {
                    self.parse_class_declaration()
                } else {
                    Err(self
                        .syntax_here("class declarations can't appear in single-statement context"))
                }
            }
            TokenKind::Keyword(Keyword::Var) => self.parse_var_statement(),
            TokenKind::Keyword(Keyword::Return) => {
                if matches!(
                    self.current_ir().kind,
                    FunctionKind::Script | FunctionKind::Module | FunctionKind::Eval(_)
                ) {
                    Err(self.syntax_here("return not in a function"))
                } else {
                    self.parse_return_statement()
                }
            }
            TokenKind::Keyword(Keyword::Throw) => self.parse_throw_statement(),
            TokenKind::Keyword(Keyword::Debugger) => self.parse_debugger_statement(),
            TokenKind::Keyword(keyword @ (Keyword::Enum | Keyword::Export | Keyword::Extends)) => {
                Err(self.syntax_here(format!("unsupported keyword: {}", keyword.as_str())))
            }
            _ => self.parse_expression_statement(completion),
        }
    }

    /// QuickJS 2026-06-04 `js_parse_statement_or_decl(TOK_DEBUGGER)` has no
    /// debugger hook: it advances past the keyword, applies ordinary ASI, and
    /// emits no bytecode. In particular, an eval root keeps the last non-empty
    /// statement completion instead of replacing it with `undefined`.
    pub(in crate::engine::compiler) fn parse_debugger_statement(&mut self) -> Result<(), Error> {
        self.advance()?;
        self.consume_statement_terminator()
    }

    pub(in crate::engine::compiler) fn parse_hoistable_function_declaration(
        &mut self,
        position: StatementPosition,
    ) -> Result<(), Error> {
        if matches!(self.current_ir().kind, FunctionKind::Script)
            && position == StatementPosition::ProgramBody
        {
            self.parse_program_function_declaration()
        } else if matches!(self.current_ir().kind, FunctionKind::Module)
            && position == StatementPosition::ProgramBody
        {
            self.parse_module_function_declaration(ModuleDeclarationExport::None)
        } else if matches!(self.current_ir().kind, FunctionKind::Eval(_))
            && position == StatementPosition::ProgramBody
        {
            self.parse_eval_program_function_declaration()
        } else if matches!(
            self.current_ir().kind,
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
        ) && position == StatementPosition::FunctionBody
        {
            self.parse_function_body_declaration()
        } else if matches!(
            position,
            StatementPosition::NestedList
                | StatementPosition::AnnexBIfArm
                | StatementPosition::AnnexBLabelBody
        ) {
            self.parse_annex_b_function_declaration()
        } else {
            Err(self.syntax_here("function declarations can't appear in single-statement context"))
        }
    }

    pub(in crate::engine::compiler) fn parse_labeled_statement(
        &mut self,
        completion: StatementCompletion,
        label_name: String,
        position: StatementPosition,
    ) -> Result<(), Error> {
        if self
            .current_ir()
            .context
            .break_controls
            .iter()
            .any(|control| control.label_name.as_deref() == Some(label_name.as_str()))
        {
            return Err(self.syntax_here("duplicate label name"));
        }

        self.advance()?;
        self.expect_punctuator(Punctuator::Colon)?;
        match self.current().kind {
            // QuickJS passes a directly attached label into an iteration
            // statement's BlockEnv. A second label first becomes a regular
            // labeled statement, preserving the pinned release's current
            // multiple-label continue behavior.
            TokenKind::Keyword(Keyword::While) => {
                self.parse_while_statement(completion, Some(label_name))
            }
            TokenKind::Keyword(Keyword::Do) => {
                self.parse_do_while_statement(completion, Some(label_name))
            }
            TokenKind::Keyword(Keyword::For) => {
                self.parse_for_statement(completion, Some(label_name))
            }
            _ => {
                let entry_depth = self.current_ir().context.stack_depth;
                self.push_break_control(
                    BreakControlKind::RegularStatement,
                    Some(label_name),
                    entry_depth,
                    0,
                );
                let body_position =
                    if !self.current_ir().strict && position.allows_labelled_annex_b() {
                        StatementPosition::AnnexBLabelBody
                    } else {
                        StatementPosition::Single
                    };
                self.parse_statement_or_decl(completion, body_position)?;
                self.require_stack_depth(entry_depth, "labeled statement")?;

                let break_target = self.current_ir().ops.len();
                let control = self.pop_break_control()?;
                if !control.continue_jumps.is_empty() {
                    return Err(Error::internal(
                        "regular labeled statement received a continue jump",
                    ));
                }
                for jump in control.break_jumps {
                    self.patch_jump(jump, break_target)?;
                }
                self.finish_control_statement();
                Ok(())
            }
        }
    }

    pub(in crate::engine::compiler) fn parse_block_statement(
        &mut self,
        completion: StatementCompletion,
    ) -> Result<(), Error> {
        self.advance()?;
        if self.is_punctuator(Punctuator::RightBrace) {
            return self.advance();
        }
        let scope = self.push_scope(ScopeKind::Block);
        while !self.is_punctuator(Punctuator::RightBrace) {
            self.parse_statement_or_decl(completion, StatementPosition::NestedList)?;
        }
        self.advance()?;
        self.pop_scope(scope)
    }

    /// QuickJS `js_parse_statement_or_decl(TOK_WITH)`: the object expression
    /// is evaluated outside the new scope, then its `ToObject` result is stored
    /// in one unspellable local owned by that scope.  Keeping the binding typed
    /// is what lets publication reject forged dynamic-environment operands.
    pub(in crate::engine::compiler) fn parse_with_statement(
        &mut self,
        completion: StatementCompletion,
    ) -> Result<(), Error> {
        let with_span = self.current().span;
        if self.current_ir().strict {
            return Err(Error::syntax(
                "invalid keyword: with",
                source_span(with_span),
            ));
        }
        self.advance()?;
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;

        let scope = self.push_scope(ScopeKind::With);
        let local = {
            let function = self.current_ir_mut();
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(with_span)),
                );
            }
            let local = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(WITH_OBJECT_LOCAL_NAME.to_owned());
            function.ir.add_binding(
                scope,
                scope,
                WITH_OBJECT_LOCAL_NAME.to_owned(),
                BindingStorage::Local(local),
                BindingKind::WithObject,
                None,
            );
            local
        };
        self.emit_instruction(Instruction::ToObject)?;
        self.emit_instruction(Instruction::InitializeLocal(local))?;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.parse_statement_or_decl(completion, StatementPosition::Single)?;
        self.pop_scope(scope)
    }

    pub(in crate::engine::compiler) fn parse_if_statement(
        &mut self,
        completion: StatementCompletion,
    ) -> Result<(), Error> {
        self.advance()?;
        let scope = self.push_scope(ScopeKind::If);
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;

        let false_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        let branch_stack = self.current_ir().context.stack_depth;
        let branch_position = if self.current_ir().strict {
            StatementPosition::Single
        } else {
            StatementPosition::AnnexBIfArm
        };
        self.parse_statement_or_decl(completion, branch_position)?;
        let joined_stack = self.current_ir().context.stack_depth;

        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Else)) {
            let end_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
            self.advance()?;
            self.patch_jump(false_jump, self.current_ir().ops.len())?;
            self.current_ir_mut().context.stack_depth = branch_stack;
            self.parse_statement_or_decl(completion, branch_position)?;
            if self.current_ir().context.stack_depth != joined_stack {
                return Err(Error::internal("if branches have unequal stack depth"));
            }
            self.patch_jump(end_jump, self.current_ir().ops.len())?;
        } else {
            if joined_stack != branch_stack {
                return Err(Error::internal(
                    "if statement changed the fallthrough stack depth",
                ));
            }
            self.patch_jump(false_jump, self.current_ir().ops.len())?;
        }
        self.current_ir_mut().context.last_member_reference = None;
        self.current_ir_mut().context.last_identifier_reference = None;
        self.current_ir_mut().context.last_optional_chain = None;
        self.anonymous_function_definition = None;
        self.pop_scope(scope)?;
        Ok(())
    }

    /// Lower the pinned QuickJS switch layout while keeping the discriminant
    /// on the operand stack through every case body. A new case test is placed
    /// behind the previous body's fallthrough jump; all consecutive matching
    /// clauses join the same body. The final failed test is patched either to
    /// the recorded default body or to the shared break/drop tail.
    pub(in crate::engine::compiler) fn parse_switch_statement(
        &mut self,
        completion: StatementCompletion,
    ) -> Result<(), Error> {
        let outer_depth = self.current_ir().context.stack_depth;
        self.advance()?;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.expect_punctuator(Punctuator::LeftParen)?;
        self.parse_expression()?;
        self.expect_punctuator(Punctuator::RightParen)?;

        let switch_depth = outer_depth
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.require_stack_depth(switch_depth, "switch discriminant")?;
        let scope = self.push_scope(ScopeKind::Switch);
        self.push_break_control(BreakControlKind::Switch, None, switch_depth, 1);
        self.expect_punctuator(Punctuator::LeftBrace)?;

        let mut pending_no_match = None;
        let mut default_target = None;
        while !self.is_punctuator(Punctuator::RightBrace) {
            match self.current().kind {
                TokenKind::Keyword(Keyword::Case) => {
                    let previous_no_match = pending_no_match.take();
                    let fallthrough_jump = if previous_no_match.is_some() {
                        Some(self.emit_instruction(Instruction::Goto(u32::MAX))?)
                    } else {
                        None
                    };
                    let test_target = self.current_ir().ops.len();
                    if let Some(previous_no_match) = previous_no_match {
                        self.patch_jump(previous_no_match, test_target)?;
                    }

                    let mut matched_jumps = Vec::new();
                    loop {
                        self.advance()?;
                        self.emit_instruction(Instruction::Dup)?;
                        self.parse_expression()?;
                        self.expect_punctuator(Punctuator::Colon)?;
                        self.emit_instruction(Instruction::StrictEq)?;

                        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Case)) {
                            matched_jumps
                                .push(self.emit_instruction(Instruction::IfTrue(u32::MAX))?);
                        } else {
                            pending_no_match =
                                Some(self.emit_instruction(Instruction::IfFalse(u32::MAX))?);
                            let body_target = self.current_ir().ops.len();
                            if let Some(fallthrough_jump) = fallthrough_jump {
                                self.patch_jump(fallthrough_jump, body_target)?;
                            }
                            for matched_jump in matched_jumps {
                                self.patch_jump(matched_jump, body_target)?;
                            }
                            self.require_stack_depth(switch_depth, "switch case tests")?;
                            break;
                        }
                    }
                }
                TokenKind::Keyword(Keyword::Default) => {
                    self.advance()?;
                    self.expect_punctuator(Punctuator::Colon)?;
                    if default_target.is_some() {
                        return Err(self.syntax_here("duplicate default"));
                    }
                    if pending_no_match.is_none() {
                        pending_no_match =
                            Some(self.emit_instruction(Instruction::Goto(u32::MAX))?);
                    }
                    default_target = Some(self.current_ir().ops.len());
                }
                _ => {
                    if pending_no_match.is_none() {
                        return Err(self.syntax_here("invalid switch statement"));
                    }
                    self.parse_statement_or_decl(completion, StatementPosition::NestedList)?;
                    self.require_stack_depth(switch_depth, "switch case body")?;
                }
            }
        }
        self.advance()?;

        let no_match_target = default_target.unwrap_or(self.current_ir().ops.len());
        if let Some(pending_no_match) = pending_no_match {
            self.patch_jump(pending_no_match, no_match_target)?;
        }
        let break_target = self.current_ir().ops.len();
        let control = self.pop_break_control()?;
        if !control.continue_jumps.is_empty() {
            return Err(Error::internal("switch received a continue jump"));
        }
        for jump in control.break_jumps {
            self.patch_jump(jump, break_target)?;
        }
        self.emit_instruction(Instruction::Drop)?;
        self.require_stack_depth(outer_depth, "switch tail")?;
        self.finish_control_statement();
        self.pop_scope(scope)?;
        Ok(())
    }

    /// Lower TryStatement with the same catch-marker/finally-subroutine shape
    /// as the pinned QuickJS release. Even a catch-only statement owns an
    /// empty `Ret` subroutine: abrupt break/continue/return code can therefore
    /// be emitted while the parser is still unaware whether a source finally
    /// clause follows.
    pub(in crate::engine::compiler) fn parse_try_statement(
        &mut self,
        completion: StatementCompletion,
    ) -> Result<(), Error> {
        let entry_depth = self.current_ir().context.stack_depth;
        if matches!(completion, StatementCompletion::Eval) {
            self.set_eval_ret_undefined()?;
        }
        self.advance()?;

        if !self.is_punctuator(Punctuator::LeftBrace) {
            return Err(self.syntax_here("expecting '{'"));
        }

        let catch_jump = self.emit_instruction(Instruction::Catch(u32::MAX))?;
        self.push_break_control(BreakControlKind::TryFinally, None, entry_depth + 1, 1);
        self.parse_block_statement(completion)?;
        self.require_stack_depth(entry_depth + 1, "try block")?;
        let try_control = self.pop_break_control()?;
        if try_control.kind != BreakControlKind::TryFinally {
            return Err(Error::internal("try block lost its finally control"));
        }
        let mut finally_gosubs = try_control.finally_gosubs;

        self.emit_instruction(Instruction::DropCatch)?;
        self.emit_instruction(Instruction::Undefined)?;
        finally_gosubs.push(self.emit_instruction(Instruction::Gosub(u32::MAX))?);
        self.emit_instruction(Instruction::Drop)?;
        let mut end_jumps = vec![self.emit_instruction(Instruction::Goto(u32::MAX))?];

        // A catch target receives the thrown value where the catch marker had
        // lived. Restore that exceptional stack shape explicitly before
        // parsing the handler's otherwise-linear IR.
        self.current_ir_mut().context.stack_depth = entry_depth + 1;

        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Catch)) {
            self.advance()?;
            let catch_scope = self.push_scope(ScopeKind::Catch);
            self.current_ir_mut().ops.push(SpannedIrOp {
                op: IrOp::PrepareCatchScope(catch_scope),
                pc_site: None,
            });
            // QuickJS emits the Catch-scope EnterScope before the exceptional
            // handler label. The jump therefore skips TDZ initialization while
            // still using that scope's statically allocated bindings. Oxide's
            // typed preparation below restores QuickJS's default-undefined
            // frame state before any pattern initializer can read a later name.
            let catch_target = self.current_ir().ops.len() - 1;
            self.patch_jump(catch_jump, catch_target)?;

            if self.is_punctuator(Punctuator::LeftBrace) {
                // Optional catch binding: discard the exception before the
                // catch body installs its own protection marker.
                self.emit_instruction(Instruction::Drop)?;
            } else {
                self.expect_punctuator(Punctuator::LeftParen)?;
                if self.is_punctuator(Punctuator::LeftBrace) {
                    self.parse_catch_object_binding_pattern()?;
                } else if self.is_punctuator(Punctuator::LeftBracket) {
                    self.parse_catch_array_binding_pattern()?;
                } else {
                    let token = self.current().clone();
                    let TokenKind::Identifier(identifier) = token.kind else {
                        return Err(self.syntax_here("identifier expected"));
                    };
                    if identifier.escaped_reserved_word {
                        return Err(self.syntax_here("identifier expected"));
                    }
                    validate_identifier_reservation(
                        &identifier,
                        token.span,
                        self.current_ir().strict,
                        IdentifierContext::Variable,
                    )?;
                    let invalid_strict_name = self.current_ir().strict
                        && matches!(identifier.value.as_str(), "eval" | "arguments");
                    let name = identifier.value;
                    self.advance()?;
                    if invalid_strict_name {
                        return Err(Error::syntax(
                            "invalid variable name in strict mode",
                            source_span(self.current().span),
                        ));
                    }
                    self.register_lexical_binding(
                        &name,
                        token.span,
                        self.current().span,
                        false,
                        false,
                    )?;
                    let catch_binding =
                        self.current_ir()
                            .binding_id_in_scope(catch_scope, &name)
                            .ok_or_else(|| Error::internal("catch binding was not registered"))?;
                    self.current_ir_mut().bindings[catch_binding.0].is_catch_parameter = true;
                    self.emit_identifier(name, token.span, IdentifierAccess::Initialize)?;
                }
                self.expect_punctuator(Punctuator::RightParen)?;
            }

            let catch2_jump = self.emit_instruction(Instruction::Catch(u32::MAX))?;
            self.expect_punctuator(Punctuator::LeftBrace)?;
            let catch_body_scope = if self.is_punctuator(Punctuator::RightBrace) {
                None
            } else {
                Some(self.push_scope(ScopeKind::Block))
            };
            self.push_break_control(BreakControlKind::TryFinally, None, entry_depth + 1, 1);
            while !self.is_punctuator(Punctuator::RightBrace) {
                if self.at_eof() {
                    return Err(self.syntax_here("unterminated catch block"));
                }
                self.parse_statement_or_decl(completion, StatementPosition::NestedList)?;
            }
            self.advance()?;
            self.require_stack_depth(entry_depth + 1, "catch block")?;
            let catch_control = self.pop_break_control()?;
            if catch_control.kind != BreakControlKind::TryFinally {
                return Err(Error::internal("catch block lost its finally control"));
            }
            finally_gosubs.extend(catch_control.finally_gosubs);
            if let Some(catch_body_scope) = catch_body_scope {
                self.pop_scope(catch_body_scope)?;
            }
            self.pop_scope(catch_scope)?;

            self.emit_instruction(Instruction::DropCatch)?;
            self.emit_instruction(Instruction::Undefined)?;
            finally_gosubs.push(self.emit_instruction(Instruction::Gosub(u32::MAX))?);
            self.emit_instruction(Instruction::Drop)?;
            end_jumps.push(self.emit_instruction(Instruction::Goto(u32::MAX))?);

            // A throw from the catch body bypasses its normal LeaveScope. This
            // deliberately preserves QuickJS's captured catch-cell lifetime
            // quirk rather than synthesizing exception-path CloseLocal ops.
            let catch2_target = self.current_ir().ops.len();
            self.patch_jump(catch2_jump, catch2_target)?;
            self.current_ir_mut().context.stack_depth = entry_depth + 1;
            finally_gosubs.push(self.emit_instruction(Instruction::Gosub(u32::MAX))?);
            self.emit_instruction(Instruction::Throw)?;
        } else if matches!(self.current().kind, TokenKind::Keyword(Keyword::Finally)) {
            // A try-finally handler retains the exception as the pending value;
            // the subroutine returns to the following rethrow.
            let catch_target = self.current_ir().ops.len();
            self.patch_jump(catch_jump, catch_target)?;
            finally_gosubs.push(self.emit_instruction(Instruction::Gosub(u32::MAX))?);
            self.emit_instruction(Instruction::Throw)?;
        } else {
            return Err(self.syntax_here("expecting catch or finally"));
        }

        let finally_target = self.current_ir().ops.len();
        for gosub in finally_gosubs {
            self.patch_jump(gosub, finally_target)?;
        }

        // Every call enters with a pending value plus the Gosub return address.
        self.current_ir_mut().context.stack_depth = entry_depth + 2;
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Finally)) {
            self.advance()?;
            self.push_break_control(BreakControlKind::FinallyBody, None, entry_depth + 2, 2);

            let saved_eval_ret = if matches!(completion, StatementCompletion::Eval) {
                let eval_ret = self.eval_ret_local()?;
                let saved = self
                    .current_ir_mut()
                    .add_synthetic_local(SyntheticLocalKind::FinallySavedEvalCompletion)?;
                self.emit_instruction(Instruction::GetLocal(eval_ret))?;
                self.emit_instruction(Instruction::PutLocal(saved))?;
                self.set_eval_ret_undefined()?;
                Some(saved)
            } else {
                None
            };

            if !self.is_punctuator(Punctuator::LeftBrace) {
                return Err(self.syntax_here("expecting '{'"));
            }
            self.parse_block_statement(completion)?;
            if let Some(saved) = saved_eval_ret {
                self.emit_instruction(Instruction::GetLocal(saved))?;
                self.emit_instruction(Instruction::PutLocal(self.eval_ret_local()?))?;
            }
            self.require_stack_depth(entry_depth + 2, "finally block")?;
            let finally_control = self.pop_break_control()?;
            if finally_control.kind != BreakControlKind::FinallyBody
                || !finally_control.finally_gosubs.is_empty()
            {
                return Err(Error::internal("finally body control is malformed"));
            }
        }
        self.emit_instruction(Instruction::Ret)?;

        let end_target = self.current_ir().ops.len();
        for jump in end_jumps {
            self.patch_jump(jump, end_target)?;
        }
        self.current_ir_mut().context.stack_depth = entry_depth;
        self.finish_control_statement();
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_expression_statement(
        &mut self,
        completion: StatementCompletion,
    ) -> Result<(), Error> {
        // QuickJS seeds `emit_source_pos` from the first token before
        // `js_parse_expr`. A more specific marker emitted by the expression at
        // the same first opcode wins; otherwise synthetic operations (notably
        // template concat lookup) inherit this statement-entry position.
        let expression_start = self.current_ir().ops.len();
        let expression_site = source_offset(self.current().span)?;
        self.parse_expression()?;
        self.inherit_source_marker_at(expression_start, expression_site)?;
        match completion {
            StatementCompletion::Eval => {
                self.emit_instruction(Instruction::PutLocal(self.eval_ret_local()?))?;
            }
            StatementCompletion::Discard => {
                self.emit_instruction(Instruction::Drop)?;
            }
        }
        self.consume_statement_terminator()
    }

    pub(in crate::engine::compiler) fn set_eval_ret_undefined(&mut self) -> Result<(), Error> {
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::PutLocal(self.eval_ret_local()?))?;
        Ok(())
    }

    pub(in crate::engine::compiler) fn eval_ret_local(&self) -> Result<u16, Error> {
        self.current_ir()
            .eval_ret_local
            .ok_or_else(|| Error::internal("eval completion local requested outside a script"))
    }

    pub(in crate::engine::compiler) fn parse_return_statement(&mut self) -> Result<(), Error> {
        if self.current_ir().class_initializer_kind == Some(ClassInitializerKind::StaticBlock) {
            return Err(self.syntax_here("return in a static initializer block"));
        }
        let statement_depth = self.current_ir().context.stack_depth;
        let return_span = self.current().span;
        self.advance()?;
        let has_value = !(self.current().line_terminator_before
            || self.at_eof()
            || self.is_punctuator(Punctuator::Semicolon)
            || self.is_punctuator(Punctuator::RightBrace));
        if !has_value {
            self.emit_instruction(Instruction::Undefined)?;
        } else {
            self.parse_expression()?;
            // QuickJS folds `call; return` to a tail-call opcode and moves the
            // source marker to the `return` keyword. Preserve that observable
            // debug site even though this typed VM keeps two instructions.
            if let Some(SpannedIrOp {
                op:
                    IrOp::Bytecode(Instruction::Call(_) | Instruction::CallMethod(_))
                    | IrOp::TemplateCall { .. },
                pc_site,
            }) = self.current_ir_mut().ops.last_mut()
            {
                *pc_site = Some(source_offset(return_span)?);
            }
        }
        self.emit_return_completion(return_span, has_value)?;
        self.consume_statement_terminator()?;
        // Parsing continues through unreachable source. Retain the enclosing
        // statement's marker/discriminant shape just as break/continue do.
        self.current_ir_mut().context.stack_depth = statement_depth;
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_throw_statement(&mut self) -> Result<(), Error> {
        let throw_span = self.current().span;
        self.advance()?;
        if self.current().line_terminator_before {
            return Err(Error::syntax(
                "line terminator not allowed after throw",
                source_span(self.current().span),
            ));
        }
        self.parse_expression()?;
        self.emit_instruction_at(Instruction::Throw, source_offset(throw_span)?)?;
        self.consume_statement_terminator()
    }

    pub(in crate::engine::compiler) fn parse_lexical_statement(&mut self) -> Result<(), Error> {
        self.parse_lexical_declarations_with_in(InMode::Allow)?;
        self.consume_statement_terminator()
    }

    pub(in crate::engine::compiler) fn parse_lexical_declarations_with_in(
        &mut self,
        mode: InMode,
    ) -> Result<(), Error> {
        self.with_in_mode(mode, Self::parse_lexical_declarations)
    }

    pub(in crate::engine::compiler) fn parse_lexical_declarations(&mut self) -> Result<(), Error> {
        let is_const = matches!(self.current().kind, TokenKind::Keyword(Keyword::Const));
        self.advance()?;

        loop {
            if self.is_punctuator(Punctuator::LeftBracket) {
                self.parse_array_binding_declaration(ForAssignmentDeclaration::Lexical, is_const)?;
            } else if self.is_punctuator(Punctuator::LeftBrace) {
                self.parse_object_binding_declaration(ForAssignmentDeclaration::Lexical, is_const)?;
            } else {
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
                self.register_lexical_binding(
                    &name,
                    token.span,
                    self.current().span,
                    is_const,
                    false,
                )?;

                let initializer_site = if self.consume_punctuator(Punctuator::Equal)? {
                    let site = source_offset(self.tokens[self.cursor - 1].span)?;
                    self.parse_assignment()?;
                    if let Some(definition) = self.take_anonymous_function_definition() {
                        let name_constant = self.add_constant(IrConstant::Primitive(
                            Value::String(JsString::try_from_utf8(&name)?),
                        ))?;
                        self.emit_anonymous_set_name(
                            definition,
                            Instruction::SetName(name_constant),
                        )?;
                    }
                    site
                } else {
                    if is_const {
                        return Err(Error::syntax(
                            "missing initializer for const variable",
                            source_span(self.current().span),
                        ));
                    }
                    self.emit_instruction(Instruction::Undefined)?;
                    source_offset(token.span)?
                };
                self.emit_identifier_at(
                    name,
                    token.span,
                    IdentifierAccess::Initialize,
                    initializer_site,
                )?;
            }

            if !self.consume_punctuator(Punctuator::Comma)? {
                break;
            }
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_var_statement(&mut self) -> Result<(), Error> {
        self.advance()?;
        self.parse_var_declarations_with_in(InMode::Allow)?;
        self.consume_statement_terminator()
    }

    pub(in crate::engine::compiler) fn parse_var_declarations_with_in(
        &mut self,
        mode: InMode,
    ) -> Result<(), Error> {
        self.with_in_mode(mode, Self::parse_var_declarations)
    }

    pub(in crate::engine::compiler) fn parse_var_declarations(&mut self) -> Result<(), Error> {
        loop {
            if self.is_punctuator(Punctuator::LeftBracket) {
                self.parse_array_binding_declaration(ForAssignmentDeclaration::Var, false)?;
            } else if self.is_punctuator(Punctuator::LeftBrace) {
                self.parse_object_binding_declaration(ForAssignmentDeclaration::Var, false)?;
            } else {
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

                let initializer_span = self.current().span;
                if self.consume_punctuator(Punctuator::Equal)? {
                    let initializer_scope = self.current_ir().context.current_scope;
                    let object_environment = self
                        .parser_scope_has_authored_with(self.current_function, initializer_scope)?;
                    if object_environment {
                        self.emit_at(
                            IrOp::IdentifierReference {
                                name: name.clone(),
                                span: token.span,
                                scope: initializer_scope,
                                access: IdentifierReferenceAccess::Prepare,
                            },
                            source_offset(token.span)?,
                        )?;
                    }
                    self.parse_assignment()?;
                    if let Some(definition) = self.take_anonymous_function_definition() {
                        // QuickJS emits a dummy OP_set_name after an anonymous
                        // closure and rewrites its atom when NamedEvaluation
                        // applies to this initializer. Keep that contextual name
                        // separate from the child bytecode's intrinsic func_name.
                        let name_constant = self.add_constant(IrConstant::Primitive(
                            Value::String(JsString::try_from_utf8(&name)?),
                        ))?;
                        self.emit_anonymous_set_name(
                            definition,
                            Instruction::SetName(name_constant),
                        )?;
                    }
                    if object_environment {
                        self.emit_at(
                            IrOp::IdentifierReference {
                                name,
                                span: token.span,
                                scope: initializer_scope,
                                access: IdentifierReferenceAccess::Set,
                            },
                            source_offset(initializer_span)?,
                        )?;
                        self.emit_instruction(Instruction::Drop)?;
                    } else {
                        self.emit_identifier_at(
                            name,
                            token.span,
                            IdentifierAccess::Put,
                            source_offset(initializer_span)?,
                        )?;
                    }
                }
            }

            if !self.consume_punctuator(Punctuator::Comma)? {
                break;
            }
        }
        Ok(())
    }
}
