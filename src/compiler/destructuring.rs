use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingSite {
    Declaration,
    Iteration(ForIterationKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingPatternKind {
    Array,
    Object,
}

enum ObjectBindingPropertyKey<'source> {
    Fixed {
        key: JsString,
        token: Token<'source>,
        shorthand: Option<Identifier<'source>>,
    },
    Computed {
        span: Span,
    },
}

struct BindingPatternScan<'source> {
    following: Token<'source>,
    has_object_rest: bool,
}

impl<'source> Parser<'source> {
    /// Parse a BindingPattern whose initializer appears after the pattern in
    /// source order. QuickJS's `js_parse_destructuring_element`
    /// emits the assignment fragment first, jumps forward to compile/evaluate
    /// the initializer, then jumps back with its value. Keep the same control
    /// inversion so binding registration remains a single parser pass without
    /// evaluating iterator machinery before the right-hand side.
    pub(super) fn parse_array_binding_declaration(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
    ) -> Result<(), Error> {
        self.parse_binding_declaration(declaration, is_const, BindingPatternKind::Array)
    }

    pub(super) fn parse_object_binding_declaration(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
    ) -> Result<(), Error> {
        self.parse_binding_declaration(declaration, is_const, BindingPatternKind::Object)
    }

    fn parse_binding_declaration(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        pattern: BindingPatternKind,
    ) -> Result<(), Error> {
        if !matches!(
            declaration,
            ForAssignmentDeclaration::Var | ForAssignmentDeclaration::Lexical
        ) {
            return Err(Error::internal(
                "binding pattern received a non-declaration target",
            ));
        }

        let pattern_span = self.current().span;
        if !self.binding_initializer_ahead(pattern) {
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
        match pattern {
            BindingPatternKind::Array => {
                self.parse_array_binding_pattern(declaration, is_const, BindingSite::Declaration)?;
            }
            BindingPatternKind::Object => {
                self.parse_object_binding_pattern(declaration, is_const, BindingSite::Declaration)?;
            }
        }
        self.require_stack_depth(entry_depth, "binding pattern assignment")?;
        let done_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;

        let initializer_target = self.current_ir().ops.len();
        self.patch_jump(initializer_jump, initializer_target)?;
        self.current_ir_mut().stack_depth = entry_depth;
        if !self.consume_punctuator(Punctuator::Equal)? {
            if let TokenKind::Punctuator(punctuator) = self.current().kind {
                return Err(self.syntax_here(format!(
                    "unexpected token in expression: '{}'",
                    punctuator.as_str()
                )));
            }
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
        self.require_stack_depth(entry_depth + 1, "binding pattern initializer")?;
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
    /// `js_parse_destructuring_element` only when the matching outer closer is
    /// followed by `=`. This preserves declaration-level error priority for a
    /// malformed or nested pattern which has no top-level initializer.
    fn binding_initializer_ahead(&self, pattern: BindingPatternKind) -> bool {
        match pattern {
            BindingPatternKind::Array => self.array_binding_following_token(),
            BindingPatternKind::Object => self.object_binding_following_token(),
        }
        .is_some_and(|token| matches!(token.kind, TokenKind::Punctuator(Punctuator::Equal)))
    }

    /// Return the first token after the matching closer without committing the
    /// lexer. QuickJS uses this skip-scan both to recognize a nested binding
    /// pattern and to prioritize a nested-rest default error before parsing or
    /// registering any inner leaf.
    fn array_binding_following_token(&self) -> Option<Token<'source>> {
        self.binding_pattern_following_token(Punctuator::LeftBracket)
    }

    fn object_binding_following_token(&self) -> Option<Token<'source>> {
        self.binding_pattern_following_token(Punctuator::LeftBrace)
    }

    /// Report whether the matching ObjectBindingPattern contains a rest
    /// property at its own nesting level. QuickJS knows `has_ellipsis` before
    /// lowering the first property and therefore creates the exclusion object
    /// before any computed key or getter can run.
    fn object_binding_has_rest(&self) -> bool {
        self.binding_pattern_scan(Punctuator::LeftBrace)
            .is_some_and(|scan| scan.has_object_rest)
    }

    fn binding_pattern_following_token(&self, opening: Punctuator) -> Option<Token<'source>> {
        self.binding_pattern_scan(opening)
            .map(|scan| scan.following)
    }

