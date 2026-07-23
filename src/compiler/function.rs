use super::{
    AnonymousFunctionDefinition, BytecodeFunctionKind, Error, FunctionId, FunctionIr,
    FunctionIrOptions, FunctionKind, FunctionSourceInfo, Identifier, IdentifierContext, IrConstant,
    IrOp, LexContext, ParentLink, Parser, Punctuator, SourceOffset, Span, SpannedIrOp,
    SuperCapabilities, TokenKind, insert_hoist_fragment, source_offset, source_span,
    validate_identifier,
};
use crate::bytecode::{DefineMethodKind, Instruction};

pub(super) struct ParsedFunctionDefinition {
    pub(super) constant: u32,
    pub(super) child: FunctionId,
    pub(super) name: Option<(String, Span)>,
}

pub(super) struct FunctionDefinitionHeader<'source> {
    pub(super) span: Span,
    pub(super) name: Option<(Identifier<'source>, Span)>,
    execution_kind: BytecodeFunctionKind,
    parent_context: LexContext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FunctionDefinitionOptions {
    kind: FunctionKind,
    execution_kind: BytecodeFunctionKind,
    private_name_binding: bool,
    class_constructor: bool,
    derived_class_constructor: bool,
    reject_duplicate_parameters: bool,
    allow_trailing_parameter_comma: bool,
    object_method_kind: Option<DefineMethodKind>,
    accessor_parameter_count: Option<usize>,
}

impl FunctionDefinitionOptions {
    const fn ordinary(private_name_binding: bool) -> Self {
        Self {
            kind: FunctionKind::Ordinary,
            execution_kind: BytecodeFunctionKind::Normal,
            private_name_binding,
            class_constructor: false,
            derived_class_constructor: false,
            reject_duplicate_parameters: false,
            allow_trailing_parameter_comma: true,
            object_method_kind: None,
            accessor_parameter_count: None,
        }
    }

    const fn object_method(method_kind: DefineMethodKind) -> Self {
        Self {
            kind: FunctionKind::Method,
            execution_kind: BytecodeFunctionKind::Normal,
            private_name_binding: false,
            class_constructor: false,
            derived_class_constructor: false,
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

    const fn generator_method() -> Self {
        Self {
            kind: FunctionKind::Method,
            execution_kind: BytecodeFunctionKind::Generator,
            private_name_binding: false,
            class_constructor: false,
            derived_class_constructor: false,
            reject_duplicate_parameters: true,
            allow_trailing_parameter_comma: true,
            object_method_kind: Some(DefineMethodKind::Method),
            accessor_parameter_count: None,
        }
    }
}

impl<'source> Parser<'source> {
    pub(super) fn parse_function_expression(&mut self) -> Result<(), Error> {
        let parsed = self.parse_function_definition(false, true)?;
        self.emit(IrOp::MakeClosure(parsed.constant))?;
        self.anonymous_function_definition = parsed
            .name
            .is_none()
            .then_some(AnonymousFunctionDefinition::Function);
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
        let parent_context = self.lexer.context();
        let span = self.current().span;
        self.advance()?;
        let execution_kind = if self.is_punctuator(Punctuator::Multiply) {
            self.advance()?;
            BytecodeFunctionKind::Generator
        } else {
            BytecodeFunctionKind::Normal
        };
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
        Ok(FunctionDefinitionHeader {
            span,
            name,
            execution_kind,
            parent_context,
        })
    }

    pub(super) fn parse_function_definition_tail(
        &mut self,
        header: FunctionDefinitionHeader<'source>,
        private_name_binding: bool,
    ) -> Result<ParsedFunctionDefinition, Error> {
        // Preserve QuickJS's declaration/expression asymmetry. Sloppy
        // `function* yield(){}` is accepted as a declaration, but a named
        // generator expression creates a private self binding and rejects the
        // same spelling as a reserved identifier.
        if private_name_binding
            && header.execution_kind == BytecodeFunctionKind::Generator
            && let Some((identifier, span)) = &header.name
            && identifier.value == "yield"
        {
            return Err(Error::syntax(
                "'yield' is a reserved identifier",
                source_span(*span),
            ));
        }
        let mut options = FunctionDefinitionOptions::ordinary(private_name_binding);
        options.execution_kind = header.execution_kind;
        self.parse_function_definition_tail_with_options(header, options)
    }

    /// Parse a synchronous object-literal method or accessor after its property
    /// name has been consumed. Unlike a named function expression, the property
    /// key is only the eventual public `.name`; it must never create a private
    /// self binding inside the function body.
    pub(super) fn parse_object_method_definition(
        &mut self,
        function_span: Span,
        method_kind: DefineMethodKind,
    ) -> Result<FunctionId, Error> {
        let parsed = self.parse_function_definition_tail_with_options(
            FunctionDefinitionHeader {
                span: function_span,
                name: None,
                execution_kind: BytecodeFunctionKind::Normal,
                parent_context: self.lexer.context(),
            },
            FunctionDefinitionOptions::object_method(method_kind),
        )?;
        self.emit(IrOp::MakeClosure(parsed.constant))?;
        self.anonymous_function_definition = None;
        Ok(parsed.child)
    }

    /// Parse one public synchronous generator method after its property name
    /// has been consumed. Grammar role remains `Method`; only callable
    /// execution metadata changes to `Generator`.
    pub(super) fn parse_generator_method_definition(
        &mut self,
        function_span: Span,
    ) -> Result<FunctionId, Error> {
        if !self.is_punctuator(Punctuator::LeftParen) {
            return Err(self.syntax_here("invalid property name"));
        }
        let parsed = self.parse_function_definition_tail_with_options(
            FunctionDefinitionHeader {
                span: function_span,
                name: None,
                execution_kind: BytecodeFunctionKind::Generator,
                parent_context: self.lexer.context(),
            },
            FunctionDefinitionOptions::generator_method(),
        )?;
        self.emit(IrOp::MakeClosure(parsed.constant))?;
        self.anonymous_function_definition = None;
        Ok(parsed.child)
    }

    /// Parse a base or derived class constructor using QuickJS's
    /// concise-method binding model. A derived constructor additionally owns
    /// the one-shot lexical `this` binding and allows `super()` through arrows
    /// and direct eval.
    pub(super) fn parse_class_constructor_definition(
        &mut self,
        function_span: Span,
        derived: bool,
    ) -> Result<ParsedFunctionDefinition, Error> {
        let mut options = FunctionDefinitionOptions::object_method(DefineMethodKind::Method);
        options.class_constructor = true;
        options.derived_class_constructor = derived;
        self.parse_function_definition_tail_with_options(
            FunctionDefinitionHeader {
                span: function_span,
                name: None,
                execution_kind: BytecodeFunctionKind::Normal,
                parent_context: self.lexer.context(),
            },
            options,
        )
    }

    fn parse_function_definition_tail_with_options(
        &mut self,
        header: FunctionDefinitionHeader<'source>,
        options: FunctionDefinitionOptions,
    ) -> Result<ParsedFunctionDefinition, Error> {
        let FunctionDefinitionHeader {
            span: function_span,
            name: function_name_token,
            execution_kind,
            parent_context,
        } = header;
        if execution_kind != options.execution_kind {
            return Err(Error::internal(
                "function header and definition execution kinds disagree",
            ));
        }
        let parent = self.current_function;
        let parent_strict = self.functions[parent].strict;
        let function_name = function_name_token
            .as_ref()
            .map(|(identifier, _)| identifier.value.clone());
        let child = self.functions.len();
        let parent_scope = self.functions[parent].current_scope;
        let super_capabilities = match options.kind {
            FunctionKind::Method if options.derived_class_constructor => {
                SuperCapabilities::CALL_AND_PROPERTY
            }
            FunctionKind::Method => SuperCapabilities::PROPERTY,
            FunctionKind::Ordinary => SuperCapabilities::NONE,
            _ => {
                return Err(Error::internal(
                    "ordinary function parser received an invalid function kind",
                ));
            }
        };
        // Parameter initializers are function code. Establish the child before
        // consuming `(` so every initializer, nested closure and pseudo-binding
        // reference is authored in the callee rather than its parent.
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
            FunctionIrOptions {
                function_name,
                private_name_binding: options.private_name_binding && function_name_token.is_some(),
                class_constructor: options.class_constructor,
                derived_class_constructor: options.derived_class_constructor,
                defined_argument_count: 0,
                has_simple_parameter_list: true,
                rest_parameter: None,
                parameters: Vec::new(),
                strict: parent_strict,
                super_capabilities,
            },
        )?);
        self.functions[child].execution_kind = options.execution_kind;
        self.current_function = child;
        let mut child_context = parent_context;
        child_context.strict = parent_strict;
        child_context.generator = options.execution_kind == BytecodeFunctionKind::Generator;
        child_context.async_function = false;
        self.relex_current_with_context(child_context)?;
        let parameter_scan = self.parenthesized_parameter_scan();
        if parameter_scan.is_some_and(|scan| scan.has_assignment) {
            self.activate_parameter_environment_from_scan(
                parameter_scan.and_then(|scan| scan.bound_name_count),
            )?;
        }
        self.expect_punctuator(Punctuator::LeftParen)?;

        let mut parameter_tokens = Vec::new();
        let mut parameter_list_end_span = self.current().span;
        if self.is_punctuator(Punctuator::RightParen) {
            self.advance()?;
        } else {
            loop {
                let is_rest = self.consume_punctuator(Punctuator::Ellipsis)?;
                if let Some(method_kind) = options.object_method_kind {
                    if is_rest && matches!(method_kind, DefineMethodKind::Setter) {
                        return Err(Error::syntax(
                            "invalid number of arguments for getter or setter",
                            source_span(self.current().span),
                        ));
                    }
                }
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
                        parameter_list_end_span = self.current().span;
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
                            "a trailing comma in this parameter list is not implemented yet",
                        ));
                    }
                    continue;
                }
                let token = self.current().clone();
                let TokenKind::Identifier(identifier) = token.kind else {
                    return Err(self.syntax_here("missing formal parameter"));
                };
                validate_identifier(&identifier, token.span, false, IdentifierContext::Argument)?;
                parameter_tokens.push((identifier.clone(), token.span));
                let parameter = identifier.value;
                self.advance()?;
                if is_rest {
                    self.register_rest_identifier_parameter(parameter, token.span)?;
                    if !self.is_punctuator(Punctuator::RightParen) {
                        return Err(self.syntax_here("expecting ')'"));
                    }
                    parameter_list_end_span = self.current().span;
                    self.advance()?;
                    break;
                }
                if self.is_punctuator(Punctuator::Equal) {
                    self.parse_default_identifier_parameter(parameter, token.span)?;
                } else {
                    self.register_plain_identifier_parameter(parameter, token.span)?;
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
            .is_some_and(|expected| self.functions[child].parameters.len() != expected)
        {
            return Err(Error::syntax(
                "invalid number of arguments for getter or setter",
                source_span(parameter_list_end_span),
            ));
        }
        self.expect_punctuator(Punctuator::LeftBrace)?;

        let has_use_strict = self.directive_prologue_has_use_strict(self.cursor, parent_strict)?;
        let strict = parent_strict || has_use_strict;
        child_context.strict = strict;
        self.relex_current_with_context(child_context)?;
        let has_simple_parameter_list = self.functions[child].has_simple_parameter_list;
        if has_use_strict && !has_simple_parameter_list {
            return Err(Error::syntax(
                "\"use strict\" not allowed in function with default or destructuring parameter",
                source_span(self.current().span),
            ));
        }
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
            for (identifier, _) in &parameter_tokens {
                validate_identifier(
                    identifier,
                    strict_validation_span,
                    true,
                    IdentifierContext::Argument,
                )?;
            }
        }
        if options.reject_duplicate_parameters || strict || !has_simple_parameter_list {
            let parameters = &self.functions[child].parameter_names;
            for (index, parameter) in parameters.iter().enumerate() {
                if parameters[..index].contains(parameter) {
                    return Err(Error::syntax(
                        "duplicate argument names not allowed in this context",
                        source_span(self.current().span),
                    ));
                }
            }
        }
        self.functions[child].strict = strict;
        if options.derived_class_constructor {
            self.functions[child].allocate_derived_constructor_pseudo_bindings()?;
        }
        if options.class_constructor {
            // Parameter parsing may replace the initial body scope with a
            // parentless Parameter Environment or a FunctionRoot pattern
            // segment. Install the guard only after that shape is final: it
            // must follow the Parameter Environment's TDZ reset, but precede
            // every default, destructuring, and rest initializer. Emitting it
            // before parsing would both block BindingPattern activation and
            // let the later rest-prologue pass move Rest ahead of the guard.
            let guard_at = if let Some(parameter_scope) = self.functions[child].parameter_scope {
                self.functions[child]
                    .ops
                    .iter()
                    .position(|operation| {
                        matches!(operation.op, IrOp::EnterScope(scope) if scope == parameter_scope)
                    })
                    .and_then(|entry| entry.checked_add(1))
                    .ok_or_else(|| Error::internal("class constructor lost its parameter scope"))?
            } else {
                0
            };
            let mut guard = vec![SpannedIrOp {
                op: IrOp::Bytecode(Instruction::CheckCtor),
                pc_site: None,
            }];
            if !options.derived_class_constructor {
                guard.extend([
                    SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::PushThis),
                        pc_site: None,
                    },
                    SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::PushActiveFunction),
                        pc_site: None,
                    },
                    SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::CallClassInstanceInitializer),
                        pc_site: None,
                    },
                    SpannedIrOp {
                        op: IrOp::Bytecode(Instruction::Drop),
                        pc_site: None,
                    },
                ]);
            }
            insert_hoist_fragment(&mut self.functions[child], guard_at, guard)?;
        }
        self.finish_identifier_parameter_environment()?;
        if options.execution_kind == BytecodeFunctionKind::Generator {
            self.insert_generator_initial_yield()?;
        }
        self.functions[child].in_function_body = true;
        self.parse_function_body()?;
        let closing_brace = self.current().span;
        self.relex_current_with_context(parent_context)?;
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
