//! Token lookahead, lexical goals and diagnostic cursor.

use crate::engine::api::error::Error;
use crate::engine::code::function::metadata::EvalKind;

use crate::engine::compiler::lexer::Identifier;
use crate::engine::compiler::lexer::Keyword;
use crate::engine::compiler::lexer::LexContext;
use crate::engine::compiler::lexer::LexicalGoal;
use crate::engine::compiler::lexer::Punctuator;
use crate::engine::compiler::lexer::TemplatePartKind;
use crate::engine::compiler::lexer::Token;
use crate::engine::compiler::lexer::TokenKind;
use crate::engine::compiler::lexer::quickjs_simple_lookahead_is_of;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::parser::context::ForHeadDelimiter;
use crate::engine::compiler::parser::context::ForIterationKind;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::diagnostics::lex_error;
use crate::engine::compiler::pseudo_binding::NEW_TARGET_LOCAL_NAME;

use crate::engine::compiler::parser::diagnostics::source_span;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn expect_punctuator(
        &mut self,
        punctuator: Punctuator,
    ) -> Result<(), Error> {
        if self.consume_punctuator(punctuator)? {
            Ok(())
        } else {
            Err(self.syntax_here(format!("expecting '{}'", punctuator.as_str())))
        }
    }

    pub(in crate::engine::compiler) fn consume_punctuator(
        &mut self,
        punctuator: Punctuator,
    ) -> Result<bool, Error> {
        if self.is_punctuator(punctuator) {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(in crate::engine::compiler) fn is_punctuator(&self, punctuator: Punctuator) -> bool {
        matches!(self.current().kind, TokenKind::Punctuator(current) if current == punctuator)
    }

    /// `of` is a QuickJS pseudo-keyword: escapes prevent it from acting as
    /// the for-of delimiter even though the decoded identifier text matches.
    pub(in crate::engine::compiler) fn is_for_of_keyword(&self) -> bool {
        matches!(
            &self.current().kind,
            TokenKind::Identifier(identifier)
                if identifier.value == "of" && !identifier.has_escape
        )
    }

    /// QuickJS `peek_token(FALSE) == TOK_OF` from an already-consumed
    /// pseudo-keyword. This simplified, non-committing lookahead skips trivia
    /// but does not let an escaped spelling act as the contextual delimiter.
    pub(in crate::engine::compiler) fn next_token_is_for_of_keyword(&self) -> bool {
        self.lexer
            .source()
            .get(self.current().span.end.byte_offset..)
            .is_some_and(quickjs_simple_lookahead_is_of)
    }

    /// Non-committing delimiter probe for a semicolon-free for head. It selects
    /// the retained record shape before the assignment fragment is lowered;
    /// the real parser still validates the complete LeftHandSideExpression and
    /// reports source-ordered syntax errors.
    pub(in crate::engine::compiler) fn for_iteration_kind_ahead(&self) -> Option<ForIterationKind> {
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

            if delimiters.is_empty() {
                match &token.kind {
                    TokenKind::Keyword(Keyword::In) => return Some(ForIterationKind::In),
                    TokenKind::Identifier(identifier)
                        if identifier.value == "of" && !identifier.has_escape =>
                    {
                        return Some(ForIterationKind::Of);
                    }
                    TokenKind::Punctuator(Punctuator::RightParen) | TokenKind::Eof => return None,
                    _ => {}
                }
            }

            match &token.kind {
                TokenKind::Punctuator(Punctuator::LeftParen) => {
                    delimiters.push(ForHeadDelimiter::Parenthesis);
                }
                TokenKind::Punctuator(Punctuator::LeftBracket) => {
                    delimiters.push(ForHeadDelimiter::Bracket);
                }
                TokenKind::Punctuator(Punctuator::LeftBrace) => {
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
                }
                TokenKind::Template(part) => match part.kind {
                    TemplatePartKind::Head => delimiters.push(ForHeadDelimiter::Template),
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

    /// Mirror QuickJS `js_parse_skip_parens_token` for the one decision needed
    /// by classic `for`: any semicolon at the outer head depth selects classic
    /// grammar, even when its NoIn initializer later stops at `in` or `of`.
    /// This is a non-committing probe; lexical failures are encountered again
    /// by the real parser in source order.
    pub(in crate::engine::compiler) fn for_head_has_top_level_semicolon(&self) -> bool {
        if !self.is_punctuator(Punctuator::LeftParen) {
            return false;
        }

        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.start);
        let mut delimiters = Vec::new();
        let mut goal = LexicalGoal::Div;
        let mut regexp_allowed = true;
        let mut has_semicolon = false;

        loop {
            let requested_goal = goal;
            goal = LexicalGoal::Div;
            let Ok(mut token) = lexer.next_token_with_goal(requested_goal) else {
                return has_semicolon;
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
                    return has_semicolon;
                };
                token = regexp;
            }

            match &token.kind {
                TokenKind::Punctuator(Punctuator::LeftParen) => {
                    if delimiters.len() >= 255 {
                        return has_semicolon;
                    }
                    delimiters.push(ForHeadDelimiter::Parenthesis);
                }
                TokenKind::Punctuator(Punctuator::LeftBracket) => {
                    if delimiters.len() >= 255 {
                        return has_semicolon;
                    }
                    delimiters.push(ForHeadDelimiter::Bracket);
                }
                TokenKind::Punctuator(Punctuator::LeftBrace) => {
                    if delimiters.len() >= 255 {
                        return has_semicolon;
                    }
                    delimiters.push(ForHeadDelimiter::Brace);
                }
                TokenKind::Punctuator(Punctuator::RightParen) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Parenthesis) {
                        return has_semicolon;
                    }
                    if delimiters.is_empty() {
                        return has_semicolon;
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBracket) => {
                    if delimiters.pop() != Some(ForHeadDelimiter::Bracket) {
                        return has_semicolon;
                    }
                }
                TokenKind::Punctuator(Punctuator::RightBrace) => {
                    if delimiters.last() == Some(&ForHeadDelimiter::Template) {
                        goal = LexicalGoal::TemplateContinuation;
                        regexp_allowed = true;
                        continue;
                    }
                    if delimiters.pop() != Some(ForHeadDelimiter::Brace) {
                        return has_semicolon;
                    }
                }
                TokenKind::Punctuator(Punctuator::Semicolon) if delimiters.len() == 1 => {
                    has_semicolon = true;
                }
                TokenKind::Template(part) => match part.kind {
                    TemplatePartKind::Head => {
                        if delimiters.len() >= 255 {
                            return has_semicolon;
                        }
                        delimiters.push(ForHeadDelimiter::Template);
                    }
                    TemplatePartKind::Middle => {
                        if delimiters.last() != Some(&ForHeadDelimiter::Template) {
                            return has_semicolon;
                        }
                    }
                    TemplatePartKind::Tail => {
                        if delimiters.pop() != Some(ForHeadDelimiter::Template) {
                            return has_semicolon;
                        }
                    }
                    TemplatePartKind::NoSubstitution => {}
                },
                TokenKind::Eof => return has_semicolon,
                _ => {}
            }
            regexp_allowed = for_head_regexp_allowed_after(&token.kind);
        }
    }

    /// QuickJS `is_let(..., DECL_MASK_OTHER)` resolves sloppy `let` before the
    /// statement parser chooses declaration or expression grammar. In
    /// particular, `let [` is always lexical and must never silently execute
    /// as a member assignment while destructuring remains an explicit boundary.
    pub(in crate::engine::compiler) fn lexical_declaration_ahead(
        &self,
        allow_line_terminated_other: bool,
    ) -> Result<bool, Error> {
        if matches!(
            self.current().kind,
            TokenKind::Keyword(Keyword::Let | Keyword::Const)
        ) {
            return Ok(true);
        }
        let TokenKind::Identifier(identifier) = &self.current().kind else {
            return Ok(false);
        };
        if identifier.value != "let" || identifier.has_escape {
            return Ok(false);
        }

        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.start);
        lexer.next_token().map_err(lex_error)?;
        let next = lexer.next_token().map_err(lex_error)?;
        let other_declaration_start = matches!(
            &next.kind,
            TokenKind::Punctuator(Punctuator::LeftBrace)
                | TokenKind::Identifier(Identifier {
                    escaped_reserved_word: false,
                    ..
                })
                | TokenKind::Keyword(Keyword::Let | Keyword::Yield | Keyword::Await)
        );
        Ok(
            matches!(&next.kind, TokenKind::Punctuator(Punctuator::LeftBracket))
                || (other_declaration_start
                    && (!next.line_terminator_before || allow_line_terminated_other)),
        )
    }

    /// QuickJS `is_label` accepts only a non-reserved Identifier followed by
    /// `:` using a non-committing simplified scanner. Keep the probe separate
    /// from the parser token cache; a lexical failure after the identifier is
    /// still reported later by the real parser in source order.
    pub(in crate::engine::compiler) fn label_ahead(&self) -> Option<String> {
        let TokenKind::Identifier(identifier) = &self.current().kind else {
            return None;
        };
        if identifier.escaped_reserved_word {
            return None;
        }
        let label_name = identifier.value.clone();
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let Ok(next) = lexer.next_token() else {
            return None;
        };
        matches!(next.kind, TokenKind::Punctuator(Punctuator::Colon)).then_some(label_name)
    }

    /// QuickJS gates generator and pseudo-keyword `async function` declarations
    /// before entering the ordinary-function parser when DECL_MASK_OTHER is
    /// absent. Preserve that diagnostic priority without consuming lookahead.
    pub(in crate::engine::compiler) fn restricted_function_declaration_ahead(
        &self,
        annex_b_function_allowed: bool,
    ) -> Result<bool, Error> {
        let generator = matches!(self.current().kind, TokenKind::Keyword(Keyword::Function));
        let async_function = self.async_function_ahead();
        if (!generator || !annex_b_function_allowed) && !async_function {
            return Ok(false);
        }
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let next = lexer.next_token().map_err(lex_error)?;
        if generator {
            Ok(matches!(
                next.kind,
                TokenKind::Punctuator(Punctuator::Multiply)
            ))
        } else {
            Ok(async_function)
        }
    }

    /// Non-committing recognition of QuickJS's `async function`
    /// pseudo-keyword pair. Escapes and a LineTerminator after `async` leave it
    /// as an ordinary IdentifierReference.
    pub(in crate::engine::compiler) fn async_function_ahead(&self) -> bool {
        let TokenKind::Identifier(identifier) = &self.current().kind else {
            return false;
        };
        if identifier.value != "async" || identifier.has_escape {
            return false;
        }
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let Ok(function) = lexer.next_token_with_goal(LexicalGoal::Div) else {
            return false;
        };
        !function.line_terminator_before
            && matches!(function.kind, TokenKind::Keyword(Keyword::Function))
    }

    pub(in crate::engine::compiler) fn at_eof(&self) -> bool {
        matches!(self.current().kind, TokenKind::Eof)
    }

    /// QuickJS arrows inherit the `new.target` capability through parse
    /// parents. A direct-eval root authenticates the inherited capability by
    /// carrying the hidden imported binding in its root environment.
    pub(in crate::engine::compiler) fn current_new_target_allowed(&self) -> bool {
        let mut function_id = self.current_function;
        loop {
            let function = &self.functions[function_id];
            match function.kind {
                FunctionKind::Ordinary | FunctionKind::Method => return true,
                FunctionKind::Script
                | FunctionKind::Module
                | FunctionKind::Eval(EvalKind::Indirect) => return false,
                FunctionKind::Eval(EvalKind::Direct) => {
                    return function
                        .binding_from_scope(function.ir.var_scope, NEW_TARGET_LOCAL_NAME)
                        .is_some();
                }
                FunctionKind::Eval(EvalKind::None) => return false,
                FunctionKind::Arrow => {
                    let Some(parent) = function.parent else {
                        return false;
                    };
                    function_id = parent.function;
                }
            }
        }
    }

    pub(in crate::engine::compiler) fn current(&self) -> &Token<'source> {
        // Construction and every advance ensure the current token exists.
        &self.tokens[self.cursor]
    }

    pub(in crate::engine::compiler) fn advance(&mut self) -> Result<(), Error> {
        self.advance_with_goal(LexicalGoal::Div)
    }

    /// Advance from a grammar delimiter to the first token of an expression,
    /// selecting RegExp only when the ordinary scanner sees a leading slash.
    pub(in crate::engine::compiler) fn advance_expression_start(&mut self) -> Result<(), Error> {
        let start = self.current().span.end;
        if self.tokens.len() > self.cursor + 1 {
            self.tokens.truncate(self.cursor + 1);
            self.lexer.seek(start);
        }
        let mut probe = self.lexer.clone();
        probe.seek(start);
        let next = probe.next_token().map_err(lex_error)?;
        let goal = if matches!(
            next.kind,
            TokenKind::Punctuator(Punctuator::Divide | Punctuator::DivideAssign)
        ) {
            LexicalGoal::RegExp
        } else {
            LexicalGoal::Div
        };
        self.advance_with_goal(goal)
    }

    pub(in crate::engine::compiler) fn advance_with_goal(
        &mut self,
        goal: LexicalGoal,
    ) -> Result<(), Error> {
        if !self.at_eof() {
            self.cursor += 1;
            self.ensure_token_with_goal(self.cursor, goal)?;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn ensure_token(&mut self, index: usize) -> Result<(), Error> {
        self.ensure_token_with_goal(index, LexicalGoal::Div)
    }

    pub(in crate::engine::compiler) fn ensure_token_with_goal(
        &mut self,
        index: usize,
        goal: LexicalGoal,
    ) -> Result<(), Error> {
        while self.tokens.len() <= index {
            let token = self.lexer.next_token_with_goal(goal).map_err(lex_error)?;
            self.tokens.push(token);
        }
        Ok(())
    }

    /// Rescan the current token after the parser has selected its lexical
    /// goal.  Seeking to the token itself intentionally avoids committing a
    /// lexer heuristic; preserve the already-observed trivia bit because the
    /// rescan starts after that trivia rather than before it.
    pub(in crate::engine::compiler) fn relex_current_with_goal(
        &mut self,
        goal: LexicalGoal,
    ) -> Result<(), Error> {
        let position = self.current().span.start;
        let line_terminator_before = self.current().line_terminator_before;
        self.tokens.truncate(self.cursor);
        self.lexer.seek(position);
        self.ensure_token_with_goal(self.cursor, goal)?;
        self.tokens[self.cursor].line_terminator_before = line_terminator_before;
        Ok(())
    }

    pub(in crate::engine::compiler) fn relex_current_with_strict(
        &mut self,
        strict: bool,
    ) -> Result<(), Error> {
        let mut context = self.lexer.context();
        context.strict = strict;
        self.relex_current_with_context(context)
    }

    /// Rescan the current token and all future tokens under one complete
    /// function lexical context. Function nesting must restore all of strict,
    /// module, generator and async state; changing only `strict` leaks a
    /// parent's contextual `yield`/`await` classification into its child.
    pub(in crate::engine::compiler) fn relex_current_with_context(
        &mut self,
        context: LexContext,
    ) -> Result<(), Error> {
        let position = self.current().span.start;
        let line_terminator_before = self.current().line_terminator_before;
        self.tokens.truncate(self.cursor);
        self.lexer.seek(position);
        self.lexer.set_context(context);
        self.ensure_token(self.cursor)?;
        self.tokens[self.cursor].line_terminator_before = line_terminator_before;
        Ok(())
    }

    /// Change how tokens after the current token are classified without
    /// reparsing the current token. QuickJS relies on this distinction when an
    /// async arrow's unparenthesized parameter was already read in its parent.
    pub(in crate::engine::compiler) fn set_future_lex_context(&mut self, context: LexContext) {
        let position = self.current().span.end;
        self.tokens.truncate(self.cursor + 1);
        self.lexer.seek(position);
        self.lexer.set_context(context);
    }

    pub(in crate::engine::compiler) fn directive_prologue_has_use_strict(
        &self,
        start: usize,
        inherited_strict: bool,
    ) -> Result<bool, Error> {
        let use_strict = "use strict".encode_utf16().collect::<Vec<_>>();
        let position = self.tokens[start].span.start;
        let mut lexer = self.lexer.clone();
        lexer.seek(position);
        let mut context = lexer.context();
        context.strict = inherited_strict;
        lexer.set_context(context);
        let mut token = lexer.next_token().map_err(lex_error)?;
        let mut found_strict = false;

        loop {
            let candidate = match &token.kind {
                TokenKind::String(literal) => {
                    !literal.has_escape && literal.value.utf16 == use_strict
                }
                _ => return Ok(found_strict),
            };

            let next = lexer.next_token().map_err(lex_error)?;
            let consumed = match &next.kind {
                TokenKind::Punctuator(Punctuator::Semicolon) => 2,
                TokenKind::Punctuator(Punctuator::RightBrace) | TokenKind::Eof => 1,
                _ if next.line_terminator_before && quickjs_directive_asi_token(&next.kind) => 1,
                _ => return Ok(found_strict),
            };
            if candidate {
                found_strict = true;
            }
            token = if consumed == 1 {
                next
            } else {
                lexer.next_token().map_err(lex_error)?
            };
            if candidate {
                let mut context = lexer.context();
                context.strict = true;
                lexer.set_context(context);
            }
        }
    }

    pub(in crate::engine::compiler) fn syntax_here(&self, message: impl Into<String>) -> Error {
        Error::syntax(message, source_span(self.current().span))
    }

    pub(in crate::engine::compiler) fn unsupported_here(
        &self,
        message: impl Into<String>,
    ) -> Error {
        Error::unsupported(message, source_span(self.current().span))
    }
}

pub(in crate::engine::compiler) fn quickjs_directive_asi_token(kind: &TokenKind<'_>) -> bool {
    matches!(
        kind,
        TokenKind::Number(_)
            | TokenKind::String(_)
            | TokenKind::Template(_)
            | TokenKind::Identifier(_)
            | TokenKind::RegExp(_)
            | TokenKind::Punctuator(Punctuator::Decrement | Punctuator::Increment)
            | TokenKind::Keyword(
                Keyword::Null
                    | Keyword::False
                    | Keyword::True
                    | Keyword::If
                    | Keyword::Return
                    | Keyword::Var
                    | Keyword::This
                    | Keyword::Delete
                    | Keyword::Typeof
                    | Keyword::New
                    | Keyword::Do
                    | Keyword::While
                    | Keyword::For
                    | Keyword::Switch
                    | Keyword::Throw
                    | Keyword::Try
                    | Keyword::Function
                    | Keyword::Debugger
                    | Keyword::With
                    | Keyword::Class
                    | Keyword::Const
                    | Keyword::Enum
                    | Keyword::Export
                    | Keyword::Import
                    | Keyword::Super
                    | Keyword::Interface
                    | Keyword::Let
                    | Keyword::Package
                    | Keyword::Private
                    | Keyword::Protected
                    | Keyword::Public
                    | Keyword::Static
            )
    )
}

/// QuickJS `is_regexp_allowed`, used only by the non-committing `for`-head
/// probe. The real parser still owns the eventual lexical goal and diagnostic.
pub(in crate::engine::compiler) fn for_head_regexp_allowed_after(kind: &TokenKind<'_>) -> bool {
    if matches!(
        kind,
        TokenKind::Identifier(identifier)
            if !identifier.has_escape && matches!(identifier.value.as_str(), "of" | "yield")
    ) {
        return true;
    }
    !matches!(
        kind,
        TokenKind::Number(_)
            | TokenKind::String(_)
            | TokenKind::RegExp(_)
            | TokenKind::Identifier(_)
            | TokenKind::Keyword(Keyword::Null | Keyword::False | Keyword::True | Keyword::This)
            | TokenKind::Punctuator(
                Punctuator::RightParen
                    | Punctuator::RightBracket
                    | Punctuator::RightBrace
                    | Punctuator::Increment
                    | Punctuator::Decrement
            )
    )
}
