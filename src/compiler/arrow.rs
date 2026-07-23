use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ArrowHead {
    Identifier,
    Parenthesized,
}

impl<'source> Parser<'source> {
    pub(super) fn parse_arrow_function(&mut self, head: ArrowHead) -> Result<(), Error> {
        self.parse_arrow_function_with_kind(
            head,
            self.current().span,
            self.lexer.context(),
            BytecodeFunctionKind::Normal,
        )
    }

    pub(super) fn parse_async_arrow_function(&mut self, head: ArrowHead) -> Result<(), Error> {
        let function_span = self.current().span;
        let parent_context = self.lexer.context();
        // Pinned QuickJS commits the token immediately after `async` in the
        // parent function before creating the async arrow. Keep that token
        // intact so the single-parameter `async await =>` asymmetry survives.
        self.advance()?;
        self.parse_arrow_function_with_kind(
            head,
            function_span,
            parent_context,
            BytecodeFunctionKind::Async,
        )
    }

    fn parse_arrow_function_with_kind(
        &mut self,
        head: ArrowHead,
        function_span: Span,
        parent_context: LexContext,
        execution_kind: BytecodeFunctionKind,
    ) -> Result<(), Error> {
        if !matches!(
            execution_kind,
            BytecodeFunctionKind::Normal | BytecodeFunctionKind::Async
        ) {
            return Err(Error::internal(
                "arrow parser received an invalid execution kind",
            ));
        }
        let parent = self.current_function;
        let parent_strict = self.functions[parent].strict;
        let mut parameter_tokens = Vec::new();
        let child = self.functions.len();
        let parent_scope = self.functions[parent].current_scope;
        let parent_execution_kind = self.functions[parent].execution_kind;
        let parent_is_class_static_block = self.functions[parent].class_initializer_kind
            == Some(ClassInitializerKind::StaticBlock);
        let super_capabilities = SuperCapabilities {
            super_call_allowed: self.functions[parent].super_call_allowed,
            super_allowed: self.functions[parent].super_allowed,
        };
        self.functions.push(FunctionIr::new(
            Some(ParentLink {
                function: parent,
                definition_scope: parent_scope,
            }),
            FunctionKind::Arrow,
            FunctionSourceInfo {
                span: function_span,
                definition: source_offset(function_span)?,
                range: None,
            },
            FunctionIrOptions {
                function_name: None,
                private_name_binding: false,
                class_constructor: false,
                derived_class_constructor: false,
                defined_argument_count: 0,
                has_simple_parameter_list: true,
                rest_parameter: None,
                parameters: Vec::new(),
                strict: parent_strict,
                super_capabilities,
            },
        )?);
        self.functions[child].execution_kind = execution_kind;
        self.functions[child].arguments_forbidden = self.functions[parent].arguments_forbidden;
        // ClassStaticBlock ContainsAwait includes immediate arrow parameter
        // initializer expressions. A nested arrow is another function
        // boundary, so this authority comes from the immediate static-block
        // parent rather than being copied transitively through arrows.
        self.functions[child].await_forbidden = parent_is_class_static_block;
        self.functions[child].await_binding_forbidden = parent_is_class_static_block;
        self.current_function = child;

        let mut parameter_context = parent_context;
        parameter_context.strict = parent_strict;
        // QuickJS classifies arrow formal parameters from the new arrow plus
        // its immediate parse parent, not from a flattened lexer context.
        // Preserve the already-read current token for its single-parameter
        // quirk, but reset future yield/await classification at every arrow
        // boundary before parsing parenthesized parameters and defaults.
        parameter_context.generator = matches!(
            parent_execution_kind,
            BytecodeFunctionKind::Generator | BytecodeFunctionKind::AsyncGenerator
        );
        parameter_context.async_function = execution_kind == BytecodeFunctionKind::Async
            || matches!(
                parent_execution_kind,
                BytecodeFunctionKind::Async | BytecodeFunctionKind::AsyncGenerator
            )
            || parent_is_class_static_block;
        self.set_future_lex_context(parameter_context);

        match head {
            ArrowHead::Identifier => {
                let token = self.current().clone();
                let TokenKind::Identifier(identifier) = token.kind else {
                    return Err(Error::internal(
                        "identifier arrow lookahead lost its parameter token",
                    ));
                };
                validate_identifier(&identifier, token.span, false, IdentifierContext::Argument)?;
                self.register_plain_identifier_parameter(identifier.value.clone(), token.span)?;
                parameter_tokens.push((identifier, token.span));
                self.advance()?;
            }
            ArrowHead::Parenthesized => {
                let parameter_scan = self.parenthesized_parameter_scan();
                if parameter_scan.is_some_and(|scan| scan.has_assignment) {
                    self.activate_parameter_environment_from_scan(
                        parameter_scan.and_then(|scan| scan.bound_name_count),
                    )?;
                }
                self.expect_punctuator(Punctuator::LeftParen)?;
                if !self.consume_punctuator(Punctuator::RightParen)? {
                    loop {
                        let is_rest = self.consume_punctuator(Punctuator::Ellipsis)?;
                        let pattern = match self.current().kind {
                            TokenKind::Punctuator(Punctuator::LeftBracket) => Some(true),
                            TokenKind::Punctuator(Punctuator::LeftBrace) => Some(false),
                            _ => None,
                        };
                        if let Some(array_pattern) = pattern {
                            let span = self.current().span;
                            self.activate_pattern_parameter_initialization()?;
                            if is_rest {
                                let start = self.register_rest_pattern_parameter()?;
                                let has_initializer = if array_pattern {
                                    self.parse_array_rest_parameter_binding_pattern(start)?
                                } else {
                                    self.parse_object_rest_parameter_binding_pattern(start)?
                                };
                                self.finish_pattern_parameter_length(start, has_initializer)?;
                                if !self.is_punctuator(Punctuator::RightParen) {
                                    return Err(self.syntax_here("expecting ')'"));
                                }
                                self.advance()?;
                                break;
                            }
                            let argument = self.append_pattern_parameter(span)?;
                            let has_initializer = if array_pattern {
                                self.parse_array_parameter_binding_pattern(argument)?
                            } else {
                                self.parse_object_parameter_binding_pattern(argument)?
                            };
                            self.finish_pattern_parameter_length(argument, has_initializer)?;
                            if self.consume_punctuator(Punctuator::RightParen)? {
                                break;
                            }
                            self.expect_punctuator(Punctuator::Comma)?;
                            if self.consume_punctuator(Punctuator::RightParen)? {
                                break;
                            }
                            continue;
                        }
                        let token = self.current().clone();
                        let TokenKind::Identifier(identifier) = token.kind else {
                            return Err(self.syntax_here("missing formal parameter"));
                        };
                        validate_identifier(
                            &identifier,
                            token.span,
                            false,
                            IdentifierContext::Argument,
                        )?;
                        parameter_tokens.push((identifier, token.span));
                        let parameter = parameter_tokens
                            .last()
                            .map(|(identifier, _)| identifier.value.clone())
                            .ok_or_else(|| Error::internal("arrow parameter disappeared"))?;
                        self.advance()?;
                        if is_rest {
                            self.register_rest_identifier_parameter(parameter, token.span)?;
                            if !self.is_punctuator(Punctuator::RightParen) {
                                return Err(self.syntax_here("expecting ')'"));
                            }
                            self.advance()?;
                            break;
                        }
                        if self.is_punctuator(Punctuator::Equal) {
                            self.parse_default_identifier_parameter(parameter, token.span)?;
                        } else {
                            self.register_plain_identifier_parameter(parameter, token.span)?;
                        }
                        if self.consume_punctuator(Punctuator::RightParen)? {
                            break;
                        }
                        self.expect_punctuator(Punctuator::Comma)?;
                        if self.consume_punctuator(Punctuator::RightParen)? {
                            break;
                        }
                    }
                }
            }
        }

        if !self.is_punctuator(Punctuator::Arrow) || self.current().line_terminator_before {
            return Err(self.syntax_here("expecting '=>'"));
        }
        self.advance_expression_start()?;
        let mut child_context = parent_context;
        child_context.strict = parent_strict;
        child_context.generator = false;
        child_context.async_function = execution_kind == BytecodeFunctionKind::Async;
        self.relex_current_with_context(child_context)?;
        self.functions[child].await_forbidden = false;
        self.functions[child].await_binding_forbidden = false;
        let block_body = self.is_punctuator(Punctuator::LeftBrace);
        if block_body {
            self.advance()?;
        }
        let has_use_strict = if block_body {
            self.directive_prologue_has_use_strict(self.cursor, parent_strict)?
        } else {
            false
        };
        let strict = parent_strict || has_use_strict;
        if block_body {
            child_context.strict = strict;
            self.relex_current_with_context(child_context)?;
        }
        let has_simple_parameter_list = self.functions[child].has_simple_parameter_list;
        if has_use_strict && !has_simple_parameter_list {
            return Err(Error::syntax(
                "\"use strict\" not allowed in function with default or destructuring parameter",
                source_span(self.current().span),
            ));
        }
        if strict {
            for (identifier, span) in &parameter_tokens {
                validate_identifier(identifier, *span, true, IdentifierContext::Argument)?;
            }
        }
        let parameters = &self.functions[child].parameter_names;
        for (index, parameter) in parameters.iter().enumerate() {
            if parameters[..index].contains(parameter) {
                return Err(Error::syntax(
                    "duplicate argument names not allowed in this context",
                    source_span(self.current().span),
                ));
            }
        }

        self.functions[child].strict = strict;
        self.finish_identifier_parameter_environment()?;
        self.functions[child].in_function_body = true;

        let range_end = if block_body {
            self.parse_function_body()?;
            let closing_brace = self.current().span;
            self.relex_current_with_context(parent_context)?;
            self.expect_punctuator(Punctuator::RightBrace)?;
            closing_brace.end.byte_offset
        } else {
            self.parse_assignment()?;
            self.emit_instruction(Instruction::Return)?;
            let range_end = self
                .tokens
                .get(self.cursor.saturating_sub(1))
                .map_or(self.current().span.start.byte_offset, |token| {
                    token.span.end.byte_offset
                });
            self.relex_current_with_context(parent_context)?;
            range_end
        };
        self.functions[child].source.range = Some(
            source_offset(function_span)?
                ..SourceOffset::try_from_usize(range_end)
                    .map_err(|error| Error::internal(error.to_string()))?,
        );
        self.current_function = parent;
        let constant = self.add_constant(IrConstant::Child(child))?;
        self.emit(IrOp::MakeClosure(constant))?;
        self.anonymous_function_definition = Some(AnonymousFunctionDefinition::Function);
        Ok(())
    }

