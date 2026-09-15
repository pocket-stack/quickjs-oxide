//! Primary expressions and array literals.

use crate::engine::api::error::Error;
use crate::engine::api::error::ErrorKind;
use crate::engine::api::error::NativeErrorMessage;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::EvalKind;
use crate::engine::code::function::metadata::FunctionKind as BytecodeFunctionKind;
use crate::engine::compiler::lexer::Keyword;
use crate::engine::compiler::lexer::LexicalGoal;
use crate::engine::compiler::lexer::NumberKind;
use crate::engine::compiler::lexer::Punctuator;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::lexer::TokenKind;
use crate::engine::compiler::model::ir::IdentifierAccess;
use crate::engine::compiler::model::ir::IrConstant;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::parser::diagnostics::IdentifierContext;
use num_traits::ToPrimitive;

use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::diagnostics::source_offset;
use crate::engine::compiler::parser::diagnostics::source_span;
use crate::engine::compiler::parser::diagnostics::strict_reserved_identifier;
use crate::engine::compiler::parser::diagnostics::validate_identifier;
use crate::engine::compiler::pseudo_binding::THIS_LOCAL_NAME;
use crate::engine::value::JsString;
use crate::engine::value::PrimitiveValue as Value;
use std::rc::Rc;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn parse_primary(
        &mut self,
        import_call_allowed: bool,
    ) -> Result<(), Error> {
        // QuickJS initially tokenizes a leading slash as `/` or `/=`, then
        // rewinds from `js_parse_postfix_expr` once the grammar has proved
        // that the current position requires a PrimaryExpression.  Keep the
        // same parser-owned decision here: operator parsing consumes genuine
        // division before it can reach this function, while a slash which is
        // asked to begin an operand is rescanned as one complete RegExp token.
        if matches!(
            self.current().kind,
            TokenKind::Punctuator(Punctuator::Divide | Punctuator::DivideAssign)
        ) {
            self.relex_current_with_goal(LexicalGoal::RegExp)?;
        }
        let token = self.current().clone();
        self.anonymous_function_definition = None;
        match token.kind {
            TokenKind::Keyword(Keyword::Null) => {
                self.advance()?;
                self.emit_instruction(Instruction::Null)?;
            }
            TokenKind::Keyword(Keyword::False) => {
                self.advance()?;
                self.emit_instruction(Instruction::PushFalse)?;
            }
            TokenKind::Keyword(Keyword::True) => {
                self.advance()?;
                self.emit_instruction(Instruction::PushTrue)?;
            }
            TokenKind::Keyword(Keyword::This) => {
                self.advance()?;
                if self.current_ir().derived_class_constructor
                    || matches!(
                        self.current_ir().kind,
                        FunctionKind::Arrow | FunctionKind::Eval(EvalKind::Direct)
                    )
                {
                    self.emit_identifier(
                        THIS_LOCAL_NAME.to_owned(),
                        token.span,
                        IdentifierAccess::Get,
                    )?;
                } else {
                    self.emit_instruction(Instruction::PushThis)?;
                }
            }
            TokenKind::Number(number) => {
                if self.current_ir().strict
                    && matches!(
                        number.kind,
                        NumberKind::LegacyOctal | NumberKind::LegacyDecimal
                    )
                {
                    return Err(Error::syntax(
                        "legacy leading-zero numeric literals are forbidden in strict mode",
                        source_span(token.span),
                    ));
                }
                self.advance()?;
                let value = parse_number(&number)
                    .map_err(|message| Error::syntax(message, source_span(token.span)))?;
                self.emit_value(value)?;
            }
            TokenKind::String(string) => {
                if self.current_ir().strict && string.has_legacy_octal_escape {
                    return Err(Error::syntax(
                        "legacy octal escapes are forbidden in strict mode",
                        source_span(token.span),
                    ));
                }
                self.advance()?;
                self.emit_atom_string(JsString::try_from_utf16(string.value.utf16)?)?;
            }
            TokenKind::Punctuator(Punctuator::LeftParen) => {
                self.advance()?;
                self.parse_expression()?;
                self.expect_punctuator(Punctuator::RightParen)?;
            }
            TokenKind::Punctuator(Punctuator::LeftBrace) => {
                self.parse_object_literal()?;
            }
            TokenKind::Punctuator(Punctuator::LeftBracket) => {
                self.parse_array_literal()?;
            }
            TokenKind::Template(_) => {
                self.parse_template_literal()?;
            }
            TokenKind::Identifier(ref identifier)
                if identifier.value == "async"
                    && !identifier.has_escape
                    && self.async_function_ahead() =>
            {
                self.parse_function_expression()?;
            }
            TokenKind::Identifier(identifier) => {
                self.reject_forbidden_identifier_reference(&identifier.value, token.span)?;
                validate_identifier(
                    &identifier,
                    token.span,
                    self.current_ir().strict,
                    IdentifierContext::Reference,
                )?;
                self.advance()?;
                let operation =
                    self.emit_identifier(identifier.value, token.span, IdentifierAccess::Get)?;
                self.current_ir_mut().context.last_identifier_reference = Some(operation);
            }
            TokenKind::Keyword(Keyword::Function) => {
                self.parse_function_expression()?;
            }
            TokenKind::Keyword(Keyword::Class) => {
                self.parse_class_expression()?;
            }
            TokenKind::Keyword(Keyword::New) => {
                self.parse_new_expression()?;
            }
            TokenKind::Keyword(Keyword::Super) => {
                self.parse_super_property(token.span)?;
            }
            TokenKind::Keyword(
                keyword @ (Keyword::Const
                | Keyword::Enum
                | Keyword::Export
                | Keyword::Extends
                | Keyword::Var),
            ) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::Keyword(Keyword::Import) => {
                self.parse_import_expression(token.span, import_call_allowed)?;
            }
            TokenKind::Keyword(Keyword::Yield)
                if matches!(
                    self.current_ir().execution_kind,
                    BytecodeFunctionKind::Generator | BytecodeFunctionKind::AsyncGenerator
                ) =>
            {
                return Err(self.syntax_here("unexpected token in expression: 'yield'"));
            }
            TokenKind::Keyword(keyword)
                if self.current_ir().strict && strict_reserved_identifier(keyword) =>
            {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::Keyword(
                keyword @ (Keyword::If
                | Keyword::For
                | Keyword::Else
                | Keyword::In
                | Keyword::Case
                | Keyword::Default
                | Keyword::Catch
                | Keyword::Finally
                | Keyword::Debugger),
            ) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::Keyword(keyword) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    keyword.as_str()
                )));
            }
            TokenKind::RegExp(literal) => {
                // Pinned QuickJS calls `compile_regexp` before advancing to the
                // next token. Preserve that diagnostic precedence and retain
                // the resulting program in immutable bytecode so evaluation
                // only allocates a fresh branded object.
                let pattern_start = token
                    .span
                    .start
                    .byte_offset
                    .checked_add(1)
                    .ok_or_else(|| Error::internal("regexp source range overflowed"))?;
                let pattern_end = pattern_start
                    .checked_add(literal.pattern.len())
                    .ok_or_else(|| Error::internal("regexp source range overflowed"))?;
                let pattern = self
                    .lexer
                    .source_range_to_js_string(pattern_start..pattern_end)?
                    .ok_or_else(|| Error::internal("regexp source range is invalid"))?;
                let flags = JsString::try_from_utf8(literal.flags)?;
                let program = crate::regexp::compile(&pattern, &flags).map_err(|error| {
                    let kind = crate::regexp::javascript_compile_error_kind(&error);
                    let message =
                        crate::regexp::javascript_compile_error_message(&error).to_owned();
                    Error::new(kind, message).with_span(source_span(token.span))
                })?;
                let constant = self.add_constant(IrConstant::RegExp {
                    pattern,
                    program: Rc::new(program),
                })?;
                self.emit_instruction_at(
                    Instruction::RegExp(constant),
                    source_offset(token.span)?,
                )?;
                self.advance()?;
            }
            TokenKind::PrivateIdentifier(identifier) => {
                let mut message = NativeErrorMessage::new();
                message.push_utf8("unexpected token in expression: '");
                message.push_utf8(identifier.raw);
                message.push_utf8("'");
                return Err(Error::from_native_message(ErrorKind::Syntax, message)
                    .with_span(source_span(token.span)));
            }
            TokenKind::Punctuator(punctuator) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    punctuator.as_str()
                )));
            }
            // QuickJS gives a raw NUL token an empty printable spelling. Do
            // not embed the byte itself in the diagnostic: the later native
            // error C-string boundary would truncate the closing quote.
            TokenKind::RawAscii(0) | TokenKind::Eof => {
                return Err(self.syntax_here("unexpected token in expression: ''"));
            }
            TokenKind::RawAscii(byte) => {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    char::from(byte)
                )));
            }
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn reject_forbidden_identifier_reference(
        &self,
        name: &str,
        span: Span,
    ) -> Result<(), Error> {
        if name == "arguments" && self.current_ir().arguments_forbidden {
            return Err(Error::syntax(
                "'arguments' identifier is not allowed in class field initializer",
                source_span(span),
            ));
        }
        Ok(())
    }

    /// Lower an Array literal with the same three phases as QuickJS
    /// `js_parse_array_literal`: a dense prefix carried by `ArrayFrom`, fixed
    /// post-prefix elements defined by atom, then a dynamic-index tail for
    /// holes and spread.  Element grammar always admits `in`, even when the
    /// literal occurs in an enclosing ExpressionNoIn.
    pub(in crate::engine::compiler) fn parse_array_literal(&mut self) -> Result<(), Error> {
        const DENSE_PREFIX_LIMIT: u32 = 32;
        const MAX_STATIC_INDEX: u32 = i32::MAX as u32;

        if !self.is_punctuator(Punctuator::LeftBracket) {
            return Err(self.syntax_here("expecting '['"));
        }
        self.advance_expression_start()?;
        let mut index = 0_u32;

        // QuickJS keeps the common small dense case entirely on the operand
        // stack and lets one ArrayFrom operation consume the prefix values.
        while !self.is_punctuator(Punctuator::RightBracket) && index < DENSE_PREFIX_LIMIT {
            if self.is_punctuator(Punctuator::Comma) || self.is_punctuator(Punctuator::Ellipsis) {
                break;
            }
            self.parse_assignment_allow_in()?;
            index += 1;
            if self.is_punctuator(Punctuator::Comma) {
                self.advance_expression_start()?;
                // A comma immediately before `]` is trailing, not a hole.
            } else if !self.is_punctuator(Punctuator::RightBracket) {
                self.expect_punctuator(Punctuator::RightBracket)?;
            }
        }
        self.emit_instruction(Instruction::ArrayFrom(u16::try_from(index).map_err(
            |_| Error::internal("dense Array literal prefix does not fit u16"),
        )?))?;

        // Holes and elements after the dense prefix retain the Array on the
        // stack. A final hole needs an explicit length write because no later
        // indexed definition extends the exotic length for it.
        let mut need_length = false;
        while !self.is_punctuator(Punctuator::RightBracket) && index < MAX_STATIC_INDEX {
            if self.is_punctuator(Punctuator::Ellipsis) {
                break;
            }
            need_length = true;
            if !self.is_punctuator(Punctuator::Comma) {
                self.parse_assignment_allow_in()?;
                let key = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::try_from_utf8(&index.to_string())?,
                )))?;
                self.emit_instruction(Instruction::DefineField(key))?;
                need_length = false;
            }
            index += 1;
            if self.is_punctuator(Punctuator::Comma) {
                self.advance_expression_start()?;
                // Continue with the next element or trailing hole.
            } else if !self.is_punctuator(Punctuator::RightBracket) {
                self.expect_punctuator(Punctuator::RightBracket)?;
            }
        }

        if self.is_punctuator(Punctuator::RightBracket) {
            if need_length {
                let length = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::from_static("length"),
                )))?;
                self.emit_instruction(Instruction::Dup)?;
                self.emit_instruction(Instruction::PushI32(
                    i32::try_from(index)
                        .map_err(|_| Error::internal("Array literal index does not fit i32"))?,
                ))?;
                self.emit_instruction(Instruction::PutField(length))?;
            }
            self.expect_punctuator(Punctuator::RightBracket)?;
            self.anonymous_function_definition = None;
            return Ok(());
        }

        // A spread, or the static-index boundary, switches to a runtime index
        // kept immediately above the Array. DefineArrayEl and Append preserve
        // both operands so the parser can continue without synthetic locals.
        self.emit_instruction(Instruction::PushI32(
            i32::try_from(index)
                .map_err(|_| Error::internal("Array literal index does not fit i32"))?,
        ))?;
        while !self.is_punctuator(Punctuator::RightBracket) {
            if self.is_punctuator(Punctuator::Ellipsis) {
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                self.emit_instruction(Instruction::Append)?;
            } else {
                need_length = true;
                if !self.is_punctuator(Punctuator::Comma) {
                    self.parse_assignment_allow_in()?;
                    self.emit_instruction(Instruction::DefineArrayEl)?;
                    need_length = false;
                }
                self.emit_instruction(Instruction::Inc)?;
            }

            if !self.is_punctuator(Punctuator::Comma) {
                break;
            }
            self.advance_expression_start()?;
        }

        if need_length {
            let length = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::from_static("length"),
            )))?;
            self.emit_instruction(Instruction::Dup1)?;
            self.emit_instruction(Instruction::PutField(length))?;
        } else {
            self.emit_instruction(Instruction::Drop)?;
        }
        self.expect_punctuator(Punctuator::RightBracket)?;
        self.anonymous_function_definition = None;
        Ok(())
    }
}

