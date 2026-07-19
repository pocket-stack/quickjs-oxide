//! QuickJS-shaped template literal and tagged-template lowering.
//!
//! The lexer has already normalized physical CR/CRLF for the raw value and
//! retained malformed escapes as an absent cooked value. This module keeps
//! template-site data structural until runtime publication, where the two
//! frozen realm-local Arrays become one stable bytecode constant identity.

use super::*;

impl<'source> Parser<'source> {
    /// Lower an untagged template exactly like QuickJS `js_parse_template`:
    /// the first cooked segment becomes the receiver for one observable
    /// `String.prototype.concat` lookup, substitutions are full comma
    /// expressions, and only non-empty later cooked segments become call
    /// arguments.
    pub(super) fn parse_template_literal(&mut self) -> Result<(), Error> {
        let mut depth = 0_usize;

        loop {
            let token = self.current().clone();
            let TokenKind::Template(part) = token.kind else {
                return Err(Error::internal(
                    "template parser lost its continuation token",
                ));
            };
            let kind = part.kind;
            let invalid_span = part.invalid_escape.as_ref().map(|error| error.span);
            let Some(cooked) = part.cooked else {
                return Err(Error::syntax(
                    "malformed escape sequence in string literal",
                    source_span(invalid_span.unwrap_or(token.span)),
                ));
            };

            if !cooked.utf16.is_empty() || depth == 0 {
                self.emit_atom_string(JsString::try_from_utf16(cooked.utf16)?)?;
                if depth == 0 {
                    if kind == TemplatePartKind::NoSubstitution {
                        self.advance()?;
                        self.anonymous_function_definition = None;
                        return Ok(());
                    }
                    let concat = self.add_constant(IrConstant::Primitive(Value::String(
                        JsString::from_static("concat"),
                    )))?;
                    // `js_parse_template` emits no source marker for either
                    // synthetic concat operation. Inherit the surrounding
                    // expression marker, just as ordinary QuickJS bytecode.
                    self.emit_instruction(Instruction::GetField2(concat))?;
                }
                depth += 1;
            }

            if kind == TemplatePartKind::Tail {
                let argument_count = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::internal("template receiver disappeared"))?;
                // Preserve the complete parser count until the reachable
                // bytecode stack has been checked, then encode QuickJS's low
                // u16 operand bits during lowering.
                self.emit(IrOp::TemplateCall {
                    argument_count,
                    method: true,
                })?;
                self.advance()?;
                self.finish_template_expression();
                return Ok(());
            }
            if !matches!(kind, TemplatePartKind::Head | TemplatePartKind::Middle) {
                return Err(Error::internal("invalid template-part transition"));
            }

            self.advance()?;
            self.parse_expression()?;
            depth += 1;
            self.expect_template_substitution_end()?;
        }
    }

    /// Parse one tagged-template suffix after its tag expression has already
    /// been emitted. Returns false when the next token is not a template.
    pub(super) fn parse_tagged_template_suffix(&mut self) -> Result<bool, Error> {
        if !matches!(self.current().kind, TokenKind::Template(_)) {
            return Ok(false);
        }

        // Parentheses preserve References. As with an ordinary call, convert
        // the last member getter or authored-with identifier read to the keep
        // form before any template substitution executes. `eval\`...\`` is
        // deliberately never a direct-eval call in QuickJS.
        let member_method = self.promote_last_member_get_for_call()?;
        let identifier_method = if member_method {
            false
        } else {
            self.promote_tail_identifier_get(IdentifierReferenceAccess::Call)?
                .is_some_and(|reference| reference.object_environment)
        };
        self.parse_tagged_template(member_method || identifier_method)?;
        Ok(true)
    }

    fn parse_tagged_template(&mut self, method: bool) -> Result<(), Error> {
        let call_span = self.current().span;
        // QuickJS emits the constant load before parsing substitutions. Keep a
        // parser-private structural placeholder at that exact stack position,
        // then append each token's cooked/raw pair as it is reached.
        let constant = self.add_constant(IrConstant::TemplateObject {
            cooked: Vec::new(),
            raw: Vec::new(),
        })?;
        self.emit(IrOp::PushConstant(constant))?;
        let mut argument_count = 1_usize;

        loop {
            let token = self.current().clone();
            let TokenKind::Template(part) = token.kind else {
                return Err(Error::internal(
                    "tagged template parser lost its continuation token",
                ));
            };
            let kind = part.kind;
            let cooked = part
                .cooked
                .map(|value| JsString::try_from_utf16(value.utf16))
                .transpose()?;
            let raw = JsString::try_from_utf16(part.raw_value.utf16)?;
            self.append_template_site_part(constant, cooked, raw)?;

            if matches!(
                kind,
                TemplatePartKind::NoSubstitution | TemplatePartKind::Tail
            ) {
                self.emit_at(
                    IrOp::TemplateCall {
                        argument_count,
                        method,
                    },
                    source_offset(call_span)?,
                )?;
                self.advance()?;
                self.finish_template_expression();
                return Ok(());
            }
            if !matches!(kind, TemplatePartKind::Head | TemplatePartKind::Middle) {
                return Err(Error::internal("invalid template-part transition"));
            }

            self.advance()?;
            self.parse_expression()?;
            argument_count = argument_count
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
            self.expect_template_substitution_end()?;
        }
    }

    fn append_template_site_part(
        &mut self,
        constant: u32,
        cooked: Option<JsString>,
        raw: JsString,
    ) -> Result<(), Error> {
        let constant = usize::try_from(constant)
            .map_err(|_| Error::internal("template constant index did not fit usize"))?;
        let Some(IrConstant::TemplateObject {
            cooked: cooked_parts,
            raw: raw_parts,
        }) = self.current_ir_mut().constants.get_mut(constant)
        else {
            return Err(Error::internal(
                "template site constant lost its structural payload",
            ));
        };
        cooked_parts.push(cooked);
        raw_parts.push(raw);
        Ok(())
    }

    fn expect_template_substitution_end(&mut self) -> Result<(), Error> {
        if !self.is_punctuator(Punctuator::RightBrace) {
            return Err(self.syntax_here("expected '}' after template expression"));
        }
        self.advance_with_goal(LexicalGoal::TemplateContinuation)
    }

    fn finish_template_expression(&mut self) {
        self.current_ir_mut().last_member_reference = None;
        self.current_ir_mut().last_identifier_reference = None;
        self.anonymous_function_definition = None;
    }
}