    fn binding_pattern_scan(&self, opening: Punctuator) -> Option<BindingPatternScan<'source>> {
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
        let mut has_object_rest = false;

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

            if root == ForHeadDelimiter::Brace
                && delimiters.as_slice() == [ForHeadDelimiter::Brace]
                && matches!(token.kind, TokenKind::Punctuator(Punctuator::Ellipsis))
            {
                has_object_rest = true;
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
                        return Some(BindingPatternScan {
                            following: lexer.next_token().ok()?,
                            has_object_rest,
                        });
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
                        return Some(BindingPatternScan {
                            following: lexer.next_token().ok()?,
                            has_object_rest,
                        });
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
        self.parse_for_binding_pattern(
            iteration_kind,
            declaration,
            is_const,
            BindingPatternKind::Array,
        )
    }

    pub(super) fn parse_for_object_binding_pattern(
        &mut self,
        iteration_kind: ForIterationKind,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
    ) -> Result<ForAssignmentTargetInfo, Error> {
        self.parse_for_binding_pattern(
            iteration_kind,
            declaration,
            is_const,
            BindingPatternKind::Object,
        )
    }

    fn parse_for_binding_pattern(
        &mut self,
        iteration_kind: ForIterationKind,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        pattern: BindingPatternKind,
    ) -> Result<ForAssignmentTargetInfo, Error> {
        if !matches!(
            declaration,
            ForAssignmentDeclaration::Var | ForAssignmentDeclaration::Lexical
        ) {
            return Err(Error::internal(
                "binding pattern received a non-declaration target",
            ));
        }
        match pattern {
            BindingPatternKind::Array => self.parse_array_binding_pattern(
                declaration,
                is_const,
                BindingSite::Iteration(iteration_kind),
            )?,
            BindingPatternKind::Object => self.parse_object_binding_pattern(
                declaration,
                is_const,
                BindingSite::Iteration(iteration_kind),
            )?,
        }
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
        site: BindingSite,
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
                if is_rest {
                    self.emit_array_binding_rest(0)?;
                } else {
                    self.emit_instruction(Instruction::ForOfNext(0))?;
                    self.emit_instruction(Instruction::Drop)?;
                }
                self.parse_nested_object_binding_element(declaration, is_const, site, is_rest)?;

                if self.is_punctuator(Punctuator::RightBracket) {
                    break;
                }
                if is_rest {
                    return Err(self.syntax_here("rest element must be the last one"));
                }
                self.expect_punctuator(Punctuator::Comma)?;
                continue;
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
            if identifier.escaped_reserved_word {
                return Err(Error::syntax(
                    "invalid destructuring target",
                    source_span(token.span),
                ));
            }
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

    /// Consume one already-evaluated value and initialize an ObjectBindingPattern.
    /// The source Object remains below each property operation until the final
    /// drop. Fixed and computed leaf reads deliberately prepare a sloppy `var`
    /// Reference before invoking the getter, while nested patterns fetch their
    /// outer value before recursively preparing any inner Reference.
    fn parse_object_binding_pattern(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        site: BindingSite,
    ) -> Result<(), Error> {
        let pattern_span = self.current().span;
        let has_rest = self.object_binding_has_rest();
        self.expect_punctuator(Punctuator::LeftBrace)?;
        self.emit_instruction_at(Instruction::ToObject, source_offset(pattern_span)?)?;
        if has_rest {
            // source exclusion -> exclusion source
            self.emit_instruction(Instruction::Object)?;
            self.emit_instruction(Instruction::Insert2)?;
            self.emit_instruction(Instruction::Drop)?;
        }

        while !self.is_punctuator(Punctuator::RightBrace) {
            if self.is_punctuator(Punctuator::Ellipsis) {
                self.parse_object_binding_rest(declaration, is_const, has_rest)?;
                break;
            }

            let property = self.parse_object_binding_property_name()?;
            let shorthand = matches!(
                &property,
                ObjectBindingPropertyKey::Fixed {
                    shorthand: Some(_),
                    ..
                }
            );
            if !shorthand {
                // Pinned QuickJS advances over the expected colon here rather
                // than validating it in `js_parse_property_name`. Preserve its
                // malformed-pattern error priority as well as the valid path.
                self.advance()?;
            }

            let computed_key_is_canonical =
                self.emit_object_binding_exclusion(&property, has_rest)?;

            let nested_pattern = if shorthand {
                None
            } else {
                self.nested_binding_pattern_kind(Punctuator::RightBrace)
            };
            if let Some(pattern) = nested_pattern {
                self.emit_nested_object_property_value(&property)?;
                self.parse_nested_binding_element(declaration, is_const, site, false, pattern)?;
            } else {
                self.parse_object_binding_leaf(
                    property,
                    declaration,
                    is_const,
                    computed_key_is_canonical,
                )?;
            }

            if self.is_punctuator(Punctuator::RightBrace) {
                break;
            }
            self.expect_punctuator(Punctuator::Comma)?;
        }

        self.expect_punctuator(Punctuator::RightBrace)?;
        self.emit_instruction(Instruction::Drop)?;
        if has_rest {
            self.emit_instruction(Instruction::Drop)?;
        }
        Ok(())
    }

    /// Add one already-evaluated property name to the exclusion object kept
    /// below the source. For computed names this is also the single observable
    /// `ToPropertyKey` conversion shared by exclusion and the following Get.
    fn emit_object_binding_exclusion(
        &mut self,
        property: &ObjectBindingPropertyKey<'source>,
        has_rest: bool,
    ) -> Result<bool, Error> {
        if !has_rest {
            return Ok(false);
        }

        match property {
            ObjectBindingPropertyKey::Fixed { key, token, .. } => {
                // exclusion source -> source exclusion
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::Null)?;
                let key = self.add_constant(IrConstant::Primitive(Value::String(key.clone())))?;
                self.emit_instruction_at(
                    Instruction::DefineField(key),
                    source_offset(token.span)?,
                )?;
                // source exclusion -> exclusion source
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
                Ok(false)
            }
            ObjectBindingPropertyKey::Computed { span } => {
                // exclusion source raw-key -> exclusion source key
                self.emit_instruction_at(Instruction::ToPropKey, source_offset(*span)?)?;
                // exclusion source key -> source exclusion key
                self.emit_instruction(Instruction::Perm3)?;
                self.emit_instruction(Instruction::Null)?;
                self.emit_instruction(Instruction::DefineArrayEl)?;
                // source exclusion key -> exclusion source key
                self.emit_instruction(Instruction::Perm3)?;
                Ok(true)
            }
        }
    }

