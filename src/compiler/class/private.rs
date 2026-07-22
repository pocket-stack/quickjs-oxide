//! QuickJS-shaped private field and synchronous-method declaration lowering.
//!
//! Each `#name` owns a lexical cell in the class body's dedicated private-name
//! scope. Data fields initialize that cell with a fresh private identity;
//! methods initialize it with the callable whose HomeObject carries the class
//! side's brand. Aggregate initializer children install the corresponding
//! brand or consume field identities through `DefinePrivateField`.

use super::super::*;
use super::ClassElementState;
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

    /// Parse and publish an ordinary synchronous private method. The method
    /// body is parsed before the namespace binding is registered, matching
    /// QuickJS's diagnostic priority when a malformed body and a duplicate
    /// private spelling coexist.
    pub(super) fn parse_private_class_method(
        &mut self,
        elements: &mut ClassElementState,
        is_static: bool,
        name: String,
        span: Span,
    ) -> Result<(), Error> {
        if name == "#constructor" {
            return Err(Error::syntax("invalid method name", source_span(span)));
        }

        let method = self.parse_object_method_definition(span, DefineMethodKind::Method)?;
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
}
