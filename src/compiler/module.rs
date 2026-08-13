//! Static ECMAScript module parsing and root-binding lowering.
//!
//! The layout follows QuickJS 2026-06-04's `JSModuleDef` boundary: module
//! declarations/imports are closure slots on the synthetic root function,
//! while this parser-owned record retains source-order request and export
//! tables until those slots have been seeded and linked.

use super::*;
use crate::module::{
    MODULE_DEFAULT_BINDING_NAME, MODULE_IMPORT_META_BINDING_NAME, ModuleExport, ModuleExportTarget,
    ModuleImport, ModuleImportAttributes, ModuleImportCollision, ModuleImportCollisionDeclaration,
    ModuleImportName, ModuleLinkInitializer, ModuleLinkInitializerValue, ModuleRequest,
    ModuleStarExport, UnlinkedModule, UnlinkedModuleTables,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ModuleBindingId(pub(super) usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModuleDeclarationOrigin {
    Var,
    Lexical {
        is_const: bool,
    },
    Function {
        constant: u32,
        inferred_name: Option<u32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModuleImportKind {
    Named,
    Namespace,
}

#[derive(Clone, Debug)]
pub(super) struct IrModuleBinding {
    pub(super) name: String,
    pub(super) declaration: Option<ModuleDeclarationOrigin>,
    pub(super) declaration_scope: Option<ScopeId>,
    pub(super) import: Option<ModuleImportKind>,
    pub(super) is_import_meta: bool,
    pub(super) closure_index: Option<u16>,
}

#[derive(Clone, Debug)]
pub(super) struct IrModuleLocalExport {
    local_name: String,
    export_name: JsString,
    span: Span,
    binding: Option<ModuleBindingId>,
}

#[derive(Clone, Debug)]
struct IrModuleImport {
    request: crate::module::ModuleRequestIndex,
    import_name: ModuleImportName,
    binding: ModuleBindingId,
}

#[derive(Debug, Default)]
pub(super) struct IrModule {
    pub(super) bindings: Vec<IrModuleBinding>,
    pub(super) declaration_order: Vec<ModuleBindingId>,
    requested_modules: Vec<ModuleRequest>,
    imports: Vec<IrModuleImport>,
    local_exports: Vec<IrModuleLocalExport>,
    exports: Vec<ModuleExport>,
    star_exports: Vec<ModuleStarExport>,
}

impl IrModule {
    pub(super) fn binding_id(&self, name: &str) -> Option<ModuleBindingId> {
        self.bindings
            .iter()
            .position(|binding| binding.name == name)
            .map(ModuleBindingId)
    }

    pub(super) fn binding(&self, id: ModuleBindingId) -> Result<&IrModuleBinding, Error> {
        self.bindings
            .get(id.0)
            .ok_or_else(|| Error::internal("module binding id is out of bounds"))
    }

    pub(super) fn binding_mut(
        &mut self,
        id: ModuleBindingId,
    ) -> Result<&mut IrModuleBinding, Error> {
        self.bindings
            .get_mut(id.0)
            .ok_or_else(|| Error::internal("module binding id is out of bounds"))
    }

    fn add_local_export(
        &mut self,
        local_name: String,
        export_name: JsString,
        span: Span,
    ) -> Result<(), Error> {
        if self
            .local_exports
            .iter()
            .any(|entry| entry.export_name == export_name)
            || self
                .exports
                .iter()
                .any(|entry| entry.export_name == export_name)
        {
            let mut message = NativeErrorMessage::new();
            message.push_utf8("duplicate exported name '");
            export_name.push_atom_get_str_to(&mut message);
            message.push_utf8("'");
            return Err(
                Error::from_native_message(ErrorKind::Syntax, message).with_span(source_span(span))
            );
        }
        self.local_exports.push(IrModuleLocalExport {
            local_name,
            export_name,
            span,
            binding: None,
        });
        Ok(())
    }

    fn add_indirect_export(
        &mut self,
        export_name: JsString,
        target: ModuleExportTarget,
        span: Span,
    ) -> Result<(), Error> {
        if self
            .local_exports
            .iter()
            .any(|entry| entry.export_name == export_name)
            || self
                .exports
                .iter()
                .any(|entry| entry.export_name == export_name)
        {
            let mut message = NativeErrorMessage::new();
            message.push_utf8("duplicate exported name '");
            export_name.push_atom_get_str_to(&mut message);
            message.push_utf8("'");
            return Err(
                Error::from_native_message(ErrorKind::Syntax, message).with_span(source_span(span))
            );
        }
        self.exports.push(ModuleExport {
            export_name,
            target,
        });
        Ok(())
    }
}

impl<'source> Parser<'source> {
    pub(super) fn parse_module_body(
        &mut self,
        checker: &mut Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<(), ModuleCompileFailure> {
        while !self.at_eof() {
            if matches!(self.current().kind, TokenKind::Keyword(Keyword::Export)) {
                self.parse_module_export(checker)?;
            } else if matches!(self.current().kind, TokenKind::Keyword(Keyword::Import))
                && self.static_import_declaration_ahead()?
            {
                self.parse_module_import(checker)?;
            } else {
                self.parse_statement_or_decl(
                    StatementCompletion::Discard,
                    StatementPosition::ProgramBody,
                )?;
            }
        }
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::Return)?;
        Ok(())
    }

    /// QuickJS distinguishes static ImportDeclaration from ImportCall and
    /// `import.meta` with one non-committing token lookahead.
    fn static_import_declaration_ahead(&self) -> Result<bool, Error> {
        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let next = lexer.next_token().map_err(lex_error)?;
        Ok(!matches!(
            next.kind,
            TokenKind::Punctuator(Punctuator::LeftParen | Punctuator::Dot)
        ))
    }

    fn parse_module_export(
        &mut self,
        checker: &mut Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<(), ModuleCompileFailure> {
        let export_span = self.current().span;
        self.advance()?;
        match self.current().kind {
            TokenKind::Punctuator(Punctuator::LeftBrace) => {
                self.parse_module_export_clause(checker)?
            }
            TokenKind::Keyword(Keyword::Var) => self.with_module_declaration_export(
                ModuleDeclarationExport::Named,
                Self::parse_var_statement,
            )?,
            TokenKind::Keyword(Keyword::Let | Keyword::Const) => self
                .with_module_declaration_export(
                    ModuleDeclarationExport::Named,
                    Self::parse_lexical_statement,
                )?,
            TokenKind::Keyword(Keyword::Function) => {
                self.parse_module_function_declaration(ModuleDeclarationExport::Named)?
            }
            TokenKind::Identifier(_) if self.async_function_ahead() => {
                self.parse_module_function_declaration(ModuleDeclarationExport::Named)?
            }
            TokenKind::Keyword(Keyword::Class) => self.with_module_declaration_export(
                ModuleDeclarationExport::Named,
                Self::parse_class_declaration,
            )?,
            TokenKind::Keyword(Keyword::Default) => {
                self.parse_module_default_export(export_span)?
            }
            TokenKind::Punctuator(Punctuator::Multiply) => {
                self.parse_module_star_export(checker)?
            }
            _ => return Err(self.syntax_here("invalid export syntax").into()),
        }
        Ok(())
    }

    fn parse_module_import(
        &mut self,
        checker: &mut Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<(), ModuleCompileFailure> {
        self.advance()?;

        if matches!(self.current().kind, TokenKind::String(_)) {
            let mut request = self.parse_module_specifier()?;
            request.attributes = self.parse_module_import_attributes(checker)?;
            self.add_module_request(request)?;
            return Ok(self.consume_statement_terminator()?);
        }

        let mut imports = Vec::new();
        let parse_secondary_clause = if matches!(self.current().kind, TokenKind::Identifier(_)) {
            let (local_name, local_span) = self.module_binding_identifier()?;
            let binding = self.register_module_import_binding(&local_name, local_span, false)?;
            imports.push((
                binding,
                ModuleImportName::Name(JsString::try_from_utf8("default")?),
            ));
            self.consume_punctuator(Punctuator::Comma)?
        } else {
            true
        };

        if parse_secondary_clause && self.is_punctuator(Punctuator::Multiply) {
            self.advance()?;
            if !self.is_contextual_keyword("as") {
                return Err(self.syntax_here("expecting 'as'").into());
            }
            self.advance()?;
            let (local_name, local_span) = self.module_binding_identifier()?;
            let binding = self.register_module_import_binding(&local_name, local_span, true)?;
            imports.push((binding, ModuleImportName::Namespace));
        } else if parse_secondary_clause && self.is_punctuator(Punctuator::LeftBrace) {
            self.expect_punctuator(Punctuator::LeftBrace)?;
            while !self.is_punctuator(Punctuator::RightBrace) {
                let imported_token = self.current().clone();
                let import_name = self.module_export_name()?;
                let (local_name, local_span) = if self.is_contextual_keyword("as") {
                    self.advance()?;
                    self.module_binding_identifier()?
                } else {
                    let TokenKind::Identifier(identifier) = imported_token.kind else {
                        return Err(Error::syntax(
                            "imported keyword requires an 'as' binding",
                            source_span(imported_token.span),
                        )
                        .into());
                    };
                    validate_identifier_reservation(
                        &identifier,
                        imported_token.span,
                        true,
                        IdentifierContext::Variable,
                    )?;
                    (identifier.value, imported_token.span)
                };
                let binding =
                    self.register_module_import_binding(&local_name, local_span, false)?;
                imports.push((binding, ModuleImportName::Name(import_name)));
                if !self.consume_punctuator(Punctuator::Comma)? {
                    break;
                }
            }
            self.expect_punctuator(Punctuator::RightBrace)?;
        } else if parse_secondary_clause {
            return Err(self
                .syntax_here("default, namespace, or named imports expected")
                .into());
        }
        let request_index = self.parse_module_from_clause(checker)?;
        let module = self.module_ir_mut()?;
        module.imports.extend(
            imports
                .into_iter()
                .map(|(binding, import_name)| IrModuleImport {
                    request: request_index,
                    import_name,
                    binding,
                }),
        );
        Ok(self.consume_statement_terminator()?)
    }

    fn add_module_request(
        &mut self,
        request: ModuleRequest,
    ) -> Result<crate::module::ModuleRequestIndex, Error> {
        let module = self.module_ir_mut()?;
        let index = u32::try_from(module.requested_modules.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many requested modules"))?;
        module.requested_modules.push(request);
        Ok(crate::module::ModuleRequestIndex(index))
    }

    fn parse_module_from_clause(
        &mut self,
        checker: &mut Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<crate::module::ModuleRequestIndex, ModuleCompileFailure> {
        if !self.is_contextual_keyword("from") {
            return Err(self.syntax_here("from clause expected").into());
        }
        self.advance()?;
        let mut request = self.parse_module_specifier()?;
        request.attributes = self.parse_module_import_attributes(checker)?;
        Ok(self.add_module_request(request)?)
    }

    fn parse_module_specifier(&mut self) -> Result<ModuleRequest, Error> {
        let token = self.current().clone();
        let TokenKind::String(literal) = token.kind else {
            return Err(self.syntax_here("string expected"));
        };
        let specifier = JsString::try_from_utf16(literal.value.utf16)?;
        self.advance()?;
        Ok(ModuleRequest {
            specifier,
            attributes: ModuleImportAttributes::Absent,
        })
    }

    fn parse_module_import_attributes(
        &mut self,
        checker: &mut Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<ModuleImportAttributes, ModuleCompileFailure> {
        if !self.is_contextual_keyword("with")
            && !matches!(self.current().kind, TokenKind::Keyword(Keyword::With))
        {
            return Ok(ModuleImportAttributes::Absent);
        }

        self.advance()?;
        self.expect_punctuator(Punctuator::LeftBrace)?;
        let mut attributes = Vec::new();
        while !self.is_punctuator(Punctuator::RightBrace) {
            let key_token = self.current().clone();
            let key = match key_token.kind {
                TokenKind::String(literal) => JsString::try_from_utf16(literal.value.utf16)?,
                TokenKind::Identifier(identifier) => JsString::try_from_utf8(&identifier.value)?,
                TokenKind::Keyword(keyword) => JsString::try_from_utf8(keyword.as_str())?,
                _ => return Err(self.syntax_here("identifier expected").into()),
            };
            self.advance()?;
            self.expect_punctuator(Punctuator::Colon)?;

            let value_token = self.current().clone();
            let TokenKind::String(literal) = value_token.kind else {
                // `js_parse_with_clause` intentionally reports a non-string
                // value at the beginning of its key, not at the value token.
                return Err(Error::syntax("string expected", source_span(key_token.span)).into());
            };
            if attributes
                .iter()
                .any(|attribute: &ModuleImportAttribute| attribute.key == key)
            {
                // QuickJS checks for the duplicate while the value token is
                // current, after proving that it is a StringLiteral.
                return Err(self.syntax_here("duplicate with key").into());
            }
            attributes.push(ModuleImportAttribute {
                key,
                value: JsString::try_from_utf16(literal.value.utf16)?,
            });
            self.advance()?;
            if !self.consume_punctuator(Punctuator::Comma)? {
                break;
            }
        }

        let attributes = ModuleImportAttributes::Present(attributes.into_boxed_slice());
        if let Some(effective) = attributes.effective()
            && let Some(checker) = checker.as_deref_mut()
        {
            checker.check(effective)?;
        }
        self.expect_punctuator(Punctuator::RightBrace)?;
        Ok(attributes)
    }

    fn module_binding_identifier(&mut self) -> Result<(String, Span), Error> {
        let token = self.current().clone();
        let TokenKind::Identifier(identifier) = token.kind else {
            return Err(self.syntax_here("identifier expected"));
        };
        // QuickJS parses a ModuleImportBinding before `add_import` applies the
        // module-specific `eval`/`arguments` early error. Keep the ordinary
        // reserved-word validation here, but let those two strict names reach
        // `register_module_import_binding` so the observable diagnostic and
        // current-token location follow the module grammar path.
        validate_identifier_reservation(
            &identifier,
            token.span,
            true,
            IdentifierContext::Variable,
        )?;
        self.advance()?;
        Ok((identifier.value, token.span))
    }

    fn register_module_import_binding(
        &mut self,
        name: &str,
        span: Span,
        is_namespace: bool,
    ) -> Result<ModuleBindingId, Error> {
        if matches!(name, "eval" | "arguments") {
            // QuickJS `add_import` diagnoses the binding after its parser has
            // advanced to the following token. `syntax_here` intentionally
            // preserves that current-token position instead of pointing back
            // at the local identifier.
            return Err(self.syntax_here("invalid import binding"));
        }
        if !matches!(self.current_ir().kind, FunctionKind::Module)
            || self.current_ir().current_scope != self.current_ir().body_scope
        {
            return Err(Error::internal(
                "module import binding escaped the module body",
            ));
        }
        let import = if is_namespace {
            ModuleImportKind::Namespace
        } else {
            ModuleImportKind::Named
        };
        let binding = {
            let module = self.module_ir_mut()?;
            if let Some(binding) = module.binding_id(name) {
                let record = module.binding_mut(binding)?;
                if record.import.is_some() {
                    return Err(Error::syntax(
                        "invalid redefinition of lexical identifier",
                        source_span(span),
                    ));
                }
                record.import = Some(import);
                binding
            } else {
                let binding = ModuleBindingId(module.bindings.len());
                module.bindings.push(IrModuleBinding {
                    name: name.to_owned(),
                    declaration: None,
                    declaration_scope: None,
                    import: Some(import),
                    is_import_meta: false,
                    closure_index: None,
                });
                binding
            }
        };

        let scope = self.current_ir().current_scope;
        if let Some(existing) = self.current_ir().binding_id_in_scope(scope, name) {
            let record = self
                .current_ir_mut()
                .bindings
                .get_mut(existing.0)
                .ok_or_else(|| Error::internal("module import binding moved"))?;
            if record.storage != BindingStorage::Module(binding) {
                return Err(Error::internal(
                    "module import collision resolved to different storage",
                ));
            }
            record.kind = BindingKind::Lexical { is_const: true };
        } else {
            let function = self.current_ir_mut();
            function.add_binding(
                scope,
                scope,
                name.to_owned(),
                BindingStorage::Module(binding),
                BindingKind::Lexical { is_const: true },
                Some(span),
            );
        }
        Ok(binding)
    }

    fn parse_module_export_clause(
        &mut self,
        checker: &mut Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<(), ModuleCompileFailure> {
        self.expect_punctuator(Punctuator::LeftBrace)?;
        let mut entries = Vec::new();
        while !self.is_punctuator(Punctuator::RightBrace) {
            let (local_name, local_span) = self.module_identifier_name()?;
            let export_name = if self.is_contextual_keyword("as") {
                self.advance()?;
                self.module_export_name()?
            } else {
                JsString::try_from_utf8(&local_name)?
            };
            entries.push((local_name, export_name, local_span));
            if !self.consume_punctuator(Punctuator::Comma)? {
                break;
            }
        }
        self.expect_punctuator(Punctuator::RightBrace)?;
        if self.is_contextual_keyword("from") {
            let request = self.parse_module_from_clause(checker)?;
            let module = self.module_ir_mut()?;
            for (import_name, export_name, span) in entries {
                module.add_indirect_export(
                    export_name,
                    ModuleExportTarget::Indirect {
                        request,
                        import_name: ModuleImportName::Name(JsString::try_from_utf8(&import_name)?),
                    },
                    span,
                )?;
            }
        } else {
            let module = self.module_ir_mut()?;
            for (local_name, export_name, span) in entries {
                module.add_local_export(local_name, export_name, span)?;
            }
        }
        Ok(self.consume_statement_terminator()?)
    }

    fn parse_module_star_export(
        &mut self,
        checker: &mut Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<(), ModuleCompileFailure> {
        self.expect_punctuator(Punctuator::Multiply)?;
        if self.is_contextual_keyword("as") {
            self.advance()?;
            // Pinned QuickJS 2026-06-04 accepts IdentifierName here, including
            // keywords, but deliberately does not accept a StringLiteral.
            let (export_name, span) = self.module_identifier_name()?;
            let request = self.parse_module_from_clause(checker)?;
            self.module_ir_mut()?.add_indirect_export(
                JsString::try_from_utf8(&export_name)?,
                ModuleExportTarget::Indirect {
                    request,
                    import_name: ModuleImportName::Namespace,
                },
                span,
            )?;
        } else {
            let request = self.parse_module_from_clause(checker)?;
            self.module_ir_mut()?
                .star_exports
                .push(ModuleStarExport { request });
        }
        Ok(self.consume_statement_terminator()?)
    }

    fn parse_module_default_export(&mut self, export_span: Span) -> Result<(), Error> {
        let default_span = self.current().span;
        self.advance()?;
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Class)) {
            return self.with_module_declaration_export(
                ModuleDeclarationExport::Default,
                Self::parse_class_declaration,
            );
        }
        if matches!(self.current().kind, TokenKind::Keyword(Keyword::Function))
            || matches!(self.current().kind, TokenKind::Identifier(_))
                && self.async_function_ahead()
        {
            return self.parse_module_function_declaration(ModuleDeclarationExport::Default);
        }

        self.parse_assignment_allow_in()?;
        if let Some(definition) = self.take_anonymous_function_definition() {
            let name = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8("default")?,
            )))?;
            self.emit_anonymous_set_name(definition, Instruction::SetName(name))?;
        }

        let binding = if let Some(binding) = self
            .module_ir_mut()?
            .binding_id(MODULE_DEFAULT_BINDING_NAME)
        {
            binding
        } else {
            self.register_lexical_binding(
                MODULE_DEFAULT_BINDING_NAME,
                default_span,
                default_span,
                false,
                false,
            )?;
            self.module_ir_mut()?
                .binding_id(MODULE_DEFAULT_BINDING_NAME)
                .ok_or_else(|| Error::internal("default module binding was not registered"))?
        };
        self.emit_identifier(
            MODULE_DEFAULT_BINDING_NAME.to_owned(),
            default_span,
            IdentifierAccess::Initialize,
        )?;
        let actual_name = self.module_ir_mut()?.binding(binding)?.name.clone();
        self.module_ir_mut()?.add_local_export(
            actual_name,
            JsString::try_from_utf8("default")?,
            export_span,
        )?;
        self.consume_statement_terminator()
    }

    fn module_identifier_name(&mut self) -> Result<(String, Span), Error> {
        let token = self.current().clone();
        let name = match token.kind {
            TokenKind::Identifier(identifier) => identifier.value,
            TokenKind::Keyword(keyword) => keyword.as_str().to_owned(),
            _ => return Err(self.syntax_here("identifier expected")),
        };
        self.advance()?;
        Ok((name, token.span))
    }

    fn module_export_name(&mut self) -> Result<JsString, Error> {
        let token = self.current().clone();
        let name = match token.kind {
            TokenKind::String(literal) => {
                String::from_utf16(&literal.value.utf16).map_err(|_| {
                    Error::syntax("contains unpaired surrogate", source_span(token.span))
                })?;
                JsString::try_from_utf16(literal.value.utf16)?
            }
            TokenKind::Identifier(identifier) => JsString::try_from_utf8(&identifier.value)?,
            TokenKind::Keyword(keyword) => JsString::try_from_utf8(keyword.as_str())?,
            _ => return Err(self.syntax_here("identifier expected")),
        };
        self.advance()?;
        Ok(name)
    }

    fn is_contextual_keyword(&self, expected: &str) -> bool {
        matches!(
            &self.current().kind,
            TokenKind::Identifier(identifier)
                if identifier.value == expected && !identifier.has_escape
        )
    }

    fn with_module_declaration_export(
        &mut self,
        export: ModuleDeclarationExport,
        parse: fn(&mut Self) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if export == ModuleDeclarationExport::None
            || self.module_declaration_export != ModuleDeclarationExport::None
            || self.module_declaration_export_target.is_some()
        {
            return Err(Error::internal(
                "nested module export declaration marker is malformed",
            ));
        }
        self.module_declaration_export = export;
        self.module_declaration_export_target =
            Some((self.current_function, self.current_ir().current_scope));
        let result = parse(self);
        self.module_declaration_export = ModuleDeclarationExport::None;
        self.module_declaration_export_target = None;
        result
    }

    pub(super) fn current_module_declaration_export(&self) -> ModuleDeclarationExport {
        if self.module_declaration_export_target
            == Some((self.current_function, self.current_ir().current_scope))
        {
            self.module_declaration_export
        } else {
            ModuleDeclarationExport::None
        }
    }

    pub(super) fn add_module_binding(
        &mut self,
        name: &str,
        declaration: ModuleDeclarationOrigin,
    ) -> Result<ModuleBindingId, Error> {
        let declaration_scope = self.current_ir().current_scope;
        let module = self.module_ir_mut()?;
        if let Some(id) = module.binding_id(name) {
            let first_declaration = module.binding(id)?.declaration.is_none();
            let replaces_with_function =
                matches!(declaration, ModuleDeclarationOrigin::Function { .. });
            if let ModuleDeclarationOrigin::Function {
                constant,
                inferred_name,
            } = declaration
            {
                module.binding_mut(id)?.declaration = Some(ModuleDeclarationOrigin::Function {
                    constant,
                    inferred_name,
                });
            } else if first_declaration {
                module.binding_mut(id)?.declaration = Some(declaration);
            }
            if first_declaration {
                module.binding_mut(id)?.declaration_scope = Some(declaration_scope);
                module.declaration_order.push(id);
            } else if replaces_with_function {
                let position = module
                    .declaration_order
                    .iter()
                    .position(|candidate| *candidate == id)
                    .ok_or_else(|| {
                        Error::internal("module declaration order omitted an existing binding")
                    })?;
                module.declaration_order.remove(position);
                module.declaration_order.push(id);
            }
            return Ok(id);
        }
        let id = ModuleBindingId(module.bindings.len());
        module.bindings.push(IrModuleBinding {
            name: name.to_owned(),
            declaration: Some(declaration),
            declaration_scope: Some(declaration_scope),
            import: None,
            is_import_meta: false,
            closure_index: None,
        });
        module.declaration_order.push(id);
        Ok(id)
    }

    pub(super) fn ensure_module_import_meta_binding(&mut self) -> Result<(), Error> {
        let binding = if let Some((index, _)) = self
            .module
            .as_ref()
            .ok_or_else(|| Error::internal("import.meta has no module record"))?
            .bindings
            .iter()
            .enumerate()
            .find(|(_, binding)| binding.is_import_meta)
        {
            ModuleBindingId(index)
        } else {
            let module = self.module_ir_mut()?;
            let binding = ModuleBindingId(module.bindings.len());
            module.bindings.push(IrModuleBinding {
                name: MODULE_IMPORT_META_BINDING_NAME.to_owned(),
                declaration: None,
                declaration_scope: None,
                import: None,
                is_import_meta: true,
                closure_index: None,
            });
            binding
        };

        let root = self
            .functions
            .first_mut()
            .ok_or_else(|| Error::internal("module parser has no root function"))?;
        let scope = root.body_scope;
        if let Some(existing) = root.binding_id_in_scope(scope, MODULE_IMPORT_META_BINDING_NAME) {
            let existing = root
                .bindings
                .get(existing.0)
                .ok_or_else(|| Error::internal("import.meta binding moved"))?;
            if existing.storage != BindingStorage::Module(binding)
                || existing.kind != (BindingKind::Lexical { is_const: true })
            {
                return Err(Error::internal("import.meta binding is malformed"));
            }
        } else {
            root.add_binding(
                scope,
                scope,
                MODULE_IMPORT_META_BINDING_NAME.to_owned(),
                BindingStorage::Module(binding),
                BindingKind::Lexical { is_const: true },
                None,
            );
        }
        Ok(())
    }

    pub(super) fn export_module_declaration(
        &mut self,
        name: &str,
        binding: ModuleBindingId,
        span: Span,
    ) -> Result<(), Error> {
        let export_name = match self.current_module_declaration_export() {
            ModuleDeclarationExport::None => return Ok(()),
            ModuleDeclarationExport::Named => JsString::try_from_utf8(name)?,
            ModuleDeclarationExport::Default => JsString::from_static("default"),
        };
        let module = self.module_ir_mut()?;
        let actual = module.binding(binding)?;
        if actual.name != name {
            return Err(Error::internal("module declaration binding name changed"));
        }
        module.add_local_export(name.to_owned(), export_name, span)
    }

    pub(super) fn parse_module_function_declaration(
        &mut self,
        export: ModuleDeclarationExport,
    ) -> Result<(), Error> {
        if !matches!(self.current_ir().kind, FunctionKind::Module) {
            return Err(Error::internal(
                "module function declaration escaped the module root",
            ));
        }
        let header =
            self.parse_function_definition_header(export != ModuleDeclarationExport::Default)?;
        let source_name = header
            .name
            .as_ref()
            .map(|(identifier, span)| (identifier.value.clone(), *span));
        let (name, declaration_span) = source_name
            .clone()
            .unwrap_or_else(|| (MODULE_DEFAULT_BINDING_NAME.to_owned(), header.span));
        // QuickJS checks only the first same-named module-global record and
        // rejects it when that record was declared at this exact scope level.
        // This is deliberately source- and scope-ordered: a later `var` may
        // reuse an earlier function cell, while a `var` first declared in a
        // nested block does not reject a later Program-level function
        // (`js_parse_function_decl2`, JS_EVAL_TYPE_MODULE, including its
        // pinned `XXX: should check scope chain` behavior).
        if source_name.is_some()
            && self
                .current_ir()
                .binding_id_from_scope(self.current_ir().current_scope, &name)
                .is_some_and(|(_, binding)| {
                    let binding = &self.current_ir().bindings[binding.0];
                    if binding.declaration_scope != self.current_ir().current_scope {
                        return false;
                    }
                    let BindingStorage::Module(module_binding) = binding.storage else {
                        return true;
                    };
                    self.module
                        .as_ref()
                        .and_then(|module| module.bindings.get(module_binding.0))
                        .and_then(|binding| binding.declaration_scope)
                        .is_some_and(|scope| scope == self.current_ir().current_scope)
                })
        {
            return Err(Error::syntax(
                "invalid redefinition of global identifier in module code",
                source_span(declaration_span),
            ));
        }
        let parsed = self.parse_function_definition_tail(header, false)?;
        if parsed.name != source_name {
            return Err(Error::internal(
                "module function name changed while parsing its definition",
            ));
        }
        let inferred_name = if source_name.is_none() {
            let name = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::from_static("default"),
            )))?;
            Some(name)
        } else {
            None
        };
        let first_declaration = self
            .module
            .as_ref()
            .and_then(|module| module.binding_id(&name))
            .and_then(|binding| self.module.as_ref()?.binding(binding).ok())
            .is_some_and(|binding| binding.declaration.is_none());
        let binding = self.add_module_binding(
            &name,
            ModuleDeclarationOrigin::Function {
                constant: parsed.constant,
                inferred_name,
            },
        )?;
        if let Some(existing) = self
            .current_ir()
            .binding_in_scope(self.current_ir().var_scope, &name)
        {
            if first_declaration {
                let declaration_scope = self.current_ir().current_scope;
                let binding_record = self
                    .current_ir_mut()
                    .bindings
                    .iter_mut()
                    .find(|candidate| {
                        candidate.name == name
                            && candidate.storage == BindingStorage::Module(binding)
                    })
                    .ok_or_else(|| Error::internal("module function binding moved"))?;
                binding_record.declaration_scope = declaration_scope;
                binding_record.declaration_span = Some(declaration_span);
            } else if existing.storage != BindingStorage::Module(binding) {
                return Err(Error::internal(
                    "module function resolved to different storage",
                ));
            }
        } else {
            let function = self.current_ir_mut();
            function.add_binding(
                function.var_scope,
                function.current_scope,
                name.clone(),
                BindingStorage::Module(binding),
                BindingKind::Normal,
                Some(declaration_span),
            );
        }
        if export != ModuleDeclarationExport::None {
            let export_name = match export {
                ModuleDeclarationExport::None => unreachable!(),
                ModuleDeclarationExport::Named => JsString::try_from_utf8(&name)?,
                ModuleDeclarationExport::Default => JsString::from_static("default"),
            };
            self.module_ir_mut()?
                .add_local_export(name.clone(), export_name, declaration_span)?;
        }
        Ok(())
    }

    fn module_ir_mut(&mut self) -> Result<&mut IrModule, Error> {
        self.module
            .as_mut()
            .ok_or_else(|| Error::internal("module parser has no module record"))
    }
}

pub(super) fn resolve_module_exports(tree: &mut FunctionTree) -> Result<(), Error> {
    let Some(module) = tree.module.as_mut() else {
        return Ok(());
    };
    let binding_names = module
        .bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| (binding.name.clone(), ModuleBindingId(index)))
        .collect::<HashMap<_, _>>();
    for export in &mut module.local_exports {
        let binding = binding_names
            .get(&export.local_name)
            .copied()
            .ok_or_else(|| {
                Error::syntax(
                    format!("exported variable '{}' does not exist", export.local_name),
                    source_span(export.span),
                )
            })?;
        export.binding = Some(binding);
    }
    Ok(())
}

pub(super) fn finish_module(
    name: JsString,
    function: UnlinkedFunction,
    has_top_level_await: bool,
    module: IrModule,
) -> Result<UnlinkedModule, Error> {
    let IrModule {
        bindings,
        declaration_order,
        requested_modules,
        imports,
        local_exports,
        mut exports,
        star_exports,
    } = module;
    let mut published_declaration_order = Vec::with_capacity(declaration_order.len());
    let mut link_initializers = Vec::new();
    let mut import_collisions = Vec::new();
    for binding_id in declaration_order {
        let binding = bindings
            .get(binding_id.0)
            .ok_or_else(|| Error::internal("module declaration order is out of bounds"))?;
        let declaration = binding.declaration.ok_or_else(|| {
            Error::internal("module declaration order referenced an import-only binding")
        })?;
        let closure_index = binding
            .closure_index
            .ok_or_else(|| Error::internal("module declaration closure was not seeded"))?;
        published_declaration_order.push(closure_index);
        let value = match declaration {
            ModuleDeclarationOrigin::Var if binding.import.is_none() => {
                Some(ModuleLinkInitializerValue::Undefined)
            }
            ModuleDeclarationOrigin::Function {
                constant,
                inferred_name,
            } => Some(ModuleLinkInitializerValue::Function {
                constant,
                inferred_name,
            }),
            ModuleDeclarationOrigin::Var | ModuleDeclarationOrigin::Lexical { .. } => None,
        };
        if let Some(value) = value {
            link_initializers.push(ModuleLinkInitializer {
                closure_index,
                value,
            });
        }
        if binding.import.is_some() {
            let declaration = match declaration {
                ModuleDeclarationOrigin::Var => ModuleImportCollisionDeclaration::Var,
                ModuleDeclarationOrigin::Lexical { .. } => {
                    ModuleImportCollisionDeclaration::Lexical
                }
                ModuleDeclarationOrigin::Function { .. } => {
                    ModuleImportCollisionDeclaration::Function
                }
            };
            import_collisions.push(ModuleImportCollision {
                closure_index,
                declaration,
            });
        }
    }
    let imports = imports
        .into_iter()
        .map(|import| {
            let closure_index = bindings
                .get(import.binding.0)
                .and_then(|binding| binding.closure_index)
                .ok_or_else(|| Error::internal("module import closure was not seeded"))?;
            Ok(ModuleImport {
                request: import.request,
                import_name: import.import_name,
                closure_index,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    for export in local_exports {
        let binding = export
            .binding
            .ok_or_else(|| Error::internal("local module export was not resolved"))?;
        let closure_index = bindings
            .get(binding.0)
            .and_then(|binding| binding.closure_index)
            .ok_or_else(|| Error::internal("local module export binding closure was not seeded"))?;
        exports.push(ModuleExport {
            export_name: export.export_name,
            target: ModuleExportTarget::Local { closure_index },
        });
    }
    Ok(UnlinkedModule::new(
        name,
        function,
        has_top_level_await,
        UnlinkedModuleTables {
            declaration_order: published_declaration_order,
            link_initializers,
            import_collisions,
            requested_modules,
            imports,
            exports,
            star_exports,
        },
    ))
}
