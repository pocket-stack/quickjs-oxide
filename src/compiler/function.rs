use super::{
    Error, ErrorKind, FunctionId, FunctionIr, FunctionKind, FunctionSourceInfo, Identifier,
    IdentifierContext, IrConstant, IrOp, MAX_LOCAL_VARIABLES, ParentLink, Parser, Punctuator,
    SourceOffset, Span, TokenKind, source_offset, source_span, validate_identifier,
};
use crate::bytecode::DefineMethodKind;

pub(super) struct ParsedFunctionDefinition {
    pub(super) constant: u32,
    pub(super) child: FunctionId,
    pub(super) name: Option<(String, Span)>,
}

pub(super) struct FunctionDefinitionHeader<'source> {
    pub(super) span: Span,
    pub(super) name: Option<(Identifier<'source>, Span)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FunctionDefinitionOptions {
    kind: FunctionKind,
    private_name_binding: bool,
    reject_duplicate_parameters: bool,
    allow_trailing_parameter_comma: bool,
    object_method_kind: Option<DefineMethodKind>,
    accessor_parameter_count: Option<usize>,
}

impl FunctionDefinitionOptions {
    const fn ordinary(private_name_binding: bool) -> Self {
        Self {
            kind: FunctionKind::Ordinary,
            private_name_binding,
            reject_duplicate_parameters: false,
            allow_trailing_parameter_comma: false,
            object_method_kind: None,
            accessor_parameter_count: None,
        }
    }

    const fn object_method(method_kind: DefineMethodKind) -> Self {
        Self {
            kind: FunctionKind::Method,
            private_name_binding: false,
            // QuickJS applies the ordinary-method UniqueFormalParameters early
            // error even when the surrounding source and body are sloppy.
            // Accessors first enforce their zero/one-parameter arity.
            reject_duplicate_parameters: matches!(method_kind, DefineMethodKind::Method),
            allow_trailing_parameter_comma: true,
            object_method_kind: Some(method_kind),
            accessor_parameter_count: match method_kind {
                DefineMethodKind::Method => None,
                DefineMethodKind::Getter => Some(0),
                DefineMethodKind::Setter => Some(1),
            },
        }
    }
}

impl<'source> Parser<'source> {
    pub(super) fn parse_function_expression(&mut self) -> Result<(), Error> {
        let parsed = self.parse_function_definition(false, true)?;
        self.emit(IrOp::MakeClosure(parsed.constant))?;
        self.anonymous_function_definition = parsed.name.is_none().then_some(parsed.child);
        Ok(())
    }

    /// Parse the common ordinary-function grammar and publish its child
    /// constant in the defining function. The caller decides whether that
    /// constant is evaluated in expression position or recorded for Program
    /// declaration hoisting.
    pub(super) fn parse_function_definition(
        &mut self,
        require_name: bool,
        private_name_binding: bool,
    ) -> Result<ParsedFunctionDefinition, Error> {
        let header = self.parse_function_definition_header(require_name)?;
        self.parse_function_definition_tail(header, private_name_binding)
    }

