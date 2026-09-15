//! Ordered declaration registration and hoisting grammar.

use crate::engine::api::error::Error;
use crate::engine::api::error::ErrorKind;
use crate::engine::code::bytecode::EvalVariableSource;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::ClosureVariableKind;
use crate::engine::code::function::metadata::EvalCallerVariableTarget;
use crate::engine::code::function::metadata::EvalKind;
use crate::engine::code::function::metadata::FunctionKind as BytecodeFunctionKind;
use crate::engine::compiler::MAX_LOCAL_VARIABLES;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::model::bindings::BindingId;
use crate::engine::compiler::model::bindings::BindingKind;
use crate::engine::compiler::model::bindings::BindingStorage;
use crate::engine::compiler::model::bindings::EvalDeclarationMode;
use crate::engine::compiler::model::bindings::EvalDeclarationTarget;
use crate::engine::compiler::model::bindings::EvalDeclarationValue;
use crate::engine::compiler::model::bindings::IrAnnexBinding;
use crate::engine::compiler::model::bindings::IrEvalDeclaration;
use crate::engine::compiler::model::bindings::IrGlobalDeclaration;
use crate::engine::compiler::model::bindings::IrHoistedFunction;
use crate::engine::compiler::model::bindings::IrProgramAnnexFunction;
use crate::engine::compiler::model::bindings::IrScopedFunction;
use crate::engine::compiler::model::bindings::binding_kind_from_closure_flags;
use crate::engine::compiler::model::ir::IdentifierAccess;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::model::scope::ScopeKind;
use crate::engine::compiler::module;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::context::PreparedScopedFunction;
use crate::engine::compiler::parser::diagnostics::source_span;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn register_var_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
    ) -> Result<(), Error> {
        if matches!(self.current_ir().kind, FunctionKind::Eval(_)) {
            return self.register_eval_var_binding(name, declaration_span, conflict_span);
        }
        if matches!(self.current_ir().kind, FunctionKind::Module) {
            if let Some((_, binding)) = self
                .current_ir()
                .binding_id_from_scope(self.current_ir().context.current_scope, name)
            {
                let BindingStorage::Module(module_binding) =
                    self.current_ir().bindings[binding.0].storage
                else {
                    return Err(Error::internal(
                        "module declaration resolved to non-module storage",
                    ));
                };
                if self
                    .module
                    .as_ref()
                    .and_then(|module| module.binding(module_binding).ok())
                    .and_then(|binding| binding.declaration)
                    .is_some_and(|declaration| {
                        matches!(declaration, module::ModuleDeclarationOrigin::Lexical { .. })
                    })
                {
                    return Err(Error::syntax(
                        "invalid redefinition of lexical identifier",
                        source_span(conflict_span),
                    ));
                }
            }
            if let Some(binding) = self
                .current_ir()
                .binding_id_in_scope(self.current_ir().var_scope, name)
            {
                let BindingStorage::Module(module_binding) =
                    self.current_ir().bindings[binding.0].storage
                else {
                    return Err(Error::internal("module var resolved to non-module storage"));
                };
                let first_declaration = self
                    .module
                    .as_ref()
                    .and_then(|module| module.binding(module_binding).ok())
                    .is_some_and(|binding| binding.declaration.is_none());
                if first_declaration {
                    let declaration_scope = self.current_ir().context.current_scope;
                    let binding = self
                        .current_ir_mut()
                        .bindings
                        .get_mut(binding.0)
                        .ok_or_else(|| Error::internal("module var binding moved"))?;
                    binding.declaration_scope = declaration_scope;
                    binding.declaration_span = Some(declaration_span);
                }
                self.add_module_binding(name, module::ModuleDeclarationOrigin::Var)?;
                self.export_module_declaration(name, module_binding, declaration_span)?;
                return Ok(());
            }
            let module_binding =
                self.add_module_binding(name, module::ModuleDeclarationOrigin::Var)?;
            let function = self.current_ir_mut();
            function.ir.add_binding(
                function.ir.var_scope,
                function.context.current_scope,
                name.to_owned(),
                BindingStorage::Module(module_binding),
                BindingKind::Normal,
                Some(declaration_span),
            );
            self.export_module_declaration(name, module_binding, declaration_span)?;
            return Ok(());
        }
        let function = &mut self.functions[self.current_function];
        let selects_arguments_object =
            matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
                && name == "arguments"
                && !function
                    .parameters
                    .iter()
                    .any(|parameter| parameter.as_deref() == Some("arguments"))
                && !function
                    .parameter_pattern_bindings
                    .iter()
                    .any(|binding| binding.name == "arguments");
        if let Some((binding_scope, binding)) =
            function.binding_id_from_scope(function.context.current_scope, name)
            && matches!(
                function.bindings[binding.0].kind,
                BindingKind::Lexical { .. }
            )
        {
            let catch_parameter = function.bindings[binding.0].is_catch_parameter;
            let masked_program_lexical = matches!(function.kind, FunctionKind::Script)
                && binding_scope == function.body_scope
                && function.first_global_declaration_is_normal(name);
            if !catch_parameter && !masked_program_lexical {
                return Err(Error::syntax(
                    "invalid redefinition of lexical identifier",
                    source_span(conflict_span),
                ));
            }
        }
        if matches!(function.kind, FunctionKind::Script) {
            function.global_declarations.push(IrGlobalDeclaration {
                name: name.to_owned(),
                is_lexical: false,
                is_const: false,
                function_constant: None,
                closure_index: None,
            });
        }
        if let Some(binding) = function.binding_in_scope(function.ir.var_scope, name) {
            if selects_arguments_object {
                let BindingStorage::Local(index) = binding.storage else {
                    return Err(Error::internal(
                        "implicit arguments declaration did not select a root local",
                    ));
                };
                if function
                    .arguments_local
                    .replace(index)
                    .is_some_and(|old| old != index)
                {
                    return Err(Error::internal(
                        "ordinary function selected more than one arguments local",
                    ));
                }
            }
            return Ok(());
        }
        if matches!(function.kind, FunctionKind::Script) {
            function.ir.add_binding(
                function.ir.var_scope,
                function.context.current_scope,
                name.to_owned(),
                BindingStorage::Global,
                BindingKind::Normal,
                Some(declaration_span),
            );
            return Ok(());
        }
        if function.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(
                Error::new(ErrorKind::JsInternal, "too many local variables")
                    .with_span(source_span(declaration_span)),
            );
        }
        let index = u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        function.locals.push(name.to_owned());
        function.ir.add_binding(
            function.ir.var_scope,
            function.context.current_scope,
            name.to_owned(),
            BindingStorage::Local(index),
            BindingKind::Normal,
            Some(declaration_span),
        );
        if selects_arguments_object
            && function
                .arguments_local
                .replace(index)
                .is_some_and(|old| old != index)
        {
            return Err(Error::internal(
                "ordinary function selected more than one arguments local",
            ));
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn current_eval_declaration_mode(
        &self,
    ) -> Result<EvalDeclarationMode, Error> {
        let function = self.current_ir();
        let FunctionKind::Eval(kind) = function.kind else {
            return Err(Error::internal(
                "eval declaration mode requested outside an eval root",
            ));
        };
        if function.strict {
            return Ok(EvalDeclarationMode::Local);
        }
        match kind {
            EvalKind::Indirect => Ok(EvalDeclarationMode::Global),
            EvalKind::Direct => match function.eval_caller_profile.variable_target {
                EvalCallerVariableTarget::Global => Ok(EvalDeclarationMode::Global),
                EvalCallerVariableTarget::ExternalBinding(index) => function
                    .external_bindings
                    .get(usize::from(index))
                    .filter(|binding| {
                        matches!(
                            binding.kind,
                            ClosureVariableKind::EvalVariableObject
                                | ClosureVariableKind::ArgEvalVariableObject
                        ) && !binding.is_lexical
                            && !binding.is_const
                            && !binding.is_catch_parameter
                    })
                    .map(|_| EvalDeclarationMode::Dynamic(EvalVariableSource::Closure(index)))
                    .ok_or_else(|| {
                        Error::internal("eval caller variable target is not authenticated")
                    }),
                EvalCallerVariableTarget::StrictLocal => Err(Error::internal(
                    "sloppy eval root retained a strict-local variable target",
                )),
            },
            EvalKind::None => Err(Error::internal("eval root has no eval kind")),
        }
    }

    pub(in crate::engine::compiler) fn eval_dynamic_declaration_target(
        &mut self,
        name: &str,
        object: EvalVariableSource,
        _conflict_span: Span,
    ) -> Result<EvalDeclarationTarget, Error> {
        let EvalVariableSource::Closure(object_index) = object else {
            return Err(Error::internal(
                "eval root declaration targeted a non-external variable object",
            ));
        };
        let external_bindings = self.current_ir().external_bindings.clone();
        for (index, binding) in external_bindings.iter().enumerate() {
            let index = u16::try_from(index)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
            if index == object_index {
                if !matches!(
                    binding.kind,
                    ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::ArgEvalVariableObject
                ) {
                    return Err(Error::internal(
                        "eval variable object external index changed",
                    ));
                }
                return Ok(EvalDeclarationTarget::Dynamic(object));
            }
            if binding.name.to_utf8_lossy() != name {
                continue;
            }
            if binding.is_lexical
                && !binding.is_catch_parameter
                && self.current_ir().eval_redeclaration.is_none()
            {
                self.current_ir_mut().eval_redeclaration = Some(name.to_owned());
            }
            let kind =
                binding_kind_from_closure_flags(binding.kind, binding.is_lexical, binding.is_const)
                    .ok_or_else(|| Error::internal("eval caller binding flags are inconsistent"))?;
            return Ok(EvalDeclarationTarget::External { index, kind });
        }
        Err(Error::internal(
            "sloppy direct eval has no variable object external",
        ))
    }

    pub(in crate::engine::compiler) fn register_eval_var_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
    ) -> Result<(), Error> {
        let mode = self.current_eval_declaration_mode()?;
        let function = self.current_ir();
        if let Some((_, binding)) =
            function.binding_id_from_scope(function.context.current_scope, name)
        {
            let binding = &function.bindings[binding.0];
            if matches!(binding.kind, BindingKind::Lexical { .. })
                && !matches!(binding.storage, BindingStorage::External(_))
                && !binding.is_catch_parameter
            {
                return Err(Error::syntax(
                    "invalid redefinition of lexical identifier",
                    source_span(conflict_span),
                ));
            }
        }

        match mode {
            EvalDeclarationMode::Dynamic(object) => {
                let target = self.eval_dynamic_declaration_target(name, object, conflict_span)?;
                self.current_ir_mut()
                    .eval_declarations
                    .push(IrEvalDeclaration {
                        name: name.to_owned(),
                        target,
                        value: EvalDeclarationValue::Undefined,
                    });
                Ok(())
            }
            EvalDeclarationMode::Local => {
                let function = self.current_ir_mut();
                let existing = function.scopes[function.ir.var_scope.0]
                    .bindings
                    .iter()
                    .rev()
                    .copied()
                    .find(|binding| {
                        let binding = &function.bindings[binding.0];
                        binding.name == name
                            && !matches!(binding.storage, BindingStorage::External(_))
                    });
                if existing.is_some() {
                    return Ok(());
                }
                if function.locals.len() >= MAX_LOCAL_VARIABLES {
                    return Err(
                        Error::new(ErrorKind::JsInternal, "too many local variables")
                            .with_span(source_span(declaration_span)),
                    );
                }
                let index = u16::try_from(function.locals.len())
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
                function.locals.push(name.to_owned());
                function.ir.add_binding(
                    function.ir.var_scope,
                    function.context.current_scope,
                    name.to_owned(),
                    BindingStorage::Local(index),
                    BindingKind::Normal,
                    Some(declaration_span),
                );
                Ok(())
            }
            EvalDeclarationMode::Global => {
                let function = self.current_ir_mut();
                function.global_declarations.push(IrGlobalDeclaration {
                    name: name.to_owned(),
                    is_lexical: false,
                    is_const: false,
                    function_constant: None,
                    closure_index: None,
                });
                let caller_lexical_conflict = function
                    .external_bindings
                    .iter()
                    .find(|binding| binding.name.to_utf8_lossy() == name)
                    .is_some_and(|binding| binding.is_lexical && !binding.is_catch_parameter);
                if caller_lexical_conflict && function.eval_redeclaration.is_none() {
                    function.eval_redeclaration = Some(name.to_owned());
                }
                let existing = function.scopes[function.ir.var_scope.0]
                    .bindings
                    .iter()
                    .rev()
                    .copied()
                    .find(|binding| {
                        let binding = &function.bindings[binding.0];
                        binding.name == name && binding.storage == BindingStorage::Global
                    });
                if existing.is_none() {
                    function.ir.add_binding(
                        function.ir.var_scope,
                        function.context.current_scope,
                        name.to_owned(),
                        BindingStorage::Global,
                        BindingKind::Normal,
                        Some(declaration_span),
                    );
                }
                Ok(())
            }
        }
    }

    pub(in crate::engine::compiler) fn register_lexical_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
        is_const: bool,
        allow_body_parameter_shadow: bool,
    ) -> Result<(), Error> {
        let scope = self.current_ir().context.current_scope;
        let scope_kind = self.current_ir().scopes[scope.0].kind;
        let is_global = matches!(scope_kind, ScopeKind::ProgramBody)
            && matches!(self.current_ir().kind, FunctionKind::Script)
            && scope == self.current_ir().body_scope;
        let is_eval_body = matches!(scope_kind, ScopeKind::ProgramBody)
            && matches!(self.current_ir().kind, FunctionKind::Eval(_))
            && scope == self.current_ir().body_scope;
        let is_module_body = matches!(scope_kind, ScopeKind::ProgramBody)
            && matches!(self.current_ir().kind, FunctionKind::Module)
            && scope == self.current_ir().body_scope;
        if is_module_body {
            if let Some(module_binding) = self
                .module
                .as_ref()
                .and_then(|module| module.binding_id(name))
            {
                let record = self
                    .module
                    .as_ref()
                    .and_then(|module| module.binding(module_binding).ok())
                    .ok_or_else(|| Error::internal("module lexical binding record is missing"))?;
                if record.declaration.is_some() || record.import.is_none() {
                    return Err(Error::syntax(
                        "invalid redefinition of lexical identifier",
                        source_span(conflict_span),
                    ));
                }
                let existing = self
                    .current_ir()
                    .binding_id_in_scope(scope, name)
                    .ok_or_else(|| {
                        Error::internal("module import has no body-scope binding record")
                    })?;
                if self.current_ir().bindings[existing.0].storage
                    != BindingStorage::Module(module_binding)
                {
                    return Err(Error::internal(
                        "module lexical collision resolved to different storage",
                    ));
                }
                let binding = self
                    .current_ir_mut()
                    .bindings
                    .get_mut(existing.0)
                    .ok_or_else(|| Error::internal("module lexical binding moved"))?;
                binding.declaration_scope = scope;
                binding.declaration_span = Some(declaration_span);
                self.add_module_binding(
                    name,
                    module::ModuleDeclarationOrigin::Lexical { is_const },
                )?;
                self.export_module_declaration(name, module_binding, declaration_span)?;
                return Ok(());
            }
            let module_binding = self
                .add_module_binding(name, module::ModuleDeclarationOrigin::Lexical { is_const })?;
            let function = self.current_ir_mut();
            function.ir.add_binding(
                scope,
                scope,
                name.to_owned(),
                BindingStorage::Module(module_binding),
                BindingKind::Lexical { is_const },
                Some(declaration_span),
            );
            self.export_module_declaration(name, module_binding, declaration_span)?;
            return Ok(());
        }

        let function = &mut self.functions[self.current_function];
        if is_eval_body
            && (function
                .eval_declarations
                .iter()
                .any(|declaration| declaration.name == name)
                || function
                    .global_declarations
                    .iter()
                    .any(|declaration| !declaration.is_lexical && declaration.name == name))
        {
            return Err(Error::syntax(
                "invalid redefinition of lexical identifier",
                source_span(conflict_span),
            ));
        }
        let supported_scope = is_global
            || is_eval_body
            || is_module_body
            || matches!(
                scope_kind,
                ScopeKind::Block
                    | ScopeKind::ClassPrivate
                    | ScopeKind::If
                    | ScopeKind::For
                    | ScopeKind::Switch
                    | ScopeKind::Catch
            )
            || (matches!(scope_kind, ScopeKind::FunctionBody)
                && matches!(
                    function.kind,
                    FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
                )
                && scope == function.body_scope);
        if !supported_scope {
            return Err(Error::internal(
                "lexical declaration escaped its supported parser scope",
            ));
        }
        let direct_catch_parameter_conflict = function.scopes[scope.0]
            .parent
            .filter(|parent| function.scopes[parent.0].kind == ScopeKind::Catch)
            .and_then(|parent| function.binding_id_in_scope(parent, name))
            .is_some_and(|binding| function.bindings[binding.0].is_catch_parameter);
        if direct_catch_parameter_conflict {
            return Err(Error::syntax(
                "invalid redefinition of lexical identifier",
                source_span(conflict_span),
            ));
        }
        if let Some(existing) = function.binding_id_in_scope(scope, name) {
            let masked_program_duplicate = is_global
                && function.first_global_declaration_is_normal(name)
                && matches!(
                    function.bindings[existing.0].kind,
                    BindingKind::Lexical { .. }
                );
            if masked_program_duplicate {
                function.global_declarations.push(IrGlobalDeclaration {
                    name: name.to_owned(),
                    is_lexical: true,
                    is_const,
                    function_constant: None,
                    closure_index: None,
                });
                return Ok(());
            }
            return Err(Error::syntax(
                "invalid redefinition of lexical identifier",
                source_span(conflict_span),
            ));
        }
        if let Some(binding) = function.binding_in_scope(function.ir.var_scope, name) {
            let message = match (binding.storage, binding.kind) {
                (BindingStorage::Argument(_), _)
                    if scope == function.body_scope && allow_body_parameter_shadow =>
                {
                    ""
                }
                (BindingStorage::Argument(_), _) if scope == function.body_scope => {
                    "invalid redefinition of parameter name"
                }
                (BindingStorage::Argument(_), _) => "",
                (BindingStorage::Local(_), BindingKind::Normal)
                    if function.scope_is_within(binding.declaration_scope, scope) =>
                {
                    "invalid redefinition of a variable"
                }
                (BindingStorage::Local(_), BindingKind::Normal) => "",
                (BindingStorage::Local(_), BindingKind::FunctionName { .. }) => {
                    // The private named-expression binding lives outside the
                    // authored environments and may be shadowed there.
                    ""
                }
                (
                    BindingStorage::Local(_),
                    BindingKind::EvalVariableObject | BindingKind::ArgEvalVariableObject,
                ) => "",
                (BindingStorage::Local(_), BindingKind::WithObject) => {
                    return Err(Error::internal(
                        "with object binding leaked into the function var scope",
                    ));
                }
                (BindingStorage::Local(_), BindingKind::Lexical { .. }) => {
                    return Err(Error::internal(
                        "lexical binding leaked into the function var scope",
                    ));
                }
                (
                    BindingStorage::Local(_),
                    BindingKind::PrivateField { .. }
                    | BindingKind::PrivateMethod { .. }
                    | BindingKind::PrivateGetter { .. }
                    | BindingKind::PrivateSetter { .. }
                    | BindingKind::PrivateGetterSetter { .. },
                ) => {
                    return Err(Error::internal(
                        "private binding leaked into the function var scope",
                    ));
                }
                (BindingStorage::External(_), _) => "",
                (BindingStorage::Module(_), BindingKind::Normal)
                    if function.scope_is_within(binding.declaration_scope, scope) =>
                {
                    "invalid redefinition of module identifier"
                }
                (BindingStorage::Module(_), BindingKind::Normal) => "",
                (BindingStorage::Module(_), _) => {
                    return Err(Error::internal(
                        "non-var module binding leaked into the function var scope",
                    ));
                }
                (BindingStorage::Global, BindingKind::Normal)
                    if function.scope_is_within(binding.declaration_scope, scope) =>
                {
                    "invalid redefinition of global identifier"
                }
                (BindingStorage::Global, BindingKind::Normal) => "",
                (BindingStorage::Global, _) => {
                    return Err(Error::internal(
                        "non-var global binding leaked into the function var scope",
                    ));
                }
            };
            if !message.is_empty() {
                return Err(Error::syntax(message, source_span(conflict_span)));
            }
        }
        if is_global {
            function.global_declarations.push(IrGlobalDeclaration {
                name: name.to_owned(),
                is_lexical: true,
                is_const,
                function_constant: None,
                closure_index: None,
            });
            function.ir.add_binding(
                scope,
                scope,
                name.to_owned(),
                BindingStorage::Global,
                BindingKind::Lexical { is_const },
                Some(declaration_span),
            );
            return Ok(());
        }
        if function.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(
                Error::new(ErrorKind::JsInternal, "too many local variables")
                    .with_span(source_span(declaration_span)),
            );
        }
        let index = u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        function.locals.push(name.to_owned());
        function.ir.add_binding(
            scope,
            scope,
            name.to_owned(),
            BindingStorage::Local(index),
            BindingKind::Lexical { is_const },
            Some(declaration_span),
        );
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_program_function_declaration(
        &mut self,
    ) -> Result<(), Error> {
        let parsed = self.parse_function_definition(true, false)?;
        let (name, declaration_span) = parsed
            .name
            .ok_or_else(|| Error::internal("required Program function lost its name"))?;
        let function = &mut self.functions[self.current_function];
        if !matches!(function.kind, FunctionKind::Script) {
            return Err(Error::internal(
                "Program function declaration escaped the root script",
            ));
        }

        // QuickJS appends one GLOBAL_FUNCTION_DECL record per syntax node,
        // including duplicates. It deliberately does not run the ordinary
        // `define_var` conflict check here, which permits a preceding Program
        // lexical with the same name.
        function.global_declarations.push(IrGlobalDeclaration {
            name: name.clone(),
            is_lexical: false,
            is_const: false,
            function_constant: Some(parsed.constant),
            closure_index: None,
        });
        if function
            .binding_in_scope(function.ir.var_scope, &name)
            .is_none()
        {
            function.ir.add_binding(
                function.ir.var_scope,
                function.context.current_scope,
                name,
                BindingStorage::Global,
                BindingKind::Normal,
                Some(declaration_span),
            );
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_eval_program_function_declaration(
        &mut self,
    ) -> Result<(), Error> {
        let parsed = self.parse_function_definition(true, false)?;
        let (name, declaration_span) = parsed
            .name
            .ok_or_else(|| Error::internal("required eval function lost its name"))?;
        if !matches!(self.current_ir().kind, FunctionKind::Eval(_)) {
            return Err(Error::internal(
                "eval function declaration escaped its synthetic root",
            ));
        }
        let conflict_span = self.current().span;
        if self
            .current_ir()
            .binding_id_in_scope(self.current_ir().body_scope, &name)
            .is_some_and(|binding| {
                matches!(
                    self.current_ir().bindings[binding.0].kind,
                    BindingKind::Lexical { .. }
                )
            })
        {
            return Err(Error::syntax(
                "invalid redefinition of lexical identifier",
                source_span(conflict_span),
            ));
        }

        match self.current_eval_declaration_mode()? {
            EvalDeclarationMode::Global => {
                let function = self.current_ir_mut();
                function.global_declarations.push(IrGlobalDeclaration {
                    name: name.clone(),
                    is_lexical: false,
                    is_const: false,
                    function_constant: Some(parsed.constant),
                    closure_index: None,
                });
                let caller_lexical_conflict = function
                    .external_bindings
                    .iter()
                    .find(|binding| binding.name.to_utf8_lossy() == name)
                    .is_some_and(|binding| binding.is_lexical && !binding.is_catch_parameter);
                if caller_lexical_conflict && function.eval_redeclaration.is_none() {
                    function.eval_redeclaration = Some(name.clone());
                }
                let has_global = function.scopes[function.ir.var_scope.0]
                    .bindings
                    .iter()
                    .copied()
                    .any(|binding| {
                        let binding = &function.bindings[binding.0];
                        binding.name == name && binding.storage == BindingStorage::Global
                    });
                if !has_global {
                    function.ir.add_binding(
                        function.ir.var_scope,
                        function.context.current_scope,
                        name,
                        BindingStorage::Global,
                        BindingKind::Normal,
                        Some(declaration_span),
                    );
                }
            }
            EvalDeclarationMode::Local => {
                self.register_eval_var_binding(&name, declaration_span, conflict_span)?;
                let function = self.current_ir_mut();
                let binding = function.scopes[function.ir.var_scope.0]
                    .bindings
                    .iter()
                    .rev()
                    .copied()
                    .find(|binding| {
                        let binding = &function.bindings[binding.0];
                        binding.name == name
                            && !matches!(binding.storage, BindingStorage::External(_))
                    })
                    .ok_or_else(|| {
                        Error::internal("eval-local function binding was not registered")
                    })?;
                if let Some(existing) = function
                    .hoisted_functions
                    .iter_mut()
                    .find(|hoist| hoist.binding == binding)
                {
                    existing.constant = parsed.constant;
                } else {
                    function.hoisted_functions.push(IrHoistedFunction {
                        binding,
                        constant: parsed.constant,
                    });
                }
            }
            EvalDeclarationMode::Dynamic(object) => {
                let target = self.eval_dynamic_declaration_target(&name, object, conflict_span)?;
                self.current_ir_mut()
                    .eval_declarations
                    .push(IrEvalDeclaration {
                        name,
                        target,
                        value: EvalDeclarationValue::Function(parsed.constant),
                    });
            }
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_function_body_declaration(
        &mut self,
    ) -> Result<(), Error> {
        let parsed = self.parse_function_definition(true, false)?;
        let (name, declaration_span) = parsed
            .name
            .ok_or_else(|| Error::internal("required function declaration lost its name"))?;
        if !matches!(
            self.current_ir().kind,
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
        ) {
            return Err(Error::internal(
                "function-body declaration escaped its ordinary function",
            ));
        }
        let conflict_span = self.current().span;
        self.register_var_binding(&name, declaration_span, conflict_span)?;

        let function = &mut self.functions[self.current_function];
        let binding = function
            .binding_id_in_scope(function.ir.var_scope, &name)
            .ok_or_else(|| Error::internal("function declaration binding was not registered"))?;
        let metadata = &function.bindings[binding.0];
        if metadata.kind != BindingKind::Normal
            || !matches!(
                metadata.storage,
                BindingStorage::Argument(_) | BindingStorage::Local(_)
            )
        {
            return Err(Error::internal(
                "function declaration did not resolve to an ordinary frame binding",
            ));
        }
        if let Some(existing) = function
            .hoisted_functions
            .iter_mut()
            .find(|hoist| hoist.binding == binding)
        {
            existing.constant = parsed.constant;
        } else {
            function.hoisted_functions.push(IrHoistedFunction {
                binding,
                constant: parsed.constant,
            });
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_annex_b_function_declaration(
        &mut self,
    ) -> Result<(), Error> {
        let function = self.current_ir();
        let program_body = matches!(function.kind, FunctionKind::Script)
            && function.context.current_scope == function.body_scope
            && matches!(
                function.scopes[function.context.current_scope.0].kind,
                ScopeKind::ProgramBody
            );
        if program_body {
            self.parse_program_annex_b_function_declaration()
        } else {
            self.parse_scoped_function_declaration()
        }
    }

    pub(in crate::engine::compiler) fn parse_program_annex_b_function_declaration(
        &mut self,
    ) -> Result<(), Error> {
        let header = self.parse_function_definition_header(true)?;
        let (name, declaration_span) = header
            .name
            .as_ref()
            .map(|(identifier, span)| (identifier.value.clone(), *span))
            .ok_or_else(|| Error::internal("required Program Annex B function lost its name"))?;
        let conflict_span = self.current().span;

        let (body_scope, var_scope) = {
            let function = self.current_ir();
            if !matches!(function.kind, FunctionKind::Script)
                || function.context.current_scope != function.body_scope
            {
                return Err(Error::internal(
                    "Program Annex B function escaped the Program body",
                ));
            }
            (function.body_scope, function.ir.var_scope)
        };
        let conflicts_with_authored_global = {
            let function = self.current_ir();
            if let Some(binding) = function.binding_id_in_scope(var_scope, &name) {
                // The root binding retains the first ordinary declaration's
                // scope. A prior nested/Annex declaration therefore masks a
                // later Program lexical in QuickJS's first-global-record
                // lookup, while an authored Program var/function still
                // conflicts here.
                function.bindings[binding.0].declaration_scope == body_scope
            } else {
                function.binding_id_in_scope(body_scope, &name).is_some()
            }
        };
        if conflicts_with_authored_global {
            return Err(Error::syntax(
                "invalid redefinition of global identifier",
                source_span(conflict_span),
            ));
        }

        let parsed = self.parse_function_definition_tail(header, false)?;
        if parsed.name.as_ref().map(|(parsed, _)| parsed.as_str()) != Some(name.as_str()) {
            return Err(Error::internal(
                "Program Annex B function header changed while parsing its child",
            ));
        }
        // QuickJS publishes this synthetic global only after the child has
        // parsed successfully. Deferred tree-wide identifier resolution still
        // lets the child capture the resulting recursive binding.
        let IrAnnexBinding::Static(binding) =
            self.ensure_annex_b_binding(&name, declaration_span)?
        else {
            return Err(Error::internal(
                "Program Annex B declaration targeted dynamic eval storage",
            ));
        };

        let authored_closure = self.emit(IrOp::MakeClosure(parsed.constant))?;
        self.emit_instruction(Instruction::Dup)?;
        self.emit_identifier_inherited(
            name.clone(),
            declaration_span,
            var_scope,
            IdentifierAccess::AnnexBPut,
        )?;
        // `JS_PARSE_FUNC_VAR` performs a second source-position write when the
        // Program-body lexical exception is active. It is observable through
        // pre-existing global accessors, whose setter runs twice.
        self.emit_identifier_inherited(name, declaration_span, body_scope, IdentifierAccess::Put)?;
        self.current_ir_mut()
            .program_annex_functions
            .push(IrProgramAnnexFunction {
                binding,
                constant: parsed.constant,
                authored_closure,
            });
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_scoped_function_declaration(
        &mut self,
    ) -> Result<(), Error> {
        let header = self.parse_function_definition_header(true)?;
        let (name, declaration_span) = header
            .name
            .as_ref()
            .map(|(identifier, span)| (identifier.value.clone(), *span))
            .ok_or_else(|| Error::internal("required scoped function lost its name"))?;
        let non_ordinary = header.execution_kind != BytecodeFunctionKind::Normal;
        let prepared = self.prepare_scoped_function(&name, declaration_span, non_ordinary)?;
        let parsed = self.parse_function_definition_tail(header, false)?;
        if parsed.name.as_ref().map(|(parsed, _)| parsed.as_str()) != Some(name.as_str()) {
            return Err(Error::internal(
                "scoped function header changed while parsing its child",
            ));
        }

        let annex_binding = if prepared.create_annex_binding {
            Some(self.ensure_annex_b_binding(&name, declaration_span)?)
        } else {
            None
        };
        let authored_closure = self.emit(IrOp::MakeClosure(parsed.constant))?;
        if annex_binding.is_some() {
            self.emit_instruction(Instruction::Dup)?;
            let root_scope = self.current_ir().var_scope;
            let access = if annex_binding == Some(IrAnnexBinding::Dynamic) {
                IdentifierAccess::Put
            } else {
                IdentifierAccess::AnnexBPut
            };
            self.emit_identifier_inherited(name, declaration_span, root_scope, access)?;
        }
        self.emit_instruction(Instruction::Drop)?;
        self.current_ir_mut()
            .scoped_functions
            .push(IrScopedFunction {
                binding: prepared.binding,
                constant: parsed.constant,
                annex_binding,
                authored_closure,
            });
        Ok(())
    }

    pub(in crate::engine::compiler) fn prepare_scoped_function(
        &mut self,
        name: &str,
        declaration_span: Span,
        lexical_only: bool,
    ) -> Result<PreparedScopedFunction, Error> {
        let function = self.current_ir();
        let scope_kind = function.scopes[function.context.current_scope.0].kind;
        let eval_program_body = matches!(function.kind, FunctionKind::Eval(_))
            && function.context.current_scope == function.body_scope
            && matches!(scope_kind, ScopeKind::ProgramBody);
        if !matches!(
            scope_kind,
            ScopeKind::Block | ScopeKind::If | ScopeKind::Switch | ScopeKind::FunctionBody
        ) && !eval_program_body
        {
            return Err(Error::internal(
                "scoped function escaped an Annex B declaration scope",
            ));
        }
        // Annex B.3.2 applies only to synchronous ordinary
        // FunctionDeclarations. Generator and async declarations remain
        // lexical even in sloppy blocks.
        let create_annex_binding = !lexical_only && self.scoped_function_is_annex_b_eligible(name);
        let conflict_span = self.current().span;
        let binding = self.register_scoped_function_binding(
            name,
            declaration_span,
            conflict_span,
            lexical_only,
        )?;
        Ok(PreparedScopedFunction {
            binding,
            create_annex_binding,
        })
    }

    pub(in crate::engine::compiler) fn scoped_function_is_annex_b_eligible(
        &self,
        name: &str,
    ) -> bool {
        let function = self.current_ir();
        if function.strict {
            return false;
        }
        if (matches!(function.kind, FunctionKind::Ordinary | FunctionKind::Method)
            && name == "arguments")
            || (matches!(
                function.kind,
                FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
            ) && function
                .parameter_names
                .iter()
                .any(|parameter| parameter == name))
        {
            return false;
        }

        let mut scope = function.context.current_scope;
        loop {
            if let Some(binding) = function.binding_in_scope(scope, name)
                && matches!(binding.kind, BindingKind::Lexical { .. })
            {
                // Annex B.3.5 deliberately treats a simple catch parameter as
                // compatible with the synthetic outer `var` introduced for a
                // block FunctionDeclaration. The catch-local lexical remains
                // the function's inner binding; only the eligibility scan
                // skips it while looking for a blocking lexical declaration.
                if binding.is_catch_parameter {
                    let Some(parent) = function.scopes[scope.0].parent else {
                        break;
                    };
                    scope = parent;
                    continue;
                }
                if matches!(function.kind, FunctionKind::Eval(_))
                    && matches!(binding.storage, BindingStorage::External(_))
                {
                    let Some(parent) = function.scopes[scope.0].parent else {
                        break;
                    };
                    scope = parent;
                    continue;
                }
                let masked_program_lexical = matches!(function.kind, FunctionKind::Script)
                    && scope == function.body_scope
                    && matches!(function.scopes[scope.0].kind, ScopeKind::ProgramBody)
                    && function.first_global_declaration_is_normal(name);
                if !masked_program_lexical {
                    return false;
                }
            }
            let Some(parent) = function.scopes[scope.0].parent else {
                break;
            };
            scope = parent;
        }
        true
    }

    pub(in crate::engine::compiler) fn register_scoped_function_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
        lexical_only: bool,
    ) -> Result<BindingId, Error> {
        let scope = self.current_ir().context.current_scope;
        if let Some(existing) = self.current_ir().binding_id_in_scope(scope, name) {
            let existing = &self.current_ir().bindings[existing.0];
            let duplicate_ordinary_function =
                existing.is_scoped_function && !existing.is_scoped_generator && !lexical_only;
            if self.current_ir().strict || !duplicate_ordinary_function {
                return Err(Error::syntax(
                    "invalid redefinition of lexical identifier",
                    source_span(conflict_span),
                ));
            }

            let function = self.current_ir_mut();
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(declaration_span)),
                );
            }
            let index = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(name.to_owned());
            let binding = function.ir.add_binding(
                scope,
                scope,
                name.to_owned(),
                BindingStorage::Local(index),
                BindingKind::Lexical { is_const: false },
                Some(declaration_span),
            );
            function.bindings[binding.0].is_scoped_function = true;
            function.bindings[binding.0].is_scoped_generator = lexical_only;
            return Ok(binding);
        }

        self.register_lexical_binding(name, declaration_span, conflict_span, false, true)?;
        let binding = self
            .current_ir()
            .binding_id_in_scope(scope, name)
            .ok_or_else(|| Error::internal("scoped function binding was not registered"))?;
        self.current_ir_mut().bindings[binding.0].is_scoped_function = true;
        self.current_ir_mut().bindings[binding.0].is_scoped_generator = lexical_only;
        Ok(binding)
    }

    pub(in crate::engine::compiler) fn ensure_annex_b_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
    ) -> Result<IrAnnexBinding, Error> {
        let eval_mode = if matches!(self.current_ir().kind, FunctionKind::Eval(_)) {
            Some(self.current_eval_declaration_mode()?)
        } else {
            None
        };

        if let Some(EvalDeclarationMode::Dynamic(object)) = eval_mode {
            let target = self.eval_dynamic_declaration_target(name, object, declaration_span)?;
            self.current_ir_mut()
                .eval_declarations
                .push(IrEvalDeclaration {
                    name: name.to_owned(),
                    target,
                    value: EvalDeclarationValue::Undefined,
                });
            return match target {
                EvalDeclarationTarget::Dynamic(_) => Ok(IrAnnexBinding::Dynamic),
                EvalDeclarationTarget::External { index, .. } => {
                    let function = self.current_ir();
                    let binding = function.scopes[function.ir.var_scope.0]
                        .bindings
                        .iter()
                        .copied()
                        .find(|binding| {
                            function.bindings[binding.0].storage == BindingStorage::External(index)
                        })
                        .ok_or_else(|| {
                            Error::internal("Annex B external target has no binding identity")
                        })?;
                    Ok(IrAnnexBinding::Static(binding))
                }
            };
        }

        let function = self.current_ir_mut();
        let root = function.ir.var_scope;
        let global = matches!(function.kind, FunctionKind::Script)
            || eval_mode == Some(EvalDeclarationMode::Global);
        if eval_mode == Some(EvalDeclarationMode::Global)
            && function
                .external_bindings
                .iter()
                .find(|binding| binding.name.to_utf8_lossy() == name)
                .is_some_and(|binding| binding.is_lexical && !binding.is_catch_parameter)
            && function.eval_redeclaration.is_none()
        {
            function.eval_redeclaration = Some(name.to_owned());
        }
        if global {
            function.global_declarations.push(IrGlobalDeclaration {
                name: name.to_owned(),
                is_lexical: false,
                is_const: false,
                function_constant: None,
                closure_index: None,
            });
        }
        if let Some(binding) =
            function.scopes[root.0]
                .bindings
                .iter()
                .rev()
                .copied()
                .find(|binding| {
                    let binding = &function.bindings[binding.0];
                    binding.name == name
                        && match (function.kind, eval_mode) {
                            (FunctionKind::Script, _) => binding.storage == BindingStorage::Global,
                            (FunctionKind::Module, _) => false,
                            (
                                FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow,
                                _,
                            ) => {
                                matches!(binding.storage, BindingStorage::Local(_))
                            }
                            (FunctionKind::Eval(_), Some(EvalDeclarationMode::Global)) => {
                                binding.storage == BindingStorage::Global
                            }
                            (FunctionKind::Eval(_), Some(EvalDeclarationMode::Local)) => {
                                matches!(binding.storage, BindingStorage::Local(_))
                            }
                            (FunctionKind::Eval(_), Some(EvalDeclarationMode::Dynamic(_)))
                            | (FunctionKind::Eval(_), None) => false,
                        }
                })
        {
            if function.bindings[binding.0].kind != BindingKind::Normal {
                return Err(Error::internal(
                    "Annex B declaration found a malformed function-root binding",
                ));
            }
            return Ok(IrAnnexBinding::Static(binding));
        }

        let storage = if global {
            BindingStorage::Global
        } else {
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(declaration_span)),
                );
            }
            let index = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(name.to_owned());
            BindingStorage::Local(index)
        };
        let binding = function.ir.add_binding(
            root,
            root,
            name.to_owned(),
            storage,
            BindingKind::Normal,
            Some(declaration_span),
        );
        Ok(IrAnnexBinding::Static(binding))
    }
}
