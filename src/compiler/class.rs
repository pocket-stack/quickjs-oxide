//! Class parsing and lowering.
//!
//! This is the first vertical slice of QuickJS `js_parse_class`: class name
//! scopes, heritage, base/derived constructors, `super()` and synchronous
//! methods/accessors, fields, private methods/accessors, and static blocks.
//! Public and private synchronous generator methods are supported; async
//! public methods reuse the ordinary async method path. Private async methods
//! and async generators remain explicit typed frontiers.

use super::function::ParsedFunctionDefinition;
use super::*;
use crate::bytecode::DefineMethodKind;
use crate::lexer::quickjs_simple_lookahead_has_line_terminator;

mod fields;
mod private;
mod static_block;

use fields::ClassElementState;

#[derive(Clone, Debug)]
enum ClassPropertyKey {
    Fixed { value: JsString },
    Computed,
    Private { name: String, span: Span },
}

impl<'source> Parser<'source> {
    pub(super) fn parse_class_declaration(&mut self) -> Result<(), Error> {
        self.parse_class(false)
    }

    pub(super) fn parse_class_expression(&mut self) -> Result<(), Error> {
        self.parse_class(true)
    }

    /// Parse one ClassDefinition while retaining QuickJS's constructor
    /// constant patching model. `DefineClass` must execute before computed
    /// method keys, so a placeholder closure operand is emitted first and
    /// patched after the body reveals an explicit or default constructor.
    fn parse_class(&mut self, expression: bool) -> Result<(), Error> {
        let class_token = self.current().clone();
        let class_start = source_offset(class_token.span)?;
        let outer_strict = self.current_ir().strict;

        // QuickJS temporarily parses the class under JS_MODE_STRICT, which
        // controls reserved words and class-method compilation. Its later
        // outer-function variable-resolution pass runs after restoring the
        // surrounding mode, so evaluated computed keys retain that outer
        // function's runtime strictness. Do not synthesize a frame-mode toggle
        // here; the pinned oracle observably permits sloppy assignment/eval in
        // a computed key inside a sloppy function.
        self.current_ir_mut().strict = true;
        self.relex_current_with_strict(true)?;
        self.advance()?;

        let name = if let TokenKind::Identifier(identifier) = self.current().kind.clone() {
            let span = self.current().span;
            // Pinned QuickJS checks only reserved-identifier status for a
            // ClassBinding, even though it parses the surrounding definition
            // in strict mode. In particular, `eval` and `arguments` remain
            // accepted class names.
            validate_identifier_reservation(&identifier, span, true, IdentifierContext::Variable)?;
            self.advance()?;
            Some((identifier.value, span))
        } else {
            None
        };
        if !expression && name.is_none() {
            return Err(self.syntax_here("class statement requires a name"));
        }

        // The declaration binding is distinct from the immutable inner class
        // name binding. It is registered in the surrounding scope before the
        // class evaluation scope is entered, just like QuickJS JS_VAR_DEF_LET.
        if !expression {
            let (name, span) = name
                .as_ref()
                .ok_or_else(|| Error::internal("class declaration lost its name"))?;
            self.register_lexical_binding(name, *span, self.current().span, false, false)?;
        }

        let class_scope = self.push_scope(ScopeKind::Block);
        if let Some((name, span)) = &name {
            // This scope already covers a future heritage expression, so a
            // same-name `extends C` observes the inner TDZ rather than the
            // declaration outside the class.
            self.register_lexical_binding(name, *span, self.current().span, true, false)?;
        }

        let has_heritage = if matches!(self.current().kind, TokenKind::Keyword(Keyword::Extends)) {
            self.advance_expression_start()?;
            // QuickJS accepts precisely LeftHandSideExpression here. The
            // constructor check is deliberately deferred until after the
            // entire heritage value has been evaluated.
            self.parse_left_hand_side_expression()?;
            self.anonymous_function_definition = None;
            true
        } else {
            self.emit_instruction(Instruction::Undefined)?;
            false
        };
        self.expect_punctuator(Punctuator::LeftBrace)?;
        // QuickJS enters a distinct private-name scope only after heritage has
        // completed. Declarations are added while parsing the complete body,
        // then child-first resolution makes them visible to every element,
        // including references which occur before their declaration.
        let private_scope = self.push_scope(ScopeKind::ClassPrivate);

        let constructor_placeholder = self.emit(IrOp::MakeClosure(u32::MAX))?;
        let class_name = name.as_ref().map_or_else(
            || Ok(JsString::from_static("")),
            |(name, _)| JsString::try_from_utf8(name),
        )?;
        let class_name_constant =
            self.add_constant(IrConstant::Primitive(Value::String(class_name)))?;
        self.emit_instruction(Instruction::DefineClass {
            name: class_name_constant,
            has_heritage,
        })?;

        let mut constructor = None;
        let mut elements = ClassElementState::default();
        while !self.is_punctuator(Punctuator::RightBrace) {
            if self.at_eof() {
                return Err(self.syntax_here("unterminated class body"));
            }
            if self.consume_punctuator(Punctuator::Semicolon)? {
                continue;
            }
            self.parse_class_element(&mut constructor, &mut elements, has_heritage)?;
        }
        let closing_brace = self.current().span;
        self.advance()?;

        let (constructor_constant, constructor_child) = match constructor {
            Some(parsed) => (parsed.constant, parsed.child),
            None if has_heritage => self.synthesize_derived_class_constructor(class_token.span)?,
            None => self.synthesize_base_class_constructor(class_token.span)?,
        };
        let class_end = SourceOffset::try_from_usize(closing_brace.end.byte_offset)
            .map_err(|error| Error::internal(error.to_string()))?;
        {
            let constructor = self
                .functions
                .get_mut(constructor_child)
                .ok_or_else(|| Error::internal("class constructor child disappeared"))?;
            constructor.class_constructor = true;
            constructor.derived_class_constructor = has_heritage;
            constructor.function_name = Some(
                name.as_ref()
                    .map_or_else(String::new, |(name, _)| name.clone()),
            );
            constructor.source.span = class_token.span;
            constructor.source.definition = class_start;
            constructor.source.range = Some(class_start..class_end);
        }
        let placeholder = self
            .current_ir_mut()
            .ops
            .get_mut(constructor_placeholder)
            .ok_or_else(|| Error::internal("class constructor placeholder disappeared"))?;
        let IrOp::MakeClosure(index) = &mut placeholder.op else {
            return Err(Error::internal(
                "class constructor placeholder changed instruction kind",
            ));
        };
        *index = constructor_constant;

        self.finish_class_instance_initializer(&mut elements)?;

        // QuickJS keeps `ctor, proto` throughout method publication, then
        // initializes the private class-name binding only after dropping the
        // prototype. Computed names therefore correctly observe its TDZ.
        self.emit_instruction(Instruction::Drop)?;
        if let Some((name, span)) = &name {
            self.emit_instruction(Instruction::Dup)?;
            self.emit_identifier(name.clone(), *span, IdentifierAccess::Initialize)?;
        }
        let static_initializer_start = self.finish_class_static_initializer(&mut elements)?;
        self.pop_scope(private_scope)?;
        self.pop_scope(class_scope)?;

        self.current_ir_mut().strict = outer_strict;
        self.relex_current_with_strict(outer_strict)?;
        if expression {
            self.anonymous_function_definition =
                name.is_none()
                    .then_some(AnonymousFunctionDefinition::Class {
                        owner: self.current_function,
                        static_initializer_start,
                    });
        } else {
            let (name, span) =
                name.ok_or_else(|| Error::internal("class declaration lost its outer binding"))?;
            self.emit_identifier(name, span, IdentifierAccess::Initialize)?;
            self.anonymous_function_definition = None;
        }
        Ok(())
    }

