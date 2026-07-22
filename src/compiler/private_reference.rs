//! Class-private data-field references and late lexical resolution.
//!
//! QuickJS keeps `#name` operations unresolved until every class declaration
//! and nested function is known.  Oxide follows that shape, but resolves each
//! operation directly to a typed local/closure source so the private atom never
//! enters the JavaScript-visible operand stack.

use super::*;

pub(super) fn private_binding_name(name: &str) -> String {
    let mut binding = String::with_capacity(name.len().saturating_add(1));
    binding.push('#');
    binding.push_str(name);
    binding
}

impl<'source> Parser<'source> {
    pub(super) fn emit_private_field_get(
        &mut self,
        name: String,
        span: Span,
        site: SourceOffset,
    ) -> Result<usize, Error> {
        let scope = self.current_ir().current_scope;
        self.emit_at(
            IrOp::PrivateField {
                name,
                span,
                scope,
                access: PrivateFieldAccess::Get,
            },
            site,
        )
    }

    pub(super) fn emit_private_field_operation(
        &mut self,
        name: String,
        span: Span,
        scope: ScopeId,
        access: PrivateFieldAccess,
        site: SourceOffset,
    ) -> Result<usize, Error> {
        self.emit_at(
            IrOp::PrivateField {
                name,
                span,
                scope,
                access,
            },
            site,
        )
    }

    /// Parse the special relational head `#name in ShiftExpression`.
    /// A bare private identifier remains invalid in every other expression
    /// position, so the ordinary PrimaryExpression parser never handles it.
    pub(super) fn parse_private_in_head(&mut self) -> Result<bool, Error> {
        if self.in_mode == InMode::Disallow {
            return Ok(false);
        }
        if !matches!(self.current().kind, TokenKind::PrivateIdentifier(_)) {
            return Ok(false);
        }

        let mut lexer = self.lexer.clone();
        lexer.seek(self.current().span.end);
        let next = lexer.next_token().map_err(lex_error)?;
        if !matches!(next.kind, TokenKind::Keyword(Keyword::In)) {
            return Ok(false);
        }

        let token = self.current().clone();
        let TokenKind::PrivateIdentifier(identifier) = token.kind else {
            unreachable!("private-in probe changed the current token")
        };
        let name = private_binding_name(&identifier.value);
        let scope = self.current_ir().current_scope;
        self.advance()?;
        if !matches!(self.current().kind, TokenKind::Keyword(Keyword::In)) {
            return Err(Error::internal(
                "private-in lookahead disagreed with the parser",
            ));
        }
        self.advance()?;
        self.parse_shift()?;
        self.emit_private_field_operation(
            name,
            token.span,
            scope,
            PrivateFieldAccess::In,
            source_offset(token.span)?,
        )?;
        self.anonymous_function_definition = None;
        Ok(true)
    }
}

