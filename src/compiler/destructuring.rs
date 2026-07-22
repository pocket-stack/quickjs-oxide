use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingSite {
    Declaration,
    Iteration(ForIterationKind),
    Catch,
    Parameter,
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
    has_assignment: bool,
}

/// Prepared DestructuringAssignmentTarget.  The operands represented here
/// stay above the active iterator record until its next value has been
/// produced; the final write deliberately uses QuickJS's NOKEEP shape.
enum DestructuringAssignmentReference {
    Identifier(IdentifierReference),
    Member(MemberReference),
}

impl DestructuringAssignmentReference {
    fn depth(&self) -> u8 {
        match self {
            Self::Identifier(reference) => u8::from(reference.object_environment),
            Self::Member(MemberReference::Field { .. }) => 1,
            Self::Member(MemberReference::Computed { .. }) => 2,
            Self::Member(MemberReference::Super { .. }) => 3,
        }
    }

    fn inferred_name(&self) -> Option<&str> {
        match self {
            Self::Identifier(reference) => Some(&reference.name),
            Self::Member(_) => None,
        }
    }

    fn site(&self) -> Result<SourceOffset, Error> {
        match self {
            Self::Identifier(reference) => source_offset(reference.span),
            Self::Member(
                MemberReference::Field { site, .. }
                | MemberReference::Computed { site }
                | MemberReference::Super { site },
            ) => Ok(*site),
        }
    }
}

impl<'source> Parser<'source> {
    /// Lower a direct AssignmentPattern while preserving the complete
    /// right-hand-side value as the AssignmentExpression result.  Parsing is
    /// control-inverted: the pattern fragment is emitted first, then the RHS
    /// is parsed in source order and duplicated before jumping back into it.
    pub(super) fn parse_array_assignment_expression(&mut self) -> Result<(), Error> {
        self.parse_assignment_pattern_expression(BindingPatternKind::Array)
    }

    pub(super) fn parse_object_assignment_expression(&mut self) -> Result<(), Error> {
        self.parse_assignment_pattern_expression(BindingPatternKind::Object)
    }

    fn parse_assignment_pattern_expression(
        &mut self,
        pattern: BindingPatternKind,
    ) -> Result<(), Error> {
        let entry_depth = self.current_ir().stack_depth;
        let initializer_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let assignment_target = self.current_ir().ops.len();

        // The backward edge carries the retained expression result below the
        // separate value consumed by the destructuring pattern.
        self.current_ir_mut().stack_depth = entry_depth
            .checked_add(2)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.parse_assignment_pattern(pattern)?;
        self.require_stack_depth(entry_depth + 1, "assignment pattern")?;
        let done_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;

        let initializer_target = self.current_ir().ops.len();
        self.patch_jump(initializer_jump, initializer_target)?;
        self.current_ir_mut().stack_depth = entry_depth;
        self.expect_punctuator(Punctuator::Equal)?;
        // QuickJS restores PF_IN_ACCEPTED for a destructuring RHS even when
        // the enclosing expression is an ExpressionNoIn.
        self.parse_assignment_allow_in()?;
        self.anonymous_function_definition = None;
        self.emit_instruction(Instruction::Dup)?;
        self.require_stack_depth(entry_depth + 2, "assignment pattern initializer")?;
        self.emit_instruction(Instruction::Goto(
            u32::try_from(assignment_target)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
        ))?;

        let done_target = self.current_ir().ops.len();
        self.patch_jump(done_jump, done_target)?;
        self.current_ir_mut().stack_depth = entry_depth + 1;
        self.current_ir_mut().last_member_reference = None;
        self.current_ir_mut().last_identifier_reference = None;
        Ok(())
    }

    /// Consume the value delivered by a synchronous for-in/of edge.  Unlike
    /// the direct expression path there is no retained RHS copy beneath the
    /// surrounding enumeration record.
    pub(super) fn parse_for_array_assignment_pattern(
        &mut self,
        _iteration_kind: ForIterationKind,
    ) -> Result<ForAssignmentTargetInfo, Error> {
        self.parse_for_assignment_pattern(BindingPatternKind::Array)
    }

