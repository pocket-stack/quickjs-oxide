//! Call, constructor, super and import expression grammar.

use crate::engine::api::error::Error;
use crate::engine::code::bytecode::ApplyKind;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::EvalKind;
use crate::engine::compiler::MAX_CALL_ARGUMENTS;
use crate::engine::compiler::lexer::Punctuator;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::lexer::TokenKind;
use crate::engine::compiler::model::ir::CallArguments;
use crate::engine::compiler::model::ir::IdentifierAccess;
use crate::engine::compiler::model::ir::IrConstant;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::diagnostics::source_offset;
use crate::engine::compiler::parser::diagnostics::source_span;
use crate::engine::compiler::pseudo_binding::ACTIVE_FUNCTION_LOCAL_NAME;
use crate::engine::compiler::pseudo_binding::HOME_OBJECT_LOCAL_NAME;
use crate::engine::compiler::pseudo_binding::NEW_TARGET_LOCAL_NAME;
use crate::engine::compiler::pseudo_binding::THIS_LOCAL_NAME;
use crate::engine::value::JsString;
use crate::engine::value::PrimitiveValue as Value;

impl<'source> Parser<'source> {
    /// Parse the contents of an already-consumed call/construct `(`.
    ///
    /// This mirrors QuickJS's two-phase lowering. Fixed arguments remain
    /// directly on the operand stack. The first spread converts the fixed
    /// prefix into a fresh dense Array plus a dynamic index; subsequent
    /// values use `DefineArrayEl` and subsequent spreads use `Append`.
    pub(in crate::engine::compiler) fn parse_call_arguments(
        &mut self,
    ) -> Result<CallArguments, Error> {
        let mut argument_count = 0_usize;
        while !self.is_punctuator(Punctuator::RightParen) {
            // QuickJS accepts 65,535 encoded fixed arguments and only rejects
            // the next parser iteration. If that next token is `...`, the
            // guard still wins before spread lowering begins.
            if argument_count >= MAX_CALL_ARGUMENTS {
                return Err(self.syntax_here("Too many call arguments"));
            }
            if self.is_punctuator(Punctuator::Ellipsis) {
                break;
            }
            // Call arguments use AssignmentExpression with `in` enabled,
            // even when the surrounding expression is a classic-for NoIn
            // initializer.
            self.parse_assignment_allow_in()?;
            argument_count += 1;
            if self.is_punctuator(Punctuator::RightParen) {
                break;
            }
            // A trailing comma advances directly to `)` and ends the loop.
            self.expect_punctuator(Punctuator::Comma)?;
        }

        if !self.is_punctuator(Punctuator::Ellipsis) {
            self.expect_punctuator(Punctuator::RightParen)?;
            return u16::try_from(argument_count)
                .map(CallArguments::Fixed)
                .map_err(|_| Error::internal("call argument count escaped the parser limit"));
        }

        let prefix = u16::try_from(argument_count)
            .map_err(|_| Error::internal("spread argument prefix escaped the parser limit"))?;
        self.emit_instruction(Instruction::ArrayFrom(prefix))?;
        self.emit_instruction(Instruction::PushI32(
            i32::try_from(argument_count)
                .map_err(|_| Error::internal("spread argument index does not fit i32"))?,
        ))?;

        // Once spread lowering begins, QuickJS no longer applies the encoded
        // fixed-call u16 count guard: the runtime Array length guard is the
        // authoritative 65,534-argument boundary.
        while !self.is_punctuator(Punctuator::RightParen) {
            if self.is_punctuator(Punctuator::Ellipsis) {
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                self.emit_instruction(Instruction::Append)?;
            } else {
                self.parse_assignment_allow_in()?;
                self.emit_instruction(Instruction::DefineArrayEl)?;
                self.emit_instruction(Instruction::Inc)?;
            }
            if self.is_punctuator(Punctuator::RightParen) {
                break;
            }
            self.expect_punctuator(Punctuator::Comma)?;
        }
        self.expect_punctuator(Punctuator::RightParen)?;
        self.emit_instruction(Instruction::Drop)?;
        Ok(CallArguments::Spread)
    }