pub(super) fn resolve_private_field_operation(
    tree: &mut FunctionTree,
    consuming_function: FunctionId,
    use_scope: ScopeId,
    name: &str,
    span: Span,
    access: PrivateFieldAccess,
) -> Result<IrOp, Error> {
    let mut owner = consuming_function;
    let mut scope = use_scope;

    loop {
        loop {
            let (parent, exact) = {
                let function = tree
                    .functions
                    .get(owner)
                    .ok_or_else(|| Error::internal("private-name owner is out of bounds"))?;
                let current = function
                    .scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("private-name use scope is out of bounds"))?;
                let exact = current.bindings.iter().rev().find_map(|binding| {
                    let binding = function.bindings.get(binding.0)?;
                    (binding.name == name).then_some(ResolvedBinding {
                        storage: binding.storage,
                        kind: binding.kind,
                    })
                });
                (current.parent, exact)
            };

            if let Some(binding) = exact {
                if !matches!(
                    binding.kind,
                    BindingKind::PrivateField { .. } | BindingKind::PrivateMethod { .. }
                ) {
                    return Err(Error::internal(
                        "private spelling resolved to a non-private binding",
                    ));
                }
                let source = if owner == consuming_function {
                    match binding.storage {
                        BindingStorage::Local(index) => PrivateNameSource::Local(index),
                        BindingStorage::External(index) => PrivateNameSource::Closure(index),
                        BindingStorage::Argument(_) | BindingStorage::Global => {
                            return Err(Error::internal(
                                "private name occupied non-lexical storage",
                            ));
                        }
                    }
                } else {
                    let (index, kind) = capture_binding_path(
                        tree,
                        owner,
                        consuming_function,
                        binding,
                        name,
                        true,
                        false,
                    )?;
                    if !binding_kinds_compatible(kind, binding.kind) {
                        return Err(Error::internal(
                            "private-name closure relay changed binding kind",
                        ));
                    }
                    PrivateNameSource::Closure(index)
                };
                let instruction = match (binding.kind, access) {
                    (_, PrivateFieldAccess::Get) => Instruction::GetPrivateField(source),
                    (_, PrivateFieldAccess::GetKeepReceiver) => {
                        Instruction::GetPrivateField2(source)
                    }
                    (BindingKind::PrivateField { .. }, PrivateFieldAccess::Put) => {
                        Instruction::PutPrivateField(source)
                    }
                    (BindingKind::PrivateField { .. }, PrivateFieldAccess::Define) => {
                        Instruction::DefinePrivateField(source)
                    }
                    (_, PrivateFieldAccess::In) => Instruction::PrivateIn(source),
                    (BindingKind::PrivateMethod { .. }, PrivateFieldAccess::Put) => {
                        let name = ensure_string_constant(
                            tree.functions.get_mut(consuming_function).ok_or_else(|| {
                                Error::internal("private-name consumer is out of bounds")
                            })?,
                            name,
                        )?;
                        Instruction::ThrowReadOnly(name)
                    }
                    (BindingKind::PrivateMethod { .. }, PrivateFieldAccess::Define) => {
                        return Err(Error::internal(
                            "private method reached data-field definition lowering",
                        ));
                    }
                    (
                        BindingKind::Normal
                        | BindingKind::Lexical { .. }
                        | BindingKind::FunctionName { .. }
                        | BindingKind::EvalVariableObject
                        | BindingKind::ArgEvalVariableObject
                        | BindingKind::WithObject,
                        _,
                    ) => {
                        return Err(Error::internal(
                            "private spelling resolved to a non-private binding",
                        ));
                    }
                };
                return Ok(IrOp::Bytecode(instruction));
            }

            let Some(parent) = parent else {
                break;
            };
            scope = parent;
        }

        let Some(parent) = tree.functions[owner].parent else {
            break;
        };
        owner = parent.function;
        scope = parent.definition_scope;
    }

    Err(syntax_atom_error(
        "undefined private field '",
        name,
        "'",
        span,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::UnlinkedFunction;

    fn collect_functions<'a>(function: &'a UnlinkedFunction, out: &mut Vec<&'a UnlinkedFunction>) {
        out.push(function);
        for child in function
            .constants()
            .iter()
            .filter_map(|constant| constant.as_child())
        {
            collect_functions(child, out);
        }
    }

    #[test]
    fn class_private_scope_keeps_field_staticness_and_computed_key_cells() {
        let tree = Parser::parse(
            "class C { [#later in {}] = 1; #instance; static #static = 2; #later; }",
            JsString::from_static("<private-scope-test>"),
        )
        .unwrap();
        let function = &tree.functions[0];
        let scope = function
            .scopes
            .iter()
            .find(|scope| scope.kind == ScopeKind::ClassPrivate)
            .expect("class-private scope");
        let bindings = scope
            .bindings
            .iter()
            .map(|binding| &function.bindings[binding.0])
            .collect::<Vec<_>>();

        assert!(bindings.iter().any(|binding| {
            binding.name == "#instance"
                && binding.kind == BindingKind::PrivateField { is_static: false }
        }));
        assert!(bindings.iter().any(|binding| {
            binding.name == "#static"
                && binding.kind == BindingKind::PrivateField { is_static: true }
        }));
        assert!(bindings.iter().any(|binding| {
            binding.name == "#later"
                && binding.kind == BindingKind::PrivateField { is_static: false }
        }));
        assert!(bindings.iter().any(|binding| {
            binding.name.starts_with("<computed_field>")
                && binding.kind == BindingKind::Lexical { is_const: true }
        }));
        assert!(function.ops.iter().any(|operation| matches!(
            &operation.op,
            IrOp::PrivateField {
                name,
                access: PrivateFieldAccess::In,
                ..
            } if name == "#later"
        )));
    }

    #[test]
    fn private_data_fields_lower_to_authenticated_sources_and_lifecycles() {
        let root = compile_unlinked_script(
            r#"
                class C {
                    #x = 1;
                    #fn = function(value) { return this.#x + value };
                    static #s = 2;
                    read() { return this.#x }
                    call() { return this.#fn(0) }
                    update() { return ++this.#x + this.#x++ + (this.#x += 1) }
                    assign(value) {
                        [this.#x] = [value];
                        ({ value: this.#x } = { value });
                        return this.#x;
                    }
                    has(value) { const probe = () => #x in value; return probe() }
                    static readStatic() { return this.#s }
                }
            "#,
        )
        .unwrap();

        let private_locals = root
            .local_definitions()
            .iter()
            .enumerate()
            .filter_map(|(index, definition)| {
                (definition.kind == ClosureVariableKind::PrivateField)
                    .then_some((u16::try_from(index).unwrap(), definition))
            })
            .collect::<Vec<_>>();
        assert_eq!(private_locals.len(), 3);
        for &(index, definition) in &private_locals {
            assert!(definition.is_lexical);
            assert!(definition.is_const);
            assert!(definition.name.as_ref().is_some_and(|name| {
                matches!(name.to_utf8_lossy().as_str(), "#x" | "#fn" | "#s")
            }));
            assert!(
                root.code()
                    .iter()
                    .any(|instruction| matches!(instruction, Instruction::SetLocalUninitialized(actual) if *actual == index))
            );
            assert!(
                root.code()
                    .iter()
                    .any(|instruction| matches!(instruction, Instruction::InitializePrivateName(actual) if *actual == index))
            );
            assert!(
                root.code()
                    .iter()
                    .any(|instruction| matches!(instruction, Instruction::CloseLocal(actual) if *actual == index))
            );
        }

        let mut functions = Vec::new();
        collect_functions(&root, &mut functions);
        assert!(functions.iter().any(|function| {
            function
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetPrivateField(_)))
        }));
        assert!(functions.iter().any(|function| {
            function
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::GetPrivateField2(_)))
        }));
        assert!(functions.iter().any(|function| {
            function
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::PutPrivateField(_)))
        }));
        assert!(functions.iter().any(|function| {
            function
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::DefinePrivateField(_)))
        }));
        assert!(functions.iter().any(|function| {
            function
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::PrivateIn(_)))
        }));

        let private_closures = functions
            .iter()
            .flat_map(|function| function.closure_variables())
            .filter(|descriptor| descriptor.kind == ClosureVariableKind::PrivateField)
            .collect::<Vec<_>>();
        assert!(!private_closures.is_empty());
        assert!(
            private_closures
                .iter()
                .all(|descriptor| descriptor.is_lexical && descriptor.is_const)
        );
    }

    #[test]
    fn direct_eval_descriptors_retain_private_name_authority() {
        let root =
            compile_unlinked_script("class C { #x = 42; read() { return eval('this.#x') } }")
                .unwrap();
        let mut functions = Vec::new();
        collect_functions(&root, &mut functions);
        let eval_binding = functions
            .iter()
            .flat_map(|function| function.eval_environments())
            .flat_map(|environment| environment.scopes.iter())
            .flat_map(|scope| scope.bindings.iter())
            .find(|binding| binding.kind == ClosureVariableKind::PrivateField)
            .expect("eval-visible private binding");
        assert!(eval_binding.is_lexical);
        assert!(eval_binding.is_const);
        assert_eq!(eval_binding.name.to_utf8_lossy(), "#x");
    }

    #[test]
    fn private_methods_lower_to_typed_cells_home_objects_and_side_brands() {
        let tree = Parser::parse(
            r#"
                class C {
                    call(value) { return value.#later() }
                    #later() { return 1 }
                    static read(value) { return #staticMethod in value }
                    static #staticMethod() { return 2 }
                }
            "#,
            JsString::from_static("<private-method-scope-test>"),
        )
        .unwrap();
        let root = &tree.functions[0];
        let scope = root
            .scopes
            .iter()
            .find(|scope| scope.kind == ScopeKind::ClassPrivate)
            .expect("class-private scope");
        let bindings = scope
            .bindings
            .iter()
            .map(|binding| &root.bindings[binding.0])
            .collect::<Vec<_>>();
        assert!(bindings.iter().any(|binding| {
            binding.name == "#later"
                && binding.kind == BindingKind::PrivateMethod { is_static: false }
        }));
        assert!(bindings.iter().any(|binding| {
            binding.name == "#staticMethod"
                && binding.kind == BindingKind::PrivateMethod { is_static: true }
        }));

        let initialized = root
            .ops
            .iter()
            .filter_map(|operation| match &operation.op {
                IrOp::Bytecode(Instruction::InitializePrivateMethod(local)) => Some(*local),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(initialized.len(), 2);
        assert!(initialized.iter().all(|local| {
            root.bindings.iter().any(|binding| {
                binding.storage == BindingStorage::Local(*local)
                    && matches!(binding.kind, BindingKind::PrivateMethod { .. })
            })
        }));

        let private_methods = tree
            .functions
            .iter()
            .filter(|function| {
                function.parent.is_some()
                    && function.class_initializer_kind.is_none()
                    && function.function_name.is_none()
                    && function.needs_home_object
            })
            .collect::<Vec<_>>();
        assert!(private_methods.len() >= 2);
        let branded_initializers = tree
            .functions
            .iter()
            .filter(|function| function.class_private_brand)
            .collect::<Vec<_>>();
        assert_eq!(branded_initializers.len(), 2);
        assert!(branded_initializers.iter().any(|function| {
            function.class_initializer_kind == Some(ClassInitializerKind::InstanceFields)
        }));
        assert!(branded_initializers.iter().any(|function| {
            function.class_initializer_kind == Some(ClassInitializerKind::StaticElements)
        }));
    }

    #[test]
    fn private_method_closure_and_eval_descriptors_keep_callable_kind() {
        let root = compile_unlinked_script(
            r#"
                class C {
                    use(value) {
                        const nested = () => value.#method;
                        eval('value.#method');
                        return nested();
                    }
                    assign(value) { this.#method = value }
                    #method() { return 42 }
                }
            "#,
        )
        .unwrap();
        let method_local = root
            .local_definitions()
            .iter()
            .enumerate()
            .find(|(_, definition)| definition.kind == ClosureVariableKind::PrivateMethod)
            .expect("private method local");
        assert!(method_local.1.is_lexical);
        assert!(method_local.1.is_const);
        assert!(
            method_local
                .1
                .name
                .as_ref()
                .is_some_and(|name| name.to_utf8_lossy() == "#method")
        );
        assert!(root.code().iter().any(|instruction| matches!(
            instruction,
            Instruction::InitializePrivateMethod(index)
                if usize::from(*index) == method_local.0
        )));

        let mut functions = Vec::new();
        collect_functions(&root, &mut functions);
        assert!(functions.iter().any(|function| {
            function
                .closure_variables()
                .iter()
                .any(|descriptor| descriptor.kind == ClosureVariableKind::PrivateMethod)
        }));
        assert!(functions.iter().any(|function| {
            function
                .eval_environments()
                .iter()
                .flat_map(|environment| environment.scopes.iter())
                .flat_map(|scope| scope.bindings.iter())
                .any(|binding| {
                    binding.kind == ClosureVariableKind::PrivateMethod
                        && binding.name.to_utf8_lossy() == "#method"
                })
        }));
        assert!(functions.iter().any(|function| {
            function
                .code()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::ThrowReadOnly(_)))
        }));
    }

    #[test]
    fn private_data_field_early_errors_remain_fail_closed() {
        for source in [
            "({}).#missing",
            "#missing in {}",
            "class C { #x; #x; }",
            "class C { #x; static #x; }",
            "class C { #constructor; }",
            "class C { #x; method(value) { delete value.#x; } }",
            "class C extends Object { #x; method() { return super.#x; } }",
            "class C { #x; method() { return #x; } }",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Syntax, "source: {source}");
        }

        for source in [
            "class C { get #value() {} }",
            "class C { set #value(value) {} }",
        ] {
            let error = compile_unlinked_script(source).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Unsupported, "source: {source}");
        }

        assert_eq!(
            compile_unlinked_script("class C { #constructor; }")
                .unwrap_err()
                .message(),
            "invalid method name"
        );
        assert_eq!(
            compile_unlinked_script("class C { #x; static #x; }")
                .unwrap_err()
                .message(),
            "private class field is already defined"
        );
        assert_eq!(
            compile_unlinked_script("class C { #constructor() {} }")
                .unwrap_err()
                .message(),
            "invalid method name"
        );
        for source in [
            "class C { #x; #x() {} }",
            "class C { #x() {} #x; }",
            "class C { #x() {} static #x() {} }",
        ] {
            assert_eq!(
                compile_unlinked_script(source).unwrap_err().message(),
                "private class field is already defined",
                "source: {source}"
            );
        }
        assert_ne!(
            compile_unlinked_script("class C { #x; #x(value =) {} }")
                .unwrap_err()
                .message(),
            "private class field is already defined"
        );
    }
}