    pub(super) fn parse_for_object_assignment_pattern(
        &mut self,
        _iteration_kind: ForIterationKind,
    ) -> Result<ForAssignmentTargetInfo, Error> {
        self.parse_for_assignment_pattern(BindingPatternKind::Object)
    }

    fn parse_for_assignment_pattern(
        &mut self,
        pattern: BindingPatternKind,
    ) -> Result<ForAssignmentTargetInfo, Error> {
        self.parse_assignment_pattern(pattern)?;
        Ok(ForAssignmentTargetInfo {
            declaration: ForAssignmentDeclaration::Assignment,
            var_initializer: None,
            is_destructuring: true,
        })
    }

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

    /// Consume the exception value delivered at a Catch target and initialize
    /// every BoundName as a mutable lexical in the statically selected catch
    /// scope. The exceptional edge deliberately skips that scope's EnterScope
    /// instruction, matching the pinned QuickJS catch-entry layout.
    /// QuickJS deliberately uses TOK_LET here rather than TOK_CATCH: only a
    /// simple `catch (identifier)` receives the Annex-B var-redeclaration
    /// exception represented by `IrBinding::is_catch_parameter`.
    pub(super) fn parse_catch_array_binding_pattern(&mut self) -> Result<(), Error> {
        self.parse_catch_binding_pattern(BindingPatternKind::Array)
    }

    pub(super) fn parse_catch_object_binding_pattern(&mut self) -> Result<(), Error> {
        self.parse_catch_binding_pattern(BindingPatternKind::Object)
    }

    fn parse_catch_binding_pattern(&mut self, pattern: BindingPatternKind) -> Result<(), Error> {
        let entry_depth = self.current_ir().stack_depth;
        let expected_depth = entry_depth
            .checked_sub(1)
            .ok_or_else(|| Error::internal("catch binding pattern has no exception value"))?;
        // QuickJS invokes the common destructuring-element parser with
        // hasval=true and allow_initializer=true for a CatchParameter, so the
        // complete pattern itself may carry a default initializer.
        self.parse_nested_binding_element(
            ForAssignmentDeclaration::Lexical,
            false,
            BindingSite::Catch,
            false,
            pattern,
        )?;
        self.require_stack_depth(expected_depth, "catch binding pattern")
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

    pub(super) fn array_assignment_pattern_ahead(&self) -> bool {
        self.array_binding_following_token()
            .is_some_and(|token| matches!(token.kind, TokenKind::Punctuator(Punctuator::Equal)))
    }

    pub(super) fn object_assignment_pattern_ahead(&self) -> bool {
        self.object_binding_following_token()
            .is_some_and(|token| matches!(token.kind, TokenKind::Punctuator(Punctuator::Equal)))
    }

    pub(super) fn for_array_assignment_pattern_ahead(
        &self,
        iteration_kind: ForIterationKind,
    ) -> bool {
        self.array_binding_following_token()
            .is_some_and(|token| Self::is_iteration_delimiter(&token, iteration_kind))
    }

    pub(super) fn for_object_assignment_pattern_ahead(
        &self,
        iteration_kind: ForIterationKind,
    ) -> bool {
        self.object_binding_following_token()
            .is_some_and(|token| Self::is_iteration_delimiter(&token, iteration_kind))
    }

    fn is_iteration_delimiter(token: &Token<'source>, iteration_kind: ForIterationKind) -> bool {
        match iteration_kind {
            ForIterationKind::In => matches!(token.kind, TokenKind::Keyword(Keyword::In)),
            ForIterationKind::Of => matches!(
                &token.kind,
                TokenKind::Identifier(identifier)
                    if identifier.value == "of" && !identifier.has_escape
            ),
        }
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

    /// QuickJS pre-scans the complete parenthesized FormalParameters before
    /// parsing the first binding. Any standalone `=` token at any delimiter
    /// depth selects the independent parentless argument scope.
    pub(super) fn parenthesized_parameter_has_assignment(&self) -> Option<bool> {
        let mut assignment_seen = false;
        self.binding_pattern_scan_recording_assignment(Punctuator::LeftParen, &mut assignment_seen)
            .map(|scan| scan.has_assignment)
            // QuickJS stops its skip scan at the 256-level delimiter bound but
            // still publishes bits accumulated before that point. In particular,
            // an earlier `=` must create the argument scope even when the suffix is
            // too deep for the lookahead scanner to finish.
            .or_else(|| assignment_seen.then_some(true))
    }

    fn binding_pattern_following_token(&self, opening: Punctuator) -> Option<Token<'source>> {
        self.binding_pattern_scan(opening)
            .map(|scan| scan.following)
    }

    fn binding_pattern_scan(&self, opening: Punctuator) -> Option<BindingPatternScan<'source>> {
        let mut assignment_seen = false;
        self.binding_pattern_scan_recording_assignment(opening, &mut assignment_seen)
    }