    fn parse_class_element(
        &mut self,
        constructor: &mut Option<ParsedFunctionDefinition>,
        elements: &mut ClassElementState,
        has_heritage: bool,
    ) -> Result<(), Error> {
        let element_span = self.current().span;
        let mut is_static = false;
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Static)) {
            let next = self.class_token_after_current()?;
            if !matches!(
                next.kind,
                TokenKind::Punctuator(
                    Punctuator::LeftParen
                        | Punctuator::Semicolon
                        | Punctuator::Equal
                        | Punctuator::RightBrace
                )
            ) {
                is_static = true;
                self.advance()?;
                if self.is_punctuator(Punctuator::LeftBrace) {
                    self.parse_class_static_block(elements, element_span)?;
                    return Ok(());
                }
            }
        }

        let function_span = self.current().span;
        let generator = self.is_punctuator(Punctuator::Multiply);
        if generator {
            self.advance()?;
        }
        let asynchronous = !generator && self.contextual_class_async_method_ahead()?;
        if asynchronous {
            self.advance()?;
            if self.is_punctuator(Punctuator::Multiply) {
                return Err(Error::unsupported(
                    "async generator class methods are not implemented yet",
                    source_span(function_span),
                ));
            }
        }

        let mut method_kind = DefineMethodKind::Method;
        if !generator
            && !asynchronous
            && let TokenKind::Identifier(identifier) = &self.current().kind
            && !identifier.has_escape
            && matches!(identifier.value.as_str(), "get" | "set")
        {
            let next = self.class_token_after_current()?;
            if !next.line_terminator_before && Self::class_property_name_starts(&next.kind) {
                method_kind = if identifier.value == "get" {
                    DefineMethodKind::Getter
                } else {
                    DefineMethodKind::Setter
                };
                self.advance()?;
            }
        }

        if is_static {
            self.emit_instruction(Instruction::Swap)?;
        }
        let key = self.parse_class_property_key()?;
        if !self.is_punctuator(Punctuator::LeftParen) {
            if generator || asynchronous {
                return Err(self.syntax_here("invalid property name"));
            }
            if method_kind != DefineMethodKind::Method {
                return Err(self.syntax_here("invalid class field"));
            }
            match key {
                ClassPropertyKey::Private { name, span } => {
                    self.parse_private_class_field(elements, is_static, name, span)?;
                }
                key => self.parse_public_class_field(elements, is_static, key, function_span)?,
            }
            if is_static {
                self.emit_instruction(Instruction::Swap)?;
            }
            return Ok(());
        }

        if let ClassPropertyKey::Private { name, span } = &key {
            if asynchronous {
                return Err(Error::unsupported(
                    "private async class methods are not implemented yet",
                    source_span(function_span),
                ));
            }
            if method_kind == DefineMethodKind::Method {
                self.parse_private_class_method(
                    elements,
                    is_static,
                    name.clone(),
                    *span,
                    function_span,
                    generator,
                )?;
            } else {
                self.parse_private_class_accessor(
                    elements,
                    is_static,
                    name.clone(),
                    *span,
                    function_span,
                    method_kind,
                )?;
            }
            if is_static {
                self.emit_instruction(Instruction::Swap)?;
            }
            return Ok(());
        }

        let fixed = match &key {
            ClassPropertyKey::Fixed { value } => Some(value),
            ClassPropertyKey::Computed => None,
            ClassPropertyKey::Private { .. } => unreachable!("private methods were lowered"),
        };
        let is_constructor_name =
            fixed.is_some_and(|name| *name == JsString::from_static("constructor"));
        if !is_static
            && is_constructor_name
            && (method_kind != DefineMethodKind::Method || generator || asynchronous)
        {
            return Err(Error::syntax(
                "invalid method name",
                source_span(function_span),
            ));
        }
        if is_static && fixed.is_some_and(|name| *name == JsString::from_static("prototype")) {
            return Err(Error::syntax(
                "invalid method name",
                source_span(function_span),
            ));
        }

        if !is_static
            && !generator
            && !asynchronous
            && method_kind == DefineMethodKind::Method
            && is_constructor_name
        {
            if constructor.is_some() {
                return Err(Error::syntax(
                    "property constructor appears more than once",
                    source_span(function_span),
                ));
            }
            *constructor =
                Some(self.parse_class_constructor_definition(function_span, has_heritage)?);
            self.anonymous_function_definition = None;
        } else {
            if generator {
                self.parse_generator_method_definition(function_span)?;
            } else if asynchronous {
                self.parse_async_method_definition(function_span)?;
            } else {
                self.parse_object_method_definition(function_span, method_kind)?;
            }
            match key {
                ClassPropertyKey::Fixed { value } => {
                    let key = self.add_constant(IrConstant::Primitive(Value::String(value)))?;
                    self.emit_instruction(Instruction::DefineMethod {
                        key,
                        kind: method_kind,
                        enumerable: false,
                    })?;
                }
                ClassPropertyKey::Computed => {
                    self.emit_instruction(Instruction::DefineMethodComputed {
                        kind: method_kind,
                        enumerable: false,
                    })?;
                }
                ClassPropertyKey::Private { .. } => unreachable!("private methods were lowered"),
            }
        }
        if is_static {
            self.emit_instruction(Instruction::Swap)?;
        }
        Ok(())
    }

    fn parse_class_property_key(&mut self) -> Result<ClassPropertyKey, Error> {
        let token = self.current().clone();
        let value = match token.kind {
            TokenKind::Identifier(identifier) => {
                self.advance()?;
                JsString::try_from_utf8(&identifier.value)?
            }
            TokenKind::Keyword(keyword) => {
                self.advance()?;
                JsString::from_static(keyword.as_str())
            }
            TokenKind::String(string) => {
                if string.has_legacy_octal_escape {
                    return Err(Error::syntax(
                        "legacy octal escapes are forbidden in strict mode",
                        source_span(token.span),
                    ));
                }
                self.advance()?;
                JsString::try_from_utf16(string.value.utf16)?
            }
            TokenKind::Number(number) => {
                if matches!(
                    number.kind,
                    NumberKind::LegacyOctal | NumberKind::LegacyDecimal
                ) {
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
            TokenKind::Punctuator(Punctuator::LeftBracket) => {
                self.advance_expression_start()?;
                self.parse_assignment_allow_in()?;
                self.emit_instruction(Instruction::ToPropKey)?;
                self.expect_punctuator(Punctuator::RightBracket)?;
                return Ok(ClassPropertyKey::Computed);
            }
            TokenKind::PrivateIdentifier(identifier) => {
                if identifier.value == "constructor" {
                    return Err(Error::syntax(
                        "invalid method name",
                        source_span(token.span),
                    ));
                }
                self.advance()?;
                return Ok(ClassPropertyKey::Private {
                    name: private_reference::private_binding_name(&identifier.value),
                    span: token.span,
                });
            }
            _ => return Err(self.syntax_here("invalid property name")),
        };
        Ok(ClassPropertyKey::Fixed { value })
    }

    fn contextual_class_async_method_ahead(&self) -> Result<bool, Error> {
        let TokenKind::Identifier(identifier) = &self.current().kind else {
            return Ok(false);
        };
        if identifier.value != "async" || identifier.has_escape {
            return Ok(false);
        }
        let next = self.class_token_after_current()?;
        let has_line_terminator = quickjs_simple_lookahead_has_line_terminator(
            &self.lexer.source()[self.current().span.end.byte_offset..],
        );
        Ok(!has_line_terminator
            && (Self::class_property_name_starts(&next.kind)
                || matches!(
                    next.kind,
                    TokenKind::Punctuator(Punctuator::Multiply | Punctuator::Semicolon)
                )))
    }

    fn class_property_name_starts(kind: &TokenKind<'_>) -> bool {
        matches!(
            kind,
            TokenKind::Identifier(_)
                | TokenKind::Keyword(_)
                | TokenKind::String(_)
                | TokenKind::Number(_)
                | TokenKind::PrivateIdentifier(_)
                | TokenKind::Punctuator(Punctuator::LeftBracket)
        )
    }

    fn class_token_after_current(&self) -> Result<Token<'source>, Error> {
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        lexer.next_token().map_err(lex_error)
    }

    fn synthesize_base_class_constructor(
        &mut self,
        class_span: Span,
    ) -> Result<(u32, FunctionId), Error> {
        let parent = self.current_function;
        let child = self.functions.len();
        let definition_scope = self.current_ir().current_scope;
        self.functions.push(FunctionIr::new(
            Some(ParentLink {
                function: parent,
                definition_scope,
            }),
            FunctionKind::Method,
            FunctionSourceInfo {
                span: class_span,
                definition: source_offset(class_span)?,
                range: None,
            },
            FunctionIrOptions {
                function_name: None,
                private_name_binding: false,
                class_constructor: true,
                derived_class_constructor: false,
                parameters: Vec::new(),
                defined_argument_count: 0,
                has_simple_parameter_list: true,
                rest_parameter: None,
                strict: true,
                super_capabilities: SuperCapabilities::PROPERTY,
            },
        )?);
        self.current_function = child;
        self.emit_instruction(Instruction::CheckCtor)?;
        self.emit_instruction(Instruction::PushThis)?;
        self.emit_instruction(Instruction::PushActiveFunction)?;
        self.emit_instruction(Instruction::CallClassInstanceInitializer)?;
        self.emit_instruction(Instruction::Drop)?;
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::Return)?;
        self.current_function = parent;
        let constant = self.add_constant(IrConstant::Child(child))?;
        Ok((constant, child))
    }

    /// QuickJS's default derived constructor forwards the frame's raw
    /// argc/argv through `OP_init_ctor`; it does not materialize a rest Array
    /// whose iterator could be monkey-patched. `InitDerivedConstructor` keeps
    /// that same non-observable forwarding boundary in the typed VM.
    fn synthesize_derived_class_constructor(
        &mut self,
        class_span: Span,
    ) -> Result<(u32, FunctionId), Error> {
        let parent = self.current_function;
        let child = self.functions.len();
        let definition_scope = self.current_ir().current_scope;
        self.functions.push(FunctionIr::new(
            Some(ParentLink {
                function: parent,
                definition_scope,
            }),
            FunctionKind::Method,
            FunctionSourceInfo {
                span: class_span,
                definition: source_offset(class_span)?,
                range: None,
            },
            FunctionIrOptions {
                function_name: None,
                private_name_binding: false,
                class_constructor: true,
                derived_class_constructor: true,
                parameters: Vec::new(),
                defined_argument_count: 0,
                has_simple_parameter_list: true,
                rest_parameter: None,
                strict: true,
                super_capabilities: SuperCapabilities::CALL_AND_PROPERTY,
            },
        )?);
        self.functions[child].allocate_derived_constructor_pseudo_bindings()?;
        self.current_function = child;
        let this = self
            .current_ir()
            .this_local
            .ok_or_else(|| Error::internal("derived constructor has no this binding"))?;
        self.emit_instruction(Instruction::CheckCtor)?;
        self.emit_instruction(Instruction::InitDerivedConstructor)?;
        self.emit_instruction(Instruction::Dup)?;
        self.emit_instruction(Instruction::InitializeDerivedLocal(this))?;
        let active = self
            .current_ir()
            .active_function_local
            .ok_or_else(|| Error::internal("derived constructor has no active function"))?;
        self.emit_instruction(Instruction::GetLocal(active))?;
        self.emit_instruction(Instruction::CallClassInstanceInitializer)?;
        self.emit_instruction(Instruction::ReturnDerived(this))?;
        self.current_function = parent;
        let constant = self.add_constant(IrConstant::Child(child))?;
        Ok((constant, child))
    }
}