    pub(in crate::engine::compiler) fn parse_new_expression(&mut self) -> Result<(), Error> {
        let new_span = self.current().span;
        self.advance()?;
        if self.consume_punctuator(Punctuator::Dot)? {
            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind else {
                return Err(self.syntax_here("expecting target"));
            };
            if identifier.value != "target" || identifier.has_escape {
                return Err(self.syntax_here("expecting target"));
            }
            if matches!(self.current_ir().kind, FunctionKind::Eval(EvalKind::None)) {
                return Err(Error::internal("eval root has no eval kind"));
            }
            if !self.current_new_target_allowed() {
                return Err(Error::syntax(
                    "new.target only allowed within functions",
                    source_span(new_span),
                ));
            }
            self.advance()?;
            if matches!(
                self.current_ir().kind,
                FunctionKind::Arrow | FunctionKind::Eval(EvalKind::Direct)
            ) {
                self.emit_identifier(
                    NEW_TARGET_LOCAL_NAME.to_owned(),
                    new_span,
                    IdentifierAccess::Get,
                )?;
            } else {
                self.emit_instruction(Instruction::PushNewTarget)?;
            }
            self.anonymous_function_definition = None;
            return Ok(());
        }

        // QuickJS parses the constructor head with calls disabled but member
        // suffixes enabled. The following `(` therefore belongs to this `new`,
        // while calls after the completed construction remain postfix calls.
        self.parse_primary(false)?;
        loop {
            if self.is_punctuator(Punctuator::OptionalChain) {
                return Err(self.syntax_here("new keyword cannot be used with an optional chain"));
            }
            if self.parse_member_suffix()? {
                continue;
            }
            if self.parse_tagged_template_suffix()? {
                continue;
            }
            break;
        }
        self.emit_instruction(Instruction::Dup)?;
        let no_arguments_span = self.current().span;
        let (arguments, construct_span) = if self.is_punctuator(Punctuator::LeftParen) {
            let call_span = self.current().span;
            self.advance()?;
            (self.parse_call_arguments()?, call_span)
        } else {
            (CallArguments::Fixed(0), no_arguments_span)
        };
        match arguments {
            CallArguments::Fixed(argument_count) => {
                self.emit_instruction_at(
                    Instruction::Construct(argument_count),
                    source_offset(construct_span)?,
                )?;
            }
            CallArguments::Spread => {
                // QuickJS emits this permutation for the shared
                // `object, function, array` apply ABI. For ordinary `new`,
                // object and function are the same value produced by `Dup`.
                self.emit_instruction(Instruction::Perm3)?;
                self.emit_instruction_at(
                    Instruction::Apply(ApplyKind::Construct),
                    source_offset(construct_span)?,
                )?;
            }
        }
        self.anonymous_function_definition = None;
        Ok(())
    }

