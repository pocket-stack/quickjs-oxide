use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArrayBindingSite {
    Declaration,
    Iteration(ForIterationKind),
}

impl ArrayBindingSite {
    fn unsupported(self, detail: &str) -> String {
        match self {
            Self::Declaration => detail.to_owned(),
            Self::Iteration(ForIterationKind::In) => format!("for-in {detail}"),
            Self::Iteration(ForIterationKind::Of) => format!("for-of {detail}"),
        }
    }
}

impl<'source> Parser<'source> {
    /// Parse an ArrayBindingPattern whose initializer appears after the
    /// pattern in source order. QuickJS's `js_parse_destructuring_element`
    /// emits the assignment fragment first, jumps forward to compile/evaluate
    /// the initializer, then jumps back with its value. Keep the same control
    /// inversion so binding registration remains a single parser pass without
    /// evaluating iterator machinery before the right-hand side.
    pub(super) fn parse_array_binding_declaration(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
    ) -> Result<(), Error> {
        if !matches!(
            declaration,
            ForAssignmentDeclaration::Var | ForAssignmentDeclaration::Lexical
        ) {
            return Err(Error::internal(
                "array binding pattern received a non-declaration target",
            ));
        }

        let pattern_span = self.current().span;
        if !self.array_binding_initializer_ahead() {
            return Err(Error::syntax(
                "variable name expected",
                source_span(pattern_span),
            ));
        }
        let entry_depth = self.current_ir().stack_depth;
        let initializer_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let assignment_target = self.current_ir().ops.len();

        // The backward edge from the initializer carries exactly its value.
        self.current_ir_mut().stack_depth = entry_depth
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.parse_array_binding_pattern(declaration, is_const, ArrayBindingSite::Declaration)?;
        self.require_stack_depth(entry_depth, "array binding assignment")?;
        let done_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;

        let initializer_target = self.current_ir().ops.len();
        self.patch_jump(initializer_jump, initializer_target)?;
        self.current_ir_mut().stack_depth = entry_depth;
        if !self.consume_punctuator(Punctuator::Equal)? {
            return Err(Error::syntax(
                "variable name expected",
                source_span(pattern_span),
            ));
        }
        // QuickJS's destructuring helper always restores PF_IN_ACCEPTED for
        // the whole-pattern initializer, including inside a classic-for NoIn
        // head. Ordinary identifier declarators continue to use NoIn.
        self.parse_assignment_allow_in()?;
        // NamedEvaluation applies to a default initializer at an individual
        // binding leaf, not to the iterable which feeds the whole pattern.
        self.anonymous_function_definition = None;
        self.require_stack_depth(entry_depth + 1, "array binding initializer")?;
        self.emit_instruction(Instruction::Goto(
            u32::try_from(assignment_target)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
        ))?;

        let done_target = self.current_ir().ops.len();
        self.patch_jump(done_jump, done_target)?;
        self.current_ir_mut().stack_depth = entry_depth;
        Ok(())
    }

    /// QuickJS first skip-scans a declaration pattern and enters
    /// `js_parse_destructuring_element` only when the matching outer `]` is
    /// followed by `=`. This preserves declaration-level error priority for a
    /// malformed or nested pattern which has no top-level initializer.
    fn array_binding_initializer_ahead(&self) -> bool {
        self.array_binding_following_token()
            .is_some_and(|token| matches!(token.kind, TokenKind::Punctuator(Punctuator::Equal)))
    }