use crate::engine::compiler::lexer::NumericRadix;
use crate::engine::value::bigint::JsBigInt;
use num_bigint::BigUint;

pub(in crate::engine::compiler) fn parse_number(
    number: &crate::engine::compiler::lexer::NumberLiteral<'_>,
) -> Result<Value, String> {
    let raw = number.raw.replace('_', "");
    if let NumberKind::BigInt(radix) = number.kind {
        let literal = raw
            .strip_suffix('n')
            .ok_or_else(|| "BigInt literal is missing its suffix".to_owned())?;
        let (digits, base) = match radix {
            NumericRadix::Binary => (literal.get(2..).unwrap_or_default(), 2),
            NumericRadix::Octal => (literal.get(2..).unwrap_or_default(), 8),
            NumericRadix::Decimal => (literal, 10),
            NumericRadix::Hexadecimal => (literal.get(2..).unwrap_or_default(), 16),
        };
        return JsBigInt::parse_radix(digits, base)
            .map(Value::BigInt)
            .map_err(|error| error.to_string());
    }

    let value = match number.kind {
        NumberKind::Integer(radix) => parse_radix_literal(&raw, radix)?,
        NumberKind::Float | NumberKind::LegacyDecimal => raw
            .parse::<f64>()
            .map_err(|_| format!("invalid numeric literal '{raw}'"))?,
        NumberKind::LegacyOctal => parse_digits(&raw, 8)?,
        NumberKind::BigInt(_) => unreachable!("handled above"),
    };
    Ok(Value::number(value))
}

/// Mirrors the token switch in QuickJS 2026-06-04 `js_parse_directives`.
/// Its observable ASI behavior is intentionally narrower than a generic
/// "can this token continue an expression" test.
pub(in crate::engine::compiler) fn parse_radix_literal(
    raw: &str,
    radix: NumericRadix,
) -> Result<f64, String> {
    let (digits, base) = match radix {
        NumericRadix::Binary => (raw.get(2..).unwrap_or_default(), 2),
        NumericRadix::Octal => (raw.get(2..).unwrap_or_default(), 8),
        NumericRadix::Decimal => (raw, 10),
        NumericRadix::Hexadecimal => (raw.get(2..).unwrap_or_default(), 16),
    };
    parse_digits(digits, base)
}

pub(in crate::engine::compiler) fn parse_digits(digits: &str, radix: u32) -> Result<f64, String> {
    if digits.is_empty() {
        return Err("numeric literal has no digits".to_owned());
    }
    let value = BigUint::parse_bytes(digits.as_bytes(), radix)
        .ok_or_else(|| format!("invalid base-{radix} numeric literal"))?;
    Ok(value.to_f64().unwrap_or(f64::INFINITY))
}