    /// Parse the ObjectLiteral-method SuperProperty subset with the same
    /// operand order as QuickJS: lexical `this` and HomeObject are fixed
    /// before a computed key expression begins. Arrows relay both authenticated
    /// pseudo bindings through ordinary closure slots.
    pub(in crate::engine::compiler) fn parse_super_property(
        &mut self,
        super_span: Span,
    ) -> Result<(), Error> {
        self.advance()?;
        if self.is_punctuator(Punctuator::LeftParen) {
            if !self.current_ir().super_call_allowed {
                return Err(
                    self.syntax_here("super() is only valid in a derived class constructor")
                );
            }
            let call_span = self.current().span;
            // Match QuickJS's `this_active_func; get_super; new.target`
            // sequence before argument evaluation. A parameter expression may
            // mutate the derived constructor's [[Prototype]], but that mutation
            // affects only a later super() call.
            self.emit_identifier(
                ACTIVE_FUNCTION_LOCAL_NAME.to_owned(),
                super_span,
                IdentifierAccess::Get,
            )?;
            self.emit_instruction(Instruction::GetSuper)?;
            self.emit_identifier(
                NEW_TARGET_LOCAL_NAME.to_owned(),
                super_span,
                IdentifierAccess::Get,
            )?;
            self.emit_instruction(Instruction::MarkSuperCall)?;
            self.advance()?;
            let arguments = self.parse_call_arguments()?;
            match arguments {
                CallArguments::Fixed(argument_count) => {
                    self.emit_instruction_at(
                        Instruction::ConstructSuper(argument_count),
                        source_offset(call_span)?,
                    )?;
                }
                CallArguments::Spread => {
                    self.emit_instruction_at(Instruction::ApplySuper, source_offset(call_span)?)?;
                }
            }
            self.emit_instruction(Instruction::Dup)?;
            self.emit_identifier(
                THIS_LOCAL_NAME.to_owned(),
                super_span,
                IdentifierAccess::InitializeDerivedThis,
            )?;
            self.emit_identifier(
                ACTIVE_FUNCTION_LOCAL_NAME.to_owned(),
                super_span,
                IdentifierAccess::Get,
            )?;
            self.emit_instruction(Instruction::CallClassInstanceInitializer)?;
            self.anonymous_function_definition = None;
            return Ok(());
        }
        if !matches!(
            self.current().kind,
            TokenKind::Punctuator(Punctuator::Dot | Punctuator::LeftBracket)
        ) {
            return Err(Error::syntax(
                "invalid use of 'super'",
                source_span(super_span),
            ));
        }

        if !self.current_ir().super_allowed {
            return Err(Error::syntax(
                "'super' is only valid in a method",
                source_span(super_span),
            ));
        }

        self.emit_identifier(
            THIS_LOCAL_NAME.to_owned(),
            super_span,
            IdentifierAccess::Get,
        )?;
        self.emit_identifier(
            HOME_OBJECT_LOCAL_NAME.to_owned(),
            super_span,
            IdentifierAccess::Get,
        )?;
        self.emit_instruction(Instruction::GetSuper)?;

        let member_span = self.current().span;
        if self.is_punctuator(Punctuator::Dot) {
            self.advance()?;
            let token = self.current().clone();
            let name = match token.kind {
                TokenKind::PrivateIdentifier(_) => {
                    return Err(Error::syntax(
                        "private class field forbidden after super",
                        source_span(token.span),
                    ));
                }
                TokenKind::Identifier(identifier) => identifier.value,
                TokenKind::Keyword(keyword) => keyword.as_str().to_owned(),
                _ => return Err(self.syntax_here("expecting field name")),
            };
            self.advance()?;
            let key = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&name)?,
            )))?;
            self.emit(IrOp::PushConstant(key))?;
        } else {
            self.advance_expression_start()?;
            self.parse_expression()?;
            self.expect_punctuator(Punctuator::RightBracket)?;
        }
        let operation =
            self.emit_instruction_at(Instruction::GetSuperValue, source_offset(member_span)?)?;
        self.current_ir_mut().context.last_member_reference = Some(operation);
        self.anonymous_function_definition = None;
        Ok(())
    }

    /// Parse and lower the Script/Eval import-expression grammar.
    pub(in crate::engine::compiler) fn parse_import_expression(
        &mut self,
        import_span: Span,
        import_call_allowed: bool,
    ) -> Result<(), Error> {
        self.advance()?;
        if self.is_punctuator(Punctuator::Dot) {
            self.advance()?;
            let exact_meta = matches!(
                &self.current().kind,
                TokenKind::Identifier(identifier)
                    if identifier.value == "meta" && !identifier.has_escape
            );
            if !exact_meta {
                return Err(self.syntax_here("meta expected"));
            }
            if self.module.is_none() {
                return Err(self.syntax_here("import.meta only valid in module code"));
            }
            self.advance()?;
            self.ensure_module_import_meta_binding()?;
            self.emit_at(
                IrOp::ImportMeta {
                    span: import_span,
                    scope: self.current_ir().context.current_scope,
                },
                source_offset(import_span)?,
            )?;
            self.anonymous_function_definition = None;
            return Ok(());
        }

        self.expect_punctuator(Punctuator::LeftParen)?;
        if !import_call_allowed {
            return Err(self.syntax_here("invalid use of 'import()'"));
        }

        // ImportCall accepts one required AssignmentExpression and at most one
        // options AssignmentExpression. Spread is not part of this grammar;
        // ordinary expression parsing therefore supplies QuickJS's exact
        // unexpected-token diagnostics for `import()` and `import(...x)`.
        self.parse_assignment_allow_in()?;
        if self.is_punctuator(Punctuator::Comma) {
            self.advance()?;
            if !self.is_punctuator(Punctuator::RightParen) {
                self.parse_assignment_allow_in()?;
                if self.is_punctuator(Punctuator::Comma) {
                    self.advance()?;
                }
            } else {
                self.emit_instruction(Instruction::Undefined)?;
            }
        } else {
            self.emit_instruction(Instruction::Undefined)?;
        }
        self.expect_punctuator(Punctuator::RightParen)?;

        self.emit_instruction_at(Instruction::Import, source_offset(import_span)?)?;
        self.anonymous_function_definition = None;
        Ok(())
    }
}