    fn binding_pattern_scan_recording_assignment(
        &self,
        opening: Punctuator,
        assignment_seen: &mut bool,
    ) -> Option<BindingPatternScan<'source>> {
        if !self.is_punctuator(opening) {
            return None;
        }
        let root = match opening {
            Punctuator::LeftParen => ForHeadDelimiter::Parenthesis,
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
        let mut has_assignment = false;

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
            if matches!(token.kind, TokenKind::Punctuator(Punctuator::Equal)) {
                has_assignment = true;
                *assignment_seen = true;
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
                    if root == ForHeadDelimiter::Parenthesis && delimiters.is_empty() {
                        return Some(BindingPatternScan {
                            following: lexer.next_token().ok()?,
                            has_object_rest,
                            has_assignment,
                        });
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
                            has_assignment,
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
                            has_assignment,
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

    pub(super) fn parse_array_parameter_binding_pattern(
        &mut self,
        argument: u16,
    ) -> Result<bool, Error> {
        self.parse_parameter_binding_pattern(argument, BindingPatternKind::Array)
    }

    pub(super) fn parse_object_parameter_binding_pattern(
        &mut self,
        argument: u16,
    ) -> Result<bool, Error> {
        self.parse_parameter_binding_pattern(argument, BindingPatternKind::Object)
    }

    pub(super) fn parse_array_rest_parameter_binding_pattern(
        &mut self,
        start: u16,
    ) -> Result<bool, Error> {
        self.parse_rest_parameter_binding_pattern(start, BindingPatternKind::Array)
    }

    pub(super) fn parse_object_rest_parameter_binding_pattern(
        &mut self,
        start: u16,
    ) -> Result<bool, Error> {
        self.parse_rest_parameter_binding_pattern(start, BindingPatternKind::Object)
    }

    fn parse_rest_parameter_binding_pattern(
        &mut self,
        start: u16,
        pattern: BindingPatternKind,
    ) -> Result<bool, Error> {
        self.emit_instruction(Instruction::Rest(start))?;
        self.parse_parameter_binding_pattern_value(pattern)
    }

    fn parse_parameter_binding_pattern(
        &mut self,
        argument: u16,
        pattern: BindingPatternKind,
    ) -> Result<bool, Error> {
        self.emit_instruction(Instruction::GetArg(argument))?;
        self.parse_parameter_binding_pattern_value(pattern)
    }

    fn parse_parameter_binding_pattern_value(
        &mut self,
        pattern: BindingPatternKind,
    ) -> Result<bool, Error> {
        let opening = match pattern {
            BindingPatternKind::Array => Punctuator::LeftBracket,
            BindingPatternKind::Object => Punctuator::LeftBrace,
        };
        let has_initializer = self
            .binding_pattern_following_token(opening)
            .is_some_and(|token| matches!(token.kind, TokenKind::Punctuator(Punctuator::Equal)));
        if !has_initializer {
            self.parse_binding_pattern(
                pattern,
                ForAssignmentDeclaration::Var,
                false,
                BindingSite::Parameter,
            )?;
            return Ok(false);
        }

        let value_depth = self.current_ir().stack_depth;
        self.emit_instruction(Instruction::Dup)?;
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::StrictEq)?;
        let use_initializer = self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
        let assignment_target = self.current_ir().ops.len();

        self.parse_binding_pattern(
            pattern,
            ForAssignmentDeclaration::Var,
            false,
            BindingSite::Parameter,
        )?;
        self.require_stack_depth(value_depth - 1, "parameter BindingPattern assignment")?;
        self.expect_punctuator(Punctuator::Equal)?;

        let done_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let initializer_target = self.current_ir().ops.len();
        self.patch_jump(use_initializer, initializer_target)?;
        self.current_ir_mut().stack_depth = value_depth;
        self.emit_instruction(Instruction::Drop)?;
        self.parse_assignment_allow_in()?;
        self.anonymous_function_definition = None;
        self.require_stack_depth(value_depth, "parameter BindingPattern initializer")?;
        self.emit_instruction(Instruction::Goto(
            u32::try_from(assignment_target)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
        ))?;

        let done_target = self.current_ir().ops.len();
        self.patch_jump(done_jump, done_target)?;
        self.current_ir_mut().stack_depth = value_depth - 1;
        Ok(true)
    }

    fn parse_assignment_pattern(&mut self, pattern: BindingPatternKind) -> Result<(), Error> {
        match pattern {
            BindingPatternKind::Array => self.parse_array_assignment_pattern(),
            BindingPatternKind::Object => self.parse_object_assignment_pattern(),
        }
    }

    /// Consume one already-evaluated value with an ArrayAssignmentPattern.
    /// Each leaf prepares its complete lvalue before IteratorStep; therefore
    /// getters, computed-key expressions, and authored `with` selection have
    /// the same ordering as pinned QuickJS.  The prepared operand count is
    /// passed to ForOfNext instead of being encoded through synthetic locals.
    fn parse_array_assignment_pattern(&mut self) -> Result<(), Error> {
        self.expect_punctuator(Punctuator::LeftBracket)?;
        self.emit_instruction(Instruction::ForOfStart)?;

        while !self.is_punctuator(Punctuator::RightBracket) {
            let is_rest = self.consume_punctuator(Punctuator::Ellipsis)?;
            let element_span = self.current().span;
            if is_rest
                && matches!(
                    self.current().kind,
                    TokenKind::Punctuator(Punctuator::Comma | Punctuator::RightBracket)
                )
            {
                return Err(Error::syntax(
                    "missing binding pattern...",
                    source_span(element_span),
                ));
            }

            let nested_object_follow = self
                .is_punctuator(Punctuator::LeftBrace)
                .then(|| self.object_binding_following_token())
                .flatten();
            let nested_array_follow = self
                .is_punctuator(Punctuator::LeftBracket)
                .then(|| self.array_binding_following_token())
                .flatten();
            let nested_pattern = self.nested_binding_pattern_kind(Punctuator::RightBracket);
            if let Some(pattern) = nested_pattern {
                let nested_follow = match pattern {
                    BindingPatternKind::Array => nested_array_follow.as_ref(),
                    BindingPatternKind::Object => nested_object_follow.as_ref(),
                };
                if is_rest
                    && nested_follow.is_some_and(|token| {
                        matches!(token.kind, TokenKind::Punctuator(Punctuator::Equal))
                    })
                {
                    return Err(Error::syntax(
                        "rest element cannot have a default value",
                        source_span(element_span),
                    ));
                }
                if is_rest {
                    // The outer rest drain runs before the nested pattern is
                    // entered, so QuickJS retains the preceding outer marker.
                    self.emit_array_rest(0, None)?;
                } else {
                    // The parent step runs before recursion and therefore
                    // retains the preceding outer marker.
                    self.emit_instruction(Instruction::ForOfNext(0))?;
                    self.emit_instruction(Instruction::Drop)?;
                }
                self.parse_nested_assignment_element(pattern, is_rest)?;
            } else if !is_rest && self.consume_punctuator(Punctuator::Comma)? {
                // QuickJS leaves an elision unmarked: a leading hole inherits
                // the pattern opener, while a later hole retains the most
                // recent target/default site.
                self.emit_instruction(Instruction::ForOfNext(0))?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::Drop)?;
                continue;
            } else {
                self.parse_array_assignment_leaf(is_rest)?;
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
        self.anonymous_function_definition = None;
        Ok(())
    }

    fn parse_array_assignment_leaf(&mut self, is_rest: bool) -> Result<(), Error> {
        let target = self.parse_destructuring_assignment_reference()?;
        let reference_depth = target.depth();
        let inferred_name = target.inferred_name().map(str::to_owned);
        let target_site = target.site()?;

        if is_rest {
            self.emit_array_assignment_rest(reference_depth, target_site)?;
        } else {
            self.emit_instruction_at(Instruction::ForOfNext(reference_depth), target_site)?;
            self.emit_instruction(Instruction::Drop)?;
            if self.consume_punctuator(Punctuator::Equal)? {
                self.emit_instruction(Instruction::Dup)?;
                self.emit_instruction(Instruction::Undefined)?;
                self.emit_instruction(Instruction::StrictEq)?;
                let has_value = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
                self.emit_instruction(Instruction::Drop)?;
                self.parse_assignment_allow_in()?;
                let anonymous_default = self.take_anonymous_function_definition();
                if let (Some(definition), Some(name)) = (anonymous_default, inferred_name) {
                    let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                        JsString::try_from_utf8(&name)?,
                    )))?;
                    self.emit_anonymous_set_name(definition, Instruction::SetName(name_constant))?;
                }
                let has_value_target = self.current_ir().ops.len();
                self.patch_jump(has_value, has_value_target)?;
            }
        }

        self.emit_destructuring_assignment_put(target)
    }

