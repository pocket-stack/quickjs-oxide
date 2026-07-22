//! Public class-field aggregate initializer lowering.
//!
//! QuickJS compiles fields into hidden instance/static functions instead of a
//! runtime descriptor list.  This module keeps that aggregation out of the
//! class grammar driver while preserving its exact evaluation phases.

use super::super::*;
use super::ClassPropertyKey;
use crate::heap::ClassInitializerKind;

#[derive(Clone, Debug, Default)]
pub(super) struct ClassElementState {
    instance_initializer: Option<FunctionId>,
    static_initializer: Option<FunctionId>,
    instance_computed_count: usize,
    static_computed_count: usize,
}

impl<'source> Parser<'source> {
    pub(super) fn ensure_class_initializer(
        &mut self,
        elements: &mut ClassElementState,
        is_static: bool,
        span: Span,
    ) -> Result<FunctionId, Error> {
        let existing = if is_static {
            elements.static_initializer
        } else {
            elements.instance_initializer
        };
        if let Some(existing) = existing {
            return Ok(existing);
        }

        let parent = self.current_function;
        let child = self.functions.len();
        let definition_scope = self.current_ir().current_scope;
        self.functions.push(FunctionIr::new(
            Some(ParentLink {
                function: parent,
                definition_scope,
            }),
            FunctionKind::Method,
            FunctionSourceInfo {
                span,
                definition: source_offset(span)?,
                range: None,
            },
            FunctionIrOptions {
                function_name: Some(if is_static {
                    "<class_static_init>".to_owned()
                } else {
                    "<class_fields_init>".to_owned()
                }),
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
        self.functions[child].class_initializer_kind = Some(if is_static {
            ClassInitializerKind::StaticElements
        } else {
            ClassInitializerKind::InstanceFields
        });
        self.functions[child].arguments_forbidden = true;
        // QuickJS installs HomeObject on both aggregate initializer closures
        // even when only a nested static block (or later nested arrow) reads
        // `super`. The internal edge also lets CallClassStaticBlock inherit the
        // constructor without exposing it on the operand stack.
        self.functions[child].needs_home_object = true;
        if is_static {
            elements.static_initializer = Some(child);
        } else {
            elements.instance_initializer = Some(child);
        }
        Ok(child)
    }

    pub(super) fn parse_public_class_field(
        &mut self,
        elements: &mut ClassElementState,
        is_static: bool,
        key: ClassPropertyKey,
        span: Span,
    ) -> Result<(), Error> {
        if matches!(&key, ClassPropertyKey::Fixed { value }
            if *value == JsString::from_static("constructor")
                || *value == JsString::from_static("prototype"))
        {
            // Pinned QuickJS 2026-06-04 rejects only syntactically fixed
            // constructor/prototype fields. A computed key which canonicalizes
            // to either spelling remains valid.
            return Err(Error::syntax("invalid field name", source_span(span)));
        }

        let parent = self.current_function;
        let child = self.ensure_class_initializer(elements, is_static, span)?;
        let computed_binding = match &key {
            ClassPropertyKey::Fixed { .. } => None,
            ClassPropertyKey::Computed => {
                let count = if is_static {
                    &mut elements.static_computed_count
                } else {
                    &mut elements.instance_computed_count
                };
                let hidden = if is_static {
                    format!("<static_computed_field>{count}")
                } else {
                    format!("<computed_field>{count}")
                };
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "too many class fields"))?;
                self.register_lexical_binding(&hidden, span, self.current().span, true, false)?;
                // The canonical PropertyKey left by parse_class_property_key is
                // consumed now, before any field initializer executes.
                self.emit_identifier(hidden.clone(), span, IdentifierAccess::Initialize)?;
                Some(hidden)
            }
            ClassPropertyKey::Private { .. } => {
                unreachable!("private field reached public field lowering")
            }
        };

        self.current_function = child;
        self.emit_instruction(Instruction::PushThis)?;
        if let Some(hidden) = &computed_binding {
            self.emit_identifier(hidden.clone(), span, IdentifierAccess::Get)?;
        }

        if self.consume_punctuator(Punctuator::Equal)? {
            self.parse_assignment()?;
        } else {
            self.emit_instruction(Instruction::Undefined)?;
            self.anonymous_function_definition = None;
        }

        let anonymous = self.take_anonymous_function_definition();
        match key {
            ClassPropertyKey::Fixed { value } => {
                let key = self.add_constant(IrConstant::Primitive(Value::String(value)))?;
                if let Some(definition) = anonymous {
                    self.emit_anonymous_set_name(definition, Instruction::SetName(key))?;
                }
                self.emit_instruction(Instruction::DefineField(key))?;
            }
            ClassPropertyKey::Computed => {
                if let Some(definition) = anonymous {
                    self.emit_anonymous_set_name(definition, Instruction::SetNameComputed)?;
                }
                self.emit_instruction(Instruction::DefineFieldComputed)?;
            }
            ClassPropertyKey::Private { .. } => {
                unreachable!("private field reached public field lowering")
            }
        }
        self.emit_instruction(Instruction::Drop)?;
        self.current_function = parent;
        self.anonymous_function_definition = None;
        self.consume_statement_terminator()
    }

    pub(super) fn finish_class_instance_initializer(
        &mut self,
        elements: &mut ClassElementState,
    ) -> Result<(), Error> {
        let Some(child) = elements.instance_initializer.take() else {
            return Ok(());
        };
        let parent = self.current_function;
        self.current_function = child;
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::Return)?;
        self.current_function = parent;
        let constant = self.add_constant(IrConstant::Child(child))?;
        self.emit(IrOp::MakeClosure(constant))?;
        self.emit_instruction(Instruction::InstallClassInstanceInitializer)?;
        Ok(())
    }

    pub(super) fn finish_class_static_initializer(
        &mut self,
        elements: &mut ClassElementState,
    ) -> Result<Option<usize>, Error> {
        let Some(child) = elements.static_initializer.take() else {
            return Ok(None);
        };
        let parent = self.current_function;
        self.current_function = child;
        self.emit_instruction(Instruction::Undefined)?;
        self.emit_instruction(Instruction::Return)?;
        self.current_function = parent;
        let constant = self.add_constant(IrConstant::Child(child))?;
        let start = self.current_ir().ops.len();
        self.emit(IrOp::MakeClosure(constant))?;
        self.emit_instruction(Instruction::RunClassStaticInitializer)?;
        Ok(Some(start))
    }
}