    pub(super) fn parse_function_definition_header(
        &mut self,
        require_name: bool,
    ) -> Result<FunctionDefinitionHeader<'source>, Error> {
        let span = self.current().span;
        self.advance()?;
        let name = if let TokenKind::Identifier(identifier) = self.current().kind.clone() {
            let span = self.current().span;
            validate_identifier(&identifier, span, false, IdentifierContext::FunctionName)?;
            self.advance()?;
            Some((identifier, span))
        } else {
            None
        };
        if require_name && name.is_none() {
            return Err(self.syntax_here("function name expected"));
        }
        Ok(FunctionDefinitionHeader { span, name })
    }

    pub(super) fn parse_function_definition_tail(
        &mut self,
        header: FunctionDefinitionHeader<'source>,
        private_name_binding: bool,
    ) -> Result<ParsedFunctionDefinition, Error> {
        self.parse_function_definition_tail_with_options(
            header,
            FunctionDefinitionOptions::ordinary(private_name_binding),
        )
    }

    /// Parse a synchronous object-literal method or accessor after its property
    /// name has been consumed. Unlike a named function expression, the property
    /// key is only the eventual public `.name`; it must never create a private
    /// self binding inside the function body.
    pub(super) fn parse_object_method_definition(
        &mut self,
        function_span: Span,
        method_kind: DefineMethodKind,
    ) -> Result<(), Error> {
        let parsed = self.parse_function_definition_tail_with_options(
            FunctionDefinitionHeader {
                span: function_span,
                name: None,
            },
            FunctionDefinitionOptions::object_method(method_kind),
        )?;
        self.emit(IrOp::MakeClosure(parsed.constant))?;
        self.anonymous_function_definition = None;
        Ok(())
    }

    fn parse_function_definition_tail_with_options(
        &mut self,
        header: FunctionDefinitionHeader<'source>,
        options: FunctionDefinitionOptions,
    ) -> Result<ParsedFunctionDefinition, Error> {
        let FunctionDefinitionHeader {
            span: function_span,
            name: function_name_token,
        } = header;
        self.expect_punctuator(Punctuator::LeftParen)?;

        let mut parameters = Vec::new();
        let mut parameter_tokens = Vec::new();
        let mut parameter_list_end_span = self.current().span;
        if self.is_punctuator(Punctuator::RightParen) {
            self.advance()?;
        } else {
            loop {
                if let Some(method_kind) = options.object_method_kind {
                    let role = match method_kind {
                        DefineMethodKind::Method => "object method",
                        DefineMethodKind::Getter => "object getter",
                        DefineMethodKind::Setter => "object setter",
                    };
                    if self.is_punctuator(Punctuator::Ellipsis) {
                        if !matches!(method_kind, DefineMethodKind::Method) {
                            return Err(Error::syntax(
                                "invalid number of arguments for getter or setter",
                                source_span(self.current().span),
                            ));
                        }
                        return Err(self.unsupported_here(format!(
                            "{role} rest parameters are not implemented yet"
                        )));
                    }
                    if matches!(
                        self.current().kind,
                        TokenKind::Punctuator(Punctuator::LeftBracket | Punctuator::LeftBrace)
                    ) {
                        return Err(self.unsupported_here(format!(
                            "{role} destructuring parameters are not implemented yet"
                        )));
                    }
                }
                let token = self.current().clone();
                let TokenKind::Identifier(identifier) = token.kind else {
                    return Err(self.syntax_here("missing formal parameter"));
                };
                validate_identifier(&identifier, token.span, false, IdentifierContext::Argument)?;
                if options.reject_duplicate_parameters && parameters.contains(&identifier.value) {
                    return Err(Error::syntax(
                        "duplicate argument names not allowed in this context",
                        source_span(token.span),
                    ));
                }
                parameter_tokens.push((identifier.clone(), token.span));
                parameters.push(identifier.value);
                if parameters.len() > MAX_LOCAL_VARIABLES {
                    return Err(Error::new(ErrorKind::JsInternal, "too many arguments")
                        .with_span(source_span(token.span)));
                }
                self.advance()?;
                if let Some(method_kind) = options.object_method_kind {
                    if self.is_punctuator(Punctuator::Equal) {
                        let role = match method_kind {
                            DefineMethodKind::Method => "object method",
                            DefineMethodKind::Getter => "object getter",
                            DefineMethodKind::Setter => "object setter",
                        };
                        return Err(self.unsupported_here(format!(
                            "{role} default parameters are not implemented yet"
                        )));
                    }
                }
                if !self.consume_punctuator(Punctuator::Comma)? {
                    if !self.is_punctuator(Punctuator::RightParen) {
                        return Err(Error::syntax(
                            "expecting ','",
                            source_span(self.current().span),
                        ));
                    }
                    parameter_list_end_span = self.current().span;
                    self.advance()?;
                    break;
                }
                if self.is_punctuator(Punctuator::RightParen) {
                    if options.allow_trailing_parameter_comma {
                        parameter_list_end_span = self.current().span;
                        self.advance()?;
                        break;
                    }
                    return Err(self.unsupported_here(
                        "a trailing comma in this simple parameter list is not implemented yet",
                    ));
                }
            }
        }
        if options
            .accessor_parameter_count
            .is_some_and(|expected| parameters.len() != expected)
        {
            return Err(Error::syntax(
                "invalid number of arguments for getter or setter",
                source_span(parameter_list_end_span),
            ));
        }
        self.expect_punctuator(Punctuator::LeftBrace)?;

        let parent = self.current_function;
        let parent_strict = self.functions[parent].strict;
        let has_use_strict = self.directive_prologue_has_use_strict(self.cursor, parent_strict)?;
        let strict = self.functions[parent].strict || has_use_strict;
        self.relex_current_with_strict(strict)?;
        if strict {
            let strict_validation_span = self.current().span;
            if let Some((identifier, _)) = &function_name_token {
                validate_identifier(
                    identifier,
                    strict_validation_span,
                    true,
                    IdentifierContext::FunctionName,
                )?;
            }
            for (index, (identifier, _)) in parameter_tokens.iter().enumerate() {
                validate_identifier(
                    identifier,
                    strict_validation_span,
                    true,
                    IdentifierContext::Argument,
                )?;
                let parameter = &identifier.value;
                if !options.reject_duplicate_parameters && parameters[..index].contains(parameter) {
                    return Err(Error::syntax(
                        "duplicate argument names not allowed in this context",
                        source_span(self.current().span),
                    ));
                }
            }
        }

        let function_name = function_name_token
            .as_ref()
            .map(|(identifier, _)| identifier.value.clone());
        let child = self.functions.len();
        let parent_scope = self.functions[parent].current_scope;
        self.functions.push(FunctionIr::new(
            Some(ParentLink {
                function: parent,
                definition_scope: parent_scope,
            }),
            options.kind,
            FunctionSourceInfo {
                span: function_span,
                definition: source_offset(function_span)?,
                range: None,
            },
            function_name,
            options.private_name_binding && function_name_token.is_some(),
            parameters,
            strict,
        )?);
        self.current_function = child;
        self.parse_function_body()?;
        let closing_brace = self.current().span;
        let mut parent_context = self.lexer.context();
        parent_context.strict = self.functions[parent].strict;
        self.lexer.set_context(parent_context);
        self.expect_punctuator(Punctuator::RightBrace)?;
        self.functions[child].source.range = Some(
            source_offset(function_span)?
                ..SourceOffset::try_from_usize(closing_brace.end.byte_offset)
                    .map_err(|error| Error::internal(error.to_string()))?,
        );
        self.current_function = parent;

        let constant = self.add_constant(IrConstant::Child(child))?;
        Ok(ParsedFunctionDefinition {
            constant,
            child,
            name: function_name_token.map(|(identifier, span)| (identifier.value, span)),
        })
    }
}