    /// Parse a syntactic LeftHandSideExpression and leave only its Reference
    /// operands on the stack.  Computed property keys intentionally remain in
    /// their raw form here: PutArrayEl/PutSuperValue performs ToPropertyKey
    /// after IteratorStep has produced the replacement value.
    fn parse_destructuring_assignment_reference(
        &mut self,
    ) -> Result<DestructuringAssignmentReference, Error> {
        self.parse_left_hand_side_expression()?;
        if let Some(target) = self.take_tail_identifier_reference()? {
            self.validate_identifier_assignment_target(&target)?;
            return Ok(DestructuringAssignmentReference::Identifier(target));
        }
        if let Some(target) = self.take_tail_member_reference()? {
            return Ok(DestructuringAssignmentReference::Member(target));
        }
        Err(self.syntax_here("invalid destructuring target"))
    }

    /// QuickJS PUT_LVALUE_NOKEEP: consume both the prepared Reference and the
    /// assigned value.  A direct destructuring expression obtains its result
    /// from the independent RHS copy below the iterator, never from this put.
    fn emit_destructuring_assignment_put(
        &mut self,
        target: DestructuringAssignmentReference,
    ) -> Result<(), Error> {
        match target {
            DestructuringAssignmentReference::Identifier(target) => {
                if target.object_environment {
                    self.emit_identifier_reference_inherited(
                        target.name,
                        target.span,
                        target.scope,
                        IdentifierReferenceAccess::Set,
                    )?;
                    self.emit_instruction(Instruction::Drop)?;
                } else {
                    self.emit_identifier_inherited(
                        target.name,
                        target.span,
                        target.scope,
                        IdentifierAccess::Put,
                    )?;
                }
            }
            DestructuringAssignmentReference::Member(MemberReference::Field { key, site }) => {
                self.emit_instruction_at(Instruction::PutField(key), site)?;
            }
            DestructuringAssignmentReference::Member(MemberReference::Computed { site }) => {
                self.emit_instruction_at(Instruction::PutArrayEl, site)?;
            }
            DestructuringAssignmentReference::Member(MemberReference::Super { site }) => {
                self.emit_instruction_at(Instruction::PutSuperValue, site)?;
            }
        }
        self.anonymous_function_definition = None;
        Ok(())
    }

