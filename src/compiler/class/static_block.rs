//! Static initialization block lowering.
//!
//! Each block is a non-escaping child of the aggregate static-elements
//! initializer.  A separate FunctionIr gives every block its own var and
//! lexical environment while the typed call preserves class `this`, super
//! HomeObject, abrupt completion, and source ordering.

use super::super::*;
use super::ClassElementState;
use crate::heap::ClassInitializerKind;

impl<'source> Parser<'source> {
    pub(super) fn parse_class_static_block(
        &mut self,
        elements: &mut ClassElementState,
        span: Span,
    ) -> Result<(), Error> {
        let outer = self.current_function;
        let aggregate = self.ensure_class_initializer(elements, true, span)?;
        let child = self.functions.len();
        let definition_scope = self.functions[aggregate].current_scope;
        self.functions.push(FunctionIr::new(
            Some(ParentLink {
                function: aggregate,
                definition_scope,
            }),
            FunctionKind::Method,
            FunctionSourceInfo {
                span,
                definition: source_offset(span)?,
                range: None,
            },
            FunctionIrOptions {
                function_name: Some("<class_static_block>".to_owned()),
                private_name_binding: false,
                class_constructor: false,
                derived_class_constructor: false,
                parameters: Vec::new(),
                defined_argument_count: 0,
                has_simple_parameter_list: true,
                rest_parameter: None,
                strict: true,
                super_capabilities: SuperCapabilities::PROPERTY,
            },
        )?);
        self.functions[child].class_initializer_kind = Some(ClassInitializerKind::StaticBlock);
        self.functions[child].arguments_forbidden = true;
        self.functions[child].await_forbidden = true;
        self.functions[child].await_binding_forbidden = true;
        self.functions[child].needs_home_object = true;

        self.current_function = child;
        self.expect_punctuator(Punctuator::LeftBrace)?;
        self.parse_function_body()?;
        let closing = self.current().span;
        self.expect_punctuator(Punctuator::RightBrace)?;
        self.functions[child].source.range = Some(source_offset(span)?..source_offset(closing)?);

        self.current_function = aggregate;
        let constant = self.add_constant(IrConstant::Child(child))?;
        self.emit(IrOp::MakeClosure(constant))?;
        self.emit_instruction(Instruction::CallClassStaticBlock)?;
        self.current_function = outer;
        self.anonymous_function_definition = None;
        Ok(())
    }
}
