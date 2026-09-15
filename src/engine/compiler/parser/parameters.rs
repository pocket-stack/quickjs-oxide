//! Formal parameter bindings and initializer environments.

use crate::engine::api::error::Error;
use crate::engine::api::error::ErrorKind;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::ParameterDefaultSource;
use crate::engine::compiler::MAX_LOCAL_VARIABLES;
use crate::engine::compiler::lexer::Punctuator;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::model::bindings::BindingKind;
use crate::engine::compiler::model::bindings::BindingStorage;
use crate::engine::compiler::model::ir::IrConstant;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::SpannedIrOp;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::model::ir::function::IrParameterPatternBinding;
use crate::engine::compiler::model::scope::IrScope;
use crate::engine::compiler::model::scope::ScopeId;
use crate::engine::compiler::model::scope::ScopeKind;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::diagnostics::source_span;
use crate::engine::value::JsString;
use crate::engine::value::PrimitiveValue as Value;

impl<'source> Parser<'source> {
    /// Add one physical argument input without deciding where authored reads
    /// resolve. A later default may promote every source binding to the
    /// independent parameter environment while retaining these slots as the
    /// call-frame input ABI.
    pub(in crate::engine::compiler) fn append_identifier_parameter(
        &mut self,
        name: String,
        span: Span,
    ) -> Result<u16, Error> {
        let function = self.current_ir_mut();
        if function.parameter_scope.is_some()
            && function
                .parameter_names
                .iter()
                .any(|parameter| parameter == &name)
        {
            return Err(Error::syntax(
                "duplicate parameter names not allowed in this context",
                source_span(span),
            ));
        }
        if function.parameters.len() >= MAX_LOCAL_VARIABLES {
            return Err(Error::new(ErrorKind::JsInternal, "too many arguments")
                .with_span(source_span(span)));
        }
        let index = u16::try_from(function.parameters.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
        function.parameters.push(Some(name.clone()));
        function.parameter_argument_locals.push(None);
        function.parameter_names.push(name.clone());
        function.ir.add_binding(
            function.ir.var_scope,
            function.ir.var_scope,
            name,
            BindingStorage::Argument(index),
            BindingKind::Normal,
            None,
        );
        Ok(index)
    }

    /// Reserve QuickJS's unnamed physical argument slot for one authored
    /// BindingPattern. Its BoundNames are registered separately as ordinary
    /// function-root variables while the raw call input remains inaccessible
    /// after the entry destructuring phase.
    pub(in crate::engine::compiler) fn append_pattern_parameter(
        &mut self,
        span: Span,
    ) -> Result<u16, Error> {
        let function = self.current_ir_mut();
        if function.parameters.len() >= MAX_LOCAL_VARIABLES {
            return Err(Error::new(ErrorKind::JsInternal, "too many arguments")
                .with_span(source_span(span)));
        }
        let index = u16::try_from(function.parameters.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
        function.parameters.push(None);
        function.parameter_argument_locals.push(None);
        function.has_simple_parameter_list = false;
        Ok(index)
    }

    /// Move the body scope entry behind non-default parameter destructuring.
    /// QuickJS evaluates these patterns in FunctionRoot: body `var` bindings
    /// exist as undefined, while body lexicals and function initializers have
    /// not been installed yet.
    pub(in crate::engine::compiler) fn activate_pattern_parameter_initialization(
        &mut self,
    ) -> Result<(), Error> {
        let function = self.current_ir_mut();
        if function.pattern_parameter_initialization {
            return Ok(());
        }
        if let Some(parameter_scope) = function.parameter_scope {
            if function.context.current_scope != parameter_scope {
                return Err(Error::internal(
                    "pattern parameter escaped its parameter environment",
                ));
            }
            function.pattern_parameter_initialization = true;
            return Ok(());
        }
        if function.context.stack_depth != 0
            || function.ops.len() != 1
            || !matches!(
                function.ops.first(),
                Some(SpannedIrOp {
                    op: IrOp::EnterScope(scope),
                    pc_site: None,
                }) if *scope == function.body_scope
            )
        {
            return Err(Error::internal(
                "pattern parameter initialization started after function body bytecode",
            ));
        }
        function.ops.clear();
        function.context.current_scope = function.ir.var_scope;
        function.pattern_parameter_initialization = true;
        Ok(())
    }

    pub(in crate::engine::compiler) fn register_pattern_parameter_binding(
        &mut self,
        name: &str,
        declaration_span: Span,
        conflict_span: Span,
    ) -> Result<(), Error> {
        let expected_scope = self
            .current_ir()
            .parameter_scope
            .unwrap_or(self.current_ir().var_scope);
        if !self.current_ir().pattern_parameter_initialization
            || self.current_ir().context.current_scope != expected_scope
        {
            return Err(Error::internal(
                "pattern parameter binding escaped its initialization phase",
            ));
        }
        if self
            .current_ir()
            .parameter_names
            .iter()
            .any(|parameter| parameter == name)
        {
            return Err(Error::syntax(
                "duplicate parameter names not allowed in this context",
                source_span(conflict_span),
            ));
        }
        self.current_ir_mut().parameter_names.push(name.to_owned());
        if self.current_ir().parameter_scope.is_none() {
            return self.register_var_binding(name, declaration_span, conflict_span);
        }

        let parameter_local =
            self.allocate_parameter_binding_local(name.to_owned(), declaration_span)?;
        self.current_ir_mut()
            .parameter_pattern_bindings
            .push(IrParameterPatternBinding {
                name: name.to_owned(),
                parameter_local,
                body_local: None,
                declaration_span,
            });
        Ok(())
    }

    pub(in crate::engine::compiler) fn allocate_parameter_binding_local(
        &mut self,
        name: String,
        span: Span,
    ) -> Result<u16, Error> {
        let function = self.current_ir_mut();
        let parameter_scope = function
            .parameter_scope
            .ok_or_else(|| Error::internal("parameter local has no parameter scope"))?;
        let local = if let Some(reserved) = function.parameter_local_reservation_count {
            let cell = function.parameter_locals.len();
            if cell >= reserved || function.locals.get(cell).is_none() {
                return Err(Error::internal(
                    "parameter binding exceeded its pre-scan reservation",
                ));
            }
            function.locals[cell] = name.clone();
            u16::try_from(cell)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?
        } else {
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(span)),
                );
            }
            let local = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(name.clone());
            local
        };
        function.parameter_locals.push(local);
        function.ir.add_binding(
            parameter_scope,
            parameter_scope,
            name,
            BindingStorage::Local(local),
            BindingKind::Lexical { is_const: false },
            Some(span),
        );
        Ok(local)
    }

    pub(in crate::engine::compiler) fn allocate_parameter_local(
        &mut self,
        argument: u16,
        span: Span,
    ) -> Result<u16, Error> {
        let argument_index = usize::from(argument);
        let name = self
            .current_ir()
            .parameters
            .get(argument_index)
            .and_then(Clone::clone)
            .ok_or_else(|| Error::internal("parameter local referenced an unnamed argument"))?;
        if self
            .current_ir()
            .parameter_argument_locals
            .get(argument_index)
            .is_none_or(Option::is_some)
        {
            return Err(Error::internal(
                "parameter argument cell was allocated more than once",
            ));
        }
        let local = self.allocate_parameter_binding_local(name, span)?;
        self.current_ir_mut().parameter_argument_locals[argument_index] = Some(local);
        Ok(local)
    }

    /// Create QuickJS's parentless argument scope. Valid source reaches this
    /// from the whole-list standalone-`=` pre-scan, before the first formal is
    /// parsed; the lazy caller remains as a defensive fallback for malformed
    /// or scanner-limit input which later exposes an identifier default.
    pub(in crate::engine::compiler) fn activate_parameter_environment_from_scan(
        &mut self,
        bound_name_count: Option<usize>,
    ) -> Result<(), Error> {
        if self.current_ir().parameter_scope.is_some() {
            return Ok(());
        }
        let scan_span = self.current().span;
        let function = self.current_ir_mut();
        if !matches!(
            function.kind,
            FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
        ) || !function.parameters.is_empty()
            || !function.parameter_names.is_empty()
            || function.pattern_parameter_initialization
            || function.ops.len() != 1
            || !matches!(
                function.ops.first(),
                Some(SpannedIrOp {
                    op: IrOp::EnterScope(scope),
                    pc_site: None,
                }) if *scope == function.body_scope
            )
        {
            return Err(Error::internal(
                "parameter environment pre-scan ran after formal parsing",
            ));
        }
        if let Some(bound_name_count) = bound_name_count {
            if !function.locals.is_empty()
                || bound_name_count > MAX_LOCAL_VARIABLES
                || function.parameter_local_reservation_count.is_some()
            {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(scan_span)),
                );
            }
            function
                .locals
                .resize(bound_name_count, "<parameter-reserved>".to_owned());
            function.parameter_local_reservation_count = Some(bound_name_count);
        }
        function.ops.clear();
        let parameter_scope = ScopeId(function.scopes.len());
        function.scopes.push(IrScope {
            parent: None,
            kind: ScopeKind::Parameter,
            is_parameter_initializer: true,
            bindings: Vec::new(),
            bindings_by_name: Default::default(),
        });
        function.parameter_scope = Some(parameter_scope);
        function.context.current_scope = parameter_scope;
        function.ops.push(SpannedIrOp {
            op: IrOp::EnterScope(parameter_scope),
            pc_site: None,
        });
        Ok(())
    }

    pub(in crate::engine::compiler) fn activate_identifier_parameter_environment(
        &mut self,
        current: u16,
        span: Span,
    ) -> Result<u16, Error> {
        if self.current_ir().parameter_scope.is_some() {
            return self.allocate_parameter_local(current, span);
        }

        let parameter_count = {
            let function = self.current_ir_mut();
            if !matches!(
                function.kind,
                FunctionKind::Ordinary | FunctionKind::Method | FunctionKind::Arrow
            ) || usize::from(current) + 1 != function.parameters.len()
                || function.pattern_parameter_initialization
                || function.parameters.iter().any(Option::is_none)
                || function.ops.len() != 1
                || !matches!(
                    function.ops.first(),
                    Some(SpannedIrOp {
                        op: IrOp::EnterScope(scope),
                        pc_site: None,
                    }) if *scope == function.body_scope
                )
            {
                return Err(Error::internal(
                    "parameter environment was activated after body bytecode",
                ));
            }
            function.ops.clear();
            let parameter_scope = ScopeId(function.scopes.len());
            function.scopes.push(IrScope {
                parent: None,
                kind: ScopeKind::Parameter,
                is_parameter_initializer: true,
                bindings: Vec::new(),
                bindings_by_name: Default::default(),
            });
            function.parameter_scope = Some(parameter_scope);
            function.context.current_scope = parameter_scope;
            function.ops.push(SpannedIrOp {
                op: IrOp::EnterScope(parameter_scope),
                pc_site: None,
            });
            function.parameters.len()
        };

        for argument in 0..parameter_count {
            let argument = u16::try_from(argument)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
            let local = self.allocate_parameter_local(argument, span)?;
            if argument < current {
                self.emit_instruction(Instruction::GetArg(argument))?;
                self.emit_instruction(Instruction::InitializeLocal(local))?;
            }
        }
        self.current_ir()
            .parameter_argument_locals
            .get(usize::from(current))
            .copied()
            .flatten()
            .ok_or_else(|| Error::internal("current parameter local was not allocated"))
    }

    pub(in crate::engine::compiler) fn register_plain_identifier_parameter(
        &mut self,
        name: String,
        span: Span,
    ) -> Result<(), Error> {
        let argument = self.append_identifier_parameter(name, span)?;
        if self.current_ir().defined_argument_count == usize::from(argument) {
            self.current_ir_mut().defined_argument_count += 1;
        }
        if self.current_ir().parameter_scope.is_some() {
            let local = self.allocate_parameter_local(argument, span)?;
            self.emit_instruction(Instruction::GetArg(argument))?;
            self.emit_instruction(Instruction::InitializeLocal(local))?;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn parse_default_identifier_parameter(
        &mut self,
        name: String,
        span: Span,
    ) -> Result<(), Error> {
        let argument = self.append_identifier_parameter(name.clone(), span)?;
        self.current_ir_mut().has_simple_parameter_list = false;
        self.current_ir_mut()
            .parameter_default_sources
            .push(ParameterDefaultSource::Argument(argument));
        let local = self.activate_identifier_parameter_environment(argument, span)?;

        self.expect_punctuator(Punctuator::Equal)?;
        self.emit_instruction(Instruction::GetArg(argument))?;
        self.emit_instruction(Instruction::Dup)?;
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::StrictEq)?;
        let has_value = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        self.emit_instruction(Instruction::Drop)?;
        self.anonymous_function_definition = None;
        self.parse_assignment_allow_in()?;
        if let Some(definition) = self.take_anonymous_function_definition() {
            let name = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&name)?,
            )))?;
            self.emit_anonymous_set_name(definition, Instruction::SetName(name))?;
        }
        self.emit_instruction(Instruction::Dup)?;
        self.emit_instruction(Instruction::PutArg(argument))?;
        let has_value_target = self.current_ir().ops.len();
        self.patch_jump(has_value, has_value_target)?;
        self.emit_instruction(Instruction::InitializeLocal(local))?;
        Ok(())
    }

    pub(in crate::engine::compiler) fn register_rest_identifier_parameter(
        &mut self,
        name: String,
        span: Span,
    ) -> Result<(), Error> {
        let argument = self.append_identifier_parameter(name, span)?;
        self.current_ir_mut().has_simple_parameter_list = false;
        self.current_ir_mut().rest_parameter = Some(argument);
        if self.current_ir().parameter_scope.is_some() {
            let local = self.allocate_parameter_local(argument, span)?;
            self.emit_instruction(Instruction::Rest(argument))?;
            self.emit_instruction(Instruction::Dup)?;
            self.emit_instruction(Instruction::PutArg(argument))?;
            self.emit_instruction(Instruction::InitializeLocal(local))?;
        } else if self.current_ir().pattern_parameter_initialization {
            self.emit_instruction(Instruction::Rest(argument))?;
            self.emit_instruction(Instruction::PutArg(argument))?;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn register_rest_pattern_parameter(
        &mut self,
    ) -> Result<u16, Error> {
        let function = self.current_ir_mut();
        if !function.pattern_parameter_initialization
            || function.rest_parameter.is_some()
            || function.rest_pattern_start.is_some()
        {
            return Err(Error::internal(
                "rest BindingPattern has malformed formal metadata",
            ));
        }
        let start = u16::try_from(function.parameters.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
        function.has_simple_parameter_list = false;
        function.rest_pattern_start = Some(start);
        Ok(start)
    }

    pub(in crate::engine::compiler) fn finish_pattern_parameter_length(
        &mut self,
        argument: u16,
        has_initializer: bool,
    ) -> Result<(), Error> {
        let function = self.current_ir_mut();
        if has_initializer {
            let source = if function.rest_pattern_start == Some(argument)
                && usize::from(argument) == function.parameters.len()
            {
                ParameterDefaultSource::RestPattern(argument)
            } else {
                ParameterDefaultSource::Argument(argument)
            };
            function.parameter_default_sources.push(source);
        }
        if !has_initializer && function.defined_argument_count == usize::from(argument) {
            function.defined_argument_count = function
                .defined_argument_count
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn allocate_parameter_pattern_body_bindings(
        &mut self,
    ) -> Result<Vec<(u16, u16)>, Error> {
        let function = self.current_ir_mut();
        let mut copies = Vec::with_capacity(function.parameter_pattern_bindings.len());
        for binding_index in 0..function.parameter_pattern_bindings.len() {
            let (name, parameter_local, declaration_span) = {
                let binding = &function.parameter_pattern_bindings[binding_index];
                (
                    binding.name.clone(),
                    binding.parameter_local,
                    binding.declaration_span,
                )
            };
            if function
                .binding_in_scope(function.ir.var_scope, &name)
                .is_some()
            {
                return Err(Error::internal(
                    "parameter pattern body binding already exists",
                ));
            }
            if function.locals.len() >= MAX_LOCAL_VARIABLES {
                return Err(
                    Error::new(ErrorKind::JsInternal, "too many local variables")
                        .with_span(source_span(declaration_span)),
                );
            }
            let body_local = u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
            function.locals.push(name.clone());
            function.ir.add_binding(
                function.ir.var_scope,
                function.ir.var_scope,
                name,
                BindingStorage::Local(body_local),
                BindingKind::Normal,
                Some(declaration_span),
            );
            function.parameter_pattern_bindings[binding_index].body_local = Some(body_local);
            copies.push((parameter_local, body_local));
        }
        Ok(copies)
    }

    pub(in crate::engine::compiler) fn finish_identifier_parameter_environment(
        &mut self,
    ) -> Result<(), Error> {
        if let Some(parameter_scope) = self.current_ir().parameter_scope {
            if self.current_ir().context.current_scope != parameter_scope
                || self.current_ir().context.stack_depth != 0
            {
                return Err(Error::internal(
                    "parameter environment finished with unbalanced parser state",
                ));
            }
            let copies = self.allocate_parameter_pattern_body_bindings()?;
            for (parameter_local, body_local) in copies.into_iter().rev() {
                self.emit_instruction(Instruction::GetLocalCheck(parameter_local))?;
                self.emit_instruction(Instruction::PutLocal(body_local))?;
            }
            self.emit(IrOp::ParameterInitializationEnd)?;
            let body_scope = self.current_ir().body_scope;
            let function = self.current_ir_mut();
            function.ops.push(SpannedIrOp {
                op: IrOp::LeaveScope(parameter_scope),
                pc_site: None,
            });
            function.context.current_scope = body_scope;
            function.ops.push(SpannedIrOp {
                op: IrOp::EnterScope(body_scope),
                pc_site: None,
            });
            return Ok(());
        }

        if self.current_ir().pattern_parameter_initialization {
            if self.current_ir().context.current_scope != self.current_ir().var_scope
                || self.current_ir().context.stack_depth != 0
            {
                return Err(Error::internal(
                    "pattern parameter initialization finished with unbalanced parser state",
                ));
            }
            let body_scope = self.current_ir().body_scope;
            let function = self.current_ir_mut();
            function.ops.push(SpannedIrOp {
                op: IrOp::ParameterInitializationEnd,
                pc_site: None,
            });
            function.context.current_scope = body_scope;
            function.ops.push(SpannedIrOp {
                op: IrOp::EnterScope(body_scope),
                pc_site: None,
            });
        }
        Ok(())
    }
}