    fn parse_object_binding_property_name(
        &mut self,
    ) -> Result<ObjectBindingPropertyKey<'source>, Error> {
        let token = self.current().clone();
        match token.kind.clone() {
            TokenKind::Identifier(identifier) => {
                let key = JsString::try_from_utf8(&identifier.value)?;
                self.advance()?;
                // QuickJS's `js_parse_property_name` permits shorthand only
                // for a non-reserved identifier. An escaped reserved spelling
                // remains an Identifier token in this lexer, but must stay on
                // the ordinary property-name path so the later target error
                // wins at the same token as upstream.
                let shorthand = (!identifier.escaped_reserved_word
                    && !self.is_punctuator(Punctuator::Colon))
                .then_some(identifier);
                Ok(ObjectBindingPropertyKey::Fixed {
                    key,
                    token,
                    shorthand,
                })
            }
            TokenKind::Keyword(keyword) => {
                self.advance()?;
                Ok(ObjectBindingPropertyKey::Fixed {
                    key: JsString::from_static(keyword.as_str()),
                    token,
                    shorthand: None,
                })
            }
            TokenKind::String(string) => {
                if self.current_ir().strict && string.has_legacy_octal_escape {
                    return Err(Error::syntax(
                        "legacy octal escapes are forbidden in strict mode",
                        source_span(token.span),
                    ));
                }
                self.advance()?;
                Ok(ObjectBindingPropertyKey::Fixed {
                    key: JsString::try_from_utf16(string.value.utf16)?,
                    token,
                    shorthand: None,
                })
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
                let key = parse_number(&number)
                    .map_err(|message| Error::syntax(message, source_span(token.span)))?
                    .to_js_string()?;
                Ok(ObjectBindingPropertyKey::Fixed {
                    key,
                    token,
                    shorthand: None,
                })
            }
            TokenKind::Punctuator(Punctuator::LeftBracket) => {
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                self.anonymous_function_definition = None;
                self.expect_punctuator(Punctuator::RightBracket)?;
                Ok(ObjectBindingPropertyKey::Computed { span: token.span })
            }
            TokenKind::PrivateIdentifier(_) => Err(Error::syntax(
                "invalid property name",
                source_span(token.span),
            )),
            _ => Err(Error::syntax(
                "invalid property name",
                source_span(token.span),
            )),
        }
    }

    fn nested_binding_pattern_kind(&self, closing: Punctuator) -> Option<BindingPatternKind> {
        let (pattern, following) = if self.is_punctuator(Punctuator::LeftBracket) {
            (
                BindingPatternKind::Array,
                self.array_binding_following_token(),
            )
        } else if self.is_punctuator(Punctuator::LeftBrace) {
            (
                BindingPatternKind::Object,
                self.object_binding_following_token(),
            )
        } else {
            return None;
        };
        let following = following?;
        match following.kind {
            TokenKind::Punctuator(Punctuator::Comma | Punctuator::Equal) => Some(pattern),
            TokenKind::Punctuator(punctuator) if punctuator == closing => Some(pattern),
            _ => None,
        }
    }

    fn emit_nested_object_property_value(
        &mut self,
        property: &ObjectBindingPropertyKey<'source>,
    ) -> Result<(), Error> {
        match property {
            ObjectBindingPropertyKey::Fixed { key, token, .. } => {
                let key = self.add_constant(IrConstant::Primitive(Value::String(key.clone())))?;
                self.emit_instruction_at(Instruction::GetField2(key), source_offset(token.span)?)?;
            }
            ObjectBindingPropertyKey::Computed { span } => {
                self.emit_instruction_at(Instruction::GetArrayEl2, source_offset(*span)?)?;
            }
        }
        self.anonymous_function_definition = None;
        Ok(())
    }

    fn parse_object_binding_leaf(
        &mut self,
        property: ObjectBindingPropertyKey<'source>,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        computed_key_is_canonical: bool,
    ) -> Result<(), Error> {
        let (fixed_key, property_span, shorthand) = match property {
            ObjectBindingPropertyKey::Fixed {
                key,
                token,
                shorthand,
            } => (
                Some(key),
                token.span,
                shorthand.map(|identifier| (token, identifier)),
            ),
            ObjectBindingPropertyKey::Computed { span } => (None, span, None),
        };

        let shorthand_binding = shorthand.is_some();
        let (token, identifier) = if let Some(binding) = shorthand {
            binding
        } else {
            let token = self.current().clone();
            let TokenKind::Identifier(identifier) = token.kind.clone() else {
                return Err(Error::syntax(
                    "invalid destructuring target",
                    source_span(token.span),
                ));
            };
            self.advance()?;
            (token, identifier)
        };
        if identifier.escaped_reserved_word {
            return Err(Error::syntax(
                "invalid destructuring target",
                source_span(token.span),
            ));
        }
        validate_identifier_reservation(
            &identifier,
            token.span,
            self.current_ir().strict,
            IdentifierContext::Variable,
        )?;
        let invalid_lexical_let =
            declaration == ForAssignmentDeclaration::Lexical && identifier.value == "let";
        if invalid_lexical_let {
            return Err(self.syntax_here("invalid lexical variable name"));
        }
        let name = identifier.value;
        if self.current_ir().strict && matches!(name.as_str(), "eval" | "arguments") {
            // For shorthand properties QuickJS has already advanced to the
            // token following the property name when it diagnoses this case.
            let span = if shorthand_binding {
                self.current().span
            } else {
                token.span
            };
            return Err(Error::syntax(
                "invalid destructuring target",
                source_span(span),
            ));
        }
        match declaration {
            ForAssignmentDeclaration::Lexical => self.register_lexical_binding(
                &name,
                token.span,
                self.current().span,
                is_const,
                false,
            )?,
            ForAssignmentDeclaration::Var => {
                self.register_var_binding(&name, token.span, self.current().span)?;
            }
            ForAssignmentDeclaration::Assignment => {
                unreachable!("object binding declaration was validated before parsing properties")
            }
        }

        let reference_scope = self.current_ir().current_scope;
        if fixed_key.is_none() && !computed_key_is_canonical {
            // A leaf computed key is canonicalized before a sloppy `var`
            // Reference is prepared and before the property getter runs.
            self.emit_instruction(Instruction::ToPropKey)?;
        }
        if declaration == ForAssignmentDeclaration::Var {
            self.emit_identifier_reference_inherited(
                name.clone(),
                token.span,
                reference_scope,
                IdentifierReferenceAccess::Prepare,
            )?;
            if fixed_key.is_some() {
                // source ref -> ref source
                self.emit_instruction(Instruction::Insert2)?;
            } else {
                // source key ref -> ref source key
                self.emit_instruction(Instruction::Insert3)?;
            }
            self.emit_instruction(Instruction::Drop)?;
        }

        if let Some(key) = fixed_key {
            let key = self.add_constant(IrConstant::Primitive(Value::String(key)))?;
            self.emit_instruction_at(Instruction::GetField2(key), source_offset(property_span)?)?;
        } else {
            self.emit_instruction_at(Instruction::GetArrayEl2, source_offset(property_span)?)?;
        }
        if declaration == ForAssignmentDeclaration::Var {
            // ref source value -> source ref value
            self.emit_instruction(Instruction::Perm3)?;
        }

        if self.consume_punctuator(Punctuator::Equal)? {
            self.emit_instruction(Instruction::Dup)?;
            self.emit_instruction(Instruction::Undefined)?;
            self.emit_instruction(Instruction::StrictEq)?;
            let has_value = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_assignment_allow_in()?;
            if self.anonymous_function_definition.take().is_some() {
                let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::try_from_utf8(&name)?,
                )))?;
                self.emit_instruction(Instruction::SetName(name_constant))?;
            }
            let has_value_target = self.current_ir().ops.len();
            self.patch_jump(has_value, has_value_target)?;
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
            self.emit_identifier_inherited(
                name,
                token.span,
                reference_scope,
                IdentifierAccess::Initialize,
            )?;
        }
        if shorthand_binding {
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_object_binding_rest(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        has_rest: bool,
    ) -> Result<(), Error> {
        let rest_span = self.current().span;
        self.advance()?;
        let token = self.current().clone();
        let TokenKind::Identifier(identifier) = token.kind.clone() else {
            return Err(Error::syntax(
                "invalid destructuring target",
                source_span(token.span),
            ));
        };
        if identifier.escaped_reserved_word {
            return Err(Error::syntax(
                "invalid destructuring target",
                source_span(token.span),
            ));
        }
        validate_identifier_reservation(
            &identifier,
            token.span,
            self.current_ir().strict,
            IdentifierContext::Variable,
        )?;
        if self.current_ir().strict && matches!(identifier.value.as_str(), "eval" | "arguments") {
            return Err(Error::syntax(
                "invalid destructuring target",
                source_span(token.span),
            ));
        }
        self.advance()?;
        if !self.is_punctuator(Punctuator::RightBrace) {
            return Err(self.syntax_here("assignment rest property must be last"));
        }
        if declaration == ForAssignmentDeclaration::Lexical && identifier.value == "let" {
            return Err(self.syntax_here("invalid lexical variable name"));
        }
        match declaration {
            ForAssignmentDeclaration::Lexical => self.register_lexical_binding(
                &identifier.value,
                token.span,
                self.current().span,
                is_const,
                false,
            )?,
            ForAssignmentDeclaration::Var => {
                self.register_var_binding(&identifier.value, token.span, self.current().span)?;
            }
            ForAssignmentDeclaration::Assignment => {
                unreachable!("object binding declaration was validated before parsing rest")
            }
        }
        if !has_rest {
            return Err(Error::internal(
                "object rest binding was absent from its pattern skip-scan",
            ));
        }

        let reference_scope = self.current_ir().current_scope;
        if declaration == ForAssignmentDeclaration::Var {
            // QuickJS prepares a potentially dynamic sloppy-var Reference
            // before allocating/enumerating the rest object.
            self.emit_identifier_reference_inherited(
                identifier.value.clone(),
                token.span,
                reference_scope,
                IdentifierReferenceAccess::Prepare,
            )?;
        }
        self.emit_instruction(Instruction::Object)?;
        let (source_depth, excluded_depth) = if declaration == ForAssignmentDeclaration::Var {
            (2, 3)
        } else {
            (1, 2)
        };
        self.emit_instruction_at(
            Instruction::CopyDataPropertiesExcluded {
                target_depth: 0,
                source_depth,
                excluded_depth,
            },
            source_offset(rest_span)?,
        )?;
        if declaration == ForAssignmentDeclaration::Var {
            self.emit_identifier_reference_inherited(
                identifier.value,
                token.span,
                reference_scope,
                IdentifierReferenceAccess::Set,
            )?;
            self.emit_instruction(Instruction::Drop)?;
        } else {
            self.emit_identifier_inherited(
                identifier.value,
                token.span,
                reference_scope,
                IdentifierAccess::Initialize,
            )?;
        }
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
        site: BindingSite,
        is_rest: bool,
    ) -> Result<(), Error> {
        self.parse_nested_binding_element(
            declaration,
            is_const,
            site,
            is_rest,
            BindingPatternKind::Array,
        )
    }

    fn parse_nested_object_binding_element(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        site: BindingSite,
        is_rest: bool,
    ) -> Result<(), Error> {
        self.parse_nested_binding_element(
            declaration,
            is_const,
            site,
            is_rest,
            BindingPatternKind::Object,
        )
    }

    fn parse_nested_binding_element(
        &mut self,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        site: BindingSite,
        is_rest: bool,
        pattern: BindingPatternKind,
    ) -> Result<(), Error> {
        let value_depth = self.current_ir().stack_depth;
        if value_depth == 0 {
            return Err(Error::internal(
                "nested binding pattern has no value to consume",
            ));
        }

        if is_rest {
            self.parse_binding_pattern(pattern, declaration, is_const, site)?;
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

        self.parse_binding_pattern(pattern, declaration, is_const, site)?;
        self.require_stack_depth(value_depth - 1, "nested binding pattern assignment")?;

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
            self.require_stack_depth(value_depth, "nested binding pattern initializer")?;
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

    fn parse_binding_pattern(
        &mut self,
        pattern: BindingPatternKind,
        declaration: ForAssignmentDeclaration,
        is_const: bool,
        site: BindingSite,
    ) -> Result<(), Error> {
        match pattern {
            BindingPatternKind::Array => {
                self.parse_array_binding_pattern(declaration, is_const, site)
            }
            BindingPatternKind::Object => {
                self.parse_object_binding_pattern(declaration, is_const, site)
            }
        }
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