    /// A default following a nested pattern is parsed after that complete
    /// pattern in source order but must execute before its inner ForOfStart.
    /// Emit the recursive fragment first and use the same backward-edge shape
    /// as QuickJS's `js_parse_destructuring_element`.
    fn parse_nested_assignment_element(
        &mut self,
        pattern: BindingPatternKind,
        is_rest: bool,
    ) -> Result<(), Error> {
        let value_depth = self.current_ir().stack_depth;
        if value_depth == 0 {
            return Err(Error::internal(
                "nested assignment pattern has no value to consume",
            ));
        }

        if is_rest {
            self.parse_assignment_pattern(pattern)?;
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

        self.parse_assignment_pattern(pattern)?;
        self.require_stack_depth(value_depth - 1, "nested assignment pattern")?;

        if self.consume_punctuator(Punctuator::Equal)? {
            let done_jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
            let initializer_target = self.current_ir().ops.len();
            self.patch_jump(use_initializer, initializer_target)?;

            self.current_ir_mut().stack_depth = value_depth;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_assignment_allow_in()?;
            self.anonymous_function_definition = None;
            self.require_stack_depth(value_depth, "nested assignment pattern initializer")?;
            self.emit_instruction(Instruction::Goto(
                u32::try_from(assignment_target)
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?,
            ))?;

            let done_target = self.current_ir().ops.len();
            self.patch_jump(done_jump, done_target)?;
            self.current_ir_mut().stack_depth = value_depth - 1;
        } else {
            self.patch_jump(use_initializer, assignment_target)?;
        }
        Ok(())
    }

    /// Consume one already-evaluated value with an ObjectAssignmentPattern.
    /// Ordinary leaves prepare their complete target Reference before reading
    /// the source property. Nested patterns deliberately do the opposite:
    /// they read the outer property before recursively preparing inner
    /// References, matching QuickJS's unified destructuring lowering.
    fn parse_object_assignment_pattern(&mut self) -> Result<(), Error> {
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
                self.parse_object_assignment_rest(has_rest)?;
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
                // Keep the same malformed-pattern error priority as object
                // binding lowering and pinned QuickJS.
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
                self.parse_nested_assignment_element(pattern, false)?;
            } else {
                self.parse_object_assignment_leaf(property, computed_key_is_canonical)?;
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
        self.anonymous_function_definition = None;
        Ok(())
    }

    fn parse_object_assignment_leaf(
        &mut self,
        property: ObjectBindingPropertyKey<'source>,
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

        if fixed_key.is_some() {
            // retained source, source-to-read
            self.emit_instruction(Instruction::Dup)?;
        } else {
            if !computed_key_is_canonical {
                self.emit_instruction_at(Instruction::ToPropKey, source_offset(property_span)?)?;
            }
            // retained source, source-to-read, canonical key
            self.emit_instruction(Instruction::Dup1)?;
        }

        let target = if let Some((token, identifier)) = shorthand {
            self.prepare_shorthand_assignment_reference(token, identifier)?
        } else {
            self.parse_destructuring_assignment_reference()?
        };
        let reference_depth = target.depth();
        let inferred_name = target.inferred_name().map(str::to_owned);
        let target_site = target.site()?;
        self.emit_object_assignment_read_reorder(fixed_key.is_none(), reference_depth)?;

        if let Some(key) = fixed_key {
            let key = self.add_constant(IrConstant::Primitive(Value::String(key)))?;
            self.emit_instruction_at(Instruction::GetField(key), target_site)?;
        } else {
            self.emit_instruction_at(Instruction::GetArrayEl, target_site)?;
        }

        if self.consume_punctuator(Punctuator::Equal)? {
            self.emit_instruction(Instruction::Dup)?;
            self.emit_instruction(Instruction::Undefined)?;
            self.emit_instruction(Instruction::StrictEq)?;
            let has_value = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
            self.emit_instruction(Instruction::Drop)?;
            self.parse_assignment_allow_in()?;
            let anonymous_default = self.take_anonymous_function_definition();
            if let (Some(definition), Some(name)) = (anonymous_default, inferred_name) {
                let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::try_from_utf8(&name)?,
                )))?;
                self.emit_anonymous_set_name(definition, Instruction::SetName(name_constant))?;
            }
            let has_value_target = self.current_ir().ops.len();
            self.patch_jump(has_value, has_value_target)?;
        }

        self.emit_destructuring_assignment_put(target)
    }

    fn prepare_shorthand_assignment_reference(
        &mut self,
        token: Token<'source>,
        identifier: Identifier<'source>,
    ) -> Result<DestructuringAssignmentReference, Error> {
        validate_identifier(
            &identifier,
            token.span,
            self.current_ir().strict,
            IdentifierContext::Reference,
        )?;
        if self.current_ir().strict && matches!(identifier.value.as_str(), "eval" | "arguments") {
            return Err(self.syntax_here("invalid destructuring target"));
        }

        let scope = self.current_ir().current_scope;
        let object_environment =
            self.parser_scope_has_authored_with(self.current_function, scope)?;
        let reference = IdentifierReference {
            name: identifier.value,
            span: token.span,
            scope,
            object_environment,
        };
        if object_environment {
            self.emit_identifier_reference_inherited(
                reference.name.clone(),
                reference.span,
                reference.scope,
                IdentifierReferenceAccess::Prepare,
            )?;
        }
        Ok(DestructuringAssignmentReference::Identifier(reference))
    }

    /// Move the retained source[/key] above a prepared Reference so the
    /// non-keeping property read leaves `reference..., value`. All sequences
    /// keep the same peak stack depth; no extra VM opcode is needed.
    fn emit_object_assignment_read_reorder(
        &mut self,
        computed: bool,
        reference_depth: u8,
    ) -> Result<(), Error> {
        match (computed, reference_depth) {
            (_, 0) => {}
            (false, 1) => {
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
            }
            (false, 2) => {
                self.emit_instruction(Instruction::Perm3)?;
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
            }
            (false, 3) => {
                self.emit_instruction(Instruction::Rot4Left)?;
            }
            (true, 1) => {
                self.emit_instruction(Instruction::Insert2)?;
                self.emit_instruction(Instruction::Drop)?;
                self.emit_instruction(Instruction::Perm3)?;
            }
            (true, 2) => {
                self.emit_instruction(Instruction::Rot4Left)?;
                self.emit_instruction(Instruction::Rot4Left)?;
            }
            (true, 3) => {
                self.emit_instruction(Instruction::Rot4Left)?;
                self.emit_instruction(Instruction::Perm5)?;
                self.emit_instruction(Instruction::Perm5)?;
                self.emit_instruction(Instruction::Perm5)?;
            }
            _ => return Err(Error::internal("unsupported destructuring Reference depth")),
        }
        Ok(())
    }

    fn parse_object_assignment_rest(&mut self, has_rest: bool) -> Result<(), Error> {
        self.advance()?;
        let target = self.parse_destructuring_assignment_reference()?;
        if !self.is_punctuator(Punctuator::RightBrace) {
            return Err(self.syntax_here("assignment rest property must be last"));
        }
        if !has_rest {
            return Err(Error::internal(
                "object rest assignment was absent from its pattern skip-scan",
            ));
        }

        let reference_depth = target.depth();
        let target_site = target.site()?;
        self.emit_instruction(Instruction::Object)?;
        self.emit_instruction_at(
            Instruction::CopyDataPropertiesExcluded {
                target_depth: 0,
                source_depth: reference_depth
                    .checked_add(1)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
                excluded_depth: reference_depth
                    .checked_add(2)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
            },
            target_site,
        )?;
        self.emit_destructuring_assignment_put(target)
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
                    if site == BindingSite::Parameter {
                        self.register_pattern_parameter_binding(
                            &name,
                            token.span,
                            self.current().span,
                        )?;
                    } else {
                        self.register_var_binding(&name, token.span, self.current().span)?;
                    }
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
            let parameter_lexical = declaration == ForAssignmentDeclaration::Var
                && site == BindingSite::Parameter
                && self.current_ir().parameter_scope.is_some();
            let next_offset = if declaration == ForAssignmentDeclaration::Var && !parameter_lexical
            {
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
                    if let Some(definition) = self.take_anonymous_function_definition() {
                        let name_constant = self.add_constant(IrConstant::Primitive(
                            Value::String(JsString::try_from_utf8(&name)?),
                        ))?;
                        self.emit_anonymous_set_name(
                            definition,
                            Instruction::SetName(name_constant),
                        )?;
                    }
                    let has_value_target = self.current_ir().ops.len();
                    self.patch_jump(has_value, has_value_target)?;
                }
            }
            if declaration == ForAssignmentDeclaration::Var && !parameter_lexical {
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
        if site == BindingSite::Catch {
            // QuickJS emits no source marker for CatchParameter ToObject. A
            // top-level nullish throw therefore retains the original throw
            // site's PC, while nested patterns inherit their preceding read.
            self.emit_instruction(Instruction::ToObject)?;
        } else {
            self.emit_instruction_at(Instruction::ToObject, source_offset(pattern_span)?)?;
        }
        if has_rest {
            // source exclusion -> exclusion source
            self.emit_instruction(Instruction::Object)?;
            self.emit_instruction(Instruction::Insert2)?;
            self.emit_instruction(Instruction::Drop)?;
        }

        while !self.is_punctuator(Punctuator::RightBrace) {
            if self.is_punctuator(Punctuator::Ellipsis) {
                self.parse_object_binding_rest(declaration, is_const, site, has_rest)?;
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
                    site,
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
        site: BindingSite,
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
                if site == BindingSite::Parameter {
                    self.register_pattern_parameter_binding(
                        &name,
                        token.span,
                        self.current().span,
                    )?;
                } else {
                    self.register_var_binding(&name, token.span, self.current().span)?;
                }
            }
            ForAssignmentDeclaration::Assignment => {
                unreachable!("object binding declaration was validated before parsing properties")
            }
        }

        let reference_scope = self.current_ir().current_scope;
        let parameter_lexical = declaration == ForAssignmentDeclaration::Var
            && site == BindingSite::Parameter
            && self.current_ir().parameter_scope.is_some();
        if fixed_key.is_none() && !computed_key_is_canonical {
            // A leaf computed key is canonicalized before a sloppy `var`
            // Reference is prepared and before the property getter runs.
            self.emit_instruction(Instruction::ToPropKey)?;
        }
        if declaration == ForAssignmentDeclaration::Var && !parameter_lexical {
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
        if declaration == ForAssignmentDeclaration::Var && !parameter_lexical {
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
            if let Some(definition) = self.take_anonymous_function_definition() {
                let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::try_from_utf8(&name)?,
                )))?;
                self.emit_anonymous_set_name(definition, Instruction::SetName(name_constant))?;
            }
            let has_value_target = self.current_ir().ops.len();
            self.patch_jump(has_value, has_value_target)?;
        }

        if declaration == ForAssignmentDeclaration::Var && !parameter_lexical {
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
        site: BindingSite,
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
                if site == BindingSite::Parameter {
                    self.register_pattern_parameter_binding(
                        &identifier.value,
                        token.span,
                        self.current().span,
                    )?;
                } else {
                    self.register_var_binding(&identifier.value, token.span, self.current().span)?;
                }
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
        let parameter_lexical = declaration == ForAssignmentDeclaration::Var
            && site == BindingSite::Parameter
            && self.current_ir().parameter_scope.is_some();
        if declaration == ForAssignmentDeclaration::Var && !parameter_lexical {
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
        let (source_depth, excluded_depth) =
            if declaration == ForAssignmentDeclaration::Var && !parameter_lexical {
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
        if declaration == ForAssignmentDeclaration::Var && !parameter_lexical {
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
        self.emit_array_rest(reference_depth, None)
    }

    fn emit_array_assignment_rest(
        &mut self,
        reference_depth: u8,
        site: SourceOffset,
    ) -> Result<(), Error> {
        self.emit_array_rest(reference_depth, Some(site))
    }

    fn emit_array_rest(
        &mut self,
        reference_depth: u8,
        site: Option<SourceOffset>,
    ) -> Result<(), Error> {
        self.emit_instruction(Instruction::ArrayFrom(0))?;
        self.emit_instruction(Instruction::PushI32(0))?;
        let next_target = self.current_ir().ops.len();
        let next = Instruction::ForOfNext(
            reference_depth
                .checked_add(2)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
        );
        if let Some(site) = site {
            self.emit_instruction_at(next, site)?;
        } else {
            self.emit_instruction(next)?;
        }
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
