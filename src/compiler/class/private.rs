//! QuickJS-shaped private field, method, and accessor declaration
//! lowering.
//!
//! Each `#name` owns a lexical cell in the class body's dedicated private-name
//! scope. Data fields initialize that cell with a fresh private identity;
//! methods initialize it with the callable whose HomeObject carries the class
//! side's brand. Aggregate initializer children install the corresponding
//! brand or consume field identities through `DefinePrivateField`.

use super::super::*;
use super::{ClassElementState, ClassMethodFlavor};
use crate::bytecode::DefineMethodKind;

impl<'source> Parser<'source> {
    fn register_private_binding(
        &mut self,
        name: &str,
        span: Span,
        kind: BindingKind,
    ) -> Result<u16, Error> {
        let function = self.current_ir_mut();
        let scope = function.current_scope;
        if function
            .scopes
            .get(scope.0)
            .is_none_or(|scope| scope.kind != ScopeKind::ClassPrivate)
        {
            return Err(Error::internal(
                "private field declaration escaped the class-private scope",
            ));
        }
        if function.binding_id_in_scope(scope, name).is_some() {
            return Err(Error::syntax(
                "private class field is already defined",
                source_span(span),
            ));
        }
        if function.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(
                Error::new(ErrorKind::JsInternal, "too many local variables")
                    .with_span(source_span(span)),
            );
        }
        let local = u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        function.locals.push(name.to_owned());
        function.add_binding(
            scope,
            scope,
            name.to_owned(),
            BindingStorage::Local(local),
            kind,
            Some(span),
        );
        Ok(local)
    }

    /// Preflight and register or pair the source-visible private accessor
    /// binding. QuickJS performs this primary-name step before parsing the
    /// accessor function, but reserves a setter's synthetic `<set>` storage
    /// only after the complete parameter list and body have been accepted.
    fn register_private_accessor_primary(
        &mut self,
        name: &str,
        span: Span,
        is_static: bool,
        method_kind: DefineMethodKind,
    ) -> Result<u16, Error> {
        let incoming = match method_kind {
            DefineMethodKind::Getter => BindingKind::PrivateGetter { is_static },
            DefineMethodKind::Setter => BindingKind::PrivateSetter { is_static },
            DefineMethodKind::Method => {
                return Err(Error::internal(
                    "ordinary private method reached accessor registration",
                ));
            }
        };
        let scope = self.current_ir().current_scope;
        let existing = self.current_ir().binding_id_in_scope(scope, name);

        let primary_local = if let Some(binding_id) = existing {
            let (existing_kind, storage) = {
                let binding = self
                    .current_ir()
                    .bindings
                    .get(binding_id.0)
                    .ok_or_else(|| Error::internal("private accessor binding disappeared"))?;
                (binding.kind, binding.storage)
            };
            let pairable = matches!(
                (existing_kind, incoming),
                (
                    BindingKind::PrivateGetter {
                        is_static: existing_static
                    },
                    BindingKind::PrivateSetter {
                        is_static: incoming_static
                    }
                ) | (
                    BindingKind::PrivateSetter {
                        is_static: existing_static
                    },
                    BindingKind::PrivateGetter {
                        is_static: incoming_static
                    }
                ) if existing_static == incoming_static
            );
            if !pairable {
                return Err(Error::syntax(
                    "private class field is already defined",
                    source_span(span),
                ));
            }
            self.current_ir_mut().bindings[binding_id.0].kind =
                BindingKind::PrivateGetterSetter { is_static };
            match storage {
                BindingStorage::Local(local) => local,
                BindingStorage::Argument(_)
                | BindingStorage::External(_)
                | BindingStorage::Global => {
                    return Err(Error::internal(
                        "private accessor primary binding has invalid storage",
                    ));
                }
            }
        } else {
            self.register_private_binding(name, span, incoming)?
        };
        Ok(primary_local)
    }

    fn register_private_setter_storage(
        &mut self,
        name: &str,
        span: Span,
        is_static: bool,
    ) -> Result<u16, Error> {
        let setter_name = private_reference::private_setter_binding_name(name);
        self.register_private_binding(&setter_name, span, BindingKind::PrivateSetter { is_static })
    }

    pub(super) fn parse_private_class_field(
        &mut self,
        elements: &mut ClassElementState,
        is_static: bool,
        name: String,
        span: Span,
    ) -> Result<(), Error> {
        if name == "#constructor" {
            return Err(Error::syntax("invalid method name", source_span(span)));
        }

        let local =
            self.register_private_binding(&name, span, BindingKind::PrivateField { is_static })?;
        // Unlike a normal lexical initializer, this opcode allocates and stores
        // the private Atom without ever constructing a public Symbol Value.
        self.emit_instruction_at(
            Instruction::InitializePrivateName(local),
            source_offset(span)?,
        )?;

        let parent = self.current_function;
        let child = self.ensure_class_initializer(elements, is_static, span)?;
        self.current_function = child;
        self.emit_instruction(Instruction::PushThis)?;

        if self.consume_punctuator(Punctuator::Equal)? {
            self.parse_assignment()?;
        } else {
            self.emit_instruction(Instruction::Undefined)?;
            self.anonymous_function_definition = None;
        }

        if let Some(definition) = self.take_anonymous_function_definition() {
            let name_constant = self.add_constant(IrConstant::Primitive(Value::String(
                JsString::try_from_utf8(&name)?,
            )))?;
            self.emit_anonymous_set_name(definition, Instruction::SetName(name_constant))?;
        }
        let scope = self.current_ir().current_scope;
        self.emit_private_field_operation(
            name.clone(),
            span,
            scope,
            PrivateFieldAccess::Define,
            source_offset(span)?,
        )?;
        self.emit_instruction(Instruction::Drop)?;
        self.current_function = parent;
        self.anonymous_function_definition = None;
        self.consume_statement_terminator()
    }

    /// Parse and publish an ordinary, async, or generator private method.
    /// The body is parsed before the namespace binding is registered, matching
    /// QuickJS's diagnostic priority when a malformed body and a duplicate
    /// private spelling coexist.
    pub(super) fn parse_private_class_method(
        &mut self,
        elements: &mut ClassElementState,
        is_static: bool,
        name: String,
        span: Span,
        function_span: Span,
        flavor: ClassMethodFlavor,
    ) -> Result<(), Error> {
        if name == "#constructor" {
            return Err(Error::syntax("invalid method name", source_span(span)));
        }

        let method = match flavor {
            ClassMethodFlavor::Ordinary => {
                self.parse_object_method_definition(function_span, DefineMethodKind::Method)?
            }
            ClassMethodFlavor::Generator => {
                self.parse_generator_method_definition(function_span)?
            }
            ClassMethodFlavor::Async => self.parse_async_method_definition(function_span)?,
        };
        // A private method needs HomeObject even when its authored body never
        // mentions `super`: the runtime derives its unforgeable brand from the
        // callable's HomeObject, as pinned QuickJS does.
        self.functions[method].needs_home_object = true;

        let local =
            self.register_private_binding(&name, span, BindingKind::PrivateMethod { is_static })?;
        let initializer = self.ensure_class_initializer(elements, is_static, span)?;
        self.functions[initializer].class_private_brand = true;

        // The surrounding class stack already holds the relevant HomeObject
        // immediately below the method closure. Initialization consumes the
        // closure, preserves that HomeObject, and stores a typed callable cell.
        self.emit_instruction_at(
            Instruction::InitializePrivateMethod(local),
            source_offset(span)?,
        )?;
        self.anonymous_function_definition = None;
        Ok(())
    }

    /// Parse and publish a private getter or setter. QuickJS performs the
    /// private-namespace duplicate/pairing check before parsing the accessor's
    /// parameters and body, so a conflicting spelling wins over any malformed
    /// function syntax. No `SetName` is emitted: pinned QuickJS leaves both
    /// private accessor function names as the empty string.
    pub(super) fn parse_private_class_accessor(
        &mut self,
        elements: &mut ClassElementState,
        is_static: bool,
        name: String,
        span: Span,
        function_span: Span,
        method_kind: DefineMethodKind,
    ) -> Result<(), Error> {
        if name == "#constructor" {
            return Err(Error::syntax("invalid method name", source_span(span)));
        }
        if method_kind == DefineMethodKind::Method {
            return Err(Error::internal(
                "ordinary private method reached accessor lowering",
            ));
        }

        let primary =
            self.register_private_accessor_primary(&name, span, is_static, method_kind)?;
        let accessor = self.parse_object_method_definition(function_span, method_kind)?;
        // Brand checks recover the private side's HomeObject from the callable
        // even when the body itself never evaluates `super`.
        self.functions[accessor].needs_home_object = true;

        let local = if method_kind == DefineMethodKind::Setter {
            self.register_private_setter_storage(&name, span, is_static)?
        } else {
            primary
        };

        let initializer = self.ensure_class_initializer(elements, is_static, span)?;
        self.functions[initializer].class_private_brand = true;
        self.emit_instruction_at(
            Instruction::InitializePrivateAccessor(local),
            source_offset(span)?,
        )?;
        self.anonymous_function_definition = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_accessor_parser(source: &str) -> Parser<'_> {
        let mut lexer = Lexer::new(source);
        let first_token = lexer.next_token().unwrap();
        let source_span = first_token.span;
        let root = FunctionIr::new(
            None,
            FunctionKind::Script,
            FunctionSourceInfo {
                span: source_span,
                definition: SourceOffset::try_from_usize(0).unwrap(),
                range: None,
            },
            FunctionIrOptions {
                function_name: Some("<private-accessor-order-test>".to_owned()),
                private_name_binding: false,
                class_constructor: false,
                derived_class_constructor: false,
                parameters: Vec::new(),
                defined_argument_count: 0,
                has_simple_parameter_list: true,
                rest_parameter: None,
                strict: true,
                super_capabilities: SuperCapabilities::NONE,
            },
        )
        .unwrap();
        let mut parser = Parser {
            lexer,
            tokens: vec![first_token],
            cursor: 0,
            current_function: 0,
            in_mode: InMode::Allow,
            functions: vec![root],
            anonymous_function_definition: None,
        };
        parser.push_scope(ScopeKind::ClassPrivate);
        parser
    }

    #[test]
    fn malformed_setter_does_not_reserve_synthetic_storage_before_body_parse() {
        let mut parser = private_accessor_parser("(...rest) {}");
        parser.functions[0]
            .locals
            .resize(MAX_LOCAL_VARIABLES - 1, "<padding>".to_owned());
        let span = parser.current().span;
        let mut elements = ClassElementState::default();

        let error = parser
            .parse_private_class_accessor(
                &mut elements,
                false,
                "#value".to_owned(),
                span,
                span,
                DefineMethodKind::Setter,
            )
            .unwrap_err();
        assert_eq!(
            error.message(),
            "invalid number of arguments for getter or setter"
        );

        let root = &parser.functions[0];
        assert_eq!(root.locals.len(), MAX_LOCAL_VARIABLES);
        assert!(
            root.binding_id_in_scope(root.current_scope, "#value")
                .is_some()
        );
        assert!(
            root.binding_id_in_scope(root.current_scope, "#value<set>")
                .is_none()
        );
    }
}