    /// Non-committing QuickJS-style ArrowParameters probe. The scanner owns
    /// no IR or scope state: it balances the cover grammar, respects template
    /// substitutions and RegExp lexical goals, and accepts `=>` only when no
    /// LineTerminator separates it from the closing parenthesis.
    fn parenthesized_arrow_ahead(&self, opening: Span) -> bool {
        let mut lexer = self.lexer.clone();
        lexer.seek(opening.start);
        let Ok(first) = lexer.next_token_with_goal(LexicalGoal::Div) else {
            return false;
        };
        if !matches!(first.kind, TokenKind::Punctuator(Punctuator::LeftParen)) {
            return false;
        }

        let mut delimiters = vec![ForHeadDelimiter::Parenthesis];
        let mut goal = LexicalGoal::Div;
        let mut regexp_allowed = true;
        loop {
            let requested_goal = goal;
            goal = LexicalGoal::Div;
            let Ok(mut token) = lexer.next_token_with_goal(requested_goal) else {
                return false;
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
                    return false;
                };
                token = regexp;
            }

            match &token.kind {
                TokenKind::Punctuator(Punctuator::LeftParen) => {
                    if delimiters.len() >= 255 {
                        return false;
                    }
                    delimiters.push(ForHeadDelimiter::Parenthesis);
                }
                TokenKind::Punctuator(Punctuator::LeftBracket) => {
                    if delimiters.len() >= 255 {
                        return false;
                    }
                    delimiters.push(ForHeadDelimiter::Bracket);
                }
                TokenKind::Punctuator(Punctuator::LeftBrace) => {
                    if delimiters.len() >= 255 {
                        return false;
                    }
                    delimiters.push(ForHeadDelimiter::Brace);
                }
                TokenKind::Punctuator(Punctuator::RightParen) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Parenthesis) {
                        return false;
                    }
                    if delimiters.is_empty() {
                        let Ok(arrow) = lexer.next_token_with_goal(LexicalGoal::Div) else {
                            return false;
                        };
                        return !arrow.line_terminator_before
                            && matches!(arrow.kind, TokenKind::Punctuator(Punctuator::Arrow));
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBracket) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Bracket) {
                        return false;
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBrace) => {
                    if delimiters.last() == Some(&ForHeadDelimiter::Template) {
                        goal = LexicalGoal::TemplateContinuation;
                        regexp_allowed = true;
                        continue;
                    }
                    if delimiters.pop() != Some(ForHeadDelimiter::Brace) {
                        return false;
                    }
                }
                TokenKind::Template(part) => match part.kind {
                    TemplatePartKind::Head => {
                        if delimiters.len() >= 255 {
                            return false;
                        }
                        delimiters.push(ForHeadDelimiter::Template);
                    }
                    TemplatePartKind::Middle => {
                        if delimiters.last() != Some(&ForHeadDelimiter::Template) {
                            return false;
                        }
                    }
                    TemplatePartKind::Tail => {
                        if delimiters.pop() != Some(ForHeadDelimiter::Template) {
                            return false;
                        }
                    }
                    TemplatePartKind::NoSubstitution => {}
                },
                TokenKind::Eof => return false,
                _ => {}
            }
            regexp_allowed = for_head_regexp_allowed_after(&token.kind);
        }
    }

    pub(super) fn arrow_head_ahead(&self) -> Option<ArrowHead> {
        match &self.current().kind {
            TokenKind::Identifier(_) => {
                let mut lexer = self.lexer.clone();
                lexer.seek(self.current().span.end);
                let next = lexer.next_token_with_goal(LexicalGoal::Div).ok()?;
                (!next.line_terminator_before
                    && matches!(next.kind, TokenKind::Punctuator(Punctuator::Arrow)))
                .then_some(ArrowHead::Identifier)
            }
            TokenKind::Punctuator(Punctuator::LeftParen)
                if self.parenthesized_arrow_ahead(self.current().span) =>
            {
                Some(ArrowHead::Parenthesized)
            }
            _ => None,
        }
    }

    /// A reserved word followed by `=>` is an attempted ArrowFunction head,
    /// not an unimplemented statement/expression form. Preserve that syntax
    /// error identity before the primary-expression frontier can classify the
    /// keyword as a missing feature.
    pub(super) fn reserved_arrow_head_ahead(&self) -> bool {
        if !matches!(self.current().kind, TokenKind::Keyword(_)) {
            return false;
        }
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let Ok(arrow) = lexer.next_token_with_goal(LexicalGoal::Div) else {
            return false;
        };
        !arrow.line_terminator_before
            && matches!(arrow.kind, TokenKind::Punctuator(Punctuator::Arrow))
    }

    pub(super) fn async_arrow_ahead(&self) -> Option<ArrowHead> {
        let TokenKind::Identifier(identifier) = &self.current().kind else {
            return None;
        };
        if identifier.value != "async" || identifier.has_escape {
            return None;
        }
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let Ok(parameter) = lexer.next_token_with_goal(LexicalGoal::Div) else {
            return None;
        };
        if parameter.line_terminator_before {
            return None;
        }
        match parameter.kind {
            TokenKind::Identifier(identifier) if !identifier.escaped_reserved_word => {
                let Ok(arrow) = lexer.next_token_with_goal(LexicalGoal::Div) else {
                    return None;
                };
                (!arrow.line_terminator_before
                    && matches!(arrow.kind, TokenKind::Punctuator(Punctuator::Arrow)))
                .then_some(ArrowHead::Identifier)
            }
            TokenKind::Punctuator(Punctuator::LeftParen) => self
                .parenthesized_arrow_ahead(parameter.span)
                .then_some(ArrowHead::Parenthesized),
            _ => None,
        }
    }
}
