//! Expression precedence, references and assignments.

use crate::engine::api::error::Error;
use crate::engine::api::error::ErrorKind;
use crate::engine::code::bytecode::ApplyKind;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::FunctionKind as BytecodeFunctionKind;
use crate::engine::compiler::lexer::Keyword;
use crate::engine::compiler::lexer::Punctuator;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::lexer::TokenKind;
use crate::engine::compiler::model::ir::CallArguments;
use crate::engine::compiler::model::ir::FunctionId;
use crate::engine::compiler::model::ir::IdentifierAccess;
use crate::engine::compiler::model::ir::IdentifierReferenceAccess;
use crate::engine::compiler::model::ir::IrConstant;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::PrivateFieldAccess;
use crate::engine::compiler::model::ir::SpannedIrOp;
use crate::engine::compiler::model::scope::ScopeId;
use crate::engine::compiler::model::scope::ScopeKind;
use crate::engine::compiler::optional_chain;
use crate::engine::compiler::optional_chain::PendingOptionalChain;
use crate::engine::compiler::parser::context::IdentifierReference;
use crate::engine::compiler::parser::context::InMode;
use crate::engine::compiler::parser::context::LogicalAssignment;
use crate::engine::compiler::parser::context::MemberReference;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::context::PowerMode;
use crate::engine::compiler::parser::diagnostics::source_offset;
use crate::engine::compiler::private_reference;
use crate::engine::value::JsString;
use crate::engine::value::PrimitiveValue as Value;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn consume_statement_terminator(
        &mut self,
    ) -> Result<(), Error> {
        if self.consume_punctuator(Punctuator::Semicolon)?
            || self.at_eof()
            || self.is_punctuator(Punctuator::RightBrace)
            || self.current().line_terminator_before
        {
            Ok(())
        } else {
            Err(self.syntax_here("expecting ';'"))
        }
    }

    pub(in crate::engine::compiler) fn parse_expression(&mut self) -> Result<(), Error> {
        self.with_in_mode(InMode::Allow, Self::parse_comma)
    }

    pub(in crate::engine::compiler) fn parse_expression_no_in(&mut self) -> Result<(), Error> {
        self.with_in_mode(InMode::Disallow, Self::parse_comma)
    }

    pub(in crate::engine::compiler) fn parse_assignment_allow_in(&mut self) -> Result<(), Error> {
        self.with_in_mode(InMode::Allow, Self::parse_assignment)
    }

    pub(in crate::engine::compiler) fn with_in_mode<T>(
        &mut self,
        mode: InMode,
        parse: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let previous = std::mem::replace(&mut self.in_mode, mode);
        let result = parse(self);
        self.in_mode = previous;
        result
    }

    pub(in crate::engine::compiler) fn parse_comma(&mut self) -> Result<(), Error> {
        self.parse_assignment()?;
        let mut has_comma = false;
        while self.is_punctuator(Punctuator::Comma) {
            self.advance()?;
            has_comma = true;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_assignment()?;
        }
        if has_comma {
            self.anonymous_function_definition = None;
            self.current_ir_mut().context.last_member_reference = None;
            self.current_ir_mut().context.last_identifier_reference = None;
            self.current_ir_mut().context.last_optional_chain = None;
        }
        Ok(())
    }

    /// Parse assignment targets through typed unresolved References. Keeping
    /// identifier writes unresolved lets the late resolver select argument,
    /// local, closure, global and private-function-name behavior after the
    /// complete nested scope tree is known.
    pub(in crate::engine::compiler) fn parse_assignment(&mut self) -> Result<(), Error> {
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Yield)) {
            return self.parse_yield_expression();
        }
        if let Some(head) = self.async_arrow_ahead() {
            return self.parse_async_arrow_function(head);
        }
        if self.reserved_arrow_head_ahead()
            && !matches!(self.current().kind, TokenKind::Keyword(Keyword::Await))
        {
            return Err(self.syntax_here("invalid arrow function parameter"));
        }
        if let Some(head) = self.arrow_head_ahead() {
            return self.parse_arrow_function(head);
        }
        // Destructuring assignment is recognized only after every arrow-head
        // probe has had its say. Like pinned QuickJS, the Array form takes a
        // control-inverted path only when the matching `]` is immediately
        // followed by `=`; otherwise this remains an ordinary Array literal.
        if self.object_assignment_pattern_ahead() {
            return self.parse_object_assignment_expression();
        }
        if self.array_assignment_pattern_ahead() {
            return self.parse_array_assignment_expression();
        }
        // QuickJS's `name0` is captured only when the AssignmentExpression
        // starts with the identifier token itself. Parenthesized lvalues are
        // valid References but intentionally do not trigger NamedEvaluation.
        let direct_identifier_name = match &self.current().kind {
            TokenKind::Identifier(identifier) => Some(identifier.value.clone()),
            _ => None,
        };
        self.parse_conditional()?;
        if self.current_ir().context.last_optional_chain.is_some()
            && matches!(
                self.current().kind,
                TokenKind::Punctuator(
                    Punctuator::Equal
                        | Punctuator::PlusAssign
                        | Punctuator::MinusAssign
                        | Punctuator::MultiplyAssign
                        | Punctuator::DivideAssign
                        | Punctuator::RemainderAssign
                        | Punctuator::ExponentAssign
                        | Punctuator::ShiftLeftAssign
                        | Punctuator::ShiftRightAssign
                        | Punctuator::UnsignedShiftRightAssign
                        | Punctuator::BitAndAssign
                        | Punctuator::BitOrAssign
                        | Punctuator::BitXorAssign
                        | Punctuator::LogicalAndAssign
                        | Punctuator::LogicalOrAssign
                        | Punctuator::NullishAssign
                )
            )
        {
            // QuickJS consumes every assignment operator before get_lvalue
            // rejects an optional-chain Reference, so the diagnostic points
            // at the first RHS token.
            self.advance()?;
            return Err(self.syntax_here("invalid assignment left-hand side"));
        }
        let logical = match self.current().kind {
            TokenKind::Punctuator(Punctuator::LogicalAndAssign) => Some(LogicalAssignment::And),
            TokenKind::Punctuator(Punctuator::LogicalOrAssign) => Some(LogicalAssignment::Or),
            TokenKind::Punctuator(Punctuator::NullishAssign) => Some(LogicalAssignment::Nullish),
            _ => None,
        };
        if let Some(logical) = logical {
            if let Some(target) =
                self.promote_tail_identifier_get(IdentifierReferenceAccess::Get)?
            {
                let infer_name = direct_identifier_name.as_deref() == Some(target.name.as_str());
                return self.parse_logical_identifier_assignment(target, logical, infer_name);
            }
            return self.parse_logical_member_assignment(logical);
        }

        let assignment_span = self.current().span;
        let compound = match self.current().kind {
            TokenKind::Punctuator(Punctuator::Equal) => None,
            TokenKind::Punctuator(Punctuator::PlusAssign) => Some(Instruction::Add),
            TokenKind::Punctuator(Punctuator::MinusAssign) => Some(Instruction::Sub),
            TokenKind::Punctuator(Punctuator::MultiplyAssign) => Some(Instruction::Mul),
            TokenKind::Punctuator(Punctuator::DivideAssign) => Some(Instruction::Div),
            TokenKind::Punctuator(Punctuator::RemainderAssign) => Some(Instruction::Mod),
            TokenKind::Punctuator(Punctuator::ExponentAssign) => Some(Instruction::Pow),
            TokenKind::Punctuator(Punctuator::ShiftLeftAssign) => Some(Instruction::Shl),
            TokenKind::Punctuator(Punctuator::ShiftRightAssign) => Some(Instruction::Sar),
            TokenKind::Punctuator(Punctuator::UnsignedShiftRightAssign) => Some(Instruction::Shr),
            TokenKind::Punctuator(Punctuator::BitAndAssign) => Some(Instruction::BitAnd),
            TokenKind::Punctuator(Punctuator::BitXorAssign) => Some(Instruction::BitXor),
            TokenKind::Punctuator(Punctuator::BitOrAssign) => Some(Instruction::BitOr),
            _ => return Ok(()),
        };

        if let Some(operation) = compound {
            if let Some(target) =
                self.promote_tail_identifier_get(IdentifierReferenceAccess::Get)?
            {
                self.advance()?;
                self.validate_identifier_assignment_target(&target)?;
                self.parse_assignment()?;
                self.emit_instruction_at(operation, source_offset(assignment_span)?)?;
                self.anonymous_function_definition = None;
                if target.object_environment {
                    self.emit_identifier_reference_inherited(
                        target.name,
                        target.span,
                        target.scope,
                        IdentifierReferenceAccess::Set,
                    )?;
                } else {
                    self.emit_identifier_inherited(
                        target.name,
                        target.span,
                        target.scope,
                        IdentifierAccess::Set,
                    )?;
                }
                return Ok(());
            }
            let Some(target) = self.promote_tail_member_get_for_compound()? else {
                // QuickJS consumes the assignment operator before rejecting
                // a non-Reference left side, so the diagnostic points at the
                // first RHS token rather than the operator.
                self.advance()?;
                return Err(self.syntax_here("invalid assignment left-hand side"));
            };
            self.advance()?;
            self.parse_assignment()?;
            self.emit_instruction_at(operation, source_offset(assignment_span)?)?;
            self.anonymous_function_definition = None;
            self.emit_member_put(target)?;
            return Ok(());
        }

        if let Some(target) = self.take_tail_identifier_reference()? {
            self.advance()?;
            self.validate_identifier_assignment_target(&target)?;
            let rhs_start = self.current_ir().ops.len();
            self.parse_assignment()?;
            self.inherit_source_marker_at(rhs_start, source_offset(target.span)?)?;
            let anonymous_rhs = self.take_anonymous_function_definition();
            if direct_identifier_name.as_deref() == Some(target.name.as_str())
                && let Some(definition) = anonymous_rhs
            {
                let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::try_from_utf8(&target.name)?,
                )))?;
                self.emit_anonymous_set_name(definition, Instruction::SetName(name_constant))?;
            }
            // QuickJS emits no source position for ordinary `=`. The Set
            // inherits the LHS marker for an unmarked RHS or the last marker
            // produced while evaluating the RHS.
            if target.object_environment {
                self.emit_identifier_reference_inherited(
                    target.name,
                    target.span,
                    target.scope,
                    IdentifierReferenceAccess::Set,
                )?;
            } else {
                self.emit_identifier_inherited(
                    target.name,
                    target.span,
                    target.scope,
                    IdentifierAccess::Set,
                )?;
            }
            self.anonymous_function_definition = None;
            return Ok(());
        }

        let Some(target) = self.take_tail_member_reference()? else {
            // As with compound assignment, QuickJS has already advanced to
            // the RHS before reporting a non-Reference left side.
            self.advance()?;
            return Err(self.syntax_here("invalid assignment left-hand side"));
        };
        self.advance()?;
        let rhs_start = self.current_ir().ops.len();
        self.parse_assignment()?;
        let site = match &target {
            MemberReference::Field { site, .. }
            | MemberReference::Computed { site }
            | MemberReference::Super { site }
            | MemberReference::Private { site, .. } => *site,
        };
        self.inherit_source_marker_at(rhs_start, site)?;

        // QuickJS does not apply NamedEvaluation to member assignment. The
        // anonymous function marker also must not escape to an enclosing
        // identifier assignment such as `x = obj.p = function(){}`.
        self.anonymous_function_definition = None;
        self.emit_member_put(target)
    }

    /// Identifier logical assignment is QuickJS's depth-zero lvalue case. The
    /// short branch already contains only the old value, while the write branch
    /// replaces it with the RHS and resolves one preserving Set operation.
    pub(in crate::engine::compiler) fn parse_logical_identifier_assignment(
        &mut self,
        target: IdentifierReference,
        logical: LogicalAssignment,
        infer_name: bool,
    ) -> Result<(), Error> {
        self.advance()?;
        self.validate_identifier_assignment_target(&target)?;
        self.emit_instruction(Instruction::Dup)?;
        if logical == LogicalAssignment::Nullish {
            self.emit_instruction(Instruction::IsUndefinedOrNull)?;
        }
        let short_circuit = self.emit_instruction(match logical {
            LogicalAssignment::Or => Instruction::IfTrue(u32::MAX),
            LogicalAssignment::And | LogicalAssignment::Nullish => Instruction::IfFalse(u32::MAX),
        })?;
        let short_circuit_depth = self.current_ir().context.stack_depth;

        self.emit_instruction(Instruction::Drop)?;
        self.parse_assignment()?;
        let anonymous_rhs = self.take_anonymous_function_definition();
        if infer_name && let Some(definition) = anonymous_rhs {
            let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&target.name)?,
            )))?;
            self.emit_anonymous_set_name(definition, Instruction::SetName(name_constant))?;
        }
        if target.object_environment {
            self.emit_identifier_reference_inherited(
                target.name,
                target.span,
                target.scope,
                IdentifierReferenceAccess::Set,
            )?;
        } else {
            self.emit_identifier_inherited(
                target.name,
                target.span,
                target.scope,
                IdentifierAccess::Set,
            )?;
        }
        let end = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let joined_depth = self.current_ir().context.stack_depth;

        let short_target = self.current_ir().ops.len();
        self.patch_jump(short_circuit, short_target)?;
        if target.object_environment {
            self.current_ir_mut().context.stack_depth = short_circuit_depth;
            self.emit_instruction(Instruction::Nip)?;
        }
        if self.current_ir().context.stack_depth != joined_depth {
            return Err(Error::internal(
                "identifier logical assignment branches have unequal stack depth",
            ));
        }
        self.patch_jump(end, self.current_ir().ops.len())?;
        self.anonymous_function_definition = None;
        self.current_ir_mut().context.last_member_reference = None;
        self.current_ir_mut().context.last_identifier_reference = None;
        self.current_ir_mut().context.last_optional_chain = None;
        Ok(())
    }

    pub(in crate::engine::compiler) fn validate_identifier_assignment_target(
        &self,
        target: &IdentifierReference,
    ) -> Result<(), Error> {
        if self.current_ir().strict && matches!(target.name.as_str(), "eval" | "arguments") {
            return Err(self.syntax_here("invalid lvalue in strict mode"));
        }
        Ok(())
    }

    /// Lower a logical member assignment with the same two-branch stack shape
    /// as QuickJS `js_parse_assign_expr2`. The kept member Reference is used
    /// only by the assignment branch; the short-circuit branch removes its
    /// base/key operands with `Nip` and returns the original property value.
    pub(in crate::engine::compiler) fn parse_logical_member_assignment(
        &mut self,
        logical: LogicalAssignment,
    ) -> Result<(), Error> {
        let Some(target) = self.promote_tail_member_get_for_compound()? else {
            // As with every other assignment operator, QuickJS advances to
            // the RHS before get_lvalue rejects a non-Reference left side.
            self.advance()?;
            return Err(self.syntax_here("invalid assignment left-hand side"));
        };
        let lvalue_depth = match &target {
            MemberReference::Field { .. } => 1,
            MemberReference::Computed { .. } => 2,
            MemberReference::Super { .. } => 3,
            MemberReference::Private { .. } => 1,
        };

        self.advance()?;
        self.emit_instruction(Instruction::Dup)?;
        if logical == LogicalAssignment::Nullish {
            self.emit_instruction(Instruction::IsUndefinedOrNull)?;
        }
        let short_circuit = self.emit_instruction(match logical {
            LogicalAssignment::Or => Instruction::IfTrue(u32::MAX),
            LogicalAssignment::And | LogicalAssignment::Nullish => Instruction::IfFalse(u32::MAX),
        })?;
        let short_circuit_depth = self.current_ir().context.stack_depth;

        self.emit_instruction(Instruction::Drop)?;
        self.parse_assignment()?;
        // Member assignment never applies NamedEvaluation to an anonymous RHS.
        self.anonymous_function_definition = None;
        self.emit_member_put(target)?;
        let end = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let joined_depth = self.current_ir().context.stack_depth;

        self.patch_jump(short_circuit, self.current_ir().ops.len())?;
        self.current_ir_mut().context.stack_depth = short_circuit_depth;
        for _ in 0..lvalue_depth {
            self.emit_instruction(Instruction::Nip)?;
        }
        if self.current_ir().context.stack_depth != joined_depth {
            return Err(Error::internal(
                "logical assignment branches have unequal stack depth",
            ));
        }
        self.patch_jump(end, self.current_ir().ops.len())?;
        self.current_ir_mut().context.last_member_reference = None;
        self.current_ir_mut().context.last_identifier_reference = None;
        self.current_ir_mut().context.last_optional_chain = None;
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_conditional(&mut self) -> Result<(), Error> {
        self.parse_coalesce()?;
        if !self.is_punctuator(Punctuator::Question) {
            return Ok(());
        }
        self.advance()?;
        self.anonymous_function_definition = None;

        let false_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        let branch_stack = self.current_ir().context.stack_depth;
        // QuickJS parses the consequent with ordinary AssignmentExpression
        // even when the surrounding classic-for initializer is NoIn.
        self.parse_assignment_allow_in()?;
        self.expect_punctuator(Punctuator::Colon)?;
        let end_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let joined_stack = self.current_ir().context.stack_depth;

        self.patch_jump(false_jump, self.current_ir().ops.len())?;
        self.current_ir_mut().context.stack_depth = branch_stack;
        self.parse_assignment()?;
        self.anonymous_function_definition = None;
        if self.current_ir().context.stack_depth != joined_stack {
            return Err(Error::internal(
                "conditional branches have unequal stack depth",
            ));
        }
        self.patch_jump(end_jump, self.current_ir().ops.len())?;
        self.current_ir_mut().context.last_member_reference = None;
        self.current_ir_mut().context.last_identifier_reference = None;
        self.current_ir_mut().context.last_optional_chain = None;
        Ok(())
    }

    /// QuickJS lowers a nullish-coalescing chain to one shared short-circuit
    /// label. Each segment preserves the selected value, and the RHS enters
    /// below the logical-and/or grammar level so unparenthesized mixing remains
    /// a syntax error instead of changing precedence.
    pub(in crate::engine::compiler) fn parse_coalesce(&mut self) -> Result<(), Error> {
        self.parse_logical_or()?;
        let mut short_circuits = Vec::new();
        while self.is_punctuator(Punctuator::NullishCoalesce) {
            self.advance()?;
            self.emit_instruction(Instruction::Dup)?;
            self.emit_instruction(Instruction::IsUndefinedOrNull)?;
            short_circuits.push(self.emit_instruction(Instruction::IfFalse(u32::MAX))?);
            self.emit_instruction(Instruction::Drop)?;
            self.parse_bitwise_or()?;
            self.anonymous_function_definition = None;
        }
        if !short_circuits.is_empty() {
            let end = self.current_ir().ops.len();
            for short_circuit in short_circuits {
                self.patch_jump(short_circuit, end)?;
            }
            self.current_ir_mut().context.last_member_reference = None;
            self.current_ir_mut().context.last_identifier_reference = None;
            self.current_ir_mut().context.last_optional_chain = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_logical_or(&mut self) -> Result<(), Error> {
        self.parse_logical_and()?;
        let mut composed = false;
        while self.is_punctuator(Punctuator::LogicalOr) {
            composed = true;
            self.advance()?;
            self.emit_instruction(Instruction::Dup)?;
            let end_jump = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_logical_and()?;
            self.patch_jump(end_jump, self.current_ir().ops.len())?;
            self.anonymous_function_definition = None;
        }
        if composed && self.is_punctuator(Punctuator::NullishCoalesce) {
            return Err(self.syntax_here("cannot mix ?? with && or ||"));
        }
        if composed {
            self.current_ir_mut().context.last_member_reference = None;
            self.current_ir_mut().context.last_identifier_reference = None;
            self.current_ir_mut().context.last_optional_chain = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_logical_and(&mut self) -> Result<(), Error> {
        self.parse_bitwise_or()?;
        let mut composed = false;
        while self.is_punctuator(Punctuator::LogicalAnd) {
            composed = true;
            self.advance()?;
            self.emit_instruction(Instruction::Dup)?;
            let end_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_bitwise_or()?;
            self.patch_jump(end_jump, self.current_ir().ops.len())?;
            self.anonymous_function_definition = None;
        }
        if composed && self.is_punctuator(Punctuator::NullishCoalesce) {
            return Err(self.syntax_here("cannot mix ?? with && or ||"));
        }
        if composed {
            self.current_ir_mut().context.last_member_reference = None;
            self.current_ir_mut().context.last_identifier_reference = None;
            self.current_ir_mut().context.last_optional_chain = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_bitwise_or(&mut self) -> Result<(), Error> {
        self.parse_bitwise_xor()?;
        while self.is_punctuator(Punctuator::BitOr) {
            let operation_span = self.current().span;
            self.advance()?;
            self.parse_bitwise_xor()?;
            self.emit_instruction_at(Instruction::BitOr, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_bitwise_xor(&mut self) -> Result<(), Error> {
        self.parse_bitwise_and()?;
        while self.is_punctuator(Punctuator::BitXor) {
            let operation_span = self.current().span;
            self.advance()?;
            self.parse_bitwise_and()?;
            self.emit_instruction_at(Instruction::BitXor, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_bitwise_and(&mut self) -> Result<(), Error> {
        self.parse_equality()?;
        while self.is_punctuator(Punctuator::BitAnd) {
            let operation_span = self.current().span;
            self.advance()?;
            self.parse_equality()?;
            self.emit_instruction_at(Instruction::BitAnd, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_equality(&mut self) -> Result<(), Error> {
        self.parse_relational()?;
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::EqualEqual) => Instruction::Eq,
                TokenKind::Punctuator(Punctuator::StrictEqual) => Instruction::StrictEq,
                TokenKind::Punctuator(Punctuator::NotEqual) => Instruction::Neq,
                TokenKind::Punctuator(Punctuator::StrictNotEqual) => Instruction::StrictNeq,
                _ => break,
            };
            self.advance()?;
            self.parse_relational()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_relational(&mut self) -> Result<(), Error> {
        if !self.parse_private_in_head()? {
            self.parse_shift()?;
        }
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::Less) => Instruction::Lt,
                TokenKind::Punctuator(Punctuator::LessEqual) => Instruction::Lte,
                TokenKind::Punctuator(Punctuator::Greater) => Instruction::Gt,
                TokenKind::Punctuator(Punctuator::GreaterEqual) => Instruction::Gte,
                TokenKind::Keyword(Keyword::Instanceof) => Instruction::InstanceOf,
                TokenKind::Keyword(Keyword::In) if self.in_mode == InMode::Disallow => break,
                TokenKind::Keyword(Keyword::In) => Instruction::In,
                _ => break,
            };
            self.advance()?;
            self.parse_shift()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_shift(&mut self) -> Result<(), Error> {
        self.parse_additive()?;
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::ShiftLeft) => Instruction::Shl,
                TokenKind::Punctuator(Punctuator::ShiftRight) => Instruction::Sar,
                TokenKind::Punctuator(Punctuator::UnsignedShiftRight) => Instruction::Shr,
                _ => break,
            };
            self.advance()?;
            self.parse_additive()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_additive(&mut self) -> Result<(), Error> {
        self.parse_multiplicative()?;
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::Plus) => Instruction::Add,
                TokenKind::Punctuator(Punctuator::Minus) => Instruction::Sub,
                _ => break,
            };
            self.advance()?;
            self.parse_multiplicative()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_multiplicative(&mut self) -> Result<(), Error> {
        self.parse_unary()?;
        loop {
            let operation_span = self.current().span;
            let operation = match self.current().kind {
                TokenKind::Punctuator(Punctuator::Multiply) => Instruction::Mul,
                TokenKind::Punctuator(Punctuator::Divide) => Instruction::Div,
                TokenKind::Punctuator(Punctuator::Remainder) => Instruction::Mod,
                _ => break,
            };
            self.advance()?;
            self.parse_unary()?;
            self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_unary(&mut self) -> Result<(), Error> {
        self.parse_unary_with_power(PowerMode::Allowed)
    }

    pub(in crate::engine::compiler) fn parse_unary_with_power(
        &mut self,
        power_mode: PowerMode,
    ) -> Result<(), Error> {
        if matches!(
            self.current().kind,
            TokenKind::Punctuator(Punctuator::Increment | Punctuator::Decrement)
        ) {
            let operator_span = self.current().span;
            let increment = self.is_punctuator(Punctuator::Increment);
            self.advance()?;
            // QuickJS passes no power flag for a prefix-update operand. This
            // leaves `**` for the outer update expression, so `++x ** 2` is
            // valid while the operand itself must still be an lvalue.
            self.parse_unary_with_power(PowerMode::None)?;
            self.lower_update_expression(operator_span, increment, false)?;
            return self.parse_power_suffix(power_mode);
        }
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Await)) {
            if !matches!(
                self.current_ir().execution_kind,
                BytecodeFunctionKind::Async | BytecodeFunctionKind::AsyncGenerator
            ) {
                return Err(self.syntax_here("unexpected 'await' keyword"));
            }
            if !self.current_ir().context.in_function_body {
                return Err(self.syntax_here("await in default expression"));
            }
            self.advance()?;
            self.parse_unary_with_power(PowerMode::Forbidden)?;
            self.emit_instruction(Instruction::Await)?;
            self.anonymous_function_definition = None;
            self.current_ir_mut().context.last_member_reference = None;
            self.current_ir_mut().context.last_identifier_reference = None;
            self.current_ir_mut().context.last_optional_chain = None;
            return Ok(());
        }
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Typeof)) {
            self.advance()?;
            let operand_start = self.current_ir().ops.len();
            self.parse_unary_with_power(PowerMode::Forbidden)?;

            // Parentheses do not change an IdentifierReference into a value
            // expression, so both `typeof missing` and `typeof (missing)` use
            // QuickJS' non-throwing global lookup.  Calls, comma expressions,
            // binary operators, and every other composed expression emit
            // additional IR and therefore retain ordinary throwing lookup.
            if self.current_ir().ops.len() == operand_start + 1
                && let Some(SpannedIrOp {
                    op: IrOp::Identifier { access, .. },
                    ..
                }) = self.current_ir_mut().ops.get_mut(operand_start)
                && *access == IdentifierAccess::Get
            {
                *access = IdentifierAccess::GetOrUndefined;
            }
            self.emit_instruction(Instruction::TypeOf)?;
            self.anonymous_function_definition = None;
            return self.parse_power_suffix(power_mode);
        }
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Delete)) {
            self.advance()?;
            let operand_start = self.current_ir().ops.len();
            self.parse_unary_with_power(PowerMode::Forbidden)?;
            // Chain close deliberately clears a terminal private Reference,
            // so optional private delete falls through to the ordinary
            // value-delete path: it performs the branded read when live and
            // returns true without deleting. Only public terminal References
            // retain metadata for the Delete-specific branch rewrite.
            let terminal_optional_chain = {
                let function = self.current_ir_mut();
                if function
                    .context
                    .last_optional_chain
                    .as_ref()
                    .is_some_and(|chain| {
                        chain.terminal_member_get == function.context.last_member_reference
                            && chain.terminal_member_get == function.ops.len().checked_sub(1)
                    })
                {
                    function.context.last_optional_chain.take()
                } else {
                    None
                }
            };
            if let Some(target) = self.take_tail_member_reference()? {
                match target {
                    MemberReference::Field { key, site } => {
                        self.emit_with_site(IrOp::PushConstant(key), Some(site))?;
                        self.emit_instruction(Instruction::Delete)?;
                    }
                    MemberReference::Computed { site } => {
                        self.emit_instruction_at(Instruction::Delete, site)?;
                    }
                    MemberReference::Super { site } => {
                        self.emit_instruction_at(Instruction::ThrowDeleteSuper, site)?;
                    }
                    MemberReference::Private { .. } => {
                        // QuickJS diagnoses after the complete operand has
                        // been parsed, so the current token is the one after
                        // the private reference.
                        return Err(self.syntax_here("cannot delete a private class field"));
                    }
                }
                if let Some(chain) = terminal_optional_chain {
                    self.rewrite_optional_chain_delete_fallback(chain)?;
                }
            } else if self.current_ir().ops.len() == operand_start + 1
                && matches!(
                    self.current_ir().ops[operand_start].op,
                    IrOp::Identifier {
                        access: IdentifierAccess::Get,
                        ..
                    }
                )
            {
                if self.current_ir().strict {
                    return Err(self.syntax_here("cannot delete a direct reference in strict mode"));
                }
                let Some(SpannedIrOp {
                    op: IrOp::Identifier { access, .. },
                    ..
                }) = self.current_ir_mut().ops.get_mut(operand_start)
                else {
                    return Err(Error::internal(
                        "direct delete identifier operation disappeared",
                    ));
                };
                *access = IdentifierAccess::Delete;
                self.current_ir_mut().context.last_identifier_reference = None;
            } else {
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::PushTrue)?;
            }
            self.anonymous_function_definition = None;
            return self.parse_power_suffix(power_mode);
        }
        let operation_span = self.current().span;
        let operation = match self.current().kind {
            TokenKind::Punctuator(Punctuator::Plus) => Some(Instruction::Plus),
            TokenKind::Punctuator(Punctuator::Minus) => Some(Instruction::Neg),
            TokenKind::Punctuator(Punctuator::BitNot) => Some(Instruction::BitNot),
            TokenKind::Punctuator(Punctuator::Not) => Some(Instruction::Not),
            TokenKind::Keyword(Keyword::Void) => {
                self.advance()?;
                self.parse_unary_with_power(PowerMode::Forbidden)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::Undefined)?;
                self.anonymous_function_definition = None;
                return self.parse_power_suffix(power_mode);
            }
            _ => None,
        };
        if let Some(operation) = operation {
            self.advance()?;
            self.parse_unary_with_power(PowerMode::Forbidden)?;
            if matches!(
                operation,
                Instruction::Plus | Instruction::Neg | Instruction::BitNot
            ) {
                self.emit_instruction_at(operation, source_offset(operation_span)?)?;
            } else {
                self.emit_instruction(operation)?;
            }
            self.anonymous_function_definition = None;
            return self.parse_power_suffix(power_mode);
        }
        self.parse_postfix()?;
        self.parse_power_suffix(power_mode)
    }

    pub(in crate::engine::compiler) fn parse_power_suffix(
        &mut self,
        power_mode: PowerMode,
    ) -> Result<(), Error> {
        if !self.is_punctuator(Punctuator::Exponent) {
            return Ok(());
        }
        match power_mode {
            PowerMode::None => return Ok(()),
            PowerMode::Forbidden => {
                return Err(Error::new(
                    ErrorKind::Syntax,
                    "unparenthesized unary expression can't appear on the left-hand side of '**'",
                ));
            }
            PowerMode::Allowed => {}
        }

        let operation_span = self.current().span;
        self.advance()?;
        self.parse_unary_with_power(PowerMode::Allowed)?;
        self.emit_instruction_at(Instruction::Pow, source_offset(operation_span)?)?;
        self.anonymous_function_definition = None;
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_postfix(&mut self) -> Result<(), Error> {
        self.parse_left_hand_side_expression()?;
        if !self.current().line_terminator_before
            && matches!(
                self.current().kind,
                TokenKind::Punctuator(Punctuator::Increment | Punctuator::Decrement)
            )
        {
            let operator_span = self.current().span;
            let increment = self.is_punctuator(Punctuator::Increment);
            self.lower_update_expression(operator_span, increment, true)?;
            self.advance()?;
        }
        Ok(())
    }

    /// Parse the LeftHandSideExpression subset shared by assignment and a
    /// for-of assignment target, deliberately stopping before postfix update.
    pub(in crate::engine::compiler) fn parse_left_hand_side_expression(
        &mut self,
    ) -> Result<(), Error> {
        self.parse_primary(true)?;
        let mut optional_chain: Option<PendingOptionalChain> = None;
        loop {
            let optional_call = if self.is_punctuator(Punctuator::OptionalChain) {
                let optional_span = self.current().span;
                self.advance()?;
                let chain = optional_chain.get_or_insert_default();
                if !self.is_punctuator(Punctuator::LeftParen) {
                    self.emit_optional_chain_test(chain, 1)?;
                    self.parse_optional_member_suffix(optional_span)?;
                    continue;
                }
                true
            } else {
                false
            };

            if self.parse_member_suffix()? {
                continue;
            }

            if optional_call || self.is_punctuator(Punctuator::LeftParen) {
                let call_span = self.current().span;
                let member_method = self.promote_last_member_get_for_call()?;
                // QuickJS selects OP_eval from the syntactic callee before
                // binding resolution. Parentheses preserve IdentifierReference,
                // while comma/member/other composed expressions have already
                // cleared this marker. Runtime identity still decides whether
                // this is original eval or an ordinary replacement call.
                //
                // `eval?.()` is deliberately indirect: the optional call
                // consumes the identifier marker as an ordinary callable
                // Reference and never emits EvalCall.
                let direct_eval_scope = if member_method || optional_call {
                    None
                } else {
                    self.take_direct_eval_scope()?
                };
                // An authored `with` may turn an identifier call into a method
                // call whose receiver is the selected object environment. The
                // unresolved Reference keeps the same two-slot call shape even
                // when resolution later proves the receiver is `undefined`.
                let identifier_method = if member_method || direct_eval_scope.is_some() {
                    false
                } else {
                    self.promote_tail_identifier_get(IdentifierReferenceAccess::Call)?
                        .is_some_and(|reference| reference.object_environment)
                };
                if optional_call && identifier_method {
                    // Pinned QuickJS 2026-06-04 promotes an authored-with
                    // identifier to a two-slot Reference, then applies the
                    // optional-call test with the one-slot accounting used by
                    // ordinary identifier calls. Its verifier rejects the
                    // resulting function before execution.
                    return Err(Error::new(ErrorKind::JsInternal, "inconsistent stack size"));
                }
                if optional_call {
                    self.emit_optional_chain_test(
                        optional_chain
                            .as_mut()
                            .ok_or_else(|| Error::internal("optional call lost its chain"))?,
                        if member_method || identifier_method {
                            2
                        } else {
                            1
                        },
                    )?;
                }
                self.advance()?;
                let arguments = self.parse_call_arguments()?;
                if let Some(scope) = direct_eval_scope {
                    self.emit_at(
                        IrOp::EvalCall {
                            arguments,
                            scope,
                            environment: None,
                        },
                        source_offset(call_span)?,
                    )?;
                } else {
                    match arguments {
                        CallArguments::Fixed(argument_count) => {
                            let instruction = if member_method || identifier_method {
                                Instruction::CallMethod(argument_count)
                            } else {
                                Instruction::Call(argument_count)
                            };
                            self.emit_instruction_at(instruction, source_offset(call_span)?)?;
                        }
                        CallArguments::Spread => {
                            if member_method || identifier_method {
                                // QuickJS `obj func array -> func obj array`.
                                self.emit_instruction(Instruction::Perm3)?;
                            } else {
                                // QuickJS `func array -> func undefined array`.
                                self.emit_instruction(Instruction::Undefined)?;
                                self.emit_instruction(Instruction::Insert2)?;
                                self.emit_instruction(Instruction::Drop)?;
                            }
                            self.emit_instruction_at(
                                Instruction::Apply(ApplyKind::Call),
                                source_offset(call_span)?,
                            )?;
                        }
                    }
                }
                self.anonymous_function_definition = None;
                continue;
            }
            if optional_chain.is_some() && matches!(self.current().kind, TokenKind::Template(_)) {
                return Err(self.syntax_here("template literal cannot appear in an optional chain"));
            }
            if self.parse_tagged_template_suffix()? {
                continue;
            }
            break;
        }
        if let Some(optional_chain) = optional_chain {
            self.finish_optional_chain(optional_chain)?;
        }
        Ok(())
    }

    /// Lower QuickJS's `get_lvalue(..., keep = TRUE)` followed by one of
    /// `inc`, `dec`, `post_inc`, or `post_dec`, then its matching
    /// `put_lvalue` keep mode. Prefix updates preserve the replacement value;
    /// postfix updates preserve the old, already-converted numeric value.
    pub(in crate::engine::compiler) fn lower_update_expression(
        &mut self,
        operator_span: Span,
        increment: bool,
        postfix: bool,
    ) -> Result<(), Error> {
        if self.current_ir().context.last_optional_chain.is_some() {
            return Err(self.syntax_here("invalid increment/decrement operand"));
        }
        let operation = match (postfix, increment) {
            (false, true) => Instruction::Inc,
            (false, false) => Instruction::Dec,
            (true, true) => Instruction::PostInc,
            (true, false) => Instruction::PostDec,
        };

        if let Some(target) = self.promote_tail_identifier_get(IdentifierReferenceAccess::Get)? {
            self.validate_identifier_assignment_target(&target)?;
            self.emit_instruction_at(operation, source_offset(operator_span)?)?;
            if target.object_environment {
                self.emit_identifier_reference_inherited(
                    target.name,
                    target.span,
                    target.scope,
                    if postfix {
                        IdentifierReferenceAccess::PostPut
                    } else {
                        IdentifierReferenceAccess::Set
                    },
                )?;
            } else {
                self.emit_identifier_inherited(
                    target.name,
                    target.span,
                    target.scope,
                    if postfix {
                        IdentifierAccess::Put
                    } else {
                        IdentifierAccess::Set
                    },
                )?;
            }
            self.anonymous_function_definition = None;
            return Ok(());
        }

        let Some(target) = self.promote_tail_member_get_for_compound()? else {
            return Err(self.syntax_here("invalid increment/decrement operand"));
        };
        self.emit_instruction_at(operation, source_offset(operator_span)?)?;
        self.anonymous_function_definition = None;
        if postfix {
            self.emit_member_post_put(target)
        } else {
            self.emit_member_put(target)
        }
    }

    /// Parse one member suffix without accepting a call. This is shared by
    /// ordinary postfix chains and constructor heads after `new`, matching
    /// QuickJS's `PF_POSTFIX_CALL` split.
    pub(in crate::engine::compiler) fn parse_member_suffix(&mut self) -> Result<bool, Error> {
        if self.is_punctuator(Punctuator::Dot) {
            let member_span = self.current().span;
            self.advance()?;
            let token = self.current().clone();
            let name = match token.kind {
                TokenKind::PrivateIdentifier(identifier) => {
                    let name = private_reference::private_binding_name(&identifier.value);
                    self.advance()?;
                    let operation =
                        self.emit_private_field_get(name, token.span, source_offset(member_span)?)?;
                    self.current_ir_mut().context.last_member_reference = Some(operation);
                    self.anonymous_function_definition = None;
                    return Ok(true);
                }
                TokenKind::Identifier(identifier) => identifier.value,
                TokenKind::Keyword(keyword) => keyword.as_str().to_owned(),
                _ => return Err(self.syntax_here("expecting field name")),
            };
            self.advance()?;
            let key = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&name)?,
            )))?;
            let operation =
                self.emit_instruction_at(Instruction::GetField(key), source_offset(member_span)?)?;
            self.current_ir_mut().context.last_member_reference = Some(operation);
            self.anonymous_function_definition = None;
            return Ok(true);
        }

        if self.is_punctuator(Punctuator::LeftBracket) {
            let member_span = self.current().span;
            self.advance()?;
            self.parse_expression()?;
            self.expect_punctuator(Punctuator::RightBracket)?;
            let operation =
                self.emit_instruction_at(Instruction::GetArrayEl, source_offset(member_span)?)?;
            self.current_ir_mut().context.last_member_reference = Some(operation);
            self.anonymous_function_definition = None;
            return Ok(true);
        }
        Ok(false)
    }

    /// QuickJS rewrites the immediately preceding member getter when `(`
    /// proves that its Reference is being called. The keep form leaves the
    /// original base below the function so `CallMethod` receives the exact
    /// receiver without re-evaluating either base or computed key.
    pub(in crate::engine::compiler) fn promote_last_member_get_for_call(
        &mut self,
    ) -> Result<bool, Error> {
        let function = self.current_ir_mut();
        if function.context.last_member_reference != function.ops.len().checked_sub(1) {
            return Ok(false);
        }
        let terminal_get = function.context.last_member_reference;
        let Some(last) = function.ops.last_mut() else {
            return Ok(false);
        };
        let promoted = match &mut last.op {
            IrOp::Bytecode(instruction @ Instruction::GetField(_)) => {
                let Instruction::GetField(key) = *instruction else {
                    unreachable!();
                };
                *instruction = Instruction::GetField2(key);
                true
            }
            IrOp::Bytecode(instruction @ Instruction::GetArrayEl) => {
                *instruction = Instruction::GetArrayEl2;
                true
            }
            IrOp::Bytecode(instruction @ Instruction::GetSuperValue) => {
                *instruction = Instruction::GetSuperValueForCall;
                true
            }
            IrOp::PrivateField { access, .. } if *access == PrivateFieldAccess::Get => {
                *access = PrivateFieldAccess::GetKeepReceiver;
                true
            }
            _ => false,
        };
        if promoted {
            function.context.stack_depth = function
                .context
                .stack_depth
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
            optional_chain::pad_grouped_method_receiver(function, terminal_get)?;
        }
        function.context.last_member_reference = None;
        Ok(promoted)
    }

    /// Keep the final identifier read in place while exposing its unresolved
    /// binding metadata to compound-assignment lowering. Parentheses preserve
    /// this marker; every operation that turns the Reference into a value
    /// clears it through `emit_with_site` or the composing parser level.
    pub(in crate::engine::compiler) fn promote_tail_identifier_get(
        &mut self,
        reference_access: IdentifierReferenceAccess,
    ) -> Result<Option<IdentifierReference>, Error> {
        let function_id = self.current_function;
        let function = self.current_ir();
        if function.context.last_identifier_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        let Some(SpannedIrOp {
            op:
                IrOp::Identifier {
                    name,
                    span,
                    scope,
                    access: IdentifierAccess::Get,
                },
            ..
        }) = function.ops.last()
        else {
            return Err(Error::internal(
                "identifier Reference marker did not point to a getter",
            ));
        };
        let object_environment = self.parser_scope_has_authored_with(function_id, *scope)?;
        let reference = IdentifierReference {
            name: name.clone(),
            span: *span,
            scope: *scope,
            object_environment,
        };
        let function = self.current_ir_mut();
        function.context.last_identifier_reference = None;
        if object_environment {
            let Some(SpannedIrOp { op, .. }) = function.ops.last_mut() else {
                return Err(Error::internal(
                    "identifier Reference marker did not point to a getter",
                ));
            };
            *op = IrOp::IdentifierReference {
                name: reference.name.clone(),
                span: reference.span,
                scope: reference.scope,
                access: reference_access,
            };
            function.context.stack_depth = function
                .context
                .stack_depth
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        }
        Ok(Some(reference))
    }

    pub(in crate::engine::compiler) fn parser_scope_has_authored_with(
        &self,
        mut function_id: FunctionId,
        mut scope: ScopeId,
    ) -> Result<bool, Error> {
        loop {
            loop {
                let current = self.functions[function_id]
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
            let Some(parent) = self.functions[function_id].parent else {
                return Ok(false);
            };
            function_id = parent.function;
            scope = parent.definition_scope;
        }
    }

    /// Consume only the parser marker for a syntactic direct-eval callee. The
    /// getter itself deliberately remains an ordinary Identifier operation so
    /// `EvalCall` retains QuickJS's undefined-receiver fallback when the
    /// resolved function is not the realm's original `%eval%`.
    pub(in crate::engine::compiler) fn take_direct_eval_scope(
        &mut self,
    ) -> Result<Option<ScopeId>, Error> {
        let function = self.current_ir_mut();
        if function.context.last_identifier_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        let Some(SpannedIrOp {
            op:
                IrOp::Identifier {
                    name,
                    scope,
                    access: IdentifierAccess::Get,
                    ..
                },
            ..
        }) = function.ops.last()
        else {
            return Err(Error::internal(
                "identifier Reference marker did not point to a getter",
            ));
        };
        if name != "eval" {
            return Ok(None);
        }
        let scope = *scope;
        function.context.last_identifier_reference = None;
        Ok(Some(scope))
    }

    /// Turn the final getter into a base-only Reference preparation for `=`.
    /// Both operations push one abstract value, so no stack-depth correction
    /// is needed while the late resolver decides whether that value is a
    /// selected object or the static `undefined` sentinel.
    pub(in crate::engine::compiler) fn take_tail_identifier_reference(
        &mut self,
    ) -> Result<Option<IdentifierReference>, Error> {
        let function_id = self.current_function;
        let function = self.current_ir();
        if function.context.last_identifier_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        let Some(SpannedIrOp {
            op:
                IrOp::Identifier {
                    name,
                    span,
                    scope,
                    access: IdentifierAccess::Get,
                },
            ..
        }) = function.ops.last()
        else {
            return Err(Error::internal(
                "identifier Reference operation disappeared",
            ));
        };
        let object_environment = self.parser_scope_has_authored_with(function_id, *scope)?;
        let reference = IdentifierReference {
            name: name.clone(),
            span: *span,
            scope: *scope,
            object_environment,
        };
        let function = self.current_ir_mut();
        function.context.last_identifier_reference = None;
        if object_environment {
            let Some(SpannedIrOp { op, .. }) = function.ops.last_mut() else {
                return Err(Error::internal(
                    "identifier Reference operation disappeared",
                ));
            };
            *op = IrOp::IdentifierReference {
                name: reference.name.clone(),
                span: reference.span,
                scope: reference.scope,
                access: IdentifierReferenceAccess::Prepare,
            };
        } else {
            function
                .ops
                .pop()
                .ok_or_else(|| Error::internal("identifier Reference operation disappeared"))?;
            function.context.stack_depth =
                function.context.stack_depth.checked_sub(1).ok_or_else(|| {
                    Error::internal("identifier lvalue removal underflowed the stack")
                })?;
        }
        Ok(Some(reference))
    }

    /// Remove the final getter while leaving its already-evaluated base/key
    /// operands on the abstract stack. This mirrors QuickJS `get_lvalue` and
    /// is shared by assignment and `delete` rewrites.
    pub(in crate::engine::compiler) fn take_tail_member_reference(
        &mut self,
    ) -> Result<Option<MemberReference>, Error> {
        let function = self.current_ir_mut();
        if function.context.last_member_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        function.context.last_member_reference = None;
        let SpannedIrOp { op, pc_site } = function
            .ops
            .pop()
            .ok_or_else(|| Error::internal("member Reference operation disappeared"))?;
        let site = pc_site.ok_or_else(|| Error::internal("member getter has no source site"))?;
        match op {
            IrOp::Bytecode(Instruction::GetField(key)) => {
                Ok(Some(MemberReference::Field { key, site }))
            }
            IrOp::Bytecode(Instruction::GetArrayEl) => {
                // Removing a 2 -> 1 getter restores the raw `[base, key]`
                // operands produced by the preceding IR.
                function.context.stack_depth = function
                    .context
                    .stack_depth
                    .checked_add(1)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                Ok(Some(MemberReference::Computed { site }))
            }
            IrOp::Bytecode(Instruction::GetSuperValue) => {
                // Removing a 3 -> 1 getter restores the authenticated method
                // receiver, frozen super base, and raw property key.
                function.context.stack_depth = function
                    .context
                    .stack_depth
                    .checked_add(2)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                Ok(Some(MemberReference::Super { site }))
            }
            IrOp::PrivateField {
                name,
                span,
                scope,
                access: PrivateFieldAccess::Get,
            } => Ok(Some(MemberReference::Private {
                name,
                span,
                scope,
                site,
            })),
            _ => Err(Error::internal(
                "member Reference marker did not point to a getter",
            )),
        }
    }

    /// Keep both the lvalue operands and the old value for compound
    /// assignment. The computed form also retains the already-converted key,
    /// exactly matching QuickJS `get_array_el3`.
    pub(in crate::engine::compiler) fn promote_tail_member_get_for_compound(
        &mut self,
    ) -> Result<Option<MemberReference>, Error> {
        let super_site = {
            let function = self.current_ir();
            if function.context.last_member_reference == function.ops.len().checked_sub(1) {
                function.ops.last().and_then(|last| {
                    matches!(last.op, IrOp::Bytecode(Instruction::GetSuperValue))
                        .then_some(last.pc_site)
                        .flatten()
                })
            } else {
                None
            }
        };
        if let Some(site) = super_site {
            {
                let function = self.current_ir_mut();
                function.context.last_member_reference = None;
                let last = function
                    .ops
                    .last_mut()
                    .ok_or_else(|| Error::internal("super Reference operation disappeared"))?;
                last.op = IrOp::Bytecode(Instruction::ToPropKey);
                last.pc_site = None;
                // Replacing 3 -> 1 with 1 -> 1 restores the three Reference
                // operands before QuickJS's dup3/get_super_value keep form.
                function.context.stack_depth = function
                    .context
                    .stack_depth
                    .checked_add(2)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
            }
            self.emit_instruction(Instruction::Dup3)?;
            self.emit_instruction_at(Instruction::GetSuperValue, site)?;
            return Ok(Some(MemberReference::Super { site }));
        }

        let function = self.current_ir_mut();
        if function.context.last_member_reference != function.ops.len().checked_sub(1) {
            return Ok(None);
        }
        function.context.last_member_reference = None;
        let last = function
            .ops
            .last_mut()
            .ok_or_else(|| Error::internal("member Reference operation disappeared"))?;
        let site = last
            .pc_site
            .ok_or_else(|| Error::internal("member getter has no source site"))?;
        let (target, extra_depth) = match &mut last.op {
            IrOp::Bytecode(instruction @ Instruction::GetField(_)) => {
                let Instruction::GetField(key) = *instruction else {
                    unreachable!();
                };
                *instruction = Instruction::GetField2(key);
                (MemberReference::Field { key, site }, 1)
            }
            IrOp::Bytecode(instruction @ Instruction::GetArrayEl) => {
                *instruction = Instruction::GetArrayEl3;
                (MemberReference::Computed { site }, 2)
            }
            IrOp::PrivateField {
                name,
                span,
                scope,
                access,
            } if *access == PrivateFieldAccess::Get => {
                *access = PrivateFieldAccess::GetKeepReceiver;
                (
                    MemberReference::Private {
                        name: name.clone(),
                        span: *span,
                        scope: *scope,
                        site,
                    },
                    1,
                )
            }
            _ => {
                return Err(Error::internal(
                    "member Reference marker did not point to a getter",
                ));
            }
        };
        function.context.stack_depth = function
            .context
            .stack_depth
            .checked_add(extra_depth)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        Ok(Some(target))
    }

    pub(in crate::engine::compiler) fn emit_member_put(
        &mut self,
        target: MemberReference,
    ) -> Result<(), Error> {
        match target {
            MemberReference::Field { key, .. } => {
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::PutField(key))?;
            }
            MemberReference::Computed { .. } => {
                self.emit_instruction(Instruction::Insert3)?;
                self.emit_instruction(Instruction::PutArrayEl)?;
            }
            MemberReference::Super { .. } => {
                self.emit_instruction(Instruction::Insert4)?;
                self.emit_instruction(Instruction::PutSuperValue)?;
            }
            MemberReference::Private {
                name,
                span,
                scope,
                site,
            } => {
                self.emit_instruction(Instruction::Insert2)?;
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

    /// QuickJS `PUT_LVALUE_KEEP_SECOND`: move the old numeric value below the
    /// kept member Reference, then consume the Reference and replacement.
    pub(in crate::engine::compiler) fn emit_member_post_put(
        &mut self,
        target: MemberReference,
    ) -> Result<(), Error> {
        match target {
            MemberReference::Field { key, .. } => {
                self.emit_instruction(Instruction::Perm3)?;
                self.emit_instruction(Instruction::PutField(key))?;
            }
            MemberReference::Computed { .. } => {
                self.emit_instruction(Instruction::Perm4)?;
                self.emit_instruction(Instruction::PutArrayEl)?;
            }
            MemberReference::Super { .. } => {
                self.emit_instruction(Instruction::Perm5)?;
                self.emit_instruction(Instruction::PutSuperValue)?;
            }
            MemberReference::Private {
                name,
                span,
                scope,
                site,
            } => {
                self.emit_instruction(Instruction::Perm3)?;
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
