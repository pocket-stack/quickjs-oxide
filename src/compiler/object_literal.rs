use super::{
    IdentifierAccess, IdentifierContext, IrConstant, Parser, parse_number, source_offset,
    source_span, validate_identifier,
};
use crate::bytecode::{DefineMethodKind, Instruction};
use crate::error::Error;
use crate::lexer::{NumberKind, Punctuator, TokenKind};
use crate::value::{JsString, Value};

impl<'source> Parser<'source> {
    /// Lower the data-property portion of QuickJS
    /// `js_parse_object_literal`. The fresh Object stays below every property
    /// operation. Fixed names reuse `DefineField`; computed names are
    /// canonicalized before their RHS and use `DefineArrayEl` followed by the
    /// same key drop as upstream. Synchronous concise methods use dedicated
    /// define-method operations so runtime naming and non-constructor metadata
    /// remain distinct from ordinary data-property NamedEvaluation.
    pub(super) fn parse_object_literal(&mut self) -> Result<(), Error> {
        if !self.is_punctuator(Punctuator::LeftBrace) {
            return Err(self.syntax_here("expecting '{'"));
        }
        self.advance()?;
        self.emit_instruction(Instruction::Object)?;
        let mut has_proto = false;

        while !self.is_punctuator(Punctuator::RightBrace) {
            if self.is_punctuator(Punctuator::Ellipsis) {
                let spread_span = self.current().span;
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                self.emit_instruction_at(
                    Instruction::CopyDataProperties,
                    source_offset(spread_span)?,
                )?;
                self.anonymous_function_definition = None;
            } else if self.is_punctuator(Punctuator::Multiply) {
                return Err(self
                    .unsupported_here("object literal generator methods are not implemented yet"));
            } else if self.is_punctuator(Punctuator::LeftBracket) {
                let property_span = self.current().span;
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                // QuickJS performs ToPropertyKey before evaluating the value.
                self.emit_instruction(Instruction::ToPropKey)?;
                self.expect_punctuator(Punctuator::RightBracket)?;
                if self.is_punctuator(Punctuator::LeftParen) {
                    self.parse_object_method_definition(property_span)?;
                    self.emit_instruction(Instruction::DefineMethodComputed {
                        kind: DefineMethodKind::Method,
                        enumerable: true,
                    })?;
                    self.anonymous_function_definition = None;
                } else {
                    if !self.is_punctuator(Punctuator::Colon) {
                        return Err(self.syntax_here("expecting ':'"));
                    }
                    self.advance_expression_start()?;
                    self.parse_assignment_allow_in()?;
                    if self.anonymous_function_definition.take().is_some() {
                        self.emit_instruction(Instruction::SetNameComputed)?;
                    }
                    self.emit_instruction(Instruction::DefineArrayEl)?;
                    self.emit_instruction(Instruction::Drop)?;
                }
            } else {
                let token = self.current().clone();
                let mut shorthand = None;
                let mut method_prefix = None;
                let key = match token.kind {
                    TokenKind::Identifier(identifier) => {
                        let name = identifier.value.clone();
                        shorthand = Some(identifier.clone());
                        if !identifier.has_escape
                            && matches!(name.as_str(), "get" | "set" | "async")
                        {
                            method_prefix = Some(name.clone());
                        }
                        self.advance()?;
                        JsString::try_from_utf8(&name)?
                    }
                    TokenKind::Keyword(keyword) => {
                        self.advance()?;
                        JsString::from_static(keyword.as_str())
                    }
                    TokenKind::String(string) => {
                        if self.current_ir().strict && string.has_legacy_octal_escape {
                            return Err(Error::syntax(
                                "legacy octal escapes are forbidden in strict mode",
                                source_span(token.span),
                            ));
                        }
                        self.advance()?;
                        JsString::try_from_utf16(string.value.utf16)?
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
                        parse_number(&number)
                            .map_err(|message| Error::syntax(message, source_span(token.span)))?
                            .to_js_string()?
                    }
                    TokenKind::PrivateIdentifier(_) => {
                        return Err(Error::syntax(
                            "private identifiers are not valid in object literals",
                            source_span(token.span),
                        ));
                    }
                    _ => return Err(self.syntax_here("invalid property name")),
                };

                let next_starts_property_name = matches!(
                    self.current().kind,
                    TokenKind::Identifier(_)
                        | TokenKind::Keyword(_)
                        | TokenKind::String(_)
                        | TokenKind::Number(_)
                        | TokenKind::Punctuator(Punctuator::LeftBracket)
                );
                let is_method_prefix = method_prefix.as_deref().is_some_and(|prefix| {
                    (next_starts_property_name
                        || (prefix == "async" && self.is_punctuator(Punctuator::Multiply)))
                        && (prefix != "async" || !self.current().line_terminator_before)
                });
                if self.is_punctuator(Punctuator::LeftParen) {
                    self.parse_object_method_definition(token.span)?;
                    let key_constant =
                        self.add_constant(IrConstant::Primitive(Value::String(key)))?;
                    self.emit_instruction(Instruction::DefineMethod {
                        key: key_constant,
                        kind: DefineMethodKind::Method,
                        enumerable: true,
                    })?;
                    self.anonymous_function_definition = None;
                } else if is_method_prefix {
                    return Err(Error::unsupported(
                        if method_prefix.as_deref() == Some("async") {
                            "async object literal methods are not implemented yet"
                        } else {
                            "object literal accessors are not implemented yet"
                        },
                        source_span(token.span),
                    ));
                } else if self.is_punctuator(Punctuator::Colon) {
                    self.advance_expression_start()?;
                    self.parse_assignment_allow_in()?;
                    if key == JsString::from_static("__proto__") {
                        if has_proto {
                            return Err(Error::syntax(
                                "duplicate __proto__ property name",
                                source_span(token.span),
                            ));
                        }
                        has_proto = true;
                        self.anonymous_function_definition = None;
                        self.emit_instruction(Instruction::SetProto)?;
                    } else {
                        let key_constant =
                            self.add_constant(IrConstant::Primitive(Value::String(key)))?;
                        if self.anonymous_function_definition.take().is_some() {
                            self.emit_instruction(Instruction::SetName(key_constant))?;
                        }
                        self.emit_instruction(Instruction::DefineField(key_constant))?;
                    }
                } else if let Some(identifier) = shorthand {
                    validate_identifier(
                        &identifier,
                        token.span,
                        self.current_ir().strict,
                        IdentifierContext::Reference,
                    )?;
                    self.emit_identifier(identifier.value, token.span, IdentifierAccess::Get)?;
                    let key_constant =
                        self.add_constant(IrConstant::Primitive(Value::String(key)))?;
                    self.emit_instruction(Instruction::DefineField(key_constant))?;
                    self.anonymous_function_definition = None;
                } else {
                    return Err(self.syntax_here("expecting ':'"));
                }
            }

            if !self.is_punctuator(Punctuator::Comma) {
                break;
            }
            self.advance()?;
        }
        self.expect_punctuator(Punctuator::RightBrace)?;
        self.anonymous_function_definition = None;
        Ok(())
    }
}
