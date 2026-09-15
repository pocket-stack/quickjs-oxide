//! Static initialization block lowering.
//!
//! Each block is a non-escaping child of the aggregate static-elements
//! initializer.  A separate FunctionIr gives every block its own var and
//! lexical environment while the typed call preserves class `this`, super
//! HomeObject, abrupt completion, and source ordering.

use crate::engine::api::error::Error;
use crate::engine::code::bytecode::Instruction;
use crate::engine::compiler::lexer::Punctuator;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::model::ir::IrConstant;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::function::FunctionIrOptions;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::model::ir::function::FunctionSourceInfo;
use crate::engine::compiler::model::ir::function::ParentLink;
use crate::engine::compiler::model::ir::function::SuperCapabilities;
use crate::engine::compiler::parser::builder::FunctionBuilder;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::diagnostics::source_offset;

use super::ClassElementState;
use crate::engine::code::function::metadata::ClassInitializerKind;

impl<'source> Parser<'source> {
    pub(super) fn parse_class_static_block(
        &mut self,
        elements: &mut ClassElementState,
        span: Span,
    ) -> Result<(), Error> {
        let outer = self.current_function;
        let aggregate = self.ensure_class_initializer(elements, true, span)?;
        let child = self.functions.len();
        let definition_scope = self.functions[aggregate].context.current_scope;
        self.functions.push(FunctionBuilder::new(
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
        self.functions[child].needs_home_object = true;

        self.current_function = child;
        // QuickJS's JS_PARSE_FUNC_CLASS_STATIC_INIT makes an unescaped
        // `await` a keyword while retaining a synchronous function kind. Let
        // each surrounding grammar production choose its own diagnostic.
        let parent_context = self.lexer.context();
        let mut block_context = parent_context;
        block_context.async_function = true;
        self.set_future_lex_context(block_context);
        self.expect_punctuator(Punctuator::LeftBrace)?;
        self.parse_function_body()?;
        let closing = self.current().span;
        self.relex_current_with_context(parent_context)?;
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