    /// Return the first token after the matching `]` without committing the
    /// lexer. QuickJS uses this skip-scan both to recognize a nested binding
    /// pattern and to prioritize a nested-rest default error before parsing or
    /// registering any inner leaf.
    fn array_binding_following_token(&self) -> Option<Token<'source>> {
        self.binding_pattern_following_token(Punctuator::LeftBracket)
    }

    fn object_binding_following_token(&self) -> Option<Token<'source>> {
        self.binding_pattern_following_token(Punctuator::LeftBrace)
    }

    fn binding_pattern_following_token(&self, opening: Punctuator) -> Option<Token<'source>> {
        if !self.is_punctuator(opening) {
            return None;
        }
        let root = match opening {
            Punctuator::LeftBracket => ForHeadDelimiter::Bracket,
            Punctuator::LeftBrace => ForHeadDelimiter::Brace,
            _ => return None,
        };

        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.start);
        let mut delimiters = Vec::new();
        let mut goal = LexicalGoal::Div;
        let mut regexp_allowed = true;

        loop {
            let requested_goal = goal;
            goal = LexicalGoal::Div;
            let Ok(mut token) = lexer.next_token_with_goal(requested_goal) else {
                return None;
            };
            if requested_goal == LexicalGoal::Div
                && regexp_allowed
                && matches!(
                    token.kind,
                    TokenKind::Punctuator(Punctuator::Divide | Punctuator::DivideAssign)
                )
            {
                lexer.seek(token.span.start);
                let Ok(regexp) = lexer.next_token_with_goal(LexicalGoal::RegExp) else {
                    return None;
                };
                token = regexp;
            }

            match &token.kind {
                TokenKind::Punctuator(Punctuator::LeftParen) => {
                    if delimiters.len() >= 255 {
                        return None;
                    }
                    delimiters.push(ForHeadDelimiter::Parenthesis);
                }
                TokenKind::Punctuator(Punctuator::LeftBracket) => {
                    if delimiters.len() >= 255 {
                        return None;
                    }
                    delimiters.push(ForHeadDelimiter::Bracket);
                }
                TokenKind::Punctuator(Punctuator::LeftBrace) => {
                    if delimiters.len() >= 255 {
                        return None;
                    }
                    delimiters.push(ForHeadDelimiter::Brace);
                }
                TokenKind::Punctuator(Punctuator::RightParen) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Parenthesis) {
                        return None;
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBracket) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Bracket) {
                        return None;
                    }
                    if root == ForHeadDelimiter::Bracket && delimiters.is_empty() {
                        return lexer.next_token().ok();
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBrace) => {
                    if delimiters.last() == Some(&ForHeadDelimiter::Template) {
                        goal = LexicalGoal::TemplateContinuation;
                        regexp_allowed = true;
                        continue;
                    }
                    if delimiters.pop() != Some(ForHeadDelimiter::Brace) {
                        return None;
                    }
                    if root == ForHeadDelimiter::Brace && delimiters.is_empty() {
                        return lexer.next_token().ok();
                    }
                }
                TokenKind::Template(part) => match part.kind {
                    TemplatePartKind::Head => {
                        if delimiters.len() >= 255 {
                            return None;
                        }
                        delimiters.push(ForHeadDelimiter::Template);
                    }
                    TemplatePartKind::Middle => {
                        if delimiters.last() != Some(&ForHeadDelimiter::Template) {
                            return None;
                        }
                    }
                    TemplatePartKind::Tail => {
                        if delimiters.pop() != Some(ForHeadDelimiter::Template) {
                            return None;
                        }
                    }
                    TemplatePartKind::NoSubstitution => {}
                },
                TokenKind::Eof => return None,
                _ => {}
            }
            regexp_allowed = for_head_regexp_allowed_after(&token.kind);
        }
    }

    /// Lower the declaration slice of QuickJS
    /// `js_parse_destructuring_element` for a for-in/of head. The value yielded
    /// by the outer enumeration becomes a second, nested iterator record.
    pub(super) fn parse_for_array_binding_pattern(
        &mut self,
        iteration_kind: ForIterationKind,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
    ) -> Result<ForAssignmentTargetInfo, Error> {
        if !matches!(
            declaration,
            ForAssignmentDeclaration::Var | ForAssignmentDeclaration::Lexical
        ) {
            return Err(Error::internal(
                "array binding pattern received a non-declaration target",
            ));
        }
        self.parse_array_binding_pattern(
            declaration,
            is_const,
            ArrayBindingSite::Iteration(iteration_kind),
        )?;
        Ok(ForAssignmentTargetInfo {
            declaration,
            var_initializer: None,
            is_destructuring: true,
        })
    }

    /// Consume one already-evaluated value and initialize an ArrayBindingPattern.
    /// Every nested array opens its own `ForOfStart` region, so abrupt inner
    /// iteration unwinds the inner iterator before the outer one while normal
    /// completion closes exactly the innermost record.
    fn parse_array_binding_pattern(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        site: ArrayBindingSite,
    ) -> Result<(), Error> {
        self.expect_punctuator(Punctuator::LeftBracket)?;
        self.emit_instruction(Instruction::ForOfStart)?;

        while !self.is_punctuator(Punctuator::RightBracket) {
            let is_rest = self.consume_punctuator(Punctuator::Ellipsis)?;
            let object_binding_span = self.current().span;
            let object_binding_follow = self
                .is_punctuator(Punctuator::LeftBrace)
                .then(|| self.object_binding_following_token())
                .flatten();
            let object_binding = object_binding_follow.as_ref().is_some_and(|token| {
                matches!(
                    token.kind,
                    TokenKind::Punctuator(
                        Punctuator::Comma | Punctuator::Equal | Punctuator::RightBracket
                    )
                )
            });
            if object_binding {
                if is_rest
                    && object_binding_follow.as_ref().is_some_and(|token| {
                        matches!(token.kind, TokenKind::Punctuator(Punctuator::Equal))
                    })
                {
                    return Err(Error::syntax(
                        "rest element cannot have a default value",
                        source_span(object_binding_span),
                    ));
                }
                return Err(self.unsupported_here(
                    site.unsupported("object destructuring bindings are not implemented yet"),
                ));
            }

            let nested_array_span = self.current().span;
            let nested_array_follow = self
                .is_punctuator(Punctuator::LeftBracket)
                .then(|| self.array_binding_following_token())
                .flatten();
            let is_nested_array = nested_array_follow.as_ref().is_some_and(|token| {
                matches!(
                    token.kind,
                    TokenKind::Punctuator(
                        Punctuator::Comma | Punctuator::Equal | Punctuator::RightBracket
                    )
                )
            });
            if is_nested_array {
                if is_rest
                    && nested_array_follow.as_ref().is_some_and(|token| {
                        matches!(token.kind, TokenKind::Punctuator(Punctuator::Equal))
                    })
                {
                    return Err(Error::syntax(
                        "rest element cannot have a default value",
                        source_span(nested_array_span),
                    ));
                }
                if is_rest {
                    self.emit_array_binding_rest(0)?;
                } else {
                    self.emit_instruction(Instruction::ForOfNext(0))?;
                    self.emit_instruction(Instruction::Drop)?;
                }
                self.parse_nested_array_binding_element(declaration, is_const, site, is_rest)?;

                if self.is_punctuator(Punctuator::RightBracket) {
                    break;
                }
                if is_rest {
                    return Err(self.syntax_here("rest element must be the last one"));
                }
                self.expect_punctuator(Punctuator::Comma)?;
                continue;
            }

            if !is_rest && self.consume_punctuator(Punctuator::Comma)? {
                self.emit_instruction(Instruction::ForOfNext(0))?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::Drop)?;
                continue;
            }

            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind else {
                if is_rest
                    && matches!(
                        token.kind,
                        TokenKind::Punctuator(Punctuator::Comma | Punctuator::RightBracket)
                    )
                {
                    return Err(Error::syntax(
                        "missing binding pattern...",
                        source_span(token.span),
                    ));
                }
                return Err(Error::syntax(
                    "invalid destructuring target",
                    source_span(token.span),
                ));
            };
            validate_identifier_reservation(
                &identifier,
                token.span,
                self.current_ir().strict,
                IdentifierContext::Variable,
            )?;
            let invalid_lexical_let =
                declaration == ForAssignmentDeclaration::Lexical && identifier.value == "let";
            let name = identifier.value;
            let strict = self.current_ir().strict;
            self.advance()?;
            if invalid_lexical_let {
                return Err(self.syntax_here("invalid lexical variable name"));
            }
            if strict && matches!(name.as_str(), "eval" | "arguments") {
                return Err(Error::syntax(
                    "invalid destructuring target",
                    source_span(token.span),
                ));
            }
            match declaration {
                ForAssignmentDeclaration::Lexical => {
                    self.register_lexical_binding(
                        &name,
                        token.span,
                        self.current().span,
                        is_const,
                        false,
                    )?;
                }
                ForAssignmentDeclaration::Var => {
                    self.register_var_binding(&name, token.span, self.current().span)?;
                }
                ForAssignmentDeclaration::Assignment => {
                    unreachable!("array binding declaration was validated before parsing elements")
                }
            }

            // QuickJS materializes a var Reference before IteratorStep so a
            // getter/next side effect cannot retarget a sloppy `with` or global
            // assignment. Lexical bindings have a fixed cell and need no
            // reference operand.
            let reference_scope = self.current_ir().current_scope;
            let next_offset = if declaration == ForAssignmentDeclaration::Var {
                self.emit_identifier_reference_inherited(
                    name.clone(),
                    token.span,
                    reference_scope,
                    IdentifierReferenceAccess::Prepare,
                )?;
                1
            } else {
                0
            };
            if is_rest {
                self.emit_array_binding_rest(next_offset)?;
            } else {
                self.emit_instruction(Instruction::ForOfNext(next_offset))?;
                self.emit_instruction(Instruction::Drop)?;
                if self.consume_punctuator(Punctuator::Equal)? {
                    self.emit_instruction(Instruction::Dup)?;
                    self.emit_instruction(Instruction::Undefined)?;
                    self.emit_instruction(Instruction::StrictEq)?;
                    let has_value = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
                    self.emit_instruction(Instruction::Drop)?;
                    // QuickJS parses an individual destructuring default with
                    // PF_IN_ACCEPTED even when the enclosing classic-for
                    // initializer is using the NoIn grammar.
                    self.parse_assignment_allow_in()?;
                    if self.anonymous_function_definition.take().is_some() {
                        let name_constant = self.add_constant(IrConstant::Primitive(
                            Value::String(JsString::try_from_utf8(&name)?),
                        ))?;
                        self.emit_instruction(Instruction::SetName(name_constant))?;
                    }
                    let has_value_target = self.current_ir().ops.len();
                    self.patch_jump(has_value, has_value_target)?;
                }
            }
            if declaration == ForAssignmentDeclaration::Var {
                self.emit_identifier_reference_inherited(
                    name,
                    token.span,
                    reference_scope,
                    IdentifierReferenceAccess::Set,
                )?;
                self.emit_instruction(Instruction::Drop)?;
            } else {
                // QuickJS's put_lvalue emits no source marker here. In
                // particular, a following normal IteratorClose failure keeps
                // the preceding initializer marker instead of moving to the
                // binding identifier.
                self.emit_identifier_inherited(
                    name,
                    token.span,
                    reference_scope,
                    IdentifierAccess::Initialize,
                )?;
            }

            if self.is_punctuator(Punctuator::RightBracket) {
                break;
            }
            if is_rest {
                return Err(self.syntax_here("rest element must be the last one"));
            }
            self.expect_punctuator(Punctuator::Comma)?;
        }

        self.expect_punctuator(Punctuator::RightBracket)?;
        self.emit_instruction(Instruction::IteratorClose)?;
        Ok(())
    }

    /// Parse a recursively nested array after its outer iterator has produced
    /// the value. QuickJS emits the nested assignment fragment first, then
    /// jumps forward to compile a following default initializer and back to
    /// the fragment. Retaining that control inversion is essential: the
    /// default must replace `undefined` before the nested `ForOfStart`, even
    /// though its source text follows the complete nested pattern.
    fn parse_nested_array_binding_element(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        site: ArrayBindingSite,
        is_rest: bool,
    ) -> Result<(), Error> {
        let value_depth = self.current_ir().stack_depth;
        if value_depth == 0 {
            return Err(Error::internal(
                "nested array binding has no value to consume",
            ));
        }

        if is_rest {
            self.parse_array_binding_pattern(declaration, is_const, site)?;
            if self.is_punctuator(Punctuator::Equal) {
                return Err(self.syntax_here("rest element cannot have a default value"));
            }
            return Ok(());
        }

        self.emit_instruction(Instruction::Dup)?;
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::StrictEq)?;
        let use_initializer = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
        let assignment_target = self.current_ir().ops.len();

        self.parse_array_binding_pattern(declaration, is_const, site)?;
        self.require_stack_depth(value_depth - 1, "nested array binding assignment")?;

        if self.consume_punctuator(Punctuator::Equal)? {
            let done_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
            let initializer_target = self.current_ir().ops.len();
            self.patch_jump(use_initializer, initializer_target)?;

            // The true edge of the undefined test retains the original value.
            self.current_ir_mut().stack_depth = value_depth;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_assignment_allow_in()?;
            // A default attached to a BindingPattern does not perform
            // identifier NamedEvaluation; only a leaf default does.
            self.anonymous_function_definition = None;
            self.require_stack_depth(value_depth, "nested array binding initializer")?;
            self.emit_instruction(Instruction::Goto(
                u32::try_from(assignment_target)
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
            ))?;

            let done_target = self.current_ir().ops.len();
            self.patch_jump(done_jump, done_target)?;
            self.current_ir_mut().stack_depth = value_depth - 1;
        } else {
            // The undefined test is deliberately retained as a side-effect-free
            // branch. QuickJS nops it out, but both edges carry the same value
            // and converge on the recursive assignment fragment.
            self.patch_jump(use_initializer, assignment_target)?;
        }
        Ok(())
    }

    /// QuickJS `js_emit_spread_code`: drain the active iterator into a fresh
    /// Array while retaining any prepared var Reference below the result.
    fn emit_array_binding_rest(&mut self, reference_depth: u8) -> Result<(), Error> {
        self.emit_instruction(Instruction::ArrayFrom(0))?;
        self.emit_instruction(Instruction::PushI32(0))?;
        let next_target = self.current_ir().ops.len();
        self.emit_instruction(Instruction::ForOfNext(
            reference_depth
                .checked_add(2)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
        ))?;
        let done_jump = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
        self.emit_instruction(Instruction::DefineArrayEl)?;
        self.emit_instruction(Instruction::Inc)?;
        self.emit_instruction(Instruction::Goto(
            u32::try_from(next_target)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
        ))?;

        let done_target = self.current_ir().ops.len();
        self.patch_jump(done_jump, done_target)?;
        // The true edge retains the terminal undefined value, whereas the
        // linearly emitted false edge has already consumed its value.
        self.current_ir_mut().stack_depth = self
            .current_ir()
            .stack_depth
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.emit_instruction(Instruction::Drop)?;
        self.emit_instruction(Instruction::Drop)?;
        Ok(())
    }
}
