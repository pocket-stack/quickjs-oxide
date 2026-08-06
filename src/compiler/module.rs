//! Static ECMAScript module parsing and root-binding lowering.
//!
//! The layout follows QuickJS 2026-06-04's `JSModuleDef` boundary: module
//! declarations/imports are closure slots on the synthetic root function,
//! while this parser-owned record retains source-order request and export
//! tables until those slots have been seeded and linked.

use super::*;
use crate::module::{
    ModuleExport, ModuleExportTarget, ModuleImport, ModuleLinkInitializer,
    ModuleLinkInitializerValue, ModuleRequest, ModuleStarExport, UnlinkedModule,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ModuleBindingId(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModuleBindingOrigin {
    Var,
    Lexical,
    Function(u32),
    #[allow(dead_code)] // Populated by the pending named-import parser slice.
    Import,
    #[allow(dead_code)] // Populated by the pending namespace-import parser slice.
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

#[derive(Debug, Default)]
pub(super) struct IrModule {
    pub(super) bindings: Vec<IrModuleBinding>,
    requested_modules: Vec<ModuleRequest>,
    imports: Vec<ModuleImport>,
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
            return Err(Error::syntax(
                format!("duplicate exported name '{}'", export_name.to_utf8_lossy()),
                source_span(span),
            ));
        }
        self.local_exports.push(IrModuleLocalExport {
            local_name,
            export_name,
            span,
            binding: None,
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
                return Err(self.unsupported_here(
                    "static import declarations are not implemented in this module slice",
                ));
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
            TokenKind::Keyword(Keyword::Default) | TokenKind::Punctuator(Punctuator::Multiply) => {
                Err(Error::unsupported(
                    "default and re-export module syntax is not implemented in this module slice",
                    source_span(export_span),
                ))
            }
            _ => Err(self.syntax_here("invalid export syntax")),
        }
    }

    fn parse_module_export_clause(&mut self) -> Result<(), Error> {
        self.expect_punctuator(Punctuator::LeftBrace)?;
        while !self.is_punctuator(Punctuator::RightBrace) {
            let (local_name, local_span) = self.module_identifier_name()?;
            let export_name = if self.is_contextual_keyword("as") {
                self.advance()?;
                self.module_export_name()?
            } else {
                JsString::try_from_utf8(&local_name)?
            };
            self.module_ir_mut()?
                .add_local_export(local_name, export_name, local_span)?;
            if !self.consume_punctuator(Punctuator::Comma)? {
                break;
            }
        }
        self.expect_punctuator(Punctuator::RightBrace)?;
        if self.is_contextual_keyword("from") {
            return Err(
                self.unsupported_here("indirect exports are not implemented in this module slice")
            );
        }
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
