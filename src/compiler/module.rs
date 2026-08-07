//! Static ECMAScript module parsing and root-binding lowering.
//!
//! The layout follows QuickJS 2026-06-04's `JSModuleDef` boundary: module
//! declarations/imports are closure slots on the synthetic root function,
//! while this parser-owned record retains source-order request and export
//! tables until those slots have been seeded and linked.

use super::*;
use crate::module::{
    ModuleExport, ModuleExportTarget, ModuleImport, ModuleImportName, ModuleLinkInitializer,
    ModuleLinkInitializerValue, ModuleRequest, ModuleStarExport, UnlinkedModule,
};

// QuickJS stores expression-form default exports in its private
// `JS_ATOM__default_` module lexical. Source text cannot spell this binding,
// so it cannot shadow or reference the implementation-owned cell.
const MODULE_DEFAULT_BINDING_NAME: &str = "<module-default>";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ModuleBindingId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModuleBindingOrigin {
    Var,
    Lexical,
    Function(u32),
    Import,
    NamespaceImport,
}

#[derive(Clone, Debug)]
pub(super) struct IrModuleBinding {
    pub(super) name: String,
    pub(super) origin: ModuleBindingOrigin,
    pub(super) is_const: bool,
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
    pub(super) fn parse_module_body(&mut self) -> Result<(), Error> {
        while !self.at_eof() {
            if matches!(self.current().kind, TokenKind::Keyword(Keyword::Export)) {
                self.parse_module_export()?;
            } else if matches!(self.current().kind, TokenKind::Keyword(Keyword::Import))
                && self.static_import_declaration_ahead()?
            {
                self.parse_module_import()?;
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

    fn parse_module_export(&mut self) -> Result<(), Error> {
        let export_span = self.current().span;
        self.advance()?;
        match self.current().kind {
            TokenKind::Punctuator(Punctuator::LeftBrace) => self.parse_module_export_clause(),
            TokenKind::Keyword(Keyword::Var) => {
                self.with_exported_module_declaration(Self::parse_var_statement)
            }
            TokenKind::Keyword(Keyword::Let | Keyword::Const) => {
                self.with_exported_module_declaration(Self::parse_lexical_statement)
            }
            TokenKind::Keyword(Keyword::Function) => self.parse_module_function_declaration(true),
            TokenKind::Identifier(_) if self.async_function_ahead() => {
                self.parse_module_function_declaration(true)
            }
            TokenKind::Keyword(Keyword::Class) => {
                self.with_exported_module_declaration(Self::parse_class_declaration)
            }
            TokenKind::Keyword(Keyword::Default) => {
                self.parse_module_default_expression_export(export_span)
            }
            TokenKind::Punctuator(Punctuator::Multiply) => self.parse_module_star_export(),
            _ => Err(self.syntax_here("invalid export syntax")),
        }
    }

    fn parse_module_import(&mut self) -> Result<(), Error> {
        let import_span = self.current().span;
        self.advance()?;

        if matches!(self.current().kind, TokenKind::String(_)) {
            let request = self.parse_module_specifier()?;
            self.reject_module_import_attributes()?;
            self.add_module_request(request)?;
            return self.consume_statement_terminator();
        }

        let mut imports = Vec::new();
        if self.is_punctuator(Punctuator::Multiply) {
            self.advance()?;
            if !self.is_contextual_keyword("as") {
                return Err(self.syntax_here("expecting 'as'"));
            }
            self.advance()?;
            let (local_name, local_span) = self.module_binding_identifier()?;
            let binding = self.register_module_import_binding(&local_name, local_span, true)?;
            imports.push((binding, ModuleImportName::Namespace));
        } else if self.is_punctuator(Punctuator::LeftBrace) {
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
                        ));
                    };
                    validate_identifier(
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
        } else {
            // Default imports (including `default, * as namespace`) stay
            // fail-closed until default declaration exports are implemented.
            return Err(Error::unsupported(
                "default imports are not implemented in this module slice",
                source_span(import_span),
            ));
        }
        let request_index = self.parse_module_from_clause()?;
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
        self.consume_statement_terminator()
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

    fn parse_module_from_clause(&mut self) -> Result<crate::module::ModuleRequestIndex, Error> {
        if !self.is_contextual_keyword("from") {
            return Err(self.syntax_here("from clause expected"));
        }
        self.advance()?;
        let request = self.parse_module_specifier()?;
        self.reject_module_import_attributes()?;
        self.add_module_request(request)
    }

    fn parse_module_specifier(&mut self) -> Result<ModuleRequest, Error> {
        let token = self.current().clone();
        let TokenKind::String(literal) = token.kind else {
            return Err(self.syntax_here("string expected"));
        };
        let specifier = JsString::try_from_utf16(literal.value.utf16)?;
        self.advance()?;
        Ok(ModuleRequest { specifier })
    }

    fn reject_module_import_attributes(&self) -> Result<(), Error> {
        if self.is_contextual_keyword("with")
            || matches!(self.current().kind, TokenKind::Keyword(Keyword::With))
        {
            return Err(self.unsupported_here(
                "module import attributes are not implemented in this module slice",
            ));
        }
        Ok(())
    }

    fn module_binding_identifier(&mut self) -> Result<(String, Span), Error> {
        let token = self.current().clone();
        let TokenKind::Identifier(identifier) = token.kind else {
            return Err(self.syntax_here("identifier expected"));
        };
        validate_identifier(&identifier, token.span, true, IdentifierContext::Variable)?;
        self.advance()?;
        Ok((identifier.value, token.span))
    }

    fn register_module_import_binding(
        &mut self,
        name: &str,
        span: Span,
        is_namespace: bool,
    ) -> Result<ModuleBindingId, Error> {
        self.register_lexical_binding(name, span, span, true, false)?;
        let module = self.module_ir_mut()?;
        let binding = module
            .binding_id(name)
            .ok_or_else(|| Error::internal("registered import binding is missing"))?;
        let binding_record = module.binding_mut(binding)?;
        binding_record.origin = if is_namespace {
            ModuleBindingOrigin::NamespaceImport
        } else {
            ModuleBindingOrigin::Import
        };
        binding_record.is_const = true;
        Ok(binding)
    }

    fn parse_module_export_clause(&mut self) -> Result<(), Error> {
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
            let request = self.parse_module_from_clause()?;
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
        self.consume_statement_terminator()
    }

    fn parse_module_star_export(&mut self) -> Result<(), Error> {
        self.expect_punctuator(Punctuator::Multiply)?;
        if self.is_contextual_keyword("as") {
            self.advance()?;
            // Pinned QuickJS 2026-06-04 accepts IdentifierName here, including
            // keywords, but deliberately does not accept a StringLiteral.
            let (export_name, span) = self.module_identifier_name()?;
            let request = self.parse_module_from_clause()?;
            self.module_ir_mut()?.add_indirect_export(
                JsString::try_from_utf8(&export_name)?,
                ModuleExportTarget::Indirect {
                    request,
                    import_name: ModuleImportName::Namespace,
                },
                span,
            )?;
        } else {
            let request = self.parse_module_from_clause()?;
            self.module_ir_mut()?
                .star_exports
                .push(ModuleStarExport { request });
        }
        self.consume_statement_terminator()
    }

    fn parse_module_default_expression_export(&mut self, export_span: Span) -> Result<(), Error> {
        let default_span = self.current().span;
        self.advance()?;
        if matches!(
            self.current().kind,
            TokenKind::Keyword(Keyword::Class | Keyword::Function)
        ) || matches!(self.current().kind, TokenKind::Identifier(_))
            && self.async_function_ahead()
        {
            return Err(Error::unsupported(
                "default function and class exports are not implemented in this module slice",
                source_span(export_span),
            ));
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

    fn with_exported_module_declaration(
        &mut self,
        parse: fn(&mut Self) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.exporting_module_declaration {
            return Err(Error::internal(
                "nested module export declaration marker is malformed",
            ));
        }
        self.exporting_module_declaration = true;
        let result = parse(self);
        self.exporting_module_declaration = false;
        result
    }

    pub(super) fn add_module_binding(
        &mut self,
        name: &str,
        origin: ModuleBindingOrigin,
        is_const: bool,
    ) -> Result<ModuleBindingId, Error> {
        let module = self.module_ir_mut()?;
        if let Some(id) = module.binding_id(name) {
            if let ModuleBindingOrigin::Function(constant) = origin {
                module.binding_mut(id)?.origin = ModuleBindingOrigin::Function(constant);
            }
            return Ok(id);
        }
        let id = ModuleBindingId(module.bindings.len());
        module.bindings.push(IrModuleBinding {
            name: name.to_owned(),
            origin,
            is_const,
            closure_index: None,
        });
        Ok(id)
    }

    pub(super) fn export_module_declaration(
        &mut self,
        name: &str,
        binding: ModuleBindingId,
        span: Span,
    ) -> Result<(), Error> {
        if !self.exporting_module_declaration {
            return Ok(());
        }
        let module = self.module_ir_mut()?;
        let actual = module.binding(binding)?;
        if actual.name != name {
            return Err(Error::internal("module declaration binding name changed"));
        }
        module.add_local_export(name.to_owned(), JsString::try_from_utf8(name)?, span)
    }

    pub(super) fn parse_module_function_declaration(
        &mut self,
        exported: bool,
    ) -> Result<(), Error> {
        if !matches!(self.current_ir().kind, FunctionKind::Module) {
            return Err(Error::internal(
                "module function declaration escaped the module root",
            ));
        }
        let header = self.parse_function_definition_header(true)?;
        let (name, declaration_span) = header
            .name
            .as_ref()
            .map(|(identifier, span)| (identifier.value.clone(), *span))
            .ok_or_else(|| Error::internal("required module function lost its name"))?;
        // QuickJS checks only the first same-named module-global record and
        // rejects it when that record was declared at this exact scope level.
        // This is deliberately source- and scope-ordered: a later `var` may
        // reuse an earlier function cell, while a `var` first declared in a
        // nested block does not reject a later Program-level function
        // (`js_parse_function_decl2`, JS_EVAL_TYPE_MODULE, including its
        // pinned `XXX: should check scope chain` behavior).
        if self
            .current_ir()
            .binding_id_from_scope(self.current_ir().current_scope, &name)
            .is_some_and(|(_, binding)| {
                self.current_ir().bindings[binding.0].declaration_scope
                    == self.current_ir().current_scope
            })
        {
            return Err(Error::syntax(
                "invalid redefinition of global identifier in module code",
                source_span(declaration_span),
            ));
        }
        let parsed = self.parse_function_definition_tail(header, false)?;
        if parsed
            .name
            .as_ref()
            .is_none_or(|(parsed_name, parsed_span)| {
                parsed_name != &name || *parsed_span != declaration_span
            })
        {
            return Err(Error::internal(
                "module function name changed while parsing its definition",
            ));
        }
        let binding =
            self.add_module_binding(&name, ModuleBindingOrigin::Function(parsed.constant), false)?;
        if self
            .current_ir()
            .binding_in_scope(self.current_ir().var_scope, &name)
            .is_none()
        {
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
        if exported {
            self.module_ir_mut()?.add_local_export(
                name.clone(),
                JsString::try_from_utf8(&name)?,
                declaration_span,
            )?;
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
    module: IrModule,
) -> Result<UnlinkedModule, Error> {
    let IrModule {
        bindings,
        requested_modules,
        imports,
        local_exports,
        mut exports,
        star_exports,
    } = module;
    let link_initializers = bindings
        .iter()
        .filter_map(|binding| {
            let value = match binding.origin {
                ModuleBindingOrigin::Var => ModuleLinkInitializerValue::Undefined,
                ModuleBindingOrigin::Function(constant) => {
                    ModuleLinkInitializerValue::Function(constant)
                }
                ModuleBindingOrigin::Lexical
                | ModuleBindingOrigin::Import
                | ModuleBindingOrigin::NamespaceImport => return None,
            };
            Some(
                binding
                    .closure_index
                    .map(|closure_index| ModuleLinkInitializer {
                        closure_index,
                        value,
                    })
                    .ok_or_else(|| {
                        Error::internal("module link initializer closure was not seeded")
                    }),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
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
        link_initializers,
        requested_modules,
        imports,
        exports,
        star_exports,
    ))
}
