//! QuickJS-shaped private data-field declaration lowering.
//!
//! Each `#name` owns a lexical cell in the class body's dedicated private-name
//! scope. Class evaluation initializes that cell with a fresh private identity;
//! aggregate instance/static initializer children capture the cell and consume
//! it only through `DefinePrivateField`.

use super::super::*;
use super::ClassElementState;

impl<'source> Parser<'source> {
    fn register_private_field_binding(
        &mut self,
        name: &str,
        span: Span,
        is_static: bool,
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
            BindingKind::PrivateField { is_static },
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

        let local = self.register_private_field_binding(&name, span, is_static)?;
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
}
